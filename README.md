# pilotdroid

用 Rust 独立开发的 Android 自动化控制库 / CLI。借鉴 uiautomator2-rs 与 scrcpy 的工程思路，但不照抄其架构——核心目标是通过**自研持久二进制协议 + 设备端批量执行**，突破「每条指令一次 HTTP/JSON-RPC 往返」的性能天花板（u2 实测 click ~263ms、exists ~26ms、screenshot ~186ms），拿到数量级的延迟改善。

## 定位

- **协议与架构自主**：ADB client 层 vendor 成熟实现（`adb_client` 等，Apache/MIT），差异化投入集中在设备端 agent 与 RPC 协议层。
- **设备端 agent：Java dex 为能力主体**（实测裁决：`app_process` + `InputManagerGlobal` 注入 p50≈3ms、30/30 成功；`/dev/input` 写在 Enforcing 下被 SELinux 全拒）。Rust native 仅为可选监听件。
- **能力探测 + 降级链**：注入（InputManagerGlobal → 隐藏 API 兜底 → UiAutomation → `input` 命令）、截图（resident → exec-out screencap → 帧流）均按设备能力运行时探测降级，以实测为准。

## 路线图（以真机实测数据驱动）

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M0 | 基线与对拍工具：u2 / adb 内置 / app_process 原型三方延迟基线 | harness 可一键复跑，基线表落档 |
| M1 | adb_client + 常驻 dex agent + 持久 socket，核心操作可用 | click e2e p50 ≤20ms、dump ≤400ms |
| M2 | 二进制分帧 + 批量/设备端条件（wait_until）+ u2 兼容层 | 10 步序列往返 = 1 |
| M3 | selector/xpath 语义层、帧流选项、Unicode 输入、OEM 矩阵扩展 | 详见 `docs/05-roadmap.md` |

## 仓库结构

```
docs/      # 设计文档与落地方案（含 adr/ 决策记录、能力矩阵、benchmark 目标）
spike/     # 真机验证脚本与实测结果
```

详细方案见 `docs/`（随首次提交落盘）。

## License

待定——建议 MIT OR Apache-2.0 双许可（Rust 生态惯例，与上游借鉴项目兼容），见 `docs/adr/` 讨论记录。
