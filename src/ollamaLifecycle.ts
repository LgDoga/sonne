/**
 * Build a stable, deduplicated list of models to unload before a request.
 * The tracked previous model is first; configured extras follow in order.
 */
export function getOllamaModelsToUnload(
  previousModel: string | null,
  nextModel: string,
  additionalModels: readonly string[] = [],
): string[] {
  const models = new Set<string>();
  if (previousModel && previousModel !== nextModel) models.add(previousModel);

  for (const model of additionalModels) {
    if (model && model !== nextModel) models.add(model);
  }

  return [...models];
}
