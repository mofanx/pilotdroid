# RESULTS-M5 — agent 原生调用面 e2e acceptance

Device: `ba0d072d` (Mi 11, A14/MIUI) · agent **0.3.1** · main @ merge-train tip.
Second device: `11.0.0.2:5555` (2602BRT18C, Android 17/SDK37) — matrix row added.

## 1. Merge train (m5-spec → m5-agent-shim → m5-host)

- 3 merges into `feature/m5-e2e`; only conflict = `agent/data/verify-*.json`
  data file (resolved: keep newest run). verify-agent.sh merged clean —
  caps=127 assertion, `-t 5000` window, query_class/scrollable all coexist.
- Post-merge: `cargo test` **68 green**, fmt/clippy clean, deploy 0.3.1,
  `verify-agent.sh` **30/30** (new `agent_ver` assertion pins 0.3.1 — no
  stale-jar false greens).

## 2. serve MCP acceptance (real device, real stdio)

- `initialize` → `{serverInfo:{name:pilotdroid,version:0.1.0}, protocolVersion:2024-11-05}` ✓
- `tools/list` → **18 tools** (tap..batch, ADR-007 v1 set complete) ✓
- `tools/call` real ops: `app_current` → `{package:com.android.settings, activity:..MainSettings}`;
  `exists{class}` → true; `tap` → `{ok,backend:im,events:2}`; multi-call stream
  returns ordered by id — serialized queue holds, no frame interleave ✓
- stdout purity: only JSON-RPC lines (banner/log → stderr) ✓
- stream trio: `stream_start` → ack+socket; `stream_read n=4 to_dir` → 4 jpg files
  (file bypass, b64:null in-line); `stream_read n=2` → base64 frames;
  `stream_stop` → `{frames_sent:6, dropped:0}` — lazy attach order honored ✓
- MIUI edge documented: `dumpsys window` emits a leading `mCurrentFocus=null`
  line — both parsers skip it; `{}` during activity transitions is correct.
- **Real MCP client (t6)**: official python `mcp` SDK 1.26.0 stdio_client +
  ClientSession — full handshake PASS: serverInfo `pilotdroid/0.1.0`,
  protocolVersion `2025-11-25` (client-proposed, echoed), caps `{tools}`,
  `tools/list` 18, `call_tool(app_current)` real result, `call_tool(tap)` ok in
  15.9ms, unknown tool → `isError` content. Script: `mcp_client_check.py`.

## 3. Four-path bench (same scenario, n=10/op, wait_finish taps)

| op | cli one-shot | rust lib | serve | shim |
|---|---|---|---|---|
| tap | 139.3 | 54.0 | 54.2 | 51.9 |
| dump | 102.6 | 56.0 | 63.6 | 54.0 |
| exists{class} | — (not on CLI surface) | 47.9 | 49.1 | 48.0 |
| screenshot(0.5) | 270.7 | 242.0 | 234.1 | 206.3 |

- **CLI fixed cost = +85ms/call** (139 vs 54) — forward+connect+HELLO per
  invocation, quantifying ADR-007's motivation exactly.
- Persistent paths agree within noise: serve ≈ lib ≈ shim (~50ms/op; the ~50ms
  is device-side `wait_finish` inject wait, not transport). serve JSON-RPC
  overhead over lib = 0–9ms — negligible.
- Note 口径: taps here are `wait_finish=true` (~50ms); fire-and-forget
  (wait_finish=false/NOACK) sits at the ~5ms baseline.

## 4. soak self-grading (new tool: `spike/e2e/soak-check.py`)

Re-graded `soak-m4.jsonl` retroactively: **PASS** — fd=59 constant ·
recoveries 17/17 (max 1156ms) · bench_fails=0 · RSS slope **−0.7 kB/min**
(90min, no leak trend). Tool exits non-zero on violation → reusable gate.

## 5. Device matrix

| device | sdk | caps | ver | checks |
|---|---|---|---|---|
| ba0d072d (Mi 11) | 34 | 127 | 0.3.1 | verify 30/30, serve, stream, soak |
| 2602BRT18C (dash) | 37 | 127 | 0.3.1 | hello/dump/tap ok (was offline last round — reachable again) |

## 6. Facade Drop / fd hygiene

facade `Session::Drop` = stream stop + media forward + control forward cleanup;
session-binding implicit stop covers process-death mid-stream (§4.8).
Regression gate: fd=59 flat from 90min soak → soak-check enforces it forward.

## Files

`e2e-m5-4path.json` (raw), `bench_m5_4path.py` (harness), `bench_session.rs`
(lib example), `soak-check.py` (grading gate), `README.md` (3-surface table +
shim supported-set + same-口径 benchmark table).
