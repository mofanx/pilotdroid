//! M5 4-path bench — Rust facade path. Runs the shared scenario on ONE
//! persistent Session (amortized 口径) and prints per-op latency JSON.
//!
//!   cargo run -p pilotdroid-agent --example bench_session -- <serial> [n]

use std::time::Instant;

use pilotdroid_adb::AdbClient;
use pilotdroid_agent::Device;
use pilotdroid_proto::payload::Selector;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let serial = std::env::args().nth(1).unwrap_or_else(|| "ba0d072d".into());
    let n: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    let dev = Device::with_transport(AdbClient::new()?, serial);
    let mut s = dev.session()?;

    let mut tap_ms = Vec::new();
    let mut dump_ms = Vec::new();
    let mut shot_ms = Vec::new();
    let mut exists_ms = Vec::new();

    for _ in 0..n {
        let t = Instant::now();
        s.tap(700.0, 1500.0)?;
        tap_ms.push(t.elapsed().as_secs_f64() * 1000.0);

        let t = Instant::now();
        let _ = s.dump()?;
        dump_ms.push(t.elapsed().as_secs_f64() * 1000.0);

        let t = Instant::now();
        let _ = s.exists(Selector {
            class: Some("android.widget.TextView".into()),
            ..Default::default()
        })?;
        exists_ms.push(t.elapsed().as_secs_f64() * 1000.0);

        let t = Instant::now();
        let _ = s.screenshot(None, 70, Some(0.5))?;
        shot_ms.push(t.elapsed().as_secs_f64() * 1000.0);
    }

    let p50 = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (v[v.len() / 2] * 10.0).round() / 10.0
    };
    println!(
        "{{\"engine\":\"lib\",\"n\":{n},\"tap_ms\":{},\"dump_ms\":{},\"exists_ms\":{},\"shot_ms\":{}}}",
        p50(&mut tap_ms),
        p50(&mut dump_ms),
        p50(&mut exists_ms),
        p50(&mut shot_ms)
    );
    Ok(())
}
