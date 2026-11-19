//! Vendor boundary for the ADB transport layer.
//!
//! ADR-001 (`docs/adr/001-adb-client选型.md`): `adb_client` is the default
//! backend, an `adb` CLI fallback may be added behind the same trait, and a
//! self-hosted smartsocket implementation is the documented exit hatch.
//! Everything above this crate talks only to [`AdbTransport`].

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;

use adb_client::{ADBDeviceExt, ADBServer};

const DEFAULT_ADB_PORT: u16 = 5037;

#[derive(Debug, thiserror::Error)]
pub enum AdbError {
    #[error("adb backend error: {0}")]
    Backend(#[from] adb_client::RustADBError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no device matching `{0}`")]
    DeviceNotFound(String),
}

pub type Result<T> = std::result::Result<T, AdbError>;

/// One entry of `adb devices` output, normalized to our own type so callers
/// never see vendor types.
#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub serial: String,
    pub state: String,
}

impl DeviceInfo {
    /// A device in `device` state is ready to serve requests.
    pub fn is_ready(&self) -> bool {
        self.state == "device"
    }
}

/// Result of a `shell` invocation, de-framed from the shell v2 stream.
#[derive(Debug, Clone, Default)]
pub struct ShellOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// Exit status reported by the device (`None` on shell v1 fallback).
    pub exit_code: Option<i32>,
}

/// The vendor boundary. All device I/O in pilotdroid goes through this trait;
/// swapping the backend underneath must not touch callers.
pub trait AdbTransport {
    /// `adb devices -l` equivalent.
    fn list_devices(&mut self) -> Result<Vec<DeviceInfo>>;

    /// Run a command through `adb shell`. Backends that speak shell v2 must
    /// de-frame the stream into stdout/stderr/exit_code; on a raw v1 stream
    /// the whole response lands in `stdout` and `exit_code` is `None`.
    fn shell(&mut self, serial: &str, command: &str) -> Result<ShellOutput>;

    /// `adb push <local> <remote>`.
    fn push(&mut self, serial: &str, local: &Path, remote: &str) -> Result<()>;

    /// `adb pull <remote> <local>`.
    fn pull(&mut self, serial: &str, remote: &str, local: &Path) -> Result<()>;

    /// `adb forward <local> <remote>` — e.g. `tcp:0` to
    /// `localabstract:pilotdroid`. The agent channel (docs/01-architecture.md)
    /// depends on this.
    fn forward(&mut self, serial: &str, local: &str, remote: &str) -> Result<()>;
}

/// Default backend: the `adb_client` crate against the local adb server.
pub struct AdbClient {
    server: ADBServer,
}

impl AdbClient {
    /// Connect to the adb server at `127.0.0.1:5037`.
    pub fn new() -> Result<Self> {
        Self::at(DEFAULT_ADB_PORT)
    }

    /// Connect to an adb server on a custom port.
    pub fn at(port: u16) -> Result<Self> {
        let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, port);
        Ok(Self {
            server: ADBServer::new(addr),
        })
    }
}

impl AdbTransport for AdbClient {
    fn list_devices(&mut self) -> Result<Vec<DeviceInfo>> {
        Ok(self
            .server
            .devices()?
            .into_iter()
            .map(|d| DeviceInfo {
                serial: d.identifier,
                state: d.state.to_string(),
            })
            .collect())
    }

    fn shell(&mut self, serial: &str, command: &str) -> Result<ShellOutput> {
        let mut device = self
            .server
            .get_device_by_name(serial)
            .map_err(|e| match e {
                adb_client::RustADBError::DeviceNotFound(_) => {
                    AdbError::DeviceNotFound(serial.to_string())
                }
                other => AdbError::Backend(other),
            })?;
        let mut raw = Vec::new();
        device.shell_command(&[command], &mut raw)?;
        // adb_client requests shell v2 when the host supports it but returns
        // the stream un-de-framed (packet headers included). Parse v2 packets;
        // anything that doesn't parse cleanly is a v1 raw stream.
        Ok(deframe_shell_v2(&raw).unwrap_or_else(|| ShellOutput {
            stdout: raw,
            stderr: Vec::new(),
            exit_code: None,
        }))
    }

    fn push(&mut self, serial: &str, local: &Path, remote: &str) -> Result<()> {
        let mut device = self
            .server
            .get_device_by_name(serial)
            .map_err(|_| AdbError::DeviceNotFound(serial.to_string()))?;
        let mut file = std::fs::File::open(local)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        device
            .push(buf.as_slice(), remote)
            .map_err(AdbError::Backend)
    }

    fn pull(&mut self, serial: &str, remote: &str, local: &Path) -> Result<()> {
        let mut device = self
            .server
            .get_device_by_name(serial)
            .map_err(|_| AdbError::DeviceNotFound(serial.to_string()))?;
        let mut file = std::fs::File::create(local)?;
        device
            .pull(&remote, &mut file as &mut dyn Write)
            .map_err(AdbError::Backend)
    }

    fn forward(&mut self, serial: &str, local: &str, remote: &str) -> Result<()> {
        let mut device = self
            .server
            .get_device_by_name(serial)
            .map_err(|_| AdbError::DeviceNotFound(serial.to_string()))?;
        device
            .forward(remote.to_string(), local.to_string())
            .map_err(AdbError::Backend)
    }
}

// ---- shell v2 de-framing ---------------------------------------------------
// AOSP shell_protocol v2 packet: [id:u8][len:u32 LE][payload]. Ids per
// packages/services/core/jni/.../shell_protocol: stdin=0, stdout=1, stderr=2,
// exit=3, close_stdin=4, window_size_change=5.

mod shell_v2 {
    pub const STDOUT: u8 = 1;
    pub const STDERR: u8 = 2;
    pub const EXIT: u8 = 3;
    pub const MAX_KNOWN_ID: u8 = 5;
}

/// Parse a complete shell v2 stream into stdout/stderr/exit_code.
/// Returns `None` when the buffer is not a well-formed v2 stream — callers
/// then treat it as shell v1 raw output.
fn deframe_shell_v2(raw: &[u8]) -> Option<ShellOutput> {
    let mut out = ShellOutput::default();
    let mut cursor = 0usize;
    let mut saw_exit = false;

    while cursor < raw.len() {
        if raw.len() - cursor < 5 {
            return None;
        }
        let id = raw[cursor];
        let len = u32::from_le_bytes(raw[cursor + 1..cursor + 5].try_into().ok()?) as usize;
        cursor += 5;
        if id > shell_v2::MAX_KNOWN_ID || len > raw.len() - cursor {
            return None;
        }
        let payload = &raw[cursor..cursor + len];
        cursor += len;
        match id {
            shell_v2::STDOUT => out.stdout.extend_from_slice(payload),
            shell_v2::STDERR => out.stderr.extend_from_slice(payload),
            shell_v2::EXIT => {
                saw_exit = true;
                if let Some(&code) = payload.first() {
                    out.exit_code = Some(code as i32);
                }
            }
            _ => {} // stdin echo / close_stdin / window_size_change: drop
        }
    }

    if saw_exit {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v2_packet(id: u8, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![id];
        p.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        p.extend_from_slice(payload);
        p
    }

    #[test]
    fn device_info_ready_state() {
        let ready = DeviceInfo {
            serial: "ba0d072d".into(),
            state: "device".into(),
        };
        let offline = DeviceInfo {
            serial: "x".into(),
            state: "offline".into(),
        };
        assert!(ready.is_ready());
        assert!(!offline.is_ready());
    }

    #[test]
    fn deframe_splits_stdout_stderr_and_exit() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&v2_packet(shell_v2::STDOUT, b"hello\n"));
        raw.extend_from_slice(&v2_packet(shell_v2::STDERR, b"oops\n"));
        raw.extend_from_slice(&v2_packet(shell_v2::EXIT, &[42]));

        let out = deframe_shell_v2(&raw).expect("well-formed v2 stream");
        assert_eq!(out.stdout, b"hello\n");
        assert_eq!(out.stderr, b"oops\n");
        assert_eq!(out.exit_code, Some(42));
    }

    #[test]
    fn deframe_rejects_v1_raw_stream() {
        // v1 output is just bytes; "34\n" must NOT parse as a v2 stream.
        assert!(deframe_shell_v2(b"34\n").is_none());
    }

    #[test]
    fn deframe_rejects_truncated_packet() {
        let mut raw = v2_packet(shell_v2::STDOUT, b"abc");
        raw.truncate(raw.len() - 1);
        assert!(deframe_shell_v2(&raw).is_none());
    }
}
