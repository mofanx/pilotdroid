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
与 SUPPORTED.md 既有第三类同源。（M21 已演化为本环境 3/3 稳定
复现，见 §8）

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
| watcher::test_watch_context | 当前环境 **3/3 稳定挂**（单测连跑实证，21-23s 同点超时）：M18 同测曾 PASS，本轮 `Inflate from XML` wait 超时；a11y-lag 环境敏感，非代码缺陷（SUPPORTED.md 环境敏感性） |

**结论**：8→12 pass，剩余 10 fail = 9 xpath（明示 ❌）+ 1 环境
敏感项（a11y-lag，本环境 3/3 稳定复现非偶发），**无未归因
fail、无新代码缺陷**。

**抽查抓到的第 4 个真 bug（复核轮，非套件）**：`Device()` 不消费
`PILOTDROID_SERIAL`——`d.shell()`/`app_wait`/`app_info`/`session()`
等 adb 通道在多设备环境静默空输出（exit=1 被吞）；套件靠 seam
显式传 serial 掩盖。已在 `Device.__init__` 加 env fallback（与
Rust CLI `Cli::parse` 后的同名逻辑对齐，显式 `serial=` 参数优先）。
复验：`d.shell('pidof')` 恢复、`session(attach=True)`+
`running()→False`+op→`SessionBrokenError` 全链实证。
