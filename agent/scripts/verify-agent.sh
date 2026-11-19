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

echo "== A1b CAPS bitmap == proto caps: INJECT=1 DUMP=2 SHOT=4 QUERY=8 BATCH=16 TEXT=64 =="
CAPS=$(echo "$HELLO" | python3 -c 'import json,sys; print(json.load(sys.stdin)["payload"]["caps"])' 2>/dev/null || echo "?")
[ "$CAPS" = "95" ] && ok "caps_bits" || bad "caps_bits" "caps=$CAPS expected 95 (INJECT|DUMP|SHOT|QUERY|BATCH|TEXT)"

echo "== A2 INJECT tap result + logcat side-effect =="
adb -s "$DEV" shell 'logcat -c' 2>/dev/null
INJ=$($CLI inject-tap 700 1600 2>&1) || true
# MIUI logs `MIUIInput: Input event injection from package: null action ...`
# for every injected event; on other ROMs grep generic injection markers.
# Marker flushes asynchronously — poll with settle (up to 3s), `-b all` since
# MIUI writes to the system buffer.
LC=0
for _ in 1 2 3 4 5 6; do
  sleep 0.5
  LC=$(adb -s "$DEV" shell 'logcat -d -b all -t 400' | grep -cE "njection from package|injectInputEvent|Input event injection" || true)
  [ "$LC" -ge 1 ] && break
done
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

echo "== M3-A1 KEY op: back keycode=4 via injectInputEvent =="
adb -s "$DEV" shell 'logcat -c' 2>/dev/null
KEY=$($CLI inject-key 4 2>&1) || true
sleep 0.5
KLC=$(adb -s "$DEV" shell 'logcat -d -b all -t 400' | grep -cE "njection from package|injectInputEvent|Input event injection" || true)
echo "$KEY" | grep -q '"ok": *true\|"ok":true' && echo "$KEY" | grep -q '"events": *2\|"events":2' \
  && ok "key_back" || bad "key_back" "resp=$KEY logcat=$KLC"

echo "== M3-A2 INJECT points sequence (kind=Swipe, 11 events) =="
PTS=$($CLI swipe-pts 700 2000 700 1400 300 10 2>&1) || true
echo "$PTS" | grep -q '"ok": *true\|"ok":true' && echo "$PTS" | grep -q '"events": *11\|"events":11' \
  && ok "swipe_points" || bad "swipe_points" "resp=$PTS"

echo "== M3-A3 SHOT quality affects bytes (30 < 80) =="
S30=$($CLI shot-stat 1.0 30 2>&1) || true
S80=$($CLI shot-stat 1.0 80 2>&1) || true
B30=$(echo "$S30" | sed -n 's/.*<\([0-9]*\)B>.*/\1/p')
B80=$(echo "$S80" | sed -n 's/.*<\([0-9]*\)B>.*/\1/p')
[ -n "$B30" ] && [ -n "$B80" ] && [ "$B30" -lt "$B80" ] \
  && ok "shot_quality" || bad "shot_quality" "q30=${B30:-?}B q80=${B80:-?}B"

echo "== M3-A4 backend force: uia forced, bogus rejected =="
BU=$($CLI inject-tap 700 1600 uia 2>&1) || true
echo "$BU" | grep -q '"ok": *true\|"ok":true' && echo "$BU" | grep -q '"backend": *"uia"\|"backend":"uia"' \
  && ok "backend_uia" || bad "backend_uia" "resp=$BU"
BB=$($CLI inject-tap 700 1600 bogus 2>&1) || true
echo "$BB" | grep -q "ERR_BACKEND" \
  && ok "backend_bogus_err" || bad "backend_bogus_err" "resp=$BB"

echo "== M2: setup deterministic editable field =="
# android:id/input is a STATIC TextView hint on MainSettings — no tap
# needed for the id-based assertions (an older version tapped a fixed
# coordinate which navigated away depending on layout -> flaky).
adb -s "$DEV" shell 'input keyevent KEYCODE_HOME' >/dev/null 2>&1
sleep 0.6
adb -s "$DEV" shell 'am start -n com.android.settings/.MainSettings' >/dev/null 2>&1
sleep 1.2

echo "== M2-A8 QUERY found + structured node =="
Q=$($CLI query '{"id":"android:id/input"}' 2>&1) || true
echo "$Q" | python3 -c 'import json,sys
p=json.load(sys.stdin)["payload"]
assert p.get("found") is True, p
n=p["node"]; assert "bounds" in n and len(n["bounds"])==4, n
print("OK node bounds",n["bounds"])' >/dev/null 2>&1 \
  && ok "query_found" || bad "query_found" "$Q"

echo "== M2-A8b QUERY attrs pruning: only requested fields returned =="
Q=$($CLI batch '[{"query":{"selector":{"id":"android:id/input"},"attrs":["text","bounds"]}}]' 2>&1) || true
echo "$Q" | python3 -c 'import json,sys
p=json.load(sys.stdin)["payload"]; n=p["results"][0]["data"]["node"]
assert set(n.keys())=={"text","bounds"}, f"node keys={sorted(n.keys())} expected [text,bounds]"
print("OK pruned",n)' >/dev/null 2>&1 \
  && ok "query_attrs" || bad "query_attrs" "$Q"

echo "== M2-A9 wait_until hit (inside BATCH) =="
W=$($CLI wait '{"id":"android:id/input"}' 3000 2>&1) || true
echo "$W" | grep -q '"code": *"OK"' && ok "wait_until_hit" || bad "wait_until_hit" "$W"

echo "== M2-A10 wait_until TIMEOUT (impossible selector, 500ms) =="
T0=$(date +%s%N)
W=$($CLI wait '{"text":"__pd_no_such_node__"}' 500 2>&1) || true
echo "$W" | grep -q '"code": *"TIMEOUT"' && ok "wait_until_timeout" || bad "wait_until_timeout" "$W"

echo "== M2-A11 TEXT -> text becomes visible in tree =="
# quicksearchbox's query_text EditText auto-focuses — deterministic
# editable target for the TEXT op (Settings search tap was layout-fragile).
adb -s "$DEV" shell 'am start -n com.android.quicksearchbox/.SearchActivityTransparent' >/dev/null 2>&1
sleep 1.2
$CLI text "pdtest" >/dev/null 2>&1 || true
sleep 0.4
Q=$($CLI query '{"text":"pdtest"}' 2>&1) || true
echo "$Q" | grep -q '"found": *true' && ok "text_visible" || bad "text_visible" "$Q"
# restore MainSettings so id=input assertions below still hold
adb -s "$DEV" shell 'am start -n com.android.settings/.MainSettings' >/dev/null 2>&1
sleep 1.0

echo "== M2-A12 BATCH per-op results =="
B=$($CLI batch '[{"inject":{"kind":"Tap","points":[{"action":0,"x":700,"y":1400,"t_ms":0},{"action":1,"x":700,"y":1400,"t_ms":75}],"wait_finish":false}},{"query":{"selector":{"id":"android:id/input"}}}]' 2>&1) || true
echo "$B" | python3 -c 'import json,sys
p=json.load(sys.stdin)["payload"]; rs=p["results"]
assert len(rs)==2, rs
assert all(r["code"]=="OK" for r in rs), rs
assert all("cost_ns" in r for r in rs), rs
assert p.get("total_ns",0)>0
print("OK")' >/dev/null 2>&1 && ok "batch_results" || bad "batch_results" "$B"

echo "== M2-A13 SHOT scale=0.5 bytes shrink + JPEG =="
$CLI shot /tmp/shot-half.jpg 0.5 >/dev/null 2>&1 || true
SZ=$(stat -c%s /tmp/shot-half.jpg 2>/dev/null || echo 0)
[ "$(xxd -p -l2 /tmp/shot-half.jpg 2>/dev/null)" = "ffd8" ] && [ "$SZ" -lt 250000 ] && [ "$SZ" -gt 5000 ] \
  && ok "shot_scale" || bad "shot_scale" "bytes=$SZ"

echo "== M2-A14 NOACK: no response frame, inject still lands =="
adb -s "$DEV" shell 'logcat -c' 2>/dev/null
NA=$($CLI noack-tap 700 1600 2>&1) || true
# MIUIInput marker writes to the system buffer and flushes asynchronously —
# poll with settle (up to 3s) instead of a single `logcat -d` snapshot.
LC=0
for _ in 1 2 3 4 5 6; do
  sleep 0.5
  LC=$(adb -s "$DEV" shell 'logcat -d -b all -t 400' | grep -cE "njection from package" || true)
  [ "$LC" -ge 1 ] && break
done
echo "$NA" | grep -q "OK: no response" && [ "$LC" -ge 1 ] \
  && ok "noack_tap" || bad "noack_tap" "out=$NA logcat=$LC"

# restore a neutral screen for subsequent runs
adb -s "$DEV" shell 'input keyevent KEYCODE_HOME' >/dev/null 2>&1 || true

echo "== A7 AGENT_STOP: CLI stop exit==0 and process gone =="
ROOT="$(cd "$HERE/../.." && pwd)"; RCLI="$ROOT/target/debug/pilotdroid-cli"
if [ -x "$RCLI" ]; then
  "$RCLI" -s "$DEV" agent stop >/dev/null 2>&1; STOP_EXIT=$?
else
  # fallback: AGENT_STOP opcode via dev client (agent exits itself)
  $CLI stop >/dev/null 2>&1; STOP_EXIT=$?
fi
sleep 0.5
ALIVE=$(adb -s "$DEV" shell 'ps -A -o ARGS' | grep -c "pilotdroid\.Agent" || true)
[ "$STOP_EXIT" = "0" ] && [ "$ALIVE" = "0" ] \
  && ok "stop_exit_clean" || bad "stop_exit_clean" "exit=$STOP_EXIT alive=$ALIVE"

# leave the device usable: relaunch the daemon for the next run
[ -x "$RCLI" ] && "$RCLI" -s "$DEV" agent start >/dev/null 2>&1 \
  || adb -s "$DEV" shell "nohup setsid env CLASSPATH=/data/local/tmp/pilotdroid/agent.jar app_process /data/local/tmp/pilotdroid pilotdroid.Agent </dev/null >/dev/null 2>&1 &"
sleep 1

echo "== summary: $PASS pass / $FAIL fail =="
python3 - "$OUT/verify-$DEV.json" <<'EOF'
import json,sys
json.dump({"assertions":"see stdout","bench":json.load(open(sys.argv[1]))},
          open(sys.argv[1],"w"),indent=2)
EOF
[ "$FAIL" -eq 0 ]
