#!/usr/bin/env node
// End-to-end smoke test of the BUILT TV bundle (dist/index.html, loaded from file:// as on a TV)
// in headless Chromium, operated with remote keys only (arrows / OK / digits / Back / media
// keys), against a real companion-hub (`demo_hub` example) serving real media files.
//
//   node scripts/smoke.mjs [--hub-bin <path to demo_hub>] [--media <dir>] [--out <screenshot dir>]
//
// Requires Playwright (`require('playwright')`), ffmpeg/ffprobe (to make sample media when
// --media is not given) and a built demo_hub (`cargo build -p companion-hub --example demo_hub`).
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, "..");
const repo = resolve(appDir, "../..");
const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 ? process.argv[i + 1] : def;
};
const outDir = resolve(arg("out", join(repo, "docs/screenshots")));
const hubBin = resolve(arg("hub-bin", process.env.DEMO_HUB_BIN || "/tmp/cargo-target/tv/debug/examples/demo_hub"));
const require = createRequire(import.meta.url);
let playwright;
try {
  playwright = require("playwright");
} catch {
  const globalRoot = execFileSync("npm", ["root", "-g"], { encoding: "utf8" }).trim();
  playwright = createRequire(join(globalRoot, "noop.js"))("playwright");
}

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok: !!ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
  if (!ok) throw new Error(`check failed: ${name} ${detail}`);
};

// ---- sample media (real files, validated with ffprobe) ------------------------------------
let mediaDir = arg("media");
let tmpMedia = null;
if (!mediaDir) {
  tmpMedia = mediaDir = mkdtempSync(join(tmpdir(), "captureone-tv-smoke-"));
  execFileSync("ffmpeg", ["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", "-f", "lavfi", "-i", "sine=frequency=440:sample_rate=48000", "-t", "24", "-c:v", "libx264", "-profile:v", "main", "-pix_fmt", "yuv420p", "-b:v", "600k", "-g", "60", "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart", join(mediaDir, "Demo recording.mp4")]);
  execFileSync("ffmpeg", ["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc=size=1920x1080:rate=1", "-frames:v", "1", join(mediaDir, "Demo screenshot.png")]);
}
const probe = execFileSync("ffprobe", ["-v", "error", "-show_entries", "format=duration:stream=codec_name,width,height", "-of", "json", join(mediaDir, "Demo recording.mp4")], { encoding: "utf8" });
const probed = JSON.parse(probe);
check("sample MP4 validated by ffprobe", probed.streams.some((s) => s.codec_name === "h264" && s.width === 1280) && Number(probed.format.duration) > 20, `${probed.streams.map((s) => s.codec_name).join("+")} ${probed.format.duration}s`);

// ---- start the hub ----------------------------------------------------------------------------
if (!existsSync(hubBin)) throw new Error(`demo_hub not found at ${hubBin}; build it with cargo build -p companion-hub --example demo_hub`);
const stateDir = mkdtempSync(join(tmpdir(), "captureone-hub-state-"));
const hub = spawn(hubBin, ["--media-dir", mediaDir, "--bind", "127.0.0.1", "--port", "0", "--state-dir", stateDir, "--no-mdns"], { stdio: ["ignore", "pipe", "inherit"] });
const hubInfo = await new Promise((resolveInfo, reject) => {
  let buf = "";
  const timer = setTimeout(() => reject(new Error("demo_hub did not start")), 30000);
  hub.stdout.on("data", (d) => {
    buf += d.toString();
    const port = /HUB_PORT=(\d+)/.exec(buf);
    const code = /PAIRING_CODE=(\d{6})/.exec(buf);
    const recs = /RECORDINGS=(\d+)/.exec(buf);
    if (port && code && recs) {
      clearTimeout(timer);
      resolveInfo({ port: Number(port[1]), code: code[1], recordings: Number(recs[1]) });
    }
  });
  hub.on("exit", (c) => reject(new Error(`demo_hub exited with ${c}`)));
});
check("companion-hub (demo_hub) started", hubInfo.recordings === 2, `127.0.0.1:${hubInfo.port}, ${hubInfo.recordings} recordings`);

mkdirSync(outDir, { recursive: true });
const indexUrl = pathToFileURL(join(appDir, "dist/index.html")).href;
const browser = await playwright.chromium.launch();
// One context: pages share the file:// origin's localStorage (the remembered pairing).
const context = await browser.newContext();
const errors = [];

async function newPage(viewport) {
  const page = await context.newPage();
  await page.setViewportSize(viewport);
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return page;
}

/** Moves focus to `selector` using only arrow keys, steering by geometry. */
async function navigateTo(page, selector, maxSteps = 40) {
  for (let i = 0; i < maxSteps; i++) {
    const step = await page.evaluate((sel) => {
      const target = document.querySelector(sel);
      const cur = document.activeElement;
      if (!target) return { error: `no ${sel}` };
      if (cur === target) return { done: true };
      const t = target.getBoundingClientRect();
      const c = cur && cur !== document.body ? cur.getBoundingClientRect() : { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 };
      const overlapY = !(t.bottom <= c.top || t.top >= c.bottom);
      const tx = t.left + t.width / 2, ty = t.top + t.height / 2, cx = c.left + c.width / 2, cy = c.top + c.height / 2;
      if (!overlapY) return { key: ty < cy ? "ArrowUp" : "ArrowDown", alt: tx < cx ? "ArrowLeft" : "ArrowRight" };
      return { key: tx < cx ? "ArrowLeft" : "ArrowRight", alt: ty < cy ? "ArrowUp" : "ArrowDown" };
    }, selector);
    if (step.error) throw new Error(step.error);
    if (step.done) return i;
    const before = await page.evaluate(() => document.activeElement && document.activeElement.outerHTML);
    await page.keyboard.press(step.key);
    const after = await page.evaluate(() => document.activeElement && document.activeElement.outerHTML);
    if (before === after) await page.keyboard.press(step.alt);
  }
  throw new Error(`could not reach ${selector} with arrow keys`);
}

const focusedId = (page) => page.evaluate(() => document.activeElement && (document.activeElement.id || document.activeElement.getAttribute("data-tab") || document.activeElement.textContent));
const shot = async (page, name) => {
  await page.waitForTimeout(350); // let focus transitions settle
  await page.screenshot({ path: join(outDir, name) });
  console.log(`  screenshot ${join(outDir, name)}`);
};

try {
  const page = await newPage({ width: 1920, height: 1080 });
  await page.goto(indexUrl);
  await page.waitForSelector("#cap-matrix .cap-row");
  check("home renders with capability matrix", (await page.locator("#cap-matrix .cap-row").count()) >= 10);
  const levels = await page.$$eval("#cap-matrix .cap-row", (rows) => Object.fromEntries(rows.map((r) => [r.getAttribute("data-feature"), r.querySelector("[data-level]").getAttribute("data-level")])));
  check("TV never claims capture", levels.displayCapture === "external-hardware" && levels.videoRecording === "companion-only", JSON.stringify(levels));
  check("initial focus on primary action", (await focusedId(page)) === "home-primary");
  await shot(page, "tv-home.png");

  // Scroll through the capability matrix with the remote.
  for (let i = 0; i < 6; i++) await page.keyboard.press("ArrowDown");
  const scrolled = await page.evaluate(() => document.getElementById("content").scrollTop);
  check("D-pad down scrolls to focused matrix rows", scrolled > 0 && (await page.evaluate(() => document.activeElement.classList.contains("cap-row"))), `scrollTop=${scrolled}`);
  await page.keyboard.press("Escape"); // Back on home: nothing to go back to (browser adapter exit is a no-op)

  // ---- Connect: address and code with number keys + on-screen keypad ----
  await navigateTo(page, "[data-tab='connect']");
  await page.keyboard.press("Enter");
  await page.waitForSelector("#keypad");
  check("Connect opens with the address field focused", (await focusedId(page)) === "field-address");
  const typeAddress = async (text) => {
    for (const ch of text) {
      if (ch === "." || ch === ":") {
        await navigateTo(page, `[data-key='${ch}']`);
        await page.keyboard.press("Enter");
      } else {
        await page.keyboard.press(ch); // remote number keys
      }
    }
  };
  await typeAddress(`127.0.0.1:${hubInfo.port}`);
  const addr = await page.textContent("#field-address .field-value");
  check("address entered via keypad + number keys", addr === `127.0.0.1:${hubInfo.port}`, addr);
  await navigateTo(page, "#field-code");
  await page.keyboard.press("Enter");
  // A wrong code first: the hub must refuse it.
  const wrongCode = hubInfo.code === "000000" ? "111111" : "000000";
  for (const ch of wrongCode) await page.keyboard.press(ch);
  check("full code moves focus to Pair", (await focusedId(page)) === "pair-button");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => /Wrong code/.test(document.getElementById("connect-status").textContent));
  check("wrong code is rejected by the hub", true, await page.textContent("#connect-status"));
  await navigateTo(page, "#field-code");
  await page.keyboard.press("Enter");
  for (const ch of hubInfo.code) await page.keyboard.press(ch);
  await shot(page, "tv-connect.png");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => /Paired with/.test(document.getElementById("connect-status").textContent), null, { timeout: 15000 });
  const token = await page.evaluate(() => JSON.parse(localStorage.getItem("captureone.tv.connection.v1")).token);
  check("paired; token stored", typeof token === "string" && token.length === 64, `${token.length}-char token`);
  await page.waitForFunction(() => /Connected/.test(document.getElementById("conn-badge").textContent), null, { timeout: 10000 });
  check("live WebSocket connection to the hub", true, await page.textContent("#conn-badge"));
  await shot(page, "tv-connect-paired.png");

  // ---- Recordings + playback ----
  await navigateTo(page, "[data-tab='recordings']");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".media-card");
  const cards = await page.$$eval(".media-card", (els) => els.map((e) => ({ id: e.getAttribute("data-media-id"), title: e.querySelector(".media-title").textContent, meta: e.querySelector(".media-meta").textContent })));
  check("recordings listed from the hub", cards.length === 2, JSON.stringify(cards));
  await page.waitForFunction(() => [...document.querySelectorAll(".media-card img")].every((i) => i.complete && i.naturalWidth > 0), null, { timeout: 10000 });
  check("thumbnails load through ?token= URLs", true);
  await shot(page, "tv-recordings.png");
  const videoCard = cards.findIndex((c) => /recording/i.test(c.title));
  await navigateTo(page, `.media-card[data-media-id='${cards[videoCard].id}']`);
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => {
    const p = document.getElementById("player");
    return p && p.getAttribute("data-state") === "playing" && Number(p.getAttribute("data-current-time")) > 0.8;
  }, null, { timeout: 20000 });
  const vinfo = await page.evaluate(() => {
    const v = document.getElementById("player-video");
    return { w: v.videoWidth, h: v.videoHeight, t: v.currentTime, d: v.duration, src: v.currentSrc.replace(/token=[0-9a-f]+/, "token=…") };
  });
  check("video plays full-screen from the hub", vinfo.w === 1280 && vinfo.h === 720 && vinfo.t > 0.8 && Math.abs(vinfo.d - 24) < 0.5, JSON.stringify(vinfo));
  await page.keyboard.press("ArrowUp"); // Up/Down show the transport overlay
  await page.waitForFunction(() => document.querySelector(".player-chrome").classList.contains("player-chrome-show"));
  await shot(page, "tv-player.png");
  await page.keyboard.press("ArrowRight"); // +10 s seek -> HTTP range request
  await page.waitForFunction((t0) => document.getElementById("player-video").currentTime > t0 + 8, vinfo.t, { timeout: 10000 });
  const t1 = await page.evaluate(() => document.getElementById("player-video").currentTime);
  check("remote seek (+10 s) works (range requests)", t1 > vinfo.t + 8, `${vinfo.t.toFixed(1)}s -> ${t1.toFixed(1)}s`);
  await page.keyboard.press("Enter"); // OK = pause
  await page.waitForFunction(() => document.getElementById("player").getAttribute("data-state") === "paused");
  check("OK toggles pause", true);
  await page.keyboard.press("Escape"); // Back closes the player
  await page.waitForFunction(() => !document.getElementById("player"));
  check("Back closes the player and restores focus", (await page.evaluate(() => document.activeElement.getAttribute("data-media-id"))) === cards[videoCard].id);

  // ---- Record remotely ----
  await navigateTo(page, "[data-tab='remote']");
  await page.keyboard.press("Enter");
  await page.waitForSelector("#cmd-start");
  check("Record remotely focuses Start", (await focusedId(page)) === "cmd-start");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.getElementById("rec-state").getAttribute("data-state") === "recording", null, { timeout: 10000 });
  await page.waitForFunction(() => /SIMULATED/.test(document.querySelector(".rec-status").textContent), null, { timeout: 5000 });
  check("start command reaches the hub; live status over WebSocket", true, (await page.textContent(".rec-status")).slice(0, 120));
  await page.waitForTimeout(1200);
  await shot(page, "tv-remote.png");
  await page.keyboard.press("Pause"); // media key (keyCode 19)
  await page.waitForFunction(() => document.getElementById("rec-state").getAttribute("data-state") === "paused", null, { timeout: 10000 });
  check("Pause media key pauses the desktop recording", true);
  await navigateTo(page, "#cmd-stop");
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.getElementById("rec-state").getAttribute("data-state") === "idle", null, { timeout: 10000 });
  check("Stop returns to idle; hub's honest result shown", true, await page.textContent("#remote-result"));

  // ---- How to record this TV ----
  await navigateTo(page, "[data-tab='howto']");
  await page.keyboard.press("Enter");
  await page.waitForSelector(".guide");
  check("guide mentions HDCP", /HDCP/.test(await page.textContent("#content")));
  await shot(page, "tv-howto.png");
  await page.keyboard.press("Escape");
  await page.waitForFunction(() => document.getElementById("cap-matrix"));
  check("Back from a screen returns Home", true);

  // ---- platform Back keys and 720p scaling (same file:// origin keeps the pairing) ----
  for (const [platform, keyCode] of [["webos", 461], ["tizen", 10009]]) {
    const p = await newPage({ width: 1280, height: 720 });
    await p.goto(`${indexUrl}?platform=${platform}`);
    await p.waitForSelector("#cap-matrix .cap-row");
    const plat = await p.evaluate(() => document.documentElement.getAttribute("data-platform"));
    await navigateTo(p, "[data-tab='recordings']");
    await p.keyboard.press("Enter");
    await p.waitForSelector(".media-card");
    await p.evaluate((code) => {
      const e = new KeyboardEvent("keydown", { bubbles: true, cancelable: true });
      Object.defineProperty(e, "keyCode", { get: () => code });
      document.activeElement.dispatchEvent(e);
    }, keyCode);
    await p.waitForFunction(() => document.getElementById("cap-matrix"));
    check(`${platform}: Back keyCode ${keyCode} returns Home`, plat === platform);
    if (platform === "webos") {
      const stage = await p.evaluate(() => document.getElementById("stage").getBoundingClientRect().width);
      check("1920x1080 layout scales to a 1280x720 viewport", Math.abs(stage - 1280) < 1, `stage width ${stage}px`);
      await shot(p, "tv-home-720p.png");
    }
    await p.close();
  }
  // Chromium logs the intentional wrong-code pairing attempt as a failed (401) resource load.
  const expected401 = errors.filter((e) => /status of 401/.test(e));
  const relevantErrors = errors.filter((e) => !/status of 401/.test(e) && !/favicon/.test(e));
  check("no page errors (only the intentional wrong-code 401)", relevantErrors.length === 0 && expected401.length === 1, relevantErrors.join(" | ") || `${expected401.length} expected 401`);
} finally {
  await browser.close();
  hub.kill("SIGTERM");
  rmSync(stateDir, { recursive: true, force: true });
  if (tmpMedia) rmSync(tmpMedia, { recursive: true, force: true });
}
console.log(`\nTV smoke test: ${results.filter((r) => r.ok).length}/${results.length} checks passed`);
