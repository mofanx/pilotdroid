#!/usr/bin/env python3
"""pilotdroid u2-compat shim v0 (ADR-005: host-side translation layer).

Drop-in-ish subset of uiautomator2's top-level API backed by the PD01
agent — same call surface, persistent session (u2 pays a JSON-RPC HTTP
round-trip per call; shim pays one socket op).

    import u2shim as u2
    d = u2.connect(port=17932)
    d.click(360, 1600); d(text="OK").click(); d.press("back")

Covered (v1): connect / click(x,y) / long_click / swipe / press /
set_text / dump_hierarchy / screenshot / app_start / app_stop /
app_current / info;
d(text|resourceId|description|className|packageName|scrollable,index).click() /
.exists / .wait / .info / .set_text / .long_click /
d(scrollable=True).scroll.(forward|backward|toEnd|toBeginning).
(xpath still rejected with ValueError — see _SUPPORTED.)
"""
import os, subprocess, sys, time

try:
    from .agent_client import Agent  # installed `pilotdroid` package
except ImportError:
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

    _SUPPORTED = ("text", "resourceId", "description", "className",
                  "packageName", "scrollable", "index")

    def _sel(self):
        bad = [k for k in self.kw if k not in self._SUPPORTED]
        if bad:
            raise ValueError(
                f"u2shim selector: unsupported keys {bad}; "
                f"supported: {list(self._SUPPORTED)} "
                f"(xpath not supported)")
        s = {}
        if "text" in self.kw:
            s["text"] = self.kw["text"]
        if "resourceId" in self.kw:
            rid = self.kw["resourceId"]
            s["id"] = rid.split(":id/")[-1]  # agent accepts bare name
        if "description" in self.kw:
            s["desc"] = self.kw["description"]
        if "className" in self.kw:
            s["class"] = self.kw["className"]  # full or simple name
        if "packageName" in self.kw:
            s["pkg"] = self.kw["packageName"]  # exact package match
        if "scrollable" in self.kw:
            s["scrollable"] = bool(self.kw["scrollable"])
        if "index" in self.kw:
            s["index"] = int(self.kw["index"])
        return s

    def _node(self):
        r = self.dev.a.query(self._sel())["payload"]
        return r.get("node") if r.get("found") else None

    @property
    def exists(self):
        return self._node() is not None

    @property
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

    def long_click(self, duration=0.5):
        node = self._node()
        if node is None:
            raise LookupError(f"selector {self.kw} found no node")
        l, t, r_, b = node["bounds"]
        return self.dev.long_click((l + r_) / 2, (t + b) / 2, duration)

    @property
    def scroll(self):
        """u2 `d(scrollable=True).scroll.<dir>` — swipes inside the
        matched scrollable node's bounds. forward/backward = one
        viewport; toEnd/toBeginning = repeat until content stops
        changing (dump-hash stable) or 10 swipes."""
        node = self._node()
        if node is None:
            raise LookupError(f"selector {self.kw} found no node")
        return _Scroll(self.dev, node["bounds"])


class _Scroll:
    def __init__(self, dev, bounds):
        self.dev, self.b = dev, bounds

    def _once(self, forward=True):
        l, t, r, b = self.b
        cx, h = (l + r) / 2, b - t
        y_from = t + h * (0.8 if forward else 0.2)
        y_to = t + h * (0.2 if forward else 0.8)
        return self.dev.swipe(cx, y_from, cx, y_to, duration=0.2)

    def forward(self):
        return self._once(True)

    def backward(self):
        return self._once(False)

    def _to(self, forward, max_swipes=10):
        prev = ""
        for _ in range(max_swipes):
            cur = hash(self.dev.dump_hierarchy())
            if cur == prev:
                return True
            prev = cur
            self._once(forward)
            time.sleep(0.4)
        return False

    def toEnd(self):
        return self._to(True)

    def toBeginning(self):
        return self._to(False)


class Device:
    def __init__(self, port=None, serial=None, agent=None):
        port = port or _detect_port(serial)
        self.a = agent or Agent(port).connect()
        self.serial = serial
        self.a.hello()

    def __call__(self, **kw):
        return _Selector(self, **kw)

    # u2 surface
    def click(self, x, y):
        return self.a.inject_tap(x, y)["payload"]["ok"]

    def long_click(self, x, y, duration=0.5):
        ms = int(duration * 1000)
        return self.a.call(0x0010, {   # T_INJECT
            "kind": "Tap", "wait_finish": False,
            "points": [{"action": 0, "x": float(x), "y": float(y), "t_ms": 0},
                       {"action": 1, "x": float(x), "y": float(y),
                        "t_ms": ms}]})["payload"]["ok"]

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
        return self._sh(cmd) == 0

    def app_stop(self, pkg):
        return self._sh(["adb", "shell", "am", "force-stop", pkg]) == 0

    def app_current(self):
        """u2 app_current() -> {package, activity} via focused window."""
        out = subprocess.run(
            self._sh_cmd(["dumpsys", "window"]),
            capture_output=True, text=True).stdout
        import re
        m = re.search(r"mCurrentFocus=\S+\s+\S+\s+([\w.]+)/([\w.$]+)", out)
        if not m:  # some ROMs only expose mFocusedApp
            m = re.search(r"([\w.]+)/([\w.$]+)\s", out)
        return {"package": m.group(1), "activity": m.group(2)} if m else {}

    def _sh_cmd(self, args):
        return (["adb"] + (["-s", self.serial] if self.serial else [])
                + ["shell"] + args)

    def _sh(self, cmd):
        if self.serial:
            cmd[1:1] = ["-s", self.serial]
        return subprocess.run(cmd, capture_output=True).returncode

    @property
    def info(self):
        h = self.a.hello()["payload"]
        return {"displayWidth": h["display"][0],
                "displayHeight": h["display"][1],
                "sdkInt": h.get("sdk"), "agent": h.get("ver")}

    def close(self):
        self.a.close()


def _detect_port(serial=None):
    """Auto-detect the local TCP port forwarding to pilotdroid_agent.

    Parses `adb forward --list` for a `localabstract:pilotdroid_agent` rule;
    when `serial` is given the rule must belong to that device. Falls back to
    the conventional 17932 with an actionable error if nothing is listening.
    """
    try:
        out = subprocess.run(["adb", "forward", "--list"],
                             capture_output=True, text=True, timeout=10).stdout
    except Exception:
        out = ""
    cands = []
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2] == "localabstract:pilotdroid_agent":
            if serial is None or parts[0] == serial:
                if parts[1].startswith("tcp:"):
                    cands.append(int(parts[1][4:]))
    if cands:
        return cands[0]
    for port in (17932,):
        try:
            Agent(port).connect().close()
            return port
        except Exception:
            pass
    raise RuntimeError(
        "no adb forward to localabstract:pilotdroid_agent found and "
        "tcp:17932 is not alive — run `pilotdroid setup` or "
        "`adb forward tcp:17932 localabstract:pilotdroid_agent`")


def connect(port=None, serial=None):
    return Device(port=port, serial=serial)
