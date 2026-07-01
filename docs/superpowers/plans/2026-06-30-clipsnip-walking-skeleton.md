# ClipSnip Walking-Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up a running ClipSnip desktop app that proves every architectural seam end-to-end: pick a video file, probe it with a bundled `ffprobe`, play the original in the WebView, set in/out points, and export a stream-copy trim with bundled `ffmpeg`.

**Architecture:** Tauri 2 native shell. React/TypeScript/Vite frontend talks to a Rust core over Tauri IPC commands. The Rust core invokes bundled `ffmpeg`/`ffprobe` **sidecar** binaries; the frontend never builds FFmpeg command lines — it sends structured intents (a path to probe; a `{source, in, out, output}` trim) and Rust composes the invocation. This walking skeleton deliberately uses the simplest "run to completion and read output" sidecar pattern (no progress streaming, no SQLite, no watcher) — those are added by later subsystem plans onto this proven stack.

**Tech Stack:** Tauri 2, Rust (edition 2021), React 18 + TypeScript + Vite, `tauri-plugin-shell`, `tauri-plugin-dialog`, `serde`/`serde_json`, system `ffmpeg`/`ffprobe` binaries bundled as sidecars.

## Global Constraints

- **Shell:** Tauri 2 (Rust core + web UI). Frontend never constructs FFmpeg argument lists — the Rust `export`/`probe` code composes every invocation.
- **Media engine:** bundled `ffmpeg` + `ffprobe` invoked as Tauri sidecars (target-triple-suffixed binaries in `src-tauri/binaries/`).
- **Reference in place:** source clip files are never copied, moved, or renamed on disk. Export always writes a *new* output file the user chooses; the original is untouched.
- **Platform for this plan:** Linux first (host target triple `x86_64-unknown-linux-gnu` on this machine — confirm in Task 1). Windows/macOS *packaging* is an explicit later step, out of scope here. Do not add cross-platform branching to the skeleton.
- **Product identity:** product name `ClipSnip`; bundle identifier `pl.medportal.clipsnip`; dev server on port `1420` (Vite/Tauri default).
- **Tauri arg-name convention:** Rust command params are `snake_case`; the frontend `invoke()` passes them as **camelCase** keys (e.g. Rust `in_secs` ↔ JS `inSecs`). Structs returned to the frontend use `#[serde(rename_all = "camelCase")]` so TS sees `durationSecs`, `videoCodec`, etc.

---

### Task 1: Toolchain check + Tauri 2 scaffold (Linux)

Stand up the bare Tauri 2 + React/TS app at the repo root and confirm a window opens. Scaffolding hiccups (interactive prompts, non-empty dir) are resolved here so later tasks start from a known-good app.

**Files:**
- Create: `package.json`, `index.html`, `vite.config.ts`, `tsconfig.json`, `src/main.tsx`, `src/App.tsx` (frontend, from template)
- Create: `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`, `src-tauri/build.rs`, `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `src-tauri/capabilities/default.json` (from template)
- Modify: `.gitignore` (template already ignores `target/`, `node_modules/`, `dist/` — confirm, add nothing if present)

**Interfaces:**
- Consumes: nothing (first task).
- Produces: a working `npm run tauri dev`; the app crate's `run()` in `src-tauri/src/lib.rs` (later tasks register commands here); the `tauri.conf.json` (later tasks add `bundle.externalBin` and `app.security.assetProtocol`).

- [ ] **Step 1: Verify the toolchain is present**

Run:
```bash
rustc --version && cargo --version && node --version && npm --version && ffmpeg -version | head -1 && ffprobe -version | head -1
```
Expected: a version line for each. If `ffmpeg`/`ffprobe` are missing, install them (`sudo pacman -S ffmpeg` on CachyOS/Arch). If Rust is missing, install via `rustup`. If the Tauri webkit deps are missing you will find out in Step 4; on Arch they are: `webkit2gtk-4.1 base-devel curl wget file openssl libappindicator-gtk3 librsvg`.

- [ ] **Step 2: Record the host target triple**

Run:
```bash
rustc --print host-tuple
```
Expected: `x86_64-unknown-linux-gnu` on this machine. **Write this value down** — Task 2 names the sidecar binaries with this exact suffix. (On Rust < 1.84 use `rustc -Vv | grep host | cut -f2 -d' '`.)

- [ ] **Step 3: Scaffold the Tauri 2 React-TS app into the repo root**

The repo root already contains `docs/`, `.git/`, and `.gitignore`, so scaffold into a temp dir and move the generated files in (avoids the "directory not empty" refusal and interactive prompts).

Run:
```bash
cd /tmp && rm -rf clipsnip-scaffold && \
npm create tauri-app@latest clipsnip-scaffold -- --template react-ts --manager npm --identifier pl.medportal.clipsnip --yes && \
cd /tmp/clipsnip-scaffold && \
rsync -a --exclude='.git' ./ "/home/milosz/Dokumenty/code/easy video editor/"
```
Expected: `package.json`, `index.html`, `src/`, and `src-tauri/` now exist at the repo root. The template's `.gitignore` files (root + `src-tauri/.gitignore`, which ignore `node_modules`, `dist`, `src-tauri/target`, and `src-tauri/gen`) are copied in — do **not** exclude them from the rsync, or generated/build output will land in git. If `create-tauri-app` flags differ in the installed version, run `npm create tauri-app@latest -- --help` and map: template = `react-ts`, manager = `npm`, identifier = `pl.medportal.clipsnip`.

Then ignore the machine-specific sidecar binaries Task 2 will create (they are dynamically-linked copies of *this* machine's system ffmpeg — not portable, must never enter git history):
```bash
cd "/home/milosz/Dokumenty/code/easy video editor"
printf '\n# Bundled sidecar binaries (machine-specific; regenerated by Task 2 Step 1)\n/src-tauri/binaries/\n' >> .gitignore
```

- [ ] **Step 4: Set product name and install deps, then run dev**

Edit `src-tauri/tauri.conf.json`: set `"productName": "ClipSnip"` and confirm `"identifier": "pl.medportal.clipsnip"`. In the same file set `app.windows[0].title` to `"ClipSnip"`.

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor" && npm install && npm run tauri dev
```
Expected: Rust compiles (first build is slow), then a native window titled **ClipSnip** opens showing the default template page. If the build fails on a missing system library, install the webkit deps from Step 1 and re-run. Close the window (Ctrl-C) to continue.

- [ ] **Step 5: Commit**

```bash
cd "/home/milosz/Dokumenty/code/easy video editor"
git add -A
git commit -m "chore: scaffold Tauri 2 + React/TS ClipSnip skeleton"
```

---

### Task 2: Bundle ffmpeg/ffprobe sidecars and prove they run from Rust

This is the **highest-risk seam** — isolate and prove it before building anything on top. Bundle the binaries, wire the shell plugin and capabilities, and surface the bundled `ffmpeg -version` string in the UI as living proof the sidecar pipeline works.

**Files:**
- Create: `src-tauri/binaries/ffmpeg-x86_64-unknown-linux-gnu`, `src-tauri/binaries/ffprobe-x86_64-unknown-linux-gnu` (copied binaries)
- Modify: `src-tauri/Cargo.toml` (add `tauri-plugin-shell`), `src-tauri/tauri.conf.json` (add `bundle.externalBin`), `src-tauri/capabilities/default.json` (shell execute scope), `src-tauri/src/lib.rs` (plugin + temp command), `src/App.tsx` (show version)
- Modify: `package.json` (add `@tauri-apps/plugin-shell`)

**Interfaces:**
- Consumes: `run()` builder from Task 1.
- Produces: a registered `tauri-plugin-shell`; the `app.shell().sidecar("ffmpeg" | "ffprobe")` capability used by Tasks 3 and 6; a temporary `ffmpeg_version() -> Result<String, String>` command (removed in Task 6).

- [ ] **Step 1: Copy system binaries into the sidecar folder with the target-triple suffix**

Run (uses the triple from Task 1 Step 2):
```bash
cd "/home/milosz/Dokumenty/code/easy video editor"
mkdir -p src-tauri/binaries
cp "$(which ffmpeg)"  src-tauri/binaries/ffmpeg-x86_64-unknown-linux-gnu
cp "$(which ffprobe)" src-tauri/binaries/ffprobe-x86_64-unknown-linux-gnu
chmod +x src-tauri/binaries/*
ls -l src-tauri/binaries
```
Expected: two executable files named with the `-x86_64-unknown-linux-gnu` suffix. These are gitignored (Task 1 Step 3) and re-created by re-running this `cp` on any machine — so `git add -A` in later steps will not commit them. (Static per-OS builds for distributable bundles are a later packaging concern — spec §9; system binaries are correct for the Linux-first skeleton.)

- [ ] **Step 2: Add the shell plugin dependency (Rust + JS)**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo add tauri-plugin-shell
cd "/home/milosz/Dokumenty/code/easy video editor" && npm install @tauri-apps/plugin-shell
```
Expected: `tauri-plugin-shell = "2"` (or `2.x`) appears in `src-tauri/Cargo.toml`; `@tauri-apps/plugin-shell` in `package.json`.

- [ ] **Step 3: Declare the sidecars in `tauri.conf.json`**

In `src-tauri/tauri.conf.json`, add to the `bundle` object:
```json
{
  "bundle": {
    "externalBin": [
      "binaries/ffmpeg",
      "binaries/ffprobe"
    ]
  }
}
```
(Merge into the existing `bundle` block — do not duplicate the key. The path is the base name *without* the triple suffix; Tauri appends the right suffix per target.)

- [ ] **Step 4: Grant the sidecar permission in capabilities**

Replace the contents of `src-tauri/capabilities/default.json` with (keep any window label the template generated; the template uses `"main"`):
```json
{
  "$schema": "../gen/schemas/desktop-schema.json",
  "identifier": "default",
  "description": "Default capability for ClipSnip",
  "windows": ["main"],
  "permissions": [
    "core:default",
    {
      "identifier": "shell:allow-execute",
      "allow": [
        { "name": "binaries/ffmpeg",  "sidecar": true, "args": true },
        { "name": "binaries/ffprobe", "sidecar": true, "args": true }
      ]
    }
  ]
}
```
We use `shell:allow-execute` because Rust calls `.output()` (run-to-completion). `"args": true` allows arbitrary arguments (needed since trim/probe args are dynamic).

- [ ] **Step 5: Register the plugin and add a temporary version command**

Edit `src-tauri/src/lib.rs`. Add near the top:
```rust
use tauri_plugin_shell::ShellExt;

#[tauri::command]
async fn ffmpeg_version(app: tauri::AppHandle) -> Result<String, String> {
    let output = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(["-version"])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().next().unwrap_or("").to_string())
}
```
In the `run()` builder, register the plugin and the command:
```rust
tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .invoke_handler(tauri::generate_handler![ffmpeg_version])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```
(If the template's `run()` already has `.invoke_handler(...)` with a `greet` command, replace the handler list with `[ffmpeg_version]` and delete the unused `greet` fn.)

- [ ] **Step 6: Show the version in the UI**

Replace `src/App.tsx` with:
```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [version, setVersion] = useState<string>("loading…");

  useEffect(() => {
    invoke<string>("ffmpeg_version")
      .then(setVersion)
      .catch((e) => setVersion(`ERROR: ${e}`));
  }, []);

  return (
    <main className="container">
      <h1>ClipSnip</h1>
      <p data-testid="ffmpeg-version">Bundled FFmpeg: {version}</p>
    </main>
  );
}

export default App;
```

- [ ] **Step 7: Run dev and verify the sidecar works**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor" && npm run tauri dev
```
Expected: the window shows **Bundled FFmpeg: ffmpeg version N.N …** (the bundled binary's version line). If it shows `ERROR: …`, the sidecar pipeline is broken — check the binary suffix (Step 1), the `externalBin` paths (Step 3), and the capability (Step 4) before moving on. Ctrl-C when verified.

- [ ] **Step 8: Commit**

```bash
cd "/home/milosz/Dokumenty/code/easy video editor"
git add -A
git commit -m "feat: bundle ffmpeg/ffprobe sidecars and verify execution from Rust"
```

---

### Task 3: ffprobe metadata — TDD parser + probe command + file picker UI

Add a pure `parse_ffprobe_json` function (TDD), wire a `probe_clip` command that runs the `ffprobe` sidecar, and let the user pick a file and see its metadata.

**Files:**
- Create: `src-tauri/src/probe.rs` (struct + parser + tests)
- Modify: `src-tauri/src/lib.rs` (declare `mod probe`, add `probe_clip` command, register it)
- Modify: `src-tauri/Cargo.toml` (ensure `serde`, `serde_json`), `package.json` (add `@tauri-apps/plugin-dialog`)
- Modify: `src-tauri/capabilities/default.json` (add dialog open permission)
- Create: `src/api.ts` (typed invoke wrappers + types)
- Modify: `src/App.tsx` (pick button + metadata display)

**Interfaces:**
- Consumes: the shell sidecar capability from Task 2.
- Produces:
  - Rust: `pub struct AudioTrack { index: u32, codec: String, channels: u32, language: Option<String> }` and `pub struct ClipMeta { duration_secs: f64, width: u32, height: u32, container: String, video_codec: String, audio_tracks: Vec<AudioTrack> }` (both `#[serde(rename_all = "camelCase")]`); `pub fn parse_ffprobe_json(json: &str) -> Result<ClipMeta, String>`; command `probe_clip(app, path: String) -> Result<ClipMeta, String>`.
  - TS: `interface ClipMeta` / `interface AudioTrack` (camelCase fields) and `async function probeClip(path: string): Promise<ClipMeta>` in `src/api.ts`.

- [ ] **Step 1: Write the failing parser test**

Create `src-tauri/src/probe.rs`:
```rust
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub index: u32,
    pub codec: String,
    pub channels: u32,
    pub language: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipMeta {
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    pub container: String,
    pub video_codec: String,
    pub audio_tracks: Vec<AudioTrack>,
}

pub fn parse_ffprobe_json(_json: &str) -> Result<ClipMeta, String> {
    Err("not implemented".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "streams": [
        { "index": 0, "codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080 },
        { "index": 1, "codec_type": "audio", "codec_name": "aac", "channels": 2, "tags": { "language": "eng" } },
        { "index": 2, "codec_type": "audio", "codec_name": "aac", "channels": 2 }
      ],
      "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.345" }
    }"#;

    #[test]
    fn parses_video_audio_and_format() {
        let m = parse_ffprobe_json(SAMPLE).unwrap();
        assert_eq!(m.width, 1920);
        assert_eq!(m.height, 1080);
        assert_eq!(m.video_codec, "h264");
        assert_eq!(m.container, "mov,mp4,m4a,3gp,3g2,mj2");
        assert!((m.duration_secs - 12.345).abs() < 1e-6);
        assert_eq!(m.audio_tracks.len(), 2);
        assert_eq!(m.audio_tracks[0].language.as_deref(), Some("eng"));
        assert_eq!(m.audio_tracks[1].language, None);
        assert_eq!(m.audio_tracks[0].channels, 2);
    }
}
```
Add `mod probe;` to the top of `src-tauri/src/lib.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo test parses_video_audio_and_format
```
Expected: FAIL — `not implemented` / assertion on `unwrap()` of `Err`.

- [ ] **Step 3: Implement the parser**

In `src-tauri/src/probe.rs`, replace the stub `parse_ffprobe_json` with:
```rust
pub fn parse_ffprobe_json(json: &str) -> Result<ClipMeta, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let streams = v["streams"].as_array().ok_or("no streams in ffprobe output")?;

    let video = streams
        .iter()
        .find(|s| s["codec_type"] == "video")
        .ok_or("no video stream found")?;
    let width = video["width"].as_u64().ok_or("video stream missing width")? as u32;
    let height = video["height"].as_u64().ok_or("video stream missing height")? as u32;
    let video_codec = video["codec_name"].as_str().unwrap_or("unknown").to_string();

    let audio_tracks = streams
        .iter()
        .filter(|s| s["codec_type"] == "audio")
        .map(|s| AudioTrack {
            index: s["index"].as_u64().unwrap_or(0) as u32,
            codec: s["codec_name"].as_str().unwrap_or("unknown").to_string(),
            channels: s["channels"].as_u64().unwrap_or(0) as u32,
            language: s["tags"]["language"].as_str().map(|x| x.to_string()),
        })
        .collect();

    let container = v["format"]["format_name"].as_str().unwrap_or("unknown").to_string();
    let duration_secs = v["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .ok_or("format.duration missing or unparseable")?;

    Ok(ClipMeta { duration_secs, width, height, container, video_codec, audio_tracks })
}
```
Ensure `serde` (with `derive`) and `serde_json` are dependencies:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo add serde --features derive && cargo add serde_json
```

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo test parses_video_audio_and_format
```
Expected: PASS (1 passed).

- [ ] **Step 5: Add the `probe_clip` command**

In `src-tauri/src/lib.rs` add:
```rust
use probe::{parse_ffprobe_json, ClipMeta};
use tauri_plugin_shell::ShellExt;

#[tauri::command]
async fn probe_clip(app: tauri::AppHandle, path: String) -> Result<ClipMeta, String> {
    let output = app
        .shell()
        .sidecar("ffprobe")
        .map_err(|e| e.to_string())?
        .args([
            "-v", "quiet",
            "-print_format", "json",
            "-show_format",
            "-show_streams",
            path.as_str(),
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    parse_ffprobe_json(&String::from_utf8_lossy(&output.stdout))
}
```
Add `probe_clip` to the `generate_handler!` list (alongside `ffmpeg_version`). (Remove the now-redundant `use tauri_plugin_shell::ShellExt;` from Task 2 if it causes a duplicate-import warning — keep exactly one.)

- [ ] **Step 6: Add the dialog plugin and a typed frontend API**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo add tauri-plugin-dialog
cd "/home/milosz/Dokumenty/code/easy video editor" && npm install @tauri-apps/plugin-dialog
```
Register it in `run()`: add `.plugin(tauri_plugin_dialog::init())` to the builder chain.
Add the dialog permission to `src-tauri/capabilities/default.json` `permissions` array: add the string `"dialog:default"`.

Create `src/api.ts`:
```ts
import { invoke } from "@tauri-apps/api/core";

export interface AudioTrack {
  index: number;
  codec: string;
  channels: number;
  language: string | null;
}

export interface ClipMeta {
  durationSecs: number;
  width: number;
  height: number;
  container: string;
  videoCodec: string;
  audioTracks: AudioTrack[];
}

export function probeClip(path: string): Promise<ClipMeta> {
  return invoke<ClipMeta>("probe_clip", { path });
}
```

- [ ] **Step 7: Wire pick + metadata into the UI**

Replace `src/App.tsx` with:
```tsx
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { probeClip, type ClipMeta } from "./api";
import "./App.css";

function App() {
  const [path, setPath] = useState<string | null>(null);
  const [meta, setMeta] = useState<ClipMeta | null>(null);
  const [error, setError] = useState<string | null>(null);

  async function pickClip() {
    setError(null);
    const selected = await open({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mkv", "mov", "avi", "webm", "flv"] }],
    });
    if (typeof selected !== "string") return;
    setPath(selected);
    try {
      setMeta(await probeClip(selected));
    } catch (e) {
      setError(String(e));
      setMeta(null);
    }
  }

  return (
    <main className="container">
      <h1>ClipSnip</h1>
      <button onClick={pickClip}>Open clip…</button>
      {path && <p>{path}</p>}
      {error && <p style={{ color: "tomato" }}>{error}</p>}
      {meta && (
        <ul data-testid="clip-meta">
          <li>Duration: {meta.durationSecs.toFixed(2)} s</li>
          <li>Resolution: {meta.width}×{meta.height}</li>
          <li>Container: {meta.container}</li>
          <li>Video codec: {meta.videoCodec}</li>
          <li>Audio tracks: {meta.audioTracks.length}</li>
        </ul>
      )}
    </main>
  );
}

export default App;
```

- [ ] **Step 8: Run dev and verify probing a real clip**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor" && npm run tauri dev
```
Expected: click **Open clip…**, pick any video; the metadata list shows a plausible duration, resolution, container, codec, and audio-track count. Ctrl-C when verified.

- [ ] **Step 9: Commit**

```bash
cd "/home/milosz/Dokumenty/code/easy video editor"
git add -A
git commit -m "feat: probe clips with bundled ffprobe and show metadata"
```

---

### Task 4: Native player (play the original in the WebView)

Play the picked clip's original file in a `<video>` element via Tauri's asset protocol.

**Files:**
- Modify: `src-tauri/tauri.conf.json` (enable `app.security.assetProtocol`)
- Modify: `src/App.tsx` (render `<video>` with `convertFileSrc`)

**Interfaces:**
- Consumes: `path` state from Task 3.
- Produces: a playing `<video ref>` element; current-time state later read by Task 5's in/out logic.

- [ ] **Step 1: Enable the asset protocol**

In `src-tauri/tauri.conf.json`, under `app.security`, add:
```json
{
  "app": {
    "security": {
      "assetProtocol": {
        "enable": true,
        "scope": ["**"]
      }
    }
  }
}
```
(Merge into the existing `app.security` object. `"**"` is a permissive dev scope so any picked path loads; a later hardening step can narrow it to the configured watched/output folders.)

- [ ] **Step 2: Render the original via `convertFileSrc`**

In `src/App.tsx`, add the import:
```tsx
import { convertFileSrc } from "@tauri-apps/api/core";
```
After the metadata list (still inside `<main>`), render the player when a path is set:
```tsx
{path && (
  <video
    src={convertFileSrc(path)}
    controls
    style={{ width: "100%", maxWidth: 720, background: "#000" }}
  />
)}
```

- [ ] **Step 3: Run dev and verify playback**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor" && npm run tauri dev
```
Expected: after opening an H.264 MP4 clip, it plays with the native controls. If the video stays black with a console error like `Not allowed to load local resource` or a media `network` error, the asset scope is rejecting the path — confirm `assetProtocol.enable` is `true` and widen the scope (try `["$HOME/**", "**"]`). HEVC/AV1/some MKV may not play in the WebView — that's expected and handled by the proxy-fallback subsystem in a later plan; verify with an H.264 MP4 here. Ctrl-C when verified.

- [ ] **Step 4: Commit**

```bash
cd "/home/milosz/Dokumenty/code/easy video editor"
git add -A
git commit -m "feat: play original clip in the webview via asset protocol"
```

---

### Task 5: In/out trim points (TDD clamp logic + UI + hotkeys)

Let the user set in/out points against the player's current time, with the invariant `0 ≤ in < out ≤ duration` enforced by a small pure function.

**Files:**
- Create: `src/trim.ts` (pure clamp helpers)
- Create: `src/trim.test.ts` (Vitest)
- Modify: `package.json` (add `vitest`), `vite.config.ts` (test config) OR `vitest.config.ts`
- Modify: `src/App.tsx` (in/out state, buttons, `I`/`O` hotkeys)

**Interfaces:**
- Consumes: video `currentTime` and `meta.durationSecs`.
- Produces: `export function setIn(current: number, out: number | null): number` and `export function setOut(current: number, inp: number | null, duration: number): number`; in/out state consumed by Task 6's export call.

- [ ] **Step 1: Add Vitest and write the failing clamp tests**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor" && npm install -D vitest
```
Create `src/trim.ts`:
```ts
const EPS = 0.01;

// Clamp a new IN point: never below 0, and strictly left of OUT (if OUT set).
export function setIn(current: number, out: number | null): number {
  const lowerBounded = Math.max(0, current);
  if (out !== null) return Math.min(lowerBounded, out - EPS);
  return lowerBounded;
}

// Clamp a new OUT point: never above duration, and strictly right of IN (if IN set).
export function setOut(current: number, inp: number | null, duration: number): number {
  const upperBounded = Math.min(duration, current);
  if (inp !== null) return Math.max(upperBounded, inp + EPS);
  return upperBounded;
}
```
Create `src/trim.test.ts`:
```ts
import { describe, expect, it } from "vitest";
import { setIn, setOut } from "./trim";

describe("trim clamps", () => {
  it("clamps IN to >= 0", () => {
    expect(setIn(-5, null)).toBe(0);
  });
  it("keeps IN strictly left of OUT", () => {
    expect(setIn(9, 8)).toBeCloseTo(7.99, 5);
  });
  it("clamps OUT to <= duration", () => {
    expect(setOut(100, null, 30)).toBe(30);
  });
  it("keeps OUT strictly right of IN", () => {
    expect(setOut(2, 5, 30)).toBeCloseTo(5.01, 5);
  });
});
```

- [ ] **Step 2: Run the tests to verify they pass (logic already written) — first confirm the runner works**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor" && npx vitest run src/trim.test.ts
```
Expected: 4 passed. (Here the pure function is trivial enough to write with its test; the value is the regression guard and the documented invariant, which Task 6 depends on.) If the runner errors on config, add a minimal `vitest.config.ts`:
```ts
import { defineConfig } from "vitest/config";
export default defineConfig({ test: { environment: "node" } });
```

- [ ] **Step 3: Wire in/out into the UI with hotkeys**

In `src/App.tsx`: add `useRef`/`useEffect` imports, a `videoRef`, `inPoint`/`outPoint` state, and handlers. Attach `ref={videoRef}` to the `<video>`. Add this inside the component:
```tsx
const videoRef = useRef<HTMLVideoElement>(null);
const [inPoint, setInPoint] = useState<number | null>(null);
const [outPoint, setOutPoint] = useState<number | null>(null);

function markIn() {
  const t = videoRef.current?.currentTime ?? 0;
  setInPoint(setIn(t, outPoint));
}
function markOut() {
  const t = videoRef.current?.currentTime ?? 0;
  setOutPoint(setOut(t, inPoint, meta?.durationSecs ?? t));
}

useEffect(() => {
  function onKey(e: KeyboardEvent) {
    if (e.key === "i" || e.key === "I") markIn();
    if (e.key === "o" || e.key === "O") markOut();
  }
  window.addEventListener("keydown", onKey);
  return () => window.removeEventListener("keydown", onKey);
}, [inPoint, outPoint, meta]);
```
Add the imports at the top: `import { setIn, setOut } from "./trim";` and extend the React import to `import { useEffect, useRef, useState } from "react";`. Add controls under the `<video>`:
```tsx
{path && (
  <div data-testid="trim-controls">
    <button onClick={markIn}>Set In (I)</button>
    <button onClick={markOut}>Set Out (O)</button>
    <span> In: {inPoint?.toFixed(2) ?? "—"} | Out: {outPoint?.toFixed(2) ?? "—"}</span>
  </div>
)}
```

- [ ] **Step 4: Run dev and verify in/out**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor" && npm run tauri dev
```
Expected: play the clip, press `I`/`O` (or the buttons) at different times; In/Out values update and never cross (In stays < Out). Ctrl-C when verified.

- [ ] **Step 5: Commit**

```bash
cd "/home/milosz/Dokumenty/code/easy video editor"
git add -A
git commit -m "feat: set in/out trim points with clamped invariants and hotkeys"
```

---

### Task 6: Stream-copy trim export (TDD arg-builder + command + real-ffmpeg integration test)

Close the loop: a pure `build_trim_copy_args` function (TDD), an `export_trim` command running the `ffmpeg` sidecar, a save dialog, and a real-ffmpeg integration test that generates a clip, trims it, and asserts the output duration — the spec §7 integration anchor. Also remove the temporary `ffmpeg_version` debug command.

**Files:**
- Create: `src-tauri/src/export.rs` (arg builder + unit test)
- Create: `src-tauri/tests/trim_integration.rs` (real-ffmpeg round-trip)
- Modify: `src-tauri/src/lib.rs` (declare `mod export`, add `export_trim`, drop `ffmpeg_version`)
- Modify: `src-tauri/capabilities/default.json` (add `dialog:default` already present; ensure save allowed — `dialog:default` covers save)
- Modify: `src/api.ts` (add `exportTrim`), `src/App.tsx` (Export button + save dialog)

**Interfaces:**
- Consumes: `path`, `inPoint`, `outPoint` from Tasks 3–5; ffmpeg sidecar from Task 2.
- Produces: `pub fn build_trim_copy_args(input: &str, in_secs: f64, out_secs: f64, output: &str) -> Vec<String>`; command `export_trim(app, source, in_secs, out_secs, output) -> Result<String, String>`; TS `exportTrim(source, inSecs, outSecs, output)`.

- [ ] **Step 1: Write the failing arg-builder test**

Create `src-tauri/src/export.rs`:
```rust
/// Build ffmpeg args for a keyframe-snapped stream-copy trim (the "Fast (lossless)" path).
/// Input-side `-ss` does a fast seek; `-t` bounds the copied duration. With `-c copy`
/// the start snaps to the nearest preceding keyframe, so output duration is within one
/// GOP of the requested span.
pub fn build_trim_copy_args(_input: &str, _in_secs: f64, _out_secs: f64, _output: &str) -> Vec<String> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_input_seek_copy_args() {
        let args = build_trim_copy_args("in.mp4", 1.0, 3.5, "out.mp4");
        assert_eq!(
            args,
            vec![
                "-y",
                "-ss", "1.000",
                "-i", "in.mp4",
                "-t", "2.500",
                "-c", "copy",
                "out.mp4",
            ]
        );
    }
}
```
Add `mod export;` to the top of `src-tauri/src/lib.rs`.

- [ ] **Step 2: Run the test to verify it fails**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo test builds_input_seek_copy_args
```
Expected: FAIL — left `[]` ≠ expected vector.

- [ ] **Step 3: Implement the arg builder**

Replace the stub in `src-tauri/src/export.rs`:
```rust
pub fn build_trim_copy_args(input: &str, in_secs: f64, out_secs: f64, output: &str) -> Vec<String> {
    let duration = out_secs - in_secs;
    vec![
        "-y".to_string(),
        "-ss".to_string(),
        format!("{:.3}", in_secs),
        "-i".to_string(),
        input.to_string(),
        "-t".to_string(),
        format!("{:.3}", duration),
        "-c".to_string(),
        "copy".to_string(),
        output.to_string(),
    ]
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo test builds_input_seek_copy_args
```
Expected: PASS.

- [ ] **Step 5: Write the real-ffmpeg integration test**

Create `src-tauri/tests/trim_integration.rs`. It calls the **same** arg vector against the system `ffmpeg`/`ffprobe` on PATH (the integration anchor decouples from the Tauri runtime; the command in Step 7 wires the identical builder to the sidecar):
```rust
use std::process::Command;

// Re-declare the builder here is not allowed (not a lib export across the bin crate),
// so this test reproduces the exact arg vector the app uses. If you change
// build_trim_copy_args, change this to match.
fn trim_args(input: &str, in_secs: f64, out_secs: f64, output: &str) -> Vec<String> {
    let duration = out_secs - in_secs;
    vec![
        "-y".into(),
        "-ss".into(),
        format!("{:.3}", in_secs),
        "-i".into(),
        input.into(),
        "-t".into(),
        format!("{:.3}", duration),
        "-c".into(),
        "copy".into(),
        output.into(),
    ]
}

fn probe_duration(path: &str) -> f64 {
    let out = Command::new("ffprobe")
        .args([
            "-v", "quiet",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            path,
        ])
        .output()
        .expect("ffprobe runs");
    String::from_utf8_lossy(&out.stdout).trim().parse().expect("duration parses")
}

#[test]
fn trims_a_generated_clip_to_requested_span() {
    let dir = std::env::temp_dir();
    let src = dir.join("clipsnip_src.mp4");
    let dst = dir.join("clipsnip_dst.mp4");
    let src_s = src.to_str().unwrap();
    let dst_s = dst.to_str().unwrap();

    // Generate a 6s 320x240 clip with a small GOP so keyframe-snapped trim is near-exact.
    let gen = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "lavfi", "-i", "testsrc=duration=6:size=320x240:rate=30",
            "-f", "lavfi", "-i", "sine=frequency=1000:duration=6",
            "-c:v", "libx264", "-g", "15", "-pix_fmt", "yuv420p",
            "-c:a", "aac",
            src_s,
        ])
        .status()
        .expect("ffmpeg generate runs");
    assert!(gen.success(), "clip generation failed");

    // Trim 1.0 .. 3.0 -> expect ~2.0s.
    let trim = Command::new("ffmpeg").args(trim_args(src_s, 1.0, 3.0, dst_s)).status().expect("ffmpeg trim runs");
    assert!(trim.success(), "trim failed");

    let d = probe_duration(dst_s);
    assert!((d - 2.0).abs() < 0.5, "expected ~2.0s, got {d}");

    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&dst);
}
```

- [ ] **Step 6: Run the integration test**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo test --test trim_integration -- --nocapture
```
Expected: PASS — the generated clip is trimmed to within 0.5 s of 2.0 s. (Requires `ffmpeg`/`ffprobe` on PATH, confirmed in Task 1.)

- [ ] **Step 7: Add the `export_trim` command and drop the debug command**

In `src-tauri/src/lib.rs`:
- add `use export::build_trim_copy_args;`
- add the command:
```rust
#[tauri::command]
async fn export_trim(
    app: tauri::AppHandle,
    source: String,
    in_secs: f64,
    out_secs: f64,
    output: String,
) -> Result<String, String> {
    let args = build_trim_copy_args(&source, in_secs, out_secs, &output);
    let result = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(args)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(String::from_utf8_lossy(&result.stderr).to_string());
    }
    Ok(output)
}
```
- delete the `ffmpeg_version` fn and remove it from `generate_handler!`; the handler list becomes `tauri::generate_handler![probe_clip, export_trim]`.

- [ ] **Step 8: Add the frontend export wrapper and UI**

In `src/api.ts` add:
```ts
export function exportTrim(
  source: string,
  inSecs: number,
  outSecs: number,
  output: string
): Promise<string> {
  return invoke<string>("export_trim", { source, inSecs, outSecs, output });
}
```
In `src/App.tsx`: import the save dialog and wrapper:
```tsx
import { open, save } from "@tauri-apps/plugin-dialog";
import { probeClip, exportTrim, type ClipMeta } from "./api";
```
Add a handler and a status line:
```tsx
const [status, setStatus] = useState<string | null>(null);

async function doExport() {
  if (!path || inPoint === null || outPoint === null) {
    setStatus("Set In and Out first.");
    return;
  }
  const dest = await save({ defaultPath: "clip-trimmed.mp4" });
  if (!dest) return;
  setStatus("Exporting…");
  try {
    const out = await exportTrim(path, inPoint, outPoint, dest);
    setStatus(`Exported: ${out}`);
  } catch (e) {
    setStatus(`Export failed: ${e}`);
  }
}
```
Add to the trim-controls block:
```tsx
<button onClick={doExport}>Export trim</button>
{status && <p data-testid="export-status">{status}</p>}
```

- [ ] **Step 9: Run dev and verify the full loop**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor" && npm run tauri dev
```
Expected: open an H.264 MP4 → it plays → set In and Out → **Export trim** → pick a destination → status shows `Exported: …`. Open the output file in any player and confirm it is the trimmed span. The original source file is unchanged on disk. Ctrl-C when verified.

- [ ] **Step 10: Run the full test suite and commit**

Run:
```bash
cd "/home/milosz/Dokumenty/code/easy video editor/src-tauri" && cargo test
cd "/home/milosz/Dokumenty/code/easy video editor" && npx vitest run
```
Expected: all Rust unit + integration tests pass; Vitest passes.
```bash
cd "/home/milosz/Dokumenty/code/easy video editor"
git add -A
git commit -m "feat: stream-copy trim export end-to-end with integration test"
```

---

## Walking-skeleton done — what this proves

After Task 6 the stack is proven end-to-end on Linux: Tauri IPC, the **FFmpeg/ffprobe sidecar** (the highest-risk integration), the native player, and the **export job-runner** — all exercised by a real trim. Pure-function TDD anchors (`parse_ffprobe_json`, `build_trim_copy_args`) and a real-ffmpeg integration test are in place and will grow with each subsystem.

**Next plan (write only after this one is built and reviewed):** *Library & auto-import* — `notify` watcher with stable-size write-completion detection, SQLite index (rusqlite), thumbnails, the reactive library grid, and the rename/favorites/virtual-folders organization, all layered onto this skeleton. Then: Editor & preview (proxy fallback) → Export engine (mixing, concat, two-pass target-size compression, streamed progress) → Portability (library export/import, relocate folder). Deferred to a later step: Windows/macOS packaging and static per-OS FFmpeg sourcing.
