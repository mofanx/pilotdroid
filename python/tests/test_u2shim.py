"""u2shim M21 unit tests — G8 Session object model + G10 settings
semantics, exercised with fake devices (no agent/device needed)."""
import os
import sys
import time
import unittest

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
        d = _bare_device()
        stub = d.xpath
        with self.assertRaises(NotImplementedError):
            stub("//node")

    def test_xpath_stub_hasattr_not_blown(self):
        # __getattr__ must raise AttributeError so hasattr() probing
        # returns False instead of surfacing NIE.
        d = _bare_device()
        stub = d.xpath("anything") if False else u2shim._XPathStub()
        self.assertFalse(hasattr(stub, "parent"))
        with self.assertRaises(NotImplementedError):
            stub("App")


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


if __name__ == "__main__":
    unittest.main()
