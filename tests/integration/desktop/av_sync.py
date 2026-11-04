#!/usr/bin/env python3
"""Measures A/V offset in a recording of periodic flash+beep content.

Usage: av_sync.py <file.mp4>  -> prints JSON {offsetsMs, medianMs, videoOnsets, audioOnsets}
Video onsets: frames whose mean luma rises above mid-grey. Audio onsets: first 1 ms block
whose peak exceeds 0.3 (the beep is 0.5; background test tones are <= 0.125).
Positive offset = audio later than video.
"""
import json
import subprocess
import sys


def video_onsets(path):
    p = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-map", "0:v:0", "-vf", "scale=1:1:flags=area,format=gray", "-f", "rawvideo", "-"], capture_output=True, check=True)
    probe = subprocess.run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "stream=avg_frame_rate", "-of", "csv=p=0", path], capture_output=True, text=True, check=True)
    n, d = probe.stdout.strip().split("/")
    fps = float(n) / float(d)
    lum = list(p.stdout)
    on = []
    for i in range(1, len(lum)):
        if lum[i] > 128 and lum[i - 1] <= 128:
            on.append(i / fps)
    return on


def audio_onsets(path, rate=48000, threshold=0.3):
    p = subprocess.run(["ffmpeg", "-v", "error", "-i", path, "-map", "0:a:0", "-ac", "1", "-ar", str(rate), "-f", "f32le", "-"], capture_output=True, check=True)
    import array

    a = array.array("f")
    a.frombytes(p.stdout)
    block = rate // 1000
    on, last_loud = [], False
    for b in range(0, len(a) // block):
        peak = max(abs(x) for x in a[b * block : (b + 1) * block])
        loud = peak > threshold
        if loud and not last_loud and (not on or b / 1000.0 - on[-1] > 0.5):
            on.append(b / 1000.0)
        last_loud = loud
    return on


def measure(path):
    v, a = video_onsets(path), audio_onsets(path)
    offsets = []
    for t in v:
        near = [x for x in a if abs(x - t) < 0.45]
        if near:
            offsets.append(round((min(near, key=lambda x: abs(x - t)) - t) * 1000, 1))
    med = sorted(offsets)[len(offsets) // 2] if offsets else None
    return {"videoOnsets": [round(x, 3) for x in v], "audioOnsets": [round(x, 3) for x in a], "offsetsMs": offsets, "medianMs": med}


# ffplay lavfi graph: 0.1 s white flash + 0.1 s 1 kHz beep at the start of every second.
FLASH_BEEP = (
    "color=c=black:s=320x240:r=30[b];color=c=white:s=320x240:r=30[w];"
    "[b][w]overlay=enable='lt(mod(t\\,1)\\,0.1)'[out0];"
    "aevalsrc='if(lt(mod(t\\,1)\\,0.1)\\,0.5*sin(2*PI*1000*t)\\,0)':s=48000:c=stereo[out1]"
)


def main():
    print(json.dumps(measure(sys.argv[1])))


if __name__ == "__main__":
    main()
