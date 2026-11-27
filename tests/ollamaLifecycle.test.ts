import test from "node:test";
import assert from "node:assert/strict";
import { getOllamaModelsToUnload } from "../src/ollamaLifecycle.ts";

test("unload queue deduplicates the tracked model and configured models", () => {
  assert.deepEqual(
    getOllamaModelsToUnload("gemma:latest", "pro:latest", ["gemma:latest", "old:latest"]),
    ["gemma:latest", "old:latest"],
  );
});

test("unload queue never unloads the model being requested", () => {
  assert.deepEqual(
    getOllamaModelsToUnload("pro:latest", "pro:latest", ["pro:latest", "gemma:latest"]),
    ["gemma:latest"],
  );
});

test("unload queue preserves order and ignores empty entries", () => {
  assert.deepEqual(
    getOllamaModelsToUnload(null, "next:latest", ["", "gemma:latest", "other:latest", "gemma:latest"]),
    ["gemma:latest", "other:latest"],
  );
});
