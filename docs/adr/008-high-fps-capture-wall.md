# ADR-008 · 高帧抓帧面：MIUI API34 七路封锁实录与 uia 档位维持

> 状态：已接受 · 日期：2026-11-25
> Supersedes：ADR-006 中「>6fps 高帧档不做」的语境假设（当时未实测）
> 范围：M28+ Track A/B 共享地基——AI 感知高帧流 substrate

## 背景

原目标（ms_0eeed9b5/t2）：device-side 流 daemon 实测 **≥15fps**
（ba0d072d，MIUI V816 / API 34 / 1440×3200），为 SoM 感知与 u2
平替两轨提供公共抓帧地基。本 ADR 记录 spike 定量证据与结论。

## 实测矩阵（全部 ba0d072d 实机）

| 路径 | 实测 | 证据 |
|---|---|---|
| `UiAutomation.takeScreenshot`（现 Streamer） | **5.6fps**（take 113ms+enc 67ms 串行，marshal 到 main 强制串行化） | `stream-stop{cap_ms:113,enc_ms:67}` |
| `screencap` 二进制（PNG 全分辨率） | 2.1fps | `time` 8 连发 3.83s |
| `SurfaceControl.screenshot*` / `ScreenCapture.captureDisplay` | **类/方法已剥离**——`getDeclaredMethods` 仅余 `setSkipScreenshot`；`DisplayAddress$Physical` 无 `getPhysicalDisplayToken`（display token 无源） | 反射探针枚举 |
| SurfaceControl VD mirror（`createDisplay`+`setDisplaySurface`+`setDisplayLayerStack(0)`+`setDisplayProjection`+`setDisplaySize`+`setDisplayPowerMode(2)`，secure∈{false,true}，ImageReader usage∈{default, GPU_COLOR_OUTPUT\|CPU_READ, HW_COMPOSER\|RENDER\|CPU_READ}，及 MediaCodec input surface） | **0 帧**：display 注册成功（`dumpsys SurfaceFlinger` 见 Display 8 "pd-spike"）但 `queued-frames=0`、surface 无 buffer | 5 个变体实测 |
| `screenrecord` 系统二进制（同一 native 路径 + 编码器 surface） | **2 帧/3s**（24KB mp4，ffprobe nb_frames=2） | 本机实跑——**ROM 级封锁，非我方实现问题** |
| `DisplayManagerGlobal.createVirtualDisplayWrapper` | `null`（DMS caller 校验，shell 无包归属） | invoke 返回 null |
| 并行多 `UiAutomation` 连接 | **全局单注册**：第二连接 `registerUiTestAutomationService` 抛 `UiAutomationService already registered`（本机 agent 占用时实测） | IllegalStateException |
| `ActivityThread.systemMain()` 系统 Context（补包归属建 DMS VD） | 进程 KillApplicationHandler 自杀 | app_process 实跑 |

## 跨 ROM 复测（2026-11-26，HUAWEI MRR-W29 / HarmonyOS 系 Android 12 / API31 / 2560×1600）

| 路径 | 实测 | 证据 |
|---|---|---|
| `UiAutomation.takeScreenshot`（同一 Streamer，uia 后端） | **实测顶 10.28fps**（req15；req5→5.03、req10→9.03 完全跟档） | `stream-stop{cap:"uia", cap_ms:63, enc_ms:31}` |
| 分相模型验证 | cap63+enc31=94ms → 理论顶 ≈10.6fps，实测 10.28 吻合 | stream-stop 分相计时 |
| `shot`（uia 截图单发往返） | rtt 95–166ms | 5 连发 |
| `screencap` 二进制（PNG 全分辨率） | 1.46s/次（shell rtt，含 adb 开销） | 单发计时 |

**分流结论（中间态，推翻"纯 MIUI 墙"一元论）**：uia 路径吞吐 = 1000/(cap_ms+enc_ms)，cap_ms 是 ROM 级抓帧延迟——MIUI/API34 cap 113ms（顶 5.6fps）、HarmonyOS API31 cap 63ms（顶 ≈10.6fps，实测 10.28）。即 **MIUI 墙属实但不是唯一墙**：uia 路径在不同 ROM 上顶 5.6–10.3fps 浮动，**已测两台零售机均不达 15fps**；另注意 HarmonyOS 上 cap+enc 流水线化理论顶 ≈1000/63≈15.9fps——流水线化在该 ROM 上首次具备跨 15fps 潜力（MIUI 上仅 ~8.9），是否实装另行裁决。

## 结论

**MIUI API34 上 shell 可达的高帧抓帧面被系统性封死**：
takeScreenshot 是唯一可用源，ceil 5.6fps；所有绕过 a11y 的
surface/composition 路径（VD 镜像全变体，含 `screenrecord` 同构）
在此 ROM 上产 0-2 帧——与 surface usage、secure flag、电源态无关，
是 SurfaceFlinger 策略层封锁。

## 决定

1. **感知档维持 uia 路径**（5.6fps 实测）——SoM 标注/树-图融合等
   AI 感知工作流在此档可用（感知不是游戏/CV 场景）；
   高帧档（≥15fps，CV/实时反馈）在本机**不可达**。
2. **Streamer 保留**（media 通道/producer-writer/背压设计不变，
   参见 ADR-006），新增 `stream-stop` 观测面：
   `{cap, cap_nulls, cap_ms, enc_ms}`——cap 后端名与分相计时
   成为后续 cap 后端插拔的固定观测契约。
3. **仅存的 ≥15fps 通路**（按可行性排序，本期均不实现）：
   a. `MediaProjection`（唯一残路）：`media_projection` service →
      consent UI 交互授权 → projection VD。代价：每次 agent 启动
      一次屏幕授权弹窗（headless 场景违和），且需 ActivityThread
      system-context 机制（本机实测进程自杀，需另解）；M 级。
   b. capture→encode **流水线化**（capture 113ms 与 encode 67ms
      重叠）可抬 uia 档至 ~8.9fps 理论顶——仍不达 15，暂缓
      （复杂度换 60% 提速不值，若未来 MP 通路仍封可再议）。
4. **跨 ROM 注记**（2026-11-26 修订，获第二数据点）：MIUI
   API34 的 5.6fps 顶是已测最劣；HarmonyOS API31 同路径实测
   10.28fps——「MIUI 墙」属实但非唯一墙，uia 路径封顶随 ROM
   抓帧延迟（cap_ms 63–113ms）在 5.6–10.3fps 间浮动，已测两台
   零售机均不达 15fps。`surface` 后端接口仍预留——AOSP/Pixel
   类 ROM 上 VD 镜像（scrcpy 同构）预计 30-60fps 仍未实测。

## 影响

- 验收项「≥15fps 实测」按证据判 **partial**：substrate（daemon/
  通道/client/观测面）齐备可用，但本机抓帧源物理上限 5.6fps。
- `fps` 参数语义不变（请求节奏，非可达承诺）；`effective_fps_hint`
  继续如实回显。
- Track A SoM/Track B 帧流需求按 uia 档设计（实测 ROM 区间
  5.6–10.3fps），高帧场景待 MP 通路或流水线化（HarmonyOS 上
  理论可跨 15fps）再行裁决。
