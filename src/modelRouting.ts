// ============================================================
// SONNE MODEL CONFIGURATION + ROUTING (pure, UI-independent)
//
// Sonne has exactly two AI profiles: "pro-ai" and "gemma".
// Pro AI has two execution backends (Thinking / Instruct) that
// share the same identity, instructions and conversation context.
// Only the Ollama model and the thinking setting differ.
//
// This file has no imports so that it can be unit-tested directly.
// ============================================================

export type ModeOption = "auto" | "chat" | "code" | "image";
export type ResolvedMode = "chat" | "code" | "image";
export type ModelProfile = "pro-ai" | "gemma";
export type ExecutionBackend = "pro-thinking" | "pro-instruct" | "gemma";

/**
 * How the `think` field of /api/chat is decided:
 * - "on":          always true (Pro AI Thinking).
 * - "off":         always false (Pro AI Instruct).
 * - "user-toggle": existing Gemma behavior (Code mode + thinking toggle).
 */
export type ThinkingPolicy = "on" | "off" | "user-toggle";

export type RuntimeOptions = {
  temperature: number;
  top_p: number;
  repeat_penalty: number;
  num_ctx: number;
  num_predict: number;
};

export type BackendConfig = {
  backend: ExecutionBackend;
  profile: ModelProfile;
  ollamaModel: string;
  thinking: ThinkingPolicy;
  /**
   * Request-level options. Only set for a model that has no Sonne
   * Modelfile (the Instruct model is the unmodified library model), so
   * that it does not fall back to Ollama's default context window.
   * Values mirror the Pro AI parameters in Model_Parameters_Specification.md.
   */
  options?: RuntimeOptions;
  /** Name shown in the existing model selector, if this backend owns an entry. */
  selectorEntry: boolean;
  label: string;
};

export const BACKENDS: readonly BackendConfig[] = [
  {
    backend: "pro-thinking",
    profile: "pro-ai",
    ollamaModel: "sonne-pro-ai:latest",
    thinking: "on",
    selectorEntry: true,
    label: "Pro AI Thinking",
  },
  {
    backend: "pro-instruct",
    profile: "pro-ai",
    ollamaModel: "qwen3:30b-a3b-instruct-2507-q4_K_M",
    thinking: "off",
    options: {
      temperature: 0.65,
      top_p: 0.9,
      repeat_penalty: 1.08,
      num_ctx: 12288,
      num_predict: 8192,
    },
    selectorEntry: false,
    label: "Pro AI Instruct",
  },
  {
    backend: "gemma",
    profile: "gemma",
    ollamaModel: "sonne-gemma:latest",
    thinking: "user-toggle",
    selectorEntry: true,
    label: "Gemma",
  },
];

export function getBackendConfig(backend: ExecutionBackend): BackendConfig {
  const found = BACKENDS.find((item) => item.backend === backend);
  if (!found) throw new Error(`Unknown backend: ${backend}`);
  return found;
}

/** Exact (non-fuzzy) lookup by Ollama model name. */
export function getBackendByModelName(name: string): BackendConfig | null {
  return BACKENDS.find((item) => item.ollamaModel === name) ?? null;
}

export function isSupportedModelName(name: string): boolean {
  return getBackendByModelName(name) !== null;
}

/**
 * Entries for the existing model selector. The selector keeps showing one
 * entry per profile, using the same names as before. Pro AI is listed when
 * either of its backends is installed; the backend is chosen internally.
 */
export function getSelectableModels(installed: readonly string[]): string[] {
  const result: string[] = [];

  for (const profile of ["pro-ai", "gemma"] as const) {
    const entry = BACKENDS.find(
      (item) => item.profile === profile && item.selectorEntry,
    );
    const anyInstalled = BACKENDS.some(
      (item) => item.profile === profile && installed.includes(item.ollamaModel),
    );

    if (entry && anyInstalled) result.push(entry.ollamaModel);
  }

  return result;
}

// ------------------------------------------------------------
// INTENT DETECTION (moved verbatim from App.tsx)
// ------------------------------------------------------------

export function containsTerm(text: string, term: string): boolean {
  const escaped = term.replace(/[.*+?^\${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(
    `(^|[^\\p{L}\\p{N}_])${escaped}(?=$|[^\\p{L}\\p{N}_])`,
    "iu",
  );

  return pattern.test(text);
}

export function detectIntent(
  prompt: string,
  modeSetting: ModeOption,
): ResolvedMode {
  if (modeSetting !== "auto") return modeSetting;

  const q = prompt.toLowerCase().trim();

  const imageRequests = [
    "намалюй",
    "згенеруй картинку",
    "згенеруй зображення",
    "зроби картинку",
    "зроби зображення",
    "draw",
    "generate image",
    "create image",
    "image of",
  ];

  if (imageRequests.some((request) => containsTerm(q, request))) {
    return "image";
  }

  // Translation intent is recognized before formatting, code-term and
  // complexity checks: a technical translation is still a translation.
  if (isTranslationRequest(q)) return "chat";

  const formattingRequests = [
    "markdown",
    "inline code",
    "жирний текст",
    "курсив",
    "таблиця",
    "таблицею",
    "форматування",
  ];

  if (formattingRequests.some((request) => containsTerm(q, request))) {
    return "chat";
  }

  const task = classifyTask(q).type;

  const explainsCode =
    task === "explanation" &&
    /(?:цей|цей\s+самий|наведен\p{L}*)\s+код|код(?:у)?\s+(?:на|мовою)|(?:this|the|following)\s+(?:code|snippet)|code\s+snippet/iu.test(q);

  if (
    task === "coding" ||
    task === "debugging" ||
    explainsCode
  ) {
    return "code";
  }

  if (isComplexTechnicalRequest(q)) {
    return "code";
  }

  return "chat";
}

/**
 * Exact-phrase check. Deliberately unchanged: it still drives Auto-mode
 * intent detection (see detectIntent), so Auto routing is not affected by
 * the broader classifier below. Pro AI backend choice uses
 * isComplexRequest(), which includes this check.
 * The original 11 phrases are unchanged; the English additions below
 * only mirror existing Ukrainian entries.
 */
export function isComplexTechnicalRequest(prompt: string): boolean {
  const complexRequests = [
    "повноцінну систему авторизації",
    "архітектуру проєкту",
    "архітектуру проекту",
    "рефакторинг багатьох файлів",
    "рефакторинг кількох файлів",
    "багатофайлові зміни",
    "multi-file",
    "складний дебагінг",
    "complex debugging",
    "складний алгоритм",
    "оптимальний алгоритм",
    // English counterparts of the Ukrainian entries above.
    "project architecture",
    "system architecture",
    "refactor multiple files",
    "refactor across files",
    "complex algorithm",
    "optimal algorithm",
  ];

  const q = prompt.toLowerCase();

  return complexRequests.some((request) => containsTerm(q, request));
}

const TRANSLATION_TERMS = [
  "переклади",
  "переклад",
  "перекласти",
  "translate",
  "translation",
];

const WORD_START = "(^|[^\\p{L}\\p{N}_])";
const WORD_END = "(?=$|[^\\p{L}\\p{N}_])";

const TRANSLATION_PATTERN = new RegExp(
  `${WORD_START}(?:${TRANSLATION_TERMS.join("|")})${WORD_END}`,
  "iu",
);

/**
 * A request to BUILD software: an authoring verb followed, in the same
 * clause, by a software artifact ("напиши функцію", "write a script").
 * Word stems are matched at a word start, any ending.
 */
const SOFTWARE_AUTHORING_PATTERN = new RegExp(
  `${WORD_START}(?:напиш|написа|створ|реаліз|розроб|write|create|implement|build|develop)` +
    `\\p{L}*[^.!?\\n]{0,60}?` +
    `(?<artifact>функці|function|скрипт|(?<!\\p{L})script(?!\\p{L})|компонент|component|модул|module|` +
    `бібліотек|librar|endpoint|ендпоїнт|(?:api|код|коду|code)${WORD_END})`,
  "iu",
);

const TECHNICAL_TERMS = [
  "api",
  // "rest" alone is an everyday word ("a good rest"); only the technical
  // phrase counts. ("REST API" is also matched by "api" above.)
  "rest api",
  "graphql",
  "http",
  "https",
  "tcp",
  "dns",
  "oauth",
  "jwt",
  "docker",
  "kubernetes",
  "git",
  "regex",
  "apis",
  "algorithm",
  "algorithms",
  "алгоритм",
  "алгоритму",
  "алгоритмом",
  "алгоритмі",
  "алгоритми",
  "алгоритмів",
  "алгоритмами",
  "алгоритмах",
  // база даних: all common case/number forms.
  "база даних",
  "бази даних",
  "базі даних",
  "базу даних",
  "базою даних",
  "баз даних",
  "базам даних",
  "базами даних",
  "базах даних",
  "database",
  "databases",
  "frontend",
  "backend",
  "фреймворк",
  "фреймворку",
  "фреймворком",
  "фреймворки",
  "фреймворків",
  "framework",
  "frameworks",
  "рекурсія",
  "рекурсії",
  "рекурсію",
  "рекурсією",
  "recursion",
];

/**
 * Plain technical explanations (no code terms) that should use Pro AI
 * Instruct in Auto mode. Translation requests are excluded on purpose so
 * they keep their existing Gemma behavior.
 */
export function isTechnicalExplanationRequest(prompt: string): boolean {
  const q = prompt.toLowerCase().trim();

  if (isTranslationRequest(q)) return false;

  const clauses = q.split(/(?<=[.!?;])\s+|\n+/u);

  return clauses.some((clause) => {
    const asksForExplanation =
      /(?:^|[^\p{L}\p{N}_])(?:explain|explanation|describe|how does|how do|how is|what is|what are|why does|why is|поясни\p{L}*|розкажи\p{L}*|як працює|як працюють|що таке|чому|яка різниця|чим\p{L}*.{0,80}?відрізняється)(?=$|[^\p{L}\p{N}_])/u.test(clause);

    return asksForExplanation &&
      TECHNICAL_TERMS.some((term) => containsTerm(clause, term));
  });
}

/**
 * Translation requests are never re-routed by the complexity classifier.
 *
 * A translation word makes the message a translation request unless the
 * user is asking to BUILD software that does translation. That is the case
 * when the software artifact itself ("function", "script", ...) of an
 * authoring phrase comes before the translation word ("Write a function
 * that performs translation"), or is the noun the translation word directly
 * modifies ("Write a translation function"). An artifact mentioned only
 * after the translation word ("Write a paragraph about translation and
 * include a function example") or text quoted after a leading translation
 * instruction ("Translate: write a function") stays a translation request.
 */
export function isTranslationRequest(prompt: string): boolean {
  const q = prompt.toLowerCase().trim();
  const translation = TRANSLATION_PATTERN.exec(q);

  if (!translation) return false;

  const authoring = SOFTWARE_AUTHORING_PATTERN.exec(q);
  const artifact = authoring?.groups?.artifact;

  if (!authoring || !artifact) return true;

  const artifactEnd = authoring.index + authoring[0].length;
  const artifactStart = artifactEnd - artifact.length;
  const translationEnd = translation.index + translation[0].length;

  // Artifact named before the translation word.
  if (artifactEnd <= translation.index) return false;

  // "translation function": the translation word directly modifies the artifact.
  if (artifactStart >= translationEnd && q.slice(translationEnd, artifactStart).trim() === "") {
    return false;
  }

  return true;
}

// ------------------------------------------------------------
// COMPLEXITY CLASSIFIER (decides Pro AI Thinking vs Instruct)
//
// Strategy: a request is complex when it asks for a demanding KIND of work
// (design / analysis / large-scale engineering) AND that work touches
// several system-level aspects (scope, multiplicity, cross-cutting
// concerns). Signals are grouped by meaning; each group counts at most
// once, so repeating words cannot inflate the score and a single
// technical word ("API", "database", "architecture") never decides alone.
// Matching is prefix-based so Ukrainian/English word forms are covered.
// ------------------------------------------------------------

type SignalGroup = "intent" | "scope" | "multiplicity" | "concern";

/** Regex sources; matched at a word start, any word ending. */
const SIGNALS: Record<SignalGroup, readonly string[]> = {
  // The kind of work: design/plan, analyse/compare, large-scale engineering.
  intent: [
    "спроєктуй", "спроектуй", "проєктуй", "проектуй", "проєктув", "проектув",
    "розроби\\p{L}*\\s+архітектур", "сплануй", "плануван", "спланувати",
    "запропонуй\\s+архітектур", "перепроєкт", "перепроект",
    "design", "architect", "plan\\b", "planning", "redesign",
    "проаналізуй", "аналіз", "порівняй", "оціни", "analy[sz]", "compare",
    "evaluate", "assess", "trade-?offs?", "компроміс",
    "рефактор", "refactor", "мігр", "migrat",
    "налагод", "дебаг", "debug", "діагностуй", "diagnos",
    "оптиміз", "optimi[sz]", "профілюван", "profil(e|ing)\\b",
  ],
  // System-level subject matter.
  scope: [
    "архітектур", "architecture", "architectural",
    "систем", "system", "платформ", "platform", "інфраструктур", "infrastructure",
    "застосунк", "додатк", "application",
    "маршрутизатор", "router", "routing", "оркестр", "orchestrat",
    "пайплайн", "pipeline", "конвеєр", "мікросервіс", "microservice",
    "бекенд", "backend", "back-end", "підсистем", "subsystem",
    "inference", "інференс", "multi-?model", "мульти-?модел",
  ],
  // More than one part is involved.
  multiplicity: [
    "кілька", "кількох", "кількома", "декілька", "декількох", "багатьох",
    "multiple", "several", "multi-", "many\\s+(?:files|modules|components|services)",
    "компоненти", "компонентів", "компонентами", "components",
    "модул", "modules?\\b", "сервіс(и|ів|ами)", "services\\b",
    "взаємопов'язан", "interconnected", "взаємод", "interact", "між\\s+компонент",
    "system-wide", "всієї\\s+системи", "по\\s+всій\\s+системі", "end-to-end",
    "across\\s+(?:the\\s+)?(?:system|files|modules|components|services)",
    "залежност", "dependenc",
  ],
  // Cross-cutting engineering concerns.
  concern: [
    "обробк\\p{L}*\\s+помилок", "error[-\\s]handling", "recovery", "відновлен",
    "відмовостійк", "fault[-\\s]toleran", "resilien", "reliab", "надійн",
    "scalab", "масштабован", "concurren", "паралельн",
    "latency", "затримк", "bottleneck", "вузьк\\p{L}*\\s+місц",
    "throughput", "пропускн", "продуктивн", "performance",
    "streaming", "потокова", "потокове", "потоков\\p{L}*\\s+генерац",
    "persistent", "персистентн", "спільн\\p{L}*\\s+пам'ят", "shared\\s+memory",
    "shared\\s+state", "state\\s+management", "consisten", "кешуван", "caching",
  ],
};

/** Marks a deliberately small request; vetoes borderline scores only. */
const SIMPLICITY_MARKERS = [
  "simple", "basic", "small", "short", "brief", "quick", "tiny", "eli5",
  "простими\\s+слов", "простий", "проста", "просте", "просту", "простою",
  "базов", "невелик", "короткий", "коротко", "швидк", "маленьк",
];

/** Distinct signal groups required (an intent group is always required). */
const COMPLEXITY_THRESHOLD = 3;
/** A simplicity marker only vetoes scores below this. */
const SIMPLICITY_VETO_BELOW = 4;

function compileStartPattern(sources: readonly string[]): RegExp {
  return new RegExp(`(^|[^\\p{L}\\p{N}_])(?:${sources.join("|")})`, "iu");
}

const GROUP_PATTERNS: Record<SignalGroup, RegExp> = {
  intent: compileStartPattern(SIGNALS.intent),
  scope: compileStartPattern(SIGNALS.scope),
  multiplicity: compileStartPattern(SIGNALS.multiplicity),
  concern: compileStartPattern(SIGNALS.concern),
};
const SIMPLICITY_PATTERN = compileStartPattern(SIMPLICITY_MARKERS);

function normalizeForClassifier(prompt: string): string {
  // Typographic apostrophes (U+2019, U+02BC, U+2018) -> ASCII, so the same word matches in either form.
  return prompt.toLowerCase().replace(/[\u2019\u02bc\u2018`]/g, "'");
}

export type ComplexityAssessment = {
  complex: boolean;
  /** Which signal groups matched (each counts once). */
  groups: SignalGroup[];
  score: number;
  reason: "exact-phrase" | "signals" | "translation" | "simple-marker" | "below-threshold";
};

/**
 * Explains why a request was (not) treated as complex. Pure; used by
 * isComplexRequest() and by tests.
 */
export function assessComplexity(prompt: string): ComplexityAssessment {
  // Translation has priority over everything below, including the legacy
  // exact phrases: translating text about architecture is not engineering.
  if (isTranslationRequest(prompt)) {
    return { complex: false, groups: [], score: 0, reason: "translation" };
  }

  // Existing exact phrases always count (outside translation requests).
  if (isComplexTechnicalRequest(prompt)) {
    return { complex: true, groups: [], score: COMPLEXITY_THRESHOLD, reason: "exact-phrase" };
  }

  const q = normalizeForClassifier(prompt);

  const groups = (Object.keys(GROUP_PATTERNS) as SignalGroup[]).filter((group) =>
    GROUP_PATTERNS[group].test(q),
  );
  const score = groups.length;

  if (!groups.includes("intent") || score < COMPLEXITY_THRESHOLD) {
    return { complex: false, groups, score, reason: "below-threshold" };
  }

  if (score < SIMPLICITY_VETO_BELOW && SIMPLICITY_PATTERN.test(q)) {
    return { complex: false, groups, score, reason: "simple-marker" };
  }

  return { complex: true, groups, score, reason: "signals" };
}

/** Pro AI complexity decision: exact phrases OR the signal-based classifier. */
export function isComplexRequest(prompt: string): boolean {
  return assessComplexity(prompt).complex;
}

// ------------------------------------------------------------

// ------------------------------------------------------------
// ROUTER 2.0 — TASK CLASSIFICATION
// ------------------------------------------------------------

export type TaskType =
  | "conversation"
  | "explanation"
  | "coding"
  | "analysis"
  | "planning"
  | "translation"
  | "debugging"
  | "comparison"
  | "writing";

export type TaskClassification = {
  type: TaskType;
};

export function classifyTask(prompt: string): TaskClassification {
  const q = prompt.toLowerCase().trim();

  if (isTranslationRequest(q)) return { type: "translation" };

  if (
    /(?:виправ|налагод|знайди помилк|debug|fix the bug|troubleshoot)/iu.test(q) ||
    /\bfix\b.{0,60}\b(?:css|html|javascript|typescript|react|python|sql|code|код)\b/iu.test(q)
  ) {
    return { type: "debugging" };
  }

  if (
    /(?:порівняй|порівняти|compare|contrast|відмінност)/iu.test(q)
  ) {
    return { type: "comparison" };
  }

  if (
    /(?:напиши|створи|реалізуй|розроби|write|create|implement|build|develop)/iu.test(q) &&
    /(?:функці|function|код|code|скрипт|script|компонент|component|модул|module|endpoint|ендпоїнт|api)/iu.test(q)
  ) {
    return { type: "coding" };
  }

  if (
    /(?:оптимізуй|оптимізувати|перепиши|рефактор|optimize|optimise|refactor)/iu.test(q) &&
    /(?:sql|javascript|typescript|react|python|html|css|код|code|query|запит|function|функці|component|компонент|api|endpoint)/iu.test(q)
  ) {
    return { type: "coding" };
  }

  if (
    /(?:напиши|створи|склади|сформулюй|write|draft|compose)/iu.test(q) &&
    /(?:лист|повідомлення|статт|опис|есе|cover letter|email|message|article|post)/iu.test(q)
  ) {
    return { type: "writing" };
  }

  if (
    /(?:спроєктуй|спроектуй|проєктуй|проектуй|сплануй|спланувати|design|architect|plan the system)/iu.test(q) &&
    /(?:api|архітектур|architecture|system|систем|database|баз[а-яіїєґ]* даних|backend|бекенд|інфраструктур|infrastructure|pipeline|пайплайн|компонент|component|модул|module)/iu.test(q)
  ) {
    return { type: "planning" };
  }

  if (
    /(?:проаналізуй|аналіз|досліди|оцінити|evaluate|analy[sz]e|investigate)/iu.test(q)
  ) {
    return { type: "analysis" };
  }

  if (
    /(?:поясни|пояснити|як працює|що таке|розкажи про|explain|how does|what is)/iu.test(q) &&
    (
      isTechnicalExplanationRequest(q) ||
      /(?:javascript|typescript|react|python|html|css|sql|api|docker|websocket|(?:програмн|системн|мікросервісн|розподілен)[а-яіїєґ]*\s+архітектур|архітектур[а-яіїєґ]*\s+(?:мікросервіс|розподілен|системн|програмн)|(?:software|system|microservice|distributed)\s+architecture)/iu.test(q)
    )
  ) {
    return { type: "explanation" };
  }

  return { type: "conversation" };
}

// ROUTING
// ------------------------------------------------------------

export type RoutingErrorCode =
  | "backend-unavailable"
  | "no-supported-models";

export type RouteResult =
  | {
      ok: true;
      backend: BackendConfig;
      /**
       * What the user's Thinking toggle means for this request. Only Gemma
       * consumes it (Pro AI backends decide `think` themselves); it keeps the
       * existing Gemma rule: the toggle applies in Code mode only.
       */
      userRequestedThinking: boolean;
      /** Final `think` value for /api/chat, resolved for the chosen backend. */
      think: boolean;
    }
  | {
      ok: false;
      code: RoutingErrorCode;
      backend: ExecutionBackend;
      message: string;
    };

/**
 * Existing Gemma thinking rule (moved from App.tsx): the toggle only
 * takes effect in Code mode.
 */
export function resolveUserRequestedThinking(
  resolvedMode: ResolvedMode,
  allowThinking: boolean,
): boolean {
  return resolvedMode === "code" && allowThinking;
}

/**
 * Pro AI backend. The Thinking toggle overrides complexity:
 * - allowThinking ON  -> always Pro AI Thinking.
 * - allowThinking OFF -> by task complexity (complex -> Thinking, else Instruct).
 * Never falls back to the other backend when a model is missing.
 */
export function chooseProBackend(
  prompt: string,
  allowThinking: boolean,
): ExecutionBackend {
  if (allowThinking) return "pro-thinking";

  return isComplexRequest(prompt) ? "pro-thinking" : "pro-instruct";
}

/**
 * Step 1: which profile handles the request. The Thinking toggle is not an
 * input here, so it can never turn a Gemma request into a Pro AI request.
 * Manual model selection always decides the profile.
 */
export function wantedProfile(
  resolvedMode: ResolvedMode,
  selectedModel: string,
  prompt: string,
  isAutoMode: boolean,
): ModelProfile {
  const manual = selectedModel !== "auto" ? getBackendByModelName(selectedModel) : null;

  if (manual) return manual.profile;

  // Explicit Code mode keeps its Pro AI routing.
  if (resolvedMode === "code") return "pro-ai";

  // Auto mode only: genuinely complex requests use Pro AI even without code or
  // technical-explanation keywords. Translations are never complex
  // (see assessComplexity), so they stay on Gemma.
  if (isAutoMode && isComplexRequest(prompt)) return "pro-ai";

  // Auto mode only: plain technical explanations use Pro AI.
  if (isAutoMode && isTechnicalExplanationRequest(prompt)) return "pro-ai";

  return "gemma";
}

/**
 * Step 2: execution backend. Pro AI applies the Thinking override;
 * Gemma is unaffected by it.
 */
export function wantedBackend(
  resolvedMode: ResolvedMode,
  selectedModel: string,
  prompt: string,
  isAutoMode: boolean,
  allowThinking: boolean,
): ExecutionBackend {
  const profile = wantedProfile(resolvedMode, selectedModel, prompt, isAutoMode);

  return profile === "pro-ai" ? chooseProBackend(prompt, allowThinking) : "gemma";
}

/**
 * Router 2.0: task-aware backend selection.
 * Kept separate from wantedBackend() until V2 behavior is verified.
 */
export function wantedBackendV2(
  resolvedMode: ResolvedMode,
  selectedModel: string,
  prompt: string,
  isAutoMode: boolean,
  allowThinking: boolean,
): ExecutionBackend {
  const manual = selectedModel !== "auto"
    ? getBackendByModelName(selectedModel)
    : null;

  // Manual model selection always keeps priority.
  if (manual?.profile === "gemma") return "gemma";

  if (manual?.profile === "pro-ai") {
    return chooseProBackend(prompt, allowThinking);
  }

  // Explicit Code mode keeps its existing Pro AI behavior.
  // Auto-detected Code must not be treated as an explicit selection.
  if (!isAutoMode && resolvedMode === "code") {
    return chooseProBackend(prompt, allowThinking);
  }

  // Explicit non-auto modes preserve their existing behavior.
  if (!isAutoMode) return "gemma";

  // Preserve established Auto-mode technical routing.
  if (
    isComplexRequest(prompt) ||
    isTechnicalExplanationRequest(prompt)
  ) {
    return chooseProBackend(prompt, allowThinking);
  }

  // Router 2.0 task-aware automatic routing.
  const task = classifyTask(prompt).type;

  const proTasks: TaskType[] = [
    "explanation",
    "coding",
    "debugging",
    "analysis",
    "planning",
    "comparison",
  ];

  if (!proTasks.includes(task)) return "gemma";

  return chooseProBackend(prompt, allowThinking);
}

export function unavailableMessage(backend: ExecutionBackend): string {
  const config = getBackendConfig(backend);

  return (
    `Модель для «${config.label}» недоступна в Ollama: ${config.ollamaModel}. ` +
    `Sonne не підміняє її іншою моделлю автоматично.`
  );
}

export function routeRequest(
  resolvedMode: ResolvedMode,
  installedModels: readonly string[],
  selectedModel: string,
  prompt: string,
  isAutoMode: boolean,
  allowThinking: boolean,
): RouteResult {
  const backend = wantedBackendV2(
    resolvedMode,
    selectedModel,
    prompt,
    isAutoMode,
    allowThinking,
  );
  const config = getBackendConfig(backend);

  if (!installedModels.includes(config.ollamaModel)) {
    return {
      ok: false,
      code: installedModels.length === 0 ? "no-supported-models" : "backend-unavailable",
      backend,
      message: unavailableMessage(backend),
    };
  }

  const userRequestedThinking = resolveUserRequestedThinking(resolvedMode, allowThinking);

  return {
    ok: true,
    backend: config,
    userRequestedThinking,
    think: resolveThink(config, userRequestedThinking),
  };
}

// ------------------------------------------------------------
// REQUEST ASSEMBLY (shared by both Pro AI backends)
// ------------------------------------------------------------

const CHAT_TASK_INSTRUCTIONS =
  "Виконуй звичайне спілкування та інформаційні завдання. Відповідай природно, прямо й без зайвої багатослівності.";

const CODE_TASK_INSTRUCTIONS =
  "Виконуй завдання відповідно до запиту. Для програмування використовуй саме потрібну мову, технологію та формат. Не перетворюй кожне завдання на HTML або UI/UX-дизайн.";

const RESPONSE_INSTRUCTIONS =
  "Відповідай мовою останнього повідомлення користувача. Якщо користувач прямо просить переклад, текст або відповідь іншою мовою, виконай саме цю мовну вимогу. Зберігай звичні технічні терміни, коли це покращує зрозумілість. Використовуй стандартний Markdown: не екрануй звичайні розділювачі Markdown у звичайному тексті та не використовуй HTML-сутності для звичайного тексту. Зберігай точний вміст inline-коду й fenced code blocks. Не додавай до відповіді службові елементи інтерфейсу або їхні назви.";

/** Same text as before; depends only on the profile instructions and mode. */
export function buildSystemPrompt(
  runtimeInstructions: string,
  resolvedMode: ResolvedMode,
): string {
  const taskInstructions =
    resolvedMode === "chat" ? CHAT_TASK_INSTRUCTIONS : CODE_TASK_INSTRUCTIONS;

  return (
    `${runtimeInstructions}\n\n` +
    `# TASK INSTRUCTIONS\n${taskInstructions}\n\n` +
    `# RESPONSE INSTRUCTIONS\n${RESPONSE_INSTRUCTIONS}`
  );
}

export function resolveThink(
  config: BackendConfig,
  userRequestedThinking: boolean,
): boolean {
  if (config.thinking === "on") return true;
  if (config.thinking === "off") return false;
  return userRequestedThinking;
}

export type ChatMessageBody = {
  role: "system" | "user" | "assistant";
  content: string;
  images?: string[] | Uint8Array[];
};

export type OllamaChatRequestBody = {
  model: string;
  stream: true;
  think: boolean;
  messages: ChatMessageBody[];
  options?: Partial<RuntimeOptions>;
};

export function buildChatRequestBody(
  config: BackendConfig,
  messages: ChatMessageBody[],
  userRequestedThinking: boolean,
  outputTokenLimit?: number,
): OllamaChatRequestBody {
  const body: OllamaChatRequestBody = {
    model: config.ollamaModel,
    stream: true,
    think: resolveThink(config, userRequestedThinking),
    messages,
  };

  if (config.options) body.options = { ...config.options };

  if (outputTokenLimit !== undefined) {
    if (!Number.isFinite(outputTokenLimit) || outputTokenLimit < 1) {
      throw new Error("Output token limit must be a positive finite number.");
    }
    body.options = {
      ...body.options,
      num_predict: Math.floor(outputTokenLimit),
    };
  }

  return body;
}
