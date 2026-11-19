//! CLI-facing agent operations — thin wrappers over [`AgentClient`] so the
//! dispatch surface is unit-testable against a fake agent without a device.

use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use pilotdroid_adb::AdbTransport;
use pilotdroid_agent::{AgentClient, AgentManager, MediaClient, StreamStats};
use pilotdroid_proto::payload::*;

/// Watchdog connect (graded recovery, M3): on connect/HELLO failure run the
/// graded recovery ladder — zero-copy restart first; reinstall only when
/// the remote jar is missing or `jar` forces a reflash (M3).
pub fn connect_or_recover(
    adb: &mut impl AdbTransport,
    serial: &str,
    port: Option<u16>,
    socket: &str,
    jar: Option<&Path>,
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

/// Tunables for `agent stream` (docs/02 §4.8 defaults live agent-side).
#[derive(Debug, Clone, Default)]
pub struct StreamOpts {
    pub fps: Option<u8>,
    pub scale: Option<f32>,
    pub quality: Option<u8>,
    pub duration: Duration,
    /// Write each received frame as `out/frame_{seq:05}.jpg`.
    pub out: Option<std::path::PathBuf>,
}

/// Outcome of one stream session — wire counters plus wall-clock rate.
#[derive(Debug)]
pub struct StreamReport {
    pub stats: StreamStats,
    pub elapsed: Duration,
    /// Agent-side totals from the stop ack.
    pub frames_sent: u64,
}

/// Receive loop until `duration` or a terminal frame (STREAM_MORE clear).
/// `out_dir` gets one JPEG per frame named by seq. Returns early on a
/// clean terminal frame; a mid-stream socket error aborts with the stats
/// collected so far still visible on `media.stats()`.
pub fn stream_collect(
    media: &mut MediaClient,
    duration: Duration,
    out_dir: Option<&Path>,
) -> Result<()> {
    let deadline = Instant::now() + duration;
    if let Some(dir) = out_dir {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    while !media.is_ended() && Instant::now() < deadline {
        let f = media.recv().context("media FRAME")?;
        if let Some(dir) = out_dir {
            let seq = media.last_seq().unwrap_or(0);
            std::fs::write(dir.join(format!("frame_{seq:05}.jpg")), &f.data)?;
        }
    }
    Ok(())
}

/// Full `agent stream` session: STREAM_CTRL start on the control client,
/// second forward to the media socket, collect for `opts.duration`,
/// STREAM_CTRL stop, media forward removed (control forward is the
/// caller's to pair with [`disconnect`]).
pub fn stream_session(
    adb: &mut impl AdbTransport,
    serial: &str,
    client: &mut AgentClient,
    opts: &StreamOpts,
) -> Result<StreamReport> {
    let start = client
        .stream_start(StreamCtrlReq {
            op: StreamOp::Start,
            fps: opts.fps,
            scale: opts.scale,
            quality: opts.quality,
            format: None,
        })
        .context("STREAM_CTRL start")?;
    let mut mgr = AgentManager::new(adb);
    let (mut media, media_port) = mgr
        .connect_media(serial, None, &start.socket)
        .context("media forward+dial")?;
    let t = Instant::now();
    let res = stream_collect(&mut media, opts.duration, opts.out.as_deref());
    let elapsed = t.elapsed();
    // Stop is best-effort: a dead agent must not swallow the collected
    // stats; session binding (§4.8) cleans up on disconnect anyway.
    let stop = client.stream_stop().unwrap_or(StreamStopAck {
        ok: false,
        frames_sent: 0,
        dropped: 0,
    });
    let _ = mgr.disconnect(serial, media_port);
    res?;
    let mut stats = media.stats().clone();
    stats.dropped = stats.dropped.max(stop.dropped);
    Ok(StreamReport {
        stats,
        elapsed,
        frames_sent: stop.frames_sent,
    })
}

/// Percentile of a sample (nearest-rank on a sorted copy).
pub fn percentile(samples: &[f64], p: f64) -> f64 {
    let mut s = samples.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((p / 100.0) * (s.len() - 1) as f64).round() as usize;
    s[i]
}
