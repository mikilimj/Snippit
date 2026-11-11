import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import logo from "./assets/logo.png";

/**
 * The OS titlebar is turned off (`decorations: false` in tauri.conf.json), so the
 * window chrome is ours: one slim bar carrying the brand, the drag region and the
 * minimize / maximize / close controls, plus invisible edge grips that restore the
 * resizing an undecorated window loses on Linux.
 *
 * Dragging and double-click-to-maximize come from Tauri's injected drag-region
 * script (`data-tauri-drag-region="deep"`) — it hit-tests clickable children, so
 * the control buttons never start a window move.
 */

/** Directions we expose as edge grips; the north edge is the bar itself. */
type ResizeDir = "East" | "West" | "South" | "SouthWest" | "SouthEast";

const RESIZE_HANDLES: { dir: ResizeDir; className: string }[] = [
  { dir: "South", className: "rz-s" },
  { dir: "East", className: "rz-e" },
  { dir: "West", className: "rz-w" },
  { dir: "SouthWest", className: "rz-sw" },
  { dir: "SouthEast", className: "rz-se" },
];

export default function TitleBar() {
  const [maximized, setMaximized] = useState(false);

  // Mirror the real window state instead of tracking our own clicks: the window
  // manager (or a double click on the bar) can toggle it behind our back.
  useEffect(() => {
    const win = getCurrentWindow();
    let disposed = false;
    const sync = () => {
      win
        .isMaximized()
        .then(v => {
          if (!disposed) setMaximized(v);
        })
        .catch(() => {});
    };
    sync();
    const unlisten = win.onResized(sync);
    return () => {
      disposed = true;
      unlisten.then(f => f());
    };
  }, []);

  function startResize(e: React.MouseEvent, dir: ResizeDir) {
    // A maximized window has no edges to drag (and the WM would fight it).
    if (e.button !== 0 || maximized) return;
    e.preventDefault();
    getCurrentWindow().startResizeDragging(dir).catch(err => {
      console.error("Failed to resize window:", err);
    });
  }

  return (
    <>
      <div className="titlebar">
        <div className="titlebar-drag" data-tauri-drag-region="deep">
          <img className="titlebar-logo" src={logo} alt="" />
          <span className="titlebar-name">Snippit</span>
        </div>

        <div className="titlebar-controls">
          <button
            className="titlebar-btn"
            title="Minimize"
            aria-label="Minimize"
            onClick={() => getCurrentWindow().minimize().catch(() => {})}
          >
            <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
              <path
                d="M1.2 7h7.6"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.1"
                strokeLinecap="round"
              />
            </svg>
          </button>

          <button
            className="titlebar-btn"
            title={maximized ? "Restore" : "Maximize"}
            aria-label={maximized ? "Restore" : "Maximize"}
            onClick={() => getCurrentWindow().toggleMaximize().catch(() => {})}
          >
            {maximized ? (
              <svg
                viewBox="0 0 10 10"
                width="10"
                height="10"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.1"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <rect x="1" y="3.2" width="5.8" height="5.8" rx="1.2" />
                <path d="M3.4 3.2v-.9A1.3 1.3 0 0 1 4.7 1h3A1.3 1.3 0 0 1 9 2.3v3a1.3 1.3 0 0 1-1.3 1.3h-1.1" />
              </svg>
            ) : (
              <svg
                viewBox="0 0 10 10"
                width="10"
                height="10"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.1"
                strokeLinejoin="round"
                aria-hidden="true"
              >
                <rect x="1.2" y="1.2" width="7.6" height="7.6" rx="1.4" />
              </svg>
            )}
          </button>

          <button
            className="titlebar-btn titlebar-btn-close"
            title="Close"
            aria-label="Close"
            onClick={() => getCurrentWindow().close().catch(() => {})}
          >
            <svg viewBox="0 0 10 10" width="10" height="10" aria-hidden="true">
              <path
                d="M1.7 1.7l6.6 6.6M8.3 1.7L1.7 8.3"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.1"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>
      </div>

      {/* Edge grips: only visible to the cursor, so the resize cursors still show
          at the window border without an OS frame providing them. */}
      <div className="window-resize-layer">
        {RESIZE_HANDLES.map(h => (
          <div
            key={h.dir}
            className={`window-resize-handle ${h.className}`}
            onMouseDown={e => startResize(e, h.dir)}
          />
        ))}
      </div>
    </>
  );
}
