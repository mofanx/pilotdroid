# RESULTS-M15 — 发布收口清账

Scope: t2 python/dist 面 — whl lazy-bootstrap、server.json schema 合规、
README 可视 demo。compat/crates/verify/SUPPORTED/RESULTS-M13 归他人。

## 1. whl lazy-bootstrap（pip UX 死胡同根除 + MCP pypi 路径解锁）

`python/pilotdroid/cli_bootstrap.py`：console script `pilotdroid-cli`
（+`pilotdroid` 别名，registry `uvx pilotdroid serve` 需要 identifier==entry
名）。首跑：platform.machine() 映射 asset 名（linux-x86_64/macos-arm64/
windows-x86_64.exe）→ tag 锚定 `v{importlib.metadata.version}` → 拉
asset + SHA256SUMS.txt → sha256 校验不匹配即拒绝执行 →
`~/.cache/pilotdroid/<tag>/` 缓存 → `os.execv` argv 透传。失败路径全给
可行动错误（支持平台列表/手动下载路径/网络检查）。

**验证**：
- `python -m unittest discover -s python/tests -q` — **5/5 PASS**
  （mock `_fetch`/`os.execv`：下载+校验+缓存命中零二次下载、sha256
  mismatch 拒绝且不写缓存、asset 不在 SUMS、网络失败可行动、未知平台列表）
- **端到端真链**：draft asset 预置 `~/.cache/pilotdroid/v0.6.0/` →
  `cli_bootstrap.main()` → cache hit → `os.execv` 真实二进制 → 输出
  `pilotdroid-cli 0.6.0`。下载支路仅差 release publish 后 URL 公开，
  由 `install-smoke.sh` 兜底端到端验。
- CI `python` job 加 unittest step + `command -v pilotdroid-cli` 断言。

## 2. server.json schema 合规（修 p3/M14 自述 bug）

M14 草稿用 snake_case，官方 `2025-10-17/server.schema.json` 全
camelCase 且 `registryType`/`identifier`/`transport` 为 Package 必填、
`description` maxLength=100、`packageArguments` 必须 typed object
（Positional/Named）。本轮全部修正并**用官方 schema 实跑
`jsonschema.validate` PASS**（desc 84 字符）。`docs/mcp/SUBMIT.md`
更新：剩余唯一 blocker=PyPI 存在性；附一行复验命令。

## 3. README 可视 demo

`docs/assets/live-view.gif`（1.47MB，480px，45 帧@8fps，真机实捕获：
media 通道 stream_start→read_frame→ffmpeg，中途 inject_swipe 制造
翻页动效）。README 顶部 `## Demo — live frame stream` 段内联引用，
与 live-view.py/`stream_read` 工具互指。"帧流"叙事第一次有一眼可见
证据（第 5 次被点名的缺口）。

## 4. 用户侧剩余（不变）

PyPI pending-publisher（`pypi.yml`/`env pypi`）→ publish draft →
`install-smoke.sh ba0d072d`（publish 后唯一真端到端）→ branch
protection（rust/python/agent-jar-sync/agent-jar-repro）。tap sha256
已在 M14 预回填，publish 后零追加。

## 5. verify-agent 环境归因新发现（t5）

连续复跑出现两族随机 FAIL，诊断锚点落盘后实锤**同一根因——MIUI
运行中锁屏/pocket 幕布**：M2 家族（query_found..query_pkg 7 连挂）时
`[m2-setup]` 诊断显示 `mCurrentFocus=android` + a11y 树仅剩
`package="android"`（keyguard/pocket curtain 特征）；同一幕布期
`takeScreenshot` 返回空 → shot_jpeg/quality/scale 随机挂（
`bytes=0`，agent.log 无 `takeScreenshot null`，非 agent 缺陷）。

已有 `screen_off_timeout=INT_MAX` 兜底仍不够（MIUI pocket 幕布是
proximity 触发非超时）。verify-agent.sh 三处硬化：
①M2 setup 锚点升级为"launch→focus→probe query→relaunch×3"，且每轮
先 `KEYCODE_WAKEUP + wm dismiss-keyguard + KEYCODE_BACK`（BACK 退出
pocket 幕布，普通屏无害）；②shot 断言 `shot_file`/`shot_stat`
有界重试 8×1s + 失败路径打印最后一次调用输出；③probe 失败时自动
dump focus+tree pkgs 作 post-mortem。硬化后 **37/37 复跑全绿**。

另：adb_client 4.x marker 包装回归修复——`&` 结尾命令（agent start
的 `nohup ... &`）与 `;` 追加产生 `&;` 语法错，改 subshell
`( {cmd} ); printf MARK $?`（p3 复核实锤+on-device 验证），补
`marked_shell` 单测钉住包装形状；`pilotdroid-cli agent start`
实机复跑成功（agent listening + verify A7 stop_exit_clean 过）。

## 5b. shot_scale 确定性失败根因（t6 复核挖出）+ 修法（t7）

t5 后 verify 在 ba0d072d 仍 36/1×3（p3 复核连挂同一断言，非幕布复发）。
**真根因**：M2-A11 先 `text("pdtest")`→搜索框建议列表弹出→M2-A12
batch 的 tap(700,1400) 点开**被 MIUI 应用锁的 app**→
`applicationlock.ConfirmAccessControl`（FLAG_SECURE）→SHOT 设备端报错
→agent 回 `{code,message,error}` payload→`agent_client.py` `p["data"]`
KeyError→0 字节×8 重试全挂。手动复现链全程实证（focus=ConfirmAccessControl
+shot KeyError 'data'；BACK+HOME 后 shot 57KB 恢复）。

修法（t7）：`batch_results` 断言后 `input keyevent KEYCODE_BACK`（点开
什么退什么，未点开时退出搜索页无副作用——后续断言不依赖其前台）+
`agent_client.py` shot cmd 把 error payload 打成可读
`shot failed: <code> <message> <error>` 替代裸 KeyError。
复跑 **37/37×2 稳定**。应用锁环境因子至此有了第四种形态：会咬
verify 自己的 tap 落点（SUPPORTED.md 第三条已覆盖语义，本条为实证）。

## §6 release-chain note (M16-t1)

- `v0.6.0` tag **kept** (baf57b90) as the M13 code snapshot — its draft
  release was deleted; the published line continues from `v0.6.1`
  (19a0c9c) which carries M14/M15/M16 (lazy-bootstrap, adb_client 4,
  env fallback, verify fixes). Do not publish v0.6.0.
- v0.6.1 draft: 7 assets (whl+sdist+3 CLI+jar+SHA256SUMS), notes =
  `docs/release-notes-v0.6.1.md`.
