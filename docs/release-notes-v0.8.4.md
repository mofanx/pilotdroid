# pilotdroid v0.8.4

Patch release — **infrastructure/harness batch; no runtime behavior
change** in `serve`, the agent protocol, or the shim semantics.
Everything here hardens the development loop that produces the
binary: CI daily-build channel, eval-harness forward hygiene, and
documentation gates. Released per the patch-window convention so the
increment doesn't sit unreleased on main.

## Version skew (explicit)

**CLI/Python `0.8.4` still ships agent.jar `0.7.4`** — unchanged since
v0.8.0; no agent redeploy needed.

## Added

- **CI daily-build channel** — every push to main builds a
  release-profile linux-x86_64 `pilotdroid-cli` and uploads it as the
  ci artifact `pilotdroid-cli-linux-x86_64` (14d retention).
  `spike/e2e/fetch-ci-cli.sh` downloads the latest successful run's
  artifact (cached per run-id) and the real-device harnesses consume
  it via the existing `PILOTDROID_CLI` override — so a device-verify
  round no longer requires a local Rust build. Local `cargo build`
  stays the other path and is still required for unpushed changes.
  Cache eviction keeps the newest 5 run-id dirs (~6.5MB each; was
  unbounded).

## Fixed

- **eval-harness `Serve` orphan forwards** — a `kill -9`ed serve
  process never runs `close()`, leaking one `adb forward` rule per
  instance. Each `Serve` now tags its own forward rule(s) with its
  pid; the next `Serve()` start sweeps rules whose tag pid is dead
  (untagged rules — fixed verify ports, manual forwards — are kept,
  matching the "remove listed ones only" hygiene rule).
- **`doc-drift` changelog gate crash** — a missing `## [Unreleased]`
  header (v0.8.3's fold dropped it wholesale) crashed the gate on a
  `split` instead of failing cleanly; it now FAILs explicitly, and
  the match is anchored to the header line so a prose mention can't
  satisfy it. PUBLISH.md documents keeping the empty header when
  folding.

## Notes (honest boundaries — unchanged)

- Runtime semantics are identical to v0.8.3: same-dump snapshot
  filling, element-identity stale guard (text-changing elements fail
  closed), annotated `elements` array, `ocr_only` elements unguarded.
- Host-side XPath stays a deliberately limited approximation; stream
  is 4–6fps on tested MIUI/API34 (ROM ceiling); Windows is
  source-build-only.

## Files

- GitHub Release assets: linux-x86_64, darwin-aarch64 binaries +
  `SHA256SUMS`, `pilotdroid-cli`, `pilotdroid-agent` sdist/wheel.
- PyPI: `pip install pilotdroid==0.8.4` — lazy bootstrap downloads
  the release binary on first `pilotdroid-cli` call.
- Homebrew: `brew install mofanx/tap/pilotdroid`.
- MCP Registry: `pilotdroid` 0.8.4.
