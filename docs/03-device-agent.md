# 03 · 设备端 agent 设计

> 状态：草案（注入路径已实测，其余待 spike-2/M1 验证） · 日期：2026-11-19
> 关联：spike/RESULTS.md、adr/002、04-capability-matrix.md

## 1. 形态

- **单个 dex jar** 推到 `/data/local/tmp/pilotdroid/agent.jar`（spike 实测 4KB dex 即可注入；完整版预估 <200KB）。
- 构建：`javac --release 8 -classpath <sdk>/android-XX/android.jar` → `d8 --min-api <min>`。**d8/SDK 仅为开发期依赖**，设备端零依赖。
- 运行：`CLASSPATH=/data/local/tmp/pilotdroid/agent.jar setsid app_process /data/local/tmp/pilotdroid pilotdroid.Agent <args>`，以 shell（uid 2000, `u:r:shell:s0`）身份常驻。
- 监听 `localabstract:pilotdroid_agent`（unix socket，免端口冲突，scrcpy 同款）。

## 2. 能力探测（启动自测 → capability bitmap）

agent 启动后逐项探测，握手时上报（详见 04-capability-matrix.md 的列定义）：

| bit | 能力 | 探测方法 | 本机结果 |
|---|---|---|---|
| 0 | inject_inputmanager_global | 反射 `InputManagerGlobal.getInstance()` + 试注入 | ✅ 3ms/30-30 |
| 1 | inject_inputmanager_legacy | 反射 `InputManager.getInstance()` | 未测（API<31 兜底） |
| 2 | uiautomation | 连接 `UiAutomation`（需 `am instrument`？否——`app_process` 下直接 `UiAutomationShell` 或 `adb shell uiautomator` 模型，实现时定） | 待 spike-2 |
| 3 | screenshot_resident | `UiAutomation.takeScreenshot`/`SurfaceControl.screenshot` | 待 spike-2 |
| 4 | dev_input_read | open `/dev/input/event*` O_RDONLY | ✅ getevent 可枚举 |
| 5 | dev_input_write | open O_WRONLY | ❌ EACCES（Enforcing） |
| 6 | virtual_display | MediaCodec+DisplayManager 虚拟显示 | 编码器能力已证 |
| 7 | ime_channel | 自定义 IME/广播注入文本 | 未实现（scope 项） |

探测失败项不阻塞启动——对应 opcode 返回 `UNSUPPORTED`，host 按降级链处理。

## 3. 注入降级链（实测排序）

```
bit0 InputManagerGlobal.injectInputEvent(ev, mode)   ← p50≈3ms（已验证, Android 14）
 → bit1 InputManager.getInstance() 隐藏 API（旧 API 级机型）
  → UiAutomation.injectInputEvent（agent 持有 UA 时；顺带获得手势同步）
   → `am`/`input` shell 命令（Runtime.exec，~30-50ms，永远可用）
```

- MotionEvent 由 host 传参数在 agent 内构造：`downTime/eventTime/action/x/y/pressure/metaState`。
- 多点/手势：单条 INJECT 可携带 MotionEvent 序列（down→move×N→up，带各自 eventTime），agent 逐帧注入——pinch/zoom/长按/曲线滑动都由此表达，无需独立 opcode。
- mode：0=async（不等待分发），2=wait_for_finish（返回真实分发结果）。默认 2，批量中间步可用 0 提效。

## 4. 截图三策略

| # | 策略 | 路径 | 本机实测 | 选用场景 |
|---|---|---|---|---|
| A | resident | `UiAutomation.takeScreenshot` / `SurfaceControl.screenshot`（agent 内） | 待 spike-2（预计 50-150ms） | 主路径：按需截图 |
| B | exec-out | `screencap`（host 侧直跑，无 agent） | 420-450ms PNG | agent 未启动/兜底 |
| C | stream | `exec-out screenrecord` → h264 流（无 agent） | ~19-22fps 静态 | 无 agent 持续流兜底 |
| C+ | stream | agent 内 VirtualDisplay + MediaCodec → FRAME 帧 | 编码器 60fps 上限已证 | M3 高帧率选项 |

选型规则：SCREENSHOT op 优先 A，A 不可用回退 B（host 侧执行）；STREAM_CTRL 优先 C+，未实现/失败回退 C。

注意：`screencap` 的 450ms 里 PNG 编码占大头，策略 A/B 均应允许 `format=jpeg` 或 raw 以压低延迟。

## 5. daemon 生命周期

```
deploy ──→ launch ──→ ready ──→ serve ──→ orphan? ──→ die
  │          │          │         │          │
  │          │          │         │          └ host 断开后仍存活（localabstract 持续监听）
  │          │          │         └ PING 超时→host 端判死→重新握手
  │          │          └ localabstract 监听就绪 + capability bitmap
  │          └ setsid + app_process；写 pid 到 /data/local/tmp/pilotdroid/agent.pid
  └ md5 校验，版本不符则重推
```

- **断连存活**：socket 是 client/server 模型倒置——agent 是 server，host 断开只是 client 走人，agent 继续监听（省电可设 idle 超时自杀，默认 30min，可配）。
- **重启恢复**：设备重启后 agent 与 socket 全灭。host 侧 session 建立时先查 pid 文件 + PING，失败则重走 deploy→launch。
- **并发**：单 agent 单连接；第二个 host 连入时 agent 拒绝（或按 flag 踢旧），避免状态错乱。
- **崩溃**：agent uncaught exception → 进程退出 → host 下次 PING 失败触发重部署；logcat tag `pilotdroid` 保留崩溃栈。

## 6. 遗留 scope 项（勿忘）

- **Unicode 文本输入**：`input text` 不支持非 ASCII。方案两选：自带 IME（pilotdroid-ime，装一次 APK）或 `am broadcast` 剪贴板粘贴。建议 M2 实现 broadcast 兜底 + M3 提供 IME 选项。TEXT opcode 已在协议中预留。
- **坐标系**：多 display（display_id 已在 opcode 中预留）、屏幕旋转下注入坐标与截图坐标的一致性（spike-2 测）。
- **screenrecord 时长**：`screenrecord` 内置 ~180s 上限，策略 C 长流需分段续传或直接用 C+。
- **keyevent / 系统按键**：INJECT 增加 `type: key`（KeyEvent 注入同 API），M1 一并覆盖。

## 7. 与 u2.jar 的关系

- M1 可直接 forward 到 u2.jar 的 9008 端口当兼容层实现（零 Java 工作量，见 02-protocol §5）。
- 自研 agent 就位后 u2.jar 仅在对拍时启用，非运行时依赖。
