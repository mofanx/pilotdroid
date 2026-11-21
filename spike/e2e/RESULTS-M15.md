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
