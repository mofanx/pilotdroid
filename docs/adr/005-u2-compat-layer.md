# ADR-005 · u2 兼容层：host 侧翻译层（JSON-RPC → PD01 帧协议）

> 状态：已接受 · 日期：2026-11-19
> 范围：M3 u2 drop-in 兼容（docs/05 M3 验收线）

## 背景

M2 已证明协议层收益（batch 序列 10.5×、TEXT 6.6×），但 u2 用户
**拿不走**——没有兼容层，benchmark 优势不转化为采用。u2 客户端语义是
per-call JSON-RPC（HTTP POST `/jsonrpc/0`），需要一个把 u2 调用翻译为
PD01 帧协议的兼容面。

## 硬约束：UiAutomation 单持有者

实测（RESULTS-M2 §生态发现）：我方 agent 持有 UiAutomation 连接时，
u2 `AutomatorServiceImpl` 启动即崩 `UiAutomation not connected`——
**同一设备同一时间只能有一个 UiAutomation 持有者**。任何"再跑一个
u2 服务进程做翻译"的方案直接出局。

## 候选

| 方案 | 说明 | 结论 |
|---|---|---|
| A. agent 内嵌 JSON-RPC 端口 | dex 内再起 HTTP/JSON-RPC server | **否**：agent 体积/复杂度膨胀（内置 HTTP server + u2 语义映射全进 dex，每次 API 迭代都要重发 jar）；app_process 无网络库依赖也徒增风险 |
| B. **host 侧翻译层** | Rust/Python HTTP server 收 u2 JSON-RPC → 翻成 PD01 帧 → 单连接复用 | **选** |
| C. host 进程内嵌 u2 server + 常驻 agent | 同 B 但要求 host 常驻 | 否：兼容层应是按需启停的进程，不要求常驻形态 |

## 决定

**host 侧翻译层（B）**：一个独立进程（首版 python `u2shim.py`，复用
`agent_client.py`）监听 u2 的 JSON-RPC 端口，把调用映射为 PD01 op，
通过持久 socket 复用连接转发给 agent。u2 客户端零改动。

- 天然满足"单 UiAutomation 持有者"——翻译层只是 agent 的另一个客户端。
- 翻译层挂掉不损 daemon；agent 零改动，wire 契约不变。
- per-call 开销 = 本地 HTTP 解析 + 复用连接 RTT（实测 ~2-45ms 抖动），
  仍远优于 u2 原生 249-700ms 路径。

## v0 API 覆盖面（顶层 ~10 项）

| u2 调用 | 映射 op |
|---|---|
| `click(x,y)` / `click(text=…)` | INJECT tap / QUERY→bounds→INJECT |
| `swipe(x1,y1,x2,y2,dur)` | INJECT 事件序列（§4.7） |
| `dump_hierarchy()` | DUMP |
| `screenshot()` | SHOT（scale 可选） |
| `set_text(text)` | TEXT |
| `app_start(pkg)` | agent shell `am start`（host 侧直发 adb，不过帧） |
| `wait(selector, timeout)` | BATCH[wait_until] |
| `exists(selector)` | QUERY found |
| `press(key)` | KEY op（§4.8） |
| `ping/health` | PING |

选择器映射：`{text,id,desc,index}` 四元组直接平移；u2 的
`className`/`packageName`/`xpath` 超集字段 v0 不映射（返回明确错误，
不静默降级）。

## 后果

- 翻译层是 v0 python 原型：吞吐/并发要求低，先验证 API 面与对拍数字；
  性能敏感场景可由 Rust `pilotdroid-compat` 重实现（同一映射表）。
- "同口径声明"规则（CONTRIBUTING）：对拍数据必须注明摊销口径
  （持久连接）vs u2 per-call 口径。
