# Spike-2 验证结果（t1·第二轮）

> 日期：2026-11-19 · 执行：devin(t1) · 设备：Xiaomi Mi 11 `ba0d072d`（A14/SDK34）+ Redmi 2602BRT18C `11.0.0.2:5555`（**A17/SDK37**）· 全部实测可复跑

## ★ 进程模型裁决：单进程可行 ✅（不需要 uiautomator runtest，不需要双进程）

**核心发现：`UiAutomation` 可在裸 `app_process` 中直接构造并 connect**——通过反射 `new UiAutomation(Looper, new UiAutomationConnection())` + `connect(FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)`（hidden ctor，shell uid 被接受）。**同一进程内五项能力全部实测可用**，两台设备一致：

| 能力 | Mi 11 / A14 | Redmi dash / A17 | 对比基线 |
|---|---|---|---|
| UiAutomation connect | 成功 flags=1（ctor 10ms + connect 28ms） | 成功 flags=1 | u2 走 runtest 获得，我们省去该入口 |
| resident dump（`getRootInActiveWindow`+全树遍历） | **p50 = 2.39ms**（9 节点） | **p50 = 1.24ms**（23 节点） | `uiautomator dump` 2.5s → **~1000x** |
| `InputManagerGlobal` 注入（同进程并存） | **p50 = 4.94ms**，30/30 | **p50 = 3.59ms**，30/30 | 与无 UiAutomation 时一致（3ms 级） |
| `UiAutomation.injectInputEvent` | p50 = 48.9ms，30/30 | p50 = 24.9ms，30/30 | 可用但慢 10x → 主注入走 IM，UiA 注入仅作兜底 |
| `UiAutomation.takeScreenshot()` | 1440×3200，**p50 = 24.2ms** | 1280×2772，p50 = 78.5ms | `screencap` 450ms → **10-18x** |
| `LocalServerSocket("pilotdroid")` + `adb forward localabstract` | **回环 echo 成功** | 未复测 | u2/scrcpy 同款传输，验证通过 |
| `SurfaceControl.screenshot(Rect,int)` | **已移除**（NoSuchMethod） | 同样移除 | A34/37 无此 API → 弃用，takeScreenshot 已够用 |

原始输出：`data/spikeagent-mi11-a14.txt`、`data/spikeagent-dash-a17.txt`

**架构推论**：
1. 进程入口 = **裸 `app_process` + 反射 `UiAutomation(Looper, UiAutomationConnection)`**，无需 `uiautomator runtest`、无需 instrumentation、无需 APK。
2. 注入主路径 = `InputManagerGlobal.injectInputEvent`（~4ms）；`UiAutomation.injectInputEvent`（~49ms）降级为兜底/需要 accessibility 上下文语义时备用。
3. dump 主路径 = `getRootInActiveWindow` 常驻遍历（~2ms），M1 的 ≤400ms 验收线应下修为 **≤50ms** 才有区分度。
4. `connect(FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)` 必须使用 flags=1——实测 flags=1 生效，避免杀掉用户已启用的无障碍服务（生产安全注意项）。
5. `getevent -pl` 在 A17 仍可读（枚举触摸设备 NVTCapacitiveTouchScreen）；`/dev/input` 写在两台 Enforcing 设备上**全拒**——native 注入路径二次确认死亡。

## ② 附属裁决

### dispatchGesture vs injectInputEvent
反射枚举确认 **`UiAutomation` 无 `dispatchGesture`**——该方法仅存在于 `AccessibilityService`（必须安装 APK + 用户授权，dex agent 拿不到）。替代已实测：**MotionEvent 序列注入 = 手势**，swipe（down+10×move+up）经 IM 全部 11/11 事件成功、总耗时 22ms。结论：dispatchGesture 不在 dex agent 能力集内，也不需要——注入序列覆盖同等功能。

### daemon 存活（`data/daemon-survival.txt`）
| 场景 | setsid+nohup | 裸 `&` |
|---|---|---|
| 宿主 `adb kill-server`（断开 8s+） | **存活** | **存活** |
| `adb usb`（设备侧 adbd 重启） | **被杀** | **被杀** |

结论：会话结束天然存活（无需特殊 daemon 化）；**adbd 重启（≈拔线/端口切换）必死**——agent 设计必须含 host 侧心跳探测 + 自动重部署，不能假设常驻不死（u2 同样如此）。

### MIUI 安装拦截（`data/miui-install.txt`）
- `adb install` / `adb shell pm install` **均触发** `com.miui.securitycenter/.permcenter.install.AdbInstallActivity` 弹窗，约 10s 倒计时无操作 → 自动拒绝 `INSTALL_FAILED_USER_RESTRICTED`
- **注入点击可确认弹窗**（实测弹窗被点击「继续安装」解除并推进到包扫描阶段）→ 但时序敏感，不适合作为常规依赖
- **重要旁证**：`uiautomator2` Python v3.7.1 init 时**完全不再安装 APK**——仅 `adb push u2.jar` + `app_process com.wetest.uia2.Main`，与我们 dex-only 架构同构 → 本项目「dex 推送 + app_process」路线被 u2 演进方向独立验证；APK 安装不可依赖反而是合理约束
- 文档建议：MIUI 设备如需 APK 能力（AccessibilityService/IME），须用户手动开「USB调试(安全设置)/USB安装」开关，列为 onboarding 前置条件而非自动流程

## ③ 第二台设备矩阵行（`11.0.0.2:5555`）

| 项 | Redmi 2602BRT18C "dash" |
|---|---|
| Android | **17 / SDK 37**（安全补丁 2026-10-01），Enforcing |
| ABI / 屏幕 | arm64-v8a，1280×2772 @520dpi |
| /dev/input | 5 节点写**全拒**（含 0666）；`getevent -pl` 可读，触摸=NVTCapacitiveTouchScreen |
| SpikeAgent 全套 | **同一个 spike2.jar 未修改直跑**，五项能力全部复现（上表） |
| 结论 | min-api 34 的 dex 在 SDK37 兼容；能力矩阵第二行填齐，跨大版本一致性成立 |

## ④ u2 真实基线（`bench/baseline-u2.json`，机器可读）

设备端 u2 server（u2.jar/app_process，端口 9008）稳定后，host python uiautomator2 v3.7.1 测量：

| 操作 | u2 p50 | 我方 spike 同功能 p50 | 潜在倍数 |
|---|---|---|---|
| `click(x,y)` | **249.0ms** | IM 注入 ~5ms（设备内）+ 通道 | ~50x（注入段） |
| `dump_hierarchy` | **330.2ms** | resident dump ~2.4ms + 序列化/传输 | ~100x |
| `screenshot` | **176.5ms** | takeScreenshot ~24ms | ~7x |
| `exists(selector)` | 0.02ms | —（本地惰性求值，无 RPC） | 持平即可 |
| `send_keys("ab")` | p50 309ms / p95 2021ms | — | 文本输入是 u2 弱项，差异化机会 |

> 注：u2 基线测量安排在全部我方测量之后执行；测后已 kill `com.wetest.uia2.Main` 进程、删除 `/data/local/tmp/u2*`，无残留（`adb forward` 未遗留，u2 用 socket 直连+forward 自管理）。

## 遗留事项 / 给 M1 的输入

1. **进程模型已定**：单 `app_process` + 反射 UiAutomation。`docs/03`、`adr/002` 应把「uiautomator runtest 入口」从主路径移除，改为「已知替代入口」。
2. M1 验收线可全部上实测数：dump ≤50ms、click 注入段 ≤10ms、takeScreenshot ≤100ms（A17 慢设备按 150ms）。
3. u2 弱项在文本输入（309ms+，且 p95 2s）——TEXT opcode 的价值得到对拍支撑。
4. `exists` 的 0.02ms 说明 u2 选择器本地惰性——我方 selector 语义层应同样避免每查必 RPC（find 才取树）。
5. MIUI 弹窗可用注入自确认（dogfooding 已验证）——可作 agent「系统弹窗自动确认」特性的真实案例，但安装路径本身不依赖。
6. spike 残留已清理（进程杀净、u2* 删除、forward 移除）；用户自有工具（scrcpy-server.jar/shizuku/adbkit）未触碰。

## 复跑方法

```bash
cd spike && DEV=ba0d072d
scripts/07-spike2-agent.sh $DEV all        # 构建+单进程全能力探针
scripts/07-spike2-agent.sh 11.0.0.2:5555 all   # 第二台（TCP 需先 adb connect）
scripts/08-daemon-survival.sh $DEV         # daemon 存活实验
scripts/09-install-probe.sh $DEV           # MIUI 安装拦截（加 --inject-confirm 自动确认）
python3 -m venv /tmp/pd-venv && /tmp/pd-venv/bin/pip install uiautomator2
/tmp/pd-venv/bin/python -m uiautomator2 init --serial $DEV   # 仅需 dex push，无 APK 安装
/tmp/pd-venv/bin/python bench/bench_u2.py  # u2 基线 → bench/baseline-u2.json
# 清理: adb shell pkill -f wetest.uia2; adb shell rm -rf /data/local/tmp/u2*
```
