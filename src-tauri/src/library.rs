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
/// (spec §4.2: removal is index-only by default).
pub fn remove_clip(conn: &Connection, id: i64) -> Result<(), String> {
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
}
