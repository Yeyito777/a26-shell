#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"
OUTPUT="${1:-$PROJECT_ROOT/notes/a26-shell/current-screen.ppm}"

adb -s "$SERIAL" shell '/data/local/tmp/su -c "A26_ROOT=/data/local/a26-linux A26_BUSYBOX=/data/local/a26-linux/busybox.static /system/bin/sh /data/local/a26-linux/a26-enter-chroot.sh /usr/bin/env DISPLAY=:0 /opt/a26-shell/bin/a26-shellshot /root/a26-shell-screen.ppm; cp /data/local/a26-linux/root/a26-shell-screen.ppm /data/local/tmp/a26-shell-screen.ppm; chmod 0644 /data/local/tmp/a26-shell-screen.ppm"'
adb -s "$SERIAL" pull /data/local/tmp/a26-shell-screen.ppm "$OUTPUT" >/dev/null
echo "$OUTPUT"
