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
- [ ] 产出基线表：u2 vs `adb` 内置命令 vs `app_process` 原型（已有注入 3ms 数据），样本 ≥30 次/项。
- [ ] harness 可一键复跑（`make bench` 或脚本），纳入 CI 可跑部分（模拟器）。

## M1 · 管线打通

**范围**
- host 库骨架 + `adb_client` 集成：设备枚举、push、forward（localabstract）、exec-out。
- dex agent v1：HELLO/capability、INJECT（tap/swipe/key/MotionEvent 序列）、SCREENSHOT（策略 A，失败回退 B）、DUMP（resident UiAutomation 或 uiautomator-dump 桥接）。
- 生命周期：deploy→setsid 拉起→握手→心跳→断连存活→重部署。
- **前置 spike-2**（开工前先跑，半日）：resident dump 延迟、resident screenshot 延迟、dispatchGesture 对比——填 04 矩阵空格。

**验收线**（Mi 11/A14 实测 — M1 e2e 结果见 `spike/e2e/RESULTS-M1.md`，feature/m1-e2e）
- [~] click e2e **p50 ≤ 20ms**：实测 rtt p50=**48.5ms**、设备内注入 **4.9ms**。未达 e2e 线——瓶颈为 MIUI adb-USB 传输底噪 ~40ms（裸 `adb shell` RTT 20-30ms），非 agent。M2 持久连接+BATCH 摊薄后复测。对 u2（249ms）已达 **5.1×**。
- [x] 注入成功率 **≥ 99%**：实测 30/30，logcat `MIUIInput` 旁证。
- [x] resident dump **p50 ≤ 400ms**：实测 rtt p50=**104ms**（设备内 ~7ms），对 u2（330ms）**3.2×**。
- [~] 按需截图 **p50 ≤ 150ms**：实测 p50=**171ms**（min 127ms），JPEG 编码+420KB 回传为主成本，与 u2（176ms）持平。
- [x] agent 断连存活：`adb kill-server` 前后同 pid，重连正常。
- [ ] 全部能力失效时退化到 `input`/`screencap`/`uiautomator dump` 可用（stretch 未实现）。

## M2 · 协议差异化（合并原 M2+M4）

**范围**
- 自研二进制分帧上线（02-protocol §2/§3），u2 JSON-RPC 兼容层并存。
- BATCH + wait_until 设备端执行；CANCEL；stream-more 中间结果。
- TEXT（ASCII 注入 + broadcast 剪贴板 Unicode 兜底）。
- **不做**独立规则引擎 DSL——wait_until + 条件参数化够用即停（需求实证后再评估）。

**验收线**
- [ ] 10 步混合序列（tap→wait_until→tap→screenshot…）**往返数 = 1**；同序列 u2 需 10 次。
- [ ] 批量内相邻注入摊销 **≤ 5ms/次**（wait_finish=false 时）。
- [ ] wait_until 命中延迟 ≤ 设备端 poll 周期 + 1 帧，且全程零主机往返。
- [ ] 兼容层：python uiautomator2 客户端连接 compat 端口完成 click/dump/screenshot 冒烟全通过。

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

## 明确的排序依据

1. **先对拍工具（M0）再写功能**——避免"更优"沦为口号。
2. **M1 已含常驻 agent**——spike 证明 dex 注入 4KB 可跑，跳过"先复用 u2.jar"阶段直接自研薄片（u2.jar 仅在兼容层需要时启用）。
3. **批量与条件下沉合并进 M2**——执行器既在设备端，条件等待只是 opcode，拆成两个里程碑是人为割裂。
4. **native sidecar 降为可选件**——注入已证不需要它，剩余唯一价值是输入监听，M3 再定夺。
