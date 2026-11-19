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
        caps: caps::INJECT_IM | caps::UIAUTOMATION | caps::SCREENSHOT,
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
