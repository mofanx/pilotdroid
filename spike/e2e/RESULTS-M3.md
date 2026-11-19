# RESULTS-M3 — u2 compat shim v0 真机对拍

Device: ba0d072d (Mi 11, Android 14, MIUI). n=20 per engine, same script
`bench_m3_compat.py --engine {shim,u2}`. Shim = `compat/u2shim.py` (ADR-005
host-side translation layer, one session socket); u2 = uiautomator2 3.7.1
(JSON-RPC HTTP per call). Same-口径: wall-clock host-side per step.

## Scenario (identical both sides)

`app_start(Settings)` → `swipe` → `app_start(quicksearchbox)` →
`d(resourceId=query_text).click()` → `set_text("wifi")` →
`d(text="wifi").exists` → `screenshot` → `press("back")`

## Headline

| scope | shim | u2 | ratio |
|---|---|---|---|
| **sequence total** | **1955.8ms** | 4085.0ms | **2.1×** |
| **device-ops subtotal**¹ | **212.4ms** | 2140.4ms | **10.1×** |

¹ swipe + sel_click + set_text + exists + press_back — the ops that are
pilotdroid's actual mechanism; app_start/screenshot are shared fixed costs
(adb shell / JPEG encode) on both sides.

## Per-op p50 (ms)

| op | shim | u2 | ratio | note |
|---|---|---|---|---|
| swipe | 14.3 | 1133.1 | **79×** | u2 swipe = per-point rpc + gesture wait |
| press_back | 3.8 | 160.3 | **42×** | KeyEvent via injectInputEvent |
| sel_click | 54.0 | 516.9 | **9.6×** | QUERY+INJECT one socket vs 2 rpc |
| set_text | 92.2 | 330.0 | **3.6×** | ACTION_SET_TEXT both sides |
| screenshot | 111.6 | 214.5 | 1.9× | encode-bound both sides (E1) |
| app_start | ~83 | ~110 | 1.3× | shim forks `adb shell`, u2 rpc |
| exists | 48.1 | 0.05 | — | u2 0.05ms anomaly: its `.exists`
returned sub-RTT (client shortcut under implicitly_wait(0)); recorded as
measured, not counted into the ratio claim |

## Watchdog no-jar recovery

`kill -9` agent → next `agent tap` (no `--jar`) → graded recovery
(restart via setsid, remote jar intact → no reinstall) → `ok=true` in
**1.15s** wall. (m3-host `connect_recover(None)` path.)

## Honest notes

- ≥5× criterion: **met on the device-ops subtotal (10.1×)**, not on the
  naive sequence total (2.1×) — fixed costs both sides pay (am start,
  screenshot encode, sleeps) dilute it. This mirrors the M2 batch story:
  the win concentrates where the transport mechanism differs.
- u2 `exists` 0.05ms is an outlier — likely a client-side shortcut; does
  not change the conclusion either way.
- UiAutomation exclusivity: u2 run required stopping our agent (u2 server
  crashes otherwise); agent restored + u2 killed after the run.
