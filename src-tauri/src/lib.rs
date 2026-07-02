mod probe;
pub mod export;
pub mod setup;
mod media_server;

use export::{build_compress_pass_args, build_trim_args, compute_target_bitrates, validate_trim, TrackConfig};
use probe::{parse_ffprobe_json, ClipMeta};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use tauri::Manager;
use tauri_plugin_shell::ShellExt;

struct MediaPort(u16);

#[tauri::command]
fn get_media_port(state: tauri::State<MediaPort>) -> u16 {
    state.0
}

#[tauri::command]
async fn probe_clip(app: tauri::AppHandle, path: String) -> Result<ClipMeta, String> {
    let output = app
        .shell()
        .sidecar("ffprobe")
        .map_err(|e| e.to_string())?
        .args([
            "-v", "error",
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

#[tauri::command]
async fn export_trim(
    app: tauri::AppHandle,
    source: String,
    in_secs: f64,
    out_secs: f64,
    output: String,
    track_configs: Vec<TrackConfig>,
) -> Result<String, String> {
    validate_trim(&source, &output, in_secs, out_secs)?;
    let args = build_trim_args(&source, in_secs, out_secs, &output, &track_configs);
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

    let mut run_error: Option<String> = None;
    for pass in [1u8, 2u8] {
        let args = build_compress_pass_args(
            &source, in_secs, out_secs, &output, &track_configs,
            video_kbps, audio_kbps, pass, &passlog_str,
        );
        let result = app
            .shell()
            .sidecar("ffmpeg")
            .map_err(|e| e.to_string())?
            .args(args)
            .output()
            .await
            .map_err(|e| e.to_string());
        match result {
            Ok(r) if r.status.success() => {}
            Ok(r) => {
                run_error = Some(String::from_utf8_lossy(&r.stderr).to_string());
                break;
            }
            Err(e) => {
                run_error = Some(e);
                break;
            }
        }
    }

    for suffix in ["-0.log", "-0.log.mbtree"] {
        let _ = std::fs::remove_file(format!("{passlog_str}{suffix}"));
    }

    match run_error {
        Some(e) => Err(e),
        None => Ok(output),
    }
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
    setup::complete_setup_at(root, &config_dir, &ingestion_mode, source)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let port = media_server::start();
            app.manage(MediaPort(port));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            probe_clip,
            export_trim,
            export_compressed,
            get_media_port,
            prepare_audio_tracks,
            get_app_root,
            check_root_dir_status,
            check_path_writable,
            complete_setup
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
