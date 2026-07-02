import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  getWatchedFolder,
  relocateWatchedFolder,
  cancelRelocate,
  type ExportProgress,
} from "./api";

/**
 * Settings surface for folder relocation (spec §4.7). v1 exposes moving the
 * watched source folder to a new path/disk with a cancellable progress bar.
 */
export function SettingsModal({ onClose }: { onClose: () => void }) {
  const [watched, setWatched] = useState<string | null | undefined>(undefined);
  const [moving, setMoving] = useState(false);
  const [progress, setProgress] = useState<ExportProgress | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    getWatchedFolder()
      .then(setWatched)
      .catch(() => setWatched(null));
  }, []);

  useEffect(() => {
    const unlisten = listen<ExportProgress>("relocate-progress", e => setProgress(e.payload));
    return () => {
      unlisten.then(f => f());
    };
  }, []);

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

          {moving && progress && (
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
          )}

          {message && (
            <div className={/fail/i.test(message) ? "msg-error" : "msg-ok"}>{message}</div>
          )}
        </div>
      </div>
    </div>
  );
}
