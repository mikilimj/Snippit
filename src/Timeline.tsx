import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import { formatTimecode } from "./timecode";

interface Props {
  duration: number;
  videoRef: RefObject<HTMLVideoElement | null>;
  inPoint: number | null;
  outPoint: number | null;
  /** Raw pointer position as a time; the caller clamps it against the other handle. */
  onChangeIn: (time: number) => void;
  onChangeOut: (time: number) => void;
}

/**
 * Dual-handle trim slider. It starts out spanning the whole clip (a missing
 * point means the corresponding clip edge). Dragging either handle moves the
 * playhead to that exact time as you go, so the picture follows the cut in real
 * time — there are no separate "set in/out" buttons.
 */
export function Timeline({ duration, videoRef, inPoint, outPoint, onChangeIn, onChangeOut }: Props) {
  const barRef = useRef<HTMLDivElement>(null);
  const dragRef = useRef<"in" | "out" | null>(null);

  // The playhead follows the video clock via rAF and stays local to this small
  // component — the app must not re-render on every video timeupdate. Reading
  // the ref each frame also survives the <video> element being swapped out for
  // a proxy. Sub-frame changes bail out via functional setState.
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
    // Hidden tabs/minimized windows don't need a live playhead; stopping the
    // loop entirely (rather than relying on browser rAF throttling) avoids
    // needless wake-ups while backgrounded.
    const onVisibility = () => (document.hidden ? stop() : start());
    document.addEventListener("visibilitychange", onVisibility);
    if (!document.hidden) start();
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      stop();
    };
  }, [videoRef]);

  const inSecs = inPoint ?? 0;
  const outSecs = outPoint ?? duration;
  const toPct = (t: number) => (duration > 0 ? (t / duration) * 100 : 0);

  // Pointer x → time on the clip. Pointer capture keeps the events coming from
  // the handle even when the pointer runs off the bar mid-drag.
  function timeAt(clientX: number): number {
    const bar = barRef.current;
    if (!bar || duration <= 0) return 0;
    const { left, width } = bar.getBoundingClientRect();
    if (width <= 0) return 0;
    return Math.max(0, Math.min(1, (clientX - left) / width)) * duration;
  }

  function emit(which: "in" | "out", clientX: number) {
    const t = timeAt(clientX);
    if (which === "in") onChangeIn(t);
    else onChangeOut(t);
  }

  function onHandleDown(which: "in" | "out") {
    return (e: ReactPointerEvent<HTMLDivElement>) => {
      if (duration <= 0) return;
      dragRef.current = which;
      // Scrubbing a cut point shouldn't fight audio playback: park the video so
      // a handle drag stays a plain scrub.
      videoRef.current?.pause();
      e.currentTarget.setPointerCapture(e.pointerId);
      emit(which, e.clientX);
    };
  }

  function onHandleMove(which: "in" | "out") {
    return (e: ReactPointerEvent<HTMLDivElement>) => {
      if (dragRef.current !== which) return;
      emit(which, e.clientX);
    };
  }

  function onHandleUp(e: ReactPointerEvent<HTMLDivElement>) {
    dragRef.current = null;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
  }

  const handleProps = (which: "in" | "out", t: number) => ({
    className: `tl-handle tl-${which}`,
    style: { left: `${toPct(t)}%` },
    role: "slider",
    "aria-label": which === "in" ? "Trim start" : "Trim end",
    "aria-valuemin": 0,
    "aria-valuemax": Math.round(duration),
    "aria-valuenow": Number(t.toFixed(1)),
    onPointerDown: onHandleDown(which),
    onPointerMove: onHandleMove(which),
    onPointerUp: onHandleUp,
    onPointerCancel: onHandleUp,
  });

  return (
    <div ref={barRef} className="tl-bar">
      <div className="tl-track" />

      <div
        className="tl-range"
        style={{ left: `${toPct(inSecs)}%`, width: `${toPct(outSecs) - toPct(inSecs)}%` }}
      />

      <div {...handleProps("in", inSecs)}>
        <div className="tl-handle-line" />
        <span>I</span>
      </div>

      <div {...handleProps("out", outSecs)}>
        <div className="tl-handle-line" />
        <span>O</span>
      </div>

      <div className="tl-cursor" style={{ left: `${toPct(currentTime)}%` }} />

      <span className="tl-time tl-time-left">{formatTimecode(inSecs)}</span>
      <span className="tl-time tl-time-right">{formatTimecode(outSecs)}</span>
    </div>
  );
}
