import { useEffect, useRef, useState } from "react";
import {
  storageScanDiff,
  storageImportClips,
  storageDeleteRemote,
  storageDownloadThumb,
  syncNow,
  type ClipRow,
  type SyncDiffResult,
} from "./api";
import { formatSize, formatDate, displayNameFor } from "./syncDiff";

function mediaUrl(mediaPort: number | null, path: string): string | undefined {
  return mediaPort ? `http://127.0.0.1:${mediaPort}/file?path=${encodeURIComponent(path)}` : undefined;
}

// `localOnly` rel-names are `/`-separated paths relative to the watched Clips/
// folder (same shape as `remoteOnly[].path`) — never an absolute path, so they
// can't be compared to `ClipRow.path` directly. Match by normalized suffix
// instead of reconstructing an absolute path from the app root: it's robust to
// both path-separator conventions and doesn't need the root passed in.
function findLibraryRow(rel: string, library: ClipRow[]): ClipRow | undefined {
  const relNorm = rel.replace(/\\/g, "/");
  return library.find(e => {
    const p = e.path.replace(/\\/g, "/");
    return p === relNorm || p.endsWith("/" + relNorm);
  });
}

type LoadState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; diff: SyncDiffResult };

/**
 * One row's thumbnail. Remote-only rows fetch lazily — only once the
 * placeholder scrolls into view — so a large diff never fires N parallel
 * downloads on mount; local-only rows just reuse whatever thumbnail the
 * library already generated (no extra fetch — the file was never uploaded,
 * so there's nothing to download). A failed remote fetch keeps the gray
 * placeholder for good; it is never retried.
 */
function DiffThumb({
  rel,
  localPath,
  mediaPort,
}: {
  rel: string | null; // set => lazily fetch this remote thumb when visible
  localPath: string | null; // pre-known local thumbnail, if any
  mediaPort: number | null;
}) {
  const [resolved, setResolved] = useState<string | null>(localPath);
  const [failed, setFailed] = useState(false);
  const placeholderRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!rel || resolved || failed) return;
    const el = placeholderRef.current;
    if (!el) return;
    let cancelled = false;
    const io = new IntersectionObserver(
      (entries, obs) => {
        if (!entries.some(e => e.isIntersecting)) return;
        obs.disconnect();
        storageDownloadThumb(rel)
          .then(p => {
            if (!cancelled) setResolved(p);
          })
          .catch(() => {
            if (!cancelled) setFailed(true);
          });
      },
      { rootMargin: "80px" }
    );
    io.observe(el);
    return () => {
      cancelled = true;
      io.disconnect();
    };
  }, [rel, resolved, failed]);

  const url = resolved ? mediaUrl(mediaPort, resolved) : undefined;
  if (url) {
    return <img className="diff-thumb" src={url} alt="" loading="lazy" />;
  }
  return <div className="diff-thumb diff-thumb-empty" ref={placeholderRef} />;
}

function DiffSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="diff-section">
      <span className="diff-section-title">{title}</span>
      {children}
    </div>
  );
}

export function SyncDiffModal({
  onClose,
  onImported,
  library,
  mediaPort,
}: {
  onClose: () => void;
  onImported: () => void;
  library: ClipRow[];
  mediaPort: number | null;
}) {
  const [state, setState] = useState<LoadState>({ kind: "loading" });
  const [selectedRemote, setSelectedRemote] = useState<Set<string>>(new Set());
  const [selectedLocal, setSelectedLocal] = useState<Set<string>>(new Set());
  const [deleting, setDeleting] = useState<Set<string>>(new Set());
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [importing, setImporting] = useState(false);
  const [importMsg, setImportMsg] = useState<string | null>(null);
  const [uploading, setUploading] = useState(false);

  // Cancellation, and the guard against setState after unmount: the import
  // loop below awaits one `storageImportClips` call per selected row (see the
  // comment on `importSelected`), and checks this ref between rows.
  const cancelRef = useRef(false);
  const mountedRef = useRef(true);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  function scan() {
    setState({ kind: "loading" });
    setSelectedRemote(new Set());
    setSelectedLocal(new Set());
    storageScanDiff()
      .then(diff => {
        if (!mountedRef.current) return;
        setState({ kind: "ready", diff });
      })
      .catch(err => {
        if (!mountedRef.current) return;
        setState({ kind: "error", message: String(err) });
      });
  }

  // Fail fast, no partial diff: either the whole scan resolves or the whole
  // thing shows an error + Retry.
  useEffect(() => {
    scan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function toggleRemote(rel: string) {
    setSelectedRemote(prev => {
      const next = new Set(prev);
      if (next.has(rel)) next.delete(rel);
      else next.add(rel);
      return next;
    });
  }

  function toggleLocal(rel: string) {
    setSelectedLocal(prev => {
      const next = new Set(prev);
      if (next.has(rel)) next.delete(rel);
      else next.add(rel);
      return next;
    });
  }

  async function deleteRemote(rel: string) {
    if (!confirm(`Delete "${displayNameFor(rel, null)}" from the server? This cannot be undone.`)) return;
    setDeleteError(null);
    setDeleting(prev => new Set(prev).add(rel));
    try {
      await storageDeleteRemote(rel);
      if (!mountedRef.current) return;
      // Remove the row (and its selection) without a full rescan.
      setState(s =>
        s.kind === "ready"
          ? { kind: "ready", diff: { ...s.diff, remoteOnly: s.diff.remoteOnly.filter(r => r.path !== rel) } }
          : s
      );
      setSelectedRemote(prev => {
        const next = new Set(prev);
        next.delete(rel);
        return next;
      });
    } catch (err) {
      if (mountedRef.current) setDeleteError(`Couldn't delete "${rel}": ${err}`);
    } finally {
      if (mountedRef.current) {
        setDeleting(prev => {
          const next = new Set(prev);
          next.delete(rel);
          return next;
        });
      }
    }
  }

  // Import is structured as one `storageImportClips` call per selected row
  // (rather than a single call with the whole array) so that: (1) progress
  // can honestly read "Importing k of n…" instead of a single opaque
  // "Importing…", and (2) cancellation has somewhere to take effect —
  // `cancelRef` is checked between rows, so a Cancel click stops before the
  // next download starts rather than only being able to abort before the
  // batch begins.
  async function importSelected() {
    const rels = Array.from(selectedRemote);
    if (rels.length === 0) return;
    cancelRef.current = false;
    setImporting(true);
    setImportMsg(`Importing 0 of ${rels.length}…`);
    let done = 0;
    for (const rel of rels) {
      if (cancelRef.current || !mountedRef.current) break;
      try {
        await storageImportClips([rel]);
      } catch (err) {
        console.error("Import failed for", rel, err);
      }
      done++;
      if (!mountedRef.current) return;
      setImportMsg(`Importing ${done} of ${rels.length}…`);
    }
    if (!mountedRef.current) return;
    setImporting(false);
    setImportMsg(null);
    onImported();
    scan(); // refresh so imported rows drop out of "remote only"
  }

  function cancelImport() {
    cancelRef.current = true;
  }

  async function uploadSelected() {
    setUploading(true);
    try {
      // plan_uploads picks up every local file missing from sync_state on its
      // own; there's no selective-upload command, so this simply kicks off
      // the same full re-plan "Sync now" uses. The selection count on the
      // button is informational (what's about to go up), not a filter.
      await syncNow();
    } finally {
      if (mountedRef.current) setUploading(false);
    }
  }

  function handleBackdropClick() {
    if (!importing) onClose();
  }

  return (
    <div className="modal-backdrop" onClick={handleBackdropClick}>
      <div className="modal-card modal-card-wide" onClick={e => e.stopPropagation()}>
        <div className="modal-header">
          <span className="modal-title">Sync diff</span>
          <button
            className="modal-close"
            onClick={onClose}
            disabled={importing}
            title={importing ? "Finish or cancel the import first" : undefined}
            aria-label="Close"
          >
            ×
          </button>
        </div>

        <div className="diff-body">
          {state.kind === "loading" && (
            <div className="diff-loading">
              <span className="diff-spinner" />
              <span className="settings-hint">Scanning for differences…</span>
            </div>
          )}

          {state.kind === "error" && (
            <div className="diff-error">
              <div className="msg-error">{state.message}</div>
              <div className="settings-actions">
                <button className="btn-sm" onClick={scan}>Retry</button>
              </div>
            </div>
          )}

          {state.kind === "ready" && (
            <>
              <DiffSection title="On server, not on this PC">
                {state.diff.remoteOnly.length === 0 ? (
                  <p className="diff-empty">Nothing — the server matches this PC.</p>
                ) : (
                  <ul className="diff-list">
                    {state.diff.remoteOnly.map(f => (
                      <li key={f.path} className="diff-row">
                        <input
                          type="checkbox"
                          checked={selectedRemote.has(f.path)}
                          onChange={() => toggleRemote(f.path)}
                        />
                        <DiffThumb rel={f.path} localPath={null} mediaPort={mediaPort} />
                        <div className="diff-row-text">
                          <span className="diff-row-name" title={f.path}>
                            {displayNameFor(f.path, null)}
                          </span>
                          <span className="diff-row-meta">
                            {formatDate(f.modTime)} · {formatSize(f.size)}
                          </span>
                        </div>
                        <button
                          className="btn-cancel diff-delete-btn"
                          disabled={deleting.has(f.path)}
                          onClick={() => deleteRemote(f.path)}
                        >
                          {deleting.has(f.path) ? "Deleting…" : "Delete from server"}
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </DiffSection>

              <DiffSection title="On this PC, not on server">
                {state.diff.localOnly.length === 0 ? (
                  <p className="diff-empty">Nothing — this PC matches the server.</p>
                ) : (
                  <ul className="diff-list">
                    {state.diff.localOnly.map(rel => {
                      const row = findLibraryRow(rel, library);
                      return (
                        <li key={rel} className="diff-row">
                          <input
                            type="checkbox"
                            checked={selectedLocal.has(rel)}
                            onChange={() => toggleLocal(rel)}
                          />
                          <DiffThumb rel={null} localPath={row?.thumbnailPath ?? null} mediaPort={mediaPort} />
                          <div className="diff-row-text">
                            <span className="diff-row-name" title={rel}>
                              {row?.displayName ?? displayNameFor(rel, null)}
                            </span>
                            <span className="diff-row-meta">
                              {row ? formatDate(row.addedAt) : "—"}
                            </span>
                          </div>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </DiffSection>

              {deleteError && <div className="msg-error">{deleteError}</div>}
            </>
          )}
        </div>

        {state.kind === "ready" && (
          <div className="diff-footer">
            <span className="diff-import-msg">{importMsg}</span>
            <div className="diff-footer-actions">
              {importing && (
                <button className="btn-cancel" onClick={cancelImport}>Cancel</button>
              )}
              <button
                className="btn-sm"
                onClick={uploadSelected}
                disabled={selectedLocal.size === 0 || uploading}
              >
                {uploading ? "Uploading…" : `Upload selected to server (${selectedLocal.size})`}
              </button>
              <button
                className="btn-export"
                onClick={importSelected}
                disabled={selectedRemote.size === 0 || importing}
              >
                {importing ? "Importing…" : `Import selected to PC (${selectedRemote.size})`}
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
