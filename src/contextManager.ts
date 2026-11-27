// ============================================================
// SONNE CONTEXT MANAGER (pure, UI-independent, no imports)
//
// Phase 2A: Context Budget + Smart History.
//
// Decides which part of the conversation is sent to the model. It works
// on a COPY of the history that is about to be sent; the conversation
// shown in the UI and stored by Sonne is never touched.
//
// All token numbers here are LOCAL ESTIMATES, not Ollama token counts.
// The real number is `prompt_eval_count` on Ollama's final chunk.
// ============================================================

export type ContextMessage = { role: "user" | "assistant"; content: string };

/** Structural subset of BackendConfig (modelRouting.ts), so this file needs no imports. */
export type ContextModelConfig = {
  ollamaModel: string;
  options?: { num_ctx: number; num_predict: number };
};

// ------------------------------------------------------------
// CONFIGURATION (single place to maintain)
// ------------------------------------------------------------

/**
 * Context windows (num_ctx) for models whose window is NOT sent per request,
 * i.e. models with a Sonne Modelfile. Keyed by exact Ollama model name.
 * Fill in from each Modelfile's `PARAMETER num_ctx`; do not guess.
 * Models with request-level `options.num_ctx` (Pro Instruct) use that value
 * automatically, so it can never drift from what Ollama is actually given.
 */
export const MODEL_CONTEXT_WINDOWS: Readonly<Record<string, number>> = {
  "sonne-gemma:latest": 8192,
  "sonne-pro-ai:latest": 12288,
};

/** Used for any model without a known window. Equals Ollama's classic default. */
export const FALLBACK_CONTEXT_WINDOW = 4096;

/** Share of the window kept free for the answer (more when the model thinks). */
export const OUTPUT_RESERVE_RATIO = 0.25;
export const OUTPUT_RESERVE_RATIO_THINKING = 0.35;
export const MIN_OUTPUT_RESERVE_TOKENS = 512;

/** Extra headroom because the estimator is approximate. */
export const SAFETY_MARGIN_RATIO = 0.05;

/** Rough per-message cost of role/template tokens. */
export const MESSAGE_OVERHEAD_TOKENS = 4;

// ------------------------------------------------------------
// TOKEN ESTIMATION (estimate only; deliberately on the high side)
// ------------------------------------------------------------

/**
 * Weighted character count, no tokenizer. Roughly: Cyrillic 2 chars/token,
 * Latin ~3 chars/token, punctuation/digits/symbols 2 chars/token, whitespace
 * cheap, other scripts 1 token/char. Not exact; tuned to over- not under-count.
 */
export function estimateTokens(text: string): number {
  let weight = 0;

  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);

    if (c === 32 || c === 10 || c === 9 || c === 13) weight += 0.15;
    else if ((c >= 65 && c <= 90) || (c >= 97 && c <= 122)) weight += 0.32;
    else if (c >= 0x0400 && c <= 0x052f) weight += 0.5; // Cyrillic
    else if (c < 128) weight += 0.5; // digits, punctuation, symbols
    else if (c >= 0xd800 && c <= 0xdfff) weight += 0.75; // surrogate half (emoji etc.)
    else weight += 1; // CJK and other scripts

  }

  return Math.ceil(weight);
}

export function estimateMessageTokens(message: { content: string }): number {
  return estimateTokens(message.content) + MESSAGE_OVERHEAD_TOKENS;
}


// ------------------------------------------------------------
// BOUNDED DOCUMENT CONTEXT
// ------------------------------------------------------------

export type BoundedContextSection = {
  label: string;
  content: string;
  /** Higher-priority sections receive their share first. */
  priority?: number;
};

export type BoundedContextStats = {
  status: "within-budget" | "truncated" | "partially-omitted" | "fixed-content-exceeds-budget";
  inputBudget: number;
  fixedTokens: number;
  availableContextTokens: number;
  usedContextTokens: number;
  historyReserveTokens: number;
  includedSections: number;
  truncatedSections: number;
  omittedSections: number;
};

export type BoundedContextResult = {
  prompt: string;
  stats: BoundedContextStats;
};

const CONTEXT_TRUNCATION_NOTICE =
  "\n[Частину вмісту не додано через ліміт контексту моделі. Оригінальний документ збережено; задай уточнювальне запитання для інших деталей.]";

const HISTORY_TRUNCATION_NOTICE =
  "\n[Частину попереднього повідомлення скорочено через ліміт контексту.]";

/**
 * Keep a complete prefix at a readable boundary and explicitly mark that the
 * source was shortened. The token estimator is conservative and deterministic.
 */
export function truncateTextToTokenBudget(
  text: string,
  maxTokens: number,
  notice = CONTEXT_TRUNCATION_NOTICE,
): string {
  if (!text.trim() || maxTokens <= 0) return "";
  if (estimateTokens(text) <= maxTokens) return text;
  if (estimateTokens(notice) >= maxTokens) return "";

  let low = 0;
  let high = text.length;
  let best = 0;

  while (low <= high) {
    const middle = Math.floor((low + high) / 2);
    let safeEnd = middle;
    // Do not split a UTF-16 surrogate pair.
    if (safeEnd > 0 && safeEnd < text.length) {
      const last = text.charCodeAt(safeEnd - 1);
      const next = text.charCodeAt(safeEnd);
      if (last >= 0xd800 && last <= 0xdbff && next >= 0xdc00 && next <= 0xdfff) {
        safeEnd -= 1;
      }
    }
    const candidate = text.slice(0, safeEnd).trimEnd() + notice;
    if (estimateTokens(candidate) <= maxTokens) {
      best = safeEnd;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }

  // Prefer a paragraph/word boundary when it costs no more than 15% of the fit.
  const floor = Math.floor(best * 0.85);
  const newline = text.lastIndexOf("\n", best);
  const space = text.lastIndexOf(" ", best);
  const boundary = Math.max(newline, space);
  if (boundary >= floor) best = boundary;

  let result = text.slice(0, best).trimEnd() + notice;
  while (best > 0 && estimateTokens(result) > maxTokens) {
    best = Math.floor(best * 0.95);
    result = text.slice(0, best).trimEnd() + notice;
  }
  return estimateTokens(result) <= maxTokens ? result : "";
}

/**
 * Attach document/archive/Vision material only within the actual model input
 * budget. The user request and system prompt are never truncated. A small
 * amount of capacity can be held back for whole-turn chat history.
 */
export function buildBoundedContextPrompt(input: {
  systemPrompt: string;
  userPrompt: string;
  sections: readonly BoundedContextSection[];
  budget: ContextBudget;
  historyReserveTokens?: number;
}): BoundedContextResult {
  const fixedTokens =
    estimateMessageTokens({ content: input.systemPrompt }) +
    estimateMessageTokens({ content: input.userPrompt });
  const historyReserveTokens = Math.max(
    0,
    Math.min(input.historyReserveTokens ?? 0, input.budget.inputBudget),
  );

  if (fixedTokens > input.budget.inputBudget) {
    return {
      prompt: input.userPrompt,
      stats: {
        status: "fixed-content-exceeds-budget",
        inputBudget: input.budget.inputBudget,
        fixedTokens,
        availableContextTokens: 0,
        usedContextTokens: 0,
        historyReserveTokens,
        includedSections: 0,
        truncatedSections: 0,
        omittedSections: input.sections.filter((section) => section.content.trim()).length,
      },
    };
  }

  const availableContextTokens = Math.max(
    0,
    input.budget.inputBudget - fixedTokens - historyReserveTokens,
  );
  const sections = input.sections
    .filter((section) => section.label.trim() && section.content.trim())
    .map((section, index) => ({ ...section, index }))
    .sort((a, b) => (b.priority ?? 0) - (a.priority ?? 0) || a.index - b.index);

  let remaining = availableContextTokens;
  let usedContextTokens = 0;
  let truncatedSections = 0;
  let omittedSections = 0;
  const included: string[] = [];

  for (let index = 0; index < sections.length; index += 1) {
    const section = sections[index];
    const sectionsLeft = sections.length - index;
    const share = Math.floor(remaining / sectionsLeft);
    const header = `\n\n--- ${section.label.trim()} ---\n`;
    const headerTokens = estimateTokens(header);
    const bodyBudget = Math.max(0, share - headerTokens);

    if (bodyBudget <= 0) {
      omittedSections += 1;
      continue;
    }

    const body = truncateTextToTokenBudget(section.content, bodyBudget);
    if (!body) {
      omittedSections += 1;
      continue;
    }

    const block = header + body;
    const blockTokens = estimateTokens(block);
    if (blockTokens > remaining) {
      omittedSections += 1;
      continue;
    }

    included.push(block);
    remaining -= blockTokens;
    usedContextTokens += blockTokens;
    if (body.length < section.content.length) truncatedSections += 1;
  }

  const prompt = input.userPrompt + included.join("");
  const finalTokens = estimateMessageTokens({ content: input.systemPrompt }) +
    estimateMessageTokens({ content: prompt });
  if (finalTokens + historyReserveTokens > input.budget.inputBudget) {
    // Defensive invariant: never send a prompt that this estimator says is over budget.
    throw new Error("Bounded context builder exceeded its token budget invariant.");
  }

  return {
    prompt,
    stats: {
      status: truncatedSections > 0
        ? "truncated"
        : omittedSections > 0
          ? "partially-omitted"
          : "within-budget",
      inputBudget: input.budget.inputBudget,
      fixedTokens,
      availableContextTokens,
      usedContextTokens,
      historyReserveTokens,
      includedSections: included.length,
      truncatedSections,
      omittedSections,
    },
  };
}

/** Detect explicit requests for a comprehensive, whole-document summary. */
export function isFullDocumentSummaryRequest(prompt: string): boolean {
  const normalized = prompt.trim().toLocaleLowerCase("uk-UA").replace(/[.!?]+$/u, "").trim();
  return /(?:повн(?:ий|ого|у|е)\s+(?:огляд|підсумок|резюме|аналіз)|детальн(?:ий|ого|у)\s+(?:огляд|підсумок|аналіз)|усього\s+документа|весь\s+документ|усі\s+розділи|всі\s+розділи|повністю\s+проаналізуй|full\s+(?:document\s+)?summary|comprehensive\s+summary|entire\s+document|all\s+sections|summari[sz]e\s+(?:the\s+)?(?:whole|entire)\s+(?:file|document))/iu.test(normalized);
}

// ------------------------------------------------------------
// CONTEXT BUDGET
// ------------------------------------------------------------

export type ContextBudget = {
  contextWindow: number;
  windowSource: "request-options" | "model-table" | "fallback";
  outputReserve: number;
  safetyMargin: number;
  /** Max estimated tokens for system prompt + history + current message. */
  inputBudget: number;
};

export function resolveContextBudget(
  config: ContextModelConfig,
  thinking: boolean,
): ContextBudget {
  let contextWindow = FALLBACK_CONTEXT_WINDOW;
  let windowSource: ContextBudget["windowSource"] = "fallback";

  if (config.options && config.options.num_ctx > 0) {
    contextWindow = config.options.num_ctx;
    windowSource = "request-options";
  } else if (MODEL_CONTEXT_WINDOWS[config.ollamaModel] > 0) {
    contextWindow = MODEL_CONTEXT_WINDOWS[config.ollamaModel];
    windowSource = "model-table";
  }

  const ratio = thinking ? OUTPUT_RESERVE_RATIO_THINKING : OUTPUT_RESERVE_RATIO;
  let outputReserve = Math.max(MIN_OUTPUT_RESERVE_TOKENS, Math.floor(contextWindow * ratio));

  // A smaller configured output limit needs no larger reserve.
  if (config.options && config.options.num_predict > 0) {
    outputReserve = Math.min(outputReserve, config.options.num_predict);
  }

  const safetyMargin = Math.ceil(contextWindow * SAFETY_MARGIN_RATIO);
  const inputBudget = Math.max(0, contextWindow - outputReserve - safetyMargin);

  return { contextWindow, windowSource, outputReserve, safetyMargin, inputBudget };
}

// ------------------------------------------------------------
// SMART HISTORY (recent-first, whole turns, deterministic)
// ------------------------------------------------------------

export type HistorySelectionStatus =
  | "within-budget" // everything fits; history unchanged
  | "trimmed" // oldest turns omitted
  | "fixed-content-exceeds-budget"; // system prompt + current message alone exceed the budget

export type ContextStats = {
  status: HistorySelectionStatus;
  /** True when any history was omitted from the model input. */
  optimized: boolean;
  contextWindow: number;
  windowSource: ContextBudget["windowSource"];
  inputBudget: number;
  outputReserve: number;
  // All token values below are ESTIMATES (not Ollama's prompt_eval_count).
  estFixedTokens: number; // system prompt + current user message
  estOriginalHistoryTokens: number;
  estSelectedHistoryTokens: number;
  estRemovedHistoryTokens: number;
  estInputTokens: number; // fixed + selected history
  totalHistoryMessages: number;
  selectedHistoryMessages: number;
  omittedHistoryMessages: number;
};

export type HistorySelection = {
  /** New array; the input array and its message objects are never modified. */
  history: ContextMessage[];
  stats: ContextStats;
};

function fitNewestHistoryTurn(
  turn: readonly ContextMessage[],
  availableTokens: number,
): ContextMessage[] {
  const userIndex = turn.findIndex((message) => message.role === "user");
  if (userIndex < 0 || availableTokens <= 0) return [];

  const userMessage = turn[userIndex];
  const userCost = estimateMessageTokens(userMessage);

  // Preserve the complete user question whenever possible. If it alone is
  // too large, retain a clearly marked prefix instead of discarding all history.
  if (userCost > availableTokens) {
    const content = truncateTextToTokenBudget(
      userMessage.content,
      availableTokens - MESSAGE_OVERHEAD_TOKENS,
      HISTORY_TRUNCATION_NOTICE,
    );
    return content ? [{ role: "user", content }] : [];
  }

  const selected: ContextMessage[] = [{
    role: "user",
    content: userMessage.content,
  }];
  const remainingTokens = availableTokens - userCost;
  if (remainingTokens <= MESSAGE_OVERHEAD_TOKENS) return selected;

  // In a multi-reply turn, the newest assistant response carries the latest
  // state. Keep that response after the complete user prompt; older replies
  // in the oversized turn are omitted before it.
  let latestAssistant: ContextMessage | undefined;
  for (let index = turn.length - 1; index > userIndex; index -= 1) {
    if (turn[index].role === "assistant") {
      latestAssistant = turn[index];
      break;
    }
  }
  if (!latestAssistant) return selected;

  if (estimateMessageTokens(latestAssistant) <= remainingTokens) {
    selected.push({ role: "assistant", content: latestAssistant.content });
    return selected;
  }

  const content = truncateTextToTokenBudget(
    latestAssistant.content,
    remainingTokens - MESSAGE_OVERHEAD_TOKENS,
    HISTORY_TRUNCATION_NOTICE,
  );
  if (content) selected.push({ role: "assistant", content });
  return selected;
}

export function selectHistory(
  history: readonly ContextMessage[],
  input: { systemPrompt: string; userPrompt: string; budget: ContextBudget },
): HistorySelection {
  const { budget } = input;
  const costs = history.map(estimateMessageTokens);
  const originalTokens = costs.reduce((sum, cost) => sum + cost, 0);
  const fixedTokens =
    estimateMessageTokens({ content: input.systemPrompt }) +
    estimateMessageTokens({ content: input.userPrompt });
  const available = budget.inputBudget - fixedTokens;

  let keepFrom = 0;
  let selectedOverride: ContextMessage[] | null = null;
  let status: HistorySelectionStatus = "within-budget";

  if (available < 0) {
    // System prompt and current message are never cut. Send them as they are
    // without history and let the caller surface the overflow.
    keepFrom = history.length;
    status = "fixed-content-exceeds-budget";
  } else if (originalTokens > available) {
    status = "trimmed";
    keepFrom = history.length;

    // Walk back one turn at a time (a user message plus the replies that
    // follow it). Older whole turns are retained while they fit.
    let used = 0;
    let end = history.length;

    while (end > 0) {
      let start = end - 1;
      while (start > 0 && history[start].role !== "user") start -= 1;

      // Replies with no user message before them would be orphaned.
      if (history[start].role !== "user") break;

      let turnCost = 0;
      for (let index = start; index < end; index += 1) turnCost += costs[index];

      if (used + turnCost > available) {
        // A too-large newest turn is the exception: salvage its user prompt
        // and latest assistant response within the remaining budget.
        if (end === history.length && used === 0) {
          selectedOverride = fitNewestHistoryTurn(history.slice(start, end), available);
        }
        break;
      }

      used += turnCost;
      keepFrom = start;
      end = start;
    }
  }

  const selectedSource = selectedOverride ?? history.slice(keepFrom);
  const selected = selectedSource.map((message) => ({
    role: message.role,
    content: message.content,
  }));
  const selectedTokens = selected.reduce(
    (sum, message) => sum + estimateMessageTokens(message),
    0,
  );
  const omitted = history.length - selected.length;
  const historyWasShortened = selectedOverride !== null && selected.some((message) =>
    message.content.includes(HISTORY_TRUNCATION_NOTICE),
  );

  return {
    history: selected,
    stats: {
      status,
      optimized: omitted > 0 || historyWasShortened,
      contextWindow: budget.contextWindow,
      windowSource: budget.windowSource,
      inputBudget: budget.inputBudget,
      outputReserve: budget.outputReserve,
      estFixedTokens: fixedTokens,
      estOriginalHistoryTokens: originalTokens,
      estSelectedHistoryTokens: selectedTokens,
      estRemovedHistoryTokens: originalTokens - selectedTokens,
      estInputTokens: fixedTokens + selectedTokens,
      totalHistoryMessages: history.length,
      selectedHistoryMessages: selected.length,
      omittedHistoryMessages: omitted,
    },
  };
}
