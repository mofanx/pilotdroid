# ADR-001 · ADB client 层：vendor 优先，重写兜底

> 状态：已接受 · 日期：2026-11-19

## 背景

独立开发需要决定 ADB host 协议（smartsocket、shell v2、sync、forward）的实现方式。AOSP `protocol.txt`/`SERVICES.TXT` 文档齐全，自研可行；同时 Rust 生态已有成熟实现。

## 候选

| 方案 | 优点 | 缺点 |
|---|---|---|
| A. `adb_client` crate（依赖或 vendor） | 现成覆盖 TCP server/USB、shell、push/pull、forward；Apache/MIT | 依赖上游维护 |
| B. `std::process` 调 `adb` CLI | 零依赖、行为与官方 adb 完全一致 | 每调用 fork 20-30ms（实测底噪）、输出解析脆弱、无法持连接 |
| C. 自研 smartsocket/shell v2/sync | 完全自主 | 1-2 周工作量，零差异化价值 |

## 决定

**选 A，默认依赖 `adb_client`（vendor 进 `pilotdroid-adb` 封装层），B 作降级路径，C 仅作兜底条款保留。**

- 用一层薄封装 `pilotdroid-adb` 隔离 API，替换实现不影响上层。
- 无 agent 的兜底操作（`screencap`/`uiautomator dump`/exec-out）经 B 路径实现，天然双保险。

## 理由

"独立开发"的目标是摆脱对**上游架构与维护节奏**的依赖（u2 的风险），不是重造每个轮子。ADB 层不产生用户可感知差异，投入应全部押在 agent 与协议层。

## 退出条款（何时改走 C）

`adb_client` 出现以下任一情况即评估重写：上游停更 >6 个月且有未修缺陷；缺 forward/localabstract 或 shell v2 关键能力且无法上游提交；许可证变更。

## 后果

- 正面：M1 省 1-2 周；双路径（crate + CLI 兜底）天然健壮。
- 负面：引入第一个第三方运行时依赖——通过 vendor + 薄封装把替换成本固定在封装层内。
