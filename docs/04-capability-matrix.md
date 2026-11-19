# 04 · 设备能力矩阵

> 状态：持续更新 · 数据基准：t1 spike（2026-11-19, 可复跑 `spike/scripts/`）
> 规则：每行结论必须挂实测证据或标注"未验证"；新机型接入时重跑 `spike/scripts/` 全量并追加行。

## 1. 实测设备

| 设备 | 型号 | Android/SDK | SELinux | ABI | 备注 |
|---|---|---|---|---|---|
| `ba0d072d` | Xiaomi M2102K1C (Mi 11 "star", SD888) | 14 / 34 | **Enforcing** | arm64-v8a | 主测试机，USB 直连 |

shell 身份：`uid=2000(shell)`，supplementary 含 `input(1004)`，`context=u:r:shell:s0`。

## 2. 能力矩阵（实测列 = Mi 11/A14；其余设备列为占位待填）

| 能力 | 探测方法 | Mi 11 / A14 | 证据 | 方案落点 |
|---|---|---|---|---|
| `/dev/input` 写注入 | `test -w` + `( : > node )` | ❌ **全拒**：8 节点全 EACCES，含 0666 节点（SELinux deny，非权限位） | spike §2, `data/getevent-pl.txt` | **从注入链移除** |
| `/dev/input` 读监听 | `getevent -pl` / `dd count=0` | ✅ 8 设备枚举（双触摸设备 cyttsp5_mt + fts） | spike §2 | native sidecar 唯一保留能力（可选件） |
| `InputManagerGlobal` 反射注入 | `injectInputEvent(ev,0/2)` 返回值 | ✅ **30/30 true，p50≈3ms**（async ~2.2ms） | spike §4 + logcat `MIUIInput` 60 行 | 注入主路径 |
| `input` shell 命令 | `input tap` e2e | ✅ 30-50ms（首调 ~100ms） | spike §3 | 最终兜底 |
| `app_process` dex 常驻 | 冷启动 wall | ✅ ~1.3s（ART ~1.1s + 内部 ~0.2s） | spike §4 | 决定"必须常驻" |
| `uiautomator dump`（一次性） | CLI dump wall | ✅ 2.25-3.13s / 9914B | spike §3, `data/uidump.xml` | 量化"per-call vs 常驻"差距；常驻 dump 待测 |
| 常驻 UiAutomation dump | — | ⚠️ 未验证（估 100-300ms） | spike §5 遗留项 | **spike-2 必测**，决定 QUERY/DUMP 验收线 |
| `screencap` | `exec-out screencap -p` | ✅ 420-450ms / 209KB PNG | spike §3 | 截图兜底路径 B |
| `exec-out screenrecord` 流 | h264 流 8.3s | ✅ ~19-22fps（静态画面；编码器 60fps 上限） | spike §3, `data/stream.h264` | 无 agent 帧流兜底 |
| `UiAutomation.takeScreenshot` | — | ⚠️ 未验证（估 50-150ms） | — | **spike-2 必测**，截图主路径 A |
| `dispatchGesture` vs `injectInputEvent` | — | ⚠️ 未验证 | — | spike-2 对比项 |
| Unicode 文本注入 | — | ⚠️ 未实现 | — | scope 项，见 03-device-agent §6 |
| 多 display / 旋转坐标一致性 | — | ⚠️ 未验证 | — | spike-2 测 display_id 行为 |

## 3. SELinux 约束结论

- **Enforcing 下 `/dev/input` 写必死**：本机 0666 节点同样被拒，说明是 `shell → input_device` 的 avc 规则而非文件权限——该结论在绝大多数厂商 Enforcing ROM 上可推广（minitouch 历史 issue 佐证），本项目**不再把 native 注入当主路径或必备降级**。
- `app_process` + 反射走 InputManagerGlobal 依赖隐藏 API 访问权限：shell 域在 A14 实测可用；**非 shell 身份（如 app 上下文）会被 greylist 拦截**——agent 必须始终以 `adb shell` 身份启动。
- MIUI 对 `package: null` 注入有 warning 日志但不拦截；其他 OEM 行为需按矩阵扩行验证。

## 4. 设备扩展协议（新机型接入流程）

1. `adb devices` 确认 serial → `spike/scripts/01-env-info.sh <serial>`。
2. 全量跑 `02`（/dev/input）`03`（基线）`04`（帧率）`06`（注入+logcat）。
3. 把结果追加 §1/§2 矩阵行；若出现与 Mi 11 矛盾结论（如 /dev/input 可写），在 03-device-agent 降级链中恢复对应分支。

## 5. 目标覆盖集（建议）

| 优先级 | 设备画像 | 目的 |
|---|---|---|
| P0 | Xiaomi A14 Enforcing（已有） | 主基准 |
| P1 | 类原生/Lineage 或 Pixel（Enforcing） | 验证非 MIUI 行为差异 |
| P1 | Android 10-12 一台 | `InputManager` vs `InputManagerGlobal` API 分界 |
| P2 | 模拟器 x86_64 | CI 回归（native sidecar 需 x86_64 构建） |
| P2 | 三星 OneUI | 已知 SELinux 最严厂商之一，压测降级链 |
