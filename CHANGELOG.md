# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **SoM 感知 MCP 工具面**（Track A·M29，`crates/pilotdroid-cli` 新
  `som.rs`——编号空间与 `compat/som.py` 逐字一致）：`describe_screen`
  返回 `{elements, text}`（id↔center↔label/rid/class/C）；
  `screenshot_annotated` 返回叠同编号框的 JPEG（image crate 纯 Rust
  解码/绘制/编码，agent.jar 零改动；退化/倒置 bounds 归一化+零面积
  跳过，与 Python 侧同策略）。真机 ba0d072d 实测：30 工具列表、
  annotated 276KB JPEG 出图。

- **watch push demux**（MCP notification 帧通道——推翻 M26 defer，
  合同授权）：`watch_arm {push:true}` 命中即发
  `notifications/pilotdroid/watch_hit`（serve 单写者循环，notification
  与 response 帧结构性无交错；非 push 命中缓冲降级 `watch_events`
  可拉，PENDING_CAP=64 防膨胀；stream 激活期间 demux 暂停——帧与
  命中共享 ch=1，命中由 agent 侧暂存待拉）；client stdin EOF/写失败
  → 自动 disarm 全部 armed 规则（agent 侧 conn close 亦有 reap，
  双保险）。真机实测：armed watch_id=9 命中 notification 到达；
  EOF 后 watch_disarm 复验 `disarmed:false` 无残留。

- **`verify_after` 闭环验证工具**（u2 无此能力——超越点）：
  `{action:{tool,args}, settle_ms, expect}`——跑一个 op→settle→
  新 dump 断言 `exists`/`absent`/`text`/`layout_changed`，返回
  `{action, checks, pass}`；action 白名单限副作用/读 op（禁
  batch/verify 递归与 stream/watch 生命周期）。真机实测
  `press HOME → layout_changed:true → pass`。

- **xpath-lite → 常见面全量求值器**（Track B，`compat/xpath_eval.py`
  新模块，stdlib-only——dump 树即「融合树」前置形态，SoM 融合后
  复用同一求值器）：axes `child`/`descendant`/`ancestor(-or-self)`/
  `parent`/`self`/`following-sibling`/`preceding-sibling`、谓词
  `[@a='v']`/`[@a]`/`[@a!='v']`/`contains`/`starts-with`/`ends-with`/
  `not(...)`、`and`/`or`（标准优先级）、`[n]` 位置谓词（1-based，
  per-context）、union `|`——原 NIE 面转实现；`last()`/`position()`/
  `text()`/非 sibling `following::`/`preceding::`/数值比较仍显式
  NIE（SUPPORTED.md 附清单，xpath 行升级「常见面无星号平齐」）。
  `parent(x)` 谓词参数、`when(xpath)` host-poll 规则自动受益。
  真机 ba0d072d：`/node[1]` 位置、`ancestor::*[@resource-id]`、
  `following-sibling::node` 命中并点击落点导航进 Animation 子菜单；
  `when(xpath|xpath).click()` 新语法规则触发导航进 Accessibility。

### Fixed

- **xpath_eval 两缺陷返工**（t9 实现复盘实锤）：
  ① `[@a!='v']` 谓词静默空集——lexer 分隔符集漏 `!`，`a!`
  被粘连为属性名致恒假；现 `!`/`=` 正确词法化为不等谓词，
  且遵循 XPath 语义「缺失属性不匹配」（区别于
  `not(@a='v')`）。② `//x[n]` 位置语义——原实现取扁平
  document-order 第 n 个（`//node[2]` 得 `['A']`），修为
  XPath spec per-parent（各 parent 的第 n 个 x 子节点，
  `//node[2]` 得 `[B,R,D]`）。单测钉 per-parent/`!=` 正反
  +无空格写法+bare `!` fail-fast；parity 真机补 `!=` 命中断言。

- **SoM annotated 倒置/退化 bounds 崩溃修复**（M28 复核实锤）：
  真实 dump 含倒置 bounds（MIUI 屏底裁切区，实测单屏 27/97
  元素）→ PIL rectangle `ValueError: y1 must be greater than or
  equal to y0`。`screenshot_annotated` 绘制前 min/max 归一化
  +零面积框跳过；编号空间不变（`_enumerate` 同一调用）。parity
  补 `test_som_screenshot_annotated_no_throw` 真机断言堵覆盖洞。

### Added

- **SoM-lite 宿主侧感知奠基**（Track A·M28，`compat/som.py` 新模块，
  **shim 面先行**——MCP serve 工具暴露留后续波次）：
  `describe_screen(dev)` 产出 token 预算友好的编号映射
  （id↔bounds 中心↔text/desc/rid/class/clickable，doc-order
  确定性编号，仅保留有属性或交互标志的节点）；
  `screenshot_annotated(dev)` 在 JPEG 上叠同编号空间框+标签
  （PIL 惰性导入，无 Pillow 时报可操作错）。parity 真机断言
  `test_som_describe_screen_consistency` 钉元素集与原始 dump
  独立重过滤的一致性。

- **`stream-stop` 抓帧观测面**（M28 spike 沉淀）：ack 新增
  `{cap, cap_nulls, cap_ms, enc_ms}`——cap 后端名（当前
  `uia`）、takeScreenshot null 计数、抓帧/编码分相均值。

### Docs

- ADR-008 收录 MIUI API34 高帧抓帧面 spike 实录（7 路径全测
  证：uia 5.6fps 为唯一可用源上限；VD 镜像含 `screenrecord`
  同构全被 ROM 封锁）——supersede ADR-006 的帧率语境假设，
  media 通道设计不变。

## [0.6.4] — 2026-11-25

### Fixed

- **`watcher.when("//xpath").click()` 静默失效**（M26——`_xpath`
  哨兵系 post-0.6.3 main 的 7193b94 引入，未发件即在
  2b5a9fe 修复）：`_xpath` 哨兵键被传入 `_Selector` →
  `ValueError` 被 watcher 循环吞掉 → 点击永不执行且规则被
  摘除。现 xpath 规则在命中时 eval xpath、点击首个匹配节点
  中心；match 与 action 之间节点消失（弹窗已关）为静默
  no-op 不杀规则。
- **`Device()` 无 serial 多设备静默失败**（M26，v0.6.3 wheel
  真实携带）：`adb forward --list` 行解析丢弃 serial →
  `serial=None` 下 adb 全调用命中 "more than one
  device/emulator" 静默 rc=1 空输出。现 `_detect_port` 返回
  `(serial, port)`：唯一 forward 行 backfill（u2 `connect()`
  首台语义）、多台都有 forward → 指引性错误、无 forward →
  `adb devices` 恰一台 adopt/多台报错；serial 显式给定且无
  该机 forward 行时跳过 17932 兜底探测（防 conn 与 `-s`
  跨机错配）。

### Added

- **watcher bounded re-arm**（M25）：规则默认仍为 fire-once，
  但 action 可挂 `_pd_max_fires` 预算多次命中再摘除——
  `_BUILTIN_DIALOGS` 转三元组，LBE 多页审查规则预算 3
  （fixture-doctor `page<8` 循环同一实测依据）。
- **`when("//xpath")` host-poll 规则**（M25）：非 wireable
  但 xpath-lite 可求值的 xpath 从 NIE 降级为 `_xpath` 哨兵
  规则——每 tick 一次 dump 由 `_XPathLite.exists` 判定；
  连 lite 子集都拒绝的表达式仍 fail-fast NIE。
- **`call/click(max_fires=N)` 公开参数**（M26）：用户 watcher
  规则的 re-arm 预算——`when(...).call(fn, max_fires=N)` 与
  `.click(max_fires=N)` 透传 `_pd_max_fires`（xpath 规则路径
  与 builtin `_WatchContext` 走同一通道）；`max_fires<1` 抛
  ValueError。

### Changed

- **skip 宣称升级**（M25）：SUPPORTED.md gap 段从"8 skip 全
  上游标记"精确为"上游 skip 的 4 面（implicitly_wait/枚举/
  count/watchers）shim 已实现并有单测覆盖"。

### Docs

- `install-smoke.sh` 默认 `VER`→0.6.3；`docs/PUBLISH.md`
  checklist 新增 Required「smoke VER bump」步；
  `RESULTS-M22-release.md` 补 v0.6.3 发布链 ledger。
- M26 内部批：`doc-drift.py` 新增 smoke-ver 闸
  （install-smoke `VER` 默认值==`pyproject` version）；新增
  `spike/e2e/test_parity_extra.py` 真机断言 4 面
  （implicitly_wait 往返/selector 枚举/count 一致/watcher
  call 命中）；README 加 u2 定位段（API 子集+verbatim
  22/0/8 实测+偏差见 SUPPORTED）。

## [0.6.3] — 2026-11-25

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
  10s 倒计时竞速 / LBE ReviewPermissions 单页 fire-once
  「确定」/ NotificationShade 收起）。动作走 host-side inject
  +settle/retry——FLAG_SECURE 弹窗会吞 device-armed a11y
  ACTION_CLICK（真机实证，非推测）。
- **`spike/e2e/u2suite/fixture-doctor.sh`**：fixture 一键恢复——
  `pm path` 幂等检测 → ApiDemos 缓存下载/push → `adb install
  --bypass-low-target-sdk-block` 期间 agent a11y 自动点
  AdbInstallActivity「继续安装」→ `monkey` 首启循环点 LBE
  审查「确定」（ReviewPermissions 为多页流程，循环直至
  LBE 退前台，上限 8 页）。conftest guard 报错消息指向
  该脚本。

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
- **doc-drift 扩展 + install-smoke shim 断言**（M24）：
  `spike/e2e/doc-drift.py` 新增 `pyreadme-xpath` 检查——wheel
  长描述口径与 SUPPORTED.md 交叉验证；`install-smoke.sh`
  新增 `Device.xpath`/`_xp_eval` 罐头 eval 断言，`VER≥0.6.3`
  门槛化（0.6.2 wheel 无 xpath-lite 不误红，≥0.6.3 拦打包
  回归）。
- **registry `0.6.1.post1` 已标记 deprecated**（回读实证）；
  `0.6.2` 保持 active 至本版发布。

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
