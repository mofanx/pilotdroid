//! Media-channel frame receiver (docs/02 §4.8, ADR-006).
//!
//! The media channel is a *second* forwarded socket — agent pushes ch=1
//! FRAME messages (msg_type 0x0001, STREAM_MORE set on all but the
//! terminal frame). Backpressure is drop-not-block on the agent side; the
//! host observes loss through `req_id`-as-seq gaps and the cumulative
//! `dropped` counter in each frame payload.

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use pilotdroid_proto::payload::MediaFrame;
use pilotdroid_proto::*;

use crate::{AgentError, Result};

/// Running counters for a stream session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StreamStats {
    /// FRAME messages successfully decoded.
    pub frames: u64,
    /// Sum of `MediaFrame.data` bytes (JPEG payload, header excluded).
    pub bytes: u64,
    /// Seq discontinuities observed (req_id skipped ahead — agent dropped
    /// under backpressure or a frame was lost mid-stream).
    pub seq_gaps: u64,
    /// Latest cumulative `dropped` counter reported by the agent.
    pub dropped: u64,
}

/// Blocking receiver for the media socket. One stream per connection;
/// the agent closes the socket (or clears STREAM_MORE) on stop/disconnect.
pub struct MediaClient {
    stream: TcpStream,
    last_seq: Option<u32>,
    ended: bool,
    stats: StreamStats,
}

/// A frame on the media channel: a video FRAME (0x0001) or a pushed
/// ui_event `WATCH_HIT` (0x0002, §4.9 — async watcher result).
#[derive(Debug)]
pub enum MediaEvent {
    Frame(MediaFrame),
    WatchHit(pilotdroid_proto::payload::WatchHit),
}

impl MediaClient {
    /// Connect to the forwarded media port. Read timeout bounds a stall —
    /// a stream that goes quiet longer than `timeout` surfaces `Timeout`.
    pub fn connect(addr: SocketAddr, timeout: Duration) -> Result<Self> {
        let stream = TcpStream::connect_timeout(&addr, timeout)?;
        stream.set_read_timeout(Some(timeout))?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            last_seq: None,
            ended: false,
            stats: StreamStats::default(),
        })
    }

    /// Receive the next frame. Validates ch=1 + msg_type 0x0001 (§4.8 —
    /// anything else on this socket is a protocol violation), tracks seq
    /// gaps, and marks the stream ended when STREAM_MORE clears.
    ///
    /// `Err(Timeout)` means the agent went quiet; `Err(Io/UnexpectedEof)`
    /// means the socket closed mid-stream — the caller decides whether
    /// that is a clean stop (`stats().frames > 0` + control-side stop ack)
    /// or a断流.
    pub fn recv(&mut self) -> Result<MediaFrame> {
        let frame = Frame::read_from(&mut self.stream).map_err(|e| match e {
            ProtocolError::Io(io) => match io.kind() {
                ErrorKind::TimedOut | ErrorKind::WouldBlock => AgentError::Timeout,
                _ => AgentError::Io(io),
            },
            other => AgentError::Protocol(other),
        })?;
        if frame.header.channel != Channel::Media
            || frame.header.msg_type != MediaType::Frame as u16
        {
            return Err(AgentError::UnexpectedResponse {
                expected: "FRAME on media channel",
                actual: frame.header.msg_type,
            });
        }
        let seq = frame.header.req_id;
        if let Some(last) = self.last_seq {
            // seq is monotonic; a jump means frames were dropped or lost.
            self.stats.seq_gaps += seq.wrapping_sub(last).saturating_sub(1) as u64;
        }
        self.last_seq = Some(seq);
        if frame.header.flags & flags::STREAM_MORE == 0 {
            self.ended = true;
        }
        let f: MediaFrame = pilotdroid_proto::payload::decode(&frame.payload)?;
        self.stats.frames += 1;
        self.stats.bytes += f.data.len() as u64;
        self.stats.dropped = f.dropped;
        Ok(f)
    }

    /// Receive the next media-channel event — a video FRAME or a `WATCH_HIT`
    /// ui_event (§4.9). Used by async-watch clients, which hold the media
    /// socket pre-start and read hits pushed by armed agent watchers.
    /// `Err(Timeout)` = nothing arrived within the read timeout.
    pub fn recv_event(&mut self) -> Result<MediaEvent> {
        let frame = Frame::read_from(&mut self.stream).map_err(|e| match e {
            ProtocolError::Io(io) => match io.kind() {
                ErrorKind::TimedOut | ErrorKind::WouldBlock => AgentError::Timeout,
                _ => AgentError::Io(io),
            },
            other => AgentError::Protocol(other),
        })?;
        if frame.header.channel != Channel::Media {
            return Err(AgentError::UnexpectedResponse {
                expected: "event on media channel",
                actual: frame.header.msg_type,
            });
        }
        match frame.header.msg_type {
            t if t == MediaType::Frame as u16 => {
                let seq = frame.header.req_id;
                if let Some(last) = self.last_seq {
                    self.stats.seq_gaps += seq.wrapping_sub(last).saturating_sub(1) as u64;
                }
                self.last_seq = Some(seq);
                if frame.header.flags & flags::STREAM_MORE == 0 {
                    self.ended = true;
                }
                let f: MediaFrame = pilotdroid_proto::payload::decode(&frame.payload)?;
                self.stats.frames += 1;
                self.stats.bytes += f.data.len() as u64;
                self.stats.dropped = f.dropped;
                Ok(MediaEvent::Frame(f))
            }
            t if t == pilotdroid_proto::payload::MSG_WATCH_HIT => {
                let h: pilotdroid_proto::payload::WatchHit =
                    pilotdroid_proto::payload::decode(&frame.payload)?;
                Ok(MediaEvent::WatchHit(h))
            }
            other => Err(AgentError::UnexpectedResponse {
                expected: "FRAME or WATCH_HIT on media channel",
                actual: other,
            }),
        }
    }

    /// True after a terminal frame (STREAM_MORE cleared) was received.
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// Seq (`req_id`) of the last received frame — for frame file naming.
    pub fn last_seq(&self) -> Option<u32> {
        self.last_seq
    }

    pub fn stats(&self) -> &StreamStats {
        &self.stats
    }
}
