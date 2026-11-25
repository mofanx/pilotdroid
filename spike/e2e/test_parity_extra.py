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


def test_xpath_full_axes_real_device():
    """Track B — axes/and-or/[n] evaluate on the real ApiDemos dump
    (was NIE pre-upgrade). Main list: 'Accessibility' sits between
    'App' and 'Animation' — ancestor/sibling/position probes + a
    sibling-axis tap that must land on Animation's submenu."""
    d = _require()
    d.app_start("io.appium.android.apis", stop=True)
    deadline = time.time() + 10
    while "Accessibility" not in d.dump_hierarchy() \
            and time.time() < deadline:
        time.sleep(0.5)
    # and/or + contains predicate
    assert d.xpath("//*[@text='Accessibility' and @index='0']"
                   ).exists, "and-predicate missed Accessibility row"
    assert d.xpath("//*[contains(@text,'Access') or "
                   "@text='NoSuchThing']").exists
    # ancestor axis resolves the android:id/list container
    anc = d.xpath("//*[@text='Accessibility']"
                  "/ancestor::*[@resource-id='android:id/list']").all()
    assert anc, "ancestor::*[@resource-id] found no list parent"
    # [n] positional predicate: first row of the main list
    first = d.xpath("//node[@resource-id='android:id/list']"
                    "/node[1]").all()
    assert first and first[0].get_text() == "Accessibility", (
        f"position[1] got {first[0].get_text() if first else None!r}")
    # t12 — `!=` predicate (was silent empty set: 'a!' lexed as name)
    rest = d.xpath("//node[@resource-id='android:id/list']"
                   "/node[@text!='Accessibility']").all()
    rest_txt = [e.get_text() for e in rest]
    assert "Animation" in rest_txt and "Accessibility" not in rest_txt, (
        f"!= predicate wrong set: {rest_txt}")
    # following-sibling axis lands on Animation; tap proves the
    # coordinate path through the new grammar
    sib = d.xpath("//*[@text='Accessibility']"
                  "/following-sibling::node[@text='Animation']").all()
    assert sib, "following-sibling::node[@text='Animation'] missed"
    sib[0].click()
    xml, deadline = d.dump_hierarchy(), time.time() + 5
    while "Bouncing Balls" not in xml and time.time() < deadline:
        time.sleep(0.4)
        xml = d.dump_hierarchy()
    assert "Bouncing Balls" in xml, (
        "sibling-axis click missed Animation submenu")


def test_watcher_xpath_new_grammar_navigates():
    """Track B t9⑤ — `when(xpath)` host-poll path benefits from the
    new grammar automatically: compound and/or xpath rule fires and
    its click lands (same nav proof as the M27 test)."""
    d = _require()
    d.app_start("io.appium.android.apis", stop=True)
    deadline = time.time() + 10
    while "Accessibility" not in d.dump_hierarchy() \
            and time.time() < deadline:
        time.sleep(0.5)
    w = d.watcher
    w._rules.clear()
    w.when("//node[@text='Accessibility' and @index] | "
           "//node[@text='NoSuchX']").click()
    w.start()
    try:
        deadline = time.time() + 12
        while w._rules and time.time() < deadline:
            time.sleep(0.4)
        assert not w._rules, (f"new-grammar xpath rule never fired; "
                              f"last_error={w.last_error}")
        xml, deadline = d.dump_hierarchy(), time.time() + 5
        while "Accessibility Node Querying" not in xml \
                and time.time() < deadline:
            time.sleep(0.4)
            xml = d.dump_hierarchy()
        assert "Accessibility Node Querying" in xml, (
            "no navigation after new-grammar xpath click")
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


def test_som_screenshot_annotated_no_throw():
    """Real screens carry inverted/degenerate bounds (27/97 measured
    on ba0d072d) — annotated must normalize/skip, never ValueError.
    Asserts valid JPEG back and same numbering space as describe."""
    sys.path.insert(0, os.path.join(os.path.dirname(__file__),
                                    "..", "..", "compat"))
    import som
    d = _require()
    try:
        from PIL import Image
    except ImportError:
        if pytest:
            pytest.skip("no Pillow")
        print("SKIP: no Pillow")
        return
    data = som.screenshot_annotated(d)  # would ValueError pre-fix
    img = Image.open(__import__("io").BytesIO(data))
    assert img.format == "JPEG" and img.size[0] > 0
    out = som.describe_screen(d)
    raw_ids = [e["id"] for e in out["elements"]]
    assert raw_ids == list(range(1, len(raw_ids) + 1))


def test_toast_get_message_real_device():
    """t15: agent a11y listener (≥0.7.1) pushes TYPE_NOTIFICATION_STATE
    _CHANGED events; ApiDemos PopupMenu1 fires a REAL toast on item
    click — get_message must return its text."""
    d = _require()
    d.app_start("io.appium.android.apis", activity=".view.PopupMenu1",
                stop=True)
    import threading
    res = {}
    t = threading.Thread(
        target=lambda: res.setdefault(
            "m", d.toast.get_message(timeout=12)))
    t.start()
    time.sleep(0.8)  # let get_message attach the media conn first
    d(text="MAKE A POPUP!").click()
    time.sleep(1.0)
    d(text="Add").click()
    t.join(13)
    assert res.get("m") == "Clicked popup menu item Add", res
    d.press("home")
    time.sleep(1.5)  # settle — a11y root goes transient right after a toast


def test_ime_surfaces_real_device():
    """t15: current_ime/ime_list read the settings+`ime` surface;
    set_ime round-trip restores the original."""
    d = _require()
    cur = d.current_ime()
    ids = d.ime_list()
    assert cur and "/" in cur, f"no current IME: {cur!r}"
    assert cur in ids, f"current {cur} not in installed {ids}"
    if len(ids) > 1:
        other = next(i for i in ids if i != cur)
        try:
            assert d.set_ime(other) == 0
            time.sleep(0.5)
            assert d.current_ime() == other
        finally:
            d.set_ime(cur)


def test_screenrecord_produces_mp4_real_device():
    """t15: file-recording path exists (u2-compatible ctx manager) —
    but on this MIUI API34 ROM the encoder is throttled (~1-3 frames
    per 4s, `screenrecord --verbose` self-reports). Assert a valid
    mp4 lands locally, NOT a usable frame rate — see SUPPORTED.md."""
    d = _require()
    out = "/tmp/pilotdroid_parity_rec.mp4"
    with d.screenrecord(out, time_limit=6):
        time.sleep(4)
    assert os.path.exists(out) and os.path.getsize(out) > 4096, out
    os.unlink(out)


def main():
    fns = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for fn in fns:
        fn()
        print(f"PASS {fn.__name__}")
    print(f"parity-extra: {len(fns)} passed")


if __name__ == "__main__":
    main()
