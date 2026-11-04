#!/usr/bin/env python3
"""End-to-end tests of captureone-cli on a real X server + PulseAudio (see run.sh).

Every recording is verified with ffprobe (duration, dimensions, codec, frame count, audio
streams) and, for audio, with ffmpeg's volumedetect. Results are summarized in
$CAPTUREONE_TEST_WORK/results.json.
"""
import sys

sys.dont_write_bytecode = True  # keep the source tree clean (av_sync is imported)

import glob
import json
import os
import shutil
import signal
import subprocess
import sys
import time
import unittest

CLI = os.environ.get("CAPTUREONE_CLI", "captureone-cli")
WORK = os.environ.get("CAPTUREONE_TEST_WORK", "/tmp/captureone-it")
SAMPLES = os.environ.get("CAPTUREONE_SAMPLES_DIR")  # copy small validated samples here
DISPLAY_ID = os.environ.get("CAPTUREONE_TEST_DISPLAY_ID", "screen")
# Shared CI/dev disks can be tight; the disk guard itself is covered by test_42/test_45.
MIN_FREE_MB = os.environ.get("CAPTUREONE_TEST_MIN_FREE_MB", "100")
EVIDENCE = []


def with_min_free(args):
    if args and args[0] == "record" and "--min-free-mb" not in args:
        return args + ["--min-free-mb", MIN_FREE_MB]
    return args


def run(args, timeout=120, check=True):
    p = subprocess.run([CLI] + with_min_free(args), capture_output=True, text=True, timeout=timeout)
    events = []
    for line in p.stderr.splitlines():
        line = line.strip()
        if line.startswith("{"):
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    out = None
    if p.stdout.strip():
        try:
            out = json.loads(p.stdout)
        except json.JSONDecodeError:
            out = None
    if check and p.returncode != 0:
        raise AssertionError(f"{' '.join(args)} exited {p.returncode}\nstdout: {p.stdout[-2000:]}\nstderr: {p.stderr[-3000:]}")
    return p.returncode, out, events


def ffprobe(path):
    p = subprocess.run(["ffprobe", "-v", "error", "-print_format", "json", "-show_format", "-show_streams", "-count_packets", path], capture_output=True, text=True, timeout=60)
    assert p.returncode == 0, p.stderr
    return json.loads(p.stdout)


def streams(info, kind):
    return [s for s in info["streams"] if s.get("codec_type") == kind]


def mean_volume(path, index=0):
    p = subprocess.run(["ffmpeg", "-hide_banner", "-nostats", "-i", path, "-map", f"0:a:{index}", "-af", "volumedetect", "-f", "null", "-"], capture_output=True, text=True, timeout=60)
    for line in p.stderr.splitlines():
        if "mean_volume:" in line:
            return float(line.split("mean_volume:")[1].split("dB")[0])
    raise AssertionError("volumedetect failed: " + p.stderr[-500:])


def rgb_pixels(path):
    p = subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-i", path, "-f", "rawvideo", "-pix_fmt", "rgb24", "-"], capture_output=True, timeout=60)
    assert p.returncode == 0
    return p.stdout


def decode_errors(path):
    """Fully decodes every stream; returns ffmpeg's error output (empty = clean)."""
    p = subprocess.run(["ffmpeg", "-hide_banner", "-nostats", "-v", "error", "-i", path, "-map", "0", "-f", "null", "-"], capture_output=True, text=True, timeout=300)
    return (p.stderr.strip() + ("" if p.returncode == 0 else f" (exit {p.returncode})")).strip()


def track_name(stream):
    tags = stream.get("tags", {})
    return tags.get("title") or tags.get("handler_name")


def event(events, name):
    return [e for e in events if e.get("event") == name]


def completed(events):
    c = event(events, "completed")
    assert len(c) == 1, f"expected exactly one completed event, got {c}"
    return c[0]


def outdir(name):
    d = os.path.join(WORK, name)
    shutil.rmtree(d, ignore_errors=True)
    os.makedirs(d)
    return d


def evidence(name, path, info, **extra):
    v = streams(info, "video")[0] if streams(info, "video") else {}
    rec = {
        "test": name,
        "file": os.path.basename(path),
        "duration": float(info["format"]["duration"]),
        "size": int(info["format"]["size"]),
        "video": {k: v.get(k) for k in ("codec_name", "codec_tag_string", "width", "height", "avg_frame_rate", "nb_read_packets", "pix_fmt")},
        "audio": [{k: a.get(k) for k in ("codec_name", "sample_rate", "channels", "duration")} | {"name": track_name(a)} for a in streams(info, "audio")],
    }
    rec.update(extra)
    EVIDENCE.append(rec)
    print("  evidence:", json.dumps(rec), file=sys.stderr)
    return rec


def keep_sample(path, name):
    if SAMPLES and os.path.getsize(path) < 3_000_000:
        os.makedirs(SAMPLES, exist_ok=True)
        shutil.copy(path, os.path.join(SAMPLES, name))


def assert_no_session_left(test, d):
    left = glob.glob(os.path.join(d, ".captureone-sessions", "*"))
    test.assertEqual(left, [], "session dir must be removed after a valid recording")


class Desktop(unittest.TestCase):
    def record(self, name, extra, duration=None, timeout=180, expect_exit=0):
        d = outdir(name)
        args = ["record", "--out", d] + extra
        if duration is not None:
            args += ["--duration", str(duration)]
        code, meta, events = run(args, timeout=timeout, check=False)
        self.assertEqual(code, expect_exit, f"exit {code}; events: {[e for e in events if e.get('event') in ('error', 'warning', 'completed')]}")
        return d, meta, events

    def check_video(self, meta, duration, width, height, codec="h264", fps=30, tol=0.4):
        self.assertTrue(meta["validation"]["valid"], meta["validation"])
        info = ffprobe(meta["path"])
        v = streams(info, "video")
        self.assertEqual(len(v), 1)
        v = v[0]
        self.assertEqual(v["codec_name"], codec)
        self.assertEqual((v["width"], v["height"]), (width, height))
        self.assertEqual(v["avg_frame_rate"], f"{fps}/1")
        dur = float(info["format"]["duration"])
        self.assertAlmostEqual(dur, duration, delta=tol, msg=f"duration {dur}")
        frames = int(v["nb_read_packets"])
        self.assertAlmostEqual(frames, duration * fps, delta=max(3, 0.03 * duration * fps), msg=f"frames {frames}")
        self.assertGreater(int(info["format"]["size"]), 10_000)
        return info

    # --- basic ---------------------------------------------------------------------
    def test_00_capabilities(self):
        _, caps, _ = run(["capabilities"])
        self.assertEqual(caps["platform"], "linux")
        for k in ("screenshot", "displayCapture", "videoRecording", "h264"):
            self.assertEqual(caps["features"][k]["level"], "native", k)
        self.assertTrue(any(e["verified"] and e["name"] == "libx264" for e in caps["encoders"]))
        self.assertIn(caps["features"]["systemAudio"]["level"], ("partial", "native"))

    def test_01_sources(self):
        _, src, _ = run(["sources"])
        displays = [s for s in src if s["kind"] == "display"]
        self.assertTrue(any(s["id"] == DISPLAY_ID and (s["width"], s["height"]) == (1920, 1080) for s in displays), displays)
        self.assertTrue(any(s["id"] == "all" for s in displays))
        windows = [s for s in src if s["kind"] == "window"]
        self.assertGreaterEqual(len(windows), 1, "the test content windows must be listed")

    def test_02_thumbnail(self):
        _, out, _ = run(["thumbnail", "--kind", "display", "--id", DISPLAY_ID, "--max-width", "320"])
        self.assertTrue(out["dataUrl"].startswith("data:image/png;base64,iVBORw0KGgo"))

    # --- video ---------------------------------------------------------------------
    def test_10_h264_5s_1080p30(self):
        d, meta, ev = self.record("h264", ["--target", f"display:{DISPLAY_ID}", "--fps", "30", "--codec", "h264"], duration=5)
        info = self.check_video(meta, 5.0, 1920, 1080)
        self.assertEqual(decode_errors(meta["path"]), "")
        self.assertEqual(completed(ev)["stopReason"], "user")
        self.assertTrue(os.path.isfile(meta["thumbnailPath"]))
        assert_no_session_left(self, d)
        final = event(ev, "final")[0]
        evidence("h264_1080p30_5s", meta["path"], info, framesEncoded=final["framesEncoded"], framesDropped=final["framesDropped"])

    def test_11_h265(self):
        d, meta, _ = self.record("h265", ["--target", f"display:{DISPLAY_ID}", "--codec", "h265", "--height", "720"], duration=4)
        info = self.check_video(meta, 4.0, 1280, 720, codec="hevc")
        self.assertEqual(streams(info, "video")[0]["codec_tag_string"], "hvc1")
        evidence("h265_720p_4s", meta["path"], info)
        keep_sample(meta["path"], "linux-x11-h265-720p.mp4")

    def test_12_scaled_720p_with_audio(self):
        d, meta, _ = self.record("h264-720p", ["--target", f"display:{DISPLAY_ID}", "--height", "720", "--system-audio", "--quality", "low"], duration=5)
        info = self.check_video(meta, 5.0, 1280, 720)
        self.assertEqual(len(streams(info, "audio")), 1)
        evidence("h264_720p_5s_system_audio", meta["path"], info, meanVolumeDb=mean_volume(meta["path"]))
        keep_sample(meta["path"], "linux-x11-h264-720p-system-audio.mp4")

    def test_13_region_exact_dims(self):
        d, meta, _ = self.record("region", ["--target", f"region:{DISPLAY_ID}:200,150,640,360"], duration=3)
        info = self.check_video(meta, 3.0, 640, 360)
        evidence("region_640x360", meta["path"], info)
        # odd-sized regions are encoded at the nearest even size
        d, meta, _ = self.record("region-odd", ["--target", f"region:{DISPLAY_ID}:10,10,333,201"], duration=2)
        self.check_video(meta, 2.0, 332, 200)

    def test_14_pause_resume(self):
        d, meta, ev = self.record("pause", ["--target", f"display:{DISPLAY_ID}", "--pause-at", "2", "--resume-at", "4"], duration=4)
        info = self.check_video(meta, 4.0, 1920, 1080)
        states = [(e["from"], e["to"]) for e in event(ev, "state")]
        self.assertIn(("recording", "paused"), states)
        self.assertIn(("paused", "recording"), states)
        final = event(ev, "final")[0]
        self.assertGreaterEqual(final["wallMs"], 6000)
        evidence("pause_resume_2s_2s_2s", meta["path"], info, wallMs=final["wallMs"])

    def test_15_60fps(self):
        d, meta, ev = self.record("fps60", ["--target", f"display:{DISPLAY_ID}", "--fps", "60"], duration=4)
        info = self.check_video(meta, 4.0, 1920, 1080, fps=60)
        final = event(ev, "final")[0]
        evidence("h264_1080p60_4s", meta["path"], info, framesEncoded=final["framesEncoded"], framesDropped=final["framesDropped"])

    def test_16_no_overwrite(self):
        d = outdir("naming")
        for _ in range(2):
            code, meta, _ = run(["record", "--out", d, "--target", f"region:{DISPLAY_ID}:0,0,320,240", "--template", "fixed", "--duration", "1"], timeout=60)
        names = sorted(os.path.basename(p) for p in glob.glob(os.path.join(d, "*.mp4")))
        self.assertEqual(names, ["fixed (1).mp4", "fixed.mp4"])

    def test_17_inspect(self):
        d, meta, _ = self.record("inspect", ["--target", f"region:{DISPLAY_ID}:0,0,640,480"], duration=2)
        _, m2, _ = run(["inspect", meta["path"]])
        self.assertTrue(m2["validation"]["valid"])
        self.assertEqual((m2["width"], m2["height"], m2["videoCodec"]), (640, 480, "h264"))

    def test_18_cancel_deletes_output(self):
        d = outdir("cancel")
        code, out, ev = run(["record", "--out", d, "--target", f"display:{DISPLAY_ID}", "--cancel-at", "2"], timeout=60)
        self.assertTrue(out["cancelled"])
        self.assertEqual(glob.glob(os.path.join(d, "*.mp4")), [])
        assert_no_session_left(self, d)

    # --- screenshots ---------------------------------------------------------------
    def test_20_screenshots(self):
        d = outdir("shots")
        for fmt, codec in (("png", "png"), ("jpeg", "mjpeg")):
            _, meta, _ = run(["screenshot", "--target", f"display:{DISPLAY_ID}", "--format", fmt, "--out", d])
            self.assertTrue(meta["validation"]["valid"])
            info = ffprobe(meta["path"])
            v = streams(info, "video")[0]
            self.assertEqual((v["codec_name"], v["width"], v["height"]), (codec, 1920, 1080))
            px = rgb_pixels(meta["path"])
            self.assertEqual(len(px), 1920 * 1080 * 3)
            distinct = len(set(px[i : i + 3] for i in range(0, len(px), 3 * 997)))
            self.assertGreater(distinct, 10, "screenshot must not be uniform")
            EVIDENCE.append({"test": f"screenshot_{fmt}", "file": os.path.basename(meta["path"]), "codec": codec, "width": v["width"], "height": v["height"], "size": meta["sizeBytes"], "distinctSampledColors": distinct})
        # region screenshot, small enough to keep as samples
        for fmt, name in (("png", "linux-x11-screenshot-region.png"), ("jpeg", "linux-x11-screenshot-region.jpg")):
            _, meta, _ = run(["screenshot", "--target", f"region:{DISPLAY_ID}:380,180,800,600", "--format", fmt, "--out", d])
            self.assertEqual((meta["width"], meta["height"]), (800, 600))
            v = streams(ffprobe(meta["path"]), "video")[0]
            self.assertEqual((v["width"], v["height"]), (800, 600))
            keep_sample(meta["path"], name)

    # --- audio ---------------------------------------------------------------------
    def test_30_system_audio(self):
        d, meta, _ = self.record("sysaudio", ["--target", f"display:{DISPLAY_ID}", "--height", "720", "--system-audio"], duration=5)
        info = self.check_video(meta, 5.0, 1280, 720)
        a = streams(info, "audio")
        self.assertEqual(len(a), 1)
        self.assertEqual((a[0]["codec_name"], int(a[0]["sample_rate"]), a[0]["channels"]), ("aac", 48000, 2))
        self.assertAlmostEqual(float(a[0]["duration"]), 5.0, delta=0.4)
        vol = mean_volume(meta["path"])
        self.assertGreater(vol, -50.0, "system audio must not be silent")
        evidence("system_audio", meta["path"], info, meanVolumeDb=vol)

    def test_31_microphone(self):
        _, devs, _ = run(["audio-devices"])
        if not any(dv["kind"] == "input" for dv in devs):
            self.skipTest("cpal sees no input device")
        d, meta, _ = self.record("mic", ["--target", f"display:{DISPLAY_ID}", "--height", "720", "--mic"], duration=4)
        info = self.check_video(meta, 4.0, 1280, 720)
        self.assertEqual(len(streams(info, "audio")), 1)
        vol = mean_volume(meta["path"])
        self.assertGreater(vol, -50.0, "microphone (default source = null-sink monitor with a tone) must not be silent")
        evidence("microphone_cpal", meta["path"], info, meanVolumeDb=vol)

    def test_32_separate_and_mixed_tracks(self):
        d, meta, _ = self.record("separate", ["--target", f"display:{DISPLAY_ID}", "--height", "480", "--mic", "--system-audio", "--tracks", "separate"], duration=3)
        info = self.check_video(meta, 3.0, 852, 480)
        a = streams(info, "audio")
        self.assertEqual(len(a), 2)
        self.assertEqual([track_name(x) for x in a], ["Microphone", "System audio"])
        vols = [mean_volume(meta["path"], i) for i in range(2)]
        evidence("tracks_separate", meta["path"], info, meanVolumeDb=vols)
        d, meta, _ = self.record("mixed", ["--target", f"display:{DISPLAY_ID}", "--height", "480", "--mic", "--system-audio", "--tracks", "mixed"], duration=3)
        info = self.check_video(meta, 3.0, 852, 480)
        self.assertEqual(len(streams(info, "audio")), 1)
        evidence("tracks_mixed", meta["path"], info, meanVolumeDb=mean_volume(meta["path"]))

    # --- limits & interruption -----------------------------------------------------
    def test_40_max_duration(self):
        d, meta, ev = self.record("maxdur", ["--target", f"display:{DISPLAY_ID}", "--max-duration", "3"])
        self.assertEqual(completed(ev)["stopReason"], "max-duration")
        info = self.check_video(meta, 3.0, 1920, 1080)
        evidence("max_duration_3s", meta["path"], info)

    def test_41_max_file_size(self):
        d, meta, ev = self.record("maxsize", ["--target", f"display:{DISPLAY_ID}", "--quality", "ultra", "--max-size-mb", "6", "--max-duration", "60"], timeout=120)
        self.assertEqual(completed(ev)["stopReason"], "max-file-size")
        self.assertTrue(meta["validation"]["valid"], meta["validation"])
        self.assertLessEqual(meta["sizeBytes"], 6_000_000)
        self.assertGreater(meta["sizeBytes"], 2_400_000, "limit handling must not be absurdly conservative")
        evidence("max_file_size_6mb", meta["path"], ffprobe(meta["path"]))

    def test_42_low_disk_refuses_start(self):
        d = outdir("lowdisk")
        code, out, ev = run(["record", "--out", d, "--target", f"display:{DISPLAY_ID}", "--min-free-mb", "100000000", "--duration", "1"], check=False)
        self.assertEqual(code, 1)
        self.assertEqual(out["error"]["code"], "STORAGE_FULL")

    def test_43_window_closed(self):
        title = f"captureone-close-{os.getpid()}"
        term = subprocess.Popen(["xterm", "-T", title, "-geometry", "80x24+300+300", "-e", "sh", "-c", "while true; do date; sleep 0.2; done"])
        try:
            win = None
            for _ in range(50):
                _, src, _ = run(["sources", "--kind", "window"])
                win = next((s for s in src if s["name"] == title), None)
                if win:
                    break
                time.sleep(0.1)
            self.assertIsNotNone(win, "test window not listed")
            d = outdir("winclose")
            proc = subprocess.Popen([CLI] + with_min_free(["record", "--out", d, "--target", f"window:{win['id']}"]), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            time.sleep(4)
            term.kill()
            out, err = proc.communicate(timeout=60)
            self.assertEqual(proc.returncode, 0, err[-2000:])
            meta = json.loads(out)
            evs = [json.loads(l) for l in err.splitlines() if l.startswith("{")]
            self.assertEqual(completed(evs)["stopReason"], "source-closed")
            w, h = win["width"] // 2 * 2, win["height"] // 2 * 2
            info = self.check_video(meta, float(meta["durationMs"]) / 1000.0, w, h, tol=0.15)
            self.assertAlmostEqual(meta["durationMs"] / 1000.0, 3.8, delta=1.0)
            evidence("window_closed", meta["path"], info, stopReason="source-closed")
        finally:
            term.kill()

    def test_46_window_resize_keeps_output_size(self):
        title = f"captureone-resize-{os.getpid()}"
        term = subprocess.Popen(["xterm", "-T", title, "-geometry", "80x24+250+250", "-e", "sh", "-c", "while true; do date; sleep 0.1; done"])
        try:
            win = None
            for _ in range(50):
                _, src, _ = run(["sources", "--kind", "window"])
                win = next((s for s in src if s["name"] == title), None)
                if win:
                    break
                time.sleep(0.1)
            self.assertIsNotNone(win)
            d = outdir("winresize")
            proc = subprocess.Popen([CLI] + with_min_free(["record", "--out", d, "--target", f"window:{win['id']}", "--duration", "5"]), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            time.sleep(1.5)
            subprocess.run(["xdotool", "windowsize", str(int(win["id"], 16)), "900", "500"], check=True)
            time.sleep(1.5)
            subprocess.run(["xdotool", "windowsize", str(int(win["id"], 16)), "300", "200"], check=True)
            out, err = proc.communicate(timeout=60)
            self.assertEqual(proc.returncode, 0, err[-2000:])
            meta = json.loads(out)
            w, h = win["width"] // 2 * 2, win["height"] // 2 * 2
            info = self.check_video(meta, 5.0, w, h)
            self.assertEqual(decode_errors(meta["path"]), "")
            evidence("window_resized_twice", meta["path"], info)
        finally:
            term.kill()

    def test_47_explicit_microphone_device(self):
        _, devs, _ = run(["audio-devices"])
        pulse = next((dv for dv in devs if dv["kind"] == "input" and dv["id"].endswith(":pulse")), None)
        if not pulse:
            self.skipTest("no ALSA 'pulse' input device")
        d, meta, _ = self.record("micdev", ["--target", f"region:{DISPLAY_ID}:0,0,320,240", "--mic", "--mic-device", pulse["id"]], duration=3)
        info = self.check_video(meta, 3.0, 320, 240)
        self.assertGreater(mean_volume(meta["path"]), -50.0)
        evidence("microphone_by_id", meta["path"], info, device=pulse["id"])

    def test_44_kill9_then_recover(self):
        d = outdir("kill9")
        proc = subprocess.Popen([CLI] + with_min_free(["record", "--out", d, "--target", f"display:{DISPLAY_ID}", "--system-audio", "--height", "720"]), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(7)
        os.kill(proc.pid, signal.SIGKILL)
        proc.wait()
        time.sleep(0.5)  # ffmpeg children see EOF and exit
        sessions = glob.glob(os.path.join(d, ".captureone-sessions", "*"))
        self.assertEqual(len(sessions), 1, "interrupted session data must remain")
        self.assertEqual(glob.glob(os.path.join(d, "*.mp4")), [])
        code, res, _ = run(["recover", "--out", d], timeout=120)
        self.assertEqual(len(res), 1, res)
        meta = res[0]["ok"]
        self.assertTrue(meta["validation"]["valid"], meta["validation"])
        info = ffprobe(meta["path"])
        dur = float(info["format"]["duration"])
        self.assertGreaterEqual(dur, 4.0, "at least everything up to the last 2 s fragment is recovered")
        self.assertLessEqual(dur, 7.5)
        v = streams(info, "video")[0]
        self.assertEqual((v["width"], v["height"]), (1280, 720))
        self.assertEqual(len(streams(info, "audio")), 1)
        self.assertIn("(recovered)", meta["fileName"])
        self.assertEqual(decode_errors(meta["path"]), "", "recovered file must decode cleanly (partial fragment trimmed)")
        assert_no_session_left(self, d)
        code, res2, _ = run(["recover", "--out", d])
        self.assertEqual(res2, [], "recovery is idempotent")
        evidence("kill9_recovered", meta["path"], info)

    def test_45_disk_full_tmpfs(self):
        mnt = os.path.join(WORK, "tmpfs")
        os.makedirs(mnt, exist_ok=True)
        sudo = [] if os.geteuid() == 0 else ["sudo", "-n"]
        opts = f"size=40m,uid={os.getuid()},gid={os.getgid()},mode=0755"
        r = subprocess.run(sudo + ["mount", "-t", "tmpfs", "-o", opts, "tmpfs", mnt], capture_output=True, text=True)
        if r.returncode != 0:
            self.skipTest("cannot mount a tmpfs here: " + r.stderr.strip())
        try:
            code, meta, ev = run(["record", "--out", mnt, "--target", f"display:{DISPLAY_ID}", "--quality", "ultra", "--min-free-mb", "5", "--max-duration", "120"], timeout=180, check=False)
            reason = completed(ev)["stopReason"]
            self.assertEqual(reason, "low-disk", ev[-5:])
            self.assertEqual(code, 0)
            self.assertTrue(meta["validation"]["valid"])
            evidence("low_disk_tmpfs_40mb", meta["path"], ffprobe(meta["path"]), stopReason=reason)
        finally:
            subprocess.run(sudo + ["umount", "-l", mnt])


class AvSync(unittest.TestCase):
    def test_50_av_sync_flash_and_beep(self):
        sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
        import av_sync

        env = dict(os.environ, SDL_AUDIODRIVER="pulseaudio")
        player = subprocess.Popen(["ffplay", "-hide_banner", "-loglevel", "error", "-noborder", "-left", "1500", "-top", "800", "-f", "lavfi", av_sync.FLASH_BEEP], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        try:
            time.sleep(2.5)
            d = outdir("avsync")
            code, meta, _ = run(["record", "--out", d, "--target", f"region:{DISPLAY_ID}:1500,800,320,240", "--system-audio", "--duration", "8"], timeout=120)
        finally:
            player.kill()
        self.assertTrue(meta["validation"]["valid"])
        m = av_sync.measure(meta["path"])
        if not m["audioOnsets"]:
            self.skipTest("ffplay could not play audio into PulseAudio here (no beeps recorded)")
        self.assertGreaterEqual(len(m["offsetsMs"]), 4, m)
        self.assertLessEqual(abs(m["medianMs"]), 100.0, m)
        EVIDENCE.append({"test": "av_sync_flash_beep", "medianOffsetMs": m["medianMs"], "offsetsMs": m["offsetsMs"]})


class Summary(unittest.TextTestResult):
    pass


if __name__ == "__main__":
    os.makedirs(WORK, exist_ok=True)
    runner = unittest.TextTestRunner(verbosity=2, resultclass=Summary)
    prog = unittest.main(testRunner=runner, exit=False, argv=[sys.argv[0]] + sys.argv[1:])
    res = prog.result
    summary = {
        "run": res.testsRun,
        "failures": len(res.failures),
        "errors": len(res.errors),
        "skipped": [(t.id(), why) for t, why in res.skipped],
        "evidence": EVIDENCE,
    }
    with open(os.path.join(WORK, "results.json"), "w") as f:
        json.dump(summary, f, indent=2)
    print(json.dumps({k: summary[k] for k in ("run", "failures", "errors", "skipped")}), file=sys.stderr)
    sys.exit(0 if res.wasSuccessful() else 1)
