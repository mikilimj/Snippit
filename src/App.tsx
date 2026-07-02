import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { probeClip, exportTrim, exportCompressed, cancelExport, prepareAudioTracks, getAppRoot, addToLibrary, listLibrary, removeFromLibrary, rescanLibrary, setClipFavorite, renameLibraryClip, createFolder, listFolders, deleteFolder, addClipToFolder, removeClipFromFolder, listClipsInFolder, listUnsortedLibrary, ensureProxy, type ClipMeta, type ClipRow, type ExportMode, type ExportProgress, type Folder, type TrackConfig, type SetupResult } from "./api";
import { exportSequence, type MusicOverlay } from "./api";
import { newClipBelongsInView, applyFavorite, applyRename, type LibraryView } from "./libraryView";
import { previewPath, shouldTryProxy } from "./preview";
import { moveSegment, removeSegment, sequenceDuration, toBackendSegments, type SequenceSegment } from "./sequence";
import { isCustomSelectionValid, resolveTargetMb, TARGET_SIZE_PRESETS } from "./exportOptions";
import { setIn, setOut } from "./trim";
import { Timeline } from "./Timeline";
import { FirstRunWizard } from "./FirstRunWizard";
import { SettingsModal } from "./SettingsModal";
import logo from "./assets/logo.png";
import "./App.css";

function basename(p: string) {
  return p.split(/[\\/]/).pop() ?? p;
}

function App() {
  const [path, setPath] = useState<string | null>(null);
  const [meta, setMeta] = useState<ClipMeta | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [videoError, setVideoError] = useState<string | null>(null);
  // Preview proxy (spec §4.4): set when the WebView can't decode the original.
  const [proxyPath, setProxyPath] = useState<string | null>(null);
  const [proxyStatus, setProxyStatus] = useState<string | null>(null);
  const videoRef = useRef<HTMLVideoElement>(null);
  const [inPoint, setInPoint] = useState<number | null>(null);
  const [outPoint, setOutPoint] = useState<number | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const [library, setLibrary] = useState<ClipRow[]>([]);
  const [folders, setFolders] = useState<Folder[]>([]);
  const [activeView, setActiveView] = useState<LibraryView>({ kind: "all" });
  const [creatingFolder, setCreatingFolder] = useState(false);
  const [newFolderName, setNewFolderName] = useState("");
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editingName, setEditingName] = useState("");
  const [mediaPort, setMediaPort] = useState<number | null>(null);
  const [sizeSelection, setSizeSelection] = useState("original");
  const [customMb, setCustomMb] = useState("");
  const [exportMode, setExportMode] = useState<ExportMode>("fast");
  const [exporting, setExporting] = useState(false);
  const [progress, setProgress] = useState<ExportProgress | null>(null);
  // Light timeline (spec §4.3): an ordered list of trimmed segments + optional music.
  const [segments, setSegments] = useState<SequenceSegment[]>([]);
  const [music, setMusic] = useState<MusicOverlay | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // `undefined` = still checking, `null` = needs first-run setup, string = configured root
  const [appRoot, setAppRoot] = useState<string | null | undefined>(undefined);

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
  // Set when the user hits Escape so the imminent blur cancels instead of commits.
  const renameCancelRef = useRef(false);

  // Keep the ref in sync so Web Audio effects can read current config
  useEffect(() => {
    trackConfigsRef.current = trackConfigs;
  }, [trackConfigs]);

  useEffect(() => {
    invoke<number>("get_media_port").then(setMediaPort);
  }, []);

  // Live export progress streamed from the Rust engine (spec §4.5).
  useEffect(() => {
    const unlisten = listen<ExportProgress>("export-progress", e => setProgress(e.payload));
    return () => {
      unlisten.then(f => f());
    };
  }, []);

  useEffect(() => {
    getAppRoot()
      .then(setAppRoot)
      .catch(() => setAppRoot(null));
  }, []);

  // Fetch the clips for the given filter view from the backend. Each view maps
  // to a dedicated query so folder/unsorted membership stays server-authoritative.
  async function loadView(view: LibraryView) {
    try {
      let rows: ClipRow[];
      switch (view.kind) {
        case "all":
          rows = await listLibrary();
          break;
        case "favorites":
          rows = (await listLibrary()).filter(c => c.favorite);
          break;
        case "unsorted":
          rows = await listUnsortedLibrary();
          break;
        case "folder":
          rows = await listClipsInFolder(view.id);
          break;
      }
      setLibrary(rows);
    } catch (err) {
      console.error("Failed to load library view:", err);
    }
  }

  // Load the clips for the active view once setup is configured, and whenever
  // the user switches views.
  useEffect(() => {
    if (typeof appRoot !== "string") return;
    loadView(activeView);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [appRoot, activeView]);

  // Load the virtual folders once setup is configured.
  useEffect(() => {
    if (typeof appRoot !== "string") return;
    listFolders()
      .then(setFolders)
      .catch(err => console.error("Failed to load folders:", err));
  }, [appRoot]);

  // Auto-import: the Rust watcher emits `clip-added` for each new/updated clip
  // it ingests. A fresh clip is unsorted and not a favorite, so only merge it
  // into views where it belongs (All / Unsorted); other views ignore it.
  useEffect(() => {
    if (typeof appRoot !== "string") return;
    const unlisten = listen<ClipRow>("clip-added", e => {
      if (!newClipBelongsInView(activeView)) return;
      const row = e.payload;
      setLibrary(prev => [row, ...prev.filter(x => x.path !== row.path)]);
    });
    return () => {
      unlisten.then(f => f());
    };
  }, [appRoot, activeView]);

  // A library import (spec §4.6) rewrites the index wholesale — reload the current
  // view and the folder list when the backend signals it changed.
  useEffect(() => {
    if (typeof appRoot !== "string") return;
    const unlisten = listen("library-changed", () => {
      loadView(activeView);
      listFolders()
        .then(setFolders)
        .catch(err => console.error("Failed to reload folders:", err));
    });
    return () => {
      unlisten.then(f => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [appRoot, activeView]);

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

  // Native-first, proxy-fallback (spec §4.4): if the WebView can't decode the
  // original, transcode a 720p H.264 proxy in the background and switch to it.
  // If the proxy itself fails, surface the error instead of looping.
  function handleVideoError(e: React.SyntheticEvent<HTMLVideoElement>) {
    const err = e.currentTarget.error;
    if (path && shouldTryProxy(proxyPath)) {
      setVideoError(null);
      setProxyStatus("This format won't play natively — building a compatible preview…");
      ensureProxy(path)
        .then(pp => {
          setProxyPath(pp);
          setProxyStatus(null);
        })
        .catch(genErr => {
          setProxyStatus(null);
          setVideoError(`Couldn't build a preview: ${genErr}`);
        });
      return;
    }
    setVideoError(
      `Playback error (code ${err?.code ?? "?"}): ${err?.message || "unsupported or inaccessible file"}`
    );
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

  function handleSetupComplete(result: SetupResult) {
    setAppRoot(result.rootPath);
    if (result.warnings.length > 0) {
      setStatus(`Setup finished with warnings: ${result.warnings.join("; ")}`);
    }
  }

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
    setExporting(true);
    setProgress({ fraction: 0, phase: targetMb === null ? "Exporting" : "Analyzing" });
    setStatus(
      targetMb === null
        ? `Exporting (${exportMode === "fast" ? "Fast" : "Compatible"})…`
        : `Compressing to ${targetMb} MB (two-pass encode)…`
    );
    try {
      const out =
        targetMb === null
          ? await exportTrim(path, inPoint, outPoint, dest, trackConfigs, exportMode)
          : await exportCompressed(path, inPoint, outPoint, dest, trackConfigs, targetMb);
      setStatus(`Exported: ${out}`);
    } catch (e) {
      const msg = String(e);
      setStatus(/cancel/i.test(msg) ? "Export cancelled." : `Export failed: ${msg}`);
    } finally {
      setExporting(false);
      setProgress(null);
    }
  }

  async function cancelExportClick() {
    setStatus("Cancelling…");
    try {
      await cancelExport();
    } catch (e) {
      console.error("Cancel failed:", e);
    }
  }

  // ── Sequence (light timeline) + music ─────────────────────────────────────────
  function addToSequence() {
    if (!path || inPoint === null || outPoint === null) {
      setStatus("Set In and Out before adding to the sequence.");
      return;
    }
    setSegments(prev => [
      ...prev,
      { id: Date.now(), path, displayName: basename(path), inSecs: inPoint, outSecs: outPoint },
    ]);
  }

  async function pickMusic() {
    const selected = await open({
      multiple: false,
      filters: [{ name: "Audio", extensions: ["mp3", "aac", "m4a", "wav", "flac", "ogg", "opus"] }],
    });
    if (typeof selected !== "string") return;
    setMusic({ path: selected, volume: 1.0, loopToLength: true });
  }

  function updateMusic(patch: Partial<MusicOverlay>) {
    setMusic(prev => (prev ? { ...prev, ...patch } : prev));
  }

  async function exportSequenceClick() {
    if (segments.length === 0) return;
    const dest = await save({ defaultPath: "sequence.mp4" });
    if (!dest) return;
    setExporting(true);
    setProgress({ fraction: 0, phase: "Stitching" });
    setStatus(`Stitching ${segments.length} clip${segments.length > 1 ? "s" : ""}…`);
    try {
      const out = await exportSequence(toBackendSegments(segments), music, dest);
      setStatus(`Exported: ${out}`);
    } catch (e) {
      const msg = String(e);
      setStatus(/cancel/i.test(msg) ? "Export cancelled." : `Export failed: ${msg}`);
    } finally {
      setExporting(false);
      setProgress(null);
    }
  }

  // ── Open clip ─────────────────────────────────────────────────────────────────
  async function openClip(p: string) {
    setError(null);
    setVideoError(null);
    setProxyPath(null);
    setProxyStatus(null);
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
      // Persist to the library index; fall back to an in-session row if the
      // write fails so the clip still shows in the sidebar. A just-opened clip
      // is unsorted, so it only appears in views that show new clips.
      addToLibrary(p, m)
        .then(row => {
          if (!newClipBelongsInView(activeView)) return;
          setLibrary(prev => [row, ...prev.filter(e => e.path !== p)]);
        })
        .catch(err => {
          console.error("Failed to add clip to library:", err);
          if (!newClipBelongsInView(activeView)) return;
          setLibrary(prev =>
            prev.some(e => e.path === p)
              ? prev
              : [
                  {
                    id: -Date.now(),
                    path: p,
                    displayName: basename(p),
                    durationSecs: m.durationSecs,
                    width: m.width,
                    height: m.height,
                    container: m.container,
                    videoCodec: m.videoCodec,
                    audioTrackCount: m.audioTracks.length,
                    thumbnailPath: null,
                    favorite: false,
                    missing: false,
                    addedAt: Math.floor(Date.now() / 1000),
                  },
                  ...prev,
                ]
          );
        });

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

  // Remove a clip from the library index only — the source file is untouched.
  async function removeFromLibraryClick(e: React.MouseEvent, entry: ClipRow) {
    e.stopPropagation();
    setLibrary(prev => prev.filter(x => x.id !== entry.id));
    if (entry.id >= 0) {
      try {
        await removeFromLibrary(entry.id);
      } catch (err) {
        console.error("Failed to remove clip from library:", err);
      }
    }
  }

  // ── Favorites, rename, folders ────────────────────────────────────────────────
  async function toggleFavorite(e: React.MouseEvent, entry: ClipRow) {
    e.stopPropagation();
    const favorite = !entry.favorite;
    setLibrary(prev => applyFavorite(prev, entry.id, favorite, activeView));
    if (entry.id < 0) return; // in-session-only fallback row, nothing to persist
    try {
      await setClipFavorite(entry.id, favorite);
    } catch (err) {
      console.error("Failed to set favorite:", err);
      loadView(activeView); // reconcile with the source of truth
    }
  }

  function startRename(e: React.MouseEvent, entry: ClipRow) {
    e.stopPropagation();
    renameCancelRef.current = false;
    setEditingId(entry.id);
    setEditingName(entry.displayName);
  }

  async function commitRename() {
    if (renameCancelRef.current) {
      renameCancelRef.current = false;
      return; // Escape was pressed — discard the edit.
    }
    const id = editingId;
    if (id === null) return;
    const name = editingName.trim();
    setEditingId(null);
    if (!name) return;
    setLibrary(prev => applyRename(prev, id, name));
    if (id < 0) return;
    try {
      await renameLibraryClip(id, name);
    } catch (err) {
      console.error("Failed to rename clip:", err);
      loadView(activeView);
    }
  }

  async function addToFolder(clipId: number, folderId: number) {
    try {
      await addClipToFolder(clipId, folderId);
      // In the Unsorted view the clip has just left this filter.
      if (activeView.kind === "unsorted") {
        setLibrary(prev => prev.filter(c => c.id !== clipId));
      }
    } catch (err) {
      console.error("Failed to add clip to folder:", err);
    }
  }

  async function removeFromCurrentFolder(e: React.MouseEvent, clipId: number) {
    e.stopPropagation();
    if (activeView.kind !== "folder") return;
    const folderId = activeView.id;
    setLibrary(prev => prev.filter(c => c.id !== clipId));
    try {
      await removeClipFromFolder(clipId, folderId);
    } catch (err) {
      console.error("Failed to remove clip from folder:", err);
      loadView(activeView);
    }
  }

  function cancelFolderCreation() {
    setCreatingFolder(false);
    setNewFolderName("");
  }

  // Only ever invoked from the Enter key, so there is no blur/Enter double-fire
  // (a second create attempt would trip the unique-name constraint).
  async function createFolderClick() {
    const name = newFolderName.trim();
    if (!name) {
      cancelFolderCreation();
      return;
    }
    try {
      const folder = await createFolder(name);
      setFolders(prev => [...prev, folder].sort((a, b) => a.name.localeCompare(b.name)));
      setNewFolderName("");
      setCreatingFolder(false);
      setActiveView({ kind: "folder", id: folder.id });
    } catch (err) {
      console.error("Failed to create folder:", err);
      setStatus(`Couldn't create folder: ${err}`);
    }
  }

  async function deleteFolderClick(e: React.MouseEvent, folder: Folder) {
    e.stopPropagation();
    try {
      await deleteFolder(folder.id);
      setFolders(prev => prev.filter(f => f.id !== folder.id));
      setActiveView(prev =>
        prev.kind === "folder" && prev.id === folder.id ? { kind: "all" } : prev
      );
    } catch (err) {
      console.error("Failed to delete folder:", err);
    }
  }

  // ── Render ────────────────────────────────────────────────────────────────────
  if (appRoot === undefined) {
    return null;
  }

  if (appRoot === null) {
    return <FirstRunWizard onComplete={handleSetupComplete} />;
  }

  return (
    <div className="app-layout">
      {settingsOpen && <SettingsModal onClose={() => setSettingsOpen(false)} />}
      {/* Library sidebar */}
      <aside className="library-panel">
        <div className="brand-row">
          <img className="brand-logo" src={logo} alt="" />
          <span className="brand-name">ClipSnip</span>
        </div>
        <div className="library-header">
          <span className="library-title">Library</span>
          <div className="library-header-actions">
            <button
              className="btn-sm"
              title="Re-scan watched folders"
              onClick={() => rescanLibrary().catch(err => console.error("Rescan failed:", err))}
            >
              ⟳
            </button>
            <button className="btn-sm" title="Settings" onClick={() => setSettingsOpen(true)}>
              ⚙
            </button>
            <button className="btn-sm" onClick={pickClip}>+ Open</button>
          </div>
        </div>
        <nav className="folder-rail">
          <button
            className={`folder-chip${activeView.kind === "all" ? " active" : ""}`}
            onClick={() => setActiveView({ kind: "all" })}
          >
            All
          </button>
          <button
            className={`folder-chip${activeView.kind === "favorites" ? " active" : ""}`}
            onClick={() => setActiveView({ kind: "favorites" })}
          >
            ★ Favorites
          </button>
          <button
            className={`folder-chip${activeView.kind === "unsorted" ? " active" : ""}`}
            onClick={() => setActiveView({ kind: "unsorted" })}
          >
            Unsorted
          </button>
          {folders.map(f => (
            <span
              key={f.id}
              className={`folder-chip folder-chip-user${
                activeView.kind === "folder" && activeView.id === f.id ? " active" : ""
              }`}
            >
              <button
                className="folder-chip-label"
                onClick={() => setActiveView({ kind: "folder", id: f.id })}
              >
                {f.name}
              </button>
              <button
                className="folder-chip-delete"
                title="Delete folder (keeps the clips)"
                aria-label="Delete folder"
                onClick={e => deleteFolderClick(e, f)}
              >
                ×
              </button>
            </span>
          ))}
          {creatingFolder ? (
            <input
              className="folder-new-input"
              autoFocus
              value={newFolderName}
              placeholder="Folder name"
              onChange={e => setNewFolderName(e.target.value)}
              onKeyDown={e => {
                if (e.key === "Enter") createFolderClick();
                if (e.key === "Escape") cancelFolderCreation();
              }}
              onBlur={cancelFolderCreation}
            />
          ) : (
            <button className="folder-chip folder-new" onClick={() => setCreatingFolder(true)}>
              + Folder
            </button>
          )}
        </nav>
        {library.length === 0 ? (
          <p className="library-empty">
            {activeView.kind === "all"
              ? "Open a clip to add it here."
              : activeView.kind === "favorites"
                ? "No favorites yet — tap ☆ on a clip."
                : activeView.kind === "unsorted"
                  ? "Nothing unsorted — every clip is filed away."
                  : "No clips in this folder yet."}
          </p>
        ) : (
          <ul className="library-list">
            {library.map(entry => (
              <li
                key={entry.id}
                className={`library-item${entry.path === path ? " active" : ""}`}
                onClick={() => openClip(entry.path)}
                title={entry.path}
              >
                {entry.thumbnailPath && mediaPort ? (
                  <img
                    className="library-thumb"
                    src={`http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(entry.thumbnailPath)}`}
                    alt=""
                    loading="lazy"
                  />
                ) : (
                  <div className="library-thumb library-thumb-empty" />
                )}
                <div className="library-item-text">
                  {editingId === entry.id ? (
                    <input
                      className="library-rename-input"
                      autoFocus
                      value={editingName}
                      onClick={e => e.stopPropagation()}
                      onChange={e => setEditingName(e.target.value)}
                      onKeyDown={e => {
                        if (e.key === "Enter") commitRename();
                        if (e.key === "Escape") {
                          renameCancelRef.current = true;
                          setEditingId(null);
                        }
                      }}
                      onBlur={commitRename}
                    />
                  ) : (
                    <span
                      className="library-name"
                      title="Double-click to rename"
                      onDoubleClick={e => startRename(e, entry)}
                    >
                      {entry.displayName}
                    </span>
                  )}
                  <span className="library-meta">
                    {entry.durationSecs.toFixed(1)}s · {entry.width}×{entry.height}
                  </span>
                </div>
                <div className="library-actions">
                  <button
                    className={`library-fav${entry.favorite ? " on" : ""}`}
                    title={entry.favorite ? "Unfavorite" : "Mark as favorite"}
                    aria-label={entry.favorite ? "Unfavorite" : "Mark as favorite"}
                    onClick={e => toggleFavorite(e, entry)}
                  >
                    {entry.favorite ? "★" : "☆"}
                  </button>
                  {folders.length > 0 && (
                    <select
                      className="library-folder-select"
                      title="Add to folder"
                      value=""
                      onClick={e => e.stopPropagation()}
                      onChange={e => {
                        const fid = Number(e.target.value);
                        if (fid) addToFolder(entry.id, fid);
                        e.target.value = "";
                      }}
                    >
                      <option value="">＋Folder</option>
                      {folders.map(f => (
                        <option key={f.id} value={f.id}>
                          {f.name}
                        </option>
                      ))}
                    </select>
                  )}
                  {activeView.kind === "folder" ? (
                    <button
                      className="library-remove"
                      title="Remove from this folder"
                      aria-label="Remove from this folder"
                      onClick={e => removeFromCurrentFolder(e, entry.id)}
                    >
                      −
                    </button>
                  ) : (
                    <button
                      className="library-remove"
                      title="Remove from library (keeps the file)"
                      aria-label="Remove from library"
                      onClick={e => removeFromLibraryClick(e, entry)}
                    >
                      ×
                    </button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
      </aside>

      {/* Editor area */}
      <main className="editor-panel">
        {!path && (
          <div className="empty-state">
            <div className="aurora-blob aurora-blob-1" />
            <div className="aurora-blob aurora-blob-2" />
            <div className="aurora-blob aurora-blob-3" />
            <div className="empty-card">
              <img className="empty-logo" src={logo} alt="" />
              <h1>ClipSnip</h1>
              <p>Open a clip, trim it, share it — in seconds.</p>
              <button className="btn-export" onClick={pickClip}>Open a clip</button>
            </div>
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
            {/* Video: original first, proxy fallback (§4.4); muted once Web Audio takes over */}
            <video
              ref={videoRef}
              key={previewPath(path, proxyPath)}
              src={mediaPort ? `http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(previewPath(path, proxyPath))}` : undefined}
              controls
              muted={audioReady}
              className="video-player"
              onTimeUpdate={e => setCurrentTime(e.currentTarget.currentTime)}
              onPlay={onVideoPlay}
              onPause={onVideoPause}
              onSeeked={onVideoSeeked}
              onVolumeChange={onVideoVolumeChange}
              onError={handleVideoError}
            />

            {proxyStatus && (
              <div className="msg-warn" data-testid="proxy-status">
                {proxyStatus}
              </div>
            )}

            {proxyPath && !proxyStatus && (
              <div className="msg-ok">
                Playing a 720p preview proxy. Export still uses the full-quality original.
              </div>
            )}

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
              {sizeSelection === "original" && (
                <label className="export-size">
                  <span className="export-size-label">Format</span>
                  <select
                    value={exportMode}
                    onChange={e => setExportMode(e.target.value as ExportMode)}
                    className="export-size-select"
                    title="Fast = instant stream copy at keyframes; Compatible = H.264 re-encode for universal playback"
                  >
                    <option value="fast">Fast (lossless copy)</option>
                    <option value="compatible">Compatible (H.264)</option>
                  </select>
                </label>
              )}
              <button className="btn-export" onClick={doExport} disabled={exporting}>
                {exporting ? "Exporting…" : "Export trim"}
              </button>
              {exporting && (
                <button className="btn-cancel" onClick={cancelExportClick}>Cancel</button>
              )}
            </div>

            {/* Light timeline: stitch trimmed clips + optional music (§4.3) */}
            <div className="sequence-panel">
              <div className="sequence-header">
                <span className="sequence-title">Sequence</span>
                <div className="sequence-actions">
                  <button
                    className="btn-sm"
                    onClick={addToSequence}
                    disabled={inPoint === null || outPoint === null}
                    title="Append the current In→Out selection to the sequence"
                  >
                    + Add clip
                  </button>
                  <button className="btn-sm" onClick={pickMusic} title="Overlay a music track">
                    ♪ {music ? "Change music" : "Add music"}
                  </button>
                </div>
              </div>

              {segments.length === 0 ? (
                <p className="sequence-empty">
                  Trim a clip, then “Add clip” to build a sequence. Straight cuts, played in order.
                </p>
              ) : (
                <>
                  <ol className="sequence-list">
                    {segments.map((seg, i) => (
                      <li key={seg.id} className="sequence-item">
                        <span className="sequence-index">{i + 1}</span>
                        <span className="sequence-name" title={seg.path}>{seg.displayName}</span>
                        <span className="sequence-span">
                          {seg.inSecs.toFixed(1)}–{seg.outSecs.toFixed(1)}s
                        </span>
                        <div className="sequence-item-actions">
                          <button
                            className="seq-move"
                            disabled={i === 0}
                            onClick={() => setSegments(prev => moveSegment(prev, i, -1))}
                            title="Move up"
                            aria-label="Move up"
                          >
                            ▲
                          </button>
                          <button
                            className="seq-move"
                            disabled={i === segments.length - 1}
                            onClick={() => setSegments(prev => moveSegment(prev, i, 1))}
                            title="Move down"
                            aria-label="Move down"
                          >
                            ▼
                          </button>
                          <button
                            className="library-remove"
                            onClick={() => setSegments(prev => removeSegment(prev, seg.id))}
                            title="Remove from sequence"
                            aria-label="Remove from sequence"
                          >
                            ×
                          </button>
                        </div>
                      </li>
                    ))}
                  </ol>
                  <div className="sequence-total">
                    Total: {sequenceDuration(segments).toFixed(1)}s
                  </div>
                </>
              )}

              {music && (
                <div className="music-row">
                  <span className="music-name" title={music.path}>♪ {basename(music.path)}</span>
                  <div className="audio-track-volume">
                    <input
                      type="range"
                      min={0}
                      max={200}
                      value={Math.round(music.volume * 100)}
                      onChange={e => updateMusic({ volume: parseInt(e.target.value) / 100 })}
                      className="audio-volume-slider"
                    />
                    <span className="audio-volume-pct">{Math.round(music.volume * 100)}%</span>
                  </div>
                  <label className="music-loop">
                    <input
                      type="checkbox"
                      checked={music.loopToLength}
                      onChange={e => updateMusic({ loopToLength: e.target.checked })}
                    />
                    Loop
                  </label>
                  <button
                    className="library-remove"
                    onClick={() => setMusic(null)}
                    title="Remove music"
                    aria-label="Remove music"
                  >
                    ×
                  </button>
                </div>
              )}

              {segments.length > 0 && (
                <button className="btn-export" onClick={exportSequenceClick} disabled={exporting}>
                  {exporting ? "Exporting…" : `Export sequence (${segments.length})`}
                </button>
              )}
            </div>

            {exporting && progress && (
              <div className="export-progress" data-testid="export-progress">
                <div className="export-progress-track">
                  <div
                    className="export-progress-fill"
                    style={{ transform: `scaleX(${Math.max(0, Math.min(1, progress.fraction))})` }}
                  />
                </div>
                <span className="export-progress-label">
                  {progress.phase} · {Math.round(progress.fraction * 100)}%
                </span>
              </div>
            )}

            {status && (
              <div
                className={
                  status.startsWith("Export failed")
                    ? "msg-error"
                    : /cancel/i.test(status)
                      ? "msg-warn"
                      : "msg-ok"
                }
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
