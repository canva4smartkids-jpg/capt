/**
 * Full-screen region selector (Tauri window "region"). Shows a frozen capture of
 * the display (so selection works without a compositor), dims everything outside
 * the selection, and reports the rect in physical pixels relative to the display.
 */
import { Icon } from "@captureone/design-system";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useCallback, useEffect, useRef, useState, type PointerEvent } from "react";
import { invoke } from "../lib/ipc";
import { HANDLES, MIN_REGION_PX, containsPoint, encodedSize, hitHandle, isValidRegion, keyboardAdjust, moveBox, rectFromPoints, resizeBox, toPhysicalRect, type Box, type Handle, type Point } from "../lib/region";

type Drag = { mode: "new"; origin: Point } | { mode: "move"; last: Point } | { mode: "resize"; handle: Handle; last: Point };

export function RegionView({ params }: { params: URLSearchParams }) {
  const displayId = params.get("displayId") ?? "";
  const dpr = window.devicePixelRatio || 1;
  const displayW = Number(params.get("w")) || Math.round(window.innerWidth * dpr);
  const displayH = Number(params.get("h")) || Math.round(window.innerHeight * dpr);
  const [box, setBox] = useState<Box | null>(null);
  const [background, setBackground] = useState<string | null>(null);
  const drag = useRef<Drag | null>(null);
  const done = useRef(false);
  const bounds = { width: window.innerWidth, height: window.innerHeight };
  const minCss = MIN_REGION_PX / dpr;

  useEffect(() => {
    document.documentElement.dataset.theme = "dark";
    document.body.classList.add("region-body");
    let shown = false;
    const show = () => {
      if (shown) return;
      shown = true;
      const w = getCurrentWindow();
      void w.show().then(() => w.setFocus()).catch(() => undefined);
    };
    // Capture the display *before* this window becomes visible, so the frozen frame shows the desktop.
    invoke("get_source_thumbnail", { kind: "display", sourceId: displayId, maxWidth: displayW })
      .then((url) => setBackground(url))
      .catch(() => setBackground(null))
      .finally(() => requestAnimationFrame(show));
    const fallback = setTimeout(show, 2500);
    return () => clearTimeout(fallback);
  }, [displayId, displayW]);

  const finish = useCallback(
    (b: Box | null) => {
      if (done.current) return;
      const rect = b ? toPhysicalRect(b, dpr, { width: displayW, height: displayH }) : null;
      if (rect && !isValidRegion(rect)) return;
      done.current = true;
      void invoke("region_selection_done", { rect }).catch(() => getCurrentWindow().close());
    },
    [dpr, displayW, displayH],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        finish(null);
      } else if (e.key === "Enter") {
        e.preventDefault();
        if (box) finish(box);
      } else if (box) {
        const next = keyboardAdjust(box, e.key, { shift: e.shiftKey, alt: e.altKey }, bounds, minCss);
        if (next) {
          e.preventDefault();
          setBox(next);
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const point = (e: PointerEvent): Point => ({ x: e.clientX, y: e.clientY });

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest(".region-toolbar")) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const p = point(e);
    const handle = box ? hitHandle(box, p, 10) : null;
    if (box && handle) drag.current = { mode: "resize", handle, last: p };
    else if (box && containsPoint(box, p)) drag.current = { mode: "move", last: p };
    else {
      drag.current = { mode: "new", origin: p };
      setBox({ x: p.x, y: p.y, width: 0, height: 0 });
    }
  };

  const onPointerMove = (e: PointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const p = point(e);
    if (d.mode === "new") setBox(rectFromPoints(d.origin, p, bounds));
    else if (box && d.mode === "move") {
      setBox(moveBox(box, p.x - d.last.x, p.y - d.last.y, bounds));
      d.last = p;
    } else if (box && d.mode === "resize") {
      setBox(resizeBox(box, d.handle, p.x - d.last.x, p.y - d.last.y, bounds, minCss));
      d.last = p;
    }
  };

  const onPointerUp = () => {
    const d = drag.current;
    drag.current = null;
    if (d?.mode === "new" && box && (box.width < minCss || box.height < minCss)) setBox(null);
  };

  const phys = box ? toPhysicalRect(box, dpr, { width: displayW, height: displayH }) : null;
  const valid = phys ? isValidRegion(phys) : false;
  const enc = phys ? encodedSize(phys) : null;
  const toolbarBelow = box ? box.y + box.height + 56 < bounds.height : true;

  return (
    <div
      className="region-root"
      style={background ? { backgroundImage: `url("${background}")` } : undefined}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onDoubleClick={() => box && valid && finish(box)}
      role="application"
      aria-label="Region selector. Drag to select an area. Enter confirms, Escape cancels, arrow keys move the selection, Alt+arrow keys resize it."
    >
      {!box && <div className="region-dim" />}
      {!box && (
        <div className="region-hint">
          <Icon name="region" size={18} /> Drag to select an area · <kbd className="co-kbd">Enter</kbd> confirm · <kbd className="co-kbd">Esc</kbd> cancel
        </div>
      )}
      {box && (
        <div className={`region-box ${valid ? "" : "is-invalid"}`} style={{ left: box.x, top: box.y, width: box.width, height: box.height }}>
          {HANDLES.map((h) => (
            <span key={h} className={`region-handle region-handle--${h}`} />
          ))}
          <span className="region-size" aria-live="polite">
            {phys ? `${phys.width} × ${phys.height}` : ""}
            {enc && phys && (enc.width !== phys.width || enc.height !== phys.height) ? ` (encodes ${enc.width}×${enc.height})` : ""}
          </span>
          {!drag.current && (
            <div className={`region-toolbar ${toolbarBelow ? "is-below" : "is-inside"}`}>
              <button type="button" className="region-toolbar__btn" onClick={() => finish(null)}>
                <Icon name="close" size={15} /> Cancel
              </button>
              <button type="button" className="region-toolbar__btn region-toolbar__btn--ok" disabled={!valid} onClick={() => finish(box)}>
                <Icon name="check" size={15} /> {valid ? "Use this area" : `Min ${MIN_REGION_PX}px`}
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
