#!/system/bin/sh
set -u

ROOT=/data/local/a26-linux
BB="$ROOT/busybox.static"
PERSIST=/data/adb/moon
BASE="$ROOT/opt/a26-audio"
RUNTIME=/data/local/tmp/moon-audio
JAR="$BASE/moon-audio-bridge.jar"
LOG="$BASE/bridge.log"
PIDFILE="$RUNTIME/bridge.pid"

valid_bridge_pid() {
    pid="$(cat "$PIDFILE" 2>/dev/null || true)"
    case "$pid" in ''|*[!0-9]*) return 1 ;; esac
    [ -r "/proc/$pid/cmdline" ] || return 1
    tr '\000' ' ' <"/proc/$pid/cmdline" 2>/dev/null | grep -q 'moon.audio.Bridge'
}

if valid_bridge_pid && kill -0 "$pid" 2>/dev/null; then
    echo "moon audio bridge already running pid=$pid"
    exit 0
fi
rm -f "$PIDFILE"

[ -x "$BB" ] || { echo "Moon BusyBox is unavailable" >&2; exit 20; }
[ -r "$JAR" ] || { echo "Moon audio bridge is unavailable" >&2; exit 21; }

# AudioTrack authorization depends on PackageManager, PermissionManager, and
# AudioService. This script must complete before the graphics takeover stops
# zygote/system_server.
framework_ready=0
for _ in $($BB seq 1 90); do
    if service check package 2>/dev/null | grep -q 'found' &&
       service check audio 2>/dev/null | grep -q 'found' &&
       [ "$(getprop init.svc.bootanim)" = stopped ]; then
        framework_ready=1
        break
    fi
    sleep 1
done
[ "$framework_ready" = 1 ] || {
    echo "Android audio authorization services are not ready" >&2
    exit 22
}

mkdir -p "$RUNTIME" "$BASE"
rm -f "$RUNTIME/pcm"
mknod "$RUNTIME/pcm" p || exit 23
if [ -r "$PERSIST/volume" ]; then
    volume="$(cat "$PERSIST/volume" 2>/dev/null || echo 50)"
else
    volume=50
fi
case "$volume" in ''|*[!0-9]*) volume=50 ;; esac
[ "$volume" -le 100 ] 2>/dev/null || volume=100
printf '%s\n' "$volume" >"$RUNTIME/volume"
chown 1000:1000 "$RUNTIME" "$RUNTIME/pcm" "$RUNTIME/volume"
chmod 0770 "$RUNTIME"
chmod 0660 "$RUNTIME/pcm" "$RUNTIME/volume"
: >"$LOG"
chown 1000:1000 "$LOG"
chmod 0640 "$LOG"

"$BB" nohup "$BB" setsid /debug_ramdisk/magisk su 1000 -c \
    "CLASSPATH=$JAR app_process /system/bin moon.audio.Bridge $RUNTIME/pcm $RUNTIME/volume $PIDFILE" \
    >>"$LOG" 2>&1 </dev/null &
launcher=$!

for _ in $($BB seq 1 150); do
    if valid_bridge_pid && kill -0 "$pid" 2>/dev/null &&
       grep -q 'moon audio bridge ready' "$LOG" 2>/dev/null; then
        echo "moon audio bridge ready pid=$pid"
        exit 0
    fi
    if ! kill -0 "$launcher" 2>/dev/null; then
        break
    fi
    "$BB" sleep 0.2
done

kill "$launcher" 2>/dev/null || true
valid_bridge_pid && kill "$pid" 2>/dev/null || true
echo 'Moon audio bridge failed to become ready' >&2
cat "$LOG" >&2 2>/dev/null || true
exit 24
