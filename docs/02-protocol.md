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
| 0x0050 | STREAM_CTRL | h→a | start/stop 帧流：fps、scale、quality、format(jpeg，h264 留位)、display_id——契约见 §4.8 |
| 0x0060 | TEXT | h→a | 文本输入（ASCII 走注入；非 ASCII 走 IME 通道，见 03-device-agent §6） |
| 0x0070 | WATCH | h→a | 阻塞式设备端 selector→action 闭环（§4.9，M10） |
| 0x0071 | WATCH_ARM | h→a | 异步 watcher 注册，立即返回 watch_id（§4.9，M11） |
| 0x0072 | WATCH_DISARM | h→a | 按 watch_id 撤除异步 watcher（§4.9，M11） |
| 0x0080 | APP_INFO | h→a | 前台应用包名（UiAutomation 窗口读取，§4.10，M11） |
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
Selector = { text?: string, id?: string, desc?: string, class?: string, scrollable?: bool, pkg?: string, index?: u32 }
```

多字段 **AND** 语义：`text`/`desc` 精确匹配；`id` 为 resource-id——`"pkg:id/name"` 全等、裸 `"name"` 按 `:id/` 后缀匹配；`class`（M5）为类名——全限定名全等、简单名按尾段匹配；`scrollable`（M5）按节点 scrollable 属性布尔过滤；`pkg`（M6）为节点所属包名精确匹配；`index` 为多命中序号（缺省 0，同时构成存在性语义：第 N 命中不存在 → 未命中；即 u2 的 **`instance`** 语义——u2shim 按此映射 `instance`→wire `index`；u2 的 `index`（节点在父中的子位置）wire 不表达，shim 走 host post-filter 按 XML `index` attr 过滤，见 compat/SUPPORTED.md）。

求值在设备端常驻 UiAutomation 树缓存上每 `poll_ms`（默认 20ms，可被 BATCH.poll_ms 覆盖）执行一次；命中 → `code:"OK"`（`data.node` 附带命中节点 bounds/text/id/desc），超时 → `code:"TIMEOUT"`（op 结果，非帧错误）。

### 4.4 QUERY / TEXT / SCREENSHOT-scale wire 契约

- `QueryReq = { selector: Selector, attrs?: string[] }` → `{ found: bool, node?: { text,id,desc,bounds:[l,t,r,b], class, clickable, enabled, scrollable } }`；exists 语义即 `found`。`attrs` 是**可选裁剪参数**：给定 `["text","bounds"]` 时 agent 只回该属性子集；省略时回全量 node。host 侧必须容忍多余字段（serde named-map 天然如此）。
- `TextReq = { text: string, clear?: bool }` → 设备端 `AccessibilityNodeInfo.ACTION_SET_TEXT` 作用于当前焦点节点（非 ASCII 同路径）；无焦点节点 → `code:"NO_FOCUS"`；应答 `{ok, chars, cost_ns}`（chars 为写入字符数）。
- `ShotReq`/`ShotAck`：请求 `{format, quality: 0-100 = 80, scale: (0,1] = 1.0}`；应答含 `{format, w, h, scale, data}`——w/h/scale 为缩放后实际几何与生效倍率的回显；`take_ns`（capture+scale 耗时）与 `enc_ns`（JPEG 编码耗时）为可选观测字段（M3 归因计时拆分，spike/e2e/m3-experiments 消费）。

### 4.7 INJECT 事件序列与 KEY（M3）

`InjectReq = { kind: "Tap"|"Swipe"|"Key", points?: MotionPoint[], wait_finish?: bool, keycode?: u32, meta?: u32, backend?: "im"|"uia" }`

- **MotionPoint = { action: u8, x: f32, y: f32, t_ms: u64, pid?: u8 }**：`action` 为 `MotionEvent.ACTION_*`（0=DOWN、1=UP、2=MOVE），`t_ms` 为相对首点的 eventTime 偏移，`pid` 为手指 id（缺省 0）。`points` 按序构成手势：tap = [DOWN, UP(+~60ms)]；**swipe = DOWN + N×MOVE + UP**（MOVE 点密度即路径采样）。**多指针（M10, CAP_MULTI_PTR=bit7）**：序列中出现 >1 个不同 `pid` 时走多指针路径——agent 维护 pid→coords 活动表，把每个 wire 点展开为携带全部活动指针的 MotionEvent（DOWN→ACTION_DOWN / ACTION_POINTER_DOWN|idx、MOVE→ACTION_MOVE、UP→ACTION_POINTER_UP|idx / ACTION_UP；idx=pid 出现序）。pinch = 两条对称收敛/发散的 pid 轨迹。`wait_finish` 缺省 false。
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
- `stream-more` 中间推送与 CANCEL 语义**不在 M2 范围**（BATCH_RESULT 单帧返回），保留 flags bit1 与 CANCEL opcode 位待帧流再起用——STREAM_MORE 于 M4 media 帧流首次启用（§4.8）。

**单 client 顶替（M8, AGENT_VER≥0.5.0）**：agent 任一时刻只服务一条 control 会话——新连接 accept 时旧 socket `shutdownInput/Output`+close（`close()` 不打断阻塞 read，必须先 FIN）。**顶替非原子（TOCTOU）**：accept 与 shutdown 之间存在调度窗口，旧 conn 上在途 op 仍可正常完成；调用方应把"新 conn 建立后旧 conn 的下一 op 必败（≤~1s 内收敛）"作为可依赖语义，而非"建立瞬间即死"。被顶替方的 stream/elements 快照随 §4.8 会话绑定一并终结。

**op 分层注记（M11）**：并非所有 u2/shim 操作都过本 conn——`shell`/`app_start`/`app_stop`/`app_current`（无 CAP_APP_INFO 时）/`app_list`/`orientation` 等是 host 侧 `adb shell` spawn，**不过 agent 会话**。推论：被顶替的 client 上 conn-op（exists/query/inject…）必败而 shell-op 仍成功——"半死"状态对调用方可见；会话自愈语义仅作用于 conn-op（M11 起 `app_current`/`app_info` 在有 CAP_APP_INFO 时优先走 conn，见 §4.10）。

### 4.8 媒体通道帧流（M4 契约，定稿——决策见 ADR-006）

**传输**：media 帧走第二条独立 socket `localabstract:pilotdroid_agent_media`（第二条 `adb forward`），与 control socket 完全隔离——慢读者不可阻塞 control op。agent 启动时与 control socket 一并绑定，任一时刻至多一个流式会话。

- **STREAM_CTRL（control 通道，0x0050）**：
  - 请求 `{"op":"start", "fps"?:u8=5, "scale"?:f32=0.5, "quality"?:u8=70, "format"?:"jpeg"}` → 应答 `{ok, socket, effective_fps_hint}`；已在流中 → `code:"ERR_STREAM_BUSY"`。
  - 请求 `{"op":"stop"}` → 应答 `{ok, frames_sent, dropped}`。
- **FRAME（media 通道，ch=1，msg_type 0x0001）**：帧头 `req_id` 为单调递增 seq（缺口即丢帧）；flags 置 `STREAM_MORE`。流结束信号二选一：**终止帧**（STREAM_MORE 清除）或 **media socket 关闭**——v0 agent 实现走 socket 关闭（stop 三路径清理即 EOF），接收方须把 EOF 同样视为流终止；payload msgpack `{format, w, h, ts_ns, take_ns, enc_ns, dropped, data}`——`dropped` 为累计丢帧计数，`ts_ns` 为采集时刻。
- **背压**：写阻塞超过一个帧间隔 → 丢帧不阻塞，`dropped++`；media 积压绝不影响 control 通道。
- **stop 清理**：`stop` / control 连接断开 / media 客户端断开 → 停采集循环、join 线程、关闭该流连接（socket 继续监听）；无残留线程。
- **会话绑定**：流绑定发起它的 control 会话；control 断开即隐式 stop（防孤儿流）。
- **编码**：v0 仅 `format:"jpeg"`（E1 归因：编码∝像素，quality 只省字节）；`"h264"` 留位待 MediaCodec 管线（M5）。`ui_event`（ch=1 type 0x0002）于 M11 起用为 **WATCH_HIT 推送**（§4.9）——写往活跃流连接，否则写往 start 前的 pending conn（watch 客户端在 armed 期间持有它）；无 media conn 则事件丢弃（动作已在设备内执行完）。

### 4.9 WATCH——设备端 selector→action 闭环（M10, CAP_WATCH=bit8）

`WatchReq = { selector, action?: {type:"click"|"none"}, timeout_ms?: u64=10000, poll_ms?: u64=150 }`

- agent 在设备内轮询 `find(selector)`（每 poll_ms 一次 tree-walk），命中即按 `action` 执行（`click`=点 bounds 中心走同一注入链；`none`=仅检测）→ 应答 `{matched, waited_ms, action_ok?, node?}`。
- 与 `wait_until` 的差别：wait_until 只等不做事；WATCH 把"出现→动作"收进**设备内单 op 闭环**，检测延迟=一次 tree-walk（u2 watcher 每轮一次 HTTP+dump 的 host 轮询没有等价物）。
- 同步阻塞 op：占用本 control 会话直到 matched 或 timeout；u2 watcher 对等语义请用下面的异步变体。

**异步 watcher（M11, CAP_WATCH_ASYNC=bit9）**：`WATCH_ARM{selector, action?, timeout_ms?=0 不限, poll_ms?=100}` → 立即应答 `{armed, watch_id}`——agent 内 Watcher 线程轮询同一 selector，**不占用 control 会话**（armed 期间 ping/query/inject 照常，`Agent.call` 客户端需自行串行化 socket）。命中（或 timeout_ms>0 到时未命中）→ 经 media socket 推送 `WATCH_HIT` ui_event（ch=1 type 0x0002）`{watch_id, matched, waited_ms, action_ok?, node?}`；`WATCH_DISARM{watch_id}` → `{disarmed}`。fire-once；会话终结即全部撤除。**资源上限（M12）**：每会话最多 **8 条** armed watcher，超出 → `ERR_LIMIT`（每条 watcher 是一个常驻线程、每 poll 一次全树 DFS 且持全局 a11y 锁——arm 不设上限会把 session 所有 QUERY 拖慢到 find 周期量级，LLM/循环 arm 最易触发）；`poll_ms` 下限 **50ms**（`watch`/`wait_until` 的阻塞路径不受此限）。**selector 建议 pkg 收敛**：MIUI 等 ROM 上节点的 `boundsInScreen` 属于窗口坐标系、可超物理 display（实测 bounds [268,3312] vs 屏高 3200），宽泛 selector（仅 class/text、无 pkg）容易在状态栏/通知窗命中"幽灵节点"——async watch 请用 `{pkg: 目标应用, ...}` 收敛。**a11y 访问串行化**：watcher 的 find 与 session 的 QUERY/DUMP 共用 `synchronized(uia)` 全局锁——并发树访问在框架层有 ~秒级 stall 风险（目标应用主线程忙碌时 a11y IPC 排队）；锁把并发危险变成"至多一次 find 的排队"（实测窗口切换/弹窗拆除期偶发 ~2s 单次 stall，同场景真 u2 app_current ~5.6s，非本实现特有）。

**serve 侧 hit 交付 = pull，push 是 demux 而非 toggle（M13 定论）**：MCP `notifications/message` 推送通道在协议内可达（`spike/e2e/mcp-notify-spike.py`：echo server 实证真实 ClientSession `logging_callback` 收到），但本协议 media socket 是**单读者**——serve 若常驻读 socket 发 notification，`watch_events`/`stream_read` 的 pull 队列即被抽空。真 push 需要 host 侧**单读者按帧类型 demux**：`watch_hit`(0x0002) → `notifications/message`，`frame`(0x0001) → 留给 stream_read pull——~50 行且引入"hit 既进通知又需缓冲供查询"的复制语义。notification 终究只是 log 面（非结构化 tool 事件，客户端仍需轮询取 payload），收益不成正比 → **保留 pull 为契约**，push 候选记入待议（若做，走 demux 方案）。

### 4.10 APP_INFO——conn 侧前台应用查询（M11, CAP_APP_INFO=bit10）

`APP_INFO{}` → `{pkg, activity: null, display:[w,h], sdk}`。经 `UiAutomation.getWindows()` 取焦点窗口根节点包名——~ms 级，替代 host 侧 `adb shell dumpsys window` spawn（实测 ~300-600ms/次）。`activity` 恒为 `null`（accessibility 窗口不暴露 activity 名；u2 取它靠 dumpsys——本实现明确拒绝该成本）。shim `app_current()` 有本 cap 时走 conn，否则回退 dumpsys。

## 5. u2 JSON-RPC 兼容层

目的：**对拍与降级**，不是长期维护两套语义。

**ADR-005 修正**：compat 层改为 **host 侧翻译层**——独立进程监听 u2 `POST /jsonrpc/0`，把方法子集（click/swipe/dumpWindowHierarchy/takeScreenshot/waitForExists/pressKey/setText/appStart/exists）翻译为 PD01 帧并经持久 socket 复用连接发给 agent。原"agent 内嵌 JSON-RPC 端口"方案被否：UiAutomation 单持有者约束下不能再起第二个服务进程，且内嵌方案使 agent 体积/迭代成本膨胀（详见 ADR-005）。

## 6. 连接管理与超时

- 单连接：host 只持一条 forward socket；多 device 按 serial 各一条 session。
- 心跳失败 → host 标记 agent 死亡 → 走 §01 生命周期重新部署。
- 请求超时：默认 5s，可被 op 级 timeout 覆盖；INJECT wait_finish 超时 2s。
- 背压：media 通道独立窗口，帧流丢帧优先于阻塞控制通道（FRAME 帧可被 agent 在队列积压时丢弃，带 dropped 计数）。

### 6.1 op `code` 错误码表（定稿）

| code | 语义 | 触发面 |
|---|---|---|
| `OK` | op 成功 | 全部 |
| `TIMEOUT` | op 级超时（如 wait_until 到时未命中）— op 结果而非帧错误 | wait_until、wait_finish |
| `DEADLINE` | BATCH 级 deadline 耗尽，余 op 跳记 | BATCH |
| `ABORTED` | 前序 op 失败/谓词不满足，余 op 中止 | BATCH |
| `ERR_BACKEND` | `backend` 强制指定的注入后端不可用 | INJECT/KEY |
| `ERR_SET_TEXT` | `ACTION_SET_TEXT` 返回 false | TEXT |
| `ERR_STREAM_BUSY` | 流已在跑 / 流被其他 session 持有 | STREAM_CTRL start |
| `ERR_STREAM_FMT` | 请求了 v0 不支持 format（仅 jpeg） | STREAM_CTRL start |
| `ERR_INTERNAL` | 未分类 agent 内部异常兜底 | 全部 |
| `UNSUPPORTED` | op/opcode 不被该 agent 版本支持 | 全部 |

### 6.2 版本命名空间（M6 定稿）

三套版本号互不混用：`ver`（帧头 byte）= **wire 帧格式版本**；`HELLO_ACK.ver`（如 `0.3.1`）= **设备端 agent.jar 构建版本**；`serve` 的 `serverInfo.version` = **host 侧 crate 版本**，`serverInfo.agentVer` 透传 HELLO.ver——一次 MCP 握手即可同时读出两侧版本。

## 7. 未定项（留给实现期）

- msgpack vs FlatBuffers（首版 msgpack，bench 后重评）。
- UI 树 wire 格式：XML 透传（兼容 u2 工具链）vs 结构化 node 数组（更小）。
- 帧流编码：JPEG 序列帧已定稿（§4.8，实测 ~5-15fps @scale0.5 受 takeScreenshot 抖动支配）；h264 需 MediaCodec 管线，归 M5。
