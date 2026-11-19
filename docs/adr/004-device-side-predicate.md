# ADR-004 · 条件等待谓词下沉设备端（wait_until / Selector DSL）

> 状态：已接受 · 日期：2026-11-19
> 规格：docs/02-protocol.md §4.3
> 修正：ADR-003 「控制/媒体通道在同一 socket 复用」调整为「media 帧流走第二条独立 socket」（§4.6）

## 背景

u2 的 `exists/wait` 语义在 host 侧轮询：每次轮询 = 一次 HTTP/JSON-RPC 往返。
本机实测（spike/e2e，持久连接）forward RTT 中位 ~40ms、p95 ~55ms，而设备内
UiAutomation 树遍历仅 ~3ms。若 wait_until 由 host 轮询，20ms 的期望 poll
周期会被放大为每 tick 一个完整 RTT——**轮询越快浪费越大**，且注入-等待-注入
序列的端到端被往返次数线性放大。

## 决定

谓词在设备端求值：

- `wait_until` 是 BATCH 内 opcode，agent 在常驻 UiAutomation 树缓存上以
  `poll_ms`（默认 20ms）自旋，命中/超时返回 op 级结果。
- Selector 用四元组 `{text?, id?, desc?, index?}` AND 语义——参数化表达
  覆盖 u2 常用选择面（text/id/description/index），**不引入 Lua/表达式
  DSL**（规则引擎 = wait_until + 参数化 selector，够用即停，YAGNI）。
- QUERY opcode 复用同一 Selector，一次性求值返回 `{found, node}`。

## 备选与否决

| 方案 | 否决理由 |
|---|---|
| host 侧轮询 DUMP | 每 tick = RTT（p50 40ms），5s 等待 ≈ 125 次往返；且 DUMP 全树 ~10KB 只为判断一个节点存在 |
| Lua/小型表达式 DSL | selector 四元组已覆盖 exists/wait 主用例；DSL 增加 agent 体积与维护面，留作 §7 未定项 |
| 谓词编译为字节码下发 | 同上，过度设计 |

## 后果

- agent 需维护常驻树遍历（spike-2 已证 ~3ms/次可行）+ 节点命中包装。
- selector 语义两侧各一份实现风险 → proto payload struct 为规范源，
  字段名即 wire 契约（同 §3 各 Ack）。
- `index` 参与存在性语义（第 N 命中），文档已写明。
