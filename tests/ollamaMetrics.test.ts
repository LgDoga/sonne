import test from "node:test";
import assert from "node:assert/strict";
import { getPromptEvalMetrics } from "../src/ollamaMetrics.ts";

test("prompt metrics fall back to total tokens when cache metrics are unavailable", () => {
  assert.deepEqual(getPromptEvalMetrics(1200, undefined), {
    total: 1200, cached: null, uncached: 1200, rateTokens: 1200,
  });
});

test("prompt metrics separate cached and uncached tokens", () => {
  assert.deepEqual(getPromptEvalMetrics(1200, 450), {
    total: 1200, cached: 450, uncached: 750, rateTokens: 750,
  });
});

test("prompt metrics handle an all-cached prompt", () => {
  assert.deepEqual(getPromptEvalMetrics(1200, 1200), {
    total: 1200, cached: 1200, uncached: 0, rateTokens: 0,
  });
});

test("prompt metrics reject invalid counts and clamp cache count to total", () => {
  assert.deepEqual(getPromptEvalMetrics(100, 150), {
    total: 100, cached: 100, uncached: 0, rateTokens: 0,
  });
  assert.deepEqual(getPromptEvalMetrics(-1, "bad"), {
    total: null, cached: null, uncached: null, rateTokens: null,
  });
});
