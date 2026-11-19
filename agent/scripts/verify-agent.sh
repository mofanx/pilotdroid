#!/usr/bin/env bash
# M1 agent device-behavior verification (written BEFORE the implementation —
# run it to watch every assertion fail, then make it pass).
#
# Usage:
#   ./verify-agent.sh [serial] [port]
#
# Requires: device reachable via adb, agent running on
# localabstract:pilotdroid_agent (deploy with ./deploy-agent.sh first).
# Writes machine-readable results to agent/data/verify-<serial>.json

set -uo pipefail
DEV="${1:-ba0d072d}"
PORT="${2:-17932}"
HERE="$(cd "$(dirname "$0")" && pwd)"
CLI="python3 $HERE/agent_client.py --port $PORT"
OUT="$HERE/../data"; mkdir -p "$OUT"
PASS=0; FAIL=0
declare -A RESULTS

ok()  { PASS=$((PASS+1)); echo "  PASS $1"; RESULTS["$1"]="pass"; }
bad() { FAIL=$((FAIL+1)); echo "  FAIL $1 :: $2"; RESULTS["$1"]="fail: $2"; }

echo "== setup: adb forward tcp:$PORT -> localabstract:pilotdroid_agent =="
adb -s "$DEV" forward "tcp:$PORT" "localabstract:pilotdroid_agent" >/dev/null

echo "== A1 HELLO_ACK fields =="
HELLO=$($CLI hello 2>&1) || true
echo "$HELLO" | python3 -c '
import json,sys
r=json.load(sys.stdin); p=r["payload"]
need=["agent","ver","proto","sdk","caps","display"]
miss=[k for k in need if k not in p]
print("OK "+json.dumps(p) if not miss else "MISS:"+",".join(miss))' > /tmp/hello.res 2>/dev/null \
  || echo "PARSE_FAIL $HELLO" > /tmp/hello.res
grep -q "^OK" /tmp/hello.res && ok "hello_ack_fields" || bad "hello_ack_fields" "$(cat /tmp/hello.res)"

echo "== A2 INJECT tap result + logcat side-effect =="
adb -s "$DEV" shell 'logcat -c' 2>/dev/null
INJ=$($CLI inject-tap 700 1600 2>&1) || true
sleep 0.5
# MIUI logs `MIUIInput: Input event injection from package: null action ...`
# for every injected event; on other ROMs grep generic injection markers.
LC=$(adb -s "$DEV" shell 'logcat -d -t 400' | grep -cE "njection from package|injectInputEvent|Input event injection" || true)
echo "$INJ" | grep -q '"ok": *true\|"ok":true' && [ "$LC" -ge 1 ] \
  && ok "inject_tap_observable" || bad "inject_tap_observable" "resp=$INJ logcat_inject_lines=$LC"

echo "== A3 INJECT swipe events =="
SW=$($CLI inject-swipe 700 2000 700 1400 300 2>&1) || true
echo "$SW" | grep -q '"ok": *true\|"ok":true' && echo "$SW" | grep -q '"events": *[0-9]' \
  && ok "inject_swipe" || bad "inject_swipe" "resp=$SW"

echo "== A4 DUMP returns valid hierarchy XML =="
$CLI dump > /tmp/dump.xml 2>/dev/null || true
grep -q "<hierarchy" /tmp/dump.xml && grep -q "<node" /tmp/dump.xml \
  && ok "dump_xml" || bad "dump_xml" "$(head -c 200 /tmp/dump.xml)"
NODES=$(grep -o "<node" /tmp/dump.xml | wc -l); echo "    nodes=$NODES"

echo "== A5 SHOT returns JPEG magic bytes =="
$CLI shot /tmp/shot.jpg >/dev/null 2>&1 || true
MAGIC=$(xxd -p -l2 /tmp/shot.jpg 2>/dev/null || echo "none")
[ "$MAGIC" = "ffd8" ] && file /tmp/shot.jpg | grep -qi jpeg \
  && ok "shot_jpeg" || bad "shot_jpeg" "magic=$MAGIC size=$(stat -c%s /tmp/shot.jpg 2>/dev/null)"

echo "== A6 PING/PONG =="
$CLI ping 2>/dev/null | grep -qE "\"type\": *4" && ok "ping_pong" \
  || bad "ping_pong" "$($CLI ping 2>&1 | head -2)"

echo "== latency bench =="
$CLI bench 30 > "$OUT/verify-$DEV.json" 2>/dev/null && cat "$OUT/verify-$DEV.json" || true

echo "== summary: $PASS pass / $FAIL fail =="
python3 - "$OUT/verify-$DEV.json" <<'EOF'
import json,sys
json.dump({"assertions":"see stdout","bench":json.load(open(sys.argv[1]))},
          open(sys.argv[1],"w"),indent=2)
EOF
[ "$FAIL" -eq 0 ]
