# Remote storage sync (SFTP / Dropbox / Google Drive) via rclone sidecar

Date: 2026-07-06
Status: approved for implementation (design reviewed with user section-by-section)

## Problem

Clips live only on the local PC. The user wants ClipSnip to mirror the `Clips/`
folder to a single connected remote (their own server over SSH, or a cloud
provider), so that:

1. New recordings dropped into `Clips/` by third-party capture software upload
   automatically, even while the ClipSnip window is closed.
2. The remote is a durable archive: deleting a clip locally never deletes it
   remotely, and remote files are only ever deleted through ClipSnip's UI.
3. A manual "scan for differences" flow shows what exists on one side but not
   the other (thumbnail, name, date, size) and lets the user selectively import
   remote clips back to the PC — including a full restore onto a fresh install.
4. Upload copies and new local recordings can independently be re-encoded down
   to a percentage of their original file size.

## Decisions made during brainstorming

- **Backends v1:** SFTP/SSH, Dropbox, Google Drive. iCloud is excluded (Apple
  offers no third-party desktop API). SMB and FTP are deferred (rclone supports
  both, so adding them later is config + UI only).
- **One remote at a time.** Disconnect/reconnect switches remotes.
- **Storage layer = rclone sidecar** (Approach A), bundled like ffmpeg/ffprobe.
  Chosen over native Rust crates (3–4× the work, requires registering our own
  OAuth apps) and over mounted-folders-only (fails the "configure everything in
  app settings" requirement).
- **Remote holds metadata too:** library index + thumbnails, so a fresh install
  restores names/favorites/folders, and the diff window can show thumbnails for
  remote-only clips.
- **Close-to-tray** keeps sync running in the background; a periodic rescan
  (every 5 minutes) backs up the live filesystem watcher.
- **Local-shrink setting** applies to *new* recordings arriving in `Clips/`:
  after the (full-size, if so configured) upload completes, the local file is
  re-encoded down and only the shrunk version is kept locally.

## Design

### rclone sidecar + `remote.rs`

- Add `binaries/rclone` as a third Tauri `externalBin` sidecar (per-platform
  binaries fetched the same way ffmpeg/ffprobe are).
- New module `src-tauri/src/remote.rs`: a thin typed wrapper that builds and
  runs rclone commands and parses their JSON output. Operations:
  - `connect_check()` — verify credentials / reachability (`rclone lsd`).
  - `ensure_layout()` — create `ClipSnip/{Clips,Thumbnails}` on the remote.
  - `list(dir)` — `rclone lsjson` → `Vec<RemoteFile { name, size, mod_time }>`.
  - `upload(local, remote)` / `download(remote, local)` — `rclone copyto`
    (idempotent, overwrites partials, verifies size/checksum).
  - `delete(remote)` — `rclone deletefile`.
- ClipSnip owns a private rclone config at `<config_dir>/rclone.conf`,
  encrypted with rclone's config-crypt (key stored alongside; goal is
  at-rest scrambling, not defense against a local attacker — same trust model
  as rclone's own default). The config holds exactly one remote named
  `clipsnip`.
- Provider connect flows:
  - *SFTP:* `rclone config create clipsnip sftp host=… user=… port=…` with
    either `pass` (rclone-obscured) or `key_file`. Remote base path is stored
    in settings, not in the rclone remote.
  - *Dropbox / Google Drive:* `rclone config create clipsnip dropbox|drive`
    triggers rclone's built-in OAuth: it serves a localhost callback and opens
    the system browser; the resulting token (with auto-refresh) lands in the
    config. Uses rclone's bundled client IDs — no developer-app registration.

### Remote layout

```
<base path or account root>/ClipSnip/
  Clips/              uploaded videos (possibly size-reduced per upload setting)
  Thumbnails/         <video-stem>.jpg per clip, powers the diff window
  library-index.json  export_index() snapshot (names, favorites, folders)
```

### Sync engine (`sync.rs`)

- New sqlite table in the existing `library.db`:

  ```sql
  CREATE TABLE IF NOT EXISTS sync_state (
    path TEXT PRIMARY KEY,       -- local absolute path
    size_bytes INTEGER NOT NULL, -- local size at upload time
    mtime INTEGER NOT NULL,      -- local mtime at upload time
    remote_name TEXT NOT NULL,   -- path under ClipSnip/Clips/
    uploaded_at INTEGER NOT NULL
  );
  ```

  `remote_name` is the file's path *relative to the local `Clips/` folder*
  (the watcher scans recursively), so remote `ClipSnip/Clips/` mirrors local
  subfolders and same-named files in different subfolders cannot collide.
  A `remote_identity` value (provider + host/account fingerprint from
  `rclone config show`) is stored in `settings.json`; connecting a remote with
  a different identity clears `sync_state`, so stale "already uploaded" rows
  can never suppress uploads to a new remote. Reconnecting the same remote
  keeps them and avoids re-uploading.

- **Queue:** single background worker, uploads serialized one at a time.
  Sources of work: (a) the existing `notify` watcher events for `Clips/`,
  (b) a 5-minute rescan tick, (c) a startup scan. Each source enqueues files
  present in `Clips/` but absent from `sync_state` (or changed size/mtime).
  The queue is derived state — it is rebuilt from disk vs. `sync_state`, so
  nothing is lost across restarts or crashes.
- **Per-clip pipeline:** wait for write-completion (existing watcher logic) →
  probe → thumbnail → if upload-size < 100%: re-encode to temp in `Cache/` →
  upload video → upload thumbnail → regenerate + upload `library-index.json` →
  insert `sync_state` row → if local-shrink is on: re-encode local file to
  temp, atomically rename over the original, update library row and
  `sync_state` (new size/mtime so it isn't re-uploaded).
- **Failure handling:** an item only enters `sync_state` after rclone exits 0
  for the video upload. Failed items retry with exponential backoff and again
  on every rescan tick. Status (idle / syncing n of m / error message) is
  emitted as Tauri events to the frontend.
- **Deletes:** sync never downloads and never deletes. A locally-deleted clip
  simply remains on the remote (its `sync_state` row is removed when the
  library row is removed, so the diff scan reports it as remote-only).
  "Delete from server" = `remote.delete(video + thumbnail)` + remove
  `sync_state` row + refresh remote index. Confirm dialog required.

### Size reduction

- Shared helper: target_mb = local file size × percent; feed the existing
  `compute_target_bitrates(target_mb, duration_secs)` (`export.rs`) and encode
  with the user's configured export device (GPU-capable). If the percent target
  is below the bitrate floor, clamp to the floor and surface a note in sync
  status rather than failing.
- Two independent settings, both defaulting to 100% / off:
  - **Upload size** (100 / 75 / 50 / 25 / custom %): applies to the uploaded
    copy only; encode goes to `Cache/`, original untouched.
  - **Local recording size** (off / 75 / 50 / 25 / custom %): applies to new
    recordings after upload; encode-to-temp then atomic rename, original never
    deleted before the replacement is fully written and probed valid.
- Custom % accepts 1–99 (integer). Settings changes affect future files only.
- Both settings persist in `settings.json` (`upload_size_percent: Option<u8>`,
  `local_size_percent: Option<u8>`, `#[serde(default)]` for backward compat,
  `None` = full size), alongside new `remote_provider: Option<String>` and
  `remote_base_path: Option<String>`.

### Settings UI — new "Storage" tab in `SettingsModal`

- **Disconnected:** provider picker (SFTP / Dropbox / Google Drive).
  - SFTP form: host, port (default 22), username, password or key-file picker,
    remote base path. "Test & Connect" runs `connect_check` + `ensure_layout`.
  - Dropbox / Google Drive: "Sign in with …" button → browser OAuth →
    `ensure_layout` at account root.
- **Connected:** provider label, status dot (synced / syncing / error + message),
  "n clips synced", Disconnect button. Disconnect deletes credentials
  (rclone config) but keeps `sync_state`; reconnecting the same remote
  (matching `remote_identity`) does not re-upload everything, while a
  different remote starts from a cleared `sync_state`.
- Upload-size and local-size dropdowns (with custom % input), the local one
  carrying a warning that it permanently re-encodes new local recordings.
- "Scan for differences" button → diff window.

### Diff window

- Compares `Clips/` (disk) against `remote.list("ClipSnip/Clips")`, matching by
  filename; produces two groups:
  - **On server, not on this PC** — row: thumbnail (lazily downloaded from
    `Thumbnails/`, cached in `Cache/`), display name (from remote index when
    available, else filename), date (remote mod-time), size, checkbox, and a
    per-row "Delete from server" button.
  - **On this PC, not on server** — same row shape (local thumbnail/metadata),
    checkbox.
- Footer actions: "Import selected to PC" (download into `Clips/`, upsert
  library rows applying names/favorites/folders from the remote index, insert
  `sync_state` rows) and "Upload selected to server" (enqueue on the normal
  pipeline). Per-transfer progress, cancellable between files.
- If the remote is unreachable the scan fails fast with an error — never a
  partial diff.

### Fresh-install restore

- The first-run wizard gains a "Restore from server or cloud" option: same
  connect forms as settings; on success it downloads `library-index.json` and
  opens the diff window with every remote clip pre-checked for import.

### Tray / background running

- New setting "Keep running in tray when window is closed" (default on).
- Window close → hide + tray icon (Tauri tray API). Tray menu: Open ClipSnip /
  Sync now / Quit. Watcher, rescan timer, and upload queue keep running while
  hidden. Quit tears down cleanly (current upload finishes or is killed; state
  is consistent either way because `sync_state` is written only after success).

### Error handling summary

- Remote unreachable / auth expired: queue items stay pending, status dot red
  with message, exponential backoff + retry on rescan ticks. OAuth refresh is
  rclone's job; if refresh fails the Storage tab shows "Reconnect needed" with
  a re-auth button.
- Partial uploads: `copyto` re-runs idempotently and overwrites.
- Re-encodes never destroy an original before the replacement is verified.
- Diff scan is all-or-nothing.

## Testing

- **TS unit tests** (co-located, pure logic): `syncDiff.ts` (local vs remote
  listing comparison, grouping, name resolution), size-percent → target-MB
  math and custom-% validation, storage-settings form validation.
- **Rust unit tests:** rclone arg-builders (config create, lsjson, copyto,
  deletefile command construction), `sync_state` CRUD + queue-rebuild logic
  against in-memory sqlite, remote-index merge logic for import.
- **Rust integration tests** (`src-tauri/tests/`): drive the real bundled
  rclone against an rclone `local` remote (temp dir) — connect, layout,
  upload/list/download/delete round-trip, idempotent re-upload — no network
  needed. One ffmpeg integration test for the percent-size re-encode path.

## Out of scope (v1)

- SMB and FTP backends (rclone makes them cheap later; UI only).
- iCloud (no third-party API).
- Multiple simultaneous remotes.
- Automatic download / two-way sync / conflict resolution.
- Syncing `Exports/` or `Cache/`.
- Bandwidth limiting and scheduling.
