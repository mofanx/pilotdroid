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

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use pilotdroid_adb::AdbTransport;
use pilotdroid_agent::{MediaEvent, ScrollDir, Session};
use pilotdroid_proto::payload::*;
use serde_json::{json, Map, Value};

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
pub fn run<T: AdbTransport, R: std::io::BufRead, W: Write>(
    inp: R,
    out: &mut W,
    mut connect: impl FnMut() -> Result<Session<T>>,
) -> Result<()> {
    let mut session: Option<Session<T>> = None;
    // Element snapshot backing tap_index/set_text_index — refreshed by
    // every list_elements call; indexes are meaningless across snapshots.
    // (elements, layout signature, focused pkg at snapshot time).
    let mut snapshot: Option<(Vec<pilotdroid_agent::ElemInfo>, u64, Option<String>)> = None;
    for line in inp.lines() {
        let line = line?;
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
                let (v, dead) = call_tool(s, id, &name, args, &mut snapshot);
                // Dead control conn (e.g. replaced by a newer conn —
                // ADR-007 last-connector-wins): drop it so the NEXT
                // tools/call lazily reconnects and re-claims the agent,
                // instead of this serve process erroring every call on a
                // socket that can never recover.
                if dead {
                    eprintln!(
                        "pilotdroid-serve: control conn lost — likely replaced \
                         by another client (last-connector-wins, ADR-007); \
                         will reconnect on next call"
                    );
                    session = None;
                }
                write_resp(out, &v)?;
                continue;
            }
            _ => err(id, -32601, &format!("method not found: {method}")),
        };
        write_resp(out, &resp)?;
    }
    Ok(())
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
                    let ack = s.watch_arm(
                        sel,
                        &str_arg(&args, "action").unwrap_or_else(|| "click".into()),
                        num_arg(&args, "timeout_ms").map(|v| v as u64),
                    )?;
                    Ok(json!(ack))
                }
                "watch_disarm" => {
                    let id =
                        num_arg(&args, "watch_id").context("watch_disarm needs watch_id")? as u32;
                    Ok(json!({"disarmed": s.watch_disarm(id)?}))
                }
                "watch_events" => match s.watch_recv() {
                    Ok(MediaEvent::WatchHit(h)) => Ok(json!({"type": "watch_hit", "hit": h})),
                    Ok(MediaEvent::Frame(f)) => Ok(json!({
                        "type": "frame", "w": f.w, "h": f.h,
                        "dropped": f.dropped,
                    })),
                    // No event yet is data, not failure — report it in the
                    // union so an armed-but-quiet watcher doesn't surface
                    // isError to the MCP client (real errors still do).
                    Err(pilotdroid_agent::AgentError::Timeout) => Ok(json!({"type": "timeout"})),
                    Err(e) => Err(e.into()),
                },
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
        {"name":"watch_arm","description":"async watcher (CAP_WATCH_ASYNC, §4.9): returns {armed,watch_id} immediately; hits arrive on the media channel — read via watch_events. Control ops stay unblocked while armed","inputSchema":json!({
            "type":"object","allOf":[sel.clone()],"properties":{"action":prop("string","default click"),"timeout_ms":prop("integer","auto-disarm deadline, optional")}})},
        {"name":"watch_disarm","description":"cancel an armed watcher by watch_id → {disarmed}; a fired (one-shot) watcher is already removed, so disarming it returns {disarmed:false} — not an error","inputSchema":schema(
            &[("watch_id",prop("integer","from watch_arm"))],&["watch_id"])},
        {"name":"watch_events","description":"read next media-channel event → {type:'watch_hit',hit:{watch_id,matched,waited_ms,action_ok,node}} or {type:'frame',...} when a stream is also active; no event within the read deadline → {type:'timeout'} (normal result, not an error)","inputSchema":schema(&[],&[])},
        {"name":"wait_for","description":"selector + timeout_ms → device-side poll (BATCH wait_until)","inputSchema":json!({
            "type":"object","allOf":[sel.clone()],"properties":{"timeout_ms":prop("integer","default 5000")}})},
        {"name":"dump","description":"UI hierarchy XML","inputSchema":schema(&[],&[])},
        {"name":"list_elements","description":"flattened indexed element list [{i,text,desc,class,bounds,clickable,editable,scrollable}] — agent observation surface; interactive_only (default true) keeps clickable/editable/scrollable nodes","inputSchema":schema(
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

    /// Minimal control-socket fake: HELLO + INJECT + QUERY.
    fn spawn_fake() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
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
                            encode(&DumpAck {
                                xml: concat!(
                                    r#"<?xml version='1.0' encoding='UTF-8' standalone='yes' ?>"#,
                                    r#"<hierarchy><node index="0" text="设置" resource-id="com.x:id/t" "#,
                                    r#"class="android.widget.TextView" package="com.x" content-desc="" "#,
                                    r#"clickable="true" bounds="[10,20][110,60]"/>"#,
                                    r#"<node index="1" text="" resource-id="com.x:id/e" "#,
                                    r#"class="android.widget.EditText" package="com.x" content-desc="输入框" "#,
                                    r#"clickable="true" bounds="[10,80][300,140]"/>"#,
                                    r#"<node index="2" text="标题" resource-id="" "#,
                                    r#"class="android.widget.TextView" package="com.x" content-desc="" "#,
                                    r#"clickable="false" bounds="[10,150][200,190]"/></hierarchy>"#,
                                )
                                .into(),
                            })
                            .unwrap(),
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
        run(BufReader::new(lines.as_bytes()), &mut out, move || {
            Device::with_transport(adb.take().unwrap(), "fake0")
                .session_opts(Some(addr.port()), "pilotdroid_agent")
                .map_err(Into::into)
        })
        .unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
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
        // — ADR-007). Call 1 hits EOF mid-op → tool error; call 2 lazily
        // reconnects and the op succeeds — serve survives being replaced.
        let addr = spawn_fake_kill_first();
        let mut out = Vec::new();
        run(
            BufReader::new(
                [
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"tap","arguments":{"x":1,"y":2}}}"#,
                    r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"tap","arguments":{"x":3,"y":4}}}"#,
                ]
                .join("\n")
                .as_bytes(),
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
        let resps: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(resps.len(), 2);
        assert_eq!(resps[0]["result"]["isError"], true);
        assert_eq!(resps[1]["result"]["isError"], false);
        let text = resps[1]["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("\"ok\":true") || text.contains("ok"),
            "got {text}"
        );
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
}
