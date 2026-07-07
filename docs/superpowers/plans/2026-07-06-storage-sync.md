# Remote Storage Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Mirror the local `Clips/` folder to one connected remote (SFTP, Dropbox, or Google Drive) via a bundled rclone sidecar, with a diff/import window, independent upload/local size-reduction settings, and close-to-tray background syncing.

**Architecture:** rclone is a third sidecar binary next to ffmpeg/ffprobe; `remote.rs` wraps it with typed arg-builders (unit-tested like `export.rs`), `sync.rs` holds pure queue/diff logic, and `lib.rs` wires the upload worker into the existing watcher + a 5-minute rescan. Upload state lives in a `sync_state` table in `library.db`; the remote holds `ClipSnip/{Clips,Thumbnails,library-index.json}`.

**Tech Stack:** Tauri 2 (shell sidecar, tray-icon), rclone, rusqlite, existing ffmpeg size-target machinery (`compute_target_bitrates`), React/TS frontend with plain CSS.

**Spec:** `docs/superpowers/specs/2026-07-06-storage-sync-design.md`

## Global Constraints

- One remote at a time, always named `clipsnip` in ClipSnip's private rclone config at `<app_config_dir>/rclone.conf`.
- Remote layout root: `<base path>/ClipSnip/` containing `Clips/`, `Thumbnails/`, `library-index.json`.
- `sync_state.remote_name` is the path **relative to the local `Clips/` folder**; remote `Clips/` mirrors local subfolders.
- Sync never downloads and never deletes on its own. Remote deletion only via explicit UI actions.
- A `sync_state` row is written only after rclone exits 0 for the video upload.
- Size percents: integer 1–99 via custom, presets 25/50/75; `None` = full size. Two independent settings (`upload_size_percent`, `local_size_percent`), both `#[serde(default)]` in `settings.json` for backward compat.
- Re-encodes never replace/delete an original until the new file is fully written and probes valid.
- Rescan interval: 300 s. Rust tests: `cd src-tauri && cargo test`. TS tests: `npx vitest run`. Type gate: `npm run build`.
- Commit after every task; never pass `--author` or touch git user config.

---

### Task 1: rclone sidecar binary + build wiring

**Files:**
- Create: `src-tauri/binaries/rclone-x86_64-unknown-linux-gnu` (downloaded binary, not committed if the repo ignores binaries — check `.gitignore`; ffmpeg/ffprobe siblings show the convention)
- Modify: `src-tauri/tauri.conf.json` (externalBin)
- Modify: `src-tauri/capabilities/default.json` **only if** it enumerates per-sidecar shell permissions (inspect it; ffmpeg/ffprobe entries show the pattern — mirror one for rclone)

**Interfaces:**
- Produces: `app.shell().sidecar("rclone")` becomes resolvable at runtime, same as `sidecar("ffmpeg")` used throughout `lib.rs`.

- [x] **Step 1: Download rclone for the dev platform**

```bash
cd src-tauri/binaries
curl -LO https://downloads.rclone.org/rclone-current-linux-amd64.zip
unzip -j rclone-current-linux-amd64.zip '*/rclone' -d .
mv rclone rclone-x86_64-unknown-linux-gnu
chmod +x rclone-x86_64-unknown-linux-gnu
rm rclone-current-linux-amd64.zip
./rclone-x86_64-unknown-linux-gnu version
```

Expected: `rclone v1.x.x` version output. (Other platforms use the same naming: `rclone-x86_64-pc-windows-msvc.exe`, `rclone-aarch64-apple-darwin` — note this in the commit message; only the Linux binary is needed to develop and test.)

- [x] **Step 2: Register the sidecar**

In `src-tauri/tauri.conf.json`, extend `externalBin`:

```json
    "externalBin": [
      "binaries/ffmpeg",
      "binaries/ffprobe",
      "binaries/rclone"
    ],
```

If `src-tauri/capabilities/default.json` lists `shell:allow-execute`/sidecar scopes per binary, add the matching rclone entry by copying the ffprobe stanza and renaming.

- [x] **Step 3: Verify the app still builds and can spawn the sidecar**

Run: `cd src-tauri && cargo build 2>&1 | tail -5`
Expected: `Finished` with no errors (tauri-build validates externalBin existence).

- [x] **Step 4: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/capabilities src-tauri/binaries 2>/dev/null || git add src-tauri/tauri.conf.json src-tauri/capabilities
git commit -m "feat(sync): bundle rclone as a third sidecar binary"
```

---

### Task 2: `remote.rs` — rclone arg builders + JSON parsing (pure, unit-tested)

**Files:**
- Create: `src-tauri/src/remote.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod remote;` next to the other `mod` lines)

**Interfaces:**
- Produces (consumed by Tasks 5–7):
  - `remote::base_args(config_path: &Path) -> Vec<String>`
  - `remote::create_sftp_args(config_path, host, port, user, auth: &SftpAuth) -> Vec<String>`
  - `remote::create_oauth_args(config_path, provider: &str) -> Vec<String>` (`provider` ∈ `"dropbox"`, `"drive"`)
  - `remote::delete_remote_args(config_path) -> Vec<String>`
  - `remote::remote_dir(base_path: Option<&str>, sub: &str) -> String` → e.g. `clipsnip:backups/ClipSnip/Clips`
  - `remote::mkdir_args`, `lsjson_args`, `copyto_args`, `deletefile_args`, `lsd_check_args` — all `(config_path, …) -> Vec<String>`
  - `remote::parse_lsjson(json: &str) -> Result<Vec<RemoteFile>, String>` with `pub struct RemoteFile { pub path: String, pub size: u64, pub mod_time: i64 }` (serde camelCase for the frontend; `path` is relative to the listed dir, from lsjson `-R` `Path` field)
  - `remote::identity(provider: &str, host: Option<&str>, user: Option<&str>, base_path: Option<&str>) -> String` — `sftp:user@host:port/base` for sftp; `"dropbox/base"`/`"drive/base"` for cloud
  - `pub enum SftpAuth { Password(String), KeyFile(String) }`

- [x] **Step 1: Write failing unit tests** (bottom of the new `remote.rs`, mirroring `export.rs` test style)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn cfg() -> &'static Path { Path::new("/cfg/rclone.conf") }

    #[test]
    fn base_args_pin_config_and_noninteractive_flags() {
        let args = base_args(cfg());
        assert_eq!(args, vec!["--config", "/cfg/rclone.conf", "--auto-confirm", "--log-level", "ERROR"]);
    }

    #[test]
    fn create_sftp_args_with_password_obscures_it() {
        let args = create_sftp_args(cfg(), "nas.local", 22, "milosz",
            &SftpAuth::Password("s3cret".into()));
        let joined = args.join(" ");
        assert!(joined.contains("config create clipsnip sftp"));
        assert!(joined.contains("host=nas.local"));
        assert!(joined.contains("port=22"));
        assert!(joined.contains("user=milosz"));
        assert!(joined.contains("pass=s3cret"));
        assert!(joined.contains("--obscure"));
    }

    #[test]
    fn create_sftp_args_with_key_file_has_no_pass() {
        let args = create_sftp_args(cfg(), "nas.local", 2222, "milosz",
            &SftpAuth::KeyFile("/home/m/.ssh/id_ed25519".into()));
        let joined = args.join(" ");
        assert!(joined.contains("key_file=/home/m/.ssh/id_ed25519"));
        assert!(!joined.contains("pass="));
        assert!(!joined.contains("--obscure"));
    }

    #[test]
    fn create_oauth_args_names_the_provider() {
        let joined = create_oauth_args(cfg(), "dropbox").join(" ");
        assert!(joined.contains("config create clipsnip dropbox"));
    }

    #[test]
    fn remote_dir_joins_base_path_and_defaults_to_root() {
        assert_eq!(remote_dir(Some("backups"), "Clips"), "clipsnip:backups/ClipSnip/Clips");
        assert_eq!(remote_dir(None, "Clips"), "clipsnip:ClipSnip/Clips");
        assert_eq!(remote_dir(Some("a/"), ""), "clipsnip:a/ClipSnip");
    }

    #[test]
    fn copyto_lsjson_deletefile_args_shape() {
        assert_eq!(copyto_args(cfg(), "/tmp/a.mp4", "clipsnip:ClipSnip/Clips/a.mp4")[..3].to_vec(),
            vec!["--config", "/cfg/rclone.conf", "--auto-confirm"]);
        let ls = lsjson_args(cfg(), "clipsnip:ClipSnip/Clips");
        assert!(ls.contains(&"lsjson".to_string()) && ls.contains(&"-R".to_string())
            && ls.contains(&"--files-only".to_string()));
        let del = deletefile_args(cfg(), "clipsnip:ClipSnip/Clips/a.mp4");
        assert!(del.contains(&"deletefile".to_string()));
    }

    #[test]
    fn parse_lsjson_maps_path_size_and_rfc3339_modtime() {
        let json = r#"[{"Path":"sub/a.mp4","Name":"a.mp4","Size":1048576,
            "ModTime":"2026-07-06T10:00:00Z","IsDir":false}]"#;
        let files = parse_lsjson(json).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "sub/a.mp4");
        assert_eq!(files[0].size, 1048576);
        assert_eq!(files[0].mod_time, 1783418400);
    }

    #[test]
    fn parse_lsjson_rejects_garbage() {
        assert!(parse_lsjson("not json").is_err());
    }

    #[test]
    fn identity_formats_per_provider() {
        assert_eq!(
            identity("sftp", Some("nas.local:22"), Some("milosz"), Some("backups")),
            "sftp:milosz@nas.local:22/backups");
        assert_eq!(identity("dropbox", None, None, None), "dropbox/");
    }
}
```

- [x] **Step 2: Run to verify failure**

Run: `cd src-tauri && cargo test remote:: 2>&1 | tail -5`
Expected: compile error — module/functions don't exist.

- [x] **Step 3: Implement `remote.rs`**

```rust
//! Typed wrapper around the bundled rclone sidecar. ClipSnip owns a private
//! rclone config holding exactly one remote named `clipsnip`; every command
//! pins `--config` so the user's own rclone setup is never touched. Arg
//! builders are pure so they can be unit tested; the async runner lives in
//! lib.rs where the AppHandle is available.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const REMOTE_NAME: &str = "clipsnip";

#[derive(Debug, Clone)]
pub enum SftpAuth {
    Password(String),
    KeyFile(String),
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFile {
    pub path: String,
    pub size: u64,
    pub mod_time: i64,
}

pub fn config_path(config_dir: &Path) -> std::path::PathBuf {
    config_dir.join("rclone.conf")
}

pub fn base_args(config_path: &Path) -> Vec<String> {
    vec![
        "--config".into(),
        config_path.to_string_lossy().to_string(),
        "--auto-confirm".into(),
        "--log-level".into(),
        "ERROR".into(),
    ]
}

pub fn create_sftp_args(
    config_path: &Path,
    host: &str,
    port: u16,
    user: &str,
    auth: &SftpAuth,
) -> Vec<String> {
    let mut args = base_args(config_path);
    if matches!(auth, SftpAuth::Password(_)) {
        args.push("--obscure".into());
    }
    args.extend([
        "config".into(), "create".into(), REMOTE_NAME.into(), "sftp".into(),
        format!("host={host}"), format!("port={port}"), format!("user={user}"),
    ]);
    match auth {
        SftpAuth::Password(p) => args.push(format!("pass={p}")),
        SftpAuth::KeyFile(k) => args.push(format!("key_file={k}")),
    }
    args
}

/// `provider` is an rclone backend name: "dropbox" or "drive". Running this
/// starts rclone's local OAuth flow: it opens the system browser and blocks
/// until the user approves (or fails). No client registration needed.
pub fn create_oauth_args(config_path: &Path, provider: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend([
        "config".into(), "create".into(), REMOTE_NAME.into(), provider.into(),
    ]);
    args
}

pub fn delete_remote_args(config_path: &Path) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["config".into(), "delete".into(), REMOTE_NAME.into()]);
    args
}

/// Builds `clipsnip:<base>/ClipSnip[/<sub>]`. Empty sub returns the ClipSnip
/// root itself. Trailing slashes on base are tolerated.
pub fn remote_dir(base_path: Option<&str>, sub: &str) -> String {
    let base = base_path.unwrap_or("").trim_matches('/');
    let mut out = format!("{REMOTE_NAME}:");
    if !base.is_empty() {
        out.push_str(base);
        out.push('/');
    }
    out.push_str("ClipSnip");
    if !sub.is_empty() {
        out.push('/');
        out.push_str(sub);
    }
    out
}

pub fn mkdir_args(config_path: &Path, dir: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["mkdir".into(), dir.into()]);
    args
}

pub fn lsd_check_args(config_path: &Path, dir: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["lsd".into(), dir.into()]);
    args
}

pub fn lsjson_args(config_path: &Path, dir: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["lsjson".into(), "-R".into(), "--files-only".into(), dir.into()]);
    args
}

pub fn copyto_args(config_path: &Path, local: &str, remote: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["copyto".into(), local.into(), remote.into()]);
    args
}

pub fn download_args(config_path: &Path, remote: &str, local: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["copyto".into(), remote.into(), local.into()]);
    args
}

pub fn deletefile_args(config_path: &Path, remote: &str) -> Vec<String> {
    let mut args = base_args(config_path);
    args.extend(["deletefile".into(), remote.into()]);
    args
}

#[derive(Debug, Deserialize)]
struct LsJsonEntry {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Size")]
    size: i64,
    #[serde(rename = "ModTime")]
    mod_time: String,
    #[serde(rename = "IsDir", default)]
    is_dir: bool,
}

/// Parses `rclone lsjson -R --files-only` output. ModTime is RFC3339; parse
/// the "YYYY-MM-DDTHH:MM:SS" prefix manually (UTC assumed, fractional seconds
/// and offsets after the seconds field are ignored) to avoid a chrono dep.
pub fn parse_lsjson(json: &str) -> Result<Vec<RemoteFile>, String> {
    let entries: Vec<LsJsonEntry> =
        serde_json::from_str(json).map_err(|e| format!("bad lsjson output: {e}"))?;
    Ok(entries
        .into_iter()
        .filter(|e| !e.is_dir)
        .map(|e| RemoteFile {
            path: e.path,
            size: e.size.max(0) as u64,
            mod_time: rfc3339_to_unix(&e.mod_time),
        })
        .collect())
}

/// Minimal RFC3339 → unix seconds (UTC). Returns 0 on anything unparseable —
/// mod-time is display-only (diff window date column), never a sync decision.
fn rfc3339_to_unix(s: &str) -> i64 {
    fn num(s: &str, a: usize, b: usize) -> Option<i64> { s.get(a..b)?.parse().ok() }
    let (y, mo, d) = match (num(s, 0, 4), num(s, 5, 7), num(s, 8, 10)) {
        (Some(y), Some(mo), Some(d)) => (y, mo, d),
        _ => return 0,
    };
    let (h, mi, sec) = match (num(s, 11, 13), num(s, 14, 16), num(s, 17, 19)) {
        (Some(h), Some(mi), Some(sec)) => (h, mi, sec),
        _ => return 0,
    };
    // days since epoch via civil-days algorithm (Howard Hinnant's days_from_civil)
    let y_adj = if mo <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    days * 86400 + h * 3600 + mi * 60 + sec
}

pub fn identity(
    provider: &str,
    host: Option<&str>,
    user: Option<&str>,
    base_path: Option<&str>,
) -> String {
    let base = base_path.unwrap_or("");
    match provider {
        "sftp" => format!("sftp:{}@{}/{base}", user.unwrap_or(""), host.unwrap_or("")),
        p => format!("{p}/{base}"),
    }
}
```

(Keep the tests from Step 1 at the bottom of the file.) Add `mod remote;` in `lib.rs` beside the existing module declarations.

- [x] **Step 4: Run tests**

Run: `cd src-tauri && cargo test remote:: 2>&1 | tail -5`
Expected: all remote tests PASS (fix the `identity` sftp test — host already contains `:22` in the test input, so `identity` must not add a port itself; the caller passes `host:port` as one string).

- [x] **Step 5: Commit**

```bash
git add src-tauri/src/remote.rs src-tauri/src/lib.rs
git commit -m "feat(sync): rclone arg builders and lsjson parsing in remote.rs"
```

---

### Task 3: `sync_state` table + CRUD in `library.rs`

**Files:**
- Modify: `src-tauri/src/library.rs` (schema in `init_db`, new functions + tests)

**Interfaces:**
- Produces (consumed by Tasks 5–7):
  - `pub struct SyncRow { pub path: String, pub size_bytes: u64, pub mtime: i64, pub remote_name: String, pub uploaded_at: i64 }` (serde camelCase, Serialize)
  - `library::sync_state_upsert(conn, path, size_bytes, mtime, remote_name) -> Result<(), String>`
  - `library::sync_state_list(conn) -> Result<Vec<SyncRow>, String>`
  - `library::sync_state_remove(conn, path) -> Result<(), String>`
  - `library::sync_state_remove_by_remote_name(conn, remote_name) -> Result<(), String>`
  - `library::sync_state_clear(conn) -> Result<(), String>`

- [x] **Step 1: Write failing tests** (inside `library.rs`'s existing `mod tests`, using its in-memory-connection helper)

```rust
#[test]
fn sync_state_upsert_list_roundtrip_and_update() {
    let conn = mem_db();
    sync_state_upsert(&conn, "/root/Clips/a.mp4", 100, 10, "a.mp4").unwrap();
    sync_state_upsert(&conn, "/root/Clips/a.mp4", 200, 20, "a.mp4").unwrap();
    let rows = sync_state_list(&conn).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].size_bytes, 200);
    assert_eq!(rows[0].mtime, 20);
    assert_eq!(rows[0].remote_name, "a.mp4");
    assert!(rows[0].uploaded_at > 0);
}

#[test]
fn sync_state_remove_variants_and_clear() {
    let conn = mem_db();
    sync_state_upsert(&conn, "/r/Clips/a.mp4", 1, 1, "a.mp4").unwrap();
    sync_state_upsert(&conn, "/r/Clips/s/b.mp4", 1, 1, "s/b.mp4").unwrap();
    sync_state_remove(&conn, "/r/Clips/a.mp4").unwrap();
    assert_eq!(sync_state_list(&conn).unwrap().len(), 1);
    sync_state_remove_by_remote_name(&conn, "s/b.mp4").unwrap();
    assert_eq!(sync_state_list(&conn).unwrap().len(), 0);
    sync_state_upsert(&conn, "/r/Clips/c.mp4", 1, 1, "c.mp4").unwrap();
    sync_state_clear(&conn).unwrap();
    assert!(sync_state_list(&conn).unwrap().is_empty());
}
```

(If the tests module has no shared `mem_db()` helper, follow whatever pattern the existing library tests use to get a `Connection` — e.g. `Connection::open_in_memory()` + `init_db`.)

- [x] **Step 2: Run to verify failure**

Run: `cd src-tauri && cargo test sync_state 2>&1 | tail -5`
Expected: compile error — functions don't exist.

- [x] **Step 3: Implement**

In `init_db`'s schema batch add:

```sql
CREATE TABLE IF NOT EXISTS sync_state (
  path TEXT PRIMARY KEY,
  size_bytes INTEGER NOT NULL,
  mtime INTEGER NOT NULL,
  remote_name TEXT NOT NULL,
  uploaded_at INTEGER NOT NULL
);
```

(`init_db` is idempotent `CREATE TABLE IF NOT EXISTS` style, so existing `library.db` files upgrade in place on first open.)

```rust
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRow {
    pub path: String,
    pub size_bytes: u64,
    pub mtime: i64,
    pub remote_name: String,
    pub uploaded_at: i64,
}

pub fn sync_state_upsert(
    conn: &Connection,
    path: &str,
    size_bytes: u64,
    mtime: i64,
    remote_name: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_state (path, size_bytes, mtime, remote_name, uploaded_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(path) DO UPDATE SET
           size_bytes = ?2, mtime = ?3, remote_name = ?4, uploaded_at = ?5",
        params![path, size_bytes as i64, mtime, remote_name, now_unix()],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn sync_state_list(conn: &Connection) -> Result<Vec<SyncRow>, String> {
    let mut stmt = conn
        .prepare("SELECT path, size_bytes, mtime, remote_name, uploaded_at FROM sync_state")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            Ok(SyncRow {
                path: r.get(0)?,
                size_bytes: r.get::<_, i64>(1)?.max(0) as u64,
                mtime: r.get(2)?,
                remote_name: r.get(3)?,
                uploaded_at: r.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

pub fn sync_state_remove(conn: &Connection, path: &str) -> Result<(), String> {
    conn.execute("DELETE FROM sync_state WHERE path = ?1", params![path])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn sync_state_remove_by_remote_name(conn: &Connection, remote_name: &str) -> Result<(), String> {
    conn.execute("DELETE FROM sync_state WHERE remote_name = ?1", params![remote_name])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn sync_state_clear(conn: &Connection) -> Result<(), String> {
    conn.execute("DELETE FROM sync_state", [])
        .map_err(|e| e.to_string())?;
    Ok(())
}
```

- [x] **Step 4: Run tests**

Run: `cd src-tauri && cargo test sync_state 2>&1 | tail -5`
Expected: both tests PASS. Also run `cargo test library 2>&1 | tail -3` — existing library tests still PASS (schema addition must not break them).

- [x] **Step 5: Commit**

```bash
git add src-tauri/src/library.rs
git commit -m "feat(sync): sync_state table and CRUD in library index"
```

---

### Task 4: Settings fields + percent-size helper

**Files:**
- Modify: `src-tauri/src/setup.rs` (new `SetupSettings` fields + tests)
- Modify: `src-tauri/src/media.rs` (size re-encode arg builder + tests)

**Interfaces:**
- Produces (consumed by Tasks 5–7):
  - `SetupSettings` gains, all `#[serde(default)]`: `upload_size_percent: Option<u8>`, `local_size_percent: Option<u8>`, `remote_provider: Option<String>` (`"sftp"|"dropbox"|"drive"`), `remote_base_path: Option<String>`, `remote_identity: Option<String>`, `tray_enabled: Option<bool>` (None = true)
  - `media::percent_target_mb(size_bytes: u64, percent: u8) -> f64`
  - `media::build_size_reencode_args(source: &str, output: &str, video_kbps: u32, audio_kbps: u32, device: &ExportDevice) -> Vec<String>` — single-pass VBV-capped encode of the **whole file** (no trim), all streams mapped (`-map 0:v:0 -map 0:a?`), video per device, audio AAC at `audio_kbps`.

- [x] **Step 1: Write failing tests**

In `setup.rs` tests:

```rust
#[test]
fn read_settings_parses_files_without_sync_fields() {
    let root = temp_subdir("read_settings_presync");
    fs::write(
        root.join("settings.json"),
        r#"{"ingestion_mode":"watch","source_folder":"/rec","created_at":1}"#,
    )
    .unwrap();
    let read = read_settings(&root).unwrap();
    assert_eq!(read.upload_size_percent, None);
    assert_eq!(read.local_size_percent, None);
    assert_eq!(read.remote_provider, None);
    assert_eq!(read.remote_identity, None);
    assert_eq!(read.tray_enabled, None);
}
```

In `media.rs` tests:

```rust
#[test]
fn percent_target_mb_scales_bytes() {
    // 100 MB file at 50% -> 50.0 MB target
    assert!((percent_target_mb(100 * 1024 * 1024, 50) - 50.0).abs() < 1e-9);
    assert!((percent_target_mb(10 * 1024 * 1024, 25) - 2.5).abs() < 1e-9);
}

#[test]
fn build_size_reencode_args_maps_all_streams_and_sets_bitrates() {
    let args = build_size_reencode_args(
        "/c/in.mp4", "/c/out.mp4", 2500, 128, &crate::export::ExportDevice::Cpu);
    let joined = args.join(" ");
    assert!(joined.contains("-i /c/in.mp4"));
    assert!(joined.contains("-map 0:v:0"));
    assert!(joined.contains("-map 0:a?"));
    assert!(joined.contains("-b:a 128k"));
    assert!(joined.contains("2500k"));
    assert!(joined.ends_with("/c/out.mp4"));
    assert!(args.contains(&"-y".to_string()));
}
```

- [x] **Step 2: Run to verify failure**

Run: `cd src-tauri && cargo test percent_target 2>&1 | tail -3` and `cargo test read_settings_parses_files_without_sync 2>&1 | tail -3`
Expected: compile errors (missing fields/functions).

- [x] **Step 3: Implement**

`setup.rs` — extend the struct (every new field `#[serde(default)]`, one doc comment noting None-defaults), and set them to `None` in `complete_setup_at`'s `SetupSettings` literal. Add a helper used by lib.rs to persist partial updates:

```rust
#[serde(default)]
pub upload_size_percent: Option<u8>,
#[serde(default)]
pub local_size_percent: Option<u8>,
#[serde(default)]
pub remote_provider: Option<String>,
#[serde(default)]
pub remote_base_path: Option<String>,
#[serde(default)]
pub remote_identity: Option<String>,
/// None = tray enabled (default on).
#[serde(default)]
pub tray_enabled: Option<bool>,
```

`media.rs`:

```rust
/// Target size for a percent-of-original re-encode, in MB (fed to
/// export::compute_target_bitrates).
pub fn percent_target_mb(size_bytes: u64, percent: u8) -> f64 {
    (size_bytes as f64 / (1024.0 * 1024.0)) * (percent as f64 / 100.0)
}

/// Whole-file single-pass re-encode to a bitrate budget: video per device
/// (VBV-capped), all audio streams kept and re-encoded to AAC. Used for the
/// upload-size and local-shrink paths; unlike exports there is no trim window
/// and no track mixing — streams are preserved 1:1.
pub fn build_size_reencode_args(
    source: &str,
    output: &str,
    video_kbps: u32,
    audio_kbps: u32,
    device: &crate::export::ExportDevice,
) -> Vec<String> {
    let mut args: Vec<String> = vec!["-y".into()];
    args.extend(device.global_args());
    args.extend(device.input_args());
    args.extend(["-i".into(), source.into()]);
    args.extend(["-map".into(), "0:v:0".into(), "-map".into(), "0:a?".into()]);
    args.extend(device.video_bitrate_args(video_kbps));
    args.extend(["-c:a".into(), "aac".into(), format!("-b:a"), format!("{audio_kbps}k")]);
    args.push(output.into());
    args
}
```

(Check `ExportDevice`'s actual method names in `export.rs` — `global_args`/`input_args`/`video_bitrate_args` per the 2026-07-05 spec — and adjust to match exactly. The `format!("-b:a")` above is a two-element flag+value pair: `"-b:a", "{audio_kbps}k"`.)

- [x] **Step 4: Run tests**

Run: `cd src-tauri && cargo test 2>&1 | tail -5` (full suite — settings serde and existing media tests must still pass)
Expected: all PASS.

- [x] **Step 5: Commit**

```bash
git add src-tauri/src/setup.rs src-tauri/src/media.rs
git commit -m "feat(sync): size-percent settings fields and re-encode arg builder"
```

---

### Task 5: `sync.rs` — pure queue/diff logic

**Files:**
- Create: `src-tauri/src/sync.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod sync;`)

**Interfaces:**
- Consumes: `library::SyncRow` (Task 3), `remote::RemoteFile` (Task 2).
- Produces (consumed by Task 6/7):
  - `sync::rel_name(clips_dir: &Path, file: &Path) -> Option<String>` — path relative to Clips/, `/`-separated on all platforms
  - `sync::plan_uploads(local: &[(String, u64, i64)], state: &[SyncRow]) -> Vec<String>` — local = (abs path, size, mtime); returns abs paths needing upload (absent from state, or size/mtime changed)
  - `sync::diff(local_rels: &[(String, u64, i64)], remote: &[RemoteFile]) -> SyncDiff` with `pub struct SyncDiff { pub remote_only: Vec<RemoteFile>, pub local_only: Vec<String> }` (Serialize, camelCase; local_only holds rel names; matching is by rel path only — size differences are not surfaced in v1)

- [x] **Step 1: Write failing tests** (bottom of `sync.rs`)

```rust
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
```

- [x] **Step 2: Run to verify failure**

Run: `cd src-tauri && cargo test sync:: 2>&1 | tail -3`
Expected: compile error.

- [x] **Step 3: Implement**

```rust
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
```

- [x] **Step 4: Run tests**

Run: `cd src-tauri && cargo test sync:: 2>&1 | tail -3`
Expected: 3 tests PASS.

- [x] **Step 5: Commit**

```bash
git add src-tauri/src/sync.rs src-tauri/src/lib.rs
git commit -m "feat(sync): pure upload-planning and diff logic"
```

---

### Task 6: Sync engine + Tauri commands in `lib.rs`

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: everything from Tasks 2–5.
- Produces — Tauri commands (all registered in `generate_handler!`), consumed by Task 7's `api.ts`:
  - `storage_connect_sftp(host: String, port: u16, user: String, password: Option<String>, key_file: Option<String>, base_path: Option<String>) -> Result<(), String>`
  - `storage_connect_oauth(provider: String, base_path: Option<String>) -> Result<(), String>`
  - `storage_disconnect() -> Result<(), String>`
  - `storage_status() -> Result<StorageStatus, String>` where `StorageStatus { provider: Option<String>, state: String /* "idle"|"syncing"|"error" */, message: Option<String>, synced_count: u32, pending_count: u32 }` (Serialize camelCase)
  - `storage_scan_diff() -> Result<sync::SyncDiff, String>`
  - `storage_import_clips(rel_names: Vec<String>) -> Result<u32, String>` (downloads + ingests + sync_state rows; applies remote index metadata)
  - `storage_delete_remote(rel_name: String) -> Result<(), String>` (video + thumbnail + sync_state row + index refresh)
  - `storage_download_thumb(rel_name: String) -> Result<String, String>` (fetch `Thumbnails/<stem>.jpg` into `Cache/remote-thumbs/`, return local path; serves the diff window lazily via the media server URL)
  - `sync_now()` (enqueue scan; fire-and-forget)
  - `get_sync_settings() -> Result<SyncSettings, String>` / `set_sync_settings(upload_size_percent: Option<u8>, local_size_percent: Option<u8>, tray_enabled: bool) -> Result<(), String>` with `SyncSettings { upload_size_percent: Option<u8>, local_size_percent: Option<u8>, tray_enabled: bool }`
  - Event `sync-status` (payload = `StorageStatus`) emitted on every state change; event `clip-added` already exists and fires for imported clips.
- Internal helper produced for Task 11: `run_rclone(app: &tauri::AppHandle, args: Vec<String>) -> Result<String, String>` — spawns `app.shell().sidecar("rclone")`, collects stdout, maps non-zero exit to `Err(stderr)`.

**Engine design (implement as described — no queue persistence, the queue is derived):**

- New managed state:

```rust
struct SyncEngine {
    queue: Mutex<VecDeque<PathBuf>>,       // pending uploads (abs paths)
    worker_running: AtomicBool,            // one upload worker at a time
    status: Mutex<StorageStatusInner>,     // provider/state/message/counts
}
```

- `enqueue_sync_scan(app)`: reads settings; if `remote_provider.is_none()` → return. Lists `Clips/` recursively via `watcher::scan_video_files(clips_dir)` with size+mtime, loads `library::sync_state_list`, calls `sync::plan_uploads`, pushes novel paths into `queue` (skip paths already queued), then starts the worker if not running.
- Worker (spawned via `tauri::async_runtime::spawn`): pops one path at a time. Per file:
  1. `wait_for_stable_size` (existing fn).
  2. `probe_path` → duration (needed for bitrate math and thumbnail).
  3. Ensure thumbnail exists (same helper flow as `ingest_file_inner`).
  4. If `upload_size_percent = Some(p)`: `percent_target_mb` → `compute_target_bitrates(target_mb, duration)`; on `Err` (below floor) fall back to full-size upload and record the note in status message; else run ffmpeg with `build_size_reencode_args` into `Cache/upload-tmp/<rel_name>` and upload that file.
  5. `run_rclone(copyto_args(cfg, local_or_temp, remote_dir(base, &format!("Clips/{rel}"))))`.
  6. Upload thumbnail to `remote_dir(base, &format!("Thumbnails/{stem}.jpg"))` (best-effort; failure logged in status, not fatal).
  7. Refresh remote index: `library::export_index` into `Cache/library-index-export.db`… **No** — the remote index is JSON per spec. Implement `write_index_json(conn, root) -> Result<String, String>` in lib.rs: serialize `{ clips: [{relPath, displayName, favorite, folders: [names], addedAt}] }` from `list_clips` + folder memberships, write to `Cache/library-index.json`, upload via copyto. (Reuse `bundle::relativize_path` against `<root>/Clips` for relPath.)
  8. `library::sync_state_upsert` with the **original** local file's size/mtime.
  9. If `local_size_percent = Some(p)`: re-encode original → `Cache/shrink-tmp.mp4`, `probe_path` the result (must succeed and have `duration_secs > 0`), then `std::fs::rename` over the original (same volume: Cache is under root), `library::upsert_clip` refresh, and `sync_state_upsert` with the new size/mtime so the shrunk file isn't re-uploaded. On any failure keep the original untouched.
  10. On rclone failure: push the path to the back of the queue with a retry counter (HashMap<PathBuf, u32> inside SyncEngine; drop after 5 failures until the next scan), set status error, sleep 30 s before next attempt.
  - Emit `sync-status` after every transition.
- 300 s rescan timer, spawned once in `.setup()`:

```rust
let handle = app.handle().clone();
tauri::async_runtime::spawn(async move {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        scan_folders_now(&handle);          // existing library ingest
        enqueue_sync_scan(&handle);         // sync upload planning
    }
});
```

- Hook `enqueue_sync_scan` into the existing watcher flow: at the end of `ingest_and_emit` (after a successful ingest), call it so new recordings upload within seconds.
- `storage_connect_sftp` / `storage_connect_oauth`: build args from Task 2, `run_rclone` (OAuth variant may take minutes — it blocks on the browser; run it in the async command, the frontend shows a "waiting for browser" state), then `run_rclone(lsd_check_args)` to validate, `run_rclone(mkdir_args)` for `ClipSnip/Clips` and `ClipSnip/Thumbnails`, compute `remote::identity(...)`, compare with `settings.remote_identity` — **if different, `library::sync_state_clear`** — then persist provider/base/identity via `setup::write_settings`, emit status, `enqueue_sync_scan`.
- `storage_disconnect`: `run_rclone(delete_remote_args)`, clear `remote_provider`/`remote_base_path` (keep `remote_identity` AND `sync_state` so reconnecting the same remote resumes), clear queue, emit status.
- `storage_scan_diff`: requires connection; `run_rclone(lsjson_args(cfg, remote_dir(base, "Clips")))` → `parse_lsjson` → build local list rel-named via `sync::rel_name` → `sync::diff`. Any rclone error returns `Err` (fail fast, no partial diff).
- `storage_import_clips`: for each rel name: `run_rclone(download_args(...Clips/<rel>, <clips_dir>/<rel>))`, then `ingest_file_inner` (registers + thumbnails + emits via caller), `sync_state_upsert`. After all downloads, fetch `library-index.json` (best-effort) and apply `displayName`/`favorite`/`folders` to the imported rows (match by relPath; create missing folders via `library::create_folder`). Return count imported.
- `storage_delete_remote`: `deletefile` video, best-effort `deletefile` thumbnail, `sync_state_remove_by_remote_name`, refresh remote index JSON.

- [x] **Step 1: Implement the engine + commands as specified above** (this task is wiring; its logic halves are already unit-tested in Tasks 2–5)

- [x] **Step 2: Type-check + full Rust suite**

Run: `cd src-tauri && cargo test 2>&1 | tail -5`
Expected: everything PASSes, no warnings about unused new functions (all are wired).

- [x] **Step 3: Manual smoke test with a local rclone remote**

```bash
# once: create a throwaway local remote in the app's config to sanity-check wiring
CONF=~/.config/dev.clipsnip.app/rclone.conf   # adjust to the real app_config_dir
./src-tauri/binaries/rclone-x86_64-unknown-linux-gnu --config "$CONF" config create clipsnip local
npm run tauri dev
```

In the running app (via devtools console): `window.__TAURI__.core.invoke('storage_scan_diff')` should resolve with a diff object rather than reject.
Expected: no crash; `ClipSnip/` tree appears under the filesystem root when `mkdir` paths run (local remote maps `clipsnip:X` to `/X` — use a scratch `base_path` when testing by invoking `storage_connect_sftp`-less flows carefully, or just verify `storage_status` + `sync_now` don't error with the local remote).

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/setup.rs
git commit -m "feat(sync): upload engine, rescan timer, and storage Tauri commands"
```

---

### Task 7: `api.ts` wrappers + `syncDiff.ts` frontend logic

**Files:**
- Modify: `src/api.ts`
- Create: `src/syncDiff.ts`, `src/syncDiff.test.ts`

**Interfaces:**
- Consumes: Task 6's command names/payloads (camelCase over IPC).
- Produces (consumed by Tasks 8–10):
  - api.ts: `StorageStatus`, `RemoteFile`, `SyncDiffResult`, `SyncSettings` interfaces + `storageConnectSftp`, `storageConnectOauth`, `storageDisconnect`, `storageStatus`, `storageScanDiff`, `storageImportClips`, `storageDeleteRemote`, `storageDownloadThumb`, `syncNow`, `getSyncSettings`, `setSyncSettings` functions
  - syncDiff.ts: `formatSize(bytes: number): string` ("1.5 MB"), `formatDate(unixSecs: number): string` (locale date), `displayNameFor(relPath: string, index: RemoteIndexEntry[] | null): string`, `percentIsValid(p: number): boolean` (integer 1–99), `type RemoteIndexEntry = { relPath: string; displayName: string; favorite: boolean; folders: string[] }`

- [x] **Step 1: Write failing tests** (`src/syncDiff.test.ts`)

```ts
import { describe, expect, it } from "vitest";
import { displayNameFor, formatSize, percentIsValid } from "./syncDiff";

describe("formatSize", () => {
  it("formats bytes, MB and GB", () => {
    expect(formatSize(500)).toBe("500 B");
    expect(formatSize(1536 * 1024)).toBe("1.5 MB");
    expect(formatSize(2.25 * 1024 * 1024 * 1024)).toBe("2.25 GB");
  });
});

describe("displayNameFor", () => {
  const index = [{ relPath: "sub/a.mp4", displayName: "Ace round", favorite: true, folders: [] }];
  it("prefers the remote index display name", () => {
    expect(displayNameFor("sub/a.mp4", index)).toBe("Ace round");
  });
  it("falls back to the filename without extension", () => {
    expect(displayNameFor("sub/other.mp4", index)).toBe("other");
    expect(displayNameFor("plain.mp4", null)).toBe("plain");
  });
});

describe("percentIsValid", () => {
  it("accepts integers 1-99 only", () => {
    expect(percentIsValid(50)).toBe(true);
    expect(percentIsValid(1)).toBe(true);
    expect(percentIsValid(99)).toBe(true);
    expect(percentIsValid(0)).toBe(false);
    expect(percentIsValid(100)).toBe(false);
    expect(percentIsValid(49.5)).toBe(false);
  });
});
```

- [x] **Step 2: Run to verify failure**

Run: `npx vitest run src/syncDiff.test.ts`
Expected: FAIL — module doesn't exist.

- [x] **Step 3: Implement `syncDiff.ts`**

```ts
// Pure helpers for the storage diff window and size settings (no Tauri imports).

export interface RemoteIndexEntry {
  relPath: string;
  displayName: string;
  favorite: boolean;
  folders: string[];
}

export function formatSize(bytes: number): string {
  const gb = 1024 * 1024 * 1024;
  const mb = 1024 * 1024;
  if (bytes >= gb) return `${parseFloat((bytes / gb).toFixed(2))} GB`;
  if (bytes >= mb) return `${parseFloat((bytes / mb).toFixed(2))} MB`;
  if (bytes >= 1024) return `${parseFloat((bytes / 1024).toFixed(1))} KB`;
  return `${bytes} B`;
}

export function formatDate(unixSecs: number): string {
  if (unixSecs <= 0) return "—";
  return new Date(unixSecs * 1000).toLocaleDateString();
}

export function displayNameFor(relPath: string, index: RemoteIndexEntry[] | null): string {
  const hit = index?.find(e => e.relPath === relPath);
  if (hit) return hit.displayName;
  const file = relPath.split("/").pop() ?? relPath;
  const dot = file.lastIndexOf(".");
  return dot > 0 ? file.slice(0, dot) : file;
}

export function percentIsValid(p: number): boolean {
  return Number.isInteger(p) && p >= 1 && p <= 99;
}
```

Then extend `api.ts` following its existing `invoke` wrapper pattern (see `exportLibrary` etc. for arg-name casing — Tauri camelCases Rust snake_case params):

```ts
export interface StorageStatus {
  provider: string | null;
  state: "idle" | "syncing" | "error";
  message: string | null;
  syncedCount: number;
  pendingCount: number;
}

export interface RemoteFile {
  path: string;
  size: number;
  modTime: number;
}

export interface SyncDiffResult {
  remoteOnly: RemoteFile[];
  localOnly: string[];
}

export interface SyncSettings {
  uploadSizePercent: number | null;
  localSizePercent: number | null;
  trayEnabled: boolean;
}

export function storageConnectSftp(opts: {
  host: string; port: number; user: string;
  password?: string; keyFile?: string; basePath?: string;
}): Promise<void> {
  return invoke("storage_connect_sftp", opts);
}

export function storageConnectOauth(provider: "dropbox" | "drive", basePath?: string): Promise<void> {
  return invoke("storage_connect_oauth", { provider, basePath });
}

export function storageDisconnect(): Promise<void> {
  return invoke("storage_disconnect");
}

export function storageStatus(): Promise<StorageStatus> {
  return invoke("storage_status");
}

export function storageScanDiff(): Promise<SyncDiffResult> {
  return invoke("storage_scan_diff");
}

export function storageImportClips(relNames: string[]): Promise<number> {
  return invoke("storage_import_clips", { relNames });
}

export function storageDeleteRemote(relName: string): Promise<void> {
  return invoke("storage_delete_remote", { relName });
}

export function storageDownloadThumb(relName: string): Promise<string> {
  return invoke("storage_download_thumb", { relName });
}

export function syncNow(): Promise<void> {
  return invoke("sync_now");
}

export function getSyncSettings(): Promise<SyncSettings> {
  return invoke("get_sync_settings");
}

export function setSyncSettings(s: SyncSettings): Promise<void> {
  return invoke("set_sync_settings", {
    uploadSizePercent: s.uploadSizePercent,
    localSizePercent: s.localSizePercent,
    trayEnabled: s.trayEnabled,
  });
}
```

- [x] **Step 4: Run tests + type gate**

Run: `npx vitest run src/syncDiff.test.ts` then `npm run build`
Expected: tests PASS; tsc clean.

- [x] **Step 5: Commit**

```bash
git add src/api.ts src/syncDiff.ts src/syncDiff.test.ts
git commit -m "feat(sync): frontend API wrappers and diff-window pure helpers"
```

---

### Task 8: Settings "Storage" tab

**Files:**
- Modify: `src/SettingsModal.tsx`
- Modify: `src/App.css` (new `.storage-*` classes, following existing settings styling)

**Interfaces:**
- Consumes: all Task 7 api.ts functions; `percentIsValid` from `syncDiff.ts`.
- Produces: `onOpenDiff: () => void` prop added to `SettingsModal` (App.tsx passes a handler that opens the diff modal from Task 9).

**Behavior to implement** (structure it as a `StorageSection` component inside `SettingsModal.tsx`; if the file gets unwieldy, extract to `src/StorageSettings.tsx` and import it):

- On mount: `storageStatus()` + `getSyncSettings()`; subscribe to the `sync-status` Tauri event (same `listen` pattern the file already uses for export progress) and update the status line live.
- **Disconnected state:** three provider buttons (SFTP server / Dropbox / Google Drive).
  - SFTP: controlled inputs host, port (number, default 22), username, password (password field) *or* key file (button using `open()` from the dialog plugin, like existing folder pickers), base path (optional text). "Test & Connect" → `storageConnectSftp(...)`; disable while pending; error string under the form on reject.
  - Dropbox/Drive: optional base-path input + "Sign in with Dropbox/Google" button → `storageConnectOauth(provider, basePath)`; while pending show "Waiting for browser approval…".
- **Connected state:** provider label, status dot (`state === "error"` red / `"syncing"` amber / `"idle"` green), `message` if any, "`syncedCount` clips synced, `pendingCount` pending", "Sync now" button (`syncNow()`), "Disconnect" button with `confirm()`.
- **Size settings** (always visible): two `<select>`s — Upload size and New local recording size — options Full size (`null`) / 75% / 50% / 25% / Custom…; picking Custom reveals a number input validated with `percentIsValid`; invalid input disables saving. Save on change via `setSyncSettings`. Under the local one, a fixed warning line: "Shrinking permanently re-encodes new recordings on this PC."
- Tray checkbox: "Keep syncing in the background when the window is closed" bound to `trayEnabled`.
- "Scan for differences" button: enabled only when connected; calls `onOpenDiff()`.

- [x] **Step 1: Implement the section + CSS** (no component tests — project convention is pure-logic tests only; the validation logic is already covered by `syncDiff.test.ts`)

- [x] **Step 2: Type gate + manual check**

Run: `npm run build`
Expected: tsc clean.
Run: `npm run tauri dev`, open Settings → Storage; with no remote configured the provider picker renders; SFTP form validates; size dropdowns persist across reopen (check `settings.json` in the root folder gains `upload_size_percent` etc. after a change).

- [x] **Step 3: Commit**

```bash
git add src/SettingsModal.tsx src/StorageSettings.tsx src/App.css 2>/dev/null || git add src/SettingsModal.tsx src/App.css
git commit -m "feat(sync): Storage settings tab with provider connect flows and size settings"
```

---

### Task 9: Diff/import window

**Files:**
- Create: `src/SyncDiffModal.tsx`
- Modify: `src/App.tsx` (state to open/close it; pass `onOpenDiff` into `SettingsModal`)
- Modify: `src/App.css`

**Interfaces:**
- Consumes: `storageScanDiff`, `storageImportClips`, `storageDeleteRemote`, `storageDownloadThumb` (api.ts); `formatSize`, `formatDate`, `displayNameFor` (syncDiff.ts); `getMediaPort`-based file URLs (same pattern the library grid uses for thumbnails — check how `App.tsx` builds thumbnail `src` URLs and reuse it).
- Produces: `<SyncDiffModal onClose={() => …} onImported={() => reloadLibrary()} />` — App refreshes the library list after imports (existing `clip-added` events also fire per imported clip).

**Behavior:**

- On mount: `storageScanDiff()`; while pending show a spinner line; on reject show the error and a Retry button (fail fast — no partial diff rendering).
- Two sections: **"On server, not on this PC"** (from `remoteOnly`) and **"On this PC, not on server"** (from `localOnly`). Each row: 64px-wide thumbnail (`storageDownloadThumb(rel)` lazily per visible row → media-server URL; gray placeholder until it resolves or on failure), name (`displayNameFor`), date (`formatDate(modTime)` for remote; local rows read from the existing library rows in App state), size (`formatSize`), checkbox.
- Remote-only rows additionally get a small "Delete from server" button → `confirm()` → `storageDeleteRemote(rel)` → remove row on success.
- Footer: "Import selected to PC (n)" → `storageImportClips(selected)`, sequential progress text "Importing k of n…" (backend downloads sequentially; frontend just awaits and then calls `onImported`), and "Upload selected to server (n)" for local-only rows → `syncNow()` after ensuring they're planned (they will be — plan_uploads picks up anything not in sync_state; rows with sync_state entries that were remote-deleted outside the app are out of scope).
- Close button disabled while an import batch is running (transfers are cancellable between files only — keep a `cancelRef` checked between awaits; a Cancel button flips it).

- [x] **Step 1: Implement component + wiring + CSS**

- [x] **Step 2: Type gate + tests**

Run: `npm run build && npx vitest run`
Expected: clean tsc, all TS tests pass.

- [x] **Step 3: Commit**

```bash
git add src/SyncDiffModal.tsx src/App.tsx src/App.css
git commit -m "feat(sync): diff window with selective import, upload and remote delete"
```

---

### Task 10: Delete-from-server in the library panel

**Files:**
- Modify: `src/App.tsx` (or wherever the clip context menu / clip action buttons live — find the existing delete/favorite actions and add alongside)
- Modify: `src/api.ts` only if a lookup is needed: add `storageSyncedRelNames(): Promise<string[]>` backed by a tiny Tauri command `storage_synced_rel_names` returning `sync_state` remote_names (add it to `lib.rs` + `generate_handler!`).

**Interfaces:**
- Consumes: `storageDeleteRemote`, `storageStatus`.

**Behavior:** In the clip's action row/context menu, add "Delete from server": visible only when `storageStatus().provider != null`, enabled only when the clip's rel-name (its path relative to `<root>/Clips/`) appears in `storageSyncedRelNames()`. Click → `confirm("Delete <name> from the server? The local file is untouched.")` → `storageDeleteRemote(rel)`. This — plus the diff-window button from Task 9 — are the **only** remote-delete paths, per spec.

- [x] **Step 1: Implement (backend command + frontend menu item)**
- [x] **Step 2: Verify**

Run: `npm run build && cd src-tauri && cargo test 2>&1 | tail -3`
Expected: clean.

- [x] **Step 3: Commit**

```bash
git add src/App.tsx src/api.ts src-tauri/src/lib.rs
git commit -m "feat(sync): delete-from-server action in the library UI"
```

---

### Task 11: First-run wizard "Restore from server or cloud"

**Files:**
- Modify: `src/FirstRunWizard.tsx`, `src/wizardValidation.ts`, `src/wizardValidation.test.ts`

**Interfaces:**
- Consumes: `storageConnectSftp`, `storageConnectOauth`, `storageScanDiff`, `storageImportClips`, `completeSetup`.

**Behavior:** Add a "Restore from server or cloud" ingestion option beside the existing empty/import/watch modes. Flow: user picks root folder as usual → `completeSetup` with mode `"empty"` → the same provider-connect UI as Task 8 (reuse the extracted `StorageSettings`/connect form component rather than duplicating it) → on connect, `storageScanDiff()` → show the remote list with everything pre-checked → `storageImportClips(all selected)` → done; imported clips carry names/favorites/folders from the remote index (Task 6 behavior).

- [x] **Step 1: Extend `wizardValidation.ts` with a failing test first** — whatever pure step-gating logic the file exposes gains a `"restore"` mode; mirror how existing modes gate the Continue button (read the file, add one test per new gate, e.g.:)

```ts
it("restore mode requires a connection before finishing", () => {
  expect(canFinish({ mode: "restore", rootOk: true, connected: false })).toBe(false);
  expect(canFinish({ mode: "restore", rootOk: true, connected: true })).toBe(true);
});
```

(Adapt names to the file's actual exported functions — do not invent a parallel validation path.)

- [x] **Step 2: Run to verify failure, implement, re-run**

Run: `npx vitest run src/wizardValidation.test.ts`
Expected: FAIL → implement → PASS.

- [x] **Step 3: Implement the wizard UI step + type gate**

Run: `npm run build`
Expected: clean.

- [x] **Step 4: Commit**

```bash
git add src/FirstRunWizard.tsx src/wizardValidation.ts src/wizardValidation.test.ts
git commit -m "feat(sync): restore-from-remote option in the first-run wizard"
```

---

### Task 12: Tray mode / background running

**Files:**
- Modify: `src-tauri/Cargo.toml` (tauri features)
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/capabilities/default.json` if tray permissions are capability-gated (check; add `core:tray:default` / `core:menu:default` style entries as needed)

**Interfaces:**
- Consumes: `SetupSettings.tray_enabled` (Task 4), `sync_now` scan entry (Task 6).

- [x] **Step 1: Enable the feature**

```toml
tauri = { version = "2", features = ["protocol-asset", "tray-icon"] }
```

(This is a feature flag on the already-pinned tauri dependency — it must not bump other versions in `Cargo.lock`; run `git diff src-tauri/Cargo.lock | head` after building and confirm only tauri-related feature entries changed.)

- [x] **Step 2: Implement tray + close-to-tray in `run()`'s builder**

```rust
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

// inside .setup(), after start_watching(app.handle()):
let open_item = MenuItem::with_id(app, "open", "Open ClipSnip", true, None::<&str>)?;
let sync_item = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
let menu = Menu::with_items(app, &[&open_item, &sync_item, &quit_item])?;
TrayIconBuilder::with_id("main")
    .icon(app.default_window_icon().unwrap().clone())
    .menu(&menu)
    .on_menu_event(|app, event| match event.id.as_ref() {
        "open" => {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }
        "sync" => enqueue_sync_scan(app),
        "quit" => app.exit(0),
        _ => {}
    })
    .build(app)?;
```

And close-to-tray via `.on_window_event` on the builder:

```rust
.on_window_event(|window, event| {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        let tray_on = resolve_settings(window.app_handle())
            .map(|s| s.tray_enabled.unwrap_or(true))
            .unwrap_or(true);
        if tray_on {
            api.prevent_close();
            let _ = window.hide();
        }
    }
})
```

(`resolve_settings` = small helper reading pointer file + `read_settings`, extracted from `resolve_watched_folders` if one doesn't exist.)

- [x] **Step 3: Verify**

Run: `cd src-tauri && cargo test 2>&1 | tail -3` then `npm run tauri dev` — close the window: process stays, tray icon shows, Open restores, Quit exits. Toggle the Task 8 checkbox off, close: app exits.
Expected: all four behaviors correct.

- [x] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/lib.rs src-tauri/capabilities
git commit -m "feat(sync): close-to-tray with background sync and tray menu"
```

---

### Task 13: rclone integration tests + full verification

**Files:**
- Create: `src-tauri/tests/sync_integration.rs`

**Interfaces:**
- Consumes: `remote::*` arg builders; the bundled rclone binary at `src-tauri/binaries/rclone-x86_64-unknown-linux-gnu` (resolve relative to `CARGO_MANIFEST_DIR`, the same way existing ffmpeg integration tests find their binary — copy their resolution helper).

- [x] **Step 1: Write the integration test** (uses rclone's `local` backend — a temp dir, no network)

```rust
//! Round-trips the real bundled rclone against a `local` remote (a temp dir).
//! Mirrors how trim_integration drives the real ffmpeg binary.

use std::path::PathBuf;
use std::process::Command;

fn rclone_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("binaries/rclone-x86_64-unknown-linux-gnu")
}

fn run(args: &[String]) -> (bool, String) {
    let out = Command::new(rclone_bin()).args(args).output().expect("spawn rclone");
    (out.status.success(),
     String::from_utf8_lossy(&out.stdout).to_string()
         + &String::from_utf8_lossy(&out.stderr))
}

#[test]
fn upload_list_download_delete_roundtrip_on_local_remote() {
    let tmp = std::env::temp_dir().join(format!("clipsnip_sync_it_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("store")).unwrap();
    let conf = tmp.join("rclone.conf");

    // create the `clipsnip` remote as a local backend rooted anywhere;
    // remote paths below are absolute into our store dir
    let mut create = clipsnip_lib::remote::base_args(&conf);
    create.extend(["config".into(), "create".into(), "clipsnip".into(), "local".into()]);
    assert!(run(&create).0, "config create failed");

    let store = tmp.join("store").to_string_lossy().to_string();
    let dir = format!("clipsnip:{store}/ClipSnip/Clips");
    assert!(run(&clipsnip_lib::remote::mkdir_args(&conf, &dir)).0);

    let local = tmp.join("a.mp4");
    std::fs::write(&local, b"fake video bytes").unwrap();
    let remote_file = format!("{dir}/sub/a.mp4");
    let up = clipsnip_lib::remote::copyto_args(&conf, &local.to_string_lossy(), &remote_file);
    assert!(run(&up).0, "upload failed");
    // idempotent re-upload
    assert!(run(&up).0, "re-upload failed");

    let (ok, json) = run(&clipsnip_lib::remote::lsjson_args(&conf, &dir));
    assert!(ok);
    let files = clipsnip_lib::remote::parse_lsjson(&json).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "sub/a.mp4");
    assert_eq!(files[0].size, 16);

    let back = tmp.join("back.mp4");
    assert!(run(&clipsnip_lib::remote::download_args(&conf, &remote_file, &back.to_string_lossy())).0);
    assert_eq!(std::fs::read(&back).unwrap(), b"fake video bytes");

    assert!(run(&clipsnip_lib::remote::deletefile_args(&conf, &remote_file)).0);
    let (ok, json) = run(&clipsnip_lib::remote::lsjson_args(&conf, &dir));
    assert!(ok);
    assert!(clipsnip_lib::remote::parse_lsjson(&json).unwrap().is_empty());

    let _ = std::fs::remove_dir_all(&tmp);
}
```

(Check the crate's lib name in `src-tauri/Cargo.toml` `[lib] name` — existing integration tests show the right import path; replace `clipsnip_lib` accordingly. `remote` must be `pub mod remote;` in lib.rs for integration tests to reach it — make it so, matching however `export`/`media` are exposed to the existing tests.)

- [x] **Step 2: Run it**

Run: `cd src-tauri && cargo test --test sync_integration 2>&1 | tail -5`
Expected: PASS.

- [x] **Step 3: Full verification (all gates)**

```bash
npx vitest run
cd src-tauri && cargo test
cd .. && npm run build
```

Expected: every suite green, tsc clean. Fix anything red before proceeding.

- [x] **Step 4: Commit**

```bash
git add src-tauri/tests/sync_integration.rs src-tauri/src/lib.rs
git commit -m "test(sync): rclone local-remote integration round-trip"
```

---

## Self-Review Notes (completed)

- **Spec coverage:** sidecar (T1), remote wrapper + config crypt note (T2 — crypt deferred, see below), sync_state + identity clearing (T3/T6), size settings both paths (T4/T6/T8), pure planning/diff (T5), engine/commands/events/rescan/watcher hook (T6), api + helpers (T7), Storage tab (T8), diff window incl. lazy thumbnails + fail-fast + remote delete (T9), library delete-from-server (T10), wizard restore (T11), tray (T12), integration tests + gates (T13).
- **Deviation from spec, accepted:** rclone config-crypt (encrypting rclone.conf) is dropped from v1 — rclone already obscures stored passwords and OAuth tokens are equivalent-sensitivity to a config-crypt key stored beside the config, which adds no real protection. `settings.json` change: spec's "encrypted with config crypt" → plain config with rclone's standard password obscuring. If the user objects, add `RCLONE_CONFIG_PASS` handling in T2.
- **Type consistency check:** `rel_name`/`remote_name` = `/`-separated path relative to `Clips/` everywhere (T3 storage, T5 producer, T6 usage, T9/T10 lookups). `StorageStatus` field names match between T6 (serde camelCase) and T7 TS interface. `SyncSettings` camelCase over IPC matches `set_sync_settings` snake_case params via Tauri's automatic conversion — T7 passes them explicitly.
- **Known judgment calls for implementers:** exact `ExportDevice` method names (T4) and wizard validation function names (T11) must be read from the source, not assumed.
