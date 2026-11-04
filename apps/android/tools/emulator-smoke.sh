#!/usr/bin/env bash
# Runs the :capture instrumented tests on a booted emulator/device and collects evidence:
# logcat lines tagged CaptureOneTest (MediaMetadata JSON of each capture) and the media the tests
# published to Movies/ and Pictures/CaptureOne, cross-checked with ffprobe when it is installed.
# Used by the emulator job in .github/workflows/android.yml (run from apps/android).
set -uo pipefail
cd "$(dirname "$0")/.."
OUT="$PWD/build/emulator-smoke"
rm -rf "$OUT" && mkdir -p "$OUT/media"

adb logcat -c || true
./gradlew --stacktrace :capture:connectedDebugAndroidTest
rc=$?

adb logcat -d -s CaptureOneTest:I > "$OUT/logcat-captureone.txt" 2>/dev/null || true
adb logcat -d -t 400 > "$OUT/logcat-tail.txt" 2>/dev/null || true
echo "===== CaptureOneTest logcat"
cat "$OUT/logcat-captureone.txt"

echo "===== media published by the tests"
for d in /sdcard/Movies/CaptureOne /sdcard/Pictures/CaptureOne; do
  adb shell ls -l "$d" 2>/dev/null || true
  adb pull "$d" "$OUT/media/" > /dev/null 2>&1 || true
done
if command -v ffprobe > /dev/null; then
  find "$OUT/media" -type f \( -name '*.mp4' -o -name '*.png' -o -name '*.jpg' \) -print0 | while IFS= read -r -d '' f; do
    echo "----- ffprobe: ${f#"$OUT/media/"} ($(stat -c %s "$f") bytes)"
    ffprobe -v error -show_entries format=duration,format_name:stream=index,codec_type,codec_name,width,height,r_frame_rate,sample_rate,channels,nb_frames \
      -of compact=p=0 "$f" || echo "ffprobe failed"
  done
else
  echo "ffprobe not installed; skipping cross-check"
fi

# The test reports (XML) for the summary.
find ../../native/android-capture/build/outputs/androidTest-results -name '*.xml' -exec grep -ho '<testsuite [^>]*>' {} \; 2>/dev/null || true
exit $rc
