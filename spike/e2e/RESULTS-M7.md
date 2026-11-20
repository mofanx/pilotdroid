# RESULTS-M7 · 诚实口径收口

设备：Mi11 `ba0d072d`（USB, A14/MIUI）；dash `11.0.0.2:5555`（adb-TCP, A17/SDK37）。AGENT_VER **0.4.0**。

## 1. 同脚本端到端双跑（引用级口径，`bench_m7_dual.py`，n=5 p50）

同一 u2 风格脚本（21 ops：info/app_current/click/selector exists+click/scroll/app 生命周期/set_text/long_click/swipe/press/dump/screenshot）总墙钟：

| 传输 | pilotdroid shim | 真 uiautomator2 | 比值 |
|---|---|---|---|
| USB（Mi11/A14） | **4769ms** | 41909ms | **8.8×** |
| adb-TCP（dash/A17，无线口径） | **5915ms** | 43874ms | **7.4×**（实测区间 4.7–7.4×） |

口径备注：同脚本同语义同 settle；两侧错误 op 均计入墙钟（u2 侧 UiObjectNotFoundError 自带 ~4s 等待——如实反映真实使用成本）。shim 在 TCP 下绝对值仅 +24%（持久 socket 摊薄传输），u2 绝对值基本不变（per-op HTTP 成本主导）。JSON：`e2e-m7-{shim,u2}-*.json`。

**比值 op 混入敏感（t3 独立复核实测）**：USB shim 4483/u2 41061（比值 9.2×）、TCP u2 42630 均在 ±3% 内复现；TCP shim 9003（+52%）——复跑当次 dash 首屏 scrollable 节点在位，两个 scroll op 真实执行（记录次为快败 LookupError）。同 op-混合口径下 TCP 比值 **4.7–7.4×**，引用建议取区间而非单点。

## 2. 版本语义补漏

`AGENT_VER 0.3.1→0.4.0` + verify `agent_ver` 断言——pkg-capable jar 与 pre-M6 jar 经 HELLO.ver 可区分，堵 M6 交付时 wire 变更未 bump 的静默忽略风险（旧 jar 忽略 `pkg` → query_pkg 假阳性，query_pkg_neg 能抓但 ver 断言本身挡不住的洞已闭合）。

## 3. verify 设备无关化——**双机 33/33 全绿**

- 锚点迁 quicksearchbox `query_text` EditText（`{"id":"query_text"}`）——MIUI/stock 同布局，`am start --activity-clear-task` 钉死初始 task（防 SubSettings 残留偏斜）。
- scrollable 断言挂 `Settings/.MainSettings --activity-clear-task`（两机首屏均为 scrollable RecyclerView）。
- logcat marker 断言分层：**resp.ok 为硬门**，`MIUIInput`/`injectInputEvent` 行数作 note（marker 发射为 OEM 依赖行为，stock AOSP IM 注入不写该类行）。
- `stream_eof` 已含"排空≤8 在途帧"（M6 修正）。

`verify-agent.sh ba0d072d 17932` → **33/33**；`11.0.0.2:5555 17942` → **33/33**（此前 29-32/33 附注消除）。

## 4. shim 可分发面

- `connect()` port auto-detect：解析 `adb forward --list` 的 `localabstract:pilotdroid_agent` 规则（serial 可选过滤）；无规则→探测 17932；全灭→报错含 `pilotdroid setup` 指引。
- `sel.info` 改 `@property`（u2 真语义，`.info()` 调用式为此前缺陷——真脚本会拿到 bound method）。
- 方法级矩阵 `compat/SUPPORTED.md`：selector keys/对象方法/Device 顶层逐条 supported/unsupported/语义差；**xpath 显式声明非目标**；contains/regex/relative/attrs 键显式拒绝。
- repo 卫生：`python/build/` + `*.egg-info/` 移出 git + gitignore（`__pycache__/` 同补）。

## 5. 已知边界（诚实记录）

- u2 与 pilotdroid agent 抢同一 UiAutomation 全局单例——双跑需交替部署（bench 流程注释已写明）。
- dash bench 场景里 scrollable 节点间歇缺（stock A17 首屏布局差异）——对拍两侧同场景同分，不影响比值口径。
- xpath / watcher / contains-regex selector 不在 v1 支持集（见 `compat/SUPPORTED.md`）。
