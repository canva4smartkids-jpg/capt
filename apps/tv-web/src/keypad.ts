/** On-screen numeric keypad model (no text inputs: TV software keyboards are slow and inconsistent). */

export type KeypadKey = "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "." | ":" | "del" | "clear";

export type KeypadMode = "address" | "code";

export class KeypadModel {
  value: string;
  readonly mode: KeypadMode;
  readonly maxLength: number;

  constructor(mode: KeypadMode, initial = "") {
    this.mode = mode;
    this.maxLength = mode === "code" ? 6 : 21; // "255.255.255.255:65535"
    this.value = "";
    for (const ch of initial) this.press(ch as KeypadKey);
  }

  /** Applies a key and returns the new value. Invalid keys for the mode are ignored. */
  press(key: KeypadKey): string {
    if (key === "del") {
      this.value = this.value.slice(0, -1);
    } else if (key === "clear") {
      this.value = "";
    } else if (this.value.length >= this.maxLength) {
      // full
    } else if (key === "." || key === ":") {
      if (this.mode === "address" && this.canAppendSeparator(key)) this.value += key;
    } else if (key >= "0" && key <= "9") {
      this.value += key;
    }
    return this.value;
  }

  private canAppendSeparator(sep: "." | ":"): boolean {
    const v = this.value;
    if (v.length === 0) return false;
    const last = v.charAt(v.length - 1);
    if (last === "." || last === ":") return false;
    if (v.indexOf(":") >= 0) return false; // nothing after the port separator but digits
    if (sep === ".") return v.split(".").length < 4;
    return v.split(".").length === 4;
  }

  isComplete(): boolean {
    return this.mode === "code" ? /^\d{6}$/.test(this.value) : parseHubAddress(this.value) !== null;
  }
}

export interface HubAddress {
  host: string;
  port: number;
}

/** Accepts `a.b.c.d` or `a.b.c.d:port` (IPv4 only; TVs and hubs live on the same LAN). */
export function parseHubAddress(input: string, defaultPort = 47800): HubAddress | null {
  const m = /^(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})(?::(\d{1,5}))?$/.exec(input.trim());
  if (!m) return null;
  const octets = [m[1], m[2], m[3], m[4]].map((o) => Number(o));
  if (octets.some((o) => !(o >= 0 && o <= 255))) return null;
  const port = m[5] !== undefined ? Number(m[5]) : defaultPort;
  if (!(port >= 1 && port <= 65535)) return null;
  return { host: octets.join("."), port };
}
