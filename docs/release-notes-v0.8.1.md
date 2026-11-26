# pilotdroid v0.8.1

Patch release — fixes a shipped semantic defect in
`verify_after.expect.state.match.i` and closes the last
describe→tap numbering trap, plus internal eval-harness fixes that
produced the first cross-OEM SoM-contract numbers.

## Version skew (explicit)

**CLI/Python `0.8.1` still ships agent.jar `0.7.4`** — the embedded
dex agent version does not equal the CLI version. All fixes in this
patch are host-side; no agent redeploy is required beyond the
0.7.4 jar already shipped with v0.8.0.

## What's new

- **`describe_screen` elements now carry `i`** — each element emits
  its interactive-space index (`i<n>` in the text map, `"i"` in the
  JSON) alongside the SoM `id`/`#n`. The `i` field shares the exact
  numbering `list_elements`/`tap_index`/`set_text_index`/
  `verify_after.match.i` consume (`clickable‖editable‖scrollable`
  counted over ALL tree nodes, including unlabeled interactive
  nodes like a bare EditText). Blind loop: `describe_screen` → take
  element `i` → `tap_index {i}` — no more cross-space guessing.
  `id`/`#n` remains the SoM space (screenshot annotation +
  `tap{x,y}` by center); `ocr_only` elements have no `i` (tap by
  center, no stale-guard). `compat/som.py` mirrors the same field.

## Fixed

- **`verify_after.expect.state.match.i`** — previously resolved `i`
  in the FULL-TREE index space and short-circuited the other match
  keys: a shipped semantic defect (wrong element could match, or
  none). Now `i` resolves in the same interactive space
  `list_elements` returns, and ANDs with `text`/`desc`/`class`.
  Text/desc/class-only matches still search the full tree
  (disabled/non-interactive nodes are legal assertion targets).
- **Docs**: `README` inspector-loop recipe corrected to consume the
  element `i` field; `compat/SUPPORTED.md` gains a three-space
  numbering table (SoM `id` / interactive `i` / full-tree index)
  and the earlier "same space" mis-statement about describe_screen
  is corrected; `docs/adr/008` gains a HarmonyOS cross-ROM row
  (see below).

## Internal / eval evidence

- **eval v3 fixes**: foreground-package gate on
  `random_index_pick`/`_settings_list` (external takeover like
  `com.miui.home` is recovered before the task body; mid-task
  takeover retries once); the IME-suppression pin now applies only
  to the non-portable suite and prints its restored baseline.
- **Portable subset reworked cross-OEM** (Huawei-driven): honest
  SoM contract assertions, `text or desc` labels, three-way
  navigation proof, toggle/input widgets excluded from random
  picks, live-list (WiFi-scan) race handling.
- **Multi-device matrix**: `CFE6R21525008201` HUAWEI MRR-W29
  (HarmonyOS-based Android 12 / API 31, USB) — `setup` 8/8, agent
  0.7.4, `loop_v3 --portable` **15/15**; `ba0d072d` (MIUI/API34)
  regression **14/15** (one honest no-nav fail: a Settings row that
  expands in place). V816/API37 remains blocked (device offline,
  user-banned address untouched).
- **Stream cross-ROM data point** (ADR-008 revised): HarmonyOS
  API31 uia path measured ceiling **10.28fps** (cap 63ms + enc 31ms
  — the `1000/(cap+enc)` model holds on both devices). Revised
  conclusion: MIUI is the worst tested (5.6fps) but not the only
  wall; the uia path tops out 5.6–10.3fps across tested ROMs and
  neither reaches 15fps. Note: capture→encode pipelining would
  theoretically reach ~15.9fps on HarmonyOS (vs ~8.9 on MIUI).
- **MCP Registry** verified healthy via the corrected curl path
  (search `limit` truncation had made a stale deprecated entry look
  like the only one): `0.8.0` active/`isLatest`, republished for
  `0.8.1` along this release. PUBLISH.md gains the registry-verify
  command + a "shipped-defect → patch window" note.

## Honest limits (unchanged)

Same as v0.8.0 notes + `compat/SUPPORTED.md`: clipboard ❌,
screenrecord ⚠️ ROM-throttled, host-side xpath is an approximation,
no blanket ≥15fps guarantee (5.6–10.3fps measured across two
retail ROMs), `watchConns` cap = 4 / replay ≤8/15s, OCR is
English-quality-tier + ~1.5–2s/frame, a11y `checked` propagation
lags ~0.5–4s on MIUI. Newly noted: `describe_screen` `id`/`#n`
(SoM/document-order) and `i` (interactive space) are two different
numberings on the same element — use `i` for `tap_index`, `id` only
for center-tap/annotation lookups.

## Verified

- `cargo test --workspace` → all green (serve suite incl.
  `verify_after_state_match_i_interactive_space`,
  `i_maps_to_list_elements_space`); fmt + clippy clean; pytest 94.
- Real-device: `verify_after match:{i,text}+checked` flip PASS,
  `i`+wrong-text → `found:false`, i-only read PASS (ba0d072d);
  `describe[i]` ↔ `list_elements[i]` 19/19 aligned.
- `loop_v3 --portable`: HUAWEI API31 15/15, MIUI API34 14/15.
