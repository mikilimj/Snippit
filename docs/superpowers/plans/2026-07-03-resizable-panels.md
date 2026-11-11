# Resizable Sidebar & Video Player Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the library sidebar width and the video player's height drag-resizable, with the resized sizes persisted across app restarts.

**Architecture:** A pure clamp/parse module (`src/resizable.ts`) backs a small custom React hook (`src/useResizable.ts`) that tracks pointer-drag state via React's Pointer Capture API, computes the new size from the drag delta, and persists to `localStorage` on release. The hook is used twice in `App.tsx` — once per resizable panel — with a thin CSS-styled `<div>` drag handle wired to each.

**Tech Stack:** React 18 + TypeScript, Vitest for unit tests, plain CSS (no new npm dependency).

## Global Constraints

- No new npm dependency — implement the drag/clamp/persistence logic by hand (per `docs/superpowers/specs/2026-07-03-resizable-panels-design.md`).
- TS tests are pure-logic unit tests only — no React component/hook tests (per `CLAUDE.md` testing conventions). `src/resizable.ts` gets a colocated `resizable.test.ts`; `src/useResizable.ts` does not.
- Sidebar: default `224` (px), min `184`, max `480`, `localStorage` key `clipsnip:sidebarWidth`.
- Video player: default `50` (vh), min `25`, max `85`, `localStorage` key `clipsnip:videoMaxHeightVh`.
- `npm run build` (tsc + vite build) is the type-check gate — run it after every task that touches `.ts`/`.tsx`.
- Styling stays plain CSS matching `src/App.css`'s existing token usage (`var(--accent-border)`, `var(--border)`, etc.) and section-comment convention (`/* ─── Section ─── */`).

---

### Task 1: Pure resize math (`resizable.ts`)

**Files:**
- Create: `src/resizable.ts`
- Create: `src/resizable.test.ts`

**Interfaces:**
- Produces: `clamp(value: number, min: number, max: number): number` and
  `parseStoredSize(raw: string | null, fallback: number, min: number, max: number): number`
  — both consumed by `useResizable.ts` in Task 2. `parseStoredSize` takes the *raw*
  string already read from `localStorage` (or `null`) — it does no I/O itself, which
  is what keeps it unit-testable under Vitest's default `node` environment (no
  `jsdom`/`localStorage` global is installed in this project).

- [x] **Step 1: Write the failing test file**

Create `src/resizable.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { clamp, parseStoredSize } from "./resizable";

describe("clamp", () => {
  it("passes through a value within range", () => {
    expect(clamp(50, 10, 100)).toBe(50);
  });
  it("clamps a value below min", () => {
    expect(clamp(5, 10, 100)).toBe(10);
  });
  it("clamps a value above max", () => {
    expect(clamp(500, 10, 100)).toBe(100);
  });
});

describe("parseStoredSize", () => {
  it("uses the parsed value when it's a valid in-range number", () => {
    expect(parseStoredSize("300", 224, 184, 480)).toBe(300);
  });
  it("clamps a parsed value below min", () => {
    expect(parseStoredSize("50", 224, 184, 480)).toBe(184);
  });
  it("clamps a parsed value above max", () => {
    expect(parseStoredSize("9999", 224, 184, 480)).toBe(480);
  });
  it("falls back to a clamped default when raw is null", () => {
    expect(parseStoredSize(null, 224, 184, 480)).toBe(224);
  });
  it("falls back to a clamped default when raw is non-numeric", () => {
    expect(parseStoredSize("not-a-number", 224, 184, 480)).toBe(224);
  });
  it("clamps the fallback itself when the fallback is out of range", () => {
    expect(parseStoredSize(null, 999, 184, 480)).toBe(480);
  });
});
```

- [x] **Step 2: Run the tests to verify they fail**

Run: `npx vitest run src/resizable.test.ts`
Expected: FAIL — `Cannot find module './resizable'` (or similar resolution error), since `src/resizable.ts` doesn't exist yet.

- [x] **Step 3: Write the implementation**

Create `src/resizable.ts`:

```ts
export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

// Parses a raw localStorage value into a clamped size. Falls back to a
// (clamped) default when the value is missing, non-numeric, or NaN.
export function parseStoredSize(
  raw: string | null,
  fallback: number,
  min: number,
  max: number
): number {
  const parsed = raw === null ? NaN : Number(raw);
  if (Number.isNaN(parsed)) return clamp(fallback, min, max);
  return clamp(parsed, min, max);
}
```

- [x] **Step 4: Run the tests to verify they pass**

Run: `npx vitest run src/resizable.test.ts`
Expected: PASS — all 9 assertions green.

- [x] **Step 5: Commit**

```bash
git add src/resizable.ts src/resizable.test.ts
git commit -m "feat: add pure clamp/parse logic for resizable panels"
```

---

### Task 2: `useResizable` drag hook

**Files:**
- Create: `src/useResizable.ts`

**Interfaces:**
- Consumes: `clamp`, `parseStoredSize` from `./resizable` (Task 1).
- Produces:
  ```ts
  interface UseResizableOptions {
    axis: "x" | "y";
    defaultSize: number;
    min: number;
    max: number;
    storageKey: string;
  }
  interface UseResizableResult {
    size: number;
    onPointerDown: (e: React.PointerEvent<HTMLElement>) => void;
    onPointerMove: (e: React.PointerEvent<HTMLElement>) => void;
    onPointerUp: (e: React.PointerEvent<HTMLElement>) => void;
  }
  function useResizable(options: UseResizableOptions): UseResizableResult;
  ```
  Consumed by `App.tsx` in Tasks 3 and 4: `size` is bound to an inline style
  (`width` in px for `axis: "x"`, `maxHeight` in `${size}vh` for `axis: "y"`), and
  the three `onPointer*` handlers are spread onto a `<div className="resize-handle-x">`
  or `<div className="resize-handle-y">` drag handle. `onPointerUp` must also be
  passed as the handle's `onPointerCancel` (covers a drag interrupted mid-gesture).

- [x] **Step 1: Write the hook**

Create `src/useResizable.ts`:

```ts
import { useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { clamp, parseStoredSize } from "./resizable";

export interface UseResizableOptions {
  axis: "x" | "y";
  defaultSize: number;
  min: number;
  max: number;
  storageKey: string;
}

export interface UseResizableResult {
  size: number;
  onPointerDown: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerMove: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerUp: (e: ReactPointerEvent<HTMLElement>) => void;
}

interface DragState {
  startPos: number;
  startSize: number;
}

// Drives a single-axis drag handle: "x" resizes by horizontal pixel delta,
// "y" resizes by vertical delta converted to vh (relative to window height).
// Reads the initial size from localStorage; persists the final size on
// pointer-up. Uses Pointer Capture so the drag tracks correctly even when
// the pointer moves off the handle element mid-gesture.
export function useResizable({
  axis,
  defaultSize,
  min,
  max,
  storageKey,
}: UseResizableOptions): UseResizableResult {
  const [size, setSize] = useState(() =>
    parseStoredSize(localStorage.getItem(storageKey), defaultSize, min, max)
  );
  const dragState = useRef<DragState | null>(null);

  function sizeFromEvent(e: ReactPointerEvent<HTMLElement>): number {
    const drag = dragState.current;
    if (!drag) return size;
    const pos = axis === "x" ? e.clientX : e.clientY;
    const deltaPx = pos - drag.startPos;
    const delta = axis === "x" ? deltaPx : (deltaPx / window.innerHeight) * 100;
    return clamp(drag.startSize + delta, min, max);
  }

  function onPointerDown(e: ReactPointerEvent<HTMLElement>) {
    e.currentTarget.setPointerCapture(e.pointerId);
    dragState.current = {
      startPos: axis === "x" ? e.clientX : e.clientY,
      startSize: size,
    };
    document.body.style.cursor = axis === "x" ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";
  }

  function onPointerMove(e: ReactPointerEvent<HTMLElement>) {
    if (!dragState.current) return;
    setSize(sizeFromEvent(e));
  }

  function onPointerUp(e: ReactPointerEvent<HTMLElement>) {
    if (!dragState.current) return;
    const finalSize = sizeFromEvent(e);
    setSize(finalSize);
    localStorage.setItem(storageKey, String(finalSize));
    dragState.current = null;
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  }

  return { size, onPointerDown, onPointerMove, onPointerUp };
}
```

- [x] **Step 2: Type-check**

Run: `npm run build`
Expected: succeeds with no TypeScript errors (this file isn't wired into `App.tsx` yet, so it's checked in isolation as an unused-but-valid module).

- [x] **Step 3: Commit**

```bash
git add src/useResizable.ts
git commit -m "feat: add useResizable drag hook"
```

---

### Task 3: Sidebar resize (CSS + wiring)

> **Superseded:** the sidebar this task made resizable was removed by the grid-only
> shell redesign (`docs/design/shell-redesign-brief.md`, phase 3). `useResizable` and
> the pure resize math from Tasks 1–2 are still in use — for the video player height
> (`snippit:videoMaxHeightVh` in `src/App.tsx`) — so only this task's wiring is gone.

**Files:**
- Modify: `src/App.css:11-20` (`.library-panel` block)
- Modify: `src/App.tsx:5` (imports), `src/App.tsx:51` (state block), `src/App.tsx:652`, `src/App.tsx:846`

**Interfaces:**
- Consumes: `useResizable` from `./useResizable` (Task 2).

- [x] **Step 1: Drop the fixed width from `.library-panel` and add the handle style**

In `src/App.css`, the current block (lines 11-20) is:

```css
.library-panel {
  width: 224px;
  min-width: 184px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-solid);
  border-right: 1px solid var(--border);
  overflow: hidden;
}
```

Replace it with (drops the fixed `width`, keeps `min-width` as a safety floor since
JS now drives the actual width):

```css
.library-panel {
  min-width: 184px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  background: var(--panel-solid);
  border-right: 1px solid var(--border);
  overflow: hidden;
}

/* ─── Resize handles ────────────────────────────────────────────────────────── */
.resize-handle-x {
  width: 6px;
  flex-shrink: 0;
  cursor: col-resize;
  background: transparent;
  transition: background 0.15s ease;
  touch-action: none;
}

.resize-handle-x:hover {
  background: var(--accent-border);
}
```

- [x] **Step 2: Import the hook in `App.tsx`**

In `src/App.tsx`, after the existing local import on line 12:

```tsx
import { Timeline } from "./Timeline";
```

add:

```tsx
import { Timeline } from "./Timeline";
import { useResizable } from "./useResizable";
```

- [x] **Step 3: Call the hook**

In `src/App.tsx`, after line 51:

```tsx
  const [settingsOpen, setSettingsOpen] = useState(false);
```

add:

```tsx
  const [settingsOpen, setSettingsOpen] = useState(false);
  const sidebar = useResizable({
    axis: "x",
    defaultSize: 224,
    min: 184,
    max: 480,
    storageKey: "clipsnip:sidebarWidth",
  });
```

- [x] **Step 4: Bind the width and add the handle**

In `src/App.tsx` line 652, change:

```tsx
      <aside className="library-panel">
```

to:

```tsx
      <aside className="library-panel" style={{ width: sidebar.size }}>
```

Then, at line 846 (the `</aside>` closing tag, immediately followed by the blank
line and `{/* Editor area */}` comment on line 848), insert a handle between the
sidebar and the editor panel:

```tsx
      </aside>

      <div
        className="resize-handle-x"
        onPointerDown={sidebar.onPointerDown}
        onPointerMove={sidebar.onPointerMove}
        onPointerUp={sidebar.onPointerUp}
        onPointerCancel={sidebar.onPointerUp}
      />

      {/* Editor area */}
      <main className="editor-panel">
```

- [x] **Step 5: Type-check**

Run: `npm run build`
Expected: succeeds with no TypeScript errors.

- [x] **Step 6: Manual verification**

Run: `npm run tauri dev`
- Confirm the sidebar renders at 224px as before.
- Hover the thin strip immediately right of the sidebar — cursor should become
  `col-resize` and the strip should highlight.
- Drag it left and right — sidebar width should track the drag, clamped between
  184px and 480px.
- Reload the app (or restart `tauri dev`) — the sidebar should reopen at the last
  dragged width (confirms the `localStorage` round-trip).

- [x] **Step 7: Commit**

```bash
git add src/App.css src/App.tsx
git commit -m "feat: make library sidebar width drag-resizable"
```

---

### Task 4: Video player resize (CSS + wiring)

**Files:**
- Modify: `src/App.css:367-375` (`.video-player` block)
- Modify: `src/App.tsx:878-891` (the `<video>` element), `src/App.tsx:891` (insert handle after)

**Interfaces:**
- Consumes: `sidebar`-style call to `useResizable` from `./useResizable` (Task 2); the
  hook is called a second time in this task with different options — both calls
  coexist in the same component.

- [x] **Step 1: Drop the fixed max-height from `.video-player` and add the handle style**

In `src/App.css`, the current block (lines 367-375) is:

```css
/* ─── Video player ──────────────────────────────────────────────────────────── */
.video-player {
  width: 100%;
  max-height: 50vh;
  background: #000;
  border-radius: var(--radius-md);
  border: 1px solid var(--border);
  flex-shrink: 0;
}
```

Replace it with (drops the fixed `max-height`; JS now drives it via inline style):

```css
/* ─── Video player ──────────────────────────────────────────────────────────── */
.video-player {
  width: 100%;
  background: #000;
  border-radius: var(--radius-md);
  border: 1px solid var(--border);
  flex-shrink: 0;
}

.resize-handle-y {
  height: 6px;
  flex-shrink: 0;
  border-radius: 3px;
  cursor: row-resize;
  background: transparent;
  transition: background 0.15s ease;
  touch-action: none;
}

.resize-handle-y:hover {
  background: var(--accent-border);
}
```

- [x] **Step 2: Call the hook a second time**

In `src/App.tsx`, immediately after the `sidebar` hook call added in Task 3 Step 3:

```tsx
  const sidebar = useResizable({
    axis: "x",
    defaultSize: 224,
    min: 184,
    max: 480,
    storageKey: "clipsnip:sidebarWidth",
  });
  const videoHeight = useResizable({
    axis: "y",
    defaultSize: 50,
    min: 25,
    max: 85,
    storageKey: "clipsnip:videoMaxHeightVh",
  });
```

- [x] **Step 3: Bind the height and add the handle**

In `src/App.tsx`, the current `<video>` element (lines 878-891) is:

```tsx
            <video
              ref={videoRef}
              key={previewPath(path, proxyPath)}
              src={mediaPort ? `http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(previewPath(path, proxyPath))}` : undefined}
              controls
              muted={audioReady}
              className="video-player"
              onTimeUpdate={e => setCurrentTime(e.currentTarget.currentTime)}
              onPlay={onVideoPlay}
              onPause={onVideoPause}
              onSeeked={onVideoSeeked}
              onVolumeChange={onVideoVolumeChange}
              onError={handleVideoError}
            />

            {proxyStatus && (
```

Replace it with (adds `style` to the `<video>`, and inserts the drag handle right
after it):

```tsx
            <video
              ref={videoRef}
              key={previewPath(path, proxyPath)}
              src={mediaPort ? `http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(previewPath(path, proxyPath))}` : undefined}
              controls
              muted={audioReady}
              className="video-player"
              style={{ maxHeight: `${videoHeight.size}vh` }}
              onTimeUpdate={e => setCurrentTime(e.currentTarget.currentTime)}
              onPlay={onVideoPlay}
              onPause={onVideoPause}
              onSeeked={onVideoSeeked}
              onVolumeChange={onVideoVolumeChange}
              onError={handleVideoError}
            />

            <div
              className="resize-handle-y"
              onPointerDown={videoHeight.onPointerDown}
              onPointerMove={videoHeight.onPointerMove}
              onPointerUp={videoHeight.onPointerUp}
              onPointerCancel={videoHeight.onPointerUp}
            />

            {proxyStatus && (
```

- [x] **Step 4: Type-check**

Run: `npm run build`
Expected: succeeds with no TypeScript errors.

- [x] **Step 5: Manual verification**

Run: `npm run tauri dev`
- Open a clip so the video player is visible.
- Confirm the video still renders at its normal size (50vh ceiling, same as before).
- Hover the thin strip directly below the video — cursor should become `row-resize`
  and the strip should highlight.
- Drag it up and down — the video's max-height should track the drag, clamped
  between 25vh and 85vh, video aspect ratio preserved (no distortion).
- Reload the app — the video player should reopen at the last dragged height
  (confirms the `localStorage` round-trip).

- [x] **Step 6: Commit**

```bash
git add src/App.css src/App.tsx
git commit -m "feat: make video player height drag-resizable"
```

---

### Task 5: Docs cleanup

**Files:**
- Modify: `CLAUDE.md`

**Interfaces:**
- None — documentation only.

- [x] **Step 1: Remove the two completed to-do items**

In `CLAUDE.md`, the current `# To-Do` section is:

```markdown
# To-Do

- [ ] Make left sidebar/banner width resizable (drag handle) — `.library-panel` in
      `src/App.css:11-20` is a fixed 224px width (`min-width: 184px`); no resizable-panel
      library is installed.
- [ ] Make video player height resizable — `.video-player` in `src/App.css:368-375` is
      `width: 100%; max-height: 50vh` inside `.editor-panel` (`src/App.css:283-292`).
```

Both items are now done — remove the entire `# To-Do` section (delete those 8 lines,
including the `# To-Do` heading and the trailing blank line before it, so the file
ends cleanly after the `## Known constraints` section).

- [x] **Step 2: Add the new pure-logic module to the architecture doc**

In `CLAUDE.md`, the current line (in the "Frontend layout" bullet list) is:

```markdown
- `trim.ts`, `sequence.ts`, `exportOptions.ts`, `libraryView.ts`, `preview.ts`,
  `wizardValidation.ts` — pure logic modules, each with a co-located `*.test.ts`
```

Change it to:

```markdown
- `trim.ts`, `sequence.ts`, `exportOptions.ts`, `libraryView.ts`, `preview.ts`,
  `wizardValidation.ts`, `resizable.ts` — pure logic modules, each with a
  co-located `*.test.ts`
```

- [x] **Step 3: Verify**

Run: `npx vitest run && npm run build`
Expected: both succeed (this task only touches Markdown, so this just confirms
nothing else regressed).

- [x] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: clear completed resizable-panel to-dos"
```
