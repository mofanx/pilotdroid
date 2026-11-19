# ADR-002 · 设备端 agent：Java（app_process dex）为能力主体，单进程常驻模型

> 状态：已接受（spike-1 + spike-2 双重实测裁决） · 日期：2026-11-19
> 证据：spike/RESULTS.md（注入/SELinux）、spike/RESULTS-2.md（进程模型/单进程五能力/双机复现）

## 背景

最初设想"Rust native 二进制为快路径、Java 薄片只读 UI 树"。两个前提需要实测：shell 能否写 `/dev/input`；`UiAutomation` 能否在不依赖 `uiautomator runtest`/instrumentation 的情况下获得。

## 实测（三机：Mi 11/A14、dash/A17、duchamp/A16，均 Enforcing）

| 探测 | 结果 |
|---|---|
| `/dev/input` 写 | **三机全拒**（EACCES，含 0666 节点）——native 注入路径死亡 |
| `app_process` + `InputManagerGlobal` 注入 | ✅ Mi11/dash：30/30，async p50≈1.5-4ms |
| **裸 `app_process` 反射构造 `UiAutomation`** | ✅ 三机 connect 成功：`new UiAutomation(Looper, UiAutomationConnection)` + `connect(flags=1)`，ctor+connect <100ms |
| 同进程五能力共存（UA + IM 注入 + resident dump + takeScreenshot + localabstract） | ✅ Mi11/dash 双机验证 |
| `UiAutomation.injectInputEvent` | ✅ 可用但 p50≈25-49ms（IM 的 ~10x）→ 降为兜底 |
| `UiAutomation.takeScreenshot` | ✅ p50≈13-24ms（A14）/ 78ms（A17） |
| `dispatchGesture` | ❌ 仅 AccessibilityService 持有（dex 拿不到）→ MotionEvent 序列覆盖手势 |
| daemon 存活 | 抗宿主断连（kill-server）；**死于 adbd 重启** → host 心跳+重部署必需 |
| ⚠️ duchamp 特例 | `INJECT_EVENTS` 被 OEM 收回：`input` 命令 SecurityException、app_process 注入即遭 SIGKILL——注入非处处可用，探测须含存活观察 |

## 裁决

| 方案 | 裁决 |
|---|---|
| Rust native + `/dev/input` 注入 | ❌ 否决（三机 SELinux 全拒） |
| 双进程（fast native + query Java） | ❌ 不需要——单进程五能力已验证 |
| `uiautomator runtest`/instrumentation 入口 | ❌ 不需要——裸 app_process 反射直连，省掉 runtest 开销 |
| **单 `app_process` dex agent，Java 为能力主体** | ✅ **采纳**：注入/UI 树/截图/传输/批量执行全在一进程 |
| native sidecar | 可选件（仅 /dev/input 读监听等），M3 再定去留 |

## 理由

- 全能力链在同一进程实测成立，无任何项需要第二个进程或 native。
- `UiAutomation` 获取路径已实测简化到"反射 ctor + connect(flags=1)"，省去 runtest 框架。
- `connect(FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)` flags=1 是生产安全要点：不杀用户已有无障碍服务。
- uiautomator2 v3.7.1 已演进为同款 dex-only 模型（无 APK 安装）——上游独立验证此架构方向。

## 后果

- 正面：单语言单进程、无 NDK/多 ABI、架构最简；dex <200KB 推送即走。
- 负面：隐藏 API 反射依赖 shell 身份；duchamp 型设备注入权限门控需 onboarding 处理；resident dump 在个别前台状态下可能返回空树（duchamp 0 节点，须容错）。
- 生命周期：进程抗断连不抗 adbd 重启——host 心跳 + 自动重部署为必备。
