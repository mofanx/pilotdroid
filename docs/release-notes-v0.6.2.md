# pilotdroid v0.6.2 — release notes draft

> Paste into the v0.6.2 GitHub Release body. Source of truth for the
> publish step; keep in sync with `main` until the tag lands.

Device-side UiAutomation agent for Android: single-jar dex under
`app_process` + `pilotdroid-cli` (agent.jar embedded — one binary does
setup/serve/bench) + `import pilotdroid` uiautomator2 drop-in shim +
stdio MCP server (27 tools).

## What's new in 0.6.2 (M20–M22)

**u2shim: G8 Session + G10 settings — verbatim suite 8 → 12 pass**
- `session()` now returns a Session object: `running()` (lazy crash
  detection — polls `app_current` per call), ctx-manager `__exit__`
  stops the app, `SessionBrokenError` on dead sessions.
- `settings` full semantics: known-key type validation
  (`xpath_debug`/`wait_timeout`/`operation_delay`), `wait_timeout`
  coupled into `_Selector.wait`, `operation_delay` between ops,
  unknown keys pass through as a transparent dict.
- Remaining verbatim fails: 9 xpath (explicit non-goal) + 1
  `watch_context` — attributed to environment (fixture app missing +
  lockscreen/Shade overlay; reproduces 3/3 in our lab while the
  watcher arm/hit path itself is verified sound — not a code defect).

**u2shim hardening**
- `Device()` now consumes `PILOTDROID_SERIAL` as fallback (explicit
  `serial=` wins) — previously adb calls silently ran serial-less in
  multi-device setups.
- `hasattr(d.xpath, 'x')` works again: `__getattr__` raises
  `AttributeError` while `__call__` still honestly raises
  `NotImplementedError`.
- Watcher media-channel failures are observable: `watcher.last_error`
  records `media_connect` refusals / frame-decode deaths instead of a
  silently dead thread; `watcher.running` turns False.
- `app_wait`/`app_current().pid` no longer rely on `pidof` alone —
  falls back to a `ps -A -o PID,NAME` scan matching `pkg`/`pkg:<sub>`
  (apps with custom `android:process` names).
- `app_current().activity` is documented as a design boundary: the
  agent exposes package but not the a11y activity name — `None` is
  expected, don't depend on the key.

**u2suite reproducibility**
- conftest session-fixture now prechecks `pm path
  io.appium.android.apis` + wakefulness/keyguard — a broken fixture
  fails in seconds instead of a 20s timeout. `run.sh` wakes the
  device and collapses the status bar before running.

**Release-chain fixes (M20)**
- `release.yml`: curated `docs/release-notes-<tag>.md` wins via
  `--notes-file`; `--generate-notes` only when absent.
- `install-smoke.sh` polls `pip index versions` (bounded ~60s) before
  installing — PyPI index propagation no longer races the smoke run.
- PyPI trusted publishing configured (GitHub OIDC,
  `mofanx/pilotdroid`/`pypi.yml`/`pypi`) — this release's upload is
  fully automated, no manual twine.
- `docs/PUBLISH.md` runbook: five user-gated steps + pre-publish
  checklist; tag = artifact baseline ≠ main HEAD.
- MCP registry: `io.github.mofanx/pilotdroid` is registered
  (published at `0.6.1.post1`; entry bumped to 0.6.2 with this
  release — the registry is a snapshot, `mcp-publisher publish` must
  re-run every release).
- Branch protection: 4 required checks on `main`.

## Assets

`pilotdroid-cli-linux-x86_64` · `pilotdroid-cli-macos-arm64` ·
`pilotdroid-cli-windows-x86_64.exe` · `agent.jar` ·
`pilotdroid-0.6.2-py3-none-any.whl` + sdist · `SHA256SUMS.txt`

- **macOS**: unsigned — `xattr -d com.apple.quarantine pilotdroid-cli`,
  or `brew install mofanx/pilotdroid/pilotdroid`
- **Windows**: unsigned — SmartScreen "More info → Run anyway", or
  `Unblock-File` in PowerShell. **Compile-verified only** — the build
  passes CI but hasn't run on a real Windows machine; report issues.
- **PyPI**: `pip install pilotdroid`

**Full Changelog**: https://github.com/mofanx/pilotdroid/compare/v0.6.1...v0.6.2
