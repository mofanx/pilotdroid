# RESULTS-M11 — 交付链断点补齐（进行中）

日期：2026-11-20。设备：ba0d072d（USB，唯一测试设备）。agent 0.6.0。

## 1. 目标

M10 交付物的断点补齐：异步 WATCH（u2 watcher 对等语义）、conn 侧
APP_CURRENT（~ms 级替代 adb spawn ~500ms）、serve 工具面 pinch/watch、
macOS quarantine 解、SUPPORTED 免费二批、韧性尾巴。

## 2. serve 面（本文件负责部分）

| 项 | 结果 | 证据 |
|---|---|---|
| `pinch_in`/`pinch_out` 工具 | ✅ tools/list 21→24 | `mcp_client_check.py` 握手回归 PASS |
| `watch` 工具（阻塞语义，§4.9） | ✅ timeout 行为正确 | tools/call 实测 `matched:false, waited_ms:955` |
| pinch 端到端 | ✅ 44 事件注入 `ok:true` | MCP tools/call `{"events":44,"ms":32,"ok":true}` |
| MCP 握手回归 | ✅ | tools (24) list + 实调通过 |

注：`watch` 保持阻塞 op；异步面由 `watch_arm`/`watch_disarm`/
`watch_events` 三工具补齐（tools 24→27），hit 经 media 通道
`{type:"watch_hit",hit:{...}}` 下发。

真机端到端（ba0d072d，MCP tools/call）：
- watch_arm → `{armed:true,watch_id:1}` 秒回
- armed 期间 `exists` 48ms（control 不阻塞，异步语义实证）
- 触发 resolver → watch_events 收 `{matched:true,action_ok:true,node:{...}}`
- watch_disarm 命中后为 `disarmed:false`（一次性 watcher 命中即摘除，语义正确）

### verify flake 修复（本轮复核发现）
`watch_parallel` 断言 36/1：机制钉死为 **transition stall**——
`keyevent HOME` 后 0.8s settle 小于本机窗口过渡（实测 0.8s→par 2.2s、
1.5s→0.2s）。sleep 0.8→1.5，2000ms 阈保留（仍区分真阻塞）。

## 3. 分发面

| 项 | 结果 |
|---|---|
| macOS quarantine | README `xattr -d` 兜底行 + `mofanx/homebrew-pilotdroid` tap（Formula 指向 v0.6.0 asset，sha256 待 tag 后回填） |
| dependabot triage | GHA bump（checkout v7/setup-python v7/upload-artifact v7/download-artifact v8）并入 feature/m11-dist；PR1 merged，PR2/3/5 superseded 关闭；PR4 adb_client 4.0 关闭 → issue #6 + dependabot ignore major |
| v0.6.0 release | draft notes 已备（pinch/WATCH/自愈/2.6× 公平臂/quarantine 注），待 publish |

## 4. 待 t1/t3 补齐（占位）

- [x] 异步 WATCH（t1）：WATCH_ARM/DISARM + ui_event WATCH_HIT；serve 侧 arm/disarm/events 三工具（本节）
- [x] shim watcher device-async + 并行对拍（t1）
- [x] conn APP_INFO + hot-path（t1）
- [x] SUPPORTED 免费二批（本节②：6 项 ✅ + instance alias；clipboard/toast 保 ❌）
- [x] multiPoints 守卫 + pinch clamp + shell 分层注（t1）
- [x] verify 37/37（含 transition-stall settle 修复）

## 5. 已知边界（沿 M10）

- `watch`/`pinch` 需要 caps MULTI_PTR/WATCH（agent≥0.6.0），旧 agent 报
  协议错误 —— shim 侧有 host-poll 回退，serve 侧直接透传错误。
- WATCH action_ok≠hit-test（点击注入成功≠命中目标 UI）——M10 已注。
