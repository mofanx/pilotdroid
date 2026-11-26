# MCP registry submission notes

`server.json` is submitted to the official MCP registry
(`mcp-publisher` / `modelcontextprotocol/registry`) as
`io.github.mofanx/pilotdroid`. The registry is a snapshot — it does not
follow releases, so `mcp-publisher publish` must be re-run after every
release once the new version is visible on PyPI.

## Status

- **Schema**: validates clean against the official
  `2025-10-17/server.schema.json` (checked with python `jsonschema` —
  camelCase fields, `description` ≤100, typed `packageArguments`).
  Re-check after any edit:
  ```bash
  python3 - <<'PY'
  import json,urllib.request,jsonschema
  s=json.load(urllib.request.urlopen("https://static.modelcontextprotocol.io/schemas/2025-10-17/server.schema.json"))
  jsonschema.validate(json.load(open("server.json")), s); print("OK")
  PY
  ```
- **uvx resolution**: the wheel ships a `pilotdroid` console script alias
  for `cli_bootstrap.main` so the registry's `uvx pilotdroid serve`
  resolves (identifier==entry-name requirement). `pilotdroid-cli` also
  exists for humans.
- **CLI delivery**: `pilotdroid`/`pilotdroid-cli` console scripts lazy-
  download the matching release binary on first run (sha256-verified
  against SHA256SUMS.txt, cached under `~/.cache/pilotdroid/<tag>/`).
  **Cold-start caveat**: the first `uvx pilotdroid serve` (per version
  tag) downloads a ~MB binary — expect a few seconds before the MCP
  `initialize` responds on a cold cache; later runs exec instantly.

## Republish steps (every release)

Bump both `version` fields in `server.json` to the new release, wait
until that version is visible on PyPI, then:

```bash
# mcp-publisher is a Go binary from modelcontextprotocol/registry
# releases (NOT on PyPI — `pipx install` will not find it):
REL=$(gh release view -R modelcontextprotocol/registry --json tagName -q .tagName)
curl -sL "https://github.com/modelcontextprotocol/registry/releases/download/${REL}/mcp-publisher_linux_amd64.tar.gz" | tar xz
./mcp-publisher login github -token "$(gh auth token)"
cd docs/mcp && /path/to/mcp-publisher publish   # reads server.json
```

Note: `mcp-publisher login github` uses the device-code flow; if it
stalls, `mcp-publisher login github -token "$(gh auth token)"` works
as an alternative auth source (registry JWT is stored locally and
expires — re-login when publish returns 401).

Registry docs: https://github.com/modelcontextprotocol/registry
