mod agent;

#[cfg(test)]
mod agent_smoke;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use pilotdroid_adb::{AdbClient, AdbTransport};
use pilotdroid_agent::{AgentManager, AGENT_SOCKET};

/// pilotdroid — independent Android automation (docs/ for design).
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Device serial. Optional when exactly one device is connected.
    #[arg(short = 's', long, global = true)]
    device: Option<String>,

    /// adb server port (default 5037).
    #[arg(long, global = true, default_value_t = 5037)]
    adb_port: u16,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List connected devices (adb devices).
    Devices,

    /// Run a command through `adb shell`, print raw output.
    ///
    /// By default each argument is single-quoted before being sent, so
    /// `shell echo "a b"` reaches the device as `echo 'a b'` (exec-style
    /// argv semantics). Use `--raw` to hand the string to remote `sh -c`
    /// verbatim, e.g. `shell --raw 'echo a; echo b >&2'`.
    Shell {
        /// Pass the command line to remote `sh -c` verbatim instead of
        /// quoting each argument.
        #[arg(long)]
        raw: bool,
        /// Command line, e.g. `input tap 700 1600` or `getprop ro.build.version.sdk`.
        #[arg(required = true, num_args = 1..)]
        command: Vec<String>,
    },

    /// Push a local file to the device.
    Push {
        /// Local source path.
        local: PathBuf,
        /// Remote destination path on the device.
        remote: String,
    },

    /// Pull a file from the device.
    Pull {
        /// Remote source path on the device.
        remote: String,
        /// Local destination path.
        local: PathBuf,
    },

    /// Manage and use the on-device agent (dex daemon over localabstract).
    Agent {
        #[command(subcommand)]
        cmd: AgentCmd,
    },
}

/// Shared connect knobs for agent ops — single source for --port/--socket.
#[derive(Args)]
struct ConnectOpts {
    /// Local tcp port for the forward (default: auto-pick a free one).
    #[arg(long)]
    port: Option<u16>,
    /// Agent's localabstract socket name.
    #[arg(long, default_value = AGENT_SOCKET)]
    socket: String,
    /// Force reflash: during watchdog recovery, reinstall this agent.jar
    /// instead of relying on the on-device copy (M3 graded recovery —
    /// without it, a dead daemon is revived by zero-copy restart only).
    #[arg(long)]
    jar: Option<PathBuf>,
}

#[derive(Subcommand)]
enum AgentCmd {
    /// Push the agent jar to the device (/data/local/tmp/pilotdroid/agent.jar).
    Install {
        /// Local agent.jar (dex) built from agent/java.
        jar: PathBuf,
    },
    /// Launch the agent daemon (setsid + app_process, survives shell exit).
    Start,
    /// Kill the agent daemon (pkill by class name).
    Stop,
    /// Tap at (x, y) through the agent — forward + HELLO + INJECT.
    Tap {
        x: f32,
        y: f32,
        /// Don't wait for the inject to finish on the device (async).
        #[arg(long)]
        nowait: bool,
        #[command(flatten)]
        conn: ConnectOpts,
    },
    /// Dump the UI hierarchy XML through the agent.
    Dump {
        #[command(flatten)]
        conn: ConnectOpts,
    },
    /// Take a screenshot through the agent and save it.
    Shot {
        /// Output path for the image.
        #[arg(short, long, default_value = "pilotdroid-shot.jpg")]
        output: PathBuf,
        /// JPEG quality 1-100.
        #[arg(long, default_value_t = 80)]
        quality: u8,
        /// Downscale factor (0,1] applied on-device before encoding.
        #[arg(long)]
        scale: Option<f32>,
        #[command(flatten)]
        conn: ConnectOpts,
    },
    /// Session-mode bench: N iterations of one op over a single persistent
    /// connection — measures amortized latency (docs/02 §4.6 口径).
    Bench {
        /// Iterations.
        n: u32,
        /// Op to measure.
        #[arg(long, default_value = "tap")]
        op: BenchOp,
        /// Tap coordinates.
        #[arg(long, default_value_t = 700.0)]
        x: f32,
        #[arg(long, default_value_t = 1600.0)]
        y: f32,
        /// Fire-and-forget injects (NOACK flag, no response frame).
        #[arg(long)]
        noack: bool,
        #[command(flatten)]
        conn: ConnectOpts,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum BenchOp {
    Ping,
    Tap,
    Dump,
    Shot,
}

/// Resolve `--device`, falling back to the sole ready device.
fn resolve_serial(cli: &Cli, adb: &mut impl AdbTransport) -> Result<String> {
    if let Some(serial) = &cli.device {
        return Ok(serial.clone());
    }
    let ready: Vec<_> = adb
        .list_devices()?
        .into_iter()
        .filter(|d| d.is_ready())
        .collect();
    match ready.len() {
        0 => bail!("no device is ready (check `adb devices`)"),
        1 => Ok(ready[0].serial.clone()),
        _ => bail!(
            "multiple devices ready ({}) — pick one with -s <serial>",
            ready
                .iter()
                .map(|d| d.serial.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Connect through `ConnectOpts` — always the graded watchdog path: a dead
/// daemon is revived by zero-copy restart; `--jar` forces a reflash and is
/// required only when the remote jar itself is gone.
fn connect_opts(
    adb: &mut impl AdbTransport,
    serial: &str,
    conn: &ConnectOpts,
) -> Result<(pilotdroid_agent::AgentClient, u16)> {
    agent::connect_or_recover(adb, serial, conn.port, &conn.socket, conn.jar.as_deref())
}

/// POSIX single-quote an argument for remote `sh -c` parsing. Without this,
/// `shell echo "a b"` arrives as `echo a b` — the remote splits on spaces and
/// argv semantics are lost. `'` itself becomes `'\''`, safe for any arg.
fn sh_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn build_shell_command(args: &[String]) -> String {
    args.iter()
        .map(|a| sh_quote(a))
        .collect::<Vec<_>>()
        .join(" ")
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut adb =
        AdbClient::at(cli.adb_port).context("connecting to adb server (is `adb` running?)")?;

    match &cli.cmd {
        Cmd::Devices => {
            for d in adb.list_devices()? {
                println!("{}\t{}", d.serial, d.state);
            }
        }
        Cmd::Shell { command, raw } => {
            let serial = resolve_serial(&cli, &mut adb)?;
            let line = if *raw {
                command.join(" ")
            } else {
                build_shell_command(command)
            };
            let out = adb.shell(&serial, &line)?;
            print!("{}", String::from_utf8_lossy(&out.stdout));
            if !out.stderr.is_empty() {
                eprint!("{}", String::from_utf8_lossy(&out.stderr));
            }
            if let Some(code) = out.exit_code {
                std::process::exit(code);
            }
        }
        Cmd::Push { local, remote } => {
            let serial = resolve_serial(&cli, &mut adb)?;
            adb.push(&serial, local, remote)
                .with_context(|| format!("pushing {} -> {}", local.display(), remote))?;
            eprintln!("pushed {} -> {}:{}", local.display(), serial, remote);
        }
        Cmd::Pull { remote, local } => {
            let serial = resolve_serial(&cli, &mut adb)?;
            adb.pull(&serial, remote, local)
                .with_context(|| format!("pulling {} -> {}", remote, local.display()))?;
            eprintln!("pulled {}:{} -> {}", serial, remote, local.display());
        }
        Cmd::Agent { cmd } => {
            let serial = resolve_serial(&cli, &mut adb)?;
            match cmd {
                AgentCmd::Install { jar } => {
                    AgentManager::new(&mut adb).install(&serial, jar)?;
                    eprintln!(
                        "installed {} -> {}:{}",
                        jar.display(),
                        serial,
                        pilotdroid_agent::AGENT_JAR_REMOTE
                    );
                }
                AgentCmd::Start => {
                    AgentManager::new(&mut adb).start(&serial)?;
                    eprintln!("agent daemon launched on {serial} (log: /data/local/tmp/pilotdroid/agent.log)");
                }
                AgentCmd::Stop => {
                    AgentManager::new(&mut adb).stop(&serial)?;
                    eprintln!("agent daemon stopped on {serial}");
                }
                AgentCmd::Tap { x, y, nowait, conn } => {
                    let (mut client, port) = connect_opts(&mut adb, &serial, conn)?;
                    let res = agent::tap(&mut client, *x, *y, !nowait);
                    let disc = agent::disconnect(&mut adb, &serial, port);
                    let (ack, ms) = res?;
                    disc?;
                    println!(
                        "{{\"op\":\"tap\",\"x\":{x},\"y\":{y},\"ok\":{},\"events\":{},\"device_ms\":{},\"rtt_ms\":{ms:.1}}}",
                        ack.ok, ack.events, ack.ms
                    );
                }
                AgentCmd::Dump { conn } => {
                    let (mut client, port) = connect_opts(&mut adb, &serial, conn)?;
                    let res = agent::dump(&mut client);
                    let disc = agent::disconnect(&mut adb, &serial, port);
                    let (xml, ms) = res?;
                    disc?;
                    print!("{xml}");
                    eprintln!("dump rtt_ms={ms:.1}");
                }
                AgentCmd::Shot {
                    output,
                    quality,
                    scale,
                    conn,
                } => {
                    let (mut client, port) = connect_opts(&mut adb, &serial, conn)?;
                    let res = agent::shot(&mut client, "jpeg", *quality, *scale);
                    let disc = agent::disconnect(&mut adb, &serial, port);
                    let (fmt, bytes, ms) = res?;
                    disc?;
                    std::fs::write(output, &bytes)
                        .with_context(|| format!("writing {}", output.display()))?;
                    println!(
                        "{{\"op\":\"shot\",\"format\":\"{fmt}\",\"bytes\":{},\"rtt_ms\":{ms:.1},\"path\":\"{}\"}}",
                        bytes.len(),
                        output.display()
                    );
                }
                AgentCmd::Bench {
                    n,
                    op,
                    x,
                    y,
                    noack,
                    conn,
                } => {
                    let (mut client, port) = connect_opts(&mut adb, &serial, conn)?;
                    let kind = match op {
                        BenchOp::Ping => agent::BenchKind::Ping,
                        BenchOp::Tap => agent::BenchKind::Tap,
                        BenchOp::Dump => agent::BenchKind::Dump,
                        BenchOp::Shot => agent::BenchKind::Shot,
                    };
                    let res = agent::bench(&mut client, kind, *n, *x, *y, *noack);
                    let disc = agent::disconnect(&mut adb, &serial, port);
                    let ms = res?;
                    disc?;
                    let mean = ms.iter().sum::<f64>() / ms.len() as f64;
                    let list = ms
                        .iter()
                        .map(|v| format!("{v:.1}"))
                        .collect::<Vec<_>>()
                        .join(",");
                    println!(
                        "{{\"op\":\"{op:?}\",\"n\":{n},\"noack\":{noack},\"mean_ms\":{mean:.1},\"p50_ms\":{:.1},\"p95_ms\":{:.1},\"min_ms\":{:.1},\"ms\":[{list}]}}",
                        agent::percentile(&ms, 50.0),
                        agent::percentile(&ms, 95.0),
                        agent::percentile(&ms, 0.0),
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sh_quote_preserves_spaces_and_escapes_quote() {
        assert_eq!(sh_quote("hello world"), "'hello world'");
        assert_eq!(sh_quote("a'b"), "'a'\\''b'");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_quote("semi;rm -rf /"), "'semi;rm -rf /'");
    }

    #[test]
    fn build_command_quotes_each_arg() {
        // Regression (t4 review): multi-arg commands must not be re-split by
        // the remote shell. `printf '%s\n' "aa bb" c` must produce two lines.
        let cmd =
            build_shell_command(&["printf".into(), "%s\\n".into(), "aa bb".into(), "c".into()]);
        assert_eq!(cmd, "'printf' '%s\\n' 'aa bb' 'c'");
    }
}
