
import test from "node:test";
import assert from "node:assert/strict";

import { classifyTask } from "../src/modelRouting.ts";

test("Router 2.0 — Task Classification", async (t) => {
  const cases = [
    {
      name: "everyday conversation",
      prompt: "Який фільм подивитися сьогодні ввечері?",
      expected: "conversation",
    },
    {
      name: "casual message with incidental React mention",
      prompt: "Сьогодні дивився відео про React, було цікаво",
      expected: "conversation",
    },
    {
      name: "technical explanation",
      prompt: "Поясни, як працює WebSocket",
      expected: "explanation",
    },
    {
      name: "simple coding task",
      prompt: "Напиши функцію JavaScript для сортування масиву",
      expected: "coding",
    },
    {
      name: "complex code analysis",
      prompt: "Проаналізуй причини витоку пам'яті в React-застосунку",
      expected: "analysis",
    },
    {
      name: "system architecture planning",
      prompt: "Спроєктуй архітектуру багатомодельної AI-системи",
      expected: "planning",
    },
    {
      name: "technical translation",
      prompt: "Переклади цей текст про Docker англійською",
      expected: "translation",
    },
    {
      name: "debugging",
      prompt: "Знайди та виправ помилку в цьому JavaScript-коді",
      expected: "debugging",
    },
    {
      name: "comparison",
      prompt: "Порівняй PostgreSQL і MongoDB за продуктивністю",
      expected: "comparison",
    },
    {
      name: "writing task",
      prompt: "Напиши професійний супровідний лист англійською",
      expected: "writing",
    },
  ];

  for (const item of cases) {
    await t.test(item.name, () => {
      const result = classifyTask(item.prompt);

      assert.equal(
        result.type,
        item.expected,
        `Prompt: "${item.prompt}"`,
      );
    });
  }
});

import { wantedBackendV2 } from "../src/modelRouting.ts";

test("Router 2.0 — Backend Selection", async (t) => {
  const cases = [
    {
      name: "everyday conversation -> Gemma",
      prompt: "Який фільм подивитися сьогодні?",
      mode: "chat",
      expected: "gemma",
    },
    {
      name: "casual React mention -> Gemma",
      prompt: "Сьогодні дивився відео про React, було цікаво",
      mode: "chat",
      expected: "gemma",
    },
    {
      name: "technical explanation -> Pro Instruct",
      prompt: "Поясни, як працює WebSocket",
      mode: "chat",
      expected: "pro-instruct",
    },
    {
      name: "simple coding task -> Pro Instruct",
      prompt: "Напиши функцію JavaScript для сортування масиву",
      mode: "code",
      expected: "pro-instruct",
    },
    {
      name: "complex architecture -> Pro Thinking",
      prompt: "Спроєктуй архітектуру багатомодельної AI-системи з обробкою помилок та масштабуванням",
      mode: "chat",
      expected: "pro-thinking",
    },
    {
      name: "technical translation -> Gemma",
      prompt: "Переклади цей текст про Docker англійською",
      mode: "chat",
      expected: "gemma",
    },
  ];

  for (const item of cases) {
    await t.test(item.name, () => {
      const result = wantedBackendV2(
        item.mode,
        "auto",
        item.prompt,
        true,
        false,
      );

      assert.equal(result, item.expected, `Prompt: "${item.prompt}"`);
    });
  }
});

test("Router 2.0 — Manual Selection & Thinking", async (t) => {
  const cases = [
    {
      name: "manual Pro AI + everyday conversation -> Pro Instruct",
      prompt: "Який фільм подивитися сьогодні?",
      selectedModel: "sonne-pro-ai:latest",
      allowThinking: false,
      expected: "pro-instruct",
    },
    {
      name: "manual Pro AI + Thinking ON -> Pro Thinking",
      prompt: "Який фільм подивитися сьогодні?",
      selectedModel: "sonne-pro-ai:latest",
      allowThinking: true,
      expected: "pro-thinking",
    },
    {
      name: "manual Pro AI + Thinking OFF + simple task -> Pro Instruct",
      prompt: "Поясни, як працює WebSocket",
      selectedModel: "sonne-pro-ai:latest",
      allowThinking: false,
      expected: "pro-instruct",
    },
    {
      name: "manual Gemma + complex architecture -> Gemma",
      prompt: "Спроєктуй архітектуру багатомодельної AI-системи з обробкою помилок та масштабуванням",
      selectedModel: "sonne-gemma:latest",
      allowThinking: true,
      expected: "gemma",
    },
    {
      name: "Auto + everyday conversation + Thinking ON -> Gemma",
      prompt: "Який фільм подивитися сьогодні?",
      selectedModel: "auto",
      allowThinking: true,
      expected: "gemma",
    },
  ];

  for (const item of cases) {
    await t.test(item.name, () => {
      const result = wantedBackendV2(
        "chat",
        item.selectedModel,
        item.prompt,
        item.selectedModel === "auto",
        item.allowThinking,
      );

      assert.equal(result, item.expected, `Prompt: "${item.prompt}"`);
    });
  }
});

test("Router 2.0 — Complexity Selection", async (t) => {
  const cases = [
    {
      name: "simple WebSocket explanation -> Pro Instruct",
      prompt: "Поясни, як працює WebSocket",
      expected: "pro-instruct",
    },
    {
      name: "complex WebSocket system architecture -> Pro Thinking",
      prompt: "Проаналізуй архітектуру WebSocket-системи, спроєктуй масштабування, обробку помилок та відновлення з'єднань між усіма сервісами",
      expected: "pro-thinking",
    },
    {
      name: "simple React component -> Pro Instruct",
      prompt: "Напиши простий React-компонент кнопки",
      expected: "pro-instruct",
    },
    {
      name: "multi-module refactoring -> Pro Thinking",
      prompt: "Виконай рефакторинг багатьох взаємопов'язаних модулів системи, проаналізуй залежності, обробку помилок та узгодженість стану",
      expected: "pro-thinking",
    },
    {
      name: "simple technology comparison -> Pro Instruct",
      prompt: "Порівняй PostgreSQL і MongoDB за основними характеристиками",
      expected: "pro-instruct",
    },
  ];

  for (const item of cases) {
    await t.test(item.name, () => {
      const result = wantedBackendV2(
        "chat",
        "auto",
        item.prompt,
        true,
        false,
      );

      assert.equal(result, item.expected, `Prompt: "${item.prompt}"`);
    });
  }
});

test("Router 2.0 — Task-Type Coverage", async (t) => {
  const cases = [
    {
      name: "simple debugging -> Pro Instruct",
      prompt: "Знайди та виправ помилку в цій простій JavaScript-функції",
      expected: "pro-instruct",
    },
    {
      name: "complex debugging -> Pro Thinking",
      prompt: "Проаналізуй причини системних помилок у багатьох взаємопов'язаних сервісах, спроєктуй діагностику, відновлення та обробку збоїв",
      expected: "pro-thinking",
    },
    {
      name: "simple analysis -> Pro Instruct",
      prompt: "Проаналізуй цей невеликий фрагмент JavaScript-коду",
      expected: "pro-instruct",
    },
    {
      name: "complex system analysis -> Pro Thinking",
      prompt: "Проаналізуй архітектуру всієї системи, залежності між модулями, продуктивність, відмовостійкість та можливості масштабування",
      expected: "pro-thinking",
    },
    {
      name: "simple planning -> Pro Instruct",
      prompt: "Сплануй просту структуру невеликого React-компонента",
      expected: "pro-instruct",
    },
    {
      name: "complex architecture planning -> Pro Thinking",
      prompt: "Спроєктуй архітектуру багатомодельної AI-системи з оркестрацією моделей, спільною пам'яттю, обробкою помилок та масштабуванням",
      expected: "pro-thinking",
    },
    {
      name: "simple comparison -> Pro Instruct",
      prompt: "Порівняй React і Vue за основними характеристиками",
      expected: "pro-instruct",
    },
    {
      name: "complex technical comparison -> Pro Thinking",
      prompt: "Проаналізуй та порівняй архітектури мікросервісної та монолітної систем, їхню відмовостійкість, масштабованість, продуктивність і компроміси",
      expected: "pro-thinking",
    },
    {
      name: "writing task -> Gemma",
      prompt: "Напиши короткий професійний супровідний лист англійською",
      expected: "gemma",
    },
    {
      name: "technical translation -> Gemma",
      prompt: "Переклади технічну документацію про архітектуру Kubernetes англійською",
      expected: "gemma",
    },
  ];

  for (const item of cases) {
    await t.test(item.name, () => {
      const result = wantedBackendV2(
        "chat",
        "auto",
        item.prompt,
        true,
        false,
      );

      assert.equal(result, item.expected, `Prompt: "${item.prompt}"`);
    });
  }
});

test("Router 2.0 — Classification Priority", async (t) => {
  const cases = [
    {
      name: "analyzing architecture -> analysis, not planning",
      prompt: "Проаналізуй архітектуру системи та її продуктивність",
      expected: "analysis",
    },
    {
      name: "explaining architecture -> explanation, not planning",
      prompt: "Поясни, як працює архітектура мікросервісів",
      expected: "explanation",
    },
    {
      name: "writing an article about architecture -> writing",
      prompt: "Напиши статтю про архітектуру програмного забезпечення",
      expected: "writing",
    },
    {
      name: "planning architecture -> planning",
      prompt: "Спроєктуй архітектуру багатомодельної AI-системи",
      expected: "planning",
    },
    {
      name: "analyzing and comparing architecture -> comparison",
      prompt: "Порівняй архітектури React і Vue",
      expected: "comparison",
    },
    {
      name: "explaining a technical term -> explanation",
      prompt: "Поясни, що таке WebSocket",
      expected: "explanation",
    },
    {
      name: "writing a cover letter -> writing",
      prompt: "Напиши професійний супровідний лист англійською",
      expected: "writing",
    },
    {
      name: "planning a React component -> planning",
      prompt: "Сплануй структуру React-компонента",
      expected: "planning",
    },
  ];

  for (const item of cases) {
    await t.test(item.name, () => {
      const result = classifyTask(item.prompt);
      assert.equal(result.type, item.expected, `Prompt: "${item.prompt}"`);
    });
  }
});
