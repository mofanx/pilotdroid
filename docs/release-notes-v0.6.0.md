# pilotdroid v0.6.0 — release notes draft

> Paste into the v0.6.0 GitHub Release body. Source of truth for the
> publish step; keep in sync with `main` until the tag lands.

Device-side UiAutomation agent for Android: single-jar dex under
`app_process` + `pilotdroid-cli` (agent.jar embedded — one binary does
setup/serve/bench) + `import pilotdroid` uiautomator2 drop-in shim +
stdio MCP server (27 tools).

## What's new (M10–M13, agent 0.7.0 / caps 0x7ff)

**Async WATCH — event-driven watchers that don't block (CAP_WATCH_ASYNC)**
- `WATCH_ARM`/`WATCH_DISARM`; hits arrive as `WATCH_HIT` on the media
  channel — control ops stay fully usable while armed. Measured: 40 ops
  during an armed watcher, **op p50 = 43ms** (same magnitude as unarmed).
- Shim `d.watcher.when(...).click()` now uses device-async semantics on
  agent ≥0.7.0 (u2 watcher-equivalent); older agents fall back to
  blocking WATCH, then host polling.
- MCP serve: `watch_arm`/`watch_disarm`/`watch_events` tools. Limits:
  max 8 armed watchers (`ERR_LIMIT`), `watch_events` returns
  `{type:"timeout"}` instead of an error on quiet media.

**Multi-pointer gestures (CAP_MULTI_PTR)**
- `pinch_in`/`pinch_out` in shim and serve. Shim clamps to display and
  rejects fully off-screen bounds (MIUI ghost bounds → LookupError).

**conn-side APP_INFO (CAP_APP_INFO)**
- `app_current`/`app_wait` resolve the foreground app over the agent
  conn — **~51ms vs ~96ms** adb spawn (~2×, low variance, no fork).

**Near-complete u2 compat surface**
- SUPPORTED.md ❌ is down to **`xpath` only** (explicitly out of scope).
  New this release: `app_wait`/`session`/`send_keys`/`open_notification`/
  `open_quick_settings`/`freeze_rotation`, `instance` alias, host
  post-filter keys (`*Contains`/`*StartsWith`/`*Matches`, attr filters,
  `.count`/`.wait_gone`, relative `.left/.right/.up/.down/.child/.sibling`),
  u2-parity `index` (XML child-position) vs `instance` (Nth-match).
  Real-script re-run: **23 ok / 1 unsupported(xpath) / 0 error**.

**Resilience & ops**
- Dead/replaced control conn self-heals (lazy reconnect, ADR-007);
  conn-replaced events warn on stderr (stdout stays pure JSON-RPC).
  One serve per device — two instances ping-pong conn ownership.
- `verify-agent.sh`: lockscreen-proof runs (screen_off_timeout pinned +
  restore), transition-stall settle, hijack retry — **37/37 green**.

## Benchmarks (ba0d072d, USB)

| scenario | pilotdroid | u2 | |
|---|---|---|---|
| watcher popup auto-heal (appear→dismiss, same-arm) | ~680ms | ~1790ms | **2.6×** |
| detect+act floor, hot conn | **39ms** | — | device-side WATCH |
| same-script e2e (M7) | — | — | **8.8× USB / 4.7–7.4× TCP** |

## Assets

`pilotdroid-cli-linux-x86_64` · `pilotdroid-cli-macos-arm64` ·
`pilotdroid-cli-windows-x86_64.exe` · `agent.jar` ·
`pilotdroid-0.6.0-py3-none-any.whl` + sdist · `SHA256SUMS.txt`

- **macOS**: unsigned — `xattr -d com.apple.quarantine pilotdroid-cli`,
  or `brew install mofanx/pilotdroid/pilotdroid`
- **Windows**: unsigned — SmartScreen "More info → Run anyway", or
  `Unblock-File` in PowerShell. **Compile-verified only** — the build
  passes CI but hasn't run on a real Windows machine; report issues.
- **PyPI**: `pip install pilotdroid`

**Full Changelog**: https://github.com/mofanx/pilotdroid/compare/v0.5.0...v0.6.0
