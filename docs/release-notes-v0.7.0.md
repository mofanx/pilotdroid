# pilotdroid v0.7.0

Device-side UiAutomation agent for Android: single-jar dex under
`app_process` + `pilotdroid-cli` (agent.jar embedded — one binary does
setup/serve/bench) + `import pilotdroid` uiautomator2 drop-in shim +
stdio MCP server (**30 tools**).

## What's new in 0.7.0 (M28–M30, Track A + Track B)

**Track A — AI-native perception & closed loop**

- **`describe_screen` + `screenshot_annotated` (SoM-lite)** —
  token-budget-friendly element map (id ↔ bounds-center ↔
  text/desc/rid/class/clickable, deterministic doc-order numbering,
  useful-node filter) plus a JPEG with the same ids drawn as boxes.
  Python (`compat/som.py`) and Rust (`som.rs`) share one numbering
  space. Annotated draw normalizes inverted/degenerate bounds
  (MIUI emits them — 27/97 on one real screen) instead of raising.
- **MCP push notifications — `notifications/pilotdroid/watch_hit`**
  demultiplexes armed-watcher hits to the connected client
  (single-writer serve loop, 150ms poll; bounded `VecDeque` fallback
  stays pullable via `watch_events`; EOF/write-failure disarms all
  rules). Streaming pauses demux — events buffer agent-side until
  stream stop.
- **`verify_after`** — run a whitelisted action, wait, then assert
  `exists`/`absent`/`text`/`layout_changed` in one call — the
  act→verify closed loop u2 doesn't have.
- **Agent 0.7.1** — `UiAutomation` a11y event listener pushes
  `TYPE_NOTIFICATION_STATE_CHANGED` on the media channel (toast
  detection backend) + stream capture observability counters
  (`cap`/`cap_nulls`/`cap_ms`/`enc_ms` on stream-stop).
- **ADR-008** — MIUI API34 capture-wall evidence: all seven shell-
  reachable frame paths measured; `takeScreenshot` ~5.6fps is the
  only usable source and the physical ceiling on this ROM
  (`screenrecord` virtual-display path included — see below).
  Supersedes ADR-006's fps assumptions.

**Track B — u2 parity & beyond**

- **xpath-lite → common-surface evaluator** (`compat/xpath_eval.py`,
  stdlib-only): `child`/`descendant`/`ancestor`/`ancestor-or-self`/
  `parent`/`self`/`following-sibling`/`preceding-sibling` axes,
  `[n]` per-parent positional predicates, `and`/`or`/`not`,
  `contains`/`starts-with`/`ends-with`, `@a='v'`/`@a!='v'`/`@a`,
  union `|`. Unsupported constructs still fail fast with
  `NotImplementedError` — approximation, not full XPath.
- **Session watchdog — atx-agent semantics**: a dead control socket
  (`Io`/EOF/`Protocol(Io)`) runs the recovery ladder (teardown →
  re-forward same port → restart → reflash-if-needed → HELLO) and
  replays the op transparently; `Timeout` deliberately not recovered
  (hung vs dead undecidable; blind replay = double-execute). Armed
  watchers/stream are session-bound and lost — `watch_recv` heals
  the control conn then surfaces the error for caller re-arm.
  Measured on ba0d072d: pkill → next op succeeds in ~1.3s.
  (Rust/MCP surface; Python shim is unchanged — see limits.)
- **`d.toast.get_message/last/reset`** — real toast detection via
  the agent's a11y events (`has_parcel` filters notifications).
  Verified: ApiDemos PopupMenu1 → `Clicked popup menu item Add`.
- **`d.current_ime()` / `d.set_ime(id)` / `d.ime_list()`** —
  settings + `ime` shell surface, round-trip verified.
- **`d.screenrecord(path, time_limit)`** — context-manager wrapper
  around `/system/bin/screenrecord` (start → SIGINT → pull).
- **`d.clipboard`** — **ROM/architecture-proven unreachable on
  ba0d072d**: the agent owns a `UiAutomationConnection`, not an app
  `Context` (`ActivityThread.systemMain()` self-kills, ADR-008); all
  three host paths measured dead — `service call clipboard` opaque
  parcels, `cmd clipboard` unimplemented, `dumpsys clipboard` silent.
  get/set raise `NotImplementedError` with the evidence inline.

**Honest limits (same as `compat/SUPPORTED.md`)**

- `screenrecord`: API works, output is ROM-throttled —
  `screenrecord --verbose` self-reports ~3 frames in 4 seconds on
  MIUI API34 (near-static mp4). Same root cause as ADR-008.
- `clipboard`: see above — ❌ with reproducible evidence, not a bug.
- xpath: common surface covered; `last()`/`position()`/`text()`/
  non-sibling `following::`/numeric comparisons still NIE.
- Session recovery lives in the Rust/MCP path only — `u2shim`'s
  `agent_client` still surfaces dead-conn errors lazily.
- No blanket fps guarantee beyond the measured ~5.6 ceiling.

**Verified**

- pytest 92/92; real-device ba0d072d parity **12/12** (incl. real
  toast text, IME round-trip, annotated-screenshot validity).
- `cargo test --workspace` 91/0; verify-agent **37/0**
  (agent 0.7.1, CAPS 0x7ff).
- uiautomator2 vendored suite: 22 passed / 0 failed / 8 skipped.
- Session recovery on-device: pkill → transparent op success ~1.3s.
