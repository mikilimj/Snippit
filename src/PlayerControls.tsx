import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { formatTimecode } from "./timecode";

interface Props {
  videoRef: RefObject<HTMLVideoElement | null>;
  duration: number;
  playing: boolean;
  onTogglePlay: () => void;
  /** Master preview volume, 0..1. */
  volume: number;
  muted: boolean;
  onVolume: (volume: number) => void;
  onToggleMute: () => void;
  onSeek: (time: number) => void;
}

/**
 * The app's only playback controller: play/pause, scrub, time readout and the
 * master volume for the preview mix. The <video> element is rendered without
 * native `controls`, so nothing pops up over the picture on hover — this bar
 * never hides and exposes no tooltips.
 */
export function PlayerControls({
  videoRef,
  duration,
  playing,
  onTogglePlay,
  volume,
  muted,
  onVolume,
  onToggleMute,
  onSeek,
}: Props) {
  const barRef = useRef<HTMLDivElement>(null);
  // Set while the pointer is down on the bar: the rAF playhead must not fight
  // the thumb position the user is dragging.
  const scrubbingRef = useRef(false);

  // Playhead position. Kept local (and read off the element every frame) so
  // playback never re-renders the app, and so it survives the <video> being
  // swapped for a proxy/remux. Sub-frame changes bail out via setState.
  const [currentTime, setCurrentTime] = useState(0);
  useEffect(() => {
    let raf = 0;
    const start = () => {
      raf = requestAnimationFrame(function tick() {
        if (!scrubbingRef.current) {
          const t = videoRef.current?.currentTime ?? 0;
          setCurrentTime(prev => (Math.abs(t - prev) < 0.03 ? prev : t));
        }
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

  const seekAt = useCallback((clientX: number) => {
    const bar = barRef.current;
    if (!bar || duration <= 0) return;
    const { left, width } = bar.getBoundingClientRect();
    if (width <= 0) return;
    const time = Math.max(0, Math.min(1, (clientX - left) / width)) * duration;
    setCurrentTime(time);
    onSeek(time);
  }, [duration, onSeek]);

  const playedPct = duration > 0 ? Math.min(100, (currentTime / duration) * 100) : 0;

  return (
    <div className="player-controls">
      <button
        className="pc-btn"
        onClick={onTogglePlay}
        aria-label={playing ? "Pause" : "Play"}
      >
        {playing ? "❚❚" : "▶"}
      </button>

      <div
        ref={barRef}
        className="pc-bar"
        role="slider"
        aria-label="Playback position"
        aria-valuemin={0}
        aria-valuemax={Math.round(duration)}
        aria-valuenow={Math.round(currentTime)}
        onPointerDown={e => {
          scrubbingRef.current = true;
          e.currentTarget.setPointerCapture(e.pointerId);
          seekAt(e.clientX);
        }}
        onPointerMove={e => {
          if (scrubbingRef.current) seekAt(e.clientX);
        }}
        onPointerUp={e => {
          scrubbingRef.current = false;
          e.currentTarget.releasePointerCapture(e.pointerId);
        }}
        onPointerCancel={() => {
          scrubbingRef.current = false;
        }}
      >
        <div className="pc-track" />
        <div className="pc-fill" style={{ width: `${playedPct}%` }} />
        <div className="pc-thumb" style={{ left: `${playedPct}%` }} />
      </div>

      <span className="pc-time">
        {formatTimecode(currentTime)} / {formatTimecode(duration)}
      </span>

      <button
        className="pc-btn"
        onClick={onToggleMute}
        aria-label={muted ? "Unmute preview" : "Mute preview"}
      >
        {muted || volume === 0 ? "🔇" : "🔊"}
      </button>
      <input
        type="range"
        min={0}
        max={100}
        step={1}
        value={Math.round((muted ? 0 : volume) * 100)}
        onChange={e => onVolume(parseInt(e.target.value) / 100)}
        className="pc-volume"
        aria-label="Preview volume"
      />
    </div>
  );
}
