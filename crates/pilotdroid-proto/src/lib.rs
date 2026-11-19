//! Binary framing for the pilotdroid agent channel.
//!
//! Placeholder for M2 — see `docs/02-protocol.md` for the wire spec this is
//! derived from. Header layout (18 bytes, big-endian):
//!
//! ```text
//!  0               4   5   6       8        10             14
//! +---------------+---+---+--------+---------+-------------+
//! | magic "PD01"  |ver|ch |msg_type|  flags  |   req_id    | (cont.)
//! +---------------+---+---+--------+---------+-------------+--------+
//! |          req_id cont.          |       payload_len     |
//! +--------------------------------+------------------------+
//! ```

use std::io::{self, Read, Write};

/// Frame magic, `"PD01"`.
pub const MAGIC: [u8; 4] = *b"PD01";
/// Wire version for the v0 framing described in docs/02-protocol.md §2.
pub const PROTOCOL_VERSION: u8 = 0;

/// Hard cap on a single frame payload. Bump deliberately, never silently.
pub const MAX_PAYLOAD_LEN: u32 = 64 * 1024 * 1024;

pub const HEADER_LEN: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Channel {
    Control = 0,
    Media = 1,
    Diag = 2,
}

impl TryFrom<u8> for Channel {
    type Error = ProtocolError;

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Control),
            1 => Ok(Self::Media),
            2 => Ok(Self::Diag),
            _ => Err(ProtocolError::InvalidChannel(v)),
        }
    }
}

/// Control-channel opcodes (docs/02-protocol.md §3).
/// Media-channel types share the namespace only inside `Channel::Media`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum OpCode {
    Hello = 0x0001,
    HelloAck = 0x0002,
    Ping = 0x0003,
    Pong = 0x0004,
    Inject = 0x0010,
    InjectResult = 0x0011,
    Dump = 0x0020,
    Query = 0x0021,
    Screenshot = 0x0030,
    Batch = 0x0040,
    BatchResult = 0x0041,
    StreamCtrl = 0x0050,
    Text = 0x0060,
    AgentStop = 0x00F0,
}

impl TryFrom<u16> for OpCode {
    type Error = ProtocolError;

    fn try_from(v: u16) -> Result<Self, Self::Error> {
        Ok(match v {
            0x0001 => Self::Hello,
            0x0002 => Self::HelloAck,
            0x0003 => Self::Ping,
            0x0004 => Self::Pong,
            0x0010 => Self::Inject,
            0x0011 => Self::InjectResult,
            0x0020 => Self::Dump,
            0x0021 => Self::Query,
            0x0030 => Self::Screenshot,
            0x0040 => Self::Batch,
            0x0041 => Self::BatchResult,
            0x0050 => Self::StreamCtrl,
            0x0060 => Self::Text,
            0x00F0 => Self::AgentStop,
            other => return Err(ProtocolError::InvalidType(other)),
        })
    }
}

/// Flag bits on the frame header (docs/02-protocol.md §2).
pub mod flags {
    pub const RESPONSE: u16 = 1 << 0;
    pub const STREAM_MORE: u16 = 1 << 1;
    pub const ERROR: u16 = 1 << 2;
}

#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("bad magic {0:02x?}, expected PD01")]
    BadMagic([u8; 4]),
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u8),
    #[error("invalid channel {0}")]
    InvalidChannel(u8),
    #[error("invalid message type 0x{0:04x}")]
    InvalidType(u16),
    #[error("payload_len {0} exceeds MAX_PAYLOAD_LEN")]
    PayloadTooLarge(u32),
}

/// Fixed-size frame header. Payload follows as msgpack bytes (M2; the serde
/// layer is intentionally not part of this skeleton).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameHeader {
    pub channel: Channel,
    pub msg_type: u16,
    pub flags: u16,
    pub req_id: u32,
    pub payload_len: u32,
}

impl FrameHeader {
    pub fn new(channel: Channel, msg_type: u16, flags: u16, req_id: u32, payload_len: u32) -> Self {
        Self {
            channel,
            msg_type,
            flags,
            req_id,
            payload_len,
        }
    }

    pub fn encode(&self, out: &mut impl Write) -> io::Result<()> {
        out.write_all(&MAGIC)?;
        out.write_all(&[PROTOCOL_VERSION, self.channel as u8])?;
        out.write_all(&self.msg_type.to_be_bytes())?;
        out.write_all(&self.flags.to_be_bytes())?;
        out.write_all(&self.req_id.to_be_bytes())?;
        out.write_all(&self.payload_len.to_be_bytes())?;
        Ok(())
    }

    pub fn decode(input: &mut impl Read) -> Result<Self, ProtocolError> {
        let mut magic = [0u8; 4];
        input.read_exact(&mut magic)?;
        if magic != MAGIC {
            return Err(ProtocolError::BadMagic(magic));
        }
        let mut b = [0u8; 2];
        input.read_exact(&mut b)?;
        if b[0] != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(b[0]));
        }
        let channel = Channel::try_from(b[1])?;

        let mut u16buf = [0u8; 2];
        input.read_exact(&mut u16buf)?;
        let msg_type = u16::from_be_bytes(u16buf);
        input.read_exact(&mut u16buf)?;
        let flags = u16::from_be_bytes(u16buf);

        let mut u32buf = [0u8; 4];
        input.read_exact(&mut u32buf)?;
        let req_id = u32::from_be_bytes(u32buf);
        input.read_exact(&mut u32buf)?;
        let payload_len = u32::from_be_bytes(u32buf);
        if payload_len > MAX_PAYLOAD_LEN {
            return Err(ProtocolError::PayloadTooLarge(payload_len));
        }

        Ok(Self::new(channel, msg_type, flags, req_id, payload_len))
    }
}

/// Decode `HEADER_LEN` bytes at once — the common framing path.
pub fn decode_header_bytes(buf: &[u8; HEADER_LEN]) -> Result<FrameHeader, ProtocolError> {
    FrameHeader::decode(&mut &buf[..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_roundtrip() {
        let hdr = FrameHeader::new(
            Channel::Control,
            OpCode::Batch as u16,
            flags::STREAM_MORE,
            42,
            512,
        );
        let mut buf = Vec::new();
        hdr.encode(&mut buf).unwrap();
        assert_eq!(buf.len(), HEADER_LEN);
        let back = FrameHeader::decode(&mut &buf[..]).unwrap();
        assert_eq!(hdr, back);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"NOPE");
        buf.extend_from_slice(&[0; HEADER_LEN - 4]);
        assert!(matches!(
            FrameHeader::decode(&mut &buf[..]),
            Err(ProtocolError::BadMagic(_))
        ));
    }

    #[test]
    fn rejects_oversized_payload() {
        let hdr = FrameHeader::new(Channel::Media, 0x0001, 0, 1, MAX_PAYLOAD_LEN + 1);
        let mut buf = Vec::new();
        hdr.encode(&mut buf).unwrap();
        assert!(matches!(
            FrameHeader::decode(&mut &buf[..]),
            Err(ProtocolError::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn opcode_codes_stable() {
        // Wire codes are part of the protocol spec; guard against accidental renumbering.
        assert_eq!(OpCode::Batch as u16, 0x0040);
        assert_eq!(OpCode::AgentStop as u16, 0x00F0);
        assert_eq!(OpCode::try_from(0x0060).unwrap(), OpCode::Text);
    }
}
