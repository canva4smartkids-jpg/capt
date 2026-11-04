#!/usr/bin/env bash
# CaptureOne desktop engine integration tests on Linux: real X11 capture (Xvfb), real
# PulseAudio (null sink + test tone), real ffmpeg encodes, ffprobe validation.
#
#   tests/integration/desktop/run.sh [--memory] [-- <unittest args, e.g. Desktop.test_10_h264_5s_1080p30>]
#
# Env: CAPTUREONE_CLI (skip the build), CAPTUREONE_TEST_WORK (output dir),
#      CAPTUREONE_SAMPLES_DIR (copy small validated samples there), CARGO_TARGET_DIR.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
MEMORY=0
PYARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --memory) MEMORY=1 ;;
    --) shift; PYARGS=("$@"); break ;;
    *) PYARGS+=("$1") ;;
  esac
  shift
done

export CAPTUREONE_TEST_WORK="${CAPTUREONE_TEST_WORK:-$(mktemp -d /tmp/captureone-it.XXXXXX)}"
mkdir -p "$CAPTUREONE_TEST_WORK"
if [ -z "${CAPTUREONE_CLI:-}" ]; then
  (cd "$ROOT" && cargo build --release -p captureone-cli)
  CAPTUREONE_CLI="${CARGO_TARGET_DIR:-$ROOT/target}/release/captureone-cli"
fi
export CAPTUREONE_CLI
echo "cli: $CAPTUREONE_CLI  work: $CAPTUREONE_TEST_WORK" >&2

PIDS=()
STARTED_PULSE=0
PULSE_MODULE=""
PREV_SINK=""
PREV_SOURCE=""
cleanup() {
  for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done
  if [ "$STARTED_PULSE" = 1 ]; then
    pulseaudio --kill 2>/dev/null || true
  else
    # Using someone's existing server: put their defaults back and remove our sink.
    [ -n "$PREV_SINK" ] && pactl set-default-sink "$PREV_SINK" 2>/dev/null || true
    [ -n "$PREV_SOURCE" ] && pactl set-default-source "$PREV_SOURCE" 2>/dev/null || true
    [ -n "$PULSE_MODULE" ] && pactl unload-module "$PULSE_MODULE" 2>/dev/null || true
  fi
}
trap cleanup EXIT

# --- X server -----------------------------------------------------------------------
if [ -z "${DISPLAY:-}" ] || ! xdpyinfo >/dev/null 2>&1; then
  export DISPLAY=":${CAPTUREONE_TEST_DISPLAY_NUM:-99}"
  if ! xdpyinfo >/dev/null 2>&1; then
    Xvfb "$DISPLAY" -screen 0 1920x1080x24 -nolisten tcp >"$CAPTUREONE_TEST_WORK/xvfb.log" 2>&1 &
    PIDS+=($!)
    for _ in $(seq 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.1; done
  fi
fi
xdpyinfo | grep -E "dimensions|depth of root" >&2

# --- PulseAudio: null sink as default output, its monitor as default input -------------
if ! pactl info >/dev/null 2>&1; then
  pulseaudio -D --exit-idle-time=-1 --disallow-exit 2>/dev/null || pulseaudio -D --exit-idle-time=-1
  STARTED_PULSE=1
  for _ in $(seq 50); do pactl info >/dev/null 2>&1 && break; sleep 0.1; done
fi
PREV_SINK="$(pactl info 2>/dev/null | sed -n 's/^Default Sink: //p')"
PREV_SOURCE="$(pactl info 2>/dev/null | sed -n 's/^Default Source: //p')"
if ! pactl list short sinks | grep -q captureone_test; then
  PULSE_MODULE="$(pactl load-module module-null-sink sink_name=captureone_test sink_properties=device.description=CaptureOneTest)"
fi
pactl set-default-sink captureone_test
pactl set-default-source captureone_test.monitor

# --- moving content + a test tone ----------------------------------------------------
SDL_AUDIODRIVER=dummy ffplay -hide_banner -loglevel error -an -noborder -left 400 -top 200 \
  -f lavfi -i "testsrc2=size=1280x720:rate=30" >/dev/null 2>&1 &
PIDS+=($!)
xterm -geometry 100x20+0+0 -e top -d 0.5 >/dev/null 2>&1 &
PIDS+=($!)
ffmpeg -hide_banner -loglevel error -re -f lavfi -i "sine=frequency=440:sample_rate=48000" \
  -f pulse -device captureone_test "captureone-test-tone" >/dev/null 2>&1 &
PIDS+=($!)
sleep 2

STATUS=0
export PYTHONDONTWRITEBYTECODE=1
python3 "$HERE/test_desktop.py" "${PYARGS[@]}" || STATUS=$?
if [ "$MEMORY" = 1 ]; then
  python3 "$HERE/memory_probe.py" || STATUS=$?
fi
echo "results: $CAPTUREONE_TEST_WORK/results.json" >&2
exit "$STATUS"
