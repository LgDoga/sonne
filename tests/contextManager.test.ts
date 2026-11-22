import { test } from "node:test";
import assert from "node:assert/strict";
import {
  type ContextBudget,
  type ContextMessage,
  estimateMessageTokens,
  estimateTokens,
  resolveContextBudget,
  selectHistory,
} from "../src/contextManager.ts";

const budgetOf = (inputBudget: number): ContextBudget => ({
  contextWindow: inputBudget * 2,
  windowSource: "request-options",
  outputReserve: 0,
  safetyMargin: 0,
  inputBudget,
});

// 40 Latin letters -> 13 tokens + overhead 4 = 17 per message.
const msg = (role: "user" | "assistant", tag: string): ContextMessage => ({
  role,
  content: tag.padEnd(40, "x"),
});
const COST = estimateMessageTokens(msg("user", "u0-")); // all test messages share this shape
const SYSTEM = "sys";
const USER = "now";
const FIXED = estimateMessageTokens({ content: SYSTEM }) + estimateMessageTokens({ content: USER });

function turns(n: number): ContextMessage[] {
  const out: ContextMessage[] = [];
  for (let i = 0; i < n; i++) out.push(msg("user", `u${i}-`), msg("assistant", `a${i}-`));
  return out;
}
const run = (history: ContextMessage[], inputBudget: number) =>
  selectHistory(history, { systemPrompt: SYSTEM, userPrompt: USER, budget: budgetOf(inputBudget) });

test("history below budget is unchanged", () => {
  const history = turns(3);
  const result = run(history, FIXED + COST * 6);
  assert.deepEqual(result.history, history);
  assert.equal(result.stats.status, "within-budget");
  assert.equal(result.stats.optimized, false);
  assert.equal(result.stats.omittedHistoryMessages, 0);
});

test("empty history works", () => {
  const result = run([], 1000);
  assert.deepEqual(result.history, []);
  assert.equal(result.stats.status, "within-budget");
  assert.equal(result.stats.estOriginalHistoryTokens, 0);
});

test("oldest messages are removed first; order and roles preserved", () => {
  const history = turns(4); // 8 messages
  const result = run(history, FIXED + COST * 4); // room for 2 turns
  assert.deepEqual(result.history, history.slice(4));
  assert.deepEqual(result.history.map((m) => m.role), ["user", "assistant", "user", "assistant"]);
  assert.equal(result.stats.status, "trimmed");
  assert.equal(result.stats.optimized, true);
});

test("stats are calculated correctly", () => {
  const result = run(turns(4), FIXED + COST * 4);
  const s = result.stats;
  assert.equal(s.totalHistoryMessages, 8);
  assert.equal(s.selectedHistoryMessages, 4);
  assert.equal(s.omittedHistoryMessages, 4);
  assert.equal(s.estOriginalHistoryTokens, COST * 8);
  assert.equal(s.estSelectedHistoryTokens, COST * 4);
  assert.equal(s.estRemovedHistoryTokens, COST * 4);
  assert.equal(s.estFixedTokens, FIXED);
  assert.equal(s.estInputTokens, FIXED + COST * 4);
  assert.ok(s.estInputTokens <= s.inputBudget);
});

test("whole turns only: no orphaned leading assistant message", () => {
  // Room for 3 messages: the 4th-from-last turn is cut whole, not split.
  const result = run(turns(3), FIXED + COST * 3);
  assert.equal(result.history.length, 2);
  assert.equal(result.history[0].role, "user");
});

test("original history and its objects are not modified", () => {
  const history = turns(4);
  const snapshot = JSON.stringify(history);
  const result = run(history, FIXED + COST * 2);
  assert.equal(JSON.stringify(history), snapshot);
  assert.equal(history.length, 8);
  assert.notEqual(result.history[0], history[history.length - 2]);
});

test("a single oversized history message stops the walk safely", () => {
  const history = [
    ...turns(1),
    { role: "user" as const, content: "x".repeat(5000) },
    msg("assistant", "a9-"),
    ...turns(1),
  ];
  const result = run(history, FIXED + COST * 3);
  // Only the newest small turn fits; the oversized turn and everything older are omitted.
  assert.equal(result.history.length, 2);
  assert.equal(result.history[0].content.startsWith("u0-"), true);
  assert.equal(result.stats.status, "trimmed");
});

test("oversized current message and system prompt are never truncated", () => {
  const big = "я".repeat(5000);
  const history = turns(2);
  const result = selectHistory(history, {
    systemPrompt: SYSTEM,
    userPrompt: big,
    budget: budgetOf(500),
  });
  assert.equal(result.stats.status, "fixed-content-exceeds-budget");
  assert.deepEqual(result.history, []);
  assert.equal(result.stats.omittedHistoryMessages, 4);
  assert.ok(result.stats.estFixedTokens > 500);
});

test("system prompt and current message are accounted for before history", () => {
  const long = "a".repeat(300);
  const small = selectHistory(turns(3), {
    systemPrompt: long,
    userPrompt: USER,
    budget: budgetOf(FIXED + COST * 6),
  });
  assert.ok(small.history.length < 6);
});

test("unknown model uses the conservative fallback", () => {
  const budget = resolveContextBudget({ ollamaModel: "something-unknown:1b" }, false);
  assert.equal(budget.windowSource, "fallback");
  assert.equal(budget.contextWindow, 4096);
  assert.ok(budget.inputBudget > 0 && budget.inputBudget < budget.contextWindow);
});

test("request-level num_ctx is used and output is reserved", () => {
  const budget = resolveContextBudget(
    { ollamaModel: "m", options: { num_ctx: 12288, num_predict: 8192 } },
    false,
  );
  assert.equal(budget.windowSource, "request-options");
  assert.equal(budget.contextWindow, 12288);
  assert.equal(budget.inputBudget, 12288 - budget.outputReserve - budget.safetyMargin);
  assert.ok(budget.outputReserve >= 512);
  const thinking = resolveContextBudget(
    { ollamaModel: "m", options: { num_ctx: 12288, num_predict: 8192 } },
    true,
  );
  assert.ok(thinking.inputBudget < budget.inputBudget);
});

test("estimator is non-zero, monotonic and heavier for Cyrillic than Latin", () => {
  assert.equal(estimateTokens(""), 0);
  assert.ok(estimateTokens("привіт світе") > estimateTokens("hello world"));
  assert.ok(estimateTokens("a".repeat(100)) > estimateTokens("a".repeat(10)));
});
