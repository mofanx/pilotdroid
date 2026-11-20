#!/usr/bin/env python3
"""M5 4-path bench — same scenario via CLI one-shot / shim / serve / Rust lib.

Scenario per iteration (identical op semantics on every path):
  tap(700,1500) -> dump -> exists(class=TextView) -> screenshot(scale=0.5)

Paths: cli  = spawn pilotdroid-cli per op (pays fwd+connect+HELLO every call)
       shim = u2shim persistent session (python)
       serve= one `pilotdroid serve` process, tools/call JSON-RPC lines
       lib  = bench_session example (cargo run), persistent Session

Writes e2e-m5-4path.json. Usage: python3 bench_m5_4path.py [n] [serial]
"""
import json, os, statistics, subprocess, sys, time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..")
CLI = os.path.join(ROOT, "target", "debug", "pilotdroid-cli")
N = int(sys.argv[1]) if len(sys.argv) > 1 else 10
DEV = sys.argv[2] if len(sys.argv) > 2 else "ba0d072d"
SEL = {"class": "android.widget.TextView"}


def p50(v):
    return round(statistics.median(v), 1)


# ---------- path: CLI one-shot ----------
def bench_cli():
    def run(args):
        t = time.perf_counter()
        subprocess.run([CLI, "-s", DEV, "agent"] + args,
                       capture_output=True, check=True)
        return (time.perf_counter() - t) * 1000

    tap, dump, shot = [], [], []
    for _ in range(N):
        tap.append(run(["tap", "700", "1500"]))
        dump.append(run(["dump"]))
        shot.append(run(["shot", "-o", "/tmp/m5cli.jpg", "--scale", "0.5"]))
    return {"engine": "cli", "tap_ms": p50(tap), "dump_ms": p50(dump),
            "exists_ms": None, "shot_ms": p50(shot)}


# ---------- path: shim (persistent python session) ----------
def bench_shim():
    sys.path.insert(0, os.path.join(ROOT, "compat"))
    import u2shim
    d = u2shim.connect(port=17932, serial=DEV)
    tap, dump, exists, shot = [], [], [], []
    for _ in range(N):
        t = time.perf_counter(); d.a.inject_tap(700, 1500, wait_finish=True)
        tap.append((time.perf_counter() - t) * 1000)
        t = time.perf_counter(); d.dump_hierarchy()
        dump.append((time.perf_counter() - t) * 1000)
        t = time.perf_counter(); d(className="android.widget.TextView").exists
        exists.append((time.perf_counter() - t) * 1000)
        t = time.perf_counter(); d.screenshot(scale=0.5)
        shot.append((time.perf_counter() - t) * 1000)
    d.close()
    return {"engine": "shim", "tap_ms": p50(tap), "dump_ms": p50(dump),
            "exists_ms": p50(exists), "shot_ms": p50(shot)}


# ---------- path: serve (one process, tools/call) ----------
def bench_serve():
    proc = subprocess.Popen([CLI, "-s", DEV, "serve"], stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                            text=True)
    req_id = 0

    def call(name, args):
        nonlocal req_id
        req_id += 1
        line = json.dumps({"jsonrpc": "2.0", "id": req_id,
                           "method": "tools/call",
                           "params": {"name": name, "arguments": args}})
        t = time.perf_counter()
        proc.stdin.write(line + "\n"); proc.stdin.flush()
        resp = json.loads(proc.stdout.readline())
        ms = (time.perf_counter() - t) * 1000
        if "error" in resp:
            raise RuntimeError(resp["error"])
        return ms, resp["result"]

    # handshake
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                   "clientInfo": {"name": "bench", "version": "0"}}}) + "\n")
    proc.stdin.flush(); proc.stdout.readline()

    tap, dump, exists, shot = [], [], [], []
    for _ in range(N):
        ms, _ = call("tap", {"x": 700, "y": 1500}); tap.append(ms)
        ms, _ = call("dump", {}); dump.append(ms)
        ms, _ = call("exists", SEL); exists.append(ms)
        ms, _ = call("screenshot", {"quality": 70, "scale": 0.5}); shot.append(ms)
    proc.stdin.close(); proc.wait(timeout=10)
    return {"engine": "serve", "tap_ms": p50(tap), "dump_ms": p50(dump),
            "exists_ms": p50(exists), "shot_ms": p50(shot)}


# ---------- path: rust lib (persistent Session) ----------
def bench_lib():
    out = subprocess.run(
        ["cargo", "run", "-q", "-p", "pilotdroid-agent", "--example",
         "bench_session", "--", DEV, str(N)],
        cwd=ROOT, capture_output=True, text=True, check=True)
    return json.loads(out.stdout.strip().splitlines()[-1])


if __name__ == "__main__":
    results = []
    for name, fn in [("cli", bench_cli), ("lib", bench_lib),
                     ("serve", bench_serve), ("shim", bench_shim)]:
        print(f"--- bench {name} ...", file=sys.stderr)
        r = fn(); results.append(r); print(json.dumps(r))
    out = {"n_per_op": N, "serial": DEV,
           "scenario": "tap -> dump -> exists(class) -> shot(0.5)",
           "same_semantics": "wait_finish taps; exists absent on CLI surface",
           "results": results}
    with open(os.path.join(HERE, "e2e-m5-4path.json"), "w") as f:
        json.dump(out, f, indent=1)
    print("-> e2e-m5-4path.json", file=sys.stderr)
