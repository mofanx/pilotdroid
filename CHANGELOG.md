# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
