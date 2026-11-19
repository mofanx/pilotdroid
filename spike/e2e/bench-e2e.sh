#!/usr/bin/env bash
# M1 e2e bench: run each opcode >=30x through the REAL Rust CLI chain
# (adb forward -> localabstract -> PD01 frames -> agent), emit JSON.
# Usage: ./bench-e2e.sh [serial] [n]
set -uo pipefail
DEV="${1:-ba0d072d}"
N="${2:-30}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
CLI="$ROOT/target/debug/pilotdroid-cli"
OUT="$HERE/e2e-m1.json"
TMP=$(mktemp -d)

[ -x "$CLI" ] || { echo "build first: cargo build --workspace"; exit 1; }

tap()   { "$CLI" -s "$DEV" agent tap 700 1600 --nowait 2>/dev/null; }
shot()  { "$CLI" -s "$DEV" agent shot -o "$TMP/s.jpg" 2>/dev/null; }

# dump prints XML to stdout (no JSON line); time it externally and validate.
dump_ms=""
for i in $(seq "$N"); do
  s=$(date +%s%N)
  "$CLI" -s "$DEV" agent dump > "$TMP/d.xml" 2>/dev/null
  ms=$(( ($(date +%s%N) - s) / 1000000 ))
  grep -q "<node" "$TMP/d.xml" && dump_ms="$dump_ms $ms"
done
NODES=$(grep -o "<node" "$TMP/d.xml" | wc -l)

tap_json="["; sep=""
for i in $(seq "$N"); do
  r=$(tap); tap_json="$tap_json$sep$r"; sep=","
done; tap_json="$tap_json]"

shot_rtt=""; shot_bytes=0
for i in $(seq "$N"); do
  r=$(shot)
  v=$(echo "$r" | python3 -c "import json,sys; d=json.load(sys.stdin); print(d['rtt_ms'],d['bytes'])" 2>/dev/null)
  [ -n "$v" ] && { shot_rtt="$shot_rtt ${v% *}"; shot_bytes=${v#* }; }
done
MAGIC=$(xxd -p -l2 "$TMP/s.jpg" 2>/dev/null)

TAP_JSON="$tap_json" DUMP_MS="$dump_ms" SHOT_RTT="$shot_rtt" \
  python3 - "$OUT" "$DEV" "$NODES" "$MAGIC" "$shot_bytes" "$HERE/baseline-u2.json" <<'PY'
import json, os, sys
def pct(ts):
    ts=sorted(float(t) for t in ts if t); n=len(ts)
    return {"n":n,"mean_ms":round(sum(ts)/n,2),"p50_ms":round(ts[n//2],2),
            "p95_ms":round(ts[int(n*0.95)],2),"min_ms":round(ts[0],2),"max_ms":round(ts[-1],2)}
tap=json.loads(os.environ["TAP_JSON"])
out={
 "device":sys.argv[2],
 "agent":"pilotdroid 0.1.0 (feature/m1-e2e merged)",
 "click":{"rtt":pct([t["rtt_ms"] for t in tap]),
          "device_ms":pct([t["device_ms"] for t in tap]),
          "ok_events":all(t["ok"] and t["events"]==2 for t in tap)},
 "dump":{"rtt":pct(os.environ["DUMP_MS"].split()),
         "nodes":int(sys.argv[3]),"valid_xml":True},
 "shot":{"rtt":pct(os.environ["SHOT_RTT"].split()),
         "jpeg_magic":sys.argv[4]=="ffd8","bytes":int(sys.argv[5])},
}
u2=json.load(open(sys.argv[6])) if os.path.exists(sys.argv[6]) else {}
out["vs_u2"]={
 "click":  {"u2_p50":u2.get("click_xy",{}).get("p50_ms"),
            "ours_p50":out["click"]["rtt"]["p50_ms"],
            "speedup":round(u2.get("click_xy",{}).get("p50_ms",0)/out["click"]["rtt"]["p50_ms"],1)},
 "dump":   {"u2_p50":u2.get("dump_hierarchy",{}).get("p50_ms"),
            "ours_p50":out["dump"]["rtt"]["p50_ms"],
            "speedup":round(u2.get("dump_hierarchy",{}).get("p50_ms",0)/out["dump"]["rtt"]["p50_ms"],1)},
 "shot":   {"u2_p50":u2.get("screenshot",{}).get("p50_ms"),
            "ours_p50":out["shot"]["rtt"]["p50_ms"],
            "speedup":round(u2.get("screenshot",{}).get("p50_ms",0)/out["shot"]["rtt"]["p50_ms"],1)},
}
json.dump(out,open(sys.argv[1],"w"),indent=2)
print(json.dumps(out["vs_u2"],indent=2))
PY
rm -rf "$TMP"
