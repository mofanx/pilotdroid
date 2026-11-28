# 在其他项目使用 pilotdroid（Devin / MCP 接入）

极简接入说明——目标：任意项目目录里的 Devin 会话都能直接操控安卓真机。
以 Ubuntu + Devin CLI 为例；MCP 配置片段对任何 MCP 客户端通用。

## 1. 设备前置（一次性）

- 目标机开 **USB 调试**，`adb devices` 能看到设备（`device` 状态）；
- 首次使用执行一次 `pilotdroid-cli -s <serial> setup`——推送 device-side
  agent 并验证注入通路（之后无需重复，除非升级 agent.jar）。

## 2. 安装 CLI 到 PATH（一次性）

```bash
# 任选其一：
pip install pilotdroid            # 首次调用 pilotdroid-cli 时自动拉 release 二进制
# 或手动：
gh release download -R mofanx/pilotdroid -p pilotdroid-cli-linux-x86_64 \
    -O ~/.local/bin/pilotdroid-cli --clobber && chmod +x ~/.local/bin/pilotdroid-cli
# 或 brew install mofanx/tap/pilotdroid
pilotdroid-cli --version          # 核验
```

## 3. Devin 用户级 MCP 注册（一次性，全项目生效）

```bash
devin mcp add pilotdroid -s user \
    -e PILOTDROID_SERIAL=<你的serial> \
    -- pilotdroid-cli serve
```

写入 `~/.config/devin/mcp_config.json`——所有项目的会话都能用。

**免确认**（可选但推荐，否则每个工具调用要按一次确认）：
`~/.config/devin/config.json` 的 `permissions.allow` 加一条
`"mcp__pilotdroid__*"`。

## 4. serial 指定方式（三级，按需选）

| 级 | 方式 | 适用 |
|---|---|---|
| 注册时 env | `-e PILOTDROID_SERIAL=ba0d072d` | 主力机固定——开箱即用 |
| 单机自动 | 不配 serial：恰好一台设备 ready 时自动选 | 只插一台机 |
| 项目覆盖 | 项目 `.devin/mcp_config.json` 同名 server 覆盖 user 级 | 某项目要调另一台机 |

多设备同时连着又没指定 serial 时 `serve` 会报
`multiple devices ready — pick one with -s <serial>`。

## 5. 使用姿势（盲驾四拍）

```
describe_screen            # 观测：elements 带 id(SoM)/i(tap 空间) 双字段
tap_index {i}              # 动作：用 elements 里的 i，不要用图编号 id
verify_after               # 闭环：动作+断言一次调用
screenshot_annotated       # 需要看图时：编号框 + 同帧 elements（i 直读）
```

完整工具面见 README「MCP tools」；u2 兼容 API 见 `compat/SUPPORTED.md`。

## 排错速查

- 工具调用报 `no element snapshot/stale` → 重 `describe_screen` 再操作
  （UI 已漂移，guard 在保护你）；
- `connect error` → `adb devices` 确认设备在线；`pilotdroid-cli -s <serial> setup` 重跑；
- 无线/离线设备（如 `x.x.x.x:5555 offline`）不被支持，用 USB。
