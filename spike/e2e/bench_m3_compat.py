#!/usr/bin/env python3
"""M3 A/B: same u2-style scenario via real uiautomator2 vs pilotdroid shim.

  python3 bench_m3_compat.py --engine shim|u2 --n 20

Scenario (identical API surface on both sides):
  app_start(Settings) -> find search field -> click -> set_text("wifi")
  -> exists(text=wifi) -> screenshot(bytes) -> press("back") -> swipe

Same-口径 note: shim reuses ONE session socket for all ops;
u2 pays a JSON-RPC HTTP call per op. Both measured wall-clock host-side.
Writes e2e-m3-<engine>.json in this dir.
"""
import argparse, json, os, statistics, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser()
ap.add_argument("--engine", choices=["shim", "u2"], required=True)
ap.add_argument("--n", type=int, default=20)
ap.add_argument("--serial", default="ba0d072d")
args = ap.parse_args()

if args.engine == "shim":
    sys.path.insert(0, os.path.join(HERE, "..", "..", "compat"))
    import u2shim as u2  # noqa: E402
    d = u2.connect(port=17932, serial=args.serial)
else:
    import uiautomator2 as u2  # noqa: E402
    d = u2.connect(args.serial)
    d.implicitly_wait(0)

QSEL = "com.android.quicksearchbox:id/query_text"

def set_text(d, s):
    if args.engine == "u2":
        return d(className="android.widget.EditText").set_text(s)
    return d.set_text(s)          # TEXT op on the focused editable node

def scenario(d):
    steps = {}
    def step(name, fn):
        t0 = time.perf_counter()
        r = fn()
        steps[name] = round((time.perf_counter() - t0) * 1000, 2)
        return r
    step("app_start_settings", lambda: d.app_start("com.android.settings",
                                                   ".MainSettings"))
    time.sleep(0.6)
    step("swipe", lambda: d.swipe(700, 2200, 700, 1200, 0.3))
    step("app_start_search", lambda: d.app_start("com.android.quicksearchbox",
                                                 ".SearchActivityTransparent"))
    time.sleep(0.8)
    # deterministic editable: quicksearchbox query_text auto-focuses
    n = d(resourceId=QSEL)
    n.wait(3)
    step("sel_click", lambda: n.click())
    step("set_text", lambda: set_text(d, "wifi"))
    step("exists", lambda: d(text="wifi").exists)
    step("screenshot", lambda: d.screenshot(
        os.devnull if args.engine == "shim" else "/tmp/u2s.jpg"))
    step("press_back", lambda: d.press("back"))
    return steps

per_op, totals = {}, []
for i in range(args.n):
    t0 = time.perf_counter()
    s = scenario(d)
    totals.append((time.perf_counter() - t0) * 1000)
    for k, v in s.items():
        per_op.setdefault(k, []).append(v)
    print(f"iter {i}: total={totals[-1]:.0f}ms {s}", flush=True)

def pct(ts):
    ts = sorted(ts)
    return {"n": len(ts), "mean": round(statistics.mean(ts), 2),
            "p50": round(ts[len(ts) // 2], 2),
            "p95": round(ts[int(len(ts) * 0.95)], 2)}

out = {"engine": args.engine, "serial": args.serial,
       "same_scope_note": "shim=one session socket; u2=JSON-RPC HTTP per call",
       "total_ms": pct(totals),
       "per_op_ms": {k: pct(v) for k, v in per_op.items()}}
path = os.path.join(HERE, f"e2e-m3-{args.engine}.json")
json.dump(out, open(path, "w"), indent=1)
print("TOTAL", out["total_ms"], "->", path)
