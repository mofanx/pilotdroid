//! High-level facade (ADR-007): `Device` → `Session` — one persistent
//! control connection, tool-shaped ops, and RAII cleanup of forwards and
//! the media stream. This is the API `pilotdroid serve` and Rust
//! consumers share; nothing here adds wire surface — every op maps onto
//! existing opcodes.

use std::path::Path;
use std::time::Duration;

use pilotdroid_adb::{AdbTransport, ShellOutput};
use pilotdroid_proto::payload::*;

use crate::{
    AgentClient, AgentError, AgentManager, MediaClient, MediaEvent, Result, AGENT_SOCKET,
    MEDIA_SOCKET,
};

/// MotionEvent action codes (same provenance as client.rs).
const ACTION_DOWN: u8 = 0;
const ACTION_UP: u8 = 1;
const ACTION_MOVE: u8 = 2;

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
            socket: socket.to_string(),
            client,
            hello,
            media: None,
            media_port: None,
            media_port_hint: None,
            generation: 0,
            media_socket: None,
            streaming: false,
            stashed_events: std::collections::VecDeque::new(),
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
    /// Control localabstract socket name — retained so mid-session
    /// crash recovery can re-forward the same socket.
    socket: String,
    client: AgentClient,
    media: Option<MediaClient>,
    media_port: Option<u16>,
    /// Test hook / user pin for the media forward port.
    media_port_hint: Option<u16>,
    /// Socket name from the start ack (attach must wait for start, §4.8).
    media_socket: Option<String>,
    streaming: bool,
    /// Recovery epoch — bumped every time `recover()` reconnects.
    /// Agent-allocated ids (watch_id) and session-bound state are only
    /// valid within one epoch: a restarted agent re-allocates ids from
    /// scratch, so hosts must treat ids from earlier epochs as dead
    /// (a stale id may collide with a new live watcher).
    generation: u64,
    /// HELLO_ACK retained so callers can expose agent build/caps.
    hello: pilotdroid_proto::payload::HelloAck,
    /// Non-frame media events (watch_hit/toast ui_events) demuxed out
    /// of an active frame stream by `stream_read` — drained by
    /// `watch_recv` so hits during streaming stay pullable after stop
    /// instead of dying with the shared ch=1 conn.
    stashed_events: std::collections::VecDeque<MediaEvent>,
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

    /// Recovery epoch: `0` on the initial session, incremented on every
    /// transparent `recover()`. Callers tracking agent-allocated ids
    /// (watch ids, armed watcher flags) must discard them when the
    /// epoch changes — the fresh agent never heard of them.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Run a control-channel op, transparently recovering a dead agent
    /// first (atx-agent semantics — docs/03 §5 watchdog). A dead socket
    /// (`Io` on read EOF/reset, `Protocol(Io)` on write) is definitive:
    /// the daemon is gone, so the recovery ladder runs and the op is
    /// replayed once against the fresh session.
    ///
    /// Replay caveat: the original request may have reached the device
    /// before the conn died — replaying a side-effecting op can double
    /// it (tap twice). `Timeout` is NOT recovered (can't distinguish a
    /// hung agent from a slow op; replay would be blind).
    ///
    /// Watcher/stream state is session-bound on the agent: recovery
    /// loses armed watchers (`watch_id`s are re-allocated from scratch,
    /// so a stale id may collide with a new live one) and any active
    /// stream. Media-side callers surface the error after healing the
    /// control conn for the next op.
    fn with_client<R>(&mut self, f: impl Fn(&mut AgentClient) -> Result<R>) -> Result<R> {
        match f(&mut self.client) {
            Err(e) if is_dead_transport(&e) => {
                self.recover()?;
                f(&mut self.client)
            }
            r => r,
        }
    }

    /// Tear down dead session state and re-run the watchdog ladder:
    /// media conn/forwards die with the agent; the stale control forward
    /// is removed; `connect_recover` restarts (or reinstalls) the daemon
    /// and re-handshakes. `port`/`hello` are refreshed — the media
    /// `port+1` strategy keeps working afterwards.
    fn recover(&mut self) -> Result<()> {
        self.teardown_media();
        self.streaming = false;
        let socket = self.socket.clone();
        let _ = AgentManager::new(&mut self.adb).disconnect(&self.serial, self.port);
        // Re-forward the SAME port: keeps the media `port+1` heuristic
        // and any external forward bookkeeping valid, and lets test
        // transports that pin a fake agent on one port keep working.
        let (client, port, hello) = AgentManager::new(&mut self.adb).connect_recover(
            &self.serial,
            Some(self.port),
            &socket,
            None,
        )?;
        self.client = client;
        self.port = port;
        self.hello = hello;
        self.generation += 1;
        Ok(())
    }

    pub fn tap(&mut self, x: f32, y: f32) -> Result<InjectAck> {
        self.with_client(|c| c.tap(x, y, true))
    }

    pub fn press(&mut self, keycode: u32, meta: Option<u32>) -> Result<InjectAck> {
        self.with_client(|c| c.press(keycode, meta))
    }

    pub fn swipe(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, ms: u64) -> Result<InjectAck> {
        self.with_client(|c| c.swipe(x1, y1, x2, y2, ms))
    }

    /// u2-parity long_click: DOWN at t=0, UP at `ms` — same Tap kind,
    /// wait_finish=false (compat/u2shim.py long_click semantics).
    pub fn long_click(&mut self, x: f32, y: f32, ms: u64) -> Result<InjectAck> {
        self.with_client(|c| {
            c.inject(InjectReq {
                kind: InjectKind::Tap,
                points: vec![
                    MotionPoint {
                        action: ACTION_DOWN,
                        x,
                        y,
                        t_ms: 0,
                        pid: None,
                    },
                    MotionPoint {
                        action: ACTION_UP,
                        x,
                        y,
                        t_ms: ms,
                        pid: None,
                    },
                ],
                wait_finish: false,
                keycode: None,
                meta: None,
                backend: None,
            })
        })
    }

    pub fn set_text(&mut self, text: &str, clear: bool) -> Result<()> {
        self.with_client(|c| c.set_text(text, clear))
    }

    /// Two-pointer pinch (CAP_MULTI_PTR): both fingers start `r0` px from
    /// center and end `r1` px away, symmetric on the x axis. pinch_in =
    /// r0 > r1 (fingers converge), pinch_out = r0 < r1.
    pub fn pinch(
        &mut self,
        cx: f32,
        cy: f32,
        r0: f32,
        r1: f32,
        duration_ms: u64,
    ) -> Result<InjectAck> {
        let steps = ((duration_ms / 15) as usize).clamp(1, 40);
        let mut points = Vec::with_capacity(steps * 2 + 4);
        points.push(MotionPoint {
            action: ACTION_DOWN,
            x: cx - r0,
            y: cy,
            t_ms: 0,
            pid: Some(0),
        });
        points.push(MotionPoint {
            action: ACTION_DOWN,
            x: cx + r0,
            y: cy,
            t_ms: 10,
            pid: Some(1),
        });
        for i in 1..=steps {
            let t = i as f32 / (steps + 1) as f32;
            let d = r0 + (r1 - r0) * t;
            let t_ms = 10 + (duration_ms as f32 * t) as u64;
            points.push(MotionPoint {
                action: ACTION_MOVE,
                x: cx - d,
                y: cy,
                t_ms,
                pid: Some(0),
            });
            points.push(MotionPoint {
                action: ACTION_MOVE,
                x: cx + d,
                y: cy,
                t_ms,
                pid: Some(1),
            });
        }
        points.push(MotionPoint {
            action: ACTION_UP,
            x: cx + r1,
            y: cy,
            t_ms: 10 + duration_ms,
            pid: Some(1),
        });
        points.push(MotionPoint {
            action: ACTION_UP,
            x: cx - r1,
            y: cy,
            t_ms: 10 + duration_ms + 10,
            pid: Some(0),
        });
        self.with_client(|c| {
            c.inject(InjectReq {
                kind: InjectKind::Swipe,
                points: points.clone(),
                wait_finish: true,
                keycode: None,
                meta: None,
                backend: None,
            })
        })
    }

    /// WATCH_ARM — async watcher (§4.9, CAP_WATCH_ASYNC): returns
    /// immediately; the agent polls device-side and pushes `WATCH_HIT`
    /// on the media socket. `watch_recv()` reads hits. The control conn
    /// stays free — parallel ops while armed are the whole point.
    pub fn watch_arm(
        &mut self,
        selector: Selector,
        action: &str,
        timeout_ms: Option<u64>,
    ) -> Result<WatchArmAck> {
        self.with_client(|c| {
            c.watch_arm(WatchArmReq {
                selector: selector.clone(),
                action: Some(WatchAction {
                    r#type: action.to_string(),
                }),
                timeout_ms,
                poll_ms: None,
            })
        })
    }

    pub fn watch_disarm(&mut self, watch_id: u32) -> Result<bool> {
        Ok(self.with_client(|c| c.watch_disarm(watch_id))?.disarmed)
    }

    /// Attach the media socket for watch events (no stream needed — the
    /// agent holds it as the pre-start pending conn, §4.8).
    pub fn watch_attach(&mut self) -> Result<()> {
        self.attach_media()
    }

    /// Read the next media-channel event; a `MediaEvent::WatchHit` is the
    /// armed watcher's result. Blocks up to `timeout_ms` (default: the
    /// media read timeout set at connect).
    /// Pop a media event stashed while a frame stream was active —
    /// socket-free, so safe to call mid-stream (a `watch_recv` at that
    /// point would steal frames off the shared media conn).
    pub fn pop_stashed(&mut self) -> Option<MediaEvent> {
        self.stashed_events.pop_front()
    }

    pub fn watch_recv(&mut self, timeout_ms: Option<u64>) -> Result<MediaEvent> {
        if let Some(ev) = self.stashed_events.pop_front() {
            return Ok(ev);
        }
        self.attach_media()?;
        let m = self.media.as_mut().unwrap();
        if let Some(ms) = timeout_ms {
            m.set_read_timeout(Duration::from_millis(ms))?;
        }
        match m.recv_event() {
            Err(e) if is_dead_transport(&e) => {
                // Media conn dead ⇒ agent gone ⇒ the armed watcher is
                // lost (session-bound). Heal the control conn for the
                // next op, then surface the failure — callers re-arm.
                let _ = self.recover();
                Err(e)
            }
            r => r,
        }
    }

    /// APP_INFO — foreground pkg inside UiAutomation (~ms), no adb spawn.
    /// `activity` is None by design (CAP_APP_INFO).
    pub fn app_info(&mut self) -> Result<AppInfoAck> {
        self.with_client(|c| c.app_info())
    }

    /// WATCH — device-side selector→action loop (CAP_WATCH, docs/02 §4.9).
    pub fn watch(&mut self, selector: Selector, action: &str, timeout_ms: u64) -> Result<WatchAck> {
        self.with_client(|c| {
            c.watch(WatchReq {
                selector: selector.clone(),
                action: Some(WatchAction {
                    r#type: action.into(),
                }),
                timeout_ms: Some(timeout_ms),
                poll_ms: None,
            })
        })
    }

    pub fn dump(&mut self) -> Result<String> {
        self.with_client(|c| c.dump())
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
        self.with_client(|c| {
            c.query(QueryReq {
                selector: selector.clone(),
                attrs: None,
            })
        })
    }

    pub fn exists(&mut self, selector: Selector) -> Result<bool> {
        self.with_client(|c| c.exists(selector.clone()))
    }

    /// `wait_until` wrapped in a single-op BATCH (§4.3). Returns
    /// (found, node) — TIMEOUT maps to `Ok((false, None))`.
    pub fn wait_for(
        &mut self,
        selector: Selector,
        timeout_ms: u64,
    ) -> Result<(bool, Option<NodeInfo>)> {
        let ack = self.with_client(|c| {
            c.batch(BatchReq {
                ops: vec![BatchOp::WaitUntil(WaitUntilReq {
                    selector: selector.clone(),
                    timeout_ms,
                    poll_ms: None,
                })],
                fail_policy: FailPolicy::Stop,
                deadline_ms: None,
                poll_ms: None,
            })
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
        self.with_client(|c| c.batch_ops(ops.clone()))
    }

    /// SCREENSHOT, written to `path` when given; returns (format, bytes).
    pub fn screenshot(
        &mut self,
        path: Option<&Path>,
        quality: u8,
        scale: Option<f32>,
    ) -> Result<(String, Vec<u8>)> {
        let (fmt, bytes) = self.with_client(|c| c.screenshot("jpeg", quality, scale))?;
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
        self.with_client(|c| c.swipe(cx, y_from, cx, y_to, 200))?;
        Ok(true)
    }

    // ---- media stream (docs/02 §4.8) ---------------------------------------

    /// STREAM_CTRL start — records the media socket name but does NOT
    /// attach: §4.8 order is start-ack → media connect, and the agent
    /// drops pre-start pending conns, so attach must be lazy
    /// (`stream_read` performs it).
    ///
    /// An already-attached media conn (e.g. an armed push watcher's)
    /// is a pre-start pending conn from the agent's view — it is killed
    /// on start, so drop the stale handle here; watch hits buffer
    /// agent-side and stay pullable via watch_events post-stop (the
    /// documented demux-pause degrade).
    pub fn stream_start(
        &mut self,
        fps: Option<u8>,
        scale: Option<f32>,
        quality: Option<u8>,
    ) -> Result<StreamStartAck> {
        self.teardown_media();
        let ack = self.with_client(|c| {
            c.stream_start(StreamCtrlReq {
                op: StreamOp::Start,
                fps,
                scale,
                quality,
                format: None,
            })
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
        let mut out = Vec::new();
        while out.len() < n as usize {
            // Demux on the shared ch=1 conn: a watch_hit/toast ui_event
            // can land between stream frames — `media.recv()` would
            // error UnexpectedResponse on it. recv_event sorts them;
            // non-frame events are stashed so watch_events can pull
            // them after the stream stops (documented degrade).
            match self.media.as_mut().unwrap().recv_event() {
                Ok(MediaEvent::Frame(f)) => {
                    if let Some(dir) = to_dir {
                        let seq = self.media.as_mut().unwrap().last_seq().unwrap_or(0);
                        std::fs::write(dir.join(format!("frame_{seq:05}.jpg")), &f.data)
                            .map_err(AgentError::Io)?;
                    }
                    out.push(f);
                    if self.media.as_mut().unwrap().is_ended() {
                        break;
                    }
                }
                Ok(
                    ev
                    @ (MediaEvent::WatchHit(_) | MediaEvent::Toast(_) | MediaEvent::WatchDead(_)),
                ) => {
                    self.stashed_events.push_back(ev);
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
    ///
    /// Drains the conn before close: watch_hit/toast ui_events
    /// interleaved with stream frames that no `stream_read` reached
    /// would die with the socket — stash them so `watch_events` can
    /// still pull them after stop.
    pub fn stream_stop(&mut self) -> Result<StreamStopAck> {
        let ack = self.with_client(|c| c.stream_stop());
        if let Some(m) = self.media.as_mut() {
            let _ = m.set_read_timeout(Duration::from_millis(150));
            loop {
                match m.recv_event() {
                    Ok(
                        ev @ (MediaEvent::WatchHit(_)
                        | MediaEvent::Toast(_)
                        | MediaEvent::WatchDead(_)),
                    ) => {
                        self.stashed_events.push_back(ev);
                    }
                    Ok(MediaEvent::Frame(_)) => continue,
                    Err(_) => break,
                }
            }
        }
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
        // Watch attaches happen before any stream_start, when
        // `media_socket` is still unset — fall back to the default name.
        let socket = self
            .media_socket
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| MEDIA_SOCKET.to_string());
        // Port strategy (M12): prefer control_port+1 so a crashed host
        // leaves at most ONE predictable orphan forward instead of an
        // accumulating random set (setup step 6 lists it). Fall back to
        // a free port when the fixed one is taken.
        let (m, port) = match self.media_port_hint {
            Some(p) => {
                AgentManager::new(&mut self.adb).connect_media(&self.serial, Some(p), &socket)?
            }
            None => AgentManager::new(&mut self.adb)
                .connect_media(&self.serial, self.port.checked_add(1), &socket)
                .or_else(|_| {
                    AgentManager::new(&mut self.adb).connect_media(&self.serial, None, &socket)
                })?,
        };
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

/// Dead-socket predicate for crash recovery: `Io` covers read EOF/reset
/// and write failures surface as `Protocol(ProtocolError::Io)` — both
/// mean the forwarded conn is gone. `Timeout` is deliberately excluded
/// (hung vs dead is undecidable; replay could double-execute the op).
fn is_dead_transport(e: &AgentError) -> bool {
    matches!(
        e,
        AgentError::Io(_) | AgentError::Protocol(pilotdroid_proto::ProtocolError::Io(_))
    )
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
    /// Widget state flags straight from the dump attrs — emitted only
    /// when set (`checked`/`selected`/`focused`) so the response stays
    /// compact; `enabled` is emitted only when the node is DISABLED
    /// (`"enabled":false`) since enabled is the overwhelmingly common
    /// case. Together they make checkbox/switch/radio loops
    /// provable (blind-drive B4 gap) — `verify_after` state predicates
    /// read the same fields.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub checked: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub selected: bool,
    #[serde(skip_serializing_if = "is_enabled")]
    pub enabled: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub focused: bool,
}

fn is_enabled(b: &bool) -> bool {
    *b
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
            checked: g("checked") == "true",
            selected: g("selected") == "true",
            // dump omits `enabled` never — but treat absent as true so
            // hand-written test fixtures don't look disabled.
            enabled: g("enabled") != "false",
            focused: g("focused") == "true",
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

#[cfg(test)]
mod tests {
    use super::*;
    use pilotdroid_adb::{AdbError, DeviceInfo, Result as AdbResult, ShellOutput};
    use pilotdroid_proto::payload::encode;
    use pilotdroid_proto::{flags, Channel, Frame, MediaType, OpCode};
    use std::collections::HashMap;
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// AdbTransport stub: `forward` spawns a localhost relay on the
    /// requested tcp:N port that proxies each accepted conn to the
    /// CURRENT `active` agent port — models real adb's accept-then-
    /// close (dial succeeds, upstream failure closes the conn).
    /// `app_process` in a shell cmd swaps `active` to `revive`
    /// (None = agent stays dead). All shell cmds + removed forwards
    /// are recorded for assertions.
    struct TestAdb {
        active: Arc<AtomicU16>,
        revive: Option<u16>,
        shells: Arc<Mutex<Vec<String>>>,
        removed: Arc<Mutex<Vec<String>>>,
        /// port → relay alive flag, so `forward_remove` can release the
        /// bound port and a later `forward` can re-bind it.
        listeners: HashMap<u16, Arc<AtomicBool>>,
    }

    impl AdbTransport for TestAdb {
        fn list_devices(&mut self) -> AdbResult<Vec<DeviceInfo>> {
            Ok(vec![])
        }
        fn shell(&mut self, _s: &str, cmd: &str) -> AdbResult<ShellOutput> {
            self.shells.lock().unwrap().push(cmd.to_string());
            if cmd.contains("app_process") {
                if let Some(p) = self.revive {
                    self.active.store(p, Ordering::SeqCst);
                }
            }
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
        fn forward(&mut self, _s: &str, local: &str, _r: &str) -> AdbResult<()> {
            let port: u16 = local.trim_start_matches("tcp:").parse().unwrap();
            let active = self.active.clone();
            if self.listeners.contains_key(&port) {
                // Recovery re-forwards the same port — the relay is
                // already bound; keep it (real adb replaces the rule).
                return Ok(());
            }
            // Bind synchronously — the dial right after `forward` must
            // not race the relay listener. A parallel test may steal
            // the port between free_port() and bind — surface an error
            // so the connect ladder retries a different port instead
            // of panicking the test.
            let l = TcpListener::bind(("127.0.0.1", port)).map_err(AdbError::Io)?;
            l.set_nonblocking(true).unwrap();
            let alive = Arc::new(AtomicBool::new(true));
            self.listeners.insert(port, alive.clone());
            thread::spawn(move || {
                while alive.load(Ordering::SeqCst) {
                    match l.accept() {
                        Ok((mut down, _)) => {
                            let target = active.load(Ordering::SeqCst);
                            thread::spawn(move || {
                                if let Ok(mut up) = TcpStream::connect(("127.0.0.1", target)) {
                                    let mut d2 = down.try_clone().unwrap();
                                    let mut u2 = up.try_clone().unwrap();
                                    thread::spawn(move || {
                                        let _ = std::io::copy(&mut d2, &mut u2);
                                        // propagate half-close like a
                                        // real adb relay
                                        let _ = u2.shutdown(std::net::Shutdown::Both);
                                    });
                                    let _ = std::io::copy(&mut up, &mut down);
                                    let _ = down.shutdown(std::net::Shutdown::Both);
                                }
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Ok(())
        }
        fn forward_remove(&mut self, _s: &str, l: &str) -> AdbResult<()> {
            // Record only — the relay stays bound so idempotent
            // `forward` on the same port keeps working (recovery path).
            self.removed.lock().unwrap().push(l.to_string());
            Ok(())
        }
    }

    /// Minimal fake agent: HELLO + DUMP, tracks accepted conns so the
    /// test can kill them (simulating process death).
    fn spawn_fake() -> (u16, Arc<Mutex<Vec<TcpStream>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let conns = Arc::new(Mutex::new(Vec::new()));
        let conns2 = conns.clone();
        thread::spawn(move || {
            while let Ok((mut s, _)) = l.accept() {
                s.set_nodelay(true).ok();
                conns2.lock().unwrap().push(s.try_clone().unwrap());
                thread::spawn(move || {
                    while let Ok(req) = Frame::read_from(&mut s) {
                        let (ty, payload) = match OpCode::try_from(req.header.msg_type) {
                            Ok(OpCode::Hello) => (
                                OpCode::HelloAck as u16,
                                encode(&HelloAck {
                                    agent: "fake".into(),
                                    caps: 0,
                                    ver: Some("0.0.0".into()),
                                    proto: None,
                                    sdk: None,
                                    abi: None,
                                    display: None,
                                    inject: None,
                                })
                                .unwrap(),
                            ),
                            Ok(OpCode::Dump) => (
                                OpCode::Dump as u16,
                                encode(&DumpAck {
                                    xml: "<hierarchy><node text=\"x\"/></hierarchy>".into(),
                                })
                                .unwrap(),
                            ),
                            _ => continue,
                        };
                        if Frame::new(
                            Channel::Control,
                            ty,
                            req.header.flags & 0x8000,
                            req.header.req_id,
                            payload,
                        )
                        .write_to(&mut s)
                        .is_err()
                        {
                            break;
                        }
                    }
                });
            }
        });
        (port, conns)
    }

    /// A port that stays dead but RESERVED: the listener stays bound
    /// (a freed :0 port can be re-handled to the recovery relay by
    /// `free_port()` — CI flake — and a dial would then connect to the
    /// relay itself) and every accepted conn is closed instantly, so
    /// dials succeed then hit immediate EOF like a dead localabstract.
    fn corpse_port() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let p = l.local_addr().unwrap().port();
        thread::spawn(move || {
            while let Ok((s, _)) = l.accept() {
                drop(s);
            }
        });
        p
    }

    /// Crash the fake agent: accepted conns die (client sees EOF/reset)
    /// and `active` points at a dead port so recovery's first dial
    /// fails just like a real dead localabstract.
    fn kill_agent(conns: &Arc<Mutex<Vec<TcpStream>>>, active: &Arc<AtomicU16>) {
        for c in conns.lock().unwrap().drain(..) {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
        active.store(corpse_port(), Ordering::SeqCst);
    }

    #[allow(clippy::type_complexity)]
    fn adb(
        active: Arc<AtomicU16>,
        revive: Option<u16>,
    ) -> (TestAdb, Arc<Mutex<Vec<String>>>, Arc<Mutex<Vec<String>>>) {
        let shells = Arc::new(Mutex::new(Vec::new()));
        let removed = Arc::new(Mutex::new(Vec::new()));
        (
            TestAdb {
                active,
                revive,
                shells: shells.clone(),
                removed: removed.clone(),
                listeners: HashMap::new(),
            },
            shells,
            removed,
        )
    }

    #[test]
    fn dead_agent_op_recovers_and_replays() {
        let (a, conns_a) = spawn_fake();
        let (b, _conns_b) = spawn_fake();
        let active = Arc::new(AtomicU16::new(a));
        let (adb_t, shells, removed) = adb(active.clone(), Some(b));
        let mut s = Device::with_transport(adb_t, "fake")
            .session_opts(None, "pilotdroid_agent")
            .unwrap();
        assert!(s.dump().unwrap().contains("hierarchy"));
        let old_port = s.port;

        kill_agent(&conns_a, &active);

        // Next op: dead socket → restart ladder → replay on revived agent.
        assert!(s.dump().unwrap().contains("hierarchy"));
        assert_eq!(s.port, old_port, "recovery re-forwards the same port");
        let sh = shells.lock().unwrap().clone();
        assert!(
            sh.iter().any(|c| c.contains("pkill")),
            "restart stop: {sh:?}"
        );
        assert!(
            sh.iter().any(|c| c.contains("app_process")),
            "restart launch: {sh:?}"
        );
        assert!(
            removed
                .lock()
                .unwrap()
                .iter()
                .any(|r| r == &format!("tcp:{old_port}")),
            "stale forward removed"
        );
    }

    #[test]
    fn dead_agent_recovery_exhausted_propagates_error() {
        let (a, conns_a) = spawn_fake();
        let active = Arc::new(AtomicU16::new(a));
        let (adb_t, _shells, _r) = adb(active.clone(), None); // never revives
        let mut s = Device::with_transport(adb_t, "fake")
            .session_opts(None, "pilotdroid_agent")
            .unwrap();
        assert!(s.dump().is_ok());
        kill_agent(&conns_a, &active);
        let err = s.dump().unwrap_err();
        assert!(
            matches!(err, AgentError::Recover { .. }),
            "expected Recover ladder error, got {err:?}"
        );
    }

    #[test]
    fn hung_agent_timeout_is_not_recovered() {
        // Agent accepts but never answers — Timeout must propagate
        // unchanged, NOT trigger the restart ladder (replay safety).
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let hung = l.local_addr().unwrap().port();
        thread::spawn(move || {
            while let Ok((s, _)) = l.accept() {
                thread::spawn(move || {
                    let mut s = s;
                    // Answer HELLO so the session establishes, then go silent.
                    if let Ok(req) = Frame::read_from(&mut s) {
                        let payload = encode(&HelloAck {
                            agent: "hung".into(),
                            caps: 0,
                            ver: Some("0.0.0".into()),
                            proto: None,
                            sdk: None,
                            abi: None,
                            display: None,
                            inject: None,
                        })
                        .unwrap();
                        let _ = Frame::new(
                            Channel::Control,
                            OpCode::HelloAck as u16,
                            req.header.flags & 0x8000,
                            req.header.req_id,
                            payload,
                        )
                        .write_to(&mut s);
                    }
                    loop {
                        thread::sleep(Duration::from_secs(60));
                    }
                });
            }
        });
        let active = Arc::new(AtomicU16::new(hung));
        let (adb_t, shells, _r) = adb(active, Some(corpse_port()));
        let mut s = Device::with_transport(adb_t, "fake")
            .session_opts(None, "pilotdroid_agent")
            .unwrap();
        let err = s.dump().unwrap_err();
        assert!(matches!(err, AgentError::Timeout), "got {err:?}");
        assert!(
            !shells
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.contains("app_process")),
            "timeout must not trigger restart"
        );
    }

    /// Media-frame payload for the demux test without pulling in
    /// serde_bytes: `Vec<u8>` encodes as a msgpack array, which
    /// `ByteBuf` deserializes fine.
    #[derive(serde::Serialize)]
    struct TestMediaFrame {
        format: &'static str,
        w: u32,
        h: u32,
        ts_ns: u64,
        dropped: u64,
        data: Vec<u8>,
    }

    /// Fake agent for stream demux: conn #1 answers HELLO + STREAM_CTRL;
    /// each later conn (media) is fed the pre-canned interleave
    /// [Frame][WatchHit][Frame][Toast][Frame] then held open.
    fn spawn_stream_fake() -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        thread::spawn(move || {
            let mut n = 0u32;
            while let Ok((mut s, _)) = l.accept() {
                s.set_nodelay(true).ok();
                n += 1;
                if n > 1 {
                    thread::spawn(move || {
                        let mut w = s;
                        let frame = |seq: u32| {
                            Frame::new(
                                Channel::Media,
                                MediaType::Frame as u16,
                                flags::STREAM_MORE,
                                seq,
                                encode(&TestMediaFrame {
                                    format: "jpeg",
                                    w: 2,
                                    h: 2,
                                    ts_ns: seq as u64,
                                    dropped: 0,
                                    data: vec![seq as u8],
                                })
                                .unwrap(),
                            )
                        };
                        let ui = |p: Vec<u8>| Frame::new(Channel::Media, MSG_WATCH_HIT, 0, 0, p);
                        let hit = ui(encode(&WatchHit {
                            watch_id: 7,
                            matched: true,
                            waited_ms: 5,
                            action_ok: Some(true),
                            node: None,
                        })
                        .unwrap());
                        let toast = ui(encode(&ToastEvent {
                            r#type: "toast".into(),
                            pkg: "fake".into(),
                            has_parcel: false,
                            text: vec!["hi".into()],
                        })
                        .unwrap());
                        let _ = frame(1).write_to(&mut w);
                        let _ = hit.write_to(&mut w);
                        let _ = frame(2).write_to(&mut w);
                        let _ = toast.write_to(&mut w);
                        let _ = frame(3).write_to(&mut w);
                        loop {
                            thread::sleep(Duration::from_secs(60));
                        }
                    });
                    continue;
                }
                thread::spawn(move || {
                    while let Ok(req) = Frame::read_from(&mut s) {
                        let (ty, payload) = match OpCode::try_from(req.header.msg_type) {
                            Ok(OpCode::Hello) => (
                                OpCode::HelloAck as u16,
                                encode(&HelloAck {
                                    agent: "fake".into(),
                                    caps: 0,
                                    ver: Some("0.0.0".into()),
                                    proto: None,
                                    sdk: None,
                                    abi: None,
                                    display: None,
                                    inject: None,
                                })
                                .unwrap(),
                            ),
                            Ok(OpCode::StreamCtrl) => {
                                let r: StreamCtrlReq = decode(&req.payload).unwrap();
                                let p = match r.op {
                                    StreamOp::Start => encode(&StreamStartAck {
                                        ok: true,
                                        socket: "pilotdroid_media".into(),
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
                            _ => continue,
                        };
                        if Frame::new(
                            Channel::Control,
                            ty,
                            req.header.flags & 0x8000,
                            req.header.req_id,
                            payload,
                        )
                        .write_to(&mut s)
                        .is_err()
                        {
                            break;
                        }
                    }
                });
            }
        });
        port
    }

    #[test]
    fn stream_demux_stashes_watch_hit_and_toast() {
        // On the shared ch=1 conn a watch_hit/toast ui_event can land
        // between stream frames — `media.recv()` used to fail the whole
        // read with UnexpectedResponse and the events died with the
        // conn at teardown. recv_event demuxes them into the stash and
        // watch_recv drains it (documented demux-pause degrade).
        let port = spawn_stream_fake();
        let active = Arc::new(AtomicU16::new(port));
        let (adb_t, _s, _r) = adb(active, None);
        let mut s = Device::with_transport(adb_t, "fake")
            .session_opts(None, "pilotdroid_agent")
            .unwrap();

        s.stream_start(None, None, None).unwrap();
        let frames = s.stream_read(2, None).unwrap();
        assert_eq!(frames.len(), 2);
        // The interleaved hit was stashed mid-read — pullable while the
        // stream is still running.
        assert!(matches!(
            s.watch_recv(Some(500)).unwrap(),
            MediaEvent::WatchHit(_)
        ));
        let frames = s.stream_read(1, None).unwrap();
        assert_eq!(frames.len(), 1);
        s.stream_stop().unwrap();
        // Toast stashed by the second read survives teardown.
        assert!(matches!(
            s.watch_recv(Some(500)).unwrap(),
            MediaEvent::Toast(_)
        ));
        assert!(!s.streaming);
    }
}
