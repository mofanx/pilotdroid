# pilotdroid v0.9.1 — unbound serve：配置零绑定补完

v0.9.0 给了会话内 `select_device` 换机，但留了一半：serve **启动时**仍要求串号可解析——零/多设备或 adb 不可达时进程直接死亡，select_device 根本进不了场。本 patch 补完另一半：**无绑定启动（unbound serve）**。

## 行为变更

- **`pilotdroid-cli serve`（不带 `-s`）**：
  - 恰好一台 ready 设备 → 自动绑定（与之前一致，零配置即用）；
  - 零台/多台 ready、或 adb 枚举失败 → **unbound 启动**：进程存活，`list_devices`/`select_device` 立即可用，其余工具显式报错引导（`no device selected — call list_devices then select_device`）而非进程崩溃；stderr 打一行提示。
- **`select_device`**：支持从 unbound 状态首次绑定（`switched_from: null`）；首次绑定失败保持 unbound（`still unbound`），不虚构回退。
- `-s <serial>` 显式指定行为不变。

## 迁移指引：废除配置绑定

MCP 配置**不再需要任何 serial 绑定**。推荐改为：

```json
"pilotdroid": {
  "command": "pilotdroid-cli",
  "args": ["serve"],
  "transport": "stdio"
}
```

（删掉 `env.PILOTDROID_SERIAL`。）插一台手机即插即用；插多台时 AI 用 `list_devices`+`select_device` 会话内任选。已有绑定配置**完全兼容**（等价于显式 `-s`），可不急着改。

## 诚实档

- unbound 启动时 `initialize` 握手的 `agentVer` 为 null（无设备可连）——客户端本就被要求容忍 null，无新问题。
- 显式 `-s` 一个不存在/离线设备仍会在首个 device 工具调用报错（未改：错误信息本就指名原因，且 lazy reconnect 可自愈）。
- adb server 完全不可达时 serve 也能 unbound 起来，但 `list_devices` 会报错——此时问题在 adb 本身，错误直通。

## 验证

- 单测 +2（unbound 拒用+绑定链、unbound 绑定失败保持 unbound），workspace **113/0**、fmt/clippy 净；
- 真机实证（两台 USB 同时在线，无 `-s` 启动）：unbound 存活 → tap 引导式拒 → list_devices 三行 current:null → select 小米 `switched:true from:null` → describe 5 元素 → 切华为 → describe 10 元素；
- install-smoke 9/0 随行。
