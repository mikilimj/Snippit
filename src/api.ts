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
