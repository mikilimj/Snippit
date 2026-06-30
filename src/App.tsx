import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { probeClip, type ClipMeta } from "./api";
import "./App.css";

function App() {
  const [path, setPath] = useState<string | null>(null);
  const [meta, setMeta] = useState<ClipMeta | null>(null);
  const [error, setError] = useState<string | null>(null);

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
          src={convertFileSrc(path)}
          controls
          style={{ width: "100%", maxWidth: 720, background: "#000" }}
        />
      )}
    </main>
  );
}

export default App;
