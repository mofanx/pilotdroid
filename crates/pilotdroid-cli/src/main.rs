use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use pilotdroid_adb::{AdbClient, AdbTransport};

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
    Shell {
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
        Cmd::Shell { command } => {
            let serial = resolve_serial(&cli, &mut adb)?;
            let out = adb.shell(&serial, &command.join(" "))?;
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
    }
    Ok(())
}
