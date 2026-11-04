#!/usr/bin/env node
/**
 * Provides ffmpeg/ffprobe sidecars for release bundles (tauri.release.conf.json ->
 * bundle.externalBin). Writes src-tauri/binaries/{ffmpeg,ffprobe}-<target-triple>[.exe].
 *
 * Licensing
 * ---------
 * CaptureOne ships ONLY LGPL-2.1+ builds of FFmpeg, dynamically or statically linked
 * without any GPL/nonfree component. H.264/H.265 encoding then relies on the operating
 * system's encoders, which FFmpeg wraps under the LGPL:
 *   - Windows: Media Foundation (h264_mf / hevc_mf), plus NVENC/QSV/AMF where present
 *   - macOS:   VideoToolbox (h264_videotoolbox / hevc_videotoolbox)
 * This avoids libx264/libx265 (GPL), which would make the whole distribution GPL.
 * The script refuses any binary whose configuration contains --enable-gpl,
 * --enable-nonfree, libx264 or libx265. On Linux the deb/rpm packages depend on the
 * distribution's ffmpeg package instead (dev builds also use the system ffmpeg).
 * Ship the FFmpeg LGPL license text and a link to the exact source with the release.
 *
 * Usage
 * -----
 *   node scripts/fetch-ffmpeg.mjs [--target <triple>]            # download a prebuilt LGPL build
 *   node scripts/fetch-ffmpeg.mjs --from-source 7.1 [--target …]  # build LGPL ffmpeg (macOS default)
 * Environment:
 *   FFMPEG_URL       archive URL (.zip / .tar.xz / .tar.gz) overriding the default for the target
 *   FFMPEG_ARCHIVE   local archive path (skips the download)
 *   FFMPEG_SHA256    expected SHA-256 of the archive (recommended for reproducible CI)
 */
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { availableParallelism, tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const binDir = resolve(here, "../src-tauri/binaries");

const BTBN = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest";
/** Prebuilt LGPL builds (BtbN publishes explicit "-lgpl" variants). macOS has no LGPL prebuilt: use --from-source. */
const DEFAULT_URLS = {
  "x86_64-pc-windows-msvc": `${BTBN}/ffmpeg-n7.1-latest-win64-lgpl-7.1.zip`,
  "aarch64-pc-windows-msvc": `${BTBN}/ffmpeg-n7.1-latest-winarm64-lgpl-7.1.zip`,
  "x86_64-unknown-linux-gnu": `${BTBN}/ffmpeg-n7.1-latest-linux64-lgpl-7.1.tar.xz`,
  "aarch64-unknown-linux-gnu": `${BTBN}/ffmpeg-n7.1-latest-linuxarm64-lgpl-7.1.tar.xz`,
};

/** Configure flags for an LGPL build that uses the OS encoders. */
const SOURCE_FLAGS = {
  darwin: ["--enable-videotoolbox", "--enable-audiotoolbox"],
  win32: ["--enable-mediafoundation"],
  linux: ["--enable-vaapi"],
};

const FORBIDDEN = [/--enable-gpl\b/, /--enable-nonfree\b/, /libx264/, /libx265/, /--enable-libfdk[-_]aac/];

function arg(name) {
  const i = process.argv.indexOf(name);
  return i > 0 ? process.argv[i + 1] : undefined;
}

function hostTriple() {
  const out = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const m = /host: (\S+)/.exec(out);
  if (!m) throw new Error("cannot determine the Rust host triple (is rustc installed?)");
  return m[1];
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

async function download(url, dest) {
  console.log(`[ffmpeg] downloading ${url}`);
  const res = await fetch(url, { redirect: "follow" });
  if (!res.ok) throw new Error(`download failed: HTTP ${res.status} for ${url}`);
  writeFileSync(dest, Buffer.from(await res.arrayBuffer()));
}

function extract(archive, into) {
  mkdirSync(into, { recursive: true });
  // bsdtar (Windows 10+, macOS) and GNU tar handle .tar.*; bsdtar also handles .zip.
  const isZip = archive.toLowerCase().endsWith(".zip");
  const r = isZip && process.platform === "linux" ? spawnSync("unzip", ["-q", archive, "-d", into], { stdio: "inherit" }) : spawnSync("tar", ["-xf", archive, "-C", into], { stdio: "inherit" });
  if (r.status !== 0) throw new Error(`failed to extract ${archive}`);
}

function findFile(root, name) {
  for (const entry of readdirSync(root)) {
    const p = join(root, entry);
    if (statSync(p).isDirectory()) {
      const found = findFile(p, name);
      if (found) return found;
    } else if (entry === name) {
      return p;
    }
  }
  return null;
}

/** Throws when the binary was configured with GPL/nonfree components. */
export function assertLgplConfiguration(versionOutput) {
  const bad = FORBIDDEN.filter((re) => re.test(versionOutput)).map((re) => re.source);
  if (bad.length > 0) throw new Error(`refusing non-LGPL ffmpeg build (configuration matches: ${bad.join(", ")})`);
}

function verify(binary, target) {
  const host = hostTriple();
  if (host !== target) {
    console.log(`[ffmpeg] cross target ${target}: cannot execute here; licence check deferred to the ${target} CI job`);
    return;
  }
  const out = execFileSync(binary, ["-hide_banner", "-version"], { encoding: "utf8" });
  assertLgplConfiguration(out);
  console.log(`[ffmpeg] ${binary}: ${out.split("\n")[0]} (LGPL configuration verified)`);
}

function install(ffmpeg, ffprobe, target) {
  mkdirSync(binDir, { recursive: true });
  const ext = target.includes("windows") ? ".exe" : "";
  const outs = [];
  for (const [src, name] of [
    [ffmpeg, "ffmpeg"],
    [ffprobe, "ffprobe"],
  ]) {
    if (!ext) chmodSync(src, 0o755);
    verify(src, target); // before installing, so a rejected build never lands in binaries/
    const dest = join(binDir, `${name}-${target}${ext}`);
    copyFileSync(src, dest);
    outs.push(dest);
  }
  console.log(`[ffmpeg] installed:\n  ${outs.join("\n  ")}`);
}

function buildFromSource(version, target, work) {
  const tarball = join(work, `ffmpeg-${version}.tar.xz`);
  return download(`https://ffmpeg.org/releases/ffmpeg-${version}.tar.xz`, tarball).then(() => {
    extract(tarball, work);
    const src = join(work, `ffmpeg-${version}`);
    const prefix = join(work, "out");
    const flags = [
      `--prefix=${prefix}`,
      "--disable-gpl",
      "--disable-nonfree",
      "--disable-doc",
      "--disable-debug",
      "--disable-ffplay",
      "--disable-network",
      "--enable-static",
      "--disable-shared",
      ...(SOURCE_FLAGS[process.platform] ?? []),
    ];
    const run = (cmd, args) => {
      const r = spawnSync(cmd, args, { cwd: src, stdio: "inherit" });
      if (r.status !== 0) throw new Error(`${cmd} ${args.join(" ")} failed`);
    };
    run("./configure", flags);
    run("make", [`-j${Math.max(2, availableParallelism())}`]);
    run("make", ["install"]);
    return { ffmpeg: join(prefix, "bin", "ffmpeg"), ffprobe: join(prefix, "bin", "ffprobe") };
  });
}

async function main() {
  const target = arg("--target") ?? hostTriple();
  const fromSource = arg("--from-source");
  const work = mkdtempSync(join(tmpdir(), "captureone-ffmpeg-"));
  try {
    if (fromSource || (target.includes("apple") && !process.env.FFMPEG_URL && !process.env.FFMPEG_ARCHIVE)) {
      const { ffmpeg, ffprobe } = await buildFromSource(fromSource ?? "7.1", target, work);
      install(ffmpeg, ffprobe, target);
      return;
    }
    let archive = process.env.FFMPEG_ARCHIVE;
    if (!archive) {
      const url = process.env.FFMPEG_URL ?? DEFAULT_URLS[target];
      if (!url) throw new Error(`no default LGPL ffmpeg build for ${target}; set FFMPEG_URL or use --from-source`);
      archive = join(work, url.split("/").pop() ?? "ffmpeg-archive");
      await download(url, archive);
    }
    if (!existsSync(archive)) throw new Error(`archive not found: ${archive}`);
    const expected = process.env.FFMPEG_SHA256;
    if (expected && sha256(archive) !== expected.toLowerCase()) throw new Error(`SHA-256 mismatch for ${archive}`);
    const out = join(work, "x");
    extract(archive, out);
    const ext = target.includes("windows") ? ".exe" : "";
    const ffmpeg = findFile(out, `ffmpeg${ext}`);
    const ffprobe = findFile(out, `ffprobe${ext}`);
    if (!ffmpeg || !ffprobe) throw new Error("archive does not contain ffmpeg and ffprobe executables");
    install(ffmpeg, ffprobe, target);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch((e) => {
    console.error(`[ffmpeg] ${e instanceof Error ? e.message : e}`);
    process.exit(1);
  });
}
