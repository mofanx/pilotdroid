# RESULTS-M13 — selector 语义对齐 + nit 清零（t1 部分）

日期：2026-11-20。设备：ba0d072d（USB）。agent 0.7.0（本轮无 jar 变更）。

## 1. chained relative selector 真 bug 修复

M12 host-filter 的锚点解析取 `self._base._kw_match`（base 叶 kwargs）——
`d(A).child(B).left(C)` 锚到"树里第一个 B"而非"B in A"，二级链式静默误解析。

修法：`_Selector._resolve(pairs, parent)` 递归——rel 锚点 =
`self._base._resolve()` 首命中（含 base 自身 instance 位置），链式任意深度正确。

回归测试入 `compat_m6_realscripts.py` **script_D_selector_semantics**（5 断言，
XML ground truth 对照）：

- `chain child→left`：shim left count == XML 计算的"r<=锚点左缘"节点数 ✓
- `child().child()` 二级链式 ✓；`child()` = 后代（count-1 恒等式）✓
- 单级 relative 不回归（sibling/left/right/up/down 路径未变）

## 2. index/instance 语义对齐 u2（M11-12 混用的纠正）

| u2 key | 旧实现（错） | M13 对齐 |
|---|---|---|
| `index` | =Nth-match | **child-position**（XML `index` attr，host post-filter 谓词） |
| `instance` | =Nth-match | 保持 Nth-match（映射 wire `index` 字段——wire 语义即此） |

实证：`d(index=1).count` == XML `index="1"` attr 节点数（21=21、77=77
逐值吻合）；`d(index=N)` 走 host 路径（`_WIRE_KEYS` 移除 index），
`d(instance=N)` 仍走 wire 快路径。SUPPORTED.md/docs/02 口径已同步。

## 3. serve 尾项

- `watch_events` + `timeout_ms` schema arg（默认 5000 = connect 期
  read timeout，min 500 clamp）——`MediaClient.set_read_timeout` +
  `Session.watch_recv(timeout_ms)`；真调 `timeout_ms:100`→0.5s
  `{type:"timeout"}` isError:false。
- conn-replaced warn 文案补 `(safe to ignore on first connection)`。

## 4. nit 账（t4 复核遗留，t1 负责部分）

- `watcher.running` 入 SUPPORTED watcher 行 → IGNORE.running 可退役（t2 侧改）
- dump TTL cache：**记 defer**——`_Scroll._to` 靠 dump 变化判定内容稳定，
  cache 会致立即假命中；mutating-op 失效跟踪面广，复杂度超 ~200ms 收益

## 5. 验证

- `compat_m6_realscripts.py`：**28 ok / 1 unsupported(xpath) / 0 error**
  （script_D 5/5 真值断言）
- `verify-agent.sh ba0d072d`：**37/37 PASS**
- `cargo test --workspace` 全绿；clippy/fmt 净
- 插曲：本 worktree target 致磁盘满 → CARGO_TARGET_DIR 指向主 worktree
  共享（AGENTS.md 既定约定）
