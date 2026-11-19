# 05 · 路线图（M0-M3）

> 状态：草案 · 日期：2026-11-19
> 原则：每个里程碑有可量化验收线；验收一律以真机实测为准（参考 `spike/` 方法），不以"能跑"为准。

## 里程碑总览

| # | 名称 | 一句话目标 |
|---|---|---|
| M0 | 基线与对拍工具 | 把"更优"变成可测量命题 |
| M1 | 管线打通 | adb_client + 常驻 dex agent + 持久 socket，核心操作可用 |
| M2 | 协议差异化 | 二进制分帧 + 批量 + 设备端条件（wait_until），往返数坍缩 |
| M3 | 语义与流 | selector/xpath、UI 语义层、帧流选项、Unicode 输入、OEM 矩阵扩展 |

## M0 · 基线与对拍工具

**范围**
- bench harness：对同一设备重复测量 click / exists / dump / screenshot 的 e2e 延迟（p50/p95/p99）。
- 基线对象：python uiautomator2（真实 u2.jar）；本机 adb 内置命令基线已有（spike §3）。
- 复用 spike 脚本方法学，输出机器可读 JSON + 表格。

**验收线**
- [x] u2 真实基线已落盘 `spike/bench/baseline-u2.json`（python uiautomator2 v3.7.1）：**click 249.0ms · dump_hierarchy 330.2ms · screenshot 176.5ms · send_keys p50 309ms/p95 2021ms**；`exists` 0.02ms 为本地惰性求值（无 RPC）。测量顺序已保证 u2 不干扰我方 resident 测量（我方测完后才起 u2，测后已 kill+清理）。
- [ ] harness 可一键复跑（`make bench` 或脚本），纳入 CI 可跑部分（模拟器）。

## M1 · 管线打通

**范围**
- host 库骨架 + `adb_client` 集成：设备枚举、push、forward（localabstract）、exec-out。（骨架已在 `feature/rust-skeleton` 落地）
- dex agent v1：HELLO/capability、INJECT（tap/swipe/key/MotionEvent 序列）、SCREENSHOT（策略 A，失败回退 B）、DUMP（裸 app_process 反射 `UiAutomation` 常驻遍历——spike-2 已裁决无需 runtest/instrumentation）。
- 生命周期：deploy→setsid 拉起→握手→心跳→断连存活→**adbd 重启/设备重启自动重部署**（spike-2 实测 daemon 抗断连不抗 adbd 重启）。

**验收线**（Mi 11/A14 实测 — M1 e2e 结果见 `spike/e2e/RESULTS-M1.md`）
- [~] click e2e **p50 ≤ 20ms**：实测 rtt p50=**48.5ms**、设备内注入 **4.9ms**。未达 e2e 线——瓶颈为 MIUI adb-USB 传输底噪 ~40ms（裸 `adb shell` RTT 20-30ms），非 agent。M2 持久连接+BATCH 摊薄后复测。对 u2（249ms）已达 **5.1×**。
- [x] 注入成功率 **≥ 99%**：实测 30/30，logcat `MIUIInput` 旁证。
- [x] resident dump **p50 ≤ 400ms**：实测 rtt p50=**104ms**（设备内 ~7ms），对 u2（330ms）**3.2×**。
- [~] 按需截图 **p50 ≤ 150ms**：实测 p50=**171ms**（min 127ms），JPEG 编码+420KB 回传为主成本，与 u2（176ms）持平。
- [x] agent 断连存活：`adb kill-server` 前后同 pid，重连正常。
- [ ] 全部能力失效时退化可用——注意 `input` 兜底在非 INJECT_EVENTS 设备上同样被拒（duchamp 实测），降级链末端是 `screencap`/`exec-out` 只读路径（stretch 未实现）。

**口径注记（spike-2 复核保留）**：
- waitfinish 验收须注明前台条件——该模式延迟随注入目标变化（3ms 轻量前台 ~15ms 重前台），故性能线按 async 口径。
- capability bitmap 探测须含「真实注入一次+观察进程存活」（duchamp 型设备 connect 成功但 inject 即死）。

## M2 · 协议差异化（合并原 M2+M4）

**范围**
- 自研二进制分帧上线（02-protocol §2/§3），u2 JSON-RPC 兼容层并存。
- BATCH + wait_until 设备端执行；CANCEL；stream-more 中间结果。
- TEXT（ASCII 注入 + broadcast 剪贴板 Unicode 兜底）。
- **不做**独立规则引擎 DSL——wait_until + 条件参数化够用即停（需求实证后再评估）。

**验收线**（实测见 `spike/e2e/RESULTS-M2.md` + `e2e-m2.json`，feature/m2-e2e）
- [x] 混合序列**往返数 = 1**：实测 tap→wait_until→tap 单帧 BATCH e2e p50=**58.5ms** vs u2 等价 **612.9ms**（**10.5×**）。
- [x] 批量内相邻注入摊销 **≤ 5ms/次**：NOACK tap 单连接 n=100 p50=**0.0ms**（fire-and-forget）；带 ack tap p50=54.2ms 仍为传输抖动主导。
- [x] wait_until 命中零主机往返：设备内 p50=**7ms**（poll 周期内），e2e 50ms 含传输。
- [ ] 兼容层：python uiautomator2 客户端连接 compat 端口冒烟（未实现，移 M3）。

## M3 · 语义与流

**范围**
- selector / xpath 查询层（XML 树或结构化 node 数组），对齐 u2 语义集。
- 帧流：策略 C+（VirtualDisplay+MediaCodec）或 JPEG 序列帧，按 spike-2 结果选型。
- Unicode IME 选项；native sidecar（/dev/input 监听）按需评估是否值得保留。
- 04 矩阵扩展至 P1 设备集。

**验收线**
- [ ] xpath 查询对 u2 既有 fixture 集结果一致（对拍）。
- [ ] 帧流 ≥15fps（h264）或声明放弃 C+ 采用兜底 C 并记录理由。
- [ ] 非 ASCII 文本输入可用（至少 broadcast 路径）。
- [ ] P1 设备集（Pixel/类原生 + A10-12）矩阵行填齐，无 P0 级能力缺失。

**M3 已交付（feature/m3-compat，实测 `spike/e2e/RESULTS-M3.md`）**
- [x] **u2 兼容 shim v0**（host 侧翻译层，ADR-005）：`compat/u2shim.py` 顶层 API（connect/click/d(text|resourceId).click|exists|wait/dump/screenshot/set_text/app_start/press/swipe）——同一脚本零改动跑通两侧，n=20 对拍：**device-ops 小计 212ms vs u2 2140ms = 10.1×**；序列总 1956 vs 4085ms = 2.1×（app_start/screenshot 固定成本摊薄，如实记录）；swipe 79×、press 42×、sel_click 9.6×。
- [x] KEY op（任意 keycode）、INJECT 事件序列、backend 强制字段、SHOT quality/take_ns-enc_ns 分段——verify 21/21。
- [x] 仲裁实验：SHOT 固定成本主导（enc 随像素非 quality）；backend A/B IM 快 21ms 维持默认；keep-alive 负结果（~43ms 尾=adbd 连续请求批量量子）；dash 2602BRT18C/SDK37 矩阵行 caps=95。
- [x] watchdog 分级恢复：kill -9 后无 --jar 调用 1.15s 自动恢复。
- 剩余：xpath fixture 对拍、Unicode IME、P1 矩阵扩展（帧流移交 M4 并交付）。

## M4 · 帧流与 daemon 韧性

**范围**
- media channel 帧流（ADR-006）：第二 localabstract `pilotdroid_agent_media`，ch=1 JPEG 序列帧，§4.8 契约。
- `agent stream` CLI + 夜间 soak（无人值守 daemon 韧性验证）。

**M4 已交付（feature/m4-e2e，实测 `spike/e2e/RESULTS-M4.md`）**
- [x] **帧流 ≥5fps @ scale=0.5 持续 ≥60s**：实测 **6.02fps / 391帧/65s / 3.7Mbps / dropped=0 / seq_gaps=0**（Mi11/A14）；req fps=10 → 6.0 有效（takeScreenshot ~115ms + JPEG ~66ms/帧 = ~6fps 天花板，E1 固定成本归因兑现）。
- [x] **流期间 control 不阻塞**：第二会话并发 tap p50=5.95ms、5-tap batch=16.4ms（=无流基线）。
- [x] **stop 后无残留**：post-stream verify-agent.sh 27/27（media EOF、owner 会话绑定、ERR_STREAM_BUSY 全部实测）。
- [x] **摊销收益量化**：SHOT 轮询天花板 5.07fps（197.2ms e2e @0.5）→ 流 6.02fps（+18%）且控制通道全程可用；带宽 77.6KB/帧 vs 全分辨率 ~420KB。
- [x] agent 侧：takeScreenshot 线程亲和性经 main-Looper marshal 解决（AMS 拒绝第二 UiAutomation 连接）；host 侧：MediaClient/StreamStats/`agent stream` 全链路。
- [x] **夜间 soak**：`spike/e2e/soak-m4.sh`——session bench + 周期 kill-9/watchdog 恢复循环 + RSS/fd 采样，报告 `soak-report.md`。
- 剩余：硬件编码路径（>6fps 需 SurfaceFlinger/MediaCodec，ADR-006 被否方案）、多设备流矩阵。

## 明确的排序依据

1. **先对拍工具（M0）再写功能**——避免"更优"沦为口号。
2. **M1 已含常驻 agent**——spike 证明 dex 注入 4KB 可跑，跳过"先复用 u2.jar"阶段直接自研薄片（u2.jar 仅在兼容层需要时启用）。
3. **批量与条件下沉合并进 M2**——执行器既在设备端，条件等待只是 opcode，拆成两个里程碑是人为割裂。
4. **native sidecar 降为可选件**——注入已证不需要它，剩余唯一价值是输入监听，M3 再定夺。
