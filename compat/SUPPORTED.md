# u2shim 方法级兼容矩阵

`compat/u2shim.py`（`pip install ./python` → `import pilotdroid`）对照
uiautomator2 `Device` 公共 API 的逐条标注。约定：**drop-in 只承诺此表内
的项目**；表外调用按 AttributeError/ValueError 显式拒绝，不静默降级。

## Selector keys（`d(**kw)`）

| u2 key | 状态 | 备注 |
|---|---|---|
| `text` `resourceId` `description` `className` `packageName` `index` | ✅ | AND 语义同 u2；全精确键时走 wire QUERY 快路径 |
| `scrollable` | ✅ | wire 原生字段 |
| `textContains` `textStartsWith` `textMatches` `descriptionContains` `descriptionStartsWith` `descriptionMatches` `classNameMatches` `packageNameMatches` `resourceIdMatches` | ✅ | M12 host post-filter：dump 全树→Python 端谓词（regex 用 fullmatch 语义，同 u2 `*Matches`） |
| `instance` | ✅ | `index` 别名（M11）；两路径均为"第 N 个命中" |
| `clickable` `checkable` `checked` `enabled` `focusable` `focused` `selected` `longClickable` | ✅ | M12 host post-filter attrs |
| relative selector（`child`/`sibling`/`left`/`right`/`up`/`down`） | ✅ | M12 host post-filter：空间向=节点 bounds 完全落在锚点方向区；child/sibling 用 dump 树父子关系 |
| `xpath` | ❌ **非目标** | u2 xpath 为可选插件，明确不做 |

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
| `.child` `.sibling` `.left` `.right` `.up` `.down` | ✅ | M12 relative selector（见 selector keys 行） |

## Device 顶层方法

| u2 API | 状态 | 语义差 |
|---|---|---|
| `connect()` | ✅ | shim 签名 `connect(port=None, serial=None)`——port 缺省 auto-detect `adb forward --list` 中 `localabstract:pilotdroid_agent` 规则 |
| `click(x,y)` `long_click(x,y,d)` `swipe` `press(key)` | ✅ | key 别名：home/back/recent(s)/power/volume_±/menu/search/enter/del |
| `set_text(s)` | ✅ | 作用于已聚焦编辑框（u2 同语义）；**原生支持 Unicode**，u2 需 ADBKeyboard IME |
| `screenshot(path)` | ✅ | shim 额外 `scale`/`quality` 参数；返回 JPEG bytes |
| `dump_hierarchy()` | ✅ | 返回 XML 字符串（agent 原生生成，节点属性集与 uiautomator dump 近似） |
| `info` | ⚠️ 子集 | 仅 displayWidth/displayHeight/sdkInt/agent |
| `app_start(pkg)` `app_stop(pkg)` `app_current()` | ✅ | app_start 用 `am start`（u2 用 monkey）；`app_current` 返回 `{package,activity}`——M11 起有 CAP_APP_INFO 时走 conn op（~ms），`activity` 为 `null`（a11y 不暴露）；否则回退 dumpsys |
| `shell(cmd)` | ✅ | 返回 `ShellResponse(output, exit_code)`，同 u2 |
| `clear_text()` | ✅ | 聚焦编辑框清空（ACTION_SET_TEXT 空串，免 IME） |
| `app_list()` `app_info(pkg)` | ✅ | `pm list`/`dumpsys package`；app_info 为 versionName/Code 子集 |
| `window_size()` | ✅ | `wm size` → `(w,h)` |
| `screen_on()` `unlock()` | ✅ | `dumpsys power` 判态 + power keyevent；unlock=wake+上滑 |
| `screen_off()` | ⚠️ | **MIUI 口袋模式陷阱**：power keyevent 关屏若贴近距离传感器，MIUI 可进入 pocket-mode → 此后全部注入输入被吞、屏幕不可点亮，**仅 reboot 可解**。建议仅在屏幕可受控测试场景使用 |
| `orientation` get/set | ✅ | `settings user_rotation`；set 冻结重力旋转（同 u2 语义） |
| `watcher` | ✅ | `d.watcher.when(...).click()` + `.start()/.run()/.stop()/.reset()`——M11 起 agent 0.7.0+（CAP_WATCH_ASYNC）走**设备端异步 watcher**（WATCH_ARM + media 通道 WATCH_HIT 推送，与 u2 server 线程语义对等且 armed 期间不阻塞 control）；旧 agent 回退阻塞 WATCH，再旧回退 host 轮询。规则命中一次即摘除 |
| `app_wait(pkg,timeout,front)` | ✅ | `pidof`+conn APP_INFO 判前台（M11） |
| `session(pkg,attach)` | ✅ 简化 | launch+wait foreground；u2 的崩溃看护重启未实现（M11） |
| `send_keys(text,clear)` | ✅ | TEXT op（ACTION_SET_TEXT 于 focused 节点，M11） |
| `open_notification()` `open_quick_settings()` | ✅ | `cmd statusbar expand-*`（M11） |
| `freeze_rotation(bool)` | ✅ 近似 | `settings accelerometer_rotation` 开关（M11） |
| `clipboard` `toast` | ❌ 未实现 | clipboard 在 API34 service-call 签名漂移不可靠；toast 需注入 overlay |

## 实测数据

- `spike/e2e/e2e-m6-compat.json`：24-op 对拍 **23 ok / 1 unsupported（xpath）/ 0 error**。
- `spike/e2e/RESULTS-M7.md`：同脚本端到端双跑 wall-clock（shim vs 真 u2）。
