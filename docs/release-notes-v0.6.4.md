# pilotdroid v0.6.4

Device-side UiAutomation agent for Android: single-jar dex under
`app_process` + `pilotdroid-cli` (agent.jar embedded — one binary does
setup/serve/bench) + `import pilotdroid` uiautomator2 drop-in shim +
stdio MCP server (27 tools).

## What's new in 0.6.4 (M25–M27)

**`Device()` no-serial multi-device silent failure — fixed**
- v0.6.3 wheel defect: `adb forward --list` parsing dropped the
  serial, so a bare `u2shim.Device()` on a multi-device host made
  every adb call fail with `more than one device/emulator` — silent
  rc=1 / empty output. `_detect_port` now returns `(serial, port)`:
  the sole matching forward row backfills serial (u2 `connect()`
  first-device semantics), several devices forwarding → guiding
  error, no forward → `adb devices` adopts exactly one device.
  Explicit `serial=` with no matching forward row now raises a
  serial-named error instead of probing tcp:17932 (a live port there
  may belong to another device — conn vs `-s` split).

**Watcher `when("//xpath")` — host-poll rules + fixed `.click()`**
- Non-wireable but xpath-lite-evaluable expressions degrade from
  `NotImplementedError` to host-poll rules (one `dump_hierarchy` +
  `_XPathLite.exists` per tick); expressions outside even the lite
  subset still fail fast.
- `when(xpath).click()` re-evaluates the xpath at fire time and
  inject-taps the first match's center — the same FLAG_SECURE-safe
  coordinate path as the builtin dialog recipes. The sentinel key no
  longer leaks into `_Selector` (previously the click action silently
  died inside the watcher loop on main; never shipped).

**Bounded re-arm + public `max_fires`**
- Rules stay fire-once by default; `call(fn, max_fires=N)` and
  `click(max_fires=N)` re-arm up to N hits before removal.
  `max_fires < 1` raises `ValueError`.
- `watch_context(builtin=True)` LBE multi-page review rule carries
  budget 3; other builtin recipes remain one-shot.

**Verified**
- uiautomator2 vendored suite: 22 passed / 0 failed / 8 skipped
  (upstream's own skip markers — the attainable ceiling).
- pytest 68/68; real-device ba0d072d parity 5/5 (implicitly_wait
  round-trip, selector enumeration, count, watcher callback, and
  xpath click navigating ApiDemos → Accessibility submenu).
- `doc-drift` now also pins the install-smoke `VER` default and
  requires CHANGELOG coverage once commits land past the last tag.

Compatibility notes and known limits are in `compat/SUPPORTED.md`
(watcher ~50–100ms re-arm window, xpath `.click()` hits the first
match, MCP watcher events are pull via `watch_events`).
