# AGENTS.md

Guidance for AI coding agents working in this repository. Applies to every agent
that reads `AGENTS.md`; `CLAUDE.md` is a symlink to this file, so edit this one.

## Project overview

Snippit is a Tauri 2 desktop app for trimming and exporting video clips: a
React/TypeScript frontend (`src/`) in a native window, backed by a Rust core
(`src-tauri/src/`). `ffmpeg`/`ffprobe` ship as sidecar binaries in
`src-tauri/binaries/` — no system install required. Styling is plain CSS
(`App.css`, no Tailwind/CSS-in-JS); layout is CSS flexbox, no grid.

## Build and test commands

Run from the repository root unless noted.

```bash
npm install                 # install JS deps
npm run tauri dev           # dev mode: hot reload for React + Rust
npm run tauri build         # production build (installer under src-tauri/target/release/bundle/)
npm run build               # tsc + vite build only (frontend, no Tauri bundling)

npx vitest run                    # all TS unit tests
npx vitest run src/trim.test.ts   # single TS test file

cd src-tauri && cargo test                    # all Rust tests (unit + integration, incl. real ffmpeg invocations)
cd src-tauri && cargo test trim_integration   # single integration test file
```

No lint script is configured; `tsc` (via `npm run build`) is the type-check gate
for the frontend. Before you call frontend work done, `npm run build` must pass;
before you call Rust work done, `cargo test` must pass.

## Architecture

**Playback goes through an embedded HTTP server, not a custom URI scheme.**
`media_server.rs` runs an axum server on `127.0.0.1` at a random port, serving local
files with HTTP range support at `/file?path=<absolute-path>`. The frontend's
`<video>` element just points at that URL. This exists because custom URI protocols
are unreliable for media in WebKitGTK; real HTTP works uniformly on Linux/Windows/macOS.

**All preview audio flows through ONE shared AudioContext — the app must register
exactly one audio stream with the OS mixer.** Media elements are never used for
audio: on WebKitGTK every media element keeps its own (muted) system audio sink
alive even when captured into Web Audio, so element-based playback showed one OS
stream per track. Instead, each audio stream in the source is extracted once
(cached by a hash of the source path) to a temp `.m4a` via `prepare_audio_tracks`,
and `audioEngine.ts` pulls ~20s PCM windows from it through the `audio_chunk`
command (ffmpeg → WAV bytes over IPC), decodes them with `decodeAudioData`, and
schedules `AudioBufferSourceNode`s (per-track `GainNode` → destination) against
the video clock. Each track's mute checkbox / 0–200% volume slider drives its
`GainNode` directly. The `<video>` plays the original (with its own audio) for
instant startup; once the engine takes over it is muted, and then swapped to a
cached video-only remux (`prepare_video_only`, stream copy) so its pipeline stops
registering an OS audio stream entirely.

**Export mirrors what was actually heard in preview** (`export.rs`): a single enabled
track at 100% volume takes the fast lossless path (`-c copy`, explicit stream maps,
no re-encode); two-or-more tracks or any non-unity volume triggers an `amix` filter
into one output stream (video stream-copied, audio re-encoded to AAC).

**Rust module layout** (`src-tauri/src/`):

- `lib.rs` — Tauri command handlers, wiring commands/events to the modules below
- `media_server.rs` — the local HTTP range-serving server for playback
- `probe.rs` — shells out to `ffprobe` for duration/resolution/codec/audio-track info
- `export.rs` — builds the `ffmpeg` export command (lossless copy vs. `amix` re-encode)
- `media.rs` — ffmpeg arg builders: proxies for unplayable clips (720p H.264 fallback),
  thumbnails, preview-audio PCM chunks (incl. WAV pipe-size patching), video-only remux
- `library.rs` — sqlite-backed clip library index (rusqlite)
- `watcher.rs` — filesystem watcher for watch-mode source folders (`notify` crate)
- `relocate.rs` — crash-safe move of a watched folder to a new disk (same-volume
  rename vs. cross-volume copy-per-file fanout)
- `bundle.rs` — portable library export/import as a zip bundle (re-roots clip paths)
- `setup.rs` — first-run setup/config

**Frontend layout** (`src/`):

- `App.tsx` — main app shell: `.app-layout` is a flex row of `aside.library-panel`
  (fixed-width sidebar) + `main.editor-panel` (video player + controls, `flex: 1`)
- `Timeline.tsx` — custom scrub timeline for setting in/out trim points (`I`/`O` keys)
- `FirstRunWizard.tsx` / `SettingsModal.tsx` — onboarding and settings UI
- `api.ts` — typed wrappers around Tauri `invoke` calls
- `audioEngine.ts` — the chunked preview-audio engine (fetch/decode/schedule on the
  shared AudioContext); its scheduling math lives in `audioChunks.ts`
- `trim.ts`, `sequence.ts`, `exportOptions.ts`, `libraryView.ts`, `preview.ts`,
  `audioChunks.ts`, `wizardValidation.ts`, `resizable.ts` — pure logic modules,
  each with a co-located `*.test.ts`

## Testing instructions

- TS tests are pure-logic unit tests only (in/out invariants, sequence math, wizard
  validation) — no React component tests.
- Rust tests split into unit tests (arg-building logic, e.g. export command
  construction) and integration tests under `src-tauri/tests/` that actually invoke
  the bundled `ffmpeg`/`ffprobe` binaries.
- New pure logic belongs in one of the `src/*.ts` modules with a co-located
  `*.test.ts`; do not add component tests. Fix failures until the whole suite is green.

## Code style

- TypeScript: match the surrounding code; `tsc` under `npm run build` is the gate
  (no ESLint/Prettier config in this repo).
- Rust: keep formatting `cargo fmt`-clean and follow the module-per-concern layout
  above — put new ffmpeg argument construction in `media.rs`/`export.rs`, not in
  command handlers in `lib.rs`.
- Keep ffmpeg work in the existing builder modules so the unit tests keep covering
  the command lines.
- Do not add CSS frameworks or a grid-based rewrite of the layout; extend `App.css`
  flexbox rules.

## Commits and pull requests

- Use conventional-commit subject lines (`feat:`, `fix:`, `chore:`, `ci:`, `docs:`)
  matching `git log`.
- Keep each commit scoped to one change; do not bundle unrelated refactors or
  reformatting.
- Never commit build output (`dist/`, `src-tauri/target/`) or secrets.
- Do not force-push `main`.

## Known constraints

- Do not bump Cargo dependencies via `cargo update` without checking for breakage
  first — verify a dependency actually needs updating before touching `Cargo.lock`.
- The sidecar `ffmpeg`/`ffprobe` binaries are required by the Rust integration tests;
  if they are missing, those failures are environmental, not code regressions.
