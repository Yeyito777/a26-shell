#!/system/bin/sh
set -u

ROOT=/data/local/a26-linux
BB="$ROOT/busybox.static"
PERSIST=/data/adb/moon
RUNTIME=/data/local/tmp/moon-boot
XORG_STATE=/data/local/tmp/a26-xorg-control
LOG="$PERSIST/boot.log"
CONFIG="$PERSIST/config"

mkdir -p "$PERSIST" "$RUNTIME" "$XORG_STATE"
chmod 0700 "$PERSIST" "$RUNTIME"
if [ -f "$LOG" ] && [ "$($BB stat -c %s "$LOG" 2>/dev/null || echo 0)" -gt 1048576 ]; then
    mv -f "$LOG" "$LOG.1"
fi
exec </dev/null >>"$LOG" 2>&1

log() {
    echo "moon-boot: $* [$(date --iso-8601=seconds 2>/dev/null || date)]"
}

read_uint() {
    value="$(cat "$1" 2>/dev/null || true)"
    case "$value" in
        ''|*[!0-9]*) echo "${2:-0}" ;;
        *) echo "$value" ;;
    esac
}

write_uint() {
    file=$1
    value=$2
    echo "$value" >"$file.new.$$"
    chmod 0600 "$file.new.$$"
    mv -f "$file.new.$$" "$file"
}

config_uint() {
    key=$1
    default=$2
    value="$(sed -n "s/^$key=//p" "$CONFIG" 2>/dev/null | tail -1)"
    case "$value" in
        ''|*[!0-9]*) echo "$default" ;;
        *) echo "$value" ;;
    esac
}

boot_id="$(cat /proc/sys/kernel/random/boot_id 2>/dev/null || echo unknown-$$)"
LOCK="$RUNTIME/lock"
if ! mkdir "$LOCK" 2>/dev/null; then
    owner="$(cat "$LOCK/pid" 2>/dev/null || true)"
    owner_boot="$(cat "$LOCK/boot-id" 2>/dev/null || true)"
    case "$owner" in ''|*[!0-9]*) owner='' ;; esac
    if [ "$owner_boot" = "$boot_id" ] && [ -n "$owner" ] && kill -0 "$owner" 2>/dev/null; then
        log "supervisor already running pid=$owner"
        exit 0
    fi
    rm -rf "$LOCK"
    mkdir "$LOCK" 2>/dev/null || exit 0
fi
echo $$ >"$LOCK/pid"
echo "$boot_id" >"$LOCK/boot-id"
cleanup_lock() { rm -rf "$LOCK"; }
trap cleanup_lock EXIT HUP INT TERM

log "supervisor start boot_id=$boot_id reason=$(getprop ro.boot.bootreason)"

[ ! -e "$PERSIST/disabled" ] || { log 'autostart disabled'; exit 0; }

if [ -e "$PERSIST/skip-once" ]; then
    rm -f "$PERSIST/skip-once"
    log 'consumed skip-once; leaving Android active'
    exit 0
fi

previous_attempt="$(cat "$PERSIST/last-attempt-boot-id" 2>/dev/null || true)"
[ "$previous_attempt" != "$boot_id" ] || {
    log 'this boot was already attempted; refusing a duplicate takeover'
    exit 0
}
echo "$boot_id" >"$PERSIST/last-attempt-boot-id"
chmod 0600 "$PERSIST/last-attempt-boot-id"

MIN_START_BATTERY="$(config_uint MIN_START_BATTERY 20)"
LOW_BATTERY_FALLBACK="$(config_uint LOW_BATTERY_FALLBACK 8)"
MAX_START_FAILURES="$(config_uint MAX_START_FAILURES 3)"
OVERRIDE_WINDOW_SECONDS="$(config_uint OVERRIDE_WINDOW_SECONDS 8)"

failures="$(read_uint "$PERSIST/start-failures" 0)"
if [ "$failures" -ge "$MAX_START_FAILURES" ]; then
    log "failure limit reached ($failures/$MAX_START_FAILURES); leaving Android active"
    exit 0
fi

# Magisk service.d runs before Android has necessarily finished initializing
# Samsung's vendor services. Let Android initialize/decrypt everything first.
ready=0
for _ in $($BB seq 1 360); do
    if [ "$(getprop sys.boot_completed)" = 1 ] &&
       [ -c /dev/dri/card0 ] && [ -c /dev/input/event0 ] &&
       [ -x "$ROOT/a26-enter-chroot.sh" ]; then
        ready=1
        break
    fi
    sleep 1
done
[ "$ready" = 1 ] || { log 'Android/vendor bootstrap timed out'; exit 20; }

case "$(getprop ro.boot.bootreason)" in
    *recovery*|*download*)
        log 'recovery/download boot detected; leaving Android active'
        exit 0
        ;;
esac

# A prior kernel panic gets one Android-only diagnostic boot instead of an
# automatic panic loop. A normal restart/power cycle reports another value.
reset_reason="$(cat /sys/module/sec_debug_reset_reason/parameters/reset_reason 2>/dev/null || true)"
if [ "$reset_reason" = 4 ]; then
    log 'previous boot was classified as kernel panic; leaving Android active'
    exit 0
fi

# Physical one-boot escape hatch: press either volume key during this short
# post-boot window. Input is observed but never consumed exclusively.
override_log="$RUNTIME/volume-override.$$"
rm -f "$override_log"
if [ -c /dev/input/event0 ] && [ "$OVERRIDE_WINDOW_SECONDS" -gt 0 ]; then
    /system/bin/getevent -ql /dev/input/event0 >"$override_log" 2>&1 &
    event_pid=$!
    sleep "$OVERRIDE_WINDOW_SECONDS"
    kill "$event_pid" 2>/dev/null || true
    wait "$event_pid" 2>/dev/null || true
    if grep -Eq 'KEY_VOLUME(UP|DOWN)[[:space:]]+DOWN' "$override_log"; then
        rm -f "$override_log"
        log 'volume-key override requested; leaving Android active'
        exit 0
    fi
fi
rm -f "$override_log"

capacity="$(read_uint /sys/class/power_supply/battery/capacity 100)"
if [ "$capacity" -lt "$MIN_START_BATTERY" ]; then
    online="$(read_uint /sys/class/power_supply/usb/online 0)"
    ac_online="$(read_uint /sys/class/power_supply/ac/online 0)"
    if [ "$online" != 1 ] && [ "$ac_online" != 1 ]; then
        log "battery is $capacity% without external power; leaving Android active"
        exit 0
    fi
    log "battery is $capacity%; waiting for $MIN_START_BATTERY% before Moon takeover"
    while [ "$capacity" -lt "$MIN_START_BATTERY" ]; do
        [ ! -e "$PERSIST/disabled" ] || { log 'disabled while charging'; exit 0; }
        sleep 60
        capacity="$(read_uint /sys/class/power_supply/battery/capacity 100)"
    done
fi

for required in \
    "$BB" \
    "$ROOT/a26-enter-chroot.sh" \
    "$ROOT/opt/a26-shell/bin/a26-shell" \
    "$ROOT/opt/a26-shell/bin/a26-shellctl" \
    /data/local/tmp/a26-xorg-takeover.sh \
    /data/local/tmp/a26-xorg-safety-watchdog.sh \
    /data/local/tmp/a26-android-graphics-restore.sh \
    "$ROOT/usr/local/sbin/a26-xorg-persistent-session"
do
    [ -x "$required" ] || {
        log "required executable missing: $required"
        exit 21
    }
done

failures=$((failures + 1))
write_uint "$PERSIST/start-failures" "$failures"
token="moon-${boot_id%%-*}-$failures"
rm -f "$ROOT/root/xorg-session.status"
rm -rf "$XORG_STATE/restore.lock"
: >"$XORG_STATE/takeover.log"
log "starting autonomous takeover token=$token attempt=$failures"
"$BB" nohup "$BB" setsid /system/bin/sh \
    /data/local/tmp/a26-xorg-takeover.sh "$token" \
    >/dev/null 2>&1 </dev/null &

xorg_ready=0
for _ in $($BB seq 1 100); do
    status="$(cat "$ROOT/root/xorg-session.status" 2>/dev/null || true)"
    if echo "$status" | grep -q '^XORG_READY=1$'; then
        xorg_ready=1
        break
    fi
    if echo "$status" | grep -q '^XORG_READY=0$'; then
        break
    fi
    sleep 1
done
if [ "$xorg_ready" != 1 ]; then
    log 'Xorg readiness was not proven; restoring Android'
    /system/bin/sh /data/local/tmp/a26-android-graphics-restore.sh
    exit 30
fi
echo "$token" >"$XORG_STATE/keep-token"
log "Xorg ready pid=$(pidof Xorg 2>/dev/null || true); safety token committed locally"

start_moon() {
    rm -f "$ROOT/run/a26-shell/control.sock"
    : >"$ROOT/root/a26-shell.log"
    "$BB" nohup "$BB" setsid /system/bin/sh "$ROOT/a26-enter-chroot.sh" \
        /usr/bin/env DISPLAY=:0 A26_SHELL_CONFIG=/etc/a26-shell/config.json \
        /opt/a26-shell/bin/a26-shell \
        >>"$ROOT/root/a26-shell.log" 2>&1 </dev/null &
}

start_moon
moon_ready=0
for _ in $($BB seq 1 100); do
    if "$BB" chroot "$ROOT" /opt/a26-shell/bin/a26-shellctl state \
        >"$RUNTIME/moon-state.new" 2>/dev/null; then
        mv -f "$RUNTIME/moon-state.new" "$RUNTIME/moon-state"
        moon_ready=1
        break
    fi
    "$BB" sleep 0.2
done
if [ "$moon_ready" != 1 ]; then
    log 'Moon control socket did not become ready; restoring Android'
    /system/bin/sh /data/local/tmp/a26-android-graphics-restore.sh
    exit 31
fi

write_uint "$PERSIST/start-failures" 0
echo "$boot_id" >"$PERSIST/last-success-boot-id"
chmod 0600 "$PERSIST/last-success-boot-id"
log 'Moon is the active default session'

shell_missing=0
while [ -n "$(pidof Xorg 2>/dev/null || true)" ]; do
    sleep 30
    if [ -e "$PERSIST/disabled" ]; then
        log 'disabled at runtime; restoring Android'
        /system/bin/sh /data/local/tmp/a26-android-graphics-restore.sh
        break
    fi
    capacity="$(read_uint /sys/class/power_supply/battery/capacity 100)"
    if [ "$capacity" -le "$LOW_BATTERY_FALLBACK" ]; then
        log "battery reached $capacity%; restoring Android charging mode"
        /system/bin/sh /data/local/tmp/a26-android-graphics-restore.sh
        break
    fi
    if [ -z "$(pidof a26-shell 2>/dev/null || true)" ]; then
        shell_missing=$((shell_missing + 1))
        if [ "$shell_missing" -ge 2 ]; then
            log 'Moon process remained absent; restoring Android'
            /system/bin/sh /data/local/tmp/a26-android-graphics-restore.sh
            break
        fi
    else
        shell_missing=0
    fi
done

log 'supervisor exit'
exit 0
