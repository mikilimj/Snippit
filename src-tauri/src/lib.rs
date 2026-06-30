mod probe;
pub mod export;

use export::build_trim_copy_args;
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![probe_clip, export_trim])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
