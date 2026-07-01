# ClipSnip

A small desktop clip-trimming tool built with Tauri (Rust) + React/TypeScript. Open a
video, mark an in/out point, control each audio track's volume live, and export a
trimmed clip — losslessly when possible.

## How it works

**Shell:** Tauri 2 wraps a React frontend in a native window (WebKitGTK on Linux,
WebView2 on Windows, WKWebView on macOS) with a Rust backend. `ffmpeg` and `ffprobe`
are bundled as sidecar binaries (`src-tauri/binaries/`) — no system install required.

**Playback:** the frontend doesn't load video files directly into the webview. Custom
URI protocols are unreliable for media in modern WebKitGTK, so instead the Rust side
runs a small embedded HTTP server ([media_server.rs](src-tauri/src/media_server.rs))
bound to `127.0.0.1` on a random port, serving any local file (with HTTP range support
for seeking) at `/file?path=<absolute-path>`. The `<video>` element just points at that
URL — real HTTP works uniformly across all three platforms.

**Probing:** opening a clip shells out to the bundled `ffprobe` to read duration,
resolution, codec, and the list of audio tracks ([probe.rs](src-tauri/src/probe.rs)).

**Trimming:** in/out points are set from the current playhead (`I` / `O` keys or
buttons) and rendered on a custom scrub timeline ([Timeline.tsx](src/Timeline.tsx)).

**Multi-track audio (live preview):** rather than relying on the browser's native
single-track audio decode, each audio stream in the source file is extracted once to a
temp `.m4a` (cached by a hash of the source path) via the `prepare_audio_tracks`
Tauri command. The frontend loads each extracted track into its own `<audio>` element
and routes it through the Web Audio API (`createMediaElementSource` → per-track
`GainNode` → `AudioContext.destination`), synced to the video's `currentTime` on
play/pause/seek. The native `<video>` element is force-muted once this graph is live,
so the only audible output is the mixed Web Audio graph — this also blocks the
browser's own built-in volume/mute control from sneaking the video's original audio
back in. Each track's checkbox (mute) and slider (0–200% volume) directly drives its
`GainNode`.

**Export:** `ffmpeg` builds the output from the same in/out points and track
enable/volume state ([export.rs](src-tauri/src/export.rs)):
- A single enabled track at 100% volume → fast lossless path: `-c copy` with explicit
  stream maps, no re-encoding.
- Two or more enabled tracks, or any non-unity volume → the tracks are combined with
  an `amix` filter into one output audio stream (video stays stream-copied, audio is
  re-encoded to AAC), so the export matches what you actually heard in preview.

## Running it

Requires Node.js and the Rust toolchain (`cargo`) installed.

```bash
npm install
```

**Development** (hot reload for both frontend and Rust changes):

```bash
npm run tauri dev
```

**Production build** (compiles a release binary + platform installer, e.g. AppImage/
`.deb` on Linux, under `src-tauri/target/release/bundle/`):

```bash
npm run tauri build
```

Once built, you can launch the app directly without going through npm, e.g. on Linux:

```bash
./src-tauri/target/release/clipsnip
```

## Tests

```bash
# Rust: ffmpeg arg-building logic + an integration test that actually invokes ffmpeg
cd src-tauri && cargo test

# TypeScript: trim in/out point invariants
npx vitest run
```
