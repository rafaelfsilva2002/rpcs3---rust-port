//! `rpcs3-hle-cellpamf` — PAMF reader HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellPamf.cpp`. PAMF (PlayStation
//! Application Media Format) is the container SCE games use for
//! prerendered video cutscenes: a muxed stream of AVC / M2V video +
//! ATRAC3+ / LPCM / AC3 audio + user data + PSMF blocks.
//!
//! The HLE surface focuses on:
//!
//! 1. `cellPamfReaderInitialize(header, size, attr)` — validate the
//!    PAMF header.
//! 2. `cellPamfReaderGetNumberOfStreams` / `SetStream` — iterate streams.
//! 3. `cellPamfStreamTypeToEsFilterId` — translate semantic type into
//!    an ES filter id.
//! 4. `cellPamfReaderGetStreamInfo` — return AVC / M2V / audio metadata.
//!
//! ## Entry points covered
//!
//! | HLE function                           | Rust wrapper                          |
//! |----------------------------------------|---------------------------------------|
//! | `cellPamfReaderInitialize`             | [`PamfReader::initialize`]            |
//! | `cellPamfReaderGetNumberOfStreams`     | [`PamfReader::number_of_streams`]     |
//! | `cellPamfReaderGetNumberOfSpecificStreams` | [`PamfReader::number_of_specific_streams`] |
//! | `cellPamfReaderSetStream`              | [`PamfReader::set_stream`]            |
//! | `cellPamfReaderGetCurrentStreamNumber` | [`PamfReader::current_stream_number`] |
//! | `cellPamfReaderGetHeader`              | [`PamfReader::header`]                |
//! | `cellPamfStreamTypeToEsFilterId`       | [`stream_type_to_es_filter_id`]       |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellPamf.h:7-17
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const STREAM_NOT_FOUND: CellError = CellError(0x8061_0501);
    pub const INVALID_PAMF: CellError = CellError(0x8061_0502);
    pub const INVALID_ARG: CellError = CellError(0x8061_0503);
    pub const UNKNOWN_TYPE: CellError = CellError(0x8061_0504);
    pub const UNSUPPORTED_VERSION: CellError = CellError(0x8061_0505);
    pub const UNKNOWN_STREAM: CellError = CellError(0x8061_0506);
    pub const EP_NOT_FOUND: CellError = CellError(0x8061_0507);
    pub const NOT_AVAILABLE: CellError = CellError(0x8061_0508);
}

// =====================================================================
// Attribute flags (cellPamf.h:19-24)
// =====================================================================

pub const ATTRIBUTE_VERIFY_ON: u32 = 1;
pub const ATTRIBUTE_MINIMUM_HEADER: u32 = 2;
pub const ATTRIBUTE_ALL_MASK: u32 = 0x3;

// =====================================================================
// Stream type / coding type (cellPamf.h:26-52)
// =====================================================================

pub const STREAM_TYPE_AVC: i32 = 0;
pub const STREAM_TYPE_M2V: i32 = 1;
pub const STREAM_TYPE_ATRAC3PLUS: i32 = 2;
pub const STREAM_TYPE_PAMF_LPCM: i32 = 3;
pub const STREAM_TYPE_AC3: i32 = 4;
pub const STREAM_TYPE_USER_DATA: i32 = 5;
pub const STREAM_TYPE_PSMF_AVC: i32 = 6;
pub const STREAM_TYPE_PSMF_ATRAC3PLUS: i32 = 7;
pub const STREAM_TYPE_PSMF_LPCM: i32 = 8;
pub const STREAM_TYPE_PSMF_USER_DATA: i32 = 9;
pub const STREAM_TYPE_VIDEO: i32 = 20; // any video (AVC or M2V)
pub const STREAM_TYPE_AUDIO: i32 = 21; // any audio
pub const STREAM_TYPE_UNK: i32 = 22;

#[must_use]
pub fn is_known_stream_type(t: i32) -> bool {
    matches!(
        t,
        STREAM_TYPE_AVC
            | STREAM_TYPE_M2V
            | STREAM_TYPE_ATRAC3PLUS
            | STREAM_TYPE_PAMF_LPCM
            | STREAM_TYPE_AC3
            | STREAM_TYPE_USER_DATA
            | STREAM_TYPE_PSMF_AVC
            | STREAM_TYPE_PSMF_ATRAC3PLUS
            | STREAM_TYPE_PSMF_LPCM
            | STREAM_TYPE_PSMF_USER_DATA
            | STREAM_TYPE_VIDEO
            | STREAM_TYPE_AUDIO
            | STREAM_TYPE_UNK
    )
}

pub const CODING_TYPE_M2V: u8 = 0x02;
pub const CODING_TYPE_AVC: u8 = 0x1b;
pub const CODING_TYPE_PAMF_LPCM: u8 = 0x80;
pub const CODING_TYPE_AC3: u8 = 0x81;
pub const CODING_TYPE_ATRAC3PLUS: u8 = 0xdc;
pub const CODING_TYPE_USER_DATA: u8 = 0xdd;
pub const CODING_TYPE_PSMF: u8 = 0xff;

#[must_use]
pub fn coding_type_to_stream_type(coding: u8) -> i32 {
    match coding {
        CODING_TYPE_AVC => STREAM_TYPE_AVC,
        CODING_TYPE_M2V => STREAM_TYPE_M2V,
        CODING_TYPE_ATRAC3PLUS => STREAM_TYPE_ATRAC3PLUS,
        CODING_TYPE_PAMF_LPCM => STREAM_TYPE_PAMF_LPCM,
        CODING_TYPE_AC3 => STREAM_TYPE_AC3,
        CODING_TYPE_USER_DATA => STREAM_TYPE_USER_DATA,
        CODING_TYPE_PSMF => STREAM_TYPE_PSMF_AVC,
        _ => STREAM_TYPE_UNK,
    }
}

// =====================================================================
// PAMF-specific audio / video constants
// =====================================================================

pub const FS_48KHZ: u8 = 1;

pub const BIT_LENGTH_16: u8 = 1;
pub const BIT_LENGTH_24: u8 = 3;

pub const AVC_PROFILE_MAIN: u8 = 77;
pub const AVC_PROFILE_HIGH: u8 = 100;

pub const AVC_LEVEL_2P1: u8 = 21;
pub const AVC_LEVEL_3P0: u8 = 30;
pub const AVC_LEVEL_3P1: u8 = 31;
pub const AVC_LEVEL_3P2: u8 = 32;
pub const AVC_LEVEL_4P1: u8 = 41;
pub const AVC_LEVEL_4P2: u8 = 42;

/// Frame-rate codes (AVC/M2V share the table; M2V values shifted by 1).
pub const AVC_FRC_24000DIV1001: u8 = 0;
pub const AVC_FRC_24: u8 = 1;
pub const AVC_FRC_25: u8 = 2;
pub const AVC_FRC_30000DIV1001: u8 = 3;
pub const AVC_FRC_30: u8 = 4;
pub const AVC_FRC_50: u8 = 5;
pub const AVC_FRC_60000DIV1001: u8 = 6;

pub const M2V_FRC_24000DIV1001: u8 = 1;
pub const M2V_FRC_24: u8 = 2;
pub const M2V_FRC_25: u8 = 3;
pub const M2V_FRC_30000DIV1001: u8 = 4;
pub const M2V_FRC_30: u8 = 5;
pub const M2V_FRC_50: u8 = 6;
pub const M2V_FRC_60000DIV1001: u8 = 7;

/// PAMF magic — first 4 bytes of a valid container header.
pub const MAGIC: &[u8; 4] = b"PAMF";

pub const SUPPORTED_VERSION: u16 = 0x01_00; // 1.0

/// Maximum streams per PAMF per the real spec (ES stream filter has
/// 16 slots; games never multiplex more than a handful).
pub const MAX_STREAMS: usize = 16;

// =====================================================================
// Data types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeStamp {
    pub upper: u32,
    pub lower: u32,
}

impl TimeStamp {
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        ((self.upper as u64) << 32) | self.lower as u64
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvcStreamInfo {
    pub profile: u8,
    pub level: u8,
    pub frame_rate_code: u8,
    pub width: u16,
    pub height: u16,
    pub aspect_ratio: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct M2vStreamInfo {
    pub profile: u8,
    pub frame_rate_code: u8,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioStreamInfo {
    pub channels: u8,
    pub fs_code: u8, // FS_48KHZ
    pub bit_length: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamInfo {
    Avc(AvcStreamInfo),
    M2v(M2vStreamInfo),
    Audio(AudioStreamInfo),
    UserData,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    pub coding_type: u8,
    pub stream_id: u8,
    pub info: StreamInfo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PamfHeader {
    pub version: u16,
    pub start_pts: TimeStamp,
    pub end_pts: TimeStamp,
    pub number_of_streams: u32,
}

// =====================================================================
// PamfReader — the main reader FSM
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
enum ReaderState {
    Uninitialized,
    Open,
}

#[derive(Clone, Debug)]
pub struct PamfReader {
    state: ReaderState,
    attribute: u32,
    header: PamfHeader,
    streams: Vec<Stream>,
    current_stream: Option<u32>,
}

impl PamfReader {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: ReaderState::Uninitialized,
            attribute: 0,
            header: PamfHeader {
                version: 0,
                start_pts: TimeStamp { upper: 0, lower: 0 },
                end_pts: TimeStamp { upper: 0, lower: 0 },
                number_of_streams: 0,
            },
            streams: Vec::new(),
            current_stream: None,
        }
    }

    /// `cellPamfReaderInitialize(header_bytes, size, attr)`.
    pub fn initialize(&mut self, header: &[u8], attribute: u32) -> Result<(), CellError> {
        if (attribute & !ATTRIBUTE_ALL_MASK) != 0 {
            return Err(errors::INVALID_ARG);
        }
        if header.len() < 8 {
            return Err(errors::INVALID_PAMF);
        }
        if &header[..4] != MAGIC {
            return Err(errors::INVALID_PAMF);
        }
        let version = u16::from_be_bytes([header[4], header[5]]);
        if version != SUPPORTED_VERSION {
            return Err(errors::UNSUPPORTED_VERSION);
        }
        self.state = ReaderState::Open;
        self.attribute = attribute;
        self.header = PamfHeader {
            version,
            start_pts: TimeStamp { upper: 0, lower: 0 },
            end_pts: TimeStamp { upper: 0, lower: 0 },
            number_of_streams: 0,
        };
        self.streams.clear();
        self.current_stream = None;
        Ok(())
    }

    /// Admin-side: populate the stream table. Real lib parses it from
    /// the header bytes; tests inject streams directly.
    pub fn set_streams(&mut self, streams: Vec<Stream>) -> Result<(), CellError> {
        self.require_open()?;
        if streams.len() > MAX_STREAMS {
            return Err(errors::INVALID_PAMF);
        }
        self.streams = streams;
        self.header.number_of_streams = u32::try_from(self.streams.len()).unwrap_or(u32::MAX);
        if !self.streams.is_empty() {
            self.current_stream = Some(0);
        }
        Ok(())
    }

    pub fn set_time_stamps(&mut self, start: TimeStamp, end: TimeStamp) -> Result<(), CellError> {
        self.require_open()?;
        if end.as_u64() < start.as_u64() {
            return Err(errors::INVALID_PAMF);
        }
        self.header.start_pts = start;
        self.header.end_pts = end;
        Ok(())
    }

    pub fn header(&self) -> Result<&PamfHeader, CellError> {
        self.require_open()?;
        Ok(&self.header)
    }

    pub fn number_of_streams(&self) -> Result<u32, CellError> {
        self.require_open()?;
        Ok(self.header.number_of_streams)
    }

    /// `cellPamfReaderGetNumberOfSpecificStreams(type)`. Counts streams
    /// matching the semantic `type` filter.
    pub fn number_of_specific_streams(&self, stream_type: i32) -> Result<u32, CellError> {
        self.require_open()?;
        if !is_known_stream_type(stream_type) {
            return Err(errors::UNKNOWN_TYPE);
        }
        let count = self
            .streams
            .iter()
            .filter(|s| stream_matches_type(s, stream_type))
            .count();
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// `cellPamfReaderSetStream(index)`. Positions the "current stream"
    /// pointer by absolute index.
    pub fn set_stream(&mut self, index: u32) -> Result<(), CellError> {
        self.require_open()?;
        if index as usize >= self.streams.len() {
            return Err(errors::STREAM_NOT_FOUND);
        }
        self.current_stream = Some(index);
        Ok(())
    }

    /// `cellPamfReaderSetStreamWithIndex(type, index)`. Positions the
    /// cursor to the `index`-th stream of `type`.
    pub fn set_stream_with_index(&mut self, stream_type: i32, index: u32) -> Result<(), CellError> {
        self.require_open()?;
        if !is_known_stream_type(stream_type) {
            return Err(errors::UNKNOWN_TYPE);
        }
        let pos = self
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| stream_matches_type(s, stream_type))
            .nth(index as usize)
            .ok_or(errors::STREAM_NOT_FOUND)?
            .0;
        self.current_stream = Some(u32::try_from(pos).unwrap_or(u32::MAX));
        Ok(())
    }

    pub fn current_stream_number(&self) -> Result<u32, CellError> {
        self.require_open()?;
        self.current_stream.ok_or(errors::STREAM_NOT_FOUND)
    }

    pub fn current_stream(&self) -> Result<&Stream, CellError> {
        let idx = self.current_stream_number()? as usize;
        self.streams.get(idx).ok_or(errors::STREAM_NOT_FOUND)
    }

    pub fn stream_at(&self, index: u32) -> Result<&Stream, CellError> {
        self.require_open()?;
        self.streams.get(index as usize).ok_or(errors::STREAM_NOT_FOUND)
    }

    fn require_open(&self) -> Result<(), CellError> {
        if self.state == ReaderState::Open {
            Ok(())
        } else {
            Err(errors::NOT_AVAILABLE)
        }
    }
}

impl Default for PamfReader {
    fn default() -> Self {
        Self::new()
    }
}

fn stream_matches_type(stream: &Stream, filter: i32) -> bool {
    let direct = coding_type_to_stream_type(stream.coding_type);
    if filter == direct {
        return true;
    }
    match filter {
        STREAM_TYPE_VIDEO => matches!(direct, STREAM_TYPE_AVC | STREAM_TYPE_M2V),
        STREAM_TYPE_AUDIO => matches!(
            direct,
            STREAM_TYPE_ATRAC3PLUS | STREAM_TYPE_PAMF_LPCM | STREAM_TYPE_AC3
        ),
        _ => false,
    }
}

// =====================================================================
// ES filter id helper
// =====================================================================

/// `cellPamfStreamTypeToEsFilterId(type, ...)`. PAMF ES filter ids are
/// the demuxer's stream selector: matching the `coding_type` byte used
/// in the AVCHD payload stream header.
pub fn stream_type_to_es_filter_id(stream_type: i32) -> Result<u8, CellError> {
    Ok(match stream_type {
        STREAM_TYPE_AVC | STREAM_TYPE_PSMF_AVC => CODING_TYPE_AVC,
        STREAM_TYPE_M2V => CODING_TYPE_M2V,
        STREAM_TYPE_ATRAC3PLUS | STREAM_TYPE_PSMF_ATRAC3PLUS => CODING_TYPE_ATRAC3PLUS,
        STREAM_TYPE_PAMF_LPCM | STREAM_TYPE_PSMF_LPCM => CODING_TYPE_PAMF_LPCM,
        STREAM_TYPE_AC3 => CODING_TYPE_AC3,
        STREAM_TYPE_USER_DATA | STREAM_TYPE_PSMF_USER_DATA => CODING_TYPE_USER_DATA,
        STREAM_TYPE_VIDEO | STREAM_TYPE_AUDIO | STREAM_TYPE_UNK => {
            return Err(errors::UNKNOWN_TYPE);
        }
        _ => return Err(errors::UNKNOWN_TYPE),
    })
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_header() -> [u8; 8] {
        let mut h = [0u8; 8];
        h[..4].copy_from_slice(MAGIC);
        h[4..6].copy_from_slice(&SUPPORTED_VERSION.to_be_bytes());
        h
    }

    fn avc_stream() -> Stream {
        Stream {
            coding_type: CODING_TYPE_AVC,
            stream_id: 0xE0,
            info: StreamInfo::Avc(AvcStreamInfo {
                profile: AVC_PROFILE_HIGH,
                level: AVC_LEVEL_4P1,
                frame_rate_code: AVC_FRC_30000DIV1001,
                width: 1280,
                height: 720,
                aspect_ratio: 14,
            }),
        }
    }

    fn atrac_stream() -> Stream {
        Stream {
            coding_type: CODING_TYPE_ATRAC3PLUS,
            stream_id: 0xBD,
            info: StreamInfo::Audio(AudioStreamInfo {
                channels: 2,
                fs_code: FS_48KHZ,
                bit_length: BIT_LENGTH_16,
            }),
        }
    }

    fn lpcm_stream() -> Stream {
        Stream {
            coding_type: CODING_TYPE_PAMF_LPCM,
            stream_id: 0xBD,
            info: StreamInfo::Audio(AudioStreamInfo {
                channels: 6,
                fs_code: FS_48KHZ,
                bit_length: BIT_LENGTH_24,
            }),
        }
    }

    fn userdata_stream() -> Stream {
        Stream { coding_type: CODING_TYPE_USER_DATA, stream_id: 0xBF, info: StreamInfo::UserData }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::STREAM_NOT_FOUND.0, 0x8061_0501);
        assert_eq!(errors::INVALID_PAMF.0, 0x8061_0502);
        assert_eq!(errors::INVALID_ARG.0, 0x8061_0503);
        assert_eq!(errors::UNKNOWN_TYPE.0, 0x8061_0504);
        assert_eq!(errors::UNSUPPORTED_VERSION.0, 0x8061_0505);
        assert_eq!(errors::UNKNOWN_STREAM.0, 0x8061_0506);
        assert_eq!(errors::EP_NOT_FOUND.0, 0x8061_0507);
        assert_eq!(errors::NOT_AVAILABLE.0, 0x8061_0508);
    }

    #[test]
    fn attribute_flags_stable() {
        assert_eq!(ATTRIBUTE_VERIFY_ON, 1);
        assert_eq!(ATTRIBUTE_MINIMUM_HEADER, 2);
        assert_eq!(ATTRIBUTE_ALL_MASK, 3);
    }

    #[test]
    fn stream_type_constants_stable() {
        assert_eq!(STREAM_TYPE_AVC, 0);
        assert_eq!(STREAM_TYPE_ATRAC3PLUS, 2);
        assert_eq!(STREAM_TYPE_PAMF_LPCM, 3);
        assert_eq!(STREAM_TYPE_AC3, 4);
        assert_eq!(STREAM_TYPE_USER_DATA, 5);
        assert_eq!(STREAM_TYPE_VIDEO, 20);
        assert_eq!(STREAM_TYPE_AUDIO, 21);
        assert_eq!(STREAM_TYPE_UNK, 22);
    }

    #[test]
    fn coding_type_constants_stable() {
        assert_eq!(CODING_TYPE_M2V, 0x02);
        assert_eq!(CODING_TYPE_AVC, 0x1b);
        assert_eq!(CODING_TYPE_PAMF_LPCM, 0x80);
        assert_eq!(CODING_TYPE_AC3, 0x81);
        assert_eq!(CODING_TYPE_ATRAC3PLUS, 0xdc);
        assert_eq!(CODING_TYPE_USER_DATA, 0xdd);
        assert_eq!(CODING_TYPE_PSMF, 0xff);
    }

    #[test]
    fn avc_profile_level_constants_stable() {
        assert_eq!(AVC_PROFILE_MAIN, 77);
        assert_eq!(AVC_PROFILE_HIGH, 100);
        assert_eq!(AVC_LEVEL_4P1, 41);
    }

    #[test]
    fn coding_to_stream_type_map() {
        assert_eq!(coding_type_to_stream_type(CODING_TYPE_AVC), STREAM_TYPE_AVC);
        assert_eq!(coding_type_to_stream_type(CODING_TYPE_M2V), STREAM_TYPE_M2V);
        assert_eq!(coding_type_to_stream_type(CODING_TYPE_ATRAC3PLUS), STREAM_TYPE_ATRAC3PLUS);
        assert_eq!(coding_type_to_stream_type(CODING_TYPE_PAMF_LPCM), STREAM_TYPE_PAMF_LPCM);
        assert_eq!(coding_type_to_stream_type(CODING_TYPE_AC3), STREAM_TYPE_AC3);
        assert_eq!(coding_type_to_stream_type(CODING_TYPE_USER_DATA), STREAM_TYPE_USER_DATA);
        assert_eq!(coding_type_to_stream_type(CODING_TYPE_PSMF), STREAM_TYPE_PSMF_AVC);
        assert_eq!(coding_type_to_stream_type(0xAB), STREAM_TYPE_UNK);
    }

    #[test]
    fn stream_type_to_es_filter_id_covers_known() {
        assert_eq!(stream_type_to_es_filter_id(STREAM_TYPE_AVC), Ok(CODING_TYPE_AVC));
        assert_eq!(stream_type_to_es_filter_id(STREAM_TYPE_M2V), Ok(CODING_TYPE_M2V));
        assert_eq!(stream_type_to_es_filter_id(STREAM_TYPE_ATRAC3PLUS), Ok(CODING_TYPE_ATRAC3PLUS));
        assert_eq!(stream_type_to_es_filter_id(STREAM_TYPE_AC3), Ok(CODING_TYPE_AC3));
        assert_eq!(stream_type_to_es_filter_id(STREAM_TYPE_USER_DATA), Ok(CODING_TYPE_USER_DATA));
    }

    #[test]
    fn stream_type_to_es_filter_id_virtual_types_rejected() {
        assert_eq!(stream_type_to_es_filter_id(STREAM_TYPE_VIDEO), Err(errors::UNKNOWN_TYPE));
        assert_eq!(stream_type_to_es_filter_id(STREAM_TYPE_AUDIO), Err(errors::UNKNOWN_TYPE));
        assert_eq!(stream_type_to_es_filter_id(STREAM_TYPE_UNK), Err(errors::UNKNOWN_TYPE));
        assert_eq!(stream_type_to_es_filter_id(999), Err(errors::UNKNOWN_TYPE));
    }

    #[test]
    fn initialize_rejects_bad_attribute_flags() {
        let mut r = PamfReader::new();
        let h = valid_header();
        assert_eq!(r.initialize(&h, 0xFF), Err(errors::INVALID_ARG));
    }

    #[test]
    fn initialize_rejects_short_header() {
        let mut r = PamfReader::new();
        assert_eq!(r.initialize(&[0u8; 4], 0), Err(errors::INVALID_PAMF));
    }

    #[test]
    fn initialize_rejects_bad_magic() {
        let mut r = PamfReader::new();
        let mut h = [0u8; 8];
        h[..4].copy_from_slice(b"XXXX");
        h[4..6].copy_from_slice(&SUPPORTED_VERSION.to_be_bytes());
        assert_eq!(r.initialize(&h, 0), Err(errors::INVALID_PAMF));
    }

    #[test]
    fn initialize_rejects_unsupported_version() {
        let mut r = PamfReader::new();
        let mut h = valid_header();
        h[4..6].copy_from_slice(&0xFFFFu16.to_be_bytes());
        assert_eq!(r.initialize(&h, 0), Err(errors::UNSUPPORTED_VERSION));
    }

    #[test]
    fn initialize_happy_path() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), ATTRIBUTE_VERIFY_ON).unwrap();
        assert_eq!(r.number_of_streams(), Ok(0));
    }

    #[test]
    fn queries_before_init_return_not_available() {
        let r = PamfReader::new();
        assert_eq!(r.number_of_streams(), Err(errors::NOT_AVAILABLE));
        assert_eq!(r.header().err(), Some(errors::NOT_AVAILABLE));
        assert_eq!(r.current_stream_number(), Err(errors::NOT_AVAILABLE));
    }

    #[test]
    fn set_streams_populates_table_and_cursor() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream(), atrac_stream()]).unwrap();
        assert_eq!(r.number_of_streams(), Ok(2));
        assert_eq!(r.current_stream_number(), Ok(0));
    }

    #[test]
    fn set_streams_over_max_rejected() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        let big: Vec<_> = (0..=MAX_STREAMS).map(|_| avc_stream()).collect();
        assert_eq!(r.set_streams(big), Err(errors::INVALID_PAMF));
    }

    #[test]
    fn set_time_stamps_inverted_rejected() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        let s = TimeStamp { upper: 0, lower: 100 };
        let e = TimeStamp { upper: 0, lower: 50 };
        assert_eq!(r.set_time_stamps(s, e), Err(errors::INVALID_PAMF));
    }

    #[test]
    fn set_time_stamps_equal_ok() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_time_stamps(TimeStamp { upper: 0, lower: 50 }, TimeStamp { upper: 0, lower: 50 }).unwrap();
    }

    #[test]
    fn number_of_specific_streams_counts_by_type() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream(), atrac_stream(), lpcm_stream(), userdata_stream()]).unwrap();
        assert_eq!(r.number_of_specific_streams(STREAM_TYPE_AVC), Ok(1));
        assert_eq!(r.number_of_specific_streams(STREAM_TYPE_ATRAC3PLUS), Ok(1));
        assert_eq!(r.number_of_specific_streams(STREAM_TYPE_VIDEO), Ok(1));
        assert_eq!(r.number_of_specific_streams(STREAM_TYPE_AUDIO), Ok(2));
        assert_eq!(r.number_of_specific_streams(STREAM_TYPE_USER_DATA), Ok(1));
    }

    #[test]
    fn number_of_specific_streams_unknown_type_rejected() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        assert_eq!(r.number_of_specific_streams(99), Err(errors::UNKNOWN_TYPE));
    }

    #[test]
    fn set_stream_by_absolute_index() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream(), atrac_stream()]).unwrap();
        r.set_stream(1).unwrap();
        assert_eq!(r.current_stream_number(), Ok(1));
    }

    #[test]
    fn set_stream_out_of_range_rejected() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream()]).unwrap();
        assert_eq!(r.set_stream(5), Err(errors::STREAM_NOT_FOUND));
    }

    #[test]
    fn set_stream_with_index_filters_by_type() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream(), atrac_stream(), lpcm_stream()]).unwrap();
        // Second audio stream → LPCM at absolute index 2.
        r.set_stream_with_index(STREAM_TYPE_AUDIO, 1).unwrap();
        assert_eq!(r.current_stream_number(), Ok(2));
    }

    #[test]
    fn set_stream_with_index_not_found_rejected() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream()]).unwrap();
        assert_eq!(r.set_stream_with_index(STREAM_TYPE_AUDIO, 0), Err(errors::STREAM_NOT_FOUND));
    }

    #[test]
    fn set_stream_with_unknown_type_rejected() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream()]).unwrap();
        assert_eq!(r.set_stream_with_index(999, 0), Err(errors::UNKNOWN_TYPE));
    }

    #[test]
    fn stream_at_index_accessor() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream(), atrac_stream()]).unwrap();
        assert_eq!(r.stream_at(0).unwrap().coding_type, CODING_TYPE_AVC);
        assert_eq!(r.stream_at(1).unwrap().coding_type, CODING_TYPE_ATRAC3PLUS);
        assert_eq!(r.stream_at(2).err(), Some(errors::STREAM_NOT_FOUND));
    }

    #[test]
    fn timestamp_as_u64_packs_correctly() {
        let ts = TimeStamp { upper: 0x0001, lower: 0xABCD_1234 };
        assert_eq!(ts.as_u64(), 0x0000_0001_ABCD_1234);
    }

    #[test]
    fn reinitialize_clears_state() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), 0).unwrap();
        r.set_streams(vec![avc_stream(), atrac_stream()]).unwrap();
        r.initialize(&valid_header(), 0).unwrap();
        assert_eq!(r.number_of_streams(), Ok(0));
        assert_eq!(r.current_stream_number(), Err(errors::STREAM_NOT_FOUND));
    }

    #[test]
    fn full_pamf_reader_lifecycle_smoke() {
        let mut r = PamfReader::new();
        r.initialize(&valid_header(), ATTRIBUTE_VERIFY_ON).unwrap();
        r.set_streams(vec![avc_stream(), atrac_stream(), userdata_stream()]).unwrap();
        r.set_time_stamps(TimeStamp { upper: 0, lower: 0 }, TimeStamp { upper: 0, lower: 90_000 * 30 })
            .unwrap();
        assert_eq!(r.number_of_streams(), Ok(3));
        assert_eq!(r.number_of_specific_streams(STREAM_TYPE_VIDEO), Ok(1));
        r.set_stream_with_index(STREAM_TYPE_AUDIO, 0).unwrap();
        assert_eq!(r.current_stream_number(), Ok(1));
        let cur = r.current_stream().unwrap();
        assert_eq!(cur.coding_type, CODING_TYPE_ATRAC3PLUS);
        let h = r.header().unwrap();
        assert_eq!(h.number_of_streams, 3);
        assert_eq!(h.end_pts.lower, 90_000 * 30);
    }
}
