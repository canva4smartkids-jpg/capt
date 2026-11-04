#!/usr/bin/env python3
"""Samples the RSS of captureone-cli (and its ffmpeg children) once per second during a
1920x1080@30 recording and checks that memory does not grow over time.

Env: CAPTUREONE_CLI, CAPTUREONE_TEST_WORK, CAPTUREONE_MEMORY_SECONDS (default 45).
Writes $CAPTUREONE_TEST_WORK/memory.json.
"""
import json
import os
import subprocess
import sys
import time

CLI = os.environ.get("CAPTUREONE_CLI", "captureone-cli")
WORK = os.environ.get("CAPTUREONE_TEST_WORK", "/tmp/captureone-it")
SECONDS = int(os.environ.get("CAPTUREONE_MEMORY_SECONDS", "45"))
DISPLAY_ID = os.environ.get("CAPTUREONE_TEST_DISPLAY_ID", "screen")


def rss_kb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1])
    except OSError:
        return None
    return None


def children(pid):
    try:
        with open(f"/proc/{pid}/task/{pid}/children") as f:
            return [int(x) for x in f.read().split()]
    except OSError:
        return []


def slope(xs, ys):
    n = len(xs)
    mx, my = sum(xs) / n, sum(ys) / n
    den = sum((x - mx) ** 2 for x in xs)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / den if den else 0.0


def median(v):
    v = sorted(v)
    return v[len(v) // 2] if v else None


def main():
    out = os.path.join(WORK, "memory")
    os.makedirs(out, exist_ok=True)
    proc = subprocess.Popen(
        [CLI, "record", "--out", out, "--target", f"display:{DISPLAY_ID}", "--fps", "30", "--system-audio", "--duration", str(SECONDS), "--min-free-mb", os.environ.get("CAPTUREONE_TEST_MIN_FREE_MB", "100")],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    samples = []
    t0 = time.time()
    while proc.poll() is None:
        cli = rss_kb(proc.pid)
        kids = sum(filter(None, (rss_kb(c) for c in children(proc.pid))))
        if cli is not None:
            samples.append((round(time.time() - t0, 1), cli, kids))
        time.sleep(1.0)
    stdout, stderr = proc.communicate()
    meta = json.loads(stdout) if stdout.strip() else {}
    final = next((json.loads(l) for l in stderr.splitlines() if '"event":"final"' in l), {})
    rec = [s for s in samples if 5 <= s[0] <= SECONDS]  # recording phase, after warm-up
    cli_mb = [s[1] / 1024 for s in rec]
    kid_mb = [s[2] / 1024 for s in rec]
    report = {
        "seconds": SECONDS,
        "exit": proc.returncode,
        "valid": meta.get("validation", {}).get("valid"),
        "durationMs": meta.get("durationMs"),
        "framesEncoded": final.get("framesEncoded"),
        "framesDropped": final.get("framesDropped"),
        "cliPeakMb": round(max(s[1] for s in samples) / 1024, 1) if samples else None,
        "cliSteadyMedianMb": round(sorted(cli_mb)[len(cli_mb) // 2], 1) if cli_mb else None,
        "cliSlopeMbPerMin": round(slope([s[0] for s in rec], cli_mb) * 60, 2) if len(rec) > 2 else None,
        # robust growth metric: median of the last quarter minus median of the first quarter
        "cliGrowthMb": round(median(cli_mb[-max(1, len(cli_mb) // 4):]) - median(cli_mb[: max(1, len(cli_mb) // 4)]), 1) if cli_mb else None,
        "ffmpegChildrenGrowthMb": round(median(kid_mb[-max(1, len(kid_mb) // 4):]) - median(kid_mb[: max(1, len(kid_mb) // 4)]), 1) if kid_mb else None,
        "ffmpegChildrenPeakMb": round(max(s[2] for s in samples) / 1024, 1) if samples else None,
        "ffmpegChildrenSlopeMbPerMin": round(slope([s[0] for s in rec], kid_mb) * 60, 2) if len(rec) > 2 else None,
        "samples": samples,
    }
    with open(os.path.join(WORK, "memory.json"), "w") as f:
        json.dump(report, f, indent=2)
    print(json.dumps({k: v for k, v in report.items() if k != "samples"}), file=sys.stderr)
    # A 1080p BGRA frame is ~8.3 MB; allow two frames of allocator/queue jitter, no more.
    frame_mb = 1920 * 1080 * 4 / 1e6
    ok = proc.returncode == 0 and bool(report["valid"]) and report["cliGrowthMb"] is not None and report["cliGrowthMb"] <= 2 * frame_mb
    ok = ok and report["ffmpegChildrenGrowthMb"] is not None and report["ffmpegChildrenGrowthMb"] <= 2 * frame_mb
    print("memory probe:", "PASS" if ok else "FAIL", file=sys.stderr)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
