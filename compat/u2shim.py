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
.exists / .wait / .info / .set_text / .get_text / .long_click /
d(scrollable=True).scroll.(forward|backward|toEnd|toBeginning).
M18 additions (RESULTS-M17 G-gap closure): app_start front-wait +
stop=True, click implicit wait, swipe host-side duration sleep, push /
pull / wlan_ip / app_list_running / app_auto_grant_permissions /
watch_context, uiautomator2.exceptions submodule (SessionBrokenError /
UiObjectNotFoundError).
M21 additions (G8/G10 gap closure): Session object model — session(pkg,
attach) returns a scoped Session (running() foreground poll,
SessionBrokenError on ops after the app dies, `with` exit stops the
app; crash-watching is a lazy poll, NOT u2's resident watcher —
RESULTS-M17 §5-G8); settings full semantics — known-key type
validation (xpath_debug must be bool etc.), wait_timeout couples into
wait()/wait_gone()/click implicit wait via d.wait_timeout +
implicitly_wait(), operation_delay=(before,after) seconds applies to
methods in operation_delay_methods (click/swipe/press by default).
`d.xpath` is a host-side approximation (xpath-lite — one dump per
query, shorthand+ElementPath subset, advanced axes NIE).
"""
import os, re, socket, subprocess, sys, threading, time, types
import xml.etree.ElementTree as ET

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

# u2's implicit wait budget for click()/exists-style resolves is
# settings['wait_timeout'] (20s upstream default — G10 coupling).
# The agent control socket times out at 10s per CALL; wait() polls in
# <=1s slices so longer budgets stay safe.
_IMPLICIT_WAIT = 8.0  # fallback when a Device has no settings store yet


class SessionBrokenError(Exception):
    """u2 `uiautomator2.exceptions.SessionBrokenError` — a session's
    target app died mid-use. Raised by Session-scoped ops once
    `running()` is False (M21-G8)."""


# shim's no-node signal IS u2's UiObjectNotFoundError role — alias so
# `except u2.UiObjectNotFoundError` catches the same exception.
UiObjectNotFoundError = LookupError

# `from uiautomator2.exceptions import X` — shim is a module, not a
# package; register a synthetic submodule so drop-in imports work.
# setdefault never clobbers a real uiautomator2 install.
_exc_mod = types.ModuleType("uiautomator2.exceptions")
_exc_mod.SessionBrokenError = SessionBrokenError
_exc_mod.UiObjectNotFoundError = UiObjectNotFoundError
sys.modules.setdefault("uiautomator2.exceptions", _exc_mod)
sys.modules.setdefault(__name__ + ".exceptions", _exc_mod)


def _rid_eq(rid, v):
    return rid == v or rid.endswith(":id/" + v)


def _cls_eq(cls, v):
    return cls == v or cls.rsplit(".", 1)[-1] == v


# bool selector kwargs → a11y XML attribute names
_BOOL_KEYS = {"clickable": "clickable", "checkable": "checkable",
              "checked": "checked", "enabled": "enabled",
              "focusable": "focusable", "focused": "focused",
              "selected": "selected", "longClickable": "long-clickable",
              "scrollable": "scrollable"}

# host-side predicate per selector key (dump+post-filter path, M12)
_PRED = {
    "text": lambda i, v: i["text"] == v,
    "textContains": lambda i, v: v in i["text"],
    "textStartsWith": lambda i, v: i["text"].startswith(v),
    "textMatches": lambda i, v:
        re.fullmatch(v, i["text"] or "") is not None,
    "resourceId": lambda i, v: _rid_eq(i["id"], v),
    "resourceIdMatches": lambda i, v:
        re.fullmatch(v, i["id"] or "") is not None,
    "description": lambda i, v: i["desc"] == v,
    "descriptionContains": lambda i, v: v in i["desc"],
    "descriptionStartsWith": lambda i, v: i["desc"].startswith(v),
    "descriptionMatches": lambda i, v:
        re.fullmatch(v, i["desc"] or "") is not None,
    "className": lambda i, v: _cls_eq(i["class"], v),
    "classNameMatches": lambda i, v:
        re.fullmatch(v, i["class"] or "") is not None,
    "packageName": lambda i, v: i["pkg"] == v,
    "packageNameMatches": lambda i, v:
        re.fullmatch(v, i["pkg"] or "") is not None,
    # u2 `index` = child position among siblings (the a11y `index` attr);
    # `instance` (Nth-match) is positional — applied in _node, not here.
    "index": lambda i, v: i["index"] == int(v),
}
_PRED.update({k: (lambda a: lambda i, v:
                  i["attrs"].get(a, False) == bool(v))(a)
             for k, a in _BOOL_KEYS.items()})

# keys the wire QUERY op can evaluate device-side (exact AND). `index` is
# deliberately NOT here — u2's `index` is the node's child-position attr
# (host-filter predicate below); the wire `index` field means Nth-match,
# which is what u2 calls `instance` — we map instance→wire index.
_WIRE_KEYS = ("text", "resourceId", "description", "className",
              "packageName", "scrollable", "instance")

# spatial directions → (node-bound edge, anchor-bound edge, must hold)
_REL = {"left": lambda nb, ab: nb[2] <= ab[0],
        "right": lambda nb, ab: nb[0] >= ab[2],
        "up": lambda nb, ab: nb[3] <= ab[1],
        "down": lambda nb, ab: nb[1] >= ab[3]}


def _info(el):
    b = [int(x) for x in re.findall(r"-?\d+", el.get("bounds", ""))[:4]]
    a = el.attrib
    return {"bounds": b if len(b) == 4 else [0, 0, 0, 0],
            "text": a.get("text", ""), "id": a.get("resource-id", ""),
            "desc": a.get("content-desc", ""), "class": a.get("class", ""),
            "pkg": a.get("package", ""),
            "index": int(a.get("index", 0)),
            "attrs": {k: a.get(v) == "true"
                      for k, v in _BOOL_KEYS.items()}}


_SETTINGS_DEFAULTS = {
    "wait_timeout": 20.0,          # u2 default; couples into wait paths
    "operation_delay": (0.0, 0.0),  # (before, after) seconds per op
    "operation_delay_methods": ["click", "swipe", "press"],
    "xpath_debug": False,
    "xpath_debug_click": False,
    "debug": False,
    "httpd_started": False,
    "uiobject_mock": {},
}
_SETTINGS_BOOL_KEYS = frozenset(
    k for k, v in _SETTINGS_DEFAULTS.items() if isinstance(v, bool))
_SETTINGS_NUM_KEYS = {"wait_timeout", "implicit_wait"}


class _Settings(dict):
    """u2 `d.settings` — dict with u2's known-key TYPE validation
    (M21-G10). `xpath_debug = 1` raises TypeError (bool required);
    `wait_timeout`/`implicit_wait` take numbers (negative → ValueError);
    `operation_delay` takes a 2-number tuple/list;
    `operation_delay_methods` takes a list of method names. Unknown keys
    stay transparent (shim fixtures use the store freely)."""

    def __init__(self):
        super().__init__(_SETTINGS_DEFAULTS)

    def __setitem__(self, key, value):
        if key in _SETTINGS_BOOL_KEYS:
            if type(value) is not bool:
                raise TypeError(
                    f"settings['{key}'] requires bool, got {type(value).__name__}")
        elif key in _SETTINGS_NUM_KEYS:
            if type(value) is bool or not isinstance(value, (int, float)):
                raise TypeError(
                    f"settings['{key}'] requires a number, got {type(value).__name__}")
            if value < 0:
                raise ValueError(f"settings['{key}'] must be >= 0")
        elif key == "operation_delay":
            if (not isinstance(value, (tuple, list)) or len(value) != 2
                    or any(type(v) is bool
                           or not isinstance(v, (int, float))
                           for v in value)):
                raise TypeError(
                    "settings['operation_delay'] requires a "
                    "(before, after) tuple of numbers")
            value = tuple(value)
        elif key == "operation_delay_methods":
            if (not isinstance(value, (list, tuple))
                    or any(not isinstance(v, str) for v in value)):
                raise TypeError(
                    "settings['operation_delay_methods'] requires a "
                    "list of method names")
            value = list(value)
        super().__setitem__(key, value)


class _Selector:
    """Lazy selector bound to a device — resolves via QUERY on use, or
    via dump+post-filter when a key the wire can't express is present
    (contains/matches/attrs/spatial — M12 host-filter batch)."""

    def __init__(self, dev, _base=None, _rel=None, **kw):
        self.dev, self.kw = dev, kw
        self._base, self._rel = _base, _rel  # relative selector (left/…)

    def _wireable(self):
        return self._rel is None and all(k in _WIRE_KEYS for k in self.kw)

    def _sel(self):
        """Wire QUERY selector — only exact-match keys. Host-only keys are
        a hard error here (dropping them would widen the match!)."""
        bad = [k for k in self.kw if k not in _WIRE_KEYS]
        if bad:
            raise ValueError(
                f"u2shim selector: keys {bad} need host post-filter "
                f"(not wire-expressible); use d.xpath() instead")
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
        if "instance" in self.kw:
            s["index"] = int(self.kw["instance"])  # wire index = Nth-match
        return s

    def _kw_match(self, info):
        bad = [k for k in self.kw
               if k not in _PRED and k != "instance"]
        if bad:
            raise ValueError(
                f"u2shim selector: unsupported keys {bad} "
                f"(use d.xpath() for xpath queries)")
        return all(_PRED[k](info, v)
                   for k, v in self.kw.items()
                   if k != "instance")

    def _index(self):
        """u2 `instance` = Nth-match positional pick (0-based)."""
        return int(self.kw.get("instance", 0))

    def _resolve(self, pairs, parent):
        """(el, info) hit pairs against an already-parsed tree. Recursive —
        the relative-selector anchor is the BASE'S OWN resolution, so
        chained `d(A).child(B).left(C)` resolves A→B→C correctly."""
        if self._rel is None:
            return [p for p in pairs if self._kw_match(p[1])]
        base_hits = self._base._resolve(pairs, parent)
        bi = self._base._index()  # base's own `instance` positional pick
        if not 0 <= bi < len(base_hits):
            return []
        ael, ab = base_hits[bi][0], base_hits[bi][1]["bounds"]
        if self._rel == "child":
            desc = {id(e) for e in ael.iter()} - {id(ael)}
            return [p for p in pairs
                    if id(p[0]) in desc and self._kw_match(p[1])]
        if self._rel == "sibling":
            par = parent.get(ael)
            sib = ({id(e) for e in par} - {id(ael)}
                   if par is not None else set())
            return [p for p in pairs
                    if id(p[0]) in sib and self._kw_match(p[1])]
        dir_ok = _REL[self._rel]
        return [p for p in pairs if self._kw_match(p[1])
                and dir_ok(p[1]["bounds"], ab)]

    def _nodes(self):
        """All matching nodes (one dump). Spatial/child/sibling resolve
        here too — they need the full tree anyway."""
        xml = self.dev.dump_hierarchy()
        root = ET.fromstring(xml)
        parent = {}
        for el in root.iter("node"):
            for ch in el:
                parent[ch] = el
        pairs = [(el, _info(el)) for el in root.iter("node")]
        return [p[1] for p in self._resolve(pairs, parent)]

    def _node(self):
        if self._wireable():
            r = self.dev.a.query(self._sel())["payload"]
            return r.get("node") if r.get("found") else None
        ns = self._nodes()
        i = self._index()
        return ns[i] if 0 <= i < len(ns) else None

    @property
    def exists(self):
        return self._node() is not None

    @property
    def count(self):
        """u2 `d(...).count` — real match count (dump+post-filter)."""
        return len(self._nodes())

    def __len__(self):
        return self.count

    def __iter__(self):
        """u2 `for el in d(...)` — iterate match info dicts (one dump)."""
        return iter(self._nodes())

    def __getitem__(self, idx):
        """u2 `d(...)[i]` — nth match (0-based) as a new Selector."""
        if not isinstance(idx, int):
            raise TypeError(idx)
        kw = dict(self.kw)
        kw["instance"] = idx
        return _Selector(self.dev, _base=self._base, _rel=self._rel,
                         **kw)

    @property
    def info(self):
        return self._node()

    def wait(self, timeout=None):
        """u2 semantics: True when the selector appears within timeout,
        False otherwise. timeout=None → settings['wait_timeout']
        (u2 implicit-wait coupling, M21-G10). The wireable path polls
        wait_until in <=1s slices — a single long wait_until would hog
        the shared control conn and starve concurrent ops (watcher
        arming; u2suite's test_watch_context deadlocks on exactly this)."""
        if timeout is None:
            timeout = self.dev.wait_timeout
        deadline = time.time() + timeout
        if self._wireable():
            while True:
                slice_ms = int(min(1000,
                                   max(50, (deadline - time.time())
                                       * 1000)))
                r = self.dev.a.wait_until(self._sel(), slice_ms)
                if r["payload"]["results"][0]["code"] == "OK":
                    return True
                if time.time() >= deadline:
                    return False
        while time.time() < deadline:
            if self._node() is not None:
                return True
            time.sleep(0.1)
        return False

    def wait_gone(self, timeout=None):
        """u2 `d(...).wait_gone(t)` — True once no node matches.
        timeout=None → settings['wait_timeout']."""
        if timeout is None:
            timeout = self.dev.wait_timeout
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self._node() is None:
                return True
            time.sleep(0.1)
        return False

    def _rel_sel(self, rel, other, kw):
        if isinstance(other, _Selector):
            if kw:
                raise ValueError("pass either a Selector or kwargs")
            kw = other.kw
        elif other is not None:
            kw.setdefault("text", other)
        return _Selector(self.dev, _base=self, _rel=rel, **kw)

    def left(self, other=None, **kw):
        return self._rel_sel("left", other, kw)

    def right(self, other=None, **kw):
        return self._rel_sel("right", other, kw)

    def up(self, other=None, **kw):
        return self._rel_sel("up", other, kw)

    def down(self, other=None, **kw):
        return self._rel_sel("down", other, kw)

    def child(self, other=None, **kw):
        return self._rel_sel("child", other, kw)

    def sibling(self, other=None, **kw):
        return self._rel_sel("sibling", other, kw)

    def click(self, timeout=None):
        """u2 `d(...).click(timeout)` — u2 resolves the object with an
        implicit wait = settings['wait_timeout'] (20s upstream default,
        M21-G10 coupling); M17-G2: the shim previously resolved
        immediately and raced activity transitions. timeout=0 opts out
        (resolve now)."""
        if timeout is None:
            timeout = getattr(self.dev, "wait_timeout", _IMPLICIT_WAIT)
        if timeout:
            ok = self.wait(timeout)
            node = self._node() if ok else None
        else:
            node = self._node()
        if node is None:
            raise LookupError(f"selector {self.kw} found no node")
        l, t, r_, b = node["bounds"]
        return self.dev.click((l + r_) / 2, (t + b) / 2)

    def get_text(self):
        """u2 `d(...).get_text()` — the matched node's text."""
        node = self._node()
        if node is None:
            raise LookupError(f"selector {self.kw} found no node")
        return node.get("text")

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

    def center(self):
        """u2 `d(...).center()` -> (x, y) of the matched node's center."""
        node = self._node()
        if node is None:
            raise LookupError(f"selector {self.kw} found no node")
        l, t, r, b = node["bounds"]
        return ((l + r) // 2, (t + b) // 2)

    def swipe(self, direction, scale=0.8):
        """u2 `d(...).swipe('left'|'right'|'up'|'down')` — swipe inside the
        matched node's bounds (finger moves `direction` over `scale` of it)."""
        node = self._node()
        if node is None:
            raise LookupError(f"selector {self.kw} found no node")
        l, t, r, b = node["bounds"]
        dx, dy = {"left": (-1, 0), "right": (1, 0),
                  "up": (0, -1), "down": (0, 1)}[str(direction).lower()]
        cx, cy = (l + r) / 2, (t + b) / 2
        w, h = r - l, b - t
        return self.dev.swipe(cx - dx * w * scale / 2,
                              cy - dy * h * scale / 2,
                              cx + dx * w * scale / 2,
                              cy + dy * h * scale / 2)

    def gesture(self, start1, start2, end1, end2, steps=100):
        """u2 `d(...).gesture(p1, p2, p3, p4)` — two-pointer gesture scoped
        to the element (pinch/drag class): ptrA start1->end1, ptrB
        start2->end2 (CAP_MULTI_PTR required)."""
        if self._node() is None:
            raise LookupError(f"selector {self.kw} found no node")
        n = max(1, int(steps))
        pts = [{"action": 0, "pid": 0, "x": float(start1[0]),
                "y": float(start1[1]), "t_ms": 0},
               {"action": 0, "pid": 1, "x": float(start2[0]),
                "y": float(start2[1]), "t_ms": 0}]
        for i in range(1, n + 1):
            f = i / n
            pts += [{"action": 1 if i == n else 2, "pid": 0,
                     "x": start1[0] + (end1[0] - start1[0]) * f,
                     "y": start1[1] + (end1[1] - start1[1]) * f,
                     "t_ms": i * 10},
                    {"action": 1 if i == n else 2, "pid": 1,
                     "x": start2[0] + (end2[0] - start2[0]) * f,
                     "y": start2[1] + (end2[1] - start2[1]) * f,
                     "t_ms": i * 10}]
        return self.dev.a.inject_multi(pts)["payload"].get("ok", False)

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
        if r <= 0 or l >= w or b <= 0 or t >= h:
            raise LookupError(
                f"node bounds {node['bounds']} fully outside display {w}x{h}")
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

    def click(self, max_fires=None):
        if "_xpath" in self.kw:
            # host-poll rule: the `_xpath` sentinel is not a selector
            # key — passing it to _Selector raises ValueError inside
            # the watcher loop (swallowed → rule silently dead,
            # click never ran — shipped-hole fix M26). Eval the xpath
            # at fire time and tap the first match's center; an empty
            # re-eval (dialog dismissed mid-tick) is a silent no-op so
            # a transient miss can't kill the rule.
            xp, dev = self.kw["_xpath"], self._w.dev

            def action():
                els = _XPathLite(dev, xp).all()
                if els:
                    els[0].click()
        else:
            def action():
                _Selector(self._w.dev, **self.kw).click()
            action._pd_click = True  # armed device-side: don't re-click
        if max_fires is not None:
            if max_fires < 1:
                raise ValueError("max_fires must be >= 1")
            action._pd_max_fires = max_fires
        self._w._rules.append((self.kw, action))
        return self._w

    def call(self, func, max_fires=None):
        """u2 `watcher.when(sel).call(fn)` — run arbitrary callback on
        match (test_xpath::test_watcher uses this). `max_fires=N` makes
        the rule re-arm up to N hits (default fire-once)."""
        if max_fires is not None:
            if max_fires < 1:
                raise ValueError("max_fires must be >= 1")
            func._pd_max_fires = max_fires
        self._w._rules.append((self.kw, func))
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
        self._running = False   # u2 watcher runs until .stop(), even
                                # with zero rules (u2suite caught the
                                # run-exits-on-empty race)
        self.last_error = None  # fatal watcher-thread failure (e.g.
                                # media_connect refused) — observable
                                # instead of a silently-dead thread

    def when(self, *args, **kw):
        # u2 allows positional text / resourceId strings. xpath-shaped
        # args translate to wire keys when 1:1-expressible; anything
        # else the xpath-lite subset can still evaluate degrades to a
        # host-poll rule (`_xpath` key) — only constructs _xp_eval
        # rejects stay NIE (probed fail-fast on an empty doc).
        for a in args:
            if isinstance(a, str) and a.startswith("@"):
                kw["resourceId"] = a[1:]
            elif isinstance(a, str) and a.startswith("//"):
                m = _XP_EQ.match(a)
                key = {"text": "text", "resource-id": "resourceId",
                       "content-desc": "description", "class": "className",
                       "package": "packageName"}.get(
                    m.group(2)) if m else None
                if m and key is not None:
                    kw[key] = m.group(3)
                else:
                    _xp_eval(ET.fromstring("<hierarchy/>"), a)
                    kw["_xpath"] = a
            else:
                kw["text" if "text" not in kw else "description"] = a
        return _WhenCtx(self, kw)

    @staticmethod
    def _fire_done(action):
        """Bounded re-arm bookkeeping: a rule is removed only after it
        has fired `_pd_max_fires` times (default 1 — u2 fire-once
        parity; builtin recipes raise the cap for multi-page dialogs)."""
        n = getattr(action, "_pd_fired", 0) + 1
        action._pd_fired = n
        return n >= getattr(action, "_pd_max_fires", 1)

    def _host_match(self, kw):
        """Match check for rules the wire can't express — the `_xpath`
        sentinel evals via _XPathLite.exists over one dump (host-poll
        semantics); leftover keys AND through _Selector."""
        xp = kw.get("_xpath")
        rest = {k: v for k, v in kw.items() if k != "_xpath"}
        ok = True if xp is None else _XPathLite(self.dev, xp).exists
        return ok and (not rest or _Selector(self.dev, **rest).exists)

    def _tick(self):
        # device mode (CAP_WATCH): the poll loop runs inside the agent —
        # detection latency = one tree-walk, not a host round-trip.
        device_mode = bool(getattr(self.dev, "caps", 0) & 0x100)
        fired = []
        for i, (kw, action) in enumerate(self._rules):
            try:
                if "_xpath" in kw:
                    # host-poll rule — one dump per tick; never
                    # wireable so the armed/device path is skipped
                    if self._host_match(kw):
                        action()
                        if self._fire_done(action):
                            fired.append(i)
                    continue
                s = _Selector(self.dev, **kw)
                if device_mode and s._wireable():
                    dev_act = "click" \
                        if getattr(action, "_pd_click", False) else "none"
                    r = self.dev.a.watch(
                        s._sel(), dev_act,
                        timeout_ms=int(self.interval * 1000),
                        poll_ms=50)["payload"]
                    if r.get("matched"):
                        if dev_act == "none":
                            try:
                                action()
                            except Exception:
                                pass
                        if self._fire_done(action):
                            fired.append(i)
                elif s.exists:
                    action()
                    if self._fire_done(action):
                        fired.append(i)
            except Exception:
                fired.append(i)  # a broken rule must not kill the loop
        for i in reversed(fired):
            self._rules.pop(i)
        return not self._rules

    def run(self):
        while self._running:
            self._tick()
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
            while self._running:
                for rule in list(self._rules):
                    if rule not in armed.values():
                        if "_xpath" in rule[0]:
                            # xpath rules can never arm — host-poll
                            # one dump per loop pass
                            if self._host_match(rule[0]):
                                try:
                                    rule[1]()
                                except Exception:
                                    pass
                                if self._fire_done(rule[1]):
                                    self._rules.remove(rule)
                                    break
                            continue
                        s = _Selector(self.dev, **rule[0])
                        if s._wireable():
                            dev_act = "click" if getattr(
                                rule[1], "_pd_click", False) else "none"
                            r = self.dev.a.watch_arm(
                                s._sel(), dev_act,
                                poll_ms=50)["payload"]
                            armed[r["watch_id"]] = rule
                        elif s.exists:
                            # host-only keys (contains/matches/attrs)
                            # can't arm device-side — poll host-side
                            try:
                                rule[1]()
                            except Exception:
                                pass
                            if self._fire_done(rule[1]):
                                self._rules.remove(rule)
                                break
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
                    if not getattr(rule[1], "_pd_click", False):
                        try:
                            rule[1]()  # .call(fn): agent was armed
                        except Exception:  # observe-only ("none")
                            pass
                    if self._fire_done(rule[1]):
                        for i, r_ in enumerate(list(self._rules)):
                            if r_ is rule:
                                self._rules.pop(i)
                                break
                    # budget left → rule stays in _rules and out of
                    # armed → next pass re-arms it (bounded re-arm)
        except Exception as e:
            # media_connect refused / frame decode died — surface it via
            # last_error + running()==False instead of a silent thread.
            self.last_error = e
            self._running = False
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
        self._running = True
        self._th = threading.Thread(target=target, daemon=True)
        self._th.start()

    def stop(self):
        self._running = False
        self._rules.clear()
        if self._th:
            self._th.join(timeout=self.interval + 1)

    def reset(self):
        self.stop()

    @property
    def running(self):
        return bool(self._th and self._th.is_alive())


# `watch_context(builtin=True)` stock-dialog parity — ROM-specific
# rules the M22/M23 fixture forensics actually hit (u2's own builtin
# set is AOSP-dialog shaped; ours cover the MIUI dialogs that broke
# real-device runs). Selectors AND-match package + label so a stray
# "确定" elsewhere can't fire.
_BUILTIN_DIALOGS = (
    # (selector, action, max_fires) — fire-once default; the third
    # element raises the bound for dialogs that are really multi-page
    # flows.
    # MIUI AdbInstallActivity — "正在通过USB安装此应用" auto-denies
    # after a 10s countdown if nobody taps 继续安装 (RESULTS-M17 §附).
    ({"packageName": "com.miui.securitycenter",
      "text": "继续安装"}, "click", 1),
    # MIUI LBE first-run permission review — multi-PAGE 确定 ack flow
    # (fixture-doctor loops pages for the same reason): max_fires=3
    # keeps the rule armed past the first page, still bounded so a
    # permanently-matching dialog can't click forever.
    ({"packageName": "com.lbe.security.miui",
      "text": "确定"}, "click", 3),
    # Expanded NotificationShade masking the a11y tree (M22 two-layer
    # env failure) — collapse, don't click. resourceIdMatches covers
    # stock/MIUI expanded-shade ids; ids a ROM doesn't use simply keep
    # the rule dormant.
    ({"resourceIdMatches":
      r"com\.android\.systemui:id/(notification_panel|"
      r"expanded_status_bar|quick_settings.*|notification_container.*)"},
     ("shell", "cmd statusbar collapse"), 1),
)


def _mk_builtin_click(dev, kw):
    """Host-side inject click for a builtin dialog rule. NOT the
    device-armed a11y ACTION_CLICK path — that is ignored by
    FLAG_SECURE dialogs (proved on MIUI AdbInstallActivity: WATCH_HIT
    consumed the rule, the 10s countdown still ran to auto-deny).
    Also settle+retry: WATCH_HIT can arrive mid slide-in animation
    where a single early tap lands before the window accepts input."""
    sel = _Selector(dev, **kw)

    def _act():  # `_` prefix: not a public method per doc-drift scan
        for _ in range(4):
            time.sleep(0.7)
            try:
                if not sel.exists:
                    return
                sel.click(timeout=0)
            except Exception:
                return
    return _act


class _WatchContext:
    """Backs `Device.watch_context` — thin ctx-manager over _Watcher.
    builtin=True pre-registers _BUILTIN_DIALOGS before user rules —
    u2 builtin parity in spirit (stock-dialog auto-dismiss), with a
    ruleset shaped by what this ROM actually throws."""

    def __init__(self, dev, builtin):
        self.dev, self.builtin = dev, builtin

    def __enter__(self):
        self._w = self.dev.watcher
        if self.builtin:
            for kw, act, max_fires in _BUILTIN_DIALOGS:
                if act == "click":
                    fn = _mk_builtin_click(self.dev, kw)
                else:
                    cmd = act[1]
                    fn = lambda c=cmd: self.dev.shell(c)
                self._w.when(**kw).call(fn, max_fires=max_fires)
        self._w.start()
        return self

    def when(self, *args, **kw):
        return self._w.when(*args, **kw)

    def __exit__(self, *exc):
        self._w.stop()
        return False


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
        serial = serial or os.environ.get("PILOTDROID_SERIAL") or None
        if port is None:
            serial, port = _detect_port(serial)  # backfills serial
        self.a = agent or Agent(port).connect()
        self._port = port
        self.serial = serial
        self.caps = self.a.hello()["payload"].get("caps", 0)

    def __call__(self, **kw):
        return _Selector(self, **kw)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    # u2 surface
    def click(self, x, y):
        self._op_delay("click", "before")
        ok = self.a.inject_tap(x, y)["payload"]["ok"]
        self._op_delay("click", "after")
        return ok

    def long_click(self, x, y, duration=0.5):
        ms = int(duration * 1000)
        return self.a.call(0x0010, {   # T_INJECT
            "kind": "Tap", "wait_finish": False,
            "points": [{"action": 0, "x": float(x), "y": float(y), "t_ms": 0},
                       {"action": 1, "x": float(x), "y": float(y),
                        "t_ms": ms}]})["payload"]["ok"]

    def swipe(self, x1, y1, x2, y2, duration=0.3, steps=10):
        self._op_delay("swipe", "before")
        ok = self.a.inject_swipe(x1, y1, x2, y2,
                                 int(duration * 1000),
                                 steps)["payload"]["ok"]
        # M17-G4 approx: the agent executes the gesture device-side
        # asynchronously (M10 design decision — not changed); u2's
        # client-side contract BLOCKS for `duration`. Host-side sleep
        # approximates the client-visible contract (wall time >= dur).
        time.sleep(duration)
        self._op_delay("swipe", "after")
        return ok

    def press(self, key):
        self._op_delay("press", "before")
        code = _KEYCODES.get(str(key).lower(), None)
        if code is None:
            code = int(key)
        ok = self.a.inject_key(code)["payload"]["ok"]
        self._op_delay("press", "after")
        return ok

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

    def app_start(self, pkg, activity=None, stop=False, wait_timeout=10.0):
        """u2 `d.app_start(pkg, stop=...)`. M17-G1: u2's contract is
        'launched AND frontmost' — this now blocks on
        `app_wait(front=True)` after the launch and returns False if the
        app did not reach the foreground (e.g. a MIUI
        ReviewPermissionsActivity gate is front instead — see
        RESULTS-M17 §4-E2)."""
        if stop:
            self.app_stop(pkg)
        comp = f"{pkg}/{activity}" if activity else pkg
        cmd = ["adb", "shell", "monkey", "-p", pkg, "-c",
               "android.intent.category.LAUNCHER", "1"] if activity is None \
            else ["adb", "shell", "am", "start", "-n", comp]
        if self._sh(cmd) != 0:
            return False
        return self.app_wait(pkg, timeout=wait_timeout, front=True) \
            is not None

    def app_stop(self, pkg):
        return self._sh(["adb", "shell", "am", "force-stop", pkg]) == 0

    def app_wait(self, pkg, timeout=20.0, front=False):
        """u2 `d.app_wait`: poll until pkg runs (front=True → foreground).
        Returns pid or None. Foreground check rides the conn APP_INFO path."""
        deadline = time.time() + timeout
        while time.time() < deadline:
            pid = self._pidof(pkg)
            if pid is not None and (not front or
                        self.app_current().get("package") == pkg):
                return pid
            time.sleep(0.5)
        return None

    def _pidof(self, pkg):
        """First pid whose process name is `pkg` or `pkg:<sub>`. `pidof
        <pkg>` alone misses custom `android:process` names and
        `:remote`-only states — u2 consults running-tasks, so we fall
        back to a `ps` scan (same source as `app_list_running`)."""
        out = self.shell(f"pidof {pkg}").output.strip().split()
        if out:
            return int(out[0])
        for line in self.shell("ps -A -o PID,NAME").output.splitlines():
            parts = line.split()
            if len(parts) == 2 and (parts[1] == pkg or
                                    parts[1].startswith(pkg + ":")):
                try:
                    return int(parts[0])
                except ValueError:
                    pass
        return None

    def session(self, pkg, attach=False, launch_timeout=None):
        """u2 `d.session(pkg, attach=False, launch_timeout=None)` — a
        package-scoped Session object (M21-G8): `running()` polls
        foreground, ops raise SessionBrokenError once the app dies,
        `with sess` stops the app on exit. The crash watch is a lazy
        poll at op time — NOT u2's resident server watcher."""
        return Session(self, pkg, attach=attach,
                       launch_timeout=launch_timeout)

    def send_keys(self, text, clear=False):
        """u2 `d.send_keys`: type into the focused field — rides the TEXT
        op (ACTION_SET_TEXT on focused node, no IME round-trip)."""
        return bool(self.a.text(text, clear=clear)["payload"].get("ok"))

    def open_notification(self):
        return self._sh(["adb", "shell", "cmd", "statusbar",
                         "expand-notifications"]) == 0

    def open_quick_settings(self):
        return self._sh(["adb", "shell", "cmd", "statusbar",
                         "expand-settings"]) == 0

    def freeze_rotation(self, freeze=True):
        """u2 `d.freeze_rotation` approximated via the system auto-rotate
        switch (u2's orientation-service lock is not reproduced)."""
        return self._sh(["adb", "shell", "settings", "put", "system",
                         "accelerometer_rotation",
                         "0" if freeze else "1"]) == 0

    def set_orientation(self, orientation):
        """u2 `d.set_orientation('l'|'n'|'r'|'u')` — alias of the
        `orientation` setter (freezes auto-rotate, sets user_rotation)."""
        self.orientation = orientation

    def swipe_ext(self, direction, scale=0.9):
        """u2 `d.swipe_ext('left'|'right'|'up'|'down', scale)` — swipe
        across `scale` fraction of the display, centered."""
        w, h = self.window_size() or (1080, 2400)
        dx, dy = {"left": (-1, 0), "right": (1, 0),
                  "up": (0, -1), "down": (0, 1)}[str(direction).lower()]
        x1 = w / 2 - dx * w * scale / 2
        y1 = h / 2 - dy * h * scale / 2
        return self.swipe(x1, y1, x1 + dx * w * scale,
                          y1 + dy * h * scale, duration=0.3)

    def double_click(self, x, y, duration=0.1):
        """u2 `d.double_click(x, y, duration)` — two taps `duration`s
        apart."""
        self.click(x, y)
        time.sleep(max(0.01, duration))
        return self.click(x, y)

    def drag(self, sx, sy, ex, ey, duration=0.5):
        """u2 `d.drag(sx, sy, ex, ey, duration)` — press-hold then move;
        coords are fractions of the display when <= 1."""
        w, h = self.window_size() or (1080, 2400)
        x1, y1 = sx * w if sx <= 1 else sx, sy * h if sy <= 1 else sy
        x2, y2 = ex * w if ex <= 1 else ex, ey * h if ey <= 1 else ey
        n = 8
        pts = [{"action": 0, "x": float(x1), "y": float(y1), "t_ms": 0},
               {"action": 2, "x": float(x1), "y": float(y1), "t_ms": 300}]
        for i in range(1, n + 1):
            f = i / n
            pts.append({"action": 1 if i == n else 2,
                        "x": x1 + (x2 - x1) * f, "y": y1 + (y2 - y1) * f,
                        "t_ms": 300 + int(duration * 1000 * f)})
        return self.a.inject_multi(pts)["payload"].get("ok", False)

    def service(self, name):
        """u2 `d.service(name)` — the on-device automation service.
        'uiautomator' (u2's canonical service name) maps to the pilotdroid
        agent process — same role, different binary."""
        if name not in ("uiautomator", "pilotdroid", "pilotdroid_agent"):
            raise ValueError(
                f"service('{name}') unsupported — pilotdroid runs a single "
                "device service (the agent); u2's 'uiautomator' maps to it")
        return _Service(self)

    def app_current(self):
        """u2 app_current() -> {package, activity}. Prefers the conn-side
        APP_INFO op (CAP_APP_INFO — UiAutomation window read, ~ms) over a
        dumpsys adb spawn (~300-600ms per call)."""
        if self.caps & 0x400:
            p = self.a.app_info()["payload"]
            if p.get("pkg"):
                info = {"package": p["pkg"],
                        "activity": p.get("activity")}
                pid = self._pidof(info["package"])
                if pid:
                    info["pid"] = pid
                return info
        out = subprocess.run(
            self._sh_cmd(["dumpsys", "window"]),
            capture_output=True, text=True).stdout
        import re
        m = re.search(r"mCurrentFocus=\S+\s+\S+\s+([\w.]+)/([\w.$]+)", out)
        if not m:  # some ROMs only expose mFocusedApp
            m = re.search(r"([\w.]+)/([\w.$]+)\s", out)
        if not m:
            return {}
        info = {"package": m.group(1), "activity": m.group(2)}
        # u2 shape includes `pid` — u2suite test_app_mixin reads it
        pid = self._pidof(info["package"])
        if pid:
            info["pid"] = pid
        return info

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

    @property
    def settings(self):
        """u2 `d.settings` — _Settings dict: known keys are type-checked
        (`xpath_debug` bool, `wait_timeout` number, `operation_delay`
        (before, after) tuple, `operation_delay_methods` list — wrong
        types raise TypeError/ValueError like u2); unknown keys stay
        transparent. `wait_timeout` couples into wait()/wait_gone()/
        click implicit wait; `operation_delay` couples into
        click/swipe/press (M21-G10)."""
        store = getattr(self, "_settings_store", None)
        if store is None:
            store = self._settings_store = _Settings()
        return store

    @property
    def wait_timeout(self):
        """u2 `d.wait_timeout` — current implicit-wait budget (reads
        settings['wait_timeout'])."""
        return self.settings["wait_timeout"]

    def implicitly_wait(self, timeout=None):
        """u2 `d.implicitly_wait()` — no arg returns the current
        settings['wait_timeout']; with arg sets it."""
        if timeout is None:
            return self.settings["wait_timeout"]
        self.settings["wait_timeout"] = timeout

    def _op_delay(self, method, phase):
        """u2 operation_delay=(before, after) seconds, applied only to
        methods in settings['operation_delay_methods']."""
        s = self.settings
        if method not in s.get("operation_delay_methods", ()):
            return
        delay = s.get("operation_delay", (0, 0))
        d = delay[0] if phase == "before" else delay[1]
        if d:
            time.sleep(float(d))

    @property
    def jsonrpc(self):
        """u2 `d.jsonrpc` — raw rpc surface. The shim maps it to the
        agent client (named ops, not u2's full jsonrpc object model —
        RESULTS-M17 §5-G6 still documents that defer)."""
        return self.a

    def xpath(self, x):
        """u2 `d.xpath(x)` — host-side approximation (xpath-lite): one
        dump per query, shorthand (text / %fuzzy% / @id / bare pkg:id) +
        `//*[@attr]`/contains()/parent()/scroll_to. NOT the u2 plugin
        pipeline; advanced axes raise NotImplementedError."""
        return _XPathLite(self, x)

    def push(self, src, dst="/sdcard/"):
        """u2 `d.push(local, remote)` — adb spawn; `src` may be a path,
        bytes, or a file-like object (u2 surface; staged via temp)."""
        if isinstance(src, (bytes, bytearray)) or hasattr(src, "read"):
            data = bytes(src) if isinstance(src, (bytes, bytearray)) \
                else src.read()
            import tempfile
            with tempfile.NamedTemporaryFile(delete=False) as f:
                f.write(data)
                tmp = f.name
            try:
                return self.push(tmp, dst)
            finally:
                os.unlink(tmp)
        cmd = (["adb"] + (["-s", self.serial] if self.serial else [])
               + ["push", src, dst])
        return subprocess.run(cmd, capture_output=True).returncode == 0

    def pull(self, src, dst="."):
        """u2 `d.pull(remote, local)` — adb spawn."""
        cmd = (["adb"] + (["-s", self.serial] if self.serial else [])
               + ["pull", src, dst])
        return subprocess.run(cmd, capture_output=True).returncode == 0

    @property
    def wlan_ip(self):
        """u2 `d.wlan_ip` — device IPv4 on wlan0, None if absent."""
        out = self.shell("ip addr show wlan0").output
        m = re.search(r"inet (\d+\.\d+\.\d+\.\d+)", out)
        return m.group(1) if m else None

    def app_list_running(self):
        """u2 `d.app_list_running()` — installed packages that have a
        live process (`ps -A` names ∩ `pm list packages`; process names
        may carry a `:sub` suffix — stripped)."""
        pkgs = set(self.app_list())
        running = set()
        for line in self.shell("ps -A -o NAME").output.splitlines():
            name = line.strip().split(":")[0]
            if name in pkgs:
                running.add(name)
        return sorted(running)

    def app_auto_grant_permissions(self, pkg):
        """u2 `d.app_auto_grant_permissions(pkg)` — `pm grant` every
        permission the manifest requests; non-runtime perms fail
        harmlessly (pm exits nonzero, ignored)."""
        out = self.shell(f"dumpsys package {pkg}").output
        m = re.search(r"requested permissions:(.*?)"
                      r"(?:install permissions|runtime permissions|"
                      r"pkg=|Packages:)", out, re.S)
        if not m:
            return False
        for p in re.findall(r"android\.permission\.[\w.]+", m.group(1)):
            self.shell(f"pm grant {pkg} {p}")
        return True

    def watch_context(self, builtin=False):
        """u2 `d.watch_context(builtin)` — context manager over
        `d.watcher`: `.when(sel).click()` registers rules; __exit__
        stops the watcher. builtin=True pre-arms the _BUILTIN_DIALOGS
        ruleset (MIUI install-confirm / LBE review / expanded shade —
        approximating u2's stock-dialog builtins, not a copy)."""
        return _WatchContext(self, builtin)

    def close(self):
        self.a.close()


class _SessionSelector(_Selector):
    """Selector created via `sess(...)` — every resolution re-checks the
    session: ops raise SessionBrokenError once the app is gone."""

    def __init__(self, sess, **kw):
        self._sess = sess
        super().__init__(sess._dev, **kw)

    def _node(self):
        self._sess._check()
        return super()._node()

    def _nodes(self):
        self._sess._check()
        return super()._nodes()

    def wait(self, timeout=None):
        self._sess._check()
        return super().wait(timeout)

    def wait_gone(self, timeout=None):
        self._sess._check()
        return super().wait_gone(timeout)


class Session:
    """u2 `d.session(pkg, attach=False, launch_timeout=None)` — a
    package-scoped device view (M21-G8).

    - attach=False launches the app; attach=True binds a running app.
    - `sess(...)` returns a _SessionSelector; any op raises
      SessionBrokenError once the app is no longer foreground —
      checked lazily per op, NOT by a resident watcher like u2's
      server-side session (RESULTS-M17 §5-G8 approximation note).
    - `with sess` exits by force-stopping the app (u2 semantics).
    - Everything else delegates to the device (`sess.click(x,y)`,
      `sess.settings`, …) via __getattr__."""

    def __init__(self, dev, pkg, attach=False, launch_timeout=None):
        self._dev, self._pkg, self._closed = dev, pkg, False
        if not attach:
            dev.app_start(pkg)
        if dev.app_wait(pkg, timeout=launch_timeout or 10.0,
                        front=True) is None:
            raise RuntimeError(f"session: {pkg} did not reach foreground")

    def __call__(self, **kw):
        self._check()
        return _SessionSelector(self, **kw)

    def __getattr__(self, name):
        dev = self.__dict__.get("_dev")
        if dev is None:
            raise AttributeError(name)
        return getattr(dev, name)

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self._closed = True
        self._dev.app_stop(self._pkg)
        return False

    def running(self):
        """u2 `sess.running()` — is the session app still foreground.
        Shim: polls app_current() per call (lazy crash detection —
        a crash surfaces on the next op, not eagerly)."""
        if self._closed:
            return False
        try:
            return self._dev.app_current().get("package") == self._pkg
        except Exception:
            return False

    def close(self):
        """u2 `sess.close()` — stop the app and seal the session."""
        self._closed = True
        self._dev.app_stop(self._pkg)

    def _check(self):
        if not self.running():
            raise SessionBrokenError(
                f"session app {self._pkg} is not running")


# ---------- xpath-lite (host-side approximation, stdlib-only) ----------
# Each query = one dump_hierarchy + ElementTree eval (~50ms on-device).
# NOT the u2 plugin pipeline — supported surface is the u2 shorthand
# set + the ElementPath subset; advanced axes/predicates raise
# NotImplementedError honestly (compat/SUPPORTED.md xpath row).

_XP_CONTAINS = re.compile(
    r"^\.//(\*|[\w-]+)\[contains\(@([\w-]+),\s*['\"]([^'\"]+)['\"]\)\]$")
_XP_EQ = re.compile(
    r"^//(\*|[\w-]+)\[@([\w-]+)=['\"]([^'\"]+)['\"]\]$")
_XP_CONTAINS_ABS = re.compile(
    r"^//(\*|[\w-]+)\[contains\(@([\w-]+),\s*['\"]([^'\"]+)['\"]\)\]$")
_XP_RID = re.compile(r"^[\w.]+:[\w/]+$")


def _xp_eval(root, x):
    """Evaluate a u2 xpath-ish expression against an ET root →
    [Element]. Shorthand: text / %contains% / @resource-id / bare
    pkg:id/name; `//` ElementPath subset + contains(). Anything the
    subset can't express raises NotImplementedError."""
    if x.startswith("%") or x.endswith("%"):
        # u2 SQL-LIKE: %v%=contains, %v=endswith, v%=startswith
        v = x.strip("%")
        if not v:
            return []
        pre, post = x.startswith("%"), x.endswith("%")
        if pre and post:
            ok = lambda t: v in t
        elif pre:
            ok = lambda t: t.endswith(v)
        else:
            ok = lambda t: t.startswith(v)
        return [e for e in root.iter() if ok(e.get("text") or "")]
    if x.startswith("@"):
        x = f".//*[@resource-id='{x[1:]}']"
    elif x.startswith("//"):
        x = "." + x
    elif _XP_RID.match(x):
        x = f".//*[@resource-id='{x}']"
    elif not x.startswith((".", "(")):
        x = f".//*[@text='{x}']"
    m = _XP_CONTAINS.match(x)
    if m:  # ET has no function calls — contains() gets a custom pass
        tag, attr, val = m.groups()
        return [e for e in root.iter(tag) if val in (e.get(attr) or "")]
    try:
        return root.findall(x)
    except SyntaxError as e:
        raise NotImplementedError(
            f"u2shim: xpath-lite subset only — {x!r} unsupported") from e


def _xp_match(el, x):
    """Predicate form of the shorthand — is `el` an `x` node? (used by
    parent() ancestor matching)."""
    if x.startswith("%") or x.endswith("%"):
        # u2 SQL-LIKE (same three states as _xp_eval)
        v = x.strip("%")
        if not v:
            return False
        t = el.get("text") or ""
        if x.startswith("%") and x.endswith("%"):
            return v in t
        return t.endswith(v) if x.startswith("%") else t.startswith(v)
    if x.startswith("@"):
        return el.get("resource-id") == x[1:]
    if _XP_RID.match(x):
        return el.get("resource-id") == x
    if x.startswith("//"):
        for pat in (_XP_EQ, _XP_CONTAINS_ABS):
            m = pat.match(x)
            if m:
                tag, attr, val = m.groups()
                return (tag in ("*", el.tag)
                        and (val in (el.get(attr) or "")
                             if pat is _XP_CONTAINS_ABS
                             else el.get(attr) == val))
        raise NotImplementedError(
            f"u2shim: ancestor match {x!r} unsupported")
    return el.get("text") == x


class _XEl:
    """One resolved xpath node — attrs straight from the a11y dump."""
    _B = re.compile(r"\[(\d+),(\d+)\]\[(\d+),(\d+)\]")

    def __init__(self, dev, el):
        self.dev, self._el = dev, el

    @property
    def bounds(self):
        m = self._B.match(self._el.get("bounds", ""))
        return tuple(int(g) for g in m.groups()) if m else (0, 0, 0, 0)

    def center(self):
        l, t, r, b = self.bounds
        return (l + r) // 2, (t + b) // 2

    @property
    def info(self):
        a = self._el.attrib
        l, t, r, b = self.bounds
        i = dict(a)
        i.update({"text": a.get("text", ""),
                  "resourceId": a.get("resource-id", ""),
                  "className": a.get("class", ""),
                  "packageName": a.get("package", ""),
                  "bounds": {"left": l, "top": t, "right": r,
                             "bottom": b}})
        return i

    def get_text(self):
        return self._el.get("text", "")

    def click(self):
        return self.dev.click(*self.center())

    def swipe(self, direction, scale=0.5):
        """u2 XMLElement.swipe — host approx: directional swipe inside
        the element's bounds (u2 semantics differ slightly; this is
        the documented approximation)."""
        l, t, r, b = self.bounds
        cx, cy = (l + r) // 2, (t + b) // 2
        dx, dy = {"up": (0, -1), "down": (0, 1),
                  "left": (-1, 0), "right": (1, 0)}[direction]
        return self.dev.swipe(cx, cy, cx + int(dx * (r - l) * scale),
                              cy + int(dy * (b - t) * scale), 0.3)


class _XPathLite:
    """`d.xpath(x)` result — chainable host-side selector. Methods
    mirror the u2 XPathSelector surface used by the verbatim suite."""

    def __init__(self, dev, xp):
        self.dev, self._xp = dev, xp

    def _eval(self):
        root = ET.fromstring(self.dev.dump_hierarchy())
        return root, _xp_eval(root, self._xp)

    @property
    def exists(self):
        return bool(self._eval()[1])

    def wait(self, timeout=None):
        if timeout is None:
            timeout = self.dev.wait_timeout
        deadline = time.time() + timeout
        while True:
            if self._eval()[1]:
                return True
            if time.time() >= deadline:
                return False
            time.sleep(min(0.5, max(0.0, deadline - time.time())))

    def all(self):
        return [_XEl(self.dev, e) for e in self._eval()[1]]

    def get(self):
        if not self.wait():
            raise UiObjectNotFoundError(
                f"u2shim: no xpath match {self._xp!r}")
        return self.all()[0]

    def click(self):
        return self.get().click()

    def get_text(self):
        return self.get().get_text()

    @property
    def info(self):
        return self.get().info

    def parent(self, x=None):
        """u2 .parent([x]) — first hit's ancestor (x → nearest matching
        ancestor via shorthand predicate)."""
        root, hits = self._eval()
        if not hits:
            return None
        pmap = {c: p for p in root.iter() for c in p}
        p = pmap.get(hits[0])
        if x is None:
            return _XEl(self.dev, p) if p is not None else None
        while p is not None and not _xp_match(p, x):
            p = pmap.get(p)
        return _XEl(self.dev, p) if p is not None else None

    def scroll_to(self, target, max_swipes=12):
        """u2 .scroll_to(x): swipe inside the scrollable node until
        `target` matches — host loop (dump ~50ms per pass)."""
        no_sc = 0
        for _ in range(max_swipes):
            root, _ = self._eval()
            t = _xp_eval(root, target)
            if t:
                return _XEl(self.dev, t[0])
            sc = next((e for e in root.iter()
                       if e.get("scrollable") == "true"), None)
            if sc is None:
                # mid-navigation dumps can show the unpopulated list
                # (scrollable=false) — tolerate a few before giving up.
                no_sc += 1
                if no_sc >= 4:
                    break
                time.sleep(0.4)
                continue
            no_sc = 0
            # Slow half-viewport drag — a fast full-height swipe flings
            # ~2 viewports and skips the band holding `target` (empirical:
            # ApiDemos Graphics 'Pictures' is missed by 0.8-scale flings).
            l, top, r, b = _XEl(self.dev, sc).bounds
            cx = (l + r) // 2
            self.dev.swipe(cx, top + int((b - top) * 0.8),
                           cx, top + int((b - top) * 0.4),
                           duration=0.7, steps=24)
            # settle: let inertia die before the next dump — a dump mid-
            # fling can catch `target` sliding through view, then the
            # caller's re-eval sees it scrolled past again.
            time.sleep(0.5)
        return None


def _adb_devices():
    """`adb devices` serial list (attached state only); [] on adb errors."""
    try:
        out = subprocess.run(["adb", "devices"], capture_output=True,
                             text=True, timeout=10).stdout
    except Exception:
        return []
    return [ln.split()[0] for ln in out.splitlines()[1:]
            if ln.split()[-1:] == ["device"]]


def _detect_port(serial=None):
    """Auto-detect (serial, port) of the TCP forward to pilotdroid_agent.

    Parses `adb forward --list` for a `localabstract:pilotdroid_agent` rule;
    when `serial` is given the rule must belong to that device. With
    `serial=None` the matched line's serial is BACKFILLED (u2.connect()
    first-device semantics — every other adb call needs `-s` under
    multi-device); several devices forwarding → guiding error. No forward
    rule at all → `adb devices` disambiguates: exactly one attached →
    adopt it; more than one → guiding error. Falls back to the
    conventional 17932 probe with an actionable error if nothing listens.
    """
    try:
        out = subprocess.run(["adb", "forward", "--list"],
                             capture_output=True, text=True, timeout=10).stdout
    except Exception:
        out = ""
    cands = []  # (serial, port)
    for line in out.splitlines():
        parts = line.split()
        if len(parts) == 3 and parts[2] == "localabstract:pilotdroid_agent":
            if serial is None or parts[0] == serial:
                if parts[1].startswith("tcp:"):
                    cands.append((parts[0], int(parts[1][4:])))
    if cands:
        if serial is None:
            serials = {s for s, _ in cands}
            if len(serials) > 1:
                raise RuntimeError(
                    f"u2shim: {len(serials)} devices forward to "
                    f"pilotdroid_agent ({', '.join(sorted(serials))}) — "
                    "pass serial= or set PILOTDROID_SERIAL")
            serial = cands[0][0]
        return serial, cands[0][1]
    if serial is None:
        devs = _adb_devices()
        if len(devs) > 1:
            raise RuntimeError(
                f"u2shim: no pilotdroid_agent forward found and "
                f"{len(devs)} devices attached ({', '.join(devs)}) — "
                "pass serial= or set PILOTDROID_SERIAL, then run "
                "`pilotdroid-cli -s <serial> setup`")
        if devs:
            serial = devs[0]
    else:
        raise RuntimeError(
            f"u2shim: no pilotdroid_agent forward found for serial "
            f"{serial!r} — run `pilotdroid-cli -s {serial} setup` (the "
            "17932 fallback probe is skipped: a live port there may be "
            "forwarded to a different device, leaving conn and `-s` "
            "calls pointed at different targets)")
    for port in (17932,):
        try:
            Agent(port).connect().close()
            return serial, port
        except Exception:
            pass
    raise RuntimeError(
        "no adb forward to localabstract:pilotdroid_agent found and "
        "tcp:17932 is not alive — run `pilotdroid-cli setup` (installed "
        "with `pip install pilotdroid`; without it, download the binary "
        "from github.com/mofanx/pilotdroid/releases), or "
        "`adb forward tcp:17932 localabstract:pilotdroid_agent`")


class _Service:
    """`d.service('uiautomator')` over the agent app_process — u2's
    atx-agent service lifecycle applied to our daemon. Shell-based
    checks work even after the control conn dies on stop()."""

    _PAT = "pilotdroid.Agent"
    _DIR = "/data/local/tmp/pilotdroid"

    def __init__(self, dev):
        self.dev = dev

    def running(self):
        return bool(self.dev.shell(
            f"pgrep -f '{self._PAT}'").output.strip())

    def stop(self):
        self.dev.shell(f"pkill -f '{self._PAT}'")
        return not self.running()

    def start(self):
        self.dev.shell(
            f"nohup setsid env CLASSPATH={self._DIR}/agent.jar "
            f"app_process {self._DIR} pilotdroid.Agent "
            f"</dev/null >{self._DIR}/agent.log 2>&1 &")
        for _ in range(40):
            if self.running():
                break
            time.sleep(0.25)
        try:
            self.dev.a = Agent(self.dev._port).connect()
        except Exception:
            pass
        return self.running()


def connect(port=None, serial=None):
    """u2 `u2.connect(addr)` — the positional may be an adb serial string
    (canonical u2 style: `u2.connect('emulator-5554')`) or a local TCP
    port (pilotdroid-native: int or digit string). `serial=` kwarg and
    no-arg auto-detect unchanged."""
    if isinstance(port, str) and not port.isdigit():
        serial = serial or port
        port = None
    return Device(port=port, serial=serial)
