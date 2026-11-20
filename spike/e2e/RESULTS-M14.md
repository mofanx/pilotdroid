# RESULTS-M14 — 发布闸前清账（dist/CI 面）

Scope: t2 dist/CI — jar 字节级可复现 CI、pip/windows 文案、tap sha256
预回填、MCP registry 草稿。compat/serve/u2shim 面归他人。

## 1. jar 字节级可复现（本地先行验证 → CI job）

本地配方（`~/Android/Sdk`，javac 25.0.5.1）:

```bash
javac --release 8 -cp $ANDROID_HOME/platforms/android-34/android.jar \
  -d agent/build agent/java/pilotdroid/Agent.java
d8 --min-api 30 --output agent/agent.jar agent/build/pilotdroid/Agent*.class
```

结果：重建 jar sha256 `92e4fcfdd0da8f56ea2e4ec9dd3922217c9bf0dfb5c4195b0440901f6d46d724`
与 committed `agent/agent.jar` **字节全同**（d8 输出 zip 内条目
时间戳固定为 epoch，确定性产物）。同一 sha256 也出现在 v0.6.0
release SHA256SUMS.txt——commit ↔ release asset ↔ 本地重建三方闭合。

CI 落地：`ci.yml` 新增 `agent-jar-repro` job（agent-jar-sync 之后），
`android-actions/setup-android` + `sdkmanager` pin `build-tools;37.0.0`
`platforms;android-34`，同一配方重编译后 `cmp` committed jar。
首次实跑结论：见下方 §CI-run（merge 后回填）。

## 2. README / release notes 诚实口径

- README 安装区：`pip install pilotdroid` 主线行（标注 0.6.0 publish
  后生效），与 git+https 并列。
- README 平台表 windows 行注 "(compile-verified only)"；
  release-notes-v0.6.0.md 同步同一句。
- release notes 安装小节本就已有 `pip install pilotdroid`（M13 已写）。

## 3. homebrew tap sha256 预回填

`gh release download v0.6.0`（draft，auth 可拉）→ 本地 sha256sum 与
`SHA256SUMS.txt` 逐项核对一致。`mofanx/homebrew-pilotdroid` formula
已重写为 `on_macos`(arm64) / `on_linux`(x86_64) 双块并回填 sha256，
commit `b15dc3e` 已推送。publish 后该步零追加操作。Windows 无 brew
目标（.exe 直连下载），注记留公式注释外。

## 4. MCP registry 草稿

`docs/mcp/server.json`（registry schema，name `io.github.mofanx/pilotdroid`，
pypi package `uvx` + `serve` args）+ `docs/mcp/SUBMIT.md`。
**诚实记录阻塞**：whl 无 `pilotdroid serve` 入口（只有 python client
`pilotdroid-agent`），`uvx pilotdroid serve` 今日必败——提交前提
见 SUBMIT.md（pypi 先行 + whl 需可起 serve 或换 mcpb/oci）。

## 5. CI-run（merge 后回填）

- [ ] agent-jar-repro job 首跑 pass/fail + 耗时
- [ ] rust/python/agent-jar-sync 全绿

## 6. 用户侧剩余（不变序）

PyPI pending-publisher → publish v0.6.0 draft →（零追加）→
branch protection required checks: `rust` `python` `agent-jar-sync`
`agent-jar-repro`（新增第四个名字）。
