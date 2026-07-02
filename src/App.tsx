import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { probeClip, exportTrim, exportCompressed, prepareAudioTracks, type ClipMeta, type TrackConfig } from "./api";
import { isCustomSelectionValid, resolveTargetMb, TARGET_SIZE_PRESETS } from "./exportOptions";
import { setIn, setOut } from "./trim";
import { Timeline } from "./Timeline";
import "./App.css";

interface LibraryEntry {
  path: string;
  meta: ClipMeta;
}

function basename(p: string) {
  return p.split(/[\\/]/).pop() ?? p;
}

function App() {
  const [path, setPath] = useState<string | null>(null);
  const [meta, setMeta] = useState<ClipMeta | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [videoError, setVideoError] = useState<string | null>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const [inPoint, setInPoint] = useState<number | null>(null);
  const [outPoint, setOutPoint] = useState<number | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const [library, setLibrary] = useState<LibraryEntry[]>([]);
  const [mediaPort, setMediaPort] = useState<number | null>(null);
  const [sizeSelection, setSizeSelection] = useState("original");
  const [customMb, setCustomMb] = useState("");

  // ── Multi-track audio state ──────────────────────────────────────────────────
  const [trackConfigs, setTrackConfigs] = useState<TrackConfig[]>([]);
  const [audioTrackPaths, setAudioTrackPaths] = useState<string[]>([]);
  const [audioReady, setAudioReady] = useState(false);
  const [audioPreparingMsg, setAudioPreparingMsg] = useState<string | null>(null);

  // Refs that live outside React's render cycle
  const trackConfigsRef = useRef<TrackConfig[]>([]);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const audioElsRef = useRef<HTMLAudioElement[]>([]);
  const gainNodesRef = useRef<GainNode[]>([]);

  // Keep the ref in sync so Web Audio effects can read current config
  useEffect(() => {
    trackConfigsRef.current = trackConfigs;
  }, [trackConfigs]);

  useEffect(() => {
    invoke<number>("get_media_port").then(setMediaPort);
  }, []);

  // Force-mute the native <video> element's own audio the moment Web Audio
  // takes over. React's `muted` prop only sets the attribute; on an
  // already-rendering <video> node WebKitGTK doesn't reliably re-read it from
  // a prop diff, so the element's first (default) embedded track kept playing
  // alongside the same track's Web Audio copy — set the DOM property directly.
  useEffect(() => {
    if (videoRef.current) videoRef.current.muted = audioReady;
  }, [audioReady]);

  // ── Tear down Web Audio nodes and <audio> elements ───────────────────────────
  function teardownAudio() {
    gainNodesRef.current.forEach(g => g.disconnect());
    audioElsRef.current.forEach(a => { a.pause(); a.src = ""; });
    gainNodesRef.current = [];
    audioElsRef.current = [];
  }

  // ── (Re-)build Web Audio graph whenever extracted track paths arrive ──────────
  useEffect(() => {
    if (!mediaPort || audioTrackPaths.length === 0) return;

    teardownAudio();

    if (!audioCtxRef.current || audioCtxRef.current.state === "closed") {
      audioCtxRef.current = new AudioContext();
    }
    const ctx = audioCtxRef.current;

    for (let i = 0; i < audioTrackPaths.length; i++) {
      const filePath = audioTrackPaths[i];
      const url = `http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(filePath)}`;

      // crossOrigin must be set BEFORE src so the browser sends a CORS request;
      // createMediaElementSource requires CORS-clean media when origins differ.
      const audioEl = new Audio();
      audioEl.crossOrigin = "anonymous";
      audioEl.src = url;
      audioEl.preload = "auto";

      let source: MediaElementAudioSourceNode;
      try {
        source = ctx.createMediaElementSource(audioEl);
      } catch (err) {
        console.error(`createMediaElementSource failed for track ${i}:`, err);
        continue;
      }

      const gain = ctx.createGain();
      const cfg = trackConfigsRef.current[i];
      gain.gain.value = (cfg?.enabled ?? true) ? (cfg?.volume ?? 1) : 0;

      source.connect(gain);
      gain.connect(ctx.destination);

      audioElsRef.current.push(audioEl);
      gainNodesRef.current.push(gain);
    }

    // If video is already playing when tracks become ready, sync and start audio
    const v = videoRef.current;
    if (v && !v.paused) {
      ctx.resume();
      audioElsRef.current.forEach(a => {
        a.currentTime = v.currentTime;
        a.play().catch(() => {});
      });
    }

    setAudioReady(true);

    return teardownAudio;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [audioTrackPaths, mediaPort]);

  // ── Sync audio elements when video plays / pauses / seeks ────────────────────
  function onVideoPlay() {
    const v = videoRef.current;
    if (!v || audioElsRef.current.length === 0) return;
    audioCtxRef.current?.resume();
    audioElsRef.current.forEach(a => {
      a.currentTime = v.currentTime;
      a.play().catch(() => {});
    });
  }

  function onVideoPause() {
    audioElsRef.current.forEach(a => a.pause());
  }

  // The native <video controls> bar exposes its own volume/mute button that
  // operates directly on the DOM element, bypassing React entirely — a user
  // clicking it (or dragging the slider) unmutes the element's own decoded
  // audio, which then plays alongside the Web Audio mix and isn't reachable
  // by the per-track sliders. Once Web Audio owns playback, force it back.
  function onVideoVolumeChange() {
    const v = videoRef.current;
    if (v && audioReady && !v.muted) {
      v.muted = true;
    }
  }

  function onVideoSeeked() {
    const v = videoRef.current;
    if (!v) return;
    audioElsRef.current.forEach(a => { a.currentTime = v.currentTime; });
  }

  // ── Per-track config update: applies immediately to the gain node ─────────────
  function updateTrackConfig(i: number, patch: Partial<TrackConfig>) {
    setTrackConfigs(prev => {
      const next = prev.map((c, idx) => idx === i ? { ...c, ...patch } : c);
      const cfg = next[i];
      const gain = gainNodesRef.current[i];
      if (gain) {
        gain.gain.value = cfg.enabled ? cfg.volume : 0;
      }
      return next;
    });
  }

  // ── Keyboard shortcuts ────────────────────────────────────────────────────────
  function markIn() {
    const t = videoRef.current?.currentTime ?? 0;
    setInPoint(setIn(t, outPoint));
  }
  function markOut() {
    const t = videoRef.current?.currentTime ?? 0;
    setOutPoint(setOut(t, inPoint, meta?.durationSecs ?? t));
  }

  function seekTo(time: number) {
    if (videoRef.current) videoRef.current.currentTime = time;
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.target as HTMLElement).tagName === "INPUT") return;
      if (e.key === "i" || e.key === "I") markIn();
      if (e.key === "o" || e.key === "O") markOut();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [inPoint, outPoint, meta]);

  // ── Export ────────────────────────────────────────────────────────────────────
  async function doExport() {
    if (!path || inPoint === null || outPoint === null) {
      setStatus("Set In and Out first.");
      return;
    }
    if (!isCustomSelectionValid(sizeSelection, customMb)) {
      setStatus("Enter a valid custom size in MB first.");
      return;
    }
    const targetMb = resolveTargetMb(sizeSelection, customMb);
    const dest = await save({ defaultPath: "clip-trimmed.mp4" });
    if (!dest) return;
    setStatus(
      targetMb === null ? "Exporting…" : `Compressing to ${targetMb} MB (two-pass encode)…`
    );
    try {
      const out =
        targetMb === null
          ? await exportTrim(path, inPoint, outPoint, dest, trackConfigs)
          : await exportCompressed(path, inPoint, outPoint, dest, trackConfigs, targetMb);
      setStatus(`Exported: ${out}`);
    } catch (e) {
      setStatus(`Export failed: ${e}`);
    }
  }

  // ── Open clip ─────────────────────────────────────────────────────────────────
  async function openClip(p: string) {
    setError(null);
    setVideoError(null);
    setPath(p);
    setInPoint(null);
    setOutPoint(null);
    setStatus(null);
    setCurrentTime(0);

    // Reset audio state for the new clip
    setAudioReady(false);
    setAudioTrackPaths([]);
    setTrackConfigs([]);
    setAudioPreparingMsg(null);
    teardownAudio();

    try {
      const m = await probeClip(p);
      setMeta(m);
      setLibrary(prev =>
        prev.some(e => e.path === p) ? prev : [...prev, { path: p, meta: m }]
      );

      if (m.audioTracks.length > 0) {
        const configs: TrackConfig[] = m.audioTracks.map(t => ({
          streamIndex: t.index,
          enabled: true,
          volume: 1.0,
        }));
        setTrackConfigs(configs);
        trackConfigsRef.current = configs;

        setAudioPreparingMsg("Extracting audio tracks…");
        const indices = m.audioTracks.map(t => t.index);
        prepareAudioTracks(p, indices)
          .then(paths => {
            setAudioTrackPaths(paths);
            setAudioPreparingMsg(null);
          })
          .catch(err => {
            setAudioPreparingMsg(null);
            console.error("Audio track extraction failed:", err);
          });
      }
    } catch (e) {
      setError(String(e));
      setMeta(null);
    }
  }

  async function pickClip() {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mkv", "mov", "avi", "webm", "flv"] }],
    });
    if (typeof selected !== "string") return;
    await openClip(selected);
  }

  // ── Render ────────────────────────────────────────────────────────────────────
  return (
    <div className="app-layout">
      {/* Library sidebar */}
      <aside className="library-panel">
        <div className="library-header">
          <span className="library-title">Library</span>
          <button className="btn-sm" onClick={pickClip}>+ Open</button>
        </div>
        {library.length === 0 ? (
          <p className="library-empty">Open a clip to add it here.</p>
        ) : (
          <ul className="library-list">
            {library.map(entry => (
              <li
                key={entry.path}
                className={`library-item${entry.path === path ? " active" : ""}`}
                onClick={() => openClip(entry.path)}
                title={entry.path}
              >
                <span className="library-name">{basename(entry.path)}</span>
                <span className="library-meta">
                  {entry.meta.durationSecs.toFixed(1)}s · {entry.meta.width}×{entry.meta.height}
                </span>
              </li>
            ))}
          </ul>
        )}
      </aside>

      {/* Editor area */}
      <main className="editor-panel">
        {!path && (
          <div className="empty-state">
            <h1>ClipSnip</h1>
            <p>Pick a video clip to start trimming.</p>
            <button onClick={pickClip}>Open clip…</button>
          </div>
        )}

        {error && <div className="msg-error">{error}</div>}

        {path && meta && (
          <div className="clip-info">
            <span className="clip-name">{basename(path)}</span>
            <span className="clip-details">
              {meta.durationSecs.toFixed(2)}s · {meta.width}×{meta.height} · {meta.videoCodec} · {meta.audioTracks.length}ch
            </span>
          </div>
        )}

        {path && (
          <>
            {/* Video: muted once Web Audio takes over */}
            <video
              ref={videoRef}
              src={mediaPort ? `http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(path)}` : undefined}
              controls
              muted={audioReady}
              className="video-player"
              onTimeUpdate={e => setCurrentTime(e.currentTarget.currentTime)}
              onPlay={onVideoPlay}
              onPause={onVideoPause}
              onSeeked={onVideoSeeked}
              onVolumeChange={onVideoVolumeChange}
              onError={e => {
                const err = (e.currentTarget as HTMLVideoElement).error;
                setVideoError(
                  `Playback error (code ${err?.code ?? "?"}): ${err?.message || "unsupported or inaccessible file"}`
                );
              }}
            />

            {videoError && (
              <div className="msg-warn">
                <strong>Video won't play:</strong> {videoError}
              </div>
            )}

            {/* Audio tracks panel */}
            {meta && meta.audioTracks.length > 0 && (
              <div className="audio-panel">
                <div className="audio-panel-header">
                  <span className="audio-panel-title">Audio Tracks</span>
                  {audioPreparingMsg && (
                    <span className="audio-preparing">{audioPreparingMsg}</span>
                  )}
                </div>
                {trackConfigs.map((cfg, i) => {
                  const track = meta.audioTracks[i];
                  if (!track) return null;
                  const label = track.language
                    ? `Track ${i + 1} (${track.language})`
                    : `Track ${i + 1}`;
                  return (
                    <div key={cfg.streamIndex} className={`audio-track-row${cfg.enabled ? "" : " muted"}`}>
                      <label className="audio-track-label">
                        <input
                          type="checkbox"
                          checked={cfg.enabled}
                          onChange={e => updateTrackConfig(i, { enabled: e.target.checked })}
                        />
                        <span className="audio-track-name">{label}</span>
                        <span className="audio-track-meta">{track.codec} · {track.channels}ch</span>
                      </label>
                      <div className="audio-track-volume">
                        <input
                          type="range"
                          min={0}
                          max={200}
                          step={1}
                          value={Math.round(cfg.volume * 100)}
                          disabled={!cfg.enabled}
                          onChange={e => updateTrackConfig(i, { volume: parseInt(e.target.value) / 100 })}
                          className="audio-volume-slider"
                        />
                        <span className="audio-volume-pct">
                          {cfg.enabled ? `${Math.round(cfg.volume * 100)}%` : "—"}
                        </span>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}

            {meta && (
              <Timeline
                duration={meta.durationSecs}
                currentTime={currentTime}
                inPoint={inPoint}
                outPoint={outPoint}
                onSeek={seekTo}
              />
            )}

            <div className="trim-controls" data-testid="trim-controls">
              <button onClick={markIn}>Set In (I)</button>
              <span className="trim-readout">
                {inPoint !== null ? inPoint.toFixed(2) + "s" : "—"} → {outPoint !== null ? outPoint.toFixed(2) + "s" : "—"}
              </span>
              <button onClick={markOut}>Set Out (O)</button>
              <label className="export-size">
                <span className="export-size-label">Size</span>
                <select
                  value={sizeSelection}
                  onChange={e => setSizeSelection(e.target.value)}
                  className="export-size-select"
                >
                  <option value="original">Original quality</option>
                  {TARGET_SIZE_PRESETS.map(mb => (
                    <option key={mb} value={String(mb)}>{mb} MB{mb === 8 ? " (Discord)" : ""}</option>
                  ))}
                  <option value="custom">Custom…</option>
                </select>
              </label>
              {sizeSelection === "custom" && (
                <input
                  type="number"
                  min={1}
                  step={1}
                  placeholder="MB"
                  value={customMb}
                  onChange={e => setCustomMb(e.target.value)}
                  className="export-size-custom"
                />
              )}
              <button className="btn-export" onClick={doExport}>Export trim</button>
            </div>

            {status && (
              <div
                className={status.startsWith("Export failed") ? "msg-error" : "msg-ok"}
                data-testid="export-status"
              >
                {status}
              </div>
            )}
          </>
        )}
      </main>
    </div>
  );
}

export default App;
