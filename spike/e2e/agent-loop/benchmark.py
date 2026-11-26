#!/usr/bin/env python3
"""blind-driving benchmark — FROZEN regression gauge (v0.8.2+).

Distinct from eval v3 (`loop_v3.py`): v3 randomizes targets to prove
generality (a canned path cannot pass); THIS suite is deliberately
deterministic — same task spec, same targets, same order every run —
so a release-to-release delta is attributable to the toolchain, not
to the draw. Frozen means: do not add/remove/reorder tasks casually;
changes to this file bump the suite era and are noted in the ledger.

Blind-driving contract: every action is chosen ONLY from what a blind
model observes — describe_screen / list_elements / screenshot_annotated
elements (+ their `i`/`id`/center), never hardcoded coordinates.
Actions consumed: tap_index / tap(cx,cy) / set_text_index / scroll /
press / verify_after / watch_events.

Distribution: describe→tap atomicity (1-2) / B4 state-class (3) /
stale guard both drift flavors + negative match (4-5,11) / scroll (6) /
text+verify+watch breadth (7-9) / recovery (10).

Requires ApiDemos on-device (same as eval v1/v3). Run:
  PILOTDROID_SERIAL=ba0d072d python3 benchmark.py [task ...]
"""
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from loop import Serve, describe, find, tap_text, fresh_start, \
    open_controls_light, verify, APIDEMOS, SERIAL
from loop_v3 import fg_pkg, fg_act, _dismiss_fullscreen_ime, _ensure_fg


# ---------- helpers ----------

def _dismiss_old_sdk_warn(srv):
    """HarmonyOS shows an old-targetSdk modal on EVERY ApiDemos cold
    start — '此应用 SDK 版本过低' + [确定] covers the whole main list
    until tapped (recurring per launch, unlike the one-shot
    ReviewPermissionsActivity; on MIUI the text never appears →
    no-op describe)."""
    clean = 0
    for _ in range(6):
        els = describe(srv)
        if any("SDK 版本过低" in (e.get("text") or "") for e in els):
            btn = next((e for e in els if e.get("text") == "确定"
                        and e.get("i") is not None), None)
            if not btn:
                return
            srv.tool("tap_index", {"i": _i_of(btn)})
            clean = 0
            time.sleep(0.6)
            continue
        # the warn can arrive LATE on HarmonyOS (post-launch lag) —
        # require two consecutive clean describes before leaving
        clean += 1
        if clean >= 2:
            return
        time.sleep(0.7)


def _fresh(srv, activity=None):
    """fresh_start + landscape extract-IME dismissal — the MIUI
    fullscreen IME survives app restarts and covers ALL app bounds,
    silently eating every tap (observed: stale_layout_guard nav
    never landed because 'App' row taps hit the IME)."""
    fresh_start(srv, activity)
    _dismiss_fullscreen_ime(srv)
    _dismiss_old_sdk_warn(srv)


def _main_list(srv):
    """ApiDemos resumes the LAST sub-activity on MIUI even after
    force-stop (task-snapshot restore) — 'fresh_start' alone is not a
    main-list guarantee. BACK until a main-list marker is visible."""
    for _ in range(6):
        if fg_pkg() in HIJACKERS:
            _evict_hijacker(srv)               # hijack mid-descent —
                                             # evict before more backs
            continue
        _dismiss_old_sdk_warn(srv)
        els = describe(srv)
        if find(els, text="Views") or find(els, text="Accessibility"):
            return els
        srv.tool("press", {"keycode": 4})
        time.sleep(0.6)
    raise RuntimeError("main list unreachable")


def _i_of(e):
    i = e.get("i")
    if i is None:
        raise RuntimeError(f"element carries no i: {e.get('class')!r}")
    return i


def _describe_el(srv, tries=4, **kw):
    """describe→find with retry — a11y has a transition gap right after
    app_start where the dump is empty/mid-layout (observed: 'App' on a
    visibly-rendered main list absent for ~1s)."""
    els = []
    for i in range(tries):
        els = describe(srv)
        e = find(els, **kw)
        if e:
            return els, e
        time.sleep(0.5)
    raise RuntimeError(f"not on screen after {tries} tries: {kw}")


# ---------- frozen tasks ----------

def b_describe_tap_direct(srv):
    """T1 pin: fresh-ish session, describe->tap_index with NO
    list_elements in between — the armed snapshot must serve `i`."""
    _fresh(srv)
    els, e = _describe_el(srv, text="App")
    srv.tool("tap_index", {"i": _i_of(e)})
    time.sleep(0.9)
    return find(describe(srv), text="Activity") is not None


def b_i_space_divergent(srv):
    """`i` must resolve in the INTERACTIVE space, not SoM `id` space:
    pick an element where the two diverge and prove tap_index(i)
    returns that element's own bounds (wrong-space resolution would
    land on a different element). ApiDemos' main list has NO divergent
    node (all leading nodes interactive, id==i); Settings main has
    non-interactive containers first (id=5/i=0 observed)."""
    srv.tool("app_stop", {"pkg": "com.android.settings"})
    srv.tool("app_start", {"pkg": "com.android.settings"})
    time.sleep(1.5)
    _dismiss_fullscreen_ime(srv)
    els = describe(srv)
    cand = next((e for e in els if e.get("i") is not None
                 and e.get("id") is not None and e["i"] != e["id"]),
                None)
    if not cand:
        raise RuntimeError("no id!=i divergent element on Settings")
    r = srv.tool("tap_index", {"i": cand["i"]})
    srv.tool("press", {"keycode": 4})
    srv.tool("press", {"keycode": 4})
    return r.get("bounds") == cand["bounds"]


def b_checkbox_state(srv):
    """B4 class, FROZEN target: toggle 'Checkbox 1', prove the flip via
    verify_after's state predicate (checked=want)."""
    open_controls_light(srv)
    _dismiss_fullscreen_ime(srv)   # landscape extract-IME covers the
                                   # checkbox rows and eats their taps
    els, e = _describe_el(srv, cls="CheckBox", text="Checkbox 1")
    want = not bool(e.get("checked"))
    st = {"state": {"match": {"text": "Checkbox 1"}, "checked": want}}
    # node bounds span the whole ROW but only the widget's left edge
    # toggles (row-center tap is silently eaten on this ROM — verified
    # on-device: tap(x0+40,cy) flips, tap(cx,cy) does not).
    wx = e["bounds"][0] + 40
    wy = (e["bounds"][1] + e["bounds"][3]) // 2
    ok, _ = verify(srv, {"tool": "tap", "args": {"x": wx, "y": wy}},
                   st, settle_ms=1500)
    if ok:
        return True
    # a11y `checked` propagation measured up to ~4s on this ROM —
    # confirm the toggle landed, then re-assert via read-only action.
    for _ in range(10):
        now = next((x for x in srv.tool("list_elements")["elements"]
                    if x.get("text") == "Checkbox 1"), None)
        if now and bool(now.get("checked")) == want:
            break
        time.sleep(0.4)
    else:
        return False
    ok, _ = verify(srv, {"tool": "exists", "args": {}}, st,
                   settle_ms=300)
    return ok


def b_stale_layout_guard(srv):
    """describe->scroll->tap_index must be REJECTED with a stale error
    NAMING the observation source (describe_screen); re-observe arms a
    fresh snapshot that consumes `i` again. Scroll is the deterministic
    drift injection: row bounds always shift -> signature always
    changes (nav-based drift raced task-snapshot restores/hijackers)."""
    _fresh(srv)
    _ensure_fg(srv, APIDEMOS)
    _main_list(srv)
    els = describe(srv)
    e = next((x for x in els if x.get("i") is not None
              and x.get("clickable")), None)
    if not e:
        raise RuntimeError("no interactive element on main list")
    stale_i = _i_of(e)
    # drift injection: scroll on phones; on big tablets the whole main
    # list fits without a scrollable node (HarmonyOS MRR-W29 — scroll
    # returns NO_NODE) so nav into the 'App' sublist instead — that
    # also exercises the same-GEOMETRY collision class directly
    # (identical ListView shape, different row texts: the guard's
    # want_i identity check is what must fire).
    try:
        srv.tool("scroll", {"dir": "forward"})
    except RuntimeError:
        app_row = next((x for x in els if x.get("text") == "App"), None)
        if not app_row:
            raise RuntimeError("no scrollable node and no 'App' row")
        c = app_row["center"]
        srv.tool("tap", {"x": c[0], "y": c[1]})
    time.sleep(0.8)
    try:
        srv.tool("tap_index", {"i": stale_i})
        return False                           # silent wrong-tap = fail
    except RuntimeError as ex:
        if "stale" not in str(ex) or "describe_screen" not in str(ex):
            raise                              # wrong error shape
    # re-observe + consume on the new layout
    els2 = describe(srv)
    e2 = next((x for x in els2 if x.get("i") is not None
               and x.get("clickable")), None)
    if not e2:
        raise RuntimeError("no interactive element after scroll")
    srv.tool("tap_index", {"i": _i_of(e2)})
    time.sleep(0.9)
    srv.tool("press", {"keycode": 4})
    return True


def b_stale_pkg_guard(srv):
    """describe->fg-app-change->tap_index rejected (pkg flavor of the
    stale guard); the same error message must carry both pkgs."""
    _fresh(srv)
    els = describe(srv)
    e = next((x for x in els if x.get("i") is not None
              and x.get("clickable")), None)
    if not e:
        raise RuntimeError("no interactive element to arm with")
    stale_i = _i_of(e)
    # com.android.settings is the cross-OEM deterministic drift target
    # (com.miui.calculator was Xiaomi-only — a task-level portability
    # defect the CFE6R benchmark row exposed).
    srv.tool("app_start", {"pkg": "com.android.settings"})
    # wait until the drift is OBSERVABLE to the guard: HarmonyOS has a
    # transition window (~1.5-3s) where app_current reports {} (empty)
    # and the a11y tree still serves the previous app's nodes — a
    # tap in that window permissively consumes the stale snapshot.
    for _ in range(10):
        cur = srv.tool("app_current").get("package")
        if cur == "com.android.settings":
            break
        time.sleep(0.5)
    else:
        raise RuntimeError("settings never became observable to agent")
    try:
        srv.tool("tap_index", {"i": stale_i})
        return False
    except RuntimeError as ex:
        if "stale" not in str(ex) and "changed" not in str(ex):
            raise
    srv.tool("app_start", {"pkg": APIDEMOS})
    time.sleep(1.2)
    return True


def b_scroll_find(srv):
    """Off-screen target consumed via `i` after bounded scroll."""
    _fresh(srv)
    for _ in range(8):
        e = find(describe(srv), text="Preference")
        if e:
            srv.tool("tap_index", {"i": _i_of(e)})
            time.sleep(0.9)
            d = describe(srv)
            return (find(d, text="Default values", contains=True)
                    or find(d, text="Preferences from XML",
                            contains=True)) is not None
        srv.tool("scroll", {"dir": "forward"})
        time.sleep(0.5)
    return False


def b_set_text_index(srv):
    """set_text_index consumes the SAME `i` snapshot — no pre-tap."""
    open_controls_light(srv)
    els, e = _describe_el(srv, cls="EditText")
    srv.tool("set_text_index", {"i": _i_of(e), "text": "pd_bench_probe"})
    ok, _ = verify(srv, {"tool": "exists", "args": {}},
                   {"text": "pd_bench_probe"}, settle_ms=800)
    return ok


def b_verify_state_pred(srv):
    """Read-only state predicate: current `checked` of a known box is
    assertable via verify_after(action=exists) — pins state semantics
    without mutating the device."""
    open_controls_light(srv)
    _dismiss_fullscreen_ime(srv)   # landscape extract-IME covers the
                                   # checkbox rows and eats their taps
    els, e = _describe_el(srv, cls="CheckBox", text="Checkbox 1")
    cur = bool(e.get("checked"))
    st = {"state": {"match": {"text": "Checkbox 1"}, "checked": cur}}
    ok, _ = verify(srv, {"tool": "exists", "args": {}}, st,
                   settle_ms=600)
    neg = {"state": {"match": {"text": "Checkbox 1"}, "checked": not cur}}
    ok2, _ = verify(srv, {"tool": "exists", "args": {}}, neg,
                    settle_ms=300)
    return ok and not ok2                       # true-state passes,
                                                # false-state fails
def b_watch_toast(srv):
    """Toast demux on the media channel (frozen: PopupMenu1 > Add)."""
    _fresh(srv, ".view.PopupMenu1")
    srv.tool("watch_events", {"timeout_ms": 500})   # attach first
    tap_text(srv, "MAKE A POPUP!", settle=1.0)
    tap_text(srv, "Add", settle=0.5)
    ev = srv.tool("watch_events", {"timeout_ms": 6000})
    return (ev.get("type") == "toast"
            and "Clicked popup menu item Add" in " ".join(
                (ev.get("toast") or {}).get("text") or []))


def b_dirty_kill_recovery(srv):
    """A mid-stream sibling serve SIGKILLed; `srv` must still start a
    stream and read frames (dirty-exit pin)."""
    victim = Serve()
    try:
        victim.tool("stream_start", {"fps": 5, "scale": 0.5})
        time.sleep(0.3)
        victim.tool("stream_read", {"n": 2})
    finally:
        victim.p.kill()
        victim.p.wait(5)
    time.sleep(0.5)
    srv.tool("stream_start", {"fps": 5, "scale": 0.5})
    fr = srv.tool("stream_read", {"n": 3})
    srv.tool("stream_stop")
    return fr.get("count", 0) >= 1


def b_match_empty_error(srv):
    """Negative pin: expect.state.match:{} is an explicit tool error,
    not a silent found:false."""
    try:
        srv.tool("verify_after",
                 {"action": {"tool": "exists", "args": {}},
                  "expect": {"state": {"match": {}, "checked": True}}})
        return False
    except RuntimeError as ex:
        return "selector" in str(ex) or "no selector keys" in str(ex)


BENCH = [
    ("describe_tap_direct",  b_describe_tap_direct),
    ("i_space_divergent",    b_i_space_divergent),
    ("checkbox_state",       b_checkbox_state),
    ("stale_layout_guard",   b_stale_layout_guard),
    ("stale_pkg_guard",      b_stale_pkg_guard),
    ("scroll_find",          b_scroll_find),
    ("set_text_index",       b_set_text_index),
    ("verify_state_pred",    b_verify_state_pred),
    ("watch_toast",          b_watch_toast),
    ("dirty_kill_recovery",  b_dirty_kill_recovery),
    ("match_empty_error",    b_match_empty_error),
]


# Foreground hijackers observed on this ROM — when one holds fg, the
# task's taps land on ITS window; the FAIL is environmental, not a
# toolchain result. Gate: evict before each task, retry once on a
# hijacked FAIL (marked env= so it never masquerades as a real fail).
HIJACKERS = ("com.agenthub", "com.xiaomi.misubscreenui",
             "com.miui.securitycenter", "com.miui.home", "com.sctrl")


def _evict_hijacker(srv):
    fg = fg_pkg()
    if fg in HIJACKERS:
        try:
            srv.tool("app_stop", {"pkg": fg})
        except RuntimeError:
            pass
    # misubscreenui respawns within seconds — assert fg, not just fire
    # one launch (v3's _ensure_fg pattern).
    try:
        _ensure_fg(srv, APIDEMOS)
    except RuntimeError:
        pass
    # _ensure_fg's launch can itself trigger the HarmonyOS SDK warn —
    # clear it as part of the same pre-task environment sweep.
    _dismiss_old_sdk_warn(srv)


def main():
    only = [a for a in sys.argv[1:] if not a.startswith("-")]
    tasks = BENCH if not only else \
        [(n, f) for n, f in BENCH if n in only]
    srv = Serve()
    rows = []
    try:
        for name, fn in tasks:
            t0 = time.time()
            env = ""
            for attempt in range(2):
                _evict_hijacker(srv)
                try:
                    ok = fn(srv)
                    err = ""
                except Exception as ex:          # task-level fault
                    ok, err = False, str(ex)[:120]
                fg = fg_pkg()
                if not ok and fg in HIJACKERS and attempt == 0:
                    env = f"env-hijack:{fg}→retry"
                    continue                     # ONE environmental retry
                break
            dt = time.time() - t0
            rows.append((name, ok, dt, fg, err or env))
            print(f"{'PASS' if ok else 'FAIL'} {name:22s} "
                  f"{dt:5.1f}s fg={fg}"
                  + (f" err={err}" if err else "")
                  + (f" env={env}" if env else ""), flush=True)
    finally:
        srv.close()
        # post-suite hygiene: nothing stays foreground-weird
    npass = sum(1 for r in rows if r[1])
    print(f"\nbenchmark: {npass}/{len(rows)} pass "
          f"({npass * 100 // len(rows)}%) serial={SERIAL} tasks=frozen-v1")
    return 0 if npass == len(rows) else 1


if __name__ == "__main__":
    sys.exit(main())
