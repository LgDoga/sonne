export type PromptEvalMetrics = {
  total: number | null;
  cached: number | null;
  uncached: number | null;
  /** Use uncached tokens when Ollama reports a cache count; otherwise use total. */
  rateTokens: number | null;
};

function validCount(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

export function getPromptEvalMetrics(
  totalValue: unknown,
  cachedValue: unknown,
): PromptEvalMetrics {
  const total = validCount(totalValue) ? totalValue : null;
  const cached = validCount(cachedValue)
    ? Math.min(cachedValue, total ?? cachedValue)
    : null;
  const uncached = total === null
    ? null
    : Math.max(0, total - (cached ?? 0));

  return {
    total,
    cached,
    uncached,
    rateTokens: cached === null ? total : uncached,
  };
}
