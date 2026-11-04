import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { ClipRow, Folder } from "./api";
import type { LibraryView } from "./libraryView";

interface Props {
  entry: ClipRow;
  /** Pointer position of the right-click that opened the menu. */
  x: number;
  y: number;
  folders: Folder[];
  activeView: LibraryView;
  storageProvider: string | null;
  syncedToServer: boolean;
  onClose: () => void;
  onToggleFavorite: () => void;
  onAddToFolder: (folderId: number) => void;
  onRemoveFromCurrentFolder: () => void;
  onDelete: () => void;
  onDeleteFromServer: () => void;
}

/**
 * Right-click menu for a grid card. It replaces the old hover overlay, so every
 * per-clip action lives in one place: favourites first, then the folder list,
 * then the destructive actions at the bottom.
 */
export default function ClipContextMenu({
  entry,
  x,
  y,
  folders,
  activeView,
  storageProvider,
  syncedToServer,
  onClose,
  onToggleFavorite,
  onAddToFolder,
  onRemoveFromCurrentFolder,
  onDelete,
  onDeleteFromServer,
}: Props) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ x, y });

  // Keep the menu inside the window when right-clicking near an edge.
  useLayoutEffect(() => {
    const el = menuRef.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setPos({
      x: Math.max(8, Math.min(x, window.innerWidth - width - 8)),
      y: Math.max(8, Math.min(y, window.innerHeight - height - 8)),
    });
  }, [x, y]);

  // Escape dismisses it; the app's own Escape handling must not close the popup
  // behind it, so stop the event here.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      onClose();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  function run(action: () => void) {
    action();
    onClose();
  }

  return (
    <div className="ctx-backdrop" onMouseDown={onClose} onContextMenu={e => e.preventDefault()}>
      <div
        ref={menuRef}
        className="ctx-menu"
        style={{ left: pos.x, top: pos.y }}
        role="menu"
        aria-label={`Actions for ${entry.displayName}`}
        onMouseDown={e => e.stopPropagation()}
      >
        <div className="ctx-title" title={entry.displayName}>{entry.displayName}</div>

        <button className="ctx-item" role="menuitem" onClick={() => run(onToggleFavorite)}>
          <span className={`ctx-icon${entry.favorite ? " on" : ""}`}>★</span>
          {entry.favorite ? "Remove from favourites" : "Add to favourites"}
        </button>

        <div className="ctx-sep" />
        <div className="ctx-label">Add to folder</div>
        {folders.length === 0 ? (
          <div className="ctx-empty">No folders yet</div>
        ) : (
          folders.map(folder => (
            <button
              key={folder.id}
              className="ctx-item"
              role="menuitem"
              onClick={() => run(() => onAddToFolder(folder.id))}
            >
              <span className="ctx-icon">#</span>
              {folder.name}
            </button>
          ))
        )}

        <div className="ctx-sep" />
        {activeView.kind === "folder" && (
          <button className="ctx-item" role="menuitem" onClick={() => run(onRemoveFromCurrentFolder)}>
            <span className="ctx-icon">−</span>
            Remove from this folder
          </button>
        )}

        {storageProvider !== null && (
          <button
            className="ctx-item"
            role="menuitem"
            disabled={!syncedToServer}
            onClick={() => run(onDeleteFromServer)}
          >
            <span className="ctx-icon">☁</span>
            {syncedToServer ? "Delete from server" : "Not uploaded to the server yet"}
          </button>
        )}

        <button className="ctx-item ctx-item-danger" role="menuitem" onClick={() => run(onDelete)}>
          <span className="ctx-icon">✕</span>
          Delete clip…
        </button>
      </div>
    </div>
  );
}
