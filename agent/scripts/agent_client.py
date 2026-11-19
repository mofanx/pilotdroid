#!/usr/bin/env python3
"""Minimal pilotdroid agent client — speaks the PD01 18-byte frame protocol.

Frame: magic"PD01"(4) ver(u8) ch(u8) type(u16) flags(u16) req_id(u32) len(u32)
       + msgpack payload   (all integers big-endian, docs/02-protocol.md §2)

Usage: agent_client.py [--port PORT] <cmd> [args]
  hello | ping | inject-tap X Y | inject-swipe X1 Y1 X2 Y2 MS | dump | shot OUT |
  stop | bench (all ops + latency JSON)
"""
import json, socket, struct, sys, time

import msgpack

MAGIC = b"PD01"
VER = 0
CH_CTRL = 0
F_RESP, F_MORE, F_ERR = 0x0001, 0x0002, 0x0004

T = dict(HELLO=0x0001, HELLO_ACK=0x0002, PING=0x0003, PONG=0x0004,
         INJECT=0x0010, INJECT_RESULT=0x0011, DUMP=0x0020,
         SCREENSHOT=0x0030, AGENT_STOP=0x00F0)


class Agent:
    def __init__(self, port=17932):
        self.port = port
        self.sock = None
        self.req_id = 0

    def connect(self):
        self.sock = socket.create_connection(("127.0.0.1", self.port), timeout=10)
        self.sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        return self

    def close(self):
        if self.sock:
            self.sock.close()

    def _recvn(self, n):
        buf = b""
        while len(buf) < n:
            d = self.sock.recv(n - len(buf))
            if not d:
                raise ConnectionError("agent closed the socket")
            buf += d
        return buf

    def call(self, mtype, payload=None):
        self.req_id += 1
        body = msgpack.packb(payload or {}, use_bin_type=True)
        hdr = MAGIC + struct.pack(">BBHHI", VER, CH_CTRL, mtype, 0, self.req_id) \
            + struct.pack(">I", len(body))
        self.sock.sendall(hdr + body)
        rh = self._recvn(18)
        assert rh[:4] == MAGIC, f"bad magic {rh[:4]!r}"
        ver, ch, rtype, flags, rid = struct.unpack(">BBHHI", rh[4:14])
        plen = struct.unpack(">I", rh[14:18])[0]
        data = self._recvn(plen) if plen else b""
        obj = msgpack.unpackb(data, raw=False) if data else {}
        return dict(type=rtype, flags=flags, req_id=rid, payload=obj,
                    raw_payload=data)

    def hello(self):
        return self.call(T["HELLO"], {"proto": VER})

    def ping(self):
        return self.call(T["PING"], {})

    def inject_tap(self, x, y, wait_finish=False):
        return self.call(T["INJECT"],
                         {"op": "tap", "x": float(x), "y": float(y),
                          "wait_finish": bool(wait_finish)})

    def inject_swipe(self, x1, y1, x2, y2, duration_ms=300, steps=10):
        return self.call(T["INJECT"],
                         {"op": "swipe", "x1": float(x1), "y1": float(y1),
                          "x2": float(x2), "y2": float(y2),
                          "duration_ms": int(duration_ms), "steps": int(steps)})

    def dump(self):
        return self.call(T["DUMP"], {})

    def shot(self, quality=80):
        return self.call(T["SCREENSHOT"], {"format": "jpeg", "quality": quality})

    def stop(self):
        return self.call(T["AGENT_STOP"], {})


def _timed(fn, n=1):
    """Return (last_result, wall_ms_list, on_device_ms_list).

    on_device = agent-internal cost_ns from the response payload, so the
    report separates adb-forward transport overhead from real work."""
    res, ts, dev = None, [], []
    for _ in range(n):
        t0 = time.perf_counter()
        res = fn()
        ts.append((time.perf_counter() - t0) * 1000)
        c = res.get("payload", {}).get("cost_ns") if isinstance(res, dict) else None
        dev.append(c / 1e6 if c else 0.0)
    return res, ts, dev


def bench(a, n=30):
    """Run every opcode n times, return latency dict."""
    out = {}
    _, ts, _ = _timed(a.hello, 3)
    out["hello_ms"] = ts
    _, ts, _ = _timed(a.ping, n)
    out["ping_ms"] = ts
    res, ts, dev = _timed(lambda: a.inject_tap(700, 1600), n)
    out["inject_tap_ms"] = ts
    out["inject_tap_dev_ms"] = dev
    out["inject_tap_last"] = res["payload"]
    res, ts, dev = _timed(lambda: a.inject_swipe(700, 2000, 700, 1500, 300), 5)
    out["inject_swipe_ms"] = ts
    out["inject_swipe_dev_ms"] = dev
    out["inject_swipe_last"] = res["payload"]
    res, ts, dev = _timed(a.dump, n)
    out["dump_ms"] = ts
    out["dump_dev_ms"] = dev
    out["dump_bytes"] = len(res["raw_payload"])
    res, ts, _ = _timed(a.shot, 10)
    out["shot_ms"] = ts
    out["shot_bytes"] = len(res["raw_payload"])
    return out


def _pct(ts):
    ts = sorted(ts)
    return {"mean": round(sum(ts) / len(ts), 2),
            "p50": round(ts[len(ts) // 2], 2),
            "p95": round(ts[int(len(ts) * 0.95)], 2),
            "min": round(ts[0], 2), "max": round(ts[-1], 2)}


def main():
    args = sys.argv[1:]
    port = 17932
    if args[:2] == ["--port", args[1] if len(args) > 1 else ""]:
        port = int(args[1]); args = args[2:]
    elif args and args[0] == "--port":
        port = int(args[1]); args = args[2:]
    cmd = args[0] if args else "hello"
    a = Agent(port).connect()
    try:
        if cmd == "hello":
            print(json.dumps(a.hello(), default=str))
        elif cmd == "ping":
            print(json.dumps(a.ping(), default=str))
        elif cmd == "inject-tap":
            print(json.dumps(a.inject_tap(args[1], args[2]), default=str))
        elif cmd == "inject-swipe":
            print(json.dumps(a.inject_swipe(*map(float, args[1:6])), default=str))
        elif cmd == "dump":
            r = a.dump()
            sys.stdout.write(r["payload"].get("data", ""))
        elif cmd == "shot":
            r = a.shot()
            open(args[1], "wb").write(r["payload"]["data"])
            print(f"wrote {len(r['payload']['data'])}B -> {args[1]}")
        elif cmd == "stop":
            print(json.dumps(a.stop(), default=str))
        elif cmd == "bench":
            n = int(args[1]) if len(args) > 1 else 30
            b = bench(a, n)
            print(json.dumps({k: (_pct(v) if k.endswith("_ms") else v)
                              for k, v in b.items()}, indent=2))
        else:
            sys.exit(f"unknown cmd {cmd}")
    finally:
        a.close()


if __name__ == "__main__":
    main()
