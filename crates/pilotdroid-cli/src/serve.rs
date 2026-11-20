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
use pilotdroid_agent::{ScrollDir, Session};
use pilotdroid_proto::payload::*;
use serde_json::{json, Map, Value};

/// MCP protocol revision we speak (echoed back to the client).
pub const MCP_VERSION: &str = "2024-11-05";

/// Serve loop: read JSON-RPC requests from `inp`, write responses to
/// `out`. `connect` is invoked lazily on the first `tools/call` — the
/// handshake and tools/list work without a device session.
pub fn run<T: AdbTransport, R: std::io::BufRead, W: Write>(
    inp: R,
    out: &mut W,
    mut connect: impl FnMut() -> Result<Session<T>>,
) -> Result<()> {
    let mut session: Option<Session<T>> = None;
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
                let pv = req
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(MCP_VERSION);
                ok(
                    id,
                    json!({
                        "protocolVersion": pv,
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {"name": "pilotdroid", "version": env!("CARGO_PKG_VERSION")},
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
                write_resp(out, &call_tool(s, id, &name, args)?)?;
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
        index: args.get("index").and_then(Value::as_u64).map(|v| v as u32),
    }
}

fn str_arg(args: &Value, k: &str) -> Option<String> {
    args.get(k).and_then(Value::as_str).map(String::from)
}

fn num_arg(args: &Value, k: &str) -> Option<f64> {
    args.get(k).and_then(Value::as_f64)
}

fn call_tool<T: AdbTransport>(
    s: &mut Session<T>,
    id: Value,
    name: &str,
    args: Value,
) -> Result<Value> {
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
        Ok(v) => Ok(tool_ok(id, v)),
        Err(e) => Ok(tool_err(id, e)),
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
    }});
    json!([
        {"name":"tap","description":"tap (x,y) — INJECT MotionEvent down/up","inputSchema":xy},
        {"name":"long_click","description":"long press (x,y,duration_ms)","inputSchema":schema(
            &[("x",prop("number","x")),("y",prop("number","y")),("duration_ms",prop("integer","hold ms, default 500"))],
            &["x","y"])},
        {"name":"swipe","description":"swipe (x1,y1)→(x2,y2) over duration_ms","inputSchema":schema(
            &[("x1",prop("number","")),("y1",prop("number","")),("x2",prop("number","")),("y2",prop("number","")),("duration_ms",prop("integer","default 300"))],
            &["x1","y1","x2","y2"])},
        {"name":"press","description":"inject KeyEvent by android keycode (back=4, home=3, recent=187)","inputSchema":schema(
            &[("keycode",prop("integer","KEYCODE_*")),("meta",prop("integer","metaState"))],&["keycode"])},
        {"name":"set_text","description":"ACTION_SET_TEXT on focused node","inputSchema":schema(
            &[("text",prop("string","")),("clear",prop("boolean","clear first, default true"))],&["text"])},
        {"name":"exists","description":"selector exists? — device-side UiAutomation tree","inputSchema":sel.clone()},
        {"name":"query","description":"selector → {found,node{text,id,desc,bounds,class,...}}","inputSchema":sel.clone()},
        {"name":"wait_for","description":"selector + timeout_ms → device-side poll (BATCH wait_until)","inputSchema":json!({
            "type":"object","allOf":[sel.clone()],"properties":{"timeout_ms":prop("integer","default 5000")}})},
        {"name":"dump","description":"UI hierarchy XML","inputSchema":schema(&[],&[])},
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
                                ver: None,
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

    fn serve_once(lines: &str) -> Vec<Value> {
        let addr = spawn_fake();
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
