#!/usr/bin/env python3
"""M6 compat matrix: run u2-style usage patterns unmodified through u2shim.

Three scripts modelled on uiautomator2 public README / doc examples. Each op
is executed independently and scored: ok / unsupported(ValueError) / error.
u2 semantics: `d(...).exists` and `d.info` are properties (no parens).
Usage: compat_m6_realscripts.py [serial] [port]
"""
import json, os, sys, time

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "compat"))
import u2shim  # noqa: E402

SERIAL = sys.argv[1] if len(sys.argv) > 1 else "ba0d072d"
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 17932
OUT = os.path.join(os.path.dirname(__file__), "e2e-m6-compat.json")


def score(fn):
    try:
        r = fn()
        return ("ok", None if r is None else str(r)[:60])
    except ValueError as e:
        return ("unsupported", str(e)[:80])
    except Exception as e:  # noqa: BLE001
        return ("error", f"{type(e).__name__}: {e}"[:80])


def main():
    d = u2shim.connect(PORT, serial=SERIAL)
    results = {}

    # ---- script A: u2 README quickstart -----------------------------------
    a = {
        "d.info (property)": lambda: d.info,
        "d.app_current": lambda: d.app_current(),
        "d.click(x,y)": lambda: d.click(500, 500),
        "d(text=...).exists": lambda: d(text="设置").exists,
        "d(index=0,class).click": lambda: d(className="android.widget.LinearLayout",
                                             index=0).click(),
        "d.screenshot(path)": lambda: d.screenshot("/tmp/m6_shot.jpg", scale=0.5),
        "d.dump_hierarchy": lambda: len(d.dump_hierarchy()),
        "d.press(back)": lambda: d.press("back"),
    }
    results["script_A_readme_quickstart"] = {k: score(v) for k, v in a.items()}

    # ---- script B: selector + scroll (u2 selector docs) --------------------
    b = {
        "d(className=...).exists": lambda: d(className="android.widget.TextView").exists,
        "d(scrollable).scroll.forward": lambda: d(scrollable=True).scroll.forward(),
        "d(scrollable).scroll.toBeginning": lambda: d(scrollable=True).scroll.toBeginning(),
        "d(packageName=...).exists": lambda: d(packageName="com.android.settings").exists,
        "d(resourceId=...).info (property)": lambda: d(resourceId="android:id/title").info,
        "d(description=...).exists": lambda: d(description="搜索").exists,
        "d(index=1).exists": lambda: d(className="android.widget.TextView",
                                        index=1).exists,
        "d(xpath=...) [u2 xpath plugin]": lambda: d(xpath="//*[@text='设置']").exists,
    }
    results["script_B_selector_scroll"] = {k: score(v) for k, v in b.items()}

    # ---- script C: app lifecycle + input (u2 cookbook style) ---------------
    c = {
        "d.app_stop(settings)": lambda: d.app_stop("com.android.settings"),
        "d.app_start(settings)": lambda: d.app_start("com.android.settings"),
        "exists(title)+1.2s": lambda: (time.sleep(1.2),
                                       d(className="TextView").exists)[1],
        "d.set_text": lambda: d.set_text("pd"),          # no focused field -> False is graceful
        "d.long_click(x,y)": lambda: d.long_click(500, 1200, 0.6),
        "d.swipe": lambda: d.swipe(500, 1800, 500, 800, duration=0.2),
        "d.press(home)": lambda: d.press("home"),
        "d.press(recent)": lambda: d.press("recent"),
    }
    results["script_C_app_input"] = {k: score(v) for k, v in c.items()}

    # ---- matrix summary ----------------------------------------------------
    flat = [v[0] for s in results.values() for v in s.values()]
    summary = {"ok": flat.count("ok"), "unsupported": flat.count("unsupported"),
               "error": flat.count("error"), "total": len(flat)}
    doc = {"serial": SERIAL, "scripts": results, "summary": summary}
    with open(OUT, "w") as f:
        json.dump(doc, f, ensure_ascii=False, indent=2)

    for sname, ops in results.items():
        print(f"\n== {sname} ==")
        for op, (st, note) in ops.items():
            print(f"  {st:11s} {op}" + (f"  -- {note}" if note else ""))
    print(f"\nsummary: {summary}  -> {OUT}")
    d.close()


if __name__ == "__main__":
    main()
