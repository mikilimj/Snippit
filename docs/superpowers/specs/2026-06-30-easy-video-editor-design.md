# Easy Video Editor — Design Spec

**Date:** 2026-06-30
**Status:** Approved design, pre-implementation

## 1. Purpose

A minimalistic, cross-platform (Windows / macOS / Linux) desktop app for **fast trimming of
game clips**. It auto-imports recordings from the folders where capture tools (OBS, GPU Screen
Recorder, NVIDIA ShadowPlay/App, AMD recorders) save them, lets the user quickly trim and
lightly stitch clips, control the multiple audio tracks that come with a recording (plus overlay
their own music), and export — including one-click **compression to a target file size** for
sharing on Discord.

The guiding principle is **ease of use**: open a clip, cut it, export it, in seconds. Power
features are deliberately out of scope.

## 2. Core use cases

1. Record gameplay with any capture tool → clip appears in the app automatically.
2. Open a clip, set in/out points, export a trimmed version near-instantly.
3. Mute/adjust the separate audio tracks (game / mic / Discord) and/or add background music.
4. Stitch a few trimmed clips together into one video.
5. Export compressed to a chosen size (8/10/25/50/100 MB or custom) for Discord.

## 3. Tech stack

| Layer | Choice | Rationale |
|-------|--------|-----------|
| Shell | **Tauri 2** (Rust core + web UI) | Small native installers (~10–15 MB), low memory, easy FFmpeg sidecar bundling, true cross-platform. |
| Frontend | **React + TypeScript + Vite** | Mature ecosystem for interactive timeline/scrubber UI; custom minimalistic dark theme. |
| Media engine | **Bundled FFmpeg + FFprobe** (sidecar binaries) | Reads/writes every relevant codec/container; handles probe, thumbnails, proxies, mixing, two-pass compression. |
| Library index | **SQLite** (rusqlite in the Rust core) | Fast local index of clips + metadata. |
| Settings | **JSON file** in the app config dir | Watched folders, output folder, defaults. |
| Folder watching | **`notify` crate** (Rust) | Cross-platform filesystem events. |

### Process / module layout

```
Frontend (React/TS)                    Rust core (Tauri commands)
┌──────────────────────────┐           ┌───────────────────────────────┐
│ Library grid             │           │ watcher    (notify)            │
│ Editor (player+scrubber) │◄─ IPC ──►│ library    (SQLite index)      │
│ Audio panel              │  events   │ probe      (ffprobe wrapper)   │
│ Export dialog            │           │ media      (ffmpeg: thumb/proxy)│
│ Settings                 │           │ export     (ffmpeg job builder) │
└──────────────────────────┘           │ settings   (JSON store)        │
                                        └───────────────────────────────┘
```

Each Rust module has one responsibility and a typed command interface exposed to the frontend.
The frontend never builds FFmpeg command lines — it sends structured intents (trim ranges,
track volumes, export options) and the `export` module composes the FFmpeg invocation.

## 4. Components

### 4.1 Settings & first-run
- User selects **one or more watched folders** (recorder output dirs) and **one output/export
  folder**. Optional library "home" folder is just another watched folder.
- Settings persisted as JSON; editable anytime in a Settings screen.

### 4.2 Library & auto-import
- **Reference in place** — clips are indexed where they already live; never copied or moved.
- On startup the app scans watched folders; the `notify` watcher then reports new files live.
- **Write-completion detection:** a newly appeared file is only ingested once its size is stable
  across a short poll interval (recorders write progressively), avoiding half-written imports.
- For each clip: run `ffprobe` → store path, duration, resolution, container, video codec,
  **audio-track count + per-track metadata**, and a generated thumbnail (single frame via ffmpeg).
- Library grid: thumbnail, filename, duration, resolution, date; updates reactively as clips
  arrive. Supported containers: `.mp4 .mkv .mov .avi .webm .flv` (anything FFmpeg demuxes).
- Removing a clip from the library is index-only by default (does not delete the source file);
  deleting the source file is a separate explicit action.
- **Rename:** clip has a user-editable display name (defaults to filename), stored in the index.
  The underlying source file is never renamed on disk — only the library's display name changes.
  This keeps "reference in place" intact even if a recorder is still writing to that filename.
- **Favorites:** a clip can be toggled as favorite; a "Favorites" filter/view in the library
  surfaces them. Stored as a boolean column on the clip row.
- **Folders (collections):** user-created **virtual folders** (e.g. by game name) for sorting the
  library. A clip can belong to zero or more folders (many-to-many), since these are organizational
  tags in the SQLite index, not filesystem moves — consistent with reference-in-place storage.
  Folders shown as a sidebar/tree in the library view; drag-and-drop or right-click "Add to
  folder" assigns clips. An "Unsorted" view shows clips not yet assigned to any folder.

### 4.3 Editor
- **Player + scrubber** with draggable **in/out handles**, frame-step buttons, and hotkeys
  (`I` set in, `O` set out, `J/K/L` shuttle, space play/pause, arrows frame-step).
- **Light timeline:** append multiple trimmed clips into an ordered sequence. v1 = straight cuts
  only (no transitions). Reorder + remove segments.
- **Audio panel:** lists every detected audio track for the current clip with **mute + volume
  (0–200%)**; an **"Add music"** action overlays a user-chosen audio file as an additional track
  with its own volume and optional loop-to-length. Mixing is applied at export.

### 4.4 Preview strategy — **native first, proxy fallback**
1. Attempt to play the **original** file directly in the WebView (`<video>`). Most game clips are
   H.264 MP4 and play instantly with no extra work.
2. If the WebView reports the codec/container is unsupported (e.g. HEVC, AV1, some `.mkv`,
   multi-track audio), the Rust core generates a **lightweight 720p H.264 proxy** in the
   background and the player switches to it for smooth scrubbing.
3. Proxies are cached (keyed by source path + mtime) under the app cache dir; the **original is
   always used for the actual export**, never the proxy.
- **Audio preview limitation (accepted for v1):** the `<video>` element plays the default/first
  audio track only. Per-track volume changes are configured via the sliders and applied at
  export. A future enhancement may render a short audio preview of the mix.

### 4.5 Export engine
- **Quality/format choice per export:**
  - **Fast (lossless):** FFmpeg **stream-copy** (`-c copy`). Used only when the job is a pure trim
    of a single clip with no audio mixing/stitching → near-instant, no quality loss. Cut points
    snap to nearest keyframe (or optional precise mode via short re-encode of the head).
  - **Compatible MP4:** re-encode to **H.264 + AAC** for universal playback/sharing.
- **Automatic re-encode** whenever the job requires it: audio mixing/level changes, music overlay,
  or stitching multiple clips (FFmpeg `concat` + `amix`/`volume` filters).
- **Target-size compression:**
  - Presets: **8 / 10 / 25 / 50 / 100 MB** + **custom**.
  - Two-pass H.264 encode. `target_total_bits = target_MB × 8 × 1024 × 1024 × safety(≈0.97)`.
    `video_bitrate = target_total_bits / duration_seconds − audio_bitrate`.
  - Audio fixed at a sane bitrate (e.g. 128 kbps AAC, lower for tiny targets).
  - **Floor / warning:** if computed video bitrate falls below a usable threshold for the
    resolution/duration, warn the user and offer to downscale resolution or shorten the clip.
- **Progress:** FFmpeg progress parsed in Rust and streamed to the UI as a progress bar; jobs are
  cancellable.

### 4.6 Move to another device (library export/import)
- **"Export library" action** produces a single portable bundle (a zip): the SQLite index
  (clips, folders, favorites, renames, watched-folder list) plus the JSON settings, with all clip
  paths stored **relative to a chosen root folder** so they can be re-rooted on the new device.
- Two modes, chosen at export time:
  - **Settings + library only (default):** bundle contains just the database + settings —
    typically a few MB. The user moves/copies the actual clip files separately (external drive,
    cloud, network share, etc.).
  - **Include clip files:** the export job also copies every referenced source file into the
    bundle folder (alongside the database). This is a long-running, cancellable, resumable copy
    job with a progress bar — same job-runner pattern as export encoding. Bundle size can reach
    the full library size, so the user picks a destination with enough free space (pre-flight
    free-space check).
- **"Import library" action** on the new device: pick the bundle, then pick where its clip root
  now lives (or confirm "files already moved separately"). Import merges into (or replaces, user's
  choice) the existing library: re-links each clip path under the new root and **reuses the
  existing "missing clip" handling** (section 6) for anything not found, rather than introducing
  new error states. Folders/favorites/renames carry over as-is since they're metadata, not files.
- Watched folders and output folder paths are **not** assumed valid on the new device — import
  prompts the user to confirm or reselect them (same flow as first-run settings, section 4.1).

### 4.7 Relocate folder (local move, same PC)
A lighter, local counterpart to 4.6: move a watched folder or the output folder to a different
path or disk **on the same machine** — e.g. moving clips from a small C: drive to a larger D:
drive or external disk — without requiring 2× the folder's size in free space.

- **Trigger:** each configured folder in Settings has a **"Move to…"** action; user picks a new
  destination path.
- **Same-volume fast path:** if source and destination are on the same filesystem/volume, the
  whole folder is moved with a single atomic OS rename — instant, zero extra disk space.
- **Cross-volume path (different disk):** bytes must actually be copied, so the move is done
  **file-by-file, streamed**, never copying the whole tree before cleaning up the old one:
  1. Copy one file to the destination.
  2. Verify the copy (size match against source).
  3. Delete the source file only after verification succeeds.
  4. Update that clip's path in the library index immediately, so the index stays correct even if
     the move is interrupted partway.
  5. Repeat for the next file.

  Peak extra disk usage is bounded by the file currently in flight, not the whole folder — this
  is what avoids needing 2× space overall.
- **Resumable & crash-safe:** progress is tracked per file. If the app is interrupted (crash,
  force-quit, cancel), re-running "Move to…" with the same destination skips already-verified
  files and continues with the rest. A source file is never deleted before its copy is verified,
  so interruption never loses data — at worst a few files briefly exist in both places, which the
  next run cleans up.
- Recurses into subfolders (recorders sometimes use date-based subfolders) and mirrors the
  structure at the destination.
- **After completion:** the corresponding setting (watched-folder or output-folder path) updates
  to the new location and the folder watcher re-targets automatically. Thumbnail/proxy cache is
  not migrated — it's disposable, keyed by source path + mtime, and regenerates lazily (matches
  the existing cache-eviction philosophy in section 9).
- **Pre-flight check:** verifies the destination has enough free space for at least the single
  largest file before starting (not the whole folder), and warns if even that isn't available.
- **Progress UI:** reuses the same cancellable progress-bar component as export/encode jobs.
  Cancelling is safe — every file is either fully moved or untouched at the original location;
  there is nothing to roll back.

## 5. Data flow examples

**New recording → library:** watcher event → wait for stable size → `ffprobe` → insert row +
thumbnail → emit `clip-added` event → grid updates.

**Trim + Discord export:** user sets in/out, picks "Compatible MP4", target 10 MB → frontend sends
`{source, in, out, tracks[], output, target_mb:10}` → `export` module computes bitrate, builds
two-pass FFmpeg command → streams progress → emits `export-done` with output path.

## 6. Error handling

- Missing/again-unreadable source file → clip flagged "missing" in the library, not crashed.
- FFprobe/FFmpeg non-zero exit → captured stderr surfaced as a readable toast; job marked failed.
- Half-written file → never ingested until size stabilizes.
- Target size impossible at acceptable quality → pre-flight warning before encoding.
- Unsupported preview codec → silent fallback to proxy generation with a small "preparing
  preview…" indicator.
- Bundled FFmpeg missing/incompatible at runtime → clear startup error.

## 7. Testing strategy

- **Rust unit tests:** bitrate math (target-size → bitrate, including floor/warn cases), FFmpeg
  command-builder output (assert generated arg vectors for trim/copy, mix, concat, two-pass),
  write-completion/stable-size detector, settings (de)serialization.
- **Probe/parse tests:** ffprobe JSON → metadata struct, including multi-audio-track clips
  (using tiny generated fixture files).
- **Integration tests:** generate small synthetic clips with FFmpeg, then exercise real
  trim/mix/concat/compress paths and assert output duration, track count, and that file size is
  within tolerance of the requested target.
- **Frontend:** component tests for the trim scrubber (in/out invariants: in < out, clamping) and
  export dialog (preset → option mapping). Manual smoke test of the full flow per OS.
- **Migration tests:** export → import round-trip preserves folders/favorites/renames; path
  re-rooting under a new root; import correctly flags clips as missing when files weren't moved.
- **Relocate tests:** same-volume rename path; cross-volume file-by-file move preserves all bytes
  (size/content match) and deletes sources only after verification; interrupting mid-move and
  resuming results in a complete, duplicate-free move with no data loss; library paths stay
  correct at every interruption point.

## 8. v1 scope & non-goals

**In scope (v1):**
- Auto-import via folder watching (reference in place)
- Library grid with metadata + thumbnails
- Clip rename, favorites, and virtual folders (e.g. organize by game)
- Trim with in/out + hotkeys
- Light timeline: stitch multiple trimmed clips (straight cuts)
- Multi-track audio mute/volume + add-music overlay (applied at export)
- Native-first preview with proxy fallback
- Export: Fast lossless vs Compatible MP4; target-size compression with presets + custom
- Library export/import for moving to another device (settings+library bundle, optional clip-file copy)
- Relocate a watched/output folder locally to a different path or disk, without 2× disk space
- Cross-platform builds (Win/Mac/Linux)

**Out of scope (later):**
- Transitions, video effects/filters, color grading
- Text/overlays/captions, image overlays
- Real-time mixed-audio preview
- GIF/other export formats
- Deep per-recorder API/hotkey integration (folder watching covers the need)
- Cloud sync / upload integrations

## 9. Open considerations (tracked, not blocking)

- Precise (frame-accurate) lossless trim vs keyframe-snapped: v1 snaps to keyframe for "Fast";
  precise mode may re-encode only the leading GOP.
- Proxy cache eviction policy (size cap / LRU) — start simple, cap by total size.
- Per-OS FFmpeg binary sourcing/licensing for bundling — confirm build pipeline during setup.
