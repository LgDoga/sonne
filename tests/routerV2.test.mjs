
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
