# RESULTS-M13 — selector 语义对齐 + nit 清零（t1 部分）

日期：2026-11-20。设备：ba0d072d（USB）。agent 0.7.0（本轮无 jar 变更）。

## 1. chained relative selector 真 bug 修复

M12 host-filter 的锚点解析取 `self._base._kw_match`（base 叶 kwargs）——
`d(A).child(B).left(C)` 锚到"树里第一个 B"而非"B in A"，二级链式静默误解析。

修法：`_Selector._resolve(pairs, parent)` 递归——rel 锚点 =
`self._base._resolve()` 首命中（含 base 自身 instance 位置），链式任意深度正确。

回归测试入 `compat_m6_realscripts.py` **script_D_selector_semantics**（5 断言，
XML ground truth 对照）：

- `chain child→left`：shim left count == XML 计算的"r<=锚点左缘"节点数 ✓
- `child().child()` 二级链式 ✓；`child()` = 后代（count-1 恒等式）✓
- 单级 relative 不回归（sibling/left/right/up/down 路径未变）

## 2. index/instance 语义对齐 u2（M11-12 混用的纠正）

| u2 key | 旧实现（错） | M13 对齐 |
|---|---|---|
| `index` | =Nth-match | **child-position**（XML `index` attr，host post-filter 谓词） |
| `instance` | =Nth-match | 保持 Nth-match（映射 wire `index` 字段——wire 语义即此） |

实证：`d(index=1).count` == XML `index="1"` attr 节点数（21=21、77=77
逐值吻合）；`d(index=N)` 走 host 路径（`_WIRE_KEYS` 移除 index），
`d(instance=N)` 仍走 wire 快路径。SUPPORTED.md/docs/02 口径已同步。

## 3. serve 尾项

- `watch_events` + `timeout_ms` schema arg（默认 5000 = connect 期
  read timeout，min 500 clamp）——`MediaClient.set_read_timeout` +
  `Session.watch_recv(timeout_ms)`；真调 `timeout_ms:100`→0.5s
  `{type:"timeout"}` isError:false。
- conn-replaced warn 文案补 `(safe to ignore on first connection)`。

## 4. nit 账（t4 复核遗留，t1 负责部分）

- `watcher.running` 入 SUPPORTED watcher 行 → IGNORE.running 可退役（t2 侧改）
- dump TTL cache：**记 defer**——`_Scroll._to` 靠 dump 变化判定内容稳定，
  cache 会致立即假命中；mutating-op 失效跟踪面广，复杂度超 ~200ms 收益

## 5. 验证

- `compat_m6_realscripts.py`：**28 ok / 1 unsupported(xpath) / 0 error**
  （script_D 5/5 真值断言）
- `verify-agent.sh ba0d072d`：**37/37 PASS**
- `cargo test --workspace` 全绿；clippy/fmt 净
- 插曲：本 worktree target 致磁盘满 → CARGO_TARGET_DIR 指向主 worktree
  共享（AGENTS.md 既定约定）

## 6. t3 证据批（main 态）

### 6a. drop-in 补齐（第三方用法驱动，~110 行 host-only）

对 Themis 71 个真实 u2 脚本做 API 普查后补齐高频缺口（全部实测生效）：

- `u2.connect("serial")` —— u2 位置参数是 serial 字符串，shim 原签名把
  第一位当 port → **全部 70 个脚本此前连不上**。现在非数字 str → serial
- `d.service("uiautomator")` —— 70/71 脚本 teardown 依赖；
  `.running()/.stop()/.start()` 映射 agent 进程（pgrep/pkill/setsid
  app_process），其他服务名 ValueError
- `swipe_ext(dir,scale)`（23 处）`set_orientation(l|n|r|u)`（4 处）
  `double_click(x,y,d)` `drag(fx..)` `.center()` `.gesture(4点双指针)`
- 普查中仍 ❌：`d.xpath()`（13 脚本，非目标）、`d.service` 其他服务名

### 6b. serve_multi 断言更新（self-heal 语义）→ **9/9 PASS**

旧口径"kicked 永久 dead"已过时（M10 self-heal 变 ping-pong）。新断言：
A kicked→A 自愈夺回→B 自愈夺回（双向）→owner 正常→owner 死后 A 两 call
内夺回→fresh serve 接管。实测全部命中预期时序。

### 6c. 第三方 u2 套件对拍（Themis benchmark, ESEC/FSE'21）

`compat_u2_external.py`：14 脚本原样跑（唯一 seam =
`sys.modules['uiautomator2']=u2shim`），sha256 留档，脚本入库
`spike/e2e/themis/`（APK 不入库，manifest 记 repo 路径）。

设备侧新坑（已解，记入方法学）：
- MIUI `INSTALL_FAILED_USER_RESTRICTED` → `pm install
  --bypass-low-target-sdk-block` + 弹窗点击
- API34 拒 targetSdk<23 → 同上 flag
- **babysitter 不能用 agent conn**——单 client 语义下第二 conn 与脚本
  互踢（实测 ConnectionError ping-pong）。改为纯 adb：`dumpsys window`
  焦点包检测 + `input tap` 固定坐标（仅限 installer/lbe 系统包，不碰
  app 内弹窗——污染 bug-repro 语义）
- `uiautomator dump` 在本机被 Killed（agent 独占 UiAutomation）——adb
  侧拿不到层级，证实只能走 dumpsys 检测路线

verdict 口径：ok=跑到底 / unsupported=撞上 shim 明示不支持 API /
miss=LookupError（选择器未命中，环境态非 API 失败，同 u2
UiObjectNotFoundError）/ timeout=超过 200s / error=其他异常。

**实测结果（e2e-m13-themis.json）：14 脚本 — 0 ok / 0 unsupported /
0 error / 11 miss / 3 timeout**。读法：

- **API 面 14/14 零失败**——connect→app_start→app_current→selector→
  click/set_text/swipe_ext/service 全部执行，无一撞 API 缺口。
  这正是 drop-in 的主张（通过率口径是"复现 bug"，不是 API 完备）
- miss 全为环境态：and-bible `d(text="OK")` 手动实证 = 应用正常前台
  但按钮显示"确定"（zh locale，真 u2 同设备同 miss）；"Navigate
  up"/"More options" 等 contentDescription 在 zh 系统本地化
- timeout×3 = app 侧崩溃/权限流：APhotoManager logcat 抓到 MIUI
  ThirdAppAssistant `problemName=JE_IAE`（app 启动即崩——恰是该
  benchmark 要复现的 bug 类），脚本固有 stale-snapshot while-loop
  （`current_app` 快照永不刷新）在 app 未上前台时必然死循环——
  真 u2 同行为
- 途中发现并修复真缺口：`_Selector.swipe(dir)` 元素内定向滑动
  （ActivityDiary#285 真用法）+ `connect` serial 位置参数 + `service`
- 安装侧修正：uninstall-first（API34 `-d` 不生效 →
  INSTALL_FAILED_VERSION_DOWNGRADE）+ install stderr 留档

### 6d. t1 后 compat 复跑留数

28 ok / 1 unsupported(xpath) / 0 error——与 t1 合并前一致，新 drop-in
块零回归。

### 6e. soak watcher 臂

soak-m4.sh bench 块加 watch_arm/disarm×50（action=none，不可命中
selector，poll_ms=50），shell 侧 `/proc/<pid>/task/comm` 计 pd-watch
残留进 `wres` 字段；soak-check.py 新 gate `watch_residue`（所有 bench
样本 wres==0，旧日志无字段跳过），fd_abs 同门 ≤90 不变。
精确性修正：wres 用 `pgrep -f pilotdroid.Agent` 取 pid（设备存在其他
app_process，原 `pid_of` 的宽匹配可能错认）。

**3min 实跑（soak-m13.jsonl；M4 原始 jsonl 保留未被覆盖）**：26 cycles
= 1300 次 arm/disarm + 1300 ops——
fd_constant PASS(75) / fd_abs PASS / **watch_residue PASS（26 样本全 0）**
/ bench_fails 0。rss_slope 报 FAIL(+2079kB/min)——3min 短窗内 RSS 是
98→115MB GC 锯齿非单调增长，该门设计给 480min 长跑，短跑内为测量伪
影（判读：wres/fd 两个泄漏直接指标全过，rss 非单调）。

## 6. dist/doc nit 批 + 发布准备（t2）

| 项 | 落地 | 实证 |
|---|---|---|
| doc-drift 中文正则 | `(\d+)\s*(?:tools?|工具)` 覆盖 "27 工具" 行；IGNORE.running 退役（SUPPORTED watcher 行已含 `.running`），close 保留豁免 | **负路径实证**：README 改 "99 工具"→`FAIL tools-claims claims=[27,99]`，恢复→PASS |
| windows asset .exe | matrix `ext` 字段：`pilotdroid-cli-windows-x86_64.exe` | release.yml diff |
| README windows 行 | 下载表 +windows x86_64 行 + SmartScreen 注（更多信息→仍要运行 / `Unblock-File`），与 macOS xattr 注同构 | README diff |
| release notes 草稿 | `docs/release-notes-v0.6.0.md`：agent 0.7.0/caps 0x7ff、异步 WATCH 头条（armed 期 op p50=43ms）、APP_INFO ~2×、SUPPORTED 仅剩 xpath、4 平台 assets、SmartScreen/quarantine 注 | 待 publish 时粘贴 |
| notify demux 定论 | docs/02 §4.9 尾段：**push=demux not toggle**——media socket 单读者，常驻推会抽空 pull 队列；真方案 = 按帧类型 demux（watch_hit→notification / frame→pull，~50 行）；notification 仅 log 面非结构化事件 → 保留 pull 契约 | docs/02 diff |

## 7. 发布链执行（t4）——draft+notes 就绪，未 publish

- `gh release delete v0.6.0 --yes`：删空壳 draft（0 asset，挡 tag 后 create）
- `git tag v0.6.0 baf57b9` + push → release.yml run **44769030582 全绿**：
  verify-tag ✓（v0.6.0 == workspace 0.6.0 == pyproject 0.6.0）+ 3 cli
  matrix + python-dist + release job
- 新 draft（tag=v0.6.0, target=main）assets **7 个**：
  `agent.jar`、`pilotdroid-cli-linux-x86_64`、`pilotdroid-cli-macos-arm64`、
  `pilotdroid-cli-windows-x86_64.exe`、`pilotdroid-0.6.0-py3-none-any.whl`、
  `pilotdroid-0.6.0.tar.gz`、`SHA256SUMS.txt`
- notes 已刷（`docs/release-notes-v0.6.0.md` 第 6 行起为正文体，
  头部 blockquote 是内部操作注、不入发布页）
- **待用户**：PyPI pending-publisher 建好 → draft 点 Publish →
  （assets 上线后）homebrew tap sha256 回填
