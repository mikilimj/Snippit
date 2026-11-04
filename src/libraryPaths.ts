// Clip paths relative to the app's own `Clips/` folder. This is the key the
// sync index stores, and it is also what tells an "imported into Snippit" copy
// apart from a file the user simply opened in place from another folder.

// A clip's rel-name: its path relative to `<root>/Clips/`, `/`-separated —
// the key sync_state stores. null when the clip lives outside the Clips folder.
export function clipRelName(appRoot: string | null | undefined, path: string): string | null {
  if (typeof appRoot !== "string") return null;
  const norm = path.replace(/\\/g, "/");
  const prefix = `${appRoot.replace(/\\/g, "/").replace(/\/$/, "")}/Clips/`;
  return norm.startsWith(prefix) ? norm.slice(prefix.length) : null;
}

/** True when the clip lives inside the app's own `Clips/` folder. */
export function isInClipsFolder(appRoot: string | null | undefined, path: string): boolean {
  return clipRelName(appRoot, path) !== null;
}

/**
 * Whether a clip can be dropped from the library while its file stays on disk.
 *
 * Clips inside `Clips/` are the app's own copies and the folder watcher re-indexes
 * them on the next scan, so "remove from Snippit only" would just make them
 * reappear — for those, deleting the file is the only real removal. Files opened
 * in place from anywhere else are never re-indexed, so hiding them is fine.
 */
export function canRemoveIndexOnly(appRoot: string | null | undefined, path: string): boolean {
  return !isInClipsFolder(appRoot, path);
}
