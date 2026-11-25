# pilotdroid v0.8.0

Minor release — the AI-native closed-loop batch. New feature surface:
element **state fields** (`checked`/`selected`/`focused`/`enabled`) +
a `verify_after` state predicate, the **`watch_dead` notification**
(watcher silent death is now observable), an optional
**`describe_screen ocr:true`** tier, and the recovery-UX trio that
landed after v0.7.1 (`session_recovered` push + armed auto re-arm +
stream-period stash→push).

## Version skew (explicit)

**CLI/Python `0.8.0` ships agent.jar `0.7.4`** — the embedded dex
agent version does not equal the CLI version. `watch_dead`,
watcher hit stash/replay, and the pendOut ghost fix require agent
≥0.7.4 / ≥0.7.3 respectively; the server reports the real agent
version in `hello` and degrades gracefully on older jars. Deployed
agent upgrades ride along with `agent install` / `setup`.

## What's new

- **Element states** — `list_elements`/`describe_screen` emit
  `checked`/`selected`/`focused` when true and `enabled` when false
  (dump-attr passthrough, token-frugal); `class` is now the **simple
  class name** everywhere (was mixed full/simple before).
  `verify_after` gains `expect.state`
  (`{match:{text|desc|class|i}, checked|selected|enabled|focused:bool}`)
  so checkbox/switch/radio loops are provable — "tap ok" vs "effect
  landed" are now separate assertions. The blind-driving B4
  checkbox task that failed in the v0.7.1-era eval (state not
  observable) now **passes**.
- **`notifications/pilotdroid/watch_dead`** — an armed watcher that
  dies abnormally (poll `Throwable` / session-kill) emits a terminal
  `{type:"watch_dead"}` event + log line, surfaced by serve as a
  notification **with bounded auto re-arm** (≤2 per rule per epoch).
  Also: watcher hits/toasts are always stashed into a ≤8-entry/15s
  replay ring and replayed to late-attaching conns — the "single
  write landed" ghost-absorption hole is closed
  (protocol unchanged: new payload `type` on the existing ui_event
  slot).
- **`describe_screen ocr:true`** (optional tier) — host-side
  tesseract TSV → line blocks → geometric fusion: text fills
  `ocr_text` on matching empty nodes, unclaimed blocks become
  indexable `ocr_only` elements (tappable center, ≤40 blocks by
  confidence for token budget). No tesseract → clean degrade with
  `ocr.available:false`, never an error.
- **Recovery UX trio** — `session_recovered` notification on epoch
  change (crashes no longer silent); armed watchers auto re-arm on
  the new session (crash→lose upgraded to transparent self-heal);
  during an active stream, stashed hits/toasts drain to push
  notifications instead of pausing.

## Fixed

- `watcher.when().click()/call()` `max_fires` now requires int ≥1
  (`1.5` previously passed validation).
- Stale/unknown `watch_disarm` no longer passes through to the
  agent — id-space collisions could kill an innocent re-allocated
  watcher after recovery.
- Agent watcher ghost-conn race (0.7.2→0.7.3): pend conns held in a
  bounded ≤4 set with broadcast + corpse culling instead of
  newest-conn-wins — backlog ghosts can no longer evict a live
  watch reader; zero-delivery hits stash for the next attach.
- Watcher silent death (0.7.3→0.7.4): see `watch_dead` above.

## Honest limits (unchanged + noted)

Same as `compat/SUPPORTED.md`: clipboard ❌ (ROM-proven
unreachable), screenrecord ⚠️ (ROM-throttled), xpath is an
approximation, no blanket fps guarantee, `watchConns` cap = 4 /
replay ≤8/15s, OCR is English-quality-tier + host-side (~1.5–2s
per frame — not for tight loops), accessibility `checked`
propagation can lag ~0.5–4s on MIUI (poll, don't single-read).
Multi-device matrix: V816 (API37) verified setup+describe only —
it's a daily-use phone, eval intentionally runs on the USB test
device.

## Verified

- `verify-agent.sh ba0d072d` → **44/0** incl. `watch_dead_event` +
  `watch_hit_stash_replay` (replay-aware assertions).
- `ghost_repro.py --watch` 5/5, `--watch-dead` 3/3.
- eval v3 (`loop_v3.py`, 9 tasks × 3 rounds incl. randomized
  index/checkbox tasks + foreground-pkg failure attribution) →
  **27/27** on ba0d072d (MIUI/API34); V816 (API37) setup 8/8 +
  agent 0.7.4 deploy + describe verified.
- Blind-driving B4 (describe→tap→`state.checked` verify) → PASS.
- `cargo test --workspace`, pytest 86, doc-drift 0 fail;
  u2 vendored suite unchanged (22/0/8).
