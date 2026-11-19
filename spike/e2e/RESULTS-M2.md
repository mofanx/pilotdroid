# RESULTS-M2 — feature/m2-e2e 真机端到端验收

- 设备: `ba0d072d`（Xiaomi Mi 11 / Android 14）；agent 0.2.0
- 分支: `feature/m2-e2e` = merge(`feature/m2-agent`@4c94437, `feature/m2-host`@ac77b4a)，基于 `feature/m2-spec`@2275b85，**零文本冲突**（首次 cargo build 遇 SIGBUS=磁盘满，清 worktree target 后通过——非代码问题）
- 全部口径: `agent bench` 单持久连接 / `agent_client.py` 同口径；e2e 含 ~40-50ms MIUI adb 传输抖动

## 行为断言
`verify-agent.sh` **15/15 PASS**（hello/caps=95/inject 可观测/swipe/dump XML/shot JPEG/ping/query_found/wait_until hit+timeout/text_visible/batch_results/shot_scale/noack/stop_clean）

## session 摊销（n=100，单连接，`agent bench`）

| op | p50 | p95 | min | 备注 |
|---|---|---|---|---|
| tap | 54.2ms | 56.0 | 14.2 | 传输抖动主导（agent 设备内 ~5ms） |
| ping | **3.3ms** | 43.2 | 2.5 | 双峰：~3ms 快路径 + ~43ms adbd 抖动尾 |
| dump | 55.5ms | 65.6 | 33.4 | |
| shot | 147.3ms | 158.2 | 104.9 | |
| **tap --noack** | **0.0ms** | 0.0 | 0.0 | fire-and-forget 发送即返回 |

## 批量序列 A/B（tap→wait_until→tap，同一确定性场景）

| | p50 | min | p95 | |
|---|---|---|---|---|
| **pilotdroid BATCH（单帧）** | **58.5ms** | 14.0 | 67.0 | n=30，全部 3×OK |
| **u2 等价脚本** | **612.9ms** | 541.4 | 649.0 | n=20（3+ 次 JSON-RPC 往返+轮询） |
| **加速比** | **10.5×** | | | |

## 单项 vs u2

| op | 我方 | u2 | 加速 |
|---|---|---|---|
| QUERY {id} | 50.0ms | —（u2 exists 走 dump 330ms 路径） | — |
| TEXT | 50.8ms | send_keys 335.4ms | **6.6×** |
| wait_until 命中 | 50.0ms（dev 7ms） | wait() 内嵌轮询，含于序列 613ms | — |
| SHOT scale=0.5 | 216.6ms / **18KB** | screenshot 176ms/~400KB | 字节 **23×** 缩减 |

注：scale=0.5 字节从 420KB→18KB，但 e2e 未显著降——takeScreenshot+JPEG 编码占主导，该屏（Settings 复杂页面）编码成本更高（142ms→217ms 场景差）。

## daemon 恢复（watchdog）

- `kill -9` agent → `alive:0`
- `agent tap --jar agent/agent.jar` → **自动恢复**：connect_recover 梯子（stop→install→start→forward→HELLO 重试一次）→ `{"ok":true,"events":2,"rtt_ms":14.4}` ✓
- 注意：`--jar` 为 opt-in；不传时报 `io error`（残留 forward 指向死 localabstract）——可考虑默认启用并内置 jar 路径

## 生态发现（新）

1. **UiAutomation 互斥**：我方 agent 持有连接时，u2 服务 `AutomatorServiceImpl` 启动即崩 `UiAutomation not connected` —— 与 u2 不能共存（对拍须先停 agent，本报告已按此流程执行并清理 u2 服务）
2. manager.rs 启动仍是 `sh -c` 包装 → ps 显示 2 进程（launcher+app_process），装饰性，建议采用 deploy-agent.sh 的 `setsid env` 单进程方案

## 遗留/转后续
- STREAM/media 通道、Key 注入：M3
- `--jar` watchdog 默认化建议
- docs/02 §7 XML-vs-结构化 UI 树未定项：M2 QUERY 已用结构化 node（更小），DUMP 保留 XML 兼容

## 复跑

```bash
cd pilotdroid-m2e2e && git checkout feature/m2-e2e
cargo build --workspace
./target/debug/pilotdroid-cli -s ba0d072d agent install agent/agent.jar
./target/debug/pilotdroid-cli -s ba0d072d agent start
agent/scripts/verify-agent.sh ba0d072d 17932            # 15/15
./target/debug/pilotdroid-cli -s ba0d072d agent bench --op tap --noack 100
```
