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
shell_pid="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "pidof a26-shell"' | tr -d '\r')"
pidfd_count="$(adb -s "$SERIAL" shell "/data/local/tmp/su -c 'ls -l /proc/$shell_pid/fd | grep -c \"anon_inode:\[pidfd\]\" || true'" | tr -d '\r')"
[[ "$pidfd_count" -ge 1 ]]

# The process enters its freezer group before exec, so every child it creates
# inherits the same group without a post-spawn race.
grep -q '^6:freezer:/moon/system$' < <(adb -s "$SERIAL" shell "/data/local/tmp/su -c 'grep freezer /proc/$app_pid/cgroup'" | tr -d '\r')
state="$($CTL state)"
python3 -c 'import json,sys; app=next(a for a in json.load(sys.stdin)["result"]["apps"] if a["app"] == "system"); assert app["freezer_cgroup"] == "/moon/system"; assert app["freezer_state"] == "thawed"' <<<"$state"

# Reproduce the physical bottom-edge gesture through the same reducer path.
"$CTL" pointer-begin 540 2290 >/dev/null
"$CTL" pointer-move 540 1810 >/dev/null
"$CTL" pointer-end 540 1690 >/dev/null
state="$($CTL state)"
[[ "$(field view <<<"$state")" == launcher ]]
[[ "$(field last_action <<<"$state")" == swipe_up_background ]]
background_pid="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "pidof a26-system 2>/dev/null || true"' | tr -d '\r')"
[[ "$background_pid" == "$app_pid" ]]
background_window="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "A26_ROOT=/data/local/a26-linux A26_BUSYBOX=/data/local/a26-linux/busybox.static /system/bin/sh /data/local/a26-linux/a26-enter-chroot.sh /bin/sh -lc '\''DISPLAY=:0 xwininfo -name a26-system 2>/dev/null || true'\''"' | tr -d '\r')"
grep -q 'Map State: IsUnMapped' <<<"$background_window"
for _ in $(seq 1 50); do
    freezer_state="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "cat /dev/freezer/moon/system/freezer.state"' | tr -d '\r')"
    [[ "$freezer_state" == FROZEN ]] && break
    sleep 0.1
done
[[ "$freezer_state" == FROZEN ]]
state="$($CTL state)"
python3 -c 'import json,sys; app=next(a for a in json.load(sys.stdin)["result"]["apps"] if a["app"] == "system"); assert app["lifecycle"] == "background"; assert app["freezer_state"] == "frozen"' <<<"$state"

# A bounded media lease temporarily thaws the hidden process tree. Releasing
# the final lease immediately restores ordinary background freezing.
"$CTL" lease acquire system media 2 >/dev/null
[[ "$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "cat /dev/freezer/moon/system/freezer.state"' | tr -d '\r')" == THAWED ]]
state="$($CTL state)"
python3 -c 'import json,sys; app=next(a for a in json.load(sys.stdin)["result"]["apps"] if a["app"] == "system"); assert app["leases"][0]["kind"] == "media"; assert 0 < app["leases"][0]["remaining_ms"] <= 2000' <<<"$state"
"$CTL" lease release system media >/dev/null
[[ "$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "cat /dev/freezer/moon/system/freezer.state"' | tr -d '\r')" == FROZEN ]]

# Reopening thaws first, then resumes the same process and remaps its window.
"$CTL" launch system >/dev/null
for _ in $(seq 1 50); do
    resumed_window="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "A26_ROOT=/data/local/a26-linux A26_BUSYBOX=/data/local/a26-linux/busybox.static /system/bin/sh /data/local/a26-linux/a26-enter-chroot.sh /bin/sh -lc '\''DISPLAY=:0 xwininfo -name a26-system 2>/dev/null || true'\''"' | tr -d '\r')"
    grep -q 'Map State: IsViewable' <<<"$resumed_window" && break
    sleep 0.1
done
[[ "$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "pidof a26-system 2>/dev/null || true"' | tr -d '\r')" == "$background_pid" ]]
grep -q 'Map State: IsViewable' <<<"$resumed_window"
[[ "$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "cat /dev/freezer/moon/system/freezer.state"' | tr -d '\r')" == THAWED ]]
"$CTL" swipe-up >/dev/null

before="$(field volume <<<"$state")"
if [[ "$before" -ge 100 ]]; then
    "$CTL" volume down >/dev/null
    expected=$((before - 5))
else
    "$CTL" volume up >/dev/null
    expected=$((before + 5))
fi
after="$(field volume <<<"$($CTL state)")"
[[ "$after" -eq "$expected" ]]

# Deterministically exercise the same one-victim LRU path used by real
# MemAvailable pressure, without allocating memory on the phone.
"$CTL" memory-pressure simulate >/dev/null
state="$($CTL state)"
python3 -c 'import json,sys; d=json.load(sys.stdin)["result"]; app=next(a for a in d["apps"] if a["app"] == "system"); assert app["lifecycle"] == "stopped" and app["pid"] is None; assert d["last_action"] == "system_evicted_memory_pressure"' <<<"$state"
[[ -z "$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "cat /dev/freezer/moon/system/cgroup.procs"' | tr -d '\r')" ]]

# Power policy locks, quiesces Samsung DSI/DPU, enters mem/deep, and uses a
# bounded RTC wake. The proven production boundary then warm-reboots because
# this firmware cannot restore either Xorg's CRTC or Exynos DWC3 in place.
# Autonomous startup must return with a new boot ID, configured USB, and a fresh
# locked Moon session carrying the persisted suspend proof.
state="$($CTL state)"
deep_before="$(python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["suspend"]["deep_suspend_count"])' <<<"$state")"
boot_before="$(adb -s "$SERIAL" shell 'cat /proc/sys/kernel/random/boot_id' | tr -d '\r')"
"$CTL" suspend test 5 >/dev/null
resumed=0
for _ in $(seq 1 720); do
    if adb -s "$SERIAL" get-state >/dev/null 2>&1; then
        # Do not invoke a26-enter-chroot while autonomous Xorg is establishing
        # its own /dev, /run, and /tmp binds. A concurrent probe can unmount the
        # takeover's device tree and make Xorg report "no screens found".
        shell_ready="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c '\''pidof a26-shell >/dev/null 2>&1 && test -S /data/local/a26-linux/run/a26-shell/control.sock && echo yes || true'\''' 2>/dev/null | tr -d '\r')"
        if [[ "$shell_ready" == yes ]]; then
            state="$($CTL state 2>/dev/null || true)"
            if python3 -c 'import json,sys; d=json.load(sys.stdin)["result"]; assert d["screen_awake"] and d["suspend"]["phase"] == "awake"' <<<"$state" 2>/dev/null; then
                resumed=1
                break
            fi
        fi
    fi
    sleep 0.5
done
[[ "$resumed" == 1 ]]
boot_after="$(adb -s "$SERIAL" shell 'cat /proc/sys/kernel/random/boot_id' | tr -d '\r')"
[[ "$boot_after" != "$boot_before" ]]
[[ "$(field view <<<"$state")" == locked ]]
[[ "$(field screen_awake <<<"$state")" == true ]]
python3 -c 'import json,sys; d=json.load(sys.stdin)["result"]; s=d["suspend"]; assert s["deep_available"] is True and s["deep_suspend_count"] == int(sys.argv[1]) + 1; assert s["last_suspend_ms"] >= 2000 and s["last_error"] is None; assert d["managed_windows"] == []' "$deep_before" <<<"$state"
case "$(adb -s "$SERIAL" shell getprop sys.usb.state | tr -d '\r')" in *adb*) ;; *) exit 41 ;; esac
[[ "$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "cat /sys/class/backlight/panel/brightness"' | tr -d '\r')" -gt 0 ]]

[[ "$(field pin_digits <<<"$state")" == 0 ]]

mkdir -p "$PROJECT_ROOT/notes/a26-shell"
printf '%s\n' "$state" >"$PROJECT_ROOT/notes/a26-shell/final-state.json"
adb -s "$SERIAL" shell '/data/local/tmp/su -c "A26_ROOT=/data/local/a26-linux A26_BUSYBOX=/data/local/a26-linux/busybox.static /system/bin/sh /data/local/a26-linux/a26-enter-chroot.sh /bin/sh -lc '\''DISPLAY=:0 xwininfo -root -tree; echo; DISPLAY=:0 xinput --list --long A26-Touchscreen'\''"' >"$PROJECT_ROOT/notes/a26-shell/x11-window-and-touch-proof.txt"
grep -q 'moon-shell' "$PROJECT_ROOT/notes/a26-shell/x11-window-and-touch-proof.txt"
grep -q 'Max number of touches: 10' "$PROJECT_ROOT/notes/a26-shell/x11-window-and-touch-proof.txt"

echo "A26_SHELL_INTEGRATION_TEST=PASS"
echo "FINAL_VIEW=locked"
echo "IPC_CONTROL=ready"
