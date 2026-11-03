import type { Folder } from "./api";
import type { LibraryView } from "./libraryView";

interface FolderRailProps {
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
}

export default function FolderRail({
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
}: FolderRailProps) {
  return (
    <nav className="folder-rail">
      <button
        className={`folder-chip${activeView.kind === "all" ? " active" : ""}`}
        onClick={() => onSelectView({ kind: "all" })}
      >
        All
      </button>
      <button
        className={`folder-chip${activeView.kind === "favorites" ? " active" : ""}`}
        onClick={() => onSelectView({ kind: "favorites" })}
      >
        ★ Favorites
      </button>
      <button
        className={`folder-chip${activeView.kind === "unsorted" ? " active" : ""}`}
        onClick={() => onSelectView({ kind: "unsorted" })}
      >
        Unsorted
      </button>
      {folders.map(f => (
        <span
          key={f.id}
          className={`folder-chip folder-chip-user${
            activeView.kind === "folder" && activeView.id === f.id ? " active" : ""
          }`}
        >
          <button
            className="folder-chip-label"
            onClick={() => onSelectView({ kind: "folder", id: f.id })}
          >
            {f.name}
          </button>
          <button
            className="folder-chip-delete"
            title="Delete folder (keeps the clips)"
            aria-label="Delete folder"
            onClick={e => onDeleteFolder(e, f)}
          >
            ×
          </button>
        </span>
      ))}
      {creatingFolder ? (
        <input
          className="folder-new-input"
          autoFocus
          value={newFolderName}
          placeholder="Folder name"
          onChange={e => onNewFolderNameChange(e.target.value)}
          onKeyDown={e => {
            if (e.key === "Enter") onCreateFolder();
            if (e.key === "Escape") onCancelCreateFolder();
          }}
          onBlur={onCancelCreateFolder}
        />
      ) : (
        <button className="folder-chip folder-new" onClick={onStartCreateFolder}>
          + Folder
        </button>
      )}
    </nav>
  );
}
