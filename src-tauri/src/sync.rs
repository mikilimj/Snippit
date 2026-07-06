//! Pure sync planning logic: which files need uploading (disk vs sync_state)
//! and local-vs-remote diffing. The IO-driving engine lives in lib.rs.

use crate::library::SyncRow;
use crate::remote::RemoteFile;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDiff {
    pub remote_only: Vec<RemoteFile>,
    pub local_only: Vec<String>,
}

/// Path of `file` relative to `clips_dir`, `/`-separated (remote naming is
/// uniform across platforms). None when the file is outside Clips/.
pub fn rel_name(clips_dir: &Path, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(clips_dir).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    Some(parts.join("/"))
}

/// Local files needing upload: not in sync_state, or size/mtime changed since
/// the recorded upload. Order follows `local` input order.
pub fn plan_uploads(local: &[(String, u64, i64)], state: &[SyncRow]) -> Vec<String> {
    let by_path: HashMap<&str, &SyncRow> =
        state.iter().map(|r| (r.path.as_str(), r)).collect();
    local
        .iter()
        .filter(|(path, size, mtime)| match by_path.get(path.as_str()) {
            Some(r) => r.size_bytes != *size || r.mtime != *mtime,
            None => true,
        })
        .map(|(path, _, _)| path.clone())
        .collect()
}

/// Groups by relative path: remote files with no local counterpart, and local
/// rel-names with no remote counterpart.
pub fn diff(local_rels: &[(String, u64, i64)], remote: &[RemoteFile]) -> SyncDiff {
    let local_set: HashSet<&str> = local_rels.iter().map(|(p, _, _)| p.as_str()).collect();
    let remote_set: HashSet<&str> = remote.iter().map(|r| r.path.as_str()).collect();
    SyncDiff {
        remote_only: remote
            .iter()
            .filter(|r| !local_set.contains(r.path.as_str()))
            .cloned()
            .collect(),
        local_only: local_rels
            .iter()
            .filter(|(p, _, _)| !remote_set.contains(p.as_str()))
            .map(|(p, _, _)| p.clone())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::SyncRow;
    use crate::remote::RemoteFile;
    use std::path::Path;

    fn row(path: &str, size: u64, mtime: i64) -> SyncRow {
        SyncRow { path: path.into(), size_bytes: size, mtime,
                  remote_name: "x".into(), uploaded_at: 1 }
    }

    #[test]
    fn rel_name_strips_clips_dir_and_uses_forward_slashes() {
        let clips = Path::new("/root/Clips");
        assert_eq!(rel_name(clips, Path::new("/root/Clips/a.mp4")).unwrap(), "a.mp4");
        assert_eq!(rel_name(clips, Path::new("/root/Clips/sub/b.mp4")).unwrap(), "sub/b.mp4");
        assert_eq!(rel_name(clips, Path::new("/elsewhere/c.mp4")), None);
    }

    #[test]
    fn plan_uploads_picks_new_and_changed_files_only() {
        let local = vec![
            ("/r/Clips/same.mp4".to_string(), 100u64, 10i64),
            ("/r/Clips/grown.mp4".to_string(), 300, 10),
            ("/r/Clips/new.mp4".to_string(), 50, 5),
        ];
        let state = vec![row("/r/Clips/same.mp4", 100, 10), row("/r/Clips/grown.mp4", 100, 10)];
        let planned = plan_uploads(&local, &state);
        assert_eq!(planned, vec!["/r/Clips/grown.mp4".to_string(), "/r/Clips/new.mp4".to_string()]);
    }

    #[test]
    fn diff_groups_by_relative_path() {
        let local = vec![("both.mp4".to_string(), 1u64, 1i64), ("localonly.mp4".to_string(), 2, 2)];
        let remote = vec![
            RemoteFile { path: "both.mp4".into(), size: 1, mod_time: 1 },
            RemoteFile { path: "sub/remoteonly.mp4".into(), size: 3, mod_time: 3 },
        ];
        let d = diff(&local, &remote);
        assert_eq!(d.remote_only.len(), 1);
        assert_eq!(d.remote_only[0].path, "sub/remoteonly.mp4");
        assert_eq!(d.local_only, vec!["localonly.mp4".to_string()]);
    }
}
