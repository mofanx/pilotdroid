//! Vendor boundary for the ADB transport layer.
//!
//! ADR-001 (`docs/adr/001-adb-client选型.md`): `adb_client` is the default
//! backend, an `adb` CLI fallback may be added behind the same trait, and a
//! self-hosted smartsocket implementation is the documented exit hatch.
//! Everything above this crate talks only to [`AdbTransport`].

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::path::Path;

use adb_client::server::ADBServer;
use adb_client::server_device::ADBServerDevice;
use adb_client::ADBDeviceExt;

const DEFAULT_ADB_PORT: u16 = 5037;

/// Strip the trailing `__PD_EXIT_<digits>__\n` marker appended to every
/// `shell` command (see `AdbTransport::shell`) and return the exit status.
/// Marker is only honored when it terminates the output; `None` on v1
/// hosts / unclosed-quote commands where the marker never prints.
/// Wrap `command` in a subshell and append a printf that emits the exit
/// marker — see [`AdbClient::shell`] for why (adb_client 4.0.0 drops the
/// v2 ExitStatus). The subshell keeps `&`-terminated commands (agent
/// start) parseable: bare `cmd &; printf` is a `sh` syntax error.
fn marked_shell(command: &str) -> String {
    const MARK: &str = "__PD_EXIT_";
    format!("( {command} ); printf '{MARK}%d__\\n' $?")
}

fn take_exit_marker(stdout: &mut Vec<u8>) -> Option<i32> {
    const MARK: &[u8] = b"__PD_EXIT_";
    let body = stdout.as_slice().strip_suffix(b"\n").unwrap_or(stdout);
    let pos = body.windows(MARK.len()).rposition(|w| w == MARK)?;
    let digits = &body[pos + MARK.len()..];
    let digits = digits.strip_suffix(b"__")?;
    if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let code = std::str::from_utf8(digits).ok()?.parse().ok()?;
    stdout.truncate(pos);
    Some(code)
}

#[derive(Debug, thiserror::Error)]
pub enum AdbError {
    /// Backend failure. Carries the vendor error's display text only — the
    /// vendor error type itself never leaks into this public enum (adr/001:
    /// `adb_client` must be replaceable behind this boundary).
    #[error("adb backend error: {0}")]
    Backend(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no device matching `{0}`")]
    DeviceNotFound(String),
    #[error("protocol error: {0}")]
    Protocol(String),
}

impl AdbError {
    /// Fold a vendor error into our public type. Vendor enums stay inside this
    /// crate's implementation; callers only ever see `Backend(String)`.
    fn backend(e: impl std::fmt::Display) -> Self {
        Self::Backend(e.to_string())
    }
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
    /// `localabstract:pilotdroid_agent`. The agent channel
    /// (docs/01-architecture.md) depends on this.
    fn forward(&mut self, serial: &str, local: &str, remote: &str) -> Result<()>;

    /// `adb forward --remove <local>` — e.g. `tcp:17932`. Implementations
    /// must remove only that one rule (never killforward-all).
    fn forward_remove(&mut self, serial: &str, local: &str) -> Result<()>;
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

    /// Vendor `get_device_by_name`, normalized to our error type. Matching on
    /// the vendor's `DeviceNotFound` variant is fine here — it's an internal
    /// detail, unlike exposing it in `AdbError`'s signature.
    fn device(&mut self, serial: &str) -> Result<ADBServerDevice> {
        self.server.get_device_by_name(serial).map_err(|e| match e {
            adb_client::RustADBError::DeviceNotFound(_) => {
                AdbError::DeviceNotFound(serial.to_string())
            }
            other => AdbError::backend(other),
        })
    }
}

impl AdbTransport for AdbClient {
    fn list_devices(&mut self) -> Result<Vec<DeviceInfo>> {
        Ok(self
            .server
            .devices()
            .map_err(AdbError::backend)?
            .into_iter()
            .map(|d| DeviceInfo {
                serial: d.identifier,
                state: d.state.to_string(),
            })
            .collect())
    }

    fn shell(&mut self, serial: &str, command: &str) -> Result<ShellOutput> {
        let device = self.device(serial)?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        // adb_client 4.x speaks shell v2 itself when the host supports it
        // (stdout/stderr arrive de-framed), but 4.0.0's v2 loop returns
        // Ok(None) on clean EOF — the parsed ExitStatus is discarded, so
        // the return value never carries the exit code. Recover it with a
        // trailing marker instead — see marked_shell().
        let marked = marked_shell(command);
        device
            .shell_command(&marked, Some(&mut stdout), Some(&mut stderr))
            .map_err(AdbError::backend)?;
        Ok(ShellOutput {
            stderr,
            exit_code: take_exit_marker(&mut stdout),
            stdout,
        })
    }

    fn push(&mut self, serial: &str, local: &Path, remote: &str) -> Result<()> {
        let device = self.device(serial)?;
        let mut file = std::fs::File::open(local)?;
        let mut buf = Vec::new();
        file.read_to_end(&mut buf)?;
        device
            .push(buf.as_slice(), remote)
            .map_err(AdbError::backend)
    }

    fn pull(&mut self, serial: &str, remote: &str, local: &Path) -> Result<()> {
        let device = self.device(serial)?;
        let mut file = std::fs::File::create(local)?;
        device.pull(remote, &mut file).map_err(AdbError::backend)
    }

    fn forward(&mut self, serial: &str, local: &str, remote: &str) -> Result<()> {
        let device = self.device(serial)?;
        device
            .forward(remote.to_string(), local.to_string())
            .map(|_| ())
            .map_err(AdbError::backend)
    }

    fn forward_remove(&mut self, serial: &str, local: &str) -> Result<()> {
        // adb_client 4.x exposes per-forward removal (2.x only had
        // killforward-all — the raw smartsocket fallback is retired).
        let device = self.device(serial)?;
        device
            .forward_remove(local.to_string())
            .map_err(AdbError::backend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_marker_parses_and_strips() {
        let mut out = b"hello\n__PD_EXIT_7__\n".to_vec();
        assert_eq!(take_exit_marker(&mut out), Some(7));
        assert_eq!(out, b"hello\n");
    }

    #[test]
    fn exit_marker_without_trailing_content() {
        let mut out = b"__PD_EXIT_0__\n".to_vec();
        assert_eq!(take_exit_marker(&mut out), Some(0));
        assert!(out.is_empty());
    }

    #[test]
    fn exit_marker_rejects_non_digits_and_misplaced() {
        let mut out = b"__PD_EXIT_x__\n".to_vec();
        assert_eq!(take_exit_marker(&mut out), None);
        let mut out2 = b"__PD_EXIT_1__ trailing\n".to_vec();
        assert_eq!(take_exit_marker(&mut out2), None);
        assert_eq!(out2, b"__PD_EXIT_1__ trailing\n");
    }

    #[test]
    fn marker_wraps_in_subshell() {
        // `&`-terminated commands (agent start) must land inside the
        // subshell — bare `cmd &; printf` is a sh syntax error.
        assert_eq!(
            marked_shell("nohup setsid env A=b c </dev/null >log 2>&1 &"),
            "( nohup setsid env A=b c </dev/null >log 2>&1 & ); printf '__PD_EXIT_%d__\\n' $?"
        );
        assert_eq!(
            marked_shell("test -f /x"),
            "( test -f /x ); printf '__PD_EXIT_%d__\\n' $?"
        );
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
}
