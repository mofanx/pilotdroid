#!/usr/bin/env python3
"""agent-loop eval harness v3 — discrimination climb over v2:

  * randomized targets (a canned path cannot pass):
      - random_index_pick: pick a RANDOM clickable entry on the ApiDemos
        main list, consume it via tap_index, prove the list is gone,
        BACK restores it. Each round hits a different index.
      - random_checkbox_state: pick a RANDOM CheckBox in Controls >
        1. Light Theme, read its emitted state flag, tap, then assert
        the flipped `checked` via verify_after's state predicate —
        the B4 gap closed by the states surface.
  * failure attribution: every FAIL carries `fg=<pkg>` — the foreground
    package at failure time (external-app takeovers like the B4 chat
    interruption are no longer silent).
  * PORTABLE subset: no ApiDemos dependency (Settings-only) — the
    multi-device correctness row for units where sideloading is
    USER_RESTRICTED (e.g. V816 API37: Install via USB off).

Run: `PILOTDROID_SERIAL=ba0d072d python3 loop_v3.py [--rounds N] [--portable] [task ...]`
"""
import os
import random
import re
import statistics
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from loop import Serve, describe, find, tap_el, fresh_start, \
    open_controls_light, verify, APIDEMOS, SERIAL
from loop_v2 import TASKS as TASKS_V2, SETTINGS


def _nav_tap(srv, label, tries=8):
    """Scroll-aware describe->tap: a11y exposes rendered nodes only, so
    long lists (ApiDemos main) need bounded scroll to reach `label`."""
    for i in range(tries):
        e = find(describe(srv), text=label)
        if e:
            tap_el(srv, e)
            time.sleep(0.9)
            return
        try:
            # forward sweep, then a back-sweep — a forward-only loop
            # can scroll PAST an early item on recycled list content.
            srv.tool("scroll",
                     {"dir": "forward" if i < tries // 2 else "back"})
        except RuntimeError:
            pass                            # NO_NODE mid-transition — retry
        time.sleep(0.5)
    raise RuntimeError(f"element not on screen: {label!r}")


def _dismiss_fullscreen_ime(srv):
    """MIUI fullscreen/extract IME (landscape) ignores KEYCODE_BACK and
    covers ALL app bounds — its only exit is the top-right Done button.
    Portrait keeps the IME below app content; nothing to do there."""
    try:
        out = subprocess.check_output(
            ["adb", "-s", SERIAL, "shell",
             "wm size; dumpsys window | grep mCurrentRotation"],
            timeout=6, text=True)
        w, h = map(int, re.search(r"(\d+)x(\d+)", out).groups())
        rot = re.search(r"ROTATION_(\d+)", out)
        if rot is None or int(rot.group(1)) not in (90, 270):
            return                       # portrait: IME not fullscreen
        w, h = max(w, h), min(w, h)      # logical landscape dims
        for _ in range(3):
            shown = subprocess.check_output(
                ["adb", "-s", SERIAL, "shell",
                 "dumpsys input_method | grep mInputShown"],
                timeout=6, text=True)
            if "mInputShown=false" in shown:
                return
            srv.tool("tap", {"x": int(w * 0.943), "y": int(h * 0.19)})
            time.sleep(0.8)
    except Exception:
        return                           # best-effort hygiene only


def fg_pkg():
    """Foreground package attribution for failure records."""
    try:
        out = subprocess.check_output(
            ["adb", "-s", SERIAL, "shell", "dumpsys", "window"],
            timeout=8, text=True, stderr=subprocess.DEVNULL)
        m = re.search(r"mCurrentFocus=Window\{[^}]* u0 ([\w.]+)/", out)
        if m:
            return m.group(1)
        m = re.search(r"mFocusedApp=ActivityRecord\{[^}]* u0 ([\w.]+)/",
                      out)
        return m.group(1) if m else "?"
    except Exception:
        return "?"


# ---------------- v3 randomized tasks ----------------

def _ensure_fg(srv, pkg, tries=3):
    """Foreground-package gate: external takeovers (com.miui.home,
    sctrl, misubscreenui) must be recovered BEFORE the task body — a
    hijacked foreground is environmental, not a task result. Raises
    with the offending pkg for fg attribution when unrecoverable."""
    for _ in range(tries):
        fg = fg_pkg()
        if fg == pkg:
            return
        srv.tool("app_start", {"pkg": pkg})
        time.sleep(1.0)
    raise RuntimeError(f"foreground held by {fg_pkg()}")


def t_random_index_pick(srv):
    """Random main-list entry via tap_index — nav away, BACK restores.
    Mid-task fg takeover (com.miui.home etc.) is environmental: if the
    result fails while fg is NOT ApiDemos, restore foreground and
    retry once rather than counting a harness-influenced FAIL."""
    for attempt in range(2):
        srv.tool("app_stop", {"pkg": APIDEMOS})
        srv.tool("app_start", {"pkg": APIDEMOS})
        _ensure_fg(srv, APIDEMOS)
        time.sleep(1.5)
        els = srv.tool("list_elements")["elements"]
        cands = [e for e in els
                 if e.get("text") and e.get("clickable")]
        if len(cands) < 3:
            time.sleep(0.8)                    # mid-transition a11y gap
            els = srv.tool("list_elements")["elements"]
            cands = [e for e in els
                     if e.get("text") and e.get("clickable")]
        if len(cands) < 3:
            raise RuntimeError("main list too thin")
        before = {e.get("text") for e in els}
        pick = random.choice(cands)
        srv.tool("tap_index", {"i": pick["i"]})
        time.sleep(1.0)
        after = srv.tool("list_elements")["elements"]
        gone = not any(e.get("text") == pick["text"] for e in after)
        srv.tool("press", {"keycode": 4})
        time.sleep(0.9)
        restored = {e.get("text") for e in
                    srv.tool("list_elements")["elements"]} == before
        if gone and restored:
            return True
        if attempt == 0 and fg_pkg() != APIDEMOS:
            continue                           # fg takeover — retry
        return False


def t_random_checkbox_state(srv):
    """Random CheckBox toggle proven by the state predicate (B4 gap)."""
    fresh_start(srv)
    _nav_tap(srv, "Views")
    _nav_tap(srv, "Controls")
    _nav_tap(srv, "1. Light Theme")
    _dismiss_fullscreen_ime(srv)
    els = srv.tool("list_elements")["elements"]
    # Real-device hazard: elements whose tap centre sits inside the
    # bottom gesture-exclusion strip (e.g. the ★Star row at ~94% H on
    # this 3200x1440 panel) silently eat injected taps — exclude them
    # from the random draw rather than misreport a device-noise fail.
    hmax = max((e["bounds"][3] for e in els), default=0)
    boxes = [e for e in els if e.get("class") == "CheckBox"
             and (e["bounds"][1] + e["bounds"][3]) / 2 < 0.88 * hmax]
    if not boxes:
        raise RuntimeError("no tappable CheckBox in list_elements")
    pick = random.choice(boxes)
    want = not bool(pick.get("checked"))     # absent flag == False
    # match by text, NOT i — state-predicate match.i resolves against
    # the FULL-tree index space (list_elements(false) inside serve),
    # while the host-visible `i` is the interactive subset's — the two
    # index spaces diverge (serve.rs:1087; flagged to t2 as a nit).
    st = {"state": {"match": {"text": pick["text"]}, "checked": want}}
    ok, _ = verify(srv, {"tool": "tap_index", "args": {"i": pick["i"]}},
                   st, settle_ms=1500)
    if ok:
        return True
    # a11y `checked` attr propagation measured up to ~4s on this ROM —
    # variable, so a fixed settle races it. Confirm the toggle landed
    # at all, then re-assert the now-stable state through the same
    # predicate (action=exists read: no side effect).
    for _ in range(10):
        els = srv.tool("list_elements")["elements"]
        e = next((e for e in els if e.get("i") == pick["i"]), None)
        if e and bool(e.get("checked")) == want:
            break
        time.sleep(0.4)
    else:
        return False                     # toggle never landed
    ok, _ = verify(srv, {"tool": "exists", "args": {}}, st,
                   settle_ms=300)
    return ok


# ---------------- PORTABLE subset (no ApiDemos) ----------------

def _settings_list(srv):
    srv.tool("app_stop", {"pkg": SETTINGS})
    srv.tool("app_start", {"pkg": SETTINGS})
    _ensure_fg(srv, SETTINGS)
    time.sleep(2.0)
    return srv.tool("list_elements")["elements"]


def t_som_shape(srv):
    """SoM contract a blind model consumes: i/class/center/text present,
    class is the simple name (t2 normalization), on ANY device."""
    els = _settings_list(srv)
    if not els:
        raise RuntimeError("Settings list_elements empty")
    for e in els:
        for k in ("i", "class", "center", "text"):
            if k not in e:
                return False
        if "." in (e.get("class") or ""):
            return False                       # simple-name contract
    return True


def t_settings_random_pick(srv):
    """Random clickable in Settings -> tap_index -> screen changed."""
    els = _settings_list(srv)
    cands = [e for e in els if e.get("text") and e.get("clickable")]
    if len(cands) < 3:
        raise RuntimeError("Settings list too thin")
    before = {e.get("text") for e in els}
    pick = random.choice(cands)
    srv.tool("tap_index", {"i": pick["i"]})
    time.sleep(1.2)
    after = srv.tool("list_elements")["elements"]
    srv.tool("press", {"keycode": 4})
    time.sleep(0.9)
    return {e.get("text") for e in after} != before


def t_verify_exists_pred(srv):
    """verify_after exists-predicate on a stable Settings anchor."""
    els = _settings_list(srv)
    anchor = next((e for e in els
                   if e.get("text") and e.get("clickable")), None)
    if not anchor:
        raise RuntimeError("no Settings anchor")
    ok, _ = verify(srv, {"tool": "exists",
                         "args": {"text": anchor["text"][:20]}},
                   {"exists": {"text": anchor["text"][:20]}},
                   settle_ms=500)
    return ok


TASKS_V3 = TASKS_V2 + [
    ("random_index_pick", t_random_index_pick),
    ("random_checkbox_state", t_random_checkbox_state),
]

PORTABLE = [
    ("som_shape", t_som_shape),
    ("settings_random_pick", t_settings_random_pick),
    ("verify_exists_pred", t_verify_exists_pred),
]


def pct(xs, p):
    xs = sorted(xs)
    if not xs:
        return 0.0
    k = max(0, min(len(xs) - 1, round(p * len(xs) - 0.5)))
    return xs[k]


def main():
    portable = "--portable" in sys.argv
    only = [a for a in sys.argv[1:]
            if not a.startswith("--")] or None
    rounds = 5
    for a in sys.argv[1:]:
        if a.startswith("--rounds="):
            rounds = int(a.split("=", 1)[1])
    tasks = PORTABLE if portable else TASKS_V3
    # Eval env: null out the default IME so auto-focused EditTexts never
    # raise a keyboard (ACTION_SET_TEXT bypasses the IME anyway — v1
    # set_text task unaffected). Only applied for ApiDemos tasks — the
    # --portable Settings subset types nothing, and on non-test devices
    # a settings write is unjustified interference. BASELINE RESTORE:
    # `old_ime` is captured BEFORE the pin and written back in finally;
    # the restored value is printed for audit. Caveat: a SIGKILL of the
    # harness skips the restore — re-run `settings put secure
    # default_input_method <value>` manually in that case.
    old_ime = None
    if not portable:
        try:
            old_ime = subprocess.check_output(
                ["adb", "-s", SERIAL, "shell",
                 "settings get secure default_input_method"],
                timeout=6, text=True).strip()
            subprocess.run(
                ["adb", "-s", SERIAL, "shell",
                 "settings put secure default_input_method "
                 "pilotdroid.eval/.NoIme"],
                timeout=6, check=False)
        except Exception:
            old_ime = None
    srv = Serve()
    res = {}          # name -> [(ok, dt)]
    fails = []        # (task, round, err, fg)
    try:
        for name, fn in tasks:
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
                fg = "" if ok else fg_pkg()
                res.setdefault(name, []).append((ok, dt))
                if not ok:
                    fails.append((name, r, err, fg))
                print(f"r{r} {'PASS' if ok else 'FAIL'} {name} "
                      f"({dt:.1f}s){(' ' + err) if err else ''}"
                      f"{(' fg=' + fg) if fg else ''}", flush=True)
    finally:
        srv.close()
        if old_ime:
            subprocess.run(
                ["adb", "-s", SERIAL, "shell",
                 f"settings put secure default_input_method {old_ime}"],
                timeout=6, check=False)
            print(f"eval env: default_input_method restored to {old_ime}")

    print("\ntask                      pass      p50s   p95s   mean")
    for name, runs in res.items():
        oks = [ok for ok, _ in runs]
        dts = [dt for _, dt in runs]
        print(f"{name:24} {sum(oks)}/{len(runs):<8} "
              f"{pct(dts, .5):6.1f} {pct(dts, .95):6.1f} "
              f"{statistics.mean(dts):6.1f}")
    np = sum(ok for runs in res.values() for ok, _ in runs)
    n = sum(len(runs) for runs in res.values())
    kind = "portable" if portable else "v3"
    print(f"\neval-{kind}: {np}/{n} ({np / n * 100:.0f}%) "
          f"serial={SERIAL}")
    if fails:
        print("failures (fg-pkg attribution):")
        for name, r, err, fg in fails:
            print(f"  {name} r{r} fg={fg} {err}")
    sys.exit(0 if np == n else 1)


if __name__ == "__main__":
    main()
