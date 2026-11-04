/** Expands an output file-name template. Tokens: {date} {time} {source} {n}. Result is filesystem-safe. */
export function expandFileName(template: string, ctx: { date: Date; source?: string; n?: number }): string {
  const d = ctx.date;
  const pad = (v: number) => String(v).padStart(2, "0");
  const date = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  const time = `${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}`;
  const out = template
    .replaceAll("{date}", date)
    .replaceAll("{time}", time)
    .replaceAll("{source}", ctx.source ?? "Screen")
    .replaceAll("{n}", String(ctx.n ?? 1));
  return sanitizeFileName(out);
}

const RESERVED = /^(con|prn|aux|nul|com[0-9]|lpt[0-9])$/i;

/** Removes characters invalid on Windows/macOS/Linux/Android and trims to 120 chars. */
export function sanitizeFileName(name: string): string {
  // eslint-disable-next-line no-control-regex
  let s = name.replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_").replace(/\s+/g, " ").trim();
  s = s.replace(/[. ]+$/, "");
  if (s.length === 0) s = "CaptureOne";
  if (RESERVED.test(s)) s = `_${s}`;
  return s.slice(0, 120);
}
