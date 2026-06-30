import { useEffect, useRef, useState } from "react";
import { open, save } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { probeClip, exportTrim, type ClipMeta } from "./api";
import { setIn, setOut } from "./trim";
import "./App.css";

function App() {
  const [path, setPath] = useState<string | null>(null);
  const [meta, setMeta] = useState<ClipMeta | null>(null);
  const [error, setError] = useState<string | null>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const [inPoint, setInPoint] = useState<number | null>(null);
  const [outPoint, setOutPoint] = useState<number | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  function markIn() {
    const t = videoRef.current?.currentTime ?? 0;
    setInPoint(setIn(t, outPoint));
  }
  function markOut() {
    const t = videoRef.current?.currentTime ?? 0;
    setOutPoint(setOut(t, inPoint, meta?.durationSecs ?? t));
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === "i" || e.key === "I") markIn();
      if (e.key === "o" || e.key === "O") markOut();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [inPoint, outPoint, meta]);

  async function doExport() {
    if (!path || inPoint === null || outPoint === null) {
      setStatus("Set In and Out first.");
      return;
    }
    const dest = await save({ defaultPath: "clip-trimmed.mp4" });
    if (!dest) return;
    setStatus("Exporting…");
    try {
      const out = await exportTrim(path, inPoint, outPoint, dest);
      setStatus(`Exported: ${out}`);
    } catch (e) {
      setStatus(`Export failed: ${e}`);
    }
  }

  async function pickClip() {
    setError(null);
    const selected = await open({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mkv", "mov", "avi", "webm", "flv"] }],
    });
    if (typeof selected !== "string") return;
    setPath(selected);
    try {
      setMeta(await probeClip(selected));
    } catch (e) {
      setError(String(e));
      setMeta(null);
    }
  }

  return (
    <main className="container">
      <h1>ClipSnip</h1>
      <button onClick={pickClip}>Open clip…</button>
      {path && <p>{path}</p>}
      {error && <p style={{ color: "tomato" }}>{error}</p>}
      {meta && (
        <ul data-testid="clip-meta">
          <li>Duration: {meta.durationSecs.toFixed(2)} s</li>
          <li>Resolution: {meta.width}×{meta.height}</li>
          <li>Container: {meta.container}</li>
          <li>Video codec: {meta.videoCodec}</li>
          <li>Audio tracks: {meta.audioTracks.length}</li>
        </ul>
      )}
      {path && (
        <video
          ref={videoRef}
          src={convertFileSrc(path)}
          controls
          style={{ width: "100%", maxWidth: 720, background: "#000" }}
        />
      )}
      {path && (
        <div data-testid="trim-controls">
          <button onClick={markIn}>Set In (I)</button>
          <button onClick={markOut}>Set Out (O)</button>
          <span> In: {inPoint?.toFixed(2) ?? "—"} | Out: {outPoint?.toFixed(2) ?? "—"}</span>
          <button onClick={doExport}>Export trim</button>
          {status && <p data-testid="export-status">{status}</p>}
        </div>
      )}
    </main>
  );
}

export default App;
