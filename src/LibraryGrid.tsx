import { useState, useRef } from "react";
import type { ClipRow, Folder } from "./api";
import type { LibraryView } from "./libraryView";
import FolderRail from "./FolderRail";
import ClipCardActions from "./ClipCardActions";
import logo from "./assets/logo.png";

function clipRelName(appRoot: string | null | undefined, path: string): string | null {
  if (typeof appRoot !== "string") return null;
  const norm = path.replace(/\\/g, "/");
  const prefix = `${appRoot.replace(/\\/g, "/").replace(/\/$/, "")}/Clips/`;
  return norm.startsWith(prefix) ? norm.slice(prefix.length) : null;
}

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
  onToggleFavorite: (e: React.MouseEvent, entry: ClipRow) => void;
  onAddToFolder: (clipId: number, folderId: number) => void;
  onRemoveFromLibrary: (e: React.MouseEvent, entry: ClipRow) => void;
  onRemoveFromCurrentFolder: (e: React.MouseEvent, clipId: number) => void;
  onRenameClip: (id: number, name: string) => void;
  onDeleteFromServer: (e: React.MouseEvent, entry: ClipRow) => void;
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
  onRemoveFromLibrary,
  onRemoveFromCurrentFolder,
  onRenameClip,
  onDeleteFromServer,
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
        <div className="brand-row">
          <img className="brand-logo" src={logo} alt="" />
          <span className="brand-name">Snippit</span>
        </div>
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
              ? "No favorites yet — tap ☆ on a clip."
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
                <div className="grid-card-actions-overlay">
                  <ClipCardActions
                    entry={entry}
                    activeView={activeView}
                    folders={folders}
                    storageProvider={storageProvider}
                    syncedToServer={syncedRels.has(clipRelName(appRoot, entry.path) ?? "")}
                    onToggleFavorite={onToggleFavorite}
                    onAddToFolder={onAddToFolder}
                    onRemoveFromLibrary={onRemoveFromLibrary}
                    onRemoveFromCurrentFolder={onRemoveFromCurrentFolder}
                    onDeleteFromServer={onDeleteFromServer}
                  />
                </div>
              </div>
              <div className="grid-card-body">
                {editingId === entry.id ? (
                  <input
                    className="grid-card-rename-input"
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
                  <div
                    className="grid-card-name"
                    onDoubleClick={e => startRename(e, entry)}
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
    </div>
  );
}
