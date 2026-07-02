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

export function exportTrim(
  source: string,
  inSecs: number,
  outSecs: number,
  output: string,
  trackConfigs: TrackConfig[],
): Promise<string> {
  return invoke<string>("export_trim", { source, inSecs, outSecs, output, trackConfigs });
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
