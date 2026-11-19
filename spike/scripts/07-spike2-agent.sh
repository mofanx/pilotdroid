#!/usr/bin/env bash
# Spike-2 core: build SpikeAgent dex harness + run all-in-one capability probe
# Usage: ./07-spike2-agent.sh [serial] [cmd]
#   cmd: all | connect | dump | inject | swipe | shot | sock | heartbeat
set -euo pipefail
DEV="${1:-${DEV:-}}"
CMD="${2:-all}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")
HERE="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
AJAR="$SDK/platforms/android-34/android.jar"
D8="$(ls -d "$SDK"/build-tools/*/d8 | sort -V | tail -1)"

mkdir -p "$HERE/build"
rm -f "$HERE"/build/SpikeAgent*.class
javac --release 8 -classpath "$AJAR" -d "$HERE/build" "$HERE/scripts/java/SpikeAgent.java"
"$D8" --min-api 34 --output "$HERE/spike2.jar" "$HERE"/build/SpikeAgent*.class
adb "${SEL[@]}" push "$HERE/spike2.jar" /data/local/tmp/spike2.jar >/dev/null
echo "== SpikeAgent $CMD on ${DEV:-default} =="
adb "${SEL[@]}" shell "CLASSPATH=/data/local/tmp/spike2.jar app_process /data/local/tmp SpikeAgent $CMD"
echo "(process stays alive after tests — UiAutomation holds threads; that's the daemon premise)"
adb "${SEL[@]}" shell 'pkill -f SpikeAgent; true' 2>/dev/null || true
