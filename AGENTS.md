# pilotdroid agent notes

- Worktrees share this repo but each gets its own `target/` — disk filled twice.
  Convention: export `CARGO_TARGET_DIR` to one shared dir (e.g.
  `export CARGO_TARGET_DIR=$HOME/.cargo-target/pilotdroid`) rather than a committed
  `.cargo/config.toml` — config files live inside each worktree, so a committed
  config cannot share across them. Fallback: `cargo clean` right after merging.
  (Details: CONTRIBUTING.md "worktree/磁盘" section.)
- Verify commands: `cargo test --workspace`, `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`,
  `agent/scripts/verify-agent.sh <serial> <port>`, `pilotdroid-cli -s <serial> setup`.
- Forward hygiene: killed `serve`/bench sessions leak `adb forward` rules —
  `pilotdroid setup` step 6 lists orphans; remove listed ones only.
- Embedded jar: `agent/agent.jar` is committed and compiled into
  `pilotdroid-cli` via `include_bytes!` — `setup`/`agent install` push it
  when no `--jar` is given. After editing `agent/java/pilotdroid/Agent.java`,
  rebuild + redeploy with `agent/scripts/deploy-agent.sh <serial>`, then
  `git add agent/agent.jar`. Stale-jar sync is enforced by verify's
  `agent_ver` assert (HELLO.ver must match AGENT_VER in Agent.java).
- `agent/data/verify-*.json` are regenerated on every verify run — ignored;
  don't commit them.
