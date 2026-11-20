# ADR-007 · host 调用面：Rust facade + `serve` stdio 工具层

> 状态：已接受 · 日期：2026-11-20
> 范围：M5 调用面（docs/05 M5 验收线——"天然适合 agent 调用"）

## 背景

M1-M4 把设备侧能力全部打通（注入/树/截图/帧流/u2 shim），但对**使用者**
仍只有一条窄路：`pilotdroid-cli` 单发子命令——每次调用 =
forward+connect+HELLO ≈ +60ms 固定成本，且输出是给人看的 JSON 行，
不是给程序用的类型化 API。两个真实缺口：

1. **库形态缺失**：Rust/Python 调用方要么 spawn CLI 进程（慢+脆），
   要么自己重实现 proto；
2. **agent 调用面缺失**：AI agent 调用工具的工业标准是
   stdio JSON-RPC（MCP 风格一行一请求）——没有它，"适合 agent
   调用"只停留在口号。

## 候选

| 方案 | 结论 |
|---|---|
| A. 只补 Rust lib facade | 不够：非 Rust 调用方（python agent 框架主流）仍需进程面 |
| B. 只做 stdio serve | Rust 调用方仍 spawn 进程，失去类型与内嵌能力 |
| C. **facade 为核 + serve 为薄壳** | **选**：一套 Session API，两层壳共用，无第三套逻辑 |

## 决定

### 1. `pilotdroid` Rust facade（新 crate 或 agent crate 顶层 re-export）

```rust
let dev = Device::connect(serial_or_none)?;        // adb + watchdog + HELLO
let s = dev.session()?;                             // 持久 control conn
s.tap(x, y)?; s.swipe(..)?; s.press(keycode)?;
s.set_text(..)?; s.dump()?; s.query(sel)?;
s.wait_for(sel, timeout)?;                          // BATCH wait_until 封装
s.screenshot(quality, scale)?;
let mut st = s.stream(fps, scale, quality)?;        // MediaClient 所有权
while let Some(f) = st.next()? { .. }               // 迭代器形态
s.stop()?;                                          // 流停止+forward 清理
```

- 所有 op 走单条持久 session（摊销口径）；`Drop` 自动断流+清 forward
- 错误面收窄：`AgentError` 直通，不向调用方暴露 req_id/frame

### 2. `pilotdroid-cli serve` —— stdio JSON-lines 工具层

**一行一请求/响应**（JSON-RPC 风格，非 MCP 依赖——零外部框架）：

```jsonl
→ {"id":1,"tool":"tap","args":{"x":700,"y":1600}}
← {"id":1,"ok":true,"result":{"events":2,"ms":4.2}}
→ {"id":2,"tool":"wait_for","args":{"text":"OK","timeout_ms":2000}}
← {"id":2,"ok":true,"result":{"found":true,"node":{..}}}
← {"id":3,"ok":false,"error":{"code":"TIMEOUT","message":"..."}}
```

**v1 工具清单**（与 u2 shim API 对齐，便于同一 agent 双后端）：

| tool | args | 底层 |
|---|---|---|
| tap / long_click / swipe | x,y / duration | INJECT |
| press | keycode(+meta) | INJECT kind=Key |
| set_text | text, clear | TEXT |
| exists / query | text/id/desc/className/scrollable/index | QUERY |
| wait_for | sel+timeout_ms | BATCH wait_until |
| dump | — | DUMP XML |
| screenshot | path, quality, scale | SHOT |
| scroll | dir(fwd/back/begin/end), sel | swipe-in-bounds |
| app_start / app_stop / app_current | pkg | shell am/dumpsys |
| stream_start / stream_read / stream_stop | fps,scale,quality / n | §4.8 |
| batch | ops[] | BATCH |

- 单进程单设备：`--device`/首连指定；session 常驻，首请求自动
  watchdog connect
- 帧流工具返回 base64 JPEG（stdio 无二进制通道），`stream_read n`
  批量拉帧；大帧走 `screenshot{path}` 文件旁路惯例

### 3. shim v1 扩面（已定稿，实现见 feature/m5-agent-shim）

`_SUPPORTED` 增 `className`/`scrollable`；scroll 对象、long_click、
app_stop、app_current。选择器 `class`/`scrollable` 字段已入 wire
（agent 端），proto Selector 同步扩——**wire 变更随本 ADR 冻结**。

## 后果

- wire 层零新增 opcode：全部工具复用现有 op 组合
- serve 输出即工具协议文档——`serve --list-tools` 自描述
- u2 shim 与 serve 共用"工具面"概念：shim=进程外 HTTP 翻译，serve=
  stdio 原生工具——两条路服务两类调用方，device 侧同一 session 后端

## 补注（M6 实测）· 多 serve 并发语义

`spike/e2e/serve_multi.py` 实测（2×`serve` 同设备）：

- 每个 serve 进程持有**自己的 adb forward 与 control 会话**（free-port
  选取，无端口冲突）——初始化/普通 op 各自可用、互不排队；
- 帧流仍**单 owner**：第二 serve `stream_start` 得 `ERR_STREAM_BUSY`；
- owner serve 进程被 SIGKILL（模拟崩溃）→ 设备侧 socket EOF →
  implicit stop（§4.8）→ 幸存 serve 立即可 `stream_start`——
  **崩溃韧性是 session-binding 自带的**，无需心跳/租约。

即：多 client 并发=设备端多会话并存，工具级互斥只有 stream 一处，
且以"先占先得 + owner 死即释放"为语义。

## 补注（M8 修订）· 单 client 顶替语义

M6 的"多会话并存"实测暴露跨进程 op 可在同一 agent 上交错——对
session-bound 状态（elements 快照、stream owner）是未定义行为。
AGENT_VER 0.5.0 起改为**单 client 顶替**（last-connector-wins）：

- accept 新 control conn → 旧 conn `shutdown+close` → 其 session 与
  stream 按 §4.8 一并终结（注意：`close()` 不打断阻塞 read，必须
  `shutdownInput/Output` 先发 FIN）；
- 被顶替的 serve 保留死 session：后续 conn op 持续 io error，
  **进程重启才能夺回**（serve 不自动 re-dial，刻意拒绝 dial-steal
  ping-pong——静默抢锁比显式失败更难调试）；
- owner serve SIGKILL → 全新 serve 立即接管（崩溃韧性不变）。

`serve_multi.py` 8 断言复跑全过（dump 探活——`app_current` 走 adb shell
不经 control conn，不能作活性探针）。stream 的 ERR_STREAM_BUSY 语义
随之过期：现在**永远轮不到第二 owner**——连接层先被顶替。

## 补注（M10 修订）· TOCTOU 窗口 + 被顶替方自愈

**语义边界修正**：last-connector-wins 的保证是 **"accept 之后无交错"**，
而非"TCP connect 之后无交错"。`connect()` 返回 ≠ 顶替完成——新 conn 先
进内核 accept 队列，agent 的 `accept()` 异步处理后才关闭旧 conn；该
TOCTOU 窗口内旧 conn 仍可完成 ~1 个 op（实测 adb-TCP + stream 负载下
`ping` 滑窗 ~20%）。socket 层无法消除此窗口，**调用方不得假设
connect 返回即刻独占**；依赖"我connect了所以对方已死"的代码是错的。

**serve 自愈**：原"进程重启才能夺回"放宽——`serve` 的 `tools/call`
命中 `AgentError::Io`（socket EOF/reset）时将 session 置空，**下一次
call 惰性重连**并按 last-connector-wins 重新夺权。失败的那次 call 仍
显式返回 tool error（不是静默重试）；两个活跃 serve 互相调用退化为
per-call 夺权（语义仍安全——每 op 原子，代价是 ~一次 HELLO 开销）。
