import type { ComponentPropsWithRef, ReactNode } from "react";
import { cx } from "./cx.js";
import { Icon, type IconName } from "./icons.js";
import { acceleratorParts, type KeyPlatform } from "./accelerator.js";

export type Tone = "neutral" | "info" | "success" | "warning" | "danger" | "accent";

export function Spinner({ size = 16, label }: { size?: number; label?: string }) {
  return (
    <span className="co-spinner" style={{ width: size, height: size }} role={label ? "status" : undefined} aria-label={label} aria-hidden={label ? undefined : true} />
  );
}

export interface CardProps extends Omit<ComponentPropsWithRef<"section">, "title"> {
  title?: ReactNode;
  subtitle?: ReactNode;
  icon?: IconName;
  actions?: ReactNode;
  padded?: boolean;
}

export function Card({ title, subtitle, icon, actions, padded = true, className, children, ...rest }: CardProps) {
  return (
    <section className={cx("co-card", !padded && "co-card--flush", className)} {...rest}>
      {(title || actions) && (
        <header className="co-card__header">
          <div className="co-card__titles">
            {title && (
              <h2 className="co-card__title">
                {icon && <Icon name={icon} size={17} />}
                {title}
              </h2>
            )}
            {subtitle && <p className="co-card__subtitle">{subtitle}</p>}
          </div>
          {actions && <div className="co-card__actions">{actions}</div>}
        </header>
      )}
      <div className="co-card__body">{children}</div>
    </section>
  );
}

export interface BadgeProps extends ComponentPropsWithRef<"span"> {
  tone?: Tone;
  icon?: IconName;
  dot?: boolean;
}

export function Badge({ tone = "neutral", icon, dot, className, children, ...rest }: BadgeProps) {
  return (
    <span className={cx("co-badge", `co-badge--${tone}`, className)} {...rest}>
      {dot && <span className="co-badge__dot" aria-hidden />}
      {icon && <Icon name={icon} size={12} />}
      {children}
    </span>
  );
}

export interface ProgressBarProps {
  /** 0..1, or undefined for indeterminate. */
  value: number | undefined;
  label: string;
  tone?: Tone;
  showValue?: boolean;
  valueText?: string;
  className?: string;
}

export function ProgressBar({ value, label, tone = "accent", showValue, valueText, className }: ProgressBarProps) {
  const clamped = value === undefined ? undefined : Math.min(1, Math.max(0, value));
  const pct = clamped === undefined ? undefined : Math.round(clamped * 100);
  return (
    <div className={cx("co-progress", `co-progress--${tone}`, clamped === undefined && "is-indeterminate", className)}>
      {showValue && (
        <div className="co-progress__head">
          <span>{label}</span>
          <span>{valueText ?? (pct !== undefined ? `${pct}%` : "")}</span>
        </div>
      )}
      <div
        className="co-progress__track"
        role="progressbar"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        aria-valuetext={valueText}
      >
        <div className="co-progress__fill" style={{ width: pct === undefined ? undefined : `${pct}%` }} />
      </div>
    </div>
  );
}

export interface EmptyStateProps {
  icon?: IconName;
  title: ReactNode;
  description?: ReactNode;
  action?: ReactNode;
  className?: string;
}

export function EmptyState({ icon = "info", title, description, action, className }: EmptyStateProps) {
  return (
    <div className={cx("co-empty", className)}>
      <div className="co-empty__icon">
        <Icon name={icon} size={28} />
      </div>
      <h3 className="co-empty__title">{title}</h3>
      {description && <p className="co-empty__desc">{description}</p>}
      {action && <div className="co-empty__action">{action}</div>}
    </div>
  );
}

export interface KeyboardShortcutProps {
  /** Tauri accelerator, e.g. "CommandOrControl+Shift+R". */
  accelerator: string;
  platform?: KeyPlatform;
  className?: string;
}

/** Renders an accelerator as key chips with an accessible text alternative. */
export function KeyboardShortcut({ accelerator, platform, className }: KeyboardShortcutProps) {
  const parts = acceleratorParts(accelerator, platform);
  if (parts.length === 0) return <span className={cx("co-kbd-group", "is-empty", className)}>Not set</span>;
  return (
    <span className={cx("co-kbd-group", className)} aria-label={parts.map((p) => p.name).join(" + ")} role="img">
      {parts.map((p, i) => (
        <kbd key={`${p.symbol}-${i}`} className="co-kbd" aria-hidden>
          {p.symbol}
        </kbd>
      ))}
    </span>
  );
}

export function VisuallyHidden({ children }: { children: ReactNode }) {
  return <span className="co-sr-only">{children}</span>;
}
