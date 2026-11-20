# MCP registry submission notes

`server.json` is a draft for the official MCP registry
(`mcp-publisher` / `modelcontextprotocol/registry`). **Do not submit yet.**

## Blockers, in order

1. **PyPI first** — the `name` is `io.github.mofanx/pilotdroid` (github
   namespace auth) but the `packages[]` entry points at `pypi/pilotdroid`,
   which only exists after the v0.6.0 release is published (pypi.yml OIDC
   pending-publisher flow).
2. **The whl cannot launch `serve` today.** `uvx pilotdroid serve` fails:
   the wheel ships `pilotdroid-agent` (a Python client entry), not the
   Rust `pilotdroid-cli` binary. Before the registry accepts this file
   we need one of:
   - ship a `pilotdroid-serve` console script in the whl that locates or
     bootstraps the CLI binary (per-platform wheels or a download shim), or
   - publish an `mcpb` bundle packaging the binary, or
   - switch `registry_type` to `oci` and ship a container image.
   The current file is written for option 1 (`package_arguments: ["serve"]`
   assuming a future console script forwards to the embedded/downloaded
   binary).

## Submission steps (once unblocked)

```bash
pipx install mcp-publisher  # or use the bundled CLI
mcp-publisher login github
cd docs/mcp && mcp-publisher publish   # reads server.json
```

Registry docs: https://github.com/modelcontextprotocol/registry
