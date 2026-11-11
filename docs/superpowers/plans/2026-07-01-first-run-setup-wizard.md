# First-Run Setup Wizard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a 3-step animated first-run wizard that lets the user choose a root
data directory for ClipSnip, pick how clips initially get into it, and persists that
choice — gating the app to show the wizard until setup is complete.

**Architecture:** Two Rust pure-logic-first modules extend the existing pattern (see
`export.rs`/`lib.rs`): `setup.rs` holds testable filesystem logic with no Tauri
dependency, and thin `#[tauri::command]` wrappers in `lib.rs` resolve the OS config
dir and call into it. The frontend gets a new `FirstRunWizard` component (with its own
pure validation module, mirroring `trim.ts`/`trim.test.ts`) that `App.tsx` renders
instead of the editor until a root directory is configured.

**Tech Stack:** Rust (`std::fs`, `serde`/`serde_json`, already-present deps only — no
new Cargo dependencies), React/TypeScript, Vitest, `@tauri-apps/plugin-dialog`.

## Global Constraints

- Do not run a blanket `cargo update` — `Cargo.lock` pins `time = 0.3.47` to avoid a
  `cookie@0.18.1` build break (see `.superpowers/sdd/progress.md` Task 1 notes). This
  plan adds no new Cargo dependencies, so this should not come up, but do not
  casually add one without checking this pin.
- No new Tauri capabilities are required: filesystem I/O is Rust-side `std::fs`
  (covered by `core:default`), and folder picking uses the already-granted
  `dialog:default` permission in `src-tauri/capabilities/default.json`.
- Frontend-facing Rust structs use `#[serde(rename_all = "camelCase")]` (matches
  `TrackConfig`/`ClipMeta` convention); Rust-internal files like `settings.json` use
  plain Rust field names (snake_case), since nothing in the frontend parses that file
  in this plan's scope.
- Tauri command invocation: JS `invoke(name, { fooBar })` auto-maps `fooBar` →
  `foo_bar` Rust parameter names (see existing `exportTrim`/`export_trim` pair in
  `src/api.ts` / `src-tauri/src/lib.rs`) — follow the same convention for new commands.

---

## Task 1: Rust setup module — pure filesystem logic

**Files:**
- Create: `src-tauri/src/setup.rs`
- Test: same file, `#[cfg(test)] mod tests` block

**Interfaces:**
- Produces (used by Task 2 and the frontend via commands):
  - `pub fn is_video_extension(path: &std::path::Path) -> bool`
  - `pub fn create_subfolders(root: &std::path::Path) -> Result<(), String>`
  - `pub fn is_dir_empty_or_absent(path: &std::path::Path) -> Result<bool, String>`
  - `pub fn copy_video_files(source: &std::path::Path, clips_dir: &std::path::Path) -> Result<(usize, Vec<String>), String>`
  - `pub fn is_writable(path: &std::path::Path) -> bool`
  - `pub fn write_settings(root: &std::path::Path, settings: &SetupSettings) -> Result<(), String>`
  - `pub fn write_pointer_file(config_dir: &std::path::Path, root_path: &str) -> Result<(), String>`
  - `pub fn read_pointer_file(config_dir: &std::path::Path) -> Option<String>`
  - `pub fn complete_setup_at(root: &std::path::Path, config_dir: &std::path::Path, ingestion_mode: &str, source_folder: Option<&std::path::Path>) -> Result<SetupResult, String>`
  - `pub struct SetupSettings { pub ingestion_mode: String, pub source_folder: Option<String>, pub created_at: u64 }`
  - `#[derive(Clone)] pub struct SetupResult { pub root_path: String, pub warnings: Vec<String> }` (serialized as `{"rootPath": ..., "warnings": [...]}`)

- [x] **Step 1: Write `src-tauri/src/setup.rs` with full implementation and tests together**

This module has no Tauri dependency, so implementation and tests are written together
in one file (matching `export.rs`'s style) rather than split into separate RED/GREEN
files. Create the file with this exact content:

```rust
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
```

- [x] **Step 2: Register the module in `src-tauri/src/lib.rs`**

Add `pub mod setup;` alongside the existing `pub mod export;` line (near the top of
the file, with the other `mod`/`pub mod` declarations):

```rust
mod probe;
pub mod export;
pub mod setup;
mod media_server;
```

- [x] **Step 3: Run the unit tests and verify they pass**

Run: `cd src-tauri && cargo test --lib setup::`
Expected: 14 tests pass (`is_video_extension_matches_known_containers_case_insensitively`,
`is_video_extension_rejects_others`, `create_subfolders_creates_clips_exports_cache`,
`is_dir_empty_or_absent_true_for_missing_path`,
`is_dir_empty_or_absent_true_for_empty_dir`,
`is_dir_empty_or_absent_false_when_populated`,
`copy_video_files_copies_only_recognized_extensions`,
`copy_video_files_collects_per_file_failures_without_aborting`,
`is_writable_true_for_a_normal_creatable_dir`,
`write_and_read_pointer_file_round_trips`, `read_pointer_file_returns_none_when_missing`,
`complete_setup_at_empty_mode_writes_settings_and_pointer_without_copying`,
`complete_setup_at_import_mode_copies_files_from_source`,
`complete_setup_at_import_mode_without_source_folder_errors`).

- [x] **Step 4: Commit**

```bash
git add src-tauri/src/setup.rs src-tauri/src/lib.rs
git commit -m "feat: add pure filesystem logic for first-run setup"
```

---

## Task 2: Tauri commands + integration test

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Create: `src-tauri/tests/setup_integration.rs`

**Interfaces:**
- Consumes: everything from Task 1 (`setup::complete_setup_at`, `setup::read_pointer_file`,
  `setup::is_dir_empty_or_absent`, `setup::is_writable`, `setup::SetupResult`).
- Produces (used by Task 4's `src/api.ts`): four new Tauri commands —
  `get_app_root() -> Result<Option<String>, String>`,
  `check_root_dir_status(path: String) -> Result<bool, String>`,
  `check_path_writable(path: String) -> bool`,
  `complete_setup(root_path: String, ingestion_mode: String, source_folder: Option<String>) -> Result<setup::SetupResult, String>`.

- [x] **Step 1: Add the three commands to `src-tauri/src/lib.rs`**

Add these new functions after the existing `export_trim` command (before the
`#[cfg_attr(mobile, ...)] pub fn run()` block):

```rust
#[tauri::command]
fn get_app_root(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    Ok(setup::read_pointer_file(&config_dir))
}

#[tauri::command]
fn check_root_dir_status(path: String) -> Result<bool, String> {
    setup::is_dir_empty_or_absent(std::path::Path::new(&path))
}

#[tauri::command]
fn check_path_writable(path: String) -> bool {
    setup::is_writable(std::path::Path::new(&path))
}

#[tauri::command]
fn complete_setup(
    app: tauri::AppHandle,
    root_path: String,
    ingestion_mode: String,
    source_folder: Option<String>,
) -> Result<setup::SetupResult, String> {
    let config_dir = app.path().app_config_dir().map_err(|e| e.to_string())?;
    let root = std::path::Path::new(&root_path);
    let source = source_folder.as_deref().map(std::path::Path::new);
    setup::complete_setup_at(root, &config_dir, &ingestion_mode, source)
}
```

- [x] **Step 2: Register the commands in the `invoke_handler`**

In the same file, update the `tauri::generate_handler!` call inside `pub fn run()`:

```rust
        .invoke_handler(tauri::generate_handler![
            probe_clip,
            export_trim,
            get_media_port,
            prepare_audio_tracks,
            get_app_root,
            check_root_dir_status,
            check_path_writable,
            complete_setup
        ])
```

- [x] **Step 3: Verify the crate builds**

Run: `cd src-tauri && cargo build`
Expected: builds with no errors (warnings about unused imports, if any, should be
fixed before moving on).

- [x] **Step 4: Write the integration test**

Create `src-tauri/tests/setup_integration.rs`:

```rust
use clipsnip_lib::setup::complete_setup_at;
use std::fs;
use std::path::PathBuf;

fn unique_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "clipsnip_setup_it_{name}_{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn complete_setup_end_to_end_import_mode_populates_clips_and_settings() {
    let root = unique_dir("root");
    let config_dir = unique_dir("config");
    let source = unique_dir("source");
    fs::write(source.join("gameplay.mp4"), b"fake video bytes").unwrap();
    fs::write(source.join("readme.txt"), b"not a clip").unwrap();

    let result = complete_setup_at(&root, &config_dir, "import", Some(&source)).unwrap();

    assert!(result.warnings.is_empty());
    assert!(root.join("Clips").join("gameplay.mp4").exists());
    assert!(!root.join("Clips").join("readme.txt").exists());
    assert!(root.join("Exports").is_dir());
    assert!(root.join("Cache").is_dir());
    assert!(root.join("settings.json").exists());
    assert!(config_dir.join("root.json").exists());

    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&config_dir);
    let _ = fs::remove_dir_all(&source);
}
```

- [x] **Step 5: Run the integration test and verify it passes**

Run: `cd src-tauri && cargo test --test setup_integration`
Expected: `complete_setup_end_to_end_import_mode_populates_clips_and_settings ... ok`

- [x] **Step 6: Run the full Rust test suite to confirm nothing else broke**

Run: `cd src-tauri && cargo test`
Expected: all tests pass (existing `export::` unit tests, existing
`trim_integration` test, and the new `setup::` + `setup_integration` tests).

- [x] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/tests/setup_integration.rs
git commit -m "feat: wire up first-run setup Tauri commands"
```

---

## Task 3: Frontend wizard validation logic

**Files:**
- Create: `src/wizardValidation.ts`
- Test: `src/wizardValidation.test.ts`

**Interfaces:**
- Produces (used by Task 4's `FirstRunWizard.tsx`):
  - `export type IngestionMode = "import" | "empty" | "watch"`
  - `export function canContinueFromRootStep(rootPath: string | null): boolean`
  - `export function canFinishIngestionStep(mode: IngestionMode | null, sourceFolder: string | null): boolean`

- [x] **Step 1: Write the failing test**

Create `src/wizardValidation.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { canContinueFromRootStep, canFinishIngestionStep } from "./wizardValidation";

describe("canContinueFromRootStep", () => {
  it("is false with no root path chosen", () => {
    expect(canContinueFromRootStep(null)).toBe(false);
  });
  it("is true once a root path is chosen", () => {
    expect(canContinueFromRootStep("/home/user/ClipSnip")).toBe(true);
  });
});

describe("canFinishIngestionStep", () => {
  it("is false with no mode selected", () => {
    expect(canFinishIngestionStep(null, null)).toBe(false);
  });
  it("is true for empty mode with no source folder", () => {
    expect(canFinishIngestionStep("empty", null)).toBe(true);
  });
  it("is false for import mode without a source folder", () => {
    expect(canFinishIngestionStep("import", null)).toBe(false);
  });
  it("is true for import mode with a source folder", () => {
    expect(canFinishIngestionStep("import", "/home/user/OBS")).toBe(true);
  });
  it("is false for watch mode without a source folder", () => {
    expect(canFinishIngestionStep("watch", null)).toBe(false);
  });
});
```

- [x] **Step 2: Run the test to verify it fails**

Run: `npx vitest run wizardValidation`
Expected: FAIL — `Cannot find module './wizardValidation'`

- [x] **Step 3: Write the implementation**

Create `src/wizardValidation.ts`:

```ts
export type IngestionMode = "import" | "empty" | "watch";

export function canContinueFromRootStep(rootPath: string | null): boolean {
  return rootPath !== null && rootPath.trim().length > 0;
}

export function canFinishIngestionStep(
  mode: IngestionMode | null,
  sourceFolder: string | null,
): boolean {
  if (mode === null) return false;
  if (mode === "import" || mode === "watch") {
    return sourceFolder !== null && sourceFolder.trim().length > 0;
  }
  return true;
}
```

- [x] **Step 4: Run the test to verify it passes**

Run: `npx vitest run wizardValidation`
Expected: PASS — 5 tests pass.

- [x] **Step 5: Commit**

```bash
git add src/wizardValidation.ts src/wizardValidation.test.ts
git commit -m "feat: add pure validation logic for first-run wizard steps"
```

---

## Task 4: FirstRunWizard component

**Files:**
- Modify: `src/api.ts`
- Create: `src/FirstRunWizard.tsx`
- Create: `src/FirstRunWizard.css`

**Interfaces:**
- Consumes: `setup::` Tauri commands from Task 2 (`get_app_root`, `check_root_dir_status`,
  `check_path_writable`, `complete_setup`); `canContinueFromRootStep`,
  `canFinishIngestionStep`, `IngestionMode` from Task 3.
- Produces (used by Task 5's `App.tsx`):
  - `export interface SetupResult { rootPath: string; warnings: string[] }` (in `api.ts`)
  - `export function getAppRoot(): Promise<string | null>` (in `api.ts`)
  - `export function completeSetup(rootPath: string, ingestionMode: IngestionMode, sourceFolder: string | null): Promise<SetupResult>` (in `api.ts`)
  - `export function FirstRunWizard(props: { onComplete: (result: SetupResult) => void }): JSX.Element` (in `FirstRunWizard.tsx`)

- [x] **Step 1: Add API bindings to `src/api.ts`**

Add these exports at the end of `src/api.ts` (after the existing `exportTrim`
function):

```ts
export interface SetupResult {
  rootPath: string;
  warnings: string[];
}

export function getAppRoot(): Promise<string | null> {
  return invoke<string | null>("get_app_root");
}

export function checkRootDirStatus(path: string): Promise<boolean> {
  return invoke<boolean>("check_root_dir_status", { path });
}

export function checkPathWritable(path: string): Promise<boolean> {
  return invoke<boolean>("check_path_writable", { path });
}

export function completeSetup(
  rootPath: string,
  ingestionMode: "import" | "empty" | "watch",
  sourceFolder: string | null,
): Promise<SetupResult> {
  return invoke<SetupResult>("complete_setup", { rootPath, ingestionMode, sourceFolder });
}
```

- [x] **Step 2: Create `src/FirstRunWizard.css`**

```css
.wiz-stage {
  position: fixed;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: #06060a;
  overflow: hidden;
}

.wiz-blob {
  position: absolute;
  border-radius: 50%;
  filter: blur(45px);
}

.wiz-blob-1 { width: 280px; height: 280px; background: #312e81; opacity: .5; top: -70px; left: -50px; animation: wizFloatA 10s ease-in-out infinite; }
.wiz-blob-2 { width: 250px; height: 250px; background: #164e63; opacity: .45; bottom: -70px; right: -40px; animation: wizFloatB 12s ease-in-out infinite; }
.wiz-blob-3 { width: 200px; height: 200px; background: #4a044e; opacity: .4; top: 60px; right: 40px; animation: wizFloatC 8s ease-in-out infinite; }

@keyframes wizFloatA { 0%, 100% { transform: translate(0, 0); } 50% { transform: translate(40px, 30px); } }
@keyframes wizFloatB { 0%, 100% { transform: translate(0, 0); } 50% { transform: translate(-30px, -20px); } }
@keyframes wizFloatC { 0%, 100% { transform: translate(0, 0); } 50% { transform: translate(-20px, 40px); } }

.wiz-panel {
  position: relative;
  z-index: 2;
  width: 420px;
  max-width: calc(100vw - 40px);
  padding: 32px 30px;
  border-radius: 16px;
  background: rgba(15, 15, 22, .55);
  backdrop-filter: blur(18px);
  border: 1px solid rgba(255, 255, 255, .08);
  box-shadow: 0 20px 60px rgba(0, 0, 0, .5);
  color: #fff;
}

.wiz-dots { display: flex; gap: 8px; justify-content: center; margin-bottom: 22px; }
.wiz-dot { width: 7px; height: 7px; border-radius: 50%; background: rgba(255, 255, 255, .25); }
.wiz-dot.active { background: #a78bfa; width: 20px; border-radius: 4px; }

.wiz-title { font-size: 19px; font-weight: 600; margin-bottom: 8px; text-align: center; }
.wiz-desc { font-size: 13px; color: rgba(255, 255, 255, .65); text-align: center; margin-bottom: 22px; line-height: 1.5; }

.wiz-pickbox {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 14px;
  border-radius: 10px;
  background: rgba(255, 255, 255, .06);
  border: 1px solid rgba(255, 255, 255, .1);
  margin-bottom: 14px;
}

.wiz-path {
  flex: 1;
  font-size: 12px;
  color: rgba(255, 255, 255, .55);
  font-family: monospace;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.wiz-btn-choose {
  padding: 7px 14px;
  border-radius: 8px;
  background: rgba(167, 139, 250, .18);
  color: #c4b5fd;
  font-size: 12px;
  font-weight: 600;
  border: 1px solid rgba(167, 139, 250, .3);
  white-space: nowrap;
  cursor: pointer;
}
.wiz-btn-choose:hover { background: rgba(167, 139, 250, .28); }

.wiz-warning {
  font-size: 12px;
  color: #ddaa44;
  background: rgba(90, 64, 0, .25);
  border: 1px solid rgba(90, 64, 0, .5);
  border-radius: 8px;
  padding: 8px 10px;
  margin-bottom: 18px;
  line-height: 1.5;
}
.wiz-warning.wiz-error { color: #ff8080; background: rgba(90, 20, 20, .25); border-color: rgba(90, 20, 20, .5); }

.wiz-modes { display: flex; flex-direction: column; gap: 8px; margin-bottom: 14px; }

.wiz-mode-option {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 10px 12px;
  border-radius: 10px;
  background: rgba(255, 255, 255, .04);
  border: 1px solid rgba(255, 255, 255, .08);
  cursor: pointer;
}
.wiz-mode-option.selected { border-color: #a78bfa; background: rgba(167, 139, 250, .12); }
.wiz-mode-option input[type="radio"] { margin-top: 3px; accent-color: #a78bfa; }
.wiz-mode-title { font-size: 13px; font-weight: 600; color: #fff; }
.wiz-mode-desc { font-size: 11px; color: rgba(255, 255, 255, .55); margin-top: 2px; }

.wiz-nav { display: flex; justify-content: space-between; align-items: center; }
.wiz-nav-single { justify-content: center; }

.wiz-back {
  font-size: 13px;
  color: rgba(255, 255, 255, .5);
  background: transparent;
  border: none;
  cursor: pointer;
  padding: 6px 4px;
}
.wiz-back:hover { color: rgba(255, 255, 255, .8); }
.wiz-back:disabled { opacity: .4; cursor: default; }

.wiz-next {
  padding: 10px 22px;
  border-radius: 10px;
  background: linear-gradient(135deg, #8b5cf6, #6366f1);
  color: #fff;
  font-size: 13px;
  font-weight: 600;
  border: none;
  box-shadow: 0 8px 20px rgba(99, 102, 241, .35);
  cursor: pointer;
}
.wiz-next:hover { filter: brightness(1.08); }
.wiz-next:disabled { opacity: .4; cursor: default; filter: none; box-shadow: none; }
```

- [x] **Step 3: Create `src/FirstRunWizard.tsx`**

```tsx
import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { checkRootDirStatus, checkPathWritable, completeSetup, type SetupResult } from "./api";
import { canContinueFromRootStep, canFinishIngestionStep, type IngestionMode } from "./wizardValidation";
import "./FirstRunWizard.css";

interface FirstRunWizardProps {
  onComplete: (result: SetupResult) => void;
}

export function FirstRunWizard({ onComplete }: FirstRunWizardProps) {
  const [step, setStep] = useState<0 | 1 | 2>(0);
  const [rootPath, setRootPath] = useState<string | null>(null);
  const [rootNonEmptyWarning, setRootNonEmptyWarning] = useState(false);
  const [rootWritable, setRootWritable] = useState(true);
  const [mode, setMode] = useState<IngestionMode | null>(null);
  const [sourceFolder, setSourceFolder] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function pickRoot() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    setRootPath(selected);
    try {
      const empty = await checkRootDirStatus(selected);
      setRootNonEmptyWarning(!empty);
    } catch {
      setRootNonEmptyWarning(false);
    }
    try {
      setRootWritable(await checkPathWritable(selected));
    } catch {
      setRootWritable(false);
    }
  }

  async function pickSourceFolder() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    setSourceFolder(selected);
  }

  async function finish() {
    if (!rootPath || !mode) return;
    setFinishing(true);
    setError(null);
    try {
      const result = await completeSetup(rootPath, mode, sourceFolder);
      onComplete(result);
    } catch (e) {
      setError(String(e));
      setFinishing(false);
    }
  }

  return (
    <div className="wiz-stage">
      <div className="wiz-blob wiz-blob-1" />
      <div className="wiz-blob wiz-blob-2" />
      <div className="wiz-blob wiz-blob-3" />

      <div className="wiz-panel">
        <div className="wiz-dots">
          <div className={`wiz-dot${step === 0 ? " active" : ""}`} />
          <div className={`wiz-dot${step === 1 ? " active" : ""}`} />
          <div className={`wiz-dot${step === 2 ? " active" : ""}`} />
        </div>

        {step === 0 && (
          <>
            <div className="wiz-title">Welcome to ClipSnip</div>
            <div className="wiz-desc">
              Fast trimming for your game clips. Let's set up where your clips and
              exports will live.
            </div>
            <div className="wiz-nav wiz-nav-single">
              <button className="wiz-next" onClick={() => setStep(1)}>Get Started</button>
            </div>
          </>
        )}

        {step === 1 && (
          <>
            <div className="wiz-title">Where should ClipSnip live?</div>
            <div className="wiz-desc">
              Pick a folder to store your clips, exports, and settings. You can change
              this later.
            </div>
            <div className="wiz-pickbox">
              <span className="wiz-path">{rootPath ?? "No folder chosen"}</span>
              <button className="wiz-btn-choose" onClick={pickRoot}>Choose folder</button>
            </div>
            {rootNonEmptyWarning && (
              <div className="wiz-warning">
                This folder isn't empty — ClipSnip will add Clips/Exports/Cache
                subfolders here.
              </div>
            )}
            {!rootWritable && (
              <div className="wiz-warning wiz-error">
                ClipSnip can't write to this folder — choose a different one.
              </div>
            )}
            <div className="wiz-nav">
              <button className="wiz-back" onClick={() => setStep(0)}>&larr; Back</button>
              <button
                className="wiz-next"
                disabled={!canContinueFromRootStep(rootPath) || !rootWritable}
                onClick={() => setStep(2)}
              >
                Continue
              </button>
            </div>
          </>
        )}

        {step === 2 && (
          <>
            <div className="wiz-title">How should ClipSnip get your clips?</div>
            <div className="wiz-desc">Choose one — you can change this later.</div>
            <div className="wiz-modes">
              <label className={`wiz-mode-option${mode === "import" ? " selected" : ""}`}>
                <input
                  type="radio"
                  name="ingestion-mode"
                  checked={mode === "import"}
                  onChange={() => setMode("import")}
                />
                <div>
                  <div className="wiz-mode-title">Import existing clips</div>
                  <div className="wiz-mode-desc">Copy clips from a folder into ClipSnip now.</div>
                </div>
              </label>
              <label className={`wiz-mode-option${mode === "empty" ? " selected" : ""}`}>
                <input
                  type="radio"
                  name="ingestion-mode"
                  checked={mode === "empty"}
                  onChange={() => setMode("empty")}
                />
                <div>
                  <div className="wiz-mode-title">Start empty</div>
                  <div className="wiz-mode-desc">Add clips later from the editor.</div>
                </div>
              </label>
              <label className={`wiz-mode-option${mode === "watch" ? " selected" : ""}`}>
                <input
                  type="radio"
                  name="ingestion-mode"
                  checked={mode === "watch"}
                  onChange={() => setMode("watch")}
                />
                <div>
                  <div className="wiz-mode-title">Watch a folder</div>
                  <div className="wiz-mode-desc">Remember a recorder's output folder for later.</div>
                </div>
              </label>
            </div>

            {(mode === "import" || mode === "watch") && (
              <div className="wiz-pickbox">
                <span className="wiz-path">{sourceFolder ?? "No folder chosen"}</span>
                <button className="wiz-btn-choose" onClick={pickSourceFolder}>Choose folder</button>
              </div>
            )}

            {error && <div className="wiz-warning wiz-error">{error}</div>}

            <div className="wiz-nav">
              <button className="wiz-back" onClick={() => setStep(1)} disabled={finishing}>&larr; Back</button>
              <button
                className="wiz-next"
                disabled={!canFinishIngestionStep(mode, sourceFolder) || finishing}
                onClick={finish}
              >
                {finishing ? "Setting up…" : "Finish"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
```

- [x] **Step 4: Type-check the project**

Run: `npx tsc --noEmit`
Expected: no errors (this only type-checks; `FirstRunWizard` isn't wired into `App.tsx`
yet, so there's no unused-export warning — `tsc` doesn't flag unused exports by
default).

- [x] **Step 5: Commit**

```bash
git add src/api.ts src/FirstRunWizard.tsx src/FirstRunWizard.css
git commit -m "feat: add FirstRunWizard component with Midnight aurora background"
```

---

## Task 5: Gate App.tsx behind the wizard

**Files:**
- Modify: `src/App.tsx`

**Interfaces:**
- Consumes: `getAppRoot`, `type SetupResult` from `src/api.ts` (Task 4);
  `FirstRunWizard` from `src/FirstRunWizard.tsx` (Task 4).

- [x] **Step 1: Add imports**

In `src/App.tsx`, update the existing import block near the top of the file:

```tsx
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { probeClip, exportTrim, prepareAudioTracks, getAppRoot, type ClipMeta, type TrackConfig, type SetupResult } from "./api";
import { setIn, setOut } from "./trim";
import { Timeline } from "./Timeline";
import { FirstRunWizard } from "./FirstRunWizard";
import "./App.css";
```

- [x] **Step 2: Add root-gating state and effect**

Inside the `App` function, add this state near the top (right after the existing
`const [mediaPort, setMediaPort] = useState<number | null>(null);` line):

```tsx
  // `undefined` = still checking, `null` = needs first-run setup, string = configured root
  const [appRoot, setAppRoot] = useState<string | null | undefined>(undefined);
```

Add this effect near the existing `useEffect(() => { invoke<number>("get_media_port")...`
block:

```tsx
  useEffect(() => {
    getAppRoot()
      .then(setAppRoot)
      .catch(() => setAppRoot(null));
  }, []);
```

- [x] **Step 3: Add the setup-complete handler**

Add this function inside `App`, near `doExport`/`openClip` (any point after `status`
state is declared):

```tsx
  function handleSetupComplete(result: SetupResult) {
    setAppRoot(result.rootPath);
    if (result.warnings.length > 0) {
      setStatus(`Setup finished with warnings: ${result.warnings.join("; ")}`);
    }
  }
```

- [x] **Step 4: Branch the render on `appRoot`**

Find the `// ── Render` comment and the `return (` that starts with
`<div className="app-layout">`. Insert the gating branches immediately before that
existing `return (`:

```tsx
  // ── Render ────────────────────────────────────────────────────────────────────
  if (appRoot === undefined) {
    return null;
  }

  if (appRoot === null) {
    return <FirstRunWizard onComplete={handleSetupComplete} />;
  }

  return (
    <div className="app-layout">
```

(The rest of the existing JSX inside that `return` block is unchanged.)

- [x] **Step 5: Type-check and build**

Run: `npx tsc --noEmit && npm run build`
Expected: both succeed with no errors.

- [x] **Step 6: Manual smoke test**

Run: `npm run tauri dev`

Verify:
1. On first launch (no `~/.config/clipsnip/root.json` on Linux — delete it first if
   it exists from prior testing), the app shows the wizard, not the editor.
2. Click through all 3 steps: Welcome → choose a root folder → pick "Start empty" →
   Finish.
3. After Finish, the existing editor UI appears (library sidebar, "Open clip…").
   Confirm the chosen root folder now contains `Clips/`, `Exports/`, `Cache/`, and
   `settings.json`.
4. Quit and relaunch the app (`npm run tauri dev` again, or just reload the window) —
   it should go straight to the editor, skipping the wizard.
5. **Visual check (per design spec §5):** confirm the aurora blobs animate smoothly
   and the glass panel's blur/transparency renders acceptably in the real WebKitGTK
   webview, not just a browser. If `backdrop-filter` doesn't render (panel looks
   fully opaque or fully transparent instead of frosted), note it — a fallback of a
   solid `rgba(15,15,22,.92)` panel background (drop the `backdrop-filter` line in
   `FirstRunWizard.css`) is the documented mitigation in the design spec, apply it if
   needed.
6. Test the "Import existing clips" mode with a folder containing a real video file
   (or reuse a clip from earlier manual testing) and confirm it's copied into
   `Clips/` after Finish.

- [x] **Step 7: Commit**

```bash
git add src/App.tsx
git commit -m "feat: gate app launch behind first-run setup wizard"
```
