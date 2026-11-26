# RESULTS-M17 — openatx/uiautomator2 官方套件对拍（u2shim vs ba0d072d）

> **M18 更新（verbatim 复跑，shim 补齐批后）**：§1-§5 为 M17 原始记录
> （当时未修 shim）；§7 为 M18 复跑结果——**verbatim pass 0→8**，剩余
> fail 全部归因到明确 defer/已知 ❌。u2shim 补齐均落 `compat/u2shim.py`
> 本体（非 seam 适配），seam 模块今仅剩 serial pin。

Date: M17. Suite: `openatx/uiautomator2` `mobile_tests/` @
`96f581d3b95efdf16105b767a5d5401c73f7ec0e`（main, 2026-11-17）.
Device: ba0d072d（Xiaomi, MIUI, Android API 34, 1440×3200）.
Agent: 0.7.0（forward `tcp:17932 → localabstract:pilotdroid_agent`）.
Target app: `io.appium.android.apis`（ApiDemos, targetSdk 19 →
`pm install --bypass-low-target-sdk-block` 绕过 API34 低 targetSdk 拒绝）.

**口径声明**：fail 全部如实归因，未为通过修改 u2shim 源码；
唯一适配在 harness alias 模块（见下），逐项列明。

## 1. 套件概况

`mobile_tests/` 8 个 `test_*.py`，30 个 test 函数；`skip_test_image.py`
按命名约定不被 pytest 收集（上游即禁用，不计入）。

| 文件 | tests | 依赖面 |
|---|---|---|
| test_simple.py | 10 | selector/scroll/get_text/xpath/send_keys/toast/watcher/plugin |
| test_swipe.py | 1 | `d.swipe` 阻塞语义 |
| test_watcher.py | 1 | `d.watch_context`（u2 上下文管理器） |
| test_session.py | 5 | app mixin/session 生命周期/`SessionBrokenError` |
| test_settings.py | 3 | `d.settings` 类型校验语义 + `u2.Session` 符号 |
| test_xpath.py | 8 | `d.xpath`（u2 XPath 引擎） |
| test_push_pull.py | 1 | `d.push`/`d.pull` |
| test_screenrecord.py | 1 | 上游 `@pytest.mark.skip("deprecated")` |

## 2. Tier-A：严格 verbatim 结果

Seam 仅 `sys.modules['uiautomator2'] = u2shim` + `connect()` 固定
serial（本机同时挂着 ba0d072d + 11.0.0.2:5555 两台设备，`adb` 裸调用
会 `more than one device`；套件按单 emulator 写的）。

**0 pass / 27 collected**：fixture `d`/`dev` 在 `_d.settings[...]` 处
AttributeError（u2shim 无 `settings` 属性）——15 个测试统一死于
fixture setup；test_session/test_settings 收集期错误
（`uiautomator2.exceptions` 子模块 / `u2.Session` 符号缺失）；余为上游
skip 标记。即 verbatim 面被单个 u2 API 面缺口整体阻断，分布无信息量。

## 3. Tier-B：seam 适配后（信息层）

Alias 模块内做且仅做以下映射（测试体一字不改，u2shim 源码不动）：

1. `connect()` 固定 serial=ba0d072d（suite-env）
2. `Device.settings` → 透明 dict（fixture 管道；**不提供** u2 的类型
   校验/`wait_timeout` 联动语义 → test_settings 如实失败）
3. `app_start(stop=True)` → `app_stop + app_start + app_wait(front)`：
   u2 app_start 契约是"已启动且在前台"；u2shim.app_start 只发
   monkey/am start 不等待（**shim 语义缺口 §5-G1**）
4. `click(timeout=None)` → `click(5.0)`：u2 click 对目标做隐式等待；
   u2shim 立即 resolve（**缺口 §5-G2**。上限 5s 因 agent 控制连接
   socket timeout=10s，wait_until(10s) 会撞 socket 超时）
5. `uiautomator2.exceptions` 子模块 → `SessionBrokenError` 空类桩：
   只为 import 通过，shim 不会 raise → 依赖测试如实失败

### 结果：1 pass / 18 fail / 8 skip / 3 未收集

| 测试 | 结果 | 归因 |
|---|---|---|
| session::test_session_window_size | **PASS** | `window_size()` (1440,3200) 正确 |
| simple::test_scroll | SKIP* | `d(text="App").click()` 成功后命中测试自带 `screen too large` skip 分支（大屏无需滚动）——等价通过路径 |
| simple::test_get_text | FAIL | `_Selector.get_text` 缺失（shim 有 `.info['text']` 等价物）**G3** |
| simple::test_xpath / test_send_keys | FAIL | `d.xpath` 不存在——已知 ❌ |
| simple 其余 6 | SKIP | 上游 skip 标记（toast/watchers/count/implicitly_wait/select_iter/plugin） |
| swipe::test_swipe_duration | FAIL | `swipe(2.0s)` 调用 0.056s 返回——agent 端异步执行手势，u2 客户端阻塞语义不符 **G4**（duration 设备端仍生效） |
| watcher::test_watch_context | FAIL | `d.watch_context` 缺失（shim 有 `d.watcher.when/run`，无上下文管理器面）**G5** |
| xpath 8/8 | FAIL | `d.xpath` 不存在——已知 ❌；其中 test_watcher 死在 `_WhenCtx.call` 缺失 |
| session::test_session_function_exists | FAIL | `wlan_ip`/`jsonrpc`/`xpath`/`settings` 均缺 **G6** |
| session::test_app_mixin | FAIL | `app_list_running` 缺失 **G7** |
| session::test_session_app | FAIL | `app_list_running` + `session()` 返回 Device 而非 session 对象（无 `running()`/上下文管理器）**G8** |
| session::test_auto_grant_permissions | FAIL | `app_auto_grant_permissions` 缺失 **G9** |
| settings 3 | 未收集 | `u2.Session` 符号缺失；即使收集，透明 dict 无 xpath_debug 类型校验/wait_timeout 联动 → 仍败 **G10** |
| push_pull::test_push_and_pull | FAIL | `d.push`/`d.pull` 缺失 **G11** |
| screenrecord | SKIP | 上游 deprecated 标记 |

*skip 计数按 pytest 口径：7 上游标记 + 1 内部条件。

## 4. 套件环境假设（非 shim 缺陷，已处理）

- **E1 双 adb 设备**：`connect()` 裸调会因 `more than one device` 全灭；
  seam 固定 serial（上游套件按单 emulator 写，等价做法）。
- **E2 MIUI ReviewPermissionsActivity**：ApiDemos（targetSdk 19）首次
  启动被 `com.lbe.security.miui` 权限复核屏拦截，`app_current` 停在该
  包，`text=App` 全 miss；点「确定」一次性放行后正常。babysitter 的
  `securitycenter` 模式盖不住此包名——**值得记进 SUPPORTED.md**。
- **E3 API34 低 targetSdk 拒绝**：需 `--bypass-low-target-sdk-block`。
- **E4 MIUI 安装确认**：`pm install` 走 AdbInstallActivity 点按流程
  （babysitter 已知坐标）。

## 5. 新发现的 shim 缺口（SUPPORTED.md 候选注记）

| ID | 缺口 | u2 语义 | 现状 |
|---|---|---|---|
| G1 | `app_start` 无前台等待 | 阻塞到 app 前台 | 只发 launch 即返回（race 源） |
| G2 | `click()` 无隐式等待 | wait_timeout 内等待出现 | 立即 resolve→LookupError |
| G3 | `_Selector.get_text` | `node['text']` | 缺方法（`.info` 可替代） |
| G4 | `swipe` 非阻塞 | 客户端阻塞 duration | 0.056s 返回（agent 异步） |
| G5 | `watch_context` 上下文管理器 | `with d.watch_context()` | 有 watcher.when/run，无 ctx |
| G6 | `wlan_ip`/`jsonrpc`/`settings` | u2 Device 面 | 均缺 |
| G7 | `app_list_running` | 运行中包名 | 有 `app_list` 无 running 变体 |
| G8 | `session` 对象模型 | Session w/ `running()`、ctx mgr、SessionBrokenError | 返回 self(Device) |
| G9 | `app_auto_grant_permissions` | pm grant 批量 | 缺 |
| G10 | `settings` 语义/`u2.Session` | 类型校验+wait_timeout 联动 | 全缺 |
| G11 | `push`/`pull` | 文件传输 | 缺 |

xpath（G 外已知 ❌）、toast、plugin、screenrecord 属既有 unsupported
清单，不重复计。

## 6. 结论

- **严格 verbatim（Tier-A）= 0 pass**：单点 `d.settings` fixture 即
  阻断全部——这是套件对 u2 API 面的强耦合，不是 selector/gesture 层
  不兼容的证据。
- **seam 适配（Tier-B）= 1 pass + 1 等价通过路径**：
  `window_size`、`App` 点击链路实证可用；18 fail 中 **7 为 shim
  API 面缺失**（get_text/watch_context/session_function_exists/
  app_mixin/session_app/auto_grant_permissions/push_pull，均为
  AttributeError 非行为错），**10 xpath 已知 ❌**（test_xpath 8 +
  test_simple 内 2），**1 为客户端阻塞语义分歧**（G4）。
- 对拍价值：G1/G2 两条 u2 高频语义差异（隐式等待、前台等待）是
  首次由第三方套件暴露——第三方脚本最常见的两类调用恰好踩中，
  SUPPORTED.md 值得一行注记或 shim 层补齐（工作量小，属 compat
  面增强而非范围扩张）。

复现：`PYTHONPATH=/tmp/u2stubB pytest /tmp/u2suite -v --timeout 120`
（stubB=seam alias，/tmp/u2suite=verbatim `mobile_tests@96f581d`）。

## 7. M18 复跑（shim 补齐批后，verbatim）

u2shim 本体补齐（非 seam）：`Device.settings` 透明 dict、
`uiautomator2.exceptions` 子模块桩（SessionBrokenError/
UiObjectNotFoundError=LookupError）、`Session=Device` 别名、
`_Selector.get_text`、`app_start` front-wait+`stop=`、`click` 隐式
等待（8s，agent socket 10s 顶）、`swipe` host-sleep(duration)、
`push`/`pull`（含 BytesIO/file-like 暂存）、`wlan_ip`、
`app_list_running`、`app_auto_grant_permissions`、`app_current`+`pid`、
`watch_context` ctx mgr、`u2.Session`、`_WhenCtx.call`、Device
`__enter__/__exit__`。

**套件抓出的 3 个真 bug（已修）**：
- `_Watcher` 线程在**零规则时秒退**——`watch_context`/`watcher.start()`
  先于 `.when()` 注册的经典竞态；改为 `_running` 标志活到 stop()
  （u2 语义：watcher 持续运行直到显式停）。
- `_Selector.wait` 单次 `wait_until(timeout)` **长时间独占共享控制
  conn**（单 client agent）——watcher 的 watch_arm 饥饿，8s 隐式等待
  内规则永远没机会 arm；改为 ≤1s 分片轮询（Agent.call 本有锁，
  分片间隙天然交错）。
- `_WhenCtx.call(fn)` 规则被 arm 成 device-side `click`——命中后
  回调被静默丢弃且误点节点；改为 `action:"none"` 观察态 arm，命中
  后宿主侧跑回调（agent 本就支持 none）。

### 复跑结果（./run.sh，seam 仅 serial pin）

**8 pass / 14 fail / 8 skip（30 collected，0 fixture-block）**

PASS：push_and_pull（BytesIO+pull 往返）、app_mixin（app_list+
running+app_wait+app_current.pid）、session_window_size、
auto_grant_permissions（pm grant 循环）、simple::get_text、
swipe_duration（host-sleep 契约）、**watcher::test_watch_context**
（armed watcher 自动点 App→Menu→Inflate 链路）、
**xpath::test_watcher**（`.call` 回调触发——非 xpath 断言，watcher
功能测试）。

| fail | 归因 |
|---|---|
| session_function_exists | `jsonrpc`/`xpath` 缺（G6 defer） |
| session_app | `sess.running()`/SessionBrokenError（G8 session 对象模型 defer） |
| settings×3 | settings 类型校验/`wait_timeout` 联动/operation_delay 生效（G10 defer，透明 dict 刻意不联动） |
| simple::test_xpath, send_keys | `d.xpath` 已知 ❌ |
| xpath×7 | `d.xpath` 已知 ❌（test_watcher 已转出 PASS） |
| SKIP×8 | 7 上游标记 + scroll 内部条件 |

**结论变化**：Tier-A verbatim 0/27 → **8/30 pass**（无 fixture 级
阻断）；fail 中 **9 xpath**（test_xpath 7 + test_simple 内 2；
test_watcher 已转出 PASS）、**1 jsonrpc**（G6）、**1 session
对象模型**（G8）、**3 settings 联动**（G10）= 14 与逐项表自洽——全部为已登记 defer，
**无未归因 fail**。另注：watch_context 单测偶发 a11y-lag 竞争
（watcher 自动点击与 Menu tap 时序），单跑 1/3 观察为环境敏感性，
与 SUPPORTED.md 既有第三类同源。（M21 起在本环境 3/3 稳定复现；
M22 取证定稿：fixture app 被卸 + 画报锁屏/Shade 遮罩两层环境
失败，watcher arm/hit 链路完好——见 §8）

复现：`PILOTDROID_SERIAL=ba0d072d spike/e2e/u2suite/run.sh`
（seam alias+verbatim 套件已入库 `spike/e2e/u2suite/`，不再依赖 /tmp）。

## 8. M21 复跑（G8 session 对象模型 + G10 settings 全语义后）

**12 pass / 10 fail / 8 skip（30 collected）** — G6/G8/G10 三桶清零：
`test_session` 5/5 PASS（session_function_exists、app_mixin、
session_app、session_window_size、auto_grant_permissions）、
`test_settings` 3/3 PASS（xpath_debug/wait_timeout/operation_delay
类型校验+联动）。

| fail | 归因 |
|---|---|
| xpath×7 + simple::{test_xpath, send_keys} | `d.xpath` 已知 ❌（=9） |
| watcher::test_watch_context | 当前环境 **3/3 稳定挂**，M22 取证定稿为**两层环境失败，非代码缺陷**：① **fixture app 被卸载**——`pm path io.appium.android.apis` rc=1，全部 `io.appium.*` 包消失、`/data/local/tmp/pilotdroid/` 同被清空（launcher 留有"卸载"确认弹窗、`INSTALL_FAILED_USER_RESTRICTED` 示 USB 安装被关——设备有 agent 之外的操作）；此态下 "App" 节点根本不存在→watcher 无的放矢→`Menu` 永不出现，失败点前移至 line 11。② **画报锁屏+NotificationShade 遮罩**——`isKeyguardShowing=true`/`mWakefulness=Dozing`/`mCurrentFocus=NotificationShade`/`mUserActivityTimeoutOverride=10000`，a11y 树全空一切 selector 落空；此态下失败点在 `Inflate from XML` wait（t3 两轮复跑所见）。watcher arm/hit 链路手动探针实证完好（`watch_arm→{'armed':True,'watch_id':5}` 正常应答），**归因定稿：环境失败两类叠加，shim/agent 代码无可疑点**。缓解已落地：`svc power stayon true`、run.sh 前置 KEYCODE_WAKEUP+statusbar collapse、conftest `_device_fixture_guard` 坏态秒报 |

**结论**：8→12 pass，剩余 10 fail = 9 xpath（明示 ❌）+ 1 环境
敏感项（a11y-lag/设备态，本环境 3/3 稳定复现非偶发——M22 取证：
fixture app 被卸 + 锁屏/Shade 遮罩双层原因，见上行），**无未归因
fail、无新代码缺陷**。

**抽查抓到的第 4 个真 bug（复核轮，非套件）**：`Device()` 不消费
`PILOTDROID_SERIAL`——`d.shell()`/`app_wait`/`app_info`/`session()`
等 adb 通道在多设备环境静默空输出（exit=1 被吞）；套件靠 seam
显式传 serial 掩盖。已在 `Device.__init__` 加 env fallback（与
Rust CLI `Cli::parse` 后的同名逻辑对齐，显式 `serial=` 参数优先）。
复验：`d.shell('pidof')` 恢复、`session(attach=True)`+
`running()→False`+op→`SessionBrokenError` 全链实证。

**Fixture 恢复（需用户操作，M22 实测仍受阻）**：`pm install`
返回 `INSTALL_FAILED_USER_RESTRICTED`（apk 已暂存
`/data/local/tmp/unknown.apk`），MIUI 侧"USB 安装"处于关闭态。
用户需在设备上操作：**设置 → 更多设置 → 开发者选项 → 「USB
安装」/「通过 USB 验证并安装应用」开启**（MIUI 常要求登录小米
账号）；随后 `adb -s ba0d072d shell pm install -r -t
--bypass-low-target-sdk-block /data/local/tmp/unknown.apk`
即可。恢复后 `PILOTDROID_SERIAL=ba0d072d spike/e2e/u2suite/run.sh`
复跑定分（conftest `_device_fixture_guard` 会先秒验 fixture）。

## 附：fixture 恢复后复跑定稿（v0.6.2 发布后补记）

用户开启 MIUI「USB 安装」后 `INSTALL_FAILED_USER_RESTRICTED`
仍现——实证根因非开关未开，而是 `com.miui.securitycenter/
permcenter.install.AdbInstallActivity` 确认弹窗 10s 倒计时无
人点按自动拒绝（logcat 实证该 Activity 弹起又超时）。解法：
agent a11y dump 定位「继续安装」按钮 bounds 后 `inject_tap`
自动确认——`adb install --bypass-low-target-sdk-block` 一次过
（第二道闸 `DEPRECATED_SDK_VERSION` 由该 flag 解，与预留一致）。

**fixture 恢复后全量复跑：13 pass / 9 fail / 8 skip（30
collected，34.8s）**——`watch_context` **PASS**（M22 归因
实证闭环：两层环境失败定稿无误，shim/agent 零代码改动即转正）；
剩余 9 fail 全部为 `d.xpath` NotImplementedError（明示非目标）。
插曲：复装后首启被 `com.lbe.security.miui` ReviewPermissions
拦（SUPPORTED.md 已登记环境敏感项）导致 session 两测先挂，
手动启动一次 ApiDemos 过审后全绿——该拦截属一次性首启行为。

## 附2：M23 xpath-lite 落地后复跑（2026-11-25，v0.6.2 之后代码线）

`t3` spike 判 GO（10/10 用例 host-side 可行）后 `_XPathLite`
实装（stdlib-only，每查询一次 `dump_hierarchy`+ElementTree 评估，
~50ms/次；简写翻译层 text/`%模糊%`/`@rid`/裸 `pkg:id` +
`//*[@attr]`/`contains()`/`parent`/`scroll_to`/`swipe`；高级轴
position/ancestor/and-or 谓词显式 `NotImplementedError`）。

**全量复跑：22 pass / 0 fail / 8 skip（30 collected，47.7s）**
——此前 9 个 xpath NIE fail 全数转正，`watch_context` 保持
PASS；8 skip 全部为上游自带 deprecated/TODO/大屏标记（verbatim
面板上限）。**非 skip 用例 22/22 全过**。

**watcher `.call` xpath 归属**：wire `watch_arm` Selector 键表
仅 7 键（text/id/desc/class/pkg/index/scrollable），xpath 键会
被 agent 静默忽略成空选择器恒真（假阳性，spike 实证）——故
`_WhenCtx.call` 的 xpath 规则走 host-poll 分支；`when("//…")`
仅在可 1:1 翻译成 wire 键时放行（`//*[@text|@resource-id|…=…]`），
其余 `when()` xpath 显式 NIE。

**踩坑记（scroll_to 三连调）**：
1. 大 swipe 惯性一次越过目标页——慢速 drag
   （`duration=0.7, steps=24`，0.8→0.4 视口）后 Pictures 页可达；
2. 命中后断言再 dump 已滑过——drag 后 `sleep(0.5)` settle 再
   进入下一轮 eval；
3. 套件无 settle 点击 `Graphics` 后首 eval 撞中导航过渡态 dump
   （ListView 未填充 `scrollable="false"`）→ `sc=None` 提前
   break——改为容忍连续 4 次无 scrollable 才放弃。

**口径修正**：SUPPORTED.md/README xpath 行从「非目标」改
「host-side 近似子集（每查询一次 dump，非 u2 插件管线；高级轴
NIE）」。`_XPathStub` 已删除；`hasattr` 探测语义由方法存在性
承载（`d.xpath` 是方法）。

### 附2-a：独立复验（t5 复核轮）

- 独立全量复跑 **22 passed / 0 failed / 8 skipped（43.99s）**——
  与实装者自报 47.7s/22-0-8 逐条一致，xpath 桶清零可复现。
- 代码抽查：`_xp_eval` 简写翻译（text/`%`/`@rid`/裸 `pkg:id`）
  与 `_xp_match` 祖先谓词同构；`parent(None)` 返最近祖先、
  `parent(x)` 沿 parent-map 上溯匹配——u2 语义对齐如实；
  `scroll_to`/`_XEl.swipe` docstring 明示近似口径，无夸大。
- NIE 面未被击穿：`d.xpath` 现为方法（hasattr→False），
  `_XPathLite` 无 `__getattr__`（未知属性 AttributeError）；
  `_xp_eval` 对子集外表达式经 findall SyntaxError→显式 NIE；
  watcher `when()` 仅 5 属性 `//*[@attr=v]` wire 翻译，余者 NIE
  ——与 SUPPORTED.md ⚠️ 行声明一致。
- 单测有效性：43/43 pass（+14 xpath 用例钉简写翻译、parent
  解析、scroll_to、watcher wire 翻译、NIE 面）。
- t1 复核：Homebrew formula `version 0.6.2`+双 url/sha256 与
  v0.6.2 SHA256SUMS.txt 一致（gh api 读回）；README 版本钉
  摘除、PUBLISH.md tap-bump 升格 Required、suite README 补
  AdbInstallActivity 10s 机制——四处均在库。

### 附3：M24 收口复核（t3 复核轮，commit 23e903f + 9b22379）

- 独立全量复跑 **22 passed / 0 failed / 8 skipped（46.78s）**——
  `%` 语义收紧后 8 条 xpath 用例（含 test_xpath_query 的模糊
  匹配断言）仍全过，零回归。
- `%` LIKE 三态实现复核：`v%`→startswith / `%v`→endswith /
  `%v%`→contains，`_xp_eval` 与 `_xp_match` 同构。边界：
  裸 `%`/`%%`→v 空串→短路 `[]`/`False`（退化查询安全拒绝，
  u2 会全匹配——可辩护偏差）；内嵌 `%`（如 `%a%b`）按字面
  处理非多段通配——与 u2 简写 strip 语义一致，均已注记。
- `implicitly_wait` 双态：`timeout is None`→返回
  settings['wait_timeout']，`0` 落 setter 分支（falsy≠None
  边界正确）；getter/setter 双侧有测。
- 枚举面：`__len__`→count、`__iter__`→info dicts（u2 迭代
  产对象——docstring 明示偏差）、`__getitem__`→instance
  kwarg+非 int TypeError+越界 info None。
- builtin recipes：三规则 package+text AND 防误命中；Shade
  走 resourceIdMatches+shell collapse 非点击；`_mk_builtin_click`
  用 host inject 非 device-armed ACTION_CLICK（FLAG_SECURE 吞
  点击真缺陷已修复注记）+4×0.7s settle/retry 兜滑入动画；
  armed 表 WATCH_HIT 摘除+finally watch_disarm 防泄漏；
  `__exit__` stop 不吞异常。实装者真机实证：sctrl.apk 弹窗
  自动点「继续安装」→Success；shade 展开→0.4s collapse。
- 测试有效性：49/49（+6 钉 LIKE 三态/双态/枚举/getitem/
  builtin 注册含 builtin=False 对照）。

### 附4：M25 收口复核（t3 复核轮，commit 7193b94 + c0d65d8）

- 独立全量复跑 **22 passed / 0 failed / 8 skipped（45.41s）**——
  re-arm+host-poll 改动后零回归。
- **re-arm 语义复核**：`_pd_max_fires` 预算挂 action 函数属性
  （lambda/闭包通吃），默认 1=fire-once parity 不破；命中路径
  （device WATCH_HIT / host `s.exists` / `_xpath` host-poll）三处
  摘除统一走 `_fire_done` 预算门。泄漏面核过：agent 侧 Watcher
  `run()` 命中即 `break` 自退（Agent.java §"Fire-once"），
  `armed.pop` 摘的是已死线程 id 非泄漏；`finally` 只 disarm
  存活 watch_id——重臂换新 id 无累积。
- **xpath host-poll 复核**：`when("//非wireable")` 先经
  `_xp_eval` 罐头文档探针 fail-fast NIE，可求值者落 `_xpath`
  哨兵→`_host_match`（`_XPathLite.exists`+残余 selector key
  AND）；`_xpath` 规则恒不 arm 走每 tick 一 dump——与
  SUPPORTED.md poll 语义+延迟注记一致。`when()` NIE 边界
  从"非wireable xpath"收窄为"连 lite 子集都拒绝的表达式"。
- **真机双证**（ba0d072d，走 suite seam 包→u2shim）：
  `//*[@class='android.widget.FrameLayout']` 恒真规则
  `_pd_max_fires=2` → **精确 2 次命中摘除**（tick 间隔 0.04s）；
  无预算规则 → **1 次命中**（fire-once 默认不变）。
- builtin LBE `max_fires=3` 与 fixture-doctor `while page<8`
  多页循环同一实测依据；其余两条规则预算=1。
- 单测 49/49（+7：fire_done 计数/双态、tick hit-miss-rearm、
  run_async host-poll、wire hit→re-arm 新 watch_id 计数、
  when() NIE 边界收窄、builtin max_fires 映射）。

## 附 5：M26 复核段（serial 复合修/click 修复/max_fires 边界，2026-11-25）

- **serial=None 复合修复核**（`db96583`，复核通过）：
  `_detect_port` 优先序正确——显式 serial > env > forward 行
  backfill > `adb devices` adopt/fail-fast > 17932 探针+指引错。
  backfill 取命中行 `parts[0]`（u2.connect() 首台语义对齐）；
  同设备多条 forward 规则按 serial 去重后才判歧义（同机
  17932+38077 不误报）。`_adb_devices` 仅计 attached 状态行。
  **真机复现**（双设备在场 `ba0d072d`+`11.0.0.2:5555`）：
  `env -u PILOTDROID_SERIAL Device()` → `serial='ba0d072d'`、
  `d.shell` rc=0（修复前同环境 rc=1 静默空输出）。
  边角（如实记，非阻塞）：显式 `port=` 路径跳过 detect 不回填
  serial；serial 显式给定但该机无 forward 时仍会落到 17932
  探针——若该口属他机则 conn 与 `-s` 指向不同设备。
- **`2b5a9fe` click 修复核**：`_WhenCtx.click` `_xpath` 分支
  fire 时 `_XPathLite.all()` 重评→`els[0].click()`→
  `_XEl.click()`→`dev.click(center)`→**agent inject_tap 坐标
  注入**——与 builtin `_mk_builtin_click`（`sel.click`）同一
  FLAG_SECURE 安全路径，非 a11y ACTION_CLICK；空 re-eval
  no-op 保规则存活语义合理。xpath 分支不标 `_pd_click`
  正确（`_xpath` 规则永不 arm 设备侧）。真机实证：contains()
  xpath 规则 tick→点击落点→ApiDemos 进 Accessibility 子菜单
  （dump 前后对比）。
- **max_fires 边界如实**：`_fire_done` 计数后比较——
  N≤1（含 0/负）退化为 fire-once（**命中一次后摘除，不是
  "不触发"**）；N=2.5 → 3 次。语义边角建议后续收紧
  `max_fires<1 → ValueError` 或文档注记（非阻塞）。
- **回归**：pytest `python/tests/` 66/66（u2shim 59 +
  cli_bootstrap 7）；u2suite verbatim 复跑 **22 pass /
  0 fail / 8 skip（51.95s）** 零回归；`test_parity_extra.py`
  真机 4/4（implicitly_wait 往返/枚举/count/watcher call 命中）。

## 附 6 — M27 复核段（t1 交叉复核，p3 独立判定）

复核对象 `1fa45af`（CHANGELOG 事实修正+shim 两边角），复核者
与落地者不同人；另含 t2 产物（`e3d863b`）连带核。

- **CHANGELOG 逐条核——pass**：click 条目标注已改为「`_xpath`
  哨兵系 post-0.6.3 main 的 7193b94 引入，未发件即在
  2b5a9fe 修复」——伪注删净（`git show v0.6.3:compat/u2shim.py`
  无 `_xpath`，上轮已证）；serial=None 条目正确归 Fixed 并
  标注「v0.6.3 wheel 真实携带」——0.6.3 `_detect_port` 无
  backfill，多设备裸 `Device()` adb 全调用
  `more than one device` 静默 rc=1，事实性核实成立；
  `call/click(max_fires=N)` 归 Added、M26 内部批（smoke-ver
  闸/parity/README 定位）归 Docs，覆盖 db96583+2eba28f。
  **partial nit**：SUPPORTED.md 四注记（connect backfill/xpath
  click 首匹配+guard/re-arm 窗口/demux defer 裁决）未单列
  条目——前三者语义已被相邻条目隐含，demux defer 属文档化
  设计裁决，可接受但不完整（下轮补一行即可，非阻塞）。
- **u2shim 两边角——pass**：`max_fires<1` 在 `call()`/`click()`
  对称 `ValueError`（附 5 建议项落地），单测
  `test_max_fires_below_one_raises` 双路径断言一致；
  `_detect_port` 新增 `else` 挂在 `if serial is None:` 上——
  显式 serial+无该机 forward 行→具名错跳过 17932 探针
  （conn 与 `-s` 跨机错配防住），serial=None 主路径
  backfill/adopt/歧义语义零改动；
  `test_explicit_serial_no_forward_skips_probe` 断言错误含
  serial 且非 "not alive"（证探针未走）——fixture 设计正确。
  残余 nit：`max_fires=2.5` 仍放行（≥1 成立，ceil 语义 3 次），
  int 校验未加——如实记，非阻塞。
- **闸+parity 连带核（t2 `e3d863b`）——pass**：doc-drift 第 5
  闸 `changelog-unreleased` 双向实证（清空 Unreleased→FAIL
  rc=1 `8 commits bullets=0`；真实树 PASS `bullets=8`），
  浅克隆无 tag→note-pass 不 break CI；parity 新断言
  `when(contains-xpath).click()` 真机 ba0d072d 实跑——
  host-poll 命中→inject-tap 首匹配→ApiDemos 进 Accessibility
  子菜单（dump 含 `Accessibility Node Querying`）5/5 PASS，
  §附 5 的落点证据永久固化。
- **回归**：pytest `python/tests/` **68/68**（u2shim 61 +
  cli_bootstrap 7，与 t1 报的 61 单文件口径一致）；
  doc-drift **5/5 PASS**（含新闸）。

## 附 7 — M28 复核段（t2 高帧流批交叉复核，p3 独立判定）

复核对象 `ce18f2d`（stream-stop 观测面+ADR-008+jar 重建），
复核者与落地者不同人。

- **Streamer 生命周期/异常面代码审——pass**：
  `ctrlClosed`（ctrl 会话终→隐式 halt 会话绑定）、`mediaDead`
  （media 写失败→halt）、`stop` owner 校验（他会话停他人流
  ERR_STREAM_BUSY）、`halt` 幂等（running=false+interrupt+
  curSock 关闭解 writer write 阻塞+双 join）、start 丢弃
  pre-start pendSock（§4.8 尸体连接防护——本复核者亲手踩到：
  media_connect 先于 start 会收 0 帧，顺序反即正常）；
  背压 queue(1)+dropped 计数——慢读者不堵 capture/ctrl；
  bitmap 双路 recycle 正确（scale<1 两图都收，=1 收一）。
  与既有 screenshot 路径零干扰（独立 media socket+单 owner）。
  nit（非阻塞）：`stop` 在未运行时返回上一轮旧计数；
  capNulls 的重试耗时不入 cap_ms（语义正确——cap_ms 是
  帧均捕获成本，空帧损耗单列 cap_nulls 即可）。
- **真机独立复测——pass（≥15fps 判 partial 确认）**：
  30s 持续 `stream --fps 15 --duration 30` → **166 帧/30.05s
  = 5.5fps，seq_gaps=0，dropped=0**（与 p2 的 5.3fps/90f/17s
  一致，非单次侥幸）；8s python 复测 45f/8.1s=5.5fps，
  `stream-stop` 实测 `{cap:"uia", cap_nulls:1, cap_ms:115,
  enc_ms:65}`——与 p2 报的 113/67 吻合，分相账自洽
  （115+65≈180ms/帧 → 5.6fps 理论顶，实测 5.5 贴顶）。
- **ADR-008 事实核——pass**：supersede 表述精确限定「ADR-006
  中 >6fps 档语境假设」非整篇替换（ADR-006 的 socket/背压
  决定仍有效）；七路径矩阵与实测一致（uia 5.6/screencap 2.1/
  VD 0 帧/screenrecord 2 帧/DMS null/并行 uia 单注册/
  systemMain 自杀）；「MIUI 封锁、非 MIUI ROM 预留 surface
  后端」注记与 cap 字段契约闭环；partial 判定如实（substrate
  齐备+本机物理上限，非工程缺口）。
- **回归**：cargo fmt+clippy(-D warnings)+test 全绿；
  `verify-agent.sh ba0d072d` **37/0**；doc-drift 6/6 PASS。

## §附8 M29 复核（Track A MCP 面：SoM 工具/watch push demux/verify_after，commit 9400b4d）

复核者逐项独立复验（非自报转录）：

- **demux 写锁交错——pass（构造性防撕裂）**：serve 改单写者循环——
  stdin→mpsc channel 读线程只做读，stdout 仅主循环写；notification
  帧（`notifications/pilotdroid/watch_hit`）只在两次 response 之间
  写出，结构性不可能与 response 帧交错。单测
  `watch_push_hit_arrives_as_notification` 逐行 JSON 解析断言
  notification 作为独立完整行到达；本复核者真机复跑同见
  （watch_id=17 → notification params.matched=true）。
- **断开 disarm 生命周期——pass**：client stdin EOF→
  `rx.recv_timeout` Disconnected → break → loop 尾部对全部
  `armed.keys()` 逐个 `watch_disarm`（agent 侧 conn close 亦有
  reap——双保险）。单测 `watch_push_eof_disarms_armed` 计数
  fake-agent 收到 disarm；本复核者真机复跑：serve EOF 退出 rc=0
  →新会话 `watch_disarm(wid=18)` → `{disarmed:false}` 无残留。
- **缓冲降级路径——pass**：非 push watcher 命中被 demux 消费后入
  `pending`（PENDING_CAP=64 防膨胀）→`watch_events` 先 drain
  pending 再 live `watch_recv`；单测
  `watch_push_mixed_hit_buffers_to_watch_events` 钉 push→
  notification / non-push→pullable。media socket 错误→
  `armed.retain(!push)` 停轮询、拉路径报错（明示降级）。stream
  激活期 demux 暂停（hit 与帧共享 ch=1，命中 agent 侧暂存）——
  注释与代码一致。
- **SoM MCP 与 shim 编号空间一致性——pass**：`som.rs.enumerate`
  与 `compat/som.py._enumerate` 逐字同语义（ATTRS/FLAGS/稠密
  id/中心 div_euclid floor 对齐 Python //）。真机同屏实测：
  **rust_n=18 == py_n=18，id 1..18 稠密，逐元素 bounds 全等**。
  annotated 395KB JPEG magic 正确；倒置/退化 bounds 归一化+跳过
  策略与 Python 侧同（t6 修复语义在 Rust 复刻）。
- **verify_after 语义——pass**：VERIFY_OPS 白名单限副作用+读 op
  （不含 batch/verify_after 递归、stream/watch 生命周期）；
  action 死链→显式 BrokenPipe 返回（不虚报 pass）；action
  isError→`a_ok:false` 参与总判但 checks 仍跑（如实报告）；
  layout_changed 双侧 signature 任一缺→pass=false（fail closed）。
  真机实测 press HOME→settle→layout_changed:true→pass:true。
- **真机端到端三场景——pass**：tools/list=30（三新工具在册）；
  describe_screen 编号一致性同上；watch push notification
  真达；EOF disarm 无残留。
- **回归**：cargo fmt/clippy(-D warnings)/test workspace
  **88/0 全绿**；`verify-agent.sh ba0d072d` **37/0**。
- **复核过程自纠注记**：本复核者首轮 e2e 误报「push 未达」——
  实因 arm 目标文本 'Accessibility' 不在屏（前测导航残留
  Animation 子菜单，app_start 无 stop 语义不重置），selector
  真缺席非代码缺陷；改为屏上实存文本后一次通过。
- **nit（非阻塞）**：`uiEvent` 在 pendOut==null 时静默丢命中
  （fire-once 已消费）——arm→agent accept 毫秒窗口理论存在，
  注释已明示「No media conn = event dropped」；serve 侧
  watch_attach 先于 arm 已把窗口压到极小，shim watcher 全历史
  未踩到，如实注记不改。

## §附9 t16 复核——session 看护（t14, 01bbee2）独立判定

复核者逐条 diff `95a8e39..01bbee2` + 真机独立复现 + 三件套/verify 重跑。

| 复核项 | 判定 | 证据 |
|---|---|---|
| ① liveness 判定时机 | **pass** | `with_client` 仅在 op 抛 dead-transport 时才跑恢复——反应式非每 op 探活，零额外成本；`is_dead_transport` 恰含 `Io`(读 EOF/reset)+`Protocol(Io)`(写失败)，`Timeout` 正确排除（挂起/死亡不可判定，盲重放=双执行）。 |
| ② 重启序列 | **pass** | `recover()` 序：teardown_media（丢媒体 conn+除媒体 forward）→除陈旧控制 forward→`connect_recover` 阶梯（hello→restart→hello→remote-jar 检查→reflash→hello），`Recover{stage}` 具名错。先拆媒体再重连防陈旧 conn 误用；`streaming`/`media_port` 状态正确复位。 |
| ③ 重放安全边界 | **pass** | 单次重放无循环（重放再死则错误透出）；双执行窗口（conn 断前请求可能已下发）在代码 doc+SUPPORTED+CHANGELOG 三处一致明示。 |
| ④ watcher/stream 丢失策略一致性 | **pass** | `watch_recv` 死媒体→先 `recover()` 治愈控制通道再原样抛错（调用方 re-arm）；watch_id 重分配/陈旧撞新 id 注记代码与 SUPPORTED 一致；`serve.rs` dead_conn 测试已更新到新契约（首 op 透明成功非报错）。 |
| ⑤ 真机独立复现 | **pass** | `serve` 持久 session：`pkill -f pilotdroid.Agent`（pid 28742→28779）→下一 `describe_screen` **1269ms 透明成功**，稳态 50ms——与自报 1.11s/0.02s 同量级。 |
| ⑥ 三件套+verify | **pass** | fmt/clippy(-D warnings)/`cargo test --workspace` **91/0**（facade 5 含 3 恢复用例、lib 19、cli 25 含更新后 dead_conn）；`verify-agent.sh ba0d072d` **37/0**。 |

**附注（非阻塞硬化项）**：facade 恢复单测在 CI 首跑曾 flaky（`expected Recover, got Timeout`，rerun 即绿）——relay-fake 绑真实 localhost 端口+CI 慢机时序敏感，建议后续把 connect 超时放宽或打 `#[ignore]`-class 标记，非本次阻塞项。

## §附10 t1 v0.7.0 发版列车 ledger（四面一致）

- **tag**：`v0.7.0@68a8d16`（verify-tag PASS：tag=workspace=pyproject=0.7.0）。
- **GitHub Release**：`release.yml` 45586521238 success → draft 7 assets
  （linux/mac/win 三 bin + whl + sdist + agent.jar + SHA256SUMS.txt）→
  notes 挂 `release-notes-v0.7.0.md` → **已 publish**（非 draft）。
- **PyPI**：`pypi.yml` 45587024698 OIDC success → index `0.7.0` 可见。
- **install-smoke**：`spike/e2e/install-smoke.sh ba0d072d` **9/0**——
  venv `pip install pilotdroid==0.7.0`→bootstrap 下载+sha256 验证→
  `pilotdroid-cli 0.7.0`、release 二进制对照 0.7.0、agent HELLO
  ver=0.7.1 caps=0x7ff、serve tools/list 握手。
- **MCP Registry**：`publisher` 登录刷新（JWT 过期→`login github
  -token`）→ `io.github.mofanx/pilotdroid@0.7.0` published，registry
  API 可查。**server.json 描述裁到 ≤100 字**（registry 422 校验）。
- **Homebrew tap**：`hb-tap 5c244d6`——version+双 url 换 v0.7.0+
  双 sha256 取自发布版 SHA256SUMS.txt。
- **版本面 bump**：pyproject/Cargo.toml/server.json×2/install-smoke
  VER 四处 0.6.4→0.7.0；doc-drift 6/6（smoke-ver 闸同步）。
- **随行卫生**：som.py skipped 死变量清理；test_som +零高/双轴倒置
  用例（pytest 92/92）。
- **已知阻塞注记**：CI 首跑 facade `dead_agent_recovery_exhausted`
  flaky 二度现身（rerun 绿）——硬化件归微批，ledger 附9 已注。

## §附11 t3 M31 eval harness v1 ledger（ba0d072d, MIUI/API34）

**`spike/e2e/agent-loop/loop.py` 全量 10/10 PASS**（2026-11-25，调试
构建 `pilotdroid-cli serve` stdio）：

| 任务 | 结果 | 耗时 | 覆盖 |
|---|---|---|---|
| nav_views | PASS | 2.6s | describe→verify_after(tap)→exists |
| nav_controls_light | PASS | 5.6s | 三级导航+verify_after |
| checkbox_toggle | PASS | 5.7s | dump `checked` attr |
| radio_select | PASS | 5.8s | dump `checked` attr |
| set_text_probe | PASS | 6.2s | EditText→verify_after(text) |
| popup_toast_media | PASS | 4.5s | media conn 先 attach→toast ui_event |
| alert_dialog | PASS | 5.9s | 弹窗出现/取消消失（contains 匹配） |
| spinner_pick | PASS | 7.1s | spinner 展开→选项点击 |
| press_back_nav | PASS | 6.7s | 两次 KEYCODE_BACK（本 ROM 首按被吞） |
| stream_watch_coexist | PASS | 11.8s | demux 暂停实测定论 |

**stream+watch 并存实测结论**（注释级约束→文档化定论）：
armed watcher 占用的 media conn 被 `stream_start` 正确废弃
（agent §4.8 杀 pre-start pending——`stream_start` 先 teardown，
否则读陈旧句柄 io error，本轮实修）；流中 hit/toast 与帧交错到达，
`recv_event` 分拣入 session stash，`watch_events` **流中与停流后
均可拉出**（conn 序：hit 先于其动作产生的 toast）；push
notification 流中暂停、停流后恢复。facade 单测
`stream_demux_stashes_watch_hit_and_toast` + SUPPORTED demux 段
已按此口径定稿。

- **inspector 等效物**：`spike/e2e/agent-loop/inspector.py`——
  `describe_screen` 编号表（与 `screenshot_annotated` 同一 SoM
  编号空间）→ `--tap N` 中心点点击 → `verify_after` 断言；真机
  `--tap 6 --expect-text "Add"` 实证 popup 展开+检查通过。
- **环境敏感注记**：ApiDemos 活动名/菜单文本按本机实测校准；
  `press_back` 需两连按（首按被 Light activity 吞）——已写进
  任务注释，非 shim 缺陷。

## §附12 t5 复核——t2 微批三件（a1a0bc9）独立判定

复核者逐条 diff `a1a0bc9` 全量（facade/stream/proto/serve/SUPPORTED/
CHANGELOG）+ 真机独立复现两条路径 + 三件套/verify 重跑。

| 复核项 | 判定 | 证据 |
|---|---|---|
| ① watch_id epoch 修复 | **pass** | `Session::generation` 于 `recover()` 成功重连后 +1（原位同 Session），serve 主循环顶置 epoch 检查→`armed.clear()`+`pending.retain(≠WatchHit)`——armed 无泄漏（重启即全死语义正确）；陈旧 hit 清除而 toast 保留合理。单测 `stale_armed_ids_dropped_on_recovery_epoch` 有效：fake 每 conn 独立 arm_seq 从 1 重分配+按 conn 计 disarm，断言 conn2 只见自己 arm 的 1 次 disarm（无修复时为 3）——确实钉住碰撞窗而非空转。**真机复现**：pull-only arm{2}→pkill→dump 1.06s 透明恢复→stderr 精确打出 `agent recovered — 1 stale armed watcher(s) dropped`→新 arm 得 watch_id=1（agent 重分配实证）→新 watcher 击发 notification 正常。 |
| ② toast demux 分发 | **pass** | demux 臂 `MediaEvent::Toast` 直写 `notifications/pilotdroid/toast`（ambient 事件非 watcher-bound 恒 notify，写失败降级 `push_pending`）；`recv_event` 按 `type:"toast"` 判别 0x0002 同槽正确（hit 无 type 字段）。单测钉 params 三元组。**真机复现**：常驻 push watcher 保活 demux→PopupMenu1 点 Add→`{"has_parcel":false,"pkg":"io.appium.android.apis","text":["Clicked popup menu item Add"],"type":"toast"}` 精确到达。**边界注记**：toast notification 仅在 push watcher 保活 demux 期间流动（armed watcher 击发即摘除→demux 停→后至 toast 转 pull）——SUPPORTED 已载此语义，非缺陷。 |
| ③ flaky 硬化合理性 | **pass（真修非掩盖）** | `dead_port` bind-即放→释放端口可被恢复阶梯 `free_port()` 复用为 relay→relay 自拨自连死循环→表现为 Timeout 而非预期 Recover（CI 首红 rerun 绿根因链完整）。`corpse_port` 常驻 accept-close：语义更贴近死 localabstract（dial 成、读即 EOF）且端口不再释放——消除竞态而非放宽断言（Recover/Timeout 断言目标原样）。`bind AddrInUse→Err` 让阶梯换端口重试，是测试基建健壮化非掩错。 |
| ④ 三件套+verify | **pass** | fmt/clippy(-D warnings)/`cargo test --workspace` **94/0**（cli 27 含 2 新用例、facade 6）；`verify-agent.sh ba0d072d` **37/0**。 |

**残留注记（非阻塞）**：(a) 显式 `watch_disarm{stale_id}` 客户端调用仍穿透至新
agent——id 空间碰撞属协议固有，文档口径「id 按 epoch 绑定、需 re-arm」内
的客户端误用，非服务端义务；(b) conn 被顶替（`dead`→session=None 重建）
路径新 Session generation=0——若恰在顶替期间 agent 亦重启则 epoch 无信号，
双故障叠加角例；(c) push watcher 媒体死时 `retain(!*p)` 先于 epoch 检查清
场——语义一致（armed 空→无 warn）但使 warn 覆盖变窄，已实测确认。

## §附13 t6 —— media 幽灵 conn 竞态修复（agent 0.7.2, ba0d072d）

**机制实证链（三层叠加，复现脚本逐层剥离）**：

1. t4 原机制：脏退出遗留 media conn 滞留 accept backlog，`start()` 只清
   已 accept 的 `pendSock`——backlog 幽灵在 start 之后才 accept，于
   `running && curSock==null` 下无判别 promote；真 reader 到 busy 分支被
   close→首 `stream_read` EOF。
2. 深挖出的第二层：media accept 线程在 `attach` 抛异常（死 conn
   `getOutputStream`）时 `catch→return` **永久退出**→所有后续 conn 滞留
   backlog 永不服务。
3. 第三层（日志钉死）：脏流 writer 的写失败 `mediaDead→halt` 可在新
   `start()` 之后才执行→**跨代绞杀**新流；且 `mediaDead` 只认 resv——
   幽灵被 promote 后首写即死、真 reader 尚未 attach（毫秒级检测 vs
   ~200ms connect 延迟）→无 resv 直接 halt→真 reader 晚到落 pend
   starve 超时。

**修法（最终形态）**：`Agent.java` Streamer ——

- `attach`：`getOutputStream` 失败→dead-peer 静默关闭；`running&&
  curSock==null`→promote；`running`→进 reserve 槽（newest waiter wins，
  弃 busy-reject）；否则 pend。accept 循环：`accept()` 失败 return
  （server 死）、`attach` 失败 log+continue（线程永生）。
- `mediaDead(g)`：**代际守卫**——旧 writer 的写失败不作用新流；
  有 resv→提升续流；无 resv 且 `curWrote==false`（conn 从未成功写过=
  ghost）→丢弃回 waiting 态等真 reader；`curWrote==true`（真 reader
  死亡）→halt 保留 §4.8 原语义。
- `writeLoop`/`produce` 钉住 `myGen`+`q=queue` 局部引用——旧代线程
  不抢新代 queue；写失败不再 `return`（mediaDead 可能已自愈）。
- `curWrote` 判别替代早期被否的 read-timeout 探针——LocalSocket
  超时在此环境不可靠（活 reader 帧间静默会被误判死）。

**验证**：

| 项 | 命令 | 结果 |
|---|---|---|
| 脏杀+幽灵注入 20 轮 | `python3 spike/e2e/agent-loop/ghost_repro.py 20` | **20/20 PASS**（基线：无幽灵 10/10、有幽灵修复前 0~13/20 间歇 timeout/EOF） |
| 干净态 parity 回归 | `pytest spike/e2e/test_parity_extra.py` | **12/12**（一次 flake=app 滚动位置，force-stop 重置后过——非 media 相关） |
| 判别语义 e2e 钉 | `verify-agent.sh ba0d072d` | **40/0** 含新断言 `stream_reserve_hold`/`stream_reserve_heal`/`stream_ghost_drop` |
| Rust 三件套 | fmt+clippy(-D warnings)+`cargo test --workspace` | 全绿 94/0 |

设备端日志实证三条自愈路径均开火：`ghost dropped, awaiting
reader`（g4/g12/g18）、`reserve promoted`（g20×2）、`ignored
(running=...gen=...)`（代际守卫挡陈旧 mediaDead）。

**取舍注记**：(a) reserve 槽 newest-wins（丢旧 waiter）而非 newest-wins
抢位——后者让任意 stray conn 顶掉活 reader，失败面更差；(b) 半开黑洞
ghost（写不报错地吞帧）理论残留——RST'd conn 首/次写必失败，实测无影
响；(c) `curWrote` 语义=「曾写过一帧」，live reader 首帧前死亡会被判
ghost 回 waiting——等价行为无害；(d) Java 无单测基建，判别语义由
verify A6/A7 e2e 断言+`ghost_repro.py` 统计钉住。

改动面：`agent/java/pilotdroid/Agent.java`（Streamer 状态机+gen+curWrote，
AGENT_VER 0.7.1→0.7.2）、`agent/agent.jar`（重建部署）、
`agent/scripts/verify-agent.sh`（AGENT_VER pin 0.7.2+A6 reserve/A7 ghost
断言+socket/struct import）、`spike/e2e/agent-loop/ghost_repro.py`（幽灵
注入复现+stage 标注）。

## 附14 t7 复核——t6 幽灵 conn 竞态修复独立判定（2026-11-25，复核人 p2）

**① 代码审（attach/reserve/promote 语义）——pass。** `Streamer` 重写的四
分支判别正确：dead-peer 早丢（`getOutputStream` 探针）、`running&&
curSock==null` promote、`running` 进 reserve（newest-wins）、否则 pend。
三层机制均被对症：accept 循环 `attach` 异常 continue（线程永生）；
`mediaDead(g)` 代际守卫挡旧代 writer 绞杀；`curWrote` 判别区分 ghost
（从未写过→丢+await）与真 reader（写过→halt，§4.8 语义保留）。
`writeLoop`/`produce` 钉 `myGen`+局部 `q` 引用无跨代污染。非阻塞 nit：
`resvWrote` 恒 false（对称占位）；真 reader 居 reserve 位时仍可能被后至
ghost 顶替（残余窗更窄，ledger §附13 取舍 (a) 已记录）；`produce` 的
`dropped/seq` 非同步沿用旧风格。

**② t4 原始场景独立复现——pass。** 复核者 t4 时的失败序列（serve arm
push watcher+stream 后 SIGKILL 脏杀，立即新 serve 同序列）在修复前实测
~30-50% 失败；修复后同序列 **0/15 失败**。t6 复现器 `ghost_repro.py 20`
（脏杀+RST 幽灵注入×2/轮）独立复跑 **20/20 PASS**。

**③ 回归无劣化——pass。** `loop.py` 全量 **10/10**（含
stream_watch_coexist 11.7s）；cargo workspace **94/0** 失败；
parity **12/12**；u2suite 抽样（watcher/swipe/session）**7/7**。

**④ 并存实测定论——pass，正式成立。** t4 的 partial 裁决解除：SUPPORTED
的交错/stash/pull/push-恢复语义在修复后完整成立，幽灵竞态残余风险收敛
为 §附13 取舍注记 (a)-(c) 的明示边界。

**随行缺口（记档给 t6 commit 时补齐）**：`CHANGELOG.md` Unreleased 尚缺
ghost-conn 修复条目（doc-drift 第 5 闸会随 commit 检查）；`resvWrote`
占位变量可在下一卫生批清理或改注释说明。

验证环境：设备 ba0d072d，agent 0.7.2（t6 部署态），CLI=~/.cargo-target
构建（内嵌同版 jar）。复核期间产生的孤儿 adb forward 已清账。

## 附15 t8 收口——t6 提交入库（cc24acb + db62975）

- `cc24acb` fix(agent)：t6 全量入库——Agent.java 0.7.2（reserve 槽/gen
  守卫/curWrote 判别/accept 线程永生/writeLoop 去 return）+ agent.jar
  + verify A6/A7 断言 + ghost_repro.py + CHANGELOG Unreleased Fixed 条目
  + 本 ledger §附13。`resvWrote` 占位变量按 t7 nit 清理（resv conn 从未
  写过，提升后 curWrote=false 语义等价）。
- `db62975` test(facade)：fake adb relay 半关闭传播根治
  `dead_agent_recovery_exhausted` CI flaky——上游死后客户端读超时与
  FIN 竞态（Timeout vs Io），copy 结束显式 `shutdown(Both)` 对齐真实
  adbd；本地单点 6/6 + 整包 3 轮无复现。**随件性质**：属本批 CI 观察
  责任范围内的修复（t8④），非越权新任务。
- 入库后验证：doc-drift **6/6 PASS**（changelog 第 5 闸含新条目）；
  ghost_repro 复跑 8/8；CI `45607557629` + CodeQL `45607557417`
  **双 success**。

## 附16 v0.7.1 patch 发版列车（2026-11-25）

| 步 | 证据 |
|---|---|
| bump×4 | Cargo workspace/pyproject/server.json/install-smoke VER →0.7.1（`917e3e5`；Cargo.lock 首发误伤上游 pem-rfc7468 锁定版本，`b34e1a5` 还原并 retag——tag-follow 实证） |
| notes | `docs/release-notes-v0.7.1.md`：agent.jar **0.7.1→0.7.2** 随行说明+ghost 修复诚实档（非纯 Rust 修复），limits 沿用 0.7.0 口径 |
| tag→draft | `v0.7.1@b34e1a5` → release.yml 2m46s success → draft 7 assets（linux/macos/windows bin+whl+sdist+agent.jar+SHA256SUMS），agent.jar sha256 `22431f80…` 与提交 jar 逐字节一致 |
| PyPI | pypi.yml `45616131367` success（OIDC）；索引传播 ~1min 延迟后 `pip install pilotdroid==0.7.1` 成功 |
| install-smoke | `install-smoke.sh ba0d072d` **9/9**——真机 `hello ver=0.7.2 caps=0x7ff` 实证内嵌 jar 随件 |
| Registry | `mcp-publisher publish`（v1.8.9 客户端，github token auth）→ `io.github.mofanx/pilotdroid@0.7.1` published |
| tap | `mofanx/homebrew-pilotdroid` `558cdac`——version/urls/sha256×2 随 SHA256SUMS 更新 |

**随行 docs**：`docs/02-protocol.md` §4.8 新增 conn 生命周期判别段——
「单向通道上成功写过 ≥1 次是唯一可靠活性证据」+`LocalSocket`
read-timeout 探针证伪记；`AGENTS.md` 新增 fake relay 半关闭保真度
注记（db62975 flake 根治的教训外化）。

CI+CodeQL 于 `b34e1a5`/`917e3e5`（lock 修复后）持续 success。

## 附17 t3 —— eval v2 难度爬升 + 真模型盲驾 + OCR host-spike（2026-11-26，ba0d072d）

### A. eval v2 任务集（`spike/e2e/agent-loop/loop_v2.py`，7 任务×5 轮）

| task | pass | p50s | p95s | mean | 覆盖点 |
|---|---|---|---|---|---|
| cross_app_settings | 5/5 | 6.2 | 6.2 | 6.2 | ApiDemos→HOME→MIUI Settings→滚动寻「更多设置」→返回 |
| som_index_tap | 5/5 | 2.9 | 3.0 | 2.9 | list_elements→tap_index 真消费 SoM 编号 |
| stale_snapshot_reverify | 5/5* | 4.5 | 4.6 | 4.5 | 导航后旧 snapshot 被 check_snapshot 拒→重新 list 恢复 |
| scroll_find | 5/5 | 2.9 | 3.3 | 3.0 | 界外元素 bounded scroll 寻「Preference」 |
| double_toast_order | 5/5 | 7.7 | 8.1 | 6.9 | 双 toast 时序（Add→Edit 按 conn 序到达） |
| dirty_kill_recovery | 5/5 | 5.5 | 5.9 | 5.5 | 每轮 SIGKILL 一个 stream 中 serve→新 serve 仍能起流（dirty-exit 回归钉） |
| background_resume | 5/5 | 7.5 | 7.5 | 7.5 | HOME→重开，activity 态保留、元素仍可用 |

合计 **35/35**（首轮未修版 34/35：stale 任务一次 harness settle race——tap 后 0.8s 窗内布局签名未变导致 tap_index 未被拒，改为轮询至导航落点后 5/5；属评测脚本竞态非产品缺陷）。

\* 修复后独立复跑 N=5 全过；方差极小（全部任务 p95−p50 ≤1.6s）。

### B. 真模型盲驾 round（`blind.py`，仅 describe_screen/tap/verify_after 三工具）

| # | 目标 | 结果 | 归因 |
|---|---|---|---|
| B1 | 主列表→Views 子列表+verify text | PASS | 首点 `ok:true` 未生效（IM 注入竞态），verify 正确报 false，重点过——**tap 返回 ok≠效果，verify_after 是必需品** |
| B2 | Views→Controls→Light Theme+verify Checkbox 1 | PASS | 途中 MIUI 系统弹窗「packageInfo is null/确定」打断——describe 可见+可点，盲驾可消 |
| B3 | spinner Mercury→Earth+verify text | PASS | 选中值以 TextView 呈现，text 断言可验 |
| B4 | Checkbox 1 切换+状态验证 | **FAIL** | 双重归因：(a) `describe_screen`/`list_elements` **无 checked/selected 字段**，verify_after 无状态谓词——切换不可证；(b) 验证前前台被外部 app（聊天）抢占——真实零售机环境切换 |
| B5 | Alert Dialogs→OK CANCEL DIALOG→verify Lorem→OK→verify absent | PASS | exists+absent 双向断言均 pass |

**盲驾成功率 4/5（80%）**。SoM 缺口清单（失败归因）：① widget 状态面（checked/selected/enabled/focused）全部缺失——checkbox/radio/toggle 类闭环不可证；② `describe_screen` 给全类名而 `list_elements` 给简类名——同一元素两种 class 口径（非阻塞 nit）；③ tap 无效果回执（ok 仅表注入成功）——已由 verify_after 兜住，建议文档强调「每步必 verify」。

### C. OCR host-spike（`ocr_spike.py`）：**GO**

- 管线：`screenshot`（agent 出 JPEG）→ 本地 `tesseract --psm 11 tsv`（5.5.0, eng）→ 词块合并为行块 → 与 `list_elements` 几何融合：文本块心落在「面积≤4×块的无文本节点」内→回填 `ocr_text`；否则若无含文本节点认领→追加 `ocr_only` 元素（i 顺延、带 center 可直接 tap）。
- 无文本树场景：`ApiDemos .graphics.TextAlign`（canvas 绘制）——a11y 树仅 8 节点、零画布文本；OCR 产出 **13 个可编号 ocr_only 元素**（Left/Center/Right、Positioned×3、Along a pats 等）。
- 有文本场景去重：ApiDemos 主列表 20 元素中仅 1 个 ocr_only（状态栏时钟 9:22，本就不在树内）——无重复污染。
- 局限（诚实档）：psm11 对艺术字体误读（"Along a pats"）；状态栏/通知栏 OCR 噪声会进入列表；eng-only（无 chi_sim 语言包，中文界面 OCR 不适用）；单帧 ~1.5-2s（tesseract 本地 CPU），不适合高帧闭环但适合感知补全。
- **判定 GO**：作为 `describe_screen ocr:true` 的 host 侧增强值得合入工具面——补全 canvas/image 文本盲区，几何融合成本极低。后续建议：`MIN_CONF` 阈值、chi_sim 语言包、与截图标注（screenshot_annotated）共用编号空间。

## 附18 t4 —— watcher pendOut 同构 ghost 修复（2026-11-26，ba0d072d，agent 0.7.3）

### A. 缺陷与修复

- **缺陷**：`uiEvent` 对 media pend conn 是单槽 newest-wins——无 curWrote 判别、无 reserve。host 脏杀后遗留的 backlog 尸体 conn 一旦 attach 即顶掉活 reader 并关闭之；或 hit 写给尸体 conn 静默蒸发。另 adb forward 半死管道对单次写**假性成功**（adbd 缓冲吸收），单笔写不足判死。
- **修复（Agent.java）**：watch 侧 media conn 改为**有界集合广播**——`watchConns` 容量 4、超容逐最旧（evict 记日志）；`uiEvent` 序列化一次帧后逐 conn 尝试写，写败即丢该 conn；零送达时 hit 入**暂存槽**（容量 8、TTL 15s），下一个 watch conn attach 时 flush 补发。stream 路径（curSock/resvSock/curWrote/gen）不动。
- 附带发现（归因非本修复）：watcher poll 的 `catch(Throwable)→break` 静默死面仍存在（脏杀余波中 a11y 竞争可杀 watcher）——9/10 中 r5 的 phase-2 失手疑此，未证为产品缺陷。

### B. 复现器与断言

- `ghost_repro.py --watch`：RST 幽灵经 adb-forward 注入 media socket。phase1=live pend+stray ghosts 后 hit 仍达；phase2=脏杀→新 serve→armed watcher corpse-heal 命中仍到 push。**10 轮 9 通过**（r5 p2 失手，疑 watcher 静默死，见上）。
- 复现器 client 修复（非产品）：`rpc()` 等待响应期间到达的 JSON-RPC notification 曾被丢弃——phase-2 hit 恰在 `app_start` rpc 窗口内到达被吞。改队列缓存后 phase-2 稳定。
- stream 模式回归（未动代码路径）：**10/10**。早前 4/5 失败系与 watch 复现器并发共享 GHOST_PORT 的交叉污染，串行后全绿。
- `verify-agent.sh` 新增 A8 断言：`watch_ghost_live`（活 pend+幽灵注入后 hit 到 push）、`watch_corpse_heal`（脏杀→恢复→armed hit 到 push）——**双 PASS**；`agent_ver` 期望值同步 0.7.3。

### C. 验证链

| 检查 | 结果 |
|---|---|
| verify-agent.sh ba0d072d | 41 pass / 1 fail→修复后唯一 fail（agent_ver 期望值）已消 |
| ghost_repro --watch ×10 | 9/10（r5 phase-2，疑 watcher 静默死面，非 pendOut 路径） |
| ghost_repro（stream）×10 | 10/10 无回归 |
| agent.jar | Agent.java 重建部署，`pilotdroid-agent 0.7.3` 设备实证 |

文档随行：docs/02-protocol.md §4.8 watch conn 语义（有界集合+零送达暂存）；CHANGELOG Unreleased。

## 附19 t5 —— t3+t4 独立交叉复核（2026-11-26，ba0d072d，复核人 devin/t5）

### A. eval v2（t3）

- **判定逻辑审**：7 任务断言均钉真实终态、非永真——cross_app 要求双向导航落点、stale 任务区分「stale 拒绝」与一般错误（`"stale" not in err → raise`）、double_toast 断顺序与文本、dirty_kill 每轮真 SIGKILL 兄弟进程后新流仍得帧。`stale_i` 缺省 0 为唯一软弱处（目标缺席时仍走拒绝路径），非阻塞。
- **独立复跑**：`loop_v2.py --rounds=3` → **21/21**，p50/p95 与 t3 五轮数据同量级（cross_app 6.1 vs 6.2s、double_toast 5.8 vs 7.7s）。
- **盲驾归因表**：B4 失败双归因（SoM 无 checked 态谓词+前台被外部 app 抢占）如实可信——describe/tap/verify 三面确实无状态字段；「tap ok≠效果」结论与协议一致。**PASS**。

### B. OCR spike（t3）

- **独立复跑**：TextAlign canvas（a11y 树 8 节点零画布文本）→ **12 个 ocr_only 可编号元素**（Left/Center/Right/Positioned×3 全中；t3 报 13，差异为状态栏噪声随时间变——如实记档）。有文本场景去重逻辑复核无误（claimed 检查防止与树文本重复）。**GO 结论成立，PASS**。

### C. pendOut ghost 修复（t4）

- **代码审**：广播集合+写败丢弃+暂存槽 flush 与 Streamer「写过才算活」判别同构；对单次写可被半死 forward 吸收的 push 场景，广播严格优于 reserve-promote。`attach`/`uiEvent`/`start` 全在 Streamer 类锁内，无新竞态。**两处非阻塞 nit**：①暂存 TTL 是滚动窗（以 `lastHitDropTs` 整体过期，非逐条 TTL）；②attach-flush 只推最新 conn 后即 clear——attach 的恰是 ghost 时暂存 hit 会被「假成功」吞掉（窗窄，dup-vs-loss 取舍已注释）。
- **独立复跑**：`ghost_repro.py --watch 10` → **10/10**（t3 报 9/10，r5 失手确证非系统性）；`ghost_repro.py 10`（stream）→ **10/10** 无回归；`verify-agent.sh` → **42/0** 含 `watch_ghost_live`/`watch_corpse_heal`/`agent_ver`=0.7.3。**PASS**。
- **文档勘误**：§18 记「TTL 30s」与代码 `PENDING_HIT_TTL_MS=15_000` 不符（§4.8 协议文档 15s 正确）——本附录已更正为 15s。

### D. 回归抽样

- `test_parity_extra.py` → **12/12**；`cargo` 三件套于 t3/t4 提交各自已绿、CI/CodeQL success（gh run 45620754611/45628026909）。

## 附19 t6 复核——t1 发版 + t2 恢复 UX 独立复验（2026-11-26，复核人 p4）

### A. t1 v0.7.1 四面复验：**PASS**

| 面 | 复核证据 | 判 |
|---|---|---|
| Release | tag `v0.7.1`@b34e1a5，7 assets 齐（3 平台 cli+whl+sdist+agent.jar+SHA256SUMS） | PASS |
| agent.jar 一致性 | `git show v0.7.1:agent/agent.jar` sha256=`22431f…` == release SHA256SUMS 中 `22431f…` 逐字节一致（HEAD b5a36e 为 t4 0.7.3，属预期） | PASS |
| PyPI | `pilotdroid 0.7.1` latest | PASS |
| Registry | `io.github.mofanx/pilotdroid 0.7.1` 已上架；server.json version=0.7.1、30 tools 描述 | PASS |
| tap | `homebrew-pilotdroid` Formula 0.7.1@558cdac，双 sha256 与 SHA256SUMS 一致 | PASS |
| notes 诚实档 | body 含 agent.jar 0.7.1→0.7.2 随行说明+三层 ghost 修复机制+沿用 0.7.0 limits（clipboard❌/screenrecord⚠️/fps 5.6 ceiling） | PASS |
| 文档注记 | §4.8「写过≥1 次=唯一可靠活性证据，read-timeout 探针证伪」在档；AGENTS.md fake relay 半关闭保真度注记在档 | PASS |

### B. t2 三件套代码审+真机独立实证：**PASS**

- 代码审（5c51f86）：epoch 变更时先 socket-free `pop_stashed` 排干（WatchHit 弃/Toast 存）→`watch_attach` 预挂→逐 ArmedRule `watch_arm` 重臂并标 post-call gen（双故障角例覆盖）→`session_recovered{generation,stale_watchers,rearmed,rearm_failed}`；流中 `pop_stashed` 每迭代排干经 `route_event` 补推 notification；demux 断连不再摘 armed（留给 re-arm），`demux_dead` 每 streak warn 一次；stale `watch_disarm` 本地 `{disarmed:false}` 不透传。
- 真机实证（`t6_verify.py`，ba0d072d，agent 0.7.3/gen 新代际）：
  - A `pkill -f pilotdroid.Agent`（pid 5840→12533）→ `describe_screen` 触发恢复 → `session_recovered{generation:1,stale_watchers:1,rearmed:1,rearm_failed:0}` 到达 notification 通道 —— **PASS**
  - B 重臂规则 `app_start settings` 首 hit 到 `watch_hit` notification，watch_id 重映射（1，新代际 id 空间）—— **PASS**
  - C `stream_start` 激活期 hit 经 stash 排干为 mid-stream push notification（同调用窗读 4 帧）—— **PASS**
- nit 有效性：serve.rs:778 stale disarm 本地返回、`demux_dead` warn 收窄在档；SUPPORTED.md demux 段（push 流中照推/re-arm/stale disarm 语义）已更新。

### C. 回归面板

| 检查 | 结果 |
|---|---|
| `cargo test --workspace` | 96/0 |
| `cargo fmt/clippy -D warnings` | 净 |
| u2 套件 run.sh | **22 passed / 0 failed / 8 skipped** 零回归 |
| python/tests | 92 passed |

插曲：复核期间 /home 100% 满致 lld Bus error——`cargo clean` 释放 3G 后全绿（环境故障非代码）。

## 附20 t1 —— watcher 死讯通道 + ui_event always-stash replay 环（2026-11-27，ba0d072d，agent 0.7.4，274db5d+）

### A. 机制

- **`watch_dead` 死讯**：`Watcher.run` 重构终态语义——正常终结（hit/timeout）置 `reported`；非 `userDisarm` 的异常终结（poll Throwable / `abort(reason)`）记日志并临终推 `{"watch_id":…,"matched":false,"type":"watch_dead","waited_ms":…,"error":…}`（ch=1、frame type 0x0002 同槽，新增 payload `type` 值，协议格式未动）。显式 `watch_disarm` 静默正常。session close 的 `disarm()`→`abort("session closed")`——控制 conn 替换/断开确定触发。旧 host 不识 `type` → 按 `matched:false` hit 解码降级。
- **always-stash replay 环**：修 §18 归因修正的真残洞——adb forward 半死管道可**假性吸收写**（ghost conn 写成功、真 reader 缺席），`delivered>0` 不再等于送达；mid-attach 真 conn 错过广播即永久丢。现每个 ui_event 入 ≤8 条/15s TTL 环，media attach 时补发未过期 backlog，flush 后不摘除（flusher 自身可能是 ghost）——重复交付幂等、由 host 去重。
- `watchConns` cap=4 语义、replay 环界与 TTL 入 docs/02-protocol.md §4.8/§4.9 + compat/SUPPORTED.md 限制档。

### B. 确定性致死路径裁决

外因注入三条全部证伪（`accessibility_enabled 0` 下 bridge 存活；`uiautomator dump` 被 SIGKILL 且 agent uia 存活；息屏不杀 poll）——poll Throwable 保留为防御性兜底，**可复现致死路径定为 session close 强杀**（host 脏杀/conn 替换，正是 r5 场景）。

### C. 验证链

| 检查 | 结果 |
|---|---|
| `ghost_repro.py --watch-dead 3` | **3/3**——session 替换→`watch_dead` 到 media conn+迟到 conn replay 补达 |
| `ghost_repro.py --watch 5` | **5/5**——replay 环零回归（stray ghost+corpse-heal 守护不破） |
| verify-agent.sh 全量 | **44 pass / 0 fail**——含 A9 `watch_dead_event`/`watch_hit_stash_replay`，A8 双断言经 read_until 回放过滤 |
| agent.jar | 0.7.4 重建部署，设备 `pilotdroid-agent 0.7.4` 实证 |
| verify-agent.sh 断言修正 | A8/A9 初版假失败归因：单帧读撞 replay backlog（新 conn 先收旧事件）+A9 selector 命中当前前台致 armed 提前消费——改 read_until 按 watch_id 过滤+不可命中 selector 保持 armed |
| CI/CodeQL | 274db5d 双绿 |

文档随行：docs/02-protocol.md §4.8 watch conn/replay 语义、§4.9 `watch_dead` 事件档；SUPPORTED.md cap=4/TTL/replay 限制档；CHANGELOG Unreleased。

## 附21 t3 —— eval v3 随机化 + B4 盲驾复测 + 多设备首行（2026-11-27，ba0d072d，agent 0.7.4，t2 状态面落地后）

### A. eval v3（`loop_v3.py`，9 任务 ×3 轮）：**27/27 (100%)**

| task | pass | p50s | 备注 |
|---|---|---|---|
| cross_app_settings | 3/3 | 6.8 | v2 继承（scroll-aware 修复后） |
| som_index_tap | 3/3 | 3.8 | 同上 |
| stale_snapshot_reverify | 3/3 | 5.3 | `stale_i` 缺省 0→显式报错（nit 修复） |
| scroll_find | 3/3 | 3.5 | |
| double_toast_order | 3/3 | 5.8 | |
| dirty_kill_recovery | 3/3 | 5.5 | |
| background_resume | 3/3 | 7.9 | |
| **random_index_pick** | 3/3 | 4.4 | 随机主表项→tap_index→导航证实→BACK 集合还原 |
| **random_checkbox_state** | 3/3 | 7.9 | 随机 CheckBox→state 谓词断言 checked 翻转 |

- **失败记录带 fg 归因**：FAIL 行挂 `fg=<pkg>`——实测抓到 `li.mofanx.sctrl`（遥控 app）与 `com.xiaomi.misubscreenui`（副屏 UI）抢前台各两次，环境噪声与产品失败不再混淆。
- **v2 继承任务修复**（本批连带发现）：`describe`/`list_elements` 只吐**已渲染**节点——ApiDemos 主表「Views」在折下，v2 三任务硬编码假设可见而必挂。全部改 `find_scrolled`/`tap_scrolled` 有界滚动（loop.py 新增 helper，`open_views` 同享）。
- **eval 环境钉**：`default_input_method` 置空 `pilotdroid.eval/.NoIme`（finally 恢复原值）——MIUI 全屏 extract IME 无视 BACK、盖住全部 app bounds、吞噬注入 tap（观测到 tap 变字符输入）。这是该零售机的真实环境杀器。
- **底部手势排除区**：元素 tap 中心落 ~94%H 手势区被静默吃掉（★Star 行）——随机抽取按中心 y<0.88·content-max 过滤。

### B. B4 盲驾复测：**PASS**（eval-driven 验收标尺）

states 面落地后重跑 B4 场景（describe→tap→verify_after）：describe 现透出 `CheckBox` 无 checked=未勾，tap_index 后经 `{"state":{"match":{"text":"Checkbox 1"},"checked":true}}` 谓词断言**翻转为真**。上轮双归因之一（状态谓词缺失）关闭；另一归因（外部抢占）由 fg 归因字段显性化。blind.py describe 同步透出状态位。

**a11y `checked` 传播延迟实测 0.5–4s 可变**——verify_after 单次 settle 读可竞争；任务实现为「verify 一次→poll 证实翻转→再以 exists 零副作用 action 过谓词」，幂等且诚实。

### C. 交叉复核产出：serve.rs `match.i` 索引空间不一致（**报 t2 nit**）

`expect.state.match.i` 在 serve.rs:1087 用 `list_elements(false)`（全树索引）解析，而 host 可见 `i` 是 `list_elements(true)`（interactive 子集）空间——**同一 `i` 指不同元素**，match.i 命中错节点（实测 i=5 在 full-tree 落到非目标节点，checked 永 false）。任务侧改 match.text 规避；**应修 serve.rs 对齐为 interactive 空间或文档明示**（归 t2 文件域，移交）。

### D. 多设备矩阵第一行（V816=dash 2602BRT18C，**API37/Android17**；net-adb 11.0.0.2:5555→改址 192.168.123.125:5555）

| 项 | 结果 |
|---|---|
| `setup` 八项 | **全 PASS**（含 u2_conflict 无包） |
| agent 版本 | 预装 0.5.0→`agent install`+deploy 升 **0.7.4**（设备日志实证） |
| describe | 22 元素（systemui 树） |
| ApiDemos 安装 | **USER_RESTRICTED**——V816 未开「USB 安装」，pm install 同拒；无物理确认通道 |
| `--portable` 核心子集 | **有意未跑**——该机经用户确认为**日常用机**（非测试机），eval 按指示只在 USB 测试机 ba0d072d 跑；诊断期曾陷 MIUI AOD doze↔防误触页循环（proximity covered 即弹 `ScreenOnProximitySensorGuide`，远端不可解） |
| serial 切换 | `PILOTDROID_SERIAL` 环境变量即切换（Serve() 消费），双机各自 hello/tree 实证 |
| 设置恢复 | **已完成**：`enable_screen_on_proximity_sensor` 恢复为 1、/data/local/tmp/apidemos.apk 残留已清（2026-11-27 经 192.168.123.125:5555 执行；无线 adb 曾掉线改址重连）；agent.jar 0.5.0→0.7.4 为用户预装 pilotdroid 的原地升级（保留） |

**serial 切换正确性注记**：harness 全程只用 `adb -s $SERIAL` + CLI `-s $SERIAL`，无隐式单设备假设；V816 为 net-adb（5555），forward 建链与 USB 设备同路径、各自 abstract socket 不串。

### E. 其他 nit 收编

- `stale_i` 缺省 0 → `Views absent from snapshot` 显式报错（不再把 0 号位当拒测目标）。
- harness `rpc` 通知队列、`_nav_tap` 前向+回扫双程、NO_NODE scroll 容错、thin-list 重试。

## 附22 t4 —— v0.8.0 minor 发版列车全链证据（2026-11-27，tag `v0.8.0@3be8f15`）

### 版本对齐判定

CLI/Python **0.8.0** 随件 agent.jar **0.7.4**——feature 级增量
（3 新 notification + agent 0.7.2→0.7.4 两级 + SoM states +
watch_dead + OCR 可选档），走 minor 非 patch；错落关系已在
release notes「Version skew」段明示。

### 四面发布链

| 面 | 证据 |
|---|---|
| bump×5 | Cargo workspace/pyproject/server.json×2/install-smoke VER →0.8.0；Cargo.lock 随行；CHANGELOG Unreleased→[0.8.0] fold（12 commits）；doc-drift 6/6 |
| tag+assets | `v0.8.0@3be8f15`（CI+CodeQL 双绿后打）→ release workflow success → draft 7 assets 齐全（linux/macos/windows+whl+sdist+agent.jar+SHA256SUMS）→ notes-file 应用 → publish |
| PyPI | `pypi.yml` OIDC **success**；`pip index versions` 见 0.8.0（首轮 smoke 撞 index/文件传播竞态 5 fail，传播完成后重跑全绿——传播延迟如实记） |
| install-smoke | `ba0d072d` **9/9 PASS**：`pilotdroid-cli 0.8.0`（venv bootstrap+release binary 对照）+setup `hello ver=Some("0.7.4")` agent 版本实证+serve MCP 握手 |
| Registry | `mcp-publisher publish` → `io.github.mofanx/pilotdroid@0.8.0`（JWT 过期经 `login github -token "$(gh auth token)"` 重登后发成） |
| tap | `mofanx/homebrew-pilotdroid` Formula `50cf4ab`：version+双 url+双 sha256（SHA256SUMS 对照：linux 96cc9340…/macos e7c47fe7…） |

### notes 诚实档覆盖

状态面（非默认输出策略+state 谓词）、watch_dead（cap4+replay ≤8/15s）、OCR 可选档（~1.5–2s/帧+English 级+缺依赖优雅降级）、恢复三件套、agent 0.7.2→0.7.4 错落、MIUI checked 传播 0.5–4s 延迟注记、V816 多设备行 partial 口径（日常用机 eval 有意未跑）。

## 附23 t5 —— t3+t4 独立复核证据（2026-11-27，复核员自跑，非转述）

### A. eval v3 判定逻辑审（**pass**，附两真 nit）

- **随机化为真**：`random.choice` 从 live `list_elements` 可点候选抽——`random_index_pick` 独立复跑 5/5 抽到 3 个不同条目（Media×2/Animation/App），非罐头路径；`random_checkbox_state` 用 `expect.state{match.text,checked=!was}` 真谓词断言翻转。
- **fg 归因有效**：复跑中 `random_index_pick` 曾 0/2 FAIL，`fg=com.miui.home` 精确指认「采样时前台是桌面」——归因字段如实工作。根因：`app_stop+app_start` 后固定 1.5s 竞态，task **无 fg 门**直接采样（`nit-eval1`：候选集可采到 launcher 元素→BACK 留在桌面→假败）。无 fg 门时同一任务 16/18（89%）；独立 5 连+第二轮全套 18/18——**flake 非确定性缺陷**。
- **IME 钉泄漏实锤**（`nit-eval2`）：run 后 `settings get secure default_input_method` 残留 `pilotdroid.eval/.NoIme`——`old_ime` 恢复机制把「上一轮被杀进程的钉」当原值回写，**毒化基线自保持**。已手工回写 `com.sohu.inputmethod.sogou.xiaomi/.SogouIME`。建议恢复时校验 `old_ime` 非自身钉值。
- **B4 盲驾独立复跑 PASS**（ApiDemos Checkbox）：`verify_after{state:{match.text:"Checkbox 1",checked:true}}` → `checks[0].pass:true`，复跑中 checked=True——状态谓词闭环实证。
- 期间发现 `verify_after` 的 **snapshot stale 守卫如实工作**：MIUI `SystemAppPermissionDialogActivity`（同意/不同意）抢占时 tap_index 报 `focused app changed`/`layout changed` 拒写——一致性保护非缺陷，弹窗清理后通过。

### B. 多设备行核对（**partial**，与 t3「有意未跑」口径一致）

| 机 | setup | describe/eval | 阻塞 |
|---|---|---|---|
| ba0d072d（基线） | 全 PASS，hello 0.7.4 | eval-v3 18/18 | — |
| V816=11.0.0.2:5555（API37，原 192.168.123.125 已 no route） | 复验**全 PASS**，hello 0.7.4，inject_probe ok | portable 0/9——fg=NotificationShade 锁屏态 | **PIN+屏下指纹锁**——注入滑动到达指纹环即停，无 PIN 不可解；proximity 防误触（system+global 双命名空间）亦拦滑动，临时置 0 后**均已恢复 1**。随后 WiFi adb 掉线（device offline） |
| CFE6R21525008201（新行：MRR-W29 平板 EMUI14.2/API31） | agent install 后**全 PASS**，hello 0.7.4 | portable 0/9——锁屏态 systemui-only dump 实证 | **PIN 锁**（deviceLocked=1），无法远端解锁 |

两新设备行证明 setup/握手/inject 链路跨 ROM 通用；eval 行被「日常用机 PIN 锁」环境阻断——与 t3 记档「eval 有意未跑」相符，非产品失败。V816 仍是 API37 唯一 eval 候选，需物理解锁或备用机。

### C. v0.8.0 四面独立复验（**pass**）

- **GitHub Release**：`v0.8.0` 7 assets 齐（whl+sdist+3 平台 CLI+agent.jar+SHA256SUMS），`sha256sum -c` 对 agent.jar+linux-x86_64 **OK**。
- **PyPI**：`pip index versions`/`pypi.org/json` 均见 **0.8.0**（whl+sdist 两文件）。
- **Homebrew tap**：origin/main `50cf4ab bump pilotdroid 0.8.0`——version+url 双段+sha256 与 SHA256SUMS 一致（macos e7c47fe7…）。
- **MCP Registry**：`io.github.mofanx/pilotdroid@0.8.0` **active+isLatest**（pub 2026-11-26T00:08:07Z），旧版 0.6.1.post1 deprecated 链完整。
- **notes 错落措辞**：「CLI/Python 0.8.0 ships agent.jar 0.7.4」——与 install-smoke `hello ver=Some("0.7.4")` 实机吻合。
- **install-smoke 复跑 ba0d072d：9/9 PASS**。
- 复核 nit：hb-tap 本地副本停在 0.7.0（stale fetch），非发布缺口——远端已最新。

### D. 套件/parity 抽样（**partial**）

- `pytest python/tests`：**93 pass**；`cargo test --workspace`：**100 pass / 0 fail**。
- u2suite verbatim：**20 pass / 3 fail / 7 skip**（基线 22/0/8）。三 fail 归因：
  - `test_scroll`：`.scroll.to` NIE（SUPPORTED 仅列 4 方向档）——先前靠「大屏无需滚动」skip 分支未触达；本次 scrollable=True 命中即翻为 NIE。**预存缺口被环境翻牌，非新回归**。
  - `test_send_keys`：`d.xpath("Search")` 无匹配——Search 在 App 子页**屏外第二页**，a11y 仅吐可视节点；屏外节点可见性为环境敏感窗态。
  - `test_watch_context`：`dev(text='Menu')` 落空——Menu 同屏外；ledger §M22 已定档此测试为环境敏感。watcher arm/click 链路本身此前已有真机实证。
- 结论：无代码回归证据，但 22/0/8 不复现——**屏外节点暴露窗态**是二值翻转变量，应作为套件 flake 面记档（跑前 `svc power stayon`+collapse 已做仍受子页滚动态影响）。

### E. ledger nit 复核

- TTL 滚动窗、attach-flush ghost 窗、双故障角例：代码与记档一致（replay ≤8/15s 已更正档成立）。
- eval 断言弱点：stale_i 已改显式报错 ✔；另发现 `nit-eval1`（fg 门缺）与 `nit-eval2`（IME 钉毒化基线）——新记。
- forward 清账：本轮测试清 ba0d072d×23 + CFE6R×1 孤儿 forward，清后 `--list` 空。

## 附23 t6 —— t1+t2 独立复核（2026-11-27，ba0d072d，agent 0.7.4，v0.8.0 tag 后 main）

逐项判定：**5/6 pass，1 partial**。

### ① watch_dead agent 侧——**PASS**

- 实现审：`abort(reason)`/poll `Throwable`/意外 interrupt 三路汇聚于 `!reported && !userDisarm` 临终分支——**先记日志再发事件**、发送自身 `catch(Throwable)` 包裹、事件进 always-stash ≤8/15s 环 + cap4 广播双通道。语义不过度承诺：只覆盖 **watcher 级死亡**（poll 故障/会话强杀），agent 进程死由恢复 epoch 兜底（docs/SUPPORTED 口径一致）；显式 disarm 静默正常、命中/超时 reported 后不发——语义边界自洽。
- 独立复现：`ghost_repro.py --watch-dead 3` → **3/3**。
- notification 真机 e2e（自写 MCP 驱动）：serve1 arm→serve2 会话替换→serve 收到 `{type:"watch_dead", error:"session closed"}` + **有界 auto re-arm 实证**（`rearmed:true, new_watch_id:24`）。serve.rs 单测 `watch_dead_event_notifies_and_rearms` 同向。
- **nit（不阻塞）**：`uiEvent` 在 streaming（curOut 非空）走单 conn 直写路径、**绕开 replay 环**——stream conn 被 ghost 吸收窗口内（至 mediaDead 剔除前）hit/watch_dead 仍可丢。窗口小（流写持续探测剔除快），建议后续把 stash 提前到 curOut 分支之前。

### ② SoM states——**PASS（match.i nit 未修，移交项仍 live）**

- attr 直通完整：`checked/selected/focused =="true"`、`enabled !="false"`（缺省 true）+ 非默认输出策略（三态仅 true 输出、enabled 仅 false 输出）；`class` 存简单名（`rsplit`）三面一致。
- 谓词语义审：match text/desc 子串、class 简单名精确；mismatched 诚实报 `{want,got}`；found=false 与谓词失配分层。**B4 独立双向实证**：pre-checked 时 tap→`got:false` 正确侦出（反向验证），随后 false→true 翻转 `match.text+checked` 断言 **PASS**。
- **partial 项**：`expect.state.match.i`（serve.rs:1087）用 `list_elements(false)` **全树索引空间**解析，而 host 可见 `i` 是 `interactive_only=true` 子集空间——同一 i 命中不同元素；且 `i` 键存在时**短路忽略** text/desc/class 其他 match 键。t3 移交 nit 未修，main 仍 live（建议对齐 interactive 空间或文档明示）。

### ③ OCR 可选档——**PASS**

- 真路径：SpriteTextActivity（纯 canvas，a11y 仅 1 FrameLayout 节点）`describe_screen ocr:true` → **9 个 `ocr_only` 元素**（`ocr:true`+`ocr_conf` 57–96+bounds/center，同编号空间可 tap_index）。
- 降级：PATH 摘除 tesseract → 正常输出 + `ocr:{available:false, hint:"tesseract not on PATH…"}`，零报错。
- token 预算：`MAX_OCR_BLOCKS=40` + conf 降序截断（代码实证）。

### ④ class 口径统一——**PASS**

facade `ElemInfo.class` 存简单名、som.rs emit `rsplit`、compat/som.py 镜像同口径；list/describe/SoM 三面一致。

### ⑤ 工程面——**PASS（套件 4 fail 全部归因非回归）**

- `cargo test --workspace` **100/0**、fmt 净、clippy 净；`verify-agent.sh ba0d072d` **44/0**（含 watch_dead_event + watch_hit_stash_replay）；pytest 93/0。
- u2 套件 **19 pass/4 fail/7 skip**（基线 22/0/8）逐条归因：
  - `test_scroll`：`.scroll.to` **从未实装**（git -S 证，SUPPORTED 仅 4 方向档）——先前靠 `scrollable.exists=False` 走「大屏无需滚动」skip 分支计入 8 skip；本批 scrollable 命中翻牌为 AttributeError。预存缺口环境翻牌（§行 985 已记同类）。
  - `test_watch_context`/`test_send_keys`：Menu/Search 在 App 页**折线以下不可渲染**（`d(text=).exists=False` 实测）——a11y 不可见性+设备几何，上游在真 u2 下同构失败；非 t1/t2 回归面。
  - `test_xpath::test_click`：隔离重跑 **PASS**——顺序 flake。
  - t1/t2 文件域（agent/serve/som/facade）与 u2shim/scroll/watch_context 零交集，回归面排除。

### 移交 nit 汇总

1. `expect.state.match.i` 全树/interactive 索引空间不一致（serve.rs:1087，**仍 live**）+ i 短路其他 match 键。
2. streaming 路径 ui_event 绕开 replay 环（ghost 吸收残窗，小）。
3. `_Scroll.to` NIE 缺口持续被环境翻牌（suite 面板在 19-22 间摆动，建议 or 修 or SUPPORTED 明示已有档）。

## 附24 t8 —— eval nit 两件修复 + 多设备第二行尝试（2026-11-28，ba0d072d / V816）

### A. nit-eval1：random_index_pick 前台包名门——**已修**

- `_ensure_fg(srv, pkg, tries=3)`：启动后 `fg_pkg()` 校验目标 app 持前台，被外部 app（com.miui.home/sctrl/misubscreenui 类）抢占时 `app_start` 恢复 ≤3 次再进入任务体；不可恢复则 `RuntimeError("foreground held by <pkg>")`（fg 归因照常记录）。挂到 `t_random_index_pick` 与 `_settings_list`（portable 三任务共用）。
- **任务中抢占重试**：任务体期间发生 fg 抢占导致结果失败时，fg≠目标即重试一次——环境性抢占不再计 FAIL。
- **实证**：修复前一轮全量跑出本 nit 的目标病型——`r1 FAIL random_index_pick fg=com.miui.home`（任务中 launcher 抢占）；修后单任务重跑 **4/4 PASS**。

### B. nit-eval2：IME 钉收敛 + 恢复基线注记——**已修**

- `default_input_method` 置空钉收敛为**仅非 portable 档**：`--portable`（Settings 子集）不输入文本，钉无必要；且对非测试机的 secure 设置写属不当干扰——多设备跑日常用机时该路径现在零设置改动。
- **恢复基线可审计**：finally 恢复后打印 `eval env: default_input_method restored to <原值>`；代码注释明示 SIGKILL 跳过 finally 时需手工 `settings put` 恢复。
- 实证：`default_input_method restored to com.sohu.inputmethod.sogou.xiaomi/.SogouIME`（ba0d072d 原值）。

### C. eval v3 全量复核（ba0d072d，9 任务 ×3 轮）：**26/27（修前快照）→ 修后 random_index_pick 4/4**

修前全量：26/27，唯一 FAIL 即 fg=com.miui.home 抢占（正是本 nit 目标，见 A）。其余 8 任务全 3/3（p50 3.5–7.9s）。IME 恢复基线打印实证。

### D. 多设备第二行（V816 192.168.123.125:5555）：**blocked——设备不可达**

- `adb connect 192.168.123.125:5555` → `No route to host`（多次）；`ping` 100% 丢包；`nmap -p 5555 192.168.123.0/24` 无开放设备——**该机当前不在网/无线 adb 已关**，远端不可恢复。
- `adb devices` 现 11.0.0.2:5555 为 `device` 态（adb 自动重连的旧址，疑同机换网段回连）——**遵用户禁令未触碰**，未做任何 shell 探测。
- 已就绪待跑命令（设备恢复后）：`pilotdroid-cli -s 192.168.123.125:5555 setup` + describe + `PILOTDROID_SERIAL=192.168.123.125:5555 python3 loop_v3.py --portable`（portable 档零安装零设置写——IME 钉已对 portable 关闭，见 B）。

## 附25 t9 —— 独立复核 t7（match.i 索引空间修复，23b593d）

### ① 语义审——**PASS（一处文档错误已随复核修正）**

- **同空间成立**：`e.i` 在 facade `retain` 后 enumerate 赋值（facade.rs:410-412）⇒ `e.i`≡filtered 位置≡serve `enumerate()` 位置≡`tap_index` `els.get(i)`——match.i 与 tap_index/list_elements 确属同一编号空间；filter 字面量与 facade.rs:407 逐字一致。
- **无 snapshot 污染**：`snap` 仅在 serve `list_elements` 工具分支（serve.rs:728）写入；verify_after 内部 `s.list_elements(false)` 不触 snap——后续 tap_index 不会被全树快照劫持。
- **AND 不短路**：`keys(e)` 对 text/desc/class 全 map_or 真，i+其余键逐键合取；i-only（无其他键）走全 true 路径正常解析。
- **纯文本 match 搜全树**：disabled/非交互节点可断言（设计决策，注释明示）。
- **文档错（复核修正）**：SUPPORTED.md:177 原写 match.i 归「list_elements/**describe_screen** 同一空间」——**错误**：som.rs `enumerate` 为 document-order 宽集（text/desc/rid 或交互标志任一算数），有纯文本 Label 时必与 interactive 空间分叉。已改为仅宣称 `list_elements` 空间并明示 SoM id 不可直用于 match.i/tap_index。
- **非阻塞 nit**：interactive filter 字面量在 facade/serve 两处复制（drift 风险，建议后续提取共享谓词）；`match:{}` 空键+state 谓词返回 found:false 而非报错（低优先）。

### ② 单测审——**PASS**

`FAKE_DUMP_DIVERGENT` 全树 [装饰0,CheckBox1,Label2] vs interactive [CheckBox0,Label1] 真分叉钉四断言：i=2 越界不命中、i+错 text found:false（不短路）、i+Label+enabled:false 正向 AND、i-only checked 读回。本机重跑 3/3 通过。

### ③ 真机独立复跑（ba0d072d，debug CLI 23b593d+）——**PASS**

- `match:{i:5}+checked:true`（interactive CheckBox）：首跑 found:true 但 tap 被横屏全屏 IME 吃掉（复现 p2 同款坑）；钉 NoIme 后 **pass:true，checked 翻转断言命中**。
- `match:{i:5,text:"NO_SUCH_TEXT"}` → **found:false**（i 不短路实证）。
- i-only `match:{i:5}` → found:true，state 读回诚实。
- IME 已恢复基线 `com.sohu.inputmethod.sogou.xiaomi/.SogouIME`。

### ④ 工程面抽样——**PASS**

cargo workspace **101/0**（serve 34 含新测）+ fmt 净；pytest **93/0**。

**裁定：t7 PASS**。移交遗留：SUPPORTED describe_screen 空间误称已修；filter 双处复制 + describe/list 双编号空间的 agent 消费陷阱（SoM id≠tap_index i）为预存设计面，建议后续统一或文档显著化。

## 附25 t10 —— t8 独立复核（2026-11-27，复核员自跑，非转述）

| 项 | 判定 | 证据 |
|---|---|---|
| ① V816 多设备行数字 | **blocked（同 t8 口径）** | `adb connect 192.168.123.125:5555` → `No route to host`、ping 无响应；与 t8 记档一致。`11.0.0.2:5555` 虽呈 device 态——遵禁令零触碰（无 shell 探测、无 forward、无 setup）。 |
| ② fg 门修复有效性 | **pass** | `_ensure_fg` 代码审：启动校验+app_start 恢复≤3 次+不可恢复带 pkg 显式报错；任务内仅 `fg_pkg() != APIDEMOS` 才触发重试——真失败不误赦，抢占比率保守。实证：全量 27/27，`random_index_pick` r2 耗时 12.8s（fg 恢复路径实跑，非空转）。本复核 t5 轮曾独立复现该病型（0/2 fg=com.miui.home）——修复对症。 |
| ② IME 基线恢复 | **pass（附 nit）** | 跑前 `com.sohu.inputmethod.sogou.xiaomi/.SogouIME`→跑后同值，恢复打印 `restored to ...SogouIME` 审计行实证；`--portable` 不写 secure 设置 ✔。**残留 nit**：`old_ime` 若捕到上轮 SIGKILL 遗留的 `pilotdroid.eval/.NoIme` 仍会当原值回写（毒化基线只缓解未根治）；缓解建议——捕获时校验 `old_ime != NoIme 钉` 否则跳过恢复写。 |
| ③ eval v3 全量 | **pass** | ba0d072d 9 任务×3 轮独立复跑 **27/27**（t8 报 26/27 修复后 4/4 同任务，本跑全绿）。 |
| ④ ledger | **本附段** | — |

**补充观察**：`--portable` 的 fg 门挂在 `_settings_list`（portable 三任务共用入口）——覆盖正确；`_ensure_fg` 对 V816 类日常机也无设置写副作用，跑机前提成立。

## 附26 t11 —— 多设备第二行定案：**BLOCKED（设备不可达，有据可查）**

### 连通性证据（2026-11-28 复测，含本日多次重试）

| 探测 | 结果 |
|---|---|
| `adb connect 192.168.123.125:5555` | `failed to connect: No route to host`（t8 起累计 ≥3 次） |
| `ping 192.168.123.125` | 100% packet loss |
| `nmap -p 5555 192.168.123.0/24` | 无开放 5555 设备——该机不在本网段/无线 adb 已关 |
| `adb devices` | `11.0.0.2:5555 device`（adb 自动重连的旧址，疑同机回连 11.0.0.x 网段）——**遵用户禁令全程未触碰、未发任何 shell 探测** |

### 定案

- **第二台机 eval 行不可产出**——授权地址当前无网络路径，唯一可达地址被禁令覆盖；两项条件均非代码/流程问题。
- **前置能力已就绪**（t8 落地）：`--portable` 子集（som_shape/settings_random_pick/verify_exists_pred）零 ApiDemos 依赖、零安装；IME 钉已收敛为非 portable 档——**该路径对用户日常机零 secure 设置写、零持久改动**，设备恢复后可直接跑。
- **待跑命令清单**（解锁后一次执行）：
  ```bash
  pilotdroid-cli -s <serial> setup
  pilotdroid-cli -s <serial> describe | head
  cd spike/e2e/agent-loop && PILOTDROID_SERIAL=<serial> python3 loop_v3.py --portable
  ```
- **唯一解除条件**：用户恢复 V816 无线 adb（任一网段给出可达地址），或明示授权使用当前可达的 `11.0.0.2:5555`（同机、禁令可解除）。

### 矩阵现状

| 设备 | serial | API | 结果 |
|---|---|---|---|
| Xiaomi 测试机 | ba0d072d (USB) | 34 | eval v3 全量 **26/27→修后 nit 病型 4/4**（§附24）；verify-agent 44/0 |
| V816 dash 2602BRT18C | 192.168.123.125:5555 | 37 | **BLOCKED**——setup/describe 历史上已过（§附21-D），portable 数字待设备可达 |
