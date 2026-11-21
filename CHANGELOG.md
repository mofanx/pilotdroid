# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.6.1]

### Added

- **u2shim API 面补齐批**（openatx 官方套件 verbatim 0/27→8/30 pass）：
  `settings` 透明 dict、`uiautomator2.exceptions` 桩、
  `Session`/`watch_context`/`get_text`/`app_start` front-wait、`click`
  隐式等待、`swipe` host-sleep、`push`/`pull`（BytesIO）、`wlan_ip`、
  `app_list_running`、`app_auto_grant_permissions`、`app_current.pid`、
  `_WhenCtx.call`、Device ctx-mgr。defer 口径：G6 jsonrpc、G8 session
  对象模型、G10 settings 全语义（SUPPORTED.md 登记）。
- **shim 三个真 bug 修复**（套件暴露）：watcher 零规则秒退（keep-alive
  到显式 stop）、`_Selector.wait` 长占共享控制 conn 改 ≤1s 分片、
  `_WhenCtx.call` 回调被误 arm 成 device-side click 改 `action:"none"`
  观察态。
- **whl lazy-bootstrap CLI**: `pip install pilotdroid` now ships
  `pilotdroid-cli`/`pilotdroid` console scripts that download the matching
  release binary on first run — sha256-verified against the release's
  SHA256SUMS.txt (refuses to exec on mismatch, cache-hit re-verified),
  cached under `~/.cache/pilotdroid/<tag>/`, `os.execv` argv passthrough.
  Unblocks the MCP registry `uvx` path.
- `pilotdroid-cli` also accepts `PILOTDROID_SERIAL` as a fallback when
  `-s/--device` is absent (declared in the MCP registry manifest).
- CI: `agent-jar-repro` job — rebuilds `agent.jar` from source
  (javac 25 + build-tools 37.0.0 + android-34, pinned) and asserts
  byte-identity with the committed jar; upgrades jar-sync from
  commit-ordering to dex provenance.
- README: live-view demo GIF (`docs/assets/live-view.gif`, real-device
  frame-stream capture).
- `docs/mcp/server.json` registry manifest draft (schema-validated) +
  `docs/mcp/SUBMIT.md` submission notes.

### Changed

- `adb_client` 2.1.15 → 4.0.0 (server/server_device API restructure);
  shell exit codes preserved via a trailing `__PD_EXIT_` marker wrapped
  in a subshell (tolerates `&`-terminated commands).
- `install-smoke.sh` exercises the real bootstrap chain
  (`pilotdroid-cli --version` inside the venv), not just `gh download`.
- compat `u2shim`/`SUPPORTED.md`: selector environment-sensitivity notes —
  zh-locale for text/desc, first-launch flow-state and MIUI app-lock
  (FLAG_SECURE) for rid/main-UI misses.

### Fixed

- verify-agent: `shot_scale` deterministic failure — the batch tap opened
  an app behind MIUI's app-lock gate; KEYCODE_BACK after `batch_results`;
  `agent_client` shot prints the agent's error payload instead of a bare
  `KeyError`.
- Pip on-ramp dead-end: u2shim's failure hint no longer references a
  `pilotdroid setup` command that doesn't exist in the wheel.

## [0.6.0]

Versioning note: `AGENT_VER` (device jar, 0.7.0) is the internal protocol
version and moves independently of release versions — bumps happen on any
wire/agent change, not on every release.

### Added

- **Multi-pointer gestures**: `pinch_in`/`pinch_out` (protocol `points[].pid`,
  `CAP_MULTI_PTR`) in agent, shim (`_Selector.pinch_*` with display clamp +
  off-screen visibility check) and MCP serve.
- **Async WATCH** (`CAP_WATCH_ASYNC`, §4.9): `WATCH_ARM`/`WATCH_DISARM` +
  `WATCH_HIT` pushed on the media channel — armed watchers don't block
  control ops (armed-period op p50 = 43ms, same magnitude as unarmed).
  Shim `watcher` uses device-async semantics on agent ≥0.7.0; serve gains
  `watch_arm`/`watch_disarm`/`watch_events` (armed hits pre-attached to the
  media conn — no silent hit-loss window; empty selectors rejected).
- **conn-side APP_INFO** (`CAP_APP_INFO`, §4.10): `app_current`/`app_wait`
  resolve the foreground app over the agent conn (~51ms vs ~96ms adb spawn).
- **Serve resilience**: dead control conns (replaced by a newer conn) now
  self-heal — next call lazily reconnects and succeeds; conn-replaced events
  log a stderr warning (stdout stays pure JSON-RPC).
- **u2shim coverage (M11+M12)**: `app_wait`/`session`/`send_keys`/
  `open_notification`/`open_quick_settings`/`freeze_rotation` + `instance`
  alias; host post-filter selector keys (`*Contains`/`*Matches`/
  `*StartsWith`, attrs, `.count`/`.wait_gone`, relative
  `.left/.right/.up/.down/.child/.sibling`) via dump+post-filter. `xpath`
  remains the only explicitly-out-of-scope item.
- **Distribution**: macOS arm64 + Windows x86_64 release assets;
  Homebrew tap `mofanx/homebrew-pilotdroid`; PyPI publish workflow
  (OIDC trusted publishing on release publish); `live-view.py` MJPEG demo.
- **doc-drift guard** (`spike/e2e/doc-drift.py`, in docs-smoke + CI):
  README tool count ↔ serve.rs, shim methods ↔ SUPPORTED.md.

### Fixed

- `verify-agent.sh` flakes: window-transition a11y stall settle (0.8→1.5s),
  foreground-hijack retry, screen-off timeout pinned to MAX during runs.
- `watch_events` timeout returns a normal `{type:"timeout"}` union instead
  of `isError`; watcher limit `MAX_WATCHERS=8` → `ERR_LIMIT`; media channel
  pins to `control_port+1` (bounded forward orphans).

## [0.5.0] — first public release

### Added

- **Device-side agent** (`agent/agent.jar`, embedded in `pilotdroid-cli` via
  `include_bytes!`): single-process Java dex running under `app_process` —
  HELLO/INJECT/DUMP/SHOT/PING/QUERY/BATCH/TEXT/STREAM over a length-prefixed
  msgpack control protocol (`docs/02-protocol.md`). Input injection via
  `InputManagerGlobal` with per-device capability probing and degrade chain;
  accessibility-backed DUMP/QUERY/TEXT (native Unicode, no IME).
- **`pilotdroid-cli`**: `agent` lifecycle subcommands (start/stop/install/query/
  ops), `setup` — an 8-step doctor (device probe, forward hygiene, embedded-jar
  deploy, inject probe, u2/atx conflict warning) — and `serve`, a stdio MCP
  server exposing 21 tools including indexed `list_elements`
  (`interactive_only` default) + `tap_index`/`set_text_index` with a
  snapshot-staleness guard, and `stream_start/read/stop` JPEG frame pull.
- **`pilotdroid-agent` Rust facade**: `Device`/`Session` API with watchdog
  tiered recovery and auto-reflash of the embedded jar when the remote copy is
  missing.
- **uiautomator2 drop-in shim** (`pip install pilotdroid` → `import
  pilotdroid.u2shim`): near-full non-xpath surface — selectors, scroll, watcher
  (host-poll thread), shell/app_list/app_info/window_size/screen_*/unlock/
  orientation/clear_text, `wait()` returning bool. Method-level matrix:
  `compat/SUPPORTED.md` (xpath explicitly out of scope).
- **Single-client agent semantics (M8)**: one control connection at a time,
  last connector wins — cross-process op interleave is impossible
  (ADR-007 amendment). Media channel session-bound to its control conn.
- **JPEG frame stream**: dedicated `localabstract` media channel, ≥5fps
  sustained with zero control-channel blocking (ADR-006); preview-tier spike
  data in `spike/e2e/m8-preview-fps.json`.

### Performance (dual-device verified, `agent/scripts/verify-agent.sh` 33/33)

- Same-script wall-clock vs real uiautomator2: **8.8× USB**, **4.7–7.4×
  adb-TCP** (range is op-mix sensitive — `spike/e2e/RESULTS-M7.md`).
- Per-op vs u2 baseline: click ~5×, batch `tap→wait→tap` 10.5×, TEXT 6.6×.

### Notes

- One device = one control client (deliberate): a second `serve`/shim conn
  replaces the first — see ADR-007.
- Python package also installs a `pilotdroid-agent` console entry point
  (raw protocol client CLI).
