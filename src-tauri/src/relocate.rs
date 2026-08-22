//! Folder relocation engine (spec §4.7): move a watched or output folder to a
//! new path. On the same volume this is a single atomic OS rename (instant, zero
//! extra space); across volumes it streams file-by-file — copy, verify the size,
//! delete the source, then update that clip's index path — so peak extra disk use
//! is bounded by one file and an interrupted move is resumable and never loses
//! data. The library index is kept in sync as files move.

use crate::library;
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};

/// Which move strategy to use — decided by the caller from [`same_volume`].
pub enum Fanout {
    /// Same filesystem: a single atomic OS rename, zero extra space.
    Rename,
    /// Different disks: stream file-by-file (copy → verify → delete source).
    CopyPerFile,
}

/// How a relocation finished.
pub enum Outcome {
    Completed,
    Cancelled,
}

/// Map a source file to its destination, preserving the path relative to the root
/// (subfolder structure is mirrored).
pub fn dest_for(src_file: &Path, src_root: &Path, dest_root: &Path) -> PathBuf {
    match src_file.strip_prefix(src_root) {
        Ok(rel) => dest_root.join(rel),
        Err(_) => dest_root.join(src_file.file_name().unwrap_or_default()),
    }
}

/// Whether `src` still needs copying to `dest`: true if the destination is absent
/// or a different size. A verified prior copy (matching size) is skipped, which is
/// what makes an interrupted move resumable.
pub fn needs_copy(src: &Path, dest: &Path) -> bool {
    match (fs::metadata(src).map(|m| m.len()), fs::metadata(dest).map(|m| m.len())) {
        (Ok(s), Ok(d)) => s != d,
        (Ok(_), Err(_)) => true, // destination missing
        (Err(_), _) => false,    // source gone — nothing to copy
    }
}

/// Recursively collect every file (not directory) under `root`.
pub fn enumerate_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(root, &mut out);
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Largest single-file size among `files` — the pre-flight free-space check only
/// needs room for the biggest file in flight, not the whole tree (spec §4.7).
pub fn largest_file_size(files: &[PathBuf]) -> u64 {
    files
        .iter()
        .filter_map(|f| fs::metadata(f).ok())
        .map(|m| m.len())
        .max()
        .unwrap_or(0)
}

/// Copy one file to `dest`, verify the byte count matches the source, then delete
/// the source. The source is removed only after verification succeeds, so an
/// interruption at worst leaves the file in both places (cleaned up next run).
pub fn copy_verify_delete(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let copied = fs::copy(src, dest).map_err(|e| format!("copy {} failed: {e}", src.display()))?;
    let src_len = fs::metadata(src).map(|m| m.len()).map_err(|e| e.to_string())?;
    if copied != src_len {
        let _ = fs::remove_file(dest); // partial/corrupt copy — drop it, keep the source
        return Err(format!(
            "verification failed for {} ({copied} != {src_len} bytes)",
            src.display()
        ));
    }
    fs::remove_file(src).map_err(|e| format!("could not remove source {}: {e}", src.display()))?;
    Ok(())
}

/// Whether two paths live on the same filesystem volume (so a rename is atomic).
#[cfg(unix)]
pub fn same_volume(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fn dev(p: &Path) -> Option<u64> {
        let mut cur = Some(p);
        while let Some(c) = cur {
            if let Ok(m) = fs::metadata(c) {
                return Some(m.dev());
            }
            cur = c.parent();
        }
        None
    }
    matches!((dev(a), dev(b)), (Some(x), Some(y)) if x == y)
}

#[cfg(not(unix))]
pub fn same_volume(_a: &Path, _b: &Path) -> bool {
    false // conservative: fall back to the always-safe per-file copy path
}

/// Move `src_root` to `dest_root`, keeping the library index in sync. `on_progress`
/// receives (done, total, current_destination); `should_cancel` is polled between
/// files so a cross-volume move can stop cleanly (every file is either fully moved
/// or untouched — nothing to roll back).
pub fn relocate_folder(
    conn: &Connection,
    src_root: &Path,
    dest_root: &Path,
    fanout: Fanout,
    should_cancel: &dyn Fn() -> bool,
    on_progress: &mut dyn FnMut(usize, usize, &Path),
) -> Result<Outcome, String> {
    match fanout {
        Fanout::Rename => {
            if let Some(parent) = dest_root.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::rename(src_root, dest_root).map_err(|e| format!("move failed: {e}"))?;
            library::relink_clip_prefix(
                conn,
                &src_root.to_string_lossy(),
                &dest_root.to_string_lossy(),
            )?;
            on_progress(1, 1, dest_root);
            Ok(Outcome::Completed)
        }
        Fanout::CopyPerFile => {
            let files = enumerate_files(src_root);
            let total = files.len();
            for (i, src_file) in files.iter().enumerate() {
                if should_cancel() {
                    return Ok(Outcome::Cancelled);
                }
                let dest_file = dest_for(src_file, src_root, dest_root);
                if needs_copy(src_file, &dest_file) {
                    copy_verify_delete(src_file, &dest_file)?;
                } else if src_file.exists() {
                    // Already verified in a prior run but the source lingered — clean it up.
                    fs::remove_file(src_file).map_err(|e| e.to_string())?;
                }
                // Update the index immediately so it stays correct even if the
                // move is interrupted after this file (spec §4.7 step 4).
                library::relink_clip_path(
                    conn,
                    &src_file.to_string_lossy(),
                    &dest_file.to_string_lossy(),
                )?;
                on_progress(i + 1, total, &dest_file);
            }
            reconcile_index(conn, src_root, dest_root)?;
            remove_empty_dirs(src_root);
            Ok(Outcome::Completed)
        }
    }
}

/// Relinks any clip still indexed under `src_root` whose bytes already live at the
/// mirrored destination — covers a prior move interrupted between copying a file
/// and updating its index row, so the index converges once the bytes are moved.
fn reconcile_index(conn: &Connection, src_root: &Path, dest_root: &Path) -> Result<(), String> {
    let src_prefix = src_root.to_string_lossy().to_string();
    for clip in library::list_clips(conn)? {
        if let Some(rest) = clip.path.strip_prefix(&src_prefix) {
            let candidate = format!("{}{}", dest_root.to_string_lossy(), rest);
            if Path::new(&candidate).exists() {
                library::relink_clip_path(conn, &clip.path, &candidate)?;
            }
        }
    }
    Ok(())
}

/// Best-effort removal of the now-empty source tree after a per-file move.
fn remove_empty_dirs(root: &Path) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                remove_empty_dirs(&p);
            }
        }
    }
    let _ = fs::remove_dir(root);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library;
    use crate::probe::ClipMeta;

    fn temp_subdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "snippit_relocate_{name}_{}_{}",
            std::process::id(),
            name.len()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn meta() -> ClipMeta {
        ClipMeta {
            duration_secs: 1.0,
            width: 2,
            height: 2,
            container: "mp4".into(),
            video_codec: "h264".into(),
            audio_tracks: vec![],
        }
    }

    #[test]
    fn dest_for_mirrors_subfolder_structure() {
        let src_root = Path::new("/old/root");
        let dest_root = Path::new("/new/place");
        assert_eq!(
            dest_for(Path::new("/old/root/sub/a.mp4"), src_root, dest_root),
            PathBuf::from("/new/place/sub/a.mp4")
        );
    }

    #[test]
    fn needs_copy_true_when_dest_missing_false_when_sizes_match() {
        let dir = temp_subdir("needs_copy");
        let src = dir.join("s.bin");
        let dest = dir.join("d.bin");
        fs::write(&src, b"hello").unwrap();
        assert!(needs_copy(&src, &dest)); // dest missing
        fs::write(&dest, b"hello").unwrap();
        assert!(!needs_copy(&src, &dest)); // same size => already moved
        fs::write(&dest, b"different length").unwrap();
        assert!(needs_copy(&src, &dest)); // size mismatch => re-copy
    }

    #[test]
    fn largest_file_size_picks_the_biggest() {
        let dir = temp_subdir("largest");
        let a = dir.join("a");
        let b = dir.join("b");
        fs::write(&a, b"12").unwrap();
        fs::write(&b, b"12345").unwrap();
        assert_eq!(largest_file_size(&[a, b]), 5);
    }

    #[test]
    fn copy_verify_delete_moves_and_removes_source() {
        let dir = temp_subdir("cvd");
        let src = dir.join("src.bin");
        let dest = dir.join("out/dest.bin");
        fs::write(&src, b"payload").unwrap();
        copy_verify_delete(&src, &dest).unwrap();
        assert!(!src.exists(), "source should be gone after a verified move");
        assert_eq!(fs::read(&dest).unwrap(), b"payload");
    }

    #[test]
    fn cross_volume_move_relocates_files_and_updates_index() {
        let src_root = temp_subdir("xvol_src");
        let dest_root = temp_subdir("xvol_dest_parent").join("moved");
        fs::create_dir_all(src_root.join("sub")).unwrap();
        let a = src_root.join("a.mp4");
        let b = src_root.join("sub/b.mp4");
        fs::write(&a, b"aaa").unwrap();
        fs::write(&b, b"bbbb").unwrap();

        let conn = Connection::open_in_memory().unwrap();
        library::init_db(&conn).unwrap();
        library::upsert_clip(&conn, a.to_str().unwrap(), &meta(), None).unwrap();
        library::upsert_clip(&conn, b.to_str().unwrap(), &meta(), None).unwrap();

        let mut seen: Vec<usize> = Vec::new();
        let outcome = relocate_folder(
            &conn,
            &src_root,
            &dest_root,
            Fanout::CopyPerFile,
            &|| false,
            &mut |done, _total, _cur| seen.push(done),
        )
        .unwrap();
        assert!(matches!(outcome, Outcome::Completed));

        // Files moved, sources gone, empty source tree cleaned up.
        assert!(dest_root.join("a.mp4").exists());
        assert!(dest_root.join("sub/b.mp4").exists());
        assert!(!a.exists());
        assert!(!src_root.exists());
        assert_eq!(seen, vec![1, 2]); // progress reported per file

        // Index now points at the new locations.
        let paths: Vec<String> =
            library::list_clips(&conn).unwrap().into_iter().map(|c| c.path).collect();
        assert!(paths.contains(&dest_root.join("a.mp4").to_string_lossy().to_string()));
        assert!(paths.contains(&dest_root.join("sub/b.mp4").to_string_lossy().to_string()));
    }

    #[test]
    fn cross_volume_move_is_resumable_and_skips_verified_files() {
        let src_root = temp_subdir("resume_src");
        let dest_root = temp_subdir("resume_dest_parent").join("moved");
        let a = src_root.join("a.mp4");
        let b = src_root.join("b.mp4");
        fs::write(&a, b"aaa").unwrap();
        fs::write(&b, b"bbbb").unwrap();
        // Simulate a prior interrupted run: 'a' was already copied AND its source
        // removed, but the index wasn't updated yet.
        fs::create_dir_all(&dest_root).unwrap();
        fs::write(dest_root.join("a.mp4"), b"aaa").unwrap();
        fs::remove_file(&a).unwrap();

        let conn = Connection::open_in_memory().unwrap();
        library::init_db(&conn).unwrap();
        library::upsert_clip(&conn, a.to_str().unwrap(), &meta(), None).unwrap();
        library::upsert_clip(&conn, b.to_str().unwrap(), &meta(), None).unwrap();

        relocate_folder(
            &conn,
            &src_root,
            &dest_root,
            Fanout::CopyPerFile,
            &|| false,
            &mut |_d, _t, _c| {},
        )
        .unwrap();

        // Both end up relinked; 'b' actually moved, source tree cleaned.
        assert!(dest_root.join("b.mp4").exists());
        assert!(!b.exists());
        let paths: Vec<String> =
            library::list_clips(&conn).unwrap().into_iter().map(|c| c.path).collect();
        assert!(paths.iter().all(|p| p.starts_with(dest_root.to_str().unwrap())));
    }

    #[test]
    fn cancel_stops_the_cross_volume_move_partway() {
        let src_root = temp_subdir("cancel_src");
        let dest_root = temp_subdir("cancel_dest_parent").join("moved");
        fs::write(src_root.join("a.mp4"), b"aaa").unwrap();
        fs::write(src_root.join("b.mp4"), b"bbb").unwrap();

        let conn = Connection::open_in_memory().unwrap();
        library::init_db(&conn).unwrap();

        // Cancel before any file is processed.
        let outcome = relocate_folder(
            &conn,
            &src_root,
            &dest_root,
            Fanout::CopyPerFile,
            &|| true,
            &mut |_d, _t, _c| {},
        )
        .unwrap();
        assert!(matches!(outcome, Outcome::Cancelled));
        // Nothing moved — sources untouched.
        assert!(src_root.join("a.mp4").exists());
    }

    #[test]
    fn same_volume_rename_moves_tree_and_rehomes_index() {
        // Source and destination share the same temp volume, so this exercises the
        // atomic-rename fast path end to end.
        let base = temp_subdir("rename_base");
        let src_root = base.join("src");
        let dest_root = base.join("dst");
        fs::create_dir_all(&src_root).unwrap();
        let a = src_root.join("a.mp4");
        fs::write(&a, b"aaa").unwrap();

        let conn = Connection::open_in_memory().unwrap();
        library::init_db(&conn).unwrap();
        library::upsert_clip(&conn, a.to_str().unwrap(), &meta(), None).unwrap();

        relocate_folder(
            &conn,
            &src_root,
            &dest_root,
            Fanout::Rename,
            &|| false,
            &mut |_d, _t, _c| {},
        )
        .unwrap();

        assert!(!src_root.exists());
        assert!(dest_root.join("a.mp4").exists());
        let paths: Vec<String> =
            library::list_clips(&conn).unwrap().into_iter().map(|c| c.path).collect();
        assert_eq!(paths, vec![dest_root.join("a.mp4").to_string_lossy().to_string()]);
    }
}
