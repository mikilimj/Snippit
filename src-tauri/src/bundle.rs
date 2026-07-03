//! Portable library bundle (spec §4.6): pack the SQLite index + settings into a
//! single zip whose clip paths are stored **relative** to a chosen root, so they
//! can be re-rooted on another device. This module holds the pure path-rewriting
//! rules and the zip pack/unpack; the row copying lives in `library`.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Whether `p` is an absolute path (unix `/…`, or Windows `C:\…` / `C:/…`).
/// Absolute paths are left untouched by re-rooting.
pub fn is_absolute_path(p: &str) -> bool {
    if p.starts_with('/') {
        return true;
    }
    let bytes = p.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Rewrite an absolute clip path to one relative to `root` (forward-slashed for
/// portability). Paths not under `root` are returned unchanged — on import they
/// stay absolute and are handled by the missing-clip system if not found.
pub fn relativize_path(abs: &str, root: &str) -> String {
    let root_trimmed = root.trim_end_matches(['/', '\\']);
    if let Some(rest) = abs.strip_prefix(root_trimmed) {
        let rel = rest.trim_start_matches(['/', '\\']);
        if !rel.is_empty() {
            return rel.replace('\\', "/");
        }
    }
    abs.to_string()
}

/// Re-root a stored clip path under `new_root`. Relative paths are joined under the
/// new root; already-absolute paths are kept as-is.
pub fn absolutize_path(stored: &str, new_root: &str) -> String {
    if is_absolute_path(stored) {
        return stored.to_string();
    }
    let root_trimmed = new_root.trim_end_matches(['/', '\\']);
    format!("{root_trimmed}/{}", stored.trim_start_matches(['/', '\\']))
}

/// Write a bundle zip containing the index DB, settings JSON, and (optionally) the
/// referenced clip files. `clip_files` are (archive_name, source_path) pairs added
/// under `clips/` — supply an empty slice for the settings-only bundle. `cancelled`
/// is polled between files; on cancel the partial zip is removed and `Ok(false)` is
/// returned. `on_file` reports (done, total) as clip files are added.
pub fn write_bundle(
    dest_zip: &Path,
    db_path: &Path,
    settings_json: &str,
    clip_files: &[(String, PathBuf)],
    cancelled: &dyn Fn() -> bool,
    on_file: &mut dyn FnMut(usize, usize),
) -> Result<bool, String> {
    let file = fs::File::create(dest_zip).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("library.db", opts).map_err(|e| e.to_string())?;
    io::copy(&mut fs::File::open(db_path).map_err(|e| e.to_string())?, &mut zip)
        .map_err(|e| e.to_string())?;

    zip.start_file("settings.json", opts).map_err(|e| e.to_string())?;
    io::copy(&mut settings_json.as_bytes(), &mut zip).map_err(|e| e.to_string())?;

    let total = clip_files.len();
    for (i, (name, src)) in clip_files.iter().enumerate() {
        if cancelled() {
            drop(zip);
            let _ = fs::remove_file(dest_zip);
            return Ok(false);
        }
        zip.start_file(format!("clips/{name}"), opts)
            .map_err(|e| e.to_string())?;
        let mut src_file = fs::File::open(src).map_err(|e| format!("read {}: {e}", src.display()))?;
        io::copy(&mut src_file, &mut zip).map_err(|e| format!("read {}: {e}", src.display()))?;
        on_file(i + 1, total);
    }

    zip.finish().map_err(|e| e.to_string())?;
    Ok(true)
}

/// Extract a bundle zip into `out_dir`, returning the extracted `library.db` path.
pub fn read_bundle(zip_path: &Path, out_dir: &Path) -> Result<PathBuf, String> {
    let file = fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let rel = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => continue, // reject path-traversal entries
        };
        let out_path = out_dir.join(&rel);
        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out_file = fs::File::create(&out_path).map_err(|e| e.to_string())?;
        io::copy(&mut entry, &mut out_file).map_err(|e| e.to_string())?;
    }

    let db = out_dir.join("library.db");
    if !db.exists() {
        return Err("Bundle is missing its library.db.".to_string());
    }
    Ok(db)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_absolute_path_detects_unix_and_windows_roots() {
        assert!(is_absolute_path("/home/a.mp4"));
        assert!(is_absolute_path("C:\\clips\\a.mp4"));
        assert!(is_absolute_path("D:/clips/a.mp4"));
        assert!(!is_absolute_path("sub/a.mp4"));
        assert!(!is_absolute_path("a.mp4"));
    }

    #[test]
    fn relativize_strips_root_and_normalizes_separators() {
        assert_eq!(relativize_path("/root/sub/a.mp4", "/root"), "sub/a.mp4");
        assert_eq!(relativize_path("/root/a.mp4", "/root/"), "a.mp4");
        assert_eq!(relativize_path("C:\\root\\sub\\a.mp4", "C:\\root"), "sub/a.mp4");
    }

    #[test]
    fn relativize_leaves_paths_outside_the_root_untouched() {
        assert_eq!(relativize_path("/elsewhere/a.mp4", "/root"), "/elsewhere/a.mp4");
    }

    #[test]
    fn absolutize_rejoins_relative_and_keeps_absolute() {
        assert_eq!(absolutize_path("sub/a.mp4", "/new"), "/new/sub/a.mp4");
        assert_eq!(absolutize_path("sub/a.mp4", "/new/"), "/new/sub/a.mp4");
        // An absolute stored path is not re-rooted.
        assert_eq!(absolutize_path("/abs/a.mp4", "/new"), "/abs/a.mp4");
    }

    #[test]
    fn relativize_then_absolutize_round_trips_under_a_new_root() {
        let rel = relativize_path("/old/games/clip.mp4", "/old");
        assert_eq!(rel, "games/clip.mp4");
        assert_eq!(absolutize_path(&rel, "/new/place"), "/new/place/games/clip.mp4");
    }

    #[test]
    fn write_then_read_bundle_round_trips_files() {
        let dir = std::env::temp_dir().join(format!("clipsnip_bundle_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("library.db");
        fs::write(&db, b"SQLITE-BYTES").unwrap();
        let clip = dir.join("a.mp4");
        fs::write(&clip, b"VIDEO").unwrap();
        let zip_path = dir.join("bundle.zip");

        let ok = write_bundle(
            &zip_path,
            &db,
            "{\"ingestionMode\":\"watch\"}",
            &[("games/a.mp4".to_string(), clip.clone())],
            &|| false,
            &mut |_d, _t| {},
        )
        .unwrap();
        assert!(ok);

        let out = dir.join("unpacked");
        let extracted_db = read_bundle(&zip_path, &out).unwrap();
        assert_eq!(fs::read(&extracted_db).unwrap(), b"SQLITE-BYTES");
        assert_eq!(fs::read(out.join("settings.json")).unwrap(), b"{\"ingestionMode\":\"watch\"}");
        assert_eq!(fs::read(out.join("clips/games/a.mp4")).unwrap(), b"VIDEO");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_bundle_cancel_removes_partial_zip() {
        let dir = std::env::temp_dir().join(format!("clipsnip_bundle_cancel_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let db = dir.join("library.db");
        fs::write(&db, b"DB").unwrap();
        let clip = dir.join("a.mp4");
        fs::write(&clip, b"V").unwrap();
        let zip_path = dir.join("bundle.zip");

        let ok = write_bundle(
            &zip_path,
            &db,
            "{}",
            &[("a.mp4".to_string(), clip)],
            &|| true, // cancel immediately
            &mut |_d, _t| {},
        )
        .unwrap();
        assert!(!ok);
        assert!(!zip_path.exists(), "partial bundle should be cleaned up");

        let _ = fs::remove_dir_all(&dir);
    }
}
