#!/usr/bin/env bash
# Runs the :tv instrumented investigation (TvCaptureInvestigationTest) on a booted Android TV emulator and
# prints the evidence (logcat tag CaptureOneTV, TVFACT lines, media) so it is readable in the job log.
# Run from apps/android. Arg 1: label for the output dir.
set -uo pipefail
cd "$(dirname "$0")/.."
LABEL="${1:-tv}"
OUT="$PWD/build/emulator-tv/$LABEL"
rm -rf "$OUT" && mkdir -p "$OUT/media"

echo "===== image facts"
for p in ro.build.fingerprint ro.product.model ro.build.version.sdk ro.build.characteristics; do echo "$p=$(adb shell getprop $p | tr -d '\r')"; done
adb shell pm list features | grep -iE 'leanback|television|touchscreen' || true
echo "consent resolves (pm):"
adb shell cmd package query-activities --brief -a android.media.projection.action.SCREEN_CAPTURE 2>&1 | head -5 || true
adb shell pm list packages | grep -iE 'systemui|tv|leanback' | head -20 || true

adb logcat -c || true
./gradlew --stacktrace :tv:connectedDebugAndroidTest
rc=$?

adb logcat -d -s CaptureOneTV:I CaptureOneTest:I > "$OUT/logcat-captureonetv.txt" 2>/dev/null || true
adb logcat -d -t 600 > "$OUT/logcat-tail.txt" 2>/dev/null || true
echo "===== TVFACT lines (gradle rc=$rc)"
grep -o 'TVFACT.*' "$OUT/logcat-captureonetv.txt" | cut -c1-1500
echo "===== media"
for d in /sdcard/Movies/CaptureOne /sdcard/Pictures/CaptureOne; do
  adb shell ls -l "$d" 2>/dev/null || true
  adb pull "$d" "$OUT/media/" > /dev/null 2>&1 || true
done
if command -v ffprobe > /dev/null; then
  find "$OUT/media" -type f \( -name '*.mp4' -o -name '*.png' \) -print0 | while IFS= read -r -d '' f; do
    echo "----- ffprobe: ${f#"$OUT/media/"} ($(stat -c %s "$f") bytes)"
    ffprobe -v error -show_entries format=duration,format_name:stream=codec_type,codec_name,width,height,r_frame_rate,nb_frames -of compact=p=0 "$f" || echo "ffprobe failed"
  done
fi
find ../android-tv/build/outputs/androidTest-results -name '*.xml' -exec grep -ho '<testsuite [^>]*>' {} \; 2>/dev/null || true
exit $rc
