import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

interface Props {
  duration: number;
  videoRef: RefObject<HTMLVideoElement | null>;
  inPoint: number | null;
  outPoint: number | null;
  onSeek: (time: number) => void;
}

function fmt(secs: number) {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  const d = Math.floor((secs % 1) * 10);
  return `${m}:${s.toString().padStart(2, "0")}.${d}`;
}

export function Timeline({ duration, videoRef, inPoint, outPoint, onSeek }: Props) {
  const barRef = useRef<HTMLDivElement>(null);

  // The playhead follows the video clock via rAF and stays local to this small
  // component — the app must not re-render on every video timeupdate. Reading
  // the ref each frame also survives the <video> element being swapped out for
  // a proxy. Sub-frame changes bail out via functional setState.
  const [currentTime, setCurrentTime] = useState(0);
  useEffect(() => {
    let raf = requestAnimationFrame(function tick() {
      const t = videoRef.current?.currentTime ?? 0;
      setCurrentTime(prev => (Math.abs(t - prev) < 0.03 ? prev : t));
      raf = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(raf);
  }, [videoRef]);

  const seekAt = useCallback((clientX: number) => {
    const bar = barRef.current;
    if (!bar || duration <= 0) return;
    const { left, width } = bar.getBoundingClientRect();
    onSeek(Math.max(0, Math.min(1, (clientX - left) / width)) * duration);
  }, [duration, onSeek]);

  const toN = (t: number) => (duration > 0 ? (t / duration) * 100 : 0);
  const pct = (t: number) => `${toN(t)}%`;

  const inPct = inPoint !== null ? toN(inPoint) : null;
  const outPct = outPoint !== null ? toN(outPoint) : null;

  return (
    <div
      ref={barRef}
      className="tl-bar"
      onPointerDown={(e) => {
        barRef.current?.setPointerCapture(e.pointerId);
        seekAt(e.clientX);
      }}
      onPointerMove={(e) => {
        if (e.buttons === 1) seekAt(e.clientX);
      }}
    >
      <div className="tl-track" />

      {inPct !== null && outPct !== null && (
        <div
          className="tl-range"
          style={{ left: `${inPct}%`, width: `${outPct - inPct}%` }}
        />
      )}

      {inPct !== null && (
        <div className="tl-handle tl-in" style={{ left: `${inPct}%` }}>
          <span>I</span>
        </div>
      )}
      {outPct !== null && (
        <div className="tl-handle tl-out" style={{ left: `${outPct}%` }}>
          <span>O</span>
        </div>
      )}

      <div className="tl-cursor" style={{ left: pct(currentTime) }} />

      <span className="tl-time tl-time-left">{fmt(currentTime)}</span>
      <span className="tl-time tl-time-right">{fmt(duration)}</span>
    </div>
  );
}
