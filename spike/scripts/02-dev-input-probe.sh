#!/usr/bin/env bash
# Spike 02: /dev/input readability & writability under SELinux Enforcing
# Usage: ./02-dev-input-probe.sh [serial]
set -euo pipefail
DEV="${1:-${DEV:-}}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")

adb "${SEL[@]}" shell '
echo "=== ls -lZ /dev/input ==="; ls -lZ /dev/input/
echo "=== permission-bit write test ==="
for f in /dev/input/event* /dev/input/mice; do
  [ -w "$f" ] && echo "w-ok $f" || echo "w-no $f"
done
echo "=== real open-for-write test (0 bytes, safe) ==="
for f in /dev/input/event*; do
  if ( : > "$f" ) 2>/dev/null; then echo "open-w-ok $f"; else echo "open-w-DENIED $f"; fi
done
echo "=== read test ==="
for f in /dev/input/event*; do
  dd if="$f" bs=1 count=0 2>/dev/null && echo "r-ok $f" || echo "r-no $f"
done
echo "=== getevent -pl (device enum) ==="
timeout 5 getevent -pl 2>&1
echo "=== avc denials ==="
dmesg 2>/dev/null | grep -i "avc.*denied" | tail -10 || echo "(dmesg unavailable or empty)"
'
