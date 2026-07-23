#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"

adb -s "$SERIAL" get-state >/dev/null
adb -s "$SERIAL" shell \
    '/data/local/tmp/su -c "/system/bin/sh /data/adb/moon/moon-audio-start.sh"'

echo 'Moon audio bridge is ready.'
