import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { probeClip, exportTrim, exportCompressed, cancelExport, prepareAudioTracks, prepareVideoOnly, getAppRoot, addToLibrary, listLibrary, removeFromLibrary, rescanLibrary, setClipFavorite, renameLibraryClip, createFolder, listFolders, deleteFolder, addClipToFolder, removeClipFromFolder, listClipsInFolder, listUnsortedLibrary, ensureProxy, type ClipMeta, type ClipRow, type ExportMode, type ExportProgress, type Folder, type TrackConfig, type SetupResult } from "./api";
import { exportSequence, type MusicOverlay } from "./api";
import { newClipBelongsInView, applyFavorite, applyRename, type LibraryView } from "./libraryView";
import { previewPath, shouldTryProxy } from "./preview";
import { ChunkedAudioEngine } from "./audioEngine";
import { moveSegment, removeSegment, sequenceDuration, toBackendSegments, type SequenceSegment } from "./sequence";
import { isCustomSelectionValid, resolveTargetMb, TARGET_SIZE_PRESETS } from "./exportOptions";
import { setIn, setOut } from "./trim";
import { Timeline } from "./Timeline";
import { useResizable } from "./useResizable";
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
  const sidebar = useResizable({
    axis: "x",
    defaultSize: 224,
    min: 184,
    max: 480,
    storageKey: "clipsnip:sidebarWidth",
  });
  const videoHeight = useResizable({
    axis: "y",
    defaultSize: 50,
    min: 25,
    max: 85,
    storageKey: "clipsnip:videoMaxHeightVh",
  });
  // `undefined` = still checking, `null` = needs first-run setup, string = configured root
  const [appRoot, setAppRoot] = useState<string | null | undefined>(undefined);

  // ── Multi-track audio state ──────────────────────────────────────────────────
  const [trackConfigs, setTrackConfigs] = useState<TrackConfig[]>([]);
  const [audioTrackPaths, setAudioTrackPaths] = useState<string[]>([]);
  const [audioReady, setAudioReady] = useState(false);
  const [audioPreparingMsg, setAudioPreparingMsg] = useState<string | null>(null);
  // Video-only remux of the active preview source; loaded once the audio engine
  // owns playback so the <video> stops registering an OS audio stream.
  const [videoOnlyPath, setVideoOnlyPath] = useState<string | null>(null);
  // Remux result waiting for the engine to take over — built eagerly at probe
  // time so the swap can happen the instant audio is ready.
  const [preparedVideoOnly, setPreparedVideoOnly] = useState<string | null>(null);

  // Refs that live outside React's render cycle
  const trackConfigsRef = useRef<TrackConfig[]>([]);
  const audioCtxRef = useRef<AudioContext | null>(null);
  const engineRef = useRef<ChunkedAudioEngine | null>(null);
  // Playback position/state to restore after the video-only source swap remounts
  // the <video> element.
  const restoreAfterSwapRef = useRef<{ time: number; paused: boolean } | null>(null);
  // A video-only remux that failed to play is never retried for this clip.
  const videoOnlyFailedRef = useRef(false);
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

  function refreshFolders() {
    listFolders()
      .then(setFolders)
      .catch(err => console.error("Failed to load folders:", err));
  }

  // Load the virtual folders once setup is configured.
  useEffect(() => {
    if (typeof appRoot !== "string") return;
    refreshFolders();
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
      refreshFolders();
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

  // ── Tear down the chunked audio engine ────────────────────────────────────────
  // Deterministic, not GC-driven: the engine holds decoded PCM buffers and live
  // AudioBufferSourceNodes; dispose stops and disconnects everything at once.
  // The AudioContext itself is kept for the app's lifetime — it is the ONE audio
  // stream the app registers with the OS mixer, and recreating it would make the
  // app flicker in and out of the system volume panel.
  function teardownAudio() {
    engineRef.current?.dispose();
    engineRef.current = null;
  }

  // ── (Re-)build the audio engine whenever extracted track paths arrive ─────────
  // Track audio never touches an <audio> element: on WebKitGTK every media
  // element registers its own (muted) system audio sink even when captured into
  // Web Audio, which made each track show up as a separate stream in the OS
  // mixer. The engine schedules ffmpeg-decoded PCM chunks on the shared
  // AudioContext instead.
  useEffect(() => {
    if (audioTrackPaths.length === 0) return;

    teardownAudio();

    if (!audioCtxRef.current || audioCtxRef.current.state === "closed") {
      audioCtxRef.current = new AudioContext();
    }
    const ctx = audioCtxRef.current;

    const initialGains = audioTrackPaths.map((_, i) => {
      const cfg = trackConfigsRef.current[i];
      return (cfg?.enabled ?? true) ? (cfg?.volume ?? 1) : 0;
    });
    const engine = new ChunkedAudioEngine(
      ctx,
      audioTrackPaths,
      meta?.durationSecs ?? 0,
      initialGains
    );
    engineRef.current = engine;

    // If video is already playing when tracks become ready, start in sync
    const v = videoRef.current;
    if (v && !v.paused) {
      engine.play(v.currentTime);
    }

    setAudioReady(true);

    return teardownAudio;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [audioTrackPaths]);

  // ── Sync the engine when video plays / pauses / seeks ─────────────────────────
  function onVideoPlay() {
    const v = videoRef.current;
    if (v) engineRef.current?.play(v.currentTime);
  }

  // pause() stops the scheduled sources; the AudioContext keeps running so the
  // app's one OS audio stream survives pause/play cycles (suspending it makes
  // WebKitGTK register a fresh sink-input on every resume).
  function onVideoPause() {
    engineRef.current?.pause();
  }

  // Periodic drift/stall correction, driven by the video's timeupdate (a few
  // times a second): re-anchors the engine if it drifts off the video clock and
  // tops up the chunk schedule. Deliberately no setState — playback must not
  // re-render the app.
  function onVideoTimeUpdate() {
    const v = videoRef.current;
    if (v) engineRef.current?.tick(v.currentTime, v.paused);
  }

  // While the video rebuffers, park the audio so it doesn't run ahead;
  // onPlaying (→ onVideoPlay) resyncs and resumes it.
  function onVideoWaiting() {
    engineRef.current?.pause();
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
    if (v) engineRef.current?.seek(v.currentTime);
  }

  // The video-only swap remounts the <video> (its key is the source path);
  // land the fresh element where playback left off. When the engine kept
  // playing through the swap, its clock — not the pre-swap snapshot — is where
  // the audio is now, so resuming there keeps the transition seamless.
  function onVideoLoadedMetadata() {
    const r = restoreAfterSwapRef.current;
    const v = videoRef.current;
    if (!r || !v) return;
    restoreAfterSwapRef.current = null;
    v.currentTime = r.paused ? r.time : engineRef.current?.currentMediaTime() ?? r.time;
    // If resuming is refused, park the engine too — otherwise audio would keep
    // running against a frozen video with no timeupdate ticks to correct it.
    if (!r.paused) v.play().catch(() => engineRef.current?.pause());
  }

  // Native-first, proxy-fallback (spec §4.4): if the WebView can't decode the
  // original, transcode a 720p H.264 proxy in the background and switch to it.
  // If the proxy itself fails, surface the error instead of looping.
  function handleVideoError(e: React.SyntheticEvent<HTMLVideoElement>) {
    const err = e.currentTarget.error;
    // A failing video-only remux is not a codec problem — the same streams just
    // played from the previous source. Drop back to it and don't retry.
    if (videoOnlyPath) {
      videoOnlyFailedRef.current = true;
      // If the remux died before loadedmetadata, the pre-swap snapshot is still
      // pending — keep it rather than the dead element's zeroed clock.
      if (!restoreAfterSwapRef.current) {
        const v = videoRef.current;
        restoreAfterSwapRef.current = v ? { time: v.currentTime, paused: v.paused } : null;
      }
      setVideoOnlyPath(null);
      return;
    }
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

  // ── Per-track config update: applies immediately to the engine's gain node ────
  function updateTrackConfig(i: number, patch: Partial<TrackConfig>) {
    setTrackConfigs(prev => {
      const next = prev.map((c, idx) => idx === i ? { ...c, ...patch } : c);
      const cfg = next[i];
      engineRef.current?.setTrackGain(i, cfg.enabled ? cfg.volume : 0);
      return next;
    });
  }

  // ── Video-only source swap ─────────────────────────────────────────────────────
  // A file with no audio track never gets an audio pipeline in the WebView, so
  // a remuxed <video> source stops registering its own (muted) stream with the
  // OS mixer — leaving the shared AudioContext as the app's single stream on
  // Linux and Windows alike. The remux starts as soon as the clip is probed
  // (it's a stream copy, usually near-instant) so the window where the original
  // — with its audio pipeline — is on screen stays as short as possible.
  useEffect(() => {
    if (!path || !meta || meta.audioTracks.length === 0 || videoOnlyFailedRef.current) return;
    const source = proxyPath ?? path;
    let cancelled = false;
    prepareVideoOnly(source)
      .then(vo => {
        if (cancelled || videoOnlyFailedRef.current) return;
        setPreparedVideoOnly(vo);
      })
      .catch(err => {
        // Not fatal: playback continues from the muted original.
        console.error("Video-only remux failed:", err);
      });
    return () => {
      cancelled = true;
    };
  }, [meta, path, proxyPath]);

  // Apply the prepared remux only once the engine owns audio — swapping earlier
  // would silence the clip (the remux has no audio to fall back on).
  useEffect(() => {
    if (!preparedVideoOnly || !audioReady || videoOnlyFailedRef.current) return;
    const v = videoRef.current;
    restoreAfterSwapRef.current = v ? { time: v.currentTime, paused: v.paused } : null;
    setVideoOnlyPath(preparedVideoOnly);
  }, [preparedVideoOnly, audioReady]);

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
      {
        id: Date.now(),
        path,
        displayName: basename(path),
        inSecs: inPoint,
        outSecs: outPoint,
        // Snapshot the audio mix as previewed right now — the export must include
        // every enabled track, not just the file's first one.
        tracks: trackConfigs.map(c => ({ ...c })),
      },
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
    // The same Size dropdown drives sequences: "Original quality" = CRF encode,
    // a size target = bitrate encode of the whole stitched sequence.
    if (!isCustomSelectionValid(sizeSelection, customMb)) {
      setStatus("Enter a valid custom size in MB first.");
      return;
    }
    const targetMb = resolveTargetMb(sizeSelection, customMb);
    const dest = await save({ defaultPath: "sequence.mp4" });
    if (!dest) return;
    setExporting(true);
    setProgress({ fraction: 0, phase: targetMb === null ? "Stitching" : "Analyzing" });
    setStatus(
      targetMb === null
        ? `Stitching ${segments.length} clip${segments.length > 1 ? "s" : ""}…`
        : `Stitching ${segments.length} clip${segments.length > 1 ? "s" : ""} into ${targetMb} MB…`
    );
    try {
      const out = await exportSequence(toBackendSegments(segments), music, dest, targetMb);
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
    // The <video> is keyed by path, so React replaces the element — but the old
    // element's GStreamer pipeline (and its HTTP connection) would linger until
    // GC. Kill it now, same reasoning as teardownAudio. Skipped when re-opening
    // the active clip: the key doesn't change, so the element lives on.
    const v = videoRef.current;
    if (v && p !== path) {
      v.pause();
      v.removeAttribute("src");
      v.load();
    }

    setError(null);
    setVideoError(null);
    setProxyPath(null);
    setProxyStatus(null);
    setPath(p);
    setInPoint(null);
    setOutPoint(null);
    setStatus(null);

    // Reset audio state for the new clip
    setAudioReady(false);
    setAudioTrackPaths([]);
    setTrackConfigs([]);
    setAudioPreparingMsg(null);
    setVideoOnlyPath(null);
    setPreparedVideoOnly(null);
    videoOnlyFailedRef.current = false;
    restoreAfterSwapRef.current = null;
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
      <aside className="library-panel" style={{ width: sidebar.size }}>
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
                    {entry.favorite && <span className="fav-inline">★ </span>}
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

      <div
        className="resize-handle-x"
        onPointerDown={sidebar.onPointerDown}
        onPointerMove={sidebar.onPointerMove}
        onPointerUp={sidebar.onPointerUp}
        onPointerCancel={sidebar.onPointerUp}
      />

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
            {/* Video: original first, proxy fallback (§4.4), then the video-only
                remux once the audio engine has taken over; muted in the interim */}
            <video
              ref={videoRef}
              key={previewPath(path, proxyPath, videoOnlyPath)}
              src={mediaPort ? `http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(previewPath(path, proxyPath, videoOnlyPath))}` : undefined}
              controls
              muted={audioReady}
              className="video-player"
              style={{ maxHeight: `${videoHeight.size}vh` }}
              onLoadedMetadata={onVideoLoadedMetadata}
              onTimeUpdate={onVideoTimeUpdate}
              onPlay={onVideoPlay}
              onPlaying={onVideoPlay}
              onWaiting={onVideoWaiting}
              onPause={onVideoPause}
              onSeeked={onVideoSeeked}
              onVolumeChange={onVideoVolumeChange}
              onError={handleVideoError}
            />

            <div
              className="resize-handle-y"
              onPointerDown={videoHeight.onPointerDown}
              onPointerMove={videoHeight.onPointerMove}
              onPointerUp={videoHeight.onPointerUp}
              onPointerCancel={videoHeight.onPointerUp}
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
                videoRef={videoRef}
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
                <button
                  className="btn-export"
                  onClick={exportSequenceClick}
                  disabled={exporting}
                  title="Uses the Size selector above: Original quality, or compress the whole sequence to the chosen size"
                >
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
