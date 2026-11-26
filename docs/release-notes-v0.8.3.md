# pilotdroid v0.8.3

Patch release — **blind-driving correctness + the last silent
wrong-tap entry point closed**. Headline: the stale guard could
collide on screens with identical geometry but different content,
letting `tap_index{i}` silently hit whatever re-occupied the slot;
plus `screenshot_annotated` now returns the same-dump elements array
so an image label resolves straight to `tap_index` — no second
describe, no id-vs-i trap.

## Version skew (explicit)

**CLI/Python `0.8.3` still ships agent.jar `0.7.4`** — all changes are
host-side; no agent redeploy needed beyond the jar already shipped
with v0.8.0.

## Fixed (shipped-defect class)

- **Same-geometry snapshot collision → silent wrong tap** —
  the layout signature behind the stale guard deliberately ignores
  `text`/`desc` (a ticking clock must not invalidate a snapshot),
  so two screens with identical tree shape but different labels —
  e.g. ApiDemos main list → App sublist, same ListView, same bounds
  — hashed identically. `tap_index{i}` then passed the guard and
  tapped whatever now sat in that slot, returning `ok:true` for a
  wrong action. The guard now verifies the **target element's
  identity** (`class`+`text`+`desc`) from the same fresh dump —
  zero extra dumps. Caught by the new frozen benchmark's
  `stale_layout_guard` task on its first run, not by a user report.
  - **New semantic boundary (honest)**: an element whose `text`
    legitimately changes between observation and tap — countdowns,
    live counters, renaming labels — now reports `element {i}
    changed since {src}` instead of tapping. This fails **closed**
    (a rejected action is recoverable; a silent wrong tap is not).
    If you hit this, re-`describe_screen` and retry with the new `i`.
  - **`verify_after.match.i` is unaffected**: it resolves `i`
    against a fresh same-call dump, never the snapshot.
  - **`ocr_only` elements have no stale guard by construction**:
    they are host-synthesized (OCR overlay) with no a11y node to
    re-verify against. Tapping an OCR index on a changing screen
    relies on the caller re-observing; this is unchanged and is
    stated so reviewers don't rediscover it as a new hole.

- **`screenshot_annotated` image labels → `tap_index` direct** —
  the response now carries the same-dump `elements` array, each
  element with both `id` (the SoM number printed on the image) and
  `i` (the interactive index `tap_index` consumes). Previously the
  image was labelled in SoM `id` space while the armed snapshot
  consumed `i` space — feeding the visible label straight to
  `tap_index` silently tapped a different element whenever the two
  spaces diverged (any screen with non-interactive nodes in front).
  The tool description also carries an inline `id`-vs-`i` warning.

## Fixed (eval / hygiene)

- `d(scrollable=True).scroll.to(**kw)` implemented (was NIE):
  forward-scrolls until the selector target exists or the dump stops
  changing; empty kwargs rejected.
- `verify-agent.sh` `shot_quality`: q30-vs-q80 byte ordering can
  invert when screen content drifts between the two SHOTs — now
  retries up to 3 back-to-back pairs.
- Eval harness poisoned-IME baseline: a SIGKILLed run leaves
  `default_input_method` pinned to `pilotdroid.eval/.NoIme`; the next
  run captured that pin as the "original" and restored it,
  cementing contamination. Now detected at capture — restore is
  skipped with a warning.
- `Serve.close()` (agent-loop harnesses) removes the adb forward
  rules it created — every spawned serve used to leak one rule.
- `u2shim.dump_hierarchy()` retries empty/truncated dumps (a11y tree
  momentarily empty during ROM screen transitions) — up to 3
  attempts.
- `loop_v3.py` empty-selection (`--portable` task named without the
  flag) now exits 2 with a usage hint instead of `ZeroDivisionError`.
- `benchmark.py` cross-OEM portability (CFE6R HarmonyOS row → 11/11):
  `com.miui.calculator` → `com.android.settings` drift injection,
  observable-transition waits, old-targetSdk modal dismissal,
  scroll→nav fallback for non-scrollable lists.

## Added

- `spike/e2e/agent-loop/benchmark.py` — frozen blind-driving
  benchmark v1 (11 tasks, deterministic targets): describe→tap
  atomicity, i-space divergence, B4 checkbox+state predicate, stale
  guards (layout+pkg), scroll, set_text, state predicates, toast,
  dirty-kill recovery, match:{} negative pin. First-round results:
  **11/11 on both ba0d072d (MIUI/API34) and CFE6R21525008201
  (HarmonyOS/API31)**, with environmental fg-hijack retries marked
  `env=` — they never mask real fails.

## Compatibility (honest boundaries — unchanged)

- Host-side XPath stays a deliberately limited approximation (see
  `docs/xpath.md` / `compat/SUPPORTED.md`).
- Stream is 4–6 fps on the tested MIUI/API34 target — a ROM ceiling,
  not a client bug (ADR-008).
- Windows is source-build-only (no prebuilt wheel/exe).
- `screenshot_annotated` on multi-display MIUI can return empty
  payloads for the auxiliary display.

## Files

- GitHub Release assets: linux-x86_64, darwin-aarch64 binaries +
  `SHA256SUMS`, `pilotdroid-cli`, `pilotdroid-agent` sdist/wheel.
- PyPI: `pip install pilotdroid==0.8.3` — lazy bootstrap downloads
  the release binary on first `pilotdroid-cli` call.
- Homebrew: `brew install mofanx/tap/pilotdroid`.
- MCP Registry: `pilotdroid` 0.8.3.
