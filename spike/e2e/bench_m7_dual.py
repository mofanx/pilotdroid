#!/usr/bin/env python3
"""M7 citation-grade benchmark: SAME u2-style script end-to-end on both
engines — real uiautomator2 vs pilotdroid shim — total wall-clock ratio.

  # kill pilotdroid agent first for --engine u2 (UiAutomation is a singleton);
  # deploy it again for --engine shim.
  python3 bench_m7_dual.py --engine u2  --serial ba0d072d --n 5
  python3 bench_m7_dual.py --engine shim --serial ba0d072d --port 17932 --n 5

Scenario = the M6 compat ops (minus xpath): identical call surface on both
sides. Per-op ms + per-iteration total wall; JSON out e2e-m7-<engine>-<san>.json
"""
import argparse, json, os, statistics, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser()
ap.add_argument("--engine", choices=["shim", "u2"], required=True)
ap.add_argument("--serial", default="ba0d072d")
ap.add_argument("--port", type=int, default=17932)
ap.add_argument("--n", type=int, default=5)
args = ap.parse_args()

if args.engine == "shim":
    sys.path.insert(0, os.path.join(HERE, "..", "..", "compat"))
    import u2shim as u2  # noqa: E402
    d = u2.connect(port=args.port, serial=args.serial)
else:
    import uiautomator2 as u2  # noqa: E402
    d = u2.connect(args.serial)
    d.implicitly_wait(0)


def sel_info(s):
    v = s.info
    return v() if callable(v) else v


def set_text(d):
    return d(className="android.widget.EditText").set_text("pd")


OPS = [
    # (name, fn) — same script, both engines
    ("d.info",            lambda: d.info),
    ("d.app_current",     lambda: d.app_current()),
    ("d.click(x,y)",      lambda: d.click(500, 500)),
    ("d(text).exists",    lambda: d(text="设置").exists),
    ("d(class,index).click", lambda: d(className="android.widget.LinearLayout", index=0).click()),
    ("d.screenshot",      lambda: d.screenshot("/tmp/m7_shot.jpg")),
    ("d.dump_hierarchy",  lambda: len(d.dump_hierarchy())),
    ("d.press(back)",     lambda: d.press("back")),
    ("d(className).exists", lambda: d(className="android.widget.TextView").exists),
    ("d(scrollable).scroll.forward", lambda: d(scrollable=True).scroll.forward()),
    ("d(scrollable).scroll.toBeginning", lambda: d(scrollable=True).scroll.toBeginning()),
    ("d(packageName).exists", lambda: d(packageName="com.android.settings").exists),
    ("d(resourceId).info", lambda: sel_info(d(className="android.widget.TextView", index=0))),
    ("d(description).exists", lambda: d(className="android.widget.TextView", index=1).exists),
    ("d.app_stop",        lambda: d.app_stop("com.android.settings")),
    ("d.app_start",       lambda: d.app_start("com.android.settings")),
    ("d.set_text(EditText)", lambda: set_text(d)),
    ("d.long_click(x,y)", lambda: d.long_click(500, 1200, 0.6)),
    ("d.swipe",           lambda: d.swipe(500, 1800, 500, 800, duration=0.2)),
    ("d.press(home)",     lambda: d.press("home")),
    ("d.press(recent)",   lambda: d.press("recent")),
]


def iteration():
    steps, errs = {}, {}
    t0 = time.perf_counter()
    for name, fn in OPS:
        t = time.perf_counter()
        try:
            fn()
            steps[name] = round((time.perf_counter() - t) * 1000, 1)
        except Exception as e:  # noqa: BLE001
            steps[name] = round((time.perf_counter() - t) * 1000, 1)
            errs[name] = f"{type(e).__name__}: {e}"[:100]
        time.sleep(0.15)   # UI settle — same for both engines
    return round((time.perf_counter() - t0) * 1000, 1), steps, errs


def main():
    # warm-up (JIT/init paths) then measure
    iteration()
    totals, allsteps, allerrs = [], {}, {}
    for i in range(args.n):
        tot, steps, errs = iteration()
        totals.append(tot)
        for k, v in steps.items():
            allsteps.setdefault(k, []).append(v)
        allerrs.update(errs)
        print(f"iter{i}: total={tot}ms" + (f" errs={errs}" if errs else ""))
    doc = {
        "engine": args.engine, "serial": args.serial, "n": args.n,
        "iter_total_ms": totals,
        "total_ms_p50": round(statistics.median(totals), 1),
        "per_op_ms_p50": {k: round(statistics.median(v), 1)
                          for k, v in allsteps.items()},
        "errors": allerrs,
    }
    san = args.serial.replace(":", "_").replace(".", "_")
    out = os.path.join(HERE, f"e2e-m7-{args.engine}-{san}.json")
    with open(out, "w") as f:
        json.dump(doc, f, ensure_ascii=False, indent=2)
    print(f"\n{args.engine}@{args.serial}: total_p50={doc['total_ms_p50']}ms -> {out}")


if __name__ == "__main__":
    main()
