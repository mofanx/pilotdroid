#!/usr/bin/env python3
"""t6 ghost-conn race reproducer (t4 mechanism):

a host that dies dirty can leave a stale media conn queued in the
agent's accept backlog (e.g. while attach is blocked in halt's
joinQuiet window); FIFO accept then promotes the ghost ahead of the
real reader under `running && curSock == null`, the real reader hits
the busy-reject and its first stream_read sees EOF.

Per round:
  1. dirty client: stream_start + read frames + os._exit (no cleanup)
  2. inject a backlog ghost: raw TCP conn to the media localabstract,
     RST'd immediately (SO_LINGER=0) — queued dead-peer conn
  3. clean client: stream_start + stream_read must still get frames

    python3 spike/e2e/agent-loop/ghost_repro.py [rounds]
"""
import os
import socket
import struct
import subprocess
import sys
import time

CLI = os.environ.get(
    "PILOTDROID_CLI",
    os.path.expanduser("~/.cargo-target/pilotdroid/debug/pilotdroid-cli"))
SERIAL = os.environ.get("PILOTDROID_SERIAL", "ba0d072d")
GHOST_PORT = 27456   # dedicated adb-forward port for injected ghosts

CLIENT = r'''
import json, os, subprocess, sys, time
cli, serial, mode = sys.argv[1], sys.argv[2], sys.argv[3]
srv = subprocess.Popen([cli, "serve", "-s", serial],
                       stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                       stderr=subprocess.DEVNULL, text=True)
_i = [0]
def rpc(name, args=None, timeout=15):
    _i[0] += 1
    srv.stdin.write(json.dumps({"jsonrpc":"2.0","id":_i[0],
        "method":"tools/call","params":{"name":name,"arguments":args or {}}})
        + "\n")
    srv.stdin.flush()
    dl = time.time() + timeout
    while time.time() < dl:
        m = json.loads(srv.stdout.readline())
        if m.get("id") == _i[0]:
            r = m["result"]
            if r.get("isError"):
                e = RuntimeError(r["content"][0]["text"][:200])
                e.stage = name
                raise e
            return json.loads(r["content"][0]["text"])
    e = TimeoutError(name)
    e.stage = name
    raise e
srv.stdin.write(json.dumps({"jsonrpc":"2.0","id":0,"method":"initialize","params":{}})+"\n")
srv.stdin.flush(); srv.stdout.readline()

if mode == "dirty":
    rpc("stream_start", {"fps": 5, "scale": 0.5})
    time.sleep(0.3)
    fr = rpc("stream_read", {"n": 2})
    sys.stdout.write("DIRTY frames:%s\n" % fr.get("count")); sys.stdout.flush()
    os._exit(9)          # dirty death — no stop, no cleanup
else:
    try:
        rpc("stream_start", {"fps": 5, "scale": 0.5})
        time.sleep(0.2)
        fr = rpc("stream_read", {"n": 2})
        rpc("stream_stop")
        sys.stdout.write("CLEAN ok frames:%s\n" % fr.get("count"))
    except Exception as e:
        # rpc() raises TimeoutError("<name>") on rpc-level timeout and
        # RuntimeError("<serve err>") on tool errors — name the stage.
        stage = getattr(e, "stage", "?")
        sys.stdout.write("CLEAN fail[%s]:%s\n" % (stage, e))
'''


def run(mode, timeout=25):
    p = subprocess.Popen(
        [sys.executable, "-u", "-c", CLIENT, CLI, SERIAL, mode],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        out, _ = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill(); p.wait()
        out = "TIMEOUT"
    return out or ""


def inject_ghost():
    """Conn that connects to the media socket then RSTs — a dead-peer
    conn for the agent's accept backlog."""
    subprocess.run(
        ["adb", "-s", SERIAL, "forward", f"tcp:{GHOST_PORT}",
         "localabstract:pilotdroid_agent_media"],
        capture_output=True)
    try:
        s = socket.create_connection(("127.0.0.1", GHOST_PORT), timeout=3)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER,
                     struct.pack("ii", 1, 0))     # close with RST
        s.close()
    except OSError:
        pass


WATCH_CLIENT = r'''
import json, os, subprocess, sys, time
cli, serial, mode = sys.argv[1], sys.argv[2], sys.argv[3]
srv = subprocess.Popen([cli, "serve", "-s", serial],
                       stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                       stderr=subprocess.DEVNULL, text=True)
_i = [0]
_notes = []      # notifications consumed while awaiting an rpc reply —
                 # a hit that lands mid-call is queued, never discarded.
def rpc(name, args=None, timeout=15):
    _i[0] += 1
    srv.stdin.write(json.dumps({"jsonrpc":"2.0","id":_i[0],
        "method":"tools/call","params":{"name":name,"arguments":args or {}}})
        + "\n")
    srv.stdin.flush()
    dl = time.time() + timeout
    while time.time() < dl:
        m = json.loads(srv.stdout.readline())
        if m.get("id") == _i[0]:
            r = m["result"]
            if r.get("isError"):
                raise RuntimeError(r["content"][0]["text"][:200])
            return json.loads(r["content"][0]["text"])
        _notes.append(m)
    raise TimeoutError(name)
def notify(timeout=12):
    import select
    for m in _notes:
        if m.get("method") == "notifications/pilotdroid/watch_hit":
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
            return "eof"
        m = json.loads(line)
        if m.get("method") == "notifications/pilotdroid/watch_hit":
            return m["params"]
srv.stdin.write(json.dumps({"jsonrpc":"2.0","id":0,"method":"initialize","params":{}})+"\n")
srv.stdin.flush(); srv.stdout.readline()

ARM = {"pkg": "com.android.settings", "action": "none", "push": True,
       "timeout_ms": 30000}
if mode == "watch-arm":
    # live pend: arm a push watcher and KEEP ALIVE until signalled —
    # the hit fires when com.android.settings reaches foreground.
    rpc("watch_arm", ARM)
    sys.stdout.write("ARMED\n"); sys.stdout.flush()
    hit = notify(15)
    sys.stdout.write("WATCH_HIT:%s\n" % bool(hit)); sys.stdout.flush()
    # phase 2: dirty death — pend corpse + armed watch_id left behind
    os._exit(9)
elif mode == "watch-clean":
    rpc("watch_arm", ARM)
    rpc("app_start", {"pkg": "com.android.settings"})
    time.sleep(1.0)        # settle past the arm-attach accept lag
    hit = notify(15)
    sys.stdout.write("CLEAN hit:%s\n" % bool(hit)); sys.stdout.flush()
'''


def watch_main(rounds):
    """pendOut ghost guard (t4): per round —

    phase 1 (live-pend guard): serve A arms a push watcher (its media
    conn is the live pendSock), then 2 RST ghost conns are injected.
    Old code let the stray ghost DISPLACE the live pend — the hit then
    wrote to a corpse and evaporated, plus the live conn was killed
    (demux EOF). Fixed code queues strays as watch-reserve; the hit
    still reaches the live reader.

    phase 2 (corpse heal): A dies dirty (os._exit) leaving a corpse
    pend + backlog ghosts; serve B's fresh watch conn queues behind the
    corpse — the next hit's failed write promotes B's reserve and
    RETRIES the event onto it instead of dropping it silently.
    """
    ok = 0
    for i in range(rounds):
        # --- phase 1: live pend + stray ghosts ---------------------
        subprocess.run(["adb", "-s", SERIAL, "shell", "input", "keyevent",
                        "KEYCODE_HOME"], capture_output=True)
        time.sleep(1.0)
        a = subprocess.Popen(
            [sys.executable, "-u", "-c", WATCH_CLIENT, CLI, SERIAL,
             "watch-arm"], stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True)
        assert "ARMED" in a.stdout.readline()
        for _ in range(2):
            inject_ghost()
            time.sleep(0.15)
        subprocess.run(["adb", "-s", SERIAL, "shell", "am", "start",
                        "com.android.settings"], capture_output=True)
        line = a.stdout.readline().strip()          # WATCH_HIT:<bool>
        a.wait(10)                                   # client os._exit(9)
        p1 = "WATCH_HIT:True" in line
        # --- phase 2: corpse pend + ghosts + fresh conn heal -------
        time.sleep(0.3)
        for _ in range(2):
            inject_ghost()
            time.sleep(0.1)
        c = run(WATCH_CLIENT, "watch-clean")
        p2 = "CLEAN hit:True" in c
        good = p1 and p2
        ok += good
        print(f"round {i}: {'PASS' if good else 'FAIL'}  p1={p1} "
              f"({line[:40]}) p2={p2} ({c.strip()[:60]})")
        time.sleep(0.3)
    subprocess.run(["adb", "-s", SERIAL, "forward", "--remove",
                    f"tcp:{GHOST_PORT}"], capture_output=True)
    print(f"\nwatch-ghost-repro: {ok}/{rounds} hits survived stray "
          f"ghosts + corpse-heal")
    sys.exit(0 if ok >= rounds * 0.95 else 1)


def run(client, mode, timeout=30):
    p = subprocess.Popen(
        [sys.executable, "-u", "-c", client, CLI, SERIAL, mode],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        out, _ = p.communicate(timeout=timeout)
    except subprocess.TimeoutExpired:
        p.kill(); p.wait()
        out = "TIMEOUT"
    return out or ""


def main():
    if len(sys.argv) > 1 and sys.argv[1] == "--watch":
        watch_main(int(sys.argv[2]) if len(sys.argv) > 2 else 10)
        return
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 20
    ok = 0
    for i in range(rounds):
        run(CLIENT, "dirty")
        for _ in range(2):           # a couple of ghosts in the backlog
            inject_ghost()
            time.sleep(0.1)
        c = run(CLIENT, "clean")
        good = "CLEAN ok" in c
        ok += good
        print(f"round {i}: {'PASS' if good else 'FAIL'}  {c.strip()[:90]}")
        time.sleep(0.2)
    subprocess.run(["adb", "-s", SERIAL, "forward", "--remove",
                    f"tcp:{GHOST_PORT}"], capture_output=True)
    print(f"\nghost-repro: {ok}/{rounds} clean reads after dirty kill + "
          f"ghost inject")
    sys.exit(0 if ok >= rounds * 0.95 else 1)


if __name__ == "__main__":
    main()
