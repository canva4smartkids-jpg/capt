#!/usr/bin/env node
/**
 * End-to-end check of the built desktop app through tauri-driver (W3C WebDriver).
 * Drives the real UI against the real backend: record → pause/resume → stop →
 * validated item in the library → preview through the asset protocol → screenshot →
 * region selection overlay → region recording. Writes UI screenshots to SHOT_DIR and a
 * JSON summary to stdout. Run via e2e/run-linux-e2e.sh (Xvfb + tauri-driver).
 *
 *   node e2e/e2e.mjs <path-to-app-binary>
 */
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const WD = process.env.WEBDRIVER_URL ?? "http://127.0.0.1:4444";
const APP = process.argv[2];
const SHOTS = process.env.SHOT_DIR ?? "./screenshots";
const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const summary = { steps: [], screenshots: [] };
let sid = "";

function log(step, data = {}) {
  summary.steps.push({ step, ...data });
  console.error(`[e2e] ${step} ${Object.keys(data).length ? JSON.stringify(data) : ""}`);
}

async function wd(method, path, body) {
  const res = await fetch(WD + path, { method, headers: { "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
  const json = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(`${method} ${path} -> ${res.status} ${JSON.stringify(json.value ?? json).slice(0, 400)}`);
  return json.value;
}
const s = (p) => `/session/${sid}${p}`;
const exec = (script, args = []) => wd("POST", s("/execute/sync"), { script, args });

async function waitFor(desc, fn, timeoutMs = 20_000, everyMs = 250) {
  const start = Date.now();
  let last;
  while (Date.now() - start < timeoutMs) {
    try {
      last = await fn();
      if (last) return last;
    } catch (e) {
      last = e;
    }
    await sleep(everyMs);
  }
  throw new Error(`timed out waiting for ${desc} (last: ${last instanceof Error ? last.message : JSON.stringify(last)})`);
}

async function find(using, value, timeoutMs = 20_000) {
  return waitFor(`${using}=${value}`, async () => (await wd("POST", s("/element"), { using, value }))[ELEMENT], timeoutMs);
}
const css = (sel, t) => find("css selector", sel, t);
const xpath = (xp, t) => find("xpath", xp, t);
const click = async (el) => wd("POST", s(`/element/${el}/click`), {});
const button = (label, t) => xpath(`//button[contains(normalize-space(.), ${JSON.stringify(label)}) and not(@disabled)]`, t);

async function shot(name, { root = false } = {}) {
  mkdirSync(SHOTS, { recursive: true });
  const file = join(SHOTS, `desktop-${name}.png`);
  await sleep(400);
  // Page shots come from WebDriver (exact webview pixels; X11 grabs of WebKit's GL surface can be
  // stale without a compositor). Root shots show overlay windows (indicator, region selector).
  if (root) execFileSync("import", ["-window", "root", file]);
  else writeFileSync(file, Buffer.from(await wd("GET", s("/screenshot")), "base64"));
  summary.screenshots.push(file);
  log("screenshot", { file });
}

async function key(value) {
  await wd("POST", s("/actions"), { actions: [{ type: "key", id: "kbd", actions: [{ type: "keyDown", value }, { type: "keyUp", value }] }] });
  await wd("DELETE", s("/actions"));
}

async function go(hash) {
  await exec(`location.hash = arguments[0]; return true;`, [hash]);
  await sleep(300);
}

async function recordFlow(label, { pause }) {
  await click(await button("Start recording"));
  log(`${label}: start clicked`);
  await css(".countdown-overlay", 10_000);
  if (label === "display") await shot("countdown");
  await waitFor("recording phase", () => exec(`return document.querySelector('.rec-panel--recording .rec-panel__timer')?.textContent ?? null;`), 30_000);
  log(`${label}: recording`);
  await sleep(4000);
  if (label === "display") {
    await shot("recording");
    await shot("indicator", { root: true });
  }
  if (pause) {
    await click(await xpath(`//button[@aria-label='Pause recording' and not(@disabled)]`));
    await waitFor("paused", () => exec(`return !!document.querySelector('.rec-panel--paused');`), 10_000);
    log(`${label}: paused`);
    await sleep(1500);
    await click(await xpath(`//button[@aria-label='Resume recording' and not(@disabled)]`));
    await waitFor("resumed", () => exec(`return !!document.querySelector('.rec-panel--recording');`), 10_000);
    log(`${label}: resumed`);
    await sleep(2500);
  }
  const status = await exec(`return [...document.querySelectorAll('.rec-panel .stat-list dd')].map(d => d.textContent);`);
  log(`${label}: live stats`, { status });
  await click(await xpath(`//button[normalize-space(.)='Stop' and not(@disabled)]`));
  log(`${label}: stop clicked`);
  const done = await waitFor(
    "saved panel",
    () => exec(`const p = document.querySelector('.rec-panel--done'); return p ? { title: p.querySelector('.muted')?.textContent, stats: [...p.querySelectorAll('dd')].map(d => d.textContent) } : (document.querySelector('.rec-panel--failed')?.textContent ?? null);`),
    90_000,
  );
  if (typeof done === "string") throw new Error(`${label} recording failed: ${done}`);
  log(`${label}: saved`, done);
  if (label === "display") await shot("saved");
  await click(await button("New recording"));
  return done;
}

async function main() {
  if (!APP) throw new Error("usage: e2e.mjs <app-binary>");
  const session = await wd("POST", "/session", { capabilities: { alwaysMatch: { "tauri:options": { application: APP } } } });
  sid = session.sessionId;
  log("session", { sid });

  // Record page with real display thumbnails.
  await css(".source-card[aria-checked='true'] .source-card__thumb img", 60_000);
  await waitFor("capabilities", () => exec(`return !!document.querySelector('.cap-summary');`), 60_000);
  const caps = await exec(`return [...document.querySelectorAll('.cap-summary .co-badge')].map(b => b.textContent);`);
  log("record page ready", { caps });
  await shot("record");

  const display = await recordFlow("display", { pause: true });
  summary.displayRecording = display;

  // Screenshot button.
  await click(await button("Screenshot (PNG)"));
  await waitFor("screenshot toast", () => exec(`return [...document.querySelectorAll('.co-toast__title')].some(t => t.textContent === 'Screenshot saved');`), 30_000);
  log("screenshot taken");

  // Library: items, validation badges, preview via asset protocol.
  await go("#/library");
  await waitFor("library items", () => exec(`return document.querySelectorAll('.media-card').length >= 2;`), 20_000);
  const items = await exec(`return [...document.querySelectorAll('.media-card')].map(c => ({ title: c.querySelector('.media-card__title')?.textContent, meta: c.querySelector('.media-card__meta')?.textContent, badge: c.querySelector('.co-badge')?.textContent, kind: c.querySelector('.media-thumb__kind')?.textContent, thumb: !!c.querySelector('.media-thumb img') }));`);
  log("library", { items });
  await sleep(800);
  await shot("library");
  await click(await css(".media-card .media-thumb__duration"));
  const video = await waitFor(
    "video metadata via asset protocol",
    () => exec(`const v = document.querySelector('.preview video'); return v && v.readyState >= 1 ? { duration: v.duration, width: v.videoWidth, height: v.videoHeight, src: v.currentSrc.slice(0, 60) } : null;`),
    20_000,
  ).catch((e) => ({ error: String(e) }));
  log("preview", { video });
  await shot("library-preview");
  await key("\uE00C"); // Escape
  await waitFor("preview closed", () => exec(`return !document.querySelector(".co-dialog");`), 5000);
  await go("#/library");
  await click(await css("button[aria-label='List view']"));
  await shot("library-list");
  await click(await css("button[aria-label='Grid view']"));

  await go("#/settings");
  await waitFor("diagnostics", () => exec(`return document.querySelector('.diag-list') ? [...document.querySelectorAll('.diag-list dd')].map(d => d.textContent) : null;`), 20_000).then((d) => log("diagnostics", { d }));
  await shot("settings");
  await exec(`document.querySelector('.app-main').scrollTo(0, 99999); return true;`);
  await shot("settings-permissions");

  await go("#/devices");
  await waitFor("hub card", () => exec(`return !!document.querySelector("[role='switch']");`), 10_000);
  await shot("devices");

  // Region selection on the real overlay window.
  await go("#/");
  await click(await css("[role='tab'][id='source-tab-region']"));
  await click(await button("Select region"));
  const mainHandle = await wd("GET", s("/window"));
  const regionHandle = await waitFor("region window", async () => (await wd("GET", s("/window/handles"))).find((h) => h !== mainHandle), 15_000);
  await wd("POST", s("/window"), { handle: regionHandle });
  await waitFor("frozen frame", () => exec(`const r = document.querySelector('.region-root'); return r && r.style.backgroundImage.startsWith('url(');`), 20_000);
  log("region overlay shown");
  await wd("POST", s("/actions"), {
    actions: [
      {
        type: "pointer",
        id: "mouse",
        parameters: { pointerType: "mouse" },
        actions: [
          { type: "pointerMove", x: 420, y: 260, origin: "viewport" },
          { type: "pointerDown", button: 0 },
          { type: "pointerMove", x: 900, y: 560, origin: "viewport", duration: 200 },
          { type: "pointerUp", button: 0 },
        ],
      },
    ],
  });
  await sleep(300);
  const size = await exec(`return document.querySelector('.region-size')?.textContent ?? null;`);
  log("region dragged", { size });
  await shot("region", { root: true });
  // Enter confirms the selection and closes the region window, so WebKitWebDriver may answer
  // the key action with "no such window" — that is the expected outcome, not a failure.
  try {
    await key("\uE007"); // Enter
  } catch (e) {
    if (!String(e).includes("no such window")) throw e;
    log("region window closed on confirm");
  }
  await wd("POST", s("/window"), { handle: mainHandle });
  const regionText = await waitFor("region summary", () => exec(`return document.querySelector('.region-summary__size')?.textContent ?? null;`), 15_000);
  log("region selected", { regionText });
  await shot("record-region");
  summary.regionRecording = await recordFlow("region", { pause: false });

  await wd("DELETE", s(""));
  console.log(JSON.stringify(summary, null, 2));
}

main().catch(async (e) => {
  console.error(`[e2e] FAILED: ${e instanceof Error ? e.stack : e}`);
  try {
    await shot("failure", { root: true });
  } catch {
    /* ignore */
  }
  if (sid) await wd("DELETE", s("")).catch(() => undefined);
  console.log(JSON.stringify({ ...summary, failed: String(e) }, null, 2));
  process.exit(1);
});
