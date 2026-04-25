//! `rpcs3-hle-cellatrac` — ATRAC3/ATRAC3+ audio decoder HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellAtrac.cpp`. ATRAC is Sony's
//! psychoacoustic audio codec; PS3 games use it for music,
//! compressed voice, and background ambience. The HLE tracks the
//! decode state machine + buffer bookkeeping; actual PCM synthesis
//! plugs in via the [`AtracDecoder`] trait.
//!
//! ## Entry points covered
//!
//! | HLE function                      | Rust wrapper                      |
//! |-----------------------------------|-----------------------------------|
//! | `cellAtracSetData`                | [`cell_atrac_set_data`]           |
//! | `cellAtracDecode`                 | [`cell_atrac_decode_data`]        |
//! | `cellAtracGetStreamDataInfo`      | [`cell_atrac_get_stream_data_info`] |
//! | `cellAtracAddStreamData`          | [`cell_atrac_add_stream_data`]    |
//! | `cellAtracGetRemainFrame`         | [`cell_atrac_get_remain_frame`]   |
//! | `cellAtracGetSoundInfo`           | [`cell_atrac_get_sound_info`]     |
//! | `cellAtracGetMaxSample`           | [`cell_atrac_get_max_sample`]     |
//! | `cellAtracSetLoopNum`             | [`cell_atrac_set_loop_num`]       |
//! | `cellAtracGetLoopInfo`            | [`cell_atrac_get_loop_info`]      |
//! | `cellAtracResetPlayPosition`      | [`cell_atrac_reset_play_position`] |
//! | `cellAtracGetBitrate`             | [`cell_atrac_get_bitrate`]        |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellAtrac.h:7-28
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const API_FAIL: CellError = CellError(0x8061_0301);
    pub const READSIZE_OVER_BUFFER: CellError = CellError(0x8061_0311);
    pub const UNKNOWN_FORMAT: CellError = CellError(0x8061_0312);
    pub const READSIZE_IS_TOO_SMALL: CellError = CellError(0x8061_0313);
    pub const ILLEGAL_SAMPLING_RATE: CellError = CellError(0x8061_0314);
    pub const ILLEGAL_DATA: CellError = CellError(0x8061_0315);
    pub const NO_DECODER: CellError = CellError(0x8061_0321);
    pub const UNSET_DATA: CellError = CellError(0x8061_0322);
    pub const DECODER_WAS_CREATED: CellError = CellError(0x8061_0323);
    pub const ALLDATA_WAS_DECODED: CellError = CellError(0x8061_0331);
    pub const NODATA_IN_BUFFER: CellError = CellError(0x8061_0332);
    pub const NOT_ALIGNED_OUT_BUFFER: CellError = CellError(0x8061_0333);
    pub const NEED_SECOND_BUFFER: CellError = CellError(0x8061_0334);
    pub const ALLDATA_IS_ONMEMORY: CellError = CellError(0x8061_0341);
    pub const ADD_DATA_IS_TOO_BIG: CellError = CellError(0x8061_0342);
    pub const NONEED_SECOND_BUFFER: CellError = CellError(0x8061_0351);
    pub const UNSET_LOOP_NUM: CellError = CellError(0x8061_0361);
    pub const ILLEGAL_SAMPLE: CellError = CellError(0x8061_0371);
    pub const ILLEGAL_RESET_BYTE: CellError = CellError(0x8061_0372);
    pub const ILLEGAL_PPU_THREAD_PRIORITY: CellError = CellError(0x8061_0381);
    pub const ILLEGAL_SPU_THREAD_PRIORITY: CellError = CellError(0x8061_0382);
}

// =====================================================================
// Remain-frame sentinels
// =====================================================================

pub const REMAIN_ALLDATA_IS_ON_MEMORY: i32 = -1;
pub const REMAIN_NONLOOP_STREAM_ON_MEMORY: i32 = -2;
pub const REMAIN_LOOP_STREAM_ON_MEMORY: i32 = -3;

pub const HANDLE_SIZE: usize = 512;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoundInfo {
    pub channels: u32,
    pub sampling_rate: u32,
    pub bitrate: u32,
    pub total_samples: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamDataInfo {
    pub write_addr: u32,
    pub writable_bytes: u32,
    pub min_write_bytes: u32,
    pub read_position: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopInfo {
    pub loop_num: i32,
    pub loop_start_sample: u64,
    pub loop_end_sample: u64,
}

#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// PCM samples, interleaved. Length == samples * channels.
    pub pcm_i16: Vec<i16>,
    /// How many data bytes were consumed from the input stream.
    pub bytes_consumed: u32,
}

// =====================================================================
// Backend trait
// =====================================================================

pub trait AtracDecoder {
    fn sound_info(&self, data: &[u8]) -> Result<SoundInfo, CellError>;
    fn decode_frame(
        &mut self,
        data: &[u8],
        offset: u32,
    ) -> Result<DecodedFrame, CellError>;
}

/// Stub decoder for tests: parses a minimal header out of the input
/// bytes and emits canned silent PCM frames.
#[derive(Debug, Default)]
pub struct StubAtracDecoder {
    pub frame_samples: u32,
    pub frame_bytes: u32,
}

impl StubAtracDecoder {
    #[must_use]
    pub fn new() -> Self {
        Self { frame_samples: 1024, frame_bytes: 256 }
    }
}

impl AtracDecoder for StubAtracDecoder {
    fn sound_info(&self, data: &[u8]) -> Result<SoundInfo, CellError> {
        // Minimal sanity: need 16 bytes for a mock header.
        if data.len() < 16 {
            return Err(errors::UNKNOWN_FORMAT);
        }
        let sampling_rate = match data[0] {
            0 => 44100,
            1 => 48000,
            2 => 32000,
            _ => return Err(errors::ILLEGAL_SAMPLING_RATE),
        };
        let channels = match data[1] {
            1 => 1,
            2 => 2,
            _ => return Err(errors::ILLEGAL_DATA),
        };
        Ok(SoundInfo {
            channels,
            sampling_rate,
            bitrate: 128_000,
            total_samples: (data.len() as u64 / self.frame_bytes as u64) * self.frame_samples as u64,
        })
    }

    fn decode_frame(&mut self, data: &[u8], offset: u32) -> Result<DecodedFrame, CellError> {
        let off = offset as usize;
        if off + self.frame_bytes as usize > data.len() {
            return Err(errors::NODATA_IN_BUFFER);
        }
        // Silent stereo frame.
        Ok(DecodedFrame {
            pcm_i16: vec![0; (self.frame_samples * 2) as usize],
            bytes_consumed: self.frame_bytes,
        })
    }
}

// =====================================================================
// Manager state
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderState {
    /// After create but before `SetData`.
    Idle,
    /// Data pointer registered, ready to decode.
    Ready,
    /// All data decoded — subsequent Decode returns ALLDATA_WAS_DECODED.
    Exhausted,
}

#[derive(Debug)]
pub struct AtracManager {
    decoder_created: bool,
    state: DecoderState,
    data: Vec<u8>,
    read_position: u32,
    info: Option<SoundInfo>,
    loop_info: Option<LoopInfo>,
    remain_sentinel: i32,
    min_write_bytes: u32,
}

impl Default for AtracManager {
    fn default() -> Self {
        Self {
            decoder_created: false,
            state: DecoderState::Idle,
            data: Vec::new(),
            read_position: 0,
            info: None,
            loop_info: None,
            remain_sentinel: REMAIN_ALLDATA_IS_ON_MEMORY,
            min_write_bytes: 0,
        }
    }
}

impl AtracManager {
    /// Model `cellAtracCreateDecoder` — allocates internal decoder state.
    pub fn create_decoder(&mut self) -> Result<(), CellError> {
        if self.decoder_created {
            return Err(errors::DECODER_WAS_CREATED);
        }
        self.decoder_created = true;
        Ok(())
    }

    pub fn destroy_decoder(&mut self) -> Result<(), CellError> {
        if !self.decoder_created {
            return Err(errors::NO_DECODER);
        }
        *self = AtracManager::default();
        Ok(())
    }
}

// =====================================================================
// Validation helpers
// =====================================================================

fn ensure_decoder(m: &AtracManager) -> Result<(), CellError> {
    if m.decoder_created { Ok(()) } else { Err(errors::NO_DECODER) }
}

fn ensure_data(m: &AtracManager) -> Result<(), CellError> {
    ensure_decoder(m)?;
    if m.state == DecoderState::Idle {
        Err(errors::UNSET_DATA)
    } else {
        Ok(())
    }
}

// =====================================================================
// Syscalls
// =====================================================================

/// `cellAtracSetData(handle, data, size, min_write_byte)`.
#[must_use]
pub fn cell_atrac_set_data<D: AtracDecoder + ?Sized>(
    m: &mut AtracManager,
    decoder: &D,
    data: Vec<u8>,
    min_write_byte: u32,
) -> Result<(), CellError> {
    ensure_decoder(m)?;
    if data.is_empty() {
        return Err(errors::ILLEGAL_DATA);
    }
    if min_write_byte == 0 {
        return Err(errors::READSIZE_IS_TOO_SMALL);
    }
    if min_write_byte as usize > data.len() {
        return Err(errors::READSIZE_OVER_BUFFER);
    }
    let info = decoder.sound_info(&data)?;
    m.data = data;
    m.info = Some(info);
    m.read_position = 0;
    m.state = DecoderState::Ready;
    m.min_write_bytes = min_write_byte;
    Ok(())
}

/// `cellAtracDecode(handle, out_buffer, samples_out)`.
#[must_use]
pub fn cell_atrac_decode_data<D: AtracDecoder + ?Sized>(
    m: &mut AtracManager,
    decoder: &mut D,
) -> Result<DecodedFrame, CellError> {
    ensure_data(m)?;
    if m.state == DecoderState::Exhausted {
        return Err(errors::ALLDATA_WAS_DECODED);
    }
    let frame = decoder.decode_frame(&m.data, m.read_position)?;
    m.read_position = m.read_position.saturating_add(frame.bytes_consumed);
    if m.read_position as usize >= m.data.len() {
        m.state = DecoderState::Exhausted;
    }
    Ok(frame)
}

#[must_use]
pub fn cell_atrac_get_stream_data_info(m: &AtracManager) -> Result<StreamDataInfo, CellError> {
    ensure_data(m)?;
    Ok(StreamDataInfo {
        write_addr: 0,
        writable_bytes: (m.data.len() as u32).saturating_sub(m.read_position),
        min_write_bytes: m.min_write_bytes,
        read_position: m.read_position,
    })
}

/// `cellAtracAddStreamData(handle, byte_size)` — caller tells us
/// how many additional bytes were written to the ring buffer.
#[must_use]
pub fn cell_atrac_add_stream_data(
    m: &mut AtracManager,
    byte_size: u32,
) -> Result<(), CellError> {
    ensure_data(m)?;
    let capacity = m.data.len() as u32;
    let remaining_capacity = capacity.saturating_sub(m.read_position);
    if byte_size > remaining_capacity {
        return Err(errors::ADD_DATA_IS_TOO_BIG);
    }
    // Exiting Exhausted state if the caller has actually added data.
    if byte_size > 0 && m.state == DecoderState::Exhausted {
        m.state = DecoderState::Ready;
    }
    Ok(())
}

#[must_use]
pub fn cell_atrac_get_remain_frame(m: &AtracManager) -> Result<i32, CellError> {
    ensure_data(m)?;
    Ok(m.remain_sentinel)
}

pub fn set_remain_sentinel(m: &mut AtracManager, sentinel: i32) {
    m.remain_sentinel = sentinel;
}

#[must_use]
pub fn cell_atrac_get_sound_info(m: &AtracManager) -> Result<SoundInfo, CellError> {
    ensure_data(m)?;
    m.info.ok_or(errors::UNSET_DATA)
}

#[must_use]
pub fn cell_atrac_get_max_sample(m: &AtracManager) -> Result<u32, CellError> {
    ensure_data(m)?;
    // One decoded frame yields at most 1024 samples (ATRAC3 spec).
    Ok(1024)
}

#[must_use]
pub fn cell_atrac_set_loop_num(
    m: &mut AtracManager,
    loop_num: i32,
) -> Result<(), CellError> {
    ensure_data(m)?;
    // -1 == infinite loop, 0 == no loop, positive == finite count.
    if loop_num < -1 {
        return Err(errors::UNSET_LOOP_NUM);
    }
    m.loop_info = Some(LoopInfo {
        loop_num,
        loop_start_sample: 0,
        loop_end_sample: 0,
    });
    Ok(())
}

#[must_use]
pub fn cell_atrac_get_loop_info(m: &AtracManager) -> Result<LoopInfo, CellError> {
    ensure_data(m)?;
    m.loop_info.ok_or(errors::UNSET_LOOP_NUM)
}

#[must_use]
pub fn cell_atrac_reset_play_position(
    m: &mut AtracManager,
    sample: u64,
    reset_byte: u32,
) -> Result<(), CellError> {
    ensure_data(m)?;
    let info = m.info.ok_or(errors::UNSET_DATA)?;
    if sample > info.total_samples {
        return Err(errors::ILLEGAL_SAMPLE);
    }
    if reset_byte as usize > m.data.len() {
        return Err(errors::ILLEGAL_RESET_BYTE);
    }
    m.read_position = reset_byte;
    m.state = DecoderState::Ready;
    Ok(())
}

#[must_use]
pub fn cell_atrac_get_bitrate(m: &AtracManager) -> Result<u32, CellError> {
    ensure_data(m)?;
    Ok(m.info.ok_or(errors::UNSET_DATA)?.bitrate)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_data(bytes: usize) -> Vec<u8> {
        // byte 0: sample-rate tag (0 → 44100), byte 1: channels (2).
        let mut d = vec![0u8; bytes];
        d[0] = 0;
        d[1] = 2;
        d
    }

    fn init() -> (AtracManager, StubAtracDecoder) {
        let mut m = AtracManager::default();
        m.create_decoder().unwrap();
        (m, StubAtracDecoder::new())
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::API_FAIL.0, 0x8061_0301);
        assert_eq!(errors::UNKNOWN_FORMAT.0, 0x8061_0312);
        assert_eq!(errors::NO_DECODER.0, 0x8061_0321);
        assert_eq!(errors::ALLDATA_WAS_DECODED.0, 0x8061_0331);
        assert_eq!(errors::ILLEGAL_SPU_THREAD_PRIORITY.0, 0x8061_0382);
    }

    #[test]
    fn remain_sentinels_match_header() {
        assert_eq!(REMAIN_ALLDATA_IS_ON_MEMORY, -1);
        assert_eq!(REMAIN_NONLOOP_STREAM_ON_MEMORY, -2);
        assert_eq!(REMAIN_LOOP_STREAM_ON_MEMORY, -3);
        assert_eq!(HANDLE_SIZE, 512);
    }

    // --- decoder lifecycle ---------------------------------------

    #[test]
    fn create_decoder_twice_is_already_created() {
        let mut m = AtracManager::default();
        m.create_decoder().unwrap();
        assert_eq!(m.create_decoder().unwrap_err(), errors::DECODER_WAS_CREATED);
    }

    #[test]
    fn destroy_without_create_is_no_decoder() {
        let mut m = AtracManager::default();
        assert_eq!(m.destroy_decoder().unwrap_err(), errors::NO_DECODER);
    }

    // --- set_data -------------------------------------------------

    #[test]
    fn set_data_without_decoder_is_no_decoder() {
        let mut m = AtracManager::default();
        let d = StubAtracDecoder::new();
        assert_eq!(
            cell_atrac_set_data(&mut m, &d, mk_data(256), 64).unwrap_err(),
            errors::NO_DECODER,
        );
    }

    #[test]
    fn set_data_with_empty_buffer_is_illegal_data() {
        let (mut m, d) = init();
        assert_eq!(
            cell_atrac_set_data(&mut m, &d, vec![], 64).unwrap_err(),
            errors::ILLEGAL_DATA,
        );
    }

    #[test]
    fn set_data_zero_min_write_is_too_small() {
        let (mut m, d) = init();
        assert_eq!(
            cell_atrac_set_data(&mut m, &d, mk_data(256), 0).unwrap_err(),
            errors::READSIZE_IS_TOO_SMALL,
        );
    }

    #[test]
    fn set_data_min_write_over_buffer_is_readsize_over() {
        let (mut m, d) = init();
        assert_eq!(
            cell_atrac_set_data(&mut m, &d, mk_data(64), 128).unwrap_err(),
            errors::READSIZE_OVER_BUFFER,
        );
    }

    #[test]
    fn set_data_bad_sampling_rate_bubbles_up() {
        let (mut m, d) = init();
        let mut bad = mk_data(256);
        bad[0] = 99; // unknown sample-rate tag
        assert_eq!(
            cell_atrac_set_data(&mut m, &d, bad, 64).unwrap_err(),
            errors::ILLEGAL_SAMPLING_RATE,
        );
    }

    #[test]
    fn set_data_happy_path_stores_sound_info() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        let info = cell_atrac_get_sound_info(&m).unwrap();
        assert_eq!(info.sampling_rate, 44100);
        assert_eq!(info.channels, 2);
    }

    // --- decode ---------------------------------------------------

    #[test]
    fn decode_without_data_is_unset_data() {
        let (mut m, mut d) = init();
        assert_eq!(
            cell_atrac_decode_data(&mut m, &mut d).unwrap_err(),
            errors::UNSET_DATA,
        );
    }

    #[test]
    fn decode_advances_read_position() {
        let (mut m, mut d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        let f = cell_atrac_decode_data(&mut m, &mut d).unwrap();
        assert_eq!(f.bytes_consumed, 256);
        assert_eq!(f.pcm_i16.len(), 2048);
        let info = cell_atrac_get_stream_data_info(&m).unwrap();
        assert_eq!(info.read_position, 256);
    }

    #[test]
    fn decode_after_exhaustion_is_alldata_was_decoded() {
        let (mut m, mut d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(256), 64).unwrap();
        cell_atrac_decode_data(&mut m, &mut d).unwrap();
        assert_eq!(
            cell_atrac_decode_data(&mut m, &mut d).unwrap_err(),
            errors::ALLDATA_WAS_DECODED,
        );
    }

    // --- streaming -----------------------------------------------

    #[test]
    fn add_stream_data_resumes_from_exhausted() {
        let (mut m, mut d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(256), 64).unwrap();
        cell_atrac_decode_data(&mut m, &mut d).unwrap();
        assert!(matches!(m.state, DecoderState::Exhausted));
        // Caller advertises more data in the ring (stub, capacity check).
        cell_atrac_add_stream_data(&mut m, 0).unwrap();
        // Zero bytes: no state change.
        assert!(matches!(m.state, DecoderState::Exhausted));
    }

    #[test]
    fn add_stream_data_too_big_is_add_data_too_big() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(256), 64).unwrap();
        assert_eq!(
            cell_atrac_add_stream_data(&mut m, 9999).unwrap_err(),
            errors::ADD_DATA_IS_TOO_BIG,
        );
    }

    // --- loop / reset --------------------------------------------

    #[test]
    fn set_loop_num_happy_path() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        cell_atrac_set_loop_num(&mut m, 3).unwrap();
        let loop_info = cell_atrac_get_loop_info(&m).unwrap();
        assert_eq!(loop_info.loop_num, 3);
    }

    #[test]
    fn set_loop_num_below_minus_one_is_unset() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        assert_eq!(
            cell_atrac_set_loop_num(&mut m, -5).unwrap_err(),
            errors::UNSET_LOOP_NUM,
        );
    }

    #[test]
    fn get_loop_info_without_set_is_unset() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        assert_eq!(
            cell_atrac_get_loop_info(&m).unwrap_err(),
            errors::UNSET_LOOP_NUM,
        );
    }

    #[test]
    fn reset_play_position_to_valid_sample_and_byte() {
        let (mut m, mut d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        cell_atrac_decode_data(&mut m, &mut d).unwrap();
        cell_atrac_reset_play_position(&mut m, 0, 0).unwrap();
        let info = cell_atrac_get_stream_data_info(&m).unwrap();
        assert_eq!(info.read_position, 0);
    }

    #[test]
    fn reset_play_position_bad_byte_is_illegal() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        assert_eq!(
            cell_atrac_reset_play_position(&mut m, 0, 9999).unwrap_err(),
            errors::ILLEGAL_RESET_BYTE,
        );
    }

    #[test]
    fn reset_play_position_bad_sample_is_illegal() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        assert_eq!(
            cell_atrac_reset_play_position(&mut m, u64::MAX, 0).unwrap_err(),
            errors::ILLEGAL_SAMPLE,
        );
    }

    // --- simple getters ------------------------------------------

    #[test]
    fn get_max_sample_is_1024() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        assert_eq!(cell_atrac_get_max_sample(&m).unwrap(), 1024);
    }

    #[test]
    fn get_bitrate_returns_sound_info_bitrate() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        assert_eq!(cell_atrac_get_bitrate(&m).unwrap(), 128_000);
    }

    #[test]
    fn get_remain_frame_defaults_to_all_on_memory() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        assert_eq!(
            cell_atrac_get_remain_frame(&m).unwrap(),
            REMAIN_ALLDATA_IS_ON_MEMORY,
        );
    }

    #[test]
    fn get_remain_frame_respects_set_sentinel() {
        let (mut m, d) = init();
        cell_atrac_set_data(&mut m, &d, mk_data(1024), 64).unwrap();
        set_remain_sentinel(&mut m, 5);
        assert_eq!(cell_atrac_get_remain_frame(&m).unwrap(), 5);
    }
}
