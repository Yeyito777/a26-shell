#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$PROJECT_ROOT/scripts/a26-shell/common.sh"
SERIAL="$(a26_resolve_serial)"

"$PROJECT_ROOT/scripts/a26-shell/ctl.sh" quit >/dev/null 2>&1 || true
for _ in $(seq 1 30); do
    pid="$(adb -s "$SERIAL" shell '/data/local/tmp/su -c "pidof a26-shell 2>/dev/null || true"' | tr -d '\r')"
    [[ -z "$pid" ]] && exit 0
    sleep 1
done
adb -s "$SERIAL" shell '/data/local/tmp/su -c "killall -TERM a26-shell 2>/dev/null || true"'
