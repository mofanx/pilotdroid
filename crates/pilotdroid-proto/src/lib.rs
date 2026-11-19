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

/// Flag bits on the frame header (docs/02-protocol.md §2, §4.5).
pub mod flags {
    pub const RESPONSE: u16 = 1 << 0;
    /// Reserved for the M3 media stream — unused on the control channel.
    pub const STREAM_MORE: u16 = 1 << 1;
    pub const ERROR: u16 = 1 << 2;
    /// Fire-and-forget: agent must not send a response frame (§4.5).
    pub const NOACK: u16 = 1 << 3;
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
    #[error("payload codec error: {0}")]
    Codec(String),
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

// ---- M1 codec additions -----------------------------------------------------

/// Media-channel message types (docs/02-protocol.md §3, ch=1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum MediaType {
    Frame = 0x0001,
    UiEvent = 0x0002,
}

impl TryFrom<u16> for MediaType {
    type Error = ProtocolError;

    fn try_from(v: u16) -> Result<Self, Self::Error> {
        Ok(match v {
            0x0001 => Self::Frame,
            0x0002 => Self::UiEvent,
            other => return Err(ProtocolError::InvalidType(other)),
        })
    }
}

impl OpCode {
    /// Every control-channel opcode, for coverage assertions.
    pub const ALL: [Self; 14] = [
        Self::Hello,
        Self::HelloAck,
        Self::Ping,
        Self::Pong,
        Self::Inject,
        Self::InjectResult,
        Self::Dump,
        Self::Query,
        Self::Screenshot,
        Self::Batch,
        Self::BatchResult,
        Self::StreamCtrl,
        Self::Text,
        Self::AgentStop,
    ];
}

/// A complete wire frame: 18-byte header + `payload_len` msgpack bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(channel: Channel, msg_type: u16, flags: u16, req_id: u32, payload: Vec<u8>) -> Self {
        let header = FrameHeader::new(channel, msg_type, flags, req_id, payload.len() as u32);
        Self { header, payload }
    }

    /// Serialize header + payload to `w`.
    pub fn write_to(&self, w: &mut impl Write) -> io::Result<()> {
        self.header.encode(w)?;
        w.write_all(&self.payload)?;
        w.flush()
    }

    /// Read one frame from `r` (header then exactly `payload_len` bytes).
    pub fn read_from(r: &mut impl Read) -> Result<Self, ProtocolError> {
        let header = FrameHeader::decode(r)?;
        let mut payload = vec![0u8; header.payload_len as usize];
        r.read_exact(&mut payload)?;
        Ok(Self { header, payload })
    }
}

/// msgpack payload schemas shared with the device agent (docs/02 §3).
/// Field names below ARE the wire contract — the Java agent must match.
pub mod payload {
    use serde::{Deserialize, Serialize};
    use serde_bytes::ByteBuf;

    use crate::ProtocolError;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct HelloReq {
        pub proto: u8,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct HelloAck {
        pub agent: String,
        pub caps: u32,
        /// Agent build version (M2 emits e.g. "0.2.0").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub ver: Option<String>,
        /// Protocol version the agent speaks.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub proto: Option<u8>,
        /// Device SDK level (`Build.VERSION.SDK_INT`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub sdk: Option<u32>,
        /// Primary ABI.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub abi: Option<String>,
        /// Active display `[w, h]` in px.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub display: Option<Vec<i32>>,
        /// Injection backend in use: "im" | "uia" | "im+uia-fallback"
        /// (fallback-chain probe, docs/02 §3 / t2 spec).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub inject: Option<String>,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum InjectKind {
        Tap,
        Swipe,
        Key,
    }

    /// One point of a MotionEvent sequence (action = MotionEvent.ACTION_*).
    #[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
    pub struct MotionPoint {
        pub action: u8,
        pub x: f32,
        pub y: f32,
        /// eventTime offset from first point, ms.
        pub t_ms: u64,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct InjectReq {
        pub kind: InjectKind,
        /// MotionEvent sequence (down/move/up + t_ms offsets). Omitted on
        /// the wire for `kind=Key` requests (§4.7).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pub points: Vec<MotionPoint>,
        #[serde(default)]
        pub wait_finish: bool,
        /// `android.view.KeyEvent.KEYCODE_*` — only for `kind=Key`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub keycode: Option<u32>,
        /// KeyEvent metaState (§4.7); absent = 0.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub meta: Option<u32>,
        /// Force injection backend: `"im"` or `"uia"` (§4.7). Absent =
        /// agent probe fallback chain (IM→UIA). A forced-but-unavailable
        /// backend must fail with `ERR_BACKEND`, not silently degrade.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub backend: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct InjectAck {
        pub ok: bool,
        pub events: u32,
        pub ms: f64,
        /// Device-internal cost (Java emits alongside ms).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub cost_ns: Option<u64>,
        /// Injection backend that handled the op ("im" | "uia" |
        /// "im+uia-fallback") — observability for the fallback chain.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub backend: Option<String>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct DumpReq {
        /// "xml" for M1; structured node tree is a later option.
        pub format: String,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct DumpAck {
        pub xml: String,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ShotReq {
        pub format: String,
        pub quality: u8,
        /// (0,1] downscale factor (§4.4); omitted on the wire when unset so
        /// M1 agents still decode the two-field shape.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub scale: Option<f32>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ShotAck {
        pub format: String,
        pub data: ByteBuf,
        /// Image dimensions after on-device scaling (M2 agent emits them;
        /// absent on M1-era replies — kept optional for compat).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub w: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub h: Option<u32>,
        /// Scale factor the agent applied (echo of ShotReq.scale, 1.0 when
        /// unscaled).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub scale: Option<f32>,
        /// Device-side capture+scale time (ns) — M3 agent emits this timing
        /// split for SHOT bottleneck attribution (spike/e2e/m3-experiments).
        /// Absent on older agents.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub take_ns: Option<u64>,
        /// Device-side JPEG encode time (ns); same provenance as `take_ns`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub enc_ns: Option<u64>,
    }

    // ---- M2 payloads (docs/02-protocol.md §4) -------------------------------

    fn default_wait_timeout_ms() -> u64 {
        5000
    }

    /// Device-side predicate selector — `{text?, id?, desc?, index?}` with AND
    /// semantics (§4.3). `id` matches `pkg:id/name` exactly or a bare `name`
    /// by `:id/` suffix; `index` picks the Nth hit (default 0) and is also
    /// existential (Nth hit missing → no match).
    #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
    pub struct Selector {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub desc: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub index: Option<u32>,
    }

    /// `wait_until` op — device-side poll on the resident UiAutomation tree
    /// (§4.3, ADR-004). Timeout is an op result (`code:"TIMEOUT"`), not a
    /// frame error.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct WaitUntilReq {
        pub selector: Selector,
        #[serde(default = "default_wait_timeout_ms")]
        pub timeout_ms: u64,
        /// Poll interval; absent → BATCH.poll_ms → agent default 20ms.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub poll_ms: Option<u64>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct QueryReq {
        pub selector: Selector,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub attrs: Option<Vec<String>>,
    }

    /// One UI node as returned by QUERY / wait_until `data.node` (§4.4).
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct NodeInfo {
        pub text: String,
        pub id: String,
        pub desc: String,
        /// `[left, top, right, bottom]` in device px.
        pub bounds: [i32; 4],
        pub class: String,
        pub clickable: bool,
        pub enabled: bool,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct QueryAck {
        pub found: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub node: Option<NodeInfo>,
    }

    /// `TEXT` op — `ACTION_SET_TEXT` on the focused node (§4.4). Failure when
    /// nothing is focused surfaces as `code:"NO_FOCUS"`.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct TextReq {
        pub text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub clear: Option<bool>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct TextAck {
        pub ok: bool,
        /// Characters written by ACTION_SET_TEXT (M2 agent reports it).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub chars: Option<u64>,
        pub cost_ns: u64,
    }

    /// BATCH-level failure policy (§4.1).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum FailPolicy {
        #[default]
        Stop,
        Continue,
    }

    /// One op inside a BATCH request. Serde's external tagging produces
    /// exactly the spec'd single-key map: `{"inject": InjectReq}` etc.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum BatchOp {
        Inject(InjectReq),
        WaitUntil(WaitUntilReq),
        Query(QueryReq),
        Dump(DumpReq),
        Screenshot(ShotReq),
        Text(TextReq),
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct BatchReq {
        pub ops: Vec<BatchOp>,
        #[serde(default)]
        pub fail_policy: FailPolicy,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub deadline_ms: Option<u64>,
        /// Default `poll_ms` for the batch's wait_until ops (§4.1).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub poll_ms: Option<u64>,
    }

    /// Per-op result inside BATCH_RESULT (§4.2). Every op gets an entry —
    /// `ABORTED` after a fail-fast, `DEADLINE` after batch deadline.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct OpResult {
        pub index: u32,
        pub code: String,
        pub message: String,
        /// Device-internal op cost.
        pub cost_ns: u64,
        /// Op-specific payload (dump xml / shot data / query node).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub data: Option<rmpv::Value>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct BatchAck {
        pub results: Vec<OpResult>,
        pub total_ns: u64,
    }

    // ---- M4 media channel (docs/02 §4.8, ADR-006) ---------------------------

    /// STREAM_CTRL request op (§4.8). `start` carries the tuning fields;
    /// `stop` takes none.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    pub enum StreamOp {
        Start,
        Stop,
    }

    /// STREAM_CTRL request — `{op:"start", fps?, scale?, quality?, format?}`.
    /// Tuning fields are start-only and skipped on the wire when unset.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct StreamCtrlReq {
        pub op: StreamOp,
        /// Target frames/sec (agent default 5).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub fps: Option<u8>,
        /// Downscale factor (0,1] (agent default 0.5).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub scale: Option<f32>,
        /// JPEG quality 1-100 (agent default 70).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub quality: Option<u8>,
        /// "jpeg" only in M4; "h264" reserved (M5).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub format: Option<String>,
    }

    /// STREAM_CTRL start response — `{ok, socket, effective_fps_hint?}`.
    /// `socket` is the localabstract name the media channel listens on.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct StreamStartAck {
        pub ok: bool,
        pub socket: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub effective_fps_hint: Option<f32>,
    }

    /// STREAM_CTRL stop response — `{ok, frames_sent, dropped}`.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct StreamStopAck {
        pub ok: bool,
        pub frames_sent: u64,
        pub dropped: u64,
    }

    /// FRAME payload on the media channel (ch=1, msg_type 0x0001). The
    /// frame header `req_id` is the monotonic seq (§4.8); `dropped` is the
    /// agent's cumulative drop counter for backpressure observability.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct MediaFrame {
        pub format: String,
        pub w: u32,
        pub h: u32,
        /// Capture timestamp (ns, device clock).
        pub ts_ns: u64,
        /// capture+scale cost — same provenance as ShotAck.take_ns.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub take_ns: Option<u64>,
        /// JPEG encode cost — same provenance as ShotAck.enc_ns.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub enc_ns: Option<u64>,
        /// Cumulative agent-side drop count at this frame.
        #[serde(default)]
        pub dropped: u64,
        pub data: ByteBuf,
    }

    /// ERROR-flagged response payload.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    pub struct ErrorAck {
        pub code: String,
        pub message: String,
    }

    pub fn encode<T: Serialize>(v: &T) -> Result<Vec<u8>, ProtocolError> {
        rmp_serde::to_vec_named(v).map_err(|e| ProtocolError::Codec(e.to_string()))
    }

    pub fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, ProtocolError> {
        rmp_serde::from_slice(bytes).map_err(|e| ProtocolError::Codec(e.to_string()))
    }
}

/// Capability bitmap bits (docs/03-device-agent.md §2).
/// Capability bitmap (`HELLO_ACK.caps`) — **the normative spec**. The Java
/// agent's `CAP_*` constants must match these bit-for-bit (enforced by the
/// `caps_bits_are_the_wire_contract` test). Semantics: one bit per
/// *feature/opcode group*, NOT per injection backend — backend selection is
/// the agent's internal concern.
pub mod caps {
    /// `INJECT` opcode served (MotionEvent sequences / taps / swipes).
    pub const INJECT: u32 = 1 << 0;
    /// `DUMP` opcode served (UI hierarchy XML via UiAutomation).
    pub const DUMP: u32 = 1 << 1;
    /// `SCREENSHOT` opcode served (`takeScreenshot` or later backends).
    pub const SHOT: u32 = 1 << 2;
    /// `QUERY` opcode served (selector find, M2).
    pub const QUERY: u32 = 1 << 3;
    /// `BATCH` opcode served (multi-op sequences, M2).
    pub const BATCH: u32 = 1 << 4;
    /// `STREAM_CTRL` served (frame/event push on the media channel).
    pub const STREAM: u32 = 1 << 5;
    /// `TEXT` opcode served (IME text input).
    pub const TEXT: u32 = 1 << 6;
}
