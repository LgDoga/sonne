import test from "node:test";
import assert from "node:assert/strict";
import {
  BACKENDS,
  getBackendConfig,
  getBackendByModelName,
  getSelectableModels,
  detectIntent,
  routeRequest,
  buildSystemPrompt,
  buildChatRequestBody,
  assessComplexity,
  isComplexRequest,
  isTranslationRequest,
  isTechnicalExplanationRequest,
} from "../src/modelRouting.ts";

const THINKING = "sonne-pro-ai:latest";
const INSTRUCT = "qwen3:30b-a3b-instruct-2507-q4_K_M";
const GEMMA = "sonne-gemma:latest";
const ALL = [THINKING, INSTRUCT, GEMMA];

// Mirrors handleSend: mode resolution, then routing.
function route(
  prompt,
  { modeSetting = "auto", selected = "auto", installed = ALL, thinking = false } = {},
) {
  const mode = detectIntent(prompt, modeSetting);
  return {
    mode,
    result: routeRequest(mode, installed, selected, prompt, modeSetting === "auto", thinking),
  };
}
const backendOf = (r) => (r.result.ok ? r.result.backend.backend : `ERR:${r.result.backend}`);

test("config: three exact models, two profiles", () => {
  assert.deepEqual(BACKENDS.map((b) => b.ollamaModel), ALL);
  assert.deepEqual([...new Set(BACKENDS.map((b) => b.profile))].sort(), ["gemma", "pro-ai"]);
  assert.equal(getBackendByModelName(INSTRUCT.toLowerCase()), null); // no fuzzy matching
  assert.equal(getBackendByModelName("sonne-pro-ai"), null);
});

test("1. complex architecture task -> Pro AI Thinking", () => {
  assert.equal(backendOf(route("Спроєктуй архітектуру проєкту для CRM на React")), "pro-thinking");
  assert.equal(backendOf(route("Design the system architecture for a chat app")), "pro-thinking");
});

test("2. multi-file refactoring -> Pro AI Thinking", () => {
  assert.equal(backendOf(route("Зроби рефакторинг кількох файлів у проєкті")), "pro-thinking");
  assert.equal(backendOf(route("I need a multi-file change in my TypeScript app")), "pro-thinking");
});

test("3. simple JavaScript function -> Pro AI Instruct", () => {
  const r = route("Напиши функцію на JavaScript, яка перевертає рядок");
  assert.equal(r.mode, "code");
  assert.equal(backendOf(r), "pro-instruct");
});

test("4. standard React component -> Pro AI Instruct", () => {
  assert.equal(backendOf(route("Створи компонент кнопки на React")), "pro-instruct");
});

test("5. simple technical explanation -> Pro AI Instruct", () => {
  const r = route("Поясни простими словами, що таке REST API");
  assert.equal(r.mode, "chat");
  assert.equal(backendOf(r), "pro-instruct");
});

test("6. translation -> Gemma (also when it mentions a technical term)", () => {
  assert.equal(backendOf(route("Переклади на англійську: доброго ранку")), "gemma");
  assert.equal(backendOf(route("Переклади на англійську: цей API працює швидко")), "gemma");
});

test("7. everyday conversation -> Gemma", () => {
  assert.equal(backendOf(route("Привіт! Як справи?")), "gemma");
  assert.equal(backendOf(route("Порадь ідею для вечері")), "gemma");
});

test("8. manual Pro AI selection -> Pro AI profile (backend internal)", () => {
  const simple = route("Привіт", { selected: THINKING });
  const complex = route("Зроби рефакторинг кількох файлів", { selected: THINKING });
  assert.equal(simple.result.backend.profile, "pro-ai");
  assert.equal(backendOf(simple), "pro-instruct");
  assert.equal(backendOf(complex), "pro-thinking");
  // manual Gemma stays Gemma, even for complex code
  assert.equal(backendOf(route("Зроби рефакторинг кількох файлів", { selected: GEMMA })), "gemma");
});

test("9. explicit Code mode -> Pro AI profile, backend by complexity", () => {
  const simple = route("Привіт", { modeSetting: "code" });
  assert.equal(simple.result.backend.profile, "pro-ai");
  assert.equal(backendOf(simple), "pro-instruct");
  assert.equal(backendOf(route("архітектуру проєкту", { modeSetting: "code" })), "pro-thinking");
});

test("explicit Chat mode keeps existing Gemma behavior", () => {
  assert.equal(backendOf(route("Поясни, що таке REST API", { modeSetting: "chat" })), "gemma");
});

test("10-12. missing model -> clear error, no silent fallback", () => {
  const noThinking = route("архітектуру проєкту", { installed: [INSTRUCT, GEMMA] });
  assert.equal(noThinking.result.ok, false);
  assert.equal(noThinking.result.backend, "pro-thinking");
  assert.match(noThinking.result.message, /sonne-pro-ai:latest/);

  const noInstruct = route("Напиши функцію на JavaScript", { installed: [THINKING, GEMMA] });
  assert.equal(noInstruct.result.ok, false);
  assert.equal(noInstruct.result.backend, "pro-instruct");
  assert.match(noInstruct.result.message, /qwen3:30b-a3b-instruct-2507-q4_K_M/);

  const noGemma = route("Привіт!", { installed: [THINKING, INSTRUCT] });
  assert.equal(noGemma.result.ok, false);
  assert.equal(noGemma.result.backend, "gemma");

  const none = route("Привіт!", { installed: [] });
  assert.equal(none.result.ok, false);
  assert.equal(none.result.code, "no-supported-models");
});

test("13. Thinking backend sends think:true and no option overrides", () => {
  const body = buildChatRequestBody(getBackendConfig("pro-thinking"), [], false);
  assert.equal(body.model, THINKING);
  assert.equal(body.think, true);
  assert.equal("options" in body, false);
});

test("14. Instruct backend sends think:false (even if the UI toggle is on)", () => {
  for (const toggle of [false, true]) {
    const body = buildChatRequestBody(getBackendConfig("pro-instruct"), [], toggle);
    assert.equal(body.model, INSTRUCT);
    assert.equal(body.think, false);
    assert.equal(body.options.num_ctx, 12288);
  }
});

test("Gemma keeps the existing thinking toggle behavior and no options", () => {
  const g = getBackendConfig("gemma");
  assert.equal(buildChatRequestBody(g, [], false).think, false);
  assert.equal(buildChatRequestBody(g, [], true).think, true);
  assert.equal("options" in buildChatRequestBody(g, [], false), false);
});

test("15. both Pro AI backends get identical system instructions", () => {
  const runtime = "RUNTIME-INSTRUCTIONS-FOR-pro-ai";
  const history = [{ role: "user", content: "a" }, { role: "assistant", content: "b" }];
  const prompt = "Напиши функцію на JavaScript";
  const mk = (cfg) =>
    buildChatRequestBody(
      cfg,
      [{ role: "system", content: buildSystemPrompt(runtime, "code") }, ...history, { role: "user", content: prompt }],
      false,
    );
  const a = mk(getBackendConfig("pro-thinking"));
  const b = mk(getBackendConfig("pro-instruct"));
  assert.deepEqual(a.messages, b.messages);
  // Both backends resolve to the same instruction profile (what invoke() receives).
  assert.equal(getBackendConfig("pro-thinking").profile, getBackendConfig("pro-instruct").profile);
});

test("selector: unchanged entries; Instruct never listed", () => {
  assert.deepEqual(getSelectableModels(ALL), [THINKING, GEMMA]);
  assert.deepEqual(getSelectableModels([INSTRUCT, GEMMA]), [THINKING, GEMMA]); // Pro AI still selectable
  assert.deepEqual(getSelectableModels([GEMMA]), [GEMMA]);
});

test("image requests do not need a model", () => {
  assert.equal(detectIntent("намалюй кота", "auto"), "image");
});

// ------------------------------------------------------------
// Thinking toggle overrides Pro AI backend selection
// ------------------------------------------------------------

const SIMPLE = "Привіт";
const COMPLEX = "Зроби рефакторинг кількох файлів";
const TECH_EXPLANATION = "Поясни простими словами, що таке REST API";
const CODE_SIMPLE = "Напиши функцію на JavaScript, яка перевертає рядок";
const EVERYDAY = "Привіт! Як справи?";
const thinkOf = (r) => (r.result.ok ? r.result.think : null);

test("T1. manual Pro AI + Thinking ON + simple -> Pro Thinking", () => {
  const r = route(SIMPLE, { selected: THINKING, thinking: true });
  assert.equal(backendOf(r), "pro-thinking");
  assert.equal(r.result.backend.ollamaModel, THINKING);
  assert.equal(thinkOf(r), true);
});

test("T2. manual Pro AI + Thinking ON + complex -> Pro Thinking", () => {
  const r = route(COMPLEX, { selected: THINKING, thinking: true });
  assert.equal(backendOf(r), "pro-thinking");
  assert.equal(thinkOf(r), true);
});

test("T3. manual Pro AI + Thinking OFF + simple -> Pro Instruct", () => {
  const r = route(SIMPLE, { selected: THINKING, thinking: false });
  assert.equal(backendOf(r), "pro-instruct");
  assert.equal(thinkOf(r), false);
});

test("T4. manual Pro AI + Thinking OFF + complex -> Pro Thinking", () => {
  const r = route(COMPLEX, { selected: THINKING, thinking: false });
  assert.equal(backendOf(r), "pro-thinking");
  assert.equal(thinkOf(r), true);
});

test("T5. explicit Code + Thinking ON -> Pro Thinking (simple and complex)", () => {
  assert.equal(backendOf(route(SIMPLE, { modeSetting: "code", thinking: true })), "pro-thinking");
  assert.equal(backendOf(route(CODE_SIMPLE, { modeSetting: "code", thinking: true })), "pro-thinking");
  assert.equal(backendOf(route(COMPLEX, { modeSetting: "code", thinking: true })), "pro-thinking");
});

test("T6. explicit Code + Thinking OFF + simple -> Pro Instruct (complex -> Thinking)", () => {
  const simple = route(CODE_SIMPLE, { modeSetting: "code", thinking: false });
  assert.equal(backendOf(simple), "pro-instruct");
  assert.equal(thinkOf(simple), false);
  assert.equal(backendOf(route(COMPLEX, { modeSetting: "code", thinking: false })), "pro-thinking");
});

test("T7. Auto technical explanation + Thinking ON -> Pro Thinking", () => {
  const r = route(TECH_EXPLANATION, { thinking: true });
  assert.equal(r.mode, "chat");
  assert.equal(backendOf(r), "pro-thinking");
  assert.equal(thinkOf(r), true);
});

test("T8. Auto technical explanation + Thinking OFF -> Pro Instruct", () => {
  const r = route(TECH_EXPLANATION, { thinking: false });
  assert.equal(backendOf(r), "pro-instruct");
  assert.equal(thinkOf(r), false);
});

test("T9. Auto everyday conversation + Thinking ON -> Gemma (never Pro AI)", () => {
  for (const prompt of [EVERYDAY, "Порадь ідею для вечері", "Переклади на англійську: цей API працює швидко"]) {
    const r = route(prompt, { thinking: true });
    assert.equal(backendOf(r), "gemma", prompt);
    assert.equal(r.result.backend.profile, "gemma", prompt);
  }
  // Existing Gemma rule: the toggle only affects Gemma in Code mode, so chat stays think:false.
  assert.equal(thinkOf(route(EVERYDAY, { thinking: true })), false);
});

test("T10. manual Gemma + Thinking ON/OFF -> Gemma with the corresponding think value", () => {
  // Gemma's existing rule: toggle applies in Code mode.
  const on = route(CODE_SIMPLE, { selected: GEMMA, thinking: true });
  const off = route(CODE_SIMPLE, { selected: GEMMA, thinking: false });
  assert.equal(on.mode, "code");
  assert.equal(backendOf(on), "gemma");
  assert.equal(backendOf(off), "gemma");
  assert.equal(thinkOf(on), true);
  assert.equal(thinkOf(off), false);
  assert.equal(buildChatRequestBody(on.result.backend, [], on.result.userRequestedThinking).think, true);
  assert.equal(buildChatRequestBody(off.result.backend, [], off.result.userRequestedThinking).think, false);

  // Gemma stays Gemma for complex prompts and for the Code-mode setting too.
  for (const thinking of [true, false]) {
    assert.equal(backendOf(route(COMPLEX, { selected: GEMMA, thinking })), "gemma");
    assert.equal(backendOf(route(COMPLEX, { selected: GEMMA, modeSetting: "code", thinking })), "gemma");
  }
  // Existing behavior preserved: outside Code mode the toggle does not change Gemma.
  assert.equal(thinkOf(route(SIMPLE, { selected: GEMMA, thinking: true })), false);
});

test("T11. explicit Chat mode: manual Pro AI gets the override; otherwise Gemma", () => {
  assert.equal(backendOf(route(SIMPLE, { modeSetting: "chat", selected: THINKING, thinking: true })), "pro-thinking");
  assert.equal(backendOf(route(SIMPLE, { modeSetting: "chat", selected: THINKING, thinking: false })), "pro-instruct");
  assert.equal(backendOf(route(TECH_EXPLANATION, { modeSetting: "chat", thinking: true })), "gemma");
  assert.equal(backendOf(route(TECH_EXPLANATION, { modeSetting: "chat", thinking: false })), "gemma");
});

test("T12. Thinking toggle never changes the profile", () => {
  const prompts = [SIMPLE, COMPLEX, TECH_EXPLANATION, CODE_SIMPLE, EVERYDAY];
  const setups = [{}, { modeSetting: "code" }, { modeSetting: "chat" }, { selected: THINKING }, { selected: GEMMA }];
  for (const prompt of prompts) {
    for (const setup of setups) {
      const label = `${prompt} / ${JSON.stringify(setup)}`;
      const on = route(prompt, { ...setup, thinking: true }).result;
      const off = route(prompt, { ...setup, thinking: false }).result;
      // All models are installed here: both routes must succeed, otherwise the
      // profile comparison below could pass vacuously (undefined === undefined).
      assert.equal(on.ok, true, `ON must route successfully: ${label}`);
      assert.equal(off.ok, true, `OFF must route successfully: ${label}`);
      assert.equal(on.backend.profile, off.backend.profile, label);
    }
  }
});

test("T13. resolved think always matches the chosen backend", () => {
  for (const prompt of [SIMPLE, COMPLEX, TECH_EXPLANATION, CODE_SIMPLE]) {
    for (const thinking of [true, false]) {
      const r = route(prompt, { selected: THINKING, thinking }).result;
      assert.equal(r.think, r.backend.backend === "pro-thinking");
      const body = buildChatRequestBody(r.backend, [], r.userRequestedThinking);
      assert.equal(body.think, r.think);
    }
  }
});

test("T14. missing models with the Thinking toggle: explicit error, no silent fallback", () => {
  // Thinking ON forces pro-thinking; Instruct installed must NOT be substituted.
  const onNoThinking = route(SIMPLE, { selected: THINKING, thinking: true, installed: [INSTRUCT, GEMMA] });
  assert.equal(onNoThinking.result.ok, false);
  assert.equal(onNoThinking.result.backend, "pro-thinking");
  assert.match(onNoThinking.result.message, /sonne-pro-ai:latest/);

  // Thinking OFF + simple -> Instruct; Thinking installed must NOT be substituted.
  const offNoInstruct = route(SIMPLE, { selected: THINKING, thinking: false, installed: [THINKING, GEMMA] });
  assert.equal(offNoInstruct.result.ok, false);
  assert.equal(offNoInstruct.result.backend, "pro-instruct");
  assert.match(offNoInstruct.result.message, /qwen3:30b-a3b-instruct-2507-q4_K_M/);

  // Same models installed, Thinking ON with Instruct missing still works (override picks Thinking).
  assert.equal(backendOf(route(SIMPLE, { selected: THINKING, thinking: true, installed: [THINKING, GEMMA] })), "pro-thinking");

  // Gemma missing, Thinking ON, everyday chat -> Gemma error (not rerouted to Pro AI).
  const noGemma = route(EVERYDAY, { thinking: true, installed: [THINKING, INSTRUCT] });
  assert.equal(noGemma.result.ok, false);
  assert.equal(noGemma.result.backend, "gemma");

  const none = route(SIMPLE, { thinking: true, installed: [] });
  assert.equal(none.result.ok, false);
  assert.equal(none.result.code, "no-supported-models");
});

// ------------------------------------------------------------
// Complexity classification (Pro AI Thinking vs Instruct)
// All backend assertions go through routeRequest() via route().
// ------------------------------------------------------------

const ARCH_REQUEST =
  "Спроєктуй архітектуру локального AI-застосунку з підтримкою кількох моделей, автоматичним маршрутизатором, спільною пам'яттю та потоковою генерацією. Опиши компоненти, їхню взаємодію та обробку помилок.";

const COMPLEX_CASES = [
  ["exact local AI architecture request (uk)", ARCH_REQUEST],
  ["multi-model AI application (en)", "Design a multi-model AI application with a router, shared memory and streaming generation. Describe the components and how they interact."],
  ["multi-model AI application (uk)", "Спроєктуй застосунок з кількома AI-моделями, маршрутизатором і потоковою генерацією"],
  ["shared persistent memory (en)", "Design a shared persistent memory architecture for several AI agents."],
  ["shared persistent memory (uk)", "Спроєктуй спільну персистентну пам\u2019ять для кількох AI-агентів, які працюють через різні сервіси"],
  ["streaming inference pipeline with error recovery (en)", "Design a streaming inference pipeline with error recovery and retries across several workers."],
  ["streaming inference pipeline with error recovery (uk)", "Спроєктуй потоковий пайплайн інференсу з обробкою помилок та відновленням після збоїв"],
  ["multi-component backend architecture (en)", "Plan a multi-component backend architecture with an API gateway, a queue and several services."],
  ["multi-component backend architecture (uk)", "Сплануй багатокомпонентну бекенд-архітектуру: шлюз, черга та кілька сервісів"],
  ["system-wide performance bottlenecks (en)", "Analyze system-wide performance bottlenecks in our application."],
  ["system-wide performance bottlenecks (uk)", "Проаналізуй вузькі місця продуктивності всієї системи"],
  ["multi-file architectural refactor (en)", "Plan a refactor of the architecture across several files and modules."],
  ["multi-file architectural refactor (uk)", "Сплануй рефакторинг архітектури, який зачіпає кілька файлів і модулів"],
  ["debugging interconnected modules (en)", "Help me debug a failure that appears when several interconnected modules interact: the router, the streaming layer and the cache."],
  ["debugging interconnected modules (uk)", "Допоможи налагодити збій, що виникає під час взаємодії кількох взаємопов'язаних модулів: маршрутизатора, потокової генерації та кешу."],
  ["architecture trade-offs comparison (en)", "Compare two architectural approaches for an event-driven system and evaluate the trade-offs for scalability and reliability."],
];

const SIMPLE_CASES = [
  ["what an API is (en)", "Explain what an API is."],
  ["what an API is (uk)", "Поясни, що таке API"],
  ["basic JavaScript function (en)", "Write a simple JavaScript function that reverses a string."],
  ["basic JavaScript function (uk)", "Напиши просту функцію на JavaScript, яка перевертає рядок"],
  ["simple React component (en)", "Create a basic React component."],
  ["simple React component (uk)", "Створи простий компонент кнопки на React"],
  ["database concept (en)", "Explain what a database index is."],
  ["database concept (uk)", "Поясни, що таке індекс у базі даних"],
  ["small CSS issue (en)", "Fix this small CSS issue: the button is not centered."],
  ["small CSS issue (uk)", "Виправ невелику проблему з CSS: кнопка не по центру"],
  ["frontend vs backend (en)", "What is the difference between frontend and backend?"],
  ["frontend vs backend (uk)", "Яка різниця між frontend і backend?"],
  ["short Docker explanation (en)", "Give me a short explanation of Docker."],
  ["short Docker explanation (uk)", "Коротко поясни, що таке Docker"],
  ["small design task", "Design a REST API for a todo list"],
  ["optimize one query", "Optimize this SQL query"],
];

const TRANSLATION_CASES = [
  "Translate this technical paragraph: the architecture of a multi-model system with a router, shared memory and streaming error recovery.",
  "Переклади цей технічний абзац: архітектура системи з кількома моделями, маршрутизатором, спільною пам'яттю та обробкою помилок",
];

test("C1. exact local AI architecture request: manual Pro AI + Thinking OFF -> Pro Thinking", () => {
  const r = route(ARCH_REQUEST, { selected: THINKING, thinking: false });
  assert.equal(backendOf(r), "pro-thinking");
  assert.equal(r.result.backend.ollamaModel, THINKING);
  assert.equal(thinkOf(r), true);
  // Not caused by the Thinking toggle: classifier alone recognizes it.
  assert.equal(assessComplexity(ARCH_REQUEST).reason, "signals");
});

test("C2. complex requests: manual Pro AI + Thinking OFF -> pro-thinking (uk + en)", () => {
  for (const [name, prompt] of COMPLEX_CASES) {
    const r = route(prompt, { selected: THINKING, thinking: false });
    assert.equal(backendOf(r), "pro-thinking", name);
    assert.equal(thinkOf(r), true, name);
  }
});

test("C3. complex requests: explicit Code mode + Thinking OFF -> pro-thinking", () => {
  for (const [name, prompt] of COMPLEX_CASES) {
    assert.equal(backendOf(route(prompt, { modeSetting: "code", thinking: false })), "pro-thinking", name);
  }
});

test("C4. simple requests: manual Pro AI + Thinking OFF -> pro-instruct (uk + en)", () => {
  for (const [name, prompt] of SIMPLE_CASES) {
    const r = route(prompt, { selected: THINKING, thinking: false });
    assert.equal(backendOf(r), "pro-instruct", name);
    assert.equal(thinkOf(r), false, name);
  }
});

test("C5. simple requests: manual Pro AI + Thinking ON -> pro-thinking (toggle override)", () => {
  for (const [name, prompt] of SIMPLE_CASES) {
    assert.equal(backendOf(route(prompt, { selected: THINKING, thinking: true })), "pro-thinking", name);
  }
});

test("C6. manual Gemma stays Gemma regardless of complexity or toggle", () => {
  for (const [name, prompt] of [...COMPLEX_CASES, ...SIMPLE_CASES]) {
    for (const thinking of [true, false]) {
      for (const modeSetting of ["auto", "chat", "code"]) {
        const r = route(prompt, { selected: GEMMA, thinking, modeSetting });
        assert.equal(backendOf(r), "gemma", `${name} / ${modeSetting} / ${thinking}`);
      }
    }
  }
});

test("C7. translation of a technical paragraph is not complex (Pro AI OFF -> Instruct, Auto -> Gemma)", () => {
  for (const prompt of TRANSLATION_CASES) {
    assert.equal(isComplexRequest(prompt), false);
    assert.equal(assessComplexity(prompt).reason, "translation");
    assert.equal(backendOf(route(prompt, { selected: THINKING, thinking: false })), "pro-instruct");
    assert.equal(backendOf(route(prompt, { thinking: false })), "gemma"); // Auto: unchanged
    assert.equal(backendOf(route(prompt, { thinking: true })), "gemma"); // toggle never moves Auto Gemma to Pro AI
  }
});

test("C8. existing exact phrases still count outside translation requests", () => {
  assert.equal(assessComplexity("архітектуру проєкту").reason, "exact-phrase");
  assert.equal(assessComplexity("Design the system architecture for a chat app").reason, "exact-phrase");
  assert.equal(backendOf(route("архітектуру проєкту", { selected: THINKING, thinking: false })), "pro-thinking");
  // Inside a translation request the same phrase no longer forces Thinking (see E-tests).
  assert.equal(assessComplexity("Переклади: архітектуру проєкту").reason, "translation");
  assert.equal(backendOf(route("Переклади: архітектуру проєкту", { selected: THINKING, thinking: false })), "pro-instruct");
});

test("C9. a single generic technical word never makes a request complex", () => {
  const generic = [
    "api", "database", "component", "function", "architecture", "algorithm", "backend", "system",
    "Explain the architecture", "What is an algorithm?", "Show a database example",
    "Що таке архітектура?", "Що таке алгоритм?", "Покажи приклад компонента",
  ];
  for (const prompt of generic) {
    assert.equal(isComplexRequest(prompt), false, prompt);
  }
});

test("C10. a complex-sounding verb without system-level breadth stays simple", () => {
  for (const prompt of [
    "Design the architecture of a blog",
    "Спроєктуй архітектуру блогу",
    "Compare frontend and backend",
    "Plan my study schedule",
    "Refactor this function",
  ]) {
    const a = assessComplexity(prompt);
    assert.equal(a.complex, false, prompt);
    assert.ok(a.score < 3, prompt);
  }
});

test("C11. an intent signal is required: scope + concerns alone are not enough", () => {
  const a = assessComplexity("The router, streaming layer and shared memory of several services are described in the system docs");
  assert.ok(a.score >= 3);
  assert.equal(a.groups.includes("intent"), false);
  assert.equal(a.complex, false);
});

test("C12. simplicity marker vetoes borderline scores only", () => {
  const borderline = assessComplexity("Design a simple architecture for a multi-user app");
  assert.equal(borderline.score, 3);
  assert.equal(borderline.complex, false);
  assert.equal(borderline.reason, "simple-marker");

  const strong = assessComplexity(
    "Design a simple-looking but multi-model architecture with a router, streaming and error recovery",
  );
  assert.ok(strong.score >= 4);
  assert.equal(strong.complex, true);
});

test("C13. typographic and ASCII apostrophes classify identically", () => {
  const ascii = assessComplexity("Спроєктуй для кількох систем спільну пам'ять");
  for (const apostrophe of ["\u2019", "\u02bc"]) {
    const typo = assessComplexity(`Спроєктуй для кількох систем спільну пам${apostrophe}ять`);
    assert.deepEqual(typo.groups, ascii.groups);
    assert.equal(typo.complex, ascii.complex);
  }
  assert.ok(ascii.groups.includes("concern"));
});

test("C14. Auto-mode routing for existing cases is unchanged by the broader classifier", () => {
  // (Complex requests without code/technical keywords now go to Pro AI in Auto: see D-tests.)
  assert.equal(backendOf(route(ARCH_REQUEST, { thinking: false })), "pro-thinking");
  assert.equal(backendOf(route(ARCH_REQUEST, { thinking: true })), "pro-thinking");
  // Existing Auto behavior: exact phrases -> code -> Pro AI Thinking; code terms -> Instruct.
  assert.equal(backendOf(route("Спроєктуй архітектуру проєкту для CRM на React")), "pro-thinking");
  assert.equal(backendOf(route("Напиши функцію на JavaScript, яка перевертає рядок")), "pro-instruct");
  assert.equal(backendOf(route("Поясни простими словами, що таке REST API")), "pro-instruct");
  assert.equal(backendOf(route("Привіт! Як справи?")), "gemma");
  assert.equal(detectIntent(ARCH_REQUEST, "auto"), "chat"); // intent detection untouched
});

test("C15. Auto technical explanation uses the same Pro AI classifier", () => {
  const complexTech = "Design a REST API gateway architecture with several services, streaming and error recovery";
  assert.equal(backendOf(route(complexTech, { thinking: false })), "pro-thinking");
  assert.equal(backendOf(route("Explain what a REST API is", { thinking: false })), "pro-instruct");
  // Standalone "REST" was removed from the keywords (false positive on "rest"); see R1/R2.
});

test("C16. missing model for a complex request -> explicit error, no fallback to Instruct", () => {
  const r = route(ARCH_REQUEST, { selected: THINKING, thinking: false, installed: [INSTRUCT, GEMMA] });
  assert.equal(r.result.ok, false);
  assert.equal(r.result.backend, "pro-thinking");
  assert.match(r.result.message, /sonne-pro-ai:latest/);

  const simple = route("Explain what an API is.", { selected: THINKING, thinking: false, installed: [THINKING, GEMMA] });
  assert.equal(simple.result.ok, false);
  assert.equal(simple.result.backend, "pro-instruct");
});

test("C17. both Pro AI backends still share the same profile and prompt assembly", () => {
  const complex = route(ARCH_REQUEST, { selected: THINKING, thinking: false }).result.backend;
  const simple = route("Explain what an API is.", { selected: THINKING, thinking: false }).result.backend;
  assert.equal(complex.profile, "pro-ai");
  assert.equal(simple.profile, "pro-ai");
  assert.notEqual(complex.backend, simple.backend);
});

// ------------------------------------------------------------
// Auto mode recognizes genuinely complex requests (-> Pro AI)
// ------------------------------------------------------------

// Auto-mode routing recorded BEFORE the complexity change for prompts that must not move.
// Value = backend with Thinking OFF. (One entry was corrected later: see the inline note.)
const AUTO_UNCHANGED = [
  ["Explain what an API is.", "pro-instruct"],
  ["Поясни, що таке API", "pro-instruct"],
  ["Write a simple JavaScript function that reverses a string.", "pro-instruct"],
  ["Create a basic React component.", "pro-instruct"],
  ["Explain what a database index is.", "pro-instruct"],
  ["Поясни, що таке індекс у базі даних", "pro-instruct"], // was gemma before "базі даних" was recognized
  ["Fix this small CSS issue: the button is not centered.", "pro-instruct"],
  ["What is the difference between frontend and backend?", "pro-instruct"],
  ["Яка різниця між frontend і backend?", "pro-instruct"],
  ["Give me a short explanation of Docker.", "pro-instruct"],
  ["Коротко поясни, що таке Docker", "pro-instruct"],
  ["Design a REST API for a todo list", "pro-instruct"],
  ["Спроєктуй архітектуру блогу", "pro-instruct"],
  ["Compare frontend and backend", "pro-instruct"],
  ["Optimize this SQL query", "pro-instruct"],
  ["Translate this technical paragraph: the architecture of a multi-model system with a router, shared memory and streaming error recovery.", "gemma"],
  ["Переклади цей технічний абзац: архітектура системи з кількома моделями, маршрутизатором, спільною пам'яттю та обробкою помилок", "gemma"],
  ["Переклади на англійську: цей API працює швидко", "gemma"],
  ["Привіт! Як справи?", "gemma"],
  ["Порадь ідею для вечері", "gemma"],
  ["Розкажи жарт", "gemma"],
  ["What's the weather like for hiking?", "gemma"],
  ["Поясни, що таке архітектура", "gemma"],
  ["Що таке алгоритм?", "pro-instruct"],
];

test("D1. Auto + complex architecture request -> Pro AI Thinking, Thinking ON and OFF", () => {
  for (const thinking of [false, true]) {
    const r = route(ARCH_REQUEST, { thinking });
    assert.equal(r.mode, "chat"); // detectIntent is untouched; no code/technical keywords
    assert.equal(r.result.ok, true);
    assert.equal(r.result.backend.profile, "pro-ai");
    assert.equal(backendOf(r), "pro-thinking");
    assert.equal(r.result.backend.ollamaModel, THINKING);
    assert.equal(thinkOf(r), true);
  }
});

test("D2. Auto + every complex case (uk + en) -> Pro AI Thinking, Thinking ON and OFF", () => {
  for (const [name, prompt] of COMPLEX_CASES) {
    for (const thinking of [false, true]) {
      const r = route(prompt, { thinking });
      assert.equal(backendOf(r), "pro-thinking", `${name} / thinking=${thinking}`);
      assert.equal(thinkOf(r), true, name);
    }
  }
});

test("D3. Auto + complex: intent detection itself is unchanged", () => {
  assert.equal(detectIntent(ARCH_REQUEST, "auto"), "chat");
  assert.equal(detectIntent("Design a shared persistent memory architecture for several AI agents.", "auto"), "chat");
});

test("D4. Auto: simple explanations, code tasks, translations and conversation route as before (Thinking OFF)", () => {
  for (const [prompt, expected] of AUTO_UNCHANGED) {
    assert.equal(backendOf(route(prompt, { thinking: false })), expected, prompt);
  }
});

test("D5. Auto + Thinking ON: only Pro AI requests change (override); Gemma requests stay Gemma", () => {
  for (const [prompt, expected] of AUTO_UNCHANGED) {
    const wanted = expected === "gemma" ? "gemma" : "pro-thinking";
    assert.equal(backendOf(route(prompt, { thinking: true })), wanted, prompt);
  }
});

test("D6. Auto: translations are never moved to Pro AI by the classifier", () => {
  for (const prompt of TRANSLATION_CASES) {
    for (const thinking of [false, true]) {
      assert.equal(backendOf(route(prompt, { thinking })), "gemma", prompt);
    }
  }
  assert.equal(isTranslationRequest("Переклади цей текст"), true);
  assert.equal(isTranslationRequest("Translate this paragraph"), true);
  assert.equal(isTranslationRequest(ARCH_REQUEST), false);
  // Corrected: an exact legacy phrase inside a translation request is still a translation.
  assert.equal(backendOf(route("Переклади: архітектуру проєкту")), "gemma");
});

test("D7. manual selection keeps priority over Auto complexity", () => {
  for (const thinking of [false, true]) {
    const gemma = route(ARCH_REQUEST, { selected: GEMMA, thinking });
    assert.equal(backendOf(gemma), "gemma");
    assert.equal(thinkOf(gemma), false); // mode is chat: Gemma's existing rule, unchanged
  }
  assert.equal(backendOf(route(ARCH_REQUEST, { selected: THINKING, thinking: false })), "pro-thinking");
  assert.equal(backendOf(route("Привіт", { selected: THINKING, thinking: false })), "pro-instruct");
  assert.equal(backendOf(route("Привіт", { selected: THINKING, thinking: true })), "pro-thinking");
});

test("D8. explicit Chat and Code modes are unchanged", () => {
  for (const thinking of [false, true]) {
    // Explicit Chat: complexity does not move a non-manual request to Pro AI.
    assert.equal(backendOf(route(ARCH_REQUEST, { modeSetting: "chat", thinking })), "gemma");
    // Explicit Code: always Pro AI.
    assert.equal(backendOf(route(ARCH_REQUEST, { modeSetting: "code", thinking })), "pro-thinking");
  }
  assert.equal(backendOf(route("Explain what an API is.", { modeSetting: "chat" })), "gemma");
  assert.equal(backendOf(route("Explain what an API is.", { modeSetting: "code", thinking: false })), "pro-instruct");
  assert.equal(backendOf(route("Explain what an API is.", { modeSetting: "code", thinking: true })), "pro-thinking");
});

test("D9. Auto + complex + missing model -> explicit error, no silent fallback", () => {
  const noThinking = route(ARCH_REQUEST, { installed: [INSTRUCT, GEMMA], thinking: false });
  assert.equal(noThinking.result.ok, false);
  assert.equal(noThinking.result.backend, "pro-thinking");
  assert.match(noThinking.result.message, /sonne-pro-ai:latest/);

  const none = route(ARCH_REQUEST, { installed: [], thinking: true });
  assert.equal(none.result.ok, false);
  assert.equal(none.result.code, "no-supported-models");
});

test("D10. Auto + complex + image request is still an image request", () => {
  assert.equal(detectIntent("Намалюй архітектуру системи з кількома моделями", "auto"), "image");
});

// ------------------------------------------------------------
// Translation priority (regression)
// ------------------------------------------------------------

const TRANSLATION_REGRESSION = [
  ["uk, legacy phrase", "Переклади: архітектуру проєкту"],
  ["en, legacy phrase", "Translate: system architecture"],
  ["en, legacy phrase 2", "Translate into Ukrainian: we need a multi-file change and complex debugging"],
  ["uk, technical words + code term", "Переклади цей код на Python: компонент викликає API через маршрутизатор"],
  ["en, technical words + code term", "Translate this into Ukrainian: the router streams API responses to the Python backend"],
  ["en, code + streaming", "Translate this code comment: streaming API with a router and shared memory"],
  ["uk, code word", "Переклади англійською: цей код працює швидко"],
];

test("E1. translation is never complex, including legacy exact phrases", () => {
  for (const [name, prompt] of TRANSLATION_REGRESSION) {
    const a = assessComplexity(prompt);
    assert.equal(a.complex, false, name);
    assert.equal(a.reason, "translation", name);
    assert.equal(isComplexRequest(prompt), false, name);
  }
});

test("E2. Auto: translations resolve to chat and route to Gemma, Thinking ON and OFF", () => {
  for (const [name, prompt] of TRANSLATION_REGRESSION) {
    for (const thinking of [false, true]) {
      const r = route(prompt, { thinking });
      assert.equal(r.mode, "chat", `${name}: resolved mode`);
      assert.equal(backendOf(r), "gemma", `${name} / thinking=${thinking}`);
      assert.equal(r.result.backend.profile, "gemma", name);
    }
  }
});

test("E3. translation intent beats formatting, code-term and complexity detection in Auto", () => {
  assert.equal(detectIntent("Переклади цей код на Python", "auto"), "chat");
  assert.equal(detectIntent("Translate this markdown table", "auto"), "chat");
  assert.equal(detectIntent("Переклади: архітектуру проєкту", "auto"), "chat");
  assert.equal(detectIntent("Translate: system architecture", "auto"), "chat");
  // Without a translation request the same words keep their existing meaning.
  assert.equal(detectIntent("архітектуру проєкту", "auto"), "code");
  assert.equal(detectIntent("Поясни цей код на Python", "auto"), "code");
});

test("E4. image intent keeps priority over translation", () => {
  assert.equal(detectIntent("Намалюй і переклади: кіт", "auto"), "image");
  assert.equal(detectIntent("Translate this, then draw a cat", "auto"), "image");
});

test("E5. manual Gemma + translation -> Gemma", () => {
  for (const [name, prompt] of TRANSLATION_REGRESSION) {
    for (const thinking of [false, true]) {
      const r = route(prompt, { selected: GEMMA, thinking });
      assert.equal(backendOf(r), "gemma", name);
      assert.equal(thinkOf(r), false, name); // chat mode: Gemma's existing thinking rule
    }
  }
});

test("E6. manual Pro AI + translation: stays Pro AI; Thinking OFF -> Instruct, ON -> Thinking (toggle only)", () => {
  for (const [name, prompt] of TRANSLATION_REGRESSION) {
    const off = route(prompt, { selected: THINKING, thinking: false });
    assert.equal(off.result.backend.profile, "pro-ai", name);
    assert.equal(backendOf(off), "pro-instruct", `${name}: translation must not trigger complexity`);
    assert.equal(thinkOf(off), false, name);

    const on = route(prompt, { selected: THINKING, thinking: true });
    assert.equal(on.result.backend.profile, "pro-ai", name);
    assert.equal(backendOf(on), "pro-thinking", `${name}: Thinking ON is the toggle override`);
  }
});

test("E7. explicit modes with a translation prompt", () => {
  for (const [name, prompt] of TRANSLATION_REGRESSION) {
    // Explicit Chat: Gemma (unless manually Pro AI, covered above).
    assert.equal(backendOf(route(prompt, { modeSetting: "chat", thinking: false })), "gemma", name);
    // Explicit Code: Pro AI by the existing rule, but translation never forces Thinking.
    const code = route(prompt, { modeSetting: "code", thinking: false });
    assert.equal(code.mode, "code", name);
    assert.equal(code.result.backend.profile, "pro-ai", name);
    assert.equal(backendOf(code), "pro-instruct", name);
    assert.equal(backendOf(route(prompt, { modeSetting: "code", thinking: true })), "pro-thinking", name);
  }
});

test("E8. translation + missing model -> explicit error, no substitution", () => {
  const noGemma = route("Переклади: архітектуру проєкту", { installed: [THINKING, INSTRUCT], thinking: true });
  assert.equal(noGemma.result.ok, false);
  assert.equal(noGemma.result.backend, "gemma");

  const noInstruct = route("Translate: system architecture", { selected: THINKING, thinking: false, installed: [THINKING, GEMMA] });
  assert.equal(noInstruct.result.ok, false);
  assert.equal(noInstruct.result.backend, "pro-instruct");
});

// ------------------------------------------------------------
// Ukrainian technical-term matching (regression)
// ------------------------------------------------------------

const DB_FORMS = [
  "база даних", "бази даних", "базі даних", "базу даних", "базою даних",
  "баз даних", "базам даних", "базами даних", "базах даних",
];

test("F1. every common form of 'база даних' is a technical explanation", () => {
  for (const form of DB_FORMS) {
    assert.equal(isTechnicalExplanationRequest(`Поясни, що таке індекс у ${form}`), true, form);
  }
  assert.equal(isTechnicalExplanationRequest("Explain what databases are"), true);
});

test("F2. simple database explanations -> Pro AI Instruct in Auto (Thinking OFF)", () => {
  for (const form of DB_FORMS) {
    const r = route(`Поясни, що таке індекс у ${form}`, { thinking: false });
    assert.equal(r.mode, "chat", form);
    assert.equal(backendOf(r), "pro-instruct", form);
  }
  assert.equal(backendOf(route("Поясни, що таке індекс у базі даних", { thinking: false })), "pro-instruct");
  assert.equal(backendOf(route("Explain what a database index is.", { thinking: false })), "pro-instruct");
  assert.equal(backendOf(route("Як працює індекс у базах даних?", { thinking: false })), "pro-instruct");
});

test("F3. other inflected technical terms are recognized (algorithm, framework, recursion)", () => {
  for (const prompt of [
    "Поясни складність алгоритму", "Розкажи про алгоритми сортування", "Чим фреймворк відрізняється від бібліотеки?",
    "Поясни рекурсію простими словами", "Explain what algorithms are", "Explain how frameworks work",
  ]) {
    assert.equal(isTechnicalExplanationRequest(prompt), true, prompt);
    assert.equal(backendOf(route(prompt, { thinking: false })), "pro-instruct", prompt);
  }
});

test("F4. complex database architecture requests -> Pro AI Thinking in Auto, both toggle states", () => {
  for (const prompt of [
    "Спроєктуй архітектуру розподіленої бази даних з реплікацією між кількома вузлами та відновленням після збоїв",
    "Design a distributed database architecture with replication across several nodes and failure recovery.",
  ]) {
    assert.equal(isComplexRequest(prompt), true, prompt);
    for (const thinking of [false, true]) {
      assert.equal(backendOf(route(prompt, { thinking })), "pro-thinking", `${prompt} / ${thinking}`);
    }
    // Manual Pro AI + Thinking OFF: complexity alone selects Thinking.
    assert.equal(backendOf(route(prompt, { selected: THINKING, thinking: false })), "pro-thinking", prompt);
    // Manual Gemma stays Gemma.
    assert.equal(backendOf(route(prompt, { selected: GEMMA, thinking: false })), "gemma", prompt);
  }
});

test("F5. ordinary conversation mentioning an unrelated technical word is never complex", () => {
  for (const prompt of [
    "I need a good rest this weekend, any ideas?",
    "Порадь ідею для вечері, бо весь день сиджу в Docker-документації",
    "My friend loves git, but tonight we just want a movie suggestion",
    "Розкажи жарт про базу даних",
  ]) {
    assert.equal(isComplexRequest(prompt), false, prompt);
    // Thinking OFF: never Pro AI Thinking.
    assert.notEqual(backendOf(route(prompt, { thinking: false })), "pro-thinking", prompt);
  }
  // Plain conversation with no technical word stays on Gemma.
  assert.equal(backendOf(route("Привіт! Як справи?", { thinking: false })), "gemma");
  assert.equal(backendOf(route("Порадь ідею для вечері", { thinking: true })), "gemma");
});

// ------------------------------------------------------------
// Router stabilization: "rest", translation intent, Instruct-only install
// ------------------------------------------------------------

test("R1. ordinary 'rest' never triggers Pro AI routing in Auto", () => {
  for (const prompt of [
    "I need a good rest this weekend.",
    "Let's rest for a while and then decide.",
    "The rest of the team is on holiday.",
  ]) {
    assert.equal(isTechnicalExplanationRequest(prompt), false, prompt);
    assert.equal(backendOf(route(prompt)), "gemma", prompt);
    assert.equal(backendOf(route(prompt, { thinking: true })), "gemma", prompt);
  }
});

test("R2. 'REST API' is still a technical term and routes to Pro AI Instruct", () => {
  for (const prompt of ["Explain what a REST API is", "Поясни, що таке REST API", "How do REST APIs work?"]) {
    assert.equal(isTechnicalExplanationRequest(prompt), true, prompt);
    assert.equal(backendOf(route(prompt)), "pro-instruct", prompt);
  }
  assert.equal(isTechnicalExplanationRequest("What is an API?"), true);
});

test("T1. direct translation requests -> Gemma in Auto (Thinking OFF and ON)", () => {
  for (const prompt of ["Переклади цей текст англійською.", "Please translate this paragraph into Ukrainian."]) {
    assert.equal(isTranslationRequest(prompt), true, prompt);
    assert.equal(route(prompt).mode, "chat", prompt);
    assert.equal(backendOf(route(prompt, { thinking: false })), "gemma", prompt);
    assert.equal(backendOf(route(prompt, { thinking: true })), "gemma", prompt);
  }
});

test("T2. writing a translation function is a coding request: Instruct (OFF) / Thinking (ON)", () => {
  for (const prompt of [
    "Напиши функцію, яка виконує переклад тексту.",
    "Write a Python function that performs translation of a text.",
    "Create a Python script to translate files.",
  ]) {
    assert.equal(isTranslationRequest(prompt), false, prompt);
    assert.equal(route(prompt).mode, "code", prompt);
    assert.equal(backendOf(route(prompt, { thinking: false })), "pro-instruct", prompt);
    assert.equal(backendOf(route(prompt, { thinking: true })), "pro-thinking", prompt);
  }
});

test("T3. translating text that merely mentions code stays a translation", () => {
  for (const prompt of ["Translate this: write a function that sorts a list", "Переклади: напиши функцію сортування"]) {
    assert.equal(isTranslationRequest(prompt), true, prompt);
    assert.equal(backendOf(route(prompt)), "gemma", prompt);
  }
});

test("T4. manual selection and explicit Code mode keep priority over translation wording", () => {
  const coding = "Напиши функцію, яка виконує переклад тексту.";
  assert.equal(backendOf(route(coding, { selected: GEMMA })), "gemma");
  assert.equal(backendOf(route("Переклади цей текст англійською.", { selected: THINKING })), "pro-instruct");
  assert.equal(backendOf(route("Переклади цей текст англійською.", { modeSetting: "code" })), "pro-instruct");
});

test("P1. Instruct + Gemma installed: Pro AI profile, Thinking OFF, simple task -> Instruct", () => {
  const installed = [INSTRUCT, GEMMA];
  assert.deepEqual(getSelectableModels(installed), [THINKING, GEMMA]);
  const r = route("Привіт, поясни що таке змінна", { selected: THINKING, installed, thinking: false });
  assert.equal(r.result.ok, true);
  assert.equal(backendOf(r), "pro-instruct");
  assert.equal(r.result.backend.profile, "pro-ai");
  assert.equal(r.result.think, false);
});

test("P2. Pro AI Thinking required but not installed -> explicit error, no substitution", () => {
  const installed = [INSTRUCT, GEMMA];
  const cases = [
    ["Привіт", { selected: THINKING, installed, thinking: true }],
    ["Зроби рефакторинг кількох файлів", { selected: THINKING, installed, thinking: false }],
    ["Напиши функцію на JavaScript", { installed, thinking: true }],
  ];
  for (const [prompt, opts] of cases) {
    const r = route(prompt, opts);
    assert.equal(r.result.ok, false, prompt);
    assert.equal(r.result.code, "backend-unavailable", prompt);
    assert.equal(r.result.backend, "pro-thinking", prompt);
    assert.match(r.result.message, /sonne-pro-ai:latest/, prompt);
  }
});

test("P3. Instruct-only install: Pro AI selectable, no silent fallback to Gemma or Thinking", () => {
  const installed = [INSTRUCT];
  assert.deepEqual(getSelectableModels(installed), [THINKING]);
  assert.equal(backendOf(route("Привіт", { selected: THINKING, installed })), "pro-instruct");
  assert.equal(backendOf(route("Привіт", { selected: THINKING, installed, thinking: true })), "ERR:pro-thinking");
  assert.equal(backendOf(route("Привіт!", { installed })), "ERR:gemma");
});

test("T5. artifact position decides: software artifact before the translation word -> coding", () => {
  for (const prompt of [
    "Write a function that performs translation.",
    "Напиши функцію, яка виконує переклад тексту.",
    "Write a translation function in Python.",
  ]) {
    assert.equal(isTranslationRequest(prompt), false, prompt);
  }
});

test("T6. authoring verb without a software artifact before the translation word -> not software-building", () => {
  for (const prompt of [
    "Write a paragraph about translation and include a function example.",
    "Напиши текст про переклад і наведи приклад функції.",
    "Translate this: write a function that sorts a list.",
  ]) {
    assert.equal(isTranslationRequest(prompt), true, prompt);
    assert.equal(route(prompt).mode, "chat", prompt);
    assert.equal(backendOf(route(prompt, { thinking: false })), "gemma", prompt);
    assert.equal(backendOf(route(prompt, { thinking: true })), "gemma", prompt);
  }
});

test("T7. Ukrainian coding request with translation routes to Pro AI; the writing request stays on Gemma", () => {
  const coding = "Напиши функцію, яка виконує переклад тексту.";
  assert.equal(backendOf(route(coding, { thinking: false })), "pro-instruct");
  assert.equal(backendOf(route(coding, { thinking: true })), "pro-thinking");
  assert.equal(backendOf(route("Напиши текст про переклад і наведи приклад функції.")), "gemma");
});


test("adaptive output token limit is sent for all backends", () => {
  const messages = [{ role: "user", content: "hello" }];

  const instruct = buildChatRequestBody(BACKENDS[1], messages, false, 3072);
  assert.equal(instruct.options?.num_predict, 3072);
  assert.equal(instruct.options?.num_ctx, 12288);
  assert.equal(instruct.options?.temperature, 0.65);

  const thinking = buildChatRequestBody(BACKENDS[0], messages, true, 2048);
  assert.deepEqual(thinking.options, { num_predict: 2048 });

  const gemma = buildChatRequestBody(BACKENDS[2], messages, false, 1024);
  assert.deepEqual(gemma.options, { num_predict: 1024 });
});

test("adaptive output token limit rejects invalid values", () => {
  assert.throws(
    () => buildChatRequestBody(BACKENDS[2], [], false, 0),
    /positive finite number/,
  );
});
