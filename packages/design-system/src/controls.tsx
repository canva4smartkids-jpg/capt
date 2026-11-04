import { useId, type ChangeEvent, type ComponentPropsWithRef, type KeyboardEvent, type ReactNode } from "react";
import { cx } from "./cx.js";
import { Icon, type IconName } from "./icons.js";
import { Spinner } from "./display.js";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger" | "record";
export type ButtonSize = "sm" | "md" | "lg" | "xl";

export interface ButtonProps extends ComponentPropsWithRef<"button"> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  icon?: IconName;
  iconRight?: IconName;
  loading?: boolean;
  block?: boolean;
}

export function Button({ variant = "secondary", size = "md", icon, iconRight, loading = false, block, className, children, disabled, type = "button", ...rest }: ButtonProps) {
  return (
    <button
      type={type}
      className={cx("co-btn", `co-btn--${variant}`, `co-btn--${size}`, block && "co-btn--block", loading && "is-loading", className)}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      {...rest}
    >
      {loading ? <Spinner size={size === "sm" ? 12 : 16} /> : icon ? <Icon name={icon} size={size === "sm" ? 14 : size === "xl" ? 22 : 18} /> : null}
      {children !== undefined && <span className="co-btn__label">{children}</span>}
      {iconRight && !loading && <Icon name={iconRight} size={size === "sm" ? 14 : 16} />}
    </button>
  );
}

export interface IconButtonProps extends Omit<ComponentPropsWithRef<"button">, "children"> {
  icon: IconName;
  /** Accessible name (also used as the tooltip). */
  label: string;
  variant?: ButtonVariant;
  size?: ButtonSize;
  pressed?: boolean;
}

export function IconButton({ icon, label, variant = "ghost", size = "md", pressed, className, type = "button", ...rest }: IconButtonProps) {
  return (
    <button
      type={type}
      className={cx("co-iconbtn", `co-btn--${variant}`, `co-iconbtn--${size}`, className)}
      aria-label={label}
      title={label}
      aria-pressed={pressed}
      {...rest}
    >
      <Icon name={icon} size={size === "sm" ? 15 : size === "lg" ? 20 : size === "xl" ? 24 : 18} />
    </button>
  );
}

export interface FieldProps {
  label: ReactNode;
  /** id of the control; generated when omitted. */
  htmlFor?: string;
  description?: ReactNode;
  error?: string | null | undefined;
  children: (ids: { id: string; describedBy: string | undefined; invalid: boolean }) => ReactNode;
  className?: string;
  inline?: boolean;
}

/** Label + description + error wiring (aria-describedby / aria-invalid) for any control. */
export function Field({ label, htmlFor, description, error, children, className, inline }: FieldProps) {
  const gen = useId();
  const id = htmlFor ?? `f${gen}`;
  const descId = description ? `${id}-desc` : undefined;
  const errId = error ? `${id}-err` : undefined;
  const describedBy = [descId, errId].filter(Boolean).join(" ") || undefined;
  return (
    <div className={cx("co-field", inline && "co-field--inline", error && "is-invalid", className)}>
      <label className="co-field__label" htmlFor={id}>
        {label}
      </label>
      <div className="co-field__control">{children({ id, describedBy, invalid: !!error })}</div>
      {description && (
        <p className="co-field__desc" id={descId}>
          {description}
        </p>
      )}
      {error && (
        <p className="co-field__error" id={errId} role="alert">
          <Icon name="alert" size={14} /> {error}
        </p>
      )}
    </div>
  );
}

export interface SelectOption<V extends string = string> {
  value: V;
  label: string;
  disabled?: boolean;
}

export interface SelectProps<V extends string = string> extends Omit<ComponentPropsWithRef<"select">, "onChange" | "value"> {
  value: V;
  options: readonly SelectOption<V>[];
  onValueChange: (value: V) => void;
}

export function Select<V extends string = string>({ value, options, onValueChange, className, ...rest }: SelectProps<V>) {
  return (
    <div className={cx("co-select", className)}>
      <select value={value} onChange={(e: ChangeEvent<HTMLSelectElement>) => onValueChange(e.target.value as V)} {...rest}>
        {options.map((o) => (
          <option key={o.value} value={o.value} disabled={o.disabled}>
            {o.label}
          </option>
        ))}
      </select>
      <Icon name="chevronDown" size={16} className="co-select__chevron" />
    </div>
  );
}

export interface TextInputProps extends ComponentPropsWithRef<"input"> {
  icon?: IconName;
  suffix?: ReactNode;
}

export function TextInput({ icon, suffix, className, ...rest }: TextInputProps) {
  return (
    <div className={cx("co-input", icon && "co-input--icon", className)}>
      {icon && <Icon name={icon} size={16} className="co-input__icon" />}
      <input {...rest} />
      {suffix && <span className="co-input__suffix">{suffix}</span>}
    </div>
  );
}

export interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: ReactNode;
  description?: ReactNode;
  disabled?: boolean;
  /** Explanation rendered when disabled (e.g. "System audio is not supported on Linux"). */
  disabledReason?: ReactNode;
  id?: string;
  className?: string;
}

export function Toggle({ checked, onChange, label, description, disabled, disabledReason, id, className }: ToggleProps) {
  const gen = useId();
  const tid = id ?? `t${gen}`;
  const descId = description || (disabled && disabledReason) ? `${tid}-desc` : undefined;
  return (
    <div className={cx("co-toggle", disabled && "is-disabled", className)}>
      <div className="co-toggle__text">
        <label className="co-toggle__label" htmlFor={tid}>
          {label}
        </label>
        {descId && (
          <p className="co-toggle__desc" id={descId}>
            {disabled && disabledReason ? disabledReason : description}
          </p>
        )}
      </div>
      <button
        id={tid}
        type="button"
        role="switch"
        aria-checked={checked}
        aria-describedby={descId}
        disabled={disabled}
        className={cx("co-switch", checked && "is-on")}
        onClick={() => onChange(!checked)}
      >
        <span className="co-switch__thumb" />
      </button>
    </div>
  );
}

export interface SliderProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  onChange: (value: number) => void;
  label: ReactNode;
  formatValue?: (value: number) => string;
  disabled?: boolean;
  id?: string;
}

export function Slider({ value, min, max, step = 1, onChange, label, formatValue = String, disabled, id }: SliderProps) {
  const gen = useId();
  const sid = id ?? `s${gen}`;
  const pct = max > min ? ((value - min) / (max - min)) * 100 : 0;
  return (
    <div className={cx("co-slider", disabled && "is-disabled")}>
      <div className="co-slider__head">
        <label htmlFor={sid}>{label}</label>
        <output htmlFor={sid}>{formatValue(value)}</output>
      </div>
      <input
        id={sid}
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        aria-valuetext={formatValue(value)}
        style={{ "--co-slider-pct": `${pct}%` } as React.CSSProperties}
        onChange={(e) => onChange(Number(e.target.value))}
      />
    </div>
  );
}

export interface SegmentedOption<V extends string> {
  value: V;
  label: ReactNode;
  icon?: IconName;
  disabled?: boolean;
  title?: string;
}

export interface SegmentedControlProps<V extends string> {
  value: V;
  options: readonly SegmentedOption<V>[];
  onChange: (value: V) => void;
  label: string;
  size?: "sm" | "md";
  className?: string;
}

/** Radio-group semantics with roving focus (arrow keys move and select). */
export function SegmentedControl<V extends string>({ value, options, onChange, label, size = "md", className }: SegmentedControlProps<V>) {
  const enabled = options.filter((o) => !o.disabled);
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const delta = e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : e.key === "ArrowLeft" || e.key === "ArrowUp" ? -1 : 0;
    if (!delta || enabled.length === 0) return;
    e.preventDefault();
    const idx = enabled.findIndex((o) => o.value === value);
    const next = enabled[(idx + delta + enabled.length) % enabled.length];
    if (next) {
      onChange(next.value);
      const btn = e.currentTarget.querySelector<HTMLButtonElement>(`[data-value="${CSS.escape(next.value)}"]`);
      btn?.focus();
    }
  };
  return (
    <div role="radiogroup" aria-label={label} className={cx("co-segmented", `co-segmented--${size}`, className)} onKeyDown={onKey}>
      {options.map((o) => {
        const selected = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={selected}
            data-value={o.value}
            tabIndex={selected ? 0 : -1}
            disabled={o.disabled}
            title={o.title}
            className={cx("co-segmented__item", selected && "is-selected")}
            onClick={() => onChange(o.value)}
          >
            {o.icon && <Icon name={o.icon} size={15} />}
            <span>{o.label}</span>
          </button>
        );
      })}
    </div>
  );
}
