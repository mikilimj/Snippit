mod probe;
pub mod export;
mod media_server;

use export::{build_trim_args, validate_trim, TrackConfig};
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
            get_media_port,
            prepare_audio_tracks
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
