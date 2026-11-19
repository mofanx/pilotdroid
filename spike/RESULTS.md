# Spike 验证结果（t1）

> 日期：2026-11-19 · 执行：devin(t1) · 全部实测数据，可复跑（见 scripts/）

## 0. 测试机

| 项 | 值 |
|---|---|
| 序列号 | `ba0d072d`（USB 直连；另有一台 `11.0.0.2:5555` TCP 设备未参与） |
| 型号 | Xiaomi M2102K1C（小米 11 "star"，Snapdragon 888） |
| Android | 14（SDK 34），安全补丁 2025-04-01 |
| SELinux | **Enforcing** |
| shell 身份 | `uid=2000(shell)`，在 `input(1004)` 组，`context=u:r:shell:s0` |
| ABI | arm64-v8a |
| 屏幕 | 1440x3200 @ 60Hz，密度 560 |
| 前台 | 测试时为 `juloo.keyboard2` 启动页（非 MIUI 桌面） |

原始信息：`data/env-info.txt`

## 1. 核心结论（先行）

| 争议事项 | 实测结论 | 对方案的影响 |
|---|---|---|
| `/dev/input` 直接写 | **不可行**。0666 权限的 event2 也被 SELinux deny（EACCES） | Rust native 注入路径在本机死亡；minitouch 路线排除 |
| `/dev/input` 读 | **可行**。`getevent -pl` 枚举出全部 8 个输入设备 | native 仅能做"输入监听"，不能做注入 |
| `app_process` + 隐藏 API 注入 | **完全可行**。`InputManagerGlobal.getInstance()` 反射可用，注入返回 true 30/30，logcat 有 `MIUIInput: Input event injection` 实证 | scrcpy 路线在本机验证通过，**注入快路径在 Java agent 层** |
| 注入实测延迟 | **~3ms/次（wait_for_finish），~2.2ms（async）** | 比 `input` 命令(30ms)快 ~10x，比 u2 HTTP 往返(文献~263ms)快 ~80x |
| `app_process` 冷启动 | 整机 wall ~1.3s（ART 启动 ~1.1s + 内部逻辑 ~0.2s） | agent 必须**常驻**，不能 per-call spawn |
| `exec-out` 流式截屏 | **可行**。raw h264 流 8s 传 595KB/151 帧（静态画面 ~19fps，编码器 60fps 能力） | 无 agent 即可获得屏幕流的兜底方案 |

**架构裁决**：本机数据支持「app_process Java agent 为能力主体」的架构修正——native 二进制注入路线在 Android 14 Enforcing 下不可行，Java agent 注入实测 3ms 已是数量级快路径。

## 2. /dev/input 探测明细

- `ls -lZ`：8 个 event 节点，`crw-rw---- root:input u:object_r:input_device:s0`（event2 为 0666）
- `test -w`：**全部 w-no**（含 0666 的 event2 → SELinux 拦截，非权限位问题）
- 真实 open-for-write 测试（`( : > node )`，零字节安全写）：**全部 EACCES**
- 读取：`dd count=0` 全部成功
- `getevent -pl` 枚举（完整输出 `data/getevent-pl.txt`）：
  - event5 `cyttsp5_mt`、event4 `fts` —— **两个**触摸设备（ABS_MT 多点触摸）
  - event6/7 耳机按键、event0-3 其他（含 fts 键盘事件）
- dmesg avc 日志为空（shell 无权限读 audit，但 EACCES 已是充分证据）

## 3. 基线延迟（host 端 wall-clock）

| 操作 | 实测 | 备注 |
|---|---|---|
| `adb shell true` | 20–30ms | 每调用固定开销下限 |
| `input tap` e2e | 30–50ms（首调 100ms） | 设备内 ~10ms + adb 开销 |
| `uiautomator dump` | **2.25–3.13s** | 9914B XML（`data/uidump.xml`）；每次 fork+UiAutomation 连接 |
| `screencap -p` 到设备文件 | 450–510ms | 209KB PNG @1440x3200 |
| `exec-out screencap -p` | 420–450ms | 管道直出，略快 |
| `screenrecord 10s` | 220 帧/10s ≈ **22fps**（静态画面） | mp4 `data/rec.mp4`，r_frame_rate=58，编码器 60fps 能力；动态画面更高 |
| `exec-out screenrecord h264` | 8.3s → 595KB / 151 帧 ≈ 19fps | raw h264 流 `data/stream.h264` |

**关键推论**：`uiautomator dump` 每次 ~2.5s 的最大头是一次性 UiAutomation 连接+ART 启动——常驻 agent 持有 UiAutomation 后每次 dump 可压到 100-300ms 级（u2 已验证此模式）。这正是"per-call vs 常驻"的量化差距。

## 4. app_process 注入实测（`scripts/java/SpikeInject.java`）

构建：`javac --release 8 -classpath android-34/android.jar` + `d8 --min-api 34` → `spike.jar`（仅 classes.dex, 4KB）推到 `/data/local/tmp`。

运行：`CLASSPATH=/data/local/tmp/spike.jar app_process /data/local/tmp SpikeInject 700 1600`

两次运行原始输出：

```
init_ms=8.40   im_class=android.hardware.input.InputManagerGlobal
async_per_tap_ms=mean=2.36 p50=2.32 p95=3.53 min=0.65 max=6.25
waitfinish_per_tap_ms=mean=3.16 p50=3.07 p95=4.15 min=2.73 max=4.25
inject_ok=30/30   total_ms=184.1   wall=1.30s

init_ms=6.14
async_per_tap_ms=mean=2.18 p50=1.89 p95=3.80 min=0.87 max=5.42
waitfinish_per_tap_ms=mean=4.38 p50=3.04 p95=18.85 min=2.29 max=27.26
inject_ok=30/30   total_ms=213.5   wall=1.30s
```

logcat 实证（事件真正进入 InputDispatcher 并分发）：
```
W MIUIInput: Input event injection from package: null action ACTION_DOWN
W MIUIInput: Input event injection from package: null action ACTION_UP
（共 60 行 = 30 tap × down+up，时间戳间隔 2-3ms 与计时吻合）
```

技术要点：
- Android 14 上入口类是 `InputManagerGlobal`（反射 `getInstance()`），`InputManager.getInstance()` 兜底
- `injectInputEvent(event, mode)`：mode 0=async, 2=wait_for_finish，返回值 true 表示分发完成
- 注入事件 `package: null`（shell 无包名）——MIUI 有日志但不拦截
- 冷启动 wall 1.3s - 内部 0.2s ≈ ART 启动 ~1.1s → **必须常驻 + 常驻后单次注入 3ms**

## 5. 对方案的决策输入

1. **agent 语言裁决**：注入与截屏快路径都在 Java/app_process 层可行且已测出数量级优势；native 二进制定位降级为「输入监听（可读 /dev/input）+ 文件/进程杂务」的可选件，或干脆砍掉。
2. **里程碑修正**：注入原型 4KB dex 即可跑，M1 可直接「adb_client + 自研 app_process agent 薄片」，不必复用 u2.jar。
3. **协议目标有了实测基线**：常驻 agent 下注入 ~3ms vs per-call ~30ms vs u2 ~263ms——「持久 socket + 批量下发」的预期收益可被量化验收。
4. **屏幕流两路皆通**：`exec-out screenrecord`（无 agent 兜底 ~19-22fps 静态）与 scrcpy 式 agent 内 VirtualDisplay+MediaCodec（60fps 上限）并存。
5. **遗留未知**（建议后续 spike）：常驻 UiAutomation 下 dump 延迟（预计 100-300ms，需写 agent 验证）；`dispatchGesture`/UiAutomation.injectInputEvent vs InputManagerGlobal 的延迟差异；screenrecord 持续流 >180s 限制；注入坐标系在多屏/旋转下的行为。

## 6. 复跑方法

```bash
cd spike
DEV=ba0d072d
scripts/01-env-info.sh $DEV           # 设备信息
scripts/02-dev-input-probe.sh $DEV    # /dev/input 探测
scripts/03-baseline-latency.sh $DEV   # 基线延迟
scripts/04-screenrecord.sh $DEV . 10  # 截屏帧率
scripts/05-build-inject-dex.sh $DEV   # 编译+推送注入 dex
scripts/06-run-inject.sh $DEV 700 1600  # 注入实测 + logcat 证据
```
