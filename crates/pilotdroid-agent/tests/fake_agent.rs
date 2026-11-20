//! Loopback integration tests: a tokio-TcpListener fake agent speaking the
//! 18-byte frame protocol, driving [`AgentClient`] end-to-end with no device.
//! Also covers [`AgentManager`] command generation via a stub AdbTransport.

use std::net::SocketAddr;
use std::path::Path;

use std::time::Duration;

use pilotdroid_adb::{AdbTransport, DeviceInfo, Result as AdbResult, ShellOutput};
use pilotdroid_agent::{
    AgentClient, AgentError, AgentManager, Device, MediaClient, AGENT_JAR_REMOTE, AGENT_MAIN_CLASS,
};
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
                            // NOACK (§4.5): execute but send no response frame.
                            if let Some(resp) = dispatch_fake(&frame) {
                                if sock.write_to_async(&resp).await.is_err() {
                                    break;
                                }
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

/// None → the fake sent no reply (NOACK requests per docs/02 §4.5).
fn dispatch_fake(req: &Frame) -> Option<Frame> {
    if req.header.flags & flags::NOACK != 0 {
        return None;
    }
    let Ok(op) = OpCode::try_from(req.header.msg_type) else {
        let p = encode(&ErrorAck {
            code: "BAD_OPCODE".into(),
            message: format!("unknown 0x{:04x}", req.header.msg_type),
        })
        .unwrap();
        return Some(Frame::new(
            req.header.channel,
            req.header.msg_type,
            flags::RESPONSE | flags::ERROR,
            req.header.req_id,
            p,
        ));
    };
    Some(match op {
        OpCode::Hello => respond(
            req,
            OpCode::HelloAck as u16,
            encode(&HelloAck {
                agent: "fake-agent/0.1".into(),
                caps: caps::INJECT
                    | caps::DUMP
                    | caps::SHOT
                    | caps::QUERY
                    | caps::BATCH
                    | caps::TEXT,
                ver: None,
                proto: None,
                sdk: None,
                abi: None,
                display: None,
                inject: None,
            })
            .unwrap(),
        ),
        OpCode::Ping => respond(req, OpCode::Pong as u16, Vec::new()),
        OpCode::Inject => {
            let r: InjectReq = decode(&req.payload).unwrap();
            respond(
                req,
                OpCode::InjectResult as u16,
                encode(&InjectAck {
                    ok: true,
                    events: if r.kind == InjectKind::Key {
                        2 // KeyEvent down+up
                    } else {
                        r.points.len() as u32
                    },
                    ms: 2.5,
                    cost_ns: None,
                    backend: r.backend.clone(),
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
        OpCode::Query => respond(
            req,
            OpCode::Query as u16,
            encode(&QueryAck {
                found: true,
                node: Some(NodeInfo {
                    text: "fake".into(),
                    id: "com.fake:id/btn".into(),
                    desc: String::new(),
                    bounds: [10, 20, 110, 60],
                    class: "android.widget.Button".into(),
                    clickable: true,
                    enabled: true,
                    scrollable: None,
                }),
            })
            .unwrap(),
        ),
        OpCode::Text => respond(
            req,
            OpCode::Text as u16,
            encode(&TextAck {
                ok: true,
                chars: Some(2),
                cost_ns: 500_000,
            })
            .unwrap(),
        ),
        OpCode::Screenshot => respond(
            req,
            OpCode::Screenshot as u16,
            encode(&ShotAck {
                format: "jpeg".into(),
                data: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00].into(),
                w: Some(720),
                h: Some(1600),
                scale: Some(1.0),
                take_ns: None,
                enc_ns: None,
            })
            .unwrap(),
        ),
        OpCode::Batch => {
            let r: BatchReq = decode(&req.payload).unwrap();
            // Per-op results (§4.2): one entry per op, ABORTED marking handled
            // by the real agent — the fake OKs everything.
            let results = (0..r.ops.len() as u32)
                .map(|index| OpResult {
                    index,
                    code: "OK".into(),
                    message: String::new(),
                    cost_ns: 1_000,
                    data: None,
                })
                .collect();
            respond(
                req,
                OpCode::BatchResult as u16,
                encode(&BatchAck {
                    results,
                    total_ns: 3_000,
                })
                .unwrap(),
            )
        }
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
    })
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

    let (fmt, bytes) = c.screenshot("jpeg", 80, Some(0.5)).unwrap();
    assert_eq!(fmt, "jpeg");
    assert_eq!(&bytes[..2], &[0xFF, 0xD8]);
}

// ---- M2 client surface (batch / query / text / noack / ping) ---------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_batch_returns_per_op_results() {
    let addr = spawn_fake(FakeBehavior::Responsive).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut c = AgentClient::connect_timeout(addr, Duration::from_secs(2)).unwrap();

    let ack = c
        .batch_ops(vec![
            BatchOp::Inject(InjectReq {
                kind: InjectKind::Tap,
                points: vec![],
                wait_finish: false,
                keycode: None,
                meta: None,
                backend: None,
            }),
            BatchOp::WaitUntil(WaitUntilReq {
                selector: Selector {
                    text: Some("fake".into()),
                    ..Default::default()
                },
                timeout_ms: 5000,
                poll_ms: None,
            }),
            BatchOp::Inject(InjectReq {
                kind: InjectKind::Tap,
                points: vec![],
                wait_finish: true,
                keycode: None,
                meta: None,
                backend: None,
            }),
        ])
        .unwrap();
    assert_eq!(ack.results.len(), 3);
    assert_eq!(
        ack.results.iter().map(|r| r.index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(ack.results.iter().all(|r| r.code == "OK"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_query_and_text_and_ping() {
    let addr = spawn_fake(FakeBehavior::Responsive).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut c = AgentClient::connect_timeout(addr, Duration::from_secs(2)).unwrap();

    assert!(c
        .exists(Selector {
            id: Some("btn".into()),
            ..Default::default()
        })
        .unwrap());
    let qa = c
        .query(QueryReq {
            selector: Selector::default(),
            attrs: None,
        })
        .unwrap();
    assert_eq!(qa.node.unwrap().bounds, [10, 20, 110, 60]);

    c.set_text("你好", true).unwrap();
    c.ping().unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_noack_inject_gets_no_frame_and_stays_in_sync() {
    let addr = spawn_fake(FakeBehavior::Responsive).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut c = AgentClient::connect_timeout(addr, Duration::from_secs(1)).unwrap();

    // Fire-and-forget: returns after the write; the fake sent no response.
    c.tap_noack(700.0, 1600.0).unwrap();
    c.tap_noack(710.0, 1600.0).unwrap();
    // A subsequent normal request still pairs correctly — proves the fake
    // honoured NOACK (an errant INJECT_RESULT would poison this hello).
    let ack = c.hello().unwrap();
    assert_eq!(ack.agent, "fake-agent/0.1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_maps_agent_error_frame() {
    let addr = spawn_fake(FakeBehavior::Responsive).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut c = AgentClient::connect_timeout(addr, Duration::from_secs(2)).unwrap();

    // STREAM_CTRL is not implemented on the fake -> ERROR-flagged response.
    let err = c.call(OpCode::StreamCtrl, Vec::new()).unwrap_err();
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
    /// Whether `test -f <agent.jar>` reports the remote jar present.
    jar_exists: bool,
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
        let exit = if command.starts_with("test -f") && !self.jar_exists {
            Some(1)
        } else {
            Some(0)
        };
        Ok(ShellOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: exit,
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
        shells.iter().any(|c| c.contains("setsid env CLASSPATH")
            && c.contains("app_process")
            && c.contains(AGENT_MAIN_CLASS)),
        "{shells:?}"
    );
    // Single process: setsid env launches app_process directly — no `sh -c`
    // wrapper (the extra sh parent was the M2 double-process wart).
    assert!(!shells.iter().any(|c| c.contains("sh -c")), "{shells:?}");
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

// ---- watchdog recovery (M2) -------------------------------------------------

/// FakeAdb whose `start` command actually launches a minimal frame-speaking
/// listener on `target_port` — mirroring a device where `app_process` binds
/// the socket. The first connect_recover attempt must fail (nothing
/// listening), the ladder must run install+start, and the retry must
/// succeed through HELLO.
struct RecoveringAdb {
    inner: FakeAdb,
    target_port: u16,
}

impl AdbTransport for RecoveringAdb {
    fn list_devices(&mut self) -> AdbResult<Vec<DeviceInfo>> {
        self.inner.list_devices()
    }
    fn shell(&mut self, serial: &str, command: &str) -> AdbResult<ShellOutput> {
        // app_process can only bind when the remote jar exists — a missing
        // jar makes restart fail, forcing the install fallback (M3 ladder).
        if command.contains("app_process") && self.inner.jar_exists {
            let port = self.target_port;
            std::thread::spawn(move || {
                let l = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
                if let Ok((mut s, _)) = l.accept() {
                    // Speak just enough of the wire protocol for HELLO.
                    while let Ok(req) = Frame::read_from(&mut s) {
                        if req.header.msg_type == OpCode::Hello as u16 {
                            let p = encode(&HelloAck {
                                agent: "recovered/1".into(),
                                caps: caps::INJECT,
                                ver: None,
                                proto: None,
                                sdk: None,
                                abi: None,
                                display: None,
                                inject: None,
                            })
                            .unwrap();
                            let _ = Frame::new(
                                req.header.channel,
                                OpCode::HelloAck as u16,
                                flags::RESPONSE,
                                req.header.req_id,
                                p,
                            )
                            .write_to(&mut s);
                        } else {
                            break;
                        }
                    }
                }
            });
        }
        self.inner.shell(serial, command)
    }
    fn push(&mut self, s: &str, l: &Path, r: &str) -> AdbResult<()> {
        self.inner.jar_exists = true; // install materializes the remote jar
        self.inner.push(s, l, r)
    }
    fn pull(&mut self, s: &str, r: &str, l: &Path) -> AdbResult<()> {
        self.inner.pull(s, r, l)
    }
    fn forward(&mut self, s: &str, l: &str, r: &str) -> AdbResult<()> {
        self.inner.forward(s, l, r)
    }
    fn forward_remove(&mut self, s: &str, l: &str) -> AdbResult<()> {
        self.inner.forward_remove(s, l)
    }
}

#[test]
fn watchdog_recovers_dead_agent_and_hellos() {
    // Reserve the target port but hold it closed — first dial must fail.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let mut adb = RecoveringAdb {
        inner: FakeAdb {
            jar_exists: true,
            ..Default::default()
        },
        target_port: port,
    };
    {
        let mut m = AgentManager::new(&mut adb);
        let (_client, _port, hello) = m
            .connect_recover(
                "fake0",
                Some(port),
                "pilotdroid_agent",
                None, // zero-copy restart: no --jar needed while remote jar exists
            )
            .unwrap();
        assert_eq!(hello.agent, "recovered/1");
    }
    let inner = &adb.inner;
    // Zero-copy path: restart (stop+start) revived the daemon — no push.
    assert_eq!(inner.pushes.len(), 0, "zero-copy restart must not push");
    assert!(
        inner.shells.iter().any(|(_, c)| c.contains("app_process")),
        "relaunch missing: {:?}",
        inner.shells
    );
    // Two dial attempts → two forward rules, both cleaned/held consistently.
    assert_eq!(inner.forward_calls.len(), 2);
}

#[test]
fn watchdog_surfaces_recovery_failure() {
    // Port permanently closed — even after the ladder the retry must fail,
    // and the error must carry the Recover variant (not a bare io error).
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let mut adb = FakeAdb::default(); // start shell does NOT launch anything
    let mut m = AgentManager::new(&mut adb);
    let res = m.connect_recover(
        "fake0",
        Some(port),
        "pilotdroid_agent",
        Some(Path::new("/tmp/agent.jar")),
    );
    let err = match res {
        Err(e) => e,
        Ok(_) => panic!("expected recovery failure, got connected client"),
    };
    assert!(
        matches!(err, AgentError::Recover { stage: "retry", .. }),
        "got {err:?}"
    );
}

#[test]
fn watchdog_installs_when_remote_jar_missing() {
    // Zero-copy restart can't help when the remote jar is gone — the graded
    // ladder must detect `test -f` failure and push the local jar (requires
    // --jar), then relaunch and HELLO.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let mut adb = RecoveringAdb {
        inner: FakeAdb {
            jar_exists: false, // remote agent.jar absent
            ..Default::default()
        },
        target_port: port,
    };
    {
        let mut m = AgentManager::new(&mut adb);
        let (_client, _port, hello) = m
            .connect_recover(
                "fake0",
                Some(port),
                "pilotdroid_agent",
                Some(Path::new("/tmp/agent.jar")),
            )
            .unwrap();
        assert_eq!(hello.agent, "recovered/1");
    }
    let inner = &adb.inner;
    assert_eq!(inner.pushes.len(), 1, "missing remote jar → install push");
    assert!(
        inner.shells.iter().any(|(_, c)| c.starts_with("test -f")),
        "remote jar probe missing: {:?}",
        inner.shells
    );
}

#[test]
fn watchdog_jar_missing_pushes_embedded_then_retries() {
    // Remote jar absent and no --jar supplied → the embedded
    // AGENT_JAR_BYTES is pushed automatically; the failure that remains
    // is the connect retry, not a jar-missing report.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let mut adb = FakeAdb {
        jar_exists: false,
        ..Default::default()
    };
    let mut m = AgentManager::new(&mut adb);
    let res = m.connect_recover("fake0", Some(port), "pilotdroid_agent", None);
    match res {
        Err(AgentError::Recover { stage, .. }) => {
            assert_eq!(stage, "retry");
        }
        Err(e) => panic!("expected Recover{{stage:retry}}, got {e:?}"),
        Ok(_) => panic!("expected recovery failure, got connected client"),
    }
    let pushes = &adb.pushes;
    assert_eq!(pushes.len(), 1, "embedded jar push expected: {pushes:?}");
    assert!(
        std::fs::read(&pushes[0].1).unwrap().len() == pilotdroid_agent::AGENT_JAR_BYTES.len(),
        "pushed payload should be the embedded jar"
    );
}

// ---- M3 client surface: press / swipe / backend ----------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn client_press_key_and_swipe_and_backend() {
    let addr = spawn_fake(FakeBehavior::Responsive).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let mut c = AgentClient::connect_timeout(addr, Duration::from_secs(2)).unwrap();

    // KEY: press(4 = KEYCODE_BACK) — fake counts 2 events (KeyEvent down+up).
    let ack = c.press(4, None).unwrap();
    assert!(ack.ok);
    assert_eq!(ack.events, 2, "key = KeyEvent DOWN+UP");

    let ack = c.swipe(100.0, 1500.0, 500.0, 800.0, 300).unwrap();
    assert!(ack.ok);
    // swipe() interpolates duration/10 MOVE points between DOWN and UP.
    assert_eq!(ack.events, 32, "swipe = DOWN + 30×MOVE + UP");

    let ack = c
        .inject(InjectReq {
            kind: InjectKind::Tap,
            points: vec![
                MotionPoint {
                    action: 0,
                    x: 10.0,
                    y: 20.0,
                    t_ms: 0,
                    pid: None,
                },
                MotionPoint {
                    action: 1,
                    x: 10.0,
                    y: 20.0,
                    t_ms: 60,
                    pid: None,
                },
            ],
            wait_finish: false,
            keycode: None,
            meta: None,
            backend: Some("im".into()),
        })
        .unwrap();
    // Fake echoes the request's backend field back.
    assert_eq!(ack.backend.as_deref(), Some("im"));
}

// ---- media-channel stream tests (docs/02 §4.8) ------------------------------

/// Spawn a stream-capable fake: control listener answers STREAM_CTRL; the
/// media listener pushes `frames` FRAMEs (seq = req_id) on first connect,
/// STREAM_MORE on all but the last. `gap_at` skips one seq number to
/// emulate an agent-side drop. Returns (control_addr, media_addr).
async fn spawn_stream_fake(frames: u32, gap_at: Option<u32>) -> (SocketAddr, SocketAddr) {
    let ctrl = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let media = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (ctrl_addr, media_addr) = (ctrl.local_addr().unwrap(), media.local_addr().unwrap());

    tokio::spawn(async move {
        let (mut sock, _) = ctrl.accept().await.unwrap();
        while let Some(frame) = read_frame(&mut sock).await {
            let resp = if frame.header.msg_type == OpCode::StreamCtrl as u16 {
                let req: StreamCtrlReq = decode(&frame.payload).unwrap();
                match req.op {
                    StreamOp::Start => respond(
                        &frame,
                        OpCode::StreamCtrl as u16,
                        encode(&StreamStartAck {
                            ok: true,
                            socket: "pilotdroid_agent_media".into(),
                            effective_fps_hint: Some(5.0),
                        })
                        .unwrap(),
                    ),
                    StreamOp::Stop => respond(
                        &frame,
                        OpCode::StreamCtrl as u16,
                        encode(&StreamStopAck {
                            ok: true,
                            frames_sent: frames as u64,
                            dropped: gap_at.map(|_| 1).unwrap_or(0),
                        })
                        .unwrap(),
                    ),
                }
            } else {
                match dispatch_fake(&frame) {
                    Some(r) => r,
                    None => continue,
                }
            };
            if sock.write_to_async(&resp).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        let (mut sock, _) = media.accept().await.unwrap();
        let mut seq: u32 = 0;
        let mut sent = 0;
        while sent < frames {
            // Skip the seq once — receiver should see a gap of 1.
            if gap_at == Some(seq) {
                seq += 1;
                continue;
            }
            let last = sent == frames - 1;
            let flags = if last { 0 } else { flags::STREAM_MORE };
            let f = Frame::new(
                Channel::Media,
                MediaType::Frame as u16,
                flags,
                seq,
                encode(&MediaFrame {
                    format: "jpeg".into(),
                    w: 540,
                    h: 1200,
                    ts_ns: 1_000 + seq as u64,
                    take_ns: Some(30_000_000),
                    enc_ns: Some(40_000_000),
                    dropped: gap_at.filter(|&g| seq > g).map(|_| 1).unwrap_or(0),
                    data: vec![0xFF, 0xD8, seq as u8, 0xD9].into(),
                })
                .unwrap(),
            );
            if sock.write_to_async(&f).await.is_err() {
                return;
            }
            seq += 1;
            sent += 1;
        }
    });
    (ctrl_addr, media_addr)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_stream_receives_frames_in_order() {
    let (ctrl_addr, media_addr) = spawn_stream_fake(8, None).await;
    tokio::time::sleep(Duration::from_millis(30)).await;

    let mut ctrl = AgentClient::connect_timeout(ctrl_addr, Duration::from_secs(2)).unwrap();
    let ack = ctrl
        .stream_start(StreamCtrlReq {
            op: StreamOp::Start,
            fps: Some(10),
            scale: Some(0.5),
            quality: Some(70),
            format: None,
        })
        .unwrap();
    assert!(ack.ok);
    assert_eq!(ack.socket, "pilotdroid_agent_media");

    let mut media = MediaClient::connect(media_addr, Duration::from_secs(2)).unwrap();
    for i in 0..8 {
        let f = media.recv().unwrap();
        assert_eq!(f.format, "jpeg");
        assert_eq!(f.ts_ns, 1_000 + i as u64);
        assert_eq!(&f.data[..], &[0xFF, 0xD8, i as u8, 0xD9]);
    }
    assert!(media.is_ended(), "last frame clears STREAM_MORE");
    assert_eq!(media.stats().frames, 8);
    assert_eq!(media.stats().seq_gaps, 0);
    assert_eq!(media.stats().bytes, 8 * 4);

    let stop = ctrl.stream_stop().unwrap();
    assert!(stop.ok);
    assert_eq!(stop.frames_sent, 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_stream_detects_seq_gap_and_dropped_count() {
    let (ctrl_addr, media_addr) = spawn_stream_fake(6, Some(3)).await;
    tokio::time::sleep(Duration::from_millis(30)).await;

    let mut ctrl = AgentClient::connect_timeout(ctrl_addr, Duration::from_secs(2)).unwrap();
    ctrl.stream_start(StreamCtrlReq {
        op: StreamOp::Start,
        fps: None,
        scale: None,
        quality: None,
        format: None,
    })
    .unwrap();

    let mut media = MediaClient::connect(media_addr, Duration::from_secs(2)).unwrap();
    while !media.is_ended() {
        media.recv().unwrap();
    }
    // seq 3 skipped → exactly one gap; agent-reported dropped propagates.
    assert_eq!(media.stats().seq_gaps, 1);
    assert_eq!(media.stats().dropped, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn media_channel_rejects_non_frame_message() {
    // §4.8: only ch=1/msg_type=0x0001 is legal on the media socket.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let f = Frame::new(Channel::Control, OpCode::Pong as u16, 0, 0, vec![]);
        sock.write_to_async(&f).await.unwrap();
    });
    let mut media = MediaClient::connect(addr, Duration::from_secs(2)).unwrap();
    match media.recv() {
        Err(AgentError::UnexpectedResponse { .. }) => {}
        other => panic!("expected UnexpectedResponse, got {other:?}"),
    }
}

// ---- facade (ADR-007): Device/Session + lazy media attach + Drop cleanup ----

/// Shared-recorder AdbTransport: the Session owns it, the test keeps a
/// handle to inspect forward/shell calls after drop.
#[derive(Clone, Default)]
struct SpyAdb {
    inner: std::rc::Rc<std::cell::RefCell<FakeAdb>>,
    shell_out: std::rc::Rc<Vec<u8>>,
}

impl AdbTransport for SpyAdb {
    fn list_devices(&mut self) -> AdbResult<Vec<DeviceInfo>> {
        Ok(vec![])
    }
    fn shell(&mut self, serial: &str, command: &str) -> AdbResult<ShellOutput> {
        self.inner
            .borrow_mut()
            .shells
            .push((serial.into(), command.into()));
        Ok(ShellOutput {
            stdout: self.shell_out.as_ref().clone(),
            stderr: vec![],
            exit_code: Some(0),
        })
    }
    fn push(&mut self, s: &str, l: &Path, r: &str) -> AdbResult<()> {
        self.inner.borrow_mut().push(s, l, r)
    }
    fn pull(&mut self, s: &str, r: &str, l: &Path) -> AdbResult<()> {
        self.inner.borrow_mut().pull(s, r, l)
    }
    fn forward(&mut self, s: &str, l: &str, r: &str) -> AdbResult<()> {
        self.inner.borrow_mut().forward(s, l, r)
    }
    fn forward_remove(&mut self, s: &str, l: &str) -> AdbResult<()> {
        self.inner.borrow_mut().forward_remove(s, l)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn facade_session_basic_ops_and_app_current() {
    let addr = spawn_fake(FakeBehavior::Responsive).await;
    tokio::time::sleep(Duration::from_millis(30)).await;

    let spy = SpyAdb {
        shell_out: std::rc::Rc::new(
            b"mCurrentFocus=Window{abc u0 com.android.settings/com.android.settings.MainSettings}\n".to_vec(),
        ),
        ..Default::default()
    };
    let mut s = Device::with_transport(spy, "fake0")
        .session_opts(Some(addr.port()), "pilotdroid_agent")
        .unwrap();
    assert!(s.tap(1.0, 2.0).unwrap().ok);
    assert!(s.exists(Selector::default()).unwrap());
    assert!(s.set_text("hi", true).is_ok());
    assert!(s.dump().unwrap().contains("hierarchy"));
    assert_eq!(
        s.app_current().unwrap(),
        Some((
            "com.android.settings".into(),
            "com.android.settings.MainSettings".into()
        ))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn facade_stream_full_lifecycle() {
    let (ctrl_addr, media_addr) = spawn_stream_fake(4, None).await;
    tokio::time::sleep(Duration::from_millis(30)).await;

    let spy = SpyAdb::default();
    let calls = spy.inner.clone();
    let mut s = Device::with_transport(spy, "fake0")
        .session_opts(Some(ctrl_addr.port()), "pilotdroid_agent")
        .unwrap();

    // Lazy attach: start must NOT open a media forward yet — the agent
    // discards pre-start pending conns (§4.8).
    let ack = s.stream_start(Some(10), Some(0.5), Some(70)).unwrap();
    assert_eq!(ack.socket, "pilotdroid_agent_media");
    assert_eq!(calls.borrow().forward_calls.len(), 1, "only control fwd");

    // First read attaches: media forward appears, frames arrive in order.
    s.set_media_port(media_addr.port());
    let frames = s.stream_read(4, None).unwrap();
    assert_eq!(frames.len(), 4);
    assert_eq!(calls.borrow().forward_calls.len(), 2, "media fwd attached");
    assert!(calls
        .borrow()
        .forward_calls
        .iter()
        .any(|(_, _, r)| r.contains("pilotdroid_agent_media")));

    let stop = s.stream_stop().unwrap();
    assert_eq!(stop.frames_sent, 4);
    // Stop removes the media forward; control stays until drop.
    assert!(!calls.borrow().forwards.is_empty());

    // Drop: control forward removed — no leaked rules.
    let port = ctrl_addr.port();
    drop(s);
    let f = calls.borrow().forwards.clone();
    assert!(
        !f.iter().any(|(_, l, _)| l == &format!("tcp:{port}")),
        "control forward leaked: {f:?}"
    );
}
