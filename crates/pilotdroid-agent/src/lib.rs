//! Host-side client and lifecycle manager for the on-device pilotdroid agent.
//!
//! Two halves, per docs/01-architecture.md and docs/03-device-agent.md:
//! - [`AgentClient`]: framed RPC over a forwarded TCP↔localabstract socket.
//! - [`AgentManager`]: deploy/launch/stop the dex agent through [`AdbTransport`]
//!   so it stays backend-agnostic (adr/001).

mod client;
mod manager;

pub use client::AgentClient;
pub use manager::{AgentManager, AGENT_DIR, AGENT_JAR_REMOTE, AGENT_MAIN_CLASS, AGENT_SOCKET};

use std::io;

use pilotdroid_adb::AdbError;
use pilotdroid_proto::ProtocolError;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("timed out waiting for agent response")]
    Timeout,
    #[error("protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("adb transport error: {0}")]
    Adb(#[from] AdbError),
    /// ERROR-flagged response from the agent (payload = ErrorAck).
    #[error("agent error {code}: {message}")]
    Remote { code: String, message: String },
    #[error("unexpected response: expected {expected}, got msg_type 0x{actual:04x}")]
    UnexpectedResponse { expected: &'static str, actual: u16 },
    #[error("req_id mismatch: sent {sent}, got {got}")]
    ReqIdMismatch { sent: u32, got: u32 },
    /// A device-side shell command (mkdir/pkill/launch) exited non-zero.
    #[error("remote shell failed (exit {code:?}): {stderr}")]
    Shell { code: Option<i32>, stderr: String },
    /// Watchdog recovery could not restore the agent (docs/02 §4.6).
    #[error("watchdog recovery failed at {stage}: {detail}")]
    Recover { stage: &'static str, detail: String },
    #[error("unimplemented: {0}")]
    Unimplemented(&'static str),
}

pub type Result<T> = std::result::Result<T, AgentError>;
