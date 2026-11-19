# RESULTS-M1 — feature/m1-e2e 真机端到端验证

- 设备: `ba0d072d`（Xiaomi Mi 11 / Android 14 / SELinux Enforcing）
- 分支: `feature/m1-e2e` = merge(`feature/m1-agent`@b52585d, `feature/m1-host`@f892113)，**零冲突**
- 链路: `pilotdroid-cli agent install → start → tap/dump/shot`（Rust CLI 全链路，每次调用独立 spawn → adb connect → forward → TCP → HELLO → op）
- 采样: 每 opcode n=30，`rtt_ms` 为 client 内围绕 op 的计时；`device_ms` 为 agent 上报的 on-device 执行时间

## 实测结果（spike/e2e/e2e-m1.json）

| op | 我方 e2e p50 | 设备内 p50 | u2 基线 p50 | 加速比 |
|---|---|---|---|---|
| click (tap) | **48.5ms** | 4.89ms | 249.0ms | **5.1×** |
| dump (25 nodes XML) | **104.0ms** | ~7ms | 330.2ms | **3.2×** |
| shot (JPEG ~420KB) | 171.3ms | ~24ms+encode | 176.5ms | 1.0× |

- 注入 30/30 `ok:true events:2`（logcat `MIUIInput` 注入日志旁证）
- dump XML 以 `<hierarchy><node…>` 校验有效
- shot `ffd8` magic + 1440×3200 baseline JPEG 可解码

## 对照 docs/05-roadmap.md M1 验收线

| 验收线 | 结果 | 说明 |
|---|---|---|
| click e2e p50 ≤20ms | **未达 (48.5ms)** | 设备内注入 4.9ms 达标；~40ms 是 MIUI adb-over-USB 传输底噪（`adb shell echo` 裸 RTT 即 20–30ms，Rust/Python 客户端同样命中）。传输优化（持久连接复用 + BATCH）是 M2 项 |
| 注入成功率 ≥99% | **达标** (30/30，logcat 抽验) | |
| resident dump p50 ≤400ms | **达标** (rtt 104ms / 设备内 ~7ms) | 一次性 uiautomator dump 2.5s 的 1/24 |
| 截图 p50 ≤150ms | **接近未达** (171ms, min 127) | JPEG 编码 + 420KB 回传是主成本；quality 可调 |
| agent 断连存活/重连 | **达标** | `adb kill-server` 前后同 pid，重连 tap 正常 |
| 失效退化 | 未实现（stretch） | |

## 集成期发现的契约分歧与修复（本轮实际修改）

两侧并行实现 payload schema 分叉，e2e 首次对接时 `agent tap` 报 `missing field 'ms'`、dump 无输出。已按 proto crate `payload` 模块（自声明 wire contract）统一：

- INJECT req：`{kind:"Tap"|"Swipe"|"Key", points:[{action,x,y,t_ms}], wait_finish}`（points 模式统一 tap/swipe/任意手势，优于 docs/02 §7 的扁平 op 模式）
- INJECT ack：`{ok,events,ms}`（`cost_ns` 保留为 serde 忽略的观测字段）
- DUMP ack：`{xml}`（原 `data` 改名）
- ERROR：`{code,message}`（保留 `error` 别名给 python dev client）
- Java agent 保留旧 `op` 路径，`agent_client.py`/`verify-agent.sh` 全部兼容，6/6 断言仍过
- **docs/02 §7 需更新为 points 契约**（建议由 docs 负责方落笔）

## 遗留问题（非阻塞，转 p3/后续）

1. **forward 泄漏**：每次 CLI `agent tap/dump/shot` 在 adbd 注册 forward 后无 `--remove`，实测累积（3 次调用后 `forward --list` 多出 4 条）。建议 `AgentManager::connect` 返回的句柄 Drop 时 remove，或 CLI 复用固定端口。
2. 传输底噪：e2e rtt 含 ~40–90ms adbd 调度/转发开销，MIUI 上不可消除——持久连接 + BATCH 摊薄即可（协议已预留 BATCH opcode）。
3. `Key` kind 未实现（M1 范围外），INJECT req `{kind:"Key"}` 返回 UNSUPPORTED error。
4. 设备上检测到 2 个 agent app_process 并存（deploy 多次残留），`agent stop` 按类名 pkill 可全清，无实际影响。

## 复跑

```bash
cd pilotdroid-e2e && git checkout feature/m1-e2e
cargo build --workspace
agent/scripts/deploy-agent.sh ba0d072d            # 或 CLI agent install + start
./target/debug/pilotdroid-cli -s ba0d072d agent install agent/agent.jar
./target/debug/pilotdroid-cli -s ba0d072d agent start
./spike/e2e/bench-e2e.sh ba0d072d 30               # -> spike/e2e/e2e-m1.json
agent/scripts/verify-agent.sh ba0d072d 17932       # Java 侧 6/6 断言
```
