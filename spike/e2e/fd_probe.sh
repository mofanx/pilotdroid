#!/usr/bin/env bash
# M6 fd probe: does repeated SHOT leak device-side fds (dmabuf/gralloc)?
# fd0 -> 20x SHOT -> fd1 -> 40x SHOT -> fd2 -> SIGUSR1 (ART GC) -> fd3
# verdict: monotonic growth = leak; plateau after first batch = one-time
# lazy init of the graphics stack (ion/binder/tracing/eventpoll), not leak.
# Usage: fd_probe.sh <serial> [port]
set -euo pipefail
DEV="${1:?serial}"; PORT="${2:-17932}"
HERE="$(cd "$(dirname "$0")" && pwd)"
CLI="python3 $HERE/../../agent/scripts/agent_client.py --port $PORT"

agent_pid() { adb -s "$DEV" shell "pgrep -f '[p]ilotdroid.Agent' | head -1" | tr -d '\r'; }
fd_count()  { adb -s "$DEV" shell "ls /proc/$(agent_pid)/fd | wc -l" | tr -d '\r'; }
fd_types()  { adb -s "$DEV" shell "ls -l /proc/$(agent_pid)/fd | sed 's/.* -> //' | sort | uniq -c | sort -rn | head -8"; }
batch()     { local n=$1; for i in $(seq 1 "$n"); do
                $CLI shot "/tmp/fdprobe_$$_$i.jpg" 0.5 >/dev/null 2>&1 || echo "shot$i failed";
              done; }

PID=$(agent_pid); [ -n "$PID" ] || { echo "no agent pid"; exit 1; }
echo "== agent pid=$PID =="
FD0=$(fd_count); echo "fd0=$FD0 (baseline)"; fd_types

echo "== batch1: 20x SHOT scale=0.5 =="; batch 20; sleep 1
FD1=$(fd_count); echo "fd1=$FD1"

echo "== batch2: 40x SHOT scale=0.5 =="; batch 40; sleep 1
FD2=$(fd_count); echo "fd2=$FD2"; fd_types

echo "== SIGUSR1 (ART GC) =="
adb -s "$DEV" shell "kill -USR1 $PID" 2>&1 || true
sleep 3
PID2=$(agent_pid)
[ "$PID2" != "$PID" ] && echo "NOTE: agent pid changed ($PID -> $PID2)"
FD3=$(fd_count); echo "fd3=$FD3 (post-GC)"

D1=$((FD1 - FD0)); D2=$((FD2 - FD1))
echo "== verdict =="
echo "delta_batch1(20shot)=$D1 delta_batch2(40shot)=$D2 post_gc_delta=$((FD3 - FD0))"
if [ "$D2" -le 2 ]; then
  echo "PASS no-leak: fd plateaus after first batch (+$D1 one-time graphics init),"
  echo "     40 further shots add $D2 fds — bounded, not per-shot accumulation"
elif [ "$D2" -ge 15 ]; then
  echo "FAIL leak: +$D2 fds on second batch — monotonic per-shot growth"
else
  echo "PARTIAL: second-batch delta $D2 — investigate /proc/$PID/fdinfo dmabuf"
fi
