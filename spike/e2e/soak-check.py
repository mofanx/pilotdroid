#!/usr/bin/env python3
"""soak self-grading gate — turn 'no leak' from narrative into a check.

  python3 soak-check.py soak-m4.jsonl

Asserts (exit 0 = PASS, prints verdict lines):
  - fd count constant across all bench samples
  - fd absolute ceiling <= 90 (SHOT/GC-deferred dmabuf spikes are
    bounded — a constant-only gate can't see a raised baseline)
  - watch residue: every bench sample reports wres == 0 live pd-watch
    threads (M13 watcher arm — absent field on old logs is skipped)
  - recoveries == kills, every recovery < 5s
  - bench_fails == 0
  - RSS linear-fit slope <= +50 kB/min (no monotonic leak)
"""
import json, sys

def main(path):
    ev = [json.loads(l) for l in open(path) if l.strip().startswith("{")]
    bench = [e for e in ev if e.get("ev") == "bench"]
    rec = [e for e in ev if e.get("ev") == "recover"]
    recf = [e for e in ev if e.get("ev") == "recover_fail"]
    done = next((e for e in ev if e.get("ev") == "done"), {})

    fd = {e.get("fd") for e in bench}
    rss = [(e["t"], int(e["rss_kb"])) for e in bench
           if str(e.get("rss_kb", "")).isdigit()]
    fails = sum(1 for e in bench if e["res"].get("ops", 0) == 0)
    kills = done.get("kills", len(rec) + len(recf))

    ok = True
    def rep(name, passed, detail):
        nonlocal ok
        ok &= passed
        print(f"{'PASS' if passed else 'FAIL'} {name}: {detail}")

    rep("fd_constant", len(fd) == 1, f"fd={sorted(fd)}")
    fds = [int(e["fd"]) for e in bench if str(e.get("fd", "")).isdigit()]
    rep("fd_abs", not fds or max(fds) <= 90,
        f"max={max(fds) if fds else '-'} (cap 90)")
    wres = [e["wres"] for e in bench if str(e.get("wres", "")).isdigit()]
    rep("watch_residue", not wres or max(int(w) for w in wres) == 0,
        f"samples={len(wres)} max={max((int(w) for w in wres), default='-')}"
        " (gate 0 live pd-watch)")
    rep("recoveries==kills", len(rec) == kills and not recf,
        f"{len(rec)}/{kills}, fails={len(recf)}")
    if rec:
        rep("recovery_time", max(e["ms"] for e in rec) < 5000,
            f"max={max(e['ms'] for e in rec)}ms")
    rep("bench_fails", fails == 0, f"{fails}")
    if len(rss) > 10:
        n = len(rss); t0 = rss[0][0]
        xs = [(t - t0) / 60 for t, _ in rss]; ys = [r for _, r in rss]
        mx, my = sum(xs) / n, sum(ys) / n
        slope = sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / \
            sum((x - mx) ** 2 for x in xs)
        rep("rss_slope", slope <= 50, f"{slope:+.1f} kB/min over {xs[-1]:.0f}min")
    print("SOAK", "PASS" if ok else "FAIL")
    sys.exit(0 if ok else 1)

if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else "soak-m4.jsonl")
