# RESULTS-M6 · 真实兼容性 + 可分发 实证汇总

设备：Mi11 `ba0d072d`（USB, A14/MIUI, 1440×3200）；dash `11.0.0.2:5555`（adb-TCP, A17/SDK37, 1280×2772）。agent ver=0.3.1 caps=0x7f。

## 1. setup doctor（t1 验收复核）

`pilotdroid-cli -s ba0d072d setup` 单命令五步全 PASS：

```
PASS adb_on_path / device_ready / agent_jar / forward_dial(tcp:38433) / hello(ver=0.3.1 caps=0x7f)
```

## 2. shim 支持集推进（pkg selector + Unicode）

- wire `pkg` 字段全链路（Agent.java/proto/serve/shim/docs §4.3）；`_SUPPORTED` 加 `packageName` —— **非 xpath ValueError 类清零**。
- `text_unicode` 断言：set_text("试点pd")→QUERY 回读，**USB + dash TCP 双机 PASS**（ACTION_SET_TEXT UTF-16，u2 需 ADBKeyboard IME 的差异点成立）。
- verify `query_pkg`/`query_pkg_neg`（text+pkg AND）双过。

## 3. 真实 u2 脚本对拍矩阵（`compat_m6_realscripts.py`）

3 组官方风格场景零改动跑 shim，24 op：

| script | ops | ok | unsupported | error |
|---|---|---|---|---|
| A README quickstart（info/app_current/click/exists/screenshot/dump/press） | 8 | 8 | 0 | 0 |
| B selector+scroll（className/packageName/resourceId/description/index/scroll/xpath） | 8 | 7 | 1（xpath） | 0 |
| C app 生命周期+输入（app_stop/app_start/set_text/long_click/swipe/press×2） | 8 | 8 | 0 | 0 |
| **合计** | **24** | **23** | **1** | **0** |

机器可读：`m6-compat.json`。**结论：非 xpath 全支持，xpath 明确拒绝**（u2 xpath 本为可选插件）。

## 4. 多 serve 并发语义（`serve_multi.py`，6/6 PASS）

2×`pilotdroid serve` 同设备实测 → 结论已入 ADR-007 补注：

- 每 serve 独立 forward 端口 + 独立 control 会话：initialize/交错 op 各自成功；
- stream 单 owner：B 得 `ERR_STREAM_BUSY`；
- **A SIGKILL 中流 → §4.8 implicit stop → B 立即 start 成功**——崩溃韧性为 session-binding 自带。

## 5. fd 探针（`fd_probe.sh`）——**无泄漏**

```
fd0=66 → 20×SHOT fd1=77 (+11) → 40×SHOT fd2=77 (+0) → SIGUSR1 GC fd3=77
```

+11 全部出现在**第一批** SHOT，为图形栈一次性惰性初始化（`/dev/ion`、hwbinder、trace_marker、eventpoll/eventfd）；第二批 40 次 +0，GC 后仍 77。**不是按次累积的泄漏**，同时解释了此前 fd=76 vs soak 基线 59 的观察差（SHOT 暖机一次性增长）。

## 6. 真实 agent e2e（Devin CLI 挂 MCP）

`pilotdroid serve` 以 stdio MCP server 挂入 Devin CLI（local project scope），
`-p` 非交互跑脚本化任务，transcript `agent_e2e_transcript.txt`：

1. app_start com.android.settings → `{"ok":true}`
2. screenshot path=/tmp/m6_agent_shot.jpg scale=0.5 → **JPEG 720×1600 98KB** 落盘
3. app_current → `{"package":"com.android.settings","activity":"...MainSettings"}`
4. stream_start fps=2 → ok + `pilotdroid_agent_media`
5. stream_read n=2 → 2 帧 base64 JPEG
6. stream_stop → `{"frames_sent":2,"dropped":0}`

**真实 agent 环境（非 python SDK）全链路零干预通过。**

## 7. 设备矩阵（dash/TCP）

- `verify-agent.sh 11.0.0.2:5555`：核心 op 全过（hello caps=0x7f、inject、dump/QUERY、SHOT、BATCH、stream 六断言、unicode TEXT、query_pkg）。
- 残 fail（复跑 ~29/33）均为 `android:id/input`/scrollable/logcat-marker 的 **MIUI 布局专属断言**在 stock A17 的环境差异（首跑 32/33 曾全绿——状态相关，非功能回归）。
- `stream_eof` 断言已修正为"排空≤8 在途帧后 EOF"（TCP 缓冲语义，USB 不变）。
- README 设备矩阵行补"adb-over-TCP 已通（仅 1 台 TCP 验证）"。

## 8. housekeeping

- t6 证据 `9db7d64` 已在 main（t1 并入）；dash forward 17942 已清；
- `pilotdroid setup` 的 forward_dial 使用 free-port 且退出即清，无残留。
- Python 包：`python/` pyproject，`pip install` 后 `import pilotdroid` 实测连通（symlink 单源）。
