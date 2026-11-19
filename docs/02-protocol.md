# 02 · RPC 协议设计（草案 v0）

> 状态：草案 · 日期：2026-11-19
> 关联：adr/003、01-architecture.md

## 1. 设计目标

| 目标 | 对 u2 体系的改进点 |
|---|---|
| 单连接复用 | u2 每调用一次 HTTP/JSON-RPC 往返；本协议一条长连接承载全部控制+媒体流 |
| 二进制分帧 | JSON 序列化/解析开销与体积都砍掉 |
| 批量 + 设备端条件 | "tap→等出现→tap" 打包下发，N 次往返 → 1 次 |
| 双向流 | agent 主动推事件（帧、UI 变化、心跳），host 不必轮询 |
| 可兼容 u2 | 同一 agent 另开 JSON-RPC 端口，python uiautomator2 客户端直接对拍 |

## 2. 帧格式

```
 0        4        8              16                24        32
+---------+--------+----------------+-----------------+--------+
|  magic  | ver |ch|  type (u16)   |   flags (u16)   | req_id  |
| "PD01"  |     |  |               |                 | (u32→)  |
+---------+--------+----------------+-----------------+--------+
|        req_id (u32)             |   payload_len (u32)        |
+--------------------------------+-----------------------------+
|                        payload (msgpack)                    |
+--------------------------------------------------------------+
```

- **magic `PD01` + ver**：版本协商失败即拒连，便于后续演进。
- **ch（channel, 2bit）**：0=control，1=media（截图/帧流/事件推送），2=diag。
- **type**：见 §3 opcode 表。
- **flags**：bit0=req/resp，bit1=stream-more（批量中间结果），bit2=error。
- **req_id**：请求-响应配对；批量结果沿用同一 req_id 多次推送。
- **payload**：msgpack（首版选型：Rust `rmp-serde` 零拷贝成熟、Java 侧 `msgpack-core` 纯 Java 无依赖；FlatBuffers 留作 v0.2 评估项）。

**socket 名**：agent 监听 `localabstract:pilotdroid_agent`（单一事实源 = `pilotdroid_agent::AGENT_SOCKET` 常量与 Java `Agent.SOCKET_NAME`，两侧必须逐字符一致）。

## 3. Opcode 表（控制通道）

| type | 名 | 方向 | 语义 |
|---|---|---|---|
| 0x0001 | HELLO | h→a | 协商：协议版本、capability bitmap 请求 |
| 0x0002 | HELLO_ACK | a→h | capability bitmap + agent 版本 + 屏幕参数 |
| 0x0003 | PING / PONG | 双向 | 心跳（默认 5s，3 次丢失判死） |
| 0x0010 | INJECT | h→a | MotionEvent 序列：tap/swipe/pinch；wait_finish 标志 |
| 0x0011 | INJECT_RESULT | a→h | 每事件返回 bool + 设备内耗时 |
| 0x0020 | DUMP | h→a | UI 树（XML 或结构化 node 列表，可带压缩 flag） |
| 0x0021 | QUERY | h→a | selector/xpath 查询，返回 bounds+attrs |
| 0x0030 | SCREENSHOT | h→a | 按需截图：format(jpeg/png/raw)、quality、scale、display_id |
| 0x0040 | BATCH | h→a | 有序 op 数组 + 失败策略（见 §4） |
| 0x0041 | BATCH_RESULT | a→h | 每 op 的结果数组（stream-more 中间推送） |
| 0x0050 | STREAM_CTRL | h→a | start/stop 帧流：fps、编码(h264/jpeg)、display_id |
| 0x0060 | TEXT | h→a | 文本输入（ASCII 走注入；非 ASCII 走 IME 通道，见 03-device-agent §6） |
| 0x00F0 | AGENT_STOP | h→a | 显式退出 |

媒体通道（ch=1）独占 type：0x0001 FRAME（编码帧 + pts + display_id）、0x0002 UI_EVENT（可选，accessibility 变更通知）。

## 4. 批量语义（核心差异化）

```
BATCH {
  fail_policy: STOP_ON_ERROR | CONTINUE,
  ops: [
    { inject: {type: tap, x, y, wait_finish: true} },
    { wait_until: {selector: {...}, timeout_ms: 5000, poll_ms: 100} },
    { inject: {type: swipe, path: [...], duration_ms} },
    { screenshot: {format: jpeg, quality: 80} },
  ]
}
```

- **wait_until 是 opcode 而非 host 侧轮询**——条件判断在设备端 UiAutomation/树缓存上跑，消除跨端往返。这是批量与"设备端规则下沉"合并成一个里程碑的原因（规则引擎 = wait_until + 简单条件-动作组合，不需要独立 DSL/Lua，先用 selector+timeout 参数化表达，够用即停）。
- 中间结果 `stream-more` 推送：host 可实时显示进度，长批量可被取消（CANCEL type，h→a，按 req_id 取消）。
- 事务语义：BATCH 非原子（UI 自动化无回滚可能），失败点之后的行为仅由 fail_policy 决定。

## 5. u2 JSON-RPC 兼容层

目的：**对拍与降级**，不是长期维护两套语义。

- agent 另监听 `localabstract:pilotdroid_agent-u2compat`（或直接 `adb forward tcp:9008`），实现 u2 的 `POST /jsonrpc/0` 方法子集：`click / swipe / dumpWindowHierarchy / takeScreenshot / waitForExists / pressKey`。
- 内部实现统一走同一执行引擎——compat 层只是编解码翻译，不改语义。
- 用法：python `uiautomator2.connect()` 直连对拍；M0-M1 也可把该端口直接 forward 到 **u2.jar**（无修改复用，Apache 2.0），在自研 agent 未覆盖的能力上兜底。

## 6. 连接管理与超时

- 单连接：host 只持一条 forward socket；多 device 按 serial 各一条 session。
- 心跳失败 → host 标记 agent 死亡 → 走 §01 生命周期重新部署。
- 请求超时：默认 5s，可被 op 级 timeout 覆盖；INJECT wait_finish 超时 2s。
- 背压：media 通道独立窗口，帧流丢帧优先于阻塞控制通道（FRAME 帧可被 agent 在队列积压时丢弃，带 dropped 计数）。

## 7. 未定项（留给实现期）

- msgpack vs FlatBuffers（首版 msgpack，bench 后重评）。
- UI 树 wire 格式：XML 透传（兼容 u2 工具链）vs 结构化 node 数组（更小）。
- 帧流编码：h264 需要 MediaCodec 管线（M3）；JPEG 序列帧实现更简单（15fps 实测可达？待 spike-2）。
