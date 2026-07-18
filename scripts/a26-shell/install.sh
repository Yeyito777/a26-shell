#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"
IMAGE="$PROJECT_ROOT/images/a26-shell-0.1.0"
CONFIG="$PROJECT_ROOT/build/a26-shell/config.json"

"$PROJECT_ROOT/scripts/a26-shell/build.sh"
"$PROJECT_ROOT/scripts/a26-shell/generate-config.py" "$CONFIG"

adb -s "$SERIAL" get-state >/dev/null
adb -s "$SERIAL" push "$IMAGE/bin/a26-shell" /data/local/tmp/a26-shell >/dev/null
adb -s "$SERIAL" push "$IMAGE/bin/a26-shellctl" /data/local/tmp/a26-shellctl >/dev/null
adb -s "$SERIAL" push "$IMAGE/bin/a26-shellshot" /data/local/tmp/a26-shellshot >/dev/null
adb -s "$SERIAL" push "$IMAGE/source/a26-shell-0.1.0.tar.gz" /data/local/tmp/a26-shell-source.tar.gz >/dev/null
adb -s "$SERIAL" push "$CONFIG" /data/local/tmp/a26-shell-config.json >/dev/null

adb -s "$SERIAL" shell '/data/local/tmp/su -c "set -e; mkdir -p /data/local/a26-linux/opt/a26-shell/bin /data/local/a26-linux/opt/a26-shell/source /data/local/a26-linux/etc/a26-shell; cp /data/local/tmp/a26-shell /data/local/a26-linux/opt/a26-shell/bin/a26-shell.new; cp /data/local/tmp/a26-shellctl /data/local/a26-linux/opt/a26-shell/bin/a26-shellctl.new; cp /data/local/tmp/a26-shellshot /data/local/a26-linux/opt/a26-shell/bin/a26-shellshot.new; cp /data/local/tmp/a26-shell-source.tar.gz /data/local/a26-linux/opt/a26-shell/source/a26-shell-0.1.0.tar.gz.new; cp /data/local/tmp/a26-shell-config.json /data/local/a26-linux/etc/a26-shell/config.json.new; chown 0:0 /data/local/a26-linux/opt/a26-shell/bin/a26-shell.new /data/local/a26-linux/opt/a26-shell/bin/a26-shellctl.new /data/local/a26-linux/opt/a26-shell/bin/a26-shellshot.new /data/local/a26-linux/opt/a26-shell/source/a26-shell-0.1.0.tar.gz.new /data/local/a26-linux/etc/a26-shell/config.json.new; chmod 0755 /data/local/a26-linux/opt/a26-shell/bin/a26-shell.new /data/local/a26-linux/opt/a26-shell/bin/a26-shellctl.new /data/local/a26-linux/opt/a26-shell/bin/a26-shellshot.new; chmod 0644 /data/local/a26-linux/opt/a26-shell/source/a26-shell-0.1.0.tar.gz.new; chmod 0600 /data/local/a26-linux/etc/a26-shell/config.json.new; mv -f /data/local/a26-linux/opt/a26-shell/bin/a26-shell.new /data/local/a26-linux/opt/a26-shell/bin/a26-shell; mv -f /data/local/a26-linux/opt/a26-shell/bin/a26-shellctl.new /data/local/a26-linux/opt/a26-shell/bin/a26-shellctl; mv -f /data/local/a26-linux/opt/a26-shell/bin/a26-shellshot.new /data/local/a26-linux/opt/a26-shell/bin/a26-shellshot; mv -f /data/local/a26-linux/opt/a26-shell/source/a26-shell-0.1.0.tar.gz.new /data/local/a26-linux/opt/a26-shell/source/a26-shell-0.1.0.tar.gz; mv -f /data/local/a26-linux/etc/a26-shell/config.json.new /data/local/a26-linux/etc/a26-shell/config.json; chmod 0700 /data/local/a26-linux/etc/a26-shell; sha256sum /data/local/a26-linux/opt/a26-shell/bin/a26-shell /data/local/a26-linux/opt/a26-shell/bin/a26-shellctl /data/local/a26-linux/opt/a26-shell/bin/a26-shellshot"'

expected_shell="$(sha256sum "$IMAGE/bin/a26-shell" | awk '{print $1}')"
expected_ctl="$(sha256sum "$IMAGE/bin/a26-shellctl" | awk '{print $1}')"
expected_shot="$(sha256sum "$IMAGE/bin/a26-shellshot" | awk '{print $1}')"
phone_hashes="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "sha256sum /data/local/a26-linux/opt/a26-shell/bin/a26-shell /data/local/a26-linux/opt/a26-shell/bin/a26-shellctl /data/local/a26-linux/opt/a26-shell/bin/a26-shellshot"' | tr -d '\r')"
grep -q "^$expected_shell  .*a26-shell$" <<<"$phone_hashes"
grep -q "^$expected_ctl  .*a26-shellctl$" <<<"$phone_hashes"
grep -q "^$expected_shot  .*a26-shellshot$" <<<"$phone_hashes"

echo "Installed a26-shell 0.1.0 under /opt/a26-shell in the phone rootfs."
