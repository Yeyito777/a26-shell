#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"

installed="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "test -x /data/local/tmp/a26-wifi-cleanup-android.sh && echo yes || true"' | tr -d '\r')"
[[ "$installed" == yes ]] || exit 0
adb -s "$SERIAL" shell \
    '/data/local/tmp/su -c "/system/bin/sh /data/local/tmp/a26-wifi-cleanup-android.sh"'
