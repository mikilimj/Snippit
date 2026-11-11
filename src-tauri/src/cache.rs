//! Temp-directory hygiene.
//!
//! The app drops work files in the OS temp dir: extracted preview-audio tracks
//! (`snippit-a<idx>-<hash>-fs.m4a`), per-chunk WAVs (`snippit-chunk-<pid>-<seq>.wav`),
//! two-pass encode logs and the scratch dirs of export/import/bundle runs. They
//! are keyed by content so re-opening a clip can reuse them, which means nothing
//! removes them while the app runs — and close-to-tray means most sessions never
//! reach a clean exit either. Left alone, /tmp accumulates a copy of every audio
//! track of every clip ever previewed.
//!
//! Two halves, because either one alone leaks: a startup sweep deletes stale
//! leftovers (the crash/kill case), and a clean exit deletes the artifacts the
//! running session created. The sweep only ever touches names this app makes, so
//! a user's own files in the temp dir are never at risk.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How old a leftover must be before the startup sweep may delete it. A day is
/// long enough that a clip opened yesterday still reuses its extracted audio and
/// short enough that interrupted runs don't pile up.
pub const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Scratch entry names the app creates itself. Underscore forms are directories
/// (export/import/bundle runs), hyphen forms are files (audio tracks, chunk
/// WAVs, two-pass logs) — cargo-test artifacts like `snippit_src.mp4` are
/// deliberately *not* matched.
const ARTIFACT_PREFIXES: [&str; 5] = [
    "snippit_export_",
    "snippit_import_",
    "snippit_bundle_",
    "snippit-2pass-",
    "snippit-seq2pass-",
];

/// Whether `name` is a temp artifact this app created (never somebody's file).
pub fn is_temp_artifact(name: &str) -> bool {
    if ARTIFACT_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return true;
    }
    // Extracted preview-audio track, and one chunk's decoded WAV.
    (name.starts_with("snippit-a") && name.ends_with("-fs.m4a"))
        || (name.starts_with("snippit-chunk-") && name.ends_with(".wav"))
}

/// Deletes `is_temp_artifact` entries in `dir` whose mtime is older than
/// `max_age`, returning how many were removed. Per-entry errors are ignored: a
/// leftover that can't be read or was deleted mid-scan must never stop the app
/// from starting.
pub fn sweep_dir(dir: &Path, now: SystemTime, max_age: Duration) -> usize {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_temp_artifact(name) {
            continue;
        }
        // No mtime (vanished, or a future timestamp from a clock jump) => leave it.
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        let Ok(age) = now.duration_since(modified) else {
            continue;
        };
        if age < max_age {
            continue;
        }
        if remove_entry(&entry.path()) {
            removed += 1;
        }
    }
    removed
}

/// Sweeps the OS temp dir with the default `MAX_AGE`.
pub fn sweep_temp_dir() -> usize {
    sweep_dir(&std::env::temp_dir(), SystemTime::now(), MAX_AGE)
}

/// Best-effort delete of the artifacts one session created (files or dirs).
pub fn remove_paths(paths: &[PathBuf]) -> usize {
    paths.iter().filter(|p| remove_entry(p)).count()
}

/// Removes a file or a whole directory tree.
fn remove_entry(path: &Path) -> bool {
    let is_dir = std::fs::symlink_metadata(path)
        .map(|m| m.is_dir())
        .unwrap_or(false);
    if is_dir {
        std::fs::remove_dir_all(path).is_ok()
    } else {
        std::fs::remove_file(path).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn test_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("snippit_cache_test_{}_{}", std::process::id(), tag));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("test dir");
        dir
    }

    #[test]
    fn recognizes_only_its_own_artifacts() {
        for name in [
            "snippit-a1-1b9d927bff749269-fs.m4a",
            "snippit-a0-ff-fs.m4a",
            "snippit-chunk-4242-7.wav",
            "snippit-2pass-9a1f",
            "snippit-seq2pass-1f2e.log",
            "snippit_export_123",
            "snippit_import_123",
            "snippit_bundle_123",
        ] {
            assert!(is_temp_artifact(name), "{name} should be swept");
        }
        for name in [
            "clip.mp4",
            "notes.txt",
            "snippit_src.mp4",
            "snippit_mig_mtime_123",
            "snippit-a1.m4a",
            "snippit-chunk-1.txt",
        ] {
            assert!(!is_temp_artifact(name), "{name} must be left alone");
        }
    }

    #[test]
    fn sweeps_stale_artifacts_and_keeps_everything_else() {
        let dir = test_dir("stale");
        let stale_track = dir.join("snippit-a1-deadbeef-fs.m4a");
        let stale_chunk = dir.join("snippit-chunk-1-1.wav");
        let stale_dir = dir.join("snippit_export_1");
        let keep = dir.join("keep.txt");
        fs::write(&stale_track, b"x").expect("write track");
        fs::write(&stale_chunk, b"x").expect("write chunk");
        fs::create_dir(&stale_dir).expect("scratch dir");
        fs::write(&keep, b"x").expect("write keep");

        // A max_age of zero makes everything that matches stale.
        assert_eq!(sweep_dir(&dir, SystemTime::now(), Duration::ZERO), 3);
        assert!(!stale_track.exists());
        assert!(!stale_chunk.exists());
        assert!(!stale_dir.exists());
        assert!(keep.exists(), "non-artifacts must survive the sweep");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn leaves_fresh_artifacts_for_the_cache_to_reuse() {
        let dir = test_dir("fresh");
        let fresh = dir.join("snippit-a0-deadbeef-fs.m4a");
        fs::write(&fresh, b"x").expect("write");

        assert_eq!(
            sweep_dir(&dir, SystemTime::now(), Duration::from_secs(3600)),
            0
        );
        assert!(fresh.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn removes_the_paths_one_session_created() {
        let dir = test_dir("session");
        let track = dir.join("snippit-a0-1234-fs.m4a");
        fs::write(&track, b"x").expect("write");

        let gone = dir.join("snippit-a0-gone-fs.m4a");
        assert_eq!(remove_paths(&[track.clone(), gone]), 1);
        assert!(!track.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_dir_is_not_an_error() {
        let missing = std::env::temp_dir().join("snippit_cache_test_never_created");
        assert_eq!(sweep_dir(&missing, SystemTime::now(), Duration::ZERO), 0);
    }
}
