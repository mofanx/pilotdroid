# 01 · 总体架构

> 状态：草案（核心裁决已有实测支持） · 日期：2026-11-19
> 关联：adr/002、spike/RESULTS.md

## 1. 分层视图

```
┌────────────────────────────────────────────────────────┐
│  pilotdroid host（纯 Rust, tokio）                       │
│  ├─ lib: 高层 API（click / swipe / find / screenshot…） │
│  ├─ CLI: pilotdroid <cmd>                              │
│  └─ session: 设备清单、agent 生命周期、批量脚本           │
├────────────────────────────────────────────────────────┤
│  ADB client 层（vendor: adb_client）                     │
│  └─ smartsocket / shell v2 / sync push / forward       │
├─────────────── transport: adb forward ─────────────────┤
│  tcp:localhost:PORT ⇄ localabstract:pilotdroid_agent         │
├────────────────────────────────────────────────────────┤
│  设备端 pilotdroid-agent（dex, app_process 常驻）         │
│  ├─ RPC loop: 分帧解码、批量执行、wait-until、心跳        │
│  ├─ 注入: InputManagerGlobal.injectInputEvent  ← 3ms实测│
│  ├─ UI 树: UiAutomation + AccessibilityNodeInfo dump   │
│  ├─ 截图: UiAutomation.takeScreenshot / SurfaceControl │
│  └─ 帧流(可选): VirtualDisplay + MediaCodec            │
├────────────────────────────────────────────────────────┤
│  native sidecar（可选、可裁）                            │
│  └─ /dev/input 只读监听、文件/进程杂务                   │
└────────────────────────────────────────────────────────┘
```

## 2. 关键裁决（全部有实测出处）

| 裁决 | 依据 |
|---|---|
| **Java agent 为能力主体**，不是"薄片" | `/dev/input` 写在 Enforcing 下全拒；`InputManagerGlobal` 注入 3ms、30/30 成功（spike §2/§4） |
| **native 仅可选监听件** | `/dev/input` 读可行（`getevent -pl` 8 设备枚举成功），写不可行 |
| **agent 必须常驻** | app_process 冷启动 wall ~1.3s（ART ~1.1s），per-call spawn 不可接受 |
| **传输 = adb forward → localabstract** | 免 TCP 端口扫描冲突，scrcpy/u2 同款；adb server 是唯一传输依赖 |
| **协议 = 持久 socket 二进制分帧 + 批量** | per-call HTTP/JSON-RPC 是要突破的天花板本身 |
| **u2 JSON-RPC 兼容层** | 让 u2.jar 与 python 客户端可接入做 A/B 对拍，M1 可零 Java 工作量打通 |

## 3. 注入路径（实测降级链）

```
InputManagerGlobal.injectInputEvent (mode 0/2)   ← 主路径，p50≈3ms，已验证
  └─ InputManager.getInstance()（隐藏 API 兜底，旧 API 级）
       └─ UiAutomation.injectInputEvent（agent 已持 UiAutomation 时）
            └─ `input tap/swipe`（shell 命令，~30-50ms，最终兜底，零依赖）
```

`/dev/input` 直写**不在链中**——在本项目的目标设备类群（Enforcing）下已被实测否决；若未来遇到 permissive 设备可作为探测项重评。

## 4. 截图路径（三策略，按能力探测选用）

| 策略 | 本机实测 | 定位 |
|---|---|---|
| A. resident `UiAutomation.takeScreenshot` / `SurfaceControl.screenshot` | 未测（spike-2 项），预计 50-150ms | 按需截图主路径 |
| B. `exec-out screencap` | 420-450ms（209KB PNG） | 无 agent 兜底 / 启动期 |
| C. 帧流：`exec-out screenrecord` h264 | 19-22fps 静态画面可用 | 无 agent 流的兜底 |
| C+. agent 内 VirtualDisplay + MediaCodec | 编码器 60fps 能力已证 | 高帧率流（M3 可选） |

## 5. 会话生命周期（host 视角）

1. **探测**：`getprop`（SDK/ABI）→ agent 版本协商 → capability bitmap 上报（注入/截图/UI 树/流 四路可用性）。
2. **部署**：dex push `/data/local/tmp/pilotdroid/`（MD5 校验，幂等）。
3. **拉起**：`app_process` + `setsid` daemon 化 → `localabstract:pilotdroid_agent` 监听 → host `adb forward` 建立 → 握手（协议版本 + capability）。
4. **服务**：单连接复用；控制帧与媒体帧同 socket 分流（逻辑 channel）。
5. **回收**：host 断开不杀 agent（重连友好）；显式 `agent.stop` 或设备重启后重新部署。

## 6. 数据流示例（批量点击，M2 形态）

```
host → agent: [BATCH] seq=[tap(x1,y1), waitUntil(node B), tap(x2,y2), screenshot]
agent 设备端顺序执行，waitUntil 在设备端轮询 UI 树（无主机往返）
agent → host: [BATCH_RESULT] ok ×3 + [MEDIA] jpeg frame
```

对比 u2：上述序列是 4 次 HTTP 往返 ≈ 1s+；本方案 1 次往返，设备内耗时 ≈ 注入 3ms×2 + waitUntil 轮询 + 截图，目标 <300ms。

## 7. 模块划分（仓库远景）

```
pilotdroid/           # Cargo workspace
├── crates/
│   ├── pilotdroid-adb       # adb_client 封装（vendor 边界，可整体替换）
│   ├── pilotdroid-proto     # 分帧协议 + 批量语义（自研）
│   ├── pilotdroid-core      # 高层 API / session
│   └── pilotdroid-cli       # CLI
├── agent/                   # dex agent 源码（javac + d8 构建）
├── docs/                    # 本文档
└── spike/                   # 验证脚本与实测数据
```
