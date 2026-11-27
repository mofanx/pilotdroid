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
- Rebuild before real-device re-verify: `cargo test` compiles test
  binaries only — `serve`/`setup`/benchmark exec `target/debug/pilotdroid-cli`.
  After editing `serve.rs`, run `cargo build -p pilotdroid-cli` first or
  the re-verify exercises the stale binary (t6 methodology nit).
- Two build paths, pick per need:
  1. **local** — `cargo build -p pilotdroid-cli` (or `--release`); required
     for unpushed changes, costs local disk (`~/.cargo-target`).
  2. **GitHub Actions** — every push to main builds a release-profile
     linux-x86_64 binary and uploads it as the ci artifact
     `pilotdroid-cli-linux-x86_64` (14d retention). Pull it with
     `spike/e2e/fetch-ci-cli.sh` and point the harnesses at it:
     `export PILOTDROID_CLI="$(spike/e2e/fetch-ci-cli.sh)"` — loop.py /
     benchmark.py / loop_v3.py / ghost_repro.py / inspector.py /
     t6_verify.py all honor the override. The artifact only exists for
     commits CI has already built, so unpushed work still needs path 1.
- Fake-adb relay fidelity (facade tests): when you extend the
  `AdbTransport` stub's `forward` relay, propagate half-close like real
  adb — after each `io::copy` ends, `shutdown(Both)` the *other*
  direction's stream. A dropped conn whose fd is still held by a
  `try_clone` shows the client a half-open socket (read timeout) instead
  of FIN — that race produced the `dead_agent_recovery_exhausted`
  Timeout-vs-Io CI flake (fixed db62975).
