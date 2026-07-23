#!/system/bin/sh
set -u

STATE_DIR=/data/local/tmp/a26-xorg-control
TOKEN="${1:?token required}"
DELAY="${2:-90}"
LOG="$STATE_DIR/watchdog-$TOKEN.log"
mkdir -p "$STATE_DIR"
exec </dev/null >>"$LOG" 2>&1

echo "watchdog armed token=$TOKEN delay=$DELAY $(date --iso-8601=seconds 2>/dev/null || date)"
sleep "$DELAY"

active="$(cat "$STATE_DIR/active-token" 2>/dev/null || true)"
keep="$(cat "$STATE_DIR/keep-token" 2>/dev/null || true)"
if [ "$active" != "$TOKEN" ]; then
    echo "watchdog obsolete; active=$active"
    exit 0
fi
if [ "$keep" = "$TOKEN" ]; then
    echo "watchdog disarmed by matching keep token"
    exit 0
fi

echo "watchdog deadline reached; restoring Android graphics"
exec /system/bin/sh /data/local/tmp/a26-android-graphics-restore.sh
