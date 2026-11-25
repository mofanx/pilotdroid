#!/usr/bin/env python3
"""agent-loop eval harness v1 (M31) — deterministic scripted-policy loop
driving `pilotdroid-cli serve` stdio MCP:

    describe_screen → pick element by label → act (tap/set_text center)
    → verify_after | custom check → pass/fail + step count

~10 deterministic ApiDemos tasks (nav / toggle / dialog / toast /
text-entry / press-back) + one stream+watch coexistence case that
measures the demux-pause constraint on real hardware.

Run: `PILOTDROID_SERIAL=ba0d072d python3 spike/e2e/agent-loop/loop.py`
Env: PILOTDROID_CLI to override the binary (default: debug build).
"""
import json
import os
import subprocess
import sys
import time

SERIAL = os.environ.get("PILOTDROID_SERIAL", "ba0d072d")
CLI = os.environ.get(
    "PILOTDROID_CLI",
    os.path.expanduser("~/.cargo-target/pilotdroid/debug/pilotdroid-cli"))
APIDEMOS = "io.appium.android.apis"


class Serve:
    """Stdio JSON-RPC client; notifications (no id) are collected."""

    def __init__(self):
        self.p = subprocess.Popen(
            [CLI, "serve", "-s", SERIAL],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL, text=True)
        self.notifications = []
        self._id = 0
        self.call("initialize", {})

    def call(self, method, params, timeout=30):
        self._id += 1
        rid = self._id
        self.p.stdin.write(json.dumps(
            {"jsonrpc": "2.0", "id": rid, "method": method,
             "params": params}) + "\n")
        self.p.stdin.flush()
        deadline = time.time() + timeout
        while time.time() < deadline:
            line = self.p.stdout.readline()
            if not line:
                raise RuntimeError("serve stdout EOF")
            m = json.loads(line)
            if m.get("id") == rid:
                return m
            self.notifications.append(m)  # push notification
        raise TimeoutError(method)

    def tool(self, name, args=None, timeout=30):
        r = self.call("tools/call", {"name": name, "arguments": args or {}},
                      timeout)
        res = r.get("result", {})
        if res.get("isError"):
            raise RuntimeError(f"{name}: {res}")
        txt = res["content"][0]["text"]
        try:
            return json.loads(txt)
        except (ValueError, TypeError):
            return {"raw": txt}

    def close(self):
        try:
            self.p.terminate()
            self.p.wait(5)
        except Exception:
            self.p.kill()


def describe(srv):
    return srv.tool("describe_screen")["elements"]


def find(els, text=None, desc=None, cls=None, contains=False):
    for e in els:
        if text is not None:
            t = e.get("text") or ""
            if (text not in t) if contains else (t != text):
                continue
        if desc is not None and desc not in (e.get("desc") or ""):
            continue
        if cls is not None and cls not in (e.get("class") or ""):
            continue
        return e
    return None


def tap_el(srv, e):
    x, y = e["center"]
    return srv.tool("tap", {"x": x, "y": y})


def find_scrolled(srv, text, contains=False, tries=10):
    """describe→find with bounded forward scroll. a11y exposes rendered
    nodes only; the ApiDemos main list hides items below the fold."""
    for _ in range(tries):
        e = find(describe(srv), text=text, contains=contains)
        if e:
            return e
        try:
            srv.tool("scroll", {"dir": "forward"})
        except RuntimeError:
            pass
        time.sleep(0.4)
    return None


def tap_scrolled(srv, label, tries=10):
    """find_scrolled + tap. Raises if still off-screen."""
    e = find_scrolled(srv, label, tries=tries)
    if not e:
        raise RuntimeError(f"element not on screen: {label!r}")
    tap_el(srv, e)
    time.sleep(0.9)
    return e


def tap_text(srv, label, settle=0.9):
    """describe → find text → tap center. Fails if absent."""
    e = find(describe(srv), text=label)
    if not e:
        raise RuntimeError(f"element not on screen: {label!r}")
    tap_el(srv, e)
    time.sleep(settle)
    return e


def verify(srv, action, expect, settle_ms=600):
    r = srv.tool("verify_after", {"action": action, "expect": expect,
                                  "settle_ms": settle_ms})
    return r.get("pass", False), r


def fresh_start(srv, activity=None):
    """Deterministic start: force-stop then launch the launcher."""
    srv.tool("app_stop", {"pkg": APIDEMOS})
    args = {"pkg": APIDEMOS}
    if activity:
        args["activity"] = activity
    srv.tool("app_start", args)
    time.sleep(1.5)


def open_views(srv):
    fresh_start(srv)
    tap_scrolled(srv, "Views")


def open_controls_light(srv):
    open_views(srv)
    tap_text(srv, "Controls")
    tap_text(srv, "1. Light Theme")


def checked(srv, label):
    """dump → find node `label` → its checked attr."""
    xml = srv.tool("dump")["xml"]
    i = xml.find(f'text="{label}"')
    seg = xml[max(0, i - 600):i + 600]
    return 'checked="true"' in seg


# ---------------- tasks ----------------

def t_nav_views(srv):
    fresh_start(srv)
    e = find(describe(srv), text="Views")
    ok, r = verify(srv, {"tool": "tap",
                         "args": {"x": e["center"][0], "y": e["center"][1]}},
                   {"exists": {"text": "Controls"}})
    return ok


def t_nav_controls_light(srv):
    open_views(srv)
    tap_text(srv, "Controls")
    ok, _ = verify(srv,
                   {"tool": "exists", "args": {"text": "1. Light Theme"}},
                   {"text": "1. Light Theme"})
    tap_text(srv, "1. Light Theme")
    return ok and find(describe(srv), text="Checkbox 1") is not None


def t_checkbox_toggle(srv):
    open_controls_light(srv)
    tap_text(srv, "Checkbox 1", settle=0.7)
    return checked(srv, "Checkbox 1")


def t_radio_select(srv):
    open_controls_light(srv)
    tap_text(srv, "RadioButton 2", settle=0.7)
    return checked(srv, "RadioButton 2")


def t_set_text(srv):
    open_controls_light(srv)
    e = find(describe(srv), cls="EditText")
    if not e:
        raise RuntimeError("no EditText on screen")
    tap_el(srv, e)
    time.sleep(0.5)
    ok, r = verify(srv, {"tool": "set_text",
                         "args": {"text": "pd_eval_probe"}},
                   {"text": "pd_eval_probe"})
    return ok


def t_popup_toast(srv):
    """Toast event on the media channel after menu click (t2 demux)."""
    fresh_start(srv, ".view.PopupMenu1")
    # attach the media conn FIRST — ui_event pushes to the live conn;
    # a toast fired before attach is dropped agent-side.
    srv.tool("watch_events", {"timeout_ms": 500})
    tap_text(srv, "MAKE A POPUP!", settle=1.0)
    tap_text(srv, "Add", settle=0.5)
    ev = srv.tool("watch_events", {"timeout_ms": 6000})
    t = ev.get("toast") or {}
    return (ev.get("type") == "toast"
            and "Clicked popup menu item Add" in " ".join(
                t.get("text") or []))


def t_alert_dialog(srv):
    fresh_start(srv)  # Alert Dialogs lives under App on the MAIN list
    tap_text(srv, "App")
    tap_text(srv, "Alert Dialogs")
    tap_text(srv, "OK CANCEL DIALOG WITH A MESSAGE", settle=0.8)
    if not find(describe(srv), text="Lorem ipsum", contains=True):
        return False
    tap_text(srv, "CANCEL", settle=0.8)
    return find(describe(srv), text="Lorem ipsum", contains=True) is None


def t_spinner_pick(srv):
    open_controls_light(srv)
    tap_text(srv, "Mercury", settle=0.9)  # default planet spinner label
    if not find(describe(srv), text="Earth"):
        return False
    tap_text(srv, "Earth", settle=0.8)
    return find(describe(srv), text="Earth") is not None


def t_press_back(srv):
    open_controls_light(srv)
    # first KEYCODE_BACK is consumed inside the Light activity on this
    # ROM (title bar focus) — two presses reach the Controls list.
    srv.tool("press", {"keycode": 4})
    time.sleep(0.8)
    srv.tool("press", {"keycode": 4})
    time.sleep(1.0)
    return find(describe(srv), text="1. Light Theme") is not None


def t_stream_watch_coexist(srv):
    """Measured verdict for the demux-pause constraint:
    arm a push watcher on a selector not yet on screen, START a frame
    stream, trigger the hit mid-stream, then assert:
      (a) NO notifications/pilotdroid/watch_hit arrived while streaming
      (b) frames still flow (stream_read works)
      (c) after stream_stop the hit is pullable via watch_events
    """
    srv.notifications.clear()
    fresh_start(srv, ".view.PopupMenu1")
    arm = srv.tool("watch_arm", {"text": "Add", "push": True})
    wid = arm.get("watch_id", 0)
    srv.tool("stream_start", {"fps": 4, "scale": 0.5})
    time.sleep(0.5)
    frames = srv.tool("stream_read", {"n": 3})
    # trigger the hit while the stream owns the media channel, then
    # pump one more read — recv_event demuxes the hit/toast out of the
    # frame flow into the session stash (demux-pause degrade).
    tap_text(srv, "MAKE A POPUP!", settle=1.2)
    srv.tool("stream_read", {"n": 2})
    pushed = [n for n in srv.notifications
              if n.get("method") == "notifications/pilotdroid/watch_hit"]
    srv.tool("stream_stop")
    # stash order is conn order (hit before its action's toast) — pull
    # until the hit surfaces; a post-stop push notification also counts
    # (demux resumed and drained the stash).
    hit_seen = False
    for _ in range(3):
        ev = srv.tool("watch_events", {"timeout_ms": 5000})
        if ev.get("type") == "watch_hit":
            hit_seen = True
            break
        if ev.get("type") == "timeout":
            break
    pushed_late = [n for n in srv.notifications
                   if n.get("method") == "notifications/pilotdroid/watch_hit"]
    srv.tool("watch_disarm", {"watch_id": wid})  # best effort
    during_stream_no_push = len(pushed) == 0
    frames_ok = bool(frames.get("count"))
    return during_stream_no_push and frames_ok and (hit_seen or pushed_late)


TASKS = [
    ("nav_views", t_nav_views),
    ("nav_controls_light", t_nav_controls_light),
    ("checkbox_toggle", t_checkbox_toggle),
    ("radio_select", t_radio_select),
    ("set_text_probe", t_set_text),
    ("popup_toast_media", t_popup_toast),
    ("alert_dialog", t_alert_dialog),
    ("spinner_pick", t_spinner_pick),
    ("press_back_nav", t_press_back),
    ("stream_watch_coexist", t_stream_watch_coexist),
]


def main():
    only = sys.argv[1:] or None
    srv = Serve()
    results = []
    try:
        for name, fn in TASKS:
            if only and name not in only:
                continue
            t0 = time.time()
            try:
                ok = bool(fn(srv))
                err = ""
            except Exception as e:
                ok, err = False, f"{type(e).__name__}: {e}"[:120]
            dt = time.time() - t0
            results.append((name, ok, dt, err))
            print(f"{'PASS' if ok else 'FAIL'} {name} ({dt:.1f}s){' ' + err if err else ''}")
    finally:
        srv.close()
    n = len(results)
    np = sum(1 for _, ok, _, _ in results if ok)
    print(f"eval-harness: {np}/{n} passed"
          + (f" — failures: {[r[0] for r in results if not r[1]]}"
             if np != n else ""))
    sys.exit(0 if np == n else 1)


if __name__ == "__main__":
    main()
