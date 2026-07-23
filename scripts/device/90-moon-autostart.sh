#!/system/bin/sh

PERSIST=/data/adb/moon
SUPERVISOR="$PERSIST/moon-boot-supervisor.sh"
BB=/data/local/a26-linux/busybox.static

[ -x "$SUPERVISOR" ] || exit 0
[ -x "$BB" ] || exit 0

"$BB" nohup "$BB" setsid /system/bin/sh "$SUPERVISOR" \
    >/dev/null 2>&1 </dev/null &
exit 0
