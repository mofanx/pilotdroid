#!/usr/bin/env bash
# Spike 04: screenrecord file + exec-out stream framerate
# Usage: ./04-screenrecord.sh [serial] [out-dir] [seconds]
set -uo pipefail
DEV="${1:-${DEV:-}}"
OUT="${2:-$(dirname "$0")/../data}"
SECS="${3:-10}"
SEL=(); [ -n "$DEV" ] && SEL=(-s "$DEV")
mkdir -p "$OUT"

echo "=== screenrecord ${SECS}s -> mp4 ==="
adb "${SEL[@]}" shell "screenrecord --time-limit $SECS --verbose /sdcard/rec.mp4" 2>&1
adb "${SEL[@]}" pull /sdcard/rec.mp4 "$OUT/rec.mp4"
ffprobe -v error -count_frames -select_streams v:0 \
  -show_entries stream=nb_read_frames,r_frame_rate,avg_frame_rate,duration \
  -of default=nw=1 "$OUT/rec.mp4"

echo "=== exec-out raw h264 stream ${SECS}s ==="
/usr/bin/time -f "wall=%e" timeout $((SECS + 10)) \
  adb "${SEL[@]}" exec-out "screenrecord --output-format=h264 --time-limit $SECS -" \
  > "$OUT/stream.h264" 2> "$OUT/sr.log"
ls -l "$OUT/stream.h264"; head -5 "$OUT/sr.log"
ffprobe -v error -count_packets -select_streams v:0 \
  -show_entries stream=nb_read_packets,duration -of default=nw=1 "$OUT/stream.h264"
