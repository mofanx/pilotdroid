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


def test_watcher_xpath_click_navigates():
    """M27固化 ledger §附5 的真机路径——when(complex-xpath).click()
    is a host-poll `_xpath` rule whose action re-evaluates _XPathLite
    and inject-taps the first match (2b5a9fe fix). ApiDemos main list
    'Accessibility' -> Accessibility submenu proves the tap landed."""
    d = _require()
    d.app_start("io.appium.android.apis", stop=True)
    deadline = time.time() + 10
    while "Accessibility" not in d.dump_hierarchy() \
            and time.time() < deadline:
        time.sleep(0.5)
    w = d.watcher
    w._rules.clear()
    w.when("//*[contains(@text,'Accessibility')]").click()
    w.start()
    try:
        deadline = time.time() + 12
        while w._rules and time.time() < deadline:
            time.sleep(0.4)
        assert not w._rules, (f"xpath click rule never fired; "
                              f"last_error={w.last_error}")
        xml, deadline = d.dump_hierarchy(), time.time() + 5
        while "Accessibility Node Querying" not in xml \
                and time.time() < deadline:
            time.sleep(0.4)
            xml = d.dump_hierarchy()
        assert "Accessibility Node Querying" in xml, (
            "no navigation after xpath click — first-match tap missed")
    finally:
        w.stop()


def test_som_describe_screen_consistency():
    """Track A SoM-lite — describe_screen element set must equal an
    INDEPENDENT re-filter of the raw dump: same count, same bounds,
    same doc order (ids are the SoM handles agents will tap by)."""
    import xml.etree.ElementTree as ET
    sys.path.insert(0, os.path.join(os.path.dirname(__file__),
                                    "..", "..", "compat"))
    import som
    d = _require()
    out = som.describe_screen(d)
    raw = ET.fromstring(d.dump_hierarchy())
    keep = [n for n in raw.iter("node")
            if any(n.get(a) for a in ("text", "content-desc",
                                      "resource-id"))
            or any(n.get(f) == "true" for f in
                   ("clickable", "scrollable", "checkable",
                    "focusable", "long-clickable"))]
    assert len(out["elements"]) == len(keep) >= 1, (
        f"som={len(out['elements'])} vs raw-filtered={len(keep)}")
    for e, n in zip(out["elements"], keep):
        l, t, r, b = e["bounds"]
        assert f"[{l},{t}][{r},{b}]" == n.get("bounds"), (
            f"id#{e['id']} bounds mismatch vs raw dump")


def main():
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for fn in fns:
        fn()
        print(f"PASS {fn.__name__}")
    print(f"parity-extra: {len(fns)} passed")


if __name__ == "__main__":
    main()
