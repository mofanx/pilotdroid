//! High-level facade (ADR-007): `Device` → `Session` — one persistent
//! control connection, tool-shaped ops, and RAII cleanup of forwards and
//! the media stream. This is the API `pilotdroid serve` and Rust
//! consumers share; nothing here adds wire surface — every op maps onto
//! existing opcodes.

use std::path::Path;
use std::time::Duration;

use pilotdroid_adb::{AdbTransport, ShellOutput};
use pilotdroid_proto::payload::*;

use crate::{AgentClient, AgentError, AgentManager, MediaClient, Result, AGENT_SOCKET};

/// MotionEvent action codes (same provenance as client.rs).
const ACTION_DOWN: u8 = 0;
const ACTION_UP: u8 = 1;

/// A connected device handle — owns the adb transport; `session()`
/// moves it into a live session (watchdog connect + HELLO).
pub struct Device<T: AdbTransport> {
    adb: T,
    serial: String,
}

impl<T: AdbTransport> Device<T> {
    /// Wrap an already-constructed transport (production: AdbClient;
    /// tests: any stub implementing AdbTransport).
    pub fn with_transport(adb: T, serial: impl Into<String>) -> Self {
        Self {
            adb,
            serial: serial.into(),
        }
    }

    /// Watchdog connect on the default control socket, auto port.
    pub fn session(self) -> Result<Session<T>> {
        self.session_opts(None, AGENT_SOCKET)
    }

    /// Watchdog connect with an explicit local forward port / socket —
    /// tests pin the port to reach a loopback fake agent.
    pub fn session_opts(self, port: Option<u16>, socket: &str) -> Result<Session<T>> {
        let Device { mut adb, serial } = self;
        let (client, port, hello) =
            AgentManager::new(&mut adb).connect_recover(&serial, port, socket, None)?;
        Ok(Session {
            adb,
            serial,
            port,
            client,
            hello,
            media: None,
            media_port: None,
            media_port_hint: None,
            media_socket: None,
            streaming: false,
        })
    }
}

/// Scroll direction (parity with u2shim `_Scroll`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollDir {
    /// Content moves up (swipe up inside bounds).
    Forward,
    /// Content moves down (swipe down).
    Backward,
    /// Repeat Forward until the UI stops changing (dump-hash stable).
    ToEnd,
    /// Repeat Backward until stable.
    ToBeginning,
}

/// Live agent session: persistent control conn + optional media stream.
/// `Drop` performs best-effort cleanup: STREAM stop, media forward,
/// control forward — the control socket close is itself the §4.8
/// session-binding implicit stop on the agent side.
pub struct Session<T: AdbTransport> {
    adb: T,
    serial: String,
    port: u16,
    client: AgentClient,
    media: Option<MediaClient>,
    media_port: Option<u16>,
    /// Test hook / user pin for the media forward port.
    media_port_hint: Option<u16>,
    /// Socket name from the start ack (attach must wait for start, §4.8).
    media_socket: Option<String>,
    streaming: bool,
    /// HELLO_ACK retained so callers can expose agent build/caps.
    hello: pilotdroid_proto::payload::HelloAck,
}

impl<T: AdbTransport> Session<T> {
    /// Agent build version from HELLO (`ver` field); `None` on pre-M2 agents.
    pub fn agent_ver(&self) -> Option<&str> {
        self.hello.ver.as_deref()
    }

    /// Capability bitmap from HELLO.
    pub fn caps(&self) -> u32 {
        self.hello.caps
    }

    pub fn tap(&mut self, x: f32, y: f32) -> Result<InjectAck> {
        self.client.tap(x, y, true)
    }

    pub fn press(&mut self, keycode: u32, meta: Option<u32>) -> Result<InjectAck> {
        self.client.press(keycode, meta)
    }

    pub fn swipe(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, ms: u64) -> Result<InjectAck> {
        self.client.swipe(x1, y1, x2, y2, ms)
    }

    /// u2-parity long_click: DOWN at t=0, UP at `ms` — same Tap kind,
    /// wait_finish=false (compat/u2shim.py long_click semantics).
    pub fn long_click(&mut self, x: f32, y: f32, ms: u64) -> Result<InjectAck> {
        self.client.inject(InjectReq {
            kind: InjectKind::Tap,
            points: vec![
                MotionPoint {
                    action: ACTION_DOWN,
                    x,
                    y,
                    t_ms: 0,
                },
                MotionPoint {
                    action: ACTION_UP,
                    x,
                    y,
                    t_ms: ms,
                },
            ],
            wait_finish: false,
            keycode: None,
            meta: None,
            backend: None,
        })
    }

    pub fn set_text(&mut self, text: &str, clear: bool) -> Result<()> {
        self.client.set_text(text, clear)
    }

    pub fn dump(&mut self) -> Result<String> {
        self.client.dump()
    }

    /// Snapshot the UI tree as a flat, index-addressable element list —
    /// the agent-observation surface (`list_elements` tool). `i` is the
    /// node's position in the returned (possibly filtered) list and is
    /// only meaningful against the snapshot it came from; callers
    /// re-list before indexing.
    ///
    /// `interactive_only` keeps clickable/editable/scrollable nodes —
    /// the ~4KB response an agent caller actually wants; pass `false`
    /// for the raw full-tree dump order.
    pub fn list_elements(&mut self, interactive_only: bool) -> Result<ElemSnapshot> {
        let xml = self.dump()?;
        let nodes = parse_nodes(&xml);
        let layout_sig = nodes_signature(&nodes);
        let mut elements = nodes;
        if interactive_only {
            elements.retain(|e| e.clickable || e.editable || e.scrollable);
        }
        for (i, e) in elements.iter_mut().enumerate() {
            e.i = i;
        }
        Ok(ElemSnapshot {
            elements,
            layout_sig,
        })
    }

    /// Recompute the layout signature of the *current* screen — the
    /// staleness probe backing `tap_index`'s snapshot guard. Cheap
    /// signal: fnv over every node's (pkg,class,bounds); text changes
    /// alone do not invalidate a snapshot.
    pub fn layout_signature(&mut self) -> Result<u64> {
        let xml = self.dump()?;
        Ok(nodes_signature(&parse_nodes(&xml)))
    }

    pub fn query(&mut self, selector: Selector) -> Result<QueryAck> {
        self.client.query(QueryReq {
            selector,
            attrs: None,
        })
    }

    pub fn exists(&mut self, selector: Selector) -> Result<bool> {
        self.client.exists(selector)
    }

    /// `wait_until` wrapped in a single-op BATCH (§4.3). Returns
    /// (found, node) — TIMEOUT maps to `Ok((false, None))`.
    pub fn wait_for(
        &mut self,
        selector: Selector,
        timeout_ms: u64,
    ) -> Result<(bool, Option<NodeInfo>)> {
        let ack = self.client.batch(BatchReq {
            ops: vec![BatchOp::WaitUntil(WaitUntilReq {
                selector,
                timeout_ms,
                poll_ms: None,
            })],
            fail_policy: FailPolicy::Stop,
            deadline_ms: None,
            poll_ms: None,
        })?;
        let r = ack.results.into_iter().next().unwrap_or(OpResult {
            index: 0,
            code: "EMPTY".into(),
            message: "no results".into(),
            cost_ns: 0,
            data: None,
        });
        match r.code.as_str() {
            "OK" => {
                let node = r.data.and_then(|d| match d {
                    rmpv::Value::Map(m) => m
                        .iter()
                        .find(|(k, _)| k.as_str() == Some("node"))
                        .and_then(|(_, v)| rmpv::ext::from_value::<NodeInfo>(v.clone()).ok()),
                    _ => None,
                });
                Ok((true, node))
            }
            "TIMEOUT" => Ok((false, None)),
            other => Err(AgentError::Remote {
                code: other.into(),
                message: r.message,
            }),
        }
    }

    pub fn batch(&mut self, ops: Vec<BatchOp>) -> Result<BatchAck> {
        self.client.batch_ops(ops)
    }

    /// SCREENSHOT, written to `path` when given; returns (format, bytes).
    pub fn screenshot(
        &mut self,
        path: Option<&Path>,
        quality: u8,
        scale: Option<f32>,
    ) -> Result<(String, Vec<u8>)> {
        let (fmt, bytes) = self.client.screenshot("jpeg", quality, scale)?;
        if let Some(p) = path {
            std::fs::write(p, &bytes).map_err(AgentError::Io)?;
        }
        Ok((fmt, bytes))
    }

    /// Raw device shell — backs app_*/scroll-aux tools. Exit-code-bearing
    /// callers check `ShellOutput.exit_code`.
    pub fn shell(&mut self, command: &str) -> Result<ShellOutput> {
        Ok(self.adb.shell(&self.serial, command)?)
    }

    /// `am force-stop` (u2 app_stop parity).
    pub fn app_stop(&mut self, pkg: &str) -> Result<()> {
        let out = self.shell(&format!("am force-stop {pkg}"))?;
        if out.exit_code == Some(0) {
            Ok(())
        } else {
            Err(AgentError::Shell {
                code: out.exit_code,
                stderr: String::from_utf8_lossy(&out.stderr).into(),
            })
        }
    }

    /// `monkey -p pkg -c LAUNCHER 1` (u2 app_start parity; `activity`
    /// switches to `am start -n pkg/activity`).
    pub fn app_start(&mut self, pkg: &str, activity: Option<&str>) -> Result<()> {
        let cmd = match activity {
            Some(act) => format!("am start -n {pkg}/{act}"),
            None => format!("monkey -p {pkg} -c android.intent.category.LAUNCHER 1"),
        };
        let out = self.shell(&cmd)?;
        if out.exit_code == Some(0) {
            Ok(())
        } else {
            Err(AgentError::Shell {
                code: out.exit_code,
                stderr: String::from_utf8_lossy(&out.stderr).into(),
            })
        }
    }

    /// u2 app_current parity: parse `mCurrentFocus=... pkg/activity`
    /// from `dumpsys window`; falls back to the first `pkg/act` pair.
    /// Returns (package, activity) or None when unfocused.
    pub fn app_current(&mut self) -> Result<Option<(String, String)>> {
        let out = self.shell("dumpsys window")?;
        Ok(parse_current_focus(&String::from_utf8_lossy(&out.stdout)))
    }

    /// u2-parity scroll: resolve the (scrollable) node, then swipe inside
    /// its bounds — 80%→20% of height for Forward, mirrored for Backward.
    /// To*/ToBeginning repeat until the dump stops changing (hash-stable)
    /// or 10 swipes, 400ms settle between swipes (u2shim `_to` semantics).
    pub fn scroll(&mut self, dir: ScrollDir, selector: Selector) -> Result<bool> {
        match dir {
            ScrollDir::Forward | ScrollDir::Backward => {
                self.scroll_once(dir == ScrollDir::Forward, selector)
            }
            _ => {
                let fwd = dir == ScrollDir::ToEnd;
                let mut prev = String::new();
                for _ in 0..10 {
                    let cur = format!("{:x}", fnv1a(self.dump()?.as_bytes()));
                    if cur == prev {
                        return Ok(true);
                    }
                    prev = cur;
                    self.scroll_once(fwd, selector.clone())?;
                    std::thread::sleep(Duration::from_millis(400));
                }
                Ok(false)
            }
        }
    }

    fn scroll_once(&mut self, forward: bool, selector: Selector) -> Result<bool> {
        let ack = self.query(selector)?;
        let b = ack.node.ok_or(AgentError::Remote {
            code: "NO_NODE".into(),
            message: "scroll selector found no node".into(),
        })?;
        let [l, t, r, bot] = b.bounds;
        let cx = (l + r) as f32 / 2.0;
        let h = (bot - t) as f32;
        let (y_from, y_to) = if forward {
            (t as f32 + h * 0.8, t as f32 + h * 0.2)
        } else {
            (t as f32 + h * 0.2, t as f32 + h * 0.8)
        };
        self.client.swipe(cx, y_from, cx, y_to, 200)?;
        Ok(true)
    }

    // ---- media stream (docs/02 §4.8) ---------------------------------------

    /// STREAM_CTRL start — records the media socket name but does NOT
    /// attach: §4.8 order is start-ack → media connect, and the agent
    /// drops pre-start pending conns, so attach must be lazy
    /// (`stream_read` performs it).
    pub fn stream_start(
        &mut self,
        fps: Option<u8>,
        scale: Option<f32>,
        quality: Option<u8>,
    ) -> Result<StreamStartAck> {
        let ack = self.client.stream_start(StreamCtrlReq {
            op: StreamOp::Start,
            fps,
            scale,
            quality,
            format: None,
        })?;
        self.streaming = true;
        self.media_socket = Some(ack.socket.clone());
        Ok(ack)
    }

    /// Read up to `n` frames (default 1). First call attaches the media
    /// socket lazily — see `stream_start`. `to_dir` writes
    /// `frame_{seq}.jpg` per frame (file-bypass for large streams; stdio
    /// callers should prefer this over base64 floods).
    pub fn stream_read(&mut self, n: u32, to_dir: Option<&Path>) -> Result<Vec<MediaFrame>> {
        if !self.streaming {
            return Err(AgentError::Remote {
                code: "NOT_STREAMING".into(),
                message: "stream_start first".into(),
            });
        }
        self.attach_media()?;
        if let Some(dir) = to_dir {
            std::fs::create_dir_all(dir).map_err(AgentError::Io)?;
        }
        let media = self.media.as_mut().unwrap();
        let mut out = Vec::new();
        for _ in 0..n {
            match media.recv() {
                Ok(f) => {
                    if let Some(dir) = to_dir {
                        let seq = media.last_seq().unwrap_or(0);
                        std::fs::write(dir.join(format!("frame_{seq:05}.jpg")), &f.data)
                            .map_err(AgentError::Io)?;
                    }
                    out.push(f);
                    if media.is_ended() {
                        break;
                    }
                }
                // Socket close or a stall ends the read — return what we
                // got; empty+closed surfaces the error to the caller.
                Err(e @ (AgentError::Timeout | AgentError::Io(_))) => {
                    if out.is_empty() {
                        return Err(e);
                    }
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// STREAM_CTRL stop + media teardown. Safe to call when not streaming.
    pub fn stream_stop(&mut self) -> Result<StreamStopAck> {
        let ack = self.client.stream_stop();
        self.teardown_media();
        self.streaming = false;
        ack
    }

    /// Lazily attach the media socket. MUST run after start-ack — the
    /// agent discards pre-start pending conns (§4.8, Agent.java
    /// Streamer.start drops pendSock). Do not "optimize" into eager
    /// attach inside `stream_start`.
    fn attach_media(&mut self) -> Result<()> {
        if self.media.is_some() {
            return Ok(());
        }
        let socket = self.media_socket.clone().unwrap_or_default();
        let (m, port) = AgentManager::new(&mut self.adb).connect_media(
            &self.serial,
            self.media_port_hint,
            &socket,
        )?;
        self.media = Some(m);
        self.media_port = Some(port);
        Ok(())
    }

    /// Test/ops hook: pin the local media forward port.
    pub fn set_media_port(&mut self, port: u16) {
        self.media_port_hint = Some(port);
    }

    pub fn is_streaming(&self) -> bool {
        self.streaming
    }

    fn teardown_media(&mut self) {
        self.media = None;
        if let Some(p) = self.media_port.take() {
            let _ = AgentManager::new(&mut self.adb).disconnect(&self.serial, p);
        }
    }
}

impl<T: AdbTransport> Drop for Session<T> {
    /// Best-effort cleanup: an active stream is stopped via STREAM_CTRL
    /// (the control socket close also implicit-stops it agent-side per
    /// §4.8 session binding), then both forwards are removed — the host
    /// must not leak `adb forward` rules across sessions.
    fn drop(&mut self) {
        if self.streaming {
            let _ = self.client.stream_stop();
        }
        self.teardown_media();
        let _ = AgentManager::new(&mut self.adb).disconnect(&self.serial, self.port);
    }
}

/// `mCurrentFocus=Window{...} pkg/activity}` — string-parse without a
/// regex dep; fallback: first bare `pkg/activity` token.
fn parse_current_focus(dumpsys: &str) -> Option<(String, String)> {
    fn split_pair(tok: &str) -> Option<(String, String)> {
        let tok = tok.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '.' && c != '/' && c != '$' && c != '_'
        });
        let (pkg, act) = tok.split_once('/')?;
        if pkg.is_empty() || act.is_empty() || !pkg.contains('.') {
            return None;
        }
        Some((pkg.to_string(), act.to_string()))
    }
    for line in dumpsys.lines() {
        if let Some(pos) = line.find("mCurrentFocus") {
            for tok in line[pos..].split_whitespace() {
                if let Some(pair) = split_pair(tok) {
                    return Some(pair);
                }
            }
        }
    }
    None
}

/// FNV-1a for dump-hash stability (scroll_to stop condition).
fn fnv1a(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ---- element observation surface (M7) --------------------------------------

/// One flattened UI node from a DUMP snapshot — index-addressable for
/// `tap_index`/`set_text_index`. Empty text/desc/class fields are
/// skipped on the wire (~2KB target for a typical Settings page).
/// Result of [`Session::list_elements`] — the filtered element list plus
/// a layout signature for staleness checks.
pub struct ElemSnapshot {
    pub elements: Vec<ElemInfo>,
    /// fnv over all nodes' (pkg,class,bounds) — compared against
    /// [`Session::layout_signature`] before index ops.
    pub layout_sig: u64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ElemInfo {
    /// Position in the returned list — index handle for tap/set_text.
    pub i: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub desc: String,
    /// Simple class name (`android.widget.EditText` → `EditText`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub class: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pkg: String,
    /// `[left, top, right, bottom]` device px — center is the tap point.
    pub bounds: [i32; 4],
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub clickable: bool,
    /// `android.widget.EditText` (or subclass tail `EditText`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub editable: bool,
    /// Node can scroll — interactive for the `scroll` tool too.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub scrollable: bool,
}

/// Parse uiautomator dump XML into a flat element list. Attribute
/// extraction is deliberately dependency-free: `<node .../>` tags are
/// scanned for `key="value"` pairs; Android's serializer guarantees
/// quoted attributes. Index `i` follows document order.
fn parse_nodes(xml: &str) -> Vec<ElemInfo> {
    fn attr<'a>(tag: &'a str, key: &str) -> Option<&'a str> {
        let pat = format!("{key}=\"");
        let v = &tag[tag.find(&pat)? + pat.len()..];
        Some(&v[..v.find('"')?])
    }
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(off) = xml[pos..].find("<node ") {
        let tag_start = pos + off;
        let tag_end = match xml[tag_start..].find('>') {
            Some(e) => tag_start + e,
            None => break,
        };
        let tag = &xml[tag_start..tag_end];
        let g = |k: &str| attr(tag, k).unwrap_or("");
        let cls = g("class");
        let simple = cls.rsplit('.').next().unwrap_or(cls);
        out.push(ElemInfo {
            i: out.len(),
            text: g("text").into(),
            desc: g("content-desc").into(),
            class: simple.into(),
            pkg: g("package").into(),
            bounds: parse_bounds(g("bounds")).unwrap_or([0, 0, 0, 0]),
            clickable: g("clickable") == "true",
            editable: simple == "EditText",
            scrollable: g("scrollable") == "true",
        });
        pos = tag_end;
    }
    out
}

/// Layout signature for staleness checks: fnv-1a over every node's
/// (pkg, class, bounds) — deliberately ignores text/desc so a ticking
/// clock or typed text does not invalidate a position snapshot.
fn nodes_signature(nodes: &[ElemInfo]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut mix = |b: &[u8]| {
        for &x in b {
            h = (h ^ x as u64).wrapping_mul(0x100000001b3);
        }
    };
    for n in nodes {
        mix(n.pkg.as_bytes());
        mix(n.class.as_bytes());
        for v in n.bounds {
            mix(&v.to_le_bytes());
        }
        mix(&[0xff]);
    }
    h
}

/// `[l,t][r,b]` → `[l,t,r,b]`.
fn parse_bounds(s: &str) -> Option<[i32; 4]> {
    let s = s.strip_prefix('[')?.strip_suffix(']')?;
    let (l, r) = s.split_once("][")?;
    let (x1, y1) = l.split_once(',')?;
    let (x2, y2) = r.split_once(',')?;
    Some([
        x1.parse().ok()?,
        y1.parse().ok()?,
        x2.parse().ok()?,
        y2.parse().ok()?,
    ])
}
