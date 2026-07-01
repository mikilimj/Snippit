export type IngestionMode = "import" | "empty" | "watch";

export function canContinueFromRootStep(rootPath: string | null): boolean {
  return rootPath !== null && rootPath.trim().length > 0;
}

export function canFinishIngestionStep(
  mode: IngestionMode | null,
  sourceFolder: string | null,
): boolean {
  if (mode === null) return false;
  if (mode === "import" || mode === "watch") {
    return sourceFolder !== null && sourceFolder.trim().length > 0;
  }
  return true;
}
