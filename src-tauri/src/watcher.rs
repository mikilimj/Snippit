//! Folder watching + auto-import (spec §4.2). Pure helpers (recursive scan,
//! write-completion detection) are unit tested; the live `notify` watcher thread
//! and per-file ingestion orchestration are wired in `lib.rs`, which owns the
//! Tauri `AppHandle` needed to run FFmpeg and emit events.

use crate::setup::is_video_extension;
use std::fs;
use std::path::{Path, PathBuf};

/// Recursively collects every recognized video file under `dir` (recorders often
/// use date-based subfolders). A missing/unreadable directory yields an empty
/// list rather than an error.
pub fn scan_video_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    scan_into(dir, &mut out);
    out
}

fn scan_into(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            scan_into(&path, out);
        } else if is_video_extension(&path) {
            out.push(path);
        }
    }
}

/// The set of folders to scan and watch, derived from setup settings. `Clips/`
/// under the root is always included (import mode copies here, and it's the
/// natural drop folder); a `watch`-mode source folder is watched in place too.
/// Duplicates (e.g. a source folder equal to `Clips/`) are collapsed.
pub fn watched_folders(
    root: &Path,
    ingestion_mode: &str,
    source_folder: Option<&str>,
) -> Vec<PathBuf> {
    let mut folders = vec![root.join("Clips")];
    if ingestion_mode == "watch" {
        if let Some(src) = source_folder {
            let src = PathBuf::from(src);
            if !folders.contains(&src) {
                folders.push(src);
            }
        }
    }
    folders
}

/// A file is considered fully written once its size has settled: the two most
/// recent size samples are equal and non-zero. Recorders write progressively, so
/// polling until this holds avoids ingesting a half-written clip (spec §4.2).
pub fn is_write_complete(samples: &[u64]) -> bool {
    match samples {
        [.., a, b] => a == b && *b > 0,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_subdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "snippit_watcher_test_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scan_video_files_finds_recursively_and_skips_non_video() {
        let root = temp_subdir("scan");
        fs::write(root.join("a.mp4"), b"v").unwrap();
        fs::write(root.join("notes.txt"), b"t").unwrap();
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::write(root.join("sub/b.mkv"), b"v").unwrap();
        fs::write(root.join("sub/deep/c.MOV"), b"v").unwrap();

        let mut found: Vec<String> = scan_video_files(&root)
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        found.sort();
        assert_eq!(found, vec!["a.mp4", "b.mkv", "c.MOV"]);
    }

    #[test]
    fn scan_video_files_on_missing_dir_is_empty() {
        let root = temp_subdir("scan_missing");
        assert!(scan_video_files(&root.join("nope")).is_empty());
    }

    #[test]
    fn watched_folders_always_includes_clips_plus_watch_source() {
        let root = Path::new("/root");
        // watch mode references the source folder in place, alongside Clips/
        let w = watched_folders(root, "watch", Some("/rec"));
        assert!(w.contains(&root.join("Clips")));
        assert!(w.contains(&PathBuf::from("/rec")));
        assert_eq!(w.len(), 2);
        // import mode already copied files into Clips/, so don't also watch source
        assert_eq!(watched_folders(root, "import", Some("/rec")), vec![root.join("Clips")]);
        // empty mode: just Clips/
        assert_eq!(watched_folders(root, "empty", None), vec![root.join("Clips")]);
    }

    #[test]
    fn watched_folders_dedups_source_equal_to_clips() {
        let root = Path::new("/root");
        let clips = root.join("Clips");
        let w = watched_folders(root, "watch", Some(clips.to_str().unwrap()));
        assert_eq!(w, vec![clips]);
    }

    #[test]
    fn write_complete_needs_two_equal_nonzero_samples() {
        assert!(!is_write_complete(&[]));
        assert!(!is_write_complete(&[100]));
        assert!(!is_write_complete(&[0, 0])); // zero-length file never "complete"
        assert!(!is_write_complete(&[100, 200])); // still growing
        assert!(is_write_complete(&[200, 200])); // size settled
        assert!(is_write_complete(&[100, 200, 200])); // settled after growth
    }
}
