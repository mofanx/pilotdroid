#!/usr/bin/env bash
# Spike-2 MIUI install interception probe.
# Attempts adb install; polls for AdbInstallActivity; optionally inject-confirms it.
# Usage: ./09-install-probe.sh [serial] [apk] [--inject-confirm]
set -uo pipefail
DEV="${1:-${DEV:-}}"
APK="${2:-$(dirname "$0")/../apk/spiketest2.apk}"
CONFIRM="${3:-}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")

echo "== adb install $APK =="
adb "${SEL[@]}" install "$APK" > /tmp/install-probe.log 2>&1 &
IPID=$!
for i in $(seq 1 30); do
  if adb "${SEL[@]}" shell 'dumpsys window | grep mCurrentFocus | tail -1' \
      | grep -q AdbInstallActivity; then
    echo "MIUI AdbInstallActivity detected at poll $i"
    adb "${SEL[@]}" shell 'screencap -p /sdcard/dlg.png' 2>/dev/null
    adb "${SEL[@]}" pull /sdcard/dlg.png "$(dirname "$0")/../data/install-dialog.png" >/dev/null 2>&1
    if [ "$CONFIRM" = "--inject-confirm" ]; then
      adb "${SEL[@]}" shell 'CLASSPATH=/data/local/tmp/spike.jar app_process /data/local/tmp SpikeInject 480 2970' | grep inject_ok
    fi
    break
  fi
  sleep 0.4
done
wait $IPID 2>/dev/null
tail -5 /tmp/install-probe.log
adb "${SEL[@]}" shell 'pm list packages | grep pilotdroid'
