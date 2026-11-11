//! Pure scheduling math for the chunked preview-audio engine. Track audio is
//! decoded by ffmpeg into short PCM chunks and scheduled as AudioBufferSourceNodes
//! on one shared AudioContext (never per-track <audio> elements — each media
//! element registers its own audio stream with the OS mixer, and the whole point
//! is that the app shows up as a single stream). These helpers map between the
//! media clock (video.currentTime) and the AudioContext clock.
//!
//! Chunk length is the engine's memory unit: every fetch becomes a float32
//! AudioBuffer, and the buffers are what a playing clip actually costs on top of
//! the video decode. Keep the two constants below small and the per-track
//! footprint stays in the single-digit MB range.

/**
 * Length of one decoded PCM chunk. Measured on a real track (48kHz stereo):
 * a 10s chunk is 1.83MB as the s16 WAV that crosses IPC and ~3.7MB once the
 * WebView decodes it to float32.
 */
export const CHUNK_SECONDS = 10;

/** Schedule the next chunk once the playhead is this close to its start. */
export const SCHEDULE_LOOKAHEAD_SECONDS = 5;

/**
 * Decoded chunks held per track: the playing chunk plus its lookahead
 * neighbour. `chunkWindow` never asks for more than two at a time, and each
 * track is cached independently, so this bounds memory PER TRACK (2 × ~3.7MB at
 * 10s/48kHz) — not for the clip as a whole. Tracks are only ever fetched while
 * they are audible, so a muted track costs nothing.
 */
export const CACHED_CHUNKS_PER_TRACK = 2;

/**
 * Engine-vs-video drift beyond which everything is torn down and re-anchored.
 * Matches the old per-element sync tolerance: resyncing inside the tolerance
 * causes an audible stutter, so small drift is left alone.
 */
export const DRIFT_TOLERANCE_SECONDS = 0.35;

/** Index of the chunk that contains media time `t`. */
export function chunkIndexAt(t: number, chunkDur = CHUNK_SECONDS): number {
  return Math.floor(Math.max(0, t) / chunkDur);
}

/**
 * The chunk indices that should be fetched and scheduled for playhead `t`:
 * the current chunk, plus the next one once the playhead is within the
 * lookahead of the boundary. Indices past the end of the media are dropped.
 */
export function chunkWindow(
  t: number,
  totalDur: number,
  chunkDur = CHUNK_SECONDS,
  lookahead = SCHEDULE_LOOKAHEAD_SECONDS
): number[] {
  if (totalDur <= 0) return [];
  const last = chunkIndexAt(Math.max(0, totalDur - 1e-6), chunkDur);
  const cur = Math.min(chunkIndexAt(t, chunkDur), last);
  const window = [cur];
  if (cur < last && (cur + 1) * chunkDur - Math.max(0, t) <= lookahead) {
    window.push(cur + 1);
  }
  return window;
}

export interface ChunkSchedule {
  /** AudioContext time at which to start the source (never in the past). */
  when: number;
  /** Seconds into the chunk's buffer to start from. */
  offset: number;
  /** Seconds of the buffer to play. */
  duration: number;
}

/**
 * Where on the AudioContext timeline chunk `idx` belongs, given the anchor pair
 * (a media time pinned to a context time when playback last (re)started).
 * Returns null when the chunk lies entirely in the past.
 */
export function chunkSchedule(
  idx: number,
  anchorMediaTime: number,
  anchorCtxTime: number,
  ctxNow: number,
  totalDur: number,
  chunkDur = CHUNK_SECONDS
): ChunkSchedule | null {
  const chunkStart = idx * chunkDur;
  const chunkLen = Math.min(chunkDur, totalDur - chunkStart);
  if (chunkLen <= 0) return null;
  const startAt = anchorCtxTime + (chunkStart - anchorMediaTime);
  if (startAt >= ctxNow) {
    return { when: startAt, offset: 0, duration: chunkLen };
  }
  const offset = ctxNow - startAt;
  if (offset >= chunkLen) return null; // already fully in the past
  return { when: ctxNow, offset, duration: chunkLen - offset };
}

/** The engine's current media-clock position implied by the anchor pair. */
export function engineMediaTime(
  ctxNow: number,
  anchorMediaTime: number,
  anchorCtxTime: number
): number {
  return anchorMediaTime + (ctxNow - anchorCtxTime);
}

/** Whether the engine has drifted far enough from the video clock to re-anchor. */
export function needsResync(
  videoTime: number,
  engineTime: number,
  tolerance = DRIFT_TOLERANCE_SECONDS
): boolean {
  return Math.abs(videoTime - engineTime) > tolerance;
}
