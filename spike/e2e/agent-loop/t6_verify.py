#!/usr/bin/env python3
"""t6 independent verification of t2 (serve recovery UX trio).

Phases:
  A. agent pkill -> next RPC heals -> session_recovered notification
     {generation,stale_watchers,rearmed,rearm_failed}
  B. re-armed watcher: first hit on the new generation reaches PUSH
     (watch_hit notification, new watch_id)
  C. stream active: a hit sorted into session stash is drained as a
     push notification mid-stream (not only via watch_events pull)
"""
import json
import os
import select
import subprocess
import sys
import time

CLI = os.environ.get(
    "PILOTDROID_CLI",
    os.path.expanduser("~/.cargo-target/pilotdroid/debug/pilotdroid-cli"))
SERIAL = os.environ.get("PILOTDROID_SERIAL", "ba0d072d")
PKG = "com.android.settings"
ARM = {"pkg": PKG, "action": "none", "push": True, "timeout_ms": 30000}

srv = subprocess.Popen([CLI, "serve", "-s", SERIAL],
                       stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                       stderr=subprocess.DEVNULL, text=True)
_i = [0]
_notes = []


def rpc(name, args=None, timeout=20):
    _i[0] += 1
    srv.stdin.write(json.dumps({"jsonrpc": "2.0", "id": _i[0],
        "method": "tools/call", "params": {"name": name,
                                           "arguments": args or {}}}) + "\n")
    srv.stdin.flush()
    dl = time.time() + timeout
    while time.time() < dl:
        m = json.loads(srv.stdout.readline())
        if m.get("id") == _i[0]:
            r = m["result"]
            if r.get("isError"):
                raise RuntimeError(f"{name}: {r['content'][0]['text'][:200]}")
            return json.loads(r["content"][0]["text"])
        _notes.append(m)
    raise TimeoutError(name)


def notify(method, timeout=15):
    for m in _notes:
        if m.get("method") == method:
            _notes.remove(m)
            return m["params"]
    dl = time.time() + timeout
    while True:
        rem = dl - time.time()
        if rem <= 0:
            return None
        r, _, _ = select.select([srv.stdout], [], [], rem)
        if not r:
            return None
        line = srv.stdout.readline()
        if not line:
            return None
        m = json.loads(line)
        if m.get("method") == method:
            return m["params"]
        _notes.append(m)


def agent_pids():
    out = subprocess.run(
        ["adb", "-s", SERIAL, "shell", "pgrep", "-f", "pilotdroid.Agent"],
        capture_output=True, text=True).stdout.split()
    return out


def pkill_agent():
    subprocess.run(["adb", "-s", SERIAL, "shell", "pkill", "-f",
                    "pilotdroid.Agent"], capture_output=True)


try:
    srv.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 0,
                                "method": "initialize",
                                "params": {}}) + "\n")
    srv.stdin.flush()
    srv.stdout.readline()

    results = {}

    # ensure trigger target is NOT already foreground for phase B
    rpc("press", {"key": "home"})
    time.sleep(1.0)

    # ---- Phase A: arm push watcher, pkill agent, heal, notification
    w1 = rpc("watch_arm", ARM)
    wid1 = w1.get("watch_id")
    pid_before = agent_pids()
    pkill_agent()
    time.sleep(0.3)
    # next RPC triggers session heal; notification may arrive interleaved
    rpc("describe_screen", {"detail": "brief"}, timeout=30)
    rec = notify("notifications/pilotdroid/session_recovered", 15)
    pid_after = agent_pids()
    results["A_session_recovered"] = bool(rec)
    results["A_fields"] = rec
    results["A_pid"] = f"{pid_before}->{pid_after}"

    # ---- Phase B: re-armed watcher first hit -> push
    rpc("app_start", {"pkg": PKG})
    time.sleep(0.8)
    hit = notify("notifications/pilotdroid/watch_hit", 15)
    results["B_hit_push"] = bool(hit)
    results["B_watch_id"] = (hit or {}).get("watch_id")
    results["B_remapped"] = bool(hit) and hit.get("watch_id") != wid1

    # ---- Phase C: stash drain mid-stream
    rpc("press", {"key": "home"})
    time.sleep(1.2)
    rpc("watch_arm", ARM)
    rpc("stream_start", {"fps": 5, "scale": 0.5})
    time.sleep(0.4)
    rpc("app_start", {"pkg": PKG})
    # stream_read demuxes the interleaved hit into the stash; the serve
    # loop's stash drain then emits it as a push notification.
    fr = rpc("stream_read", {"n": 4}, timeout=25)
    hit2 = notify("notifications/pilotdroid/watch_hit", 15)
    results["C_stream_frames"] = fr.get("count")
    results["C_hit_midstream_push"] = bool(hit2)
    try:
        rpc("stream_stop")
    except Exception:
        pass

    print(json.dumps(results, indent=2, ensure_ascii=False))
    ok = (results["A_session_recovered"] and results["B_hit_push"]
          and results["B_remapped"] and results["C_hit_midstream_push"])
    print("T6-VERIFY:", "PASS" if ok else "FAIL")
except Exception as e:
    print("EXC:", type(e).__name__, str(e)[:300])
    print("T6-VERIFY: FAIL")
finally:
    srv.kill()
