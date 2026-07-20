#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"

adb -s "$SERIAL" get-state >/dev/null
xorg="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "pidof Xorg 2>/dev/null || true"' | tr -d '\r')"
[[ -n "$xorg" ]] || { echo "Xorg is not running" >&2; exit 20; }

"$PROJECT_ROOT/scripts/a26-shell/wifi-start.sh"
"$PROJECT_ROOT/scripts/a26-shell/stop.sh" >/dev/null 2>&1 || true
"$PROJECT_ROOT/scripts/wm-stop.sh" >/dev/null 2>&1 || true
adb -s "$SERIAL" shell '/data/local/tmp/su -c ": > /data/local/a26-linux/root/a26-shell.log; /data/local/a26-linux/busybox.static nohup /data/local/a26-linux/busybox.static setsid /system/bin/sh /data/local/a26-linux/a26-enter-chroot.sh /usr/bin/env DISPLAY=:0 A26_SHELL_CONFIG=/etc/a26-shell/config.json /opt/a26-shell/bin/a26-shell >>/data/local/a26-linux/root/a26-shell.log 2>&1 </dev/null &"'

for _ in $(seq 1 60); do
    if "$PROJECT_ROOT/scripts/a26-shell/ctl.sh" state >/tmp/a26-shell-state.json 2>/dev/null; then
        cat /tmp/a26-shell-state.json
        exit 0
    fi
    sleep 1
done
adb -s "$SERIAL" shell '/data/local/tmp/su -c "cat /data/local/a26-linux/root/a26-shell.log"' >&2
exit 30
