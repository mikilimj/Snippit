import { useState, useRef } from "react";
import type { ClipRow, Folder } from "./api";
import type { LibraryView } from "./libraryView";
import { clipRelName } from "./libraryPaths";
import FolderRail from "./FolderRail";
import ClipContextMenu from "./ClipContextMenu";

interface LibraryGridProps {
  library: ClipRow[];
  // Folder rail
  folders: Folder[];
  activeView: LibraryView;
  onSelectView: (view: LibraryView) => void;
  onDeleteFolder: (e: React.MouseEvent, folder: Folder) => void;
  creatingFolder: boolean;
  newFolderName: string;
  onNewFolderNameChange: (name: string) => void;
  onCreateFolder: () => void;
  onCancelCreateFolder: () => void;
  onStartCreateFolder: () => void;
  // Clip actions
  onOpenClip: (path: string) => void;
  onToggleFavorite: (entry: ClipRow) => void;
  onAddToFolder: (clipId: number, folderId: number) => void;
  onRemoveFromCurrentFolder: (clipId: number) => void;
  onRenameClip: (id: number, name: string) => void;
  onDeleteFromServer: (entry: ClipRow) => void;
  /** Opens the delete confirmation (index-only vs. erase the file). */
  onRequestDelete: (entry: ClipRow) => void;
  // Infra
  mediaPort: number | null;
  storageProvider: string | null;
  syncedRels: Set<string>;
  appRoot: string | null | undefined;
  // Toolbar
  onRescan: () => void;
  onSettings: () => void;
  onPickClip: () => void;
}

export default function LibraryGrid({
  library,
  folders,
  activeView,
  onSelectView,
  onDeleteFolder,
  creatingFolder,
  newFolderName,
  onNewFolderNameChange,
  onCreateFolder,
  onCancelCreateFolder,
  onStartCreateFolder,
  onOpenClip,
  onToggleFavorite,
  onAddToFolder,
  onRemoveFromCurrentFolder,
  onRenameClip,
  onDeleteFromServer,
  onRequestDelete,
  mediaPort,
  storageProvider,
  syncedRels,
  appRoot,
  onRescan,
  onSettings,
  onPickClip,
}: LibraryGridProps) {
  const [editingId, setEditingId] = useState<number | null>(null);
  const [editingName, setEditingName] = useState("");
  const renameCancelRef = useRef(false);
  // Card the right-click menu is open for, plus where it was opened.
  const [menu, setMenu] = useState<{ entry: ClipRow; x: number; y: number } | null>(null);

  // Clicking the name turns it into an input (single click: no double-click to
  // discover). The click must not reach the card, which would open the clip.
  function startRename(e: React.MouseEvent, entry: ClipRow) {
    e.stopPropagation();
    renameCancelRef.current = false;
    setEditingId(entry.id);
    setEditingName(entry.displayName);
  }

  function commitRename() {
    if (renameCancelRef.current) {
      renameCancelRef.current = false;
      return;
    }
    const id = editingId;
    if (id === null) return;
    const name = editingName.trim();
    setEditingId(null);
    if (!name) return;
    onRenameClip(id, name);
  }

  return (
    <div className="grid-view">
      <div className="grid-header">
        <div className="grid-toolbar">
          <button
            className="btn-sm"
            title="Re-scan watched folders"
            onClick={onRescan}
          >
            ⟳
          </button>
          <button className="btn-sm" title="Settings" onClick={onSettings}>
            ⚙
          </button>
          <button className="btn-sm" onClick={onPickClip}>+ Open</button>
        </div>
      </div>
      <FolderRail
        folders={folders}
        activeView={activeView}
        onSelectView={onSelectView}
        onDeleteFolder={onDeleteFolder}
        creatingFolder={creatingFolder}
        newFolderName={newFolderName}
        onNewFolderNameChange={onNewFolderNameChange}
        onCreateFolder={onCreateFolder}
        onCancelCreateFolder={onCancelCreateFolder}
        onStartCreateFolder={onStartCreateFolder}
      />
      {library.length === 0 ? (
        <p className="grid-empty">
          {activeView.kind === "all"
            ? "Open a clip to add it here."
            : activeView.kind === "favorites"
              ? "No favorites yet — right-click a clip to add one."
              : activeView.kind === "unsorted"
                ? "Nothing unsorted — every clip is filed away."
                : "No clips in this folder yet."}
        </p>
      ) : (
        <div className="grid-cards">
          {library.map(entry => (
            <div
              key={entry.id}
              className="grid-card"
              onClick={() => onOpenClip(entry.path)}
              onContextMenu={e => {
                e.preventDefault();
                setMenu({ entry, x: e.clientX, y: e.clientY });
              }}
              title={entry.path}
            >
              <div className="grid-thumb-wrap">
                {entry.thumbnailPath && mediaPort ? (
                  <img
                    className="grid-thumb"
                    src={`http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(entry.thumbnailPath)}`}
                    alt=""
                    loading="lazy"
                  />
                ) : (
                  <div className="grid-thumb grid-thumb-empty" />
                )}
                <span className="grid-duration">
                  {entry.durationSecs.toFixed(1)}s
                </span>
                {entry.favorite && <span className="grid-fav-badge">★</span>}
              </div>
              <div className="grid-card-body">
                {editingId === entry.id ? (
                  <input
                    className="grid-card-rename-input"
                    autoFocus
                    value={editingName}
                    onClick={e => e.stopPropagation()}
                    onContextMenu={e => e.stopPropagation()}
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
                  <div
                    className="grid-card-name"
                    title="Click to rename"
                    onClick={e => startRename(e, entry)}
                  >
                    {entry.favorite && <span className="fav-inline">★ </span>}
                    {entry.displayName}
                  </div>
                )}
                <div className="grid-card-meta">
                  {entry.width}×{entry.height}
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {menu && (
        <ClipContextMenu
          entry={menu.entry}
          x={menu.x}
          y={menu.y}
          folders={folders}
          activeView={activeView}
          storageProvider={storageProvider}
          syncedToServer={syncedRels.has(clipRelName(appRoot, menu.entry.path) ?? "")}
          onClose={() => setMenu(null)}
          onToggleFavorite={() => onToggleFavorite(menu.entry)}
          onAddToFolder={folderId => onAddToFolder(menu.entry.id, folderId)}
          onRemoveFromCurrentFolder={() => onRemoveFromCurrentFolder(menu.entry.id)}
          onDelete={() => onRequestDelete(menu.entry)}
          onDeleteFromServer={() => onDeleteFromServer(menu.entry)}
        />
      )}
    </div>
  );
}
