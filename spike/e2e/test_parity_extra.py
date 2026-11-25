#!/usr/bin/env python3
"""test_parity_extra — real-device parity assertions for the four faces
the upstream u2 suite marks skipped (implicitly_wait / select_iter /
count / watchers) but the shim implements. Non-verbatim: lives outside
`spike/e2e/u2suite/suite/` so it is not bound by the verbatim-file rule.

Run: `PILOTDROID_SERIAL=<serial> python3 spike/e2e/test_parity_extra.py`
or collect under pytest (skips cleanly on a device-less host).
"""
import os
import re
import sys
import time

sys.path.insert(0, os.path.join(os.path.dirname(__file__),
                                "..", "..", "compat"))
import u2shim  # noqa: E402

try:
    import pytest
except ImportError:
    pytest = None

_DEV = None


def _dev():
    global _DEV
    if _DEV is None:
        try:
            d = u2shim.Device()
            d.dump_hierarchy()  # proves conn + agent round-trip
            _DEV = d
        except Exception:
            _DEV = False
    return _DEV or None


def _require():
    d = _dev()
    if not d:
        if pytest:
            pytest.skip("no device (set PILOTDROID_SERIAL / check adb)")
        print("SKIP: no device reachable")
        sys.exit(0)
    return d


def test_implicitly_wait_roundtrip():
    """Upstream `test_implicitly_wait` (Need fix skip) — getter/setter
    dual-state round-trip on the live settings dict."""
    d = _require()
    old = d.implicitly_wait()
    d.implicitly_wait(2.0)
    try:
        assert d.implicitly_wait() == 2.0
        assert d.wait_timeout == 2.0
    finally:
        d.implicitly_wait(old)
    assert d.implicitly_wait() == old


def test_selector_iter_enumerates():
    """Upstream `test_select_iter` (TODO skip) — __iter__ lazily yields
    matched info dicts; every screen has a FrameLayout node."""
    d = _require()
    infos = list(d(className="android.widget.FrameLayout"))
    assert infos, "selector __iter__ yielded nothing"
    assert all("bounds" in i for i in infos)


def test_count_len_consistent():
    """Upstream `test_count` (TODO skip) — len(sel) == sel.count ==
    real match count via dump+post-filter."""
    d = _require()
    sel = d(className="android.widget.FrameLayout")
    assert len(sel) == sel.count >= 1


def test_watcher_when_text_call_hit():
    """Upstream `test_watchers` (Need upgrade skip) — when(text).call
    fires the callback once and disarms (real media channel)."""
    d = _require()
    m = re.search(r'text="([^"]{2,})"', d.dump_hierarchy())
    if not m:
        if pytest:
            pytest.skip("screen has no text nodes")
        print("SKIP: no text nodes on screen")
        return
    hits = []
    w = d.watcher
    w._rules.clear()
    w.when(m.group(1)).call(lambda: hits.append(1))
    w.start()
    try:
        deadline = time.time() + 8
        while w._rules and time.time() < deadline:
            time.sleep(0.3)
        assert hits == [1], f"watcher hits={hits} last_error={w.last_error}"
    finally:
        w.stop()


def main():
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for fn in fns:
        fn()
        print(f"PASS {fn.__name__}")
    print(f"parity-extra: {len(fns)} passed")


if __name__ == "__main__":
    main()
