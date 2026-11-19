# 04 · 设备能力矩阵

> 状态：持续更新 · 数据基准：t1 spike-1（2026-11-19）+ t1 spike-2（同日）+ t3 独立复核（同日，duchamp 行为新增）
> 规则：每行结论必须挂实测证据或标注"未验证"；新机型接入时重跑 `spike/scripts/` 全量并追加行。

## 1. 实测设备

| serial | 型号 | Android/SDK | SELinux | ABI | 备注 |
|---|---|---|---|---|---|
| `ba0d072d` | Xiaomi M2102K1C (Mi 11 "star", SD888) | 14 / 34 | Enforcing | arm64-v8a | 主测试机，USB 直连，1440×3200@60Hz |
| `11.0.0.2:5555` | Redmi 2602BRT18C "dash" | **17 / 37** | Enforcing | arm64-v8a | TCP，1280×2772 |
| `192.168.123.130:5555` | Redmi 2311DRK48C "duchamp" | **16 / 36** | Enforcing | arm64-v8a | TCP，1220×2712，前台常挂 Douyin —— **注入全灭机型**，见 §3 |

三机 shell 均为 `uid=2000(shell)` + `input(1004)` 组 + `u:r:shell:s0`。

## 2. 能力矩阵

| 能力 | 探测方法 | Mi 11 / A14 | dash / A17 | duchamp / A16 | 方案落点 |
|---|---|---|---|---|---|
| `/dev/input` 写 | `test -w` + `( : > node )` | ❌ 8/8 EACCES（含 0666 节点） | ❌ 5/5 拒 | ❌ 5/5 拒 | **从注入链移除**（三机独立确认，非孤例） |
| `/dev/input` 读 | `getevent -pl` | ✅ 8 设备 | ✅ NVTCapacitiveTouchScreen | ✅ focaltech_ts | native sidecar 可选监听件 |
| `InputManagerGlobal` 注入 | `injectInputEvent` 30× | ✅ 30/30，async p50≈1.5-2ms；waitfinish p50 3-15ms（**随前台目标变化**，见 §3 注） | ✅ 30/30 p50≈3.6ms | ❌ **进程被 SIGKILL** | 主路径，但需探测（duchamp 型设备不可用） |
| `UiAutomation.injectInputEvent` | dex agent 30× | ✅ 30/30 p50≈33-49ms | ✅ 30/30 p50≈25ms | ❌ **进程被 SIGKILL** | 兜底注入（慢 10x，但有 a11y 语义） |
| `input` shell 命令 | `input tap` | ✅ e2e 30-90ms | — | ❌ **SecurityException（uid=2000 无 INJECT_EVENTS）** | 最终兜底——但非处处可用，探测必做 |
| `dispatchGesture` | 反射枚举 | ❌ 不存在（仅 AccessibilityService 有，dex 拿不到） | 同 | — | **MotionEvent 序列即手势**（swipe 11/11 事件 22ms） |
| `app_process` dex 常驻 | 冷启动 wall | ✅ ~1.3s（ART ~1.1s） | ✅ | ✅ | 必须常驻；duchamp 型设备另有存活问题（§3） |
| `uiautomator dump`（一次性） | CLI dump wall | ✅ 2.25-3.13s（首调冷 ~11s） | — | — | per-call 模式反面教材 |
| **裸 `app_process` 构造 UiAutomation** | 反射 `new UiAutomation(Looper, UiAutomationConnection)` + `connect(flags=1)` | ✅ ctor ~10ms + connect ~15-28ms | ✅ | ✅ ctor 21-64ms + connect 14-42ms | **进程模型裁决：单进程可行，无需 runtest/instrumentation** |
| resident dump（`getRootInActiveWindow`+遍历） | 常驻后每轮 | ✅ **p50≈1.8-2.4ms**（30 节点） | ✅ **p50≈1.2ms**（23 节点） | ⚠️ connect 成功但返回 **0 节点**（前台 AOD/Douyin splash 状态相关，待复测） | DUMP/QUERY 主路径 |
| `UiAutomation.takeScreenshot` | 常驻调用 | ✅ 1440×3200 **p50≈13-24ms** | ✅ 1280×2772 p50≈78.5ms | 未测（进程先被杀） | 截图主路径 A |
| `SurfaceControl.screenshot` | 反射 | ❌ **A34 已移除**（NoSuchMethod） | ❌ A37 同样移除（+`DisplayCaptureArgs` 不存在） | — | **弃用**，takeScreenshot 已够用 |
| `screencap` | `exec-out screencap -p` | ✅ 380-450ms / 129-209KB PNG | — | — | 兜底路径 B |
| `exec-out screenrecord` 流 | h264 8.3s | ✅ ~19-22fps（静态；编码器 60fps 上限） | — | — | 无 agent 帧流兜底 C |
| `localabstract` + `adb forward` 回环 | `LocalServerSocket` + forward + echo | ✅ `echo_bytes=16` 真实回环 | — | — | 传输拓扑验证通过 |
| daemon 断连存活 | setsid+心跳日志 | ✅ 宿主 `adb kill-server` 存活；**`adb usb`（adbd 重启）必死** | — | ✅ setsid 存活 >50s（无 UA 时） | host 心跳 + 自动重部署是硬需求 |
| `adb install`（APK） | `pm install` 实测 | ❌ MIUI 弹窗拦截（~10s 超时自动拒绝） | — | — | 不依赖 APK 路径；注入可自确认弹窗但不作常规依赖 |
| Unicode 文本注入 | — | ⚠️ 未实现 | — | — | scope 项，见 03 §6 |
| 多 display / 旋转坐标一致性 | — | ⚠️ 未验证 | — | — | 待测 |

## 3. 约束结论（spike-2 + 复核修订）

### SELinux 层
- **`/dev/input` 写必死已三机独立确认**（A14/A16/A17，含 0666 节点）——是 `shell→input_device` avc 规则，非权限位。native 注入路径正式移除。

### 权限层（新发现：比 SELinux 更严的一刀）
- **`INJECT_EVENTS` 可被 OEM 整体收回**：duchamp（Redmi A16）上 `uid=2000` 连 `input` 命令都被 SecurityException 拒（logcat `Permission Denial ... requires INJECT_EVENTS`），且 app_process 直调 `injectInputEvent` 时**进程被 SIGKILL**（非异常返回）。同 ROM 族的 Mi 11（A14）与 dash（A17）无此现象。
- **假设**：符合 MIUI/HyperOS「USB调试(安全设置)」开关未启用的特征（该开关控制 shell 模拟点击权限）。**待证**：用户手动开启后复测。**方案落点**：注入探测必须验证「调用后进程是否存活」而非仅看返回值；onboarding 文档把该开关列为 MIUI 设备前置条件。
- 推论：`UiAutomation.connect` 成功 ≠ 注入可用（duchamp：connect✓ 但 injectInputEvent 即死）。能力探测 bit0/1 必须附带**实际注入一次并观察存活**。

### 进程层
- 单 `app_process` 进程可同时持有 UiAutomation + InputManagerGlobal + LocalServerSocket（Mi11/dash 双机验证）；duchamp 上 connect 后可存活，注入即死。
- wait_for_finish 注入延迟**随前台注入目标变化**（轻量前台 ~3ms，重负载前台 ~15ms；async 恒 ~1.5-2ms）——性能验收线按 async 口径或明确前台条件。
- daemon 抗宿主断连、不抗 adbd 重启；duchamp 心跳证实 setsid 存活正常。

## 4. 设备扩展协议（新机型接入流程）

1. `adb devices` → `spike/scripts/01-env-info.sh <serial>`。
2. `02`（/dev/input）`03`（基线）`07`（单进程全能力探针，**必含存活观察**）`06`（IM 注入）。
3. **duchamp 教训**：connect 成功不代表注入可用——矩阵行必须记录「inject 调用后进程存活与否」与 `input tap` 的 SecurityException 检查。
4. 结果追加 §1/§2；矛盾结论回填 03-device-agent 降级链。

## 5. 目标覆盖集

| 优先级 | 设备画像 | 状态/目的 |
|---|---|---|
| P0 | Xiaomi A14（Mi 11） | ✅ 全绿基准机 |
| P0 | Redmi A17（dash） | ✅ 跨大版本一致 |
| P1 | Redmi A16（duchamp） | ⚠️ **注入权限门控机型**——待用户开安全设置开关后复测；降级链关键样本 |
| P1 | Android 10-12 一台 | `InputManager` vs `InputManagerGlobal` API 分界 |
| P2 | 模拟器 x86_64 | CI 协议单测（能力结论禁用模拟器） |
| P2 | 三星 OneUI | SELinux 最严厂商压测 |
