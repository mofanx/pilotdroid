# RESULTS-M12 — UX/资源洞 + host-filter 批（t1 部分）

日期：2026-11-20。设备：ba0d072d（USB）。agent 0.7.0，caps 0x7ff。

## 1. watch UX/资源包（t5 复核发现的坑 → 闭环）

| 项 | 修复 | 实测 |
|---|---|---|
| `watch_events` 静默 = isError | 超时映射正常 union `{type:"timeout"}`（isError 留给真错误） | MCP 真调：5.2s → `{"type":"timeout"}` isError:false |
| armed watcher 无上限 | agent `MAX_WATCHERS=8` → `ERR_LIMIT` | 真机：8 条 armed，第 9 条 `{code:"ERR_LIMIT"}` |
| poll_ms 下限 | watchArm `Math.max(50,...)`（全树 DFS 持全局 a11y 锁，更密轮询饿死 QUERY） | — |
| serve 顶替静默 | dead-conn 分支 stderr warn（stdout 仍纯 JSON-RPC） | serve 测试输出实证 |

## 2. verify 锁屏 flake 根治

`screen_off_timeout` 全程钉 MAX + EXIT trap 恢复原值——覆盖脚本中途
锁屏（此前 wakeup+swipe 只管开头，实测运行中 MIUI 仍会自动回落锁屏，
造成 selector 假阴性整类 flake）。

## 3. media forward 端口固定化

`attach_media` 无 hint 时优先 `control_port+1`（占用回退 free_port）。
被杀的 host 最多留 **1 条可预测孤儿**，不再累积随机集合。实证：MCP
serve 测试残留 `tcp:34263`(control)+`tcp:34264`(media=+1) 整对。
存量孤儿（43713/33045/17933）已清。

## 4. selector host-filter 批（compat 面最后一刀）

含 wire 不可表达键时 `_Selector` 整体切 dump+post-filter（`_nodes()`）：

- **谓词键**：`textContains`/`textStartsWith`/`textMatches`、
  `descriptionContains`/`descriptionStartsWith`/`descriptionMatches`、
  `classNameMatches`/`packageNameMatches`/`resourceIdMatches`、
  attrs `clickable`/`checkable`/`checked`/`enabled`/`focusable`/
  `focused`/`selected`/`longClickable`
- **方法**：`.count`（真计数）、`.wait_gone(t)`、`.wait(t)` host 路径、
  relative `.left/.right/.up/.down/.child/.sibling`
- **安全**：`_sel()` 对 host 键硬错（防静默丢键→空 selector→匹配一切）；
  watcher 两路径按 `_wireable()` 分流——host 键规则回退 host 轮询。
- **口径注**：wire QUERY 只走 active window；dump 路径含全窗口树
  （更贴近 u2）。SUPPORTED.md ❌ 仅剩 `xpath`（非目标）。

真机抽测（Settings 页）：`textContains`/`textMatches`/`textStartsWith`/
`clickable`/`sibling`(真值1=shim1)/`up`/`down`/`wait_gone` 全对；
compat_m6_realscripts 复跑 **23 ok / 1 unsupported(xpath) / 0 error**。

## 5. live-view demo

`spike/e2e/live-view.py`：stream 帧 → MJPEG `http://127.0.0.1:8080/`。
实测 6s 收 31 帧（~5fps，与 stream_start(fps=5) 口径一致），README
benchmark 行已挂引用。

## 6. 验证全绿

- `verify-agent.sh ba0d072d` → **37/37 PASS**（锁屏钉 + restore 生效，
  全程无 flake）
- `cargo test --workspace` 全绿（+1 新测试
  `watch_events_quiet_media_reports_timeout_union`，静默 media→timeout
  union）；clippy/fmt 净

## 7. dist/doc 防线（t2）

| 项 | 落地 | 证据 |
|---|---|---|
| doc-drift guard | `spike/e2e/doc-drift.py`：serve.rs tools() 工具名集 ↔ README `N tools` 断言；u2shim 四类（Device/_Selector/_Watcher/_Scroll）公开方法 ↔ SUPPORTED.md 全文交叉断言（IGNORE={close,running}，shim 内部件非 u2 面）；挂 docs-smoke step4 + ci.yml python job | 实证：README 改前 21→FAIL，修 27→PASS；方法侧 0 missing |
| README 修正 | 21 tools→27（现行值）；serve 段补"两实例并存 ping-pong 夺权"明示句 | doc-drift PASS |
| 版本对齐 | workspace + python/pyproject 0.5.0→**0.6.0**（与 draft tag v0.6.0 一致；lock 同步）；AGENT_VER=0.7.0 为协议内部版本、与发布版本独立演进（CHANGELOG 注） | `cargo metadata` 全 member=0.6.0；release.yml verify-tag 会强校验 tag==workspace==pyproject |
| Windows matrix | release.yml +`windows-latest / x86_64-pc-windows-msvc` → `pilotdroid-cli-windows-x86_64`（.exe）；bin 名入 matrix 字段；rusb 为 vendored（源码 cc 编译，无系统 libusb 依赖） | dispatch 实跑见 ci run 记录 |
| MCP notification spike | `spike/e2e/mcp-notify-spike.py` + `mcp-notify-spike.json`：(A) in-proc echo server 发 `notifications/message` → 真 ClientSession `logging_callback` 实测收到（`ServerNotification` 经 message_handler 亦可见）——**推通道协议内可达**；(B) 真 serve capabilities 仅 `tools`，无 logging cap——现状纯 pull | 结论：watch_hit 保持 pull（watch_events）为契约；notification 只能当 log 面（非结构化 tool 事件），tooling 仍需轮询取 payload。可选：未来 hit 镜像为 logging notification 供人类观察 |
