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

## ⑤ 独立复核（t3·devin，2026-11-19 晚）

复核方式：独立重跑 `scripts/02`、`03`、`06`、`07` + 手动 sock 回环；Mi 11 `ba0d072d`，**前台=com.kunenglish**（与 t1 测量时 juloo.keyboard2 启动页不同——关键变量）。

| t1 报告值 | 复核实测 | 判定 |
|---|---|---|
| /dev/input 写 8/8 EACCES | 8/8 `open-w-DENIED` | ✅ 复现 |
| IM 注入 async ~2.2ms | async p50=1.40-1.56ms | ✅ 复现 |
| IM 注入 waitfinish p50≈3.07ms | **waitfinish p50=13.5-14.7ms** | ⚠️ **未复现**——waitfinish 延迟随前台注入目标变化（轻量前台 3ms ↔ 重前台 15ms）；async 口径不受影响 |
| inject_ok 30/30 | 30/30 + 29/30（1 次抖动） | ✅ 基本复现 |
| `uiautomator dump` 2.25-3.13s | 2.25-2.38s（冷首调 11.25s） | ✅ 复现 |
| `screencap` 450-510ms | 420-450ms（exec-out 380-410ms） | ✅ 复现 |
| `input tap` e2e 30-50ms | 60-90ms | ⚠️ 偏慢——同前台变量 |
| UA 反射 connect（ctor10+conn28ms） | ctor 9.6ms + connect 14.6ms flags=1 | ✅ 复现 |
| resident dump p50≈2.39ms | p50=1.84ms（30 节点） | ✅ 复现 |
| agent 内 IM 注入 p50≈4.94ms | p50=10.59ms | ⚠️ ~2x，同前台变量；数量级成立 |
| UiA 注入 p50≈48.9ms | p50=33.35ms | ✅ 同量级 |
| takeScreenshot p50≈24.2ms | p50=13.32ms | ✅ 复现（更快） |
| SurfaceControl.screenshot 移除 | NoSuchMethod + DisplayCaptureArgs 不存在 | ✅ 复现 |
| localabstract+forward echo | `echo_bytes=16` 真实回环 | ✅ 复现 |

**复核结论**：架构裁决全部成立（单进程、IM 主注入、resident dump/截图数量级优势）。**一处措辞修正**：「注入 ~3ms」应限定为 async 口径或注明前台条件——waitfinish 实测 3-15ms 区间依赖注入目标应用；已回填 docs/04 §3 与 docs/05 M1 验收线。

### ⚠️ 复核新发现：第三台设备 duchamp（`192.168.123.130:5555`, Redmi 2311DRK48C, A16/SDK36, Enforcing）——注入全灭机型

复核期间该设备在线，顺手采集矩阵行，发现**全部注入路径不可用**（2/2 复现，原始数据 `data/spikeagent-duchamp-a16.txt`、`env-info-duchamp.txt`、`devinput-duchamp.txt`）：

- `/dev/input` 写：5/5 拒（与另两机一致）
- `app_process`+`InputManagerGlobal` 注入（spike.jar，无 UA）：init 后 **SIGKILL**
- `app_process`+`UiAutomation.injectInputEvent`：connect 成功后注入时 **SIGKILL**
- `input tap` 命令：**SecurityException**——`uid=2000` 无 `INJECT_EVENTS`（logcat `Permission Denial ... uid=2000 requires INJECT_EVENTS`）
- 对照：UiAutomation `connect` 本身可存活（setsid >20s）、纯 heartbeat 进程存活 >50s、resident dump 可跑但返回 0 节点（前台 Douyin splash 状态）
- **推断**：符合 MIUI/HyperOS「USB调试(安全设置)」未开启时收回 shell 模拟点击权限的特征；connect 成功 ≠ 注入可用
- **方案影响**：①能力探测必须「真实注入一次+观察进程存活」；②onboarding 前置条件 +1；③该机型上 u2 同样会失败——非我方独有问题；④降级链末端在此类设备只剩只读能力。已回填 docs/04 矩阵行与 §3 权限层结论。

另注：`scripts/07-spike2-agent.sh` 提交时缺执行位（mode 100644），已在本次复核中修正为 100755。
