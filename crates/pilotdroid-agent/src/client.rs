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

    /// INJECT a MotionEvent sequence (kind + points describe the gesture).
    pub fn inject(&mut self, req: InjectReq) -> Result<InjectAck> {
        let f = self.request(OpCode::Inject, encode(&req)?)?;
        expect(&f, OpCode::InjectResult, "INJECT_RESULT")?;
        Ok(decode(&f.payload)?)
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

    /// SCREENSHOT — returns (format, image bytes).
    pub fn screenshot(&mut self, format: &str, quality: u8) -> Result<(String, Vec<u8>)> {
        let f = self.request(
            OpCode::Screenshot,
            encode(&ShotReq {
                format: format.into(),
                quality,
            })?,
        )?;
        expect(&f, OpCode::Screenshot, "SCREENSHOT response")?;
        let ack: ShotAck = decode(&f.payload)?;
        Ok((ack.format, ack.data.into_vec()))
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
