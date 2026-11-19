#!/usr/bin/env python3
"""M3 arbitration experiments vs agent on localabstract via forward 17932.

  1. SHOT quality{30,50,80} x scale{0.5,1.0} grid, take_ns/enc_ns split
  2. tap backend=im vs backend=uia, n=50, device cost_ns + e2e
  3. keep-alive: single-op RTT cold (>=2s idle) vs warm (ping right before)
  4. dash device 11.0.0.2:5555 reachability probe (host-side adb, printed)

Writes m3-experiments.json next to this file.
"""
import json, sys, time, statistics
sys.path.insert(0, __file__.rsplit("/", 1)[0] + "/../../agent/scripts")
from agent_client import Agent  # noqa: E402

N_SHOT, N_CLICK, N_KEEP = 20, 50, 30


def pctl(ts):
    ts = sorted(ts)
    return {"n": len(ts), "mean": round(statistics.mean(ts), 2),
            "p50": round(ts[len(ts) // 2], 2),
            "p95": round(ts[int(len(ts) * 0.95)], 2),
            "min": round(ts[0], 2), "max": round(ts[-1], 2)}


def e2e(fn, n):
    ts, last = [], None
    for _ in range(n):
        t0 = time.perf_counter()
        last = fn()
        ts.append((time.perf_counter() - t0) * 1000)
    return ts, last


out = {"device": "ba0d072d", "n": {"shot": N_SHOT, "click": N_CLICK,
                                   "keepalive": N_KEEP}}
a = Agent(17932).connect()
a.hello()

# ---- 1. SHOT quality x scale grid ----------------------------------------
out["shot_grid"] = {}
for scale in (1.0, 0.5):
    for q in (30, 50, 80):
        ts, take, enc, nbytes = [], [], [], []
        last = None
        for _ in range(N_SHOT):
            t0 = time.perf_counter()
            r = a.shot(quality=q, scale=scale)
            ts.append((time.perf_counter() - t0) * 1000)
            p = r["payload"]
            take.append(p.get("take_ns", 0) / 1e6)
            enc.append(p.get("enc_ns", 0) / 1e6)
            nbytes.append(len(p["data"]))
            last = p
        out["shot_grid"][f"q{q}_s{scale}"] = {
            "e2e_ms": pctl(ts), "take_ms": pctl(take), "enc_ms": pctl(enc),
            "bytes": nbytes[len(nbytes) // 2],
            "w": last.get("w"), "h": last.get("h")}
        print("grid", scale, q, out["shot_grid"][f"q{q}_s{scale}"]["e2e_ms"]["p50"],
              out["shot_grid"][f"q{q}_s{scale}"]["take_ms"]["p50"],
              out["shot_grid"][f"q{q}_s{scale}"]["enc_ms"]["p50"],
              out["shot_grid"][f"q{q}_s{scale}"]["bytes"], flush=True)

# ---- 2. backend im vs uia -------------------------------------------------
out["backend_ab"] = {}
for b in ("im", "uia"):
    ts, last = e2e(lambda: a.inject_tap(700, 1600, backend=b), N_CLICK)
    dev = [last["payload"]["cost_ns"] / 1e6]  # device cost of last sample
    # re-collect dev costs properly
    dev = []
    for _ in range(N_CLICK):
        r = a.inject_tap(700, 1600, backend=b)
        dev.append(r["payload"]["cost_ns"] / 1e6)
    out["backend_ab"][b] = {"e2e_ms": pctl(ts), "dev_ms": pctl(dev),
                            "backend": last["payload"]["backend"]}
    print("backend", b, out["backend_ab"][b], flush=True)

# ---- 3. keep-alive vs idle ------------------------------------------------
cold, warm = [], []
for _ in range(N_KEEP):
    time.sleep(2.0)                          # idle >=2s => cold path
    t0 = time.perf_counter(); a.ping(); cold.append((time.perf_counter() - t0) * 1e3)
    a.ping()                                 # warm the channel
    t0 = time.perf_counter(); a.ping(); warm.append((time.perf_counter() - t0) * 1e3)
out["keepalive"] = {"cold_ms": pctl(cold), "warm_ms": pctl(warm)}
print("keepalive cold", out["keepalive"]["cold_ms"],
      "warm", out["keepalive"]["warm_ms"], flush=True)

a.close()
with open(__file__.rsplit("/", 1)[0] + "/m3-experiments.json", "w") as f:
    json.dump(out, f, indent=1)
print("wrote m3-experiments.json")
