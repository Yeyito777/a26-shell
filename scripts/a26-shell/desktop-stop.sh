#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"

"$PROJECT_ROOT/scripts/a26-shell/stop.sh" || true
"$PROJECT_ROOT/scripts/a26-shell/wifi-stop.sh" || true
"$PROJECT_ROOT/scripts/stop-native-xorg.sh"

echo "A26 native desktop stopped; Android graphics restored."
