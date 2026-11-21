# RESULTS-M16 — 发布决断执行 ledger

目标：v0.6.0 tag 不含 M14/M15——按原 tag publish 会上线旧 whl。
决断：bump 0.6.1，tag 含全部 M14–M16 工作后再发布。

## 版本与发布链（t1, p3）

- pyproject + Cargo workspace + lock：`0.6.0→0.6.1`；CHANGELOG 0.6.1 条目
  （M14 jar-repro/pip UX + M15 lazy-bootstrap/adb_client4/marker/app-lock
  + M16 cache-verify/env）。
- `v0.6.1` tag = `19a0c9c`（含 p2 的 env 提交；tag 首指 e8d9b44 时 t2 恰入
  main，已 retag 保证二进制与 CHANGELOG 一致；stale-sha release run
  `44884080843` 已取消防 asset 竞争）。
- release run `44884111305` 六 job 全 success → draft 7 assets
  （whl+sdist+3 CLI+agent.jar+SHA256SUMS）；notes =
  `docs/release-notes-v0.6.1.md`。
- `v0.6.0` draft 删除；tag `baf57b9` 保留为 **M13 快照（未发布）**——
  `release-notes-v0.6.0.md` 头注同步（勿 publish）。

## env 承诺兑现（t2, p2）

- `PILOTDROID_SERIAL` env fallback：`Cli::parse` 后单点，`-s` 优先，
  覆盖全部子命令（含 serve）——server.json `environmentVariables`
  不再是悬空承诺。实机三向实证（t4 复核）：env 消费 / bogus 拒绝 /
  `-s` 覆盖 env。
- u2shim `_detect_port` 错误串改 pip-first（`pilotdroid-cli setup`），
  compat/ 与 python/ whl 副本 diff 全同无漂移；实机触发 RuntimeError
  文案确认（t4）。
- CONTRIBUTING「CARGO_TARGET_DIR 共享」建议→**必做**+理由；
  m10agent target 清理；本 worktree 残留 739M 由 t4 复核时 clean
  （df /home 98%→96%）。

## bootstrap 强化（t1, p3）

- cache-hit 重验 sha256：命中也拉 SHA256SUMS 比对，不符→删缓存重下；
  SUMS 不可达（离线）→跑缓存。+2 单测（corrupt-redownload /
  offline-exec），**7/7 PASS**。

## install-smoke 修链（t1 + r2t1, p3）

- step3b 新增：venv 内 `pilotdroid-cli --version` 走真 bootstrap 链
  （下载→校验→缓存→execv）——whl lazy-bootstrap 的唯一端到端验
  （单测全 mock）。
- r2t1：三处滞留 `0.6.0` pin 修复——版本参数化
  `VER="${PILOTDROID_SMOKE_VER:-0.6.1}"`（pip pin + gh tag 单点）；
  step4 gh download 降为对照步；step5 改走 venv console script
  （真用户路径 + cache-hit execv 再验）。

## 卫生批

- issue #6 关闭（评论指 e851e02 adb_client 4.0.0 迁移）。
- 远端 stale 分支清理：m10/m11/m13/m14/m15 共 9 条（merge-base 确认后删）；
  dependabot 分支留 bot 管理。
- `v0.5.0` draft 删除（M17 补，tag 留）。
- CodeQL workflow 入库：python autobuild + java manual javac
  （android-34）；首跑 `44884174375` success。
- SUBMIT.md：uvx 冷启动预期管理（首调下载 ~MB，initialize 延迟秒级）。

## 交叉复核抓出的真缺陷（闭环）

| 抓出 | 内容 | 处置 |
|---|---|---|
| t3→t1 | install-smoke 三处 0.6.0 滞留 pin（gh download v0.6.0 draft 已删必挂） | r2t1：VER 参数化 + step5 console script，merge 00b091f |
| t3→t1 | tap formula pin v0.6.0 全套（publish 后 brew 静默装旧版） | M17-t1：0.6.1 预回填 ebf4f3e |
| 自查 | server.json `PILOTDROID_SERIAL` 无代码消费（幻影 env var） | t2：env fallback 落地，t4 三向实证 |

## 待办（publish 前后）

- 用户四连击：PyPI pending-publisher → publish v0.6.1 →
  `spike/e2e/install-smoke.sh ba0d072d` → branch protection
  （rust/python/agent-jar-sync/agent-jar-repro）。
- publish 后：MCP registry 提交（server.json 0.6.1 + schema PASS）、
  tap URL 激活即生效。
