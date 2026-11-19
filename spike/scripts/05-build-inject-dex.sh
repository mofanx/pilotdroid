#!/usr/bin/env bash
# Spike 05: build SpikeInject.java -> dex -> spike.jar, push to device
# Usage: ./05-build-inject-dex.sh [serial]
set -euo pipefail
DEV="${1:-${DEV:-}}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")
HERE="$(cd "$(dirname "$0")/.." && pwd)"
SDK="${ANDROID_HOME:-$HOME/Android/Sdk}"
AJAR="$SDK/platforms/android-34/android.jar"
D8="$(ls -d "$SDK"/build-tools/*/d8 | sort -V | tail -1)"

mkdir -p "$HERE/build"
javac --release 8 -classpath "$AJAR" -d "$HERE/build" "$HERE/scripts/java/SpikeInject.java"
"$D8" --min-api 34 --output "$HERE/spike.jar" "$HERE/build/SpikeInject.class"
unzip -l "$HERE/spike.jar"
adb "${SEL[@]}" push "$HERE/spike.jar" /data/local/tmp/spike.jar
echo "pushed. run: adb ${SEL[*]+-s $DEV} shell 'CLASSPATH=/data/local/tmp/spike.jar app_process /data/local/tmp SpikeInject 700 1600'"
