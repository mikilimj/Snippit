import { describe, it, expect } from "vitest";
import {
  newClipBelongsInView,
  applyFavorite,
  applyRename,
  sortByCapture,
  upsertByCapture,
  captureLabel,
  type LibraryView,
} from "./libraryView";
import type { ClipRow } from "./api";

function row(over: Partial<ClipRow>): ClipRow {
  return {
    id: 1,
    path: "/v/a.mp4",
    displayName: "a.mp4",
    durationSecs: 1,
    width: 1,
    height: 1,
    container: "mp4",
    videoCodec: "h264",
    audioTrackCount: 0,
    thumbnailPath: null,
    favorite: false,
    missing: false,
    addedAt: 0,
    capturedAt: null,
    effectiveCapturedAt: 0,
    ...over,
  };
}

const ALL: LibraryView = { kind: "all" };
const FAVES: LibraryView = { kind: "favorites" };
const UNSORTED: LibraryView = { kind: "unsorted" };
const FOLDER: LibraryView = { kind: "folder", id: 7 };

describe("newClipBelongsInView", () => {
  it("shows freshly imported clips in All and Unsorted", () => {
    // A newly ingested clip has no folder membership and isn't a favorite.
    expect(newClipBelongsInView(ALL)).toBe(true);
    expect(newClipBelongsInView(UNSORTED)).toBe(true);
  });

  it("hides freshly imported clips from Favorites and specific folders", () => {
    expect(newClipBelongsInView(FAVES)).toBe(false);
    expect(newClipBelongsInView(FOLDER)).toBe(false);
  });
});

describe("applyFavorite", () => {
  it("patches the favorite flag of the matching row", () => {
    const clips = [row({ id: 1 }), row({ id: 2 })];
    const next = applyFavorite(clips, 2, true, ALL);
    expect(next.find(c => c.id === 2)?.favorite).toBe(true);
    expect(next.find(c => c.id === 1)?.favorite).toBe(false);
  });

  it("drops a row from the Favorites view when it is un-favorited", () => {
    const clips = [row({ id: 1, favorite: true }), row({ id: 2, favorite: true })];
    const next = applyFavorite(clips, 1, false, FAVES);
    expect(next.map(c => c.id)).toEqual([2]);
  });

  it("keeps a newly-favorited row visible in the Favorites view", () => {
    const clips = [row({ id: 1, favorite: false })];
    const next = applyFavorite(clips, 1, true, FAVES);
    expect(next.map(c => c.id)).toEqual([1]);
    expect(next[0].favorite).toBe(true);
  });
});

describe("applyRename", () => {
  it("updates only the matching row's display name", () => {
    const clips = [row({ id: 1, displayName: "old" }), row({ id: 2, displayName: "keep" })];
    const next = applyRename(clips, 1, "new");
    expect(next.find(c => c.id === 1)?.displayName).toBe("new");
    expect(next.find(c => c.id === 2)?.displayName).toBe("keep");
  });
});

describe("sortByCapture", () => {
  it("orders by effective capture time, newest first", () => {
    const clips = [
      row({ id: 1, effectiveCapturedAt: 100 }),
      row({ id: 2, effectiveCapturedAt: 300 }),
      row({ id: 3, effectiveCapturedAt: 200 }),
    ];
    expect(sortByCapture(clips).map(c => c.id)).toEqual([2, 3, 1]);
  });

  it("orders by capture time, not by insertion id", () => {
    // The old behaviour was `ORDER BY id DESC`, so a high id always won. A clip
    // ingested late but captured early must now sort behind older-ingested footage.
    const lateIngest = row({ id: 99, effectiveCapturedAt: 10 });
    const earlyIngest = row({ id: 2, effectiveCapturedAt: 5_000 });
    expect(sortByCapture([lateIngest, earlyIngest]).map(c => c.id)).toEqual([2, 99]);
  });

  it("falls back to id DESC for clips captured in the same second", () => {
    const a = row({ id: 4, effectiveCapturedAt: 700 });
    const b = row({ id: 9, effectiveCapturedAt: 700 });
    expect(sortByCapture([a, b]).map(c => c.id)).toEqual([9, 4]);
  });

  it("does not mutate the input list", () => {
    const clips = [row({ id: 1, effectiveCapturedAt: 1 }), row({ id: 2, effectiveCapturedAt: 2 })];
    sortByCapture(clips);
    expect(clips.map(c => c.id)).toEqual([1, 2]);
  });
});

describe("upsertByCapture", () => {
  it("inserts a new clip at its capture-ordered position, not at the top", () => {
    const clips = [
      row({ id: 1, path: "/v/new.mp4", effectiveCapturedAt: 900 }),
      row({ id: 2, path: "/v/old.mp4", effectiveCapturedAt: 100 }),
    ];
    // A recorder dropping a later, older-captured file must not jump the queue.
    const inserted = upsertByCapture(clips, row({ id: 3, path: "/v/mid.mp4", effectiveCapturedAt: 500 }));
    expect(inserted.map(c => c.path)).toEqual(["/v/new.mp4", "/v/mid.mp4", "/v/old.mp4"]);
  });

  it("replaces an existing row with the same path instead of duplicating it", () => {
    const clips = [row({ id: 1, path: "/v/a.mp4", effectiveCapturedAt: 100 })];
    const updated = upsertByCapture(clips, row({ id: 1, path: "/v/a.mp4", effectiveCapturedAt: 800 }));
    expect(updated).toHaveLength(1);
    expect(updated[0].effectiveCapturedAt).toBe(800);
  });
});

describe("captureLabel", () => {
  it("renders a date for clips that carry capture metadata", () => {
    const label = captureLabel(row({ capturedAt: 1_700_000_000 }));
    expect(label).not.toBeNull();
    expect(label).toMatch(/2023/);
  });

  it("shows nothing when the container has no capture metadata", () => {
    // An mtime fallback sorts the clip but must not be presented as a
    // recording date — it would be the copy date, which is misleading.
    expect(captureLabel(row({ capturedAt: null, effectiveCapturedAt: 1_700_000_000 }))).toBeNull();
  });
});
