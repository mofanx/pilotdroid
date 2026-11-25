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


def main():
    rounds = int(sys.argv[1]) if len(sys.argv) > 1 else 20
    ok = 0
    for i in range(rounds):
        run("dirty")
        for _ in range(2):           # a couple of ghosts in the backlog
            inject_ghost()
            time.sleep(0.1)
        c = run("clean")
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
