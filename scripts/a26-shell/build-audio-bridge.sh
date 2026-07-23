#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
SOURCE="$PROJECT_ROOT/src/a26-audio-bridge"
BUILD="$PROJECT_ROOT/build/a26-audio-bridge"
OUTPUT="$PROJECT_ROOT/images/a26-audio-bridge"
ANDROID_PLATFORM=35_r02
R8_VERSION=9.1.31
ANDROID_URL="https://dl.google.com/android/repository/platform-$ANDROID_PLATFORM.zip"
R8_URL="https://dl.google.com/dl/android/maven2/com/android/tools/r8/$R8_VERSION/r8-$R8_VERSION.jar"
ANDROID_ARCHIVE_SHA256=0988cacad01b38a18a47bac14a0695f246bc76c1b06c0eeb8eb0dc825ab0c8e0
ANDROID_JAR_SHA256=4566663c3876e022b4fa4ced8c8697c4ab1688267f090114fd92d027b32e619b
R8_SHA256=3b4de3053885da105e39c15212261d22653d6d1b5eb92323dd04ae913cc8286f

mkdir -p "$BUILD/deps" "$BUILD/classes" "$BUILD/dex" "$OUTPUT"

fetch() {
    local url=$1 path=$2 hash=$3
    if [[ ! -f "$path" ]] || ! printf '%s  %s\n' "$hash" "$path" | sha256sum -c - >/dev/null 2>&1; then
        rm -f "$path"
        curl -fL --retry 3 --output "$path" "$url"
    fi
    printf '%s  %s\n' "$hash" "$path" | sha256sum -c - >/dev/null
}

ANDROID_ARCHIVE="$BUILD/deps/platform-$ANDROID_PLATFORM.zip"
ANDROID_JAR="$BUILD/deps/android-35.jar"
R8_JAR="$BUILD/deps/r8-$R8_VERSION.jar"
fetch "$ANDROID_URL" "$ANDROID_ARCHIVE" "$ANDROID_ARCHIVE_SHA256"
if [[ ! -f "$ANDROID_JAR" ]] ||
   ! printf '%s  %s\n' "$ANDROID_JAR_SHA256" "$ANDROID_JAR" | sha256sum -c - >/dev/null 2>&1; then
    unzip -p "$ANDROID_ARCHIVE" android-35/android.jar >"$ANDROID_JAR.new"
    mv -f "$ANDROID_JAR.new" "$ANDROID_JAR"
fi
printf '%s  %s\n' "$ANDROID_JAR_SHA256" "$ANDROID_JAR" | sha256sum -c - >/dev/null
fetch "$R8_URL" "$R8_JAR" "$R8_SHA256"

rm -rf "$BUILD/classes" "$BUILD/dex"
mkdir -p "$BUILD/classes" "$BUILD/dex"
javac --release 8 -cp "$ANDROID_JAR" -d "$BUILD/classes" \
    "$SOURCE/moon/audio/Bridge.java"
jar --create --file "$BUILD/classes.jar" --date 2020-01-01T00:00:00Z \
    -C "$BUILD/classes" .
java -cp "$R8_JAR" com.android.tools.r8.D8 \
    --min-api 21 --lib "$ANDROID_JAR" --output "$BUILD/dex" "$BUILD/classes.jar"
jar --create --file "$OUTPUT/moon-audio-bridge.jar" --date 2020-01-01T00:00:00Z \
    -C "$BUILD/dex" classes.dex

unzip -t "$OUTPUT/moon-audio-bridge.jar" >/dev/null
unzip -l "$OUTPUT/moon-audio-bridge.jar" | grep -q 'classes.dex'
sha256sum "$OUTPUT/moon-audio-bridge.jar" > "$OUTPUT/SHA256SUMS"
(cd "$OUTPUT" && sha256sum -c SHA256SUMS)
