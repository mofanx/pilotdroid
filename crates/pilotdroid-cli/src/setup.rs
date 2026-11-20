//! `pilotdroid setup` — eight-step doctor for a bare checkout:
//!
//! 1. `adb` binary on PATH;
//! 2. target device visible and `ready` via `adb devices`;
//! 3. agent.jar present on-device (pushes `--jar`, else the embedded
//!    `AGENT_JAR_BYTES`, when the remote copy is missing);
//! 4. `adb forward` + TCP dial to the agent socket;
//! 5. HELLO handshake — report agent `ver` + `caps`;
//! 6. u2/atx conflict probe (running uiautomator2 holds the
//!    UiAutomation singleton → WARN);
//! 7. inject_probe — dead-zone tap + PING re-check, fail-fast when
//!    INJECT is revoked;
//! 8. forward hygiene — orphaned `pilotdroid_agent*` rules (WARN).
//!
//! Every failed step prints one actionable hint. Exit status is 0 iff
//! all checks pass — CI can gate on the exit code, humans read stdout.

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
    /// Advisory only — never fails the doctor (orphan forwards are
    /// harmless clutter, not a broken setup).
    fn warn(name: &'static str, detail: impl Into<String>, hint: &'static str) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
            hint: Some(hint),
        }
    }
}

fn report(checks: &[Check]) {
    for c in checks {
        println!(
            "{} {} — {}",
            if !c.ok {
                "FAIL"
            } else if c.hint.is_some() {
                "WARN"
            } else {
                "PASS"
            },
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

    // 3. agent.jar on-device — absent a remote copy we push --jar when
    // supplied, else the embedded AGENT_JAR_BYTES (one-command setup).
    let remote_ok = mgr.remote_jar_exists(&serial).unwrap_or(false);
    let pushed = if remote_ok {
        false
    } else {
        let local = match jar.as_deref() {
            Some(j) => Some(j.to_path_buf()),
            None => pilotdroid_agent::embedded_jar_path().ok(),
        };
        local.is_some_and(|j| mgr.install(&serial, &j).is_ok())
    };
    checks.push(if remote_ok || pushed {
        Check::pass(
            "agent_jar",
            if pushed {
                format!(
                    "pushed {} jar to {serial}",
                    if jar.is_some() { "--jar" } else { "embedded" }
                )
            } else {
                format!("remote jar present on {serial}")
            },
        )
    } else {
        Check::fail(
            "agent_jar",
            "remote jar missing and embedded push failed",
            "run `pilotdroid agent install` or check adb write access to /data/local/tmp",
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
                "daemon may have died mid-check — `agent start`; jar/socket mismatch? `agent install`",
            )),
        }
    } else {
        checks.push(Check::fail(
            "hello",
            "skipped — dial failed",
            "fix step 4 first",
        ));
    }

    // 6. u2/atx conflict probe: a running uiautomator2/atx stack holds the
    // UiAutomation singleton — our agent can dial + HELLO yet silently
    // fail to INJECT. WARN not FAIL: an installed-but-idle u2 harms
    // nothing, and even a live one only blocks *our* inject path.
    checks.push(u2_conflict(
        mgr.shell(&serial, "pm list packages"),
        mgr.shell(&serial, "ps -A"),
    ));

    // 7. inject_probe: a harmless tap in the status-bar dead zone + a PING
    // re-check — catches ROMs that grant UiAutomation but revoke INJECT
    // mid-flight (duchamp-class), which steps 4-5 cannot see.
    if let Some(c) = client.as_mut() {
        let probe = c
            .tap(5.0, 5.0, true)
            .map_err(|e| e.to_string())
            .and_then(|_| c.ping().map_err(|e| e.to_string()));
        checks.push(match probe {
            Ok(()) => Check::pass("inject_probe", "INJECT(5,5)+PING ok"),
            Err(e) => Check::fail(
                "inject_probe",
                format!("inject or post-inject ping failed: {e}"),
                "INJECT permission may be revoked — check UiAutomation/accessibility grants; on conflict see step u2_conflict",
            ),
        });
    } else {
        checks.push(Check::fail(
            "inject_probe",
            "skipped — dial failed",
            "fix step 4 first",
        ));
    }

    // Don't leak the forward rule from the check.
    if client.is_some() {
        let _ = mgr.disconnect(&serial, used_port);
    }

    // 8. forward hygiene: `pilotdroid_agent*` rules whose local port has
    // no established client conn are orphans (leaked by killed serve/
    // bench sessions — Drop never ran). List, never delete: a multi-serve
    // sibling's rule is also "orphan-looking" if its conn isn't active.
    // (M12: media attaches pin `control_port+1`, so repeated media
    // orphans collapse onto one predictable rule rather than a set.)
    checks.push(forward_hygiene());

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

/// u2/atx conflict probe. `pm_out` is the raw `pm list packages` stdout,
/// `ps_out` the raw `ps -A` stdout — both parsed host-side so transports
/// never need shell pipes.
fn u2_conflict(
    pm_out: std::result::Result<pilotdroid_adb::ShellOutput, pilotdroid_adb::AdbError>,
    ps_out: std::result::Result<pilotdroid_adb::ShellOutput, pilotdroid_adb::AdbError>,
) -> Check {
    let installed: Vec<String> = pm_out
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| l.contains("uiautomator") || l.contains("atx"))
                .map(|l| l.trim_start_matches("package:").to_string())
                .collect()
        })
        .unwrap_or_default();
    let running: Vec<String> = ps_out
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter(|l| {
                    (l.contains("uiautomator") || l.contains("atx")) && !l.contains("pilotdroid")
                })
                .map(|l| l.split_whitespace().last().unwrap_or("?").to_string())
                .collect()
        })
        .unwrap_or_default();
    if !running.is_empty() {
        Check::warn(
            "u2_conflict",
            format!("uiautomator/atx process(es) running: {}", running.join(", ")),
            "u2/atx holds the UiAutomation singleton — `am force-stop com.github.uiautomator` (or stop u2) before relying on INJECT",
        )
    } else if !installed.is_empty() {
        Check::pass(
            "u2_conflict",
            format!("u2/atx installed but idle ({})", installed.join(", ")),
        )
    } else {
        Check::pass("u2_conflict", "no uiautomator/atx packages")
    }
}

/// `adb forward --list` rows targeting `pilotdroid_agent*` →
/// (serial, local_port, remote).
fn parse_forward_rules(list: &str) -> Vec<(String, u16, String)> {
    list.lines()
        .filter(|l| l.contains("pilotdroid_agent"))
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let serial = f.next()?;
            let port = f.next()?.strip_prefix("tcp:")?.parse().ok()?;
            let remote = f.next()?;
            Some((serial.to_string(), port, remote.to_string()))
        })
        .collect()
}

/// Local TCP ports (decimal) that currently have an ESTABLISHED conn,
/// parsed from `ss -tn state established` output.
fn established_ports(ss_out: &str) -> std::collections::HashSet<u16> {
    ss_out
        .lines()
        .skip(1)
        .filter_map(|l| {
            // columns: State Recv-Q Send-Q Local:Port Peer:Port ...
            let local = l.split_whitespace().nth(3)?;
            local.rsplit(':').next()?.parse().ok()
        })
        .collect()
}

fn forward_hygiene() -> Check {
    let list = std::process::Command::new("adb")
        .args(["forward", "--list"])
        .output();
    let Ok(out) = list else {
        return Check::fail(
            "forward_hygiene",
            "adb forward --list failed",
            "adb server down?",
        );
    };
    let rules = parse_forward_rules(&String::from_utf8_lossy(&out.stdout));
    if rules.is_empty() {
        return Check::pass("forward_hygiene", "no pilotdroid forward rules");
    }
    let ss = std::process::Command::new("ss")
        .args(["-tn", "state", "established"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let active = established_ports(&ss);
    let orphans: Vec<String> = rules
        .iter()
        .filter(|(_, p, _)| !active.contains(p))
        .map(|(s, p, _)| format!("{s} tcp:{p}"))
        .collect();
    if orphans.is_empty() {
        Check::pass(
            "forward_hygiene",
            format!("{} rule(s), all with live conns", rules.len()),
        )
    } else {
        Check::warn(
            "forward_hygiene",
            format!(
                "{} orphan rule(s) of {}: {}",
                orphans.len(),
                rules.len(),
                orphans.join(" ")
            ),
            "killed sessions leak rules — `adb forward --remove tcp:<port>` (listed ones only; live conns are safe)",
        )
    }
}
