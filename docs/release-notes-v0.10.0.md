# pilotdroid v0.10.0 — usage stats：diagnostics 新面

新增两个 MCP 工具（32→34），开启 automation 之外的第二个观测面——**回溯式应用使用统计**。

## 新工具

- **`usage_access_status`** — 探测本机 UsageStatsManager 可达性（`available`/`source`/`detail`）；
- **`usage_stats`** — 按天聚合的应用使用统计：`packages` 过滤+`days`(1–30)或显式 `begin_ms`/`end_ms` 时间窗，返回逐日 `foreground_ms`/`visible_ms`/`launch_count`/`last_time_used`。

典型问法：「蛋仔派对最近一周每天玩了多久」「这个 app 最近被打开过吗」。

## 实现与信任模型（诚实档）

- **shell 身份直读** UsageStatsManager——`com.android.shell` 出厂持有 `PACKAGE_USAGE_STATS`，**不需要**用户授权、appops 写入或 helper APK；隐私管控在 host 侧 MCP 权限层（`mcp__pilotdroid__usage_*` 粒度），设备侧不设假门；
- **诚实边界四条**（详见 `compat/SUPPORTED.md` M33）：①per-user(user 0)可见——work profile/MIUI Second Space/双开**不可见**，缺失不得读作未使用；②日 bucket 按设备 lazy-rollover 时钟切（实测 MIUI 22:13/HarmonyOS 23:59），**非午夜**；③保留深度 OEM 自定（实测日文件 MIUI 5 vs HarmonyOS 10）——`partial:true`+`earliest_data_ms` 是一等字段，窗口缺口**不得**读作未使用；④`visible_ms`/`launch_count` 不可得时为 -1；
- **禁条**：永不使用 `dumpsys usagestats --checkin`（有状态副作用）；dumpsys 解析刻意不做（滚动窗给不出逐日真值，伪降级不如显式 unavailable）；
- **定位**：diagnostics/观测面，非 automation 面。前瞻式「是否进前台」用 `app_current`+watcher；本面只答回溯式「用了多久/开过没有」。

## 版本错落诚实档

本次发件 host cli 0.10.0 随行 **agent.jar 0.7.5**（协议面+2 opcodes `USAGE_STATS`/`USAGE_ACCESS`，caps +`CAP_USAGE=2048`）。`pilotdroid-cli setup` 或任意 session 首连会自动重刷设备侧 jar——无需手工干预；`initialize` 握手的 `agentVer` 字段可核对。

## 验证

- spike 定路（binder 直调双机真出数，cmd 死路）→ agent op → serve 工具面，三层各自独立复核 pass；
- 数据正确性**四方互证**：spike 探针/agent 实装/用户手工 APK/MCP 端到端逐字节一致；
- `partial` 字段真实开火：同 7 日窗小米 `partial:true`（5 日保留）vs 华为 `false`（8 日覆盖）；
- verify-agent 49/0（含 M33 五钉）、cargo 115/0、fmt/clippy/doc-drift 净、install-smoke 9/0 随行。
