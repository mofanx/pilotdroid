# u2shim 方法级兼容矩阵

`compat/u2shim.py`（`pip install ./python` → `import pilotdroid`）对照
uiautomator2 `Device` 公共 API 的逐条标注。约定：**drop-in 只承诺此表内
的项目**；表外调用按 AttributeError/ValueError 显式拒绝，不静默降级。

## Selector keys（`d(**kw)`）

| u2 key | 状态 | 备注 |
|---|---|---|
| `text` `resourceId` `description` `className` `packageName` | ✅ | AND 语义同 u2；全精确键时走 wire QUERY 快路径 |
| `scrollable` | ✅ | wire 原生字段 |
| `textContains` `textStartsWith` `textMatches` `descriptionContains` `descriptionStartsWith` `descriptionMatches` `classNameMatches` `packageNameMatches` `resourceIdMatches` | ✅ | M12 host post-filter：dump 全树→Python 端谓词（regex 用 fullmatch 语义，同 u2 `*Matches`） |
| `index` `instance` | ✅ | M13 语义对齐 u2：**`index`** = 节点在父中的子位置（XML `index` attr，host post-filter 谓词）；**`instance`** = 第 N 个命中（0-based；wire `index` 字段语义即此，shim 映射 instance→wire index）。注意 M11-12 曾把两者混为 Nth-match——旧脚本的 `d(index=N)` 语义会变 |
| `clickable` `checkable` `checked` `enabled` `focusable` `focused` `selected` `longClickable` | ✅ | M12 host post-filter attrs |
| relative selector（`child`/`sibling`/`left`/`right`/`up`/`down`） | ✅ | M12 host post-filter：空间向=节点 bounds 完全落在锚点方向区；child/sibling 用 dump 树父子关系 |
| `xpath` | ⚠️ **host-side 近似子集** | M23 xpath-lite：每次查询一次 `dump_hierarchy`+ElementTree（~50ms），**非 u2 插件管线**。覆盖：text 简写 / `%` SQL-LIKE 三态（`v%`=前缀、`%v`=后缀、`%v%`=contains，M24 由一律 contains 收紧为 u2 语义）/ `@resource-id` / 裸 `pkg:id/x` / `//*[@attr]` / `contains()` / `parent()` / `scroll_to` / `all()`/`get()`/`exists`/`wait()`/`click()`/`get_text()`/`info`/`swipe`。位置轴/`and-or` 谓词/ancestor 轴等高级面 → 显式 `NotImplementedError`。watcher `when()` 仅接 `//*[@text|@resource-id|@content-desc|@class|@package=...]` 1:1 wire 翻译，复杂 xpath 规则 NIE |

> 解析路径注（M12）：全精确键走 wire QUERY（仅 active window）；含
> contains/matches/attrs/relative 时整体切 dump+post-filter（全窗口树，
> 更贴近 u2——u2 selector 同样能看到非 active 窗口节点）。两条路径对
> 同一 selector 可能给出不同命中集（如通知栏文本），属口径差异非 bug。

## Selector 对象方法（`d(...).X`）

| u2 API | 状态 | 语义差 |
|---|---|---|
| `.exists` | ✅ | property，同 u2 |
| `.click()` `.long_click(d)` `.set_text(s)` | ✅ | `.set_text` = 先点击再注入文本，同 u2 |
| `.info` | ✅ | property，同 u2；返回字段为子集（text/id/desc/bounds/class/clickable/enabled/scrollable/pkg） |
| `.wait(timeout)` | ✅ | M8 修正为 u2 语义：返回 bool |
| `.scroll.<dir>` | ✅ | forward/backward/toEnd/toBeginning（shim 额外需要 `scrollable=True` 选择器先命中） |
| `.pinch_in(pct)` `.pinch_out(pct)` | ✅ | M10 多指针 INJECT（CAP_MULTI_PTR），作用于命中节点 bounds（clamp 到 display）；⚠️ MIUI 鬼 bounds（a11y 报屏外坐标，如 [268,3312]@3200 高）→ LookupError 而非静默注入屏边 |
| `.gesture(s1,e1,s2,e2)` | ✅ | 任意双点轨迹可用 `points[].pid` 直接表达 |
| `.drag_to(x,y)` `.swipe` `.fling` | ✅ | 单指针 points 免费项（此前误标 ❌） |
| `.wait_gone(timeout)` `.count` | ✅ | M12 host post-filter：真计数/消失轮询 |
| `len(d(sel))` `for el in d(sel)` `d(sel)[i]` | ✅ | M24：`__len__`=count、`__iter__` 惰性枚举命中 info 字典（一次 dump）、`__getitem__`=第 i 个命中新 Selector（`instance=i`，0-based） |
| `.child` `.sibling` `.left` `.right` `.up` `.down` | ✅ | M12 relative selector（见 selector keys 行） |

## Device 顶层方法

| u2 API | 状态 | 语义差 |
|---|---|---|
| `connect(addr)` | ✅ | 位置参数兼容两种口径：adb serial 字符串（u2 风格 `u2.connect('ba0d072d')`）或本地 TCP port（int/digit-str，pilotdroid 风格）；`serial=` kwarg 与无参 auto-detect（扫 `adb forward --list` 中 `localabstract:pilotdroid_agent`）不变（M13） |
| `service("uiautomator")` | ✅ 映射 | u2 的设备端服务生命周期（`.running()/.stop()/.start()`）映射到 pilotdroid agent 进程——同角色不同实现；其他服务名报 ValueError（M13，Themis 全量脚本 teardown 依赖） |
| `click(x,y)` `long_click(x,y,d)` `swipe` `press(key)` | ✅ | key 别名：home/back/recent(s)/power/volume_±/menu/search/enter/del |
| `swipe_ext(dir,scale)` `double_click(x,y,d)` `drag(fx1,fy1,fx2,fy2,d)` | ✅ | M13 drop-in 补齐（Themis 实测用法驱动）：swipe_ext=居中定向扫屏；drag 坐标 ≤1 按屏幕比例，含 300ms 按住起点（M13） |
| `set_orientation('l'|'n'|'r'|'u')` | ✅ | `orientation` setter 别名（M13） |
| `d(...).center()` `d(...).gesture(p1,p2,p3,p4,steps)` | ✅ | center=命中节点中心坐标；gesture=节点内双指针手势（CAP_MULTI_PTR，M13） |
| `set_text(s)` | ✅ | 作用于已聚焦编辑框（u2 同语义）；**原生支持 Unicode**，u2 需 ADBKeyboard IME |
| `screenshot(path)` | ✅ | shim 额外 `scale`/`quality` 参数；返回 JPEG bytes |
| `dump_hierarchy()` | ✅ | 返回 XML 字符串（agent 原生生成，节点属性集与 uiautomator dump 近似） |
| `info` | ⚠️ 子集 | 仅 displayWidth/displayHeight/sdkInt/agent |
| `app_start(pkg)` `app_stop(pkg)` `app_current()` | ✅ | app_start 用 `am start`（u2 用 monkey）；`app_current` 返回 `{package,activity}`——M11 起有 CAP_APP_INFO 时走 conn op（~ms），`activity` 为 `null`（a11y 不暴露，**设计边界**，勿依赖该键）；否则回退 dumpsys |
| `shell(cmd)` | ✅ | 返回 `ShellResponse(output, exit_code)`，同 u2 |
| `clear_text()` | ✅ | 聚焦编辑框清空（ACTION_SET_TEXT 空串，免 IME） |
| `app_list()` `app_info(pkg)` | ✅ | `pm list`/`dumpsys package`；app_info 为 versionName/Code 子集 |
| `window_size()` | ✅ | `wm size` → `(w,h)` |
| `screen_on()` `unlock()` | ✅ | `dumpsys power` 判态 + power keyevent；unlock=wake+上滑 |
| `screen_off()` | ⚠️ | **MIUI 口袋模式陷阱**：power keyevent 关屏若贴近距离传感器，MIUI 可进入 pocket-mode → 此后全部注入输入被吞、屏幕不可点亮，**仅 reboot 可解**。建议仅在屏幕可受控测试场景使用 |
| `orientation` get/set | ✅ | `settings user_rotation`；set 冻结重力旋转（同 u2 语义） |
| `watcher` | ✅ | `d.watcher.when(...).click()` + `.start()/.run()/.stop()/.reset()/.running`——M11 起 agent 0.7.0+（CAP_WATCH_ASYNC）走**设备端异步 watcher**（WATCH_ARM + media 通道 WATCH_HIT 推送，与 u2 server 线程语义对等且 armed 期间不阻塞 control）；旧 agent 回退阻塞 WATCH，再旧回退 host 轮询。规则命中一次即摘除 |
| `watch_context(builtin)` | ✅ 近似 | ctx-mgr 语义同 u2；`builtin=True` 预挂 `_BUILTIN_DIALOGS`（非 u2 stock 规则复制，是本机实测击中的 ROM 弹窗集）：MIUI AdbInstallActivity「继续安装」、LBE 审查「确定」、展开 NotificationShade→`cmd statusbar collapse`。**点击类规则走 host-side inject 坐标点击**（device-arm 仅做观察）——agent 的 a11y ACTION_CLICK 会被 FLAG_SECURE 弹窗静默吞掉（M24 真机实证：规则命中但 10s 倒计时仍走完自动拒绝）；shade 规则经 resourceIdMatches→host-poll，ROM 无此 id 则规则静默休眠 |
| `app_wait(pkg,timeout,front)` | ✅ | `pidof`+conn APP_INFO 判前台（M11） |
| `session(pkg,attach)` | ✅ 近似 | 返回 Session 对象：`running()` 前台轮询、app 消亡后 ops 抛 `SessionBrokenError`、`with` 退出 force-stop（M21-G8）。**崩溃看护是 op 时惰性轮询，非 u2 server 常驻看护**——崩溃在下一次操作时暴露而非即时 |
| `send_keys(text,clear)` | ✅ | TEXT op（ACTION_SET_TEXT 于 focused 节点，M11） |
| `open_notification()` `open_quick_settings()` | ✅ | `cmd statusbar expand-*`（M11） |
| `freeze_rotation(bool)` | ✅ 近似 | `settings accelerometer_rotation` 开关（M11） |
| `clipboard` `toast` | ❌ 未实现 | clipboard 在 API34 service-call 签名漂移不可靠；toast 需注入 overlay |

## 实测数据

- `spike/e2e/m6-compat.json`：24-op 对拍 **23 ok / 1 unsupported（xpath）/ 0 error**（M6 历史值，对拍不再重跑；M23 xpath-lite 起 xpath 已转 host-side 近似子集，见上行）。
- `spike/e2e/RESULTS-M7.md`：同脚本端到端双跑 wall-clock（shim vs 真 u2）。

## Selector 环境敏感性（Themis 14 脚本对拍实证，M13/M14）

`text`/`description` 选择器跟随**系统 locale**——zh-CN 设备上应用自身
字符串即本地化（"More options"→"更多选项"、"OK"→"确定"、"SHUFFLE
ALL"→"随机播放所有曲目"，均 dump 直证）。en 脚本在 zh 设备 miss 属
预期行为，真 u2 同设备同 miss，非 shim 差异。

`resourceId` 选择器**不**随 locale 变化——rid miss 的根因是流程态
（首启 onboarding/许可证页挡住目标界面）或应用版本漂移，与语言无关。

**MIUI 应用锁（security gate）是第三类环境因子**：被加锁的 app 每次
启动先弹数字键盘/图案门——永久挡住 main UI（非首启态，重跑不会自己
消失）。benchmark/verify 跑含此类 app 的脚本前需在系统设置解锁，或
把被锁 app 移出评测名单（Markor 实测踩到：脚本等 "More options" 但
屏幕停在应用锁 PIN 盘）。

**第四类环境因子——MIUI 权限复核屏（com.lbe.security.miui）**：
targetSdk<23 的 app（如 ApiDemos targetSdk19）首启被
ReviewPermissionsActivity 拦截，`app_current` 停在 lbe 包，目标 UI
上一切 selector 全 miss；一次性手动「确定」放行后正常。
benchmark/suite 跑旧 targetSdk app 前需先放行（openatx u2 官方
套件实跑实证；API34 还需 `pm install --bypass-low-target-sdk-block`
才能装上此类 apk）。

## u2 API 面缺口（openatx 官方套件 M17 对拍，详见 RESULTS-M17-u2suite.md）

`mobile_tests/` verbatim 对拍：Tier-A 0 pass/27（fixture `d.settings`
单点阻断）；Tier-B 1 pass / 18 fail——如实分桶 **7 API 面缺 /
10 xpath（已知 ❌）/ 1 swipe 阻塞语义分歧**，G1–G11 明细见 ledger §5。

- **defer（明示不复制 u2 内部对象模型）**：G6 `jsonrpc` 等 u2 内部面
  ——`d.jsonrpc` 现为 agent client 透传（可访问 shim 底层 ops，非 u2
  jsonrpc 对象模型）；`d.xpath` 现为 host-side 近似子集（xpath-lite，
  见上行表格——非 u2 插件管线，高级轴 NIE）。
- **M21 已实装（近似注记）**：G8 session 对象模型（`Session`/
  `running()`/`with` 退出 stop/`SessionBrokenError`——惰性轮询非
  常驻看护）；G10 settings 全语义（已知键类型校验——
  `xpath_debug` bool/`wait_timeout` 数值/`operation_delay` (before,
  after) tuple/`operation_delay_methods` 列表；`wait_timeout` 联动
  `wait()`/`wait_gone()`/`click` 隐式等待 + `d.wait_timeout`/
  `implicitly_wait()`；`operation_delay` 联动 click/swipe/press）。
  未知 settings 键仍透明放行（shim fixture 管道——宽于 u2 的
  NameError 口径）。
- **G4 swipe**：agent 端异步手势是 M10 架构决策，设备端 duration
  仍生效但调用不阻塞 u2 客户端口径——不改 agent 阻塞。
- **M18 已实装**（shim 层补齐批，verbatim 8/30 pass）：`get_text`、
  `watch_context`、`push`、`pull`、`app_list_running`、
  `app_auto_grant_permissions`（及 `settings`、`wlan_ip`、
  `app_current.pid`、`app_start` front-wait/`stop=`、`click` 隐式等待、
  `swipe` host-sleep、`watch_context` ctx-mgr）。
- **M21 方法面**（doc-drift 注册）：`session()`→`Session` 对象
  （`running`/`close`/`__enter__`/`__exit__`/委托 `__getattr__`）、
  `implicitly_wait`（M24 起无参 getter/有参 setter 双态）、`wait_timeout`、`jsonrpc`（透传）、`xpath`（`_XPathLite`，M23）、
  `_Settings` 校验字典。
- 其余 shim 层补齐进度随 0.6.2 changelog / ledger 更新。
