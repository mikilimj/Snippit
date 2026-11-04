import type { RefObject } from "react";
import { formatTimecode } from "./timecode";
import { useVideoClock } from "./useVideoClock";

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
  fullscreen: boolean;
  onToggleFullscreen: () => void;
}

function PlayIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
      <path d="M7 4.5v15l13-7.5z" fill="currentColor" />
    </svg>
  );
}

function PauseIcon() {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true">
      <rect x="6" y="4.5" width="4" height="15" rx="1" fill="currentColor" />
      <rect x="14" y="4.5" width="4" height="15" rx="1" fill="currentColor" />
    </svg>
  );
}

/** Plain monochrome speaker: no emoji, so it stays legible at 16px. */
function SpeakerIcon({ muted }: { muted: boolean }) {
  return (
    <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true">
      <path d="M4 9.5h3.2L12 5.6v12.8L7.2 14.5H4z" fill="currentColor" />
      {muted ? (
        <path
          d="M16 9.5l5 5m0-5l-5 5"
          stroke="currentColor"
          strokeWidth="1.7"
          strokeLinecap="round"
          fill="none"
        />
      ) : (
        <g stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" fill="none">
          <path d="M15.4 9.4a4 4 0 0 1 0 5.2" />
          <path d="M18.1 6.9a7.4 7.4 0 0 1 0 10.2" />
        </g>
      )}
    </svg>
  );
}

function FullscreenIcon({ active }: { active: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="16"
      height="16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {active ? (
        <path d="M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5" />
      ) : (
        <path d="M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" />
      )}
    </svg>
  );
}

/**
 * Transport buttons for the popup: play/pause, position readout, mute + master
 * preview volume and fullscreen. Position scrubbing and trimming live together
 * on the timeline right above it, so there is exactly one place that moves the
 * playhead. The <video> has no native controls, so nothing pops up on hover.
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
  fullscreen,
  onToggleFullscreen,
}: Props) {
  const currentTime = useVideoClock(videoRef);

  return (
    <div className="player-controls">
      <button className="pc-btn" onClick={onTogglePlay} aria-label={playing ? "Pause" : "Play"}>
        {playing ? <PauseIcon /> : <PlayIcon />}
      </button>

      <span className="pc-time">
        {formatTimecode(currentTime)} / {formatTimecode(duration)}
      </span>

      <span className="pc-spacer" />

      <button
        className="pc-btn"
        onClick={onToggleMute}
        aria-label={muted ? "Unmute preview" : "Mute preview"}
      >
        <SpeakerIcon muted={muted || volume === 0} />
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

      <button
        className="pc-btn"
        onClick={onToggleFullscreen}
        aria-label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
      >
        <FullscreenIcon active={fullscreen} />
      </button>
    </div>
  );
}
