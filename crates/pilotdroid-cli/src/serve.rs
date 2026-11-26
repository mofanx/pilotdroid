//! `pilotdroid serve` — stdio JSON-RPC tool layer (ADR-007).
//!
//! MCP-compatible minimal surface: `initialize` →
//! `notifications/initialized` → `tools/list` → `tools/call`, one
//! JSON message per line. **Requests are processed strictly in arrival
//! order on a single thread** — the in-process serial queue is what
//! makes concurrent `tools/call` pipelining safe: frames on the control
//! socket can never interleave because there is exactly one writer per
//! request/response pair (§4.6 single in-flight). Callers wanting
//! parallelism get queued, not corrupted.
//!
//! stdout carries ONLY JSON-RPC responses — every banner/diagnostic
//! goes to stderr (agent callers parse stdout byte-for-byte).

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result};
use pilotdroid_adb::AdbTransport;
use pilotdroid_agent::{ElemInfo, MediaEvent, ScrollDir, Session};
use pilotdroid_proto::payload::*;
use serde_json::{json, Map, Value};

use crate::som;

/// Media-poll cadence while push-armed watchers exist (serve stays
/// single-writer: the demux happens between requests on the loop
/// thread, so notification frames can never tear a response frame).
const PUSH_POLL_MS: u64 = 150;
/// Events consumed by the push demux but not deliverable (non-push
/// watcher hit, frame, or a failed notification write) stay pullable
/// through `watch_events` — the documented degrade path.
const PENDING_CAP: usize = 64;

/// Ops allowed inside `verify_after.action` — side-effect or read ops
/// only; no batch/verify recursion, no stream/watch lifecycle tools
/// (their acks don't map onto the action result envelope).
const VERIFY_OPS: &[&str] = &[
    "tap",
    "long_click",
    "swipe",
    "press",
    "set_text",
    "scroll",
    "pinch_in",
    "pinch_out",
    "app_start",
    "app_stop",
    "exists",
    "query",
    "wait_for",
    "dump",
    "list_elements",
    "tap_index",
    "set_text_index",
    "screenshot",
];

/// MCP protocol revisions we implement, oldest → newest. Negotiation
/// is a whitelist: an offered version we don't implement gets our
/// newest back — echoing arbitrary offers would overclaim support.
pub const SUPPORTED_VERSIONS: &[&str] = &["2024-11-05", "2025-11-25"];

/// Pick the `protocolVersion` for the initialize response: echo a
/// supported offer, otherwise report our newest (clients must
/// tolerate a different version than they proposed).
fn negotiate_version(offered: Option<&str>) -> &'static str {
    if let Some(v) = offered {
        if let Some(&sv) = SUPPORTED_VERSIONS.iter().find(|&&s| s == v) {
            return sv;
        }
    }
    SUPPORTED_VERSIONS[SUPPORTED_VERSIONS.len() - 1]
}

/// Serve loop: read JSON-RPC requests from `inp`, write responses to
/// `out`. `initialize` attempts `connect` once to fill
/// `serverInfo.agentVer` (null when the device is unreachable — the
/// handshake stays device-free); the first `tools/call` (re)connects
/// on demand.
pub fn run<T: AdbTransport, R: std::io::BufRead + Send + 'static, W: Write>(
    inp: R,
    out: &mut W,
    mut connect: impl FnMut() -> Result<Session<T>>,
) -> Result<()> {
    let mut session: Option<Session<T>> = None;
    // Element snapshot backing tap_index/set_text_index — refreshed by
    // every list_elements call; indexes are meaningless across snapshots.
    // (elements, layout signature, focused pkg at snapshot time).
    let mut snapshot: Option<(Vec<pilotdroid_agent::ElemInfo>, u64, Option<String>)> = None;
    // Armed async watchers: watch_id → the re-armable rule (selector,
    // action, push flag, epoch tag). `push=true` (watch_arm arg) routes
    // a hit to an MCP notification instead of only the watch_events
    // pull — Track A demux, overriding the M26 defer.
    let mut armed: HashMap<u32, ArmedRule> = HashMap::new();
    // Recovery epoch observed from Session::generation() — the t14
    // watchdog reconnects transparently, but a restarted agent
    // re-allocates watch ids from scratch: rules in `armed` tagged with
    // an older gen are re-armed on the new agent (watch_id remapped),
    // stale buffered hits dropped, and the client notified via
    // notifications/pilotdroid/session_recovered.
    let mut epoch: u64 = 0;
    // Media events consumed by the demux that weren't pushed — pullable
    // via watch_events (degrade path). Bounded: a client that never
    // pulls shouldn't grow memory forever.
    let mut pending: VecDeque<MediaEvent> = VecDeque::new();
    // Media-demux liveness flag: suppress repeated warnings while the
    // conn stays broken; armed rules are kept (not dropped) so the next
    // epoch change re-arms them once the agent heals.
    let mut demux_dead = false;

    // stdin → channel: lines() blocks forever otherwise, and the demux
    // needs timed wakeups while push-armed watchers exist. The reader
    // thread only reads; stdout stays single-writer on this thread.
    let (tx, rx) = mpsc::channel::<std::io::Result<String>>();
    std::thread::spawn(move || {
        for line in inp.lines() {
            let done = line.is_err();
            if tx.send(line).is_err() || done {
                break;
            }
        }
    });

    let result = loop {
        if let Some(s) = session.as_mut() {
            // Epoch check first: a control op during the previous
            // iteration may have healed a dead agent — watchers armed on
            // it are dead agent-side, so re-arm them from their stored
            // selectors (watch_id remapped to the new allocation).
            let cur_gen = s.generation();
            if cur_gen != epoch {
                epoch = cur_gen;
                // Buffered hits from the dead epoch reference dead ids —
                // drop them (toasts/frames stay pullable).
                while let Some(ev) = s.pop_stashed() {
                    if !matches!(ev, MediaEvent::WatchHit(_)) {
                        push_pending(&mut pending, ev);
                    }
                }
                pending.retain(|e| !matches!(e, MediaEvent::WatchHit(_)));
                let mut stale_n = 0usize;
                let mut rearmed = 0usize;
                let mut failed = 0usize;
                let old = std::mem::take(&mut armed);
                if old.iter().any(|(_, r)| r.gen != cur_gen) {
                    // Best effort pre-attach so early hits aren't dropped.
                    let _ = s.watch_attach();
                }
                for (id, mut rule) in old {
                    if rule.gen == cur_gen {
                        armed.insert(id, rule);
                        continue;
                    }
                    stale_n += 1;
                    match s.watch_arm(rule.sel.clone(), &rule.action, rule.timeout_ms) {
                        Ok(ack) => {
                            // Tag the gen observed *after* the call — a
                            // re-arm can itself trigger another recovery
                            // (double-fault corner), and the id belongs
                            // to the newest agent.
                            rule.gen = s.generation();
                            rule.dead_rearms = 0; // fresh agent, fresh budget
                            armed.insert(ack.watch_id, rule);
                            rearmed += 1;
                        }
                        Err(_) => failed += 1,
                    }
                }
                let note = json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/pilotdroid/session_recovered",
                    "params": {
                        "generation": cur_gen,
                        "stale_watchers": stale_n,
                        "rearmed": rearmed,
                        "rearm_failed": failed,
                    },
                });
                if let Err(e) = write_resp(out, &note) {
                    break Err(e);
                }
                if stale_n > 0 {
                    eprintln!(
                        "pilotdroid-serve: agent recovered — {stale_n} stale watcher(s), \
                         {rearmed} re-armed, {failed} failed"
                    );
                }
                demux_dead = false;
            }
            // Stash drain (socket-free): hits/toasts that interleaved
            // into the media conn while a frame stream was active were
            // sorted into the session stash by stream_read/stream_stop —
            // push them here so notifications keep flowing mid-stream
            // instead of waiting for a pull or stream end.
            let drain: Result<()> = loop {
                let Some(ev) = s.pop_stashed() else {
                    break Ok(());
                };
                if let Err(e) = route_event(out, &mut pending, &mut armed, ev, s) {
                    break Err(e);
                }
            };
            if let Err(e) = drain {
                break Err(e);
            }
        }
        // Push demux (Track A): while any watcher is armed with
        // push=true, poll the media channel between requests. Gated on
        // !streaming — frames and hits share ch=1, and consuming frames
        // here would starve stream_read; interleaved events still reach
        // the client via the stash drain above.
        let push_active = armed.values().any(|r| r.push);
        if push_active {
            if let Some(s) = session.as_mut() {
                if !s.is_streaming() {
                    match s.watch_recv(Some(PUSH_POLL_MS)) {
                        Ok(ev) => {
                            demux_dead = false;
                            if let Err(e) = route_event(out, &mut pending, &mut armed, ev, s) {
                                break Err(e);
                            }
                        }
                        Err(pilotdroid_agent::AgentError::Timeout) => {}
                        Err(e) => {
                            // Media socket broken — keep armed rules: the
                            // epoch change from watch_recv's heal will
                            // re-arm them; warn only once per broken
                            // streak.
                            if !demux_dead {
                                demux_dead = true;
                                eprintln!("pilotdroid-serve: watch push demux paused: {e}");
                            }
                        }
                    }
                }
            }
        }
        let line = if push_active {
            match rx.recv_timeout(Duration::from_millis(PUSH_POLL_MS)) {
                Ok(l) => l?,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                // Client stdin EOF → disconnect: disarm below, exit clean.
                Err(mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
            }
        } else {
            match rx.recv() {
                Ok(l) => l?,
                Err(mpsc::RecvError) => break Ok(()),
            }
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                write_resp(
                    out,
                    &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("parse error: {e}")}}),
                )?;
                continue;
            }
        };
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        // Notifications carry no id and must never get a response.
        let Some(id) = req.get("id").cloned() else {
            continue;
        };
        let resp = match method {
            "initialize" => {
                let pv = negotiate_version(
                    req.pointer("/params/protocolVersion")
                        .and_then(Value::as_str),
                );
                // One handshake exposes both version namespaces
                // (docs/02 §7): serverInfo.version = host crate,
                // agentVer = on-device jar build from HELLO. Connect
                // eagerly so the field is populated when the device is
                // reachable; null keeps the handshake device-free.
                let agent_ver = match session.as_ref() {
                    Some(s) => s.agent_ver().map(str::to_string),
                    None => match connect() {
                        Ok(s) => {
                            let v = s.agent_ver().map(str::to_string);
                            session = Some(s);
                            v
                        }
                        Err(_) => None,
                    },
                };
                ok(
                    id,
                    json!({
                        "protocolVersion": pv,
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {
                            "name": "pilotdroid",
                            "version": env!("CARGO_PKG_VERSION"),
                            "agentVer": agent_ver,
                        },
                    }),
                )
            }
            "ping" => ok(id, json!({})),
            "tools/list" => ok(id, json!({"tools": tools()})),
            "resources/list" => ok(id, json!({"resources": []})),
            "prompts/list" => ok(id, json!({"prompts": []})),
            "tools/call" => {
                let name = req
                    .pointer("/params/name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let args = req
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let s = match session.as_mut() {
                    Some(s) => s,
                    None => match connect() {
                        Ok(s) => session.insert(s),
                        Err(e) => {
                            write_resp(out, &tool_err(id, e))?;
                            continue;
                        }
                    },
                };
                let (v, dead) =
                    call_tool(s, id, &name, args, &mut snapshot, &mut armed, &mut pending);
                // Dead control conn (e.g. replaced by a newer conn —
                // ADR-007 last-connector-wins): drop it so the NEXT
                // tools/call lazily reconnects and re-claims the agent,
                // instead of this serve process erroring every call on a
                // socket that can never recover.
                if dead {
                    eprintln!(
                        "pilotdroid-serve: control conn lost — likely replaced \
                         by another client (last-connector-wins, ADR-007); \
                         will reconnect on next call \
                         (safe to ignore on first connection)"
                    );
                    session = None;
                }
                write_resp(out, &v)?;
                continue;
            }
            _ => err(id, -32601, &format!("method not found: {method}")),
        };
        if let Err(e) = write_resp(out, &resp) {
            break Err(e);
        }
    };
    // Client disconnect or write failure: disarm anything still armed —
    // the contract is armed rules must not outlive the MCP session that
    // created them (agent also reaps on conn close; explicit is belt&
    // braces for watchers armed on a shared/stale conn).
    if let Some(s) = session.as_mut() {
        for wid in armed.keys() {
            let _ = s.watch_disarm(*wid);
        }
    }
    result
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"result":result})
}

fn err(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

/// Transport-level dead conn: the socket went EOF/reset (incl. being
/// replaced by a newer control conn — ADR-007). Deliberately does NOT
/// include `AgentError::Timeout` — a slow read on a live session is
/// transient, not fatal.
fn is_dead_conn(e: &anyhow::Error) -> bool {
    matches!(
        e.downcast_ref::<pilotdroid_agent::AgentError>(),
        Some(pilotdroid_agent::AgentError::Io(_))
    )
}

fn tool_err(id: Value, e: anyhow::Error) -> Value {
    ok(
        id,
        json!({
            "content": [{"type":"text","text":format!("{e:#}")}],
            "isError": true,
        }),
    )
}

fn tool_ok(id: Value, result: Value) -> Value {
    ok(
        id,
        json!({
            "content": [{"type":"text","text":serde_json::to_string(&result).unwrap_or_default()}],
            "isError": false,
        }),
    )
}

fn write_resp<W: Write>(out: &mut W, v: &Value) -> Result<()> {
    writeln!(out, "{v}")?;
    out.flush().context("stdout write")
}

/// Buffer a demuxed-but-unpushed media event for the watch_events pull
/// path; oldest evicted past PENDING_CAP.
fn push_pending(pending: &mut VecDeque<MediaEvent>, ev: MediaEvent) {
    if pending.len() >= PENDING_CAP {
        pending.pop_front();
    }
    pending.push_back(ev);
}

/// An armed watcher plus the payload needed to re-arm it after a
/// session recovery — the restarted agent re-allocates watch ids, so
/// `gen` tags which recovery epoch the id belongs to (a watcher armed
/// on the current agent is kept; anything older is re-armed).
struct ArmedRule {
    push: bool,
    sel: Selector,
    action: String,
    timeout_ms: Option<u64>,
    gen: u64,
    /// Bounded abnormal-death re-arms within one epoch (watch_dead
    /// path) — reset when the epoch re-arm gives the rule a fresh agent.
    dead_rearms: u8,
}

/// Re-arm a watcher that died abnormally (watch_dead ui_event) —
/// bounded per rule (`dead_rearms` cap) so a crash-looping selector
/// can't storm ARM calls between recovery epochs. Mutates `params`
/// with the outcome; caller re-inserts nothing — this fn owns the map.
fn rearm_dead<T: AdbTransport>(
    s: &mut Session<T>,
    armed: &mut HashMap<u32, ArmedRule>,
    dead_id: u32,
    params: &mut serde_json::Map<String, Value>,
) {
    params.insert("armed".into(), json!(true));
    let Some(mut rule) = armed.remove(&dead_id) else {
        return;
    };
    if rule.dead_rearms >= 2 {
        params.insert("rearm_capped".into(), json!(true));
        armed.insert(dead_id, rule); // keep for the next epoch heal
        return;
    }
    match s.watch_arm(rule.sel.clone(), &rule.action, rule.timeout_ms) {
        Ok(ack) => {
            rule.gen = s.generation();
            rule.dead_rearms += 1;
            armed.insert(ack.watch_id, rule);
            params.insert("rearmed".into(), json!(true));
            params.insert("new_watch_id".into(), json!(ack.watch_id));
        }
        Err(e) => {
            params.insert("rearm_failed".into(), json!(e.to_string()));
            armed.insert(dead_id, rule); // keep for the next epoch heal
        }
    }
}

/// Route a demuxed/stashed media event: a WatchHit on a push-armed
/// watcher and ambient Toast events become MCP notifications; anything
/// else buffers into the watch_events pull path. A fired watcher is
/// one-shot agent-side — removed from `armed` either way. A WatchDead
/// is always notified (armed→dead must never be silent) and triggers
/// a bounded auto re-arm of the still-armed rule.
fn route_event<W: Write, T: AdbTransport>(
    out: &mut W,
    pending: &mut VecDeque<MediaEvent>,
    armed: &mut HashMap<u32, ArmedRule>,
    ev: MediaEvent,
    s: &mut Session<T>,
) -> Result<()> {
    match ev {
        MediaEvent::WatchHit(h) => {
            let want_push = armed.remove(&h.watch_id).map(|r| r.push).unwrap_or(false);
            if want_push {
                let note = json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/pilotdroid/watch_hit",
                    "params": h,
                });
                if let Err(e) = write_resp(out, &note) {
                    // stdout dead — buffer then bail (the event stays
                    // consistent for the pull path).
                    push_pending(pending, MediaEvent::WatchHit(h));
                    return Err(e);
                }
            } else {
                push_pending(pending, MediaEvent::WatchHit(h));
            }
        }
        MediaEvent::Toast(t) => {
            let note = json!({
                "jsonrpc": "2.0",
                "method": "notifications/pilotdroid/toast",
                "params": t,
            });
            if let Err(e) = write_resp(out, &note) {
                push_pending(pending, MediaEvent::Toast(t));
                return Err(e);
            }
        }
        MediaEvent::WatchDead(d) => {
            let mut params = serde_json::Map::from_iter([
                ("watch_id".into(), json!(d.watch_id)),
                ("waited_ms".into(), json!(d.waited_ms)),
                ("error".into(), json!(d.error)),
            ]);
            if armed.contains_key(&d.watch_id) {
                rearm_dead(s, armed, d.watch_id, &mut params);
            }
            let note = json!({
                "jsonrpc": "2.0",
                "method": "notifications/pilotdroid/watch_dead",
                "params": Value::Object(params),
            });
            if let Err(e) = write_resp(out, &note) {
                push_pending(pending, MediaEvent::WatchDead(d));
                return Err(e);
            }
        }
        ev => push_pending(pending, ev),
    }
    Ok(())
}

// ---- tool dispatch ----------------------------------------------------------

/// Selector fields arrive flat in `arguments` (u2-style kw args).
fn selector_of(args: &Value) -> Selector {
    Selector {
        text: str_arg(args, "text"),
        id: str_arg(args, "id"),
        desc: str_arg(args, "desc"),
        class: str_arg(args, "className").or_else(|| str_arg(args, "class")),
        scrollable: args.get("scrollable").and_then(Value::as_bool),
        pkg: str_arg(args, "packageName").or_else(|| str_arg(args, "pkg")),
        index: args.get("index").and_then(Value::as_u64).map(|v| v as u32),
    }
}

fn str_arg(args: &Value, k: &str) -> Option<String> {
    args.get(k).and_then(Value::as_str).map(String::from)
}

fn num_arg(args: &Value, k: &str) -> Option<f64> {
    args.get(k).and_then(Value::as_f64)
}

/// `describe_screen ocr:true` — tesseract presence is probed once per
/// process (a missing binary can't appear mid-session).
fn tesseract_available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        std::process::Command::new("tesseract")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    })
}

/// tesseract TSV (`--psm 11`) → merged line blocks, mirroring
/// `ocr_spike.py`: level-5 word rows with conf ≥ 40 and at least one
/// alphanumeric char, grouped by (block, par, line).
fn ocr_blocks(img: &std::path::Path) -> Result<Vec<som::OcrBlock>> {
    let out = std::process::Command::new("tesseract")
        .arg(img)
        .args(["stdout", "--psm", "11", "tsv"])
        .output()
        .context("spawn tesseract")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    type Words = Vec<(i32, i32, i32, i32, f32, String)>;
    let mut words: std::collections::BTreeMap<(String, String, String), Words> = Default::default();
    for ln in stdout.lines().skip(1) {
        let c: Vec<&str> = ln.split('\t').collect();
        if c.len() < 12 || c[0] != "5" {
            continue;
        }
        let Ok(conf) = c[10].parse::<f32>() else {
            continue;
        };
        let txt = c[11].trim();
        if conf < 40.0 || !txt.chars().any(|ch| ch.is_alphanumeric()) {
            continue;
        }
        let (Ok(x), Ok(y), Ok(w), Ok(h)) = (
            c[6].parse::<i32>(),
            c[7].parse::<i32>(),
            c[8].parse::<i32>(),
            c[9].parse::<i32>(),
        ) else {
            continue;
        };
        words
            .entry((c[2].into(), c[3].into(), c[4].into()))
            .or_default()
            .push((x, y, w, h, conf, txt.to_string()));
    }
    Ok(words
        .into_values()
        .map(|mut ws| {
            ws.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
            let l = ws.iter().map(|w| w.0).min().unwrap();
            let t = ws.iter().map(|w| w.1).min().unwrap();
            let r = ws.iter().map(|w| w.0 + w.2).max().unwrap();
            let b = ws.iter().map(|w| w.1 + w.3).max().unwrap();
            som::OcrBlock {
                text: ws.iter().map(|w| w.5.clone()).collect::<Vec<_>>().join(" "),
                bounds: [l, t, r, b],
                conf: ws.iter().map(|w| w.4).sum::<f32>() / ws.len() as f32,
            }
        })
        .collect())
}

/// Snapshot staleness guard for index ops: the focused package and the
/// layout signature must match the snapshot's — a changed screen means
/// the cached indexes point at the wrong bounds. Fails closed: a wrong
/// tap is more expensive than a re-list.
fn check_snapshot<T: AdbTransport>(
    s: &mut Session<T>,
    snap: &(Vec<pilotdroid_agent::ElemInfo>, u64, Option<String>),
) -> Result<()> {
    let cur_pkg = s.app_current().ok().flatten().map(|(p, _)| p);
    anyhow::ensure!(
        cur_pkg == snap.2,
        "snapshot stale — focused app changed ({:?} → {cur_pkg:?}); re-run list_elements",
        snap.2
    );
    let cur_sig = s.layout_signature()?;
    anyhow::ensure!(
        cur_sig == snap.1,
        "snapshot stale — layout changed since list_elements; re-run list_elements"
    );
    Ok(())
}

/// Returns the JSON-RPC response plus a "dead conn" flag — set when the
/// op failed on a transport-level socket error (`AgentError::Io`), in
/// which case the caller must drop the session so a later call reconnects.
fn call_tool<T: AdbTransport>(
    s: &mut Session<T>,
    id: Value,
    name: &str,
    args: Value,
    snap: &mut Option<(Vec<pilotdroid_agent::ElemInfo>, u64, Option<String>)>,
    armed: &mut HashMap<u32, ArmedRule>,
    pending: &mut VecDeque<MediaEvent>,
) -> (Value, bool) {
    let r: std::result::Result<Value, anyhow::Error> =
        (|| -> std::result::Result<Value, anyhow::Error> {
            match name {
                "tap" => {
                    let a = s.tap(
                        num_arg(&args, "x").unwrap_or(0.0) as f32,
                        num_arg(&args, "y").unwrap_or(0.0) as f32,
                    )?;
                    Ok(json!(a))
                }
                "long_click" => {
                    let a = s.long_click(
                        num_arg(&args, "x").unwrap_or(0.0) as f32,
                        num_arg(&args, "y").unwrap_or(0.0) as f32,
                        num_arg(&args, "duration_ms").unwrap_or(500.0) as u64,
                    )?;
                    Ok(json!(a))
                }
                "swipe" => {
                    let a = s.swipe(
                        num_arg(&args, "x1").unwrap_or(0.0) as f32,
                        num_arg(&args, "y1").unwrap_or(0.0) as f32,
                        num_arg(&args, "x2").unwrap_or(0.0) as f32,
                        num_arg(&args, "y2").unwrap_or(0.0) as f32,
                        num_arg(&args, "duration_ms").unwrap_or(300.0) as u64,
                    )?;
                    Ok(json!(a))
                }
                "press" => {
                    let a = s.press(
                        num_arg(&args, "keycode").unwrap_or(0.0) as u32,
                        args.get("meta").and_then(Value::as_u64).map(|v| v as u32),
                    )?;
                    Ok(json!(a))
                }
                "set_text" => {
                    s.set_text(
                        &str_arg(&args, "text").unwrap_or_default(),
                        args.get("clear").and_then(Value::as_bool).unwrap_or(true),
                    )?;
                    Ok(json!({"ok": true}))
                }
                "exists" => Ok(json!({"exists": s.exists(selector_of(&args))?})),
                "query" => Ok(json!(s.query(selector_of(&args))?)),
                "wait_for" => {
                    let (found, node) = s.wait_for(
                        selector_of(&args),
                        num_arg(&args, "timeout_ms").unwrap_or(5000.0) as u64,
                    )?;
                    Ok(json!({"found": found, "node": node}))
                }
                "dump" => Ok(json!({"xml": s.dump()?})),
                "list_elements" => {
                    let interactive = args
                        .get("interactive_only")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let sn = s.list_elements(interactive)?;
                    let pkg = s.app_current().ok().flatten().map(|(p, _)| p);
                    let out = json!({
                        "count": sn.elements.len(),
                        "interactive_only": interactive,
                        "elements": sn.elements,
                    });
                    *snap = Some((sn.elements, sn.layout_sig, pkg));
                    Ok(out)
                }
                "tap_index" => {
                    let i = num_arg(&args, "i").context("tap_index needs i")? as usize;
                    let (els, _, _) = snap
                        .as_ref()
                        .context("no element snapshot — call list_elements first")?;
                    check_snapshot(s, snap.as_ref().unwrap())?;
                    let e = els.get(i).with_context(|| {
                        format!(
                            "index {i} out of range — call list_elements first ({} cached)",
                            els.len()
                        )
                    })?;
                    let [l, t, r, b] = e.bounds;
                    let a = s.tap((l + r) as f32 / 2.0, (t + b) as f32 / 2.0)?;
                    Ok(json!({"i": i, "bounds": e.bounds, "result": a}))
                }
                "set_text_index" => {
                    let i = num_arg(&args, "i").context("set_text_index needs i")? as usize;
                    let (els, _, _) = snap
                        .as_ref()
                        .context("no element snapshot — call list_elements first")?;
                    check_snapshot(s, snap.as_ref().unwrap())?;
                    let e = els.get(i).with_context(|| {
                        format!(
                            "index {i} out of range — call list_elements first ({} cached)",
                            els.len()
                        )
                    })?;
                    let [l, t, r, b] = e.bounds;
                    // tap the field first so ACTION_SET_TEXT lands on it
                    s.tap((l + r) as f32 / 2.0, (t + b) as f32 / 2.0)?;
                    s.set_text(
                        &str_arg(&args, "text").unwrap_or_default(),
                        args.get("clear").and_then(Value::as_bool).unwrap_or(true),
                    )?;
                    Ok(json!({"i": i, "ok": true}))
                }
                "screenshot" => {
                    let path = str_arg(&args, "path").map(PathBuf::from);
                    let (fmt, bytes) = s.screenshot(
                        path.as_deref(),
                        num_arg(&args, "quality").unwrap_or(80.0) as u8,
                        args.get("scale").and_then(Value::as_f64).map(|v| v as f32),
                    )?;
                    // File bypass preferred for stdio (§serve contract);
                    // base64 only when no path given.
                    Ok(json!({
                        "format": fmt,
                        "bytes": bytes.len(),
                        "path": path,
                        "b64": path.is_none().then(|| b64(&bytes)),
                    }))
                }
                "scroll" => {
                    let dir = match str_arg(&args, "dir").as_deref() {
                        Some("backward") | Some("back") => ScrollDir::Backward,
                        Some("to_end") | Some("toEnd") => ScrollDir::ToEnd,
                        Some("to_beginning") | Some("toBeginning") => ScrollDir::ToBeginning,
                        _ => ScrollDir::Forward,
                    };
                    let mut sel = selector_of(&args);
                    if sel == Selector::default() {
                        sel.scrollable = Some(true); // u2 d(scrollable=True) default
                    }
                    Ok(json!({"done": s.scroll(dir, sel)?}))
                }
                "app_start" => {
                    s.app_start(
                        &str_arg(&args, "pkg").context("app_start needs pkg")?,
                        str_arg(&args, "activity").as_deref(),
                    )?;
                    Ok(json!({"ok": true}))
                }
                "app_stop" => {
                    s.app_stop(&str_arg(&args, "pkg").context("app_stop needs pkg")?)?;
                    Ok(json!({"ok": true}))
                }
                "app_current" => {
                    let cur = s.app_current()?;
                    Ok(match cur {
                        Some((package, activity)) => {
                            json!({ "package": package, "activity": activity })
                        }
                        None => json!({}),
                    })
                }
                "stream_start" => Ok(json!(s.stream_start(
                    args.get("fps").and_then(Value::as_u64).map(|v| v as u8),
                    args.get("scale").and_then(Value::as_f64).map(|v| v as f32),
                    args.get("quality").and_then(Value::as_u64).map(|v| v as u8),
                )?)),
                "stream_read" => {
                    let n = num_arg(&args, "n").unwrap_or(1.0) as u32;
                    let dir = str_arg(&args, "to_dir").map(PathBuf::from);
                    let frames = s.stream_read(n, dir.as_deref())?;
                    let out: Vec<Value> = frames
                        .iter()
                        .map(|f| {
                            json!({
                                "format": f.format, "w": f.w, "h": f.h,
                                "take_ns": f.take_ns, "enc_ns": f.enc_ns,
                                "dropped": f.dropped,
                                "b64": dir.is_none().then(|| b64(&f.data)),
                            })
                        })
                        .collect();
                    Ok(json!({"frames": out, "count": out.len()}))
                }
                "stream_stop" => Ok(json!(s.stream_stop()?)),
                "pinch_in" | "pinch_out" => {
                    let cx = num_arg(&args, "cx").context("pinch needs cx")? as f32;
                    let cy = num_arg(&args, "cy").context("pinch needs cy")? as f32;
                    let radius = num_arg(&args, "radius_px").unwrap_or(300.0) as f32;
                    let pct = (num_arg(&args, "percent").unwrap_or(50.0) / 100.0) as f32;
                    let dur = num_arg(&args, "duration_ms").unwrap_or(300.0) as u64;
                    let (r0, r1) = if name == "pinch_in" {
                        (radius, radius * (1.0 - pct))
                    } else {
                        (radius * (1.0 - pct), radius)
                    };
                    Ok(json!(s.pinch(cx, cy, r0, r1, dur)?))
                }
                "watch" => {
                    let ack = s.watch(
                        selector_of(&args),
                        &str_arg(&args, "action").unwrap_or_else(|| "click".into()),
                        num_arg(&args, "timeout_ms").unwrap_or(5000.0) as u64,
                    )?;
                    Ok(json!(ack))
                }
                "watch_arm" => {
                    let sel = selector_of(&args);
                    anyhow::ensure!(
                        sel.text.is_some()
                            || sel.id.is_some()
                            || sel.desc.is_some()
                            || sel.class.is_some()
                            || sel.scrollable.is_some()
                            || sel.pkg.is_some()
                            || sel.index.is_some(),
                        "watch_arm needs a non-empty selector (empty matches \
                         everything → instant spurious hit)"
                    );
                    // Pre-attach the media conn: the agent pushes WATCH_HIT
                    // onto the pending media socket — without it attached,
                    // a hit fired before the first watch_events call is
                    // silently dropped (§4.8/§4.9).
                    s.watch_attach()?;
                    let push = args.get("push").and_then(Value::as_bool).unwrap_or(false);
                    let action = str_arg(&args, "action").unwrap_or_else(|| "click".into());
                    let timeout_ms = num_arg(&args, "timeout_ms").map(|v| v as u64);
                    let ack = s.watch_arm(sel.clone(), &action, timeout_ms)?;
                    // gen tagged post-call — the arm itself may have
                    // healed a dead agent, and the id belongs to it.
                    armed.insert(
                        ack.watch_id,
                        ArmedRule {
                            push,
                            sel,
                            action,
                            timeout_ms,
                            gen: s.generation(),
                            dead_rearms: 0,
                        },
                    );
                    Ok(json!(ack))
                }
                "watch_disarm" => {
                    let id =
                        num_arg(&args, "watch_id").context("watch_disarm needs watch_id")? as u32;
                    // Forward only for ids armed in the tracked map — a
                    // stale/unknown id must not reach the agent: after a
                    // recovery the restarted agent re-allocated that id
                    // to a live watcher (t5 nit (a), stale-disarm hole).
                    if armed.remove(&id).is_some() {
                        Ok(json!({"disarmed": s.watch_disarm(id)?}))
                    } else {
                        Ok(json!({"disarmed": false}))
                    }
                }
                "watch_events" => {
                    // Demux leftovers first (non-push hits, frames,
                    // failed-push events) — pull is the degrade path.
                    if let Some(ev) = pending.pop_front() {
                        match ev {
                            MediaEvent::WatchHit(h) => {
                                armed.remove(&h.watch_id);
                                Ok(json!({"type": "watch_hit", "hit": h}))
                            }
                            MediaEvent::Frame(f) => Ok(json!({
                                "type": "frame", "w": f.w, "h": f.h,
                                "dropped": f.dropped,
                            })),
                            MediaEvent::Toast(t) => Ok(json!({"type": "toast", "toast": t})),
                            MediaEvent::WatchDead(d) => {
                                let mut p = serde_json::Map::from_iter([
                                    ("watch_id".into(), json!(d.watch_id)),
                                    ("waited_ms".into(), json!(d.waited_ms)),
                                    ("error".into(), json!(d.error)),
                                ]);
                                if armed.contains_key(&d.watch_id) {
                                    rearm_dead(s, armed, d.watch_id, &mut p);
                                }
                                Ok(json!({"type": "watch_dead",
                                          "dead": Value::Object(p)}))
                            }
                        }
                    } else {
                        match s
                            .watch_recv(num_arg(&args, "timeout_ms").map(|v| (v as u64).max(500)))
                        {
                            Ok(MediaEvent::WatchHit(h)) => {
                                armed.remove(&h.watch_id);
                                Ok(json!({"type": "watch_hit", "hit": h}))
                            }
                            Ok(MediaEvent::Frame(f)) => Ok(json!({
                                "type": "frame", "w": f.w, "h": f.h,
                                "dropped": f.dropped,
                            })),
                            Ok(MediaEvent::Toast(t)) => Ok(json!({"type": "toast", "toast": t})),
                            Ok(MediaEvent::WatchDead(d)) => {
                                let mut p = serde_json::Map::from_iter([
                                    ("watch_id".into(), json!(d.watch_id)),
                                    ("waited_ms".into(), json!(d.waited_ms)),
                                    ("error".into(), json!(d.error)),
                                ]);
                                if armed.contains_key(&d.watch_id) {
                                    rearm_dead(s, armed, d.watch_id, &mut p);
                                }
                                Ok(json!({"type": "watch_dead",
                                          "dead": Value::Object(p)}))
                            }
                            // No event yet is data, not failure — report it in the
                            // union so an armed-but-quiet watcher doesn't surface
                            // isError to the MCP client (real errors still do).
                            Err(pilotdroid_agent::AgentError::Timeout) => {
                                Ok(json!({"type": "timeout"}))
                            }
                            Err(e) => Err(e.into()),
                        }
                    }
                }
                "describe_screen" => {
                    let xml = s.dump()?;
                    let ml = num_arg(&args, "max_label").map(|v| v as usize);
                    if args.get("ocr").and_then(Value::as_bool).unwrap_or(false) {
                        // OCR tier (host-side, optional): screenshot →
                        // local tesseract TSV → geometric fusion into
                        // the same element space. Degrades gracefully —
                        // no tesseract = normal describe + a hint,
                        // never an error.
                        if !tesseract_available() {
                            let mut v = som::describe(&xml, ml);
                            v["ocr"] = json!({
                                "available": false,
                                "hint": "tesseract not on PATH — install \
                                         tesseract-ocr to enable ocr:true",
                            });
                            return Ok(v);
                        }
                        let tmp = std::env::temp_dir()
                            .join(format!("pilotdroid-ocr-{}.jpg", std::process::id()));
                        let (_f, _b) = s.screenshot(Some(&tmp), 95, Some(1.0))?;
                        let blocks = ocr_blocks(&tmp)?;
                        Ok(som::describe_ocr(&xml, ml, &blocks))
                    } else {
                        Ok(som::describe(&xml, ml))
                    }
                }
                "screenshot_annotated" => {
                    let quality = num_arg(&args, "quality").unwrap_or(80.0) as u8;
                    let path = str_arg(&args, "path").map(PathBuf::from);
                    let xml = s.dump()?;
                    // scale fixed at 1.0 — bounds are display px (same
                    // contract as compat/som.py).
                    let (_fmt, jpeg) = s.screenshot(None, quality, Some(1.0))?;
                    let data = som::annotate(&jpeg, &xml, quality)?;
                    if let Some(p) = &path {
                        std::fs::write(p, &data)
                            .with_context(|| format!("write {}", p.display()))?;
                    }
                    Ok(json!({
                        "format": "jpeg", "bytes": data.len(), "path": path,
                        "b64": path.is_none().then(|| b64(&data)),
                    }))
                }
                "verify_after" => {
                    // Post-action closed-loop check (no u2 equivalent):
                    // run one op → settle → assert on a fresh dump.
                    let action = args.get("action").context("verify_after needs action")?;
                    let tool = action
                        .get("tool")
                        .and_then(Value::as_str)
                        .context("action.tool required")?;
                    anyhow::ensure!(
                        VERIFY_OPS.contains(&tool),
                        "verify_after action must be a side-effect op, got '{tool}'"
                    );
                    let aargs = action.get("args").cloned().unwrap_or_else(|| json!({}));
                    let settle = num_arg(&args, "settle_ms").unwrap_or(500.0) as u64;
                    let before_sig = s.layout_signature().ok();
                    let (v2, dead2) = call_tool(s, Value::Null, tool, aargs, snap, armed, pending);
                    if dead2 {
                        return Err(pilotdroid_agent::AgentError::Io(std::io::Error::new(
                            std::io::ErrorKind::BrokenPipe,
                            "verify_after action lost the control conn",
                        ))
                        .into());
                    }
                    let a_ok = !v2
                        .pointer("/result/isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let a_detail = v2
                        .pointer("/result/content/0/text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    std::thread::sleep(Duration::from_millis(settle));
                    let expect = args.get("expect").context("verify_after needs expect")?;
                    let mut checks = Vec::new();
                    if let Some(sel_v) = expect.get("exists") {
                        let found = s.exists(selector_of(sel_v))?;
                        checks.push(json!({"kind":"exists","pass":found,"found":found}));
                    }
                    if let Some(sel_v) = expect.get("absent") {
                        let found = s.exists(selector_of(sel_v))?;
                        checks.push(json!({"kind":"absent","pass":!found,"found":found}));
                    }
                    if let Some(t) = expect.get("text").and_then(Value::as_str) {
                        let sel = Selector {
                            text: Some(t.to_string()),
                            ..Default::default()
                        };
                        let found = s.exists(sel)?;
                        checks.push(json!({"kind":"text","pass":found,"found":found}));
                    }
                    if let Some(want) = expect.get("layout_changed").and_then(Value::as_bool) {
                        let pass = match (before_sig, s.layout_signature().ok()) {
                            (Some(a), Some(b)) => (a != b) == want,
                            _ => false,
                        };
                        checks.push(json!({"kind":"layout_changed","pass":pass,"want":want}));
                    }
                    // Widget-state predicate ("tap ok≠effect" layer): a
                    // state check needs the widget AFTER the action —
                    // match fields select the element in a fresh
                    // list_elements, the remaining keys are asserted
                    // booleans. `expect.state` =
                    //   {"match":{"text"|"desc"|"class"|"i":…},
                    //    "checked":bool,"selected":bool,
                    //    "enabled":bool,"focused":bool}
                    // text/desc match is substring; class is the simple
                    // name. Any state key may be true or false — a
                    // checkbox toggle verifies both ways (B4 gap).
                    // `match.i` resolves in the INTERACTIVE index space —
                    // the same numbering list_elements returns (its
                    // interactive_only default), i.e. the same space
                    // tap_index/set_text_index consume; derived locally
                    // from one dump via
                    // the same filter the facade applies. Given with
                    // other keys it ANDs: the element at i must also
                    // satisfy text/desc/class (i never short-circuits).
                    // text/desc/class alone search the FULL tree — a
                    // disabled/non-interactive node is a legitimate
                    // state-predicate target and text match carries no
                    // numbering ambiguity.
                    if let Some(st) = expect.get("state") {
                        let els = s.list_elements(false)?.elements;
                        let m = st.get("match").cloned().unwrap_or_else(|| json!({}));
                        let mi = m.get("i").and_then(Value::as_u64).map(|v| v as usize);
                        let t = str_arg(&m, "text");
                        let d = str_arg(&m, "desc");
                        let c = str_arg(&m, "class");
                        let keys = |e: &ElemInfo| {
                            t.as_ref().map_or(true, |v| e.text.contains(v))
                                && d.as_ref().map_or(true, |v| e.desc.contains(v))
                                && c.as_ref().map_or(true, |v| e.class == *v)
                        };
                        let found = if let Some(i) = mi {
                            els.iter()
                                .filter(|e| e.clickable || e.editable || e.scrollable)
                                .enumerate()
                                .find(|(idx, e)| *idx == i && keys(e))
                                .map(|(_, e)| e)
                        } else {
                            els.iter()
                                .find(|e| (t.is_some() || d.is_some() || c.is_some()) && keys(e))
                        };
                        let mut mismatched = serde_json::Map::new();
                        let mut pass = found.is_some();
                        if let Some(e) = found {
                            for k in ["checked", "selected", "enabled", "focused"] {
                                let Some(want) = st.get(k).and_then(Value::as_bool) else {
                                    continue;
                                };
                                let got = match k {
                                    "checked" => e.checked,
                                    "selected" => e.selected,
                                    "enabled" => e.enabled,
                                    _ => e.focused,
                                };
                                if got != want {
                                    mismatched.insert(k.into(), json!({"want":want,"got":got}));
                                    pass = false;
                                }
                            }
                        }
                        checks.push(json!({
                            "kind": "state", "pass": pass,
                            "found": found.is_some(),
                            "mismatched": Value::Object(mismatched),
                        }));
                    }
                    anyhow::ensure!(!checks.is_empty(), "expect had no recognized check");
                    let pass = a_ok && checks.iter().all(|c| c["pass"] == true);
                    Ok(json!({
                        "action": {"tool": tool, "ok": a_ok, "detail": a_detail},
                        "settle_ms": settle,
                        "checks": checks,
                        "pass": pass,
                    }))
                }
                "batch" => {
                    let ops: Vec<BatchOp> = serde_json::from_value(
                        args.get("ops").cloned().unwrap_or_else(|| json!([])),
                    )?;
                    Ok(json!(s.batch(ops)?))
                }
                other => anyhow::bail!("unknown tool: {other}"),
            }
        })();
    match r {
        Ok(v) => (tool_ok(id, v), false),
        Err(e) => {
            let dead = is_dead_conn(&e);
            (tool_err(id, e), dead)
        }
    }
}

// ---- tool catalogue ----------------------------------------------------------

fn prop(t: &str, d: &str) -> Value {
    json!({"type": t, "description": d})
}

fn schema(props: &[(&str, Value)], required: &[&str]) -> Value {
    let mut m = Map::new();
    for (k, v) in props {
        m.insert(k.to_string(), v.clone());
    }
    json!({"type":"object","properties":m,"required":required})
}

fn tools() -> Value {
    let xy = schema(
        &[("x", prop("number", "x px")), ("y", prop("number", "y px"))],
        &["x", "y"],
    );
    let sel = json!({"type":"object","properties":{
        "text": prop("string","exact text"), "id": prop("string","resource-id"),
        "desc": prop("string","content-desc"), "className": prop("string","class FQN or simple name"),
        "scrollable": prop("boolean","scrollable attr"), "index": prop("integer","nth match"),
        "packageName": prop("string","owning package, exact"),
    }});
    json!([
        {"name":"tap","description":"tap (x,y) — INJECT MotionEvent down/up","inputSchema":xy},
        {"name":"long_click","description":"long press (x,y,duration_ms)","inputSchema":schema(
            &[("x",prop("number","x")),("y",prop("number","y")),("duration_ms",prop("integer","hold ms, default 500"))],
            &["x","y"])},
        {"name":"pinch_in","description":"two-finger pinch-in (zoom out) centered at (cx,cy); percent = % of radius travelled (CAP_MULTI_PTR)","inputSchema":schema(
            &[("cx",prop("number","center x px")),("cy",prop("number","center y px")),("radius_px",prop("number","start radius, default 300")),("percent",prop("number","0-100, default 50")),("duration_ms",prop("integer","default 300"))],
            &["cx","cy"])},
        {"name":"pinch_out","description":"two-finger pinch-out (zoom in) centered at (cx,cy); same args as pinch_in (CAP_MULTI_PTR)","inputSchema":schema(
            &[("cx",prop("number","center x px")),("cy",prop("number","center y px")),("radius_px",prop("number","start radius, default 300")),("percent",prop("number","0-100, default 50")),("duration_ms",prop("integer","default 300"))],
            &["cx","cy"])},
        {"name":"swipe","description":"swipe (x1,y1)→(x2,y2) over duration_ms","inputSchema":schema(
            &[("x1",prop("number","")),("y1",prop("number","")),("x2",prop("number","")),("y2",prop("number","")),("duration_ms",prop("integer","default 300"))],
            &["x1","y1","x2","y2"])},
        {"name":"press","description":"inject KeyEvent by android keycode (back=4, home=3, recent=187)","inputSchema":schema(
            &[("keycode",prop("integer","KEYCODE_*")),("meta",prop("integer","metaState"))],&["keycode"])},
        {"name":"set_text","description":"ACTION_SET_TEXT on focused node","inputSchema":schema(
            &[("text",prop("string","")),("clear",prop("boolean","clear first, default true"))],&["text"])},
        {"name":"exists","description":"selector exists? — device-side UiAutomation tree","inputSchema":sel.clone()},
        {"name":"query","description":"selector → {found,node{text,id,desc,bounds,class,...}}","inputSchema":sel.clone()},
        {"name":"watch","description":"device-side selector→action watch loop (CAP_WATCH, docs/02 §4.9) — blocks this session until the selector is matched+acted or timeout_ms","inputSchema":json!({
            "type":"object","allOf":[sel.clone()],"properties":{"action":prop("string","default click"),"timeout_ms":prop("integer","default 5000")}})},
        {"name":"watch_arm","description":"async watcher (CAP_WATCH_ASYNC, §4.9): returns {armed,watch_id} immediately; hits arrive on the media channel — read via watch_events, or push=true to get a notifications/pilotdroid/watch_hit notification between responses (hits interleaved into an active frame stream are still pushed via the session stash; client disconnect auto-disarms; session recovery re-arms rules automatically — see notifications/pilotdroid/session_recovered; an abnormal watcher death surfaces as notifications/pilotdroid/watch_dead and is re-armed bounded). Control ops stay unblocked while armed","inputSchema":json!({
            "type":"object","allOf":[sel.clone()],"properties":{"action":prop("string","default click"),"timeout_ms":prop("integer","auto-disarm deadline, optional"),"push":prop("boolean","emit MCP notification on hit instead of pull-only, default false")}})},
        {"name":"watch_disarm","description":"cancel an armed watcher by watch_id → {disarmed}; a fired (one-shot) watcher is already removed, so disarming it returns {disarmed:false} — not an error","inputSchema":schema(
            &[("watch_id",prop("integer","from watch_arm"))],&["watch_id"])},
        {"name":"watch_events","description":"read next media-channel event → {type:'watch_hit',hit:{watch_id,matched,waited_ms,action_ok,node}} or {type:'toast',...} or {type:'watch_dead',dead:{watch_id,error,...}} (agent ≥0.7.4 abnormal watcher death — serve auto re-arms bounded) or {type:'frame',...} when a stream is also active; no event within the deadline → {type:'timeout'} (normal result, not an error)","inputSchema":schema(&[("timeout_ms",prop("integer","default 5000, min 500"))],&[])},
        {"name":"wait_for","description":"selector + timeout_ms → device-side poll (BATCH wait_until)","inputSchema":json!({
            "type":"object","allOf":[sel.clone()],"properties":{"timeout_ms":prop("integer","default 5000")}})},
        {"name":"dump","description":"UI hierarchy XML","inputSchema":schema(&[],&[])},
        {"name":"list_elements","description":"flattened indexed element list [{i,text,desc,class,bounds,clickable,editable,scrollable}] + widget state flags emitted non-default only (checked/selected/focused:true, enabled:false) — agent observation surface; interactive_only (default true) keeps clickable/editable/scrollable nodes","inputSchema":schema(
            &[("interactive_only",prop("boolean","filter to interactive nodes, default true"))],&[])},
        {"name":"tap_index","description":"tap element i from the last list_elements snapshot (bounds center); fails with 'snapshot stale' if layout/pkg drifted — re-list","inputSchema":schema(
            &[("i",prop("integer","element index"))],&["i"])},
        {"name":"set_text_index","description":"focus element i (tap) then ACTION_SET_TEXT; same snapshot-stale guard as tap_index","inputSchema":schema(
            &[("i",prop("integer","element index")),("text",prop("string","")),("clear",prop("boolean","default true"))],
            &["i","text"])},
        {"name":"screenshot","description":"JPEG capture; path = file bypass (preferred), else base64","inputSchema":schema(
            &[("path",prop("string","output file")),("quality",prop("integer","1-100, default 80")),("scale",prop("number","(0,1] downscale"))],&[])},
        {"name":"scroll","description":"scroll inside a (scrollable) node; dir: forward|backward|to_end|to_beginning","inputSchema":json!({
            "type":"object","allOf":[sel.clone()],"properties":{"dir":prop("string","default forward")}})},
        {"name":"app_start","description":"launch app: monkey -p pkg or am start -n pkg/activity","inputSchema":schema(
            &[("pkg",prop("string","")),("activity",prop("string","optional activity"))],&["pkg"])},
        {"name":"app_stop","description":"am force-stop pkg","inputSchema":schema(&[("pkg",prop("string",""))],&["pkg"])},
        {"name":"app_current","description":"{package,activity} of the focused window","inputSchema":schema(&[],&[])},
        {"name":"stream_start","description":"start media-channel JPEG frame stream (§4.8)","inputSchema":schema(
            &[("fps",prop("integer","default 5")),("scale",prop("number","default 0.5")),("quality",prop("integer","default 70"))],&[])},
        {"name":"stream_read","description":"read n frames (default 1); to_dir = file bypass","inputSchema":schema(
            &[("n",prop("integer","default 1")),("to_dir",prop("string","write frame_{seq}.jpg here"))],&[])},
        {"name":"stream_stop","description":"stop stream; returns {frames_sent,dropped}","inputSchema":schema(&[],&[])},
        {"name":"describe_screen","description":"SoM-lite perception: a11y tree → token-budget map — dense element ids (document order) ↔ bounds center ↔ label/rid/simple-class/flags C=clickable K=checked S=selected F=focused D=disabled. Each element also carries `i` = the list_elements/tap_index/set_text_index/verify_after.match.i interactive index — feed it to tap_index (stale-guarded) instead of guessing: element `id`/`#n` is SoM space (tap by center only), `i` is tap space. NOTE: tap_index needs a list_elements snapshot first — describe_screen does not arm one; call list_elements once before indexed actions. ocr:true adds host-side tesseract fusion (optional dep — degrades to normal output + hint when absent): canvas/image text becomes ocr_only elements tappable by center. Same numbering space as screenshot_annotated and compat/som.py","inputSchema":schema(
            &[("max_label",prop("integer","label char cap, default 28")),("ocr",prop("boolean","OCR text fusion tier, needs local tesseract, default false"))],&[])},
        {"name":"screenshot_annotated","description":"SoM-lite perception: screenshot JPEG with the describe_screen element ids drawn as numbered boxes at their bounds (scale fixed 1.0; path = file bypass preferred, else base64)","inputSchema":schema(
            &[("path",prop("string","output file")),("quality",prop("integer","1-100, default 80"))],&[])},
        {"name":"verify_after","description":"closed-loop verify (no u2 equivalent): run one action op → settle_ms → assert on fresh dump. action={tool,args} (side-effect/read ops only); expect={exists:<sel>} and/or {absent:<sel>} and/or {text:<s>} and/or {layout_changed:<bool>} and/or {state:{match:{text|desc|class|i},checked|selected|enabled|focused:bool}} — the state kind asserts widget flags after the action (tap ok≠effect); match.i uses the same interactive index space as list_elements (ANDs with other match keys, never short-circuits); returns {action,checks,pass}","inputSchema":schema(
            &[("action",prop("object","{tool,args} — e.g. {tool:'tap',args:{x,y}}")),("settle_ms",prop("integer","default 500")),("expect",prop("object","{exists|absent|text|layout_changed|state}"))],
            &["action","expect"])},
        {"name":"batch","description":"BATCH op array — {ops:[{inject|wait_until|query|dump|screenshot|text}]}","inputSchema":schema(
            &[("ops",prop("array","ordered ops"))],&["ops"])},
    ])
}

// ---- base64 (tiny, no dep — serve needs it for path-less binaries) -----------

fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = c.iter().fold(0u32, |a, &b| (a << 8) | b as u32) << ((3 - c.len()) * 8);
        s.push(T[(n >> 18 & 63) as usize] as char);
        s.push(T[(n >> 12 & 63) as usize] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    s
}

// ---- tests: std fake agent + stub adb, no tokio, no device -----------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::BufReader;
    use std::net::{SocketAddr, TcpListener};
    use std::path::Path;
    use std::rc::Rc;
    use std::sync::Mutex;
    use std::thread;
    use std::time::Duration;

    use pilotdroid_adb::{AdbTransport, DeviceInfo, Result as AdbResult, ShellOutput};
    use pilotdroid_agent::Device;
    use pilotdroid_proto::*;

    #[derive(Default)]
    struct TestAdb {
        forwards: Rc<RefCell<Vec<(String, String, String)>>>,
    }

    impl AdbTransport for TestAdb {
        fn list_devices(&mut self) -> AdbResult<Vec<DeviceInfo>> {
            Ok(vec![])
        }
        fn shell(&mut self, _s: &str, _c: &str) -> AdbResult<ShellOutput> {
            Ok(ShellOutput {
                stdout: vec![],
                stderr: vec![],
                exit_code: Some(0),
            })
        }
        fn push(&mut self, _s: &str, _l: &Path, _r: &str) -> AdbResult<()> {
            Ok(())
        }
        fn pull(&mut self, _s: &str, _r: &str, _l: &Path) -> AdbResult<()> {
            Ok(())
        }
        fn forward(&mut self, s: &str, l: &str, r: &str) -> AdbResult<()> {
            self.forwards
                .borrow_mut()
                .push((s.into(), l.into(), r.into()));
            Ok(())
        }
        fn forward_remove(&mut self, _s: &str, _l: &str) -> AdbResult<()> {
            Ok(())
        }
    }

    /// Minimal control-socket fake: HELLO + INJECT + QUERY + DUMP +
    /// SCREENSHOT (tiny grey JPEG — enough for the som annotate path).
    const FAKE_DUMP: &str = concat!(
        r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>"#,
        r#"<hierarchy><node index="0" text="设置" resource-id="com.x:id/t" "#,
        r#"class="android.widget.TextView" package="com.x" content-desc="" "#,
        r#"clickable="true" checked="true" focused="true" "#,
        r#"bounds="[10,20][110,60]"/>"#,
        r#"<node index="1" text="" resource-id="com.x:id/e" "#,
        r#"class="android.widget.EditText" package="com.x" content-desc="输入框" "#,
        r#"clickable="true" bounds="[10,80][300,140]"/>"#,
        r#"<node index="2" text="标题" resource-id="" "#,
        r#"class="android.widget.TextView" package="com.x" content-desc="" "#,
        r#"clickable="false" enabled="false" "#,
        r#"bounds="[10,150][200,190]"/></hierarchy>"#,
    );

    /// Dump where the first node is NON-interactive — the interactive
    /// index space diverges from the full-tree space:
    ///   full:       0=装饰(non-click) 1=CheckBox 2=Label
    ///   interactive:                0=CheckBox    1=Label
    const FAKE_DUMP_DIVERGENT: &str = concat!(
        r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>"#,
        r#"<hierarchy><node index="0" text="装饰" resource-id="" "#,
        r#"class="android.widget.TextView" package="com.x" content-desc="" "#,
        r#"clickable="false" bounds="[10,10][200,40]"/>"#,
        r#"<node index="1" text="Checkbox 1" resource-id="com.x:id/cb" "#,
        r#"class="android.widget.CheckBox" package="com.x" content-desc="" "#,
        r#"clickable="true" checked="false" bounds="[10,50][300,100]"/>"#,
        r#"<node index="2" text="Label" resource-id="com.x:id/l" "#,
        r#"class="android.widget.TextView" package="com.x" content-desc="" "#,
        r#"clickable="true" enabled="false" bounds="[10,110][300,150]"/></hierarchy>"#,
    );

    fn spawn_fake() -> SocketAddr {
        spawn_fake_dump(FAKE_DUMP)
    }

    /// Same fake protocol surface, caller-supplied DUMP xml — for tests
    /// that need a different a11y tree shape.
    fn spawn_fake_dump(xml: &'static str) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
            .encode_image(&image::RgbImage::from_pixel(360, 200, image::Rgb([180; 3])))
            .unwrap();
        thread::spawn(move || {
            while let Ok((mut sock, _)) = listener.accept() {
                while let Ok(req) = Frame::read_from(&mut sock) {
                    let (ty, payload) = match OpCode::try_from(req.header.msg_type) {
                        Ok(OpCode::Hello) => (
                            OpCode::HelloAck as u16,
                            encode(&HelloAck {
                                agent: "fake/0".into(),
                                caps: caps::INJECT | caps::QUERY,
                                ver: Some("9.9.9-fake".into()),
                                proto: None,
                                sdk: None,
                                abi: None,
                                display: None,
                                inject: None,
                            })
                            .unwrap(),
                        ),
                        Ok(OpCode::Inject) => (
                            OpCode::InjectResult as u16,
                            encode(&InjectAck {
                                ok: true,
                                events: 2,
                                ms: 1.0,
                                cost_ns: None,
                                backend: None,
                            })
                            .unwrap(),
                        ),
                        Ok(OpCode::Dump) => (
                            OpCode::Dump as u16,
                            encode(&DumpAck { xml: xml.into() }).unwrap(),
                        ),
                        Ok(OpCode::Query) => (
                            OpCode::Query as u16,
                            encode(&QueryAck {
                                found: true,
                                node: Some(NodeInfo {
                                    text: "t".into(),
                                    id: "p:id/n".into(),
                                    desc: String::new(),
                                    bounds: [0, 0, 10, 10],
                                    class: "android.widget.Button".into(),
                                    clickable: true,
                                    enabled: true,
                                    scrollable: None,
                                }),
                            })
                            .unwrap(),
                        ),
                        Ok(OpCode::Screenshot) => (
                            OpCode::Screenshot as u16,
                            encode(&ShotAck {
                                format: "jpeg".into(),
                                data: serde_bytes::ByteBuf::from(jpeg.clone()),
                                w: Some(360),
                                h: Some(200),
                                scale: Some(1.0),
                                take_ns: None,
                                enc_ns: None,
                            })
                            .unwrap(),
                        ),
                        _ => {
                            let p = encode(&ErrorAck {
                                code: "UNSUPPORTED".into(),
                                message: "fake".into(),
                            })
                            .unwrap();
                            let resp = Frame::new(
                                req.header.channel,
                                req.header.msg_type,
                                flags::RESPONSE | flags::ERROR,
                                req.header.req_id,
                                p,
                            );
                            if resp.write_to(&mut sock).is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    let resp = Frame::new(
                        req.header.channel,
                        ty,
                        flags::RESPONSE,
                        req.header.req_id,
                        payload,
                    );
                    if resp.write_to(&mut sock).is_err() {
                        break;
                    }
                }
            }
        });
        addr
    }

    /// Fake whose DUMP answer flips on the second call — drives the
    /// snapshot-stale guard (layout signature differs at tap_index time).
    fn spawn_fake_flipping() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let dumps = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        thread::spawn(move || {
            while let Ok((mut sock, _)) = listener.accept() {
                while let Ok(req) = Frame::read_from(&mut sock) {
                    let (ty, payload) = match OpCode::try_from(req.header.msg_type) {
                        Ok(OpCode::Hello) => (
                            OpCode::HelloAck as u16,
                            encode(&HelloAck {
                                agent: "fake/0".into(),
                                caps: caps::INJECT | caps::QUERY,
                                ver: Some("9.9.9-fake".into()),
                                proto: None,
                                sdk: None,
                                abi: None,
                                display: None,
                                inject: None,
                            })
                            .unwrap(),
                        ),
                        Ok(OpCode::Dump) => {
                            let n = dumps.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let xml = if n == 0 {
                                concat!(
                                    r#"<?xml version='1.0'?><hierarchy>"#,
                                    r#"<node text="A" class="android.widget.TextView" "#,
                                    r#"package="com.x" clickable="true" bounds="[0,0][10,10]"/>"#,
                                    r#"</hierarchy>"#,
                                )
                            } else {
                                // bounds drifted → signature mismatch
                                concat!(
                                    r#"<?xml version='1.0'?><hierarchy>"#,
                                    r#"<node text="A" class="android.widget.TextView" "#,
                                    r#"package="com.x" clickable="true" bounds="[0,0][20,20]"/>"#,
                                    r#"</hierarchy>"#,
                                )
                            };
                            (
                                OpCode::Dump as u16,
                                encode(&DumpAck { xml: xml.into() }).unwrap(),
                            )
                        }
                        _ => {
                            let p = encode(&ErrorAck {
                                code: "UNSUPPORTED".into(),
                                message: "fake".into(),
                            })
                            .unwrap();
                            let resp = Frame::new(
                                req.header.channel,
                                req.header.msg_type,
                                flags::RESPONSE | flags::ERROR,
                                req.header.req_id,
                                p,
                            );
                            if resp.write_to(&mut sock).is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    let resp = Frame::new(
                        req.header.channel,
                        ty,
                        flags::RESPONSE,
                        req.header.req_id,
                        payload,
                    );
                    if resp.write_to(&mut sock).is_err() {
                        break;
                    }
                }
            }
        });
        addr
    }

    /// Fake that kills its FIRST accepted conn right after the HELLO
    /// handshake (simulating a newer conn taking over, ADR-007); later
    /// conns get the normal fake — drives the dead-conn self-heal.
    fn spawn_fake_kill_first() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let first = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        thread::spawn(move || {
            while let Ok((mut sock, _)) = listener.accept() {
                let kill_after_hello = first.swap(false, std::sync::atomic::Ordering::SeqCst);
                let mut served_hello = false;
                while let Ok(req) = Frame::read_from(&mut sock) {
                    if kill_after_hello && served_hello {
                        break; // drop conn 1 — next op on it hits EOF
                    }
                    let (ty, payload) = match OpCode::try_from(req.header.msg_type) {
                        Ok(OpCode::Hello) => (
                            OpCode::HelloAck as u16,
                            encode(&HelloAck {
                                agent: "fake/0".into(),
                                caps: caps::INJECT | caps::QUERY,
                                ver: Some("9.9.9-fake".into()),
                                proto: None,
                                sdk: None,
                                abi: None,
                                display: None,
                                inject: None,
                            })
                            .unwrap(),
                        ),
                        Ok(OpCode::Inject) => (
                            OpCode::InjectResult as u16,
                            encode(&InjectAck {
                                ok: true,
                                events: 2,
                                ms: 1.0,
                                cost_ns: None,
                                backend: None,
                            })
                            .unwrap(),
                        ),
                        _ => {
                            let p = encode(&ErrorAck {
                                code: "UNSUPPORTED".into(),
                                message: "fake".into(),
                            })
                            .unwrap();
                            let resp = Frame::new(
                                req.header.channel,
                                req.header.msg_type,
                                flags::RESPONSE | flags::ERROR,
                                req.header.req_id,
                                p,
                            );
                            if resp.write_to(&mut sock).is_err() {
                                break;
                            }
                            continue;
                        }
                    };
                    let resp = Frame::new(
                        req.header.channel,
                        ty,
                        flags::RESPONSE,
                        req.header.req_id,
                        payload,
                    );
                    if resp.write_to(&mut sock).is_err() {
                        break;
                    }
                    if ty == OpCode::HelloAck as u16 {
                        served_hello = true;
                    }
                }
            }
        });
        addr
    }

    fn serve_once(lines: &str) -> Vec<Value> {
        serve_once_at(lines, spawn_fake())
    }

    fn serve_once_at(lines: &str, addr: SocketAddr) -> Vec<Value> {
        let mut adb = Some(TestAdb::default());
        let mut out = Vec::new();
        run(
            std::io::Cursor::new(lines.as_bytes().to_vec()),
            &mut out,
            move || {
                Device::with_transport(adb.take().unwrap(), "fake0")
                    .session_opts(Some(addr.port()), "pilotdroid_agent")
                    .map_err(Into::into)
            },
        )
        .unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// Bind an adjacent (control, control+1) port pair — the media
    /// socket must sit at control_port+1 for attach_media's heuristic.
    /// Probing avoids :0 races between parallel tests.
    fn watch_pair() -> (TcpListener, TcpListener, SocketAddr) {
        for _ in 0..60 {
            let probe = TcpListener::bind("127.0.0.1:0").unwrap();
            let p = probe.local_addr().unwrap().port();
            drop(probe);
            let Ok(ctrl) = TcpListener::bind(("127.0.0.1", p - 1)) else {
                continue;
            };
            let Ok(media) = TcpListener::bind(("127.0.0.1", p)) else {
                continue;
            };
            let addr = ctrl.local_addr().unwrap();
            return (ctrl, media, addr);
        }
        panic!("no free adjacent port pair for watch fake");
    }

    /// Media socket: push each (delay_ms, req_id, payload) as a ch=1
    /// msg_type 0x0002 ui_event, then hold the conn open.
    fn spawn_media(l: TcpListener, frames: Vec<(u64, u32, Vec<u8>)>) {
        thread::spawn(move || {
            if let Ok((mut msock, _)) = l.accept() {
                for (delay, rid, payload) in frames {
                    thread::sleep(Duration::from_millis(delay));
                    let f = Frame::new(
                        Channel::Media,
                        pilotdroid_proto::payload::MSG_WATCH_HIT,
                        flags::STREAM_MORE,
                        rid,
                        payload,
                    );
                    if f.write_to(&mut msock).is_err() {
                        break;
                    }
                }
                thread::sleep(Duration::from_secs(5));
            }
        });
    }

    /// Control side shared by watch fakes: WatchArm → {armed,
    /// watch_id:seq}, WatchDisarm counted.
    fn watch_ctrl(listener: TcpListener, disarms: std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicU32, Ordering};
        let arm_seq = std::sync::Arc::new(AtomicU32::new(0));
        thread::spawn(move || {
            while let Ok((mut sock, _)) = listener.accept() {
                let disarms = disarms.clone();
                let arm_seq = arm_seq.clone();
                thread::spawn(move || {
                    while let Ok(req) = Frame::read_from(&mut sock) {
                        let (ty, payload) = match OpCode::try_from(req.header.msg_type) {
                            Ok(OpCode::Hello) => (
                                OpCode::HelloAck as u16,
                                encode(&HelloAck {
                                    agent: "fake/0".into(),
                                    caps: caps::INJECT | caps::QUERY,
                                    ver: Some("9.9.9-fake".into()),
                                    proto: None,
                                    sdk: None,
                                    abi: None,
                                    display: None,
                                    inject: None,
                                })
                                .unwrap(),
                            ),
                            Ok(OpCode::WatchArm) => (
                                OpCode::WatchArm as u16,
                                encode(&WatchArmAck {
                                    armed: true,
                                    watch_id: arm_seq.fetch_add(1, Ordering::SeqCst) + 1,
                                })
                                .unwrap(),
                            ),
                            Ok(OpCode::WatchDisarm) => {
                                disarms.fetch_add(1, Ordering::SeqCst);
                                (
                                    OpCode::WatchDisarm as u16,
                                    encode(&WatchDisarmAck { disarmed: true }).unwrap(),
                                )
                            }
                            Ok(OpCode::StreamCtrl) => {
                                let r: StreamCtrlReq = decode(&req.payload).unwrap();
                                let p = match r.op {
                                    StreamOp::Start => encode(&StreamStartAck {
                                        ok: true,
                                        socket: "pilotdroid_agent_media".into(),
                                        effective_fps_hint: None,
                                    })
                                    .unwrap(),
                                    StreamOp::Stop => encode(&StreamStopAck {
                                        ok: true,
                                        frames_sent: 3,
                                        dropped: 0,
                                    })
                                    .unwrap(),
                                };
                                (OpCode::StreamCtrl as u16, p)
                            }
                            _ => {
                                let p = encode(&ErrorAck {
                                    code: "UNSUPPORTED".into(),
                                    message: "fake".into(),
                                })
                                .unwrap();
                                let resp = Frame::new(
                                    req.header.channel,
                                    req.header.msg_type,
                                    flags::RESPONSE | flags::ERROR,
                                    req.header.req_id,
                                    p,
                                );
                                if resp.write_to(&mut sock).is_err() {
                                    break;
                                }
                                continue;
                            }
                        };
                        let resp = Frame::new(
                            req.header.channel,
                            ty,
                            flags::RESPONSE,
                            req.header.req_id,
                            payload,
                        );
                        if resp.write_to(&mut sock).is_err() {
                            break;
                        }
                    }
                });
            }
        });
    }

    /// Watch-capable fake: WatchArm → {armed, watch_id:seq},
    /// WatchDisarm counted; a media listener at addr+1 pushes the given
    /// WATCH_HIT frames at the given delays, then stays open.
    fn spawn_fake_watch(
        hits: Vec<(u64, u32)>,
    ) -> (SocketAddr, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let (ctrl, media, addr) = watch_pair();
        let disarms = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        watch_ctrl(ctrl, disarms.clone());
        let frames = hits
            .into_iter()
            .map(|(delay, wid)| {
                (
                    delay,
                    wid,
                    encode(&WatchHit {
                        watch_id: wid,
                        matched: true,
                        waited_ms: delay,
                        action_ok: Some(true),
                        node: None,
                    })
                    .unwrap(),
                )
            })
            .collect();
        spawn_media(media, frames);
        (addr, disarms)
    }

    /// Watch fake whose media channel pushes a `watch_dead` ui_event
    /// (agent ≥0.7.4 abnormal watcher death) — same 0x0002 slot,
    /// discriminated by `type:"watch_dead"`. The control side re-arms
    /// (watch_ctrl hands out the next watch_id).
    fn spawn_fake_watch_dead() -> SocketAddr {
        let (ctrl, media, addr) = watch_pair();
        watch_ctrl(
            ctrl,
            std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        );
        spawn_media(
            media,
            vec![(
                120,
                0,
                encode(&WatchDeadEvent {
                    r#type: "watch_dead".into(),
                    watch_id: 1,
                    waited_ms: 500,
                    error: "poll fault".into(),
                })
                .unwrap(),
            )],
        );
        addr
    }

    /// Watch fake whose media channel pushes a toast ui_event (agent
    /// ≥0.7.1 a11y listener) instead of a WATCH_HIT — same ch=1
    /// msg_type 0x0002 slot, discriminated by `type:"toast"`.
    fn spawn_fake_toast() -> SocketAddr {
        let (ctrl, media, addr) = watch_pair();
        let disarms = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        watch_ctrl(ctrl, disarms);
        spawn_media(
            media,
            vec![(
                80,
                0,
                encode(&pilotdroid_proto::payload::ToastEvent {
                    r#type: "toast".into(),
                    pkg: "com.x".into(),
                    has_parcel: false,
                    text: vec!["hi toast".into()],
                })
                .unwrap(),
            )],
        );
        addr
    }

    /// Revivable fake: conn 1 serves `kill_after` non-HELLO requests
    /// then dies (agent crash); later conns serve normally. Watch ids
    /// restart at 1 per conn (a restarted agent re-allocates) and
    /// WatchDisarm calls are counted per conn — the test asserts the
    /// post-recovery conn only sees disarms for its own watchers.
    /// Returns (control addr, per-conn disarm counts).
    fn spawn_fake_revive(kill_after: usize) -> (SocketAddr, std::sync::Arc<Mutex<Vec<usize>>>) {
        use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
        let (ctrl, media, addr) = watch_pair();
        let disarms: std::sync::Arc<Mutex<Vec<usize>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        let conn_seq = std::sync::Arc::new(AtomicUsize::new(0));
        {
            let disarms = disarms.clone();
            thread::spawn(move || {
                while let Ok((mut sock, _)) = ctrl.accept() {
                    let idx = conn_seq.fetch_add(1, Ordering::SeqCst);
                    disarms.lock().unwrap().push(0);
                    let disarms = disarms.clone();
                    thread::spawn(move || {
                        let arm_seq = AtomicU32::new(0);
                        let mut served = 0usize;
                        while let Ok(req) = Frame::read_from(&mut sock) {
                            let (ty, payload) = match OpCode::try_from(req.header.msg_type) {
                                Ok(OpCode::Hello) => (
                                    OpCode::HelloAck as u16,
                                    encode(&HelloAck {
                                        agent: "fake/0".into(),
                                        caps: caps::INJECT | caps::QUERY,
                                        ver: Some("9.9.9-fake".into()),
                                        proto: None,
                                        sdk: None,
                                        abi: None,
                                        display: None,
                                        inject: None,
                                    })
                                    .unwrap(),
                                ),
                                Ok(OpCode::WatchArm) => (
                                    OpCode::WatchArm as u16,
                                    encode(&WatchArmAck {
                                        armed: true,
                                        watch_id: arm_seq.fetch_add(1, Ordering::SeqCst) + 1,
                                    })
                                    .unwrap(),
                                ),
                                Ok(OpCode::WatchDisarm) => {
                                    disarms.lock().unwrap()[idx] += 1;
                                    (
                                        OpCode::WatchDisarm as u16,
                                        encode(&WatchDisarmAck { disarmed: true }).unwrap(),
                                    )
                                }
                                Ok(OpCode::Dump) => (
                                    OpCode::Dump as u16,
                                    encode(&DumpAck {
                                        xml: "<hierarchy/>".into(),
                                    })
                                    .unwrap(),
                                ),
                                _ => continue,
                            };
                            let resp = Frame::new(
                                req.header.channel,
                                ty,
                                flags::RESPONSE,
                                req.header.req_id,
                                payload,
                            );
                            if resp.write_to(&mut sock).is_err() {
                                break;
                            }
                            if ty != OpCode::HelloAck as u16 {
                                served += 1;
                                if idx == 0 && served >= kill_after {
                                    // Agent "crashes": conn dies mid-
                                    // session — next client op sees EOF.
                                    let _ = sock.shutdown(std::net::Shutdown::Both);
                                    break;
                                }
                            }
                        }
                    });
                }
            });
        }
        // Media conn: accept and hold each conn (watch_attach needs
        // it; a dropped conn reads as EOF and stops the demux early).
        // No events are pushed for this test.
        thread::spawn(move || {
            let mut held = Vec::new();
            while let Ok((s, _)) = media.accept() {
                held.push(s);
            }
        });
        (addr, disarms)
    }

    #[test]
    fn watch_push_hit_arrives_as_notification() {
        // arm(push:true) → hit pushed as a JSON-RPC notification between
        // responses (demux); every stdout line stays a complete frame —
        // single-writer loop means no interleave tearing by construction.
        let (addr, _disarms) = spawn_fake_watch(vec![(80, 1)]);
        let resps = serve_once_at(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":"watch_arm","arguments":{"text":"OK","push":true}}})
            .to_string(),
            addr,
        );
        assert_eq!(resps[0]["result"]["isError"], false);
        let body: Value =
            serde_json::from_str(resps[0]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body["armed"], true);
        let notes: Vec<&Value> = resps
            .iter()
            .filter(|v| v["method"].as_str() == Some("notifications/pilotdroid/watch_hit"))
            .collect();
        assert_eq!(
            notes.len(),
            1,
            "expected one watch_hit notification: {resps:?}"
        );
        assert_eq!(notes[0]["params"]["watch_id"], 1);
        assert_eq!(notes[0]["params"]["matched"], true);
    }

    #[test]
    fn watch_push_eof_disarms_armed() {
        // Client stdin EOF while a push watcher is armed → serve
        // auto-disarms it before exit (armed rules must not outlive the
        // MCP session — Track A demux contract).
        let (addr, disarms) = spawn_fake_watch(vec![]);
        let _ = serve_once_at(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":"watch_arm","arguments":{"text":"OK","push":true}}})
            .to_string(),
            addr,
        );
        // The EOF-disarm may still be in flight right at return — wait.
        for _ in 0..40 {
            if disarms.load(std::sync::atomic::Ordering::SeqCst) >= 1 {
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
        panic!("no disarm issued on client EOF");
    }

    #[test]
    fn watch_push_mixed_hit_buffers_to_watch_events() {
        // push watcher + pull watcher armed together: the non-push hit
        // consumed by the demux buffers for watch_events (degrade path);
        // the push hit surfaces as a notification.
        let (addr, _disarms) = spawn_fake_watch(vec![(60, 2), (120, 1)]);
        let resps = serve_once_at(
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                       "params":{"name":"watch_arm","arguments":{"text":"OK","push":true}}})
                .to_string(),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                       "params":{"name":"watch_arm","arguments":{"text":"NO","push":false}}})
                .to_string(),
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
                       "params":{"name":"watch_events","arguments":{}}})
                .to_string(),
            ]
            .join("\n"),
            addr,
        );
        let notes: Vec<&Value> = resps
            .iter()
            .filter(|v| v["method"].as_str() == Some("notifications/pilotdroid/watch_hit"))
            .collect();
        assert_eq!(notes.len(), 1, "only the push hit notifies: {resps:?}");
        assert_eq!(notes[0]["params"]["watch_id"], 1);
        let ev: Value =
            serde_json::from_str(resps[3]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(ev["type"], "watch_hit");
        assert_eq!(ev["hit"]["watch_id"], 2, "non-push hit must be pullable");
    }

    #[test]
    fn verify_after_runs_action_then_asserts() {
        // tap + exists(text) → pass; absent(...) → not found → also pass.
        let resps = serve_once(
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                       "params":{"name":"verify_after","arguments":{
                           "action":{"tool":"tap","args":{"x":1,"y":2}},
                           "settle_ms":10,
                           "expect":{"exists":{"text":"OK"}}}}})
                .to_string(),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                       "params":{"name":"verify_after","arguments":{
                           "action":{"tool":"tap","args":{"x":1,"y":2}},
                           "settle_ms":10,
                           "expect":{"absent":{"text":"NEVER_THERE"}}}}})
                .to_string(),
            ]
            .join("\n"),
        );
        // Fake Query always found=true → exists passes, absent fails.
        for (i, want_pass) in [(0usize, true), (1usize, false)] {
            let body: Value =
                serde_json::from_str(resps[i]["result"]["content"][0]["text"].as_str().unwrap())
                    .unwrap();
            assert_eq!(body["pass"], want_pass, "case {i}: {body}");
            assert_eq!(body["action"]["ok"], true);
        }
        let body: Value =
            serde_json::from_str(resps[0]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body["checks"][0]["kind"], "exists");
    }

    #[test]
    fn verify_after_state_predicate() {
        // The fake dump: node0 设置 checked+focused, node2 标题
        // enabled=false. State checks must read post-action widget
        // flags — both true and false assertions, plus a mismatch.
        let resps = serve_once(
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                       "params":{"name":"verify_after","arguments":{
                           "action":{"tool":"tap","args":{"x":1,"y":2}},
                           "settle_ms":10,
                           "expect":{"state":{"match":{"text":"设置"},
                                              "checked":true}}}}})
                .to_string(),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                       "params":{"name":"verify_after","arguments":{
                           "action":{"tool":"tap","args":{"x":1,"y":2}},
                           "settle_ms":10,
                           "expect":{"state":{"match":{"text":"标题"},
                                              "enabled":false}}}}})
                .to_string(),
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
                       "params":{"name":"verify_after","arguments":{
                           "action":{"tool":"tap","args":{"x":1,"y":2}},
                           "settle_ms":10,
                           "expect":{"state":{"match":{"text":"设置"},
                                              "checked":false}}}}})
                .to_string(),
            ]
            .join("\n"),
        );
        let bodies: Vec<Value> = resps
            .iter()
            .map(|r| {
                serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
            })
            .collect();
        assert_eq!(bodies[0]["pass"], true, "checked:true hit: {bodies:?}");
        assert_eq!(bodies[0]["checks"][0]["kind"], "state");
        assert_eq!(bodies[1]["pass"], true, "enabled:false hit: {bodies:?}");
        assert_eq!(bodies[2]["pass"], false, "mismatch must fail");
        assert_eq!(bodies[2]["checks"][0]["mismatched"]["checked"]["got"], true);
    }

    #[test]
    fn verify_after_state_match_i_interactive_space() {
        // FAKE_DUMP_DIVERGENT: full-tree [装饰0, Checkbox1, Label2] vs
        // interactive [Checkbox0, Label1]. match.i must resolve in the
        // INTERACTIVE space (same numbering as list_elements), and i
        // must AND with other match keys — never short-circuit.
        let call = |expect: Value| {
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                   "params":{"name":"verify_after","arguments":{
                       "action":{"tool":"tap","args":{"x":1,"y":2}},
                       "settle_ms":10,
                       "expect": expect}}})
            .to_string()
        };
        let resps = serve_once_at(
            &[
                // i=2 exists only in the full tree — interactive space
                // has no such index → not found.
                call(json!({"state":{"match":{"i":2}, "checked":false}})),
                // i=1 interactive = Label; text "Checkbox 1" must also
                // hold — i must not short-circuit the other keys.
                call(json!({"state":{"match":{"i":1,"text":"Checkbox 1"},
                                    "checked":false}})),
                // Positive AND: i=1 + text Label + enabled:false all hold.
                call(json!({"state":{"match":{"i":1,"text":"Label"},
                                    "enabled":false}})),
                // Positive i-only in interactive space: i=0 = CheckBox.
                call(json!({"state":{"match":{"i":0}, "checked":false}})),
            ]
            .join("\n"),
            spawn_fake_dump(FAKE_DUMP_DIVERGENT),
        );
        let bodies: Vec<Value> = resps
            .iter()
            .map(|r| {
                serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
            })
            .collect();
        assert_eq!(
            bodies[0]["checks"][0]["found"], false,
            "i=2 must not resolve in interactive space: {bodies:?}"
        );
        assert_eq!(
            bodies[1]["checks"][0]["found"], false,
            "i+text must AND, not short-circuit: {bodies:?}"
        );
        assert_eq!(bodies[2]["checks"][0]["found"], true, "{bodies:?}");
        assert_eq!(
            bodies[2]["pass"], true,
            "enabled:false on interactive i=1: {bodies:?}"
        );
        assert_eq!(bodies[3]["checks"][0]["found"], true, "{bodies:?}");
        assert_eq!(
            bodies[3]["pass"], true,
            "checked:false on interactive i=0: {bodies:?}"
        );
    }

    #[test]
    fn watch_dead_event_notifies_and_rearms() {
        // armed push watcher dies abnormally agent-side → watch_dead
        // ui_event → notification (never silent) + bounded auto re-arm.
        let resps = serve_once_at(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":"watch_arm",
                              "arguments":{"text":"OK","push":true}}})
            .to_string(),
            spawn_fake_watch_dead(),
        );
        let notes: Vec<_> = resps
            .iter()
            .filter(|v| v["method"].as_str() == Some("notifications/pilotdroid/watch_dead"))
            .collect();
        assert_eq!(notes.len(), 1, "watch_dead must notify: {resps:?}");
        assert_eq!(notes[0]["params"]["error"], "poll fault");
        assert_eq!(notes[0]["params"]["armed"], true);
        assert_eq!(notes[0]["params"]["rearmed"], true);
        assert_eq!(notes[0]["params"]["new_watch_id"], 2);
    }

    #[test]
    fn describe_screen_and_annotated_shot() {
        // SoM tools over the fake dump: 3 useful nodes (the fake's
        // non-clickable node carries text → still useful per the filter).
        let resps = serve_once(
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                       "params":{"name":"describe_screen","arguments":{}}})
                .to_string(),
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                       "params":{"name":"screenshot_annotated","arguments":{}}})
                .to_string(),
            ]
            .join("\n"),
        );
        let body: Value =
            serde_json::from_str(resps[0]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body["elements"].as_array().unwrap().len(), 3);
        assert!(body["text"].as_str().unwrap().contains("#1 [60,40]"));
        let ann: Value =
            serde_json::from_str(resps[1]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(ann["format"], "jpeg");
        assert!(ann["bytes"].as_u64().unwrap() > 100);
        assert!(ann["b64"].as_str().unwrap().len() > 100);
    }

    #[test]
    fn watch_events_quiet_media_reports_timeout_union() {
        // A media conn that accepts then stays silent → the read deadline
        // yields {type:"timeout"} as a normal result (isError stays false;
        // real errors still surface isError).
        let addr = spawn_fake();
        let media = TcpListener::bind(("127.0.0.1", addr.port() + 1)).unwrap();
        thread::spawn(move || {
            let _sock = media.accept().unwrap().0; // stay open, say nothing
            thread::sleep(Duration::from_secs(10));
        });
        let resps = serve_once_at(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":"watch_events","arguments":{}}})
            .to_string(),
            addr,
        );
        assert_eq!(resps[0]["result"]["isError"], false);
        let body: Value =
            serde_json::from_str(resps[0]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body["type"], "timeout");
    }

    #[test]
    fn handshake_then_tools_call_in_serial_order() {
        // MCP handshake sequence + pipelined tools/call: the serial queue
        // guarantees ordered, non-interleaved responses.
        let resps = serve_once(
            &[
                r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
                r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tap","arguments":{"x":1,"y":2}}}"#,
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"exists","arguments":{"text":"OK"}}}"#,
            ]
            .join("\n"),
        );
        assert_eq!(resps.len(), 4, "notification must not get a response");
        assert_eq!(resps[0]["id"], 0);
        assert_eq!(resps[0]["result"]["serverInfo"]["name"], "pilotdroid");
        assert_eq!(resps[0]["result"]["protocolVersion"], "2024-11-05");
        // initialize eagerly connected → HELLO.ver is transparent through.
        assert_eq!(resps[0]["result"]["serverInfo"]["agentVer"], "9.9.9-fake");
        // tools/list: full v1 catalogue incl. stream tools.
        let names: Vec<&str> = resps[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        for t in [
            "tap",
            "stream_start",
            "stream_read",
            "scroll",
            "app_current",
        ] {
            assert!(names.contains(&t), "missing tool {t} in {names:?}");
        }
        // tools/call responses in request order (serial queue, no interleave).
        assert_eq!(resps[2]["id"], 2);
        assert_eq!(resps[3]["id"], 3);
        assert_eq!(resps[2]["result"]["isError"], false);
        let body: Value =
            serde_json::from_str(resps[2]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body["ok"], true);
        let body3: Value =
            serde_json::from_str(resps[3]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body3["exists"], true);
    }

    #[test]
    fn protocol_version_whitelist_negotiation() {
        // Supported offer → echoed; unknown offer → our newest, never
        // an echo of a version we don't implement.
        let resps = serve_once(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"9999-01-01","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
            ]
            .join("\n"),
        );
        assert_eq!(resps[0]["result"]["protocolVersion"], "2025-11-25");
        assert_eq!(resps[1]["result"]["protocolVersion"], "2025-11-25");
    }

    #[test]
    fn element_index_tools() {
        // Observation surface: list → tap_index → set_text_index all
        // resolve bounds-center from the same snapshot.
        let resps = serve_once(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_elements","arguments":{}}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tap_index","arguments":{"i":1}}}"#,
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"tap_index","arguments":{"i":9}}}"#,
            ]
            .join("\n"),
        );
        let body: Value =
            serde_json::from_str(resps[0]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body["count"], 2);
        let e0 = &body["elements"][0];
        assert_eq!(e0["i"], 0);
        assert_eq!(e0["text"], "设置");
        assert_eq!(e0["class"], "TextView"); // simple name
        assert_eq!(e0["bounds"], json!([10, 20, 110, 60]));
        assert_eq!(e0["clickable"], true);
        assert_eq!(body["elements"][1]["editable"], true); // EditText
                                                           // tap_index hits the EditText center: (10+300)/2=155, (80+140)/2=110
        let tap: Value =
            serde_json::from_str(resps[1]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(tap["i"], 1);
        assert_eq!(tap["result"]["ok"], true);
        // out-of-range index → tool error, not protocol error
        assert_eq!(resps[2]["result"]["isError"], true);
    }

    #[test]
    fn list_elements_interactive_only_filters() {
        // Default keeps only interactive nodes (the non-clickable 标题
        // TextView is filtered out); interactive_only=false lists all 3.
        let resps = serve_once(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_elements","arguments":{"interactive_only":false}}}"#,
            ]
            .join("\n"),
        );
        let body: Value =
            serde_json::from_str(resps[0]["result"]["content"][0]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(body["count"], 3);
        assert_eq!(body["interactive_only"], false);
        assert_eq!(body["elements"][2]["text"], "标题");
        assert!(body["elements"][2].get("clickable").is_none()); // false elided
    }

    #[test]
    fn tap_index_rejects_stale_snapshot() {
        // list_elements snapshots layout A; the flipping fake serves
        // layout B at guard time → "snapshot stale" tool error.
        let resps = serve_once_at(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_elements","arguments":{}}}"#,
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tap_index","arguments":{"i":0}}}"#,
            ]
            .join("\n"),
            spawn_fake_flipping(),
        );
        assert_eq!(resps[1]["result"]["isError"], true);
        let text = resps[1]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("stale"), "expected stale error, got {text}");
    }

    #[test]
    fn index_tools_require_snapshot_first() {
        // tap_index before any list_elements → tool error naming the fix.
        let resps = serve_once(
            &[
                r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tap_index","arguments":{"i":0}}}"#,
            ]
            .join("\n"),
        );
        assert_eq!(resps[0]["result"]["isError"], true);
        let text = resps[0]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("list_elements"), "got {text}");
    }

    #[test]
    fn dead_conn_self_heals_next_call() {
        // The fake kills conn 1 right after HELLO (a newer conn took over
        // — ADR-007). Call 1 hits EOF mid-op → Session::with_client runs
        // the recovery ladder and replays it transparently (t14 session
        // watchdog / atx-agent semantics) — the caller never sees the
        // crash; call 2 continues on the healed conn.
        let addr = spawn_fake_kill_first();
        let mut out = Vec::new();
        run(
            std::io::Cursor::new(
                [
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tap","arguments":{"x":1,"y":2}}}"#,
                    r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tap","arguments":{"x":3,"y":4}}}"#,
                ]
                .join("\n")
                .into_bytes(),
            ),
            &mut out,
            // fresh transport per connect — exercises the reconnect path
            move || {
                Device::with_transport(TestAdb::default(), "fake0")
                    .session_opts(Some(addr.port()), "pilotdroid_agent")
                    .map_err(Into::into)
            },
        )
        .unwrap();
        let lines: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        // The transparent recovery surfaces as a session_recovered
        // notification on stdout — id-less, interleaved with responses.
        let notes: Vec<&Value> = lines
            .iter()
            .filter(|v| v["method"].as_str() == Some("notifications/pilotdroid/session_recovered"))
            .collect();
        assert_eq!(notes.len(), 1, "recovery must notify: {lines:?}");
        assert_eq!(notes[0]["params"]["generation"], 1);
        assert_eq!(notes[0]["params"]["stale_watchers"], 0);
        let resps: Vec<&Value> = lines.iter().filter(|v| v["id"].is_u64()).collect();
        assert_eq!(resps.len(), 2);
        for (i, r) in resps.iter().enumerate() {
            assert_eq!(
                r["result"]["isError"], false,
                "call {i} should succeed via transparent recovery: {r}"
            );
            let text = r["result"]["content"][0]["text"].as_str().unwrap();
            assert!(
                text.contains("\"ok\":true") || text.contains("ok"),
                "call {i} got {text}"
            );
        }
    }

    #[test]
    fn unknown_method_and_bad_json_errors() {
        let resps = serve_once(
            &[
                "not json at all",
                r#"{"jsonrpc":"2.0","id":9,"method":"bogus/method"}"#,
            ]
            .join("\n"),
        );
        assert_eq!(resps[0]["error"]["code"], -32700);
        assert_eq!(resps[1]["error"]["code"], -32601);
        assert_eq!(resps[1]["id"], 9);
    }

    #[test]
    fn stdout_carries_only_json_lines() {
        // stdout purity: every line must be a complete JSON object — agent
        // callers parse stdout byte-for-byte; banners/logs are stderr-only.
        let addr = spawn_fake();
        let mut adb = Some(TestAdb::default());
        let mut out = Vec::new();
        run(
            BufReader::new(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.as_bytes()),
            &mut out,
            move || {
                Device::with_transport(adb.take().unwrap(), "fake0")
                    .session_opts(Some(addr.port()), "pilotdroid_agent")
                    .map_err(Into::into)
            },
        )
        .unwrap();
        for line in String::from_utf8(out).unwrap().lines() {
            assert!(line.starts_with('{'), "non-JSON on stdout: {line}");
            serde_json::from_str::<Value>(line).unwrap();
        }
    }

    #[test]
    fn b64_roundtrip_shape() {
        // spot-check encoder against RFC 4648 vectors.
        assert_eq!(b64(b""), "");
        assert_eq!(b64(b"f"), "Zg==");
        assert_eq!(b64(b"fo"), "Zm8=");
        assert_eq!(b64(b"foo"), "Zm9v");
        assert_eq!(b64(b"fooba"), "Zm9vYmE=");
        assert_eq!(b64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn toast_event_arrives_as_notification() {
        // Agent ≥0.7.1 pushes {type:"toast"} on the same ui_event slot
        // as WATCH_HIT; while a push watcher keeps the demux polling,
        // it must surface as notifications/pilotdroid/toast (not fail
        // the WatchHit decode that made the demux stop on t15's media).
        let addr = spawn_fake_toast();
        let resps = serve_once_at(
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                    "params":{"name":"watch_arm","arguments":{"text":"OK","push":true}}})
            .to_string(),
            addr,
        );
        assert_eq!(resps[0]["result"]["isError"], false);
        let notes: Vec<&Value> = resps
            .iter()
            .filter(|v| v["method"].as_str() == Some("notifications/pilotdroid/toast"))
            .collect();
        assert_eq!(notes.len(), 1, "expected toast notification: {resps:?}");
        assert_eq!(notes[0]["params"]["type"], "toast");
        assert_eq!(notes[0]["params"]["pkg"], "com.x");
        assert_eq!(notes[0]["params"]["text"][0], "hi toast");
    }

    #[test]
    fn armed_watchers_rearmed_on_recovery_epoch() {
        // Watch ids are agent-allocated and restart at 1 after a crash:
        // armed={1,2} pre-crash → recovery → the epoch check re-arms both
        // rules on the new agent (fresh ids {1,2} again), notifies the
        // client via session_recovered, then the explicit arm gets id 3.
        // EOF cleanup disarms exactly the live watchers on conn2 (3) —
        // no stale disarm leaks, and the re-armed rules stay tracked.
        let (addr, disarms) = spawn_fake_revive(2);
        let arm = |id: u64| {
            json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
                   "params":{"name":"watch_arm","arguments":{"text":"OK","push":true}}})
            .to_string()
        };
        let resps = serve_once_at(
            &[
                arm(1),
                arm(2),
                // conn dies here; the dump triggers transparent recovery
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"dump","arguments":{}}}"#
                    .to_string(),
                arm(4),
            ]
            .join("\n"),
            addr,
        );
        let notes: Vec<&Value> = resps
            .iter()
            .filter(|v| v["method"].as_str() == Some("notifications/pilotdroid/session_recovered"))
            .collect();
        let resps: Vec<&Value> = resps.iter().filter(|v| v["id"].is_u64()).collect();
        assert_eq!(resps[0]["result"]["isError"], false);
        assert_eq!(resps[1]["result"]["isError"], false);
        // Post-recovery dump must succeed transparently.
        assert_eq!(
            resps[2]["result"]["isError"], false,
            "dump after recovery: {resps:?}"
        );
        assert_eq!(resps[3]["result"]["isError"], false);
        assert_eq!(notes.len(), 1, "session_recovered must fire: {resps:?}");
        assert_eq!(notes[0]["params"]["generation"], 1);
        assert_eq!(notes[0]["params"]["stale_watchers"], 2);
        assert_eq!(notes[0]["params"]["rearmed"], 2);
        assert_eq!(notes[0]["params"]["rearm_failed"], 0);
        let counts = disarms.lock().unwrap();
        assert!(
            counts.len() >= 2,
            "recovery should have opened a second conn: {counts:?}"
        );
        assert_eq!(
            counts[1], 3,
            "conn2 disarms = 2 re-armed + 1 explicitly armed: {counts:?}"
        );
    }

    /// Stream-capable fake: control serves WatchArm/Disarm/StreamCtrl;
    /// media conn #1 (the watch_attach pre-attach) is held, conn #2 (the
    /// stream reader) gets FRAME/WATCH_HIT/FRAME/TOAST/FRAME — the hit
    /// and toast interleave into the frame stream the way the real
    /// agent interleaves ui_events on ch=1.
    fn spawn_fake_stream_hit() -> SocketAddr {
        let (ctrl, media, addr) = watch_pair();
        let disarms = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        watch_ctrl(ctrl, disarms);
        thread::spawn(move || {
            let mut held = Vec::new();
            let mut n = 0usize;
            while let Ok((mut msock, _)) = media.accept() {
                n += 1;
                if n < 2 {
                    held.push(msock);
                    continue;
                }
                thread::spawn(move || {
                    let frame = |seq: u32| {
                        Frame::new(
                            Channel::Media,
                            MediaType::Frame as u16,
                            flags::STREAM_MORE,
                            seq,
                            encode(&MediaFrame {
                                format: "jpeg".into(),
                                w: 2,
                                h: 2,
                                ts_ns: seq as u64,
                                take_ns: None,
                                enc_ns: None,
                                dropped: 0,
                                data: vec![seq as u8].into(),
                            })
                            .unwrap(),
                        )
                    };
                    let ui = |p: Vec<u8>| Frame::new(Channel::Media, MSG_WATCH_HIT, 0, 0, p);
                    let hit = ui(encode(&WatchHit {
                        watch_id: 1,
                        matched: true,
                        waited_ms: 5,
                        action_ok: Some(true),
                        node: None,
                    })
                    .unwrap());
                    let toast = ui(encode(&pilotdroid_proto::payload::ToastEvent {
                        r#type: "toast".into(),
                        pkg: "com.x".into(),
                        has_parcel: false,
                        text: vec!["mid-stream".into()],
                    })
                    .unwrap());
                    for f in [frame(1), hit, frame(2), toast, frame(3)] {
                        if f.write_to(&mut msock).is_err() {
                            return;
                        }
                    }
                    thread::sleep(Duration::from_secs(5));
                });
            }
        });
        addr
    }

    #[test]
    fn stashed_hit_and_toast_push_during_stream() {
        // Frames and ui_events share ch=1: while a frame stream is
        // active, recv_event sorts the interleaved hit/toast into the
        // session stash — the serve drain then pushes them as
        // notifications mid-stream (no pull, no wait for stream_stop).
        let addr = spawn_fake_stream_hit();
        let resps = serve_once_at(
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                       "params":{"name":"watch_arm","arguments":{"text":"OK","push":true}}})
                .to_string(),
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"stream_start","arguments":{"fps":5,"scale":0.5}}}"#.to_string(),
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"stream_read","arguments":{"n":3}}}"#.to_string(),
                r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"stream_stop","arguments":{}}}"#.to_string(),
            ]
            .join("\n"),
            addr,
        );
        // stream_read returned 3 frames with the hit/toast stashed.
        let text = resps[2]["result"]["content"][0]["text"].as_str().unwrap();
        assert_eq!(resps[2]["result"]["isError"], false, "stream_read: {text}");
        let hits: Vec<&Value> = resps
            .iter()
            .filter(|v| v["method"].as_str() == Some("notifications/pilotdroid/watch_hit"))
            .collect();
        assert_eq!(hits.len(), 1, "stashed hit must push mid-stream: {resps:?}");
        assert_eq!(hits[0]["params"]["watch_id"], 1);
        let toasts: Vec<&Value> = resps
            .iter()
            .filter(|v| v["method"].as_str() == Some("notifications/pilotdroid/toast"))
            .collect();
        assert_eq!(
            toasts.len(),
            1,
            "stashed toast must push mid-stream: {resps:?}"
        );
        assert_eq!(toasts[0]["params"]["text"][0], "mid-stream");
    }

    #[test]
    fn stale_watch_disarm_not_forwarded() {
        // watch_id 99 was never armed — forwarding it would let a stale
        // id kill a coincidentally re-allocated live watcher after a
        // recovery (t5 nit (a)). Unknown ids return disarmed:false
        // locally and never reach the agent; id 1 disarms normally.
        let (addr, disarms) = spawn_fake_watch(vec![]);
        let resps = serve_once_at(
            &[
                json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                       "params":{"name":"watch_arm","arguments":{"text":"OK"}}})
                .to_string(),
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"watch_disarm","arguments":{"watch_id":99}}}"#.to_string(),
                r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"watch_disarm","arguments":{"watch_id":1}}}"#.to_string(),
            ]
            .join("\n"),
            addr,
        );
        let text99 = resps[1]["result"]["content"][0]["text"].as_str().unwrap();
        let text1 = resps[2]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text99.contains("\"disarmed\":false"), "stale id: {text99}");
        assert!(text1.contains("\"disarmed\":true"), "live id: {text1}");
        // EOF cleanup also ran — total forwarded disarms must be exactly
        // 1 (the known watcher); the stale id never reached the fake.
        for _ in 0..40 {
            if disarms.load(std::sync::atomic::Ordering::SeqCst) >= 1 {
                break;
            }
            thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(disarms.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
