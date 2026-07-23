#!/system/bin/sh
set -u

ROOT=/data/local/a26-linux
PERSIST=/data/adb/moon
RUNTIME=/data/local/tmp/moon-audio
PIDFILE="$RUNTIME/bridge.pid"

if [ -r "$RUNTIME/volume" ]; then
    volume="$(cat "$RUNTIME/volume" 2>/dev/null || true)"
    case "$volume" in
        ''|*[!0-9]*) ;;
        *)
            [ "$volume" -le 100 ] 2>/dev/null || volume=100
            printf '%s\n' "$volume" >"$PERSIST/volume.new.$$"
            chown 0:0 "$PERSIST/volume.new.$$"
            chmod 0600 "$PERSIST/volume.new.$$"
            mv -f "$PERSIST/volume.new.$$" "$PERSIST/volume"
            ;;
    esac
fi

pid="$(cat "$PIDFILE" 2>/dev/null || true)"
case "$pid" in ''|*[!0-9]*) pid='' ;; esac
if [ -n "$pid" ] && [ -r "/proc/$pid/cmdline" ] &&
   tr '\000' ' ' <"/proc/$pid/cmdline" 2>/dev/null | grep -q 'moon.audio.Bridge'; then
    kill -TERM "$pid" 2>/dev/null || true
    for _ in $(seq 1 30); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 1
    done
    kill -KILL "$pid" 2>/dev/null || true
fi

rm -f "$PIDFILE" "$RUNTIME/pcm"
echo 'moon audio bridge stopped'
