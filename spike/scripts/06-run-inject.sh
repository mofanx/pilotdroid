#!/usr/bin/env bash
# Spike 06: run the app_process hidden-API injection spike + capture logcat proof
# Usage: ./06-run-inject.sh [serial] [x] [y]
set -uo pipefail
DEV="${1:-${DEV:-}}"
X="${2:-700}"; Y="${3:-1600}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")

adb "${SEL[@]}" shell 'logcat -c' 2>/dev/null || true
/usr/bin/time -f "wall=%e" adb "${SEL[@]}" shell \
  "CLASSPATH=/data/local/tmp/spike.jar app_process /data/local/tmp SpikeInject $X $Y"
echo "=== logcat injection proof ==="
adb "${SEL[@]}" shell 'logcat -d -t 300' | grep -iE "inject|inputdispatch" | tail -20
