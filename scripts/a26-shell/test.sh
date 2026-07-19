#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
CTL="$PROJECT_ROOT/scripts/a26-shell/ctl.sh"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"
: "${A26_SHELL_PIN:?A26_SHELL_PIN must be present in the environment}"

field() {
    python3 -c 'import json,sys; data=json.load(sys.stdin); value=data["result"][sys.argv[1]]; print(str(value).lower() if isinstance(value, bool) else value)' "$1"
}

state="$($CTL state)"
[[ "$(field view <<<"$state")" == locked ]]

# A known-wrong value must not unlock. This is intentionally not the real PIN.
for _ in 1 2 3 4 5 6; do "$CTL" tap 540 1685 >/dev/null; done
state="$($CTL state)"
[[ "$(field view <<<"$state")" == locked ]]
[[ "$(field failed_attempts <<<"$state")" == 1 ]]

# Inject the provisioned PIN without logging or embedding it in project source.
while IFS= read -r -n1 digit; do
    # adb otherwise inherits the here-string and consumes the remaining PIN.
    [[ -n "$digit" ]] || continue
    case "$digit" in
        1) coordinates=(270 830) ;;
        2) coordinates=(540 830) ;;
        3) coordinates=(810 830) ;;
        4) coordinates=(270 1115) ;;
        5) coordinates=(540 1115) ;;
        6) coordinates=(810 1115) ;;
        7) coordinates=(270 1400) ;;
        8) coordinates=(540 1400) ;;
        9) coordinates=(810 1400) ;;
        0) coordinates=(540 1685) ;;
        *) exit 11 ;;
    esac
    "$CTL" tap "${coordinates[0]}" "${coordinates[1]}" >/dev/null </dev/null
done <<<"$A26_SHELL_PIN"
state="$($CTL state)"
[[ "$(field view <<<"$state")" == launcher ]]

"$CTL" tap 255 555 >/dev/null
state="$($CTL state)"
[[ "$(field view <<<"$state")" == system ]]
[[ "$(field current_app <<<"$state")" == System ]]

# The System scene must be a separately managed process/window, not an
# in-process shell renderer.
external_ready=0
for _ in $(seq 1 50); do
    app_pid="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "pidof a26-system 2>/dev/null || true"' | tr -d '\r')"
    app_window="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "A26_ROOT=/data/local/a26-linux A26_BUSYBOX=/data/local/a26-linux/busybox.static /system/bin/sh /data/local/a26-linux/a26-enter-chroot.sh /bin/sh -lc '\''DISPLAY=:0 xwininfo -name a26-system 2>/dev/null || true'\''"' | tr -d '\r')"
    if [[ -n "$app_pid" ]] && grep -q 'Map State: IsViewable' <<<"$app_window"; then
        external_ready=1
        break
    fi
    sleep 0.1
done
[[ "$external_ready" == 1 ]]

# Reproduce the physical bottom-edge gesture through the same reducer path.
"$CTL" pointer-begin 540 2290 >/dev/null
"$CTL" pointer-move 540 1810 >/dev/null
"$CTL" pointer-end 540 1690 >/dev/null
state="$($CTL state)"
[[ "$(field view <<<"$state")" == launcher ]]
[[ "$(field last_action <<<"$state")" == swipe_up_close ]]
for _ in $(seq 1 50); do
    app_pid="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "pidof a26-system 2>/dev/null || true"' | tr -d '\r')"
    [[ -z "$app_pid" ]] && break
    sleep 0.1
done
[[ -z "$app_pid" ]]

before="$(field volume <<<"$state")"
"$CTL" volume up >/dev/null
after="$(field volume <<<"$($CTL state)")"
[[ "$after" -eq $((before + 5)) ]]

# Power policy locks before blanking and wakes only to the lock screen.
"$CTL" screen off >/dev/null
state="$($CTL state)"
[[ "$(field view <<<"$state")" == locked ]]
[[ "$(field screen_awake <<<"$state")" == false ]]
"$CTL" screen on >/dev/null
state="$($CTL state)"
[[ "$(field view <<<"$state")" == locked ]]
[[ "$(field screen_awake <<<"$state")" == true ]]

"$CTL" lock >/dev/null
state="$($CTL state)"
[[ "$(field view <<<"$state")" == locked ]]
[[ "$(field pin_digits <<<"$state")" == 0 ]]

mkdir -p "$PROJECT_ROOT/notes/a26-shell"
printf '%s\n' "$state" >"$PROJECT_ROOT/notes/a26-shell/final-state.json"
adb -s "$SERIAL" shell '/data/local/tmp/su -c "A26_ROOT=/data/local/a26-linux A26_BUSYBOX=/data/local/a26-linux/busybox.static /system/bin/sh /data/local/a26-linux/a26-enter-chroot.sh /bin/sh -lc '\''DISPLAY=:0 xwininfo -root -tree; echo; DISPLAY=:0 xinput --list --long A26-Touchscreen'\''"' >"$PROJECT_ROOT/notes/a26-shell/x11-window-and-touch-proof.txt"
grep -q 'a26-shell-ui' "$PROJECT_ROOT/notes/a26-shell/x11-window-and-touch-proof.txt"
grep -q 'Max number of touches: 10' "$PROJECT_ROOT/notes/a26-shell/x11-window-and-touch-proof.txt"

echo "A26_SHELL_INTEGRATION_TEST=PASS"
echo "FINAL_VIEW=locked"
echo "IPC_CONTROL=ready"
