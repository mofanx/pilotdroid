# Soak report — M4 daemon resilience (unattended)

Run: `spike/e2e/soak-m4.sh ba0d072d 90` → 90 min wall clock on `ba0d072d`
(Mi 11 / Android 14 / MIUI), agent 0.3.0, raw data `soak-m4.jsonl` (1395 events).
Pattern: persistent-session bench cycles (mixed ping/tap/query/dump, 50 ops each)
+ `kill -9` every ~5min → watchdog zero-copy restart + RSS/fd sampling per cycle.

## Headline

| metric | result | verdict |
|---|---|---|
| total ops | **69,550** (1391 cycles × 50) | ≥1000 target crushed |
| bench failures | **0** | no crash, no wedged session |
| kill -9 cycles | 17 → recoveries **17/17 (100%)** | watchdog self-heals every time |
| recovery time | p50 **1141ms** (min 1103, max 1156) | zero-copy restart, no --jar needed |
| fd count | **59 constant across all 1391 samples** | zero fd leak |
| RSS | 139.5–160.0MB oscillation | GC sawtooth, **no monotonic growth** |
| op latency | p50 median 52.3ms (range 42.7–57.0), p95 ≤ 91.7ms | no drift/degradation over 90min |

## Notes

- RSS band (~140–160MB) is the Dalvik heap sawtooth — the dump-heavy workload
  allocates transient tree/XML buffers; it never trends upward over 1391 samples.
- fd=59 flat is the strongest leak signal available: each cycle opens+closes a
  control session; media socket attach/release cycles in verify don't leak either.
- Recovery is boringly reliable: every kill reproduced the same ~1.1s
  `stop+start` zero-copy path from M3 watchdog work.
- Op mix p50 ~52ms is dominated by `dump` (the heavy op in the 4-way mix);
  tap-only cycles would sit at the ~5ms baseline.
- Nothing crashed. Nothing leaked. Nothing degraded.

## Reproduce

```
spike/e2e/soak-m4.sh ba0d072d 90    # minutes; runs unattended, jsonl out
```
