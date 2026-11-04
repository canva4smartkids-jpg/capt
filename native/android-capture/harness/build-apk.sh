#!/usr/bin/env bash
# Builds the framework-only CaptureOne engine harness APK without AGP, using only the Android SDK's
# build-tools (aapt2, d8, zipalign, apksigner) and the host-mode Gradle build (Kotlin against android.jar).
# Output: artifacts/android/captureone-engine-harness.apk (signed).
#
# Signing key (first match): env CAPTUREONE_KEYSTORE / CAPTUREONE_KEYSTORE_PASSWORD / CAPTUREONE_KEY_ALIAS
# [/ CAPTUREONE_KEY_PASSWORD], else captureone.signing.* in ~/.gradle/gradle.properties, else a throwaway
# key generated under build/. Keystores never live in the repository.
set -euo pipefail

HARNESS="$(cd "$(dirname "$0")" && pwd)"
CAPTURE="$(cd "$HARNESS/.." && pwd)"
REPO="$(cd "$CAPTURE/../.." && pwd)"
ROOT="$REPO/apps/android"
SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-/opt/android-sdk}}"
BT="$SDK/build-tools/35.0.1"
ANDROID_JAR="$SDK/platforms/android-35/android.jar"
GRADLE="${GRADLE:-$( [ -x /opt/gradle-8.14.3/bin/gradle ] && echo /opt/gradle-8.14.3/bin/gradle || echo "$ROOT/gradlew")}"
W="$HARNESS/build/apk"
OUT_DIR="$REPO/artifacts/android"
rm -rf "$W" && mkdir -p "$W" "$OUT_DIR"

echo "== compile (host mode, Kotlin against android.jar)"
(cd "$ROOT" && "$GRADLE" -Pcaptureone.hostOnly=true --console=plain -q :harness:jar :harness:harnessRuntimeClasspath)
JARS=("$HARNESS/build/libs/harness.jar" "$HARNESS"/build/runtime-classpath/*.jar)

echo "== d8 --min-api 26"
mkdir -p "$W/dex"
"$BT/d8" --release --min-api 26 --lib "$ANDROID_JAR" --output "$W/dex" "${JARS[@]}" > "$W/d8.log" 2>&1 || { cat "$W/d8.log"; exit 1; }

echo "== manifest merge (harness + :capture)"
MERGER_CP="$(find "$SDK/cmdline-tools/latest/lib" -name '*.jar' | tr '\n' ':')"
sed 's#<manifest xmlns:android="http://schemas.android.com/apk/res/android">#<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="dev.captureone.capture"><uses-sdk android:minSdkVersion="26" android:targetSdkVersion="35"/>#' \
  "$CAPTURE/src/main/AndroidManifest.xml" > "$W/capture-manifest.xml"
sed 's#<manifest xmlns:android="http://schemas.android.com/apk/res/android">#<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="dev.captureone.harness">#' \
  "$HARNESS/src/main/AndroidManifest.xml" > "$W/main-manifest.xml"
java -cp "$MERGER_CP" com.android.manifmerger.Merger --main "$W/main-manifest.xml" --libs "$W/capture-manifest.xml" \
  --property min_sdk_version=26 --property target_sdk_version=35 --property version_code=1 --property version_name=0.1.0-harness \
  --out "$W/AndroidManifest.xml"

echo "== assets (presets.json copied from packages/settings at build time)"
mkdir -p "$W/assets/captureone"
cp "$REPO/packages/settings/presets.json" "$W/assets/captureone/presets.json"

echo "== aapt2 compile + link"
"$BT/aapt2" compile --dir "$CAPTURE/src/main/res" -o "$W/capture-res.zip"
"$BT/aapt2" link -o "$W/unaligned.apk" -I "$ANDROID_JAR" --manifest "$W/AndroidManifest.xml" \
  --min-sdk-version 26 --target-sdk-version 35 -A "$W/assets" "$W/capture-res.zip"

echo "== add dex + Java resources"
python3 -I - "$W/unaligned.apk" "$W/dex" "${JARS[@]}" <<'PY'
import sys, zipfile, os
apk, dexdir, jars = sys.argv[1], sys.argv[2], sys.argv[3:]
with zipfile.ZipFile(apk, "a", zipfile.ZIP_DEFLATED) as z:
    names = set(z.namelist())
    for f in sorted(os.listdir(dexdir)):
        z.write(os.path.join(dexdir, f), f)
        names.add(f)
    for j in jars:
        with zipfile.ZipFile(j) as src:
            for n in src.namelist():
                if n.endswith("/") or n.endswith(".class") or n in names: continue
                if n.startswith("META-INF/") and (n.endswith((".SF", ".RSA", ".DSA", ".EC")) or n == "META-INF/MANIFEST.MF" or n.startswith("META-INF/versions/")): continue
                z.writestr(n, src.read(n))
                names.add(n)
PY

echo "== zipalign + apksigner"
"$BT/zipalign" -p -f 4 "$W/unaligned.apk" "$W/aligned.apk"
prop() { [ -f "$HOME/.gradle/gradle.properties" ] && sed -n "s/^$1=//p" "$HOME/.gradle/gradle.properties" | tail -1; }
KS="${CAPTUREONE_KEYSTORE:-$(prop captureone.signing.storeFile)}"
KS_PASS="${CAPTUREONE_KEYSTORE_PASSWORD:-$(prop captureone.signing.storePassword)}"
KEY_ALIAS="${CAPTUREONE_KEY_ALIAS:-$(prop captureone.signing.keyAlias)}"
KEY_PASS="${CAPTUREONE_KEY_PASSWORD:-$(prop captureone.signing.keyPassword)}"
if [ -z "$KS" ] || [ ! -f "$KS" ]; then
  KS="$W/throwaway.jks"; KS_PASS="harness-throwaway"; KEY_ALIAS="harness"; KEY_PASS="$KS_PASS"
  keytool -genkeypair -keystore "$KS" -storetype PKCS12 -alias "$KEY_ALIAS" -keyalg RSA -keysize 2048 -validity 365 \
    -storepass "$KS_PASS" -keypass "$KEY_PASS" -dname "CN=CaptureOne harness (throwaway)" > /dev/null 2>&1
fi
APK="$OUT_DIR/captureone-engine-harness.apk"
KS_PASS="$KS_PASS" KEY_PASS="${KEY_PASS:-$KS_PASS}" "$BT/apksigner" sign --ks "$KS" --ks-key-alias "$KEY_ALIAS" \
  --ks-pass env:KS_PASS --key-pass env:KEY_PASS --out "$APK" "$W/aligned.apk"

echo "== verify"
"$BT/apksigner" verify --verbose --print-certs "$APK" | grep -E "Verified|Signer #1 certificate (DN|SHA-256)|v[0-9]" | head -8
"$BT/aapt2" dump badging "$APK" | grep -E "^ *(package|minSdkVersion|targetSdkVersion|uses-permission|uses-feature|launchable|leanback)"
"$BT/aapt2" dump xmltree --file AndroidManifest.xml "$APK" | grep -E "foregroundServiceType" 
unzip -l "$APK" | grep -E "classes.*\.dex|assets/captureone/presets.json"
ls -l "$APK"
