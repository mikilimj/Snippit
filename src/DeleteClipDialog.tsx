import { useState } from "react";
import type { ClipRow } from "./api";

/** How a clip leaves the app: index-only, or file-and-index. */
export type DeleteMode = "index" | "file";

interface Props {
  entry: ClipRow;
  /** False for the app's own `Clips/` copies — the watcher would re-index those. */
  canRemoveOnly: boolean;
  onCancel: () => void;
  onConfirm: (mode: DeleteMode) => void;
}

/**
 * Confirmation for deleting a clip. Nothing is removed until this is confirmed,
 * and a clip is never deleted from disk by accident: the safer "just hide it"
 * option is preselected wherever it is actually offered.
 */
export default function DeleteClipDialog({ entry, canRemoveOnly, onCancel, onConfirm }: Props) {
  const [mode, setMode] = useState<DeleteMode>(canRemoveOnly ? "index" : "file");

  return (
    <div className="modal-backdrop" onMouseDown={e => { if (e.target === e.currentTarget) onCancel(); }}>
      <div className="modal-card delete-card" role="dialog" aria-modal="true" aria-label="Delete clip">
        <div className="modal-header">
          <span className="modal-title">Delete clip</span>
          <button className="modal-close" onClick={onCancel} aria-label="Cancel">
            ×
          </button>
        </div>

        <div className="modal-body delete-body">
          <p className="delete-name" title={entry.path}>{entry.displayName}</p>

          <label className={`delete-option${canRemoveOnly ? "" : " disabled"}`}>
            <input
              type="radio"
              name="delete-mode"
              value="index"
              checked={mode === "index"}
              disabled={!canRemoveOnly}
              onChange={() => setMode("index")}
            />
            <span className="delete-option-text">
              <strong>Remove from Snippit only</strong>
              <span>
                {canRemoveOnly
                  ? "The file stays where it is on disk."
                  : "Not available for clips in your Clips folder — Snippit would just index the file again."}
              </span>
            </span>
          </label>

          <label className="delete-option">
            <input
              type="radio"
              name="delete-mode"
              value="file"
              checked={mode === "file"}
              onChange={() => setMode("file")}
            />
            <span className="delete-option-text">
              <strong>Delete the file from disk</strong>
              <span>Permanent — the video is erased and removed from Snippit.</span>
            </span>
          </label>
        </div>

        <div className="delete-footer">
          <button className="btn-cancel" onClick={onCancel}>Cancel</button>
          <button className="btn-danger" onClick={() => onConfirm(mode)}>
            {mode === "file" ? "Delete file" : "Remove from Snippit"}
          </button>
        </div>
      </div>
    </div>
  );
}
