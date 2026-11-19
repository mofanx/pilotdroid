#!/usr/bin/env bash
# Build + push + launch the pilotdroid dex agent.
# Usage: ./deploy-agent.sh [serial] [--kill-only]
set -euo pipefail
DEV="${1:-${DEV:-ba0d072d}}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
AJAR="$SDK/platforms/android-34/android.jar"
D8="$(ls -d "$SDK"/build-tools/*/d8 | sort -V | tail -1)"
REMOTE=/data/local/tmp/pilotdroid

if [ "${2:-}" = "--kill-only" ]; then
  adb -s "$DEV" shell 'pkill -f "pilotdroid.Agent"; true' || true
  echo "agent killed"; exit 0
fi

mkdir -p "$HERE/build"
rm -f "$HERE"/build/pilotdroid/*.class 2>/dev/null || true
javac --release 8 -classpath "$AJAR" -d "$HERE/build" "$HERE/java/pilotdroid/Agent.java"
"$D8" --min-api 30 --output "$HERE/agent.jar" "$HERE"/build/pilotdroid/Agent*.class
adb -s "$DEV" shell "mkdir -p $REMOTE"
adb -s "$DEV" push "$HERE/agent.jar" "$REMOTE/agent.jar" >/dev/null
adb -s "$DEV" shell 'pkill -f "pilotdroid.Agent"; true' || true
adb -s "$DEV" shell "nohup setsid sh -c 'CLASSPATH=$REMOTE/agent.jar app_process $REMOTE pilotdroid.Agent' </dev/null >$REMOTE/agent.log 2>&1 &"
sleep 2
adb -s "$DEV" shell "cat $REMOTE/agent.log; ps -A | grep app_process | grep -v grep | head -3"
