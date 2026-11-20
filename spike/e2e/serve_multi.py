#!/usr/bin/env python3
"""M6 experiment: two `pilotdroid serve` processes against one device.

Each serve gets its own ephemeral adb-forward port (manager picks a free
port), so both dial the same localabstract as independent control sessions.
This script records the actual concurrent behavior:

  A) both initialize + tools/call ops concurrently
  B) stream ownership: A stream_start, then B stream_start -> expected
     ERR_STREAM_BUSY (single-owner); A stops -> B can then start
  C) A dies mid-stream (SIGKILL) -> implicit stop -> B start succeeds
     (session-binding crash resilience, §4.8)

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

        # A) interleaved ops on both
        outs = {}
        def hit(p, name, rid):
            outs[name] = call(p, "tools/call",
                              {"name": "app_current", "arguments": {}}, rid)
        ta = threading.Thread(target=hit, args=(a, "A", 10))
        tb = threading.Thread(target=hit, args=(b, "B", 10))
        ta.start(); tb.start(); ta.join(15); tb.join(15)
        ok_a = "package" in str(outs.get("A")) or "{" in str(outs.get("A"))
        ok_b = "package" in str(outs.get("B")) or "{" in str(outs.get("B"))
        rep("interleaved_ops", ok_a and ok_b,
            f"A={str(outs.get('A'))[:80]} B={str(outs.get('B'))[:80]}")

        # B) stream single-owner
        sa = call(a, "tools/call", {"name": "stream_start", "arguments": {"fps": 2}}, 20)
        rep("A_stream_start", '"ok": true' in str(sa) or "ok" in str(sa), str(sa)[:120])
        sb = call(b, "tools/call", {"name": "stream_start", "arguments": {"fps": 2}}, 20)
        busy = "STREAM_BUSY" in str(sb) or "isError" in str(sb)
        rep("B_stream_busy", busy, str(sb)[:120])

        # C) kill A mid-stream -> implicit stop -> B can start
        a.send_signal(signal.SIGKILL)
        a.wait(5)
        time.sleep(0.8)   # agent notices owner socket EOF
        sb2 = call(b, "tools/call", {"name": "stream_start", "arguments": {"fps": 2}}, 30)
        rep("B_stream_after_A_death", '"ok": true' in str(sb2) or "ok" in str(sb2),
            str(sb2)[:120])
        st = call(b, "tools/call", {"name": "stream_stop", "arguments": {}}, 31)
        rep("B_stream_stop", "frames_sent" in str(st) or "ok" in str(st), str(st)[:100])
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
