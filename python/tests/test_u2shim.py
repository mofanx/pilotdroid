"""u2shim M21 unit tests — G8 Session object model + G10 settings
semantics, exercised with fake devices (no agent/device needed)."""
import os
import sys
import time
import unittest
import xml.etree.ElementTree as ET

sys.path.insert(0, os.path.join(os.path.dirname(__file__),
                                "..", "..", "compat"))
import u2shim  # noqa: E402


def _bare_device(**attrs):
    """A Device without an agent conn — host-side logic only."""
    d = u2shim.Device.__new__(u2shim.Device)
    for k, v in attrs.items():
        setattr(d, k, v)
    return d


class TestSettingsValidation(unittest.TestCase):
    def setUp(self):
        self.s = u2shim._Settings()

    def test_xpath_debug_requires_bool(self):
        with self.assertRaises(TypeError):
            self.s["xpath_debug"] = 1
        self.s["xpath_debug"] = True
        self.assertTrue(self.s["xpath_debug"])
        self.s["xpath_debug"] = False
        self.assertFalse(self.s["xpath_debug"])

    def test_wait_timeout_number(self):
        self.s["wait_timeout"] = 19.0
        self.assertEqual(self.s["wait_timeout"], 19.0)
        self.s["wait_timeout"] = 10
        self.assertEqual(self.s["wait_timeout"], 10)
        with self.assertRaises(TypeError):
            self.s["wait_timeout"] = "10"
        with self.assertRaises(TypeError):
            self.s["wait_timeout"] = True  # bool is not a number here
        with self.assertRaises(ValueError):
            self.s["wait_timeout"] = -1

    def test_operation_delay_tuple(self):
        self.s["operation_delay"] = (1, 0)
        self.assertEqual(self.s["operation_delay"], (1, 0))
        self.s["operation_delay"] = [0, 2]  # list coerced to tuple
        self.assertEqual(self.s["operation_delay"], (0, 2))
        for bad in (5, "x", (1,), (1, 2, 3), ("a", 0)):
            with self.assertRaises(TypeError):
                self.s["operation_delay"] = bad

    def test_operation_delay_methods_list(self):
        self.s["operation_delay_methods"] = ["press", "click"]
        self.assertEqual(self.s["operation_delay_methods"],
                         ["press", "click"])
        for bad in ("press", [1], 3):
            with self.assertRaises(TypeError):
                self.s["operation_delay_methods"] = bad

    def test_unknown_keys_transparent(self):
        self.s["my_fixture_flag"] = "anything"
        self.assertEqual(self.s["my_fixture_flag"], "anything")

    def test_defaults(self):
        self.assertEqual(self.s["wait_timeout"], 20.0)
        self.assertEqual(self.s["operation_delay"], (0.0, 0.0))
        self.assertIn("click", self.s["operation_delay_methods"])


class TestWaitTimeoutCoupling(unittest.TestCase):
    def test_wait_timeout_property_and_implicitly_wait(self):
        d = _bare_device()
        self.assertEqual(d.wait_timeout, 20.0)
        d.settings["wait_timeout"] = 19.0
        self.assertEqual(d.wait_timeout, 19.0)
        d.implicitly_wait(15)
        self.assertEqual(d.settings["wait_timeout"], 15)

    _XML = ('<hierarchy><node text="hi" bounds="[0,0][10,10]" '
            'index="0"/></hierarchy>')

    def test_wait_uses_settings_wait_timeout(self):
        d = _bare_device()
        d.dump_hierarchy = lambda: self._XML
        d.settings["wait_timeout"] = 0.3
        start = time.time()
        self.assertFalse(d(textContains="absent").wait())
        self.assertLess(time.time() - start, 2.0)

    def test_wait_gone_uses_settings_wait_timeout(self):
        d = _bare_device()
        d.dump_hierarchy = lambda: self._XML
        d.settings["wait_timeout"] = 0.3
        start = time.time()
        # textContains forces the host-filter path (bare device has no
        # agent conn for the wire QUERY path); node stays → False.
        self.assertFalse(d(textContains="hi").wait_gone())
        self.assertLess(time.time() - start, 2.0)


class TestOperationDelay(unittest.TestCase):
    def test_delay_before_and_after(self):
        d = _bare_device()
        d.settings["operation_delay"] = (0.05, 0.05)
        d.settings["operation_delay_methods"] = ["click"]
        start = time.time()
        d._op_delay("click", "before")
        d._op_delay("click", "after")
        self.assertGreaterEqual(time.time() - start, 0.09)

    def test_method_not_in_methods_list_skips(self):
        d = _bare_device()
        d.settings["operation_delay"] = (0.5, 0.5)
        d.settings["operation_delay_methods"] = ["press"]
        start = time.time()
        d._op_delay("click", "before")
        d._op_delay("click", "after")
        self.assertLess(time.time() - start, 0.2)


class _FakeDevice:
    """Duck-typed device for Session (no agent needed)."""

    def __init__(self, pkg="io.test"):
        self.pkg = pkg
        self.cur = {"package": pkg, "pid": 1}
        self.started, self.stopped = [], []

    def app_start(self, pkg, **kw):
        self.started.append(pkg)
        self.cur = {"package": pkg, "pid": 2}
        return True

    def app_wait(self, pkg, timeout=20.0, front=False):
        return self.cur["pid"]

    def app_current(self):
        return self.cur

    def app_stop(self, pkg):
        self.stopped.append(pkg)
        self.cur = {"package": "com.other", "pid": 9}


class TestSession(unittest.TestCase):
    def test_running_and_session_broken(self):
        dev = _FakeDevice()
        sess = u2shim.Session(dev, "io.test")
        self.assertTrue(sess.running())
        dev.app_stop("io.test")
        self.assertFalse(sess.running())
        with self.assertRaises(u2shim.SessionBrokenError):
            sess(text="x")

    def test_context_exit_stops_app(self):
        dev = _FakeDevice()
        with dev and u2shim.Session(dev, "io.test") as sess:
            self.assertTrue(sess.running())
        self.assertEqual(dev.stopped, ["io.test"])
        self.assertFalse(sess.running())

    def test_attach_does_not_launch(self):
        dev = _FakeDevice()
        sess = u2shim.Session(dev, "io.test", attach=True)
        self.assertEqual(dev.started, [])
        self.assertTrue(sess.running())

    def test_delegates_to_device(self):
        dev = _FakeDevice()
        sess = u2shim.Session(dev, "io.test")
        self.assertIs(sess.app_current.__func__, dev.app_current.__func__)


class TestStubs(unittest.TestCase):
    def test_jsonrpc_passthrough(self):
        d = _bare_device()
        d.a = object()
        self.assertIs(d.jsonrpc, d.a)

    def test_xpath_stub_errors_honestly(self):
        # xpath-lite era: d.xpath(x) returns a lite selector — honesty
        # moved to eval-time NIE for axes outside the subset.
        d = _bare_device()
        lite = d.xpath("anything")
        self.assertIsInstance(lite, u2shim._XPathLite)
        d.dump_hierarchy = lambda: "<hierarchy/>"
        with self.assertRaises(NotImplementedError):
            d.xpath("//x[position()=1]").exists

    def test_xpath_stub_hasattr_not_blown(self):
        # d.xpath is now a bound method — hasattr() probing on it stays
        # False for unknown names (was the _XPathStub.__getattr__ fix).
        d = _bare_device()
        self.assertFalse(hasattr(d.xpath, "parent"))
        self.assertTrue(hasattr(d.xpath("App"), "parent"))


class TestSerialEnv(unittest.TestCase):
    """Device() falls back to PILOTDROID_SERIAL; explicit arg wins —
    same contract as the Rust CLI's env fallback."""

    def _mk(self, serial=None):
        class _A:
            def hello(self):
                return {"payload": {"caps": 0}}

        return u2shim.Device(port=1, serial=serial, agent=_A())

    def test_env_fallback(self):
        old = os.environ.get("PILOTDROID_SERIAL")
        os.environ["PILOTDROID_SERIAL"] = "envdev"
        try:
            self.assertEqual(self._mk().serial, "envdev")
        finally:
            if old is None:
                del os.environ["PILOTDROID_SERIAL"]
            else:
                os.environ["PILOTDROID_SERIAL"] = old

    def test_explicit_serial_wins(self):
        os.environ["PILOTDROID_SERIAL"] = "envdev"
        try:
            self.assertEqual(self._mk(serial="explicit").serial,
                             "explicit")
        finally:
            del os.environ["PILOTDROID_SERIAL"]


class TestPidofFallback(unittest.TestCase):
    """app_wait must not die when `pidof` returns nothing — custom
    android:process names (pkg:remote) are found via the ps scan."""

    def test_remote_process_name(self):
        d = _bare_device()
        outputs = {"pidof io.test": "",
                   "ps -A -o PID,NAME": "PID NAME\n777 io.test:remote\n"}
        d.shell = lambda c: u2shim.ShellResponse(
            outputs.get(c, ""), 0)
        d.app_current = lambda: {"package": "com.other"}
        self.assertEqual(d.app_wait("io.test", timeout=1, front=False),
                         777)


class TestWatcherError(unittest.TestCase):
    def test_run_async_failure_observable(self):
        # media_connect blowing up must surface via last_error +
        # running()==False, not a silently dead thread.
        import socket as _s
        d = _bare_device()
        d._media_forward = lambda: ("x", 0)
        d.caps = 0x400
        w = u2shim._Watcher(d)
        def boom(_):
            raise _s.timeout("media refused")
        old = u2shim.media_connect
        u2shim.media_connect = boom
        try:
            w._running = True
            w._run_async()
        finally:
            u2shim.media_connect = old
        self.assertIsInstance(w.last_error, _s.timeout)
        self.assertFalse(w._running)


class TestXPathLite(unittest.TestCase):
    """d.xpath(x) — host-side approximation (one dump per query).
    Fake device: dump_hierarchy returns a fixed a11y tree."""

    _XML = (
        '<hierarchy>'
        '<node resource-id="android:id/list" class="android.widget.ListView"'
        ' scrollable="true" bounds="[0,100][1000,2000]" text="">'
        '<node text="App" bounds="[0,100][1000,300]" index="0"/>'
        '<node text="Alarm" bounds="[0,300][1000,500]" index="1"/>'
        '<node text="Accessibility" bounds="[0,500][1000,700]" index="2"/>'
        '</node>'
        '<node resource-id="io.test:id/field" text="hello"'
        ' bounds="[0,2100][500,2200]" index="0"/>'
        '</hierarchy>')

    def _d(self, xml=None):
        d = _bare_device()
        d.dump_hierarchy = lambda: xml or self._XML
        return d

    # -- shorthand translation / eval ------------------------------------
    def test_shorthand_text(self):
        self.assertEqual(len(u2shim._xp_eval(
            ET.fromstring(self._XML), "App")), 1)

    def test_shorthand_fuzzy_contains(self):
        root = ET.fromstring(self._XML)
        self.assertEqual(len(u2shim._xp_eval(root, "%ccessibility")), 1)
        self.assertEqual(len(u2shim._xp_eval(root, "Accessibilit%")), 1)
        self.assertEqual(len(u2shim._xp_eval(root, "%cessib%")), 1)

    def test_shorthand_like_three_states(self):
        # u2 SQL-LIKE: v%=prefix, %v=suffix, %v%=contains — NOT a
        # blanket-contains collapse (was the M24 drift fix).
        root = ET.fromstring(
            '<hierarchy><node text="App"/><node text="Apple"/>'
            '<node text="Pineapple"/></hierarchy>')
        self.assertEqual(len(u2shim._xp_eval(root, "App%")), 2)
        self.assertEqual(len(u2shim._xp_eval(root, "%pple")), 2)
        self.assertEqual(len(u2shim._xp_eval(root, "%pp%")), 3)
        self.assertEqual(len(u2shim._xp_eval(root, "%App")), 1)
        self.assertEqual(len(u2shim._xp_eval(root, "%")), 0)

    def test_xp_match_like_states(self):
        el = ET.fromstring('<node text="Pineapple"/>')
        self.assertTrue(u2shim._xp_match(el, "%apple"))
        self.assertTrue(u2shim._xp_match(el, "Pine%"))
        self.assertTrue(u2shim._xp_match(el, "%eappl%"))
        self.assertFalse(u2shim._xp_match(el, "%pine"))
        self.assertFalse(u2shim._xp_match(el, "apple%"))

    def test_shorthand_at_id_and_bare_rid(self):
        root = ET.fromstring(self._XML)
        self.assertEqual(len(u2shim._xp_eval(root, "@android:id/list")), 1)
        self.assertEqual(
            len(u2shim._xp_eval(root, "io.test:id/field")), 1)

    def test_native_elementpath_and_contains(self):
        root = ET.fromstring(self._XML)
        self.assertEqual(len(u2shim._xp_eval(root, "//*[@text='App']")), 1)
        self.assertEqual(
            len(u2shim._xp_eval(root, "//*[@text='Nope']")), 0)
        self.assertEqual(len(u2shim._xp_eval(
            root, "//*[contains(@text,'Alarm')]")), 1)

    def test_unsupported_axis_raises_nie(self):
        root = ET.fromstring(self._XML)
        for bad in ["//x[position()=1]", "//x[contains(@a,'b') and @c]",
                    "//parent::node"]:
            with self.assertRaises(NotImplementedError, msg=bad):
                u2shim._xp_eval(root, bad)

    # -- _XPathLite API ---------------------------------------------------
    def test_exists_get_text_all(self):
        d = self._d()
        self.assertTrue(d.xpath("App").exists)
        self.assertFalse(d.xpath("Nope").exists)
        self.assertEqual(d.xpath("App").get_text(), "App")
        self.assertEqual(len(d.xpath("//*[@text]").all()), 5)
        self.assertEqual(len(d.xpath("//*[@text='App']").all()), 1)

    def test_wait_hit_and_miss(self):
        d = self._d()
        d.settings["wait_timeout"] = 0.3
        self.assertTrue(d.xpath("App").wait())
        self.assertFalse(d.xpath("Nope").wait())

    def test_click_by_bounds_center(self):
        d = self._d()
        hits = []
        d.click = lambda x, y: hits.append((x, y)) or True
        d.xpath("App").click()
        self.assertEqual(hits, [(500, 200)])  # bounds [0,100][1000,300]

    def test_info_shape(self):
        d = self._d()
        info = d.xpath("io.test:id/field").info
        self.assertEqual(info["resourceId"], "io.test:id/field")
        self.assertEqual(info["text"], "hello")
        self.assertEqual(info["bounds"]["right"], 500)

    def test_parent_matching_ancestor(self):
        d = self._d()
        p = d.xpath("App").parent("@android:id/list")
        self.assertIsNotNone(p)
        self.assertEqual(p.info["resourceId"], "android:id/list")
        # unmatched ancestor arg → climbs to root → None
        self.assertIsNone(d.xpath("App").parent("@no/such"))

    def test_scroll_to_finds_target_after_swipe(self):
        d = self._d()
        xmls = [self._XML,
                '<hierarchy><node text="Target"'
                ' bounds="[0,0][1,1]"/></hierarchy>']
        d.dump_hierarchy = lambda: xmls.pop(0) if xmls else xmls[-1]
        d.swipe = lambda *a, **kw: True
        el = d.xpath("@android:id/list").scroll_to("Target")
        self.assertIsNotNone(el)
        self.assertEqual(el.get_text(), "Target")

    def test_get_missing_raises_u2_not_found(self):
        d = self._d()
        d.settings["wait_timeout"] = 0.1
        with self.assertRaises(u2shim.UiObjectNotFoundError):
            d.xpath("Nope").get()

    def test_xpath_method_attr_probing(self):
        # bound-method surface: hasattr probing still False for unknown
        d = self._d()
        self.assertFalse(hasattr(d.xpath, "nosuch"))
        self.assertTrue(hasattr(d.xpath("x"), "parent"))

    # -- watcher xpath shorthand -------------------------------------------
    def test_watcher_when_xpath_to_wire(self):
        w = u2shim._Watcher.__new__(u2shim._Watcher)
        w._rules = []
        w.when("//*[@text='OK']").click()
        self.assertEqual(w._rules[-1][0]["text"], "OK")
        w.when("@io.test:id/field").click()
        self.assertEqual(w._rules[-1][0]["resourceId"],
                         "io.test:id/field")
        with self.assertRaises(NotImplementedError):
            w.when("//*[contains(@text,'x')]")


class TestImplicitlyWaitDual(unittest.TestCase):
    """d.implicitly_wait() — no arg reads, with arg sets (M24)."""

    def test_getter_setter(self):
        d = _bare_device()
        cur = d.implicitly_wait()
        self.assertEqual(cur, d.settings["wait_timeout"])
        d.implicitly_wait(7.5)
        self.assertEqual(d.implicitly_wait(), 7.5)
        self.assertEqual(d.wait_timeout, 7.5)


class TestSelectorIteration(unittest.TestCase):
    """`len(d(sel))` / `for el in d(sel)` / `d(sel)[i]` — dump+post-
    filter enumeration (u2 select_iter surface)."""

    _XML = ('<hierarchy>'
            '<node text="Row" bounds="[0,0][100,50]" index="0"/>'
            '<node text="Row" bounds="[0,50][100,100]" index="1"/>'
            '<node text="Other" bounds="[0,100][100,150]" index="2"/>'
            '</hierarchy>')

    def _d(self):
        d = _bare_device()
        d.dump_hierarchy = lambda: self._XML
        return d

    def test_len_and_iter(self):
        d = self._d()
        sel = d(textContains="Ro")
        self.assertEqual(len(sel), 2)
        self.assertEqual([n["text"] for n in sel], ["Row", "Row"])

    def test_getitem_nth(self):
        d = self._d()
        # textContains stays on the dump+post-filter path — no agent
        rows = d(textContains="Row")
        second = rows[1]
        self.assertIsInstance(second, u2shim._Selector)
        self.assertEqual(second.kw["instance"], 1)
        self.assertEqual(second.info["bounds"],
                         [0, 50, 100, 100])
        self.assertIsNone(rows[5].info)
        with self.assertRaises(TypeError):
            rows["x"]


class TestBuiltinDialogs(unittest.TestCase):
    """watch_context(builtin=True) — _BUILTIN_DIALOGS pre-armed before
    the watcher starts (no thread in the test)."""

    def test_rules_registered(self):
        d = _bare_device()
        w = u2shim._Watcher(d)
        w.start = lambda: None
        w.stop = lambda: None
        d._watcher = w
        with d.watch_context(builtin=True):
            kws = [r[0] for r in w._rules]
            self.assertIn({"packageName": "com.miui.securitycenter",
                           "text": "继续安装"}, kws)
            self.assertIn({"packageName": "com.lbe.security.miui",
                           "text": "确定"}, kws)
            self.assertTrue(
                any("resourceIdMatches" in k for k in kws))
        # builtin=False: nothing pre-armed
        d2 = _bare_device()
        w2 = u2shim._Watcher(d2)
        w2.start = lambda: None
        w2.stop = lambda: None
        d2._watcher = w2
        with d2.watch_context(builtin=False):
            self.assertEqual(w2._rules, [])


if __name__ == "__main__":
    unittest.main()
