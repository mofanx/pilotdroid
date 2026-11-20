# pilotdroid

用 Rust 独立开发的 Android 自动化控制库 / CLI。借鉴 uiautomator2-rs 与 scrcpy 的工程思路，但不照抄其架构——核心目标是通过**自研持久二进制协议 + 设备端批量执行**，突破「每条指令一次 HTTP/JSON-RPC 往返」的性能天花板（u2 实测 click ~263ms、exists ~26ms、screenshot ~186ms），拿到数量级的延迟改善。

## 定位

- **协议与架构自主**：ADB client 层 vendor 成熟实现（`adb_client` 等，Apache/MIT），差异化投入集中在设备端 agent 与 RPC 协议层。
- **设备端 agent：Java dex 为能力主体**（实测裁决：`app_process` + `InputManagerGlobal` 注入 p50≈3ms、30/30 成功；`/dev/input` 写在 Enforcing 下被 SELinux 全拒）。Rust native 仅为可选监听件。
- **能力探测 + 降级链**：注入（InputManagerGlobal → 隐藏 API 兜底 → UiAutomation → `input` 命令）、截图（resident → exec-out screencap → 帧流）均按设备能力运行时探测降级，以实测为准。

## 三种调用面（M5，ADR-007）

同一设备后端，三种接入形态，按调用方选：

| 形态 | 适合 | 开销 | 入口 |
|---|---|---|---|
| `pilotdroid-cli agent <op>` | 人工/脚本单发 | ~85ms/call（含 fwd+connect+HELLO） | 子进程每次拉起 |
| `pilotdroid-cli serve`（stdio MCP） | **AI agent 一行接入** | 持久会话摊销 ~0 | `pilotdroid-cli serve` + JSON-RPC 21 工具 |
| Rust `pilotdroid-agent` facade | Rust 库内嵌 | 持久 Session | `Device::with_transport(..).session()` |

另有 **u2 兼容 shim**（`compat/u2shim.py`，host 侧翻译层，`pip install ./python`
后 `import pilotdroid`）：u2 脚本零改动跑通。
支持集（明确不暗示全覆盖）：`click/long_click/swipe/press/set_text/dump_hierarchy/
screenshot/app_start/app_stop/app_current/info` + selector `text/resourceId/
description/className/packageName/scrollable/index` + `d(scrollable=True).scroll.*`；
仅 `xpath` 显式拒绝（ValueError，非目标）。M6 真实脚本对拍：**23/24 op 通过，唯一
unsupported = xpath**（`spike/e2e/e2e-m6-compat.json`）；方法级矩阵见
`compat/SUPPORTED.md`。**xpath 明确不在
supported-set 目标内**（u2 xpath 本为可选插件，非 drop-in 承诺）。

MCP client 挂接（mcpServers 片段，二进制名 `pilotdroid-cli`，cargo install 后在 PATH）：

```json
{"mcpServers": {"pilotdroid": {"command": "pilotdroid-cli", "args": ["serve"]}}}
```

多设备场景补 `-s`：`"args": ["serve", "-s", "<serial>"]`（`-s` 是 serve 子命令选项）。

经官方 python `mcp` SDK 与 Devin CLI 实测握手通过（`spike/e2e/mcp_client_check.py`
+ `agent_e2e_transcript.txt`）；initialize 应答同时透出 `agentVer`（设备 jar 版本）。

## 实测数据（真机，同口径）

| 口径 | pilotdroid | uiautomator2 | 来源 |
|---|---|---|---|
| 同脚本端到端（21 op 总墙钟，M7） | 4769ms | 41909ms | **8.8×** `spike/e2e/RESULTS-M7.md` |
| 同脚本端到端（adb-TCP 无线口径） | 5915ms | 43874ms | **4.7–7.4×**（op 混入敏感）`spike/e2e/RESULTS-M7.md` |
| 设备内 op 序列（shim 对拍 n=20） | 212ms | 2140ms | **10.1×** `spike/e2e/RESULTS-M3.md` |
| 批量 tap→wait→tap e2e | 58.5ms | 612.9ms | **10.5×** `spike/e2e/RESULTS-M2.md` |
| 帧流 live view | 6.0fps@0.5 持续 65s | 无此能力 | `spike/e2e/RESULTS-M4.md` |
| CLI vs 持久路径 | +85ms/call 固定成本 | — | `spike/e2e/RESULTS-M5.md` |
| 中文/Unicode 文本输入 | `ACTION_SET_TEXT` 原生支持 | 需 ADBKeyboard IME | `verify-agent.sh` `text_unicode` |
| 设备矩阵 | Mi11/A14（USB）verify 33/33 + dash/A17（**adb-over-TCP 已通**）核心 op 全过 | — | `verify-agent.sh`；无线仅 1 台 TCP 验证 |

## 路线图（以真机实测数据驱动）

| 里程碑 | 内容 | 验收 |
|---|---|---|
| M0 | 基线与对拍工具：u2 / adb 内置 / app_process 原型三方延迟基线 | harness 可一键复跑，基线表落档 |
| M1 | adb_client + 常驻 dex agent + 持久 socket，核心操作可用 | click e2e p50 ≤20ms、dump ≤400ms |
| M2 | 二进制分帧 + 批量/设备端条件（wait_until）+ u2 兼容层 | 10 步序列往返 = 1 |
| M3 | selector/xpath 语义层、帧流选项、Unicode 输入、OEM 矩阵扩展 | 详见 `docs/05-roadmap.md` |
| M5 | agent 原生调用面（serve/facade/shim v1） | 详见 `docs/05-roadmap.md` + ADR-007 |
| M6 | 真实兼容性 + 可分发（pkg selector / Unicode 实证 / pip 包 / TCP 矩阵） | 详见 `docs/05-roadmap.md` |

## 仓库结构

```
docs/      # 设计文档与落地方案（含 adr/ 决策记录、能力矩阵、benchmark 目标）
spike/     # 真机验证脚本与实测结果
```

详细方案见 `docs/`（随首次提交落盘）。

## License

MIT，见 `LICENSE`。
