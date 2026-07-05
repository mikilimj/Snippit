//! Preview-source rules for the native-first / proxy-fallback strategy (spec §4.4),
//! extended with the video-only remux that keeps the <video> element off the OS
//! audio mixer once the chunked engine (audioEngine.ts) owns playback.

/**
 * The file the `<video>` element should load, in order of preference: the
 * video-only remux of the active source once it exists (its pipeline carries no
 * audio, so the element registers no OS audio stream), else the generated proxy,
 * else the untouched original. Export never uses this — it always reads the
 * original path.
 */
export function previewPath(
  originalPath: string,
  proxyPath: string | null,
  videoOnlyPath: string | null = null
): string {
  return videoOnlyPath ?? proxyPath ?? originalPath;
}

/**
 * After a playback error, whether to attempt building a proxy. Only when we were
 * still playing the original (no proxy yet) — re-proxying a failed proxy would
 * loop forever.
 */
export function shouldTryProxy(proxyPath: string | null): boolean {
  return proxyPath === null;
}
