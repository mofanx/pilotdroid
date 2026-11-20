//! `pilotdroid setup` — five-step doctor for a bare checkout:
//!
//! 1. `adb` binary on PATH;
//! 2. target device visible and `ready` via `adb devices`;
//! 3. agent.jar present on-device (pushed when `--jar` is supplied);
//! 4. `adb forward` + TCP dial to the agent socket;
//! 5. HELLO handshake — report agent `ver` + `caps`.
//!
//! Every failed step prints one actionable hint. Exit status is 0 iff
//! all five pass — CI can gate on the exit code, humans read stdout.

use std::path::PathBuf;

use anyhow::{bail, Result};
use pilotdroid_adb::AdbTransport;
use pilotdroid_agent::AgentManager;

struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
    hint: Option<&'static str>,
}

impl Check {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
            hint: None,
        }
    }
    fn fail(name: &'static str, detail: impl Into<String>, hint: &'static str) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
            hint: Some(hint),
        }
    }
}

fn report(checks: &[Check]) {
    for c in checks {
        println!(
            "{} {} — {}",
            if c.ok { "PASS" } else { "FAIL" },
            c.name,
            c.detail
        );
        if let Some(h) = c.hint {
            println!("  → {h}");
        }
    }
}

/// Try to wake the adb server, then connect. `None` = adb server
/// unreachable even after `adb start-server`.
pub fn adb_transport_or_start_server(adb_port: u16) -> Option<pilotdroid_adb::AdbClient> {
    if let Ok(a) = pilotdroid_adb::AdbClient::at(adb_port) {
        return Some(a);
    }
    let _ = std::process::Command::new("adb")
        .arg("start-server")
        .output();
    pilotdroid_adb::AdbClient::at(adb_port).ok()
}

/// Degraded report when the adb server itself is unreachable — steps
/// 2+ can't run without a transport.
pub fn run_unreachable(adb_bin_ok: bool) -> Result<()> {
    report(&[
        if adb_bin_ok {
            Check::pass("adb_on_path", "`adb version` exited 0")
        } else {
            Check::fail(
                "adb_on_path",
                "adb not found on PATH or exited non-zero",
                "install Android platform-tools (apt install adb / sdkmanager) and re-run",
            )
        },
        Check::fail(
            "device_ready",
            "adb server unreachable even after `adb start-server`",
            "check the adb server port / restart adb",
        ),
    ]);
    bail!("setup: adb server unreachable");
}

/// Run the doctor. `serial_opt` pins the device (`-s`); `None` requires
/// exactly one ready device. `jar` is an optional local agent.jar used
/// to push when the remote copy is missing.
pub fn run<T: AdbTransport>(
    mut adb: T,
    serial_opt: Option<String>,
    jar: Option<PathBuf>,
    port: Option<u16>,
    socket: &str,
) -> Result<()> {
    let mut checks = Vec::new();

    // 1. adb binary on PATH (the AdbTransport talks to an already-running
    // adb server — this check is about the CLI binary being installed).
    let adb_ok = std::process::Command::new("adb")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    checks.push(if adb_ok {
        Check::pass("adb_on_path", "`adb version` exited 0")
    } else {
        Check::fail(
            "adb_on_path",
            "adb not found on PATH or exited non-zero",
            "install Android platform-tools (apt install adb / sdkmanager) and re-run",
        )
    });

    // 2. device visible + ready.
    let serial = match adb.list_devices() {
        Ok(devs) => {
            let ready: Vec<_> = devs.into_iter().filter(|d| d.is_ready()).collect();
            match serial_opt.as_deref() {
                Some(s) if ready.iter().any(|d| d.serial == s) => {
                    checks.push(Check::pass("device_ready", format!("{s} ready")));
                    Some(s.to_string())
                }
                Some(s) => {
                    checks.push(Check::fail(
                        "device_ready",
                        format!("{s} not in ready devices"),
                        "check cable/`adb devices` — authorize the host or `adb connect <ip>:<port>`",
                    ));
                    None
                }
                None if ready.len() == 1 => {
                    let s = ready[0].serial.clone();
                    checks.push(Check::pass("device_ready", format!("{s} ready")));
                    Some(s)
                }
                None => {
                    checks.push(Check::fail(
                        "device_ready",
                        format!("{} ready device(s)", ready.len()),
                        "plug exactly one device or pass -s <serial>",
                    ));
                    None
                }
            }
        }
        Err(e) => {
            checks.push(Check::fail(
                "device_ready",
                format!("list_devices failed: {e}"),
                "is the adb server reachable? try `adb devices` / `adb start-server`",
            ));
            None
        }
    };
    let Some(serial) = serial else {
        report(&checks);
        bail!("setup: 3 dependent step(s) skipped — fix the failed checks first");
    };

    let mut mgr = AgentManager::new(&mut adb);

    // 3. agent.jar on-device (push when --jar supplied and remote absent).
    let remote_ok = mgr.remote_jar_exists(&serial).unwrap_or(false);
    let jar_ok = if remote_ok {
        true
    } else if let Some(j) = jar.as_deref() {
        mgr.install(&serial, j).is_ok()
    } else {
        false
    };
    checks.push(if jar_ok {
        Check::pass("agent_jar", format!("remote jar present on {serial}"))
    } else {
        Check::fail(
            "agent_jar",
            "remote jar missing (or push failed)",
            "pass --jar <path>/agent.jar or run `pilotdroid agent install <jar>`",
        )
    });

    // 4. forward + dial.
    let dialed = mgr.connect_socket(&serial, port, socket);
    let (mut client, used_port) = match dialed {
        Ok((c, p)) => {
            checks.push(Check::pass(
                "forward_dial",
                format!("tcp:{p} → localabstract:{socket} dialed"),
            ));
            (Some(c), p)
        }
        Err(e) => {
            checks.push(Check::fail(
                "forward_dial",
                format!("forward/dial failed: {e}"),
                "agent daemon may be down — `pilotdroid agent start`, or check the socket name",
            ));
            (None, 0)
        }
    };

    // 5. HELLO.
    if let Some(c) = client.as_mut() {
        match c.hello() {
            Ok(h) => checks.push(Check::pass(
                "hello",
                format!("ver={:?} caps=0x{:x}", h.ver, h.caps),
            )),
            Err(e) => checks.push(Check::fail(
                "hello",
                format!("HELLO failed: {e}"),
                "jar/socket mismatch? deploy the matching jar via `agent install`",
            )),
        }
    } else {
        checks.push(Check::fail(
            "hello",
            "skipped — dial failed",
            "fix step 4 first",
        ));
    }

    // Don't leak the forward rule from the check.
    if client.is_some() {
        let _ = mgr.disconnect(&serial, used_port);
    }

    report(&checks);
    if checks.iter().all(|c| c.ok) {
        Ok(())
    } else {
        bail!(
            "setup: {} check(s) failed",
            checks.iter().filter(|c| !c.ok).count()
        );
    }
}
