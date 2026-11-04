import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { probeClip, exportTrim, exportCompressed, cancelExport, prepareAudioTracks, prepareVideoOnly, getAppRoot, addToLibrary, listLibrary, removeFromLibrary, rescanLibrary, setClipFavorite, renameLibraryClip, createFolder, listFolders, deleteFolder, addClipToFolder, removeClipFromFolder, listClipsInFolder, listUnsortedLibrary, ensureProxy, storageStatus, storageSyncedRelNames, storageDeleteRemote, type ClipMeta, type ClipRow, type ExportMode, type ExportProgress, type Folder, type TrackConfig, type SetupResult } from "./api";
import { exportSequence, type MusicOverlay } from "./api";
import { newClipBelongsInView, applyFavorite, applyRename, upsertByCapture, type LibraryView } from "./libraryView";
import { previewPath, shouldTryProxy } from "./preview";
import { ChunkedAudioEngine } from "./audioEngine";
import { moveSegment, removeSegment, sequenceDuration, toBackendSegments, type SequenceSegment } from "./sequence";
import { isCustomSelectionValid, resolveTargetMb, TARGET_SIZE_PRESETS } from "./exportOptions";
import { setIn, setOut } from "./trim";
import { Timeline } from "./Timeline";
import { PlayerControls } from "./PlayerControls";
import { useResizable } from "./useResizable";
import { FirstRunWizard } from "./FirstRunWizard";
import { SettingsModal } from "./SettingsModal";
import { SyncDiffModal } from "./SyncDiffModal";
import LibraryGrid from "./LibraryGrid";
import "./App.css";

function basename(p: string) {
  return p.split(/[\\/]/).pop() ?? p;
}

// A clip's rel-name: its path relative to `<root>/Clips/`, `/`-separated — the
// key sync_state stores. null when the clip lives outside the Clips folder.
function clipRelName(appRoot: string | null | undefined, path: string): string | null {
  if (typeof appRoot !== "string") return null;
  const norm = path.replace(/\\/g, "/");
  const prefix = `${appRoot.replace(/\\/g, "/").replace(/\/$/, "")}/Clips/`;
  return norm.startsWith(prefix) ? norm.slice(prefix.length) : null;
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
  const [diffOpen, setDiffOpen] = useState(false);
  const [storageProvider, setStorageProvider] = useState<string | null>(null);
  const [syncedRels, setSyncedRels] = useState<Set<string>>(new Set());
  // Playback state for the transport bar. Toggled on play/pause only, so
  // playback never re-renders the app.
  const [playing, setPlaying] = useState(false);
  // Master preview volume/mute: one multiply on top of the per-track mix.
  const [masterVolume, setMasterVolume] = useState(1);
  const [masterMuted, setMasterMuted] = useState(false);
  const videoHeight = useResizable({
    axis: "y",
    defaultSize: 50,
    min: 25,
    max: 85,
    storageKey: "snippit:videoMaxHeightVh",
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
  // Master preview gain, mirrored in a ref so a freshly built engine picks up
  // the current bar setting without rebuilding on every volume change.
  const masterGainRef = useRef(1);
  // Playback position/state to restore after the video-only source swap remounts
  // the <video> element.
  const restoreAfterSwapRef = useRef<{ time: number; paused: boolean } | null>(null);
  // A video-only remux that failed to play is never retried for this clip.
  const videoOnlyFailedRef = useRef(false);

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
      setLibrary(prev => upsertByCapture(prev, row));
    });
    return () => {
      unlisten.then(f => f());
    };
  }, [appRoot, activeView]);

  // "Delete from server" enablement: which provider is connected, and which
  // clips sync_state records as uploaded. Refreshed on every sync-status
  // transition, since finished uploads grow the synced set.
  useEffect(() => {
    if (typeof appRoot !== "string") return;
    let stale = false;
    const refresh = () => {
      storageStatus()
        .then(s => {
          if (!stale) setStorageProvider(s.provider);
        })
        .catch(() => {});
      storageSyncedRelNames()
        .then(rels => {
          if (!stale) setSyncedRels(new Set(rels));
        })
        .catch(() => {});
    };
    refresh();
    const unlisten = listen("sync-status", refresh);
    return () => {
      stale = true;
      unlisten.then(f => f());
    };
  }, [appRoot]);

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
  // The transport's mute has to win too: before the engine is ready the element
  // is still the one making sound, and it has no volume of its own to scale.
  useEffect(() => {
    if (videoRef.current) videoRef.current.muted = audioReady || masterMuted;
  }, [audioReady, masterMuted]);

  // Apply the transport bar's master volume to the live engine (and remember it
  // for engines built later, i.e. the next clip).
  useEffect(() => {
    const gain = masterMuted ? 0 : masterVolume;
    masterGainRef.current = gain;
    engineRef.current?.setMasterGain(gain);
  }, [masterVolume, masterMuted]);

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
    engine.setMasterGain(masterGainRef.current);

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
    setPlaying(true);
    const v = videoRef.current;
    if (v) engineRef.current?.play(v.currentTime);
  }

  // pause() stops the scheduled sources; the AudioContext keeps running so the
  // app's one OS audio stream survives pause/play cycles (suspending it makes
  // WebKitGTK register a fresh sink-input on every resume).
  function onVideoPause() {
    setPlaying(false);
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

  // The element has no native controls any more, but a volume-change can still
  // come from elsewhere (OS media keys, a stale attribute on a swapped source).
  // Unmuting it would play its own decoded audio alongside the Web Audio mix,
  // where no slider can reach it — so once Web Audio owns playback, force it back.
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

  // Trim slider handles: clamp against the opposite point, then jump the
  // playhead there so the picture follows the cut in real time while dragging.
  function changeIn(time: number) {
    const next = setIn(time, outPoint);
    setInPoint(next);
    seekTo(next);
  }

  function changeOut(time: number) {
    if (meta === null) return;
    const next = setOut(time, inPoint, meta.durationSecs);
    setOutPoint(next);
    seekTo(next);
  }

  // ── Transport bar ─────────────────────────────────────────────────────────────
  function togglePlay() {
    const v = videoRef.current;
    if (!v) return;
    if (v.paused) {
      v.play().catch(err => console.error("Playback failed:", err));
    } else {
      v.pause();
    }
  }

  // Dragging the volume slider up is also how you get out of mute.
  function setPreviewVolume(volume: number) {
    setMasterVolume(volume);
    if (volume > 0 && masterMuted) setMasterMuted(false);
  }

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.target as HTMLElement).tagName === "INPUT") return;
      if (e.key === "Escape" && path) {
        closeEditor();
        return;
      }
      if (e.key === "i" || e.key === "I") markIn();
      if (e.key === "o" || e.key === "O") markOut();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [inPoint, outPoint, meta, path]);

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
  // Everything transient about the open clip lives here so the popup can be
  // opened and closed without leaking preview/audio state into the next clip.
  function resetPreview() {
    setError(null);
    setVideoError(null);
    setProxyPath(null);
    setProxyStatus(null);
    setInPoint(null);
    setOutPoint(null);
    setStatus(null);
    setPlaying(false);

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
  }

  // Close the player popup: kill the media pipeline first (the <video> is keyed
  // by path, and React would otherwise let its GStreamer pipeline linger), then
  // drop the clip so the popup unmounts and the grid is all that's left.
  function closeEditor() {
    const v = videoRef.current;
    if (v) {
      v.pause();
      v.removeAttribute("src");
      v.load();
    }
    resetPreview();
    setPath(null);
    setMeta(null);
  }

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

    resetPreview();
    setPath(p);
    setMeta(null);

    try {
      const m = await probeClip(p);
      setMeta(m);
      // The trim slider starts out covering the whole clip: from the very first
      // frame to the very last one.
      setInPoint(0);
      setOutPoint(m.durationSecs);
      // Persist to the library index; fall back to an in-session row if the
      // write fails so the clip still shows in the sidebar. A just-opened clip
      // is unsorted, so it only appears in views that show new clips.
      addToLibrary(p, m)
        .then(row => {
          if (!newClipBelongsInView(activeView)) return;
          setLibrary(prev => upsertByCapture(prev, row));
        })
        .catch(err => {
          console.error("Failed to add clip to library:", err);
          if (!newClipBelongsInView(activeView)) return;
          // In-session stand-in for the row the backend would have returned. Its
          // sort key must match the backend's `COALESCE(captured_at, file_mtime)`
          // so the clip lands where a reload would put it.
          const now = Math.floor(Date.now() / 1000);
          const fallback: ClipRow = {
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
            addedAt: now,
            capturedAt: m.capturedAt,
            effectiveCapturedAt: m.capturedAt ?? m.fileMtime ?? now,
          };
          setLibrary(prev =>
            prev.some(e => e.path === p) ? prev : upsertByCapture(prev, fallback),
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

  // One of only two remote-delete paths (the other is the diff window); the
  // local file is never touched.
  async function deleteFromServer(e: React.MouseEvent, entry: ClipRow) {
    e.stopPropagation();
    const rel = clipRelName(appRoot, entry.path);
    if (!rel) return;
    if (!confirm(`Delete "${entry.displayName}" from the server? The local file is untouched.`)) return;
    try {
      await storageDeleteRemote(rel);
      setSyncedRels(prev => {
        const next = new Set(prev);
        next.delete(rel);
        return next;
      });
    } catch (err) {
      console.error("Failed to delete from server:", err);
    }
  }

  async function doRename(id: number, name: string) {
    setLibrary(prev => applyRename(prev, id, name));
    if (id < 0) return;
    try {
      await renameLibraryClip(id, name);
    } catch (err) {
      console.error("Failed to rename clip:", err);
      loadView(activeView);
    }
  }

  // Called by the grid view's inline rename (it has its own editingId state).
  function handleGridRename(id: number, name: string) {
    doRename(id, name);
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

  // ── Render ──────────────────────────────────────────────────────────────────
  if (appRoot === undefined) {
    return null;
  }

  if (appRoot === null) {
    return <FirstRunWizard onComplete={handleSetupComplete} />;
  }

  // The grid is the whole app; a clip opens in a floating editor popup on top of
  // it. Both the "x" in the popup header and a click on the backdrop (the empty
  // space around the card) close it — the header's mousedown is stopped so a
  // drag that starts inside never registers as an outside click.
  return (
    <>
      {settingsOpen && (
        <SettingsModal
          onClose={() => setSettingsOpen(false)}
          onOpenDiff={() => {
            setSettingsOpen(false);
            setDiffOpen(true);
          }}
        />
      )}
      {diffOpen && (
        <SyncDiffModal
          onClose={() => setDiffOpen(false)}
          onImported={() => loadView(activeView)}
          library={library}
          mediaPort={mediaPort}
        />
      )}
      <LibraryGrid
        library={library}
        folders={folders}
        activeView={activeView}
        onSelectView={setActiveView}
        onDeleteFolder={deleteFolderClick}
        creatingFolder={creatingFolder}
        newFolderName={newFolderName}
        onNewFolderNameChange={setNewFolderName}
        onCreateFolder={createFolderClick}
        onCancelCreateFolder={cancelFolderCreation}
        onStartCreateFolder={() => setCreatingFolder(true)}
        onOpenClip={openClip}
        onToggleFavorite={toggleFavorite}
        onAddToFolder={addToFolder}
        onRemoveFromLibrary={removeFromLibraryClick}
        onRemoveFromCurrentFolder={removeFromCurrentFolder}
        onRenameClip={handleGridRename}
        onDeleteFromServer={deleteFromServer}
        mediaPort={mediaPort}
        storageProvider={storageProvider}
        syncedRels={syncedRels}
        appRoot={appRoot}
        onRescan={() => rescanLibrary().catch(err => console.error("Rescan failed:", err))}
        onSettings={() => setSettingsOpen(true)}
        onPickClip={pickClip}
      />

      {path && (
        <div
          className="clip-modal-backdrop"
          onMouseDown={e => {
            if (e.target === e.currentTarget) closeEditor();
          }}
        >
          <div
            className="clip-modal"
            role="dialog"
            aria-modal="true"
            aria-label={`Editing ${basename(path)}`}
            onMouseDown={e => e.stopPropagation()}
          >
            <div className="clip-modal-header">
              <div className="clip-info">
                <span className="clip-name">{basename(path)}</span>
                {meta && (
                  <span className="clip-details">
                    {meta.durationSecs.toFixed(2)}s · {meta.width}×{meta.height} · {meta.videoCodec} · {meta.audioTracks.length}ch
                  </span>
                )}
              </div>
              <button
                className="modal-close clip-modal-close"
                onClick={closeEditor}
                aria-label="Close player"
              >
                ×
              </button>
            </div>

            <div className="clip-modal-body">
              {error && <div className="msg-error">{error}</div>}

              {/* Video: original first, proxy fallback (§4.4), then the video-only
                  remux once the audio engine has taken over; muted in the interim */}
              <video
                ref={videoRef}
                key={previewPath(path, proxyPath, videoOnlyPath)}
                src={mediaPort ? `http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(previewPath(path, proxyPath, videoOnlyPath))}` : undefined}
                muted={audioReady || masterMuted}
                className="video-player"
                style={{ maxHeight: `${videoHeight.size}vh` }}
                onLoadedMetadata={onVideoLoadedMetadata}
                onTimeUpdate={onVideoTimeUpdate}
                onPlay={onVideoPlay}
                onPlaying={onVideoPlay}
                onWaiting={onVideoWaiting}
                onPause={onVideoPause}
                onEnded={onVideoPause}
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

              {/* The one and only playback controller — the <video> has no native
                  controls, so nothing overlays the picture on hover. */}
              <PlayerControls
                videoRef={videoRef}
                duration={meta?.durationSecs ?? 0}
                playing={playing}
                onTogglePlay={togglePlay}
                volume={masterVolume}
                muted={masterMuted}
                onVolume={setPreviewVolume}
                onToggleMute={() => setMasterMuted(m => !m)}
                onSeek={seekTo}
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
                <div className="trim-section">
                  <div className="trim-header">
                    <span className="audio-panel-title">Trim</span>
                    <span className="trim-readout">
                      {inPoint !== null ? inPoint.toFixed(2) + "s" : "—"} → {outPoint !== null ? outPoint.toFixed(2) + "s" : "—"}
                      {inPoint !== null && outPoint !== null
                        ? ` · ${(outPoint - inPoint).toFixed(2)}s`
                        : ""}
                    </span>
                  </div>
                  <Timeline
                    duration={meta.durationSecs}
                    videoRef={videoRef}
                    inPoint={inPoint}
                    outPoint={outPoint}
                    onChangeIn={changeIn}
                    onChangeOut={changeOut}
                  />
                </div>
              )}

              <div className="trim-controls" data-testid="trim-controls">
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
            </div>
          </div>
        </div>
      )}
    </>
  );
}

export default App;
