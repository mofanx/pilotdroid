# RESULTS-M22-release — v0.6.2 发布链实证 ledger

> 记录 v0.6.2 发布列车每个环节的可复核证据。日期：2026-11-24。

## Tag / Release

| 项 | 值 | 证据 |
|---|---|---|
| tag | `v0.6.2` @ `81d5541` | `git rev-parse v0.6.2^{}`（t1 复核 ==HEAD） |
| release.yml | run `45428050511` → success | gh run view |
| release 状态 | published（非 draft），`2026-11-24T15:53:44Z` | `gh release view` |
| assets（7） | agent.jar / whl / sdist / linux / macos-arm64 / windows.exe / SHA256SUMS.txt | `gh release view --json assets` 复核 |
| sha256 | `sha256sum -c SHA256SUMS.txt` 6/6 OK（assets 内非和文件 1 行除外） | t1 实跑 |
| whl 内容 | 含 M21/M22 shim 符号：`Session`/`SessionBrokenError`/`_Settings`/`_pidof`/`last_error`/`PILOTDROID_SERIAL` | whl 解包 grep 全 True |
| notes 正文 | `docs/release-notes-v0.6.2.md` curated 内容（release.yml notes-file 分支，非 dependabot 生成稿） | t1 复核 |

## PyPI（OIDC 首绿）

| 项 | 值 |
|---|---|
| pypi.yml run | `45430764102` → success（23s，**trusted publisher 历史首次自动上传**） |
| PyPI 版本 | `pilotdroid 0.6.2`：`bdist_wheel` + `sdist` 双产物，`latest=0.6.2` |

## install-smoke（全真复验，t2 本轮实跑）

`PILOTDROID_SMOKE_VER=0.6.2 spike/e2e/install-smoke.sh ba0d072d` →
**8 pass / 0 fail**：

1. `pilotdroid==0.6.2` PyPI 索引可见（try 1，索引传播零等待）
2. 干净 venv `pip install pilotdroid==0.6.2`
3. `import pilotdroid.u2shim`
4. 连接错误提示指向 `pilotdroid-cli`（可行动，非死胡同）
5. **lazy bootstrap 真链**：venv `pilotdroid-cli --version` → 下载
   v0.6.2 二进制 + SHA256 校验 + execv → 输出 `pilotdroid-cli 0.6.2`
6. `gh release download` 裸二进制对照 → `pilotdroid-cli 0.6.2`
7. `cli -s ba0d072d setup`：adb/device_ready/agent_jar/forward_dial/
   hello(`ver=0.7.0 caps=0x7ff`)/u2_conflict/inject_probe 全 PASS；
   发现 2 orphan forward（tcp:17932/17933，被杀会话泄漏）——
   已按 AGENTS.md 规则仅删列出的两条
8. `serve` MCP initialize + `tools/list` 握手 → `"tools"` 命中

## MCP registry

- `docs/mcp/server.json` 已 bump 0.6.2（t3/t4 预备件内）。
- **registry 重发（`mcp-publisher publish` →
  `io.github.mofanx/pilotdroid@0.6.2`）留给用户闸**——registry 是
  快照不跟随，不重发则永久停在 `0.6.1.post1`（PUBLISH.md Step 6）。

## 已登记环境项（如实，非回归）

- ApiDemos fixture **已恢复**（发布后用户开「USB 安装」+ agent
  a11y 自动点掉 `AdbInstallActivity` 10s 倒计时确认框，
  `adb install --bypass-low-target-sdk-block` Success）。
- `watch_context` 归因**实证闭环**：fixture 恢复后全量复跑
  **13 pass / 9 fail / 8 skip**——watch_context PASS，剩余
  9 fail 全为 xpath 明示非目标，零代码改动。两层环境失败
  （fixture 缺失 + 锁屏/Shade）定稿无误。详见
  RESULTS-M17-u2suite.md 末附记。

## v0.6.3 发布链（M23/M24 成果兑现，2026-11-25）

| 项 | 值 | 证据 |
|---|---|---|
| tag | `v0.6.3` @ `d6d32c2` | `git rev-parse v0.6.3^{}` |
| release.yml | run `45521202140` → success | gh run list |
| release 状态 | published，assets 恰 7（agent.jar/whl/sdist/linux/macos-arm64/windows.exe/SHA256SUMS.txt） | `gh release view --json assets,isDraft` |
| sha256 | `sha256sum -c` 6/6 OK | 本机下载实测 |
| notes 正文 | `docs/release-notes-v0.6.3.md` curated（M23/M24 真实内容） | `gh release view --json body` |
| pypi.yml | run `45521551479` → success（**OIDC 第二次绿**） | gh run list |
| PyPI | `pilotdroid 0.6.3` whl+sdist，`latest=0.6.3` | `/pypi/pilotdroid/json` |
| install-smoke | `PILOTDROID_SMOKE_VER=0.6.3` → **9 pass / 0 fail**——step 2b `Device.xpath`+`_xp_eval` 罐头 eval **真断言通过**（非 SKIP；0.6.3 wheel 首个含 xpath-lite 的发布件） | t3 实跑 |
| MCP registry | `io.github.mofanx/pilotdroid`：`0.6.3`→active+isLatest / `0.6.2`→active / `0.6.1.post1`→deprecated | `/v0/servers/.../versions` 读回 |
| Homebrew tap | `mofanx/homebrew-pilotdroid` `384afa1`：version+url×2+sha256×2=0.6.3（取自发布 SUMS） | `gh api` 读回 |

**发版尾巴遗留（M25 本批修）**：`install-smoke.sh` VER 默认
滞留 0.6.2（裸跑测的是上一版）——根因是 runbook 无此步，
本批同步在 PUBLISH.md 补 Required 条目 + VER→0.6.3 +
CHANGELOG 加 `## [Unreleased]` 桩。
