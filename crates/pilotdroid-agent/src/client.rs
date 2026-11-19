//! Framed RPC client over a TCP stream (typically `adb forward`-ed to
//! `localabstract:pilotdroid`).

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use pilotdroid_proto::payload::*;
use pilotdroid_proto::*;

use crate::{AgentError, Result};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

// MotionEvent.ACTION_* (android.view.MotionEvent) — stable across API levels.
const ACTION_DOWN: u8 = 0;
const ACTION_UP: u8 = 1;
const ACTION_MOVE: u8 = 2;
/// Gap between DOWN and UP for a synthesized tap.
const TAP_DURATION_MS: u64 = 60;

/// One-shot framed RPC client. Sync I/O — the socket is a localhost forward,
/// so a blocking TcpStream with timeouts is sufficient and keeps the client
/// usable from any thread context.
pub struct AgentClient {
    stream: TcpStream,
    next_req: u32,
}

impl AgentClient {
    /// Connect to `addr` (e.g. `127.0.0.1:17900`) with the default timeout.
    pub fn connect(addr: SocketAddr) -> Result<Self> {
        Self::connect_timeout(addr, DEFAULT_TIMEOUT)
    }

    pub fn connect_timeout(addr: SocketAddr, timeout: Duration) -> Result<Self> {
        let stream = TcpStream::connect_timeout(&addr, timeout)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_write_timeout(Some(timeout))?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            next_req: 0,
        })
    }

    /// HELLO handshake — returns agent version + capability bitmap.
    pub fn hello(&mut self) -> Result<HelloAck> {
        let f = self.request(
            OpCode::Hello,
            encode(&HelloReq {
                proto: PROTOCOL_VERSION,
            })?,
        )?;
        expect(&f, OpCode::HelloAck, "HELLO_ACK")?;
        Ok(decode(&f.payload)?)
    }

    /// PING — heartbeat round trip (response must be PONG).
    pub fn ping(&mut self) -> Result<()> {
        let f = self.request(OpCode::Ping, Vec::new())?;
        expect(&f, OpCode::Pong, "PONG")
    }

    /// INJECT a MotionEvent sequence (kind + points describe the gesture).
    pub fn inject(&mut self, req: InjectReq) -> Result<InjectAck> {
        let f = self.request(OpCode::Inject, encode(&req)?)?;
        expect(&f, OpCode::InjectResult, "INJECT_RESULT")?;
        Ok(decode(&f.payload)?)
    }

    /// INJECT with the NOACK flag — fire-and-forget, no response frame is
    /// sent by the agent (docs/02 §4.5). Execution errors are dropped
    /// device-side; use only for discardable ops.
    pub fn inject_noack(&mut self, req: InjectReq) -> Result<()> {
        self.request_noack(OpCode::Inject, encode(&req)?)
    }

    /// Convenience: simple tap at (x, y) — ACTION_DOWN + ACTION_UP.
    pub fn tap(&mut self, x: f32, y: f32, wait_finish: bool) -> Result<InjectAck> {
        self.inject(InjectReq {
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
                    t_ms: TAP_DURATION_MS,
                },
            ],
            wait_finish,
            keycode: None,
            meta: None,
            backend: None,
        })
    }

    /// Convenience: [`tap`](Self::tap) + NOACK — returns after the write.
    pub fn tap_noack(&mut self, x: f32, y: f32) -> Result<()> {
        self.inject_noack(InjectReq {
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
                    t_ms: TAP_DURATION_MS,
                },
            ],
            wait_finish: false,
            keycode: None,
            meta: None,
            backend: None,
        })
    }

    /// Convenience: KEY — KeyEvent down+up through the same
    /// `injectInputEvent` channel (docs/02 §4.7). `keycode` is an
    /// `android.view.KeyEvent.KEYCODE_*` value (back=4, home=3,
    /// recent=187); `meta` is the metaState, `None` = 0.
    pub fn press(&mut self, keycode: u32, meta: Option<u32>) -> Result<InjectAck> {
        self.inject(InjectReq {
            kind: InjectKind::Key,
            points: vec![],
            wait_finish: false,
            keycode: Some(keycode),
            meta,
            backend: None,
        })
    }

    /// Convenience: swipe — interpolates a straight DOWN→MOVE…→UP event
    /// sequence (docs/02 §4.7). `steps` MOVE points are placed ~10ms apart;
    /// `duration_ms` is the total gesture time.
    pub fn swipe(
        &mut self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        duration_ms: u64,
    ) -> Result<InjectAck> {
        let steps = ((duration_ms / 10) as usize).clamp(1, 60);
        let mut points = Vec::with_capacity(steps + 2);
        points.push(MotionPoint {
            action: ACTION_DOWN,
            x: x1,
            y: y1,
            t_ms: 0,
        });
        for i in 1..=steps {
            let t = i as f32 / (steps + 1) as f32;
            points.push(MotionPoint {
                action: ACTION_MOVE,
                x: x1 + (x2 - x1) * t,
                y: y1 + (y2 - y1) * t,
                t_ms: (duration_ms as f32 * t) as u64,
            });
        }
        points.push(MotionPoint {
            action: ACTION_UP,
            x: x2,
            y: y2,
            t_ms: duration_ms,
        });
        self.inject(InjectReq {
            kind: InjectKind::Swipe,
            points,
            wait_finish: true,
            keycode: None,
            meta: None,
            backend: None,
        })
    }

    /// DUMP — returns the UI hierarchy XML.
    pub fn dump(&mut self) -> Result<String> {
        let f = self.request(
            OpCode::Dump,
            encode(&DumpReq {
                format: "xml".into(),
            })?,
        )?;
        expect(&f, OpCode::Dump, "DUMP response")?;
        let ack: DumpAck = decode(&f.payload)?;
        Ok(ack.xml)
    }

    /// QUERY — selector lookup on the resident tree (docs/02 §4.4).
    pub fn query(&mut self, req: QueryReq) -> Result<QueryAck> {
        let f = self.request(OpCode::Query, encode(&req)?)?;
        expect(&f, OpCode::Query, "QUERY response")?;
        Ok(decode(&f.payload)?)
    }

    /// Convenience: `query` reduced to the exists bit.
    pub fn exists(&mut self, selector: Selector) -> Result<bool> {
        Ok(self
            .query(QueryReq {
                selector,
                attrs: None,
            })?
            .found)
    }

    /// TEXT — `ACTION_SET_TEXT` on the focused node (docs/02 §4.4).
    pub fn set_text(&mut self, text: &str, clear: bool) -> Result<()> {
        let f = self.request(
            OpCode::Text,
            encode(&TextReq {
                text: text.into(),
                clear: Some(clear),
            })?,
        )?;
        expect(&f, OpCode::Text, "TEXT response")?;
        let ack: TextAck = decode(&f.payload)?;
        if !ack.ok {
            return Err(AgentError::Remote {
                code: "TEXT_FAILED".into(),
                message: "ACTION_SET_TEXT returned false".into(),
            });
        }
        Ok(())
    }

    /// BATCH — ordered op array, one response frame with per-op results
    /// (docs/02 §4.1/4.2).
    pub fn batch(&mut self, req: BatchReq) -> Result<BatchAck> {
        let f = self.request(OpCode::Batch, encode(&req)?)?;
        expect(&f, OpCode::BatchResult, "BATCH_RESULT")?;
        Ok(decode(&f.payload)?)
    }

    /// Convenience: BATCH with default policy (fail-stop, no deadline).
    pub fn batch_ops(&mut self, ops: Vec<BatchOp>) -> Result<BatchAck> {
        self.batch(BatchReq {
            ops,
            fail_policy: FailPolicy::Stop,
            deadline_ms: None,
            poll_ms: None,
        })
    }

    /// SCREENSHOT — returns (format, image bytes). `scale` ∈ (0,1]
    /// downscales on-device before encoding (docs/02 §4.4); `None` keeps the
    /// M1 two-field wire shape.
    pub fn screenshot(
        &mut self,
        format: &str,
        quality: u8,
        scale: Option<f32>,
    ) -> Result<(String, Vec<u8>)> {
        let f = self.request(
            OpCode::Screenshot,
            encode(&ShotReq {
                format: format.into(),
                quality,
                scale,
            })?,
        )?;
        expect(&f, OpCode::Screenshot, "SCREENSHOT response")?;
        let ack: ShotAck = decode(&f.payload)?;
        Ok((ack.format, ack.data.into_vec()))
    }

    /// STREAM_CTRL start (docs/02 §4.8) — the returned `socket` names the
    /// localabstract the media channel listens on; the caller forwards a
    /// second port to it and connects a [`MediaClient`](crate::MediaClient).
    /// `ERR_STREAM_BUSY` (Remote) when a stream is already bound to a
    /// control session.
    pub fn stream_start(&mut self, req: StreamCtrlReq) -> Result<StreamStartAck> {
        let f = self.request(OpCode::StreamCtrl, encode(&req)?)?;
        expect(&f, OpCode::StreamCtrl, "STREAM_CTRL response")?;
        Ok(decode(&f.payload)?)
    }

    /// STREAM_CTRL stop — agent joins the capture thread and reports
    /// `{frames_sent, dropped}` (§4.8 cleanup semantics).
    pub fn stream_stop(&mut self) -> Result<StreamStopAck> {
        let f = self.request(
            OpCode::StreamCtrl,
            encode(&StreamCtrlReq {
                op: StreamOp::Stop,
                fps: None,
                scale: None,
                quality: None,
                format: None,
            })?,
        )?;
        expect(&f, OpCode::StreamCtrl, "STREAM_CTRL response")?;
        Ok(decode(&f.payload)?)
    }

    /// Ask the agent to exit (AGENT_STOP). The agent may close the socket
    /// before/instead of replying, so a missing response is success too.
    pub fn stop(&mut self) -> Result<()> {
        let req_id = self.alloc_req();
        Frame::new(
            Channel::Control,
            OpCode::AgentStop as u16,
            0,
            req_id,
            Vec::new(),
        )
        .write_to(&mut self.stream)?;
        match self.read_response() {
            Ok(resp) if resp.header.flags & flags::ERROR != 0 => {
                let ack: ErrorAck = decode(&resp.payload)?;
                Err(AgentError::Remote {
                    code: ack.code,
                    message: ack.message,
                })
            }
            _ => Ok(()),
        }
    }

    /// Low-level escape hatch: send `op` + raw msgpack `payload`, apply the
    /// common req_id/timeout/ERROR-flag handling, return the response frame.
    /// Useful for opcodes without a typed wrapper yet (e.g. Query, Batch).
    pub fn call(&mut self, op: OpCode, payload: Vec<u8>) -> Result<Frame> {
        self.request(op, payload)
    }

    fn alloc_req(&mut self) -> u32 {
        let id = self.next_req;
        self.next_req = self.next_req.wrapping_add(1);
        id
    }

    fn request_noack(&mut self, op: OpCode, payload: Vec<u8>) -> Result<()> {
        let req_id = self.alloc_req();
        Frame::new(Channel::Control, op as u16, flags::NOACK, req_id, payload)
            .write_to(&mut self.stream)?;
        Ok(())
    }

    fn request(&mut self, op: OpCode, payload: Vec<u8>) -> Result<Frame> {
        let req_id = self.alloc_req();
        Frame::new(Channel::Control, op as u16, 0, req_id, payload).write_to(&mut self.stream)?;
        let resp = self.read_response()?;
        if resp.header.req_id != req_id {
            return Err(AgentError::ReqIdMismatch {
                sent: req_id,
                got: resp.header.req_id,
            });
        }
        if resp.header.flags & flags::ERROR != 0 {
            let ack: ErrorAck = decode(&resp.payload)?;
            return Err(AgentError::Remote {
                code: ack.code,
                message: ack.message,
            });
        }
        Ok(resp)
    }

    fn read_response(&mut self) -> Result<Frame> {
        Frame::read_from(&mut self.stream).map_err(|e| match e {
            ProtocolError::Io(io) => map_read_error(io),
            other => AgentError::Protocol(other),
        })
    }
}

fn expect(f: &Frame, ty: OpCode, what: &'static str) -> Result<()> {
    if f.header.msg_type != ty as u16 {
        return Err(AgentError::UnexpectedResponse {
            expected: what,
            actual: f.header.msg_type,
        });
    }
    Ok(())
}

/// Map stream read errors to [`AgentError`], treating timeout-ish kinds
/// (`TimedOut`/`WouldBlock` on a socket with read timeout set) as Timeout.
fn map_read_error(e: std::io::Error) -> AgentError {
    match e.kind() {
        ErrorKind::TimedOut | ErrorKind::WouldBlock => AgentError::Timeout,
        _ => AgentError::Io(e),
    }
}
