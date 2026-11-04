#!/usr/bin/env bash
# Local verification where Google Maven (AGP/AndroidX/Compose) is unreachable.
# Proves what can be proven without AGP; it does NOT produce installable APKs (CI does).
#   1. host-mode Gradle build: :capture compiled against android.jar (-no-jdk), JVM unit tests of all
#      modules, Compose UI type-checked against Compose for Desktop
#   2. d8 --min-api 26 over the engine + runtime dependencies (dexability for minSdk 26)
#   3. manifest merge (app/tv + :capture) with the SDK's manifest merger
#   4. aapt2 compile + link of each app's resources and merged manifest against android.jar,
#      then `aapt2 dump badging/xmltree` (permissions, FGS type, leanback, SDK levels)
#   5. standalone Android lint over the :capture sources
# Usage: apps/android/tools/host-verify.sh [output-dir]
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"          # apps/android
REPO="$(cd "$HERE/../.." && pwd)"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/opt/android-sdk}}"
BT="$SDK/build-tools/35.0.1"
ANDROID_JAR="$SDK/platforms/android-35/android.jar"
GRADLE="${GRADLE:-$( [ -x /opt/gradle-8.14.3/bin/gradle ] && echo /opt/gradle-8.14.3/bin/gradle || echo "$HERE/gradlew")}"
OUT="${1:-$HERE/build/host-verify}"
CAPTURE="$REPO/native/android-capture"
rm -rf "$OUT" && mkdir -p "$OUT"

step() { printf '\n==== %s\n' "$*"; }

step "1/5 host-mode Gradle build + unit tests"
(cd "$HERE" && "$GRADLE" -Pcaptureone.hostOnly=true --console=plain -q \
  :capture:test :app:test :tv:test :capture:jar :capture:hostRuntimeClasspath)
python3 - "$REPO" <<'PY'
import glob, re, sys
repo = sys.argv[1]
total = fail = 0
for mod in ["native/android-capture", "apps/android/app", "apps/android-tv"]:
    t = f = 0
    for x in glob.glob(f"{repo}/{mod}/build/test-results/test/*.xml"):
        m = re.search(r'tests="(\d+)" skipped="(\d+)" failures="(\d+)" errors="(\d+)"', open(x).read())
        t += int(m[1]); f += int(m[3]) + int(m[4])
    print(f"  {mod}: {t} tests, {f} failed")
    total += t; fail += f
print(f"  total: {total} tests, {fail} failed")
sys.exit(1 if fail else 0)
PY

step "2/5 d8 --min-api 26 (engine + runtime deps)"
mkdir -p "$OUT/dex"
"$BT/d8" --release --min-api 26 --lib "$ANDROID_JAR" --output "$OUT/dex" \
  "$CAPTURE/build/libs/capture.jar" "$CAPTURE"/build/host-runtime-classpath/*.jar > "$OUT/d8.log" 2>&1 || { cat "$OUT/d8.log"; exit 1; }
# build-tools 35.0.1's D8 predates Kotlin 2.2 metadata and logs "Info:" lines about it; they are harmless for dexing.
ls -l "$OUT/dex"

step "3/5 manifest merge"
MERGER_CP="$(find "$SDK/cmdline-tools/latest/lib" -name '*.jar' | tr '\n' ':')"
# AGP injects package/minSdk into library manifests; emulate that for the standalone merger.
sed 's#<manifest xmlns:android="http://schemas.android.com/apk/res/android">#<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="dev.captureone.capture"><uses-sdk android:minSdkVersion="26" android:targetSdkVersion="35"/>#' \
  "$CAPTURE/src/main/AndroidManifest.xml" > "$OUT/capture-manifest.xml"
merge() { # name package main-manifest
  sed "s#<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\">#<manifest xmlns:android=\"http://schemas.android.com/apk/res/android\" package=\"$2\">#" "$3" > "$OUT/$1-main.xml"
  java -cp "$MERGER_CP" com.android.manifmerger.Merger --main "$OUT/$1-main.xml" --libs "$OUT/capture-manifest.xml" \
    --property min_sdk_version=26 --property target_sdk_version=35 --property version_code=1 --property version_name=0.1.0 \
    --out "$OUT/$1-merged.xml"
}
merge app dev.captureone.android "$HERE/app/src/main/AndroidManifest.xml"
merge tv dev.captureone.tv "$REPO/apps/android-tv/src/main/AndroidManifest.xml"

step "4/5 aapt2 compile + link (resources + merged manifests) and dump"
"$BT/aapt2" compile --dir "$CAPTURE/src/main/res" -o "$OUT/capture-res.zip"
link() { # name resdir
  "$BT/aapt2" compile --dir "$2" -o "$OUT/$1-res.zip"
  "$BT/aapt2" link -o "$OUT/$1-resources.apk" -I "$ANDROID_JAR" --manifest "$OUT/$1-merged.xml" \
    --min-sdk-version 26 --target-sdk-version 35 --auto-add-overlay "$OUT/$1-res.zip" -R "$OUT/capture-res.zip"
  "$BT/aapt2" dump badging "$OUT/$1-resources.apk" > "$OUT/$1-badging.txt"
  "$BT/aapt2" dump xmltree --file AndroidManifest.xml "$OUT/$1-resources.apk" > "$OUT/$1-manifest-tree.txt"
  echo "--- $1 badging (resource-only APK, not the shipping APK)"
  grep -E "^ *(package|minSdkVersion|targetSdkVersion|uses-permission|uses-feature|leanback-launchable|launchable)" "$OUT/$1-badging.txt" || true
  echo "--- $1 services"
  grep -A3 "E: service" "$OUT/$1-manifest-tree.txt" | grep -E "name|foregroundServiceType|permission" || true
}
link app "$HERE/app/src/main/res"
link tv "$REPO/apps/android-tv/src/main/res"
grep -q "foregroundServiceType" "$OUT/app-manifest-tree.txt"
grep -q "uses-feature: name='android.software.leanback'" "$OUT/tv-badging.txt"
grep -q "leanback-launchable-activity" "$OUT/tv-badging.txt"

step "5/5 standalone lint on :capture"
LINT_CP="$CAPTURE/build/classes/kotlin/main"
LIBS="$(ls "$CAPTURE"/build/host-runtime-classpath/*.jar | tr '\n' ':')"
mkdir -p "$OUT/lint-project"
cp "$OUT/capture-manifest.xml" "$OUT/lint-project/AndroidManifest.xml"
ANDROID_HOME="$SDK" "$SDK/cmdline-tools/latest/bin/lint" --quiet --exitcode --sdk-home "$SDK" --compile-sdk-version 35 \
  --sources "$CAPTURE/src/main/kotlin" --resources "$CAPTURE/src/main/res" --classpath "$LINT_CP" --libraries "$LIBS" \
  --disable HardcodedText,SetTextI18n,GradleDependency,NewerVersionAvailable,MissingApplicationIcon,DiscouragedApi \
  --xml "$OUT/lint-capture.xml" --text "$OUT/lint-capture.txt" "$OUT/lint-project" 2>/dev/null || LINT_RC=$?
echo "lint exit code: ${LINT_RC:-0} (report: $OUT/lint-capture.txt)"
tail -3 "$OUT/lint-capture.txt" || true

step "done: outputs in $OUT"
