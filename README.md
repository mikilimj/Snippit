<p align="center">
  <img src="logo.png" alt="ClipSnip logo" width="96" height="96">
</p>

<h1 align="center">ClipSnip</h1>

<p align="center">
  A fast desktop clip-trimming tool with live multi-track audio mixing,<br>
  a clip library, and lossless export — built with Tauri (Rust) + React/TypeScript.
</p>

<p align="center">
  <a href="https://github.com/mikilimj/ClipSnip/actions/workflows/build.yml"><img src="https://img.shields.io/github/actions/workflow/status/mikilimj/ClipSnip/build.yml" alt="Build status"></a>
  <img src="https://img.shields.io/badge/version-1.2.0-blue" alt="Version">
  <img src="https://img.shields.io/badge/platforms-Linux%20%7C%20Windows-lightgrey" alt="Platforms">
</p>

Open a video, mark an in/out point, control each audio track's volume live, and
export a trimmed clip — losslessly when possible, no re-encode required for the
common case. `ffmpeg`, `ffprobe`, and `rclone` ship as sidecar binaries, so there's
nothing to install separately.

> [!IMPORTANT]
> **ClipSnip does not record anything.** It's an editor for clips you already
> have on disk — trim, mix, and export. To capture gameplay or your screen in
> the first place, pair it with a dedicated recorder, for example:
>
> - **[GPU Screen Recorder](https://git.dec05eba.com/gpu-screen-recorder/about/)** (Linux) — recommended, low-overhead GPU-accelerated capture
> - **[OBS Studio](https://obsproject.com/)** (Linux/Windows/macOS) — full-featured, widely used
> - **Steam** built-in game recording (Linux/Windows/macOS, via the Steam client)

## Features

- **Trim with instant preview** — set in/out points from the current playhead
  (`I` / `O` keys) on a custom scrub timeline, with a fast lossless (`-c copy`)
  export path whenever no re-encoding is needed.
- **Live multi-track audio mixing** — mute or adjust the volume (0–200%) of each
  audio track independently while previewing, all routed through a single shared
  `AudioContext` so the OS only ever sees one audio stream. Export mixes exactly
  what you heard in preview.
- **Clip library** — a persistent, searchable library with favorites and folders,
  backed by SQLite, plus watch-mode folders that auto-index new recordings as they
  land on disk.
- **Sequence export & compression** — stitch multiple trimmed clips into one
  export, or re-encode down to a target file size.
- **Remote storage sync** — mirror your `Clips/` folder to SFTP, Dropbox, or
  Google Drive via a bundled `rclone` sidecar: automatic background uploads, a
  diff view for pulling remote-only clips back down, and a full restore path for
  fresh installs.
- **Cross-platform, no dependencies to install** — `ffmpeg`/`ffprobe`/`rclone`
  ship with the app; playback goes through a local HTTP server instead of a
  custom URI scheme, so media works reliably on Linux, Windows, and macOS.

## Getting started

**Requirements:** [Node.js](https://nodejs.org/) and the [Rust toolchain](https://rustup.rs/)
(`cargo`).

```bash
git clone https://github.com/mikilimj/ClipSnip.git
cd ClipSnip
npm install
```

Run in development mode (hot reload for both the React frontend and Rust backend):

```bash
npm run tauri dev
```

Build a production binary + platform installer (AppImage/`.deb` on Linux, MSI on
Windows) under `src-tauri/target/release/bundle/`:

```bash
npm run tauri build
```

Once built, launch the binary directly without going through npm, e.g. on Linux:

```bash
./src-tauri/target/release/clipsnip
```

### Arch Linux / AUR

Package definitions are available under [`packaging/aur/`](packaging/aur/):
`clipsnip` (builds from source) and `clipsnip-bin` (prebuilt `.deb` repackaged).

### Running the tests

```bash
npx vitest run              # TypeScript unit tests (pure logic, no UI)
cd src-tauri && cargo test   # Rust unit + integration tests (real ffmpeg invocations)
```

`npm run build` (`tsc` + `vite build`) is the type-check gate for the frontend; no
separate lint script is configured.

## Architecture

For a deeper look at how playback, audio mixing, export, and storage sync are
wired together, see [`CLAUDE.md`](CLAUDE.md) — it documents the Rust module
layout (`src-tauri/src/`), the frontend structure (`src/`), and the reasoning
behind some of the less obvious design decisions (e.g. why playback goes through
an HTTP server, why audio never uses `<audio>`/`<video>` elements directly).

## Getting help

- **Bugs and feature requests:** open a [GitHub issue](https://github.com/mikilimj/ClipSnip/issues).
- **Design background:** the `docs/superpowers/` folder holds the specs and
  plans written while building major features (first-run setup, resizable
  panels, GPU export, storage sync), useful context if you're digging into why
  something works the way it does.

## Maintainers & contributing

ClipSnip is maintained by [@mikilimj](https://github.com/mikilimj). Issues and
pull requests are welcome — please open an issue to discuss significant changes
before submitting a PR.
