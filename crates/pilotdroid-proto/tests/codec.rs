//! TDD tests for the M1 frame codec — written before the implementation.
//! Covers: header/frame roundtrip, malformed frame rejection, payload length
//! boundaries, opcode table completeness.

use pilotdroid_proto::payload::*;
use pilotdroid_proto::*;

#[test]
fn frame_roundtrip_with_payload() {
    let frame = Frame::new(
        Channel::Control,
        OpCode::Inject as u16,
        flags::RESPONSE,
        7,
        vec![0xde, 0xad, 0xbe, 0xef],
    );
    let mut buf = Vec::new();
    frame.write_to(&mut buf).unwrap();
    assert_eq!(buf.len(), HEADER_LEN + 4);
    let back = Frame::read_from(&mut &buf[..]).unwrap();
    assert_eq!(back.header, frame.header);
    assert_eq!(back.payload, frame.payload);
}

#[test]
fn frame_empty_payload() {
    let frame = Frame::new(Channel::Control, OpCode::Ping as u16, 0, 1, vec![]);
    let mut buf = Vec::new();
    frame.write_to(&mut buf).unwrap();
    let back = Frame::read_from(&mut &buf[..]).unwrap();
    assert!(back.payload.is_empty());
    assert_eq!(back.header.payload_len, 0);
}

#[test]
fn rejects_truncated_header() {
    let hdr = FrameHeader::new(Channel::Control, OpCode::Hello as u16, 0, 1, 4);
    let mut buf = Vec::new();
    hdr.encode(&mut buf).unwrap();
    buf.truncate(HEADER_LEN - 1);
    assert!(Frame::read_from(&mut &buf[..]).is_err());
}

#[test]
fn rejects_truncated_payload() {
    let frame = Frame::new(
        Channel::Control,
        OpCode::Dump as u16,
        0,
        1,
        vec![1, 2, 3, 4],
    );
    let mut buf = Vec::new();
    frame.write_to(&mut buf).unwrap();
    buf.truncate(HEADER_LEN + 2); // payload incomplete
    assert!(Frame::read_from(&mut &buf[..]).is_err());
}

#[test]
fn rejects_bad_magic_in_frame() {
    let frame = Frame::new(Channel::Control, OpCode::Hello as u16, 0, 1, vec![1]);
    let mut buf = Vec::new();
    frame.write_to(&mut buf).unwrap();
    buf[0] = b'X';
    assert!(matches!(
        Frame::read_from(&mut &buf[..]),
        Err(ProtocolError::BadMagic(_))
    ));
}

#[test]
fn rejects_unsupported_version() {
    let frame = Frame::new(Channel::Control, OpCode::Hello as u16, 0, 1, vec![]);
    let mut buf = Vec::new();
    frame.write_to(&mut buf).unwrap();
    buf[4] = PROTOCOL_VERSION + 1;
    assert!(matches!(
        Frame::read_from(&mut &buf[..]),
        Err(ProtocolError::UnsupportedVersion(_))
    ));
}

#[test]
fn rejects_invalid_channel() {
    let frame = Frame::new(Channel::Control, OpCode::Hello as u16, 0, 1, vec![]);
    let mut buf = Vec::new();
    frame.write_to(&mut buf).unwrap();
    buf[5] = 9; // channel field
    assert!(matches!(
        Frame::read_from(&mut &buf[..]),
        Err(ProtocolError::InvalidChannel(9))
    ));
}

#[test]
fn payload_len_boundary_max_ok() {
    // Max-size payload must roundtrip.
    let hdr = FrameHeader::new(
        Channel::Media,
        MediaType::Frame as u16,
        0,
        1,
        MAX_PAYLOAD_LEN,
    );
    let mut buf = Vec::new();
    hdr.encode(&mut buf).unwrap();
    // append the full payload (64 MiB) — heavy but honest boundary test
    buf.resize(HEADER_LEN + MAX_PAYLOAD_LEN as usize, 0xAB);
    let back = Frame::read_from(&mut &buf[..]).unwrap();
    assert_eq!(back.payload.len(), MAX_PAYLOAD_LEN as usize);
}

#[test]
fn payload_len_boundary_max_plus_one_rejected() {
    let hdr = FrameHeader::new(
        Channel::Media,
        MediaType::Frame as u16,
        0,
        1,
        MAX_PAYLOAD_LEN + 1,
    );
    let mut buf = Vec::new();
    hdr.encode(&mut buf).unwrap();
    assert!(matches!(
        Frame::read_from(&mut &buf[..]),
        Err(ProtocolError::PayloadTooLarge(_))
    ));
}

#[test]
fn opcode_table_covers_every_variant() {
    // If a new opcode is added without updating ALL, this fails — the wire
    // table is the shared contract with the Java agent.
    for op in OpCode::ALL {
        assert_eq!(OpCode::try_from(op as u16).unwrap(), op);
    }
    assert_eq!(OpCode::ALL.len(), 14);
    // Duplicate check
    let mut codes: Vec<u16> = OpCode::ALL.iter().map(|o| *o as u16).collect();
    codes.sort_unstable();
    codes.dedup();
    assert_eq!(codes.len(), OpCode::ALL.len());
}

#[test]
fn payload_msgpack_roundtrip_all_types() {
    let hello = HelloAck {
        agent: "pilotdroid/0.1".into(),
        caps: caps::INJECT | caps::DUMP | caps::SHOT,
        ver: None,
        proto: None,
        sdk: None,
        abi: None,
        display: None,
        inject: None,
    };
    assert_eq!(decode::<HelloAck>(&encode(&hello).unwrap()).unwrap(), hello);

    let inj = InjectReq {
        kind: InjectKind::Tap,
        points: vec![
            MotionPoint {
                action: 0,
                x: 700.0,
                y: 1600.0,
                t_ms: 0,
            },
            MotionPoint {
                action: 1,
                x: 700.0,
                y: 1600.0,
                t_ms: 15,
            },
        ],
        wait_finish: true,
    };
    assert_eq!(decode::<InjectReq>(&encode(&inj).unwrap()).unwrap(), inj);

    let shot = ShotAck {
        format: "jpeg".into(),
        data: serde_bytes::ByteBuf::from(vec![0xFF, 0xD8, 0xFF]),
        w: None,
        h: None,
        scale: None,
    };
    assert_eq!(decode::<ShotAck>(&encode(&shot).unwrap()).unwrap(), shot);

    let err = ErrorAck {
        code: "UNSUPPORTED".into(),
        message: "DUMP pending".into(),
    };
    assert_eq!(decode::<ErrorAck>(&encode(&err).unwrap()).unwrap(), err);
}

#[test]
fn payload_decode_rejects_garbage() {
    assert!(decode::<HelloAck>(&[0xFF, 0xFF, 0xFF]).is_err());
}

#[test]
fn caps_bits_are_the_wire_contract() {
    // Normative bitmap — Java Agent CAP_* must match (t4 review found the
    // two sides had diverged; caps=0x7 on the wire = INJECT|DUMP|SHOT).
    assert_eq!(caps::INJECT, 0x01);
    assert_eq!(caps::DUMP, 0x02);
    assert_eq!(caps::SHOT, 0x04);
    assert_eq!(caps::QUERY, 0x08);
    assert_eq!(caps::BATCH, 0x10);
    assert_eq!(caps::STREAM, 0x20);
    assert_eq!(caps::TEXT, 0x40);
    assert_eq!(caps::INJECT | caps::DUMP | caps::SHOT, 0x07);
}

// ---- M2 contract tests (docs/02-protocol.md §4, written red-first) ----------

#[test]
fn noack_flag_is_bit3() {
    assert_eq!(flags::NOACK, 0x0008);
}

#[test]
fn selector_roundtrip_and_sparse_wire() {
    let req = WaitUntilReq {
        selector: Selector {
            text: Some("确定".into()),
            id: Some("com.pkg:id/ok".into()),
            ..Default::default()
        },
        timeout_ms: 5000,
        poll_ms: Some(20),
    };
    assert_eq!(decode::<WaitUntilReq>(&encode(&req).unwrap()).unwrap(), req);

    // Optional selector fields must be absent on the wire, not nil.
    let sparse = WaitUntilReq {
        selector: Selector {
            text: Some("x".into()),
            ..Default::default()
        },
        timeout_ms: 100,
        poll_ms: None,
    };
    let v: rmpv::Value = rmp_serde::from_slice(&encode(&sparse).unwrap()).unwrap();
    let sel = &v.as_map().unwrap()[0].1;
    assert_eq!(sel.as_map().unwrap().len(), 1);
}

#[test]
fn wait_until_defaults_match_spec() {
    // Minimal wire payload {selector:{}} → timeout_ms=5000, poll_ms absent (§4.3).
    let raw = rmpv::Value::Map(vec![(
        rmpv::Value::from("selector"),
        rmpv::Value::Map(vec![]),
    )]);
    let req: WaitUntilReq = decode(&rmp_serde::to_vec_named(&raw).unwrap()).unwrap();
    assert_eq!(req.timeout_ms, 5000);
    assert_eq!(req.poll_ms, None);
}

#[test]
fn batch_op_encodes_as_single_key_map() {
    // §4.1: each op is a one-key map — {"inject": {...}}, {"wait_until": {...}}…
    let cases: Vec<(BatchOp, &str)> = vec![
        (
            BatchOp::Inject(InjectReq {
                kind: InjectKind::Tap,
                points: vec![],
                wait_finish: false,
            }),
            "inject",
        ),
        (
            BatchOp::WaitUntil(WaitUntilReq {
                selector: Selector::default(),
                timeout_ms: 100,
                poll_ms: None,
            }),
            "wait_until",
        ),
        (
            BatchOp::Query(QueryReq {
                selector: Selector::default(),
                attrs: None,
            }),
            "query",
        ),
        (
            BatchOp::Dump(DumpReq {
                format: "xml".into(),
            }),
            "dump",
        ),
        (
            BatchOp::Screenshot(ShotReq {
                format: "jpeg".into(),
                quality: 80,
                scale: Some(0.5),
            }),
            "screenshot",
        ),
        (
            BatchOp::Text(TextReq {
                text: "hi".into(),
                clear: None,
            }),
            "text",
        ),
    ];
    for (op, key) in cases {
        let v: rmpv::Value = rmp_serde::from_slice(&encode(&op).unwrap()).unwrap();
        let m = v.as_map().unwrap();
        assert_eq!(m.len(), 1, "{op:?}");
        assert_eq!(m[0].0.as_str(), Some(key), "{op:?}");
        assert_eq!(decode::<BatchOp>(&encode(&op).unwrap()).unwrap(), op);
    }
}

#[test]
fn batch_req_defaults_and_fail_policy_names() {
    // fail_policy wire names per §4.1.
    for (p, s) in [
        (FailPolicy::Stop, "stop"),
        (FailPolicy::Continue, "continue"),
    ] {
        let v: rmpv::Value = rmp_serde::from_slice(&encode(&p).unwrap()).unwrap();
        assert_eq!(v.as_str(), Some(s));
    }
    // omitted fields → defaults (stop / no deadline / no poll_ms).
    let raw = rmpv::Value::Map(vec![(rmpv::Value::from("ops"), rmpv::Value::Array(vec![]))]);
    let req: BatchReq = decode(&rmp_serde::to_vec_named(&raw).unwrap()).unwrap();
    assert_eq!(req.fail_policy, FailPolicy::Stop);
    assert_eq!(req.deadline_ms, None);
    assert_eq!(req.poll_ms, None);
}

#[test]
fn batch_ack_decodes_per_op_results() {
    let ack = BatchAck {
        results: vec![
            OpResult {
                index: 0,
                code: "OK".into(),
                message: String::new(),
                cost_ns: 1_234_000,
                data: None,
            },
            OpResult {
                index: 1,
                code: "TIMEOUT".into(),
                message: "selector not found".into(),
                cost_ns: 5_000_000_000,
                data: None,
            },
            OpResult {
                index: 2,
                code: "OK".into(),
                message: String::new(),
                cost_ns: 10,
                data: Some(rmpv::Value::from(true)),
            },
        ],
        total_ns: 5_001_244_000,
    };
    assert_eq!(decode::<BatchAck>(&encode(&ack).unwrap()).unwrap(), ack);
}

#[test]
fn shot_req_scale_omitted_when_none() {
    // M1 backward compat: scale must not appear on the wire when unset.
    let req = ShotReq {
        format: "jpeg".into(),
        quality: 80,
        scale: None,
    };
    let v: rmpv::Value = rmp_serde::from_slice(&encode(&req).unwrap()).unwrap();
    assert_eq!(v.as_map().unwrap().len(), 2);
    let scaled = ShotReq {
        scale: Some(0.5),
        ..req
    };
    let v: rmpv::Value = rmp_serde::from_slice(&encode(&scaled).unwrap()).unwrap();
    assert_eq!(v.as_map().unwrap().len(), 3);
    assert_eq!(
        decode::<ShotReq>(&encode(&scaled).unwrap()).unwrap(),
        scaled
    );
}

#[test]
fn query_and_text_payloads_roundtrip() {
    let q = QueryReq {
        selector: Selector {
            id: Some("ok".into()),
            ..Default::default()
        },
        attrs: Some(vec!["text".into(), "bounds".into()]),
    };
    assert_eq!(decode::<QueryReq>(&encode(&q).unwrap()).unwrap(), q);

    let qa = QueryAck {
        found: true,
        node: Some(NodeInfo {
            text: "OK".into(),
            id: "com.pkg:id/ok".into(),
            desc: String::new(),
            bounds: [1, 2, 3, 4],
            class: "android.widget.Button".into(),
            clickable: true,
            enabled: true,
        }),
    };
    assert_eq!(decode::<QueryAck>(&encode(&qa).unwrap()).unwrap(), qa);

    let t = TextReq {
        text: "你好".into(),
        clear: Some(true),
    };
    assert_eq!(decode::<TextReq>(&encode(&t).unwrap()).unwrap(), t);
}

// ---- M2 cleanup: wire-redundant fields landed in proto (t5 review) ---------

#[test]
fn text_ack_decodes_agent_chars_field() {
    // Java agent emits {ok, chars, cost_ns} — proto must carry chars (t5).
    let ack = TextAck {
        ok: true,
        chars: Some(6),
        cost_ns: 9_227_761,
    };
    assert_eq!(decode::<TextAck>(&encode(&ack).unwrap()).unwrap(), ack);
    // Older/other agents without chars still decode.
    let raw = rmpv::Value::Map(vec![
        (rmpv::Value::from("ok"), rmpv::Value::from(true)),
        (rmpv::Value::from("cost_ns"), rmpv::Value::from(5u64)),
    ]);
    let back: TextAck = decode(&rmp_serde::to_vec_named(&raw).unwrap()).unwrap();
    assert_eq!(back.chars, None);
}

#[test]
fn shot_ack_decodes_agent_geometry_fields() {
    // Java M2 agent emits {format,data,w,h,scale} — proto must carry them (t5).
    let ack = ShotAck {
        format: "jpeg".into(),
        data: vec![0xFF, 0xD8].into(),
        w: Some(720),
        h: Some(1600),
        scale: Some(0.5),
    };
    assert_eq!(decode::<ShotAck>(&encode(&ack).unwrap()).unwrap(), ack);
    // M1-era {format,data} replies still decode with the extras absent.
    let raw = rmpv::Value::Map(vec![
        (rmpv::Value::from("format"), rmpv::Value::from("jpeg")),
        (
            rmpv::Value::from("data"),
            rmpv::Value::Binary(vec![0xFF, 0xD8]),
        ),
    ]);
    let back: ShotAck = decode(&rmp_serde::to_vec_named(&raw).unwrap()).unwrap();
    assert_eq!(back.w, None);
    assert_eq!(back.scale, None);
}

#[test]
fn hello_and_inject_acks_carry_agent_extras() {
    // M2 Java emits ver/proto/sdk/abi/display/inject on HELLO_ACK and
    // cost_ns/backend on INJECT_RESULT — proto must carry them (t5 sweep).
    let hello = HelloAck {
        agent: "pilotdroid".into(),
        caps: 0x5f,
        ver: Some("0.2.0".into()),
        proto: Some(0),
        sdk: Some(34),
        abi: Some("arm64-v8a".into()),
        display: Some(vec![1440, 3200]),
        inject: Some("im+uia-fallback".into()),
    };
    assert_eq!(decode::<HelloAck>(&encode(&hello).unwrap()).unwrap(), hello);
    // M1 minimal {agent,caps} still decodes.
    let raw = rmpv::Value::Map(vec![
        (rmpv::Value::from("agent"), rmpv::Value::from("pilotdroid")),
        (rmpv::Value::from("caps"), rmpv::Value::from(7u64)),
    ]);
    let back: HelloAck = decode(&rmp_serde::to_vec_named(&raw).unwrap()).unwrap();
    assert_eq!(back.caps, 7);
    assert_eq!(back.inject, None);

    let inj = InjectAck {
        ok: true,
        events: 2,
        ms: 4.9,
        cost_ns: Some(4_900_000),
        backend: Some("uia".into()),
    };
    assert_eq!(decode::<InjectAck>(&encode(&inj).unwrap()).unwrap(), inj);
}
