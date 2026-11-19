import json, statistics, time
import uiautomator2 as u2

d = u2.connect("ba0d072d")
d.implicitly_wait(0)
info = d.info
print("device:", info.get("productName"), info.get("sdkInt"))

def bench(name, fn, n=30):
    ts = []
    ok = 0
    for _ in range(n):
        t0 = time.perf_counter()
        try:
            fn(); ok += 1
        except Exception as e:
            print(name, "ERR", e)
        ts.append((time.perf_counter() - t0) * 1000)
    ts.sort()
    return {
        "n": n, "ok": ok,
        "mean_ms": round(statistics.mean(ts), 2),
        "p50_ms": round(ts[n // 2], 2),
        "p95_ms": round(ts[int(n * 0.95)], 2),
        "min_ms": round(ts[0], 2),
        "max_ms": round(ts[-1], 2),
    }

res = {"device": "ba0d072d", "u2_version": "3.7.1", "ts": int(time.time())}
res["click_xy"] = bench("click", lambda: d.click(700, 1600))
res["exists"] = bench("exists", lambda: d(text="不存在控件xyz").exists)
res["dump_hierarchy"] = bench("dump", lambda: d.dump_hierarchy())
res["screenshot"] = bench("shot", lambda: d.screenshot(), n=10)
res["set_text_ascii"] = bench("settext", lambda: d.send_keys("ab"), n=10)
out = "/home/yan/.openclaw/workspace-devin/pilotdroid/spike/bench/baseline-u2.json"
import os; os.makedirs(os.path.dirname(out), exist_ok=True)
json.dump(res, open(out, "w"), indent=2, ensure_ascii=False)
print(json.dumps(res, indent=2))
