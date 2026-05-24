//! `rpcs3-hle-celladec` — generic audio decoder framework HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellAdec.cpp`. The Adec API is a
//! dispatch layer that multiplexes across actual audio codec
//! decoders (MP3, AAC, AC3, DTS, ATRAC3/+, LPCM, CELP, TrueHD, ...).
//! Games open a decoder for a specific codec type, push access
//! units (encoded frames), decode, and pop PCM output.
//!
//! ## Entry points covered
//!
//! | HLE function            | Rust wrapper                 |
//! |-------------------------|------------------------------|
//! | `cellAdecQueryAttr`     | [`cell_adec_query_attr`]     |
//! | `cellAdecOpen`          | [`cell_adec_open`]           |
//! | `cellAdecClose`         | [`cell_adec_close`]          |
//! | `cellAdecStartSeq`      | [`cell_adec_start_seq`]      |
//! | `cellAdecEndSeq`        | [`cell_adec_end_seq`]        |
//! | `cellAdecDecodeAu`      | [`cell_adec_decode_au`]      |
//! | `cellAdecGetPcm`        | [`cell_adec_get_pcm`]        |
//! | `cellAdecGetPcmItem`    | [`cell_adec_get_pcm_item`]   |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes (generic, from cellAdec.h:8-14)
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const FATAL: CellError = CellError(0x8061_0001);
    pub const SEQ: CellError = CellError(0x8061_0002);
    pub const ARG: CellError = CellError(0x8061_0003);
    pub const BUSY: CellError = CellError(0x8061_0004);
    pub const EMPTY: CellError = CellError(0x8061_0005);

    // Codec-specific errors (subset).
    pub const M4AAC_FATAL: CellError = CellError(0x8061_2401);
    pub const M4AAC_SEQ: CellError = CellError(0x8061_2402);
    pub const M4AAC_ARG: CellError = CellError(0x8061_2403);
    pub const M4AAC_EMPTY: CellError = CellError(0x8061_2405);
    pub const CELP_SEQ: CellError = CellError(0x8061_2E04);
}

// =====================================================================
// Codec-type constants (byte-exact vs cellAdec.h:193+)
// =====================================================================

pub const CODEC_INVALID1: u32 = 0;
pub const CODEC_LPCM_PAMF: u32 = 1;
pub const CODEC_AC3: u32 = 2;
pub const CODEC_ATRACX: u32 = 3;
pub const CODEC_MP3: u32 = 4;
pub const CODEC_ATRAC3: u32 = 5;
pub const CODEC_MPEG_L2: u32 = 6;
pub const CODEC_M2AAC: u32 = 7;
pub const CODEC_EAC3: u32 = 8;
pub const CODEC_TRUEHD: u32 = 9;
pub const CODEC_DTS: u32 = 10;
pub const CODEC_CELP: u32 = 11;
pub const CODEC_LPCM_BLURAY: u32 = 12;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attr {
    pub work_mem_size: u32,
    pub adec_ver: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenParam {
    pub codec_type: u32,
    pub channel_num: u32,
    pub sampling_rate: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessUnit {
    /// Guest address of input AU bytes.
    pub start_addr: u32,
    /// Size of this AU in bytes.
    pub size: u32,
    /// Presentation timestamp (90 kHz units, split into u64).
    pub pts: u64,
    pub user_data: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PcmFrame {
    /// Interleaved f32 samples (L,R,L,R,...).
    pub samples: Vec<f32>,
    pub channel_num: u32,
    pub sampling_rate: u32,
    pub pts: u64,
}

// =====================================================================
// Backend trait
// =====================================================================

pub trait AdecDecoder {
    /// Report the memory requirements for this codec.
    fn query(&self, codec_type: u32) -> Result<Attr, CellError>;
    /// Decode a single access unit.
    fn decode(
        &mut self,
        codec_type: u32,
        open_param: &OpenParam,
        au_bytes: &[u8],
        pts: u64,
    ) -> Result<PcmFrame, CellError>;
}

/// Stub decoder that emits silent frames sized by the input AU.
#[derive(Debug, Default)]
pub struct StubAdecDecoder {
    /// How many samples to emit per AU (per channel). Defaults to 1024.
    pub samples_per_au: u32,
}

impl StubAdecDecoder {
    pub fn new() -> Self { Self { samples_per_au: 1024 } }
}

impl AdecDecoder for StubAdecDecoder {
    fn query(&self, codec_type: u32) -> Result<Attr, CellError> {
        if !is_known_codec(codec_type) {
            return Err(errors::ARG);
        }
        Ok(Attr {
            work_mem_size: 0x20_0000,  // 2 MB sufficient for most codecs
            adec_ver: 0x0002_0000,
        })
    }

    fn decode(
        &mut self,
        codec_type: u32,
        open_param: &OpenParam,
        au_bytes: &[u8],
        pts: u64,
    ) -> Result<PcmFrame, CellError> {
        if !is_known_codec(codec_type) {
            return Err(errors::ARG);
        }
        if au_bytes.is_empty() {
            return Err(errors::EMPTY);
        }
        let samples_total = (self.samples_per_au * open_param.channel_num) as usize;
        Ok(PcmFrame {
            samples: vec![0.0; samples_total],
            channel_num: open_param.channel_num,
            sampling_rate: open_param.sampling_rate,
            pts,
        })
    }
}

pub fn is_known_codec(c: u32) -> bool {
    matches!(
        c,
        CODEC_LPCM_PAMF | CODEC_AC3 | CODEC_ATRACX | CODEC_MP3
        | CODEC_ATRAC3 | CODEC_MPEG_L2 | CODEC_M2AAC | CODEC_EAC3
        | CODEC_TRUEHD | CODEC_DTS | CODEC_CELP | CODEC_LPCM_BLURAY,
    )
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum DecoderState {
    Closed,
    Open,
    InSequence,
}

#[derive(Debug, Clone)]
struct DecoderHandle {
    state: DecoderState,
    open_param: OpenParam,
    pending_pcm: std::collections::VecDeque<PcmFrame>,
}

#[derive(Debug, Default)]
pub struct AdecManager {
    handles: std::collections::BTreeMap<u32, DecoderHandle>,
    next_id: u32,
}

// =====================================================================
// Syscalls
// =====================================================================

/// `cellAdecQueryAttr(type, attr_out)`.
#[must_use]
pub fn cell_adec_query_attr<D: AdecDecoder + ?Sized>(
    decoder: &D,
    codec_type: u32,
) -> Result<Attr, CellError> {
    decoder.query(codec_type)
}

/// `cellAdecOpen(type, res, cb, handle_out)`.
#[must_use]
pub fn cell_adec_open(
    m: &mut AdecManager,
    param: OpenParam,
) -> Result<u32, CellError> {
    if !is_known_codec(param.codec_type) {
        return Err(errors::ARG);
    }
    if param.channel_num == 0 || param.channel_num > 8 {
        return Err(errors::ARG);
    }
    if param.sampling_rate == 0 {
        return Err(errors::ARG);
    }
    m.next_id += 1;
    let id = m.next_id;
    m.handles.insert(
        id,
        DecoderHandle {
            state: DecoderState::Open,
            open_param: param,
            pending_pcm: Default::default(),
        },
    );
    Ok(id)
}

#[must_use]
pub fn cell_adec_close(m: &mut AdecManager, handle: u32) -> Result<(), CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::ARG)?;
    if h.state == DecoderState::InSequence {
        // Must end sequence first.
        return Err(errors::SEQ);
    }
    m.handles.remove(&handle);
    Ok(())
}

#[must_use]
pub fn cell_adec_start_seq(m: &mut AdecManager, handle: u32) -> Result<(), CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::ARG)?;
    match h.state {
        DecoderState::Open => {
            h.state = DecoderState::InSequence;
            Ok(())
        }
        _ => Err(errors::SEQ),
    }
}

#[must_use]
pub fn cell_adec_end_seq(m: &mut AdecManager, handle: u32) -> Result<(), CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::ARG)?;
    match h.state {
        DecoderState::InSequence => {
            h.state = DecoderState::Open;
            h.pending_pcm.clear();
            Ok(())
        }
        _ => Err(errors::SEQ),
    }
}

/// `cellAdecDecodeAu(handle, au, pts, user_data, au_bytes)`. The
/// backend must produce a [`PcmFrame`] that's queued for later
/// [`cell_adec_get_pcm`] / [`cell_adec_get_pcm_item`] retrieval.
#[must_use]
pub fn cell_adec_decode_au<D: AdecDecoder + ?Sized>(
    m: &mut AdecManager,
    decoder: &mut D,
    handle: u32,
    au: AccessUnit,
    au_bytes: &[u8],
) -> Result<(), CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::ARG)?;
    if h.state != DecoderState::InSequence {
        return Err(errors::SEQ);
    }
    if au_bytes.len() as u32 != au.size {
        return Err(errors::ARG);
    }
    let frame = decoder.decode(h.open_param.codec_type, &h.open_param, au_bytes, au.pts)?;
    h.pending_pcm.push_back(frame);
    Ok(())
}

#[must_use]
pub fn cell_adec_get_pcm(
    m: &mut AdecManager,
    handle: u32,
) -> Result<PcmFrame, CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::ARG)?;
    if h.state != DecoderState::InSequence {
        return Err(errors::SEQ);
    }
    h.pending_pcm.pop_front().ok_or(errors::EMPTY)
}

/// Same as [`cell_adec_get_pcm`] but with full metadata (PCM + item).
#[must_use]
pub fn cell_adec_get_pcm_item(
    m: &mut AdecManager,
    handle: u32,
) -> Result<(PcmFrame, u32), CellError> {
    let frame = cell_adec_get_pcm(m, handle)?;
    let remaining = m
        .handles
        .get(&handle)
        .map_or(0, |h| h.pending_pcm.len() as u32);
    Ok((frame, remaining))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_open_param(codec: u32) -> OpenParam {
        OpenParam { codec_type: codec, channel_num: 2, sampling_rate: 48000 }
    }

    fn stub_au() -> AccessUnit {
        AccessUnit { start_addr: 0x1000, size: 256, pts: 123_456, user_data: 0 }
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::FATAL.0, 0x8061_0001);
        assert_eq!(errors::SEQ.0, 0x8061_0002);
        assert_eq!(errors::ARG.0, 0x8061_0003);
        assert_eq!(errors::BUSY.0, 0x8061_0004);
        assert_eq!(errors::EMPTY.0, 0x8061_0005);
        assert_eq!(errors::M4AAC_SEQ.0, 0x8061_2402);
        assert_eq!(errors::CELP_SEQ.0, 0x8061_2E04);
    }

    #[test]
    fn codec_type_constants_stable() {
        assert_eq!(CODEC_INVALID1, 0);
        assert_eq!(CODEC_LPCM_PAMF, 1);
        assert_eq!(CODEC_AC3, 2);
        assert_eq!(CODEC_MP3, 4);
        assert_eq!(CODEC_ATRAC3, 5);
        assert_eq!(CODEC_M2AAC, 7);
        assert_eq!(CODEC_DTS, 10);
        assert_eq!(CODEC_LPCM_BLURAY, 12);
    }

    #[test]
    fn is_known_codec_accepts_all_12() {
        for c in [CODEC_LPCM_PAMF, CODEC_AC3, CODEC_ATRACX, CODEC_MP3, CODEC_ATRAC3,
                  CODEC_MPEG_L2, CODEC_M2AAC, CODEC_EAC3, CODEC_TRUEHD, CODEC_DTS,
                  CODEC_CELP, CODEC_LPCM_BLURAY] {
            assert!(is_known_codec(c), "codec {c}");
        }
    }

    #[test]
    fn is_known_codec_rejects_invalid1_and_unknown() {
        assert!(!is_known_codec(CODEC_INVALID1));
        assert!(!is_known_codec(0xFF));
    }

    // --- query ----------------------------------------------------

    #[test]
    fn query_attr_returns_memory_size() {
        let d = StubAdecDecoder::new();
        let attr = cell_adec_query_attr(&d, CODEC_MP3).unwrap();
        assert!(attr.work_mem_size >= 0x10_0000);
    }

    #[test]
    fn query_attr_invalid_codec_is_arg() {
        let d = StubAdecDecoder::new();
        assert_eq!(cell_adec_query_attr(&d, 0xFF).unwrap_err(), errors::ARG);
    }

    // --- open / close ---------------------------------------------

    #[test]
    fn open_returns_handle() {
        let mut m = AdecManager::default();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        assert_eq!(h, 1);
    }

    #[test]
    fn open_invalid_codec_is_arg() {
        let mut m = AdecManager::default();
        let p = OpenParam { codec_type: 0xFF, ..stub_open_param(0) };
        assert_eq!(cell_adec_open(&mut m, p).unwrap_err(), errors::ARG);
    }

    #[test]
    fn open_invalid_channels_is_arg() {
        let mut m = AdecManager::default();
        let p = OpenParam { channel_num: 0, ..stub_open_param(CODEC_MP3) };
        assert_eq!(cell_adec_open(&mut m, p).unwrap_err(), errors::ARG);
        let p = OpenParam { channel_num: 9, ..stub_open_param(CODEC_MP3) };
        assert_eq!(cell_adec_open(&mut m, p).unwrap_err(), errors::ARG);
    }

    #[test]
    fn open_zero_sample_rate_is_arg() {
        let mut m = AdecManager::default();
        let p = OpenParam { sampling_rate: 0, ..stub_open_param(CODEC_MP3) };
        assert_eq!(cell_adec_open(&mut m, p).unwrap_err(), errors::ARG);
    }

    #[test]
    fn close_unknown_handle_is_arg() {
        let mut m = AdecManager::default();
        assert_eq!(cell_adec_close(&mut m, 999).unwrap_err(), errors::ARG);
    }

    #[test]
    fn close_during_sequence_is_seq() {
        let mut m = AdecManager::default();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        assert_eq!(cell_adec_close(&mut m, h).unwrap_err(), errors::SEQ);
    }

    // --- sequence lifecycle ---------------------------------------

    #[test]
    fn start_seq_twice_is_seq() {
        let mut m = AdecManager::default();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        assert_eq!(cell_adec_start_seq(&mut m, h).unwrap_err(), errors::SEQ);
    }

    #[test]
    fn end_seq_without_start_is_seq() {
        let mut m = AdecManager::default();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        assert_eq!(cell_adec_end_seq(&mut m, h).unwrap_err(), errors::SEQ);
    }

    #[test]
    fn start_end_round_trip() {
        let mut m = AdecManager::default();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        cell_adec_end_seq(&mut m, h).unwrap();
        cell_adec_close(&mut m, h).unwrap();
    }

    // --- decode ---------------------------------------------------

    #[test]
    fn decode_before_start_seq_is_seq() {
        let mut m = AdecManager::default();
        let mut d = StubAdecDecoder::new();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        let bytes = vec![0u8; 256];
        assert_eq!(
            cell_adec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap_err(),
            errors::SEQ,
        );
    }

    #[test]
    fn decode_size_mismatch_is_arg() {
        let mut m = AdecManager::default();
        let mut d = StubAdecDecoder::new();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        let au = AccessUnit { size: 100, ..stub_au() };
        let bytes = vec![0u8; 50];
        assert_eq!(
            cell_adec_decode_au(&mut m, &mut d, h, au, &bytes).unwrap_err(),
            errors::ARG,
        );
    }

    #[test]
    fn decode_queues_pcm_for_get() {
        let mut m = AdecManager::default();
        let mut d = StubAdecDecoder::new();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();

        let bytes = vec![0u8; 256];
        cell_adec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap();
        let pcm = cell_adec_get_pcm(&mut m, h).unwrap();
        assert_eq!(pcm.channel_num, 2);
        assert_eq!(pcm.sampling_rate, 48000);
        assert_eq!(pcm.samples.len(), 1024 * 2);  // 1024 samples * 2 channels
        assert_eq!(pcm.pts, 123_456);
    }

    #[test]
    fn get_pcm_on_empty_queue_is_empty() {
        let mut m = AdecManager::default();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        assert_eq!(cell_adec_get_pcm(&mut m, h).unwrap_err(), errors::EMPTY);
    }

    #[test]
    fn get_pcm_without_sequence_is_seq() {
        let mut m = AdecManager::default();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        assert_eq!(cell_adec_get_pcm(&mut m, h).unwrap_err(), errors::SEQ);
    }

    #[test]
    fn get_pcm_item_reports_remaining_count() {
        let mut m = AdecManager::default();
        let mut d = StubAdecDecoder::new();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();

        let bytes = vec![0u8; 256];
        for _ in 0..3 {
            cell_adec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap();
        }
        let (_pcm, remaining) = cell_adec_get_pcm_item(&mut m, h).unwrap();
        assert_eq!(remaining, 2);
    }

    #[test]
    fn end_seq_clears_pending_pcm() {
        let mut m = AdecManager::default();
        let mut d = StubAdecDecoder::new();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        let bytes = vec![0u8; 256];
        cell_adec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap();
        cell_adec_end_seq(&mut m, h).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        assert_eq!(cell_adec_get_pcm(&mut m, h).unwrap_err(), errors::EMPTY);
    }

    #[test]
    fn decode_empty_au_is_empty() {
        let mut m = AdecManager::default();
        let mut d = StubAdecDecoder::new();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        let au = AccessUnit { size: 0, ..stub_au() };
        assert_eq!(
            cell_adec_decode_au(&mut m, &mut d, h, au, &[]).unwrap_err(),
            errors::EMPTY,
        );
    }

    #[test]
    fn full_pipeline_open_start_decode_get_end_close() {
        let mut m = AdecManager::default();
        let mut d = StubAdecDecoder::new();
        let h = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        cell_adec_start_seq(&mut m, h).unwrap();
        let bytes = vec![0u8; 256];
        cell_adec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap();
        cell_adec_get_pcm(&mut m, h).unwrap();
        cell_adec_end_seq(&mut m, h).unwrap();
        cell_adec_close(&mut m, h).unwrap();
    }

    #[test]
    fn multiple_handles_isolated() {
        let mut m = AdecManager::default();
        let mut d = StubAdecDecoder::new();
        let h1 = cell_adec_open(&mut m, stub_open_param(CODEC_MP3)).unwrap();
        let h2 = cell_adec_open(&mut m, stub_open_param(CODEC_AC3)).unwrap();
        cell_adec_start_seq(&mut m, h1).unwrap();
        cell_adec_start_seq(&mut m, h2).unwrap();
        let bytes = vec![0u8; 256];
        cell_adec_decode_au(&mut m, &mut d, h1, stub_au(), &bytes).unwrap();
        // h2 queue should be empty.
        assert_eq!(cell_adec_get_pcm(&mut m, h2).unwrap_err(), errors::EMPTY);
        cell_adec_get_pcm(&mut m, h1).unwrap();
    }
}
