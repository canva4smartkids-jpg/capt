#!/usr/bin/env node
// Packages apps/tv-web/dist as an LG webOS TV web app (.ipk) with @webos-tools/cli (ares-package),
// then inspects the produced ipk (an `ar` archive: debian-binary, control.tar.gz, data.tar.gz)
// and copies it to artifacts/webos/.
//
//   node scripts/build.mjs               build tv-web if needed, package, verify, copy
//   node scripts/build.mjs --check-only  validate appinfo.json and icons only (used by `pnpm test`)
import { execFileSync } from "node:child_process";
import { copyFileSync, cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, "..");
const repo = resolve(appDir, "../..");
const webDist = join(repo, "apps/tv-web/dist");
const work = join(appDir, "build");
const stage = join(work, "app");
const out = join(work, "out");
const artifacts = join(repo, "artifacts/webos");
const checkOnly = process.argv.includes("--check-only");

function fail(msg) {
  console.error(`lg-tv: ${msg}`);
  process.exit(1);
}

function pngSize(path) {
  const b = readFileSync(path);
  if (b.length < 24 || b.toString("ascii", 1, 4) !== "PNG") fail(`${path} is not a PNG`);
  return { width: b.readUInt32BE(16), height: b.readUInt32BE(20) };
}

// ---- validate the manifest -------------------------------------------------------------
const appinfo = JSON.parse(readFileSync(join(appDir, "appinfo.json"), "utf8"));
const tvWebPkg = JSON.parse(readFileSync(join(repo, "apps/tv-web/package.json"), "utf8"));
for (const k of ["id", "version", "vendor", "type", "main", "title", "icon", "largeIcon", "splashBackground"]) {
  if (!appinfo[k]) fail(`appinfo.json is missing "${k}"`);
}
if (appinfo.id !== "dev.captureone.tv") fail(`unexpected app id ${appinfo.id}`);
if (appinfo.type !== "web") fail(`appinfo.type must be "web"`);
if (appinfo.disableBackHistoryAPI !== true) fail("disableBackHistoryAPI must be true (the app handles Back = keyCode 461 itself)");
if (!Array.isArray(appinfo.requiredPermissions)) fail("requiredPermissions must be an array");
if (!/^\d+\.\d+\.\d+$/.test(appinfo.version)) fail("version must be x.y.z");
if (appinfo.version !== tvWebPkg.version) fail(`appinfo version ${appinfo.version} != tv-web ${tvWebPkg.version}`);
const expected = { [appinfo.icon]: [80, 80], [appinfo.largeIcon]: [130, 130], [appinfo.splashBackground]: [1920, 1080] };
for (const [file, [w, h]] of Object.entries(expected)) {
  const s = pngSize(join(appDir, "assets", file));
  if (s.width !== w || s.height !== h) fail(`${file} is ${s.width}x${s.height}, expected ${w}x${h}`);
}
console.log("lg-tv: appinfo.json and icons OK");
if (checkOnly) process.exit(0);

// ---- assemble ------------------------------------------------------------------------------
if (!existsSync(join(webDist, "index.html"))) {
  console.log("lg-tv: building apps/tv-web …");
  execFileSync("pnpm", ["--filter", "@captureone/tv-web", "build"], { cwd: repo, stdio: "inherit" });
}
rmSync(work, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
cpSync(webDist, stage, { recursive: true });
// The Tizen webapis placeholder is meaningless on webOS.
const indexPath = join(stage, "index.html");
writeFileSync(indexPath, readFileSync(indexPath, "utf8").replace("<!--TIZEN_WEBAPIS-->", ""));
writeFileSync(join(stage, "appinfo.json"), JSON.stringify(appinfo, null, 2) + "\n");
for (const f of Object.keys(expected)) copyFileSync(join(appDir, "assets", f), join(stage, f));

// ---- package with ares-package (@webos-tools/cli) -----------------------------------------
mkdirSync(out, { recursive: true });
const ares = join(appDir, "node_modules/.bin/ares-package");
if (!existsSync(ares)) fail("ares-package not found; run pnpm install");
// --no-minify: the bundle is already minified ES2015; re-minifying is unnecessary.
execFileSync(ares, [stage, "--outdir", out, "--no-minify"], { stdio: "inherit" });
const ipkName = readdirSync(out).find((f) => f.endsWith(".ipk"));
if (!ipkName) fail("ares-package produced no .ipk");
const ipk = join(out, ipkName);

// ---- inspect the ipk (ar archive) ----------------------------------------------------------
function readAr(path) {
  const buf = readFileSync(path);
  if (buf.toString("ascii", 0, 8) !== "!<arch>\n") fail(`${path} is not an ar archive`);
  const members = [];
  let off = 8;
  while (off + 60 <= buf.length) {
    const name = buf.toString("ascii", off, off + 16).trim().replace(/\/$/, "");
    const size = parseInt(buf.toString("ascii", off + 48, off + 58).trim(), 10);
    const data = buf.subarray(off + 60, off + 60 + size);
    members.push({ name, size, data });
    off += 60 + size + (size % 2);
  }
  return members;
}
const members = readAr(ipk);
const names = members.map((m) => m.name);
console.log(`lg-tv: ${ipkName} members: ${names.join(", ")}`);
if (names[0] !== "debian-binary") fail("first ipk member must be debian-binary");
const control = members.find((m) => m.name.startsWith("control.tar"));
const data = members.find((m) => m.name.startsWith("data.tar"));
if (!control || !data) fail("ipk lacks control/data tarballs");
const tmp = join(work, "inspect");
mkdirSync(tmp, { recursive: true });
writeFileSync(join(tmp, control.name), control.data);
writeFileSync(join(tmp, data.name), data.data);
const list = (f) => execFileSync("tar", ["-tzf", join(tmp, f)], { encoding: "utf8" }).split("\n").filter(Boolean);
const dataFiles = list(data.name);
const controlFiles = list(control.name);
execFileSync("tar", ["-xzf", join(tmp, control.name), "-C", tmp]);
const controlText = readFileSync(join(tmp, controlFiles.find((f) => f.endsWith("control"))), "utf8");
const base = `usr/palm/applications/${appinfo.id}/`;
for (const f of ["index.html", "appinfo.json", "assets/app.js", "assets/style.css", "icon.png", "largeIcon.png", "splash.png"]) {
  if (!dataFiles.some((d) => d.replace(/^\.\//, "") === base + f)) fail(`ipk data is missing ${base + f}`);
}
if (!/Package:\s*dev\.captureone\.tv/.test(controlText)) fail("control file has the wrong package name");
if (!new RegExp(`Version:\\s*${appinfo.version.replace(/\./g, "\\.")}`).test(controlText)) fail("control file has the wrong version");
console.log(`lg-tv: control:\n${controlText.trim().replace(/^/gm, "   ")}`);
console.log(`lg-tv: data.tar contains ${dataFiles.filter((f) => !f.endsWith("/")).length} files under ${base}`);

mkdirSync(artifacts, { recursive: true });
copyFileSync(ipk, join(artifacts, ipkName));
writeFileSync(
  join(artifacts, `${ipkName}.contents.txt`),
  [`# ${ipkName}`, `ar members: ${names.join(", ")}`, "", "## control", controlText.trim(), "", "## data", ...dataFiles].join("\n") + "\n",
);
console.log(`lg-tv: wrote ${join(artifacts, ipkName)}`);
