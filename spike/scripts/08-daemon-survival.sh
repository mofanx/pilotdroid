#!/usr/bin/env bash
# Spike-2 daemon survival: does a bg app_process survive host kill-server / adbd restart?
# Usage: ./08-daemon-survival.sh [serial]
set -uo pipefail
DEV="${1:-${DEV:-}}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")
HB_A=/data/local/tmp/hb-a.log; HB_B=/data/local/tmp/hb-b.log
RUN="CLASSPATH=/data/local/tmp/spike2.jar app_process /data/local/tmp SpikeAgent heartbeat"

adb "${SEL[@]}" shell "rm -f $HB_A $HB_B"
adb "${SEL[@]}" shell "nohup setsid sh -c \"$RUN $HB_A\" </dev/null >/dev/null 2>&1 &"
adb "${SEL[@]}" shell "sh -c \"$RUN $HB_B\" </dev/null >/dev/null 2>&1 &"
sleep 4
echo "--- before kill-server ---"
adb "${SEL[@]}" shell "wc -l $HB_A $HB_B; ps -A | grep app_process | grep -v grep | wc -l"
adb kill-server; sleep 8
echo "--- after host kill-server ---"
adb "${SEL[@]}" shell "wc -l $HB_A $HB_B; ps -A | grep app_process | grep -v grep | wc -l"
echo "--- adbd restart (adb usb) ---"
adb "${SEL[@]}" usb 2>&1 || true
sleep 6; adb "${SEL[@]}" wait-for-device
sleep 4
adb "${SEL[@]}" shell "wc -l $HB_A $HB_B 2>/dev/null; ps -A | grep app_process | grep -v grep | wc -l"
adb "${SEL[@]}" shell "rm -f $HB_A $HB_B; pkill -f SpikeAgent; true" 2>/dev/null || true
