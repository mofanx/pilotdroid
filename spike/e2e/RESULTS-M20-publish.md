# M20 / 发布终局 ledger — v0.6.1 上线实证

## PyPI
- `twine upload`（本地凭据，用户授权）：`pilotdroid 0.6.1` 于
  2026-11-22 上线 `pypi.org/project/pilotdroid/0.6.1/`；404 抢注敞口终结。
- 上传前验证闸（全过才传）：tag=6acb534；`python -m build` whl+sdist；
  unzip/解包双实证 `u2shim.py` 46745B 含 `def settings`/`def watch_context`；
  `twine check` 双 PASSED；draft 唯一+body=curated notes。

## GitHub Release
- `gh release edit v0.6.1 --draft=false`（用户授权自动化）：published
  2026-11-22T01:56:57Z，7 assets 公开可下。
- `pypi.yml` run 44973793253 触发即红——**trusted-publishing exchange
  failure**（PyPI 侧 publisher 未配置），非重复版本错。包已由本地
  twine 在库，无影响；未来若走 CI 上传需在 PyPI 项目页配
  `owner:mofanx/repo:pilotdroid/workflow:pypi.yml/env:pypi`。

## install-smoke（ba0d072d，release 发布后重跑）
- **8 pass / 0 fail**：索引轮询(1 try)→pip install→import u2shim→
  错误提示可行动→**bootstrap 下载→sha256→execv**（0.6.1）→
  对照二进制→cli setup（agent 0.7.0 caps 0x7ff）→serve MCP 握手。
- 首跑曾 4/4：step0 `pip -q index versions` 静默 bug（505f7bf 修）+
  bootstrap 404（draft 态预期）。
- WARN：17932/17933 两条 dev forward 记 orphan，留待确认后清。

## 磁盘
- 97%→82%：清 `.cargo-target/pilotdroid`(3.5G)+`.gradle/caches`(4G)+tmp。
  sessions.db(11G) 未动（活应用状态）。

## 用户五连击完成度
- [x] ①PyPI 上传（本地 twine 替代 pending-publisher 路径）
- [x] ②publish draft（自动化执行）
- [x] ③pypi.yml 结论已知（红，可信发布未配——见上）
- [x] ④install-smoke 8/8
- [x] ⑤branch protection：4 checks+strict=true（gh api PUT 实证返回）
- [ ] ⑥MCP registry 提交（docs/mcp/SUBMIT.md）

## MCP registry（post1 收尾）

- `mcp-publisher`（Go `publisher`）+ GitHub PAT（gh token）登录；
  device-flow 浏览器代点三次 not_found（code 服务端拒）→ 改 `-token` 直登。
- 首 publish 400：registry 要求 PyPI README 含
  `mcp-name: io.github.mofanx/pilotdroid`——0.6.1 不可改 →
  **0.6.1.post1**（PEP440 post-release）：README +mcp-name 行、
  `_tag()` 剥 `.postN`（仍指向 v0.6.1 release 二进制）、
  server.json 两处 version→0.6.1.post1、pytest 7/7、twine 上传。
- `publisher publish` → **io.github.mofanx/pilotdroid@0.6.1.post1
  已上官方 registry**（`/v0/servers?search=pilotdroid` 回读实证）。
