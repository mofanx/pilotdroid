# ADR-006 · 媒体通道帧流：ch=1 独立 socket + JPEG 序列帧

> 状态：已接受 · 日期：2026-11-19
> 范围：M4 帧流/live view（docs/05 M4 验收线）

## 背景与定位（由 E1 归因裁决决定）

M3 E1 实验证明单发 SHOT 是**固定成本主导**：`takeScreenshot` 抖动
20-120ms + JPEG 编码 ∝ 像素（半分辨率 ~35-65ms），`quality`/`scale`
调参只省字节不省 e2e。因此帧流的定位**不是**"更快的单发截图"，而是：

1. **摊销**——复用编码器/位图缓冲，takeScreenshot→encode→send 流水化，
   连续帧均摊固定成本；
2. **并发**——media 帧与控制通道隔离，流式抓取期间 control op
   （tap/query/batch）不排队。

目标验收：≥5fps @ scale=0.5 持续 ≥60s；流期间 control op 延迟不回归。

## 候选

| 方案 | 结论 |
|---|---|
| A. control socket 上跑 media 帧 | **否**：§4.6 已禁——一个慢读者就阻塞全部 ctrl op，背压策略再细也救不了共享队列 |
| B. **第二条独立 localabstract socket** | **选** |
| C. screenrecord/MediaCodec H.264 | 否（本期）：管线重得多（surface→编码器→解复用），且 wire 契约不变时可后补；M4 先 JPEG 序列帧验证机制，H.264 归 M5 |
| D. 同一 socket 内协议升级 | 否：STREAM_CTRL 期间失去 control 通道，违背"并发"目标 |

## 决定

**ch=1 走第二条独立 socket** `localabstract:pilotdroid_agent_media`，
由第二条 `adb forward` 暴露。agent 启动时与 control socket 一并绑定；
任一时刻至多一个流式会话。

## wire 契约（docs/02 §4.8 同步）

- **STREAM_CTRL（control 通道，opcode 0x0050）**
  - `start {fps?:5, scale?:0.5, quality?:70, format?:"jpeg"}` →
    ack `{ok, socket, effective_fps_hint}`；已在流中 → `ERR_STREAM_BUSY`
  - `stop` → ack `{ok, frames_sent, dropped}`
  - 缺省值依据 E1：scale=0.5 使编码 ~35-65ms/帧，fps=5 留出
    takeScreenshot 抖动余量
- **FRAME（media 通道，ch=1，msg_type 0x0001）**
  - 18B 帧头 `req_id` = 单调递增 seq——host 检测缺口即知丢帧
  - flags = `STREAM_MORE`（每帧置位；流终止帧不置——**STREAM_MORE
    于 M4 首次启用**，语义保持 docs 原义不变）
  - payload = msgpack `{format, w, h, ts_ns, take_ns, enc_ns, dropped, data}`：
    `dropped` 为累计丢帧计数（背压观测），`ts_ns` 为采集时刻
- **背压**：agent 发送循环写阻塞超过一个帧间隔 → **丢帧不阻塞**，
  dropped++；media socket 积压绝不影响 control 通道
- **stop 清理语义**：`stop` 或控制连接断开或 media 客户端断开 →
  停止采集循环、join 线程、关闭该流连接（socket 保持监听下一个会话）；
  无残留线程/缓冲区
- **会话关系**：流绑定发起它的 control 会话——control 断开即隐式 stop
  （防孤儿流）；单实例（ERR_STREAM_BUSY）

## 后果

- host 侧：`adb forward` 需建第二条规则（watchdog/连接管理同步扩展）；
  AgentClient 增加只读帧接收器，断流按协议错误处理
- JPEG 路径 fps 上限受 takeScreenshot 抖动支配（实测 ~5-15fps
  @scale0.5）；要 30fps 需 H.264——契约已为 `format:"h264"` 留位
- `ui_event`（0x0002）留作 accessibility 推送，本期不实现
