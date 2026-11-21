# MCP registry submission notes

`server.json` is a draft for the official MCP registry
(`mcp-publisher` / `modelcontextprotocol/registry`). **Do not submit yet.**

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

## Remaining blocker

**PyPI first** — `name` uses the github namespace but the `packages[]`
entry points at `pypi/pilotdroid==0.6.0`, which only exists after the
v0.6.0 release is published (pypi.yml OIDC pending-publisher flow).

## Submission steps (once unblocked)

```bash
pipx install mcp-publisher
mcp-publisher login github
cd docs/mcp && mcp-publisher publish   # reads server.json
```

Registry docs: https://github.com/modelcontextprotocol/registry
