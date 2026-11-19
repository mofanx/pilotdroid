# M3 arbitration experiments (ba0d072d + 11.0.0.2:5555)

Data: `m3-experiments.json` (this dir). Runner: `m3-experiments.py`.
Agent 0.2.0 with `take_ns`/`enc_ns` split fields and `backend` force.

## E1 — SHOT quality × scale grid: encoding fixed-cost dominates

n=20 per cell, Settings screen.

| cell | e2e p50 | take p50 | enc p50 | bytes |
|---|---|---|---|---|
| s1.0 q30 | 126.8 | 21.3 | 95.2 | 40.8K |
| s1.0 q50 | 130.7 | 26.7 | 105.0 | 44.5K |
| s1.0 q80 | 131.2 | 25.6 | 101.9 | 54.4K |
| s0.5 q30 | 129.3* | 88.3* | 39.4 | 28.7K |
| s0.5 q80 | 150.6* | 92.1* | 32.8 | 54.3K |

*s0.5 = isolated rerun; the in-grid first cells ate GC from the s1.0 run
(take p50 125ms). take_ms is jittery in general (20–120ms).

**Attribution**: `enc` scales with *pixels* (~100ms @ full res → ~35ms @ half),
**not** with the quality knob (q30 vs q80 ≈ same ms; only bytes differ, 40K
vs 54K at s1.0). `takeScreenshot` itself is a jittery 20–120ms fixed cost.
So e2e ≈ 130-150ms regardless of quality → **fixed cost dominated; the
`quality` param buys bytes only (~zero e2e gain).** Real lever for shot
latency = pixel count (scale) or the M3 media channel, not quality.

## E2 — backend A/B: IM is ~21ms faster device-side — keep IM default

n=50 tap each.

| backend | dev p50 | e2e p50 | e2e p95 |
|---|---|---|---|
| im | **2.57ms** | 6.6ms | 47.3 |
| uia | 23.9ms | 66.9ms | 73.8 |

Confirms spike-2 (im 1.5-4ms / uia 25-49ms). `backend` force field works for
attribution; **do not change the default** — UIA stays fallback-only for the
OEM-revoked-INJECT_EVENTS case (duchamp). Written into docs/03 §3.

## E3 — keep-alive does NOT kill the jitter tail (negative result)

n=30. cold = ping after ≥2s idle; warm = ping immediately after a ping.

| path | p50 | p95 | max |
|---|---|---|---|
| cold | **3.16ms** | 7.2 | 7.3 |
| warm | 42.9ms | 47.1 | 48.1 |

Inverted vs hypothesis: the ~43ms tail is **not** USB/adbd sleep — it's a
scheduling/batching quantum that hits *consecutive* requests. After ≥2s
idle the first op is always fast (~3ms); back-to-back ops land ~50/50 in
the ~43ms bucket. Implications: (a) keep-alive is useless-to-harmful for
single-op latency; (b) the fix for sequential latency is BATCH (already
shipped — ops inside a batch pay it once), or spacing ops ≥~50ms; (c) the
bimodal p50s we see in bench are this quantum, not transport noise.

## E4 — second device row (11.0.0.2:5555, TCP)

2602BRT18C · Android 17 (SDK 37) · 1280x2772 · arm64-v8a · **caps=95 ·
inject=im** — full M2+M3 surface works on SDK 37 incl. KeyEvent path.

| op | p50 | p95 |
|---|---|---|
| ping | 16.7 | 57.6 |
| tap | 22.4 | 29.1 |
| dump | 23.1 | 60.1 |

TCP transport RTT ~15ms higher than USB but well under the adb-forward
bimodal tail; agent capabilities identical.
