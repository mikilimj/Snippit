# ClipSnip — First-Run Setup Wizard — Design Spec

**Date:** 2026-07-01
**Status:** Approved design, pre-implementation
**Relates to:** [2026-06-30-easy-video-editor-design.md](2026-06-30-easy-video-editor-design.md) §4.1 (Settings & first-run)

## 1. Purpose

Before ClipSnip can build out its library (auto-import, watching, SQLite index — none of
which exist yet), it needs somewhere on disk to keep its own data: settings, a clip
folder, exports, cache. This spec covers a first-run wizard that lets the user choose
that root directory, pick how clips will initially get into it, and persists the choice —
nothing more. It does not implement folder watching, a copy-import job, or the library
grid; those are separate future specs.

## 2. Scope

**In scope:**
- Animated first-run wizard UI (3 steps): welcome, root-directory picker, ingestion-mode
  picker.
- Root directory validation, subfolder creation (`Clips/`, `Exports/`, `Cache/`).
- Persisting the setup as two files: a small OS-level pointer file and a root-level
  `settings.json`.
- Gating app launch: show the wizard when unconfigured, the existing editor when
  configured.
- If "Import (copy)" mode is chosen, a **synchronous best-effort file copy** from the
  chosen source folder into `Clips/` (no progress bar, no resumability — see §8 for why
  this is intentionally minimal).

**Out of scope (future specs):**
- Live folder watching (`notify` crate), write-completion detection.
- SQLite clip index, library grid, thumbnails, favorites, virtual folders.
- Changing the root directory after first run (Settings screen).
- Cancellable/resumable/progress-tracked import (that pattern is reserved for the
  heavier "relocate folder" feature in the original design spec §4.7).

## 3. Deliberate divergence from the original design spec

The original design spec (§4.2) establishes "reference in place — never copied or
moved" as a storage principle for watched folders. This spec introduces an **Import
(copy)** ingestion mode that intentionally copies files into `<root>/Clips/`, breaking
that principle for files brought in this way. **Watch folders** mode still honors
reference-in-place. Both modes coexist by user choice at setup time. This is a
conscious scope expansion made during this design's brainstorming, not an oversight —
flagging it explicitly so a future library spec doesn't silently contradict either
principle.

## 4. Architecture & data flow

### 4.1 Two persisted files

- **Bootstrap pointer file** — `<os-config-dir>/clipsnip/root.json` (via Tauri's
  `app_config_dir()`), written once on wizard completion:
  ```json
  { "root_path": "/absolute/path/chosen/by/user" }
  ```
  This is the *only* thing ClipSnip needs to find before it knows anything else. If
  it's missing or unreadable, the app has never been set up (or the root moved) and the
  wizard runs again.

- **Root-level settings** — `<root_path>/settings.json`, written once on wizard
  completion:
  ```json
  {
    "ingestion_mode": "import" | "empty" | "watch",
    "source_folder": "/absolute/path" | null,
    "created_at": "2026-07-01T12:00:00Z"
  }
  ```
  `source_folder` is the folder the user picked to import from or watch; `null` for
  `"empty"` mode.

### 4.2 Subfolders

On completion, create (idempotently — `create_dir_all`, no error if already present):
- `<root>/Clips/`
- `<root>/Exports/`
- `<root>/Cache/`

### 4.3 New Rust commands (in a new `src-tauri/src/setup.rs`, `pub mod setup` in `lib.rs`)

- `get_app_root() -> Option<String>` — reads the pointer file; returns `None` if it
  doesn't exist or fails to parse (treated the same as "never set up").
- `complete_setup(root_path: String, ingestion_mode: String, source_folder: Option<String>) -> Result<SetupResult, String>`:
  1. Validate `root_path` is an absolute path and is writable (attempt to create it if
     it doesn't exist; error if creation/write-check fails).
  2. Create the three subfolders.
  3. If `ingestion_mode == "import"`, copy files directly inside `source_folder`
     (non-recursive, top-level files only) whose extension matches the video
     containers the original design spec already recognizes (`.mp4 .mkv .mov .avi
     .webm .flv`, case-insensitive) into `Clips/`; non-matching files are silently
     skipped (not an error, not a warning — they're simply not clips). Per-file *copy*
     failures among matching files are collected and skipped, not fatal (see §7).
  4. Write `settings.json` inside `root_path`.
  5. Write the pointer file to the OS config dir.
  6. Return `SetupResult { root_path, warnings: Vec<String> }` (warnings = skipped
     files, non-empty-dir notice, etc.) so the frontend can show a toast.

No new Tauri capabilities are required: directory/file I/O happens via `std::fs` in
Rust commands (already covered by `core:default`), and the folder picker uses the
existing `dialog:default` permission.

### 4.4 Frontend gating (`App.tsx`)

On mount, `App` calls `get_app_root()` before rendering its existing editor UI:
- `None` → render `<FirstRunWizard onComplete={...} />` instead of the editor.
- `Some(root)` → render the editor as it works today (unchanged).

`FirstRunWizard`'s `onComplete` callback receives the `SetupResult`, and `App` then
switches its render branch to the editor — no full page reload needed. This is the only
change to `App.tsx`'s existing behavior; the editor itself (trim, playback, export) is
untouched by this spec.

## 5. Wizard UI

Three steps, single React component `FirstRunWizard.tsx` with local step state
(no routing library needed):

1. **Welcome** — animated gradient background full-bleed, app name/short tagline, "Get
   Started" button.
2. **Root directory** — same background, centered glass panel: title, description,
   a picker row (chosen path display + "Choose folder" button opening the native
   dialog via `@tauri-apps/plugin-dialog`), Back/Continue. Continue is disabled until a
   path is chosen. If the chosen folder is non-empty, show an inline warning below the
   picker row but do not block Continue (§7).
3. **Ingestion mode** — same background, panel with three radio-style options (Import
   copy / Start empty / Watch folders). Choosing Import or Watch reveals a second
   folder-picker row (single folder, not multi) for the source. "Finish" calls
   `complete_setup` and shows a spinner while it runs (it's synchronous file I/O, so
   this could take a moment for Import mode with many/large files — no progress bar in
   this spec, see §2).

**Visual design** (validated with the user via mockups): dark "Midnight" aurora —
near-black background (`#06060a`) with three blurred, slowly-drifting colored blobs
(indigo `#312e81`, teal `#164e63`, magenta `#4a044e`, opacities .4–.5), animating via
CSS `@keyframes` transform + `filter: blur(45px)`. The wizard panel is a centered glass
card (`rgba(15,15,22,.55)` background, `backdrop-filter: blur(18px)`, subtle border) at
~420px wide.

**WebKitGTK verification required:** the mockup above was validated in a standard
browser, not WebKitGTK (the actual runtime per this project's existing memory notes on
WebKitGTK-specific rendering quirks). During implementation, run `npm run tauri dev` and
visually confirm the blur/glass effects render acceptably. If `backdrop-filter` or the
animated `filter: blur()` blobs perform poorly or don't render, fall back to a solid
`rgba(15,15,22,.92)` panel with no backdrop-filter, and/or reduce blob blur radius —
functionality is unaffected either way, this only affects visual polish.

## 6. Ingestion modes (semantics captured now, execution mostly deferred)

- **Import (copy):** copies top-level files from the chosen source folder into
  `Clips/` during `complete_setup` (see §4.3 step 3). This is the only ingestion mode
  with real file-moving behavior in this spec.
- **Start empty:** no-op beyond recording the mode; user adds clips later via the
  existing "open clip" flow.
- **Watch folders:** records `source_folder` in `settings.json` only. No `notify`
  watcher is wired up in this spec — that's the deferred future library spec. The
  folder is just remembered for when that feature exists.

## 7. Error handling

- **Root path not writable / can't be created** → inline error in the wizard, Continue
  stays disabled, user must pick a different path.
- **Root path non-empty** → inline warning, non-blocking (user can proceed).
- **Import copy: individual file fails** (permissions, disk full mid-copy, etc.) →
  skip that file, collect its name in `warnings`, continue with the rest. After
  `complete_setup` returns, if `warnings` is non-empty, show a toast summarizing skipped
  files once the user lands in the editor.
- **Import copy: entire source folder unreadable** → this fails `complete_setup`
  itself (`Result::Err`), wizard shows the error and lets the user go back and pick a
  different source or switch ingestion mode.
- **Pointer file present but root path no longer exists** (moved/deleted externally) →
  out of scope for this spec; `get_app_root()` still returns the stored path as-is, and
  the editor's existing "missing file" handling (or lack thereof) applies. A future spec
  should detect this and re-offer the wizard.

## 8. Why the import copy is minimal (no progress bar)

The original design spec reserves progress bars, cancellation, and resumability for
long-running jobs like export encoding (§4.5) and folder relocation (§4.7). First-run
import is a one-time, synchronous, best-effort copy of whatever's in one folder — adding
a job-runner/progress-UI for it now would be building infrastructure ahead of the
need (YAGNI). If it proves painful in practice (large libraries, slow drives), a later
spec can upgrade it to the same cancellable job pattern used elsewhere.

## 9. Testing strategy

- **Rust unit tests** (`setup.rs`): `complete_setup` creates the three subfolders;
  writes valid `settings.json` matching input; import mode copies only recognized video
  extensions and reports per-file failures in `warnings` without aborting; rejects a
  root path that can't be written to; pointer file round-trips through `get_app_root`.
- **Frontend:** component test for `FirstRunWizard` step navigation (Back/Continue
  gating, e.g. Continue disabled without a chosen root path); manual smoke test of the
  full 3-step flow in `npm run tauri dev`, including the WebKitGTK visual check from §5.
- **Integration:** a test that runs `complete_setup` against a temp directory tree with
  a few real files, then asserts `Clips/` contains them and `settings.json` /
  pointer-file content matches.

## 10. Open considerations (tracked, not blocking)

- What happens if the wizard is interrupted partway (app closed after root dir chosen
  but before Finish)? Currently: pointer file is only written at the very end, so a
  restart just re-runs the wizard from step 1. Any subfolders created in a
  previously-attempted root are harmless leftovers (idempotent creation). Acceptable
  for v1; not actively handled beyond that.
- Root directory relocation after first run is explicitly deferred to a future
  Settings-screen spec (ties into the original design's §4.7 "Relocate folder").
