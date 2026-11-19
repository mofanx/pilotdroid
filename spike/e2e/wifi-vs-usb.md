# WiFi vs USB adb-forward 延迟对照 + SHOT 字节账（M2 快赢实验）

设备 ba0d072d（Mi 11 / Android 14 / MIUI），agent 同进程（pid 31413），
持久 socket，`agent_client.py bench`（n=30）+ 手动 shot 拆分。
两通道各自 `adb forward` 到同一 `localabstract:pilotdroid_agent`。

## 1. WiFi(192.168.123.124:5555) vs USB forward RTT

| 指标 | USB p50 (min/p95) | WiFi p50 (min/p95) | 结论 |
|---|---|---|---|
| ping RTT | 2.58 (1.57/43.0) | 10.44 (6.48/54.1) | WiFi 基线 RTT ~4× 更差 |
| tap e2e | 9.35 (2.19/53.1) | 21.41 (6.32/59.6) | WiFi 全面更差 |
| dump e2e (9.7KB) | 47.06 (11.6/58.0) | 51.10 (47.9/70.2) | 略差 |
| shot e2e (420KB) | 147.6 (96.3/322) | 249.0 (210.6/353) | WiFi 差 ~100ms |

**结论：负结果——WiFi forward 不优于 USB，不采用。** USB p50 ping 2.6ms
证明链路本身可低至 ~2ms；此前观察的 "~40ms e2e 底噪"实为**抖动尾**
（USB ping p95=43ms，MIUI USB 调度所致），而非固定传输下限。
推论：M2 压降点击 e2e 的正解仍是 BATCH/NOACK 摊薄往返次数，
换传输介质无效。

操作注记：`adb tcpip 5555` 重启了 adbd，杀死了运行中的 agent daemon
（pid 30943→重启后重新部署为 31413）。**WiFi 实验必须承担一次
daemon 重启代价**——已写入已知风险。

## 2. SHOT 字节账（编码 vs 传输拆分）

- JPEG `quality=80, scale=1.0`：**420,446 B**，e2e p50 **159.7ms**
- 设备内 cost_ns：SHOT 响应未上报（恒 0，agent 未填）→ 用差分法估算：
  dump(9.7KB) e2e p50 47ms ≈ RTT 抖动+小包；shot−dump ≈ 113ms / 410KB
  → 有效吞吐 ~3.6MB/s ≈ **0.26ms/KB**
- 拆分估计：传输 ~110ms（420KB × 0.26），编码+截图 ~30-50ms
  （spike-2 实测 takeScreenshot 13-24ms + JPEG 编码）
- **传输占 ~70%** → `scale` 参数是正解：scale=0.5 → ~105KB →
  预计 e2e ~55-80ms，命中 ≤80ms 验收线。

## 3. 附带数据（USB 持久连接基线，供 M2 验收对照）

- ping mean 6.66 / p50 2.58；tap e2e p50 9.35（dev 内 ~1.1-5ms）；
  dump e2e p50 47.06（dev 3.28，9.7KB）；shot e2e p50 147.6（420KB）
- per-invocation CLI（先前数据）≈ +60ms：spawn+forward+HELLO 摊销，
  M2 session 模式预期回收该段。

原始 JSON：/tmp/bench-usb.json、/tmp/bench-wifi.json（未入库，临时文件）。
