# 03 · 设备端 agent 设计

> 状态：进程模型已裁决（spike-2 实测） · 日期：2026-11-19
> 关联：spike/RESULTS.md、spike/RESULTS-2.md、adr/002、04-capability-matrix.md

## 1. 形态（spike-2 实测裁决）

- **单个 dex jar** 推到 `/data/local/tmp/pilotdroid/agent.jar`（spike 实测 4KB dex 即可注入；完整版预估 <200KB）。
- 构建：`javac --release 8 -classpath <sdk>/android-XX/android.jar` → `d8 --min-api 34`。**d8/SDK 仅为开发期依赖**，设备端零依赖；min-api 34 的 dex 在 SDK37 直跑无修改（实测）。
- 运行：`CLASSPATH=/data/local/tmp/pilotdroid/agent.jar setsid app_process /data/local/tmp/pilotdroid pilotdroid.Agent <args>`，以 shell（uid 2000, `u:r:shell:s0`）身份常驻。
- 监听 `localabstract:pilotdroid_agent`（`LocalServerSocket` + `adb forward` 回环已实测，echo 16B）。
- **进程模型：单进程承载全部能力**。`UiAutomation` 可在裸 `app_process` 中反射构造：`new UiAutomation(Looper, UiAutomationConnection)` + `connect(FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)`（flags=1 必须，避免杀用户已有无障碍服务）。**无需 `uiautomator runtest`、无需 instrumentation、无需 APK**。同进程内 UiAutomation + InputManagerGlobal + 截图 + socket 五能力共存已双机实测。
- **u2 同构旁证**：uiautomator2 v3.7.1 的 init 同样只 push dex + `app_process` 起 Main，不再安装 APK——dex-only 路线与上游演进方向一致。

## 2. 能力探测（启动自测 → capability bitmap）

| bit | 能力 | 探测方法 | 实测结果 |
|---|---|---|---|
| 0 | inject_inputmanager | 反射 `InputManagerGlobal`/`InputManager.getInstance()` + **真实注入一次并观察进程存活** | Mi11/dash ✅ ~4ms；duchamp 注入即死（bit 必须含存活观察，connect 成功≠注入可用） |
| 1 | inject_uiautomation | `UiAutomation.injectInputEvent` 试注入 | ✅ ~25-49ms；同 duchamp 陷阱 |
| 2 | uiautomation | 反射 ctor + `connect(flags=1)` | ✅ 三机 connect 成功（duchamp dump 返回 0 节点——前台状态相关，探测应验证非空） |
| 3 | screenshot_resident | `UiAutomation.takeScreenshot` | ✅ 13-24ms（A14）/ 78ms（A17）；`SurfaceControl.screenshot` 已从 API 中移除，不再探测 |
| 4 | dev_input_read | open `/dev/input/event*` O_RDONLY | ✅ 三机可读 |
| 5 | dev_input_write | open O_WRONLY | ❌ 三机全拒（Enforcing），从链中移除 |
| 6 | virtual_display | MediaCodec+DisplayManager | 编码器能力已证，虚拟显示未验证 |
| 7 | ime_channel | 自定义 IME/广播注入文本 | 未实现（scope 项）；MIUI APK 安装被弹窗拦截，IME 路径需用户先开安全设置 |
| 8 | shell_inject | `input tap` | Mi11 ✅；duchamp ❌ SecurityException（INJECT_EVENTS 被收）——兜底也非处处可用 |

探测失败项不阻塞启动——对应 opcode 返回 `UNSUPPORTED`，host 按降级链处理。

## 3. 注入降级链（spike-2 实测排序）

```
bit0 InputManagerGlobal.injectInputEvent(ev, mode)   ← async p50≈1.5-4ms（已验证）
 → bit1 UiAutomation.injectInputEvent               ← p50≈25-49ms，有 a11y 语义
  → bit8 `input` shell 命令                          ← ~30-90ms，非处处可用（duchamp 无 INJECT_EVENTS）
```

- **`dispatchGesture` 不在能力集内**——仅 `AccessibilityService`（须 APK+用户授权）才有，dex agent 拿不到（spike-2 反射枚举确认）。替代已实测：MotionEvent 序列注入（down+N×move+up）经 IM 全部成功——pinch/zoom/长按/曲线滑动都由序列表达，无需独立手势 opcode。
- MotionEvent 由 host 传参数在 agent 内构造：`downTime/eventTime/action/x/y/pressure/metaState`。
- mode：0=async（p50~1.5-2ms，稳定），2=wait_for_finish（返回真实分发结果，但**延迟随前台目标变化**：轻量前台 ~3ms，重负载前台 ~15ms——性能验收按 async 口径或固定前台条件测量）。默认 2，批量中间步用 0 提效。
- **存活探测**：能力探测时对 bit0/bit1 真实注入一次——duchamp 型设备会在 `injectInputEvent` 时把进程 SIGKILL（SecurityException 都不给），探测必须观察"调用后进程还在不在"。

## 4. 截图三策略（spike-2 已裁决主路径）

| # | 策略 | 路径 | 实测 | 选用场景 |
|---|---|---|---|---|
| A | resident | `UiAutomation.takeScreenshot`（agent 内） | **1440×3200 p50≈13-24ms（A14）/ 78.5ms（A17）** | 主路径：按需截图 |
| B | exec-out | `screencap`（host 侧直跑，无 agent） | 380-450ms PNG | agent 未启动/兜底 |
| C | stream | `exec-out screenrecord` → h264 流（无 agent） | ~19-22fps 静态 | 无 agent 持续流兜底 |
| C+ | stream | agent 内 VirtualDisplay + MediaCodec → FRAME 帧 | 编码器 60fps 上限已证 | M3 高帧率选项 |

`SurfaceControl.screenshot` 已**从 A34/A37 API 移除**（NoSuchMethod + `DisplayCaptureArgs` 不存在）——从选型中删除，策略 A 唯一路径即 takeScreenshot。

注意：`screencap` 的 450ms 里 PNG 编码占大头，策略 B 应允许 `format=jpeg`/raw 压低延迟；策略 A 已 13-78ms 量级，压缩收益有限但保留 format 参数。

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

- **断连存活（spike-2 实测）**：宿主 `adb kill-server` 后 setsid 进程存活（裸 `&` 也活）——session 断开不杀 agent，host 重连即可。**但 `adb usb`（设备侧 adbd 重启，≈拔线/端口切换）必死**——所以不能假设常驻不死：host 侧心跳探测 + 自动重部署是硬需求（u2 同样如此）。duchamp 上纯进程存活正常（>50s setsid）。
- **重启恢复**：设备重启或 adbd 重启后 agent 与 socket 全灭。host 侧 session 建立时先查 pid 文件 + PING，失败则重走 deploy→launch。
- **并发**：单 agent 单连接；第二个 host 连入时 agent 拒绝（或按 flag 踢旧），避免状态错乱。
- **崩溃**：agent uncaught exception → 进程退出 → host 下次 PING 失败触发重部署；logcat tag `pilotdroid` 保留崩溃栈。

## 6. 遗留 scope 项（勿忘）

- **Unicode 文本输入**：`input text` 不支持非 ASCII（且 duchamp 型设备上 `input` 本身就被权限拒）。方案两选：`am broadcast` 剪贴板粘贴（无 APK 依赖，M2 优先）或自带 IME（pilotdroid-ime，装一次 APK——但 MIUI 上 `adb install` 被弹窗拦截，需用户开「USB调试(安全设置)/USB安装」前置开关，见下）。TEXT opcode 已在协议中预留。**对拍价值已量化**：u2 `send_keys` p50≈309ms/p95≈2s 是其弱项。
- **MIUI onboarding 前置条件**（spike-2 实测）：①`adb install`/`pm install` 均触发 MIUI AdbInstallActivity 弹窗，~10s 无操作自动拒绝——APK 路径不可依赖；②duchamp 型设备连 INJECT_EVENTS 都被收回——用户须手动开「USB调试(安全设置)」。两项都写进 onboarding 检查清单，非自动流程。
- **坐标系**：多 display（display_id 已在 opcode 中预留）、屏幕旋转下注入坐标与截图坐标的一致性（待测）。
- **screenrecord 时长**：`screenrecord` 内置 ~180s 上限，策略 C 长流需分段续传或直接用 C+。
- **keyevent / 系统按键**：INJECT 增加 `type: key`（KeyEvent 注入同 API），M1 一并覆盖。
- **exists 语义**：u2 的 `exists(selector)` 是本地惰性求值（0.02ms，无 RPC）——我方 selector 层应同样避免"每查必 RPC"，find/wait 才取树（M3 设计注意）。

## 7. 与 u2.jar 的关系

- M1 可直接 forward 到 u2.jar 的 9008 端口当兼容层实现（零 Java 工作量，见 02-protocol §5）。
- 自研 agent 就位后 u2.jar 仅在对拍时启用，非运行时依赖。
