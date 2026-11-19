# RESULTS-M4 — media-channel frame stream e2e acceptance

Device: `ba0d072d` (Mi 11, Android 14 / MIUI, 1440×3200) · agent 0.3.0 · host `feature/m4-e2e` = m4-agent@8b201ea + m4-host@3cfaa8e merged.
Date: measured live, all numbers real-device.

## 1. Sustained stream (acceptance: ≥5fps @ scale=0.5, n≥60s)

`agent stream --duration 65 --fps 10 --scale 0.5 --quality 70` (Rust host, real wire):

| metric | value |
|---|---|
| frames | 391 in 65.009s |
| **effective fps** | **6.02** (req=10 → saturated at take+enc ceiling ~166ms/frame) |
| bytes | 30,354,505 (~3.74 Mbps, ~77.6 KB/frame @ 720×1600) |
| seq_gaps | 0 |
| dropped | 0 (frames_sent==received==391) |

Per-second curve (python client, 30s twin run): steady 5–6 fps/sec bucket,
take p50 ~115ms + enc p50 ~66ms device-side → ~181ms/frame production cost,
identical to the M3-E1 finding that SHOT is fixed-cost dominated (takeScreenshot).
JPEG quality/scale move bytes, not latency.

## 2. Amortization vs single-shot SHOT (same scale=0.5 q=70)

| path | per-frame cost | ceiling | notes |
|---|---|---|---|
| SHOT request/poll | **197.2ms e2e p50** | 5.07 fps | each call = fwd+req+capture+enc+resp |
| STREAM | 166ms/frame | 6.0 fps sustained | one connect, push model, media socket |

- Stream delivers **+18% fps over the poll ceiling** at identical scale/quality —
  the per-request round-trip (HELLO-less but still 2×RTT+scheduling) is amortized.
- Bandwidth: 77.6KB/frame vs ~420KB full-res SHOT → **5.4× less** for live-view use.
- 391 frames via SHOT polling ≥ 77s best-case; stream did 65s **with control free**.

## 3. Control channel NOT blocked during stream (acceptance)

Second control session while stream active: `tap` ×10 → **p50 5.95ms**
(4.0–7.7ms steady, first-touch 15.3/51.2ms = conn warmup);
5-tap BATCH = 16.4ms. Unchanged vs no-stream baseline (~5ms amortized).
Producer→queue(1)→writer isolation holds: media conn slowness can only drop
frames (dropped counter), never stall control.

## 4. Cleanup (acceptance: stop → no residue)

Post-stream `verify-agent.sh` = **27/27 PASS** — control socket, media socket,
producer/writer threads all released (stream_eof asserts agent-side close;
repeated start/stop cycles in the 6-assertion M4 block all green).

## 5. Contract conformance spot-check

- `start` ack: `{ok:true, socket:"pilotdroid_agent_media", effective_fps_hint}` ✓
- busy: second-session start → `ERR_STREAM_BUSY` ✓
- stop ack: `{ok:true, frames_sent, dropped}` ✓
- session binding: owner control disconnect = implicit stop (verified — earlier
  "0 frames" false alarm was one-shot client disconnects, not an agent bug)
- media conn after stop → EOF ✓

## Data files

- `m4-stream-agent.json` — agent-side take/enc/gap percentiles (20s × 2 runs)
- `m4.json` — this acceptance run (host CLI output + concurrent-ops + shot baseline)
- `soak-m4.sh` / `soak-m4.jsonl` — night soak (see soak-report.md)

## Known limits (for roadmap)

- Ceiling ≈6fps @0.5 on Mi11: takeScreenshot ~110ms + JPEG ~65ms serial per frame.
  Higher fps needs hardware encoder / SurfaceFlinger capture — documented ADR-006
  rejected-alternatives territory.
- fps_hint is best-effort: req 10 → 6.0 actual (producer saturates honestly,
  drops stay 0 as long as reader keeps up).
