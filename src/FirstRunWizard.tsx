import { useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { join, videoDir } from "@tauri-apps/api/path";
import {
  checkRootDirStatus,
  checkPathWritable,
  completeSetup,
  storageScanDiff,
  storageImportClips,
  type SetupResult,
  type SyncDiffResult,
} from "./api";
import { canContinueFromRootStep, canFinishIngestionStep, canFinishRestoreStep, type IngestionMode } from "./wizardValidation";
import { displayNameFor, formatSize } from "./syncDiff";
import { StorageConnectForm } from "./StorageSettings";
import logo from "./assets/logo.png";
import "./FirstRunWizard.css";

interface FirstRunWizardProps {
  onComplete: (result: SetupResult) => void;
}

export function FirstRunWizard({ onComplete }: FirstRunWizardProps) {
  const [step, setStep] = useState<0 | 1 | 2 | 3>(0);
  const [rootPath, setRootPath] = useState<string | null>(null);
  const [rootNonEmptyWarning, setRootNonEmptyWarning] = useState(false);
  const [rootWritable, setRootWritable] = useState(true);
  const [mode, setMode] = useState<IngestionMode | null>(null);
  const [sourceFolder, setSourceFolder] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Restore-mode step 3: setup already completed (as "empty"), now connect a
  // remote and pull the clips down before entering the app.
  const [setupResult, setSetupResult] = useState<SetupResult | null>(null);
  const [connected, setConnected] = useState(false);
  const [remoteDiff, setRemoteDiff] = useState<SyncDiffResult | null>(null);
  const [diffError, setDiffError] = useState<string | null>(null);
  const [restoreSelected, setRestoreSelected] = useState<Set<string>>(new Set());
  const [importing, setImporting] = useState(false);

  async function checkRootPath(path: string) {
    try {
      const empty = await checkRootDirStatus(path);
      setRootNonEmptyWarning(!empty);
    } catch {
      setRootNonEmptyWarning(false);
    }
    try {
      setRootWritable(await checkPathWritable(path));
    } catch {
      setRootWritable(false);
    }
  }

  // Suggest <Videos>/ClipSnip as a starting point so users who don't care to
  // browse can just hit Continue; "Choose folder" still overrides it.
  useEffect(() => {
    (async () => {
      try {
        const suggested = await join(await videoDir(), "Snippit");
        setRootPath(suggested);
        await checkRootPath(suggested);
      } catch {
        // No Videos dir resolvable (e.g. unset XDG_VIDEOS_DIR) — fall back to
        // requiring a manual pick, same as before this default existed.
      }
    })();
  }, []);

  async function pickRoot() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    setRootPath(selected);
    await checkRootPath(selected);
  }

  async function pickSourceFolder() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    setSourceFolder(selected);
  }

  async function finish() {
    if (!rootPath || !mode) return;
    setFinishing(true);
    setError(null);
    try {
      // Restore mode sets up an empty library first; the clips arrive from the
      // remote in the extra step that follows.
      const result = await completeSetup(rootPath, mode === "restore" ? "empty" : mode, sourceFolder);
      if (mode === "restore") {
        setSetupResult(result);
        setFinishing(false);
        setStep(3);
        return;
      }
      onComplete(result);
    } catch (e) {
      setError(String(e));
      setFinishing(false);
    }
  }

  function scanRemote() {
    setRemoteDiff(null);
    setDiffError(null);
    storageScanDiff()
      .then(d => {
        setRemoteDiff(d);
        // Everything pre-checked: restoring wants the whole library by default.
        setRestoreSelected(new Set(d.remoteOnly.map(f => f.path)));
      })
      .catch(e => setDiffError(String(e)));
  }

  function toggleRestoreRow(rel: string) {
    setRestoreSelected(prev => {
      const next = new Set(prev);
      if (next.has(rel)) next.delete(rel);
      else next.add(rel);
      return next;
    });
  }

  async function finishRestore() {
    if (!setupResult) return;
    setImporting(true);
    setError(null);
    try {
      if (restoreSelected.size > 0) {
        await storageImportClips(Array.from(restoreSelected));
      }
      onComplete(setupResult);
    } catch (e) {
      setError(String(e));
      setImporting(false);
    }
  }

  return (
    <div className="wiz-stage">
      <div className="aurora-blob aurora-blob-1" />
      <div className="aurora-blob aurora-blob-2" />
      <div className="aurora-blob aurora-blob-3" />

      <div className="wiz-panel">
        <div className="wiz-dots">
          <div className={`wiz-dot${step === 0 ? " active" : ""}`} />
          <div className={`wiz-dot${step === 1 ? " active" : ""}`} />
          <div className={`wiz-dot${step === 2 ? " active" : ""}`} />
          {mode === "restore" && <div className={`wiz-dot${step === 3 ? " active" : ""}`} />}
        </div>

        {step === 0 && (
          <>
            <img className="wiz-logo" src={logo} alt="" />
            <div className="wiz-title">Welcome to Snippit</div>
            <div className="wiz-desc">
              Fast trimming for your game clips. Let's set up where your clips and
              exports will live.
            </div>
            <div className="wiz-nav wiz-nav-single">
              <button className="wiz-next" onClick={() => setStep(1)}>Get Started</button>
            </div>
          </>
        )}

        {step === 1 && (
          <>
            <div className="wiz-title">Where should Snippit live?</div>
            <div className="wiz-desc">
              Pick a folder to store your clips, exports, and settings. You can change
              this later.
            </div>
            <div className="wiz-pickbox">
              <span className="wiz-path">{rootPath ?? "No folder chosen"}</span>
              <button className="wiz-btn-choose" onClick={pickRoot}>Choose folder</button>
            </div>
            {rootNonEmptyWarning && (
              <div className="wiz-warning">
                This folder isn't empty — Snippit will add Clips/Exports/Cache
                subfolders here.
              </div>
            )}
            {!rootWritable && (
              <div className="wiz-warning wiz-error">
                Snippit can't write to this folder — choose a different one.
              </div>
            )}
            <div className="wiz-nav">
              <button className="wiz-back" onClick={() => setStep(0)}>&larr; Back</button>
              <button
                className="wiz-next"
                disabled={!canContinueFromRootStep(rootPath) || !rootWritable}
                onClick={() => setStep(2)}
              >
                Continue
              </button>
            </div>
          </>
        )}

        {step === 2 && (
          <>
            <div className="wiz-title">How should Snippit get your clips?</div>
            <div className="wiz-desc">Choose one — you can change this later.</div>
            <div className="wiz-modes">
              <label className={`wiz-mode-option${mode === "import" ? " selected" : ""}`}>
                <input
                  type="radio"
                  name="ingestion-mode"
                  checked={mode === "import"}
                  onChange={() => setMode("import")}
                />
                <div>
                  <div className="wiz-mode-title">Import existing clips</div>
                  <div className="wiz-mode-desc">Copy clips from a folder into Snippit now.</div>
                </div>
              </label>
              <label className={`wiz-mode-option${mode === "empty" ? " selected" : ""}`}>
                <input
                  type="radio"
                  name="ingestion-mode"
                  checked={mode === "empty"}
                  onChange={() => setMode("empty")}
                />
                <div>
                  <div className="wiz-mode-title">Start empty</div>
                  <div className="wiz-mode-desc">Add clips later from the editor.</div>
                </div>
              </label>
              <label className={`wiz-mode-option${mode === "watch" ? " selected" : ""}`}>
                <input
                  type="radio"
                  name="ingestion-mode"
                  checked={mode === "watch"}
                  onChange={() => setMode("watch")}
                />
                <div>
                  <div className="wiz-mode-title">Watch a folder</div>
                  <div className="wiz-mode-desc">Remember a recorder's output folder for later.</div>
                </div>
              </label>
              <label className={`wiz-mode-option${mode === "restore" ? " selected" : ""}`}>
                <input
                  type="radio"
                  name="ingestion-mode"
                  checked={mode === "restore"}
                  onChange={() => setMode("restore")}
                />
                <div>
                  <div className="wiz-mode-title">Restore from server or cloud</div>
                  <div className="wiz-mode-desc">Download a synced library from SFTP, Dropbox or Google Drive.</div>
                </div>
              </label>
            </div>

            {(mode === "import" || mode === "watch") && (
              <div className="wiz-pickbox">
                <span className="wiz-path">{sourceFolder ?? "No folder chosen"}</span>
                <button className="wiz-btn-choose" onClick={pickSourceFolder}>Choose folder</button>
              </div>
            )}

            {error && <div className="wiz-warning wiz-error">{error}</div>}

            <div className="wiz-nav">
              <button className="wiz-back" onClick={() => setStep(1)} disabled={finishing}>&larr; Back</button>
              <button
                className="wiz-next"
                disabled={!canFinishIngestionStep(mode, sourceFolder) || finishing}
                onClick={finish}
              >
                {finishing ? "Setting up…" : mode === "restore" ? "Continue" : "Finish"}
              </button>
            </div>
          </>
        )}

        {step === 3 && (
          <>
            <div className="wiz-title">Restore from server or cloud</div>
            {!connected ? (
              <>
                <div className="wiz-desc">
                  Connect the storage that holds your synced library.
                </div>
                <StorageConnectForm
                  onConnected={() => {
                    setConnected(true);
                    scanRemote();
                  }}
                />
              </>
            ) : (
              <>
                <div className="wiz-desc">
                  Pick the clips to download into your new library.
                </div>
                {diffError !== null ? (
                  <>
                    <div className="wiz-warning wiz-error">{diffError}</div>
                    <div className="wiz-nav wiz-nav-single">
                      <button className="wiz-next" onClick={scanRemote}>Retry</button>
                    </div>
                  </>
                ) : remoteDiff === null ? (
                  <div className="wiz-desc">Scanning the remote library…</div>
                ) : remoteDiff.remoteOnly.length === 0 ? (
                  <div className="wiz-warning">Nothing to restore — the remote library is empty.</div>
                ) : (
                  <ul className="wiz-restore-list">
                    {remoteDiff.remoteOnly.map(f => (
                      <li key={f.path}>
                        <label className="wiz-restore-row">
                          <input
                            type="checkbox"
                            checked={restoreSelected.has(f.path)}
                            onChange={() => toggleRestoreRow(f.path)}
                          />
                          <span className="wiz-restore-name" title={f.path}>
                            {displayNameFor(f.path, null)}
                          </span>
                          <span className="wiz-restore-size">{formatSize(f.size)}</span>
                        </label>
                      </li>
                    ))}
                  </ul>
                )}
              </>
            )}

            {error && <div className="wiz-warning wiz-error">{error}</div>}

            {connected && diffError === null && (
              <div className="wiz-nav wiz-nav-single">
                <button
                  className="wiz-next"
                  disabled={!canFinishRestoreStep(connected) || remoteDiff === null || importing}
                  onClick={finishRestore}
                >
                  {importing
                    ? "Importing…"
                    : restoreSelected.size > 0
                      ? `Import ${restoreSelected.size} clip${restoreSelected.size === 1 ? "" : "s"} & finish`
                      : "Finish without importing"}
                </button>
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
