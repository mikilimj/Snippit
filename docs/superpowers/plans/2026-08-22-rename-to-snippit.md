# Rename ClipSnip → Snippit Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebrand the ClipSnip desktop app to Snippit across code, config, packaging, CI, and (with explicit confirmation) the GitHub repo itself — with no functional regressions.

**Architecture:** This is a mechanical rename, not new logic. Every task is a find/replace pass over a known set of files (enumerated below from a full-repo case-insensitive grep for `clipsnip`), followed by a build/test verification instead of red/green TDD. The casing rule is fixed and mechanical:
- `ClipSnip` → `Snippit` (product name, titles, folder names, doc-comment prose)
- `clipsnip` → `snippit` (package/crate/binary names, identifiers, localStorage keys, temp-file prefixes)
- `clipsnip_lib` → `snippit_lib` (Rust lib crate name)
- `pl.medportal.clipsnip` → `pl.medportal.snippit` (Tauri app identifier)

## Global Constraints

- **Do not touch `docs/superpowers/plans/*.md` or `docs/superpowers/specs/*.md`** (except this new file). Those are point-in-time historical records of past work — rewriting them to say "Snippit" would falsify history. Confirmed by user decision during planning.
- **Bundle identifier changes too** (`pl.medportal.clipsnip` → `pl.medportal.snippit`). User confirmed this explicitly, accepting that any existing installed copy of the app will have its app-data path change — on next launch it will look empty/unconfigured and the first-run wizard will run again. This also resets the internal rclone remote (Task 4) since that remote's identity and folder name are being renamed too.
- **AUR package names change** (`clipsnip`/`clipsnip-bin` → `snippit`/`snippit-bin`). User confirmed this explicitly, accepting that existing AUR users get no automatic upgrade path — they must manually uninstall the old package and install the new one.
- **GitHub repo rename is gated** (Task 10): the plan includes the `gh repo rename` step, but whoever executes this plan MUST pause and get explicit user go-ahead immediately before running it, per the safety rules around account-level, hard-to-reverse actions. Do not run it as part of an unattended batch.
- No lint script exists; `npm run build` (tsc) is the frontend type-check gate. `cd src-tauri && cargo test` is the Rust gate (includes real ffmpeg/ffprobe integration tests — slow, but must be run at least once at the end).
- Never run `cargo update` — Cargo.lock's local-package name entry updates automatically from a plain `cargo build`/`cargo check` once `Cargo.toml`'s `name` field changes; that's the only lockfile change expected.

---

### Task 1: Rust crate rename (package + lib name)

**Files:**
- Modify: `src-tauri/Cargo.toml:2` (`name = "clipsnip"` → `name = "snippit"`), `:14` (`name = "clipsnip_lib"` → `name = "snippit_lib"`)
- Modify: `src-tauri/src/main.rs:5` (`clipsnip_lib::run()` → `snippit_lib::run()`)
- Modify (import path `clipsnip_lib::` → `snippit_lib::`, one or more occurrences each): `src-tauri/tests/proxy_integration.rs:3`, `src-tauri/tests/preview_audio_integration.rs:3`, `src-tauri/tests/thumbnail_integration.rs:3`, `src-tauri/tests/vaapi_smoke.rs:9`, `src-tauri/tests/setup_integration.rs:1`, `src-tauri/tests/trim_integration.rs:3`, `src-tauri/tests/sync_integration.rs:29,30,34,35,40,45,47,53,56,57,59` (also the local-var literal `clipsnip:` remote-name-prefixed path strings in this file get updated together with Task 4, since they encode the remote name), `src-tauri/tests/compress_integration.rs:3`, `src-tauri/tests/sequence_integration.rs:3`
- Regenerate: `src-tauri/Cargo.lock` (automatic, via build/check in the verify step)

- [ ] **Step 1: Edit `Cargo.toml`**

In `src-tauri/Cargo.toml`, change:
```toml
[package]
name = "clipsnip"
```
to:
```toml
[package]
name = "snippit"
```
and change:
```toml
[lib]
name = "clipsnip_lib"
```
to:
```toml
[lib]
name = "snippit_lib"
```

- [ ] **Step 2: Update `main.rs`**

In `src-tauri/src/main.rs`, change:
```rust
    clipsnip_lib::run()
```
to:
```rust
    snippit_lib::run()
```

- [ ] **Step 3: Update every integration test's crate import**

In each of the 8 files listed above under `src-tauri/tests/`, replace every `clipsnip_lib::` with `snippit_lib::` (this is a compile-time requirement, not cosmetic — the crate no longer exists under the old name after Step 1).

- [ ] **Step 4: Regenerate Cargo.lock and verify the build**

Run: `cd src-tauri && cargo check --tests 2>&1 | tail -40`
Expected: compiles cleanly. `git diff Cargo.lock` should show only the local package's `name` field changing (from `clipsnip`/`clipsnip_lib` to `snippit`/`snippit_lib`), no dependency version changes.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/main.rs src-tauri/tests/
git commit -m "chore: rename Rust crate clipsnip -> snippit"
```

---

### Task 2: Tauri app identity

**Files:**
- Modify: `src-tauri/tauri.conf.json:3,5,15`
- Modify: `src-tauri/capabilities/default.json:4`
- Modify: `index.html:7`

- [ ] **Step 1: Edit `tauri.conf.json`**

Change:
```json
  "productName": "ClipSnip",
```
to:
```json
  "productName": "Snippit",
```
Change:
```json
  "identifier": "pl.medportal.clipsnip",
```
to:
```json
  "identifier": "pl.medportal.snippit",
```
Change:
```json
        "title": "ClipSnip",
```
to:
```json
        "title": "Snippit",
```

- [ ] **Step 2: Edit `capabilities/default.json`**

Change:
```json
  "description": "Default capability for ClipSnip",
```
to:
```json
  "description": "Default capability for Snippit",
```

- [ ] **Step 3: Edit `index.html`**

Change:
```html
    <title>ClipSnip</title>
```
to:
```html
    <title>Snippit</title>
```

- [ ] **Step 4: Verify with a dev run**

Run: `npm run tauri dev`
Expected: native window opens titled "Snippit". Close it (Ctrl-C) once confirmed.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/tauri.conf.json src-tauri/capabilities/default.json index.html
git commit -m "chore: rebrand app identity to Snippit"
```

---

### Task 3: Frontend branding strings

**Files:**
- Modify: `src/App.tsx:71,78,845,1071`
- Modify: `src/FirstRunWizard.tsx:61,158,171,182,188,206,218`
- Modify: `src/StorageSettings.tsx:192`
- Modify: `src/SettingsModal.tsx:101`
- Modify: `src/theme.css:1`
- Modify: `src/wizardValidation.test.ts:9`

- [ ] **Step 1: `App.tsx` — localStorage keys and headings**

Line 71: `storageKey: "clipsnip:sidebarWidth",` → `storageKey: "snippit:sidebarWidth",`
Line 78: `storageKey: "clipsnip:videoMaxHeightVh",` → `storageKey: "snippit:videoMaxHeightVh",`
Line 845: `<span className="brand-name">ClipSnip</span>` → `<span className="brand-name">Snippit</span>`
Line 1071: `<h1>ClipSnip</h1>` → `<h1>Snippit</h1>`

(Renaming the localStorage keys means any existing user's saved sidebar/video-panel sizes reset to defaults on first load post-rename — consistent with the identifier reset already accepted in Task 2.)

- [ ] **Step 2: `FirstRunWizard.tsx` — wizard copy and suggested folder name**

Line 61: `const suggested = await join(await videoDir(), "ClipSnip");` → `const suggested = await join(await videoDir(), "Snippit");`
Line 158: `<div className="wiz-title">Welcome to ClipSnip</div>` → `Welcome to Snippit`
Line 171: `<div className="wiz-title">Where should ClipSnip live?</div>` → `Where should Snippit live?`
Line 182: `This folder isn't empty — ClipSnip will add Clips/Exports/Cache` → `... Snippit will add ...`
Line 188: `ClipSnip can't write to this folder — choose a different one.` → `Snippit can't write ...`
Line 206: `<div className="wiz-title">How should ClipSnip get your clips?</div>` → `How should Snippit get your clips?`
Line 218: `<div className="wiz-mode-desc">Copy clips from a folder into ClipSnip now.</div>` → `... into Snippit now.`

- [ ] **Step 3: `StorageSettings.tsx`, `SettingsModal.tsx`, `theme.css`, `wizardValidation.test.ts`**

`StorageSettings.tsx:192`: `placeholder="/ClipSnip"` → `placeholder="/Snippit"`
`SettingsModal.tsx:101`: `defaultPath: "clipsnip-library.zip"` → `defaultPath: "snippit-library.zip"`
`theme.css:1`: comment `/* ─── ClipSnip "Midnight aurora" theme tokens ...` → `/* ─── Snippit "Midnight aurora" theme tokens ...`
`wizardValidation.test.ts:9`: `expect(canContinueFromRootStep("/home/user/ClipSnip")).toBe(true);` → `.../home/user/Snippit`

- [ ] **Step 4: Run frontend checks**

Run: `npx vitest run`
Expected: all tests pass (the only test touched, `wizardValidation.test.ts`, doesn't assert on the string's content beyond path validity, so this is a safe rename).

Run: `npm run build`
Expected: `tsc` + vite build succeed with no errors.

- [ ] **Step 5: Commit**

```bash
git add src/App.tsx src/FirstRunWizard.tsx src/StorageSettings.tsx src/SettingsModal.tsx src/theme.css src/wizardValidation.test.ts
git commit -m "chore: rebrand frontend copy to Snippit"
```

---

### Task 4: Rust internal branding (remote identity, tray menu, comments)

**Files:**
- Modify: `src-tauri/src/remote.rs:1,2,10,82,91,285,305,310,311,312,317,319,322`
- Modify: `src-tauri/src/lib.rs:1285,1390,1422,1423,1932,2246`
- Modify (test/temp-file prefix strings only, cosmetic): `src-tauri/tests/sync_integration.rs:22,27,34` (the `ClipSnip`/`clipsnip` literals not already handled in Task 1), `src-tauri/src/library.rs:850,874`, `src-tauri/src/bundle.rs:163,194`, `src-tauri/src/watcher.rs:73`, `src-tauri/src/relocate.rs:215`, `src-tauri/src/setup.rs:121,213`, `src-tauri/src/lib.rs:203,260,492,579,798,851`, `src-tauri/tests/preview_audio_integration.rs:23,60,90,125,126`, `src-tauri/tests/proxy_integration.rs:10,11`, `src-tauri/tests/thumbnail_integration.rs:8,9`, `src-tauri/tests/trim_integration.rs:21,22`, `src-tauri/tests/compress_integration.rs:10,11,12,56,57`, `src-tauri/tests/sequence_integration.rs:24,25,26,27,92,93`, `src-tauri/tests/vaapi_smoke.rs:53,54,83,84,85,124`

- [ ] **Step 1: `remote.rs` — the constant, the folder segment, doc comments, and unit tests**

Change the doc comment:
```rust
//! Typed wrapper around the bundled rclone sidecar. ClipSnip owns a private
//! rclone config holding exactly one remote named `clipsnip`; every command
```
to:
```rust
//! Typed wrapper around the bundled rclone sidecar. Snippit owns a private
//! rclone config holding exactly one remote named `snippit`; every command
```

Change:
```rust
pub const REMOTE_NAME: &str = "clipsnip";
```
to:
```rust
pub const REMOTE_NAME: &str = "snippit";
```

Change the doc comment and body of the folder-path builder:
```rust
/// Builds `clipsnip:<base>/ClipSnip[/<sub>]`. Empty sub returns the ClipSnip
```
to:
```rust
/// Builds `snippit:<base>/Snippit[/<sub>]`. Empty sub returns the Snippit
```
and:
```rust
    out.push_str("ClipSnip");
```
to:
```rust
    out.push_str("Snippit");
```

In the `#[cfg(test)]` module, update every literal string that hardcodes the old remote name or folder segment so the assertions match the new constant/builder output — replace each occurrence of `clipsnip` with `snippit` and `ClipSnip` with `Snippit` in the test bodies (lines 285, 305, 310–312, 317, 319, 322 per the pre-edit grep — re-grep after editing to confirm none remain: `grep -in clipsnip src-tauri/src/remote.rs` should return nothing).

- [ ] **Step 2: `lib.rs` — tray menu label and comments**

Change the user-visible tray menu item:
```rust
                let open_item = MenuItem::with_id(app, "open", "Open ClipSnip", true, None::<&str>)?;
```
to:
```rust
                let open_item = MenuItem::with_id(app, "open", "Open Snippit", true, None::<&str>)?;
```

Update the surrounding comments (lines 1285, 1390, 1422–1423, 1932) that reference "ClipSnip" or the `clipsnip` remote name in prose, replacing with "Snippit"/`snippit` to match Step 1.

- [ ] **Step 3: Batch-rename cosmetic temp-file-prefix literals**

These are internal scratch-file name prefixes used only by tests/temp dirs (e.g. `"clipsnip_test_chunk_{}_{start}_{dur}.wav"`, `"clipsnip_export_{}"`, `"clipsnip_bundle_{}"` etc.) — they carry no functional meaning tied to the app's identity, but since Task 1 already touches these same files for the crate-import rename, replace `clipsnip` with `snippit` in these literals too while you're in the file, for consistency. Run a targeted sweep:

```bash
grep -rl "clipsnip" src-tauri/src src-tauri/tests | xargs sed -i '' 's/clipsnip/snippit/g; s/ClipSnip/Snippit/g'
```

(macOS `sed -i ''` syntax — adjust for Linux/CI if run elsewhere.) This is safe as a blanket substitution in these two directories specifically because Task 1 and Steps 1–2 above have already handled every semantically-load-bearing occurrence by hand, and no other case-variant of the string exists there per the original repo-wide grep.

- [ ] **Step 4: Run the Rust test suite**

Run: `cd src-tauri && cargo test 2>&1 | tail -60`
Expected: all unit and integration tests pass, including `remote.rs`'s unit tests and `sync_integration.rs` (which exercises a real local rclone remote named `snippit` end-to-end).

- [ ] **Step 5: Confirm no stragglers**

Run: `grep -rin clipsnip src-tauri/src src-tauri/tests`
Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src src-tauri/tests
git commit -m "chore: rebrand internal Rust identifiers and tray menu to Snippit"
```

---

### Task 5: npm package identity

**Files:**
- Modify: `package.json:2`
- Modify: `package-lock.json` (top-level `name` field and the `""` package entry's `name` field)

- [ ] **Step 1: Edit `package.json`**

Change:
```json
  "name": "clipsnip",
```
to:
```json
  "name": "snippit",
```

- [ ] **Step 2: Regenerate the lockfile**

Run: `npm install`
Expected: `package-lock.json`'s two `"name": "clipsnip"` occurrences (top-level and the `""`-keyed root package) update to `"snippit"` automatically; no dependency versions should change. Confirm with `git diff package-lock.json` — only `name` fields should differ (plus whatever pre-existing version-bump diff was already present before this plan, if not yet committed).

- [ ] **Step 3: Commit**

```bash
git add package.json package-lock.json
git commit -m "chore: rename npm package clipsnip -> snippit"
```

---

### Task 6: README.md and CLAUDE.md

**Files:**
- Modify: `README.md:2,5,13,24,58,73,74,94,100,122,130`
- Modify: `CLAUDE.md:24`

- [ ] **Step 1: Edit `README.md`**

Replace, line by line:
- L2: `alt="ClipSnip logo"` → `alt="Snippit logo"`
- L5: `<h1 align="center">ClipSnip</h1>` → `<h1 align="center">Snippit</h1>`
- L13: both `mikilimj/ClipSnip` occurrences (workflow link + shields.io badge URL) → `mikilimj/Snippit`
- L24: `**ClipSnip does not record anything.**` → `**Snippit does not record anything.**`
- L58: `github.com/mikilimj/ClipSnip/releases` → `github.com/mikilimj/Snippit/releases`
- L73: `git clone https://github.com/mikilimj/ClipSnip.git` → `.../Snippit.git`
- L74: `cd ClipSnip` → `cd Snippit`
- L94: `./src-tauri/target/release/clipsnip` → `./src-tauri/target/release/snippit`
- L100: `` `clipsnip` (builds from source) and `clipsnip-bin` (prebuilt `.deb` repackaged).`` → `` `snippit` (builds from source) and `snippit-bin` (prebuilt `.deb` repackaged).``
- L122: `github.com/mikilimj/ClipSnip/issues` → `github.com/mikilimj/Snippit/issues`
- L130: `ClipSnip is maintained by` → `Snippit is maintained by`

(Note: the `<img src="logo.png">` file itself is untouched by this task — if the logo image has "ClipSnip" baked into the artwork as a wordmark, that's a design asset that needs to be redrawn separately; this plan only covers text/code.)

- [ ] **Step 2: Edit `CLAUDE.md`**

Change:
```markdown
ClipSnip is a Tauri 2 desktop app: a React/TypeScript frontend (`src/`) in a native
```
to:
```markdown
Snippit is a Tauri 2 desktop app: a React/TypeScript frontend (`src/`) in a native
```

- [ ] **Step 3: Verify**

Run: `grep -in clipsnip README.md CLAUDE.md`
Expected: no output.

- [ ] **Step 4: Commit**

```bash
git add README.md CLAUDE.md
git commit -m "docs: rebrand README and CLAUDE.md to Snippit"
```

---

### Task 7: AUR packaging rename

**Files:**
- Rename dir: `packaging/aur/clipsnip/` → `packaging/aur/snippit/`
- Rename dir: `packaging/aur/clipsnip-bin/` → `packaging/aur/snippit-bin/`
- Rename file: `packaging/aur/snippit/clipsnip.desktop` → `packaging/aur/snippit/snippit.desktop`
- Modify: `packaging/aur/snippit/PKGBUILD`, `packaging/aur/snippit/snippit.desktop`, `packaging/aur/snippit-bin/PKGBUILD`

- [ ] **Step 1: Rename the directories and desktop file with git**

```bash
git mv packaging/aur/clipsnip packaging/aur/snippit
git mv packaging/aur/clipsnip-bin packaging/aur/snippit-bin
git mv packaging/aur/snippit/clipsnip.desktop packaging/aur/snippit/snippit.desktop
```

- [ ] **Step 2: Edit `packaging/aur/snippit/snippit.desktop`**

Replace all four fields:
```ini
Name=ClipSnip
```
→ `Name=Snippit`
```ini
Exec=clipsnip
```
→ `Exec=snippit`
```ini
Icon=clipsnip
```
→ `Icon=snippit`
```ini
StartupWMClass=ClipSnip
```
→ `StartupWMClass=Snippit`

- [ ] **Step 3: Edit `packaging/aur/snippit/PKGBUILD`**

Replace:
- `pkgname=clipsnip` → `pkgname=snippit`
- `url="https://github.com/mikilimj/ClipSnip"` → `url="https://github.com/mikilimj/Snippit"`
- `"clipsnip.desktop"` (source array entry) → `"snippit.desktop"`
- both `cd "ClipSnip-$pkgver"` occurrences → `cd "Snippit-$pkgver"`
- the comment `# app resolves sidecars next to /usr/bin/clipsnip, ...` → `/usr/bin/snippit`
- `install -Dm755 src-tauri/target/release/clipsnip "$pkgdir/usr/bin/clipsnip"` → both `clipsnip` → `snippit`
- `install -Dm644 "$srcdir/clipsnip.desktop" ...` → `"$srcdir/snippit.desktop"`, and the destination `.../applications/clipsnip.desktop` → `.../applications/snippit.desktop`
- all four icon-install lines' `.../apps/clipsnip.png` → `.../apps/snippit.png`

- [ ] **Step 4: Edit `packaging/aur/snippit-bin/PKGBUILD`**

Replace:
- `pkgname=clipsnip-bin` → `pkgname=snippit-bin`
- `url="https://github.com/mikilimj/ClipSnip"` → `url="https://github.com/mikilimj/Snippit"`
- `provides=('clipsnip')` → `provides=('snippit')`
- `conflicts=('clipsnip')` → `conflicts=('snippit')`
- `source=("$url/releases/download/v$pkgver/ClipSnip_${pkgver}_amd64.deb")` → `.../Snippit_${pkgver}_amd64.deb`
- the comment `# them next to /usr/bin/clipsnip, ...` → `/usr/bin/snippit`

- [ ] **Step 5: Verify no stragglers**

Run: `grep -rin clipsnip packaging/`
Expected: no output.

- [ ] **Step 6: Commit**

```bash
git add packaging/
git commit -m "chore: rename AUR packages clipsnip/clipsnip-bin -> snippit/snippit-bin"
```

---

### Task 8: CI workflow update

**Files:**
- Modify: `.github/workflows/build.yml:157,162,163,186,190,193,195,202,205,206`

- [ ] **Step 1: Update the manual-run artifact name**

Change:
```yaml
          name: clipsnip-${{ matrix.target }}
```
to:
```yaml
          name: snippit-${{ matrix.target }}
```

- [ ] **Step 2: Update the AUR job's comment, paths, and pkgnames**

Change the comment above `publish-aur`:
```yaml
  # Pushes version bumps for both AUR packages (clipsnip builds the release
  # tag from source, clipsnip-bin repacks the .deb uploaded by the build job).
```
to:
```yaml
  # Pushes version bumps for both AUR packages (snippit builds the release
  # tag from source, snippit-bin repacks the .deb uploaded by the build job).
```

Change the PKGBUILD path list:
```yaml
          sed -i "s/^pkgver=.*/pkgver=$version/; s/^pkgrel=.*/pkgrel=1/" \
            packaging/aur/clipsnip/PKGBUILD packaging/aur/clipsnip-bin/PKGBUILD
          sed -i "s|^source=(.*|source=(\"\$url/releases/download/v\$pkgver/$deb_asset\")|" \
            packaging/aur/clipsnip-bin/PKGBUILD
```
to:
```yaml
          sed -i "s/^pkgver=.*/pkgver=$version/; s/^pkgrel=.*/pkgrel=1/" \
            packaging/aur/snippit/PKGBUILD packaging/aur/snippit-bin/PKGBUILD
          sed -i "s|^source=(.*|source=(\"\$url/releases/download/v\$pkgver/$deb_asset\")|" \
            packaging/aur/snippit-bin/PKGBUILD
```

Change the two publish steps:
```yaml
      - name: Publish clipsnip to AUR
        uses: KSXGitHub/github-actions-deploy-aur@v4.1.3
        with:
          pkgname: clipsnip
          pkgbuild: packaging/aur/clipsnip/PKGBUILD
          assets: packaging/aur/clipsnip/clipsnip.desktop
```
to:
```yaml
      - name: Publish snippit to AUR
        uses: KSXGitHub/github-actions-deploy-aur@v4.1.3
        with:
          pkgname: snippit
          pkgbuild: packaging/aur/snippit/PKGBUILD
          assets: packaging/aur/snippit/snippit.desktop
```
and:
```yaml
      - name: Publish clipsnip-bin to AUR
        uses: KSXGitHub/github-actions-deploy-aur@v4.1.3
        with:
          pkgname: clipsnip-bin
          pkgbuild: packaging/aur/clipsnip-bin/PKGBUILD
```
to:
```yaml
      - name: Publish snippit-bin to AUR
        uses: KSXGitHub/github-actions-deploy-aur@v4.1.3
        with:
          pkgname: snippit-bin
          pkgbuild: packaging/aur/snippit-bin/PKGBUILD
```

- [ ] **Step 3: Verify YAML is well-formed**

Run: `python3 -c "import yaml, sys; yaml.safe_load(open('.github/workflows/build.yml'))" && echo OK`
Expected: `OK` (this only checks syntax, not semantics — GitHub will validate the rest on next push).

- [ ] **Step 4: Verify no stragglers**

Run: `grep -in clipsnip .github/workflows/build.yml`
Expected: no output.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/build.yml
git commit -m "ci: update workflow for snippit rename"
```

---

### Task 9: Repo-wide verification sweep

**Files:** none (verification only)

- [ ] **Step 1: Full repo-wide grep, excluding intentionally-untouched paths**

Run:
```bash
grep -rIn -i clipsnip . \
  --exclude-dir=node_modules --exclude-dir=target --exclude-dir=dist --exclude-dir=.git \
  --exclude-dir=docs \
  | grep -v Cargo.lock
```
Expected: no output. (The `docs` exclude covers the historical `docs/superpowers/plans/*.md` and `specs/*.md` per Global Constraints; `Cargo.lock` is excluded because third-party crates could coincidentally contain the substring — inspect any hits manually rather than assuming they're the old package.)

If anything unexpected shows up, fix it now before moving on — this step exists specifically to catch anything the earlier tasks' file lists missed.

- [ ] **Step 2: Full build + full test pass**

Run: `npm run build`
Expected: succeeds.

Run: `npx vitest run`
Expected: all pass.

Run: `cd src-tauri && cargo test`
Expected: all pass (this re-runs the real ffmpeg/ffprobe integration tests too — slow, but this is the last gate before calling the rename done).

- [ ] **Step 3: Manual smoke check**

Run: `npm run tauri dev`, click through: window title bar reads "Snippit", the in-app header/brand name reads "Snippit", first-run wizard (if triggered, e.g. by clearing the app's config dir) suggests a `.../Snippit` library folder, Settings modal's export-bundle default filename is `snippit-library.zip`. Close the window when done.

---

### Task 10: GitHub repository rename (STOP — confirm with the user first)

**This task is a hard-to-reverse, account-visible action. Do not run Step 1 without the user's explicit go-ahead in the current conversation, even though they already agreed in principle during planning — confirm the specific moment.**

**Files:** none (remote operation) plus local git config

- [ ] **Step 1: Confirm, then rename**

Ask: "Ready to rename github.com/mikilimj/ClipSnip → mikilimj/Snippit now? GitHub will auto-redirect the old URL for a while, but anything hardcoding the old URL outside this repo (bookmarks, other tools) will need updating separately."

Once confirmed, run:
```bash
gh repo rename Snippit --repo mikilimj/ClipSnip
```

- [ ] **Step 2: Update the local git remote**

```bash
git remote set-url origin git@github.com:mikilimj/Snippit.git
git remote -v
```
Expected: both `fetch` and `push` show `mikilimj/Snippit.git`.

- [ ] **Step 3: Push the accumulated commits**

Push the commits from Tasks 1–9 (and this task, if any files changed) to the renamed remote, and confirm the GitHub Actions build badge in the freshly-committed README resolves against the new repo name.

---

### Task 11 (optional, manual, do last): Rename the local working directory

**Not part of the automated plan execution** — this changes the filesystem path this whole repo lives at (currently `/Users/miloszjablonski/Documents/code/ClipSnip`), which would pull the rug out from under any active session/terminal with that path as its cwd. Do this yourself, after everything above is committed and pushed, e.g.:

```bash
mv /Users/miloszjablonski/Documents/code/ClipSnip /Users/miloszjablonski/Documents/code/Snippit
```

Then `cd` any open terminals/editors into the new path.
