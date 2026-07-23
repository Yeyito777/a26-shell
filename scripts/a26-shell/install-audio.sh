#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"

"$PROJECT_ROOT/scripts/a26-shell/build-audio-bridge.sh"
AUDIO_JAR="$PROJECT_ROOT/images/a26-audio-bridge/moon-audio-bridge.jar"
START="$PROJECT_ROOT/scripts/device/moon-audio-start.sh"
STOP="$PROJECT_ROOT/scripts/device/moon-audio-stop.sh"
CHROOT="$PROJECT_ROOT/scripts/device/a26-enter-chroot.sh"

for script in "$START" "$STOP" "$CHROOT"; do
    sh -n "$script"
done

adb -s "$SERIAL" get-state >/dev/null
adb -s "$SERIAL" push "$AUDIO_JAR" /data/local/tmp/moon-audio-bridge.jar >/dev/null
adb -s "$SERIAL" push "$START" /data/local/tmp/moon-audio-start.sh >/dev/null
adb -s "$SERIAL" push "$STOP" /data/local/tmp/moon-audio-stop.sh >/dev/null
adb -s "$SERIAL" push "$CHROOT" /data/local/tmp/a26-enter-chroot.sh >/dev/null

adb -s "$SERIAL" shell '/data/local/tmp/su -c '\''
set -eu
mkdir -p /data/adb/moon /data/local/a26-linux/opt/a26-audio
chmod 0700 /data/adb/moon

cp /data/local/tmp/moon-audio-start.sh /data/adb/moon/moon-audio-start.sh.new
cp /data/local/tmp/moon-audio-stop.sh /data/adb/moon/moon-audio-stop.sh.new
cp /data/local/tmp/moon-audio-bridge.jar \
    /data/local/a26-linux/opt/a26-audio/moon-audio-bridge.jar.new
cp /data/local/tmp/a26-enter-chroot.sh \
    /data/local/a26-linux/a26-enter-chroot.sh.new
chown 0:0 \
    /data/adb/moon/moon-audio-start.sh.new \
    /data/adb/moon/moon-audio-stop.sh.new \
    /data/local/a26-linux/opt/a26-audio/moon-audio-bridge.jar.new \
    /data/local/a26-linux/a26-enter-chroot.sh.new
chmod 0700 \
    /data/adb/moon/moon-audio-start.sh.new \
    /data/adb/moon/moon-audio-stop.sh.new
chmod 0644 /data/local/a26-linux/opt/a26-audio/moon-audio-bridge.jar.new
chmod 0755 /data/local/a26-linux/a26-enter-chroot.sh.new

mv -f /data/adb/moon/moon-audio-start.sh.new \
    /data/adb/moon/moon-audio-start.sh
mv -f /data/adb/moon/moon-audio-stop.sh.new \
    /data/adb/moon/moon-audio-stop.sh
mv -f /data/local/a26-linux/opt/a26-audio/moon-audio-bridge.jar.new \
    /data/local/a26-linux/opt/a26-audio/moon-audio-bridge.jar
mv -f /data/local/a26-linux/a26-enter-chroot.sh.new \
    /data/local/a26-linux/a26-enter-chroot.sh

rm -f \
    /data/local/tmp/moon-audio-start.sh \
    /data/local/tmp/moon-audio-stop.sh \
    /data/local/tmp/moon-audio-bridge.jar \
    /data/local/tmp/a26-enter-chroot.sh
'\'''

expected="$(sha256sum "$AUDIO_JAR" | awk '{print $1}')"
actual="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c '\''sha256sum /data/local/a26-linux/opt/a26-audio/moon-audio-bridge.jar | cut -d" " -f1'\''' | tr -d '\r')"
[[ "$actual" == "$expected" ]]
echo 'Installed the Moon AudioTrack bridge and native-session runtime helpers.'
