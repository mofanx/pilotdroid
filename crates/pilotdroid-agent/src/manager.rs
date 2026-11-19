//! Agent lifecycle: deploy jar, launch daemon, stop, connect.
//! All device I/O goes through [`AdbTransport`] — no direct adb calls here.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;

use pilotdroid_adb::AdbTransport;

use crate::client::DEFAULT_TIMEOUT;
use crate::{AgentClient, AgentError, Result};

/// On-device working directory for the agent.
pub const AGENT_DIR: &str = "/data/local/tmp/pilotdroid";
/// Remote path the dex jar is pushed to.
pub const AGENT_JAR_REMOTE: &str = "/data/local/tmp/pilotdroid/agent.jar";
/// Entry class inside the dex jar (`pilotdroid.Agent` — must match agent/).
pub const AGENT_MAIN_CLASS: &str = "pilotdroid.Agent";
/// The localabstract socket name the agent listens on.
pub const AGENT_SOCKET: &str = "pilotdroid";

const AGENT_LOG: &str = "/data/local/tmp/pilotdroid/agent.log";

/// Deploy/launch/connect orchestration over an [`AdbTransport`].
pub struct AgentManager<'a, T: AdbTransport> {
    adb: &'a mut T,
}

impl<'a, T: AdbTransport> AgentManager<'a, T> {
    pub fn new(adb: &'a mut T) -> Self {
        Self { adb }
    }

    /// `agent install`: ensure dir + push jar (idempotent; caller should
    /// compare md5 if incremental deploy matters — M1 pushes unconditionally).
    pub fn install(&mut self, serial: &str, local_jar: &Path) -> Result<()> {
        self.run_shell(serial, &format!("mkdir -p {AGENT_DIR}"))?;
        self.adb.push(serial, local_jar, AGENT_JAR_REMOTE)?;
        Ok(())
    }

    /// `agent start`: launch the dex detached so it survives this shell
    /// session (daemon model, docs/03 §5). Mirrors deploy-agent.sh:
    /// `nohup setsid sh -c 'CLASSPATH=… app_process …' </dev/null >log 2>&1 &`
    pub fn start(&mut self, serial: &str) -> Result<()> {
        let cmd = format!(
            "nohup setsid sh -c 'CLASSPATH={AGENT_JAR_REMOTE} app_process {AGENT_DIR} {AGENT_MAIN_CLASS}' </dev/null >{AGENT_LOG} 2>&1 &"
        );
        self.run_shell(serial, &cmd)
    }

    /// `agent stop`: kill by process name. Polite AGENT_STOP RPC is preferred
    /// when a client is connected; this is the unconditional fallback.
    pub fn stop(&mut self, serial: &str) -> Result<()> {
        self.run_shell(serial, &format!("pkill -f {AGENT_MAIN_CLASS} || true"))
    }

    /// forward `tcp:{port}` → `localabstract:{AGENT_SOCKET}` for `serial`,
    /// then connect. `port=None` picks a free localhost port.
    /// Returns the client and the local port actually used.
    pub fn connect(&mut self, serial: &str, port: Option<u16>) -> Result<(AgentClient, u16)> {
        self.connect_socket(serial, port, AGENT_SOCKET)
    }

    /// Same as [`connect`](Self::connect) but against a non-default socket
    /// name (interop with agent builds that deviate from docs/03).
    pub fn connect_socket(
        &mut self,
        serial: &str,
        port: Option<u16>,
        socket: &str,
    ) -> Result<(AgentClient, u16)> {
        let port = match port {
            Some(p) => p,
            None => Self::free_port()?,
        };
        self.adb.forward(
            serial,
            &format!("tcp:{port}"),
            &format!("localabstract:{socket}"),
        )?;
        let client = AgentClient::connect_timeout(agent_addr(port), DEFAULT_TIMEOUT)?;
        Ok((client, port))
    }

    fn run_shell(&mut self, serial: &str, cmd: &str) -> Result<()> {
        let out = self.adb.shell(serial, cmd)?;
        if out.exit_code != Some(0) {
            return Err(AgentError::Shell {
                code: out.exit_code,
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            });
        }
        Ok(())
    }

    /// Pick a free localhost port for the forward.
    fn free_port() -> std::io::Result<u16> {
        let l = std::net::TcpListener::bind(agent_addr(0))?;
        Ok(l.local_addr()?.port())
    }
}

pub(crate) fn agent_addr(port: u16) -> SocketAddr {
    SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port)
}
