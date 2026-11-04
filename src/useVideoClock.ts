import { useEffect, useState, type RefObject } from "react";

/**
 * Frame-fresh position of the <video>, kept local to whichever component
 * displays it so playback never re-renders the app. Reading the ref every frame
 * also survives the <video> being swapped for a proxy or video-only remux.
 * Sub-frame changes bail out via functional setState, and the loop is parked
 * while the window is backgrounded.
 */
export function useVideoClock(videoRef: RefObject<HTMLVideoElement | null>): number {
  const [currentTime, setCurrentTime] = useState(0);

  useEffect(() => {
    let raf = 0;
    const start = () => {
      raf = requestAnimationFrame(function tick() {
        const t = videoRef.current?.currentTime ?? 0;
        setCurrentTime(prev => (Math.abs(t - prev) < 0.03 ? prev : t));
        raf = requestAnimationFrame(tick);
      });
    };
    const stop = () => cancelAnimationFrame(raf);
    const onVisibility = () => (document.hidden ? stop() : start());
    document.addEventListener("visibilitychange", onVisibility);
    if (!document.hidden) start();
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [videoRef]);

  return currentTime;
}
