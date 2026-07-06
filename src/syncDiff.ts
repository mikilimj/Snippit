// Pure helpers for the storage diff window and size settings (no Tauri imports).

export interface RemoteIndexEntry {
  relPath: string;
  displayName: string;
  favorite: boolean;
  folders: string[];
}

export function formatSize(bytes: number): string {
  const gb = 1024 * 1024 * 1024;
  const mb = 1024 * 1024;
  if (bytes >= gb) return `${parseFloat((bytes / gb).toFixed(2))} GB`;
  if (bytes >= mb) return `${parseFloat((bytes / mb).toFixed(2))} MB`;
  if (bytes >= 1024) return `${parseFloat((bytes / 1024).toFixed(1))} KB`;
  return `${bytes} B`;
}

export function formatDate(unixSecs: number): string {
  if (unixSecs <= 0) return "—";
  return new Date(unixSecs * 1000).toLocaleDateString();
}

export function displayNameFor(relPath: string, index: RemoteIndexEntry[] | null): string {
  const hit = index?.find(e => e.relPath === relPath);
  if (hit) return hit.displayName;
  const file = relPath.split("/").pop() ?? relPath;
  const dot = file.lastIndexOf(".");
  return dot > 0 ? file.slice(0, dot) : file;
}

export function percentIsValid(p: number): boolean {
  return Number.isInteger(p) && p >= 1 && p <= 99;
}
