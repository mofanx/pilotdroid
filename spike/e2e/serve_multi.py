#!/usr/bin/env python3
"""M6 experiment: two `pilotdroid serve` processes against one device.

Each serve gets its own ephemeral adb-forward port (manager picks a free
port), so both dial the same localabstract as independent control sessions.
This script records the actual concurrent behavior:

Agent enforces SINGLE-CLIENT semantics: a new control conn replaces the
previous one (its socket is closed; its session+stream die via §4.8).
Since M10 the serve SELF-HEALS: a tools/call on a dead conn fails once,
the session is dropped, and the NEXT call lazily re-dials — re-stealing
ownership (last-connector-wins ping-pong, documented ADR-007). Ops never
interleave mid-call.

Expected behavior asserted here (probes use `dump`, an op that really
travels the control conn — `app_current` is adb-shell-only and would pass
even on a dead conn):
  A) B initialize -> B owns; A's first dump io-errors (kick detected).
  B) A's second dump SUCCEEDS — self-heal re-dial re-claims the agent
     (reconnect-takeover), which in turn kicks B.
  C) B likewise self-heals within two calls — ping-pong proven both ways.
  D) owner works normally (dump ok, stream start/stop ok).
  E) owner death (SIGKILL) frees the agent -> the kicked side self-heals
     within two calls; a FRESH serve also takes over.

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
        #    First `dump` io-errors (kick detected) — the serve drops the
        #    dead session.
        ra1 = call(a, "tools/call", {"name": "dump", "arguments": {}}, 10)
        kicked = "io error" in str(ra1) or "failed to fill" in str(ra1) \
            or "Broken pipe" in str(ra1) or "isError': True" in str(ra1)
        rep("A_kicked_after_B_init", kicked, str(ra1)[:140])

        # B) self-heal: A's NEXT call re-dials and RE-CLAIMS ownership
        #    (last-connector-wins) — dump now succeeds.
        ra2 = call(a, "tools/call", {"name": "dump", "arguments": {}}, 11)
        rep("A_self_heals_retakes",
            "hierarchy" in str(ra2), str(ra2)[:140])

        # C) ping-pong the other way: A's re-dial kicked B — B's first
        #    call may io-error, its second must succeed (reconnect
        #    takeover both directions proven).
        rb0 = call(b, "tools/call", {"name": "dump", "arguments": {}}, 10)
        rb1 = call(b, "tools/call", {"name": "dump", "arguments": {}}, 11)
        b_ok = "hierarchy" in str(rb0) or "hierarchy" in str(rb1)
        rep("B_self_heals_retakes", b_ok,
            f"first={str(rb0)[:60]} second={str(rb1)[:60]}")

        # D) current owner works normally on the live conn.
        rb2 = call(b, "tools/call", {"name": "dump", "arguments": {}}, 12)
        rep("owner_dump_ok", "hierarchy" in str(rb2), str(rb2)[:100])
        sb = call(b, "tools/call", {"name": "stream_start", "arguments": {"fps": 2}}, 20)
        rep("owner_stream_start", '"ok":true' in str(sb).replace(" ", ""), str(sb)[:120])
        st = call(b, "tools/call", {"name": "stream_stop", "arguments": {}}, 21)
        rep("owner_stream_stop", "frames_sent" in str(st) or '"ok":true' in str(st).replace(" ", ""),
            str(st)[:100])

        # E) owner death frees the agent: the kicked side self-heals
        #    within two calls, and a FRESH serve also takes over.
        b.send_signal(signal.SIGKILL)
        b.wait(5)
        time.sleep(0.8)
        ra3 = call(a, "tools/call", {"name": "dump", "arguments": {}}, 12)
        ra4 = call(a, "tools/call", {"name": "dump", "arguments": {}}, 13)
        rep("A_retakes_after_owner_gone",
            "hierarchy" in str(ra3) or "hierarchy" in str(ra4),
            f"first={str(ra3)[:60]} second={str(ra4)[:60]}")
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
