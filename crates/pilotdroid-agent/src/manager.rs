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
pub const AGENT_SOCKET: &str = "pilotdroid_agent";

const AGENT_LOG: &str = "/data/local/tmp/pilotdroid/agent.log";

/// Settle delay after `agent start` — app_process needs a moment to bind
/// the localabstract socket before the forward dial can succeed.
const RECOVER_SETTLE: std::time::Duration = std::time::Duration::from_millis(800);

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
    /// The `[p]` glob avoids pkill matching this very `sh -c` cmdline
    /// (t4 review: plain `-f pilotdroid.Agent` self-kills → exit 143).
    pub fn stop(&mut self, serial: &str) -> Result<()> {
        self.run_shell(serial, "pkill -f '[p]ilotdroid.Agent' || true")
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
        // Dial failure must not leak the forward rule — remove it on the
        // error path (the rule outlives the failed socket otherwise).
        let client = match AgentClient::connect_timeout(agent_addr(port), DEFAULT_TIMEOUT) {
            Ok(c) => c,
            Err(e) => {
                let _ = self.adb.forward_remove(serial, &format!("tcp:{port}"));
                return Err(e);
            }
        };
        Ok((client, port))
    }

    /// Remove the `tcp:{port}` forward created by
    /// [`connect`](Self::connect). Callers must pair every successful
    /// connect with a disconnect — adb-server forward rules outlive the
    /// client socket (t4 review found ~95 leaked rules after a bench run).
    pub fn disconnect(&mut self, serial: &str, port: u16) -> Result<()> {
        Ok(self.adb.forward_remove(serial, &format!("tcp:{port}"))?)
    }

    /// Watchdog connect (M2): forward + dial + HELLO; on any failure run the
    /// recovery ladder — stop (best effort), reinstall `jar`, relaunch,
    /// settle — and retry the whole sequence **once**. Returns the client,
    /// its port, and the HELLO ack of whichever attempt succeeded.
    pub fn connect_recover(
        &mut self,
        serial: &str,
        port: Option<u16>,
        socket: &str,
        jar: &Path,
    ) -> Result<(AgentClient, u16, pilotdroid_proto::payload::HelloAck)> {
        match self.connect_hello(serial, port, socket) {
            ok @ Ok(_) => ok,
            Err(first) => {
                self.recover(serial, jar).map_err(|e| AgentError::Recover {
                    stage: "ladder",
                    detail: e.to_string(),
                })?;
                self.connect_hello(serial, port, socket)
                    .map_err(|second| AgentError::Recover {
                        stage: "retry",
                        detail: format!("first: {first}; after recover: {second}"),
                    })
            }
        }
    }

    /// Recovery ladder: stop any half-dead agent → reinstall `jar` → relaunch
    /// → settle while app_process binds localabstract. Best-effort stop: a
    /// dead/absent agent must not fail the ladder.
    pub fn recover(&mut self, serial: &str, jar: &Path) -> Result<()> {
        let _ = self.stop(serial);
        self.install(serial, jar)?;
        self.start(serial)?;
        std::thread::sleep(RECOVER_SETTLE);
        Ok(())
    }

    /// forward + dial + HELLO handshake (undoes the forward on any failure).
    fn connect_hello(
        &mut self,
        serial: &str,
        port: Option<u16>,
        socket: &str,
    ) -> Result<(AgentClient, u16, pilotdroid_proto::payload::HelloAck)> {
        let (mut client, port) = self.connect_socket(serial, port, socket)?;
        match client.hello() {
            Ok(ack) => Ok((client, port, ack)),
            Err(e) => {
                let _ = self.disconnect(serial, port);
                Err(e)
            }
        }
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
