import { useRef, type PointerEvent as ReactPointerEvent, type RefObject } from "react";
import { formatTimecode } from "./timecode";
import { useVideoClock } from "./useVideoClock";

interface Props {
  duration: number;
  videoRef: RefObject<HTMLVideoElement | null>;
  inPoint: number | null;
  outPoint: number | null;
  /** Raw pointer position as a time; the caller clamps it against the other handle. */
  onChangeIn: (time: number) => void;
  onChangeOut: (time: number) => void;
  onSeek: (time: number) => void;
}

/**
 * The clip's timetable, sitting directly under the player: the bar itself is
 * the scrub area for the playhead, and the two handles on it are the trim
 * points. Dragging a handle moves the playhead to that exact time as you go, so
 * the picture follows the cut in real time. There are no separate in/out
 * buttons, and no second scrub bar anywhere else.
 */
export function Timeline({
  duration,
  videoRef,
  inPoint,
  outPoint,
  onChangeIn,
  onChangeOut,
  onSeek,
}: Props) {
  const barRef = useRef<HTMLDivElement>(null);
  // Which gesture is in flight: scrubbing the playhead, or dragging a handle.
  const dragRef = useRef<"seek" | "in" | "out" | null>(null);
  const currentTime = useVideoClock(videoRef);

  const inSecs = inPoint ?? 0;
  const outSecs = outPoint ?? duration;
  const toPct = (t: number) => (duration > 0 ? (t / duration) * 100 : 0);

  // Pointer x → time on the clip. Pointer capture keeps the events coming from
  // the grabbed element even when the pointer runs off the bar mid-drag.
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
      // Handles sit on the scrub surface — don't let this also start a scrub.
      e.stopPropagation();
      dragRef.current = which;
      // Trimming shouldn't fight playback: park the video so the drag is a scrub.
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

  function onBarDown(e: ReactPointerEvent<HTMLDivElement>) {
    if (duration <= 0) return;
    dragRef.current = "seek";
    e.currentTarget.setPointerCapture(e.pointerId);
    onSeek(timeAt(e.clientX));
  }

  function onBarMove(e: ReactPointerEvent<HTMLDivElement>) {
    if (dragRef.current !== "seek") return;
    onSeek(timeAt(e.clientX));
  }

  function onBarUp(e: ReactPointerEvent<HTMLDivElement>) {
    if (dragRef.current !== "seek") return;
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
    <div
      ref={barRef}
      className="tl-bar"
      role="slider"
      tabIndex={0}
      aria-label="Playback position and trim points"
      aria-valuemin={0}
      aria-valuemax={Math.round(duration)}
      aria-valuenow={Number(currentTime.toFixed(1))}
      onPointerDown={onBarDown}
      onPointerMove={onBarMove}
      onPointerUp={onBarUp}
      onPointerCancel={onBarUp}
      onKeyDown={e => {
        // Arrow keys nudge the playhead by a second, Home/End jump to the ends.
        if (duration <= 0) return;
        const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        if (step !== 0) {
          e.preventDefault();
          onSeek(Math.max(0, Math.min(duration, currentTime + step)));
        } else if (e.key === "Home" || e.key === "End") {
          e.preventDefault();
          onSeek(e.key === "Home" ? 0 : duration);
        }
      }}
    >
      <div className="tl-track" />

      {/* How much of the clip has been played, then the kept in→out range. */}
      <div className="tl-played" style={{ width: `${toPct(currentTime)}%` }} />
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
      <div className="tl-thumb" style={{ left: `${toPct(currentTime)}%` }} />

      <span className="tl-time tl-time-left">{formatTimecode(inSecs)}</span>
      <span className="tl-time tl-time-right">{formatTimecode(outSecs)}</span>
    </div>
  );
}
