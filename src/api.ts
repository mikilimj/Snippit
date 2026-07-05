import { invoke } from "@tauri-apps/api/core";

export interface AudioTrack {
  index: number;
  codec: string;
  channels: number;
  language: string | null;
}

export interface ClipMeta {
  durationSecs: number;
  width: number;
  height: number;
  container: string;
  videoCodec: string;
  audioTracks: AudioTrack[];
}

export interface TrackConfig {
  streamIndex: number;
  enabled: boolean;
  volume: number; // 0.0–2.0; 1.0 = original level
}

export function probeClip(path: string): Promise<ClipMeta> {
  return invoke<ClipMeta>("probe_clip", { path });
}

/**
 * Ensure a 720p H.264 preview proxy exists for a clip the WebView can't play
 * natively, returning its file path (spec §4.4). Cached by source + mtime; the
 * original is always used for export.
 */
export function ensureProxy(path: string): Promise<string> {
  return invoke<string>("ensure_proxy", { path });
}

/** A persisted library row (SQLite index). Mirrors Rust `library::ClipRow`. */
export interface ClipRow {
  id: number;
  path: string;
  displayName: string;
  durationSecs: number;
  width: number;
  height: number;
  container: string;
  videoCodec: string;
  audioTrackCount: number;
  thumbnailPath: string | null;
  favorite: boolean;
  missing: boolean;
  addedAt: number;
}

/** Insert (or refresh the metadata of) a clip in the persistent library index. */
export function addToLibrary(path: string, meta: ClipMeta): Promise<ClipRow> {
  return invoke<ClipRow>("add_to_library", { path, meta });
}

/** Load all persisted library clips, newest first. */
export function listLibrary(): Promise<ClipRow[]> {
  return invoke<ClipRow[]>("list_library");
}

/** Remove a clip from the library index only (never deletes the source file). */
export function removeFromLibrary(id: number): Promise<void> {
  return invoke<void>("remove_from_library", { id });
}

/**
 * Re-scan the configured watched folders for clips added since launch. Newly
 * indexed clips are delivered asynchronously via `clip-added` events.
 */
export function rescanLibrary(): Promise<void> {
  return invoke<void>("rescan_library");
}

/** A virtual folder (user-named grouping of clips). Mirrors Rust `library::Folder`. */
export interface Folder {
  id: number;
  name: string;
}

/** Set (or clear) a clip's favorite flag. */
export function setClipFavorite(id: number, favorite: boolean): Promise<void> {
  return invoke<void>("set_clip_favorite", { id, favorite });
}

/** Rename a clip's library display name (the file on disk is never renamed). */
export function renameLibraryClip(id: number, name: string): Promise<void> {
  return invoke<void>("rename_library_clip", { id, name });
}

/** Create a new virtual folder. Rejects if the name is already taken. */
export function createFolder(name: string): Promise<Folder> {
  return invoke<Folder>("create_folder", { name });
}

/** List all virtual folders, ordered alphabetically. */
export function listFolders(): Promise<Folder[]> {
  return invoke<Folder[]>("list_folders");
}

/** Delete a virtual folder. Its clips survive and revert to Unsorted. */
export function deleteFolder(folderId: number): Promise<void> {
  return invoke<void>("delete_folder", { folderId });
}

/** Add a clip to a folder (idempotent; a clip may live in many folders). */
export function addClipToFolder(clipId: number, folderId: number): Promise<void> {
  return invoke<void>("add_clip_to_folder", { clipId, folderId });
}

/** Detach a clip from a single folder, leaving other memberships intact. */
export function removeClipFromFolder(clipId: number, folderId: number): Promise<void> {
  return invoke<void>("remove_clip_from_folder", { clipId, folderId });
}

/** List the clips that belong to a given folder, newest first. */
export function listClipsInFolder(folderId: number): Promise<ClipRow[]> {
  return invoke<ClipRow[]>("list_clips_in_folder", { folderId });
}

/** List clips that are not in any folder — the "Unsorted" view. */
export function listUnsortedLibrary(): Promise<ClipRow[]> {
  return invoke<ClipRow[]>("list_unsorted_library");
}

/** Extract each requested audio stream to a temp M4A file and return their paths. */
export function prepareAudioTracks(path: string, streamIndices: number[]): Promise<string[]> {
  return invoke<string[]>("prepare_audio_tracks", { path, streamIndices });
}

/**
 * Decode `dur` seconds of an extracted track starting at `start` to WAV bytes.
 * Feeds the chunked preview-audio engine (single OS audio stream — see
 * audioEngine.ts).
 */
export function audioChunk(path: string, start: number, dur: number): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("audio_chunk", { path, start, dur });
}

/**
 * Remux the current preview source into a cached video-only copy. Loaded into
 * the <video> once Web Audio owns playback so the element's pipeline stops
 * registering its own (muted) audio stream with the OS mixer.
 */
export function prepareVideoOnly(path: string): Promise<string> {
  return invoke<string>("prepare_video_only", { path });
}

/** Fast = stream-copy (near-instant); Compatible = H.264/AAC re-encode (universal). */
export type ExportMode = "fast" | "compatible";

/** Progress streamed from the Rust export engine (spec §4.5). */
export interface ExportProgress {
  fraction: number; // 0.0–1.0 of the whole job
  phase: string; // "Exporting" | "Analyzing" | "Compressing"
}

export function exportTrim(
  source: string,
  inSecs: number,
  outSecs: number,
  output: string,
  trackConfigs: TrackConfig[],
  mode: ExportMode,
): Promise<string> {
  return invoke<string>("export_trim", { source, inSecs, outSecs, output, trackConfigs, mode });
}

/** Cancel the in-flight export (kills ffmpeg); the export promise then rejects with a cancel message. */
export function cancelExport(): Promise<void> {
  return invoke<void>("cancel_export");
}

/** Two-pass H.264/AAC export compressed to a target file size in MB. */
export function exportCompressed(
  source: string,
  inSecs: number,
  outSecs: number,
  output: string,
  trackConfigs: TrackConfig[],
  targetMb: number,
): Promise<string> {
  return invoke<string>("export_compressed", {
    source,
    inSecs,
    outSecs,
    output,
    trackConfigs,
    targetMb,
  });
}

/** One trimmed clip in a stitched sequence. Mirrors Rust `export::Segment`. */
export interface Segment {
  input: string;
  inSecs: number;
  outSecs: number;
  /** Per-track audio config snapshotted when the clip was added to the sequence. */
  tracks: TrackConfig[];
}

/** A music bed mixed over a sequence. Mirrors Rust `export::MusicOverlay`. */
export interface MusicOverlay {
  path: string;
  volume: number; // 0.0–2.0
  loopToLength: boolean;
}

/**
 * Stitch trimmed segments (+ optional music) into one H.264/AAC MP4 (spec §4.3).
 * `targetMb` compresses the whole sequence to a file size, like single-clip export.
 */
export function exportSequence(
  segments: Segment[],
  music: MusicOverlay | null,
  output: string,
  targetMb: number | null,
): Promise<string> {
  return invoke<string>("export_sequence", { segments, music, output, targetMb });
}

/** One export encode device (CPU or a probed hardware encoder). */
export interface ExportDeviceInfo {
  id: string;
  label: string;
  available: boolean;
}

/** CPU plus every hardware encoder candidate on this OS, with probed availability. */
export function listExportDevices(): Promise<ExportDeviceInfo[]> {
  return invoke<ExportDeviceInfo[]>("list_export_devices");
}

/** The device id currently used for export re-encodes ("cpu" when unset). */
export function getExportDevice(): Promise<string> {
  return invoke<string>("get_export_device");
}

/** Persist the export device choice. */
export function setExportDevice(device: string): Promise<void> {
  return invoke<void>("set_export_device", { device });
}

/** The watched-mode source folder, or null when nothing is relocatable (§4.7). */
export function getWatchedFolder(): Promise<string | null> {
  return invoke<string | null>("get_watched_folder");
}

/** Move the watched source folder to `dest`; progress arrives via `relocate-progress`. */
export function relocateWatchedFolder(dest: string): Promise<string> {
  return invoke<string>("relocate_watched_folder", { dest });
}

/** Cancel the running folder relocation (safe at any point). */
export function cancelRelocate(): Promise<void> {
  return invoke<void>("cancel_relocate");
}

/** Summary of a library import. Mirrors Rust `library::ImportStats`. */
export interface ImportStats {
  added: number;
  missing: number;
  folders: number;
}

/** Export the library to a portable zip bundle; progress via `bundle-progress` (§4.6). */
export function exportLibrary(dest: string, includeFiles: boolean): Promise<string> {
  return invoke<string>("export_library", { dest, includeFiles });
}

/** Import a bundle, re-rooting clip paths under `newRoot`; merge or replace (§4.6). */
export function importLibrary(
  bundlePath: string,
  newRoot: string,
  replace: boolean,
): Promise<ImportStats> {
  return invoke<ImportStats>("import_library", { bundlePath, newRoot, replace });
}

/** Cancel the running library export/import. */
export function cancelBundle(): Promise<void> {
  return invoke<void>("cancel_bundle");
}

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
