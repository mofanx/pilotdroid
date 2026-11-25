# pilotdroid v0.6.3

Device-side UiAutomation agent for Android: single-jar dex under
`app_process` + `pilotdroid-cli` (agent.jar embedded — one binary does
setup/serve/bench) + `import pilotdroid` uiautomator2 drop-in shim +
stdio MCP server (27 tools).

## What's new in 0.6.3 (M23–M24)

**u2shim xpath-lite — verbatim suite reaches its ceiling: 22/0/8**
- `d.xpath(x)` is now a host-side approximation (~210 lines,
  stdlib-only): one `dump_hierarchy` + ElementTree per query.
  Supported: text shorthand, `%fuzzy%` wildcards, `@resource-id`,
  bare `pkg:id/name`, `//*[@attr='v']`, `contains()`, `parent()`,
  `scroll_to()`, `all()`/`get()`/`exists`/`wait()`/`click()`/
  `get_text()`/`info`/`swipe`. Advanced axes/positional/`and-or`
  predicates raise `NotImplementedError` honestly — this is not the
  u2 xpath plugin pipeline.
- Official uiautomator2 suite (vendored verbatim): all 9 xpath tests
  flipped from fail to pass → **22 passed / 0 failed / 8 skipped**
  (the 8 skips are upstream's own deprecated/TODO/large-screen
  markers — the attainable ceiling).
- Watcher `when()` translates simple `//*[@text|@resource-id|
  @content-desc|@class|@package=...]` xpath to wire selectors 1:1;
  non-wireable xpath raises NIE.

**`%` wildcard = u2 SQL-LIKE three-state (was a super-match bug)**
- `v%`→startswith, `%v`→endswith, `%v%`→contains. Previously every
  wildcard collapsed to contains — a superset-matching deviation.

**Small compat surface补齐**
- `d.implicitly_wait()` is now dual-state: no-arg returns the current
  `settings['wait_timeout']`, with arg sets it.
- Selectors are enumerable: `len(d(...))`, `for el in d(...)`,
  `d(...)[i]` (maps to `instance=`).

**`watch_context(builtin=True)` — MIUI stock-dialog recipes**
- Ships the three MIUI dialogs our fixture forensics actually hit:
  `AdbInstallActivity`「继续安装」(races a 10s auto-deny countdown),
  LBE `ReviewPermissions`「确定」(single-page fire-once), and
  expanded NotificationShade collapse.
- Actions use host-side inject + settle/retry — FLAG_SECURE dialogs
  swallow device-armed a11y ACTION_CLICK (proved on-device).

**Fixture recovery, scripted**
- `spike/e2e/u2suite/fixture-doctor.sh`: `pm path` idempotent check →
  cached ApiDemos download/push → `adb install
  --bypass-low-target-sdk-block` while the agent auto-taps the
  MIUI install dialog → `monkey` first-launch loop-taps the
  multi-page LBE review (up to 8 pages). conftest guard now points
  at it.

**Docs & tooling**
- xpath wording corrected everywhere (README / python README /
  SUPPORTED.md): "approximation subset", not "unsupported".
- `doc-drift.py` cross-checks the wheel long-description wording
  against SUPPORTED.md (`pyreadme-xpath`).
- `install-smoke.sh` asserts `Device.xpath` + a canned `_xp_eval`
  on installed wheels, gated `VER>=0.6.3`.
- `docs/mcp/SUBMIT.md` documents republish-per-release; registry
  `0.6.1.post1` marked deprecated.

## Install

- `pip install pilotdroid` — console scripts `pilotdroid` /
  `pilotdroid-cli` lazy-download the matching binary from this
  release, verify it against `SHA256SUMS.txt`, cache, and exec.
- Or download a platform binary / `pilotdroid-cli-*` asset directly,
  or `brew install mofanx/tap/pilotdroid`.
