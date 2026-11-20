#!/usr/bin/env python3
"""M10 watch-latency tri-bench: same popup scenario on three backends.

  Scenario: `am start -a android.intent.action.DELETE` on MIUI raises the
  package-manager resolver dialog ("选择要使用的应用" + 取消 button).
  A watcher armed on text="取消" dismisses it. Metric = time from
  dialog appearing in the tree to it being dismissed.

  Arms (--mode):
    device-watch : agent WATCH op — poll loop lives inside the agent
                   (CAP_WATCH, poll_ms=50). waited_ms is device-measured.
    shim-poll    : u2shim watcher with caps masked to 0 → forced host-poll
                   mode (one QUERY round-trip per interval, interval=0.5s).
                   Latency measured by an independent exists() monitor.
    u2           : real uiautomator2 `d.watcher.when(...).click()` (default
                   server poll). Same monitor technique.

  Usage:
    python3 bench_m10_watch.py --mode device-watch --n 7
    python3 bench_m10_watch.py --mode shim-poll    --n 7
    # then: deploy-agent.sh ba0d072d --kill-only  (UiAutomation singleton)
    python3 bench_m10_watch.py --mode u2           --n 7
  JSON out: e2e-m10-watch-<mode>.json
"""
import argparse, json, os, random, statistics, subprocess, sys, threading, time

HERE = os.path.dirname(os.path.abspath(__file__))
DEV = "ba0d072d"
PORT = 17932
# Resolver runs in system pkg "android" — pkg-scoping avoids matching the
# MIUI launcher's own btnCancel node (com.miui.home) when the launcher is
# foreground.
SEL = {"text": "取消", "pkg": "android"}
SEL_TEXT = "取消"
TRIGGER = ["adb", "-s", DEV, "shell", "am", "start", "-a",
           "android.intent.action.DELETE", "-d", "package:com.miui.calculator"]
POPUP_PKG_HINT = "android"  # resolver runs in system pkg on this MIUI


def trigger_popup():
    subprocess.run(TRIGGER, capture_output=True)


def dismiss_leftover(press_back=True):
    if press_back:
        subprocess.run(["adb", "-s", DEV, "shell", "input", "keyevent", "4"],
                       capture_output=True)


def stats(xs):
    return {"n": len(xs), "mean": round(statistics.mean(xs), 1),
            "p50": round(statistics.median(xs), 1),
            "min": round(min(xs), 1), "max": round(max(xs), 1)}


# ---------- arm 0: device WATCH, element already present -----------------
def arm_device_watch_hot(n):
    """Popup is already up when WATCH is issued — waited_ms ≈ pure
    detect+act floor (one tree-walk + inject)."""
    sys.path.insert(0, os.path.join(HERE, "..", "..", "agent", "scripts"))
    import agent_client as ac
    a = ac.Agent(PORT).connect(); a.hello()
    lat, fired = [], 0
    for i in range(n):
        dismiss_leftover(); time.sleep(0.6)
        trigger_popup(); time.sleep(1.0)      # dialog fully rendered
        r = a.watch(SEL, "click",
                    timeout_ms=10000, poll_ms=50)["payload"]
        ok = r.get("matched") and r.get("action_ok", True)
        fired += bool(ok)
        if ok:
            lat.append(round(r["waited_ms"], 1))
        print(f"  iter{i}: {'ok' if ok else 'miss'}")
        dismiss_leftover()
    return {"arm": "device-watch-hot", "poll": "agent-side poll_ms=50",
            "detect_and_act_ms": stats(lat),
            "fired": f"{fired}/{n}",
            "note": "element present at issue time: waited_ms ≈ single "
                    "tree-walk + inject — the WATCH detection floor"}


# ---------- arm 1: device WATCH ------------------------------------------
def arm_device_watch(n):
    sys.path.insert(0, os.path.join(HERE, "..", "..", "agent", "scripts"))
    import agent_client as ac
    a = ac.Agent(PORT).connect(); a.hello()

    latencies, ready, fired = [], [], 0

    def one(i):
        nonlocal fired
        dismiss_leftover(); time.sleep(0.6)
        sel = SEL
        res = {}
        done = threading.Event()

        def w():
            res["r"] = a.watch(sel, "click", timeout_ms=15000, poll_ms=50)
            res["t_ret"] = time.time()
            done.set()

        th = threading.Thread(target=w); th.start()
        time.sleep(0.4)                      # watch armed
        armed = time.time()
        trigger_popup()
        t_trig = time.time()
        done.wait(timeout=18); th.join(timeout=2)
        r = res["r"]["payload"]
        ok = r.get("matched") and r.get("action_ok", True)
        fired += bool(ok)
        latencies.append(round(r["waited_ms"] - (t_trig - armed) * 1000, 1))
        ready.append(round(r["waited_ms"], 1))
        dismiss_leftover()
        return ok

    for i in range(n):
        print(f"  iter{i}: {'ok' if one(i) else 'miss'}")
    return {"arm": "device-watch", "poll": "agent-side poll_ms=50",
            "waited_ms": stats(ready),
            "post_trigger_ms": stats(latencies),
            "fired": f"{fired}/{n}",
            "note": "waited_ms=device-measured watch→acted; "
                    "post_trigger_ms=waited_ms-(armed→trigger e2e) ≈ "
                    "detection+action after the am-start e2e finished"}


# ---------- arms 2/3: host-poll monitor -----------------------------------
def monitor_exists(sel_fn, stop_flag, events):
    while not stop_flag["stop"]:
        try:
            if sel_fn():
                if "appear" not in events:
                    events["appear"] = time.time()
            else:
                if "appear" in events and "gone" not in events:
                    events["gone"] = time.time()
                    return
        except Exception:
            pass
        time.sleep(0.025)


def arm_host_poll(mode, n):
    if mode == "shim-poll":
        sys.path.insert(0, os.path.join(HERE, "..", "..", "compat"))
        import u2shim as u2
        d = u2.connect(port=PORT, serial=DEV)
        d.caps = 0  # force host-poll watcher path (no CAP_WATCH)
    else:
        import uiautomator2 as u2
        d = u2.connect(DEV)
        d.implicitly_wait(0)

    lat, appear_lag, fired = [], [], 0
    for i in range(n):
        dismiss_leftover(); time.sleep(0.6)
        if mode == "shim-poll":
            # Host-poll watcher IS an exists()+click loop at `interval` —
            # instrument the poll itself instead of a parallel monitor
            # (single-client socket serializes ops; a fast monitor would
            #  starve the watcher and inflate the very thing we measure).
            res = {}
            flag = {"stop": False}

            def poll_loop():
                while not flag["stop"]:
                    if d(text=SEL_TEXT, packageName="android").exists:
                        res["t_found"] = time.time()
                        d(text=SEL_TEXT, packageName="android").click()
                        res["t_gone"] = time.time()
                        return
                    time.sleep(0.5)
            th = threading.Thread(target=poll_loop, daemon=True)
            th.start()
            t_arm = time.time()
            trigger_popup()
            th.join(timeout=20)
            flag["stop"] = True
            if "t_gone" in res:
                fired += 1
                lat.append(round((res["t_found"] - t_arm) * 1000, 1))
                appear_lag.append(round((res["t_gone"] - res["t_found"])
                                        * 1000, 1))
            print(f"  iter{i}: {'ok' if 't_gone' in res else 'miss'}")
        else:
            # u2's watcher runs inside its server — a host monitor polls
            # concurrently without starving it.
            ev, flag = {}, {"stop": False}
            mon = threading.Thread(
                target=monitor_exists,
                args=(lambda: d(text=SEL_TEXT, packageName="android").exists, flag, ev),
                daemon=True)
            d.watcher.when(
                '//*[@text="取消" and @package="android"]').click()
            d.watcher.start()          # u2 default interval
            mon.start()
            time.sleep(0.3)
            t_arm = time.time()
            trigger_popup()
            deadline = time.time() + 20
            while time.time() < deadline and "gone" not in ev:
                time.sleep(0.05)
            flag["stop"] = True; mon.join(timeout=2)
            d.watcher.stop()
            if "appear" in ev and "gone" in ev:
                fired += 1
                lat.append(round((ev["gone"] - ev["appear"]) * 1000, 1))
                appear_lag.append(round((ev["appear"] - t_arm) * 1000, 1))
            print(f"  iter{i}: {'ok' if 'gone' in ev else 'miss'}")
        dismiss_leftover()
    if mode == "shim-poll":
        return {"arm": mode, "poll": "host exists()+click loop @500ms "
                "(= shim watcher host-poll mode)",
                "trigger_to_found_ms": stats(lat) if lat else None,
                "found_to_gone_ms": stats(appear_lag) if appear_lag else None,
                "fired": f"{fired}/{n}",
                "note": "trigger→found is worst-case bound = 500ms poll "
                        "interval + one QUERY; found→gone = click latency. "
                        "No parallel monitor — single-client socket would "
                        "serialize and inflate both"}
    return {"arm": mode, "poll": "u2 server-side watcher + host monitor",
            "appear_to_gone_ms": stats(lat) if lat else None,
            "armed_to_appear_ms": stats(appear_lag) if appear_lag else None,
            "fired": f"{fired}/{n}",
            "note": "appear→gone = dialog lifetime incl. u2 watcher poll "
                    "interval + click; armed→appear ≈ am-start latency "
                    "(context, not watch overhead)"}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", required=True,
                    choices=["device-watch", "device-watch-hot",
                             "shim-poll", "u2"])
    ap.add_argument("--n", type=int, default=7)
    args = ap.parse_args()
    random.seed()

    if args.mode == "device-watch":
        out = arm_device_watch(args.n)
    elif args.mode == "device-watch-hot":
        out = arm_device_watch_hot(args.n)
    else:
        out = arm_host_poll(args.mode, args.n)
    path = os.path.join(HERE, f"e2e-m10-watch-{args.mode}.json")
    with open(path, "w") as f:
        json.dump(out, f, indent=2, ensure_ascii=False)
    print(json.dumps(out, indent=2, ensure_ascii=False))
    print("wrote", path)


if __name__ == "__main__":
    main()
