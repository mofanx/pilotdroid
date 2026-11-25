# Publishing pilotdroid — release runbook

Step-by-step for cutting a release. Version-generic — substitute
`<VER>` and the current draft id. Written during the v0.6.1 cycle
(draft `453736770`, tag `6acb534`); concrete ids in this file are
worked examples from that cycle.

## Pre-publish checklist (verified during the v0.6.1 cycle)

- [ ] `git rev-parse v<VER>` points at the intended release commit —
      (v0.6.1's baseline was `6acb534`). This does **not** have to equal
      main HEAD: only commits touching asset content
      (`crates/`/`python/`/`compat/`/`agent/`/`Cargo.*`) require the
      tag to follow (tag-follow rule — see CONTRIBUTING.md
      "tag-内容跟随规则"); docs/workflow/test-script commits do not.
- [ ] Draft has exactly the expected assets (7 assets for v0.6.x — linux/macos/
      windows binaries, whl, sdist, agent.jar, SHA256SUMS.txt).
- [ ] **No stale same-tag drafts** — `gh release list` should show ONE
      draft per tag. `gh release download` silently picks the oldest;
      delete extras (`gh api -X DELETE repos/.../releases/<id>`).
- [ ] Draft body is the real notes (not auto-generated dependabot
      list): `gh release view v<VER> --json body` should contain the
      `docs/release-notes-<tag>.md` content. If a retag rebuilt the
      draft after notes were applied, re-run
      `gh release edit v<VER> --notes-file docs/release-notes-<tag>.md`.

## The five user steps

### 1. PyPI pending publisher (one-time, kills the name-squat window)

PyPI → Account settings → Publishing → "Add a new pending publisher":

| field          | value          |
|----------------|----------------|
| PyPI name      | `pilotdroid`   |
| Owner          | `mofanx`       |
| Repository     | `pilotdroid`   |
| Workflow       | `pypi.yml`     |
| Environment    | `pypi`         |

The `environment` field is **required, exact-match** — `pypi.yml` sets
`environment: pypi`, so the OIDC token carries an
`environment: "pypi"` claim; leaving it blank or using another value
makes the token exchange fail with 403 at publish time. Optionally
pre-create the GitHub `pypi` environment with a required-reviewer rule
if you want a second human gate (the draft review below is already
one); GitHub auto-creates the env on first run otherwise.

### 2. Publish the draft

Releases → the v<VER> draft → verify body shows the full notes (not a
dependabot changelog) → **Publish release**. This fires
`.github/workflows/pypi.yml` (`release: [published]`).

### 3. Wait for pypi.yml + index propagation

`gh run watch` the pypi run to `success`, then wait for the PyPI index
to show the version (seconds to minutes). `install-smoke.sh` step 0
already polls `pip index versions pilotdroid` (12×5s) before
installing — just don't start it before the workflow is green.

### 4. Post-publish smoke

```
spike/e2e/install-smoke.sh ba0d072d
```

Covers: `pip install pilotdroid==<VER>` in a venv → `pilotdroid-cli
--version` → lazy bootstrap download + sha256 verify + cache → direct
release-binary check against SHA256SUMS.txt.

### 5. Branch protection

GitHub → Settings → Branches → protect `main`, required status checks:
`rust`, `python`, `agent-jar-sync`, `agent-jar-repro` (optionally
`codeql`).

## Follow-ups after publish

- **Required: bump the Homebrew tap** — `mofanx/homebrew-pilotdroid`
  `Formula/pilotdroid.rb`: `version`, both `url`s (v<VER>), and both
  `sha256`s from the published `SHA256SUMS.txt`, then commit+push.
  Skipping this leaves `brew install/upgrade` pinned to the previous
  release (it stayed on 0.6.1 until manually bumped after v0.6.2
  shipped).
- **Step 6 (post-publish): MCP registry submission** — follow
  `docs/mcp/SUBMIT.md`. Prerequisite: `pilotdroid==<VER>` must be
  visible on the PyPI index (step-3 polling covers this; the registry
  resolves `uvx pilotdroid` live). SUBMIT.md intentionally says
  "Do not submit yet" until then — including its cold-start caveat.
  The registry entry is a **snapshot, not a follower**: bump
  `docs/mcp/server.json` to the new version and re-run
  `mcp-publisher publish` on **every** release — skipping this leaves
  the registry pinned at the previous version (as it stayed on
  `0.6.1.post1` after that one-off post release).
- **Required: bump the install-smoke default** —
  `spike/e2e/install-smoke.sh` `VER="${PILOTDROID_SMOKE_VER:-<VER>}"`
  to the version just shipped (one line; also refresh the stale
  header comment if it names an old cycle). Skipping this leaves
  the distribution smoke testing the *previous* release by default
  (it stayed on 0.6.2 after v0.6.3 shipped — caught in the M25
  read-only review).
- Housekeeping: remove stale worktrees (`pilotdroid-m10dist` is
  outdated — `git worktree remove`), check `df -h` (disk was at 97%).
