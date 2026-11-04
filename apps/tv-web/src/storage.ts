/** Remembers the paired hub. localStorage can be unavailable or throw on some TV firmwares. */

export interface SavedConnection {
  hubUrl: string;
  token: string;
  hubName: string;
  hubId: string;
  pairedAt: string;
}

const KEY = "captureone.tv.connection.v1";

function storage(): Storage | null {
  try {
    return typeof localStorage !== "undefined" ? localStorage : null;
  } catch {
    return null;
  }
}

export function loadConnection(): SavedConnection | null {
  try {
    const raw = storage()?.getItem(KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as Partial<SavedConnection>;
    if (typeof v.hubUrl === "string" && typeof v.token === "string" && v.token.length >= 32) {
      return { hubUrl: v.hubUrl, token: v.token, hubName: v.hubName ?? "", hubId: v.hubId ?? "", pairedAt: v.pairedAt ?? "" };
    }
  } catch {
    /* corrupt entry */
  }
  return null;
}

export function saveConnection(c: SavedConnection): boolean {
  try {
    storage()?.setItem(KEY, JSON.stringify(c));
    return true;
  } catch {
    return false;
  }
}

export function clearConnection(): void {
  try {
    storage()?.removeItem(KEY);
  } catch {
    /* ignore */
  }
}

const LAST_ADDRESS = "captureone.tv.lastAddress.v1";

export function loadLastAddress(): string {
  try {
    return storage()?.getItem(LAST_ADDRESS) ?? "";
  } catch {
    return "";
  }
}

export function saveLastAddress(a: string): void {
  try {
    storage()?.setItem(LAST_ADDRESS, a);
  } catch {
    /* ignore */
  }
}
