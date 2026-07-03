import { useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import { clamp, parseStoredSize } from "./resizable";

export interface UseResizableOptions {
  axis: "x" | "y";
  defaultSize: number;
  min: number;
  max: number;
  storageKey: string;
}

export interface UseResizableResult {
  size: number;
  onPointerDown: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerMove: (e: ReactPointerEvent<HTMLElement>) => void;
  onPointerUp: (e: ReactPointerEvent<HTMLElement>) => void;
}

interface DragState {
  startPos: number;
  startSize: number;
}

// Drives a single-axis drag handle: "x" resizes by horizontal pixel delta,
// "y" resizes by vertical delta converted to vh (relative to window height).
// Reads the initial size from localStorage; persists the final size on
// pointer-up. Uses Pointer Capture so the drag tracks correctly even when
// the pointer moves off the handle element mid-gesture.
export function useResizable({
  axis,
  defaultSize,
  min,
  max,
  storageKey,
}: UseResizableOptions): UseResizableResult {
  const [size, setSize] = useState(() =>
    parseStoredSize(localStorage.getItem(storageKey), defaultSize, min, max)
  );
  const dragState = useRef<DragState | null>(null);

  function sizeFromEvent(e: ReactPointerEvent<HTMLElement>): number {
    const drag = dragState.current;
    if (!drag) return size;
    const pos = axis === "x" ? e.clientX : e.clientY;
    const deltaPx = pos - drag.startPos;
    const delta = axis === "x" ? deltaPx : (deltaPx / window.innerHeight) * 100;
    return clamp(drag.startSize + delta, min, max);
  }

  function onPointerDown(e: ReactPointerEvent<HTMLElement>) {
    e.currentTarget.setPointerCapture(e.pointerId);
    dragState.current = {
      startPos: axis === "x" ? e.clientX : e.clientY,
      startSize: size,
    };
    document.body.style.cursor = axis === "x" ? "col-resize" : "row-resize";
    document.body.style.userSelect = "none";
  }

  function onPointerMove(e: ReactPointerEvent<HTMLElement>) {
    if (!dragState.current) return;
    setSize(sizeFromEvent(e));
  }

  function onPointerUp(e: ReactPointerEvent<HTMLElement>) {
    if (!dragState.current) return;
    const finalSize = sizeFromEvent(e);
    setSize(finalSize);
    localStorage.setItem(storageKey, String(finalSize));
    dragState.current = null;
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  }

  return { size, onPointerDown, onPointerMove, onPointerUp };
}
