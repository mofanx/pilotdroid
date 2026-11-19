#!/usr/bin/env bash
# Spike 03: baseline latency measurements (host end-to-end, wall clock)
# Usage: ./03-baseline-latency.sh [serial] [out-dir]
set -uo pipefail
DEV="${1:-${DEV:-}}"
OUT="${2:-$(dirname "$0")/../data}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")
mkdir -p "$OUT"

run_n() { # label, n, cmd...
  local label="$1" n="$2"; shift 2
  echo "=== $label (N=$n) ==="
  for i in $(seq "$n"); do
    /usr/bin/time -f "%e" "$@" 2>&1 | tail -1
  done
}

run_n "adb shell spawn overhead"    8 adb "${SEL[@]}" shell true
run_n "input tap e2e"              10 adb "${SEL[@]}" shell input tap 700 1600
run_n "uiautomator dump e2e"        5 adb "${SEL[@]}" shell uiautomator dump /sdcard/uidump.xml
adb "${SEL[@]}" shell 'wc -c /sdcard/uidump.xml'
run_n "screencap to device file"    5 adb "${SEL[@]}" shell screencap -p /sdcard/sc.png
run_n "exec-out screencap pipe"     5 sh -c "adb ${SEL[*]+-s $DEV} exec-out screencap -p > '$OUT/sc.png'"
ls -l "$OUT/sc.png"
