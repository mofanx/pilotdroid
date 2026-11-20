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

注：`watch` 当前为阻塞 op（占用 control 会话至 matched/timeout）；
t1 异步 arm/disarm 落地后语义升级对 MCP 客户端透明（schema 已按
selector+action+timeout_ms 对齐）。

## 3. 分发面

| 项 | 结果 |
|---|---|
| macOS quarantine | README `xattr -d` 兜底行 + `mofanx/homebrew-pilotdroid` tap（Formula 指向 v0.6.0 asset，sha256 待 tag 后回填） |
| dependabot triage | GHA bump（checkout v7/setup-python v7/upload-artifact v7/download-artifact v8）并入 feature/m11-dist；PR1 merged，PR2/3/5 superseded 关闭；PR4 adb_client 4.0 关闭 → issue #6 + dependabot ignore major |
| v0.6.0 release | draft notes 已备（pinch/WATCH/自愈/2.6× 公平臂/quarantine 注），待 publish |

## 4. 待 t1/t3 补齐（占位）

- [ ] 异步 WATCH：ui_event(ch=1) WATCH_HIT 帧 + arm 期间并行 op 实证
- [ ] shim watcher device-async 语义 + "并行驱动"对拍臂
- [ ] conn APP_CURRENT/APP_INFO op + hot-path 实测
- [ ] SUPPORTED 免费二批（app_wait/session/send_keys/clipboard/...）
- [ ] multiPoints cnt=0 守卫 + pinch bounds clamp + shell-op 分层注
- [ ] verify-agent.sh WATCH/pinch 断言后 33+/33+

## 5. 已知边界（沿 M10）

- `watch`/`pinch` 需要 caps MULTI_PTR/WATCH（agent≥0.6.0），旧 agent 报
  协议错误 —— shim 侧有 host-poll 回退，serve 侧直接透传错误。
- WATCH action_ok≠hit-test（点击注入成功≠命中目标 UI）——M10 已注。
