import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { checkRootDirStatus, checkPathWritable, completeSetup, type SetupResult } from "./api";
import { canContinueFromRootStep, canFinishIngestionStep, type IngestionMode } from "./wizardValidation";
import "./FirstRunWizard.css";

interface FirstRunWizardProps {
  onComplete: (result: SetupResult) => void;
}

export function FirstRunWizard({ onComplete }: FirstRunWizardProps) {
  const [step, setStep] = useState<0 | 1 | 2>(0);
  const [rootPath, setRootPath] = useState<string | null>(null);
  const [rootNonEmptyWarning, setRootNonEmptyWarning] = useState(false);
  const [rootWritable, setRootWritable] = useState(true);
  const [mode, setMode] = useState<IngestionMode | null>(null);
  const [sourceFolder, setSourceFolder] = useState<string | null>(null);
  const [finishing, setFinishing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function pickRoot() {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return;
    setRootPath(selected);
    try {
      const empty = await checkRootDirStatus(selected);
      setRootNonEmptyWarning(!empty);
    } catch {
      setRootNonEmptyWarning(false);
    }
    try {
      setRootWritable(await checkPathWritable(selected));
    } catch {
      setRootWritable(false);
    }
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
      const result = await completeSetup(rootPath, mode, sourceFolder);
      onComplete(result);
    } catch (e) {
      setError(String(e));
      setFinishing(false);
    }
  }

  return (
    <div className="wiz-stage">
      <div className="wiz-blob wiz-blob-1" />
      <div className="wiz-blob wiz-blob-2" />
      <div className="wiz-blob wiz-blob-3" />

      <div className="wiz-panel">
        <div className="wiz-dots">
          <div className={`wiz-dot${step === 0 ? " active" : ""}`} />
          <div className={`wiz-dot${step === 1 ? " active" : ""}`} />
          <div className={`wiz-dot${step === 2 ? " active" : ""}`} />
        </div>

        {step === 0 && (
          <>
            <div className="wiz-title">Welcome to ClipSnip</div>
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
            <div className="wiz-title">Where should ClipSnip live?</div>
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
                This folder isn't empty — ClipSnip will add Clips/Exports/Cache
                subfolders here.
              </div>
            )}
            {!rootWritable && (
              <div className="wiz-warning wiz-error">
                ClipSnip can't write to this folder — choose a different one.
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
            <div className="wiz-title">How should ClipSnip get your clips?</div>
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
                  <div className="wiz-mode-desc">Copy clips from a folder into ClipSnip now.</div>
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
                {finishing ? "Setting up…" : "Finish"}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
