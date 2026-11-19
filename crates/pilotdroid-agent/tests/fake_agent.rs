//! Loopback integration tests: a tokio-TcpListener fake agent speaking the
//! 18-byte frame protocol, driving [`AgentClient`] end-to-end with no device.
//! Also covers [`AgentManager`] command generation via a stub AdbTransport.

use std::net::SocketAddr;
use std::path::Path;

use std::time::Duration;

use pilotdroid_adb::{AdbTransport, DeviceInfo, Result as AdbResult, ShellOutput};
use pilotdroid_agent::{AgentClient, AgentError, AgentManager, AGENT_JAR_REMOTE, AGENT_MAIN_CLASS};
use pilotdroid_proto::payload::*;
use pilotdroid_proto::*;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Spawn a fake agent. `behavior` selects how it answers requests.
async fn spawn_fake(behavior: FakeBehavior) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(v) => v,
                Err(_) => return,
            };
            match behavior {
                FakeBehavior::Responsive => {
                    tokio::spawn(async move {
                        while let Some(frame) = read_frame(&mut sock).await {
                            let resp = dispatch_fake(&frame);
                            if sock.write_to_async(&resp).await.is_err() {
                                break;
                            }
                        }
                    });
                }
                FakeBehavior::Silent => {
                    // Accept and hang — used by the timeout test.
                    std::mem::forget(sock);
                }
            }
        }
    });
    addr
}

#[derive(Clone, Copy)]
enum FakeBehavior {
    Responsive,
    Silent,
}

// ---- fake-agent wire side (tokio) ------------------------------------------

async fn read_frame(sock: &mut tokio::net::TcpStream) -> Option<Frame> {
    let mut hdr_buf = [0u8; HEADER_LEN];
    sock.read_exact(&mut hdr_buf).await.ok()?;
    let header = decode_header_bytes(&hdr_buf).ok()?;
    let mut payload = vec![0u8; header.payload_len as usize];
    sock.read_exact(&mut payload).await.ok()?;
    Some(Frame { header, payload })
}

trait WriteFrame {
    async fn write_to_async(&mut self, f: &Frame) -> std::io::Result<()>;
}

impl WriteFrame for tokio::net::TcpStream {
    async fn write_to_async(&mut self, f: &Frame) -> std::io::Result<()> {
        let mut buf = Vec::new();
        f.write_to(&mut buf)?;
        self.write_all(&buf).await
    }
}

fn respond(req: &Frame, ty: u16, payload: Vec<u8>) -> Frame {
    Frame::new(
        req.header.channel,
        ty,
        flags::RESPONSE,
        req.header.req_id,
        payload,
    )
}

fn dispatch_fake(req: &Frame) -> Frame {
    let Ok(op) = OpCode::try_from(req.header.msg_type) else {
        let p = encode(&ErrorAck {
            code: "BAD_OPCODE".into(),
            message: format!("unknown 0x{:04x}", req.header.msg_type),
        })
        .unwrap();
        return Frame::new(
            req.header.channel,
            req.header.msg_type,
            flags::RESPONSE | flags::ERROR,
            req.header.req_id,
            p,
        );
    };
    match op {
        OpCode::Hello => respond(
            req,
            OpCode::HelloAck as u16,
            encode(&HelloAck {
                agent: "fake-agent/0.1".into(),
                caps: caps::INJECT | caps::DUMP | caps::SHOT,
            })
            .unwrap(),
        ),
        OpCode::Inject => {
            let r: InjectReq = decode(&req.payload).unwrap();
            respond(
                req,
                OpCode::InjectResult as u16,
                encode(&InjectAck {
                    ok: true,
                    events: r.points.len() as u32,
                    ms: 2.5,
                })
                .unwrap(),
            )
        }
        OpCode::Dump => respond(
            req,
            OpCode::Dump as u16,
            encode(&DumpAck {
                xml: r#"<?xml version="1.0"?><hierarchy><node text="fake"/></hierarchy>"#.into(),
            })
            .unwrap(),
        ),
        OpCode::Screenshot => respond(
            req,
            OpCode::Screenshot as u16,
            encode(&ShotAck {
                format: "jpeg".into(),
                data: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00].into(),
            })
            .unwrap(),
        ),
        _ => {
            let p = encode(&ErrorAck {
                code: "UNSUPPORTED".into(),
                message: format!("{op:?} not implemented on fake"),
            })
            .unwrap();
            Frame::new(
                req.header.channel,
                req.header.msg_type,
                flags::RESPONSE | flags::ERROR,
                req.header.req_id,
                p,
            )
        }
    }
}

// ---- client tests -----------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_full_rpc_path() {
    let addr = spawn_fake(FakeBehavior::Responsive).await;
    tokio::time::sleep(Duration::from_millis(30)).await;

    let mut c = AgentClient::connect_timeout(addr, Duration::from_secs(2)).unwrap();

    let ack = c.hello().unwrap();
    assert_eq!(ack.agent, "fake-agent/0.1");
    assert!(ack.caps & caps::DUMP != 0);

    let inj = c.tap(700.0, 1600.0, true).unwrap();
    assert!(inj.ok);
    assert_eq!(inj.events, 2); // down + up

    let xml = c.dump().unwrap();
    assert!(xml.contains("<hierarchy>"));

    let (fmt, bytes) = c.screenshot("jpeg", 80).unwrap();
    assert_eq!(fmt, "jpeg");
    assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_maps_agent_error_frame() {
    let addr = spawn_fake(FakeBehavior::Responsive).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut c = AgentClient::connect_timeout(addr, Duration::from_secs(2)).unwrap();

    // QUERY is not implemented on the fake -> ERROR-flagged response.
    let err = c.call(OpCode::Query, Vec::new()).unwrap_err();
    match err {
        AgentError::Remote { code, .. } => assert_eq!(code, "UNSUPPORTED"),
        other => panic!("expected Remote, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_times_out_when_agent_silent() {
    let addr = spawn_fake(FakeBehavior::Silent).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut c = AgentClient::connect_timeout(addr, Duration::from_millis(300)).unwrap();
    let err = c.hello().unwrap_err();
    assert!(matches!(err, AgentError::Timeout), "got {err:?}");
}

// ---- manager tests (no device needed) ---------------------------------------

/// Stub AdbTransport recording every call — verifies the manager's command
/// surface without a device.
#[derive(Default)]
struct FakeAdb {
    shells: Vec<(String, String)>,
    pushes: Vec<(String, String, String)>,
    forwards: Vec<(String, String, String)>,
    /// Append-only log of forward() calls (removed entries stay here so
    /// tests can assert the forward spec even after forward_remove).
    forward_calls: Vec<(String, String, String)>,
}

impl AdbTransport for FakeAdb {
    fn list_devices(&mut self) -> AdbResult<Vec<DeviceInfo>> {
        Ok(vec![DeviceInfo {
            serial: "fake0".into(),
            state: "device".into(),
        }])
    }
    fn shell(&mut self, serial: &str, command: &str) -> AdbResult<ShellOutput> {
        self.shells.push((serial.into(), command.into()));
        Ok(ShellOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: Some(0),
        })
    }
    fn push(&mut self, serial: &str, local: &Path, remote: &str) -> AdbResult<()> {
        self.pushes
            .push((serial.into(), local.display().to_string(), remote.into()));
        Ok(())
    }
    fn pull(&mut self, _s: &str, _r: &str, _l: &Path) -> AdbResult<()> {
        Ok(())
    }
    fn forward(&mut self, serial: &str, local: &str, remote: &str) -> AdbResult<()> {
        let rec = (serial.into(), local.into(), remote.into());
        self.forward_calls.push(rec.clone());
        self.forwards.push(rec);
        Ok(())
    }
    fn forward_remove(&mut self, _s: &str, local: &str) -> AdbResult<()> {
        self.forwards.retain(|(_, l, _)| l != local);
        Ok(())
    }
}

#[test]
fn manager_emits_expected_command_surface() {
    let mut adb = FakeAdb::default();
    {
        let mut m = AgentManager::new(&mut adb);
        m.install("fake0", Path::new("/tmp/agent.jar")).unwrap();
        m.start("fake0").unwrap();
        m.stop("fake0").unwrap();
    }

    let shells: Vec<&str> = adb.shells.iter().map(|(_, c)| c.as_str()).collect();
    assert!(shells.iter().any(|c| c.contains("mkdir -p")), "{shells:?}");
    assert!(
        shells.iter().any(|c| c.contains("setsid")
            && c.contains("app_process")
            && c.contains(AGENT_MAIN_CLASS)),
        "{shells:?}"
    );
    assert!(shells.iter().any(|c| c.contains("pkill")), "{shells:?}");

    assert_eq!(adb.pushes.len(), 1);
    assert_eq!(adb.pushes[0].2, AGENT_JAR_REMOTE);
}

#[test]
fn manager_connect_forwards_to_localabstract() {
    // Manager.connect must forward tcp:<free> -> localabstract:pilotdroid then
    // dial it — we can't complete a real client connect on the fake Adb
    // (nothing listens), so assert the forward call shape + failure mapping.
    let mut adb = FakeAdb::default();
    let mut m = AgentManager::new(&mut adb);
    let res = m.connect("fake0", Some(19731));
    assert!(res.is_err()); // nothing listening on 127.0.0.1:19731
    assert_eq!(adb.forward_calls.len(), 1);
    assert_eq!(adb.forward_calls[0].1, "tcp:19731");
    assert_eq!(adb.forward_calls[0].2, "localabstract:pilotdroid_agent");
    // ...but a failed dial must clean the rule up again.
    assert!(adb.forwards.is_empty(), "leaked: {:?}", adb.forwards);
}

#[test]
fn repeated_connect_disconnect_leaves_no_forward_rules() {
    // t4 review found ~95 leaked `adb forward` rules on the server — every
    // connect() must be paired with disconnect().
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        while let Ok((s, _)) = listener.accept() {
            drop(s);
        }
    });

    let mut adb = FakeAdb::default();
    {
        let mut m = AgentManager::new(&mut adb);
        for _ in 0..5 {
            let (_client, port) = m.connect("fake0", Some(addr.port())).unwrap();
            m.disconnect("fake0", port).unwrap();
        }
    }
    assert!(adb.forwards.is_empty(), "leaked rules: {:?}", adb.forwards);
}
