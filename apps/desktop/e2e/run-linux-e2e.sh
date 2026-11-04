#!/usr/bin/env bash
# Runs the built CaptureOne desktop app under Xvfb and drives it end-to-end through
# tauri-driver + WebKitWebDriver (e2e.mjs), then verifies every produced MP4 with ffprobe.
#
#   apps/desktop/e2e/run-linux-e2e.sh [app-binary] [screenshot-dir]
# Requires: Xvfb, xdotool, ImageMagick (import), ffprobe, WebKitWebDriver (webkit2gtk-driver),
# tauri-driver (cargo install tauri-driver --locked), dbus-run-session.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
APP="${1:-${CARGO_TARGET_DIR:-$HERE/../../../target}/release/captureone-desktop}"
SHOT_DIR="${2:-$HERE/../../../docs/screenshots}"
DISPLAY_NUM="${E2E_DISPLAY:-:98}"
WORK="$(mktemp -d /tmp/captureone-e2e.XXXXXX)"
export HOME="$WORK/home" XDG_CONFIG_HOME="$WORK/home/.config" XDG_DATA_HOME="$WORK/home/.local/share" XDG_CACHE_HOME="$WORK/home/.cache"
export XDG_RUNTIME_DIR="$WORK/run" CAPTUREONE_OUTPUT_DIR="$WORK/home/Videos/CaptureOne" SHOT_DIR
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; }
trap cleanup EXIT

Xvfb "$DISPLAY_NUM" -screen 0 1600x1000x24 -nolisten tcp >"$WORK/xvfb.log" 2>&1 & PIDS+=($!)
export DISPLAY="$DISPLAY_NUM"
for _ in $(seq 50); do xdpyinfo >/dev/null 2>&1 && break; sleep 0.1; done
xsetroot -solid '#24324d'
# Something to look at on the desktop (thumbnails, region selection, recordings).
xterm -geometry 96x28+60+60 -bg '#0f172a' -fg '#e2e8f0' -fa Monospace -fs 11 -T "Terminal" -e "bash -c 'while true; do date; ls -la /usr/share | head -20; sleep 1; done'" & PIDS+=($!)
pulseaudio --daemonize=no --exit-idle-time=-1 >"$WORK/pulse.log" 2>&1 & PIDS+=($!)
sleep 1

echo "[e2e] app=$APP work=$WORK"
dbus-run-session -- bash -c "
  tauri-driver --port 4444 --native-driver /usr/bin/WebKitWebDriver >'$WORK/tauri-driver.log' 2>&1 &
  DRIVER=\$!
  for _ in \$(seq 50); do curl -sf http://127.0.0.1:4444/status >/dev/null && break; sleep 0.2; done
  node '$HERE/e2e.mjs' '$APP' >'$WORK/summary.json'; RC=\$?
  kill \$DRIVER 2>/dev/null; exit \$RC
" ; RC=$?
cat "$WORK/summary.json"

echo "[e2e] produced files:"
find "$CAPTUREONE_OUTPUT_DIR" -maxdepth 1 -type f -printf '%f\t%s bytes\n' | sort
for f in "$CAPTUREONE_OUTPUT_DIR"/*.mp4; do
  [ -e "$f" ] || continue
  echo "[e2e] ffprobe $(basename "$f"):"
  ffprobe -v error -show_entries format=duration,size:stream=codec_type,codec_name,width,height,avg_frame_rate -of compact "$f"
done
echo "[e2e] library rows: $(sqlite3 "$XDG_DATA_HOME/dev.captureone.desktop/library.sqlite3" 'select count(*) from media' 2>/dev/null || echo 'sqlite3 not installed')"
echo "[e2e] app log: $XDG_DATA_HOME/dev.captureone.desktop/logs (if any), tauri-driver log: $WORK/tauri-driver.log"
exit $RC
