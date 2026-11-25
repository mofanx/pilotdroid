#!/usr/bin/env python3
"""Blind-agent driver for the eval-v2 real-model round (M32 t3).

Each invocation is one MCP op — describe | tap | verify — and nothing
else is exposed to the driver. `describe` prints a compact SoM list
(index, class, text/desc, centre) exactly as a blind model would see
it; the model picks a centre and calls `tap X Y`.

  python3 blind.py describe
  python3 blind.py tap <x> <y>
  python3 blind.py verify '{"exists":{"text":"..."}}' [settle_ms]
  python3 blind.py verify '{"layout_changed":true}'
"""
import json
import sys

sys.path.insert(0, __import__("os").path.dirname(
    __import__("os").path.abspath(__file__)))
from loop import Serve


def main():
    cmd = sys.argv[1]
    srv = Serve()
    try:
        if cmd == "describe":
            els = srv.tool("describe_screen")["elements"]
            for i, e in enumerate(els):
                lab = e.get("text") or e.get("desc") or ""
                c = e.get("center", "?")
                print(f"[{i}] {e.get('class', '?'):20} {str(c):>13} "
                      f"{lab[:60]}")
        elif cmd == "tap":
            print(srv.tool("tap", {"x": int(sys.argv[2]),
                                   "y": int(sys.argv[3])}))
        elif cmd == "verify":
            expect = json.loads(sys.argv[2])
            ms = int(sys.argv[3]) if len(sys.argv) > 3 else 600
            r = srv.tool("verify_after",
                         {"action": {"tool": "exists", "args": {}},
                          "expect": expect, "settle_ms": ms})
            print(json.dumps(r))
        else:
            sys.exit(f"unknown cmd {cmd}")
    finally:
        srv.close()


if __name__ == "__main__":
    main()
