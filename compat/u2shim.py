#!/usr/bin/env python3
"""pilotdroid u2-compat shim v0 (ADR-005: host-side translation layer).

Drop-in-ish subset of uiautomator2's top-level API backed by the PD01
agent — same call surface, persistent session (u2 pays a JSON-RPC HTTP
round-trip per call; shim pays one socket op).

    import u2shim as u2
    d = u2.connect(port=17932)
    d.click(360, 1600); d(text="OK").click(); d.press("back")

Covered: connect / click(x,y) / long_click / swipe / press / set_text /
clear_text / dump_hierarchy / screenshot / shell / app_start / app_stop /
app_current / app_list / app_info / info / window_size / screen_on /
screen_off / unlock / orientation (get+set) / watcher;
d(text|resourceId|description|className|packageName|scrollable,index).click() /
.exists / .wait / .info / .set_text / .long_click /
d(scrollable=True).scroll.(forward|backward|toEnd|toBeginning).
(xpath still rejected with ValueError — see _SUPPORTED.)
"""
import os, re, socket, subprocess, sys, threading, time

try:
    from .agent_client import (Agent, media_connect,  # installed `pilotdroid` package
                               read_frame)
except ImportError:
    sys.path.insert(0, os.path.join(os.path.dirname(__file__),
                                    "..", "agent", "scripts"))
    from agent_client import Agent, media_connect, read_frame  # noqa: E402

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
        """u2 semantics: True when the selector appears within timeout,
        False otherwise."""
        r = self.dev.a.wait_until(self._sel(), int(timeout * 1000))
        return r["payload"]["results"][0]["code"] == "OK"

    def click(self, timeout=None):
        if timeout:
            ok = self.wait(timeout)
            node = self._node() if ok else None
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

    def _pinch(self, outward, percent=100, steps=10, duration=0.2):
        """u2 `d().pinch_in/out(percent, steps)` — two-pointer gesture over
        the matched node's bounds (CAP_MULTI_PTR agent required)."""
        node = self._node()
        if node is None:
            raise LookupError(f"selector {self.kw} found no node")
        l, t, r, b = node["bounds"]
        # MIUI dialogs can report bounds outside the display (e.g.
        # [268,3312] on a 3200px-tall screen) — an unclamped center
        # injects at coordinates nothing can receive.
        w, h = self.dev.a.hello()["payload"]["display"]
        cx = min(max((l + r) / 2, 0), w - 1)
        cy = min(max((t + b) / 2, 0), h - 1)
        half = (r - l) / 2
        edge = half * 0.9 * min(percent, 100) / 100.0
        r0, r1 = (half * 0.9, edge) if not outward else (edge, half * 0.9)
        r0, r1 = min(r0, cx, w - 1 - cx), min(r1, cx, w - 1 - cx)
        if r0 < 2 or r1 < 2:
            raise LookupError(
                f"node bounds {node['bounds']} outside display {w}x{h}")
        dur_ms = int(duration * 1000)
        pts = [{"action": 0, "pid": 0, "x": cx - r0, "y": cy, "t_ms": 0},
               {"action": 0, "pid": 1, "x": cx + r0, "y": cy, "t_ms": 10}]
        for i in range(1, steps + 1):
            f = i / (steps + 1)
            d = r0 + (r1 - r0) * f
            pts += [{"action": 2, "pid": 0, "x": cx - d, "y": cy,
                     "t_ms": 10 + int(dur_ms * f)},
                    {"action": 2, "pid": 1, "x": cx + d, "y": cy,
                     "t_ms": 10 + int(dur_ms * f)}]
        pts += [{"action": 1, "pid": 1, "x": cx + r1, "y": cy,
                 "t_ms": 10 + dur_ms},
                {"action": 1, "pid": 0, "x": cx - r1, "y": cy,
                 "t_ms": 20 + dur_ms}]
        return self.dev.a.inject_multi(pts)["payload"].get("ok", False)

    def pinch_in(self, percent=100, steps=10):
        return self._pinch(False, percent, steps)

    def pinch_out(self, percent=100, steps=10):
        return self._pinch(True, percent, steps)

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


class _WhenCtx:
    """One watcher rule builder: `d.watcher.when(text='x').click()`."""

    def __init__(self, watcher, kw):
        self._w, self.kw = watcher, kw

    def click(self):
        def action():
            _Selector(self._w.dev, **self.kw).click()
        self._w._rules.append((self.kw, action))
        return self._w


class _Watcher:
    """u2 `d.watcher` parity — host-side poll thread (u2 polls inside its
    server; the semantic is the same: when a registered selector appears,
    run its action once). Rules fire once each; the thread exits when all
    have fired or `.stop()` is called."""

    def __init__(self, dev, interval=2.0):
        self.dev, self.interval = dev, interval
        self._rules = []        # [(selector_kw, action)]
        self._th = None

    def when(self, *args, **kw):
        # u2 allows positional text / resourceId strings
        for a in args:
            kw["text" if "text" not in kw else "description"] = a
        return _WhenCtx(self, kw)

    def _tick(self):
        # device mode (CAP_WATCH): the poll loop runs inside the agent —
        # detection latency = one tree-walk, not a host round-trip.
        device_mode = bool(getattr(self.dev, "caps", 0) & 0x100)
        fired = []
        for i, (kw, action) in enumerate(self._rules):
            try:
                if device_mode:
                    sel = _Selector(self.dev, **kw)._sel()
                    r = self.dev.a.watch(
                        sel, "click", timeout_ms=int(self.interval * 1000),
                        poll_ms=50)["payload"]
                    if r.get("matched"):
                        fired.append(i)  # action already ran device-side
                elif _Selector(self.dev, **kw).exists:
                    action()
                    fired.append(i)
            except Exception:
                fired.append(i)  # a broken rule must not kill the loop
        for i in reversed(fired):
            self._rules.pop(i)
        return not self._rules

    def run(self):
        while not self._tick():
            time.sleep(self.interval)

    def _run_async(self):
        """CAP_WATCH_ASYNC path (agent 0.7.0+): arm each rule once, then
        block on the media socket for WATCH_HIT frames — detection runs
        inside the agent and the user's thread keeps the control conn
        (u2 server-thread parity). New rules added while running are
        armed on the next loop pass."""
        armed = {}  # watch_id -> rule tuple
        ms = None
        try:
            ms = media_connect(self.dev._media_forward())
            ms.settimeout(0.5)
            while self._rules:
                for rule in list(self._rules):
                    if rule not in armed.values():
                        sel = _Selector(self.dev, **rule[0])._sel()
                        r = self.dev.a.watch_arm(sel, "click",
                                                 poll_ms=50)["payload"]
                        armed[r["watch_id"]] = rule
                try:
                    fr = read_frame(ms)
                except socket.timeout:
                    continue
                if fr is None:
                    break
                _ch, rtype, _flags, _rid, p = fr
                rule = armed.pop(p.get("watch_id"), None) \
                    if rtype == 0x0002 else None  # WATCH_HIT
                if rule is not None and p.get("matched"):
                    for i, r_ in enumerate(list(self._rules)):
                        if r_ is rule:
                            self._rules.pop(i)
                            break
        finally:
            for wid in armed:
                try:
                    self.dev.a.watch_disarm(wid)
                except Exception:
                    pass
            if ms:
                ms.close()

    def start(self, interval=None):
        if interval:
            self.interval = interval
        if self._th and self._th.is_alive():
            return
        target = self._run_async \
            if getattr(self.dev, "caps", 0) & 0x200 else self.run
        self._th = threading.Thread(target=target, daemon=True)
        self._th.start()

    def stop(self):
        self._rules.clear()
        if self._th:
            self._th.join(timeout=self.interval + 1)

    def reset(self):
        self.stop()

    @property
    def running(self):
        return bool(self._th and self._th.is_alive())


class ShellResponse:
    """u2 `d.shell` return shape (output + exit_code attribute access)."""
    __slots__ = ("output", "exit_code")

    def __init__(self, output, exit_code):
        self.output, self.exit_code = output, exit_code

    def __str__(self):
        return self.output


_ORIENTATION = {0: "natural", 1: "left", 2: "upsidedown", 3: "right"}
_ORIENTATION_SET = {"natural": 0, "n": 0, "left": 1, "l": 1, "90": 1,
                   "upsidedown": 2, "u": 2, "180": 2, "right": 3, "r": 3,
                   "270": 3}


class Device:
    def __init__(self, port=None, serial=None, agent=None):
        port = port or _detect_port(serial)
        self.a = agent or Agent(port).connect()
        self.serial = serial
        self.caps = self.a.hello()["payload"].get("caps", 0)

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

    def clear_text(self):
        """u2 `d.clear_text()` — clear the currently focused field.
        ACTION_SET_TEXT with empty payload achieves the same end state."""
        return self.a.text("", clear=True)["payload"].get("ok", False)

    def shell(self, command):
        """u2 `d.shell('cmd')` -> ShellResponse(output, exit_code)."""
        r = subprocess.run(self._sh_cmd([command]),
                           capture_output=True, text=True, timeout=60)
        return ShellResponse(r.stdout.rstrip("\n"), r.returncode)

    def window_size(self):
        """u2 `d.window_size()` -> (w, h)."""
        out = self.shell("wm size").output
        m = re.search(r"(\d+)x(\d+)", out)
        return (int(m.group(1)), int(m.group(2))) if m else None

    def app_list(self):
        """u2 `d.app_list()` -> [package names]."""
        out = self.shell("pm list packages").output
        return sorted(l.split(":", 1)[1].strip()
                      for l in out.splitlines() if ":" in l)

    def app_info(self, pkg):
        """u2 `d.app_info(pkg)` subset: versionName/versionCode/flags."""
        out = self.shell(f"dumpsys package {pkg}").output
        def grab(key):
            m = re.search(key + r"=([^\s]+)", out)
            return m.group(1) if m else None
        return {"packageName": pkg if "versionCode" in out or
                "versionName" in out else None,
                "versionName": grab("versionName"),
                "versionCode": grab("versionCode")}

    def _wakefulness(self):
        out = self.shell("dumpsys power").output
        m = re.search(r"mWakefulness=(\w+)", out)
        return m.group(1) if m else None

    def screen_on(self):
        if self._wakefulness() != "Awake":
            self.press("power")
            time.sleep(0.5)
        return True

    def screen_off(self):
        if self._wakefulness() == "Awake":
            self.press("power")
            time.sleep(0.5)
        return True

    def unlock(self):
        """u2 `d.unlock()`: wake + swipe up past the lockscreen."""
        self.screen_on()
        w, h = self.window_size() or (1080, 2400)
        return self.swipe(w / 2, h * 0.8, w / 2, h * 0.2, duration=0.15)

    @property
    def orientation(self):
        # user_rotation mirrors the display rotation the setter writes;
        # dumpsys input has no stable SurfaceOrientation field across ROMs.
        out = self.shell("settings get system user_rotation").output.strip()
        if out.isdigit():
            return _ORIENTATION.get(int(out))
        out = self.shell("dumpsys input").output
        m = re.search(r"displayId=0.*?orientation=(\d)", out, re.S)
        return _ORIENTATION.get(int(m.group(1))) if m else None

    @orientation.setter
    def orientation(self, value):
        rot = _ORIENTATION_SET[str(value).lower()]
        self.shell("settings put system accelerometer_rotation 0")
        self.shell(f"settings put system user_rotation {rot}")

    @property
    def watcher(self):
        if not hasattr(self, "_watcher"):
            self._watcher = _Watcher(self)
        return self._watcher

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
        """u2 app_current() -> {package, activity}. Prefers the conn-side
        APP_INFO op (CAP_APP_INFO — UiAutomation window read, ~ms) over a
        dumpsys adb spawn (~300-600ms per call)."""
        if self.caps & 0x400:
            p = self.a.app_info()["payload"]
            if p.get("pkg"):
                return {"package": p["pkg"], "activity": p.get("activity")}
        out = subprocess.run(
            self._sh_cmd(["dumpsys", "window"]),
            capture_output=True, text=True).stdout
        import re
        m = re.search(r"mCurrentFocus=\S+\s+\S+\s+([\w.]+)/([\w.$]+)", out)
        if not m:  # some ROMs only expose mFocusedApp
            m = re.search(r"([\w.]+)/([\w.$]+)\s", out)
        return {"package": m.group(1), "activity": m.group(2)} if m else {}

    def _media_forward(self):
        """adb forward for the ch=1 media socket (watch hits). Local port
        = control port + 1, matching verify-agent.sh convention."""
        port = self.a.port + 1
        cmd = (["adb"] + (["-s", self.serial] if self.serial else [])
               + ["forward", f"tcp:{port}",
                  "localabstract:pilotdroid_agent_media"])
        subprocess.run(cmd, capture_output=True, check=True)
        return port

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
