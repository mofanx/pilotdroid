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
- **flags**：bit0=req/resp，bit1=stream-more（M3 帧流起用，M2 恒 0），bit2=error，bit3=NOACK（fire-and-forget，见 §4.5）。
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

## 4. 批量与设备端谓词语义（M2 契约，定稿）

### 4.1 BATCH 请求

```
BATCH {
  ops: [ <op>, ... ],                  // ≥1；每项为单键 map（见下）
  fail_policy: "stop" | "continue",    // 缺省 "stop"
  deadline_ms: u64,                    // 可选：batch 级总时限
  poll_ms: u64,                        // 可选：wait_until 默认轮询间隔，缺省 20
}
```

`<op>` = 单键 map：`{"inject": InjectReq}`、`{"wait_until": WaitReq}`、`{"query": QueryReq}`、`{"dump": DumpReq}`、`{"screenshot": ShotReq}`、`{"text": TextReq}`。

### 4.2 BATCH_RESULT 响应（单响应帧）

```
{ results: [
    { index: u32,                 // 对应 ops 下标
      code: "OK"|"TIMEOUT"|"DEADLINE"|"ABORTED"|"ERR_*",
      message: string,
      cost_ns: u64,               // 该 op 设备内耗时
      data?: <该 op 的响应 payload> },  // dump→{xml}, shot→{format,data}, query→{found,node}
    ...
  ],
  total_ns: u64 }
```

- **每 op 必有一条 result**：fail_policy=stop 下首个非 OK 之后记 `ABORTED`；`deadline_ms` 语义（实现口径，t5 复核对齐）：deadline 耗尽时**首个**未执行 op 记 `DEADLINE`，**其后**所有未执行 op 记 `ABORTED`——DEADLINE 是"撞上时限的那一个"，ABORTED 是"被其连坐跳过的"。
- op 自身失败 ≠ 帧错误；整帧畸形才走 ERROR flag + ErrorAck。
- 事务语义：BATCH 非原子，失败点之后仅由 fail_policy 决定。

### 4.3 wait_until（设备端谓词求值，ADR-004）

```
WaitReq = { selector: Selector, timeout_ms: u64 = 5000, poll_ms?: u64 }
Selector = { text?: string, id?: string, desc?: string, index?: u32 }
```

四元组 **AND** 语义：`text`/`desc` 精确匹配；`id` 为 resource-id——`"pkg:id/name"` 全等、裸 `"name"` 按 `:id/` 后缀匹配；`index` 为多命中序号（缺省 0，同时构成存在性语义：第 N 命中不存在 → 未命中）。

求值在设备端常驻 UiAutomation 树缓存上每 `poll_ms`（默认 20ms，可被 BATCH.poll_ms 覆盖）执行一次；命中 → `code:"OK"`（`data.node` 附带命中节点 bounds/text/id/desc），超时 → `code:"TIMEOUT"`（op 结果，非帧错误）。

### 4.4 QUERY / TEXT / SCREENSHOT-scale wire 契约

- `QueryReq = { selector: Selector, attrs?: string[] }` → `{ found: bool, node?: { text,id,desc,bounds:[l,t,r,b], class, clickable, enabled } }`；exists 语义即 `found`。`attrs` 是**可选裁剪参数**：给定 `["text","bounds"]` 时 agent 只回该属性子集；省略时回全量 node。host 侧必须容忍多余字段（serde named-map 天然如此）。
- `TextReq = { text: string, clear?: bool }` → 设备端 `AccessibilityNodeInfo.ACTION_SET_TEXT` 作用于当前焦点节点（非 ASCII 同路径）；无焦点节点 → `code:"NO_FOCUS"`；应答 `{ok, chars, cost_ns}`（chars 为写入字符数）。
- `ShotReq`/`ShotAck`：请求 `{format, quality: 0-100 = 80, scale: (0,1] = 1.0}`；应答含 `{format, w, h, scale, data}`——w/h/scale 为缩放后实际几何与生效倍率的回显；`take_ns`（capture+scale 耗时）与 `enc_ns`（JPEG 编码耗时）为可选观测字段（M3 归因计时拆分，spike/e2e/m3-experiments 消费）。

### 4.7 INJECT 事件序列与 KEY（M3）

`InjectReq = { kind: "Tap"|"Swipe"|"Key", points?: MotionPoint[], wait_finish?: bool, keycode?: u32, meta?: u32, backend?: "im"|"uia" }`

- **MotionPoint = { action: u8, x: f32, y: f32, t_ms: u64 }**：`action` 为 `MotionEvent.ACTION_*`（0=DOWN、1=UP、2=MOVE），`t_ms` 为相对首点的 eventTime 偏移。`points` 按序构成手势：tap = [DOWN, UP(+~60ms)]；**swipe = DOWN + N×MOVE + UP**（MOVE 点密度即路径采样）。`wait_finish` 缺省 false。
- **kind="Key"**：`keycode`（`android.view.KeyEvent.KEYCODE_*`，back=4/home=3/recent=187，任意 keycode 合法）走同一 `injectInputEvent` 通道构造 `KeyEvent`（DOWN+UP）；`points` 此时省略。可选 `meta`（metaState，缺省 0）。
- **`backend` 可选强制字段**：`"im"`（InputManager 路径）或 `"uia"`（`UiAutomation.injectInputEvent`）；缺省走 agent 内置探测回退链（IM→UIA，connect 期探测，`HELLO_ACK.inject` 上报生效后端）。强制指定后端失败 → `code:"ERR_BACKEND"` op 错误而非静默降级。
- `ShotReq` 增 `scale: f32 ∈ (0,1]`（缺省 1.0），设备端缩放后再编码（实测账见 spike/e2e/wifi-vs-usb.md §2 + RESULTS-M2：420KB→18KB，但编码主导 e2e，字节缩减 23×）。

### 4.5 NOACK flag（flags bit3 = 0x0008）

请求帧置 NOACK → agent 执行后**不回响应帧**（fire-and-forget）。约束：

- 仅对无返回值的 op 合法（INJECT/TEXT 等）；对 DUMP/QUERY/SHOT/BATCH 置 NOACK 视为协议错误，agent 回 ERROR 帧（响应型 op 的错误不可被 NOACK 吞掉）。
- NOACK 下执行错误静默丢弃；`req_id` 仍递增占号（日志可对账）。host 不得等待 NOACK 请求的回包。

### 4.6 控制通道配对约束（定稿）

- ch=0 严格 request/response：一请求至多一响应（NOACK 为零）；M2 期 host 侧**串行**（至多一个在途请求）。
- **禁止跨通道交错**：media 帧（ch=1）不得出现在本 socket——未来帧流使用第二条独立 socket/forward（STREAM_CTRL 激活时建立）。agent 收到非 control 帧按协议错误拒连。
- `stream-more` 中间推送与 CANCEL 语义**不在 M2 范围**（BATCH_RESULT 单帧返回），保留 flags bit1 与 CANCEL opcode 位待 M3 帧流再起用。

## 5. u2 JSON-RPC 兼容层

目的：**对拍与降级**，不是长期维护两套语义。

**ADR-005 修正**：compat 层改为 **host 侧翻译层**——独立进程监听 u2 `POST /jsonrpc/0`，把方法子集（click/swipe/dumpWindowHierarchy/takeScreenshot/waitForExists/pressKey/setText/appStart/exists）翻译为 PD01 帧并经持久 socket 复用连接发给 agent。原"agent 内嵌 JSON-RPC 端口"方案被否：UiAutomation 单持有者约束下不能再起第二个服务进程，且内嵌方案使 agent 体积/迭代成本膨胀（详见 ADR-005）。

## 6. 连接管理与超时

- 单连接：host 只持一条 forward socket；多 device 按 serial 各一条 session。
- 心跳失败 → host 标记 agent 死亡 → 走 §01 生命周期重新部署。
- 请求超时：默认 5s，可被 op 级 timeout 覆盖；INJECT wait_finish 超时 2s。
- 背压：media 通道独立窗口，帧流丢帧优先于阻塞控制通道（FRAME 帧可被 agent 在队列积压时丢弃，带 dropped 计数）。

## 7. 未定项（留给实现期）

- msgpack vs FlatBuffers（首版 msgpack，bench 后重评）。
- UI 树 wire 格式：XML 透传（兼容 u2 工具链）vs 结构化 node 数组（更小）。
- 帧流编码：h264 需要 MediaCodec 管线（M3）；JPEG 序列帧实现更简单——takeScreenshot p50≈13-24ms（A14）意味着 ~40fps 理论上限，15fps 流可达性已被 spike-2 支撑。
