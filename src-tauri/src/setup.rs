use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const VIDEO_EXTENSIONS: [&str; 6] = ["mp4", "mkv", "mov", "avi", "webm", "flv"];

pub fn is_video_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SetupSettings {
    pub ingestion_mode: String,
    pub source_folder: Option<String>,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupResult {
    pub root_path: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct RootPointer {
    root_path: String,
}

/// Creates Clips/, Exports/, Cache/ under `root` (idempotent).
pub fn create_subfolders(root: &Path) -> Result<(), String> {
    for name in ["Clips", "Exports", "Cache"] {
        fs::create_dir_all(root.join(name))
            .map_err(|e| format!("Failed to create {name}/: {e}"))?;
    }
    Ok(())
}

/// True if `path` doesn't exist yet, or exists as an empty directory.
/// Err if it exists but isn't a directory, or can't be read.
pub fn is_dir_empty_or_absent(path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(true);
    }
    if !path.is_dir() {
        return Err(format!("{} exists and is not a directory", path.display()));
    }
    let mut entries = fs::read_dir(path).map_err(|e| e.to_string())?;
    Ok(entries.next().is_none())
}

/// Copies top-level (non-recursive) files from `source` whose extension matches a
/// recognized video container into `clips_dir`. Non-matching files are silently
/// skipped. Per-file copy failures are collected into the warnings list rather than
/// aborting the whole operation; a failure to even read `source` itself is fatal.
pub fn copy_video_files(source: &Path, clips_dir: &Path) -> Result<(usize, Vec<String>), String> {
    let entries =
        fs::read_dir(source).map_err(|e| format!("Cannot read {}: {e}", source.display()))?;
    let mut copied = 0usize;
    let mut warnings = Vec::new();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warnings.push(format!("Failed to read a directory entry: {e}"));
                continue;
            }
        };
        let path = entry.path();
        if !path.is_file() || !is_video_extension(&path) {
            continue;
        }
        let dest = clips_dir.join(entry.file_name());
        match fs::copy(&path, &dest) {
            Ok(_) => copied += 1,
            Err(e) => warnings.push(format!(
                "Failed to copy {}: {e}",
                entry.file_name().to_string_lossy()
            )),
        }
    }

    Ok((copied, warnings))
}

/// Best-effort writability probe: attempts to create `path` (if missing) and write
/// then remove a marker file inside it. Used to block the wizard's Continue button
/// on an unwritable root before the user finishes the rest of the flow — real
/// failures during `complete_setup_at` are still handled defensively on top of this.
pub fn is_writable(path: &Path) -> bool {
    if fs::create_dir_all(path).is_err() {
        return false;
    }
    let probe = path.join(".clipsnip_write_test");
    match fs::write(&probe, b"") {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

pub fn write_settings(root: &Path, settings: &SetupSettings) -> Result<(), String> {
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(root.join("settings.json"), json).map_err(|e| e.to_string())
}

pub fn write_pointer_file(config_dir: &Path, root_path: &str) -> Result<(), String> {
    fs::create_dir_all(config_dir).map_err(|e| e.to_string())?;
    let pointer = RootPointer {
        root_path: root_path.to_string(),
    };
    let json = serde_json::to_string_pretty(&pointer).map_err(|e| e.to_string())?;
    fs::write(config_dir.join("root.json"), json).map_err(|e| e.to_string())
}

/// Returns None if the pointer file is missing, unreadable, or malformed — all
/// treated identically as "never set up".
pub fn read_pointer_file(config_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(config_dir.join("root.json")).ok()?;
    let pointer: RootPointer = serde_json::from_str(&content).ok()?;
    Some(pointer.root_path)
}

/// Reads `<root>/settings.json`. None if it's missing or unparseable.
pub fn read_settings(root: &Path) -> Option<SetupSettings> {
    let content = fs::read_to_string(root.join("settings.json")).ok()?;
    serde_json::from_str(&content).ok()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Orchestrates first-run setup: creates the root + subfolders, copies clips in
/// import mode, writes settings.json and the OS-level pointer file.
pub fn complete_setup_at(
    root: &Path,
    config_dir: &Path,
    ingestion_mode: &str,
    source_folder: Option<&Path>,
) -> Result<SetupResult, String> {
    fs::create_dir_all(root).map_err(|e| format!("Cannot create {}: {e}", root.display()))?;
    create_subfolders(root)?;

    let mut warnings = Vec::new();
    if ingestion_mode == "import" {
        let source =
            source_folder.ok_or_else(|| "Import mode requires a source folder".to_string())?;
        let (_copied, copy_warnings) = copy_video_files(source, &root.join("Clips"))?;
        warnings.extend(copy_warnings);
    }

    let settings = SetupSettings {
        ingestion_mode: ingestion_mode.to_string(),
        source_folder: source_folder.map(|p| p.to_string_lossy().to_string()),
        created_at: now_unix(),
    };
    write_settings(root, &settings)?;
    write_pointer_file(config_dir, &root.to_string_lossy())?;

    Ok(SetupResult {
        root_path: root.to_string_lossy().to_string(),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_subdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "clipsnip_setup_test_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn is_video_extension_matches_known_containers_case_insensitively() {
        assert!(is_video_extension(Path::new("clip.MP4")));
        assert!(is_video_extension(Path::new("clip.mkv")));
        assert!(is_video_extension(Path::new("clip.WebM")));
    }

    #[test]
    fn is_video_extension_rejects_others() {
        assert!(!is_video_extension(Path::new("notes.txt")));
        assert!(!is_video_extension(Path::new("clip")));
    }

    #[test]
    fn create_subfolders_creates_clips_exports_cache() {
        let root = temp_subdir("subfolders");
        create_subfolders(&root).unwrap();
        assert!(root.join("Clips").is_dir());
        assert!(root.join("Exports").is_dir());
        assert!(root.join("Cache").is_dir());
    }

    #[test]
    fn is_dir_empty_or_absent_true_for_missing_path() {
        let root = temp_subdir("absent_parent");
        let missing = root.join("does-not-exist");
        assert!(is_dir_empty_or_absent(&missing).unwrap());
    }

    #[test]
    fn is_dir_empty_or_absent_true_for_empty_dir() {
        let dir = temp_subdir("empty_dir");
        assert!(is_dir_empty_or_absent(&dir).unwrap());
    }

    #[test]
    fn is_dir_empty_or_absent_false_when_populated() {
        let dir = temp_subdir("populated_dir");
        fs::write(dir.join("file.txt"), b"hi").unwrap();
        assert!(!is_dir_empty_or_absent(&dir).unwrap());
    }

    #[test]
    fn copy_video_files_copies_only_recognized_extensions() {
        let source = temp_subdir("copy_source");
        let dest = temp_subdir("copy_dest");
        fs::write(source.join("a.mp4"), b"video").unwrap();
        fs::write(source.join("notes.txt"), b"text").unwrap();
        let (copied, warnings) = copy_video_files(&source, &dest).unwrap();
        assert_eq!(copied, 1);
        assert!(warnings.is_empty());
        assert!(dest.join("a.mp4").exists());
        assert!(!dest.join("notes.txt").exists());
    }

    #[test]
    fn copy_video_files_collects_per_file_failures_without_aborting() {
        let source = temp_subdir("copy_fail_source");
        let dest = temp_subdir("copy_fail_dest");
        fs::write(source.join("good.mp4"), b"video").unwrap();
        fs::write(source.join("bad.mkv"), b"video").unwrap();
        // Make the destination for "bad.mkv" a directory so fs::copy onto it fails,
        // deterministically simulating a per-file copy failure without OS permission
        // hacks.
        fs::create_dir(dest.join("bad.mkv")).unwrap();
        let (copied, warnings) = copy_video_files(&source, &dest).unwrap();
        assert_eq!(copied, 1);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("bad.mkv"));
    }

    #[test]
    fn is_writable_true_for_a_normal_creatable_dir() {
        let dir = temp_subdir("writable_probe");
        assert!(is_writable(&dir));
        // Note: a deliberately-unwritable-path case isn't asserted here — permission
        // semantics are environment-dependent (e.g. tests running as root bypass
        // Unix permission checks), which would make that assertion flaky in CI.
    }

    #[test]
    fn write_and_read_pointer_file_round_trips() {
        let config_dir = temp_subdir("pointer_config");
        write_pointer_file(&config_dir, "/some/root").unwrap();
        assert_eq!(
            read_pointer_file(&config_dir),
            Some("/some/root".to_string())
        );
    }

    #[test]
    fn read_pointer_file_returns_none_when_missing() {
        let config_dir = temp_subdir("pointer_missing");
        assert_eq!(read_pointer_file(&config_dir), None);
    }

    #[test]
    fn read_settings_round_trips_what_write_settings_wrote() {
        let root = temp_subdir("read_settings");
        let settings = SetupSettings {
            ingestion_mode: "watch".to_string(),
            source_folder: Some("/rec/out".to_string()),
            created_at: 42,
        };
        write_settings(&root, &settings).unwrap();
        let read = read_settings(&root).unwrap();
        assert_eq!(read.ingestion_mode, "watch");
        assert_eq!(read.source_folder, Some("/rec/out".to_string()));
    }

    #[test]
    fn read_settings_returns_none_when_missing() {
        let root = temp_subdir("read_settings_missing");
        assert!(read_settings(&root).is_none());
    }

    #[test]
    fn complete_setup_at_empty_mode_writes_settings_and_pointer_without_copying() {
        let root = temp_subdir("complete_empty_root");
        let config_dir = temp_subdir("complete_empty_config");
        let result = complete_setup_at(&root, &config_dir, "empty", None).unwrap();
        assert_eq!(result.root_path, root.to_string_lossy());
        assert!(result.warnings.is_empty());
        assert!(root.join("Clips").is_dir());
        assert_eq!(
            read_pointer_file(&config_dir),
            Some(root.to_string_lossy().to_string())
        );
        let settings_json = fs::read_to_string(root.join("settings.json")).unwrap();
        let settings: SetupSettings = serde_json::from_str(&settings_json).unwrap();
        assert_eq!(settings.ingestion_mode, "empty");
        assert_eq!(settings.source_folder, None);
    }

    #[test]
    fn complete_setup_at_import_mode_copies_files_from_source() {
        let root = temp_subdir("complete_import_root");
        let config_dir = temp_subdir("complete_import_config");
        let source = temp_subdir("complete_import_source");
        fs::write(source.join("clip.mp4"), b"video").unwrap();
        let result = complete_setup_at(&root, &config_dir, "import", Some(&source)).unwrap();
        assert!(result.warnings.is_empty());
        assert!(root.join("Clips").join("clip.mp4").exists());
        let settings_json = fs::read_to_string(root.join("settings.json")).unwrap();
        let settings: SetupSettings = serde_json::from_str(&settings_json).unwrap();
        assert_eq!(
            settings.source_folder,
            Some(source.to_string_lossy().to_string())
        );
    }

    #[test]
    fn complete_setup_at_import_mode_without_source_folder_errors() {
        let root = temp_subdir("complete_import_missing_root");
        let config_dir = temp_subdir("complete_import_missing_config");
        assert!(complete_setup_at(&root, &config_dir, "import", None).is_err());
    }
}
