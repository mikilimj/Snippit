import type { ClipRow, Folder } from "./api";
import type { LibraryView } from "./libraryView";

interface ClipCardActionsProps {
  entry: ClipRow;
  activeView: LibraryView;
  folders: Folder[];
  storageProvider: string | null;
  syncedToServer: boolean;
  onToggleFavorite: (e: React.MouseEvent, entry: ClipRow) => void;
  onAddToFolder: (clipId: number, folderId: number) => void;
  onRemoveFromLibrary: (e: React.MouseEvent, entry: ClipRow) => void;
  onRemoveFromCurrentFolder: (e: React.MouseEvent, clipId: number) => void;
  onDeleteFromServer: (e: React.MouseEvent, entry: ClipRow) => void;
}

export default function ClipCardActions({
  entry,
  activeView,
  folders,
  storageProvider,
  syncedToServer,
  onToggleFavorite,
  onAddToFolder,
  onRemoveFromLibrary,
  onRemoveFromCurrentFolder,
  onDeleteFromServer,
}: ClipCardActionsProps) {
  return (
    <div className="library-actions">
      <button
        className={`library-fav${entry.favorite ? " on" : ""}`}
        title={entry.favorite ? "Unfavorite" : "Mark as favorite"}
        aria-label={entry.favorite ? "Unfavorite" : "Mark as favorite"}
        onClick={e => onToggleFavorite(e, entry)}
      >
        {entry.favorite ? "★" : "☆"}
      </button>
      {storageProvider !== null && (
        <button
          className="library-remove library-cloud-delete"
          title={
            syncedToServer
              ? "Delete from server (keeps the local file)"
              : "Not uploaded to the server yet"
          }
          aria-label="Delete from server"
          disabled={!syncedToServer}
          onClick={e => onDeleteFromServer(e, entry)}
        >
          ☁
        </button>
      )}
      {folders.length > 0 && (
        <select
          className="library-folder-select"
          title="Add to folder"
          value=""
          onClick={e => e.stopPropagation()}
          onChange={e => {
            const fid = Number(e.target.value);
            if (fid) onAddToFolder(entry.id, fid);
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
          onClick={e => onRemoveFromCurrentFolder(e, entry.id)}
        >
          −
        </button>
      ) : (
        <button
          className="library-remove"
          title="Remove from library (keeps the file)"
          aria-label="Remove from library"
          onClick={e => onRemoveFromLibrary(e, entry)}
        >
          ×
        </button>
      )}
    </div>
  );
}
