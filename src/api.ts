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

export function probeClip(path: string): Promise<ClipMeta> {
  return invoke<ClipMeta>("probe_clip", { path });
}

export function exportTrim(
  source: string,
  inSecs: number,
  outSecs: number,
  output: string
): Promise<string> {
  return invoke<string>("export_trim", { source, inSecs, outSecs, output });
}
