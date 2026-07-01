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
