# RESULTS-M8 · 分布可用 + 防护加固

设备：Mi11 `ba0d072d`（USB, A14/MIUI）；dash `11.0.0.2:5555`（adb-TCP, A17/SDK37）。AGENT_VER **0.5.0**（workspace/pyproject 版本同步对齐）。

## 1. jar 内嵌分发——免 `--jar` 真一键

`agent/agent.jar`（19.2KB→19.5KB）入库 + `include_bytes!` →
`AGENT_JAR_BYTES`/`embedded_jar_path()`（版本化 temp 名 + 原子 rename）。
生效面：`setup` 无 `--jar` 自动推内嵌、`agent install` jar 参数变可选、
watchdog `connect_recover` 远端缺 jar 时自动用内嵌重试。同步法写进
AGENTS.md：改 Agent.java → `deploy-agent.sh` 重建 → `git add agent/agent.jar`；
verify `agent_ver` 断言兜底（jar↔java 漂移会假绿）。M10 起 CI 加
`agent-jar-sync` 提交序守卫。

## 2. doctor 5→8 步

新增 `u2_conflict`（pm list+ps 双通道：运行中 uiautomator/atx→WARN，
装而未跑→PASS 注明）与 `inject_probe`（5,5 死区 tap+PING 复核，
duchamp 类 revoke fail-fast）；`forward_hygiene` 挪至末步（此时自身
conn 已断、不被误列）。真机实测 **8/8 绿 exit 0**，u2_conflict 准确
检出 `io.appium.uiautomator2.server`+`com.github.uiautomator` idle；
顺手清出 15 条孤儿 forward。

## 3. 观察面加固

`list_elements(interactive_only=默认 true)`：clickable/editable/
**scrollable**（ElemInfo 新字段）过滤+重编号——dump ~8× 压缩。
快照守卫 = focused pkg + **layout signature**（fnv over 全树
pkg/class/bounds——文本变化不误伤）；tap_index/set_text_index 前双检，
漂移报 "snapshot stale — re-run list_elements"。真机：interactive 131
元素命中、tap_index 通过守卫。fake 测试 ×3（过滤/stale 拒绝/无快照报错）。

## 4. 多 serve 跨进程语义——单 client 顶替

**问题定性**（M6 实测遗留）：两条 control conn 并存 → 跨进程 op 交错，
session-bound 状态（elements 快照、stream owner）为未定义行为。

**实现**（Agent.java 0.5.0）：accept 新 control conn → 旧 conn
`shutdownInput/Output + close`（`close()` 不打断阻塞读，必须先发 FIN）
→ 其 session/stream 按 §4.8 终结。最后连接者胜，跨进程交错不可能。

**实测**：`serve_multi.py` 重写 8 断言全绿（被顶替 serve 死透、fresh
serve 接管、零交错）；verify `client_replace`/`stream_eof` 段按新语义。

**已知窗口**（M10 补注）：保证是"accept 后无交错"——connect 返回至
agent accept 间有 TOCTOU 窗口，旧 conn 仍可完成 ~1 op（adb-TCP+stream
负载下 ping 滑窗 ~20%，t3 定量）。verify `client_replace` 改用
poll-until-dead 消化此窗口。M10 起 serve io-error 自愈（死 conn 惰性
重连夺权，失败 call 仍显式 tool error）。

## 5. shim 免费扩面——SUPPORTED ❌ 砍半

零协议变更新增：`shell`/`app_list`/`app_info`/`window_size`/
`screen_on/off`/`unlock`/`orientation` get+set/`clear_text`/`wait`→bool
修正/`watcher`（host-poll 线程，命中即摘）。真机冒烟 **9/9 全过**。
`compat.py` 矩阵 **23 ok / 1 unsupported(xpath) / 0 error**。

## 6. 预览档 fps spike（`m8-preview-fps.json`）

SHOT p50 表 + 流有效帧率：scale0.25/q50 → **8.9fps effective**；
scale0.15/q50 → **10.0fps effective**（抽核实测 9.9fps@0.25——声明
偏保守）。可作 live-view 预设档依据。

## 7. 回归网 + 卫生

- `docs-smoke.py`（README 可照抄面回归：mcpServers 块可解析+二进制
  存在+serve --help 有 `-s`+内联子命令全真）7/7 PASS，M9 入 CI；
- soak-check 加 `fd_abs≤90`（基线数据 PASS——SHOT warmup +11 fd 盲区
  清零）；CI 离线 lane（fmt/clippy/test，M8 发现已存在未覆盖）；
- verify-*.json `git rm --cached`+gitignore（每轮伪 diff 消除）；
- Cargo workspace/crate 发布元数据 + `pyproject [project.scripts]`。

## 8. verify 双机 + cargo

`verify-agent.sh`：**ba0d072d 33/33、11.0.0.2:5555 33/33**（首轮 30/33
的三 FAIL 全归因：2×post-reboot keyguard 残留、1×§4 TOCTOU 窗口内
ping 滑窗——均非缺陷）。cargo test 75 绿 + fmt/clippy `-D warnings` 净。

## 9. 已知边界（诚实记录）

- MIUI `screen_off` 可能触发 `ScreenOnProximitySensorGuide` 口袋模式
  遮罩——全输入（含 `am start`）僵死，`settings` 关闭无效，**仅 reboot
  可解**。MIUI 上调用 shim `screen_off()` 前须知此风险。
- `agent start` 在 adb-TCP（dash）曾现一次"报 launched 但进程缺席"，
  复测正常——瞬态标记。
- selector contains/regex/relative/attrs 键、xpath、pinch/多指针手势
  仍不在支持集（SUPPORTED.md 逐条口径）。
