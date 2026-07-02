import type { ClipRow } from "./api";

/**
 * The clip filter currently applied to the library sidebar. The special views
 * map to dedicated backend queries; `folder` filters by membership (spec §4.2).
 */
export type LibraryView =
  | { kind: "all" }
  | { kind: "favorites" }
  | { kind: "unsorted" }
  | { kind: "folder"; id: number };

/**
 * Whether a clip that was *just imported or opened* should appear in `view`.
 * A fresh clip has no folder membership and is not a favorite, so it only
 * belongs in the "All" and "Unsorted" views — never in Favorites or a folder.
 */
export function newClipBelongsInView(view: LibraryView): boolean {
  return view.kind === "all" || view.kind === "unsorted";
}

/**
 * Returns a new clip list with the given clip's favorite flag set. In the
 * Favorites view an un-favorited clip is dropped, since it no longer belongs
 * there; in every other view the row is patched in place.
 */
export function applyFavorite(
  clips: ClipRow[],
  id: number,
  favorite: boolean,
  view: LibraryView,
): ClipRow[] {
  if (view.kind === "favorites" && !favorite) {
    return clips.filter(c => c.id !== id);
  }
  return clips.map(c => (c.id === id ? { ...c, favorite } : c));
}

/** Returns a new clip list with the matching clip's display name updated. */
export function applyRename(clips: ClipRow[], id: number, name: string): ClipRow[] {
  return clips.map(c => (c.id === id ? { ...c, displayName: name } : c));
}
