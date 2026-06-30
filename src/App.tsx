import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

function App() {
  const [version, setVersion] = useState<string>("loading…");

  useEffect(() => {
    invoke<string>("ffmpeg_version")
      .then(setVersion)
      .catch((e) => setVersion(`ERROR: ${e}`));
  }, []);

  return (
    <main className="container">
      <h1>ClipSnip</h1>
      <p data-testid="ffmpeg-version">Bundled FFmpeg: {version}</p>
    </main>
  );
}

export default App;
