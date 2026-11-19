#!/usr/bin/env bash
# Build the RustCade APK without Gradle: aapt2 + javac + d8 + zipalign + apksigner.
#
# The app bundles the web build (HTML/JS + the Rust core compiled to WASM) as
# assets and serves it to a WebView from a private https origin, so it runs
# fully offline and needs no INTERNET permission.
set -euo pipefail
cd "$(dirname "$0")"

SDK="${ANDROID_SDK_ROOT:-${ANDROID_HOME:-$PWD/../android-sdk}}"
BUILD_TOOLS="${BUILD_TOOLS:-34.0.0}"
PLATFORM="$SDK/platforms/android-34/android.jar"
BT="$SDK/build-tools/$BUILD_TOOLS"

case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*) EXT=".exe" ;;
  *) EXT="" ;;
esac

AAPT2="$BT/aapt2$EXT"
ZIPALIGN="$BT/zipalign$EXT"

if [ -f "$BT/apksigner.bat" ]; then
  APKSIGNER="$BT/apksigner.bat"
else
  APKSIGNER="$BT/apksigner"
fi

if [ -f "$BT/d8.bat" ]; then
  D8="$BT/d8.bat"
else
  D8="$BT/d8"
fi

for tool in "$AAPT2" "$ZIPALIGN" "$APKSIGNER" "$D8" "$PLATFORM"; do
  if [ ! -e "$tool" ]; then
    echo "missing: $tool" >&2
    echo "install with: \$SDK/cmdline-tools/latest/bin/sdkmanager --sdk_root=\"\$SDK\" 'platforms;android-34' 'build-tools;$BUILD_TOOLS'" >&2
    exit 1
  fi
done

if [ -n "${JAVA_HOME:-}" ] && [ -x "$JAVA_HOME/bin/javac" ]; then
  JAVAC="$JAVA_HOME/bin/javac"
  KEYTOOL="$JAVA_HOME/bin/keytool"
else
  JAVAC=javac
  KEYTOOL=keytool
fi

echo "==> staging web build"
# Assets are added with `jar` rather than aapt2's -A: on Windows aapt2 writes
# backslash-separated entry names, which AssetManager cannot open on device.
rm -rf assets stage build
mkdir -p stage/assets build/classes build/dex
cp -r ../web stage/assets/www

echo "==> compiling java"
find src -name '*.java' > build/sources.txt
"$JAVAC" -source 8 -target 8 -classpath "$PLATFORM" -d build/classes @build/sources.txt 2> >(grep -v 'bootstrap class path' >&2 || true)

echo "==> dexing"
find build/classes -name '*.class' > build/classlist.txt
"$D8" --lib "$PLATFORM" --output build/dex $(cat build/classlist.txt)

echo "==> packaging resources and assets"
"$AAPT2" compile --dir res -o build/res.zip
"$AAPT2" link \
  -o build/app-unsigned.apk \
  -I "$PLATFORM" \
  --manifest AndroidManifest.xml \
  --min-sdk-version 24 \
  --target-sdk-version 34 \
  build/res.zip

echo "==> adding classes.dex and assets"
cp build/dex/classes.dex build/classes.dex
(cd build && jar uf app-unsigned.apk classes.dex && jar uf app-unsigned.apk -C ../stage assets)

echo "==> aligning and signing"
"$ZIPALIGN" -f 4 build/app-unsigned.apk build/app-aligned.apk
if [ ! -f debug.keystore ]; then
  "$KEYTOOL" -genkeypair -keystore debug.keystore -alias androiddebugkey \
    -storepass android -keypass android -keyalg RSA -keysize 2048 -validity 10000 \
    -dname "CN=RustCade Debug,O=RustCade,C=US" >/dev/null 2>&1
fi
"$APKSIGNER" sign \
  --ks debug.keystore \
  --ks-pass pass:android \
  --key-pass pass:android \
  --out build/RustCade.apk \
  build/app-aligned.apk

"$APKSIGNER" verify --print-certs build/RustCade.apk | head -5
echo "==> built android/build/RustCade.apk ($(wc -c < build/RustCade.apk) bytes)"
