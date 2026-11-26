# pilotdroid v0.8.2

Patch release — the observation→action chain is now **atomic**:
`describe_screen`/`screenshot_annotated` arm the index snapshot from
the SAME dump their output describes, closing the last
describe→tap_index TOCTOU window. Plus an explicit error for an
empty `verify_after` state match.

## Version skew (explicit)

**CLI/Python `0.8.2` still ships agent.jar `0.7.4`** — all changes are
host-side; no agent redeploy needed beyond the jar already shipped
with v0.8.0.

## Changed

- **Perception ops fill the index snapshot from their own dump** —
  `describe_screen` and `screenshot_annotated` now populate the
  `tap_index`/`set_text_index` snapshot via `list_elements_from(xml)`
  (same bytes they parsed, no second dump). Consequence:
  `describe_screen → tap_index {i}` works directly on a fresh session
  — and more importantly the `i` ↔ snapshot binding is **atomic**:
  under the old 5-beat recipe (0.8.1's interim fix) `describe.i` came
  from dump A while the snapshot came from dump B, so a live-screen
  re-render between the two calls could silently point `i` at the
  wrong element while the stale-guard still passed. That residual
  wrong-tap class is now eliminated, not just narrowed.
  The inspector recipe is back to four beats; `list_elements` remains
  a valid way to (re)arm the snapshot. Snapshot errors now name
  their provenance (`list_elements`/`describe_screen`/
  `screenshot_annotated`) in stale/out-of-range messages.
- **Interactive predicate single-sourced** —
  `pilotdroid_agent::is_interactive` is the one definition behind
  `list_elements` filtering, `verify_after.match.i` resolution, and
  `describe_screen`'s `i` field (was three copies that could drift).

## Fixed

- **`verify_after.expect.state.match:{}`** with no selector keys now
  errors explicitly ("need one of text|desc|class|i") instead of
  silently returning `found:false` — an assertion that could never
  pass reported as an empty result rather than a usage error.

## Honest limits (unchanged)

Same as v0.8.1 + `compat/SUPPORTED.md`: clipboard ❌, screenrecord ⚠️
ROM-throttled, host-side xpath approximation, no blanket ≥15fps
guarantee (5.6–10.3fps measured), `watchConns` cap 4 / replay ≤8/15s,
OCR English-tier ~1.5–2s/frame, a11y `checked` lag ~0.5–4s on MIUI.

## Verified

- Fresh-session real device: `describe_screen → tap_index {i:0}` →
  tap landed (no `list_elements` first) — the exact recipe a model
  would run.
- `cargo test --workspace` 105/0 (3 new tests pin the snap fill +
  provenance + empty-match error); fmt/clippy/doc-drift clean;
  pytest 94.
