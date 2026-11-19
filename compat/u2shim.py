#!/usr/bin/env python3
"""pilotdroid u2-compat shim v0 (ADR-005: host-side translation layer).

Drop-in-ish subset of uiautomator2's top-level API backed by the PD01
agent — same call surface, persistent session (u2 pays a JSON-RPC HTTP
round-trip per call; shim pays one socket op).

    import u2shim as u2
    d = u2.connect(port=17932)
    d.click(360, 1600); d(text="OK").click(); d.press("back")

Covered (v0): connect / click(x,y) / d(text|resourceId|description,
index).click() / d(...).exists / d(...).wait / dump_hierarchy /
screenshot / set_text / app_start (via `am start`) / press(key) / swipe.
"""
import os, subprocess, sys, time

sys.path.insert(0, os.path.join(os.path.dirname(__file__),
                                "..", "agent", "scripts"))
from agent_client import Agent  # noqa: E402

_KEYCODES = {"home": 3, "back": 4, "recent": 187, "recents": 187,
             "power": 26, "volume_up": 24, "volume_down": 25,
             "menu": 82, "search": 84, "enter": 66, "del": 67}


class _Selector:
    """Lazy selector bound to a device — resolves via QUERY on use."""

    def __init__(self, dev, **kw):
        self.dev, self.kw = dev, kw

    def _sel(self):
        s = {}
        if "text" in self.kw:
            s["text"] = self.kw["text"]
        if "resourceId" in self.kw:
            rid = self.kw["resourceId"]
            s["id"] = rid.split(":id/")[-1]  # agent accepts bare name
        if "description" in self.kw:
            s["desc"] = self.kw["description"]
        if "index" in self.kw:
            s["index"] = int(self.kw["index"])
        return s

    def _node(self):
        r = self.dev.a.query(self._sel())["payload"]
        return r.get("node") if r.get("found") else None

    @property
    def exists(self):
        return self._node() is not None

    def info(self):
        return self._node()

    def wait(self, timeout=10.0):
        t0 = time.time()
        r = self.dev.a.wait_until(self._sel(), int(timeout * 1000))
        res = r["payload"]["results"][0]
        ok = res["code"] == "OK"
        return self._node() if ok else None

    def click(self, timeout=None):
        if timeout:
            node = self.wait(timeout)
        else:
            node = self._node()
        if node is None:
            raise LookupError(f"selector {self.kw} found no node")
        l, t, r_, b = node["bounds"]
        return self.dev.click((l + r_) / 2, (t + b) / 2)

    def set_text(self, s):
        # u2 semantics: click the field first, then set
        node = self._node()
        if node is not None:
            l, t, r_, b = node["bounds"]
            self.dev.click((l + r_) / 2, (t + b) / 2)
        return self.dev.set_text(s)


class Device:
    def __init__(self, port=17932, serial=None, agent=None):
        self.a = agent or Agent(port).connect()
        self.serial = serial
        self.a.hello()

    def __call__(self, **kw):
        return _Selector(self, **kw)

    # u2 surface
    def click(self, x, y):
        return self.a.inject_tap(x, y)["payload"]["ok"]

    def swipe(self, x1, y1, x2, y2, duration=0.3, steps=10):
        return self.a.inject_swipe(x1, y1, x2, y2,
                                   int(duration * 1000), steps)["payload"]["ok"]

    def press(self, key):
        code = _KEYCODES.get(str(key).lower(), None)
        if code is None:
            code = int(key)
        return self.a.inject_key(code)["payload"]["ok"]

    def set_text(self, s):
        p = self.a.text(s)["payload"]
        return p.get("ok", False)

    def dump_hierarchy(self):
        return self.a.dump()["payload"]["xml"]

    def screenshot(self, path=None, scale=1.0, quality=80):
        data = self.a.shot(quality=quality, scale=scale)["payload"]["data"]
        if path:
            with open(path, "wb") as f:
                f.write(data)
        return data

    def app_start(self, pkg, activity=None):
        comp = f"{pkg}/{activity}" if activity else pkg
        cmd = ["adb", "shell", "monkey", "-p", pkg, "-c",
               "android.intent.category.LAUNCHER", "1"] if activity is None \
            else ["adb", "shell", "am", "start", "-n", comp]
        if self.serial:
            cmd[1:1] = ["-s", self.serial]
        return subprocess.run(cmd, capture_output=True).returncode == 0

    @property
    def info(self):
        h = self.a.hello()["payload"]
        return {"displayWidth": h["display"][0],
                "displayHeight": h["display"][1],
                "sdkInt": h.get("sdk"), "agent": h.get("ver")}

    def close(self):
        self.a.close()


def connect(port=17932, serial=None):
    return Device(port=port, serial=serial)
