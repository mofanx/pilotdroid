//! CLI-facing agent operations — thin wrappers over [`AgentClient`] so the
//! dispatch surface is unit-testable against a fake agent without a device.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use pilotdroid_adb::AdbTransport;
use pilotdroid_agent::{AgentClient, AgentManager};
use pilotdroid_proto::payload::*;

/// forward localabstract → tcp, connect, HELLO handshake.
pub fn connect(
    adb: &mut impl AdbTransport,
    serial: &str,
    port: Option<u16>,
    socket: &str,
) -> Result<(AgentClient, u16)> {
    let mut mgr = AgentManager::new(adb);
    let (mut client, port) = mgr
        .connect_socket(serial, port, socket)
        .context("connecting to agent")?;
    // HELLO failure (dead/mismatched agent) must not leave the forward rule.
    let hello = match client.hello() {
        Ok(h) => h,
        Err(e) => {
            let _ = mgr.disconnect(serial, port);
            return Err(e).context("HELLO handshake");
        }
    };
    eprintln!(
        "agent {} (caps 0x{:08x}) via tcp:{port}",
        hello.agent, hello.caps
    );
    Ok((client, port))
}

/// Watchdog variant of [`connect`] (M2): on connect/HELLO failure run the
/// recovery ladder — reinstall `jar` + relaunch — then retry once.
pub fn connect_or_recover(
    adb: &mut impl AdbTransport,
    serial: &str,
    port: Option<u16>,
    socket: &str,
    jar: &Path,
) -> Result<(AgentClient, u16)> {
    match AgentManager::new(adb).connect_recover(serial, port, socket, jar) {
        Ok((client, port, hello)) => {
            eprintln!(
                "agent {} (caps 0x{:08x}) via tcp:{port}",
                hello.agent, hello.caps
            );
            Ok((client, port))
        }
        Err(e) => Err(e).context("agent watchdog recovery"),
    }
}

/// Remove the forward created by [`connect`].
pub fn disconnect(adb: &mut impl AdbTransport, serial: &str, port: u16) -> Result<()> {
    AgentManager::new(adb)
        .disconnect(serial, port)
        .context("removing forward")
}

pub fn tap(
    client: &mut AgentClient,
    x: f32,
    y: f32,
    wait_finish: bool,
) -> Result<(InjectAck, f64)> {
    let t = Instant::now();
    let ack = client.tap(x, y, wait_finish).context("INJECT")?;
    Ok((ack, t.elapsed().as_secs_f64() * 1e3))
}

pub fn dump(client: &mut AgentClient) -> Result<(String, f64)> {
    let t = Instant::now();
    let xml = client.dump().context("DUMP")?;
    Ok((xml, t.elapsed().as_secs_f64() * 1e3))
}

pub fn shot(
    client: &mut AgentClient,
    format: &str,
    quality: u8,
    scale: Option<f32>,
) -> Result<(String, Vec<u8>, f64)> {
    let t = Instant::now();
    let (fmt, bytes) = client
        .screenshot(format, quality, scale)
        .context("SCREENSHOT")?;
    Ok((fmt, bytes, t.elapsed().as_secs_f64() * 1e3))
}

/// Op kinds the session-mode `agent bench` can measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchKind {
    Ping,
    Tap,
    Dump,
    Shot,
}

/// Session-mode benchmark: N iterations of one op over a single reused
/// connection — the amortized latency that per-invocation CLI runs cannot
/// see (spawn + forward + HELLO ≈ +60ms per call).
/// Returns per-op wall ms in issue order.
pub fn bench(
    client: &mut AgentClient,
    kind: BenchKind,
    n: u32,
    x: f32,
    y: f32,
    noack: bool,
) -> Result<Vec<f64>> {
    let mut out = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let t = Instant::now();
        match kind {
            BenchKind::Ping => client.ping().context("PING")?,
            BenchKind::Tap if noack => client.tap_noack(x, y).context("INJECT/NOACK")?,
            BenchKind::Tap => {
                client.tap(x, y, true).context("INJECT")?;
            }
            BenchKind::Dump => {
                client.dump().context("DUMP")?;
            }
            BenchKind::Shot => {
                client.screenshot("jpeg", 80, None).context("SCREENSHOT")?;
            }
        }
        out.push(t.elapsed().as_secs_f64() * 1e3);
    }
    Ok(out)
}

/// Percentile of a sample (nearest-rank on a sorted copy).
pub fn percentile(samples: &[f64], p: f64) -> f64 {
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((p / 100.0) * (s.len() - 1) as f64).round() as usize;
    s[i]
}
