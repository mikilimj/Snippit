import { describe, it, expect } from "vitest";
import {
  newClipBelongsInView,
  applyFavorite,
  applyRename,
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
