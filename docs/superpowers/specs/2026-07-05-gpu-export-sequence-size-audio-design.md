# GPU-accelerated export, sequence size target, export device setting, sequence audio fix

Date: 2026-07-05
Status: approved for implementation (autonomous session; decisions documented below)

## Problems

1. **No GPU acceleration on export.** Every re-encoding export path (Compatible trim,
   target-size compression, sequence stitch) hardcodes `libx264` — CPU-only encode and
   software decode.
2. **The Size dropdown only applies to single-clip exports.** Sequence export always
   re-encodes at CRF 20 with no way to hit a target file size.
3. **No way to choose CPU vs GPU.** The user wants a settings option to pick the device
   used for export encode/decode.
4. **Bug: sequence export loses audio tracks.** `build_sequence_args` maps each
   segment's audio as `[i:a]`, which resolves to the *first* audio stream only, and
   per-track configs (mute/volume) are never sent to `export_sequence` at all. Preview
   mixes every enabled track, so exports don't match what was heard whenever a source
   has more than one audio track.

## Design

### Export device model (`export.rs` + `setup.rs`)

- New `ExportDevice` enum: `Cpu` (libx264, default), `Vaapi` (`h264_vaapi`),
  `Nvenc` (`h264_nvenc`), `Qsv` (`h264_qsv`), `Amf` (`h264_amf`),
  `VideoToolbox` (`h264_videotoolbox`). String ids: `"cpu"`, `"vaapi"`, `"nvenc"`,
  `"qsv"`, `"amf"`, `"videotoolbox"`.
- Per-device knowledge, exposed as methods consumed by the arg builders:
  - `global_args()` — `Vaapi` needs `-init_hw_device vaapi=hw -filter_hw_device hw`.
  - `input_args()` — hardware **decode** flags placed before each `-i`:
    vaapi→`-hwaccel vaapi`, nvenc→`-hwaccel cuda`, videotoolbox→`-hwaccel videotoolbox`,
    amf→`-hwaccel d3d11va`. Decoded frames stay in system memory so existing software
    filters (scale/pad/concat) keep working; ffmpeg falls back to software decode per
    stream when a codec isn't hw-decodable. QSV stays encode-only (its decode path
    can't fall back cleanly).
  - `video_quality_args()` — quality-mode encode flags (replaces
    `-c:v libx264 -preset veryfast -crf 20`): vaapi `-qp 22`, nvenc `-preset p5 -cq 23`,
    qsv `-global_quality 23`, amf `-quality balanced -rc cqp -qp_i 22 -qp_p 24`,
    videotoolbox `-q:v 60`.
  - `video_bitrate_args(kbps)` — target-size mode: CPU `-b:v Nk` (caller adds two-pass
    flags); GPU `-b:v Nk -maxrate Nk -bufsize 2Nk` single pass.
  - `filter_suffix()` — vaapi encoders only accept hardware frames, so the video filter
    chain gains a trailing `format=nv12,hwupload` (as `-vf` where there is no filter
    graph, or appended after `concat` output for sequences).
  - `supports_two_pass()` — true only for CPU; hardware rate control is single-pass.
- The **Fast (lossless copy)** trim path is untouched — nothing is encoded there.

### Device detection + setting persistence

- `list_export_devices` Tauri command: for each OS-appropriate candidate
  (Linux: vaapi/nvenc/qsv; Windows: nvenc/amf/qsv; macOS: videotoolbox) run a tiny
  functional probe via the ffmpeg sidecar
  (`-f lavfi -i color=black:s=128x128:d=0.1 … -c:v <enc> -f null -`), because every
  encoder is compiled into the BtbN build regardless of the actual hardware. Results
  cached in managed state for the app's lifetime. CPU is always available.
- `SetupSettings` gains `export_device: Option<String>` (`#[serde(default)]` so
  existing settings.json files still parse). `relocate_watched_folder` preserves it
  when rewriting settings.
- `get_export_device` / `set_export_device` commands read/write it; export commands
  resolve the device server-side (missing/unknown/unconfigured → CPU).
- Settings UI: new "Export encoding" section in `SettingsModal` — a select listing
  CPU plus each detected device, unavailable ones disabled with a hint. Saves on
  change.

### Sequence target size (same Size dropdown)

- `export_sequence` gains `target_mb: Option<f64>`. The existing Size dropdown state
  in `App.tsx` now also drives sequence export: "Original quality" → CRF/quality mode
  as today; a size → `compute_target_bitrates(target_mb, total_duration)` and
  bitrate-mode encode. CPU: two-pass (same filter graph, pass 1 to the null sink,
  audio dropped); GPU: single pass with `-maxrate`/`-bufsize`.

### Sequence audio fix

- `Segment` gains `tracks: Vec<TrackConfig>` (`#[serde(default)]`). The frontend
  snapshots the clip's current track configs when "Add clip" is pressed
  (`SequenceSegment.tracks`), so each segment carries exactly what the preview mixed.
- Per segment the filter graph now builds:
  - no configs (legacy callers) → `[i:a]` as before;
  - one enabled track at unity → that stream directly;
  - multiple enabled tracks / non-unity volume → per-track `volume` + `amix`
    (mirrors `build_trim_args` semantics);
  - all tracks muted, or clip has no audio tracks at all → `anullsrc` silence trimmed
    to the segment length (keeps `concat` fed and fixes video-only segments too).
- Every segment's audio chain is normalized with
  `aresample=48000` + `aformat=sample_fmts=fltp:channel_layouts=stereo` before
  `concat` — concat requires identical audio params across segments, and mixed
  sources (or amix fltp output next to passthrough) would otherwise fail.

### Progress/phases

- `export_compressed` on GPU runs one pass ("Compressing", full bar) instead of two.
- Sequence with a size target on CPU: "Analyzing" [0, .5] + "Stitching" [.5, 1];
  GPU or no target: "Stitching" full-bar as today.

## Testing

- Rust unit tests: arg-builder coverage per device (vaapi init/hwupload placement,
  nvenc single-pass bitrate args, no `-pass` for GPU), sequence per-segment audio
  chains (multi-track amix, muted → anullsrc, normalization present), sequence
  bitrate/two-pass args, settings round-trip with and without `export_device`.
- TS unit tests: `toBackendSegments` carries `tracks`; size resolution reused for
  sequences.
- Existing integration tests keep running on CPU; no GPU-dependent CI tests.

## Out of scope

- HW-accelerated preview/proxy/thumbnail generation (preview already hw-decodes via
  GStreamer; proxies are rare one-offs).
- Full zero-copy GPU filter graphs (scale_vaapi/scale_cuda) — CPU filters keep the
  pipeline uniform and robust across drivers; decode+encode are the heavy parts.
- HEVC/AV1 output; exports stay H.264/AAC MP4.
