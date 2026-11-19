//! CLI-layer smoke: drive agent::connect + tap/dump/shot against a std
//! TcpListener fake agent — no tokio, no device.

use std::io;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use pilotdroid_adb::{AdbTransport, DeviceInfo, Result as AdbResult, ShellOutput};
use pilotdroid_proto::payload::*;
use pilotdroid_proto::*;

use crate::agent;

// ---- std fake agent --------------------------------------------------------

fn spawn_fake() -> (SocketAddr, mpsc::Receiver<u16>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((mut sock, _)) = listener.accept() {
            while let Ok(req) = Frame::read_from(&mut sock) {
                let ty = req.header.msg_type;
                let (resp_ty, payload) = match OpCode::try_from(ty) {
                    Ok(OpCode::Hello) => (
                        OpCode::HelloAck as u16,
                        encode(&HelloAck {
                            agent: "fake/0".into(),
                            caps: caps::DUMP,
                            ver: None,
                            proto: None,
                            sdk: None,
                            abi: None,
                            display: None,
                            inject: None,
                        })
                        .unwrap(),
                    ),
                    Ok(OpCode::Inject) => (
                        OpCode::InjectResult as u16,
                        encode(&InjectAck {
                            ok: true,
                            events: 2,
                            ms: 1.5,
                            cost_ns: None,
                            backend: None,
                        })
                        .unwrap(),
                    ),
                    Ok(OpCode::Dump) => (
                        OpCode::Dump as u16,
                        encode(&DumpAck {
                            xml: "<hierarchy/>".into(),
                        })
                        .unwrap(),
                    ),
                    Ok(OpCode::Screenshot) => (
                        OpCode::Screenshot as u16,
                        encode(&ShotAck {
                            format: "jpeg".into(),
                            data: vec![0xFF, 0xD8].into(),
                            w: None,
                            h: None,
                            scale: None,
                            take_ns: None,
                            enc_ns: None,
                        })
                        .unwrap(),
                    ),
                    _ => {
                        let p = encode(&ErrorAck {
                            code: "UNSUPPORTED".into(),
                            message: "no".into(),
                        })
                        .unwrap();
                        let f = Frame::new(
                            req.header.channel,
                            ty,
                            flags::RESPONSE | flags::ERROR,
                            req.header.req_id,
                            p,
                        );
                        let _ = f.write_to(&mut sock);
                        let _ = tx.send(ty);
                        continue;
                    }
                };
                if Frame::new(
                    req.header.channel,
                    resp_ty,
                    flags::RESPONSE,
                    req.header.req_id,
                    payload,
                )
                .write_to(&mut sock)
                .is_err()
                {
                    break;
                }
                let _ = tx.send(ty);
            }
        }
    });
    (addr, rx)
}

struct NoopAdb;
impl AdbTransport for NoopAdb {
    fn list_devices(&mut self) -> AdbResult<Vec<DeviceInfo>> {
        Ok(vec![])
    }
    fn shell(&mut self, _s: &str, _c: &str) -> AdbResult<ShellOutput> {
        Ok(ShellOutput {
            stdout: vec![],
            stderr: vec![],
            exit_code: Some(0),
        })
    }
    fn push(&mut self, _s: &str, _l: &Path, _r: &str) -> AdbResult<()> {
        Ok(())
    }
    fn pull(&mut self, _s: &str, _r: &str, _l: &Path) -> AdbResult<()> {
        Ok(())
    }
    fn forward(&mut self, _s: &str, _l: &str, _r: &str) -> AdbResult<()> {
        Ok(())
    }
    fn forward_remove(&mut self, _s: &str, _l: &str) -> AdbResult<()> {
        Ok(())
    }
}

// io::Read/Write adapters for Frame::{read_from,write_to} are satisfied by
// &mut TcpStream already.

#[test]
fn cli_agent_ops_against_fake() -> io::Result<()> {
    let (addr, seen) = spawn_fake();
    let mut adb = NoopAdb;
    let (mut client, port) =
        agent::connect_or_recover(&mut adb, "fake0", Some(addr.port()), "pilotdroid", None)
            .unwrap();
    assert_eq!(port, addr.port());

    let (ack, _ms) = agent::tap(&mut client, 700.0, 1600.0, true).unwrap();
    assert!(ack.ok && ack.events == 2);

    let (xml, _ms) = agent::dump(&mut client).unwrap();
    assert_eq!(xml, "<hierarchy/>");

    let (fmt, bytes, _ms) = agent::shot(&mut client, "jpeg", 80, None).unwrap();
    assert_eq!(fmt, "jpeg");
    assert_eq!(bytes, vec![0xFF, 0xD8]);

    // fake saw HELLO, INJECT, DUMP, SCREENSHOT in order.
    let ops: Vec<u16> = [
        seen.recv_timeout(Duration::from_secs(2)).unwrap(),
        seen.recv_timeout(Duration::from_secs(2)).unwrap(),
        seen.recv_timeout(Duration::from_secs(2)).unwrap(),
        seen.recv_timeout(Duration::from_secs(2)).unwrap(),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        ops,
        vec![
            OpCode::Hello as u16,
            OpCode::Inject as u16,
            OpCode::Dump as u16,
            OpCode::Screenshot as u16
        ]
    );
    Ok(())
}
