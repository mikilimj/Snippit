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

/**
 * Orders clips newest-captured first, mirroring the backend's
 * `ORDER BY effective_captured_at DESC, id DESC`. The backend already returns
 * rows in this order; this exists for lists mutated on the client (a clip just
 * ingested or opened), which must not break the ordering.
 */
export function sortByCapture(clips: ClipRow[]): ClipRow[] {
  return [...clips].sort(
    (a, b) => b.effectiveCapturedAt - a.effectiveCapturedAt || b.id - a.id,
  );
}

/**
 * Inserts `row` into `clips` (replacing any row with the same path) while
 * preserving newest-captured-first order. Used when a newly ingested clip
 * arrives: it is not necessarily the newest capture on disk (a recorder can
 * drop an older file later), so simply prepending it would be wrong.
 */
export function upsertByCapture(clips: ClipRow[], row: ClipRow): ClipRow[] {
  return sortByCapture([row, ...clips.filter(c => c.path !== row.path)]);
}

/**
 * A short, locale-aware label for when a clip was captured — the date only, since
 * the sidebar row is narrow and the time adds little when sorting is by capture
 * date. `null` when the container carried no capture metadata at all, so the UI
 * can stay quiet rather than showing a fallback mtime as if it were a recording
 * date.
 */
export function captureLabel(row: ClipRow): string | null {
  if (row.capturedAt === null) return null;
  return new Date(row.capturedAt * 1000).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
