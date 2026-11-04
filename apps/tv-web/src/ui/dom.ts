/** Tiny DOM builder (no framework: keeps the bundle small for 2018-era TV browsers). */

type Child = Node | string | number | null | undefined | false;
type Attrs = Record<string, string | number | boolean | null | undefined | ((ev: Event) => void)>;

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, attrs: Attrs | null = null, ...children: (Child | Child[])[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (attrs) {
    for (const name of Object.keys(attrs)) {
      const v = attrs[name];
      if (v === null || v === undefined || v === false) continue;
      if (typeof v === "function") {
        el.addEventListener(name.replace(/^on/, "").toLowerCase(), v);
      } else if (name === "class") {
        el.className = String(v);
      } else if (v === true) {
        el.setAttribute(name, "");
      } else {
        el.setAttribute(name, String(v));
      }
    }
  }
  append(el, children);
  return el;
}

function append(el: Node, children: (Child | Child[])[]): void {
  for (const c of children) {
    if (Array.isArray(c)) append(el, c);
    else if (c === null || c === undefined || c === false) continue;
    else el.appendChild(typeof c === "string" || typeof c === "number" ? document.createTextNode(String(c)) : c);
  }
}

export function clear(el: Element): void {
  while (el.firstChild) el.removeChild(el.firstChild);
}

/** A focusable button operated with OK/Enter. */
export function button(label: string | Node, onClick: () => void, opts: { cls?: string; id?: string; autofocus?: boolean; disabled?: boolean } = {}): HTMLButtonElement {
  return h(
    "button",
    {
      type: "button",
      class: `btn ${opts.cls ?? ""}`.trim(),
      id: opts.id,
      "data-focusable": true,
      "data-autofocus": opts.autofocus,
      disabled: opts.disabled,
      onclick: () => onClick(),
    },
    label,
  );
}

export function formatDuration(ms: number | undefined): string {
  if (ms === undefined || !(ms >= 0)) return "";
  const total = Math.floor(ms / 1000);
  const hh = Math.floor(total / 3600);
  const mm = Math.floor((total % 3600) / 60);
  const ss = total % 60;
  const two = (n: number) => (n < 10 ? `0${n}` : String(n));
  return hh > 0 ? `${hh}:${two(mm)}:${two(ss)}` : `${mm}:${two(ss)}`;
}

export function formatBytes(n: number | undefined): string {
  if (n === undefined || !(n >= 0)) return "";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

export function formatDate(iso: string): string {
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const two = (n: number) => (n < 10 ? `0${n}` : String(n));
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())} ${two(d.getHours())}:${two(d.getMinutes())}`;
}
