# Resizable sidebar & video player — design

Date: 2026-07-03

## Problem

Two items from CLAUDE.md's To-Do list:

- Left sidebar/banner width is fixed (`.library-panel` in `src/App.css:11-20`, 224px,
  `min-width: 184px`). No resizable-panel library is installed.
- Video player height is fixed (`.video-player` in `src/App.css:368-375`,
  `width: 100%; max-height: 50vh`). Not resizable.

Both should become drag-resizable, with the resized size persisted across app
restarts.

## Approach

Custom lightweight React hook + pure clamp/persistence logic. No new dependency.

Alternatives considered:
- **CSS-only `resize:` property** — native browser resize handles render
  inconsistently across platforms (this app targets Linux/WebKitGTK primarily) and
  don't give us a hook for persistence.
- **A resizable-panel library** (`re-resizable`, `allotment`, etc.) — more features
  than two single-axis drag handles need; conflicts with this codebase's
  minimal-dependency style (plain CSS, no large UI libs).

The custom hook keeps this in line with the existing pattern of small pure-logic
modules with colocated tests (`trim.ts`, `sequence.ts`, etc.) — the clamp/parse math
is pure and tested; the hook itself (React-specific glue) is not, matching this
repo's "no React component/hook tests" convention.

## Components

### `src/resizable.ts` (new — pure logic, tested)

- `clamp(value: number, min: number, max: number): number`
- `readStoredSize(key: string, fallback: number, min: number, max: number): number`
  — reads `localStorage[key]`, parses as a float, clamps to `[min, max]`; falls back
  to `fallback` (clamped) if the value is missing, non-numeric, or out of a sane
  range.

Colocated `src/resizable.test.ts` covers: value within range passes through; value
below min/above max clamps; missing/corrupt/non-numeric stored value falls back to
default.

### `src/useResizable.ts` (new — hook, untested per project convention)

```ts
useResizable({ axis: "x" | "y", defaultSize, min, max, storageKey }): {
  size: number;
  onPointerDown: (e: React.PointerEvent) => void;
}
```

- Initializes `size` from `readStoredSize(storageKey, defaultSize, min, max)`.
- `onPointerDown` on the handle element: calls `setPointerCapture`, then tracks
  `pointermove` / `pointerup` on `window` for the duration of the drag.
- On each `pointermove`, computes the new size from the accumulated pointer delta
  (`axis: "x"` → horizontal delta in px; `axis: "y"` → vertical delta converted to
  `vh` via `window.innerHeight`), clamps to `[min, max]`, updates state.
- Toggles a `resizing` class on `document.body` for the duration of the drag
  (`cursor: col-resize` / `row-resize` + `user-select: none`, so text doesn't get
  selected while dragging).
- On `pointerup`, writes the final size to `localStorage[storageKey]`.

### Sidebar wiring (`App.tsx`, `App.css`)

- `const sidebar = useResizable({ axis: "x", defaultSize: 224, min: 184, max: 480, storageKey: "clipsnip:sidebarWidth" })`
- `<aside className="library-panel" style={{ width: sidebar.size }}>` — CSS drops
  the fixed `width: 224px` but keeps `min-width: 184px` as a safety floor.
- A new handle element, last child of `.library-panel`, absolutely positioned on the
  right edge (`~6px` wide, `cursor: col-resize`, subtle hover highlight via
  `var(--border)` / `var(--accent-border)`), wired to `sidebar.onPointerDown`.

### Video player wiring (`App.tsx`, `App.css`)

- `const videoHeight = useResizable({ axis: "y", defaultSize: 50, min: 25, max: 85, storageKey: "clipsnip:videoMaxHeightVh" })`
- `<video ... style={{ maxHeight: \`${videoHeight.size}vh\` }} className="video-player" />`
  — CSS drops the fixed `max-height: 50vh`; width/aspect-ratio behavior unchanged.
- A new handle element directly below the video (`~6px` tall, `cursor: row-resize`,
  same hover treatment), wired to `videoHeight.onPointerDown`.

## Ranges & persistence

| Panel         | Default | Min  | Max  | Unit | localStorage key              |
|---------------|---------|------|------|------|--------------------------------|
| Sidebar width | 224     | 184  | 480  | px   | `clipsnip:sidebarWidth`        |
| Video height  | 50      | 25   | 85   | vh   | `clipsnip:videoMaxHeightVh`    |

## Known limitation

The sidebar's 480px ceiling isn't viewport-aware — on a very narrow window a maxed
sidebar could crowd the editor panel. `.editor-panel` already has `min-width: 0`, so
this can't break layout (no overflow/crash), just look tight. Not adding
viewport-relative clamping — out of scope unless it becomes an actual problem.

## Testing

- `src/resizable.test.ts`: unit tests for `clamp` and `readStoredSize` (in-range,
  below-min, above-max, corrupt/missing storage value).
- No new Rust changes — this is frontend-only.
- Manual check in `npm run tauri dev`: drag both handles to their min/max, reload
  the app, confirm sizes persisted.
