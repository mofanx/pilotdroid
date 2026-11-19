mod agent;

#[cfg(test)]
mod agent_smoke;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use pilotdroid_adb::{AdbClient, AdbTransport};
use pilotdroid_agent::AgentManager;

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
        /// Local tcp port for the forward (default: auto-pick a free one).
        #[arg(long)]
        port: Option<u16>,
        /// Agent's localabstract socket name.
        #[arg(long, default_value = "pilotdroid_agent")]
        socket: String,
    },
    /// Dump the UI hierarchy XML through the agent.
    Dump {
        /// Local tcp port for the forward (default: auto-pick).
        #[arg(long)]
        port: Option<u16>,
        /// Agent's localabstract socket name.
        #[arg(long, default_value = "pilotdroid_agent")]
        socket: String,
    },
    /// Take a screenshot through the agent and save it.
    Shot {
        /// Output path for the image.
        #[arg(short, long, default_value = "pilotdroid-shot.jpg")]
        output: PathBuf,
        /// JPEG quality 1-100.
        #[arg(long, default_value_t = 80)]
        quality: u8,
        /// Local tcp port for the forward (default: auto-pick).
        #[arg(long)]
        port: Option<u16>,
        /// Agent's localabstract socket name.
        #[arg(long, default_value = "pilotdroid_agent")]
        socket: String,
    },
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
                AgentCmd::Tap {
                    x,
                    y,
                    nowait,
                    port,
                    socket,
                } => {
                    let (mut client, port) = agent::connect(&mut adb, &serial, *port, socket)?;
                    let res = agent::tap(&mut client, *x, *y, !nowait);
                    let disc = agent::disconnect(&mut adb, &serial, port);
                    let (ack, ms) = res?;
                    disc?;
                    println!(
                        "{{\"op\":\"tap\",\"x\":{x},\"y\":{y},\"ok\":{},\"events\":{},\"device_ms\":{},\"rtt_ms\":{ms:.1}}}",
                        ack.ok, ack.events, ack.ms
                    );
                }
                AgentCmd::Dump { port, socket } => {
                    let (mut client, port) = agent::connect(&mut adb, &serial, *port, socket)?;
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
                    port,
                    socket,
                } => {
                    let (mut client, port) = agent::connect(&mut adb, &serial, *port, socket)?;
                    let res = agent::shot(&mut client, "jpeg", *quality);
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
