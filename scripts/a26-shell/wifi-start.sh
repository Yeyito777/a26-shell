#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"

installed="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "test -x /data/local/a26-linux/opt/a26-system/libexec/a26-wifi-start && echo yes || true"' | tr -d '\r')"
[[ "$installed" == yes ]] || exit 0

adb -s "$SERIAL" shell '/data/local/tmp/su -c '\''set -eu
/data/local/a26-linux/busybox.static chroot /data/local/a26-linux \
    /opt/a26-system/libexec/a26-wifi-start'\'''
