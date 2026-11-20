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
