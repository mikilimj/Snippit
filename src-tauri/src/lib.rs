mod probe;
pub mod export;
pub mod setup;
pub mod library;
pub mod media;
pub mod watcher;
mod media_server;

use export::{
    build_compress_pass_args, build_sequence_args, build_trim_args, build_trim_compatible_args,
    compute_target_bitrates, parse_progress_line, validate_trim, MusicOverlay, Segment, TrackConfig,
};
use probe::{parse_ffprobe_json, ClipMeta};
use serde::Serialize;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
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
        let out_path = tmp_dir.join(format!("clipsnip-a{idx}-{path_hash:x}.m4a"));
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

/// The single in-flight export job: the spawned ffmpeg child (so it can be
/// killed) and a flag distinguishing a user cancel from a genuine failure. Only
/// one export runs at a time.
#[derive(Default)]
struct ExportJob {
    child: Mutex<Option<CommandChild>>,
    cancelled: AtomicBool,
}

/// Progress event streamed to the UI while an export runs (spec §4.5).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExportProgress {
    fraction: f64,
    phase: String,
}

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
        build_trim_compatible_args(&source, in_secs, out_secs, &output, &track_configs)
    } else {
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

    // Two passes map onto the overall bar as [0, 0.5] (analyze) and [0.5, 1.0]
    // (encode). Cancel/failure short-circuits, then the pass logs are cleaned up.
    let mut outcome: Result<RunOutcome, String> = Ok(RunOutcome::Done);
    for (idx, pass) in [1u8, 2u8].into_iter().enumerate() {
        let args = build_compress_pass_args(
            &source, in_secs, out_secs, &output, &track_configs,
            video_kbps, audio_kbps, pass, &passlog_str,
        );
        let phase = if pass == 1 { "Analyzing" } else { "Compressing" };
        outcome = run_ffmpeg_with_progress(&app, args, duration, phase, idx as f64 * 0.5, 0.5).await;
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
/// segment; the whole job re-encodes to H.264/AAC. Progress/cancel as per §4.5.
#[tauri::command]
async fn export_sequence(
    app: tauri::AppHandle,
    segments: Vec<Segment>,
    music: Option<MusicOverlay>,
    output: String,
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
    let args = build_sequence_args(&segments, first.width, first.height, music.as_ref(), &output);
    match run_ffmpeg_with_progress(&app, args, total, "Stitching", 0.0, 1.0).await? {
        RunOutcome::Done => Ok(output),
        RunOutcome::Cancelled => Err("Export cancelled.".to_string()),
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

    let cache = Path::new(&root).join("Cache").join("thumbnails");
    let _ = std::fs::create_dir_all(&cache);
    let thumb = media::thumbnail_path(&cache, &path_str, file_mtime_unix(path));
    let thumb_str = thumb.to_string_lossy().to_string();
    let thumb_opt = if thumb.exists() {
        Some(thumb_str)
    } else if generate_thumbnail(app, &path_str, &thumb_str, thumb_timestamp(meta.duration_secs))
        .await
        .is_ok()
        && thumb.exists()
    {
        Some(thumb_str)
    } else {
        None
    };

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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMABUF/EGL renderer fails to create an EGL display inside our
    // AppImage (its bundled libEGL/Mesa is older than the host driver), which
    // aborts the web process and leaves a blank gray window. Disable that
    // renderer only for AppImage runs so native/deb builds keep GPU-accelerated
    // rendering. The APPIMAGE env var is set by the AppImage runtime.
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
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
            // Start auto-import if already configured; a first-run launch starts
            // it from complete_setup instead.
            start_watching(app.handle());
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
            ensure_proxy
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
