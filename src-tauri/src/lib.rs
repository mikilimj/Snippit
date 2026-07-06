mod probe;
pub mod bundle;
pub mod export;
pub mod setup;
pub mod library;
pub mod media;
pub mod relocate;
pub mod watcher;
pub mod remote;
pub mod sync;
mod media_server;

use export::{
    build_compress_pass_args, build_sequence_args, build_trim_args, build_trim_compatible_args,
    compute_target_bitrates, parse_progress_line, validate_trim, ExportDevice, MusicOverlay,
    Segment, SequenceTarget, TrackConfig,
};
use probe::{parse_ffprobe_json, ClipMeta};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tauri_plugin_shell::process::{CommandChild, CommandEvent};
use tauri_plugin_shell::ShellExt;

struct MediaPort(u16);

#[tauri::command]
fn get_media_port(state: tauri::State<MediaPort>) -> u16 {
    state.0
}

// ── Export encode device (GPU acceleration, 2026-07-05 design) ─────────────────

/// Cached hardware-encoder probe results — probing costs one tiny ffmpeg run per
/// candidate, and availability can't change without a driver swap, so once per
/// app launch is plenty.
#[derive(Default)]
struct DeviceProbe(tokio::sync::Mutex<Option<Vec<ExportDeviceInfo>>>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportDeviceInfo {
    id: String,
    label: String,
    available: bool,
}

/// Hardware encoder candidates worth probing on this OS.
fn device_candidates() -> Vec<ExportDevice> {
    if cfg!(target_os = "macos") {
        vec![ExportDevice::VideoToolbox]
    } else if cfg!(windows) {
        vec![ExportDevice::Nvenc, ExportDevice::Amf, ExportDevice::Qsv]
    } else {
        vec![ExportDevice::Vaapi, ExportDevice::Nvenc, ExportDevice::Qsv]
    }
}

/// True if `device` can actually encode on this machine. Every hardware encoder
/// is compiled into the sidecar regardless of the hardware present, so the only
/// reliable signal is a functional probe: encode a frame of black into the null
/// muxer and see whether ffmpeg exits cleanly.
async fn probe_device(app: &tauri::AppHandle, device: ExportDevice) -> bool {
    let mut args: Vec<String> = vec!["-v".into(), "error".into()];
    args.extend(device.global_args());
    args.extend([
        "-f".to_string(),
        "lavfi".to_string(),
        "-i".to_string(),
        "color=black:s=128x128:d=0.1".to_string(),
    ]);
    if let Some(suffix) = device.filter_suffix() {
        args.extend(["-vf".to_string(), suffix.to_string()]);
    }
    args.extend([
        "-c:v".to_string(),
        device.encoder().to_string(),
        "-f".to_string(),
        "null".to_string(),
        "-".to_string(),
    ]);
    let Ok(cmd) = app.shell().sidecar("ffmpeg") else {
        return false;
    };
    match cmd.args(args).output().await {
        Ok(out) => out.status.success(),
        Err(_) => false,
    }
}

/// Lists CPU + every probed hardware device, with availability. Probes once per
/// launch and serves the cached result afterwards.
#[tauri::command]
async fn list_export_devices(app: tauri::AppHandle) -> Result<Vec<ExportDeviceInfo>, String> {
    let state = app.state::<DeviceProbe>();
    let mut cache = state.0.lock().await;
    if let Some(cached) = cache.as_ref() {
        return Ok(cached.clone());
    }
    let mut devices = vec![ExportDeviceInfo {
        id: ExportDevice::Cpu.id().to_string(),
        label: ExportDevice::Cpu.label().to_string(),
        available: true,
    }];
    for candidate in device_candidates() {
        devices.push(ExportDeviceInfo {
            id: candidate.id().to_string(),
            label: candidate.label().to_string(),
            available: probe_device(&app, candidate).await,
        });
    }
    *cache = Some(devices.clone());
    Ok(devices)
}

/// The device id configured for exports ("cpu" when unset or before setup).
#[tauri::command]
fn get_export_device(app: tauri::AppHandle) -> String {
    current_export_device(&app).id().to_string()
}

/// Persists the export device choice into `<root>/settings.json`.
#[tauri::command]
fn set_export_device(app: tauri::AppHandle, device: String) -> Result<(), String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root = setup::read_pointer_file(&config_dir)
        .ok_or_else(|| "Library is not set up yet".to_string())?;
    let root_path = std::path::Path::new(&root);
    let mut settings =
        setup::read_settings(root_path).ok_or_else(|| "No settings found.".to_string())?;
    settings.export_device = Some(ExportDevice::from_id(&device).id().to_string());
    setup::write_settings(root_path, &settings)
}

/// Resolves the configured export device, defaulting to CPU when setup hasn't
/// run, the field is unset, or the id is unknown.
fn current_export_device(app: &tauri::AppHandle) -> ExportDevice {
    let device = app
        .path()
        .app_config_dir()
        .ok()
        .and_then(|dir| setup::read_pointer_file(&dir))
        .and_then(|root| setup::read_settings(std::path::Path::new(&root)))
        .and_then(|s| s.export_device);
    match device {
        Some(id) => ExportDevice::from_id(&id),
        None => ExportDevice::Cpu,
    }
}

/// Runs ffprobe on a path and parses its metadata. Shared by the `probe_clip`
/// command and the background auto-import path.
async fn probe_path(app: &tauri::AppHandle, path: &str) -> Result<ClipMeta, String> {
    let output = app
        .shell()
        .sidecar("ffprobe")
        .map_err(|e| e.to_string())?
        .args([
            "-v", "error",
            "-print_format", "json",
            "-show_format",
            "-show_streams",
            path,
        ])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    parse_ffprobe_json(&String::from_utf8_lossy(&output.stdout))
}

#[tauri::command]
async fn probe_clip(app: tauri::AppHandle, path: String) -> Result<ClipMeta, String> {
    probe_path(&app, &path).await
}

/// Extract individual audio tracks to temp M4A files so the frontend can play
/// them as separate <audio> elements for per-track live volume control.
/// Returns one absolute path per requested stream index, in the same order.
/// Reuses cached temp files if the same clip was prepared before.
#[tauri::command]
async fn prepare_audio_tracks(
    app: tauri::AppHandle,
    path: String,
    stream_indices: Vec<u32>,
) -> Result<Vec<String>, String> {
    let tmp_dir = std::env::temp_dir();
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    let path_hash = hasher.finish();

    let mut results = Vec::new();
    for &idx in &stream_indices {
        // "fs" marks faststart output — older cache files without it lack the
        // up-front moov atom and would force an extra end-of-file fetch.
        let out_path = tmp_dir.join(format!("clipsnip-a{idx}-{path_hash:x}-fs.m4a"));
        let out_str = out_path.to_string_lossy().to_string();

        if !out_path.exists() {
            let result = app
                .shell()
                .sidecar("ffmpeg")
                .map_err(|e| e.to_string())?
                .args([
                    "-y",
                    "-i", &path,
                    "-map", &format!("0:{idx}"),
                    "-vn",
                    "-c:a", "aac",
                    "-b:a", "192k",
                    // moov atom up front so playback starts from a single
                    // progressive request instead of seeking to EOF first.
                    "-movflags", "+faststart",
                    &out_str,
                ])
                .output()
                .await
                .map_err(|e| e.to_string())?;

            if !result.status.success() {
                return Err(String::from_utf8_lossy(&result.stderr).to_string());
            }
        }

        results.push(out_str);
    }
    Ok(results)
}

/// Decode one time window of an extracted preview audio track to 16-bit PCM WAV
/// bytes (raw ArrayBuffer on the JS side). The frontend schedules these chunks
/// on its single shared AudioContext instead of per-track <audio> elements —
/// each element registers its own audio stream with the OS mixer (WebKitGTK
/// keeps a muted system sink per element even when captured into Web Audio),
/// so chunked decode is what keeps the app down to one stream.
#[tauri::command]
async fn audio_chunk(
    app: tauri::AppHandle,
    path: String,
    start: f64,
    dur: f64,
) -> Result<tauri::ipc::Response, String> {
    if !(start.is_finite() && dur.is_finite() && dur > 0.0) {
        return Err(format!("invalid chunk window: start={start} dur={dur}"));
    }
    // ffmpeg writes the WAV to a unique temp file which is read back and
    // deleted. NEVER capture it from stdout: the shell plugin collects stdout
    // line by line and re-joins with '\n', which corrupts binary PCM into
    // deafening static (every 0x0A byte is treated as a line break).
    static CHUNK_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = CHUNK_SEQ.fetch_add(1, Ordering::Relaxed);
    let out = std::env::temp_dir().join(format!(
        "clipsnip-chunk-{}-{seq}.wav",
        std::process::id()
    ));
    let out_str = out.to_string_lossy().to_string();

    let result = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(media::build_audio_chunk_args(&path, start.max(0.0), dur, &out_str))
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        let _ = std::fs::remove_file(&out);
        return Err(String::from_utf8_lossy(&result.stderr).to_string());
    }
    let mut bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out);
    if !media::patch_wav_sizes(&mut bytes) {
        return Err("ffmpeg produced no decodable audio for this chunk".to_string());
    }
    Ok(tauri::ipc::Response::new(bytes))
}

/// Remux the current preview source into a cached video-only copy (stream copy,
/// no re-encode). Once Web Audio owns playback the <video> element is switched
/// to this file: with no audio track present, the WebView never builds an audio
/// pipeline for it, so the element stops registering a (muted) stream with the
/// OS mixer and the app is left with exactly one — the AudioContext's.
#[tauri::command]
async fn prepare_video_only(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    // Cached beside the proxies when the library exists; temp dir otherwise.
    let cache = match setup::read_pointer_file(&config_dir) {
        Some(root) => std::path::Path::new(&root).join("Cache").join("proxies"),
        None => std::env::temp_dir(),
    };
    let _ = std::fs::create_dir_all(&cache);
    let mtime = file_mtime_unix(std::path::Path::new(&path));
    let out = media::video_only_path(&cache, &path, mtime);
    let out_str = out.to_string_lossy().to_string();
    if out.exists() {
        return Ok(out_str); // reuse the cached remux
    }
    let result = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(media::build_video_only_args(&path, &out_str))
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        // Never leave a truncated file behind — it would be "cached" forever.
        let _ = std::fs::remove_file(&out);
        return Err(String::from_utf8_lossy(&result.stderr).to_string());
    }
    Ok(out_str)
}

/// The single in-flight export job: the spawned ffmpeg child (so it can be
/// killed) and a flag distinguishing a user cancel from a genuine failure. Only
/// one export runs at a time.
#[derive(Default)]
struct ExportJob {
    child: Mutex<Option<CommandChild>>,
    cancelled: AtomicBool,
}

/// Progress event streamed to the UI while an export runs (spec §4.5). Reused by
/// the folder relocation job (spec §4.7) via the `relocate-progress` event.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportProgress {
    fraction: f64,
    phase: String,
}

/// Cancel flag for the single in-flight folder relocation (spec §4.7).
#[derive(Default)]
struct RelocateJob(AtomicBool);

/// Cancel flag for the single in-flight library export/import bundle (spec §4.6).
#[derive(Default)]
struct BundleJob(AtomicBool);

/// Outcome of one ffmpeg run under progress/cancel supervision.
enum RunOutcome {
    Done,
    Cancelled,
}

/// Runs one ffmpeg invocation via the sidecar, parsing its `-progress` stream and
/// emitting `export-progress` events scaled into `[base, base + span]` of the
/// overall job. The spawned child is registered in `ExportJob` so `cancel_export`
/// can kill it. Returns `Cancelled` if the user cancelled, `Err` on ffmpeg
/// failure (with the tail of stderr).
async fn run_ffmpeg_with_progress(
    app: &tauri::AppHandle,
    args: Vec<String>,
    duration: f64,
    phase: &str,
    base: f64,
    span: f64,
) -> Result<RunOutcome, String> {
    // `-progress pipe:1` streams key=value status to stdout; `-nostats` silences
    // the noisy default stderr meter.
    let mut full = vec![
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
    ];
    full.extend(args);

    let (mut rx, child) = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(full)
        .spawn()
        .map_err(|e| e.to_string())?;

    *app.state::<ExportJob>().child.lock().unwrap() = Some(child);

    let mut success = false;
    let mut stderr_tail = String::new();
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(bytes) => {
                let line = String::from_utf8_lossy(&bytes);
                if let Some(frac) = parse_progress_line(&line, duration) {
                    let _ = app.emit(
                        "export-progress",
                        ExportProgress {
                            fraction: (base + span * frac).clamp(0.0, 1.0),
                            phase: phase.to_string(),
                        },
                    );
                }
            }
            CommandEvent::Stderr(bytes) => {
                stderr_tail.push_str(&String::from_utf8_lossy(&bytes));
                stderr_tail.push('\n');
                // Keep only the tail — ffmpeg errors are at the end.
                if stderr_tail.len() > 4000 {
                    stderr_tail.drain(0..stderr_tail.len() - 4000);
                }
            }
            CommandEvent::Terminated(payload) => success = payload.code == Some(0),
            _ => {}
        }
    }

    *app.state::<ExportJob>().child.lock().unwrap() = None;

    if app.state::<ExportJob>().cancelled.load(Ordering::SeqCst) {
        return Ok(RunOutcome::Cancelled);
    }
    if success {
        Ok(RunOutcome::Done)
    } else if stderr_tail.trim().is_empty() {
        Err("ffmpeg exited with an error".to_string())
    } else {
        Err(stderr_tail.trim().to_string())
    }
}

/// Cancels the running export (if any) by killing its ffmpeg child. The export
/// command then resolves with a neutral "cancelled" result rather than an error.
#[tauri::command]
fn cancel_export(app: tauri::AppHandle) {
    let job = app.state::<ExportJob>();
    job.cancelled.store(true, Ordering::SeqCst);
    let child = job.child.lock().unwrap().take();
    if let Some(child) = child {
        let _ = child.kill();
    }
}

/// Trim export. `mode` selects **Fast** (stream-copy, near-instant, keyframe cut
/// points) or **Compatible** (H.264/AAC re-encode for universal playback) per
/// spec §4.5. Progress is streamed and the job is cancellable.
#[tauri::command]
async fn export_trim(
    app: tauri::AppHandle,
    source: String,
    in_secs: f64,
    out_secs: f64,
    output: String,
    track_configs: Vec<TrackConfig>,
    mode: String,
) -> Result<String, String> {
    validate_trim(&source, &output, in_secs, out_secs)?;
    app.state::<ExportJob>()
        .cancelled
        .store(false, Ordering::SeqCst);

    let duration = out_secs - in_secs;
    let args = if mode == "compatible" {
        let device = current_export_device(&app);
        build_trim_compatible_args(&source, in_secs, out_secs, &output, &track_configs, &device)
    } else {
        // Fast mode stream-copies — no encode, so the device setting is irrelevant.
        build_trim_args(&source, in_secs, out_secs, &output, &track_configs)
    };

    match run_ffmpeg_with_progress(&app, args, duration, "Exporting", 0.0, 1.0).await? {
        RunOutcome::Done => Ok(output),
        RunOutcome::Cancelled => Err("Export cancelled.".to_string()),
    }
}

/// Two-pass H.264/AAC export that compresses the trimmed span to a target file
/// size (Discord presets). Pass 1 analyzes, pass 2 writes the real output; the
/// x264 pass log lives in the temp dir and is cleaned up on both success and
/// failure.
#[tauri::command]
async fn export_compressed(
    app: tauri::AppHandle,
    source: String,
    in_secs: f64,
    out_secs: f64,
    output: String,
    track_configs: Vec<TrackConfig>,
    target_mb: f64,
) -> Result<String, String> {
    validate_trim(&source, &output, in_secs, out_secs)?;
    let (video_kbps, audio_kbps) = compute_target_bitrates(target_mb, out_secs - in_secs)?;

    let mut hasher = DefaultHasher::new();
    output.hash(&mut hasher);
    let passlog = std::env::temp_dir().join(format!("clipsnip-2pass-{:x}", hasher.finish()));
    let passlog_str = passlog.to_string_lossy().to_string();

    app.state::<ExportJob>()
        .cancelled
        .store(false, Ordering::SeqCst);
    let duration = out_secs - in_secs;
    let device = current_export_device(&app);

    // On CPU, two passes map onto the overall bar as [0, 0.5] (analyze) and
    // [0.5, 1.0] (encode); hardware encoders can't two-pass, so they run a single
    // full-bar VBV-capped pass. Cancel/failure short-circuits, then the pass logs
    // are cleaned up.
    let passes: &[u8] = if device.supports_two_pass() { &[1, 2] } else { &[2] };
    let span = 1.0 / passes.len() as f64;
    let mut outcome: Result<RunOutcome, String> = Ok(RunOutcome::Done);
    for (idx, &pass) in passes.iter().enumerate() {
        let args = build_compress_pass_args(
            &source, in_secs, out_secs, &output, &track_configs,
            video_kbps, audio_kbps, pass, &passlog_str, &device,
        );
        let phase = if pass == 1 { "Analyzing" } else { "Compressing" };
        outcome =
            run_ffmpeg_with_progress(&app, args, duration, phase, idx as f64 * span, span).await;
        match &outcome {
            Ok(RunOutcome::Done) => {}
            _ => break, // cancelled or errored
        }
    }

    for suffix in ["-0.log", "-0.log.mbtree"] {
        let _ = std::fs::remove_file(format!("{passlog_str}{suffix}"));
    }

    match outcome? {
        RunOutcome::Done => Ok(output),
        RunOutcome::Cancelled => Err("Export cancelled.".to_string()),
    }
}

/// Stitches an ordered list of trimmed segments into one clip, optionally mixing
/// in a music overlay (spec §4.3). The output resolution is taken from the first
/// segment; the whole job re-encodes to H.264/AAC. With `target_mb` the sequence
/// is compressed to that size like a single-clip export (two-pass on CPU,
/// single-pass VBV on GPU). Progress/cancel as per §4.5.
#[tauri::command]
async fn export_sequence(
    app: tauri::AppHandle,
    segments: Vec<Segment>,
    music: Option<MusicOverlay>,
    output: String,
    target_mb: Option<f64>,
) -> Result<String, String> {
    if segments.is_empty() {
        return Err("Add at least one clip to the sequence first.".to_string());
    }
    let mut total = 0.0;
    for s in &segments {
        if s.in_secs >= s.out_secs {
            return Err(format!("Segment '{}' has an invalid in/out range.", s.input));
        }
        if output == s.input {
            return Err("Output path must differ from every source clip.".to_string());
        }
        total += s.out_secs - s.in_secs;
    }
    if let Some(m) = &music {
        if output == m.path {
            return Err("Output path must differ from the music file.".to_string());
        }
    }

    // The first segment sets the target frame; the rest are letterboxed to it.
    let first = probe_path(&app, &segments[0].input).await?;

    app.state::<ExportJob>()
        .cancelled
        .store(false, Ordering::SeqCst);
    let device = current_export_device(&app);

    let bitrates = match target_mb {
        Some(mb) => Some(compute_target_bitrates(mb, total)?),
        None => None,
    };

    let mut hasher = DefaultHasher::new();
    output.hash(&mut hasher);
    let passlog = std::env::temp_dir().join(format!("clipsnip-seq2pass-{:x}", hasher.finish()));
    let passlog_str = passlog.to_string_lossy().to_string();

    // Quality mode is one pass; a size target adds a CPU analysis pass first.
    let passes: &[u8] = match &bitrates {
        Some(_) if device.supports_two_pass() => &[1, 2],
        _ => &[2],
    };
    let span = 1.0 / passes.len() as f64;
    let mut outcome: Result<RunOutcome, String> = Ok(RunOutcome::Done);
    for (idx, &pass) in passes.iter().enumerate() {
        let target = bitrates.map(|(video_kbps, audio_kbps)| SequenceTarget {
            video_kbps,
            audio_kbps,
            pass,
            passlog: passlog_str.clone(),
        });
        let args = build_sequence_args(
            &segments, first.width, first.height, music.as_ref(), &output, &device,
            target.as_ref(),
        );
        let phase = if pass == 1 { "Analyzing" } else { "Stitching" };
        outcome =
            run_ffmpeg_with_progress(&app, args, total, phase, idx as f64 * span, span).await;
        match &outcome {
            Ok(RunOutcome::Done) => {}
            _ => break, // cancelled or errored
        }
    }

    if bitrates.is_some() {
        for suffix in ["-0.log", "-0.log.mbtree"] {
            let _ = std::fs::remove_file(format!("{passlog_str}{suffix}"));
        }
    }

    match outcome? {
        RunOutcome::Done => Ok(output),
        RunOutcome::Cancelled => Err("Export cancelled.".to_string()),
    }
}

/// The watched-mode source folder (spec §4.7), or None when there's nothing
/// relocatable (empty/import mode, or setup incomplete).
#[tauri::command]
fn get_watched_folder(app: tauri::AppHandle) -> Option<String> {
    let config_dir = app.path().app_config_dir().ok()?;
    let root = setup::read_pointer_file(&config_dir)?;
    let settings = setup::read_settings(std::path::Path::new(&root))?;
    if settings.ingestion_mode == "watch" {
        settings.source_folder
    } else {
        None
    }
}

/// Fraction complete for a `(done, total)` progress pair; `total == 0` reads as done.
fn progress_fraction(done: usize, total: usize) -> f64 {
    if total == 0 {
        1.0
    } else {
        done as f64 / total as f64
    }
}

/// Requests cancellation of the running folder relocation. Safe at any point —
/// every file is either fully moved or untouched (spec §4.7).
#[tauri::command]
fn cancel_relocate(app: tauri::AppHandle) {
    app.state::<RelocateJob>().0.store(true, Ordering::SeqCst);
}

/// Moves the watched source folder to `dest` (spec §4.7): an atomic rename on the
/// same volume, else a crash-safe file-by-file copy, keeping the index in sync.
/// Emits `relocate-progress`; updates the watched-folder setting on success (the
/// live watcher re-targets on next launch).
#[tauri::command]
async fn relocate_watched_folder(app: tauri::AppHandle, dest: String) -> Result<String, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root = setup::read_pointer_file(&config_dir)
        .ok_or_else(|| "Library is not set up yet".to_string())?;
    let root_path = std::path::PathBuf::from(&root);
    let settings =
        setup::read_settings(&root_path).ok_or_else(|| "No settings found.".to_string())?;
    if settings.ingestion_mode != "watch" {
        return Err("Only a watch-mode source folder can be relocated.".to_string());
    }
    let src = settings
        .source_folder
        .clone()
        .ok_or_else(|| "There is no watched folder to move.".to_string())?;
    if dest == src {
        return Err("The destination is the same as the current folder.".to_string());
    }
    let src_p = PathBuf::from(&src);
    let dest_p = PathBuf::from(&dest);
    if !src_p.exists() {
        return Err(format!("The source folder {src} no longer exists."));
    }
    if dest_p.starts_with(&src_p) {
        return Err("Cannot move a folder into itself.".to_string());
    }

    app.state::<RelocateJob>().0.store(false, Ordering::SeqCst);
    let fanout = if relocate::same_volume(&src_p, dest_p.parent().unwrap_or(&dest_p)) {
        relocate::Fanout::Rename
    } else {
        relocate::Fanout::CopyPerFile
    };

    let app2 = app.clone();
    let src_run = src_p.clone();
    let dest_run = dest_p.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let conn = open_library(&app2)?;
        relocate::relocate_folder(
            &conn,
            &src_run,
            &dest_run,
            fanout,
            &|| app2.state::<RelocateJob>().0.load(Ordering::SeqCst),
            &mut |done, total, cur| {
                let fraction = progress_fraction(done, total);
                let _ = app2.emit(
                    "relocate-progress",
                    ExportProgress {
                        fraction,
                        phase: format!(
                            "Moving {}",
                            cur.file_name().and_then(|n| n.to_str()).unwrap_or("…")
                        ),
                    },
                );
            },
        )
    })
    .await
    .map_err(|e| e.to_string())??;

    if matches!(outcome, relocate::Outcome::Cancelled) {
        return Err("Move cancelled.".to_string());
    }

    // Re-point the watched-folder setting at the new location.
    let updated = setup::SetupSettings {
        ingestion_mode: settings.ingestion_mode,
        source_folder: Some(dest.clone()),
        created_at: settings.created_at,
        export_device: settings.export_device,
        upload_size_percent: settings.upload_size_percent,
        local_size_percent: settings.local_size_percent,
        remote_provider: settings.remote_provider,
        remote_base_path: settings.remote_base_path,
        remote_identity: settings.remote_identity,
        tray_enabled: settings.tray_enabled,
    };
    setup::write_settings(&root_path, &updated)?;
    Ok(dest)
}

/// Requests cancellation of the running library export/import (spec §4.6).
#[tauri::command]
fn cancel_bundle(app: tauri::AppHandle) {
    app.state::<BundleJob>().0.store(true, Ordering::SeqCst);
}

/// Copies a directory tree file-by-file, mirroring structure, with progress and
/// cancel. Returns false if cancelled. Used to lay a bundle's clip files down at
/// the chosen import root.
fn copy_tree(
    src_root: &Path,
    dest_root: &Path,
    cancelled: &dyn Fn() -> bool,
    on_progress: &mut dyn FnMut(usize, usize),
) -> Result<bool, String> {
    let files = relocate::enumerate_files(src_root);
    let total = files.len();
    for (i, f) in files.iter().enumerate() {
        if cancelled() {
            return Ok(false);
        }
        let dest = relocate::dest_for(f, src_root, dest_root);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(f, &dest).map_err(|e| format!("copy {}: {e}", f.display()))?;
        on_progress(i + 1, total);
    }
    Ok(true)
}

/// Exports the library index + settings to a portable zip bundle at `dest` (spec
/// §4.6). With `include_files`, every referenced clip is copied into the bundle.
/// Clip paths are stored relative to the watched folder (or app root). Emits
/// `bundle-progress`; cancellable.
#[tauri::command]
async fn export_library(
    app: tauri::AppHandle,
    dest: String,
    include_files: bool,
) -> Result<String, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root = setup::read_pointer_file(&config_dir)
        .ok_or_else(|| "Library is not set up yet".to_string())?;
    let root_path = PathBuf::from(&root);
    let settings = setup::read_settings(&root_path);
    let relativize_root = settings
        .as_ref()
        .filter(|s| s.ingestion_mode == "watch")
        .and_then(|s| s.source_folder.clone())
        .unwrap_or_else(|| root.clone());
    let settings_json =
        std::fs::read_to_string(root_path.join("settings.json")).unwrap_or_else(|_| "{}".to_string());

    app.state::<BundleJob>().0.store(false, Ordering::SeqCst);
    let app2 = app.clone();
    let dest_path = PathBuf::from(&dest);
    let done = tauri::async_runtime::spawn_blocking(move || {
        let src_conn = open_library(&app2)?;
        let tmp = std::env::temp_dir().join(format!("clipsnip_export_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let bundle_db = tmp.join("library.db");
        let files = library::export_index(&src_conn, &bundle_db, &relativize_root)?;
        let clip_files: Vec<(String, PathBuf)> = if include_files {
            files.into_iter().map(|(rel, abs)| (rel, PathBuf::from(abs))).collect()
        } else {
            Vec::new()
        };
        let result = bundle::write_bundle(
            &dest_path,
            &bundle_db,
            &settings_json,
            &clip_files,
            &|| app2.state::<BundleJob>().0.load(Ordering::SeqCst),
            &mut |d, t| {
                let fraction = progress_fraction(d, t);
                let _ = app2.emit(
                    "bundle-progress",
                    ExportProgress { fraction, phase: format!("Bundling {d}/{t} files") },
                );
            },
        );
        let _ = std::fs::remove_dir_all(&tmp);
        result
    })
    .await
    .map_err(|e| e.to_string())??;

    if done {
        Ok(dest)
    } else {
        let _ = std::fs::remove_file(&dest);
        Err("Export cancelled.".to_string())
    }
}

/// Imports a bundle into the current library (spec §4.6): re-roots clip paths under
/// `new_root`, lays down any bundled clip files there, and merges (or replaces).
/// Emits `bundle-progress` and, on success, `library-changed`.
#[tauri::command]
async fn import_library(
    app: tauri::AppHandle,
    bundle_path: String,
    new_root: String,
    replace: bool,
) -> Result<library::ImportStats, String> {
    app.state::<BundleJob>().0.store(false, Ordering::SeqCst);
    let app2 = app.clone();
    let bundle_p = PathBuf::from(&bundle_path);
    let new_root_p = PathBuf::from(&new_root);
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let tmp = std::env::temp_dir().join(format!("clipsnip_import_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let bundle_db = bundle::read_bundle(&bundle_p, &tmp)?;

        // If the bundle carried clip files, lay them down under the chosen root.
        let clips_src = tmp.join("clips");
        if clips_src.is_dir() {
            let copied = copy_tree(
                &clips_src,
                &new_root_p,
                &|| app2.state::<BundleJob>().0.load(Ordering::SeqCst),
                &mut |d, t| {
                    let fraction = progress_fraction(d, t);
                    let _ = app2.emit(
                        "bundle-progress",
                        ExportProgress { fraction, phase: format!("Copying {d}/{t} files") },
                    );
                },
            )?;
            if !copied {
                let _ = std::fs::remove_dir_all(&tmp);
                return Ok::<Option<library::ImportStats>, String>(None);
            }
        }

        let dest_conn = open_library(&app2)?;
        let bundle_conn = library::open_db(&bundle_db)?;
        let stats = library::import_index(&dest_conn, &bundle_conn, &new_root, replace)?;
        let _ = std::fs::remove_dir_all(&tmp);
        Ok(Some(stats))
    })
    .await
    .map_err(|e| e.to_string())??;

    match outcome {
        Some(stats) => {
            let _ = app.emit("library-changed", ());
            Ok(stats)
        }
        None => Err("Import cancelled.".to_string()),
    }
}

/// Opens the library database that lives inside the configured app root
/// (`<root>/library.db`). Errors if setup hasn't run yet.
fn open_library(app: &tauri::AppHandle) -> Result<rusqlite::Connection, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root = setup::read_pointer_file(&config_dir)
        .ok_or_else(|| "Library is not set up yet".to_string())?;
    library::open_db(&std::path::Path::new(&root).join("library.db"))
}

#[tauri::command]
fn add_to_library(
    app: tauri::AppHandle,
    path: String,
    meta: ClipMeta,
) -> Result<library::ClipRow, String> {
    let conn = open_library(&app)?;
    library::upsert_clip(&conn, &path, &meta, None)
}

#[tauri::command]
fn list_library(app: tauri::AppHandle) -> Result<Vec<library::ClipRow>, String> {
    let conn = open_library(&app)?;
    library::list_clips(&conn)
}

#[tauri::command]
fn remove_from_library(app: tauri::AppHandle, id: i64) -> Result<(), String> {
    let conn = open_library(&app)?;
    library::remove_clip(&conn, id)
}

#[tauri::command]
fn set_clip_favorite(app: tauri::AppHandle, id: i64, favorite: bool) -> Result<(), String> {
    let conn = open_library(&app)?;
    library::set_favorite(&conn, id, favorite)
}

#[tauri::command]
fn rename_library_clip(app: tauri::AppHandle, id: i64, name: String) -> Result<(), String> {
    let conn = open_library(&app)?;
    library::rename_clip(&conn, id, &name)
}

#[tauri::command]
fn create_folder(app: tauri::AppHandle, name: String) -> Result<library::Folder, String> {
    let conn = open_library(&app)?;
    library::create_folder(&conn, &name)
}

#[tauri::command]
fn list_folders(app: tauri::AppHandle) -> Result<Vec<library::Folder>, String> {
    let conn = open_library(&app)?;
    library::list_folders(&conn)
}

#[tauri::command]
fn delete_folder(app: tauri::AppHandle, folder_id: i64) -> Result<(), String> {
    let conn = open_library(&app)?;
    library::delete_folder(&conn, folder_id)
}

#[tauri::command]
fn add_clip_to_folder(app: tauri::AppHandle, clip_id: i64, folder_id: i64) -> Result<(), String> {
    let conn = open_library(&app)?;
    library::add_clip_to_folder(&conn, clip_id, folder_id)
}

#[tauri::command]
fn remove_clip_from_folder(
    app: tauri::AppHandle,
    clip_id: i64,
    folder_id: i64,
) -> Result<(), String> {
    let conn = open_library(&app)?;
    library::remove_clip_from_folder(&conn, clip_id, folder_id)
}

#[tauri::command]
fn list_clips_in_folder(
    app: tauri::AppHandle,
    folder_id: i64,
) -> Result<Vec<library::ClipRow>, String> {
    let conn = open_library(&app)?;
    library::list_clips_in_folder(&conn, folder_id)
}

#[tauri::command]
fn list_unsorted_library(app: tauri::AppHandle) -> Result<Vec<library::ClipRow>, String> {
    let conn = open_library(&app)?;
    library::list_unsorted_clips(&conn)
}

/// Returns a preview proxy for `path`, generating a cached 720p H.264 MP4 if one
/// doesn't already exist (spec §4.4). Called by the UI only when the WebView
/// can't play the original natively; the original is still used for export.
#[tauri::command]
async fn ensure_proxy(app: tauri::AppHandle, path: String) -> Result<String, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root = setup::read_pointer_file(&config_dir)
        .ok_or_else(|| "Library is not set up yet".to_string())?;
    let cache = std::path::Path::new(&root).join("Cache").join("proxies");
    let _ = std::fs::create_dir_all(&cache);
    let mtime = file_mtime_unix(std::path::Path::new(&path));
    let proxy = media::proxy_path(&cache, &path, mtime);
    let proxy_str = proxy.to_string_lossy().to_string();
    if proxy.exists() {
        return Ok(proxy_str); // reuse the cached proxy
    }
    generate_proxy(&app, &path, &proxy_str).await?;
    Ok(proxy_str)
}

#[tauri::command]
fn get_app_root(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(setup::read_pointer_file(&config_dir))
}

#[tauri::command]
fn check_root_dir_status(path: String) -> Result<bool, String> {
    setup::is_dir_empty_or_absent(std::path::Path::new(&path))
}

#[tauri::command]
fn check_path_writable(path: String) -> bool {
    setup::is_writable(std::path::Path::new(&path))
}

#[tauri::command]
fn complete_setup(
    app: tauri::AppHandle,
    root_path: String,
    ingestion_mode: String,
    source_folder: Option<String>,
) -> Result<setup::SetupResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root = std::path::Path::new(&root_path);
    let source = source_folder.as_deref().map(std::path::Path::new);
    let result = setup::complete_setup_at(root, &config_dir, &ingestion_mode, source)?;
    // First-run: setup() couldn't start the watcher (no root yet), so start now.
    start_watching(&app);
    Ok(result)
}

// ── Auto-import: startup scan + live folder watcher (spec §4.2) ────────────────

/// Ensures the scan/watcher starts at most once (from `setup()` on a configured
/// launch, or from `complete_setup` on first run).
struct WatchStarted(AtomicBool);
/// Paths currently being ingested, so duplicate filesystem events are dropped.
struct InFlight(Arc<Mutex<HashSet<PathBuf>>>);

/// Timestamp (seconds) to grab the thumbnail frame from — a little into the clip,
/// but never past the end of very short clips.
fn thumb_timestamp(duration_secs: f64) -> f64 {
    (duration_secs * 0.1).clamp(0.0, 2.0)
}

async fn generate_thumbnail(
    app: &tauri::AppHandle,
    source: &str,
    out: &str,
    at_secs: f64,
) -> Result<(), String> {
    let args = media::build_thumbnail_args(source, out, at_secs, 320);
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
    Ok(())
}

/// Transcodes `source` into a 720p H.264 preview proxy at `out` (spec §4.4).
async fn generate_proxy(app: &tauri::AppHandle, source: &str, out: &str) -> Result<(), String> {
    let args = media::build_proxy_args(source, out);
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
    Ok(())
}

/// Polls a file's size until it settles (recorders write progressively). Returns
/// false if it never stabilizes within the timeout.
async fn wait_for_stable_size(path: &Path) -> bool {
    let mut samples: Vec<u64> = Vec::new();
    for _ in 0..40 {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        samples.push(size);
        if watcher::is_write_complete(&samples) {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    }
    false
}

fn file_mtime_unix(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Probes, thumbnails, and indexes one file. Returns the row, or None if the file
/// never stabilized or isn't probeable.
async fn ingest_file_inner(
    app: &tauri::AppHandle,
    path: &Path,
) -> Result<Option<library::ClipRow>, String> {
    if !wait_for_stable_size(path).await {
        return Ok(None);
    }
    let path_str = path.to_string_lossy().to_string();
    let meta = match probe_path(app, &path_str).await {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };

    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root = match setup::read_pointer_file(&config_dir) {
        Some(r) => r,
        None => return Ok(None),
    };

    let thumb_opt =
        ensure_thumbnail(app, Path::new(&root), &path_str, meta.duration_secs).await;

    let conn = library::open_db(&Path::new(&root).join("library.db"))?;
    let row = library::upsert_clip(&conn, &path_str, &meta, thumb_opt.as_deref())?;
    Ok(Some(row))
}

/// Ingests a file (de-duped against concurrent events) and emits `clip-added`.
async fn ingest_and_emit(
    app: tauri::AppHandle,
    path: PathBuf,
    in_flight: Arc<Mutex<HashSet<PathBuf>>>,
) {
    if !in_flight.lock().unwrap().insert(path.clone()) {
        return; // already being ingested from another event
    }
    let result = ingest_file_inner(&app, &path).await;
    in_flight.lock().unwrap().remove(&path);
    if let Ok(Some(row)) = result {
        let _ = app.emit("clip-added", row);
        // A fresh (or grown) clip may need uploading — plan it so new recordings
        // sync within seconds. No-ops when sync is off (no remote provider set).
        enqueue_sync_scan(&app);
    }
}

/// Resolves the configured watched folders, or None if setup hasn't run.
fn resolve_watched_folders(app: &tauri::AppHandle) -> Option<(String, Vec<PathBuf>)> {
    let config_dir = app.path().app_config_dir().ok()?;
    let root = setup::read_pointer_file(&config_dir)?;
    let (mode, source) = setup::read_settings(Path::new(&root))
        .map(|s| (s.ingestion_mode, s.source_folder))
        .unwrap_or_else(|| ("empty".to_string(), None));
    let folders = watcher::watched_folders(Path::new(&root), &mode, source.as_deref());
    Some((root, folders))
}

/// Scans every watched folder and ingests clips not already indexed.
fn scan_folders_now(app: &tauri::AppHandle) {
    let Some((root, folders)) = resolve_watched_folders(app) else {
        return;
    };
    let in_flight = app.state::<InFlight>().0.clone();
    let existing: HashSet<String> = library::open_db(&Path::new(&root).join("library.db"))
        .and_then(|c| library::list_clips(&c))
        .map(|rows| rows.into_iter().map(|r| r.path).collect())
        .unwrap_or_default();

    for folder in &folders {
        for file in watcher::scan_video_files(folder) {
            if existing.contains(&file.to_string_lossy().to_string()) {
                continue;
            }
            tauri::async_runtime::spawn(ingest_and_emit(app.clone(), file, in_flight.clone()));
        }
    }
}

/// Runs the initial scan and starts the live `notify` watcher. Idempotent.
fn start_watching(app: &tauri::AppHandle) {
    let started = app.state::<WatchStarted>();
    if started.0.swap(true, Ordering::SeqCst) {
        return; // already running
    }
    let Some((_root, folders)) = resolve_watched_folders(app) else {
        // Not configured yet; allow a later call after first-run setup.
        started.0.store(false, Ordering::SeqCst);
        return;
    };

    scan_folders_now(app);

    let in_flight = app.state::<InFlight>().0.clone();
    let app_for_thread = app.clone();
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
    let mut fs_watcher = match notify::recommended_watcher(move |res| {
        let _ = tx.send(res);
    }) {
        Ok(w) => w,
        Err(_) => return,
    };

    use notify::Watcher;
    for folder in &folders {
        let _ = std::fs::create_dir_all(folder);
        let _ = fs_watcher.watch(folder, notify::RecursiveMode::Recursive);
    }

    std::thread::spawn(move || {
        // Holding the watcher here keeps it alive; dropping it stops watching.
        let _keep_alive = fs_watcher;
        for res in rx {
            let Ok(event) = res else { continue };
            if !matches!(
                event.kind,
                notify::EventKind::Create(_) | notify::EventKind::Modify(_)
            ) {
                continue;
            }
            for path in event.paths {
                if !setup::is_video_extension(&path) {
                    continue;
                }
                tauri::async_runtime::spawn(ingest_and_emit(
                    app_for_thread.clone(),
                    path,
                    in_flight.clone(),
                ));
            }
        }
    });
}

/// Manually re-scans watched folders for clips added while the app was closed or
/// that a watcher event missed. New clips arrive via `clip-added` events.
#[tauri::command]
fn rescan_library(app: tauri::AppHandle) {
    scan_folders_now(&app);
}

/// Ensures a clip's thumbnail exists in the library thumbnail cache, returning its
/// path (or None if generation failed). Same flow used by ingest and by the sync
/// upload worker.
async fn ensure_thumbnail(
    app: &tauri::AppHandle,
    root: &Path,
    path_str: &str,
    duration_secs: f64,
) -> Option<String> {
    let cache = root.join("Cache").join("thumbnails");
    let _ = std::fs::create_dir_all(&cache);
    let thumb = media::thumbnail_path(&cache, path_str, file_mtime_unix(Path::new(path_str)));
    let thumb_str = thumb.to_string_lossy().to_string();
    if thumb.exists() {
        return Some(thumb_str);
    }
    if generate_thumbnail(app, path_str, &thumb_str, thumb_timestamp(duration_secs))
        .await
        .is_ok()
        && thumb.exists()
    {
        Some(thumb_str)
    } else {
        None
    }
}

// ── Remote storage sync (spec 2026-07-06) ─────────────────────────────────────
//
// One rclone remote (`clipsnip`) backs the whole feature; every upload path is
// derived, never persisted as a queue. Sync only ever *uploads* — it never
// deletes or downloads on its own. A single worker drains the derived queue one
// file at a time; a 300 s timer re-plans. See task-6 brief for the full design.

/// Transient sync state exposed to the UI. `provider` / counts are recomputed from
/// settings + DB on read; `state`/`message` live in the engine.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StorageStatus {
    provider: Option<String>,
    state: String, // "idle" | "syncing" | "error"
    message: Option<String>,
    synced_count: u32,
    pending_count: u32,
}

struct StorageStatusInner {
    state: String,
    message: Option<String>,
}

impl Default for StorageStatusInner {
    fn default() -> Self {
        StorageStatusInner {
            state: "idle".to_string(),
            message: None,
        }
    }
}

/// Managed sync state. The queue is *derived* (rebuilt by `enqueue_sync_scan`),
/// never persisted; `worker_running` guarantees a single in-flight uploader.
#[derive(Default)]
struct SyncEngine {
    queue: Mutex<VecDeque<PathBuf>>,
    worker_running: AtomicBool,
    status: Mutex<StorageStatusInner>,
    /// Per-file consecutive upload failures; a file is dropped after 5 until the
    /// next scan re-plans it.
    retries: Mutex<HashMap<PathBuf, u32>>,
}

/// The sync settings surfaced to the UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SyncSettings {
    upload_size_percent: Option<u8>,
    local_size_percent: Option<u8>,
    tray_enabled: bool,
}

/// One clip entry in the remote `library-index.json` (the portable, JSON-per-spec
/// mirror of the library metadata).
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct IndexClip {
    rel_path: String,
    display_name: String,
    favorite: bool,
    folders: Vec<String>,
    added_at: i64,
}

#[derive(Serialize, Deserialize)]
struct LibraryIndex {
    clips: Vec<IndexClip>,
}

/// Runs an rclone sidecar invocation, returning stdout on success and mapping a
/// non-zero exit to `Err(stderr)`. The single choke-point every remote op flows
/// through (Task 11 relies on it).
async fn run_rclone(app: &tauri::AppHandle, args: Vec<String>) -> Result<String, String> {
    let output = app
        .shell()
        .sidecar("rclone")
        .map_err(|e| e.to_string())?
        .args(args)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

/// Runs a plain (no-progress) ffmpeg invocation for the sync re-encode paths.
async fn run_ffmpeg_plain(app: &tauri::AppHandle, args: Vec<String>) -> Result<(), String> {
    let result = app
        .shell()
        .sidecar("ffmpeg")
        .map_err(|e| e.to_string())?
        .args(args)
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if result.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&result.stderr).to_string())
    }
}

/// Path of ClipSnip's private rclone config (never the user's own rclone setup).
fn rclone_config_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let _ = std::fs::create_dir_all(&config_dir);
    Ok(remote::config_path(&config_dir))
}

/// The configured library root, or an error if setup hasn't run.
fn resolve_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root =
        setup::read_pointer_file(&config_dir).ok_or_else(|| "Library is not set up yet".to_string())?;
    Ok(PathBuf::from(root))
}

/// Requires an active remote connection, returning `(root, base_path)`. Errors when
/// no provider is configured — every remote-facing command gates on this.
fn require_connection(app: &tauri::AppHandle) -> Result<(PathBuf, Option<String>), String> {
    let root = resolve_root(app)?;
    let settings = setup::read_settings(&root).ok_or_else(|| "No settings found.".to_string())?;
    if settings.remote_provider.is_none() {
        return Err("No remote storage is connected.".to_string());
    }
    Ok((root, settings.remote_base_path))
}

/// The remote path of the base directory itself (parent of `ClipSnip/`), used to
/// probe reachability with `lsd` before the ClipSnip tree is created.
fn remote_base_target(base: Option<&str>) -> String {
    let b = base.unwrap_or("").trim_matches('/');
    if b.is_empty() {
        format!("{}:", remote::REMOTE_NAME)
    } else {
        format!("{}:{}", remote::REMOTE_NAME, b)
    }
}

/// Joins a `/`-separated remote-relative name onto a local base, one path
/// component at a time (so it's a real nested path on every OS).
fn join_rel(base: &Path, rel: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in rel.split('/') {
        out.push(part);
    }
    out
}

/// The thumbnail stem for a clip's rel-name: its filename without extension.
/// Used identically on upload, download, and delete so the three agree.
fn thumb_stem(rel_name: &str) -> String {
    let file = rel_name.rsplit('/').next().unwrap_or(rel_name);
    match file.rsplit_once('.') {
        Some((stem, _)) => stem.to_string(),
        None => file.to_string(),
    }
}

/// Every video under `clips_dir`, paired with its size and mtime.
fn scan_local_files(clips_dir: &Path) -> Vec<(PathBuf, u64, i64)> {
    watcher::scan_video_files(clips_dir)
        .into_iter()
        .map(|p| {
            let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            let mtime = file_mtime_unix(&p);
            (p, size, mtime)
        })
        .collect()
}

/// Builds the current status snapshot (provider + counts recomputed from disk).
fn current_status(app: &tauri::AppHandle) -> StorageStatus {
    let engine = app.state::<SyncEngine>();
    let (state, message) = {
        let inner = engine.status.lock().unwrap();
        (inner.state.clone(), inner.message.clone())
    };
    let pending_count = engine.queue.lock().unwrap().len() as u32;
    let provider = resolve_root(app)
        .ok()
        .and_then(|r| setup::read_settings(&r))
        .and_then(|s| s.remote_provider);
    let synced_count = open_library(app)
        .ok()
        .and_then(|c| library::sync_state_list(&c).ok())
        .map(|v| v.len() as u32)
        .unwrap_or(0);
    StorageStatus {
        provider,
        state,
        message,
        synced_count,
        pending_count,
    }
}

/// Emits the current status snapshot on the `sync-status` channel.
fn emit_status(app: &tauri::AppHandle) {
    let status = current_status(app);
    let _ = app.emit("sync-status", status);
}

/// Updates the engine's transient state/message and emits `sync-status`.
fn set_status(app: &tauri::AppHandle, state: &str, message: Option<String>) {
    {
        let engine = app.state::<SyncEngine>();
        let mut inner = engine.status.lock().unwrap();
        inner.state = state.to_string();
        inner.message = message;
    }
    emit_status(app);
}

/// Serializes the library metadata to `<root>/Cache/library-index.json` (the
/// portable JSON mirror uploaded alongside the clips) and returns its path.
fn write_index_json(conn: &rusqlite::Connection, root: &Path) -> Result<String, String> {
    let clips_root = root.join("Clips");
    let clips_root_str = clips_root.to_string_lossy().to_string();

    // Clip → folder-name memberships, gathered once.
    let mut folder_map: HashMap<i64, Vec<String>> = HashMap::new();
    for f in library::list_folders(conn)? {
        for c in library::list_clips_in_folder(conn, f.id)? {
            folder_map.entry(c.id).or_default().push(f.name.clone());
        }
    }

    let mut clips = Vec::new();
    for c in library::list_clips(conn)? {
        clips.push(IndexClip {
            rel_path: bundle::relativize_path(&c.path, &clips_root_str),
            display_name: c.display_name,
            favorite: c.favorite,
            folders: folder_map.remove(&c.id).unwrap_or_default(),
            added_at: c.added_at,
        });
    }

    let json = serde_json::to_string_pretty(&LibraryIndex { clips }).map_err(|e| e.to_string())?;
    let cache = root.join("Cache");
    std::fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
    let path = cache.join("library-index.json");
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}

/// Rewrites and uploads the remote `library-index.json`.
async fn refresh_remote_index(
    app: &tauri::AppHandle,
    root: &Path,
    base: Option<&str>,
) -> Result<(), String> {
    let json_path = {
        let conn = open_library(app)?;
        write_index_json(&conn, root)?
    };
    let cfg = rclone_config_path(app)?;
    let remote = remote::remote_dir(base, "library-index.json");
    run_rclone(app, remote::copyto_args(&cfg, &json_path, &remote)).await?;
    Ok(())
}

/// Best-effort: downloads the remote `library-index.json` and applies its
/// `displayName`/`favorite`/`folders` to the just-imported clips (matched by
/// relPath). Missing folders are created. Never fails the import.
async fn apply_remote_index(
    app: &tauri::AppHandle,
    root: &Path,
    base: Option<&str>,
    imported: &[String],
) {
    let Ok(cfg) = rclone_config_path(app) else {
        return;
    };
    let cache = root.join("Cache");
    let _ = std::fs::create_dir_all(&cache);
    let local_json = cache.join("remote-library-index.json");
    let local_str = local_json.to_string_lossy().to_string();
    let remote = remote::remote_dir(base, "library-index.json");
    if run_rclone(app, remote::download_args(&cfg, &remote, &local_str))
        .await
        .is_err()
    {
        return; // no index on the remote yet — fine
    }
    let Ok(content) = std::fs::read_to_string(&local_json) else {
        return;
    };
    let Ok(index) = serde_json::from_str::<LibraryIndex>(&content) else {
        return;
    };
    let Ok(conn) = open_library(app) else {
        return;
    };
    let clips_root = root.join("Clips");
    let wanted: HashSet<&str> = imported.iter().map(|s| s.as_str()).collect();
    for entry in index.clips {
        if !wanted.contains(entry.rel_path.as_str()) {
            continue;
        }
        let abs = join_rel(&clips_root, &entry.rel_path)
            .to_string_lossy()
            .to_string();
        let Ok(Some(id)) = library::clip_id_by_path(&conn, &abs) else {
            continue;
        };
        let _ = library::rename_clip(&conn, id, &entry.display_name);
        let _ = library::set_favorite(&conn, id, entry.favorite);
        for fname in &entry.folders {
            let fid = match library::folder_id_by_name(&conn, fname) {
                Ok(Some(fid)) => fid,
                _ => match library::create_folder(&conn, fname) {
                    Ok(f) => f.id,
                    Err(_) => continue,
                },
            };
            let _ = library::add_clip_to_folder(&conn, id, fid);
        }
    }
}

/// Re-plans the derived upload queue: scans `Clips/`, compares against
/// `sync_state`, and enqueues novel/changed files (skipping ones already queued).
/// No-op when no remote provider is configured. Starts the worker afterwards.
fn enqueue_sync_scan(app: &tauri::AppHandle) {
    let Ok(root) = resolve_root(app) else {
        return;
    };
    let Some(settings) = setup::read_settings(&root) else {
        return;
    };
    if settings.remote_provider.is_none() {
        return;
    }
    let clips_dir = root.join("Clips");
    let local: Vec<(String, u64, i64)> = scan_local_files(&clips_dir)
        .into_iter()
        .map(|(p, s, m)| (p.to_string_lossy().to_string(), s, m))
        .collect();
    let state = match open_library(app).and_then(|c| library::sync_state_list(&c)) {
        Ok(s) => s,
        Err(_) => return,
    };
    let planned = sync::plan_uploads(&local, &state);
    {
        let engine = app.state::<SyncEngine>();
        let mut queue = engine.queue.lock().unwrap();
        let already: HashSet<PathBuf> = queue.iter().cloned().collect();
        for p in planned {
            let pb = PathBuf::from(p);
            if !already.contains(&pb) {
                queue.push_back(pb);
            }
        }
    }
    emit_status(app);
    start_sync_worker(app);
}

/// Starts the single upload worker if one isn't already running.
fn start_sync_worker(app: &tauri::AppHandle) {
    if app
        .state::<SyncEngine>()
        .worker_running
        .swap(true, Ordering::SeqCst)
    {
        return; // already draining
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        sync_worker(&app).await;
        app.state::<SyncEngine>()
            .worker_running
            .store(false, Ordering::SeqCst);
    });
}

/// Drains the derived queue one file at a time. On failure: retry counter++,
/// 5-strike drop, 30 s backoff, error status. Emits `sync-status` on every
/// transition.
async fn sync_worker(app: &tauri::AppHandle) {
    loop {
        let path = {
            let engine = app.state::<SyncEngine>();
            let mut q = engine.queue.lock().unwrap();
            q.pop_front()
        };
        let Some(path) = path else {
            break;
        };
        set_status(app, "syncing", None);
        match upload_one(app, &path).await {
            Ok(note) => {
                app.state::<SyncEngine>()
                    .retries
                    .lock()
                    .unwrap()
                    .remove(&path);
                let empty = app.state::<SyncEngine>().queue.lock().unwrap().is_empty();
                set_status(app, if empty { "idle" } else { "syncing" }, note);
            }
            Err(e) => {
                let count = {
                    let engine = app.state::<SyncEngine>();
                    let mut retries = engine.retries.lock().unwrap();
                    let c = retries.entry(path.clone()).or_insert(0);
                    *c += 1;
                    *c
                };
                if count >= 5 {
                    app.state::<SyncEngine>()
                        .retries
                        .lock()
                        .unwrap()
                        .remove(&path);
                    set_status(
                        app,
                        "error",
                        Some(format!(
                            "Gave up on {} after 5 attempts: {e}",
                            path.display()
                        )),
                    );
                } else {
                    app.state::<SyncEngine>()
                        .queue
                        .lock()
                        .unwrap()
                        .push_back(path.clone());
                    set_status(
                        app,
                        "error",
                        Some(format!("Upload failed (attempt {count}/5): {e}")),
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }
        }
    }
}

/// Uploads one clip: stabilize → probe → thumbnail → (optional) size re-encode →
/// upload video → upload thumbnail → refresh remote index → record `sync_state`
/// (with the ORIGINAL size/mtime) → (optional) local shrink. `sync_state` is only
/// written after the video upload's rclone exit 0. Returns an optional status note
/// (e.g. a full-size fallback) on success.
async fn upload_one(app: &tauri::AppHandle, path: &Path) -> Result<Option<String>, String> {
    let root = resolve_root(app)?;
    let settings = setup::read_settings(&root).ok_or_else(|| "No settings found.".to_string())?;
    let base = settings.remote_base_path.as_deref();
    let cfg = rclone_config_path(app)?;
    let clips_dir = root.join("Clips");
    let rel = sync::rel_name(&clips_dir, path).ok_or_else(|| "File is outside Clips/.".to_string())?;
    let device = current_export_device(app);

    // 1. Wait for the recorder to finish writing.
    if !wait_for_stable_size(path).await {
        return Err("File never stabilized.".to_string());
    }
    // 2. Probe (duration drives bitrate math + thumbnail).
    let path_str = path.to_string_lossy().to_string();
    let meta = probe_path(app, &path_str).await?;
    // Original identity, captured before any local shrink, for sync_state.
    let orig_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let orig_mtime = file_mtime_unix(path);

    // 3. Thumbnail.
    let thumb_path = ensure_thumbnail(app, &root, &path_str, meta.duration_secs).await;

    // 4. Optional upload-size re-encode; fall back to full-size below the floor.
    let mut note: Option<String> = None;
    let upload_path: String = if let Some(percent) = settings.upload_size_percent {
        let target_mb = media::percent_target_mb(orig_size, percent);
        match compute_target_bitrates(target_mb, meta.duration_secs) {
            Ok((video_kbps, audio_kbps)) => {
                let out = join_rel(&root.join("Cache").join("upload-tmp"), &rel);
                if let Some(parent) = out.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let out_str = out.to_string_lossy().to_string();
                run_ffmpeg_plain(
                    app,
                    media::build_size_reencode_args(
                        &path_str, &out_str, video_kbps, audio_kbps, &device,
                    ),
                )
                .await?;
                out_str
            }
            Err(e) => {
                note = Some(format!("Uploaded {rel} at full size ({e})"));
                path_str.clone()
            }
        }
    } else {
        path_str.clone()
    };

    // 5. Upload the video. sync_state is only recorded after this succeeds.
    let remote_video = remote::remote_dir(base, &format!("Clips/{rel}"));
    let upload_result = run_rclone(app, remote::copyto_args(&cfg, &upload_path, &remote_video)).await;
    if upload_path != path_str {
        let _ = std::fs::remove_file(&upload_path); // clean the temp re-encode
    }
    upload_result?;

    // 6. Upload the thumbnail (best-effort).
    if let Some(tp) = &thumb_path {
        let remote_thumb = remote::remote_dir(base, &format!("Thumbnails/{}.jpg", thumb_stem(&rel)));
        if let Err(e) = run_rclone(app, remote::copyto_args(&cfg, tp, &remote_thumb)).await {
            note = Some(append_note(note, format!("thumbnail upload failed: {e}")));
        }
    }

    // 7. Refresh the remote index (best-effort; video is already up).
    if let Err(e) = refresh_remote_index(app, &root, base).await {
        note = Some(append_note(note, format!("index refresh failed: {e}")));
    }

    // 8. Record sync_state with the ORIGINAL size/mtime.
    {
        let conn = open_library(app)?;
        library::sync_state_upsert(&conn, &path_str, orig_size, orig_mtime, &rel)?;
    }

    // 9. Optional local shrink (keeps the original untouched on any failure).
    if let Some(percent) = settings.local_size_percent {
        if let Err(e) =
            shrink_local(app, &root, path, &path_str, &rel, orig_size, meta.duration_secs, percent, &device)
                .await
        {
            note = Some(append_note(note, format!("local shrink skipped: {e}")));
        }
    }

    Ok(note)
}

/// Joins a secondary note onto an optional primary one.
fn append_note(existing: Option<String>, extra: String) -> String {
    match existing {
        Some(n) => format!("{n}; {extra}"),
        None => extra,
    }
}

/// Re-encodes the original in place to a size budget: encode to a temp file,
/// probe-validate it (must decode with a positive duration), then atomically
/// rename over the original (Cache and Clips share the library root volume) and
/// refresh the clip row + `sync_state` with the new size/mtime so the shrunk file
/// isn't re-uploaded. Any failure leaves the original intact.
#[allow(clippy::too_many_arguments)]
async fn shrink_local(
    app: &tauri::AppHandle,
    root: &Path,
    path: &Path,
    path_str: &str,
    rel: &str,
    orig_size: u64,
    duration_secs: f64,
    percent: u8,
    device: &ExportDevice,
) -> Result<(), String> {
    let target_mb = media::percent_target_mb(orig_size, percent);
    let (video_kbps, audio_kbps) = compute_target_bitrates(target_mb, duration_secs)?;

    let cache = root.join("Cache");
    let _ = std::fs::create_dir_all(&cache);
    let tmp = cache.join("shrink-tmp.mp4");
    let tmp_str = tmp.to_string_lossy().to_string();
    let _ = std::fs::remove_file(&tmp);

    let encode = run_ffmpeg_plain(
        app,
        media::build_size_reencode_args(path_str, &tmp_str, video_kbps, audio_kbps, device),
    )
    .await;
    if let Err(e) = encode {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }

    // Never replace the original until the re-encode is proven valid.
    let probed = match probe_path(app, &tmp_str).await {
        Ok(m) if m.duration_secs > 0.0 => m,
        Ok(_) => {
            let _ = std::fs::remove_file(&tmp);
            return Err("re-encode produced a zero-duration file".to_string());
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };

    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }

    let new_size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let new_mtime = file_mtime_unix(path);
    let conn = open_library(app)?;
    let _ = library::upsert_clip(&conn, path_str, &probed, None);
    library::sync_state_upsert(&conn, path_str, new_size, new_mtime, rel)?;
    Ok(())
}

/// Shared connect finalization: validate reachability, create the remote tree,
/// clear `sync_state` if the identity changed, persist provider/base/identity,
/// then plan an initial scan.
async fn finalize_connect(
    app: &tauri::AppHandle,
    provider: &str,
    identity_host: Option<&str>,
    user: Option<&str>,
    base_path: Option<String>,
) -> Result<(), String> {
    let cfg = rclone_config_path(app)?;
    let base = base_path.as_deref();

    // Reachability / credential check against the base dir (which exists before the
    // ClipSnip tree does).
    run_rclone(app, remote::lsd_check_args(&cfg, &remote_base_target(base)))
        .await
        .map_err(|e| format!("Could not reach the remote: {e}"))?;
    // Create the remote tree.
    run_rclone(app, remote::mkdir_args(&cfg, &remote::remote_dir(base, "Clips"))).await?;
    run_rclone(app, remote::mkdir_args(&cfg, &remote::remote_dir(base, "Thumbnails"))).await?;

    let identity = remote::identity(provider, identity_host, user, base);
    let root = resolve_root(app)?;
    let mut settings =
        setup::read_settings(&root).ok_or_else(|| "No settings found.".to_string())?;
    // A different remote than last time invalidates the whole upload ledger.
    if settings.remote_identity.as_deref() != Some(identity.as_str()) {
        let conn = open_library(app)?;
        library::sync_state_clear(&conn)?;
    }
    settings.remote_provider = Some(provider.to_string());
    settings.remote_base_path = base_path.clone();
    settings.remote_identity = Some(identity);
    setup::write_settings(&root, &settings)?;

    set_status(app, "idle", None);
    enqueue_sync_scan(app);
    Ok(())
}

/// Connects an SFTP remote (password or key-file auth), validates it, and starts
/// syncing.
#[tauri::command]
async fn storage_connect_sftp(
    app: tauri::AppHandle,
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    key_file: Option<String>,
    base_path: Option<String>,
) -> Result<(), String> {
    let cfg = rclone_config_path(&app)?;
    let auth = match (password, key_file) {
        (Some(p), _) if !p.is_empty() => remote::SftpAuth::Password(p),
        (_, Some(k)) if !k.is_empty() => remote::SftpAuth::KeyFile(k),
        _ => return Err("SFTP requires a password or a key file.".to_string()),
    };
    run_rclone(&app, remote::create_sftp_args(&cfg, &host, port, &user, &auth)).await?;
    let identity_host = format!("{host}:{port}");
    finalize_connect(&app, "sftp", Some(&identity_host), Some(&user), base_path).await
}

/// Connects an OAuth remote (Dropbox / Google Drive). rclone opens the system
/// browser and blocks until the user approves — this can take minutes.
#[tauri::command]
async fn storage_connect_oauth(
    app: tauri::AppHandle,
    provider: String,
    base_path: Option<String>,
) -> Result<(), String> {
    let cfg = rclone_config_path(&app)?;
    set_status(&app, "syncing", Some("Waiting for browser authorization…".to_string()));
    let result = run_rclone(&app, remote::create_oauth_args(&cfg, &provider)).await;
    if let Err(e) = result {
        set_status(&app, "error", Some(format!("Authorization failed: {e}")));
        return Err(e);
    }
    finalize_connect(&app, &provider, None, None, base_path).await
}

/// Disconnects the remote: deletes the rclone remote config, clears
/// provider/base and the queue, but KEEPS `remote_identity` and `sync_state` so
/// reconnecting the same remote resumes rather than re-uploading.
#[tauri::command]
async fn storage_disconnect(app: tauri::AppHandle) -> Result<(), String> {
    let cfg = rclone_config_path(&app)?;
    let _ = run_rclone(&app, remote::delete_remote_args(&cfg)).await; // best-effort
    let root = resolve_root(&app)?;
    if let Some(mut settings) = setup::read_settings(&root) {
        settings.remote_provider = None;
        settings.remote_base_path = None;
        setup::write_settings(&root, &settings)?;
    }
    {
        let engine = app.state::<SyncEngine>();
        engine.queue.lock().unwrap().clear();
        engine.retries.lock().unwrap().clear();
    }
    set_status(&app, "idle", None);
    Ok(())
}

/// Current sync status snapshot.
#[tauri::command]
fn storage_status(app: tauri::AppHandle) -> Result<StorageStatus, String> {
    Ok(current_status(&app))
}

/// Lists the remote/local diff for the "what's on each side" view. Fails fast on
/// any rclone error (no partial diff).
#[tauri::command]
async fn storage_scan_diff(app: tauri::AppHandle) -> Result<sync::SyncDiff, String> {
    let (root, base) = require_connection(&app)?;
    let cfg = rclone_config_path(&app)?;
    let json = run_rclone(
        &app,
        remote::lsjson_args(&cfg, &remote::remote_dir(base.as_deref(), "Clips")),
    )
    .await?;
    let remote_files = remote::parse_lsjson(&json)?;
    let clips_dir = root.join("Clips");
    let local: Vec<(String, u64, i64)> = scan_local_files(&clips_dir)
        .into_iter()
        .filter_map(|(p, s, m)| sync::rel_name(&clips_dir, &p).map(|rel| (rel, s, m)))
        .collect();
    Ok(sync::diff(&local, &remote_files))
}

/// Downloads + ingests the named remote clips (never automatic — user-initiated),
/// recording `sync_state` and applying the remote index metadata. Returns the
/// number successfully imported.
#[tauri::command]
async fn storage_import_clips(app: tauri::AppHandle, rel_names: Vec<String>) -> Result<u32, String> {
    let (root, base) = require_connection(&app)?;
    let cfg = rclone_config_path(&app)?;
    let clips_dir = root.join("Clips");
    let mut imported = 0u32;

    for rel in &rel_names {
        let remote = remote::remote_dir(base.as_deref(), &format!("Clips/{rel}"));
        let local = join_rel(&clips_dir, rel);
        if let Some(parent) = local.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let local_str = local.to_string_lossy().to_string();
        run_rclone(&app, remote::download_args(&cfg, &remote, &local_str)).await?;

        if let Ok(Some(row)) = ingest_file_inner(&app, &local).await {
            let _ = app.emit("clip-added", row);
            let size = std::fs::metadata(&local).map(|m| m.len()).unwrap_or(0);
            let mtime = file_mtime_unix(&local);
            let conn = open_library(&app)?;
            library::sync_state_upsert(&conn, &local_str, size, mtime, rel)?;
            imported += 1;
        }
    }

    apply_remote_index(&app, &root, base.as_deref(), &rel_names).await;
    emit_status(&app);
    Ok(imported)
}

/// Deletes a clip from the remote: the video, its thumbnail (best-effort), the
/// `sync_state` row, and refreshes the remote index.
#[tauri::command]
async fn storage_delete_remote(app: tauri::AppHandle, rel_name: String) -> Result<(), String> {
    let (root, base) = require_connection(&app)?;
    let cfg = rclone_config_path(&app)?;
    run_rclone(
        &app,
        remote::deletefile_args(&cfg, &remote::remote_dir(base.as_deref(), &format!("Clips/{rel_name}"))),
    )
    .await?;
    let _ = run_rclone(
        &app,
        remote::deletefile_args(
            &cfg,
            &remote::remote_dir(base.as_deref(), &format!("Thumbnails/{}.jpg", thumb_stem(&rel_name))),
        ),
    )
    .await;
    {
        let conn = open_library(&app)?;
        library::sync_state_remove_by_remote_name(&conn, &rel_name)?;
    }
    refresh_remote_index(&app, &root, base.as_deref()).await?;
    emit_status(&app);
    Ok(())
}

/// Fetches a remote clip's thumbnail into `Cache/remote-thumbs/` and returns the
/// local path (served lazily in the diff window via the media server).
#[tauri::command]
async fn storage_download_thumb(app: tauri::AppHandle, rel_name: String) -> Result<String, String> {
    let (root, base) = require_connection(&app)?;
    let cfg = rclone_config_path(&app)?;
    let stem = thumb_stem(&rel_name);
    let dir = root.join("Cache").join("remote-thumbs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let local = dir.join(format!("{stem}.jpg"));
    let local_str = local.to_string_lossy().to_string();
    let remote = remote::remote_dir(base.as_deref(), &format!("Thumbnails/{stem}.jpg"));
    run_rclone(&app, remote::download_args(&cfg, &remote, &local_str)).await?;
    Ok(local_str)
}

/// Fire-and-forget: re-plan the upload queue now (e.g. from a "Sync now" button).
#[tauri::command]
fn sync_now(app: tauri::AppHandle) {
    enqueue_sync_scan(&app);
}

/// Reads the sync-related settings (size targets + tray toggle).
#[tauri::command]
fn get_sync_settings(app: tauri::AppHandle) -> Result<SyncSettings, String> {
    let root = resolve_root(&app)?;
    let s = setup::read_settings(&root).ok_or_else(|| "No settings found.".to_string())?;
    Ok(SyncSettings {
        upload_size_percent: s.upload_size_percent,
        local_size_percent: s.local_size_percent,
        tray_enabled: s.tray_enabled.unwrap_or(true),
    })
}

/// Persists the sync-related settings. Percents, when set, must be integers in
/// 1–99 (IPC input is not trusted even though the UI validates too).
#[tauri::command]
fn set_sync_settings(
    app: tauri::AppHandle,
    upload_size_percent: Option<u8>,
    local_size_percent: Option<u8>,
    tray_enabled: bool,
) -> Result<(), String> {
    for p in [upload_size_percent, local_size_percent].into_iter().flatten() {
        if !(1..=99).contains(&p) {
            return Err(format!("Size percent must be between 1 and 99 (got {p})."));
        }
    }
    let root = resolve_root(&app)?;
    let mut s = setup::read_settings(&root).ok_or_else(|| "No settings found.".to_string())?;
    s.upload_size_percent = upload_size_percent;
    s.local_size_percent = local_size_percent;
    s.tray_enabled = Some(tray_enabled);
    setup::write_settings(&root, &s)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's EGL platform-display init aborts with "Could not create
    // default EGL display: EGL_BAD_PARAMETER" inside our AppImage, leaving a
    // blank gray window. Root cause: linuxdeploy bundles our own
    // libwayland-client.so (from the ubuntu-22.04 CI runner), which is older
    // than the host's Wayland/Mesa stack; WebKit's GPU/Web process (which
    // re-execs, so it picks up env vars set here) loads the mismatched
    // bundled copy instead of the host's and fails to create the display.
    // Disabling the DMABUF/compositing renderers alone does not avoid this -
    // preloading the host's libwayland-client.so does. Only applied for
    // AppImage runs (APPIMAGE env var, set by the AppImage runtime) so
    // native/deb builds keep GPU-accelerated rendering with the correctly
    // matched system libs.
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");

        if std::env::var_os("LD_PRELOAD").is_none() {
            let candidates = [
                "/usr/lib/x86_64-linux-gnu/libwayland-client.so.0",
                "/usr/lib64/libwayland-client.so.0",
                "/usr/lib/libwayland-client.so.0",
            ];
            if let Some(host_lib) = candidates
                .iter()
                .find(|path| std::path::Path::new(path).exists())
            {
                std::env::set_var("LD_PRELOAD", host_lib);
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let port = media_server::start();
            app.manage(MediaPort(port));
            app.manage(WatchStarted(AtomicBool::new(false)));
            app.manage(InFlight(Arc::new(Mutex::new(HashSet::new()))));
            app.manage(ExportJob::default());
            app.manage(RelocateJob::default());
            app.manage(BundleJob::default());
            app.manage(DeviceProbe::default());
            app.manage(SyncEngine::default());
            // Start auto-import if already configured; a first-run launch starts
            // it from complete_setup instead.
            start_watching(app.handle());
            // Resume any pending uploads planned before the last shutdown.
            enqueue_sync_scan(app.handle());
            // Periodic re-plan: pick up library additions and sync uploads the
            // live watcher may have missed. 300 s per spec.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                    scan_folders_now(&handle);
                    enqueue_sync_scan(&handle);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe_clip,
            export_trim,
            export_compressed,
            export_sequence,
            cancel_export,
            get_media_port,
            prepare_audio_tracks,
            audio_chunk,
            prepare_video_only,
            get_app_root,
            check_root_dir_status,
            check_path_writable,
            complete_setup,
            add_to_library,
            list_library,
            remove_from_library,
            rescan_library,
            set_clip_favorite,
            rename_library_clip,
            create_folder,
            list_folders,
            delete_folder,
            add_clip_to_folder,
            remove_clip_from_folder,
            list_clips_in_folder,
            list_unsorted_library,
            ensure_proxy,
            get_watched_folder,
            relocate_watched_folder,
            cancel_relocate,
            export_library,
            import_library,
            cancel_bundle,
            list_export_devices,
            get_export_device,
            set_export_device,
            storage_connect_sftp,
            storage_connect_oauth,
            storage_disconnect,
            storage_status,
            storage_scan_diff,
            storage_import_clips,
            storage_delete_remote,
            storage_download_thumb,
            sync_now,
            get_sync_settings,
            set_sync_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
