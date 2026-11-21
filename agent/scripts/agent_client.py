#!/usr/bin/env python3
"""Minimal pilotdroid agent client — speaks the PD01 18-byte frame protocol.

Frame: magic"PD01"(4) ver(u8) ch(u8) type(u16) flags(u16) req_id(u32) len(u32)
       + msgpack payload   (all integers big-endian, docs/02-protocol.md §2)

Usage: agent_client.py [--port PORT] <cmd> [args]
  hello | ping | inject-tap X Y | inject-swipe X1 Y1 X2 Y2 MS | dump | shot OUT |
  stop | bench (all ops + latency JSON)
"""
import json, socket, struct, sys, threading, time

import msgpack

MAGIC = b"PD01"
VER = 0
CH_CTRL = 0
F_RESP, F_MORE, F_ERR, F_NOACK = 0x0001, 0x0002, 0x0004, 0x0008

T = dict(HELLO=0x0001, HELLO_ACK=0x0002, PING=0x0003, PONG=0x0004,
         INJECT=0x0010, INJECT_RESULT=0x0011, DUMP=0x0020, QUERY=0x0021,
         SCREENSHOT=0x0030, BATCH=0x0040, BATCH_RESULT=0x0041,
         STREAM_CTRL=0x0050, FRAME=0x0001, WATCH_HIT=0x0002,
         TEXT=0x0060, WATCH=0x0070, WATCH_ARM=0x0071,
         WATCH_DISARM=0x0072, APP_INFO=0x0080, AGENT_STOP=0x00F0)


class Agent:
    def __init__(self, port=17932):
        self.port = port
        self.sock = None
        self.req_id = 0
        self._lock = threading.Lock()

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

    def call(self, mtype, payload=None, flags=0):
        # one control conn serializes all ops (a second conn would REPLACE
        # this session) — lock so a watcher thread + user thread can share.
        with self._lock:
            return self._call_locked(mtype, payload, flags)

    def _call_locked(self, mtype, payload=None, flags=0):
        self.req_id += 1
        body = msgpack.packb(payload or {}, use_bin_type=True)
        hdr = MAGIC + struct.pack(">BBHHI", VER, CH_CTRL, mtype, flags,
                                  self.req_id) + struct.pack(">I", len(body))
        self.sock.sendall(hdr + body)
        if flags & F_NOACK:
            return None  # fire-and-forget: contract says no response frame
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

    def inject_tap(self, x, y, wait_finish=False, backend=None):
        req = {"op": "tap", "x": float(x), "y": float(y),
               "wait_finish": bool(wait_finish)}
        if backend:
            req["backend"] = backend
        return self.call(T["INJECT"], req)

    def inject_swipe(self, x1, y1, x2, y2, duration_ms=300, steps=10,
                     backend=None):
        req = {"op": "swipe", "x1": float(x1), "y1": float(y1),
               "x2": float(x2), "y2": float(y2),
               "duration_ms": int(duration_ms), "steps": int(steps)}
        if backend:
            req["backend"] = backend
        return self.call(T["INJECT"], req)

    def inject_key(self, keycode, meta=0, backend=None):
        req = {"kind": "Key", "keycode": int(keycode),
               "meta": int(meta), "wait_finish": False}
        if backend:
            req["backend"] = backend
        return self.call(T["INJECT"], req)

    def inject_swipe_pts(self, x1, y1, x2, y2, duration_ms=300, steps=10,
                         backend=None):
        pts = []
        for i in range(int(steps) + 1):
            f = i / int(steps)
            pts.append({"action": 0 if i == 0 else (1 if i == int(steps) else 2),
                        "x": float(x1) + (float(x2) - float(x1)) * f,
                        "y": float(y1) + (float(y2) - float(y1)) * f,
                        "t_ms": int(int(duration_ms) * f)})
        req = {"kind": "Swipe", "points": pts, "wait_finish": False}
        if backend:
            req["backend"] = backend
        return self.call(T["INJECT"], req)

    def inject_multi(self, points, wait_finish=True, backend=None):
        """Multi-pointer INJECT: points=[{action,x,y,t_ms,pid}] — >1 distinct
        pid takes the agent's multi-pointer path (pinch/gesture class)."""
        req = {"kind": "Swipe", "points": points,
               "wait_finish": bool(wait_finish)}
        if backend:
            req["backend"] = backend
        return self.call(T["INJECT"], req)

    def watch(self, selector, action="click", timeout_ms=10000, poll_ms=None):
        """WATCH op (§4.9): device-side selector->action loop — detection
        latency is one tree-walk, not a host round-trip per poll."""
        req = {"selector": selector, "action": {"type": action},
               "timeout_ms": int(timeout_ms)}
        if poll_ms:
            req["poll_ms"] = int(poll_ms)
        return self.call(T["WATCH"], req)

    def watch_arm(self, selector, action="click", timeout_ms=0, poll_ms=None):
        """WATCH_ARM (§4.9 async): arms a device-side poll thread — returns
        immediately; hits arrive as WATCH_HIT frames on the media socket
        (media_connect + read_frame, ch=1 type 0x0002)."""
        req = {"selector": selector, "action": {"type": action},
               "timeout_ms": int(timeout_ms)}
        if poll_ms:
            req["poll_ms"] = int(poll_ms)
        return self.call(T["WATCH_ARM"], req)

    def watch_disarm(self, watch_id):
        return self.call(T["WATCH_DISARM"], {"watch_id": int(watch_id)})

    def app_info(self):
        """APP_INFO (CAP_APP_INFO): foreground pkg via UiAutomation — ~ms,
        no dumpsys spawn."""
        return self.call(T["APP_INFO"], {})

    def inject_tap_noack(self, x, y):
        return self.call(T["INJECT"],
                         {"kind": "Tap",
                          "points": [{"action": 0, "x": float(x),
                                      "y": float(y), "t_ms": 0},
                                     {"action": 1, "x": float(x),
                                      "y": float(y), "t_ms": 75}],
                          "wait_finish": False}, flags=F_NOACK)

    def dump(self):
        return self.call(T["DUMP"], {})

    def query(self, selector):
        return self.call(T["QUERY"], {"selector": selector})

    def wait_until(self, selector, timeout_ms=5000, poll_ms=None):
        req = {"selector": selector, "timeout_ms": int(timeout_ms)}
        if poll_ms:
            req["poll_ms"] = int(poll_ms)
        return self.batch([{"wait_until": req}])

    def batch(self, ops, fail_policy="stop", deadline_ms=None, poll_ms=None):
        req = {"ops": ops, "fail_policy": fail_policy}
        if deadline_ms:
            req["deadline_ms"] = int(deadline_ms)
        if poll_ms:
            req["poll_ms"] = int(poll_ms)
        return self.call(T["BATCH"], req)

    def text(self, s, clear=False):
        return self.call(T["TEXT"], {"text": str(s), "clear": bool(clear)})

    def shot(self, quality=80, scale=1.0):
        return self.call(T["SCREENSHOT"],
                         {"format": "jpeg", "quality": quality,
                          "scale": float(scale)})

    def stream_start(self, fps=5, scale=0.5, quality=70, fmt="jpeg"):
        return self.call(T["STREAM_CTRL"],
                         {"op": "start", "fps": int(fps),
                          "scale": float(scale), "quality": int(quality),
                          "format": fmt})

    def stream_stop(self):
        return self.call(T["STREAM_CTRL"], {"op": "stop"})

    def stop(self):
        return self.call(T["AGENT_STOP"], {})


def media_connect(port):
    """Open the ch=1 media socket (second adb forward)."""
    s = socket.create_connection(("127.0.0.1", port), timeout=10)
    s.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
    return s


def read_frame(sock):
    """Read one PD01 frame; returns (ch, type, flags, seq, payload) or None on EOF."""
    def recvn(n):
        buf = b""
        while len(buf) < n:
            d = sock.recv(n - len(buf))
            if not d:
                return None
            buf += d
        return buf
    rh = recvn(18)
    if rh is None:
        return None
    assert rh[:4] == MAGIC, f"bad magic {rh[:4]!r}"
    _ver, ch, rtype, flags, rid = struct.unpack(">BBHHI", rh[4:14])
    plen = struct.unpack(">I", rh[14:18])[0]
    data = recvn(plen) if plen else b""
    return (ch, rtype, flags, rid,
            msgpack.unpackb(data, raw=False) if data else {})


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
        elif cmd == "inject-tap":  # inject-tap X Y [backend]
            print(json.dumps(a.inject_tap(args[1], args[2],
                                          backend=args[3] if len(args) > 3 else None), default=str))
        elif cmd == "inject-swipe":  # inject-swipe X1 Y1 X2 Y2 MS [backend]
            print(json.dumps(a.inject_swipe(*map(float, args[1:6]),
                                            backend=args[6] if len(args) > 6 else None), default=str))
        elif cmd == "inject-key":  # inject-key KEYCODE [meta] [backend]
            print(json.dumps(a.inject_key(args[1],
                                          meta=args[2] if len(args) > 2 else 0,
                                          backend=args[3] if len(args) > 3 else None), default=str))
        elif cmd == "swipe-pts":  # swipe-pts X1 Y1 X2 Y2 MS STEPS [backend]
            print(json.dumps(a.inject_swipe_pts(*map(float, args[1:7]),
                                                backend=args[7] if len(args) > 7 else None), default=str))
        elif cmd == "noack-tap":
            a.inject_tap_noack(args[1], args[2])
            # contract: no frame comes back — a read must time out
            a.sock.settimeout(0.5)
            try:
                a._recvn(18)
                print("VIOLATION: got a response to a NOACK request")
            except socket.timeout:
                print("OK: no response frame (fire-and-forget honored)")
        elif cmd == "dump":
            r = a.dump()
            sys.stdout.write(r["payload"].get("xml") or r["payload"].get("data", ""))
        elif cmd == "query":  # query '{"text":"x"}' or '{"id":"...","index":N}'
            print(json.dumps(a.query(json.loads(args[1])), default=str))
        elif cmd == "wait":  # wait '{"text":"x"}' [timeout_ms]
            sel = json.loads(args[1])
            print(json.dumps(a.wait_until(sel, int(args[2]) if len(args) > 2 else 5000), default=str))
        elif cmd == "batch":  # batch '<ops json array>'
            print(json.dumps(a.batch(json.loads(args[1])), default=str))
        elif cmd == "text":
            print(json.dumps(a.text(args[1], clear=len(args) > 2), default=str))
        elif cmd == "shot":  # shot OUT [scale] [quality]
            scale = float(args[2]) if len(args) > 2 else 1.0
            qual = int(args[3]) if len(args) > 3 else 80
            r = a.shot(quality=qual, scale=scale)
            p = r["payload"]
            if "data" not in p:
                # agent answered with an error payload (e.g. FLAG_SECURE
                # window focused — MIUI app-lock/keyguard blocks capture)
                sys.exit(f"shot failed: {p.get('code', '?')} "
                         f"{p.get('message', '')} {p.get('error', '')}".strip())
            open(args[1], "wb").write(p["data"])
            print(f"wrote {len(p['data'])}B {p.get('w')}x{p.get('h')} scale={p.get('scale',1)} -> {args[1]}")
        elif cmd == "shot-stat":  # shot-stat SCALE QUALITY -> timing+bytes JSON
            r = a.shot(quality=int(args[2]), scale=float(args[1]))
            p = r["payload"]
            p["data"] = f"<{len(p['data'])}B>"
            print(json.dumps(p, default=str))
        elif cmd == "stream-start":  # stream-start [fps] [scale] [quality]
            print(json.dumps(a.stream_start(
                fps=args[1] if len(args) > 1 else 5,
                scale=args[2] if len(args) > 2 else 0.5,
                quality=args[3] if len(args) > 3 else 70), default=str))
        elif cmd == "stream-stop":
            print(json.dumps(a.stream_stop(), default=str))
        elif cmd == "stream-recv":  # stream-recv MEDIA_PORT [n] [timeout_s]
            import agent_client as _self  # noqa: F401
            ms = media_connect(int(args[1]))
            want = int(args[2]) if len(args) > 2 else 5
            ms.settimeout(float(args[3]) if len(args) > 3 else 10)
            t0 = time.time(); frames = []
            while len(frames) < want:
                try:
                    f = read_frame(ms)
                except socket.timeout:
                    break
                if f is None:
                    frames.append({"eof": True}); break
                ch, ty, fl, seq, p = f
                p2 = dict(p)
                if isinstance(p2.get("data"), (bytes, bytearray)):
                    p2["jpeg"] = p2["data"][:2] == b"\xff\xd8"
                    p2["bytes"] = len(p2["data"]); del p2["data"]
                p2["seq"], p2["ch"], p2["type"], p2["flags"] = seq, ch, ty, fl
                frames.append(p2)
            ms.close()
            print(json.dumps({"n": len(frames), "wall_s": round(time.time() - t0, 2),
                              "frames": frames}, default=str))
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
