# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **u2shim xpath-lite**（M23，官方套件 verbatim **9 xpath fail 全数
  转正 → 22 pass / 0 fail / 8 skip** 达可达上限）：`d.xpath(x)` 改
  为 host-side 近似子集 `_XPathLite`/`_XEl`（~210 行 stdlib-only，
  每查询一次 `dump_hierarchy`+ElementTree）——text 简写 /
  `%模糊%` / `@resource-id` / 裸 `pkg:id/name` / `//*[@attr]` /
  `contains()` / `parent()` / `scroll_to` / `all()`/`get()`/`exists`/
  `wait()`/`click()`/`get_text()`/`info`/`swipe`；高级轴/位置轴/
  and-or 谓词显式 `NotImplementedError`。watcher `when()` 支持
  `//*[@text|@resource-id|@content-desc|@class|@package=...]` 1:1
  wire 翻译，非 wireable xpath 显式 NIE。
- **`%` LIKE 三态语义**（M24）：`v%`→startswith / `%v`→endswith /
  `%v%`→contains 按 u2 SQL-LIKE 语义精确实现（不再一律坍缩
  contains——此前为超集匹配偏差）。
- **`implicitly_wait` getter/setter 双态 + selector 迭代**（M24）：
  `d.implicitly_wait()` 无参返回当前值；`_Selector` 实装
  `__len__`/`__iter__`/`__getitem__`（skip 掩蔽面补齐）。
- **`watch_context(builtin=True)` MIUI dialog recipes**（M24）：
  u2 通用规则外加 MIUI 规则集（`AdbInstallActivity`「继续安装」
  10s 倒计时竞速 / LBE ReviewPermissions 多页「确定」/
  NotificationShade 收起）。
- **`spike/e2e/u2suite/fixture-doctor.sh`**：fixture 一键恢复——
  `pm path` 幂等检测 → ApiDemos 缓存下载/push → `adb install
  --bypass-low-target-sdk-block` 期间 agent a11y 自动点
  AdbInstallActivity「继续安装」→ `monkey` 首启自动点多页 LBE
  审查「确定」。conftest guard 报错消息指向该脚本。

### Changed

- **Homebrew tap 升格发布必做步骤**（docs/PUBLISH.md）：formula
  bump 从 follow-up 改为 Required（v0.6.2 发布时 tap 曾滞留
  0.6.1，已由本次 bump 实证修复并验证 sha256）。
- **xpath 口径文档修正**：`xpath` 从「不支持/非目标」改为
  「host-side 近似子集（非 u2 插件管线，高级轴 NIE）」——
  README.md / python/README.md / compat/SUPPORTED.md 三处同步。
- **registry 重发流程文档化**（docs/mcp/SUBMIT.md）：
  draft→published 状态、republish-per-release 步骤、`login github
  -token` fallback（registry JWT 过期 401→re-login 实证）。

### Docs

- `spike/e2e/RESULTS-M22-release.md`：v0.6.2 发布链 ledger
  （tag/7 assets/OIDC 首绿/smoke 8:8/环境项）。
- `spike/e2e/RESULTS-M17-u2suite.md`：fixture 恢复后复跑定稿
  （13/9/8）+ M23 xpath-lite 落地复跑（22/0/8）+ 独立复验附记
  + AdbInstallActivity 10s 自动拒绝机制与 a11y 解法。
- `spike/e2e/u2suite/README.md`：MIUI USB 安装确认弹窗机制说明。

## [0.6.2] — 2026-11-24

### Added

- **u2shim G8 Session 对象模型 + G10 settings 全语义**（M21，openatx
  官方套件 verbatim 8→12 pass）：`session()` 返回 Session 对象
  （`running()` 惰性崩溃检测、ctx-mgr `__exit__` 停 app、attach）、
  `SessionBrokenError`；`settings` 已知键类型校验
  （`xpath_debug`/`wait_timeout`/`operation_delay`）、`wait_timeout`
  与 `_Selector.wait` 联动、`operation_delay` 操作间隔、未知键透明
  dict 透传。剩余 10 fail = 9 xpath（明示不支持）+ 1 `watch_context`
  （环境敏感，非代码缺陷）。
- **u2shim watcher 错误可观测面**（M22）：`_Watcher.last_error` —
  `_run_async` 的 `media_connect` 拒绝/帧解码失败不再静默死线程，
  异常写入 `last_error` 并使 `running` 变 False。
- **u2suite conftest fixture 预检 + 设备前置**（M22）：session 级
  fixture 查 `pm path io.appium.android.apis` + `mWakefulness`/
  keyguard，坏则秒报 fixture broken（不再 20s 超时）；run.sh 加
  wakeup + `cmd statusbar collapse` 前置。

### Fixed

- **`Device()` 消费 `PILOTDROID_SERIAL` 环境变量**（serial bug4）：
  显式 `serial=` 优先、env 兜底——此前多设备环境下 adb 路径静默空
  serial。单测钉死两级优先级。
- **`_XPathStub.__getattr__` 改抛 `AttributeError`**（`__call__` 仍为
  `NotImplementedError`）——`hasattr(d.xpath, 'x')` 探测不再被
  NIE 击穿。
- **`app_wait` 进程判定兜底**：`pidof` 落空时扫 `ps -A -o PID,NAME`
  匹配 `pkg`/`pkg:<sub>`（自定义 `android:process` 进程名场景，
  u2 走 running-tasks 而非 pidof）；`app_current().pid` 同路径统一。
- **release.yml**：存在 `docs/release-notes-<tag>.md` 时用
  `--notes-file`，仅缺失时才回退 `--generate-notes`——修掉 curated
  notes 被 dependabot-only 生成稿覆写的坑。
- **install-smoke.sh**：安装前轮询 `pip index versions`（12×5s 有界
  等待），并修掉 `pip -q` 静默列表输出导致索引可见性误判的问题。

### Documentation / infra

- **MCP registry 首次发布**：`io.github.mofanx/pilotdroid@0.6.1.post1`
  （README 加 `mcp-name` 所有权标记；`cli_bootstrap._tag()` 剥
  `.postN` 使 post 版仍解析主 release tag）。
- **`docs/PUBLISH.md` 发布 runbook**：五步用户闸 + 预发布 checklist；
  明确 tag=发布基准≠HEAD 语义；Step 6 记录 registry 快照不跟随、
  每次 release 需重跑 `mcp-publisher publish`。
- **GitHub 环境 `pypi` + trusted publisher 已配**（owner/repo/workflow/
  environment 四元组）——0.6.2 起 PyPI 发布走 OIDC 全自动。
- **分支保护启用**：main 强制 4 required checks（rust/python/
  agent-jar-sync/agent-jar-repro）。
- `RESULTS-M17-u2suite.md`：`watch_context` 归因定稿——fixture app
  缺失 + 锁屏/Shade 遮罩两层环境失败（当前环境 3/3 稳定复现，
  watcher arm/hit 链路已证完好，非代码缺陷）。

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
