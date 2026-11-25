# pilotdroid v0.7.1

Patch release — one real-device reliability fix (media ghost conn
race) plus post-0.7.0 items that were already on main: toast MCP push
notification, watch_id epoch guard, and stream/watch demux
hardening.

## What's fixed (the reason for this patch)

**Media ghost-conn race — first `stream_read` failed after a dirty
host exit** (agent.jar **0.7.1 → 0.7.2**, `Agent.java` Streamer — a
device-side agent fix, not only a Rust change):

A killed host could leave a stale media conn queued in the agent's
accept backlog. `stream_start` only cleared already-accepted pending
conns, so a ghost accepted *after* start was promoted under
`curSock == null`, and the real reader was busy-rejected → first
`stream_read` hit EOF (or starved when the stale writer's late
`mediaDead` halted the fresh stream, or when the accept thread died
on a bad attach). The fix keeps the §4.8 semantics — one serving
conn, dead reader still stops the stream — while making the
classification honest:

- second conn while streaming queues in a **reserve slot** (newest
  waiter wins) instead of being closed;
- `mediaDead(gen)` is **generation-guarded** — a previous stream's
  writer can no longer halt the new stream;
- **`curWrote` liveness** — a conn that dies before any successful
  write is a backlog ghost: dropped, stream keeps waiting for the
  real reader; a conn that wrote ≥1 frame then dies is a real reader
  loss → halt;
- the accept thread survives `attach` failures instead of exiting
  permanently.

Verified on ba0d072d: dirty-kill + RST'd ghost injection
**20/20** clean reads (0–13/20 intermittent before);
`verify-agent.sh` gained `stream_reserve_hold` /
`stream_reserve_heal` / `stream_ghost_drop` assertions (**40/0**);
reproducer at `spike/e2e/agent-loop/ghost_repro.py`.

## Also in 0.7.1

- **`notifications/pilotdroid/toast`** — agent a11y toast events
  (ch=1, msg_type 0x0002 with `{type:"toast"}`) are demultiplexed into
  an MCP notification while a push watcher keeps the media conn
  alive; otherwise they stay pullable via `watch_events`.
- **watch_id epoch guard** — after session recovery, stale armed
  watcher ids are dropped instead of colliding with ids the restarted
  agent re-assigns (stale `watch_disarm` no longer kills an innocent
  new watcher).
- **stream/watch demux hardening** — `stream_start` tears down stale
  media handles; `stream_read`/`stream_stop` demux interleaved
  ui_events into a session stash so `watch_events` can pull hits
  during and after streaming.

## Honest limits (unchanged)

Same as v0.7.0 / `compat/SUPPORTED.md`: clipboard ❌ (ROM-proven
unreachable), screenrecord ⚠️ (ROM-throttled output), xpath is an
approximation, no blanket fps guarantee beyond the measured ~5.6
ceiling, session recovery lives in the Rust/MCP path only.

## Verified

- `ghost_repro.py 20` → **20/20** clean reads after dirty kill +
  ghost injection (ba0d072d, agent 0.7.2).
- `verify-agent.sh ba0d072d` → **40/0** incl. new stream assertions.
- `cargo test --workspace` 94/0; pytest parity **12/12**.
- agent-loop harness 10/10; u2 vendored suite 22/0/8 unchanged.
