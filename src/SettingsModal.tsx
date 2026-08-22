import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import {
  getWatchedFolder,
  relocateWatchedFolder,
  cancelRelocate,
  exportLibrary,
  importLibrary,
  cancelBundle,
  listExportDevices,
  getExportDevice,
  setExportDevice,
  type ExportDeviceInfo,
  type ExportProgress,
} from "./api";
import { StorageSection } from "./StorageSettings";

function ProgressBar({ progress }: { progress: ExportProgress }) {
  return (
    <div className="export-progress">
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
  );
}

/**
 * Settings surface for folder relocation (spec §4.7). v1 exposes moving the
 * watched source folder to a new path/disk with a cancellable progress bar.
 */
export function SettingsModal({
  onClose,
  onOpenDiff,
}: {
  onClose: () => void;
  onOpenDiff?: () => void;
}) {
  const [watched, setWatched] = useState<string | null | undefined>(undefined);
  const [moving, setMoving] = useState(false);
  const [progress, setProgress] = useState<ExportProgress | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  // Export encode device (GPU acceleration, 2026-07-05 design)
  const [devices, setDevices] = useState<ExportDeviceInfo[] | null>(null);
  const [device, setDevice] = useState<string>("cpu");
  const [deviceMsg, setDeviceMsg] = useState<string | null>(null);

  // Library bundle (§4.6)
  const [includeFiles, setIncludeFiles] = useState(false);
  const [replace, setReplace] = useState(false);
  const [bundling, setBundling] = useState(false);
  const [bundleProgress, setBundleProgress] = useState<ExportProgress | null>(null);
  const [bundleMsg, setBundleMsg] = useState<string | null>(null);

  useEffect(() => {
    getWatchedFolder()
      .then(setWatched)
      .catch(() => setWatched(null));
    getExportDevice()
      .then(setDevice)
      .catch(() => setDevice("cpu"));
    // Probing runs one tiny ffmpeg encode per candidate on first open; cached after.
    listExportDevices()
      .then(setDevices)
      .catch(err => {
        console.error("Device probe failed:", err);
        setDevices([{ id: "cpu", label: "CPU (software x264)", available: true }]);
      });
  }, []);

  async function chooseDevice(id: string) {
    const previous = device;
    setDevice(id);
    setDeviceMsg(null);
    try {
      await setExportDevice(id);
    } catch (e) {
      setDevice(previous);
      setDeviceMsg(`Couldn't save the device choice: ${e}`);
    }
  }

  useEffect(() => {
    const unRelocate = listen<ExportProgress>("relocate-progress", e => setProgress(e.payload));
    const unBundle = listen<ExportProgress>("bundle-progress", e => setBundleProgress(e.payload));
    return () => {
      unRelocate.then(f => f());
      unBundle.then(f => f());
    };
  }, []);

  async function doExport() {
    const dest = await save({ defaultPath: "snippit-library.zip" });
    if (!dest) return;
    setBundling(true);
    setBundleProgress({ fraction: 0, phase: "Bundling…" });
    setBundleMsg(null);
    try {
      const out = await exportLibrary(dest, includeFiles);
      setBundleMsg(`Exported bundle to ${out}.`);
    } catch (e) {
      const msg = String(e);
      setBundleMsg(/cancel/i.test(msg) ? "Export cancelled." : `Export failed: ${msg}`);
    } finally {
      setBundling(false);
      setBundleProgress(null);
    }
  }

  async function doImport() {
    const bundle = await open({
      multiple: false,
      filters: [{ name: "Bundle", extensions: ["zip"] }],
    });
    if (typeof bundle !== "string") return;
    const newRoot = await open({ directory: true, multiple: false, title: "Where do the clips live?" });
    if (typeof newRoot !== "string") return;
    setBundling(true);
    setBundleProgress({ fraction: 0, phase: "Importing…" });
    setBundleMsg(null);
    try {
      const stats = await importLibrary(bundle, newRoot, replace);
      setBundleMsg(
        `Imported ${stats.added} clip${stats.added === 1 ? "" : "s"} and ${stats.folders} folder${
          stats.folders === 1 ? "" : "s"
        }${stats.missing > 0 ? ` — ${stats.missing} not found at that root` : ""}.`
      );
    } catch (e) {
      const msg = String(e);
      setBundleMsg(/cancel/i.test(msg) ? "Import cancelled." : `Import failed: ${msg}`);
    } finally {
      setBundling(false);
      setBundleProgress(null);
    }
  }

  async function moveTo() {
    const dest = await open({ directory: true, multiple: false });
    if (typeof dest !== "string") return;
    setMoving(true);
    setProgress({ fraction: 0, phase: "Preparing…" });
    setMessage(null);
    try {
      const to = await relocateWatchedFolder(dest);
      setWatched(to);
      setMessage(`Moved to ${to}. The watcher re-targets on next launch.`);
    } catch (e) {
      const msg = String(e);
      setMessage(/cancel/i.test(msg) ? "Move cancelled." : `Move failed: ${msg}`);
    } finally {
      setMoving(false);
      setProgress(null);
    }
  }

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal-card" onClick={e => e.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">Settings</span>
          <button className="modal-close" onClick={onClose} aria-label="Close">×</button>
        </div>

        <div className="modal-body">
          <div className="settings-section">
            <span className="settings-label">Watched folder</span>
            {watched === undefined ? (
              <p className="settings-hint">Loading…</p>
            ) : watched === null ? (
              <p className="settings-hint">
                No watched folder is configured, so there's nothing to relocate here.
              </p>
            ) : (
              <>
                <p className="settings-path" title={watched}>{watched}</p>
                <div className="settings-actions">
                  <button className="btn-sm" onClick={moveTo} disabled={moving}>
                    {moving ? "Moving…" : "Move to…"}
                  </button>
                  {moving && (
                    <button
                      className="btn-cancel"
                      onClick={() => cancelRelocate().catch(() => {})}
                    >
                      Cancel
                    </button>
                  )}
                </div>
                <p className="settings-hint">
                  Same-disk moves are instant; moving to another disk copies file-by-file and is
                  safe to cancel — nothing is deleted until its copy is verified.
                </p>
              </>
            )}

            {moving && progress && <ProgressBar progress={progress} />}

            {message && (
              <div className={/fail/i.test(message) ? "msg-error" : "msg-ok"}>{message}</div>
            )}
          </div>

          <div className="settings-section">
            <span className="settings-label">Export encoding</span>
            <p className="settings-hint">
              Device used to encode exports (Compatible mode, size targets, and sequences).
              Fast lossless copies never re-encode, so they're unaffected. GPU encoding is
              much faster; x264 squeezes slightly better quality into tight size targets.
            </p>
            {devices === null ? (
              <p className="settings-hint">Detecting encoders…</p>
            ) : (
              <select
                className="export-size-select"
                value={device}
                onChange={e => chooseDevice(e.target.value)}
              >
                {devices.map(d => (
                  <option key={d.id} value={d.id} disabled={!d.available}>
                    {d.label}
                    {d.available ? "" : " — not available"}
                  </option>
                ))}
              </select>
            )}
            {deviceMsg && <div className="msg-error">{deviceMsg}</div>}
          </div>

          <div className="settings-section">
            <span className="settings-label">Library bundle</span>
            <p className="settings-hint">
              Move your library to another device: export a portable bundle, then import it there and
              point it at wherever the clips now live. Folders, favorites, and renames carry over.
            </p>
            <label className="music-loop">
              <input
                type="checkbox"
                checked={includeFiles}
                onChange={e => setIncludeFiles(e.target.checked)}
              />
              Include clip files (larger bundle)
            </label>
            <div className="settings-actions">
              <button className="btn-sm" onClick={doExport} disabled={bundling}>
                {bundling ? "Working…" : "Export bundle…"}
              </button>
              <button className="btn-sm" onClick={doImport} disabled={bundling}>
                Import bundle…
              </button>
              {bundling && (
                <button className="btn-cancel" onClick={() => cancelBundle().catch(() => {})}>
                  Cancel
                </button>
              )}
            </div>
            <label className="music-loop">
              <input type="checkbox" checked={replace} onChange={e => setReplace(e.target.checked)} />
              Replace existing library on import (instead of merging)
            </label>

            {bundling && bundleProgress && <ProgressBar progress={bundleProgress} />}

            {bundleMsg && (
              <div className={/fail/i.test(bundleMsg) ? "msg-error" : "msg-ok"}>{bundleMsg}</div>
            )}
          </div>

          <StorageSection onOpenDiff={onOpenDiff} />
        </div>
      </div>
    </div>
  );
}
