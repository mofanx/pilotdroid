//! CLI-facing agent operations — thin wrappers over [`AgentClient`] so the
//! dispatch surface is unit-testable against a fake agent without a device.

use std::time::Instant;

use anyhow::{Context, Result};
use pilotdroid_adb::AdbTransport;
use pilotdroid_agent::{AgentClient, AgentManager};
use pilotdroid_proto::payload::InjectAck;

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
    let hello = client.hello().context("HELLO handshake")?;
    eprintln!(
        "agent {} (caps 0x{:08x}) via tcp:{port}",
        hello.agent, hello.caps
    );
    Ok((client, port))
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

pub fn shot(client: &mut AgentClient, format: &str, quality: u8) -> Result<(String, Vec<u8>, f64)> {
    let t = Instant::now();
    let (fmt, bytes) = client.screenshot(format, quality).context("SCREENSHOT")?;
    Ok((fmt, bytes, t.elapsed().as_secs_f64() * 1e3))
}
