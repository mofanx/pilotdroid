#!/usr/bin/env python3
"""agent-loop eval harness v2 (M32) — difficulty climb over v1:

  * cross-app navigation (ApiDemos -> Settings -> back)
  * SoM index consumption (list_elements -> tap_index, not text-center)
  * deliberate stale-snapshot -> check_snapshot rejects, fresh snap heals
  * scroll-to-find off-screen element
  * ordered double-toast (timing)
  * fault injection every run: SIGKILL a mid-stream sibling serve, then
    prove a fresh stream still works (dirty-exit regression pin; the
    deeper backlog-ghost race stays in ghost_repro.py)
  * background/resume coherence

Reports task x N rounds with p50/p95 timing + success-rate table for
the ledger.

Run: `PILOTDROID_SERIAL=ba0d072d python3 loop_v2.py [--rounds N] [task ...]`
"""
import os
import statistics
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from loop import Serve, describe, find, tap_el, tap_text, fresh_start, \
    open_controls_light, find_scrolled, APIDEMOS

SETTINGS = "com.android.settings"


# ---------------- v2 tasks ----------------

def _settings_target(els):
    for label in ("About phone", "关于手机", "我的设备", "System",
                  "Additional settings", "更多设置", "WLAN", "蓝牙"):
        hit = find(els, text=label, contains=True)
        if hit:
            return hit
    return None


def t_cross_app_settings(srv):
    """Leave ApiDemos for Settings, find a known entry, return."""
    fresh_start(srv)
    srv.tool("press", {"keycode": 3})          # HOME
    time.sleep(0.8)
    srv.tool("app_start", {"pkg": SETTINGS})
    time.sleep(1.5)
    hit = _settings_target(describe(srv))
    if not hit:
        for _ in range(4):
            srv.tool("scroll", {"dir": "forward"})
            time.sleep(0.6)
            hit = _settings_target(describe(srv))
            if hit:
                break
    if not hit:
        return False
    srv.tool("app_start", {"pkg": APIDEMOS})
    time.sleep(1.2)
    return find_scrolled(srv, "Views") is not None


def t_som_index_tap(srv):
    """Consume the SoM index space: list_elements -> tap_index."""
    srv.tool("app_stop", {"pkg": APIDEMOS})
    srv.tool("app_start", {"pkg": APIDEMOS})
    time.sleep(1.5)
    find_scrolled(srv, "Views")          # render the row first — a11y
    els = srv.tool("list_elements")["elements"]
    target = next((e for e in els if e.get("text") == "Views"), None)
    if not target:
        raise RuntimeError("Views not in list_elements")
    srv.tool("tap_index", {"i": target["i"]})
    time.sleep(0.9)
    return find(describe(srv), text="Controls") is not None


def t_stale_snapshot_reverify(srv):
    """list_elements, navigate away -> tap_index must be rejected as
    stale; a fresh list_elements must work."""
    srv.tool("app_stop", {"pkg": APIDEMOS})
    srv.tool("app_start", {"pkg": APIDEMOS})
    time.sleep(1.5)
    find_scrolled(srv, "Views")          # render the row first
    els = srv.tool("list_elements")["elements"]
    stale_i = next((e["i"] for e in els if e.get("text") == "Views"), None)
    if stale_i is None:
        raise RuntimeError("Views absent from snapshot — stale_i undefined")
    tap_text(srv, "Views")                       # snapshot now stale
    # poll until the nav actually landed — tapping a still-fresh
    # snapshot is a harness race, not a stale-rejection failure.
    for _ in range(8):
        if find(describe(srv), text="Controls"):
            break
        time.sleep(0.4)
    try:
        srv.tool("tap_index", {"i": stale_i})
        return False                           # should have been rejected
    except RuntimeError as e:
        if "stale" not in str(e):
            raise
    els2 = srv.tool("list_elements")["elements"]
    t2 = next((e for e in els2 if e.get("text") == "Controls"), None)
    if not t2:
        return False
    srv.tool("tap_index", {"i": t2["i"]})
    time.sleep(0.9)
    return find(describe(srv), text="1. Light Theme") is not None


def t_scroll_find(srv):
    """Find an element that starts off-screen via bounded scroll."""
    fresh_start(srv)
    for _ in range(8):
        e = find(describe(srv), text="Preference")
        if e:
            tap_el(srv, e)
            time.sleep(0.9)
            return (find(describe(srv), text="Default values",
                         contains=True) is not None
                    or find(describe(srv), text="Preferences from XML",
                            contains=True) is not None)
        srv.tool("scroll", {"dir": "forward"})
        time.sleep(0.5)
    return False


def t_double_toast_order(srv):
    """Two popup clicks -> both toasts pulled in conn order."""
    fresh_start(srv, ".view.PopupMenu1")
    srv.tool("watch_events", {"timeout_ms": 400})   # attach media conn
    tap_text(srv, "MAKE A POPUP!", settle=1.0)
    tap_text(srv, "Add", settle=0.6)
    tap_text(srv, "MAKE A POPUP!", settle=1.0)
    tap_text(srv, "Edit", settle=0.5)
    got = []
    for _ in range(3):
        ev = srv.tool("watch_events", {"timeout_ms": 5000})
        if ev.get("type") == "toast":
            got.append(" ".join((ev.get("toast") or {}).get("text") or []))
        if len(got) >= 2 or ev.get("type") == "timeout":
            break
    return (len(got) == 2
            and "item Add" in got[0] and "item Edit" in got[1])


def t_dirty_kill_recovery(srv):
    """Fault injection: a mid-stream sibling serve is SIGKILLed; `srv`
    must still start a stream and read frames (dirty-exit pin)."""
    victim = Serve()
    try:
        victim.tool("stream_start", {"fps": 5, "scale": 0.5})
        time.sleep(0.3)
        victim.tool("stream_read", {"n": 2})
    finally:
        victim.p.kill()                            # dirty death, no cleanup
        victim.p.wait(5)
    time.sleep(0.5)
    srv.tool("stream_start", {"fps": 5, "scale": 0.5})
    fr = srv.tool("stream_read", {"n": 3})
    srv.tool("stream_stop")
    return fr.get("count", 0) >= 1


def t_background_resume(srv):
    """HOME mid-flow, relaunch — activity state retained, ops work."""
    open_controls_light(srv)
    srv.tool("press", {"keycode": 3})
    time.sleep(0.8)
    srv.tool("app_start", {"pkg": APIDEMOS})
    time.sleep(1.2)
    return find(describe(srv), text="Checkbox 1") is not None


TASKS = [
    ("cross_app_settings", t_cross_app_settings),
    ("som_index_tap", t_som_index_tap),
    ("stale_snapshot_reverify", t_stale_snapshot_reverify),
    ("scroll_find", t_scroll_find),
    ("double_toast_order", t_double_toast_order),
    ("dirty_kill_recovery", t_dirty_kill_recovery),
    ("background_resume", t_background_resume),
]


def pct(xs, p):
    xs = sorted(xs)
    if not xs:
        return 0.0
    k = max(0, min(len(xs) - 1, round(p * len(xs) - 0.5)))
    return xs[k]


def main():
    only = [a for a in sys.argv[1:] if not a.startswith("--")] or None
    rounds = 5
    for a in sys.argv[1:]:
        if a.startswith("--rounds="):
            rounds = int(a.split("=", 1)[1])
    srv = Serve()
    res = {}   # name -> [(ok, dt)]
    try:
        for name, fn in TASKS:
            if only and name not in only:
                continue
            for r in range(rounds):
                t0 = time.time()
                try:
                    ok = bool(fn(srv))
                    err = ""
                except Exception as e:
                    ok, err = False, f"{type(e).__name__}: {e}"[:110]
                dt = time.time() - t0
                res.setdefault(name, []).append((ok, dt))
                print(f"r{r} {'PASS' if ok else 'FAIL'} {name} "
                      f"({dt:.1f}s){(' ' + err) if err else ''}",
                      flush=True)
    finally:
        srv.close()

    print("\ntask                      pass      p50s   p95s   mean")
    for name, runs in res.items():
        oks = [ok for ok, _ in runs]
        dts = [dt for _, dt in runs]
        print(f"{name:24} {sum(oks)}/{len(runs):<8} "
              f"{pct(dts, .5):6.1f} {pct(dts, .95):6.1f} "
              f"{statistics.mean(dts):6.1f}")
    np = sum(ok for runs in res.values() for ok, _ in runs)
    n = sum(len(runs) for runs in res.values())
    print(f"\neval-v2: {np}/{n} ({np / n * 100:.0f}%)")
    sys.exit(0 if np == n else 1)


if __name__ == "__main__":
    main()
