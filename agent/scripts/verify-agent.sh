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

# MIUI's UiAutomation capture intermittently hands back a null bitmap
# (`cap: null bitmap` in agent.log — a transient, not an agent defect).
# Bounded retry before counting a real miss; same precedent as the
# launch+read retry in the M11 watch section below.
shot_file() {  # shot_file <out.jpg> [scale] — retry until JPEG magic
  for _ in 1 2 3 4 5 6 7 8; do
    $CLI shot "$@" >/dev/null 2>&1 || true
    [ "$(xxd -p -l2 "$1" 2>/dev/null)" = "ffd8" ] && return 0
    sleep 1
  done
  echo "  [shot-retry] last call: $($CLI shot "$@" 2>&1 | head -2 | tr '\n' ' ')"
  return 1
}
shot_stat() {  # shot_stat <scale> <quality> — echo first output containing <N>B
  local s=""
  for _ in 1 2 3 4 5 6 7 8; do
    s=$($CLI shot-stat "$1" "$2" 2>/dev/null) || true
    echo "$s" | grep -q '<[0-9]*B>' && break
    sleep 1
  done
  echo "$s"
}

echo "== setup: adb forward tcp:$PORT -> localabstract:pilotdroid_agent =="
adb -s "$DEV" forward "tcp:$PORT" "localabstract:pilotdroid_agent" >/dev/null

# Forward-hygiene sanity: killed serve/bench sessions leak forward rules
# (agent conn is single-client — a pile of stale rules is a symptom, not
# a failure). >4 rules to our sockets = WARN + list; remove with
# `adb forward --remove tcp:<port>` (the canonical $PORT pair is kept).
FWD_RULES=$(adb forward --list | grep "$DEV.*localabstract:pilotdroid_agent" || true)
FWD_COUNT=$(printf '%s\n' "$FWD_RULES" | grep -c . || true)
if [ "$FWD_COUNT" -gt 4 ]; then
  echo "  WARN forward_hygiene: $FWD_COUNT rules to pilotdroid_agent on $DEV (>4)"
  printf '%s\n' "$FWD_RULES" | sed 's/^/    /'
fi

# Keyguard pre-step: a locked/asleep screen makes every downstream UI
# assertion flake. Wake + swipe-unlock is best-effort (no-op when already
# unlocked); MIUI pocket-mode users should keep the screen on manually.
adb -s "$DEV" shell 'input keyevent KEYCODE_WAKEUP' >/dev/null 2>&1 || true
adb -s "$DEV" shell 'wm size' 2>/dev/null | grep -o '[0-9]*x[0-9]*' | head -1 \
  | awk -Fx '{print "adb -s '"$DEV"' shell input swipe "$1/2" "$2*0.9" "$1/2" "$2*0.15" 600"}' \
  | sh >/dev/null 2>&1 || true
sleep 0.5

# Lockscreen cure: MIUI can re-lock mid-run (screen_off_timeout ~30-60s)
# → every downstream selector false-negatives at random points. Pin the
# screen on for the whole run, restore the user's value on exit (trap
# covers early failures too).
SAVED_TMO=$(adb -s "$DEV" shell 'settings get system screen_off_timeout' 2>/dev/null | tr -d ' \r')
adb -s "$DEV" shell 'settings put system screen_off_timeout 2147483647' >/dev/null 2>&1 || true
restore_tmo() {
  case "$SAVED_TMO" in ''|*[!0-9]*) ;; *)
    adb -s "$DEV" shell "settings put system screen_off_timeout $SAVED_TMO" >/dev/null 2>&1;; esac
}
trap restore_tmo EXIT

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
echo "$HELLO" | grep -q '"ver": *"0\.7\.5"\|"ver":"0\.7\.5"' \
  && ok "agent_ver" || bad "agent_ver" "$(echo "$HELLO" | head -c 200)"

echo "== A1b CAPS bitmap == INJECT=1 DUMP=2 SHOT=4 QUERY=8 BATCH=16 STREAM=32 TEXT=64 MPTR=128 WATCH=256 WATCH_ASYNC=512 APP_INFO=1024 USAGE=2048 =="
CAPS=$(echo "$HELLO" | python3 -c 'import json,sys; print(json.load(sys.stdin)["payload"]["caps"])' 2>/dev/null || echo "?")
[ "$CAPS" = "4095" ] && ok "caps_bits" || bad "caps_bits" "caps=$CAPS expected 4095 (|APP_INFO|USAGE)"

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
  LC=$(adb -s "$DEV" shell 'logcat -d -b all -t 5000' | grep -cE "njection from package|injectInputEvent|Input event injection" || true)
  [ "$LC" -ge 1 ] && break
done
# marker emission is OEM-dependent (MIUIInput on MIUI; stock AOSP emits
# nothing for IM injection) — resp.ok is the hard gate, marker is a note.
echo "$INJ" | grep -q '"ok": *true\|"ok":true' \
  && ok "inject_tap_observable" "logcat_marker_lines=$LC" || bad "inject_tap_observable" "resp=$INJ logcat_inject_lines=$LC"

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
shot_file /tmp/shot.jpg
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
KLC=$(adb -s "$DEV" shell 'logcat -d -b all -t 5000' | grep -cE "njection from package|injectInputEvent|Input event injection" || true)
echo "$KEY" | grep -q '"ok": *true\|"ok":true' && echo "$KEY" | grep -q '"events": *2\|"events":2' \
  && ok "key_back" || bad "key_back" "resp=$KEY logcat=$KLC"

echo "== M3-A2 INJECT points sequence (kind=Swipe, 11 events) =="
PTS=$($CLI swipe-pts 700 2000 700 1400 300 10 2>&1) || true
echo "$PTS" | grep -q '"ok": *true\|"ok":true' && echo "$PTS" | grep -q '"events": *11\|"events":11' \
  && ok "swipe_points" || bad "swipe_points" "resp=$PTS"

echo "== M3-A3 SHOT quality affects bytes (30 < 80) =="
# content can drift between the two SHOTs (animated/transient frames
# carry different entropy) — retry back-to-back pairs so a drifting
# frame can't fail an otherwise valid quality ordering.
SQ=0
for _i in 1 2 3; do
  S30=$(shot_stat 1.0 30)
  S80=$(shot_stat 1.0 80)
  B30=$(echo "$S30" | sed -n 's/.*<\([0-9]*\)B>.*/\1/p')
  B80=$(echo "$S80" | sed -n 's/.*<\([0-9]*\)B>.*/\1/p')
  [ -n "$B30" ] && [ -n "$B80" ] && [ "$B30" -lt "$B80" ] && SQ=$_i && break
done
[ "$SQ" -gt 0 ] && ok "shot_quality" || bad "shot_quality" "q30=${B30:-?}B q80=${B80:-?}B"

echo "== M3-A4 backend force: uia forced, bogus rejected =="
BU=$($CLI inject-tap 700 1600 uia 2>&1) || true
echo "$BU" | grep -q '"ok": *true\|"ok":true' && echo "$BU" | grep -q '"backend": *"uia"\|"backend":"uia"' \
  && ok "backend_uia" || bad "backend_uia" "resp=$BU"
BB=$($CLI inject-tap 700 1600 bogus 2>&1) || true
echo "$BB" | grep -q "ERR_BACKEND" \
  && ok "backend_bogus_err" || bad "backend_bogus_err" "resp=$BB"

echo "== M2: setup deterministic editable field =="
# Anchor: quicksearchbox's `query_text` EditText — auto-focused, same
# layout on MIUI and stock AOSP (the old android:id/input anchor is a
# MIUI-only Settings hint). --activity-clear-task pins a fresh activity
# so a stacked SubSettings can't skew the a11y tree.
# The M2-A8..A9 query family anchors on the a11y-tree visibility of
# `query_text`, which on MIUI can lag mCurrentFocus by seconds after the
# preceding inject/backend ops — window focus alone is not sufficient.
# Launch → focus-wait → probe the anchor itself; relaunch up to 3 rounds
# (~15s worst) before letting the family take its shot.
for _ in 1 2 3; do
  # Curtain first: mid-run keyguard/pocket-mode (focus+tree = pkg
  # `android` only — observed) makes am start land behind it. WAKE +
  # dismiss-keyguard + BACK (exits the MIUI pocket-mode curtain; harmless
  # on a normal screen) before relaunching.
  adb -s "$DEV" shell 'input keyevent KEYCODE_WAKEUP; wm dismiss-keyguard; input keyevent KEYCODE_BACK' >/dev/null 2>&1
  adb -s "$DEV" shell 'am start -n com.android.quicksearchbox/.SearchActivityTransparent --activity-clear-task' >/dev/null 2>&1
  for _ in 1 2 3 4 5; do
    adb -s "$DEV" shell 'dumpsys window | grep mCurrentFocus' 2>/dev/null \
      | grep -q SearchActivityTransparent && break
    sleep 1
  done
  $CLI query '{"id":"query_text"}' 2>/dev/null | grep -q '"found": *true' && break
  sleep 1
done
# Post-mortem: if the anchor still isn't queryable after 3 relaunches,
# record which window actually holds focus + which packages the a11y tree
# sees (MIUI overlay/pocket-mode diagnosis, cf. RESULTS-M13 §6c).
$CLI query '{"id":"query_text"}' 2>/dev/null | grep -q '"found": *true' || {
  echo "  [m2-setup] anchor invisible — focus=$(adb -s "$DEV" shell \
    'dumpsys window | grep mCurrentFocus' | tail -1 | tr -d ' ')"
  echo "  [m2-setup] tree pkgs=$($CLI dump 2>/dev/null \
    | grep -o 'package=\"[^\"]*\"' | sort -u | tr '\n' ' ')"
}

echo "== M2-A8 QUERY found + structured node =="
Q=$($CLI query '{"id":"query_text"}' 2>&1) || true
echo "$Q" | python3 -c 'import json,sys
p=json.load(sys.stdin)["payload"]
assert p.get("found") is True, p
n=p["node"]; assert "bounds" in n and len(n["bounds"])==4, n
print("OK node bounds",n["bounds"])' >/dev/null 2>&1 \
  && ok "query_found" || bad "query_found" "$Q"

echo "== M2-A8b QUERY attrs pruning: only requested fields returned =="
Q=$($CLI batch '[{"query":{"selector":{"id":"query_text"},"attrs":["text","bounds"]}}]' 2>&1) || true
echo "$Q" | python3 -c 'import json,sys
p=json.load(sys.stdin)["payload"]; n=p["results"][0]["data"]["node"]
assert set(n.keys())=={"text","bounds"}, f"node keys={sorted(n.keys())} expected [text,bounds]"
print("OK pruned",n)' >/dev/null 2>&1 \
  && ok "query_attrs" || bad "query_attrs" "$Q"

echo "== M5-A1 QUERY class selector (className mapping) =="
Q=$($CLI query '{"class":"android.widget.TextView"}' 2>&1) || true
echo "$Q" | python3 -c 'import json,sys
p=json.load(sys.stdin)["payload"]
assert p.get("found") is True, p
n=p["node"]; assert "TextView" in n.get("class",""), n
print("OK class node",n.get("class"))' >/dev/null 2>&1 \
  && ok "query_class" || bad "query_class" "$Q"

echo "== M5-A2 QUERY scrollable selector (shim scroll backing) =="
# Settings main = scrollable preference list on both MIUI and stock;
# clear-task guarantees we land on MainSettings, not a resumed SubSettings.
adb -s "$DEV" shell 'am start -n com.android.settings/.MainSettings --activity-clear-task' >/dev/null 2>&1
sleep 1.2
Q=$($CLI query '{"scrollable":true}' 2>&1) || true
echo "$Q" | python3 -c 'import json,sys
p=json.load(sys.stdin)["payload"]
assert p.get("found") is True, p
n=p["node"]; assert n.get("scrollable") is True, n
print("OK scrollable bounds",n["bounds"])' >/dev/null 2>&1 \
  && ok "query_scrollable" || bad "query_scrollable" "$Q"

echo "== M2-A9 wait_until hit (inside BATCH) =="
adb -s "$DEV" shell 'am start -n com.android.quicksearchbox/.SearchActivityTransparent' >/dev/null 2>&1
sleep 1.0
W=$($CLI wait '{"id":"query_text"}' 3000 2>&1) || true
echo "$W" | grep -q '"code": *"OK"' && ok "wait_until_hit" || bad "wait_until_hit" "$W"

echo "== M2-A10 wait_until TIMEOUT (impossible selector, 500ms) =="
T0=$(date +%s%N)
W=$($CLI wait '{"text":"__pd_no_such_node__"}' 500 2>&1) || true
echo "$W" | grep -q '"code": *"TIMEOUT"' && ok "wait_until_timeout" || bad "wait_until_timeout" "$W"

echo "== M2-A11 TEXT -> text becomes visible in tree =="
# quicksearchbox's query_text EditText auto-focuses — deterministic
# editable target for the TEXT op (Settings search tap was layout-fragile).
adb -s "$DEV" shell 'am start -n com.android.quicksearchbox/.SearchActivityTransparent --activity-clear-task' >/dev/null 2>&1
sleep 1.2
$CLI text "pdtest" >/dev/null 2>&1 || true
sleep 0.4
Q=$($CLI query '{"text":"pdtest"}' 2>&1) || true
echo "$Q" | grep -q '"found": *true' && ok "text_visible" || bad "text_visible" "$Q"

echo "== M6-A1 TEXT unicode (ACTION_SET_TEXT is UTF-16 — no IME needed) =="
$CLI text "试点pd" >/dev/null 2>&1 || true
sleep 0.4
Q=$($CLI query '{"text":"试点pd"}' 2>&1) || true
echo "$Q" | grep -q '"found": *true' && ok "text_unicode" || bad "text_unicode" "$Q"

echo "== M6-A2 QUERY pkg selector (owning package exact match) =="
# foreground is quicksearchbox — the typed node lives in its window tree
Q=$($CLI query '{"text":"试点pd","pkg":"com.android.quicksearchbox"}' 2>&1) || true
echo "$Q" | grep -q '"found": *true' && ok "query_pkg" || bad "query_pkg" "$Q"
Q=$($CLI query '{"text":"试点pd","pkg":"com.nonexistent.pkg"}' 2>&1) || true
echo "$Q" | grep -q '"found": *false' && ok "query_pkg_neg" || bad "query_pkg_neg" "$Q"
# foreground stays quicksearchbox — the batch query below anchors on it
sleep 0.5

echo "== M2-A12 BATCH per-op results =="
B=$($CLI batch '[{"inject":{"kind":"Tap","points":[{"action":0,"x":700,"y":1400,"t_ms":0},{"action":1,"x":700,"y":1400,"t_ms":75}],"wait_finish":false}},{"query":{"selector":{"id":"query_text"}}}]' 2>&1) || true
echo "$B" | python3 -c 'import json,sys
p=json.load(sys.stdin)["payload"]; rs=p["results"]
assert len(rs)==2, rs
assert all(r["code"]=="OK" for r in rs), rs
assert all("cost_ns" in r for r in rs), rs
assert p.get("total_ns",0)>0
print("OK")' >/dev/null 2>&1 && ok "batch_results" || bad "batch_results" "$B"

# The batch tap lands inside the suggestion list that "pdtest" populated —
# on this MIUI build it can open an app behind the security-app-lock gate
# (applicationlock.ConfirmAccessControl, FLAG_SECURE → SHOT returns an
# error payload). Back out of whatever the tap opened so shot_scale runs
# on an ordinary screen. Harmless when nothing opened (BACK on the
# search activity just exits it; later asserts don't need it foreground).
adb -s "$DEV" shell 'input keyevent KEYCODE_BACK' >/dev/null 2>&1
sleep 0.5

echo "== M2-A13 SHOT scale=0.5 bytes shrink + JPEG =="
shot_file /tmp/shot-half.jpg 0.5
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
  LC=$(adb -s "$DEV" shell 'logcat -d -b all -t 5000' | grep -cE "njection from package" || true)
  [ "$LC" -ge 1 ] && break
done
echo "$NA" | grep -q "OK: no response" \
  && ok "noack_tap" "logcat_marker_lines=$LC" || bad "noack_tap" "out=$NA logcat=$LC"

# restore a neutral screen for subsequent runs
adb -s "$DEV" shell 'input keyevent KEYCODE_HOME' >/dev/null 2>&1 || true

echo "== M4 STREAM (§4.8: one owner session held for start->frames->stop) =="
adb -s "$DEV" forward tcp:17933 localabstract:pilotdroid_agent_media >/dev/null 2>&1
STREAM_OUT=$(python3 - "$HERE" "$PORT" 17933 <<'PYEOF' 2>&1
import sys, time, json, socket, struct
sys.path.insert(0, sys.argv[1])
import agent_client as ac
PORT, MPORT = int(sys.argv[2]), int(sys.argv[3])

def rep(name, ok_, detail=""):
    print(f"{name}|{'PASS' if ok_ else 'FAIL'}|{str(detail)[:300]}")

a = ac.Agent(PORT).connect()

# A1 start ack: ok + media socket name
try:
    p = a.stream_start(5, 0.5, 70)["payload"]
    rep("stream_start", p.get("ok") is True
        and "pilotdroid_agent_media" in str(p.get("socket")), json.dumps(p))
except Exception as e:
    rep("stream_start", False, e)

# A2 frames on ch=1: >=4 frames, type=FRAME, monotonic seq, jpeg payload
ms = None
try:
    ms = ac.media_connect(MPORT); ms.settimeout(15)
    frames, t0 = [], time.time()
    while len(frames) < 5 and time.time() - t0 < 15:
        f = ac.read_frame(ms)
        if f is None:
            break
        frames.append(f)
    seqs = [f[3] for f in frames]
    jpg = all(len(f[4].get("data") or b"") > 100 and
              f[4].get("w") > 0 and f[4].get("h") > 0 for f in frames)
    rep("stream_frames",
        len(frames) >= 4 and all(f[0] == 1 and f[1] == 1 for f in frames)
        and seqs == sorted(seqs) and jpg,
        f"n={len(frames)} seqs={seqs} wall={round(time.time()-t0,1)}s")
except Exception as e:
    rep("stream_frames", False, e)

# A3 single-client: a second control conn REPLACES the first — op on `a`
# fails (socket closed by agent), and a's stream dies with its session (§4.8).
b = None
try:
    b = ac.Agent(PORT).connect()
    # Replacement is NOT atomic (§4.6 TOCTOU): an op in flight when the new
    # conn arrives may still land — the old conn just has to be dead
    # shortly after. Poll <=1s instead of asserting instant death.
    killed = False
    deadline = time.time() + 1.0
    while time.time() < deadline:
        try:
            a.ping()
        except Exception:
            killed = True
            break
        time.sleep(0.05)
    rep("client_replace", killed, "replaced conn dead within 1s")
except Exception as e:
    rep("client_replace", False, e)

# A4 implicit stop: replaced session's stream freed -> media conn EOF
try:
    if ms is None:
        raise Exception("no media conn")
    ms.settimeout(5)
    # TCP-forwarded media conns may have in-flight frames after stop —
    # drain until EOF (bounded), not "next frame must be EOF".
    extra = 0
    while ac.read_frame(ms) is not None:
        extra += 1
        if extra > 8:
            raise Exception(f"no EOF after {extra} in-flight frames")
    rep("stream_eof", True, f"eof after {extra} in-flight frames")
    ms.close()
except Exception as e:
    rep("stream_eof", False, e)

# A4b replacement conn owns a fresh stream: b start -> ok, stop -> counters
try:
    if b is None:
        raise Exception("no replacement conn")
    p = b.stream_start(5, 0.5, 70)["payload"]
    ok_start = p.get("ok") is True
    q = b.stream_stop()["payload"]
    rep("stream_stop", ok_start and q.get("ok") is True
        and q.get("frames_sent", 0) >= 0, f"start={json.dumps(p)} stop={json.dumps(q)}")
except Exception as e:
    rep("stream_stop", False, e)

# A5 fps throttle: fps=2 for ~4s -> effective fps in [1..5]
try:
    if b is None:
        raise Exception("no replacement conn")
    b.stream_start(2, 0.5, 70)
    ms2 = ac.media_connect(MPORT); ms2.settimeout(10)
    n, t0 = 0, time.time()
    while time.time() - t0 < 4:
        if ac.read_frame(ms2) is None:
            break
        n += 1
    wall = time.time() - t0
    b.stream_stop()
    ms2.close()
    fps = n / wall if wall else 0
    rep("stream_fps", 1 <= fps <= 5 and n >= 3,
        f"n={n} wall={round(wall,1)}s fps={round(fps,1)}")
except Exception as e:
    rep("stream_fps", False, e)

# A6 media reserve (agent >=0.7.2, t4 ghost-conn): a second media conn
# queues behind the owner instead of being busy-rejected, and self-heals
# into the reader the moment the owner conn dies — mediaDead promotes
# the reserve rather than halting the stream.
try:
    b.stream_start(5, 0.5, 70)
    m1 = ac.media_connect(MPORT); m1.settimeout(10)
    if ac.read_frame(m1) is None:
        raise Exception("owner conn got no frame")
    m2 = ac.media_connect(MPORT); m2.settimeout(1.5)
    queued = False
    try:
        ac.read_frame(m2)          # reserve gets no frames while waiting
    except Exception:
        queued = True              # timeout/read-block = conn held open
    rep("stream_reserve_hold", queued,
        "reserve conn must stay open (queued, not busy-closed)")
    m1.close()                     # owner dies abruptly
    m2.settimeout(8)
    f = ac.read_frame(m2)
    rep("stream_reserve_heal", f is not None and f[1] == 1,
        f"promoted reserve delivered frame={f is not None}")
    m2.close()
    b.stream_stop()
except Exception as e:
    rep("stream_reserve_hold", False, e)
    rep("stream_reserve_heal", False, e)
    try:
        b.stream_stop()
    except Exception:
        pass

# A7 ghost-conn classification (t4/t6): a dead-peer conn already queued
# in the accept backlog when the stream starts must not kill the stream —
# the agent promotes it, sees the first write fail (never wrote ok),
# drops it, and the real reader still gets frames. Before the fix the
# reader was busy-rejected and its first read saw EOF.
try:
    b.stream_start(5, 0.5, 70)
    # backlog ghost: connect then RST (SO_LINGER=0) so accept() hands the
    # agent an already-dead peer
    g = socket.create_connection(("127.0.0.1", MPORT), timeout=3)
    g.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER,
                 struct.pack("ii", 1, 0))
    g.close()
    time.sleep(0.15)               # let the ghost reach promote/write-fail
    mr = ac.media_connect(MPORT); mr.settimeout(10)
    f = ac.read_frame(mr)
    rep("stream_ghost_drop", f is not None and f[1] == 1,
        f"reader frame after ghost={f is not None}")
    mr.close()
    b.stream_stop()
except Exception as e:
    rep("stream_ghost_drop", False, e)
    try:
        b.stream_stop()
    except Exception:
        pass

if b is not None:
    b.close()
a.close()
PYEOF
)
while IFS='|' read -r _name _st _det; do
  [ -z "$_name" ] && continue
  [ "$_st" = "PASS" ] && ok "$_name" || bad "$_name" "$_det"
done <<< "$STREAM_OUT"

echo "== M11 ASYNC WATCH + APP_INFO + pinch smoke (ch=1 media fwd tcp:17933 above) =="
M11_OUT=$(python3 - "$HERE" "$PORT" 17933 "$DEV" <<'PYEOF' 2>&1
import subprocess, sys, time, json, socket, struct
sys.path.insert(0, sys.argv[1])
import agent_client as ac
PORT, MPORT, DEV = int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]

def rep(name, ok_, detail=""):
    print(f"{name}|{'PASS' if ok_ else 'FAIL'}|{str(detail)[:300]}")

def sh(*args):
    return subprocess.run(["adb", "-s", DEV, "shell", *args],
                          capture_output=True, text=True)

a = ac.Agent(PORT).connect()

# C1 conn-side APP_INFO: foreground pkg without a dumpsys spawn
try:
    # Apps with lockscreen-hijack features (e.g. com.kunenglish on
    # ba0d072d) can steal foreground between am-start and the read —
    # retry the launch+read up to 3x before calling it a real miss.
    p, ms = {}, 0
    for _ in range(3):
        sh("am", "start", "com.android.settings")
        time.sleep(1.2)
        t0 = time.time()
        p = a.app_info()["payload"]
        ms = (time.time() - t0) * 1000
        if p.get("pkg") == "com.android.settings":
            break
    rep("app_info_conn",
        p.get("pkg") == "com.android.settings" and ms < 150,
        f"pkg={p.get('pkg')} op_ms={ms:.0f}")
except Exception as e:
    rep("app_info_conn", False, e)

# C2 pinch smoke: two-pointer gesture dispatches ok (CAP_MULTI_PTR)
try:
    sh("input", "keyevent", "KEYCODE_HOME")
    time.sleep(0.8)
    h = a.hello()["payload"]
    w, hh = h["display"]
    cx, cy = w / 2, hh / 2
    pts = [{"action": 0, "pid": 0, "x": cx - 300, "y": cy, "t_ms": 0},
           {"action": 0, "pid": 1, "x": cx + 300, "y": cy, "t_ms": 10}]
    for i in range(1, 11):
        d = 300 - 250 * i / 11
        pts += [{"action": 2, "pid": 0, "x": cx - d, "y": cy,
                 "t_ms": 10 + 30 * i},
                {"action": 2, "pid": 1, "x": cx + d, "y": cy,
                 "t_ms": 10 + 30 * i}]
    pts += [{"action": 1, "pid": 1, "x": cx - 50, "y": cy, "t_ms": 340},
            {"action": 1, "pid": 0, "x": cx + 50, "y": cy, "t_ms": 350}]
    r = a.inject_multi(pts)["payload"]
    rep("pinch_smoke", r.get("ok") is True, json.dumps(r)[:200])
except Exception as e:
    rep("pinch_smoke", False, e)

# agent>=0.7.4: a freshly attached media conn replays the unexpired
# ui_event backlog first — frames must be filtered for the expected
# watch_id, not matched on the very first read.
def read_until(msock, pred, tries=8):
    for _ in range(tries):
        fr = ac.read_frame(msock)
        if fr is None:
            return None
        if fr[1] == 0x0002 and pred(fr[4]):
            return fr[4]
    return None

# C3 async WATCH round-trip: arm -> trigger -> WATCH_HIT on media socket
ms = None
try:
    sh("input", "keyevent", "KEYCODE_HOME")
    time.sleep(0.8)
    ms = ac.media_connect(MPORT)
    ms.settimeout(10)
    r = a.watch_arm({"pkg": "com.android.settings"}, "none",
                    poll_ms=50)["payload"]
    wid = r.get("watch_id")
    assert r.get("armed") and wid, r
    sh("am", "start", "com.android.settings")
    p = read_until(ms, lambda p: p.get("watch_id") == wid
                   and p.get("matched"))
    hit = p is not None
    rep("watch_arm_hit", hit,
        f"hit p={json.dumps(p)[:150] if p else 'none'}")
except Exception as e:
    rep("watch_arm_hit", False, e)

# C4 parallel-drive: while armed, N control ops all land (non-blocking)
try:
    sh("input", "keyevent", "KEYCODE_HOME")
    # HOME→launcher window transition ~1s on this device — every op during
    # the transition contends on the a11y snapshot (~2.2s vs ~0.2s after,
    # measured M11). Settle past it or par_ms measures the transition,
    # not watcher non-blocking.
    time.sleep(1.5)
    r = a.watch_arm({"pkg": "com.android.settings"}, "none",
                    poll_ms=50)["payload"]
    wid2 = r.get("watch_id")
    assert wid2, r
    t0 = time.time()
    for _ in range(10):
        a.ping()
    n_found = a.query({"pkg": "com.miui.home"})["payload"].get("found")
    par_ms = (time.time() - t0) * 1000
    # trigger -> hit still arrives after the parallel ops
    sh("am", "start", "com.android.settings")
    if ms is None:
        raise Exception("no media conn")
    ms.settimeout(10)
    hit2 = read_until(ms, lambda p: p.get("watch_id") == wid2
                      and p.get("matched")) is not None
    rep("watch_parallel", hit2 and par_ms < 2000,
        f"10ping+query={par_ms:.0f}ms home_found={n_found} hit={hit2}")
except Exception as e:
    rep("watch_parallel", False, e)

# A8 watch-conn ghost guard (agent >=0.7.3, t4): stray/dead-backlog
# media conns must not displace or starve a live watch conn — ui_event
# pushes go to EVERY held watch conn and corpses drop on write-fail.
# (a) two RST ghosts behind a live watch conn -> the hit still arrives;
# (b) a fresh conn behind a corpse+ghosts -> broadcast reaches it.
def rst_ghost():
    g = socket.create_connection(("127.0.0.1", MPORT), timeout=3)
    g.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER,
                 struct.pack("ii", 1, 0))
    g.close()

try:
    sh("input", "keyevent", "KEYCODE_HOME")
    time.sleep(1.5)
    wa = ac.media_connect(MPORT); wa.settimeout(10)
    r = a.watch_arm({"pkg": "com.android.settings"}, "none",
                    poll_ms=50)["payload"]
    wid8 = r.get("watch_id")
    assert wid8, r
    for _ in range(2):
        rst_ghost(); time.sleep(0.12)
    sh("am", "start", "com.android.settings")
    hit8 = read_until(wa, lambda p: p.get("watch_id") == wid8
                      and p.get("matched"))
    rep("watch_ghost_live", hit8 is not None,
        f"live conn survived stray ghosts: hit={hit8 is not None}")
    # (b) kill the live conn dirty (RST) -> corpse stays in the set;
    # a fresh conn + more ghosts -> the next hit must reach the fresh
    # conn (broadcast self-heal, no silent drop).
    wa.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER,
                  struct.pack("ii", 1, 0))
    wa.close()
    time.sleep(0.3)
    sh("input", "keyevent", "KEYCODE_HOME")
    time.sleep(1.0)
    wb = ac.media_connect(MPORT); wb.settimeout(10)
    for _ in range(2):
        rst_ghost(); time.sleep(0.12)
    r2 = a.watch_arm({"pkg": "com.android.settings"}, "none",
                     poll_ms=50)["payload"]
    wid9 = r2.get("watch_id")
    assert wid9, r2
    sh("am", "start", "com.android.settings")
    hit9 = read_until(wb, lambda p: p.get("watch_id") == wid9
                      and p.get("matched"))
    rep("watch_corpse_heal", hit9 is not None,
        f"fresh conn behind corpse+ghosts got hit: hit={hit9 is not None}")
    wb.close()
except Exception as e:
    rep("watch_ghost_live", False, e)
    rep("watch_corpse_heal", False, e)

# A9 watch_dead + stash replay (agent >=0.7.4): an armed watcher that
# dies abnormally — poll fault or session teardown — emits a dying
# {type:"watch_dead"} ui_event instead of vanishing silently; and the
# always-stash replay ring delivers an event to a conn that attaches
# after the broadcast window (ghost-absorbed write = fake delivery).
try:
    # (a) control-conn replace kills the session -> armed watcher aborts
    # -> watch_dead reaches the still-attached media conn. Selector is
    # deliberately unmatchable: a satisfiable one fires first and leaves
    # nothing armed to abort.
    wd = ac.media_connect(MPORT); wd.settimeout(10)
    time.sleep(0.3)
    r = a.watch_arm({"pkg": "__nonexistent__.pkg"},
                    "none", poll_ms=50)["payload"]
    widD = r.get("watch_id")
    assert widD, r
    a2 = ac.Agent(PORT).connect()     # replaces a's session -> abort
    dead = read_until(wd, lambda p: p.get("type") == "watch_dead"
                      and p.get("watch_id") == widD)
    rep("watch_dead_event", dead is not None,
        f"dead={json.dumps(dead, ensure_ascii=False)[:160] if dead else 'no frame'}")
    wd.close(); a2.close()
except Exception as e:
    rep("watch_dead_event", False, e)

try:
    # (b) ghosts absorb the hit broadcast; a conn attaching AFTER the
    # push still gets the event via the stash replay ring (r5 fix).
    a3 = ac.Agent(PORT).connect()
    sh("input", "keyevent", "KEYCODE_HOME")
    time.sleep(1.5)
    for _ in range(3):
        rst_ghost(); time.sleep(0.15)
    r = a3.watch_arm({"pkg": "com.android.settings"}, "none",
                     poll_ms=50)["payload"]
    widR = r.get("watch_id")
    assert widR, r
    sh("am", "start", "com.android.settings")
    fg = False
    for _ in range(16):
        if a3.query({"pkg": "com.android.settings"})["payload"].get("found"):
            fg = True
            break
        time.sleep(0.5)
    time.sleep(0.4)   # broadcast already absorbed by the ghost set
    wr = ac.media_connect(MPORT); wr.settimeout(10)
    hitR = read_until(wr, lambda p: p.get("watch_id") == widR
                      and p.get("matched"))
    rep("watch_hit_stash_replay", hitR is not None,
        f"fg={fg} replay_hit={hitR is not None}")
    wr.close(); a3.close()
except Exception as e:
    rep("watch_hit_stash_replay", False, e)

if ms is not None:
    ms.close()
a.close()
PYEOF
)
while IFS='|' read -r _name _st _det; do
  [ -z "$_name" ] && continue
  [ "$_st" = "PASS" ] && ok "$_name" || bad "$_name" "$_det"
done <<< "$M11_OUT"

echo "== M33 USAGE_STATS: shell-identity UsageStatsManager bridge =="
U_OUT=$(python3 - "$HERE" "$PORT" <<'PYEOF' 2>&1
import sys, time
sys.path.insert(0, sys.argv[1])
import agent_client as ac
PORT = int(sys.argv[2])

def rep(name, ok_, detail=""):
    print(f"{name}|{'PASS' if ok_ else 'FAIL'}|{str(detail)[:300]}")

a = ac.Agent(PORT).connect()

try:
    p = a.usage_access()["payload"]
    rep("usage_access_probe",
        p.get("available") is True and p.get("source") == "api",
        f"available={p.get('available')} source={p.get('source')} detail={str(p.get('detail'))[:80]}")
except Exception as e:
    rep("usage_access_probe", False, e)

try:
    end = int(time.time() * 1000)
    begin = end - 3 * 24 * 3600 * 1000
    r = a.usage_stats(begin, end)["payload"]
    entries = r.get("entries") or []
    launcher = [e for e in entries if "launcher" in str(e.get("package", ""))]
    e0 = entries[0] if entries else {}
    keys_ok = all(k in e0 for k in ("package", "bucket_first_ms", "bucket_last_ms",
                                     "foreground_ms", "visible_ms", "launch_count",
                                     "last_time_used"))
    rep("usage_stats_query",
        r.get("source") == "api" and len(entries) > 50 and keys_ok
            and isinstance(r.get("partial"), bool) and "earliest_data_ms" in r,
        f"entries={len(entries)} keys_ok={keys_ok} partial={r.get('partial')} earliest={r.get('earliest_data_ms')}")
    nonzero = [e for e in entries if (e.get("foreground_ms") or 0) > 0]
    rep("usage_stats_nonzero",
        len(nonzero) > 0 and len(launcher) > 0,
        f"nonzero_fg={len(nonzero)} launcher_entries={len(launcher)}")
except Exception as e:
    rep("usage_stats_query", False, e)
    rep("usage_stats_nonzero", False, e)

try:
    end = int(time.time() * 1000)
    begin = end - 3 * 24 * 3600 * 1000
    rf = a.usage_stats(begin, end, packages=["com.android.settings"])["payload"]
    entf = rf.get("entries") or []
    pkgs = set(str(e.get("package")) for e in entf)
    rep("usage_stats_filter",
        pkgs <= {"com.android.settings"},
        f"filtered_entries={len(entf)} pkgs={list(pkgs)[:3]}")
except Exception as e:
    rep("usage_stats_filter", False, e)

try:
    end = int(time.time() * 1000)
    r = a.usage_stats(end, end - 1000)
    # error frames surface as payload.code on the ERR flag, not exceptions
    code = (r.get("payload") or {}).get("code", "")
    rep("usage_stats_badargs", code == "ERR_ARGS", f"code={code!r}")
except Exception as e:
    rep("usage_stats_badargs", "ERR_ARGS" in str(e), str(e)[:120])
PYEOF
)
echo "$U_OUT" | sed 's/^/  /'
while IFS='|' read -r _name _st _det; do
  [ -z "$_name" ] && continue
  [ "$_st" = "PASS" ] && ok "$_name" || bad "$_name" "$_det"
done <<< "$U_OUT"

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
