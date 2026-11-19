# pilotdroid

用 Rust 独立开发的 Android 自动化控制库 / CLI。借鉴 uiautomator2-rs 与 scrcpy 的工程思路，但不照抄其架构——核心目标是通过**自研持久二进制协议 + 设备端批量执行**，突破「每条指令一次 HTTP/JSON-RPC 往返」的性能天花板（u2 实测 click ~263ms、exists ~26ms、screenshot ~186ms），拿到数量级的延迟改善。

## 定位

- **协议与架构自主**：ADB client 层 vendor 成熟实现（`adb_client` 等，Apache/MIT），差异化投入集中在设备端 agent 与 RPC 协议层。
- **设备端 agent 双件制**：Java 薄片（scrcpy 式 `app_process` server：注入 / 采集 / UI 树查询）为主，Rust native 组件仅承担批量/规则执行器。
- **能力探测 + 降级链**：注入（injectInputEvent → /dev/input → `input tap`）、截图（screenrecord 流 / MediaCodec 虚拟显示 / `screencap`）均按设备能力运行时探测降级，以实测为准。

## 路线图（以真机实测数据驱动）

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M0 | 无 agent 基线：ADB vendor + 内置命令（`uiautomator dump`/`input tap`/`screencap`）端到端跑通 | 全设备零安装可用 |
| M1 | Java agent 复用 u2.jar + JSON-RPC 打通，Python uiautomator2 客户端 A/B 对拍 | 功能等价 uiautomator2-rs 核心 |
| M2 | 持久 socket + 二进制分帧 + 批量/条件执行，双协议兼容层并存 | 批量序列 N 次往返 → 1 次 |
| M3 | scrcpy 式注入与帧流接管，Java 薄片收敛为纯 UI 查询层 | 注入 P99、帧率达到实测目标线 |

## 仓库结构

```
docs/      # 设计文档与落地方案（含 adr/ 决策记录、能力矩阵、benchmark 目标）
spike/     # 真机验证脚本与实测结果
```

详细方案见 `docs/`（随首次提交落盘）。

## License

待定——建议 MIT OR Apache-2.0 双许可（Rust 生态惯例，与上游借鉴项目兼容），见 `docs/adr/` 讨论记录。
