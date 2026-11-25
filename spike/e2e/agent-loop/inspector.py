#!/usr/bin/env python3
"""AI-inspector equivalent — the minimal observe→pick→act→verify loop an
MCP client (or a human) runs against `pilotdroid-cli serve`.

    python3 inspector.py                     # list numbered elements
    python3 inspector.py --tap 3             # tap element #3, verify
    python3 inspector.py --tap 3 --expect-text "Controls"

The numbering space is `describe_screen`'s document-order dense ID —
the same space `screenshot_annotated` draws on the screenshot, so an
agent can fuse "what the model sees" with "what it can act on" without
coordinate math.

Env: PILOTDROID_SERIAL (default ba0d072d), PILOTDROID_CLI.
"""
import argparse
import json
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from loop import Serve, describe  # noqa: E402


def label(e):
    for k in ("text", "desc", "rid"):
        if e.get(k):
            return str(e[k])
    return e.get("class") or "?"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--tap", type=int, default=None,
                    help="dense element index to tap")
    ap.add_argument("--expect-text", default=None,
                    help="postcondition: this text must exist after the tap")
    args = ap.parse_args()

    srv = Serve()
    try:
        els = describe(srv)
        print(f"{len(els)} elements (dense index → center/label):")
        for e in els:
            mark = "clickable" if e.get("clickable") else "        "
            print(f"  [{e['id']:2d}] {mark} {e['center']} {label(e)[:60]}")
        if args.tap is None:
            print("\nre-run with --tap N to act on an element")
            return

        e = next((e for e in els if e["id"] == args.tap), None)
        if e is None:
            sys.exit(f"no element with dense id {args.tap}")
        x, y = e["center"]
        action = {"tool": "tap", "args": {"x": x, "y": y}}
        expect = {"exists": {"text": args.expect_text}} \
            if args.expect_text else {"layout_changed": True}
        r = srv.tool("verify_after",
                     {"action": action, "expect": expect,
                      "settle_ms": 800})
        print(f"\ntap [{args.tap}] {label(e)[:40]} → "
              f"verify_after: {json.dumps(r, ensure_ascii=False)[:200]}")
        sys.exit(0 if r.get("pass") else 1)
    finally:
        srv.close()


if __name__ == "__main__":
    main()
