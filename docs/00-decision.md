# 00 · 决策记录：独立开发的范围与边界

> 状态：已确认（有 t1 实测支持） · 日期：2026-11-19
> 关联：spike/RESULTS.md、adr/001、adr/002、adr/003

## 1. 决策

**独立开发，但"独立"只落在有差异化价值的层。** 具体边界：

| 层 | 策略 | 理由 |
|---|---|---|
| ADB client（smartsocket / shell / sync / forward） | **vendor** 成熟 Rust 实现（`adb_client` 等，Apache/MIT），重写兜底 | 无差异化价值；AOSP 协议文档齐全，该层风险最低。见 adr/001 |
| 设备端 agent（注入 / UI 树 / 截图 / 流） | **自研** | 拉开差距的核心。实测表明能力主体必须是 Java（app_process dex），见 adr/002 |
| RPC 协议（持久 socket + 二进制分帧 + 批量语义） | **自研** | u2 体系性能天花板所在（per-call HTTP/JSON-RPC）。这是"更优"的主要来源。见 adr/003 |
| host 库 / CLI | **自研** | 产品本体 |
| 兼容性兜底（u2.jar / python uiautomator2 客户端） | **可复用**（Apache 2.0），仅作 A/B 对拍与降级，不作运行时依赖 | 初期零成本打通设备端，且提供可证伪的对照基线 |

## 2. 为什么不直接 fork uiautomator2-rs

- 它的性能天花板是**架构性的**：每条指令一次 HTTP/JSON-RPC 往返。fork 只能改实现，改不了往返模型。
- 本机实测（t1 spike）：`app_process` 内注入 **p50 ≈ 3ms**，而 u2 文献值 click ≈ 263ms——数量级差距只能由"常驻 agent + 持久通道 + 批量下发"拿到，这是协议层的胜利，不是实现层的。
- 维护面：fork 后仍要追上游 Java agent（u2.jar）演进，独立性目标反而落空。

**vendor 与自研的分界线 = 该层是否贡献用户可感知的差异。** ADB 层不贡献，协议与 agent 层贡献。

## 3. 成本表（按 spike 修正后）

| 工作项 | 借鉴/依据 | 估摸工作量 | 备注 |
|---|---|---|---|
| ADB 接入 | `adb_client` crate | 2-3 天集成 | 不重写；见 adr/001 |
| dex agent 薄片（注入 + 截图 + UiAutomation dump） | spike 已验证 4KB dex 可注入 | 1-2 周 | d8 工具链仅开发依赖 |
| 持久 socket 通道（localabstract + forward） | scrcpy 同款拓扑 | 3-5 天 | 含握手/心跳/重连 |
| 二进制分帧 + 批量/条件执行器 | docs/02-protocol.md | 1-2 周 | 含设备端 wait-until |
| UI 语义层（selector/xpath） | uiautomator2-rs、sxd-xpath | 1-2 周 | M3 |
| 帧流（可选） | scrcpy-server 设计 / exec-out screenrecord 兜底 | 1 周+ | 实测兜底 ~19-22fps 已可用 |
| native sidecar（可选） | — | 3-5 天 | **仅输入监听**；注入已被 SELinux 否决，见 adr/002 |

达到 uiautomator2-rs 核心覆盖度：约 6-8 周（比纯自研 ADB 路径省约 2 周）。

## 4. 成功标准（可证伪）

- click e2e p50 比 u2 基线低一个数量级（目标 <30ms；本机注入底噪 3ms，通道余量实测核定）。
- 批量 10 步序列往返数 = 1（vs u2 的 10）。
- 截图按需 <150ms（screencap 基线 450ms，resident UiAutomation 待 spike 验证上限）。
- 全能力有降级链：任一快路径失效时退化到内置命令可用。

## 5. 非目标

- 不重写 ADB 协议栈（除非 `adb_client` 出现阻塞性缺陷，见 adr/001 退出条款）。
- 不做 root 依赖能力；所有能力在 `adb shell`（uid=2000, `u:r:shell:s0`）身份下可用为准。
- 不追求无线调试/TLS 配对直连（M 版后再议）；经 adb server 的 forward 是唯一传输假设。
- 不做 iOS、不做图像识别框架；截图/帧流只供上层消费。

## 6. 风险登记（详见 04-capability-matrix.md）

| 风险 | 实测状态 | 缓解 |
|---|---|---|
| `/dev/input` 注入 | ❌ Android 14 Enforcing 全拒（8/8 EACCES） | 已在架构中移除该路径；InputManagerGlobal 注入已验证 3ms |
| Java agent 冷启动 | ~1.3s | 常驻 daemon 化（setsid），非 per-call |
| UiAutomation 常驻 dump 延迟 | 未验证（估计 100-300ms） | M1 前做 spike-2 验证 |
| OEM 差异（非 MIUI 设备） | 未覆盖 | agent 启动自测输出 capability bitmap，见 03-device-agent.md |
| 隐藏 API 访问限制（Android 9+ greylist） | 本机反射可用 | 降级链兜底；多版本 probe 矩阵扩展 |
