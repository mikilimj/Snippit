//! Preview-source rules for the native-first / proxy-fallback strategy (spec §4.4).

/**
 * The file the `<video>` element should load: the generated proxy once we have
 * one, otherwise the untouched original. Export never uses this — it always
 * reads the original path.
 */
export function previewPath(originalPath: string, proxyPath: string | null): string {
  return proxyPath ?? originalPath;
}

/**
 * After a playback error, whether to attempt building a proxy. Only when we were
 * still playing the original (no proxy yet) — re-proxying a failed proxy would
 * loop forever.
 */
export function shouldTryProxy(proxyPath: string | null): boolean {
  return proxyPath === null;
}

export interface AudioSyncCommand {
  /** Snap the audio element's currentTime to the video clock. */
  seek: boolean;
  /** Resume a track that stalled (paused itself) while the video kept playing. */
  play: boolean;
}

/**
 * Periodic keep-in-sync check for one preview audio track against the video
 * clock. Audio elements run on their own pipelines and can stall on a buffer
 * underrun without ever resuming, or drift relative to the video; both must be
 * corrected, but tracks within the tolerance are left untouched — seeking an
 * in-sync track causes an audible stutter.
 */
export function audioSyncCommand(
  videoTime: number,
  videoPaused: boolean,
  audioTime: number,
  audioPaused: boolean,
  driftTolerance = 0.35
): AudioSyncCommand {
  if (videoPaused) return { seek: false, play: false };
  if (audioPaused) return { seek: true, play: true };
  return { seek: Math.abs(videoTime - audioTime) > driftTolerance, play: false };
}
