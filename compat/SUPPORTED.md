# u2shim 方法级兼容矩阵

`compat/u2shim.py`（`pip install ./python` → `import pilotdroid`）对照
uiautomator2 `Device` 公共 API 的逐条标注。约定：**drop-in 只承诺此表内
的项目**；表外调用按 AttributeError/ValueError 显式拒绝，不静默降级。

## Selector keys（`d(**kw)`）

| u2 key | 状态 | 备注 |
|---|---|---|
| `text` `resourceId` `description` `className` `packageName` `index` | ✅ | AND 语义同 u2 |
| `scrollable` | ✅ | wire 原生字段 |
| `textContains` `textStartsWith` `textMatches` `descriptionContains` `descriptionMatches` `classNameMatches` … | ❌ 显式拒绝 | agent 只做精确匹配；contains/regex 未实现 |
| `instance` | ❌ | 用 `index` 等价 |
| `clickable` `checked` `enabled` `focusable` `selected` `longClickable` … | ❌ 显式拒绝 | attrs 过滤语义未实现 |
| relative selector（`child`/`sibling`/`left`/`right`/`up`/`down`） | ❌ 显式拒绝 | |
| `xpath` | ❌ **非目标** | u2 xpath 为可选插件，明确不做 |

## Selector 对象方法（`d(...).X`）

| u2 API | 状态 | 语义差 |
|---|---|---|
| `.exists` | ✅ | property，同 u2 |
| `.click()` `.long_click(d)` `.set_text(s)` | ✅ | `.set_text` = 先点击再注入文本，同 u2 |
| `.info` | ✅ | property，同 u2；返回字段为子集（text/id/desc/bounds/class/clickable/enabled/scrollable/pkg） |
| `.wait(timeout)` | ⚠️ 语义差 | shim 返回 node-or-None；u2 返回 bool |
| `.scroll.<dir>` | ✅ | forward/backward/toEnd/toBeginning（shim 额外需要 `scrollable=True` 选择器先命中） |
| `.drag_to` `.swipe` `.pinch_in/out` `.gesture` `.fling` | ❌ | |

## Device 顶层方法

| u2 API | 状态 | 语义差 |
|---|---|---|
| `connect()` | ✅ | shim 签名 `connect(port=None, serial=None)`——port 缺省 auto-detect `adb forward --list` 中 `localabstract:pilotdroid_agent` 规则 |
| `click(x,y)` `long_click(x,y,d)` `swipe` `press(key)` | ✅ | key 别名：home/back/recent(s)/power/volume_±/menu/search/enter/del |
| `set_text(s)` | ✅ | 作用于已聚焦编辑框（u2 同语义）；**原生支持 Unicode**，u2 需 ADBKeyboard IME |
| `screenshot(path)` | ✅ | shim 额外 `scale`/`quality` 参数；返回 JPEG bytes |
| `dump_hierarchy()` | ✅ | 返回 XML 字符串（agent 原生生成，节点属性集与 uiautomator dump 近似） |
| `info` | ⚠️ 子集 | 仅 displayWidth/displayHeight/sdkInt/agent |
| `app_start(pkg)` `app_stop(pkg)` `app_current()` | ✅ | app_start 用 `am start`（u2 用 monkey）；`app_current` 返回 `{package,activity}` |
| `watcher` `app_list` `app_info` `app_wait` `session` `send_keys` `clear_text` `clipboard` `toast` `open_notification` `open_quick_settings` `screen_on/off` `unlock` `orientation` `freeze_rotation` `window_size` `shell`（结构化返回） | ❌ 未实现 | 按需加 |

## 实测数据

- `spike/e2e/e2e-m6-compat.json`：24-op 对拍 **23 ok / 1 unsupported（xpath）/ 0 error**。
- `spike/e2e/RESULTS-M7.md`：同脚本端到端双跑 wall-clock（shim vs 真 u2）。
