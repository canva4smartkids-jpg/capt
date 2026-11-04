#!/usr/bin/env python3
"""Dependency license audit for the CaptureOne monorepo.

Collects licenses for:
  * Rust crates (cargo metadata, all targets incl. Windows/macOS-only deps)
  * npm production deps of every workspace package (pnpm -r ls --prod --depth Infinity)
  * Gradle/Android: declared libraries from libs.versions.toml (or build/licenses.json when present)
  * Binaries & system libraries (ffmpeg, libpulse, WebKitGTK, WebView2, TV tooling)
and writes docs/release/LICENSES-AUDIT.md plus a machine-readable JSON file.

Flags copyleft licenses (GPL/AGPL/LGPL/MPL/EPL/CDDL) for review. Usage:
  python3 scripts/license-audit.py
"""
from __future__ import annotations

import json
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT_MD = ROOT / "docs" / "release" / "LICENSES-AUDIT.md"
OUT_JSON = ROOT / "docs" / "release" / "licenses-audit.json"

STRONG_COPYLEFT = re.compile(r"\b(A?GPL)", re.I)
WEAK_COPYLEFT = re.compile(r"\b(LGPL|MPL|EPL|CDDL|CC-BY-SA)", re.I)


def classify(lic: str) -> str:
    if not lic or lic == "UNKNOWN":
        return "unknown"
    # "MIT OR Apache-2.0" style expressions: permissive if any alternative is permissive.
    alts = [a.strip(" ()") for a in re.split(r"\s+OR\s+|/", lic)]
    def one(a: str) -> str:
        if STRONG_COPYLEFT.search(a) and not WEAK_COPYLEFT.search(a):
            return "strong-copyleft"
        if WEAK_COPYLEFT.search(a):
            return "weak-copyleft"
        return "permissive"
    kinds = {one(a) for a in alts}
    if "permissive" in kinds:
        return "permissive"
    if "weak-copyleft" in kinds:
        return "weak-copyleft"
    return "strong-copyleft"


def rust() -> list[dict]:
    try:
        meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=ROOT, stderr=subprocess.DEVNULL))
    except Exception:
        meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1"], cwd=ROOT, stderr=subprocess.DEVNULL))
    workspace = set(meta.get("workspace_members", []))
    out = []
    for p in meta["packages"]:
        if p["id"] in workspace:
            continue
        out.append({"ecosystem": "cargo", "name": p["name"], "version": p["version"], "license": p.get("license") or ("file:" + p["license_file"] if p.get("license_file") else "UNKNOWN"), "repository": p.get("repository") or ""})
    return out


def npm() -> list[dict]:
    """Production npm dependencies of every workspace package.

    `pnpm licenses list -r` only reports the workspace root, so walk
    `pnpm -r ls --prod --depth Infinity --json` and read each package's own package.json.
    Workspace members (link: / private first-party) are reported with ecosystem `npm-workspace`.
    """
    try:
        raw = subprocess.check_output(["pnpm", "-r", "ls", "--prod", "--depth", "Infinity", "--json"], cwd=ROOT, stderr=subprocess.DEVNULL)
    except Exception as e:  # pnpm missing or failure
        print(f"warning: pnpm ls failed: {e}", file=sys.stderr)
        return []
    projects = json.loads(raw or b"[]")
    workspace_names = {p["name"] for p in projects}
    seen: dict[tuple[str, str], dict] = {}

    def lic_of(path: str) -> tuple[str, str]:
        try:
            pj = json.loads((Path(path) / "package.json").read_text())
        except Exception:
            return "UNKNOWN", ""
        lic = pj.get("license")
        if isinstance(lic, dict):
            lic = lic.get("type")
        if not lic and isinstance(pj.get("licenses"), list):
            lic = " OR ".join(x.get("type", "?") for x in pj["licenses"])
        repo = pj.get("homepage") or (pj.get("repository") or {}).get("url", "") if isinstance(pj.get("repository"), dict) else pj.get("homepage") or ""
        return lic or "UNKNOWN", repo

    def walk(deps: dict | None) -> None:
        for name, v in (deps or {}).items():
            version = v.get("version", "")
            path = v.get("path", "")
            key = (name, version)
            if key not in seen:
                if name in workspace_names:
                    lic, _ = lic_of(path)
                    seen[key] = {"ecosystem": "npm-workspace", "name": name, "version": version, "license": "first-party (" + lic + ")" if lic != "UNKNOWN" else "first-party", "repository": ""}
                else:
                    lic, repo = lic_of(path)
                    seen[key] = {"ecosystem": "npm", "name": name, "version": version, "license": lic, "repository": repo}
            walk(v.get("dependencies"))

    for proj in projects:
        walk(proj.get("dependencies"))
    return list(seen.values())


ANDROID_LICENSES = {
    # module prefix -> (license, note). Declared in apps/android/gradle/libs.versions.toml.
    "org.jetbrains.kotlinx": "Apache-2.0",
    "com.squareup.okhttp3": "Apache-2.0",
    "androidx.": "Apache-2.0",
    "org.jetbrains.compose": "Apache-2.0",
    "junit:junit": "EPL-1.0",
    "com.networknt": "Apache-2.0",
}
ANDROID_TEST_ONLY = ("junit:junit", "com.networknt", "androidx.test", "kotlinx-coroutines-test", "mockwebserver")
ANDROID_HOST_ONLY = ("org.jetbrains.compose",)


def gradle() -> list[dict]:
    """Declared Android libraries (not resolved: Google Maven is unreachable in the audit environment)."""
    f = ROOT / "apps" / "android" / "build" / "licenses.json"
    if f.exists():
        return [dict(ecosystem="maven", **d) for d in json.loads(f.read_text())]
    toml = (ROOT / "apps" / "android" / "gradle" / "libs.versions.toml").read_text()
    versions = dict(re.findall(r'^(\w+)\s*=\s*"([^"]+)"', toml.split("[libraries]")[0], re.M))
    libs = toml.split("[libraries]")[1].split("[plugins]")[0]
    out = []
    for m in re.finditer(r'^([\w-]+)\s*=\s*\{([^}]*)\}', libs, re.M):
        alias, body = m.groups()
        mod = re.search(r'module\s*=\s*"([^"]+)"', body)
        if not mod:
            continue
        ver = re.search(r'version\.ref\s*=\s*"([^"]+)"', body)
        lit = re.search(r'version\s*=\s*"([^"]+)"', body)
        version = versions.get(ver.group(1), "?") if ver else (lit.group(1) if lit else "(BOM)")
        lic = next((v for k, v in ANDROID_LICENSES.items() if mod.group(1).startswith(k)), "UNKNOWN")
        scope = "test-only" if any(t in mod.group(1) or t in alias for t in ANDROID_TEST_ONLY) else "host-mode only (never packaged)" if mod.group(1).startswith(ANDROID_HOST_ONLY) else "shipped"
        out.append({"ecosystem": "maven", "name": mod.group(1), "version": version, "license": lic, "repository": "", "note": f"declared, not resolved; {scope}"})
    return out


# name, license, how it is used, redistribution rule
BINARIES = [
    ("ffmpeg / ffprobe (desktop sidecar)", "LGPL-2.1-or-later (build must be --disable-gpl --disable-nonfree)", "Spawned as a separate process (never linked). Bundled by apps/desktop/scripts/fetch-ffmpeg.mjs, whose licensing guard refuses GPL/nonfree builds (x264/x265/fdk-aac). Hardware encoders (VideoToolbox/MediaFoundation/VAAPI/NVENC via system drivers) are used instead of libx264/x265.", "Ship the LGPL build with its license text and a source offer / link to the exact FFmpeg source."),
    ("libpulse (Linux system audio)", "LGPL-2.1-or-later", "Dynamically linked system library (libpulse.so.0); not bundled.", "Dynamic linking; no obligation beyond not statically linking."),
    ("WebKitGTK (Linux webview via Tauri/wry)", "LGPL-2.1-or-later / BSD-2-Clause", "System library, dynamically linked; declared as a .deb dependency, not bundled.", "None beyond dynamic linking."),
    ("Microsoft WebView2 Runtime (Windows)", "Proprietary (Microsoft Evergreen runtime license)", "Installed on the user machine (bootstrapper); not redistributed in the repo.", "Use the Evergreen bootstrapper per Microsoft terms."),
    ("WKWebView / ScreenCaptureKit / AVFoundation (macOS)", "Apple system frameworks", "OS-provided.", "None."),
    ("Tizen Studio CLI / webOS CLI (ares-*)", "Tizen: Flora/Apache-2.0 components; webOS CLI: Apache-2.0", "Build-time packaging tools for the TV web apps only; not distributed.", "None (not shipped)."),
]


def main() -> None:
    deps = rust() + npm() + gradle()
    for d in deps:
        d["class"] = classify(d["license"])
    OUT_JSON.parent.mkdir(parents=True, exist_ok=True)
    OUT_JSON.write_text(json.dumps(deps, indent=1) + "\n")

    by_eco = defaultdict(list)
    for d in deps:
        by_eco[d["ecosystem"]].append(d)
    lines = ["# Dependency license audit", "", "Generated by `scripts/license-audit.py`. Copyleft entries require review before distribution.", ""]
    lines += ["| Ecosystem | Packages | Permissive | Weak copyleft | Strong copyleft | Unknown |", "|---|---|---|---|---|---|"]
    for eco, items in sorted(by_eco.items()):
        c = defaultdict(int)
        for i in items:
            c[i["class"]] += 1
        lines.append(f"| {eco} | {len(items)} | {c['permissive']} | {c['weak-copyleft']} | {c['strong-copyleft']} | {c['unknown']} |")
    flagged = [d for d in deps if d["class"] != "permissive" and d["ecosystem"] != "maven"] + [d for d in deps if d["ecosystem"] == "maven" and d["class"] != "permissive"]
    lines += ["", "## Entries needing review", ""]
    if flagged:
        lines += ["| Ecosystem | Package | Version | License | Class |", "|---|---|---|---|---|"]
        for d in sorted(flagged, key=lambda x: (x["class"], x["ecosystem"], x["name"])):
            lines.append(f"| {d['ecosystem']} | {d['name']} | {d['version']} | {d['license']} | {d['class']} |")
    else:
        lines.append("None.")
    lines += ["", "## License totals", ""]
    totals = defaultdict(int)
    for d in deps:
        totals[d["license"]] += 1
    lines += ["| License expression | Count |", "|---|---|"] + [f"| {k} | {v} |" for k, v in sorted(totals.items(), key=lambda kv: -kv[1])]
    mav = [d for d in deps if d["ecosystem"] == "maven"]
    if mav:
        lines += ["", "## Android (Gradle) - declared, not resolved", "",
                  "Google Maven is not reachable from the audit environment, so transitive dependencies were not resolved. The table lists the libraries declared in `apps/android/gradle/libs.versions.toml` with their published licenses. Re-run with a `licenses.json` produced in CI to cover transitives.", "",
                  "| Library | Version | License | Note |", "|---|---|---|---|"]
        lines += [f"| {d['name']} | {d['version']} | {d['license']} | {d.get('note', '')} |" for d in sorted(mav, key=lambda x: x["name"])]
    lines += ["", "## Binaries & system libraries", "", "| Component | License | Use | Redistribution rule |", "|---|---|---|---|"]
    lines += [f"| {n} | {l} | {u} | {r} |" for n, l, u, r in BINARIES]
    OUT_MD.write_text("\n".join(lines) + "\n")
    print(f"{len(deps)} dependencies audited; {len(flagged)} need review -> {OUT_MD.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
