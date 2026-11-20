#!/usr/bin/env python3
"""M6 experiment: two `pilotdroid serve` processes against one device.

Each serve gets its own ephemeral adb-forward port (manager picks a free
port), so both dial the same localabstract as independent control sessions.
This script records the actual concurrent behavior:

Agent 0.5.0 enforces SINGLE-CLIENT semantics: a new control conn replaces
the previous one (its socket is closed; its session+stream die via §4.8
session binding). The serve watchdog re-dials on failure, so two serves
produce a dial-steal pattern — last connector wins, loser sees io errors
or silently resteals on its next call. Ops never interleave mid-call.

Expected behavior asserted here (probes use `dump`, an op that really
travels the control conn — `app_current` is adb-shell-only and would pass
even on a dead conn):
  A) B initialize -> B owns the agent; A's conn ops io-error (kicked).
     The kicked serve keeps its dead session — calls stay failing until
     the serve process restarts (documented: restart to re-take).
  B) the owning serve ops normally (dump ok, stream start/stop ok)
  C) owner death (SIGKILL) frees the agent -> a FRESH serve takes over

Usage: serve_multi.py <serial> [cli_binary]
"""
import json, os, signal, subprocess, sys, threading, time

SERIAL = sys.argv[1] if len(sys.argv) > 1 else "ba0d072d"
CLI = sys.argv[2] if len(sys.argv) > 2 else "target/debug/pilotdroid-cli"
INIT = {"jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {
    "protocolVersion": "2024-11-05", "capabilities": {},
    "clientInfo": {"name": "serve_multi", "version": "0"}}}
results = []


def rep(name, ok, note=""):
    results.append((name, ok, note))
    print(f"  {'PASS' if ok else 'FAIL'} {name} :: {note}")


def spawn():
    p = subprocess.Popen([CLI, "-s", SERIAL, "serve"],
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL, text=True, bufsize=1)
    return p


def call(p, method, params=None, rid=1, timeout=10):
    req = {"jsonrpc": "2.0", "id": rid, "method": method,
           "params": params or {}}
    p.stdin.write(json.dumps(req) + "\n")
    p.stdin.flush()
    p.stdout.flush()
    deadline = time.time() + timeout
    while time.time() < deadline:
        line = p.stdout.readline()
        if not line:
            return {"error": "eof"}
        try:
            r = json.loads(line)
        except json.JSONDecodeError:
            continue
        if r.get("id") == rid:
            return r
    return {"error": "timeout"}


def payload(r):
    return r.get("result", {}).get("content", [{}])[0].get("text", r)


def main():
    a = spawn()
    b = spawn()
    try:
        ra = call(a, "initialize", INIT["params"])
        rb = call(b, "initialize", INIT["params"])
        rep("both_initialize",
            "serverInfo" in str(ra) and "serverInfo" in str(rb),
            f"A={ra.get('result',{}).get('serverInfo')} B={rb.get('result',{}).get('serverInfo')}")

        # A) single-client: after B's init conn, A's control conn is dead.
        #    `dump` travels the conn -> must io-error (kicked). The dead
        #    session is retained, so further calls keep failing (documented:
        #    the kicked serve must restart to re-take ownership).
        ra1 = call(a, "tools/call", {"name": "dump", "arguments": {}}, 10)
        kicked = "io error" in str(ra1) or "failed to fill" in str(ra1) \
            or "Broken pipe" in str(ra1) or "isError': True" in str(ra1)
        rep("A_kicked_after_B_init", kicked, str(ra1)[:140])
        ra2 = call(a, "tools/call", {"name": "dump", "arguments": {}}, 11)
        rep("A_stays_dead", "io error" in str(ra2) or "isError" in str(ra2),
            str(ra2)[:140])

        # B) owning serve works normally on the live conn.
        rb1 = call(b, "tools/call", {"name": "dump", "arguments": {}}, 10)
        rep("B_dump_ok", "<hierarchy" in str(rb1) or "hierarchy" in str(rb1),
            str(rb1)[:100])
        sb = call(b, "tools/call", {"name": "stream_start", "arguments": {"fps": 2}}, 20)
        rep("B_stream_start", '"ok":true' in str(sb).replace(" ", ""), str(sb)[:120])
        st = call(b, "tools/call", {"name": "stream_stop", "arguments": {}}, 21)
        rep("B_stream_stop", "frames_sent" in str(st) or '"ok":true' in str(st).replace(" ", ""),
            str(st)[:100])

        # C) owner death frees the agent: a FRESH serve takes over. The
        #    previously-kicked A stays dead (session already dead) — only a
        #    restart re-takes ownership.
        b.send_signal(signal.SIGKILL)
        b.wait(5)
        time.sleep(0.8)
        ra3 = call(a, "tools/call", {"name": "dump", "arguments": {}}, 12)
        rep("A_still_dead_after_owner_gone",
            "io error" in str(ra3) or "isError" in str(ra3), str(ra3)[:120])
        c = spawn()
        try:
            rc = call(c, "tools/call", {"name": "dump", "arguments": {}}, 10)
            rep("fresh_serve_takes_over",
                "hierarchy" in str(rc), str(rc)[:120])
        finally:
            c.kill()
    finally:
        for p in (a, b):
            try:
                p.kill()
            except Exception:
                pass
    n_fail = sum(1 for r in results if not r[1])
    print(f"\nsummary: {len(results)-n_fail} pass / {n_fail} fail")
    sys.exit(1 if n_fail else 0)


if __name__ == "__main__":
    main()
