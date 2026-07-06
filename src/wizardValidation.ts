export type IngestionMode = "import" | "empty" | "watch" | "restore";

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

// Restore mode's extra step: nothing can be imported until a remote is connected.
export function canFinishRestoreStep(connected: boolean): boolean {
  return connected;
}
