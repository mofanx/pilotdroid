#!/usr/bin/env bash
# M4 night soak — unattended daemon-resilience long run.
# Per spec: session bench n>=1000 ops + periodic kill/watchdog-recovery loop
# (~5min) + agent RSS/fd sampling. Output: soak-m4.jsonl + soak-report.md data.
#
# Usage: soak-m4.sh <serial> <minutes>   e.g. soak-m4.sh ba0d072d 480
set -uo pipefail
DEV="${1:-ba0d072d}"
MINUTES="${2:-60}"
KILL_EVERY_S=300            # kill agent every ~5min
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
RCLI="$ROOT/target/debug/pilotdroid-cli"
OUT="$HERE/soak-m4.jsonl"
: > "$OUT"
adb -s "$DEV" forward tcp:17932 localabstract:pilotdroid_agent >/dev/null 2>&1

pid_of() { adb -s "$DEV" shell "pgrep -f app_process" | tr -d '\r' | head -1; }
agent_mem_fd() {           # echoes "rss_kb fd_count" or "- -"
  local p; p=$(pid_of)
  [ -z "$p" ] && { echo "- -"; return; }
  local rss fd
  rss=$(adb -s "$DEV" shell "awk '/VmRSS/{print \$2}' /proc/$p/status" | tr -d '\r')
  fd=$(adb -s "$DEV" shell "ls /proc/$p/fd 2>/dev/null | wc -l" | tr -d '\r')
  echo "${rss:-?} ${fd:-?}"
}

END=$((SECONDS + MINUTES * 60))
LAST_KILL=$SECONDS
CYCLE=0; TOTAL_OPS=0; KILLS=0; RECOVERIES=0; FAILS=0

log() { echo "{\"t\":$(date +%s),$1}" | tee -a "$OUT"; }

while [ $SECONDS -lt $END ]; do
  CYCLE=$((CYCLE+1))
  # --- bench block: one persistent session, mixed ops (n=50 per cycle) ---
  RES=$(python3 - "$HERE" <<'PY' 2>&1
import sys, time, json, statistics
sys.path.insert(0, sys.argv[1] + "/../../agent/scripts")
import agent_client as ac
try:
    a = ac.Agent(17932).connect()
    a.hello()
    lat = []
    for i in range(50):
        t = time.perf_counter()
        op = i % 4
        if op == 0: a.ping()
        elif op == 1: a.inject_tap(700, 1500)
        elif op == 2: a.query({"text": "Settings"})   # harmless miss
        else: a.dump()
        lat.append((time.perf_counter() - t) * 1000)
    a.close()
    print(json.dumps({"ops": 50, "p50": round(statistics.median(lat),1),
                      "p95": round(sorted(lat)[int(len(lat)*.95)],1)}))
except Exception as e:
    print(json.dumps({"ops": 0, "err": str(e)[:200]}))
PY
)
  OPS=$(echo "$RES" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("ops",0))' 2>/dev/null || echo 0)
  TOTAL_OPS=$((TOTAL_OPS + OPS))
  read -r RSS FD <<< "$(agent_mem_fd)"
  log "\"ev\":\"bench\",\"cycle\":$CYCLE,\"res\":$RES,\"rss_kb\":\"$RSS\",\"fd\":\"$FD\""
  [ "$OPS" -eq 0 ] && FAILS=$((FAILS+1))

  # --- periodic kill -> watchdog recovery check ---
  if [ $((SECONDS - LAST_KILL)) -ge $KILL_EVERY_S ]; then
    KILLS=$((KILLS+1))
    P=$(pid_of)
    [ -n "$P" ] && adb -s "$DEV" shell "kill -9 $P" >/dev/null 2>&1
    sleep 2
    T0=$(date +%s%N)
    "$RCLI" -s "$DEV" agent bench --op ping 1 >/dev/null 2>&1  # watchdog: zero-copy restart
    RC=$?
    REC_MS=$(( ($(date +%s%N) - T0) / 1000000 ))
    if [ $RC -eq 0 ]; then
      RECOVERIES=$((RECOVERIES+1))
      read -r RSS2 FD2 <<< "$(agent_mem_fd)"
      log "\"ev\":\"recover\",\"kill_n\":$KILLS,\"ms\":$REC_MS,\"rss_kb\":\"$RSS2\",\"fd\":\"$FD2\""
    else
      log "\"ev\":\"recover_fail\",\"kill_n\":$KILLS,\"rc\":$RC"
    fi
    LAST_KILL=$SECONDS
  fi
  sleep 1
done

log "\"ev\":\"done\",\"cycles\":$CYCLE,\"total_ops\":$TOTAL_OPS,\"kills\":$KILLS,\"recoveries\":$RECOVERIES,\"bench_fails\":$FAILS"
echo "SOAK DONE ops=$TOTAL_OPS kills=$KILLS recov=$RECOVERIES bench_fails=$FAILS -> $OUT"
