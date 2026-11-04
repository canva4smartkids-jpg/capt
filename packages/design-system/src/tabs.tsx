import { useRef, type KeyboardEvent, type ReactNode } from "react";
import { cx } from "./cx.js";
import { Icon, type IconName } from "./icons.js";

export interface TabItem<V extends string> {
  id: V;
  label: ReactNode;
  icon?: IconName;
  disabled?: boolean;
  badge?: ReactNode;
}

export interface TabsProps<V extends string> {
  /** Prefix for tab/panel ids so TabPanel can reference them. */
  idPrefix: string;
  label: string;
  tabs: readonly TabItem<V>[];
  value: V;
  onChange: (value: V) => void;
  className?: string;
}

export const tabId = (prefix: string, id: string) => `${prefix}-tab-${id}`;
export const panelId = (prefix: string, id: string) => `${prefix}-panel-${id}`;

/** WAI-ARIA tabs with automatic activation and roving tabindex (Arrow/Home/End). */
export function Tabs<V extends string>({ idPrefix, label, tabs, value, onChange, className }: TabsProps<V>) {
  const listRef = useRef<HTMLDivElement>(null);
  const enabled = tabs.filter((t) => !t.disabled);
  const focusTab = (id: V) => listRef.current?.querySelector<HTMLButtonElement>(`#${CSS.escape(tabId(idPrefix, id))}`)?.focus();
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const idx = enabled.findIndex((t) => t.id === value);
    let next: TabItem<V> | undefined;
    if (e.key === "ArrowRight") next = enabled[(idx + 1) % enabled.length];
    else if (e.key === "ArrowLeft") next = enabled[(idx - 1 + enabled.length) % enabled.length];
    else if (e.key === "Home") next = enabled[0];
    else if (e.key === "End") next = enabled[enabled.length - 1];
    if (next) {
      e.preventDefault();
      onChange(next.id);
      focusTab(next.id);
    }
  };
  return (
    <div ref={listRef} role="tablist" aria-label={label} className={cx("co-tabs", className)} onKeyDown={onKey}>
      {tabs.map((t) => {
        const selected = t.id === value;
        return (
          <button
            key={t.id}
            id={tabId(idPrefix, t.id)}
            type="button"
            role="tab"
            aria-selected={selected}
            aria-controls={panelId(idPrefix, t.id)}
            tabIndex={selected ? 0 : -1}
            disabled={t.disabled}
            className={cx("co-tab", selected && "is-selected")}
            onClick={() => onChange(t.id)}
          >
            {t.icon && <Icon name={t.icon} size={16} />}
            <span>{t.label}</span>
            {t.badge !== undefined && <span className="co-tab__badge">{t.badge}</span>}
          </button>
        );
      })}
    </div>
  );
}

export interface TabPanelProps {
  idPrefix: string;
  id: string;
  active: boolean;
  children: ReactNode;
  className?: string;
}

export function TabPanel({ idPrefix, id, active, children, className }: TabPanelProps) {
  return (
    <div role="tabpanel" id={panelId(idPrefix, id)} aria-labelledby={tabId(idPrefix, id)} hidden={!active} tabIndex={0} className={cx("co-tabpanel", className)}>
      {active ? children : null}
    </div>
  );
}
