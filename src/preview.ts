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
