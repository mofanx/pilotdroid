#!/usr/bin/env python3
"""M11 async-WATCH parallel-drive bench: the u2-parity scenario.

  Watcher armed device-side (WATCH_ARM), then N control ops run on the
  SAME conn while armed — with M10's blocking WATCH this was impossible
  (the op held the whole session). Popup is triggered and the WATCH_HIT
  ui_event must still arrive on the media socket.

  Scenario = same resolver popup as bench_m10_watch.py
  (`text=取消 pkg=android`).

  Metrics per iter:
    parallel_ms  — wall for PING+QUERY pairs issued while armed
    trigger_hit_ms — am start -> WATCH_HIT frame received (incl ~430ms
                     MIUI resolver render)
    waited_ms    — agent-side detect+act (first poll pass -> hit)

  Usage: python3 bench_m11_parallel.py --n 5
  JSON out: e2e-m11-watch-parallel.json
"""
import argparse, json, os, statistics, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "..", "agent", "scripts"))
import agent_client as ac

DEV, PORT, MPORT = "ba0d072d", 17932, 17933
SEL = {"text": "取消", "pkg": "android"}
TRIGGER = ["adb", "-s", DEV, "shell", "am", "start", "-a",
           "android.intent.action.DELETE", "-d", "package:com.miui.calculator"]


def stats(xs):
    return {"n": len(xs), "mean": round(statistics.mean(xs), 1),
            "p50": round(statistics.median(xs), 1),
            "min": round(min(xs), 1), "max": round(max(xs), 1)}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--n", type=int, default=5)
    args = ap.parse_args()

    subprocess.run(["adb", "-s", DEV, "forward",
                    f"tcp:{MPORT}", "localabstract:pilotdroid_agent_media"],
                   capture_output=True)
    a = ac.Agent(PORT).connect()
    caps = a.hello()["payload"]["caps"]
    assert caps & 0x200, f"CAP_WATCH_ASYNC missing caps={hex(caps)}"

    par, ops_all, hit_ms, waited, hits = [], [], [], [], 0
    for i in range(args.n):
        subprocess.run(["adb", "-s", DEV, "shell", "input", "keyevent", "4"],
                       capture_output=True)
        time.sleep(1.2)   # let the back-press transition fully settle —
                          # getRootInActiveWindow stalls ~s mid-transition
        ms = ac.media_connect(MPORT)
        ms.settimeout(15)
        r = a.watch_arm(SEL, "click", poll_ms=50)["payload"]
        wid = r["watch_id"]

        # parallel-drive: mixed control ops while the watcher polls
        t0, ops = time.time(), []
        for _ in range(4):
            for fn in (a.ping, lambda: a.query({"pkg": "android"})):
                t1 = time.time(); fn()
                ops.append(round((time.time() - t1) * 1000, 1))
        ops_all += ops
        par.append(round((time.time() - t0) * 1000, 1))
        print(f"  iter{i} op latencies: {ops}")

        subprocess.run(TRIGGER, capture_output=True)
        t_trig = time.time()
        fr = ac.read_frame(ms)
        if (fr and fr[0] == 1 and fr[1] == 0x0002
                and fr[4].get("watch_id") == wid
                and fr[4].get("matched")):
            hits += 1
            hit_ms.append(round((time.time() - t_trig) * 1000, 1))
            waited.append(round(fr[4].get("waited_ms", -1), 1))
            print(f"  iter{i}: hit par={par[-1]}ms "
                  f"trig→hit={hit_ms[-1]}ms waited={waited[-1]}ms "
                  f"action_ok={fr[4].get('action_ok')}")
        else:
            print(f"  iter{i}: MISS frame={fr}")
        ms.close()
        subprocess.run(["adb", "-s", DEV, "shell", "input", "keyevent", "4"],
                       capture_output=True)

    a.close()
    out = {"arm": "device-watch-async-parallel",
           "agent": "0.7.0", "caps": hex(caps),
           "op_latency_ms": stats(ops_all),
           "ops_over_500ms": sum(1 for x in ops_all if x > 500),
           "parallel_window_ms": stats(par),
           "trigger_to_hit_ms": stats(hit_ms),
           "device_detect_act_ms": stats(waited),
           "hits": f"{hits}/{args.n}",
           "note": "per-op latencies of 4×(PING+QUERY) on the same conn "
                   "while armed — blocking-WATCH could not interleave "
                   "these. ops_over_500 = a11y-provider stalls during "
                   "resolver teardown (framework-side: the dying app's "
                   "main thread queues a11y IPC ~2s — real u2 measured "
                   "~5.6s on app_current in the same scenario). "
                   "trigger_to_hit includes ~430ms resolver render; "
                   "waited_ms counts from arm (incl. settle+parallel "
                   "window), not detect latency."}
    dst = os.path.join(HERE, "e2e-m11-watch-parallel.json")
    json.dump(out, open(dst, "w"), indent=2)
    print(json.dumps(out, indent=2))


if __name__ == "__main__":
    main()
