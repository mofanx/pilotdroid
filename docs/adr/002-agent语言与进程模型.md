# ADR-002 · 设备端 agent：Java（app_process dex）为能力主体，常驻 daemon 进程模型

> 状态：已接受（实测裁决） · 日期：2026-11-19
> 证据：spike/RESULTS.md §2/§3/§4

## 背景

最初设想"Rust native 二进制为快路径、Java 薄片只读 UI 树"。前提是 shell 身份可写 `/dev/input`（minitouch 模式）。该前提必须实测。

## 实测（Xiaomi Mi 11, Android 14, SELinux Enforcing）

| 探测 | 结果 |
|---|---|
| `/dev/input/event*` 写（含 0666 节点） | **8/8 EACCES**——SELinux `shell→input_device` 拦截，非文件权限 |
| `/dev/input` 读 | ✅ `getevent -pl` 全枚举 |
| `app_process` + `InputManagerGlobal.injectInputEvent` | ✅ **30/30 成功，p50≈3ms**（async ~2.2ms），logcat `MIUIInput` 60 行实证分发 |
| `app_process` 冷启动 | ~1.3s wall（ART ~1.1s） |
| `input` 命令 e2e | 30-50ms |

## 候选与裁决

| 方案 | 裁决 |
|---|---|
| Rust native + `/dev/input` 注入 | ❌ 否决：目标设备类群 Enforcing 下不可用，且 3ms 的 Java 注入已使"更快"失去意义 |
| 纯 Java agent（dex） | ❌ 独立不完整：`/dev/input` 监听、文件/进程杂务等 native 友好场景缺失——但这些全是**可选**能力 |
| **Java agent 为能力主体 + native 可选 sidecar** | ✅ **采纳**：注入/截图/UI 树/帧流全在 Java 层；native 仅做输入监听等可选件，M3 再定去留 |

进程模型：**常驻 daemon**——`setsid app_process` 拉起，`localabstract` 监听，host 断连不死，设备重启重部署。冷启动 1.3s 决定 per-call spawn 不可行。

## 理由

- 证据直接：唯一实测可行的快注入路径在 Java 层（scrcpy 同款 `InputManager` 反射，且其兼容性已被 scrcpy 在 API 21+ 全量验证）。
- UI 树（accessibility）、UiAutomation、MediaCodec 本就必须在 Android framework 内跑——Java 层反正躲不掉，不如让它当主体。
- native 注入"更快"的假设在本机不成立（写不了），即使可写设备上也仅省去已很小的 3ms 中的分发前段。

## 后果

- 正面：单语言 agent、无需 NDK/多 ABI 构建、无 SELinux 灰色地带。
- 负面：dex 需 javac+d8 开发期工具链；隐藏 API 反射依赖 shell 身份（app 内不可用）；若未来要做低层输入监听需另引 native 构建链。
- 遗留：常驻 UiAutomation dump 延迟、resident screenshot 延迟待 spike-2 坐实（见 04-capability-matrix.md）。
