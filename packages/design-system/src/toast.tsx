import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { cx } from "./cx.js";
import { Icon, type IconName } from "./icons.js";
import { IconButton } from "./controls.js";

export type ToastTone = "info" | "success" | "warning" | "danger";

export interface ToastOptions {
  tone?: ToastTone;
  title: string;
  message?: string;
  /** ms; 0 keeps the toast until dismissed. Defaults: 5s (info/success), 8s (warning), sticky (danger). */
  timeout?: number;
  action?: { label: string; onClick: () => void };
  /** Toasts with the same key replace each other instead of stacking. */
  key?: string;
}

export interface ToastItem extends ToastOptions {
  id: number;
  tone: ToastTone;
}

interface ToastApi {
  push: (t: ToastOptions) => number;
  dismiss: (id: number) => void;
  clear: () => void;
}

const ToastContext = createContext<ToastApi | null>(null);

const ICONS: Record<ToastTone, IconName> = { info: "info", success: "check", warning: "alert", danger: "alert" };
const DEFAULT_TIMEOUT: Record<ToastTone, number> = { info: 5000, success: 5000, warning: 8000, danger: 0 };

export function ToastProvider({ children, max = 4 }: { children: ReactNode; max?: number }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  const timers = useRef(new Map<number, ReturnType<typeof setTimeout>>());

  const dismiss = useCallback((id: number) => {
    const t = timers.current.get(id);
    if (t) clearTimeout(t);
    timers.current.delete(id);
    setItems((xs) => xs.filter((x) => x.id !== id));
  }, []);

  const push = useCallback(
    (opts: ToastOptions) => {
      const id = nextId.current++;
      const tone = opts.tone ?? "info";
      const item: ToastItem = { ...opts, id, tone };
      setItems((xs) => {
        const kept = opts.key ? xs.filter((x) => x.key !== opts.key) : xs;
        return [...kept, item].slice(-max);
      });
      const timeout = opts.timeout ?? DEFAULT_TIMEOUT[tone];
      if (timeout > 0) timers.current.set(id, setTimeout(() => dismiss(id), timeout));
      return id;
    },
    [dismiss, max],
  );

  const clear = useCallback(() => {
    timers.current.forEach((t) => clearTimeout(t));
    timers.current.clear();
    setItems([]);
  }, []);

  useEffect(() => () => timers.current.forEach((t) => clearTimeout(t)), []);

  const api = useMemo(() => ({ push, dismiss, clear }), [push, dismiss, clear]);
  return (
    <ToastContext.Provider value={api}>
      {children}
      <ToastViewport items={items} onDismiss={dismiss} />
    </ToastContext.Provider>
  );
}

export function useToast(): ToastApi {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>");
  return ctx;
}

function ToastViewport({ items, onDismiss }: { items: ToastItem[]; onDismiss: (id: number) => void }) {
  return (
    <div className="co-toasts" aria-live="polite" aria-relevant="additions">
      {items.map((t) => (
        <div key={t.id} className={cx("co-toast", `co-toast--${t.tone}`)} role={t.tone === "danger" ? "alert" : "status"}>
          <span className="co-toast__icon">
            <Icon name={ICONS[t.tone]} size={18} />
          </span>
          <div className="co-toast__content">
            <p className="co-toast__title">{t.title}</p>
            {t.message && <p className="co-toast__message">{t.message}</p>}
            {t.action && (
              <button
                type="button"
                className="co-toast__action"
                onClick={() => {
                  t.action?.onClick();
                  onDismiss(t.id);
                }}
              >
                {t.action.label}
              </button>
            )}
          </div>
          <IconButton icon="close" label="Dismiss notification" size="sm" onClick={() => onDismiss(t.id)} />
        </div>
      ))}
    </div>
  );
}
