#!/usr/bin/env node
// Packages apps/tv-web/dist as a Samsung Tizen TV web app (.wgt) and copies it to artifacts/tizen/.
//
//   node scripts/build.mjs                 auto: sign with the Tizen CLI in Docker if available,
//                                          otherwise build an UNSIGNED .wgt
//   node scripts/build.mjs --signer=docker require Docker signing (pulls vitalets/tizen-webos-sdk
//                                          with exponential backoff; Docker Hub rate-limits)
//   node scripts/build.mjs --signer=none   unsigned .wgt only
//   node scripts/build.mjs --check-only    validate config.xml and the icon only
//
// Signing here uses a throwaway author certificate + Tizen's public default distributor
// certificate. That is enough for the emulator and to validate the package, but retail Samsung
// TVs only install packages signed with a Samsung-issued certificate (Samsung account, Tizen
// Studio Certificate Manager, distributor certificate bound to the TV's DUID).
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { readZip, writeZip } from "./zip.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const appDir = resolve(here, "..");
const repo = resolve(appDir, "../..");
const webDist = join(repo, "apps/tv-web/dist");
const work = join(appDir, "build");
const stage = join(work, "wgt-src");
const artifacts = join(repo, "artifacts/tizen");
const IMAGE = process.env.TIZEN_SDK_IMAGE || "vitalets/tizen-webos-sdk:latest";
const arg = (name) => process.argv.find((a) => a.startsWith(`--${name}=`))?.split("=")[1];
const signer = arg("signer") || "auto";
const checkOnly = process.argv.includes("--check-only");

function fail(msg) {
  console.error(`samsung-tv: ${msg}`);
  process.exit(1);
}

// ---- validate config.xml -------------------------------------------------------------------
const config = readFileSync(join(appDir, "config.xml"), "utf8");
const tvWebVersion = JSON.parse(readFileSync(join(repo, "apps/tv-web/package.json"), "utf8")).version;
const attr = (re) => (re.exec(config) || [])[1];
const pkgId = attr(/<tizen:application[^>]*\bpackage="([^"]+)"/);
const appId = attr(/<tizen:application[^>]*\bid="([^"]+)"/);
const version = attr(/<widget[^>]*\bversion="([^"]+)"/);
if (!pkgId || !/^[0-9A-Za-z]{10}$/.test(pkgId)) fail(`tizen:application package must be 10 alphanumerics, got ${pkgId}`);
if (!appId || !appId.startsWith(`${pkgId}.`)) fail(`tizen:application id must be <package>.<name>, got ${appId}`);
if (version !== tvWebVersion) fail(`config.xml version ${version} != tv-web ${tvWebVersion}`);
if (!/<tizen:profile name="tv-samsung"\s*\/>/.test(config)) fail('missing <tizen:profile name="tv-samsung"/>');
for (const p of ["http://tizen.org/privilege/internet", "http://tizen.org/privilege/tv.inputdevice", "http://developer.samsung.com/privilege/productinfo"]) {
  if (!config.includes(`<tizen:privilege name="${p}"/>`)) fail(`missing privilege ${p}`);
}
if (!/screen-orientation="landscape"/.test(config)) fail("screen-orientation must be landscape");
if (!/<tizen:content-security-policy>[^<]*connect-src[^<]*ws:/.test(config)) fail("CSP must allow LAN http/ws in connect-src");
if (!/<content src="index.html"\s*\/>/.test(config)) fail("content must be index.html");
const icon = readFileSync(join(appDir, "icon.png"));
if (icon.toString("ascii", 1, 4) !== "PNG") fail("icon.png is not a PNG");
console.log(`samsung-tv: config.xml OK (package ${pkgId}, app ${appId}, version ${version}); icon ${icon.readUInt32BE(16)}x${icon.readUInt32BE(20)}`);
if (checkOnly) process.exit(0);

// ---- stage ---------------------------------------------------------------------------------
if (!existsSync(join(webDist, "index.html"))) {
  console.log("samsung-tv: building apps/tv-web …");
  execFileSync("pnpm", ["--filter", "@captureone/tv-web", "build"], { cwd: repo, stdio: "inherit" });
}
rmSync(work, { recursive: true, force: true });
mkdirSync(stage, { recursive: true });
cpSync(webDist, stage, { recursive: true });
const indexPath = join(stage, "index.html");
// Samsung product APIs (webapis.productinfo) are provided by the TV runtime at $WEBAPIS.
const html = readFileSync(indexPath, "utf8");
if (!html.includes("<!--TIZEN_WEBAPIS-->")) fail("tv-web index.html lacks the <!--TIZEN_WEBAPIS--> placeholder");
writeFileSync(indexPath, html.replace("<!--TIZEN_WEBAPIS-->", '<script src="$WEBAPIS/webapis/webapis.js"></script>'));
copyFileSync(join(appDir, "config.xml"), join(stage, "config.xml"));
copyFileSync(join(appDir, "icon.png"), join(stage, "icon.png"));

function listFiles(dir) {
  const out = [];
  for (const name of readdirSync(dir).sort()) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) out.push(...listFiles(p));
    else out.push(relative(stage, p).split("\\").join("/"));
  }
  return out;
}
const appFiles = listFiles(stage);

// ---- sign with the Tizen CLI in Docker -----------------------------------------------------
function docker(args, opts = {}) {
  return spawnSync("docker", args, { encoding: "utf8", ...opts });
}
function ensureImage() {
  if (docker(["version", "--format", "{{.Server.Version}}"]).status !== 0) return false;
  if (docker(["image", "inspect", IMAGE]).status === 0) return true;
  let delay = 15;
  for (let i = 1; i <= 8; i++) {
    console.log(`samsung-tv: docker pull ${IMAGE} (attempt ${i})`);
    if (docker(["pull", IMAGE], { stdio: "inherit" }).status === 0) return true;
    console.log(`samsung-tv: pull failed (Docker Hub rate limit?), retrying in ${delay}s`);
    execFileSync("sleep", [String(delay)]);
    delay = Math.min(delay * 2, 600);
  }
  return false;
}

let signed = false;
if (signer !== "none") {
  if (ensureImage()) {
    const r = docker(
      ["run", "--rm", "-i", "-v", `${stage}:/work/src`, "-v", `${here}/tizen-sign.sh:/usr/local/bin/tizen-sign.sh:ro`, IMAGE, "bash", "-lc", "tizen-sign.sh /work/src"],
      { stdio: ["pipe", "inherit", "inherit"] },
    );
    signed = r.status === 0 && existsSync(join(stage, "CaptureOne.wgt"));
    if (!signed && signer === "docker") fail("Tizen CLI signing failed");
  } else if (signer === "docker") {
    fail(`Docker image ${IMAGE} is not available`);
  } else {
    console.log("samsung-tv: Docker/Tizen CLI unavailable; building an UNSIGNED .wgt");
  }
}

let wgt;
if (signed) {
  wgt = readFileSync(join(stage, "CaptureOne.wgt"));
} else {
  wgt = writeZip(appFiles.map((name) => ({ name, data: readFileSync(join(stage, name)) })));
}

// ---- inspect / verify ----------------------------------------------------------------------
const entries = readZip(wgt);
for (const f of ["config.xml", "index.html", "icon.png", "assets/app.js", "assets/style.css"]) {
  if (!entries.has(f)) fail(`.wgt lacks ${f}`);
}
if (!entries.get("index.html").toString("utf8").includes('<script src="$WEBAPIS/webapis/webapis.js"></script>')) fail("index.html in .wgt lacks webapis.js");
const verifyDigests = (sigName) => {
  const xml = entries.get(sigName)?.toString("utf8");
  if (!xml) fail(`${sigName} missing from signed package`);
  const refs = [...xml.matchAll(/<Reference URI="([^"#][^"]*)">\s*<DigestMethod Algorithm="[^"]*#(sha256|sha512)"><\/DigestMethod>\s*<DigestValue>([^<]+)<\/DigestValue>/g)];
  if (refs.length === 0) fail(`${sigName} has no file references`);
  for (const [, uri, alg, value] of refs) {
    const name = decodeURIComponent(uri);
    const data = entries.get(name);
    if (!data) fail(`${sigName} references missing file ${name}`);
    const digest = createHash(alg).update(data).digest("base64");
    if (digest !== value.replace(/\s+/g, "")) fail(`${sigName}: digest mismatch for ${name}`);
  }
  return refs.map((r) => decodeURIComponent(r[1]));
};
let summary;
if (signed) {
  const authorRefs = verifyDigests("author-signature.xml");
  const distRefs = verifyDigests("signature1.xml");
  for (const f of appFiles) {
    if (!authorRefs.includes(f)) fail(`author signature does not cover ${f}`);
    if (!distRefs.includes(f)) fail(`distributor signature does not cover ${f}`);
  }
  if (!distRefs.includes("author-signature.xml")) fail("distributor signature does not cover author-signature.xml");
  summary = `signed (throwaway author cert + Tizen default distributor cert); ${authorRefs.length} author and ${distRefs.length} distributor digests verified`;
} else {
  summary = "UNSIGNED (Tizen CLI not available): sign with Tizen Studio + a Samsung certificate before installing on a TV";
}

mkdirSync(artifacts, { recursive: true });
const outName = `CaptureOne-${version}${signed ? "" : "-unsigned"}.wgt`;
writeFileSync(join(artifacts, outName), wgt);
writeFileSync(join(artifacts, `${outName}.contents.txt`), [`# ${outName}`, summary, "", ...[...entries.keys()].sort().map((n) => `${n}\t${entries.get(n).length}`)].join("\n") + "\n");
console.log(`samsung-tv: ${outName}: ${[...entries.keys()].sort().join(", ")}`);
console.log(`samsung-tv: ${summary}`);
console.log(`samsung-tv: wrote ${join(artifacts, outName)}`);
