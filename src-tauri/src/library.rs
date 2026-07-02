//! SQLite-backed library index. Clips are referenced in place (spec §4.2): the
//! index stores metadata + user organization (display name, favorite, folders),
//! never the media bytes. All functions take a `&Connection` so they can be unit
//! tested against an in-memory database; the file-backed connection is opened by
//! `open_db` from `<root>/library.db`.

use crate::probe::ClipMeta;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipRow {
    pub id: i64,
    pub path: String,
    pub display_name: String,
    pub duration_secs: f64,
    pub width: u32,
    pub height: u32,
    pub container: String,
    pub video_codec: String,
    pub audio_track_count: u32,
    pub thumbnail_path: Option<String>,
    pub favorite: bool,
    pub missing: bool,
    pub added_at: i64,
}

/// A virtual folder: a user-named grouping of clips (spec §4.2). Folders never
/// move files on disk — membership is a pure many-to-many index relation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: i64,
    pub name: String,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn filename_of(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Opens (creating if absent) the file-backed library database and ensures its
/// schema exists.
pub fn open_db(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    init_db(&conn)?;
    Ok(conn)
}

/// Creates the schema if it does not already exist. Idempotent — safe to call on
/// every open.
pub fn init_db(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS clips (
            id                INTEGER PRIMARY KEY,
            path              TEXT NOT NULL UNIQUE,
            display_name      TEXT NOT NULL,
            duration_secs     REAL NOT NULL,
            width             INTEGER NOT NULL,
            height            INTEGER NOT NULL,
            container         TEXT NOT NULL,
            video_codec       TEXT NOT NULL,
            audio_track_count INTEGER NOT NULL,
            thumbnail_path    TEXT,
            favorite          INTEGER NOT NULL DEFAULT 0,
            missing           INTEGER NOT NULL DEFAULT 0,
            added_at          INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS folders (
            id         INTEGER PRIMARY KEY,
            name       TEXT NOT NULL UNIQUE,
            created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS clip_folders (
            clip_id   INTEGER NOT NULL,
            folder_id INTEGER NOT NULL,
            PRIMARY KEY (clip_id, folder_id)
        );",
    )
    .map_err(|e| e.to_string())
}

const SELECT_COLS: &str = "id, path, display_name, duration_secs, width, height, \
    container, video_codec, audio_track_count, thumbnail_path, favorite, missing, added_at";

fn row_from(row: &rusqlite::Row) -> rusqlite::Result<ClipRow> {
    Ok(ClipRow {
        id: row.get(0)?,
        path: row.get(1)?,
        display_name: row.get(2)?,
        duration_secs: row.get(3)?,
        width: row.get::<_, i64>(4)? as u32,
        height: row.get::<_, i64>(5)? as u32,
        container: row.get(6)?,
        video_codec: row.get(7)?,
        audio_track_count: row.get::<_, i64>(8)? as u32,
        thumbnail_path: row.get(9)?,
        favorite: row.get::<_, i64>(10)? != 0,
        missing: row.get::<_, i64>(11)? != 0,
        added_at: row.get(12)?,
    })
}

/// Inserts a clip, or updates the probed metadata of an existing one keyed by
/// `path`. User-owned fields (`display_name`, `favorite`) are preserved across
/// re-ingestion; an existing thumbnail is kept when `thumbnail_path` is `None`.
/// Any prior `missing` flag is cleared, since a successful probe proves the file
/// is readable again. Returns the resulting row.
pub fn upsert_clip(
    conn: &Connection,
    path: &str,
    meta: &ClipMeta,
    thumbnail_path: Option<&str>,
) -> Result<ClipRow, String> {
    conn.execute(
        "INSERT INTO clips
            (path, display_name, duration_secs, width, height, container,
             video_codec, audio_track_count, thumbnail_path, favorite, missing, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, 0, ?10)
         ON CONFLICT(path) DO UPDATE SET
            duration_secs     = excluded.duration_secs,
            width             = excluded.width,
            height            = excluded.height,
            container         = excluded.container,
            video_codec       = excluded.video_codec,
            audio_track_count = excluded.audio_track_count,
            thumbnail_path    = COALESCE(excluded.thumbnail_path, clips.thumbnail_path),
            missing           = 0",
        params![
            path,
            filename_of(path),
            meta.duration_secs,
            meta.width as i64,
            meta.height as i64,
            meta.container,
            meta.video_codec,
            meta.audio_tracks.len() as i64,
            thumbnail_path,
            now_unix(),
        ],
    )
    .map_err(|e| e.to_string())?;

    conn.query_row(
        &format!("SELECT {SELECT_COLS} FROM clips WHERE path = ?1"),
        params![path],
        row_from,
    )
    .map_err(|e| e.to_string())
}

/// Lists all indexed clips, most recently added first.
pub fn list_clips(conn: &Connection) -> Result<Vec<ClipRow>, String> {
    let mut stmt = conn
        .prepare(&format!("SELECT {SELECT_COLS} FROM clips ORDER BY id DESC"))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_from)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Removes a clip from the index only. The source file on disk is never touched
/// (spec §4.2: removal is index-only by default). Any folder memberships pointing
/// at the clip are dropped too, so no orphaned `clip_folders` rows remain.
pub fn remove_clip(conn: &Connection, id: i64) -> Result<(), String> {
    conn.execute("DELETE FROM clip_folders WHERE clip_id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM clips WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Toggles a clip's favorite flag.
pub fn set_favorite(conn: &Connection, id: i64, favorite: bool) -> Result<(), String> {
    conn.execute(
        "UPDATE clips SET favorite = ?1 WHERE id = ?2",
        params![favorite as i64, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Sets a clip's library display name. The source file on disk is never renamed
/// (spec §4.2: only the index's display name changes).
pub fn rename_clip(conn: &Connection, id: i64, display_name: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE clips SET display_name = ?1 WHERE id = ?2",
        params![display_name, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Updates a single clip's stored path after its file was moved on disk (spec
/// §4.7 cross-volume move). No-op if no clip currently has `old_path`.
pub fn relink_clip_path(conn: &Connection, old_path: &str, new_path: &str) -> Result<(), String> {
    conn.execute(
        "UPDATE clips SET path = ?1 WHERE path = ?2",
        params![new_path, old_path],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Re-homes every clip whose path lies under `old_prefix` to the same relative
/// location under `new_prefix` (spec §4.7 same-volume folder rename). Returns how
/// many rows were rewritten. Matching is by path prefix, done in Rust to avoid
/// SQL `LIKE` metacharacter pitfalls in real filesystem paths.
pub fn relink_clip_prefix(
    conn: &Connection,
    old_prefix: &str,
    new_prefix: &str,
) -> Result<usize, String> {
    let mut stmt = conn
        .prepare("SELECT id, path FROM clips")
        .map_err(|e| e.to_string())?;
    let rows: Vec<(i64, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;

    let mut changed = 0;
    for (id, path) in rows {
        if let Some(rest) = path.strip_prefix(old_prefix) {
            let new_path = format!("{new_prefix}{rest}");
            conn.execute(
                "UPDATE clips SET path = ?1 WHERE id = ?2",
                params![new_path, id],
            )
            .map_err(|e| e.to_string())?;
            changed += 1;
        }
    }
    Ok(changed)
}

/// Creates a new virtual folder and returns it. Fails if the name is already
/// taken (folder names are unique).
pub fn create_folder(conn: &Connection, name: &str) -> Result<Folder, String> {
    conn.execute(
        "INSERT INTO folders (name, created_at) VALUES (?1, ?2)",
        params![name, now_unix()],
    )
    .map_err(|e| e.to_string())?;
    Ok(Folder {
        id: conn.last_insert_rowid(),
        name: name.to_string(),
    })
}

/// Lists all virtual folders, ordered alphabetically by name.
pub fn list_folders(conn: &Connection) -> Result<Vec<Folder>, String> {
    let mut stmt = conn
        .prepare("SELECT id, name FROM folders ORDER BY name COLLATE NOCASE")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok(Folder {
                id: row.get(0)?,
                name: row.get(1)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Deletes a virtual folder and all of its clip memberships. The clips
/// themselves are untouched (they simply revert to unsorted).
pub fn delete_folder(conn: &Connection, folder_id: i64) -> Result<(), String> {
    conn.execute(
        "DELETE FROM clip_folders WHERE folder_id = ?1",
        params![folder_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM folders WHERE id = ?1", params![folder_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Adds a clip to a folder. Idempotent — re-adding an existing membership is a
/// no-op (a clip may belong to many folders at once).
pub fn add_clip_to_folder(conn: &Connection, clip_id: i64, folder_id: i64) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO clip_folders (clip_id, folder_id) VALUES (?1, ?2)",
        params![clip_id, folder_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Detaches a clip from a single folder, leaving its other memberships intact.
pub fn remove_clip_from_folder(
    conn: &Connection,
    clip_id: i64,
    folder_id: i64,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM clip_folders WHERE clip_id = ?1 AND folder_id = ?2",
        params![clip_id, folder_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Lists the clips that belong to a given folder, most recently added first.
pub fn list_clips_in_folder(conn: &Connection, folder_id: i64) -> Result<Vec<ClipRow>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SELECT_COLS} FROM clips
             JOIN clip_folders ON clip_folders.clip_id = clips.id
             WHERE clip_folders.folder_id = ?1
             ORDER BY clips.id DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![folder_id], row_from)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

/// Lists clips that are not in any folder — the "Unsorted" view (spec §4.2),
/// most recently added first.
pub fn list_unsorted_clips(conn: &Connection) -> Result<Vec<ClipRow>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SELECT_COLS} FROM clips
             WHERE id NOT IN (SELECT clip_id FROM clip_folders)
             ORDER BY id DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], row_from)
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::{AudioTrack, ClipMeta};
    use rusqlite::Connection;

    fn mem_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    fn sample_meta() -> ClipMeta {
        ClipMeta {
            duration_secs: 12.5,
            width: 1920,
            height: 1080,
            container: "mov,mp4".into(),
            video_codec: "h264".into(),
            audio_tracks: vec![
                AudioTrack {
                    index: 1,
                    codec: "aac".into(),
                    channels: 2,
                    language: Some("eng".into()),
                },
                AudioTrack {
                    index: 2,
                    codec: "aac".into(),
                    channels: 2,
                    language: None,
                },
            ],
        }
    }

    #[test]
    fn init_db_is_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        init_db(&conn).unwrap(); // second call must not error
        assert_eq!(list_clips(&conn).unwrap().len(), 0);
    }

    #[test]
    fn upsert_inserts_a_clip_with_defaults() {
        let conn = mem_db();
        let row = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        assert_eq!(row.path, "/vids/a.mp4");
        assert_eq!(row.display_name, "a.mp4"); // defaults to filename
        assert!((row.duration_secs - 12.5).abs() < 1e-9);
        assert_eq!(row.width, 1920);
        assert_eq!(row.height, 1080);
        assert_eq!(row.audio_track_count, 2);
        assert!(!row.favorite);
        assert!(!row.missing);
        assert_eq!(row.thumbnail_path, None);
        assert!(row.id > 0);
    }

    #[test]
    fn upsert_is_unique_by_path_and_updates_metadata() {
        let conn = mem_db();
        let first = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        let mut m2 = sample_meta();
        m2.width = 1280;
        m2.height = 720;
        let second = upsert_clip(&conn, "/vids/a.mp4", &m2, Some("/cache/a.jpg")).unwrap();
        assert_eq!(list_clips(&conn).unwrap().len(), 1); // no duplicate row
        assert_eq!(second.id, first.id); // same row id preserved
        assert_eq!(second.width, 1280);
        assert_eq!(second.thumbnail_path.as_deref(), Some("/cache/a.jpg"));
    }

    #[test]
    fn upsert_preserves_user_fields_favorite_and_display_name() {
        let conn = mem_db();
        let row = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        set_favorite(&conn, row.id, true).unwrap();
        rename_clip(&conn, row.id, "My Best Frag").unwrap();
        // Re-ingest (e.g. a watcher re-probe) must NOT clobber user-set fields.
        let reingested = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        assert!(reingested.favorite);
        assert_eq!(reingested.display_name, "My Best Frag");
    }

    #[test]
    fn upsert_preserves_existing_thumbnail_when_reingested_without_one() {
        let conn = mem_db();
        upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), Some("/cache/a.jpg")).unwrap();
        let reingested = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        assert_eq!(reingested.thumbnail_path.as_deref(), Some("/cache/a.jpg"));
    }

    #[test]
    fn list_clips_returns_newest_first() {
        let conn = mem_db();
        upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        upsert_clip(&conn, "/vids/b.mp4", &sample_meta(), None).unwrap();
        let clips = list_clips(&conn).unwrap();
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].path, "/vids/b.mp4"); // most recently added first
        assert_eq!(clips[1].path, "/vids/a.mp4");
    }

    #[test]
    fn remove_clip_deletes_only_the_index_row() {
        let conn = mem_db();
        let row = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        upsert_clip(&conn, "/vids/b.mp4", &sample_meta(), None).unwrap();
        remove_clip(&conn, row.id).unwrap();
        let clips = list_clips(&conn).unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].path, "/vids/b.mp4");
    }

    #[test]
    fn set_favorite_and_rename_persist() {
        let conn = mem_db();
        let row = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        set_favorite(&conn, row.id, true).unwrap();
        rename_clip(&conn, row.id, "Clutch").unwrap();
        let clips = list_clips(&conn).unwrap();
        assert!(clips[0].favorite);
        assert_eq!(clips[0].display_name, "Clutch");
    }

    #[test]
    fn create_folder_returns_row_and_lists_alphabetically() {
        let conn = mem_db();
        let games = create_folder(&conn, "Games").unwrap();
        let apex = create_folder(&conn, "Apex").unwrap();
        assert!(games.id > 0);
        assert_eq!(games.name, "Games");
        let folders = list_folders(&conn).unwrap();
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].name, "Apex"); // sorted by name, not insertion order
        assert_eq!(folders[0].id, apex.id);
        assert_eq!(folders[1].name, "Games");
        assert_eq!(folders[1].id, games.id);
    }

    #[test]
    fn folder_names_are_unique() {
        let conn = mem_db();
        create_folder(&conn, "Dupe").unwrap();
        assert!(create_folder(&conn, "Dupe").is_err());
    }

    #[test]
    fn add_clip_to_folder_is_idempotent_and_filters_listing() {
        let conn = mem_db();
        let a = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        let b = upsert_clip(&conn, "/vids/b.mp4", &sample_meta(), None).unwrap();
        let folder = create_folder(&conn, "Faves").unwrap();
        add_clip_to_folder(&conn, a.id, folder.id).unwrap();
        add_clip_to_folder(&conn, a.id, folder.id).unwrap(); // duplicate is a no-op
        let in_folder = list_clips_in_folder(&conn, folder.id).unwrap();
        assert_eq!(in_folder.len(), 1);
        assert_eq!(in_folder[0].id, a.id);
        // b was never added.
        assert!(!in_folder.iter().any(|c| c.id == b.id));
    }

    #[test]
    fn remove_clip_from_folder_detaches_only_that_membership() {
        let conn = mem_db();
        let a = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        let f1 = create_folder(&conn, "One").unwrap();
        let f2 = create_folder(&conn, "Two").unwrap();
        add_clip_to_folder(&conn, a.id, f1.id).unwrap();
        add_clip_to_folder(&conn, a.id, f2.id).unwrap();
        remove_clip_from_folder(&conn, a.id, f1.id).unwrap();
        assert_eq!(list_clips_in_folder(&conn, f1.id).unwrap().len(), 0);
        assert_eq!(list_clips_in_folder(&conn, f2.id).unwrap().len(), 1);
    }

    #[test]
    fn list_unsorted_returns_clips_in_no_folder() {
        let conn = mem_db();
        let a = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        let b = upsert_clip(&conn, "/vids/b.mp4", &sample_meta(), None).unwrap();
        let folder = create_folder(&conn, "Sorted").unwrap();
        add_clip_to_folder(&conn, a.id, folder.id).unwrap();
        let unsorted = list_unsorted_clips(&conn).unwrap();
        assert_eq!(unsorted.len(), 1);
        assert_eq!(unsorted[0].id, b.id); // only the un-foldered clip
    }

    #[test]
    fn delete_folder_removes_folder_and_memberships_but_keeps_clips() {
        let conn = mem_db();
        let a = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        let folder = create_folder(&conn, "Temp").unwrap();
        add_clip_to_folder(&conn, a.id, folder.id).unwrap();
        delete_folder(&conn, folder.id).unwrap();
        assert_eq!(list_folders(&conn).unwrap().len(), 0);
        // The clip itself survives and becomes unsorted again.
        assert_eq!(list_clips(&conn).unwrap().len(), 1);
        assert_eq!(list_unsorted_clips(&conn).unwrap().len(), 1);
    }

    #[test]
    fn removing_a_clip_also_clears_its_folder_memberships() {
        let conn = mem_db();
        let a = upsert_clip(&conn, "/vids/a.mp4", &sample_meta(), None).unwrap();
        let folder = create_folder(&conn, "Keep").unwrap();
        add_clip_to_folder(&conn, a.id, folder.id).unwrap();
        remove_clip(&conn, a.id).unwrap();
        // No orphaned membership rows left pointing at the deleted clip.
        assert_eq!(list_clips_in_folder(&conn, folder.id).unwrap().len(), 0);
    }

    #[test]
    fn relink_clip_path_updates_a_single_clips_location() {
        let conn = mem_db();
        upsert_clip(&conn, "/old/a.mp4", &sample_meta(), None).unwrap();
        upsert_clip(&conn, "/old/b.mp4", &sample_meta(), None).unwrap();
        relink_clip_path(&conn, "/old/a.mp4", "/new/a.mp4").unwrap();
        let paths: Vec<String> = list_clips(&conn).unwrap().into_iter().map(|c| c.path).collect();
        assert!(paths.contains(&"/new/a.mp4".to_string()));
        assert!(paths.contains(&"/old/b.mp4".to_string())); // untouched
        assert!(!paths.contains(&"/old/a.mp4".to_string()));
    }

    #[test]
    fn relink_clip_prefix_rehomes_every_clip_under_a_moved_root() {
        let conn = mem_db();
        upsert_clip(&conn, "/old/root/a.mp4", &sample_meta(), None).unwrap();
        upsert_clip(&conn, "/old/root/sub/b.mp4", &sample_meta(), None).unwrap();
        upsert_clip(&conn, "/elsewhere/c.mp4", &sample_meta(), None).unwrap();
        let n = relink_clip_prefix(&conn, "/old/root", "/new/place").unwrap();
        assert_eq!(n, 2); // only the two under the old root
        let paths: Vec<String> = list_clips(&conn).unwrap().into_iter().map(|c| c.path).collect();
        assert!(paths.contains(&"/new/place/a.mp4".to_string()));
        assert!(paths.contains(&"/new/place/sub/b.mp4".to_string()));
        assert!(paths.contains(&"/elsewhere/c.mp4".to_string())); // outside the root, untouched
    }
}
