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
    // follow it). A turn that does not fit stops the walk, so what is kept
    // is always a contiguous, whole-turn tail of the conversation.
    let used = 0;
    let end = history.length;

    while (end > 0) {
      let start = end - 1;

      while (start > 0 && history[start].role !== "user") start--;

      // Replies with no user message before them would be orphaned.
      if (history[start].role !== "user") break;

      let turnCost = 0;

      for (let i = start; i < end; i++) turnCost += costs[i];
      if (used + turnCost > available) break;

      used += turnCost;
      keepFrom = start;
      end = start;
    }
  }

  const selected = history.slice(keepFrom).map((message) => ({
    role: message.role,
    content: message.content,
  }));
  const selectedTokens = costs.slice(keepFrom).reduce((sum, cost) => sum + cost, 0);
  const omitted = history.length - selected.length;

  return {
    history: selected,
    stats: {
      status,
      optimized: omitted > 0,
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
