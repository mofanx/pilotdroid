# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

## [0.9.1] — 2026-11-28

### Fixed
- `serve` no longer dies at startup when the serial can't be resolved
    (zero or multiple ready devices, or adb unreachable): without `-s`
    it now starts UNBOUND — the MCP process stays up, `list_devices`/
    `select_device` work immediately, and every other tool errors with
    guidance ("no device selected …") until the client binds a device.
    Exactly one ready device still auto-binds. This is the missing
    other half of v0.9.0 device switching: MCP configs no longer need
    ANY serial binding — plug in any single phone and it just works,
    plug in several and the AI picks mid-session. Hard-failing here
    meant a binding-free config became a dead server whenever the wrong
    number of phones was attached.
- `select_device` binds correctly from the unbound state
    (`switched_from: null`) and a failed first bind stays unbound
    instead of inventing a stale fallback.

## [0.9.0] — 2026-11-28

### Added
- `select_device`/`list_devices` tools: mid-session device switching —
    rebinds the MCP session to any adb device in one call, tearing down
    the old connection and invalidating all per-device state (element
    snapshots/indexes, armed watchers, buffered events); the cleared
    inventory is reported in the response, connect failure reverts to
    the previous device, and snapshot consumption across devices errors
    explicitly (serial-tagged snapshots) instead of silently tapping
    the wrong phone. The spawn-time serial is now just the initial
    device — no more MCP-config edit + restart to switch phones.
- `docs/use-from-other-projects.md` — Devin/MCP 跨项目接入说明（用户级
  注册、`select_device` 会话内切换为推荐换机方式、免确认权限、盲驾四拍、
  排错速查）。

## [0.8.4] — 2026-11-28

### Added
- CI daily-build channel: every push to main uploads a release-profile
    linux-x86_64 `pilotdroid-cli` as the ci artifact
    `pilotdroid-cli-linux-x86_64` (14d retention);
    `spike/e2e/fetch-ci-cli.sh` downloads it and the real-device
    harnesses consume it via the existing `PILOTDROID_CLI` override —
    the local `cargo build` path is unchanged and remains the fallback
    for unpushed work. `fetch-ci-cli.sh` evicts run-id cache dirs
    beyond the newest 5 (~6.5MB each; was unbounded).

### Fixed
- eval-harness `Serve`: orphan `adb forward` rules left by `kill -9`ed
    serve processes are now swept at the next `Serve()` start via
    dead-pid ownership tags (untagged rules — fixed verify ports,
    manual forwards — are kept); `close()` cleanup unchanged.
- `doc-drift` changelog gate: a missing `## [Unreleased]` header now
    FAILs instead of crashing (v0.8.3 fold regression); PUBLISH.md
    documents keeping the empty header.

## [0.8.3] — 2026-11-26

### Fixed
- `tap_index`/`set_text_index` stale guard missed same-GEOMETRY
  different-content screens: the layout signature deliberately ignores
  text (a ticking clock must not invalidate a snapshot), so two screens
  with identical tree shape but different labels collided and
  `tap_index{i}` silently tapped whatever now sat in the slot — e.g.
  ApiDemos main list → App sublist. The guard now also verifies the tap
  target's element identity (class+text+desc) from the same dump —
  caught by the new blind-driving benchmark's stale_layout_guard.
- `d(scrollable=True).scroll.to(**kw)` was NIE (`AttributeError`) —
  implemented: forward-scroll until the selector target exists or the
  dump stops changing (u2 semantics); u2suite `test_scroll` path.
- eval harness poisoned-IME baseline: a SIGKILLed run leaves
  `default_input_method` pinned to `pilotdroid.eval/.NoIme`; the next
  run captured that pin as the "original" and restored it, cementing
  the contamination. Now detected at capture — restore is skipped and
  a warning tells the user to re-set the real IME manually.
- `screenshot_annotated` now returns the same-dump `elements` array
  (each element carries `id` = image label and `i` = tap space) so a
  labelled box resolves straight to `tap_index`/`set_text_index` — no
  second describe, and the id-vs-i footgun is closed at the payload level.
- `verify-agent.sh` `shot_quality` flake: q30-vs-q80 byte ordering can
  invert when screen content drifts between the two SHOTs. Now retries
  up to 3 back-to-back pairs and passes on the first valid pair.
- `Serve.close()` (agent-loop harnesses) now removes the adb forward
  rules created during its own lifetime — every spawned serve used to
  leak one `tcp:* → localabstract:pilotdroid_agent` rule per instance.
- `u2shim.dump_hierarchy()` retries empty/truncated dumps (a11y tree
  momentarily empty on ROM screen transitions → `ParseError: no
  element found`) — up to 3 attempts.
- `loop_v3.py` empty-selection crash: naming a portable-only task
  without `--portable` divided by zero in the summary — now exits 2
  with a usage hint.
- `benchmark.py` cross-OEM portability (found by the CFE6R HarmonyOS
  row, now 11/11): `stale_pkg_guard` used Xiaomi-only
  `com.miui.calculator` as the drift injection → `com.android.settings`
  + waits until the drift is observable to the agent (HarmonyOS
  app_current/a11y transition window); recurring old-targetSdk modal
  dismissed via `_dismiss_old_sdk_warn`; `stale_layout_guard` scroll
  injection falls back to nav when no scrollable node exists (tablets
  whose main list fits on one screen).

### Added
- `spike/e2e/agent-loop/benchmark.py` — frozen blind-driving benchmark
  (v1, 11 tasks): describe→tap atomicity, i-space divergence, B4
  checkbox+state predicate, stale guard (layout+pkg flavors), scroll,
  set_text, state-predicate verify, toast, dirty-kill recovery,
  match:{} negative pin. Deterministic targets (unlike eval v3's
  randomized draw) so release-to-release deltas are attributable;
  environmental fg-hijack retry marked `env=` never masks real fails.


## [0.8.2] — 2026-11-26

### Changed

- **观测面填同构快照——describe→tap_index 原子化**：`describe_screen`/
  `screenshot_annotated` 现以自身 dump 同构填充索引快照（溯源标
  `describe_screen`/`screenshot_annotated`）——`describe→tap_index{i}`
  在新鲜会话直接可跑，无 `list_elements` 前置拍、无 TOCTOU 窗
  （stale-guard 仍护观测→动作间漂移，报错带快照来源名）。interactive
  谓词单源化（`pilotdroid_agent::is_interactive`，facade/serve/som
  三处共用）。README 配方降回**四拍**，`SUPPORTED`/工具描述同步
  撤「需先 list_elements」注记。

### Fixed

- **`expect.state.match:{}` 空键显式报错**：此前静默 `found:false`
  （断言无从成立却不报错），现显式 tool error 提示需
  text|desc|class|i 其一。

## [0.8.1] — 2026-11-26

### Added

- **`describe_screen` 元素透出 interactive 空间 `i`**：每个元素
  在 SoM `id`（document-order，截图标注/中心点坐标空间）之外
  新增 `i` 字段——与 `list_elements`/`tap_index`/
  `set_text_index`/`verify_after.match.i` 同一编号空间
  （`clickable‖editable‖scrollable` 全树计数，含非 `useful` 的
  交互节点如裸 EditText）；非交互元素与 `ocr_only` 合成元素
  `i` 缺省（仅 `center` 可点）。compat `som.py` 镜像随行。
  根治盲驾 `describe→tap_index` 跨空间错配：README 四拍配方
  勘正为消费 `i` 字段，`SUPPORTED` 新增三编号空间对照表。

### Fixed

- **`expect.state.match.i` 索引空间修正**：此前 `i` 按全树
  （`interactive_only=false`）索引解析且短路其余 match 键——与
  host 可见的 interactive 编号空间不一致（`i` 错位/盲区）。
  现 `match.i` 与 `list_elements`（及 `describe_screen` 元素
  `i` 字段）同一 interactive 编号空间，且与其余 match 键做
  AND 组合（不再短路）；纯 `text`/`desc`/`class` match 仍搜
  全树（disabled/非交互节点是合法断言目标）。

### Internal

- **eval v3 nit 两件**（`spike/e2e/agent-loop/loop_v3.py`）：
  `random_index_pick`/`_settings_list` 加**前台包名门**——启动后
  `fg_pkg()` 校验目标 app 持前台，被 `com.miui.home` 类外部
  抢占先 `app_start` 恢复（≤3 次）再进入任务体；任务中遭遇
  抢占且结果失败时归因 fg≠目标再重试一次，环境性抢占不再记
  FAIL。`default_input_method` 置空钉**收敛为非 portable 档**
  （`--portable` Settings 子集不输入文本，非测试机不应被写
  secure 设置）；恢复路径打印还原后的 IME 基线值供审计。
- **eval portable 子集跨 OEM 修正**（`loop_v3.py`，华为
  MRR-W29/HarmonyOS-API31 实测驱动）：`som_shape` 断言对齐
  ElemInfo 真实 schema（i/class/bounds+≥1 交互标志，去掉从未
  存在的 `center`/`text` 强制）；元素标签收敛 `_label=text or
  desc`（HarmonyOS 行标签在 desc）；Settings 落地页不再假设
  MIUI 主列表；`settings_random_pick` 导航证据三路化（fg
  activity 变化/签名+数量齐变/选中行 label 前缀离场），开关与
  输入控件类移出随机候选（非测试机不翻转用户可见状态）；
  活页面（WiFi 扫描）`snapshot stale` 重选一次、exists 谓词
  连续两 miss 才记 FAIL。首个跨 OEM 数字：HUAWEI API31
  **15/15**、MIUI API34 **14/15**（ledger §附27）。
- **发布面与 ADR 注记**：`docs/PUBLISH.md` 增 Step 6b registry
  curl 复核路径（`limit` 截断曾致 deprecated 旧条被误判为
  最新；正确命令校验 `isLatest`+`active`）+「发件缺陷→patch
  窗口」流程注记；`docs/adr/008` 增 HarmonyOS API31 跨 ROM
  复测——uia 路径实测顶 **10.28fps**（cap 63ms+enc 31ms，
  理论 10.6 吻合），分流定案「中间态」：MIUI 最劣但非唯一墙，
  已测两机均不达 15fps，HarmonyOS 流水线化理论可跨 15fps。

## [0.8.0] — 2026-11-26

### Added

- **元素状态面**：`list_elements`/`describe_screen`/`compat/som.py`
  统一透出 widget 状态——`checked`/`selected`/`focused` 仅 true 时
  输出、`enabled` 仅 false 时输出（dump attr 直通、省 token）；
  `class` 口径全面对齐**简单类名**。`verify_after` 新增
  `expect.state` 状态谓词（`{match:{text|desc|class|i},
  checked|selected|enabled|focused:bool}`）——checkbox/switch/
  radio 闭环可证，「tap ok≠效果」分层成 ACK vs 效果断言。
- **`watch_dead` serve 映射**：agent ≥0.7.4 的
  `{type:"watch_dead"}` ui_event 转
  `notifications/pilotdroid/watch_dead`（armed→dead 不再静默），
  并对仍 armed 的规则做**有界 auto re-arm**（每规则每 epoch ≤2
  次防死亡风暴，params 带 `rearmed`/`new_watch_id`/`rearm_capped`
  /`rearm_failed`）；`watch_events` pull 返回
  `{type:"watch_dead"}` 同样带 re-arm 结果。
- **`describe_screen ocr:true` 可选档**：host 侧 tesseract TSV→
  行块→几何融合——块心落在 ≤4×块面积的无文本节点内回填
  `ocr_text`，无认领块转 `ocr_only` 可编号元素（center 可点，
  上限 40 块按置信度截断控 token）；tesseract 不在 PATH 时优雅
  降级为正常输出+`ocr.available:false` 提示，永不报错。

### Fixed

- **`max_fires` 非整收口**：`watcher.when().click()/call()` 的
  `max_fires` 现要求 int ≥1（此前 `1.5` 穿透校验）。

- **恢复 UX 闭环**：session 恢复（`Session::generation` epoch 变更）
  时 serve 推 `notifications/pilotdroid/session_recovered`
  `{generation,stale_watchers,rearmed,rearm_failed}`——崩溃/顶替不
  再静默；**armed watcher 自动 re-arm**（按存存的 selector/action/
  timeout 重发 ARM，watch_id 映射新分配，「崩溃即丢」升级为透明
  自愈），恢复代际的首个 hit 照推到 push（真机实证：pkill→dump
  恢复→`Add` hit notification 到达）。media conn 死掉时 demux 不
  再摘除 armed 规则——留给 epoch re-arm 自愈（warn 覆盖收窄洞修
  复，且每 streak 只 warn 一次）。
- **stream 激活期 stash→push**：帧流激活时 `recv_event` 分拣入
  session stash 的 watch_hit/toast 由 serve 每迭代排干并补发为
  `notifications/pilotdroid/*`——「push 暂停」档消除（真机实证：
  流中 `Add` hit 以完整 node payload 到达 notification）。
- **stale `watch_disarm` 穿透**：陈旧/未登记的 watch_id 显式
  disarm 不再透传至 agent——id 空间碰撞下可能误杀恢复后重分配的
  活 watcher；未知 id 本地返回 `{disarmed:false}`。
- **watcher pendOut ghost 判别**（agent 0.7.2→0.7.3）：media conn
  的 watcher 持有路径（pend）由「单 conn 最新者胜」改为 ≤4 conn
  持有集合——`uiEvent` 推送广播至每个持有 conn、写失败即判尸体
  丢弃（curWrote 同构：区分 ghost/reader 记日志），stray/backlog
  conn 不再挤掉活 watch reader；零送达时 hit 暂存（≤8/15s TTL），
  下一个 watch conn attach 时补发——覆盖 arm-attach 竞态与
  all-corpse 窗口，脏杀后 armed push watcher 命中不再静默丢。
  附：`ghost_repro.py --watch` 复现器（live-pend 保活+corpse 自愈
  两阶段）+ `verify-agent.sh` A8 双断言。诚实档：单笔写进半死
  adb-forward 管道可被 adbd 吸收成「假成功」——广播设计正是为
  消除对单笔写判死的依赖。
- **watcher 静默死可观测化 + event replay 环**（agent 0.7.3→0.7.4）：
  armed watcher 非正常终结（poll Throwable/会话终结强杀）临终推
  `{type:"watch_dead",watch_id,error}` ui_event 并记日志——armed→dead
  在两次 recovery epoch 之间不再静默（显式 `watch_disarm` 仍静默
  正常）。`ui_event` 改为**全量入 ≤8 条/15s TTL replay 环**、新
  attach 的 watch conn 补发未过期 backlog——修复 ghost 吸收写
  假成功时「真 conn 迟到则 hit 永久丢」的残洞（`delivered>0` 非
  活性证据的推论）。附：`ghost_repro.py --watch-dead` 复现器 3/3
  + `verify-agent.sh` A9 双断言（`watch_dead_event`、
  `watch_hit_stash_replay`）。

### Internal

- **eval v2 难度爬升**（`spike/e2e/agent-loop/loop_v2.py`）：7 个
  高难任务（跨 app Settings 往返、`list_elements`→`tap_index`
  真消费 SoM 编号、故意 stale snapshot 被拒+重验证、界外滚动
  寻元素、双 toast 时序、每轮 SIGKILL 故障注入+重流、后台
  resume）× N≥5 轮输出成功率+p50/p95——真机 35/35，全部任务
  p95−p50≤1.6s（ledger §附17）。
- **真模型盲驾 round**（`blind.py`）：仅 describe/tap/verify_after
  三工具 5 任务 4/5——失败归因：SoM 无 checked/selected 状态面
  （checkbox 类闭环不可证）+真实零售机前台抢占；并记录 tap ok
  ≠效果回执、describe/list 类名口径差两项 nit。
- **OCR host-spike GO**（`ocr_spike.py`）：screenshot→本地
  tesseract→行块与树几何融合→`ocr_only` 可编号元素——canvas
  文本场景（ApiDemos TextAlign）8 树节点外产出 13 个 OCR 元素，
  文本场景零重复污染（ledger §附17）。
- **eval v3**（`spike/e2e/agent-loop/loop_v3.py`）：v2 全集+2 个
  **随机化区分度**任务（`random_index_pick` 随机主表项 tap_index、
  `random_checkbox_state` 随机 CheckBox 索引+`verify_after` state
  谓词断言翻转）；失败记录带**前台包名归因**（`fg=`，外部 app
  抢前台不再静默）；`stale_i` 缺省 0 修正为显式报错；eval 期
  `default_input_method` 置空防 IME 抢触（finally 恢复）；v2
  继承任务全面加 scroll-aware 导航（a11y 只吐已渲染节点）。
  真机 ba0d072d **27/27**；`--portable` Settings 子集无 ApiDemos
  依赖用于多设备矩阵行（ledger §附21）。

## [0.7.1] — 2026-11-25

### Added

- **Toast MCP push notification**：agent ≥0.7.1 a11y listener 推送的
  `{type:"toast"}` ui_event（与 `WATCH_HIT` 共用 ch=1 msg_type
  0x0002 槽位，按 `type` 字段判别）经 serve push demux 分发为
  `notifications/pilotdroid/toast` MCP notification；非 push 路径
  降级为 `watch_events` pull 返回 `{"type":"toast",...}`。真机
  实证：ApiDemos `PopupMenu1` → `Clicked popup menu item Add`
  通知到达 stdout。

### Fixed

- **stream 期间 watch_hit/toast ui_event 丢失**：帧流与 ui_event
  共享 ch=1 conn——`stream_read` 的 `media.recv()` 只认 FRAME，命中
  事件要么触发 `UnexpectedResponse` 断流、要么随 conn teardown 静
  默丢失；armed push watcher 先占 media conn 时 `stream_start` 又
  复用 agent 已在启动时杀掉的陈旧句柄。修复：`stream_start` 先
  `teardown_media`；`stream_read`/`stream_stop` 改走 `recv_event`
  分拣，非帧事件入 session stash，`watch_recv` 先排 stash——hit
  流中与停流后均可经 `watch_events` 拉出（eval
  `stream_watch_coexist` 真机实证）。facade 单测
  `stream_demux_stashes_watch_hit_and_toast` 钉交错解码语义。
- **session 恢复后 watch_id 碰撞窗**：`Session::generation()`
  epoch 计数——agent 重启后 watch_id 从头重分配，serve `armed`
  map 里的陈旧 id 会误指新 agent 的同号 watcher（disarm 错杀）；
  epoch 变更即清 `armed` 并丢弃缓冲的过期 `WATCH_HIT`。单测钉
  「恢复→stale disarm 不达新 agent」+真机 pkill 恢复实证。
- **session-watchdog 测试 flaky 硬化**：`dead_port()`（bind 即放）
  释放的端口可被恢复阶梯的 `free_port()` 复用为 relay 端口——
  relay 上游拨号拨回自身成自连死循环（CI 曾见首红 rerun 绿）；
  改常驻 accept-close `corpse_port()`；并行测试抢端口时 relay
  `bind` 遇 AddrInUse 由 panic 改为返回 Err 让阶梯换端口重试。
- **media 幽灵 conn 竞态——脏退出后首 `stream_read` 失败**（agent
  0.7.2，`Agent.java` Streamer）：host 脏杀遗留的 media conn 滞留
  accept backlog，新流 `start()` 只清已 accept 的 pending——幽灵
  在 start 之后才 accept，于 `running && curSock==null` 下被无
  判别 promote，真 reader 被 busy-close→首读 EOF；且旧流 writer
  的写失败 `mediaDead→halt` 可跨代绞杀刚启动的新流，accept 线程
  还会因 `attach` 异常永久退出。修复：attach 死 conn
  `getOutputStream` 失败即弃；运行中第二 conn 进 reserve 槽
  （newest waiter wins，不再 busy-reject；stray conn 不抢位）；
  `mediaDead(g)` 代际守卫挡旧 writer；`curWrote` 判别——从未成功
  写过的 conn 死后丢回 waiting 态等真 reader（ghost），写活过的
  才判真 reader 死亡 halt（§4.8 语义保留）；accept 线程 attach
  异常 log+continue 永生；`writeLoop`/`produce` 钉代际号+局部
  queue 引用。e2e 断言 `stream_reserve_hold`/`stream_reserve_heal`/
  `stream_ghost_drop`+`spike/e2e/agent-loop/ghost_repro.py` 统计
  钉判别语义：脏杀+RST 幽灵注入 20/20（修复前 0~13/20 间歇失败）。

## [0.7.0] — 2026-11-25

### Added

- **Session 看护——agent 崩溃透明自恢复**（Track B，`crates/
  pilotdroid-agent` `Session::with_client`，atx-agent 语义）：控制
  通道 op 遇死 socket（读 EOF/reset、`Protocol(Io)` 写失败）→
  自动跑 watchdog 阶梯（停尸→重拉 daemon→复用同端口 re-forward→
  HELLO 握手）→原位重放本次 op——调用侧「崩溃后下一次 op 前已
  重启」，不再暴露 `Io` 错误。边界明示：`Timeout` 不恢复（挂起
  vs 死亡不可判定，盲重放会双执行）；重放幂等限定——请求可能在
  conn 断前已下发，副作用 op 存在双执行理论窗口；armed watcher
  与 stream 是 agent-session-bound，崩溃即丢失（media 侧
  `watch_recv` 遇死连接先治愈控制通道再原样抛错，调用方 re-arm；
  watch_id 重新分配，陈旧 id 可能撞新 id）。恢复阶梯穷尽时报
  `Recover{stage}` 具名错。真机实测：`pkill -f pilotdroid.Agent`
  后下一 `describe_screen` 1.11s 透明成功（含 800ms 拉起 settle），
  再次 op 恢复 0.02s 常态。

- **Track B 四 ❌ 面清零/证伪**（t15，`compat/u2shim.py` +
  `agent/java` 0.7.1 + `SUPPORTED.md`）：
  - `d.toast.get_message(timeout,default)`/`.last`/`.reset` ✅
    ——agent 注册 `UiAutomation.setOnAccessibilityEventListener`，
    `TYPE_NOTIFICATION_STATE_CHANGED` → media ch 推
    `{type:"toast",pkg,has_parcel,text[]}`；真 toast 无 parcel
    与通知区分。u2 同事件机制。真机实证：ApiDemos `PopupMenu1`
    菜单点击 → `Clicked popup menu item Add`。与运行中 watcher
    互斥（media 单连接，`RuntimeError` 明示）。
  - `d.current_ime()`/`d.set_ime(id)`/`d.ime_list()` ✅——
    `settings get secure default_input_method` + `ime set/list
    -s`，真机切换 round-trip 验证。
  - `d.screenrecord(output,time_limit)` ⚠️——u2 同 shell 机制
    ctx-manager（后台录制→SIGINT 收尾→pull），链路可用但
    MIUI API34 编码器节流 ~1-3 帧/4s（`screenrecord --verbose`
    自报），产出近静态 mp4——API ✅、录屏实效 ⚠️，与 ADR-008
    流式墙同根因。
  - `d.clipboard` ❌ NIE 证伪落地——`UiAutomationConnection` 无
    app `Context`（`systemMain()` 自杀，ADR-008）；宿主
    `service call` 未公开 parcel / `cmd clipboard` 未实现 /
    `dumpsys` 静默三路实证全死，SUPPORTED 附复现命令。

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
