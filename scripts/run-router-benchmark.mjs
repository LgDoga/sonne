
import {
  assessComplexity,
  detectIntent,
  isTechnicalExplanationRequest,
  routeRequest,
} from "../src/modelRouting.ts";

import { benchmarkConfig, benchmarkCases } from "./router-benchmark-cases.mjs";

const EXPECTED_CLARIFICATION = "CL";

const results = benchmarkCases.map((testCase) => {
  const mode = detectIntent(testCase.prompt, benchmarkConfig.modeSetting);
  const complexity = assessComplexity(testCase.prompt);
  const technical = isTechnicalExplanationRequest(testCase.prompt);

  const result = routeRequest(
    mode,
    benchmarkConfig.installedModels,
    benchmarkConfig.selectedModel,
    testCase.prompt,
    benchmarkConfig.modeSetting === "auto",
    benchmarkConfig.allowThinking
  );

  const actual = result.ok
    ? result.backend.backend
    : `ERR:${result.code}`;

  const isClarificationCase = testCase.expected === EXPECTED_CLARIFICATION;

  let status;

  if (isClarificationCase) {
    status = "UNSUPPORTED";
  } else {
    status = actual === testCase.expected ? "PASS" : "FAIL";
  }

  return {
    id: testCase.id,
    category: testCase.id[0],
    expected: testCase.expected,
    actual,
    status,
    mode,
    complexity: complexity.score,
    groups: complexity.groups.join(",") || "-",
    reason: complexity.reason,
    technical,
    contextProvided: Boolean(testCase.contextProvided),
    requiresFixture: Boolean(testCase.requiresFixture),
    prompt: testCase.prompt,
  };
});

const total = results.length;
const passed = results.filter((item) => item.status === "PASS").length;
const failed = results.filter((item) => item.status === "FAIL").length;
const unsupported = results.filter(
  (item) => item.status === "UNSUPPORTED"
).length;

const comparable = total - unsupported;
const accuracy = comparable > 0
  ? ((passed / comparable) * 100).toFixed(1)
  : "N/A";

console.log("\n========== SONNE AUTO ROUTER BENCHMARK ==========\n");

console.log("Configuration:");
console.log(`Mode: ${benchmarkConfig.modeSetting}`);
console.log(`Thinking: ${benchmarkConfig.allowThinking ? "ON" : "OFF"}`);
console.log(`Installed models: ${benchmarkConfig.installedModels.length}`);
console.log(`Test cases: ${total}`);

console.log("\nOverall results:");
console.log(`PASS: ${passed}`);
console.log(`FAIL: ${failed}`);
console.log(`UNSUPPORTED: ${unsupported}`);
console.log(`Backend accuracy: ${accuracy}% (${passed}/${comparable})`);

console.log("\nResults by category:");

const categories = [...new Set(results.map((item) => item.category))];

for (const category of categories) {
  const categoryResults = results.filter(
    (item) => item.category === category
  );

  const categoryPassed = categoryResults.filter(
    (item) => item.status === "PASS"
  ).length;

  const categoryFailed = categoryResults.filter(
    (item) => item.status === "FAIL"
  ).length;

  const categoryUnsupported = categoryResults.filter(
    (item) => item.status === "UNSUPPORTED"
  ).length;

  console.log(
    `${category}: ${categoryPassed} PASS | ${categoryFailed} FAIL | ${categoryUnsupported} UNSUPPORTED | ${categoryResults.length} total`
  );
}

const failures = results.filter((item) => item.status === "FAIL");

console.log("\nRouting mismatches:");

if (failures.length === 0) {
  console.log("No routing mismatches found.");
} else {
  for (const item of failures) {
    console.log(`\n[${item.id}] ${item.status}`);
    console.log(`Expected: ${item.expected}`);
    console.log(`Actual: ${item.actual}`);
    console.log(`Mode: ${item.mode}`);
    console.log(`Complexity: ${item.complexity}`);
    console.log(`Groups: ${item.groups}`);
    console.log(`Reason: ${item.reason}`);
    console.log(`Technical explanation: ${item.technical}`);
    console.log(`Prompt: ${item.prompt}`);
  }
}

console.log("\nUnsupported clarification cases:");

for (const item of results.filter(
  (item) => item.status === "UNSUPPORTED"
)) {
  console.log(`[${item.id}] Expected clarification, actual: ${item.actual}`);
  console.log(`Prompt: ${item.prompt}`);
}

console.log("\nBenchmark limitations:");

const contextCases = results.filter((item) => item.contextProvided);
const fixtureCases = results.filter((item) => item.requiresFixture);

console.log(
  `Context-dependent cases: ${contextCases.length}. Current routeRequest receives only the current prompt, not conversation history.`
);

console.log(
  `Fixture-dependent cases: ${fixtureCases.length}. Fixtures are not passed to the router; these cases test routing selection only, not debugging quality.`
);

console.log("\n=================================================\n");
