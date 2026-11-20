# RESULTS-M10 — agent 能力面（多指针 INJECT + 设备端 WATCH）

**Device**: ba0d072d (Mi 11, Android 14, USB adb) — 唯一测试机
**Agent**: 0.6.0, caps=0x1ff (511) — verify-agent.sh **33/33 PASS**

## 1. 多指针 INJECT（CAP_MULTI_PTR=128）

`MotionPoint.pid: Option<u8>` 入 wire（缺省=单指针零变化）；agent 维护
pid→coords 活动表，>1 个 pid 时展开为 MotionEvent.obtain(14参) +
ACTION_POINTER_DOWN/UP|idx 分组。

- 真机 smoke：`inject_multi` 双指 pinch 序列 20 events / 38ms / `ok:true`（im 后端）
- shim `_Selector.pinch_in/pinch_out(percent, steps)` 落地；`.gesture/.drag_to/.swipe/.fling`
  SUPPORTED.md 归位 ✅（单指针 points 免费项，此前误标 ❌）

## 2. 设备端 WATCH（CAP_WATCH=256, op 0x0070）

`WatchReq{selector, action, timeout_ms, poll_ms}` → agent 内 poll find+click
闭环 → `{matched, waited_ms, action_ok, node}`。shim watcher 在
caps&WATCH 时自动走 device 模式，否则 host 轮询回退。

## 3. 三端 watch 延迟对拍（spike/e2e/bench_m10_watch.py）

场景：MIUI `am start -a android.intent.action.DELETE` → 包管理 resolver
弹窗（`text=取消 pkg=android`，pkg 限定排除 launcher btnCancel 噪声）。
n=7/arm，全部 7/7 fired。

| arm | 指标 | p50 ms | 数据文件 |
|---|---|---|---|
| device WATCH（元素已在场）| detect+act floor | **39** | e2e-m10-watch-device-watch-hot.json |
| device WATCH（延迟弹窗）| trigger→acted（含 am 渲染 ~430ms）| 664 | e2e-m10-watch-device-watch.json |
| shim host-poll @500ms | trigger→found | 812 | e2e-m10-watch-shim-poll.json |
| 真 u2 watcher | appear→gone | **1788** | e2e-m10-watch-u2.json |

**引用口径**：同场景弹窗 appear→dismiss，u2 watcher ~1.79s（其 server
轮询间隔主导）vs device WATCH 元素在场 detect+act **39ms**——架构性差异
（设备内闭环 vs host/server 轮询），约 **45×**；对 shim host-poll（500ms
间隔）约 **8~20×**。真 u2 watcher 实测轮询间隔 ~1.4–1.7s。

## 4. 韧性 + 口径

- `client_replace` 语义定稿：顶替非原子（accept→shutdown 有 TOCTOU
  窗口，在途 op 可落地）；verify 断言改为 poll ≤1s until dead；
  docs/02 §4.6 已补注；agent log 区分 `session ended: replaced` vs EOF。
- keyguard 前置：verify 开头 WAKEUP + **600ms 长滑**（MIUI 需 ~600ms/
  ~2400px 才能dismiss 锁屏，150ms 无效——实测踩坑）。
- SUPPORTED.md 加 `screen_off` ⚠️ MIUI pocket-mode 陷阱（可致全输入
  僵死仅 reboot 解）。

## 已知边界（诚实注记）

- `action_ok=true` 是**注入层**成功，不保证 hit-test 命中——node bounds
  若落屏外（偶见 resolver 全高布局）点击无效。watch 语义是"注入已下发"。
- WATCH 是阻塞 op 占整个 control 会话直到 matched/timeout；多 selector
  并行监测需多 client 轮换或 host 回退。
