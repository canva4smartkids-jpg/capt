import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { cx } from "./cx.js";
import { IconButton } from "./controls.js";

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  footer?: ReactNode;
  size?: "sm" | "md" | "lg" | "xl";
  /** CSS selector of the element focused on open (defaults to the first focusable). */
  initialFocus?: string;
  /** "alertdialog" for destructive confirmations. */
  role?: "dialog" | "alertdialog";
  className?: string;
}

const FOCUSABLE = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), video[controls], [tabindex]:not([tabindex="-1"])';

const scoped = (container: string) =>
  FOCUSABLE.split(",")
    .map((sel) => `${container} ${sel.trim()}`)
    .join(", ");

/** Modal dialog: focus trap, Escape to close, focus restored to the opener, background click closes. */
export function Dialog({ open, onClose, title, description, children, footer, size = "md", initialFocus, role = "dialog", className }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const descId = useId();
  const restoreRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    if (!open) return;
    restoreRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const node = ref.current;
    const target = (initialFocus && node?.querySelector<HTMLElement>(initialFocus)) || node?.querySelector<HTMLElement>(scoped(".co-dialog__body") + ", " + scoped(".co-dialog__footer")) || node;
    target?.focus();
    return () => {
      restoreRef.current?.focus?.();
    };
  }, [open, initialFocus]);

  // Escape closes even when focus left the dialog (e.g. a focused <video> was replaced).
  // Inner handlers that consume Escape call stopPropagation, which keeps it from reaching here.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  useEffect(() => {
    if (!open) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape" && !e.defaultPrevented) {
        e.preventDefault();
        onCloseRef.current();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [open]);

  if (!open) return null;

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Tab" || !ref.current) return;
    const items = Array.from(ref.current.querySelectorAll<HTMLElement>(FOCUSABLE)).filter((el) => !el.hasAttribute("hidden"));
    if (items.length === 0) {
      e.preventDefault();
      return;
    }
    const first = items[0]!;
    const last = items[items.length - 1]!;
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  };

  return createPortal(
    <div className="co-dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={ref}
        role={role}
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={description ? descId : undefined}
        tabIndex={-1}
        className={cx("co-dialog", `co-dialog--${size}`, className)}
        onKeyDown={onKeyDown}
      >
        <header className="co-dialog__header">
          <div>
            <h2 id={titleId} className="co-dialog__title">
              {title}
            </h2>
            {description && (
              <p id={descId} className="co-dialog__desc">
                {description}
              </p>
            )}
          </div>
          <IconButton icon="close" label="Close dialog" size="sm" onClick={onClose} />
        </header>
        {children && <div className="co-dialog__body">{children}</div>}
        {footer && <footer className="co-dialog__footer">{footer}</footer>}
      </div>
    </div>,
    document.body,
  );
}
