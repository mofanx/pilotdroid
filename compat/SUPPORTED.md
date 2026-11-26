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
| `xpath` | ✅ **常见面无星号平齐**（host-side 求值，非 u2 插件管线） | M23 xpath-lite→Track B 全量升级：每次查询一次 `dump_hierarchy`+自研求值器 `compat/xpath_eval.py`（~50ms，stdlib-only）。覆盖：text 简写 / `%` SQL-LIKE 三态 / `@resource-id` / 裸 `pkg:id/x` / `//`、`/`、`*` 步 / **axes：child·descendant·ancestor(-or-self)·parent·self·following-sibling·preceding-sibling** / 谓词：`[@a='v']` `[@a]` `[@a!='v']` `contains/starts-with/ends-with` `not(...)` **`and`/`or` 组合**（标准优先级）**`[n]` 位置谓词**（1-based，XPath spec per-parent——`//x[n]`=各 parent 第 n 个 x 子节点而非扁平序第 n 个）/ union `|` / `parent()` / `scroll_to` / `all()`/`get()`/`exists`/`wait()`/`click()`/`get_text()`/`info`/`swipe`。仍 NIE 的超常见面清单：`last()`/`position()`/`text()`/`comment()`/`namespace::`/`attribute::`/`following::`/`preceding::`（非 sibling）/ 数值比较 `<` `>` `<=` `>=`。watcher `when()`：可翻为 wire 键的简单式照常设备端 arm；可求值复杂式降级 host-poll 规则（`_xpath` sentinel，M25）；超出子集者注册即 NIE fail-fast。`when(xpath).click()` 首匹配（doc order）注记同前 |

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
| `connect(addr)` | ✅ | 位置参数兼容两种口径：adb serial 字符串（u2 风格 `u2.connect('ba0d072d')`）或本地 TCP port（int/digit-str，pilotdroid 风格）；`serial=` kwarg 与无参 auto-detect（扫 `adb forward --list` 中 `localabstract:pilotdroid_agent`）不变（M13）。`serial=None` 时从命中的 forward 行 **backfill serial**（对齐 `u2.connect()` 无参选首台语义——其余 adb 路径多设备下需 `-s`，M26）；多台设备都有 forward → 指引性错误（传 `serial=` 或 `PILOTDROID_SERIAL`）；无 forward → `adb devices` 恰 1 台 adopt、>1 台同口径报错 |
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
| `watcher` | ✅ | `d.watcher.when(...).click()` + `.start()/.run()/.stop()/.reset()/.running`——M11 起 agent 0.7.0+（CAP_WATCH_ASYNC）走**设备端异步 watcher**（WATCH_ARM + media 通道 WATCH_HIT 推送，与 u2 server 线程语义对等且 armed 期间不阻塞 control）；旧 agent 回退阻塞 WATCH，再旧回退 host 轮询。规则命中按 `max_fires` 计数摘除——`call(fn, max_fires=N)`/`click(max_fires=N)` 公开参数透传（M26；默认 1=u2 fire-once 语义不变）；超 1 时命中后留在规则表，设备端规则下一 pass 自动 re-arm（bounded re-arm，M25）。re-arm 摘除→重臂间隙 ~50-100ms 内快闪的同匹配事件可能漏（arm 模型固有，host-poll 规则无此窗） |
| `watch_context(builtin)` | ✅ 近似 | ctx-mgr 语义同 u2；`builtin=True` 预挂 `_BUILTIN_DIALOGS`（非 u2 stock 规则复制，是本机实测击中的 ROM 弹窗集）：MIUI AdbInstallActivity「继续安装」、LBE 审查「确定」、展开 NotificationShade→`cmd statusbar collapse`。**点击类规则走 host-side inject 坐标点击**（device-arm 仅做观察）——agent 的 a11y ACTION_CLICK 会被 FLAG_SECURE 弹窗静默吞掉（M24 真机实证：规则命中但 10s 倒计时仍走完自动拒绝）；LBE 审查是**多页**确定流程（fixture-doctor 同因循环），规则 `max_fires=3` 命中后 re-arm 续点（M25，仍是有界上限防永久匹配规则死循环）；shade 规则经 resourceIdMatches→host-poll，ROM 无此 id 则规则静默休眠 |
| `app_wait(pkg,timeout,front)` | ✅ | `pidof`+conn APP_INFO 判前台（M11） |
| `session(pkg,attach)` | ✅ 近似 | 返回 Session 对象：`running()` 前台轮询、app 消亡后 ops 抛 `SessionBrokenError`、`with` 退出 force-stop（M21-G8）。**崩溃看护是 op 时惰性轮询，非 u2 server 常驻看护**——崩溃在下一次操作时暴露而非即时 |
| `send_keys(text,clear)` | ✅ | TEXT op（ACTION_SET_TEXT 于 focused 节点，M11） |
| `open_notification()` `open_quick_settings()` | ✅ | `cmd statusbar expand-*`（M11） |
| `freeze_rotation(bool)` | ✅ 近似 | `settings accelerometer_rotation` 开关（M11） |
| `clipboard` | ❌ ROM/架构级不可达（t15 实证） | agent 走 `UiAutomationConnection` 无 app `Context`（`ActivityThread.systemMain()` 会自杀，ADR-008）；宿主侧三路实证全死：①`service call clipboard 1/2` 返回未公开 parcel（负状态码）②`cmd clipboard` 无该子命令实现 ③`dumpsys clipboard` 静默。复现：`adb shell service call clipboard 1` |
| `toast` `.get_message(timeout,default)` `.last` `.reset` | ✅ 实现（agent ≥0.7.1） | u2 同机制：a11y `TYPE_NOTIFICATION_STATE_CHANGED` 事件推 media ch，`has_parcel` 区分真 toast（无 parcel）与通知；与运行中 watcher 互斥（media 单连接）。真机实证：ApiDemos `PopupMenu1` 菜单点击 → `Clicked popup menu item Add`。MCP serve 下 toast 事件亦推送 `notifications/pilotdroid/toast`（push-armed watcher 保活 demux 期间），`watch_events` pull 返回 `{"type":"toast"}` |
| `screenrecord(output,time_limit)` ctx-mgr | ⚠️ API 在，输出 ROM 节流 | `/system/bin/screenrecord` 存在且 SIGINT/pull 链路通，但 MIUI API34 编码器 ~1-3 帧/4s（`screenrecord --verbose` 自报 `recorded 3 frames in 4 seconds`）→ 产出近静态 mp4，非可用录屏。与 ADR-008 流式墙同根因（虚拟显示源受限）。复现：`adb shell screenrecord --verbose --time-limit 4 /sdcard/x.mp4` |
| `current_ime()` `set_ime(id)` `ime_list()` | ✅ | `settings get secure default_input_method` + `ime set/list -s`；真机 round-trip 切回验证（ba0d072d） |

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
- **当前套件终态（M23/M24 后，0.6.3 已发布）**：verbatim **22 pass
  / 0 fail / 8 skip**（达可达上限）。8 skip 全为上游自带
  deprecated/TODO/大屏标记——其中 **4 面（`implicitly_wait`/
  `select_iter`/`count`/`watchers`）shim 已实现并有本地单测覆盖**
  （M21/M24/M25），skip 归因为上游标记而非 shim 缺口。
- 其余 shim 层补齐进度随 0.6.3 changelog / ledger 更新。
- **notify demux 已落地（M29，推翻 M26 defer）**：`watch_arm {push:true}`
  命中即发 `notifications/pilotdroid/watch_hit` MCP notification；
  非 push 命中缓冲降级 `watch_events` 可拉；client 断开自动 disarm
  全部 armed 规则（agent conn close 亦 reap，双保险）；stream 激活期
  实时轮询暂停（帧/命中共享 ch=1，demux 读会抢帧）——**实测定论（M31
  eval `stream_watch_coexist`+M32 stash→push，ba0d072d）**：流中
  hit/toast 与帧交错在同一 conn 到达，host 侧 `recv_event` 分拣入
  session stash，serve 每迭代排干 stash——**push notification 流中
  照推**（hit 到达不再等停流），`watch_events` pull 亦可用（hit 先
  于其动作产生的 toast）。pull 面
  `watch_arm`/`watch_disarm`/`watch_events` 保持齐备。
  同槽位的 toast ui_event 经 demux 推
  `notifications/pilotdroid/toast`（上表 toast 行）。
  session 恢复后 armed 规则按原 selector **自动 re-arm**（watch_id
  映射新分配，透明自愈），并推
  `notifications/pilotdroid/session_recovered`
  `{generation,stale_watchers,rearmed,rearm_failed}`；陈旧/未知 id 的
  显式 `watch_disarm` 不透传至 agent（防 id 空间碰撞误杀新 watcher，
  返回 `{disarmed:false}`）。
- **watcher 死讯（agent 0.7.4）**：armed watcher 非正常终结
  （poll 内部故障/会话终结强杀）→ 设备端临终推
  `{type:"watch_dead"}` ui_event（ch=1 同槽按 type 判别，serve 转
  `notifications/pilotdroid/watch_dead` 并对仍 armed 的规则做
  **有界 auto re-arm**（每规则每 epoch ≤2 次，rearm 结果随
  notification params 上报；pull 面 `watch_events` 返回
  `{type:"watch_dead"}`）——armed→dead 在两次 recovery epoch 之间
  不再静默；显式 `watch_disarm` 属正常撤除不发事件。agent 侧
  watch conn 集合 **cap=4**（最旧出列），ui_event 另进 ≤8 条/15s
  TTL replay 环——迟到 attach 的 conn 可补收近期事件（重复交付
  幂等）。
- **超出 u2 面的 MCP 工具**（shim 不覆盖，记此防口径漂移）：
  `describe_screen`/`screenshot_annotated`（SoM-lite 感知，编号空间
  与 `compat/som.py` 一致；`describe_screen ocr:true` 为可选
  host-tesseract 档——缺失时优雅降级正常输出+`ocr.available:false`
  提示，canvas/图片文本转 `ocr_only` 元素按 center 可点）、
  `verify_after`（操作后 dump 断言闭环，`expect.state` 支持
  checked/selected/enabled/focused 状态谓词——「tap ok≠效果」
  分层成 ACK vs 效果断言，u2 无此能力；`match.i` 归
  `list_elements` 同一 interactive 编号空间（即 `tap_index`/
  `set_text_index` 消费的空间）、
  与其余 match 键 AND 组合，纯 text/desc/class match 搜全树）、
  `watch_arm{push}`（上条）。
- **编号空间对照（三空间，勿混用）**：
  | 空间 | 产出面 | 过滤规则 | 消费面 |
  |---|---|---|---|
  | SoM `id`/`#n`（document order 宽集） | `describe_screen`、`screenshot_annotated`、`compat/som.py` | 有 text/desc/rid 或任一 interactive flag | 仅 `[cx,cy]` 中心点 → `tap{x,y}` |
  | interactive `i`（0-based） | `list_elements`（默认 `interactive_only`）、`describe_screen` 元素 `i` 字段 | `clickable‖editable‖scrollable` 全树计数（含非 useful 交互节点，如裸 EditText） | `tap_index`、`set_text_index`、`verify_after.match.i` |
  | 全树 index（`node index=` attr） | `list_elements{interactive_only:false}`、agent 端 XML dump | 无过滤 | 无直接消费面——勿与 `i` 混用 |
  `describe_screen` 自 0.8.1 起每元素同时携带 `id` 与 `i`——盲驾
  四拍配方取 `i` 喂 `tap_index`（带 stale-guard），`id` 仅供中心点
  坐标路径与截图标注对照；`ocr_only` 合成元素无 `i`（无 a11y
  节点，仅 center 可点、先天无 stale-guard）。
- **元素状态面（M32）**：`list_elements`/`describe_screen`/compat
  `som.py` 元素统一透出 widget 状态——`checked`/`selected`/`focused`
  仅 true 时输出、`enabled` 仅 false 时输出（省 token）；`class`
  口径全面统一为**简单类名**（此前 describe 给全类名、
  list_elements 给简名，盲驾混淆源）。
- **Session 看护（M30，atx-agent 语义）**：Rust `Session` 控制通道
  op 遇死 socket 自动恢复并重放——崩溃在下一次 op 内透明治愈，
  非惰性抛错（serve/MCP 全工具面受益）。边界：`Timeout` 不恢复
  （挂起/死亡不可判定）；重放幂等限定（conn 断前请求可能已下发，
  副作用 op 有双执行窗口）；**armed watcher 与 stream 是
  agent-session-bound，崩溃即丢失**——`watch_recv` 遇死连接先治愈
  控制通道再原样抛错，调用方 re-arm 即可；watch_id 重新分配，陈旧
  id 可能撞新 id。恢复阶梯穷尽 → `Recover{stage}` 具名错。
