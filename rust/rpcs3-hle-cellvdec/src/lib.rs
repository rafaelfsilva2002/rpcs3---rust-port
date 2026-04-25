//! `rpcs3-hle-cellvdec` — video decoder framework HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellVdec.cpp`. Companion of
//! `rpcs3-hle-celladec`: same general shape (QueryAttr → Open →
//! StartSeq → DecodeAu → GetPicture → EndSeq → Close) but emits
//! YUV420 pictures instead of PCM frames. Output flows into
//! `cellVpost` for YUV→RGBA conversion before hitting the display.
//!
//! ## Entry points covered
//!
//! | HLE function            | Rust wrapper                 |
//! |-------------------------|------------------------------|
//! | `cellVdecQueryAttr`     | [`cell_vdec_query_attr`]     |
//! | `cellVdecOpen`          | [`cell_vdec_open`]           |
//! | `cellVdecClose`         | [`cell_vdec_close`]          |
//! | `cellVdecStartSeq`      | [`cell_vdec_start_seq`]      |
//! | `cellVdecEndSeq`        | [`cell_vdec_end_seq`]        |
//! | `cellVdecDecodeAu`      | [`cell_vdec_decode_au`]      |
//! | `cellVdecGetPicture`    | [`cell_vdec_get_picture`]    |
//! | `cellVdecGetPicItem`    | [`cell_vdec_get_pic_item`]   |
//! | `cellVdecSetFrameRate`  | [`cell_vdec_set_frame_rate`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellVdec.h:5-13
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ARG: CellError = CellError(0x8061_0101);
    pub const SEQ: CellError = CellError(0x8061_0102);
    pub const BUSY: CellError = CellError(0x8061_0103);
    pub const EMPTY: CellError = CellError(0x8061_0104);
    pub const AU: CellError = CellError(0x8061_0105);
    pub const PIC: CellError = CellError(0x8061_0106);
    pub const FATAL: CellError = CellError(0x8061_0180);
}

// =====================================================================
// Codec types (byte-exact vs cellVdec.h:15-26)
// =====================================================================

pub const CODEC_MPEG2: i32 = 0;
pub const CODEC_AVC: i32 = 1;  // aka H.264
pub const CODEC_MPEG4: i32 = 2;
pub const CODEC_VC1: i32 = 3;
pub const CODEC_DIVX: i32 = 5;
pub const CODEC_JVT: i32 = 7;
pub const CODEC_DIVX3_11: i32 = 9;
pub const CODEC_MVC: i32 = 11;
pub const CODEC_MVC2: i32 = 13;

// =====================================================================
// Frame rate constants (subset of CellVdecFrameRate)
// =====================================================================

pub const FRAME_RATE_23976: u32 = 0;  // 23.976 Hz
pub const FRAME_RATE_24: u32 = 1;
pub const FRAME_RATE_25: u32 = 2;
pub const FRAME_RATE_2997: u32 = 3;   // 29.97 Hz
pub const FRAME_RATE_30: u32 = 4;
pub const FRAME_RATE_50: u32 = 5;
pub const FRAME_RATE_5994: u32 = 6;   // 59.94 Hz
pub const FRAME_RATE_60: u32 = 7;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attr {
    pub decoder_ver: u32,
    pub mem_size: u32,
    pub cmd_depth: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenParam {
    pub codec_type: i32,
    /// Profile level — codec-dependent (e.g. H264 level 4.1, MPEG2 main).
    pub profile_level: u32,
    pub frame_rate: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccessUnit {
    pub start_addr: u32,
    pub size: u32,
    pub pts: u64,
    pub dts: u64,
    pub user_data: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PictureFormat {
    /// 0 = YUV420, other values = codec-specific.
    pub format_type: u32,
    /// Color matrix (BT601/BT709 constants match cellVpost).
    pub color_matrix: u32,
    pub alpha: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pts: u64,
    pub dts: u64,
    pub user_data: u64,
    /// YUV420 planar bytes, laid out [Y, U, V].
    pub yuv_bytes: Vec<u8>,
}

// =====================================================================
// Backend trait
// =====================================================================

pub trait VdecDecoder {
    fn query(&self, codec_type: i32) -> Result<Attr, CellError>;
    fn decode(
        &mut self,
        codec_type: i32,
        open_param: &OpenParam,
        au_bytes: &[u8],
        au: &AccessUnit,
    ) -> Result<Picture, CellError>;
}

/// Stub decoder — emits fixed-size green YUV frames.
#[derive(Debug, Clone, Copy)]
pub struct StubVdecDecoder {
    pub width: u32,
    pub height: u32,
}

impl Default for StubVdecDecoder {
    fn default() -> Self { Self { width: 640, height: 480 } }
}

impl VdecDecoder for StubVdecDecoder {
    fn query(&self, codec_type: i32) -> Result<Attr, CellError> {
        if !is_known_codec(codec_type) {
            return Err(errors::ARG);
        }
        Ok(Attr {
            decoder_ver: 0x0002_0000,
            mem_size: 0x0120_0000,  // 18 MB, typical for H.264 HD
            cmd_depth: 4,
        })
    }

    fn decode(
        &mut self,
        codec_type: i32,
        _open_param: &OpenParam,
        au_bytes: &[u8],
        au: &AccessUnit,
    ) -> Result<Picture, CellError> {
        if !is_known_codec(codec_type) {
            return Err(errors::ARG);
        }
        if au_bytes.is_empty() {
            return Err(errors::EMPTY);
        }
        // YUV420 size = W*H for Y + W*H/2 for interleaved UV.
        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 2;
        let mut bytes = vec![0u8; y_size + uv_size];
        // "Green" approx in YUV: Y≈150, U=43, V=21.
        for b in &mut bytes[..y_size] { *b = 150; }
        for b in &mut bytes[y_size..y_size + uv_size / 2] { *b = 43; }
        for b in &mut bytes[y_size + uv_size / 2..] { *b = 21; }
        Ok(Picture {
            width: self.width,
            height: self.height,
            pts: au.pts,
            dts: au.dts,
            user_data: au.user_data,
            yuv_bytes: bytes,
        })
    }
}

pub fn is_known_codec(c: i32) -> bool {
    matches!(c,
        CODEC_MPEG2 | CODEC_AVC | CODEC_MPEG4 | CODEC_VC1 | CODEC_DIVX
        | CODEC_JVT | CODEC_DIVX3_11 | CODEC_MVC | CODEC_MVC2,
    )
}

pub fn is_known_frame_rate(r: u32) -> bool {
    matches!(r,
        FRAME_RATE_23976 | FRAME_RATE_24 | FRAME_RATE_25 | FRAME_RATE_2997
        | FRAME_RATE_30 | FRAME_RATE_50 | FRAME_RATE_5994 | FRAME_RATE_60,
    )
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderState {
    Closed,
    Open,
    InSequence,
}

#[derive(Debug, Clone)]
struct DecoderHandle {
    state: DecoderState,
    open_param: OpenParam,
    pending_pictures: std::collections::VecDeque<Picture>,
}

#[derive(Debug, Default)]
pub struct VdecManager {
    handles: std::collections::BTreeMap<u32, DecoderHandle>,
    next_id: u32,
}

// =====================================================================
// Syscalls
// =====================================================================

#[must_use]
pub fn cell_vdec_query_attr<D: VdecDecoder + ?Sized>(
    decoder: &D,
    codec_type: i32,
) -> Result<Attr, CellError> {
    decoder.query(codec_type)
}

#[must_use]
pub fn cell_vdec_open(
    m: &mut VdecManager,
    param: OpenParam,
) -> Result<u32, CellError> {
    if !is_known_codec(param.codec_type) {
        return Err(errors::ARG);
    }
    if !is_known_frame_rate(param.frame_rate) {
        return Err(errors::ARG);
    }
    m.next_id += 1;
    let id = m.next_id;
    m.handles.insert(
        id,
        DecoderHandle {
            state: DecoderState::Open,
            open_param: param,
            pending_pictures: Default::default(),
        },
    );
    Ok(id)
}

#[must_use]
pub fn cell_vdec_close(m: &mut VdecManager, handle: u32) -> Result<(), CellError> {
    let h = m.handles.get(&handle).ok_or(errors::ARG)?;
    if h.state == DecoderState::InSequence {
        return Err(errors::SEQ);
    }
    m.handles.remove(&handle);
    Ok(())
}

#[must_use]
pub fn cell_vdec_start_seq(m: &mut VdecManager, handle: u32) -> Result<(), CellError> {
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
pub fn cell_vdec_end_seq(m: &mut VdecManager, handle: u32) -> Result<(), CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::ARG)?;
    match h.state {
        DecoderState::InSequence => {
            h.state = DecoderState::Open;
            h.pending_pictures.clear();
            Ok(())
        }
        _ => Err(errors::SEQ),
    }
}

#[must_use]
pub fn cell_vdec_decode_au<D: VdecDecoder + ?Sized>(
    m: &mut VdecManager,
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
        return Err(errors::AU);
    }
    let pic = decoder.decode(h.open_param.codec_type, &h.open_param, au_bytes, &au)?;
    h.pending_pictures.push_back(pic);
    Ok(())
}

#[must_use]
pub fn cell_vdec_get_picture(
    m: &mut VdecManager,
    handle: u32,
) -> Result<Picture, CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::ARG)?;
    if h.state != DecoderState::InSequence {
        return Err(errors::SEQ);
    }
    h.pending_pictures.pop_front().ok_or(errors::EMPTY)
}

#[must_use]
pub fn cell_vdec_get_pic_item(
    m: &mut VdecManager,
    handle: u32,
) -> Result<(Picture, u32), CellError> {
    let pic = cell_vdec_get_picture(m, handle)?;
    let remaining = m
        .handles
        .get(&handle)
        .map_or(0, |h| h.pending_pictures.len() as u32);
    Ok((pic, remaining))
}

#[must_use]
pub fn cell_vdec_set_frame_rate(
    m: &mut VdecManager,
    handle: u32,
    frame_rate: u32,
) -> Result<(), CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::ARG)?;
    if !is_known_frame_rate(frame_rate) {
        return Err(errors::ARG);
    }
    h.open_param.frame_rate = frame_rate;
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_param(codec: i32) -> OpenParam {
        OpenParam { codec_type: codec, profile_level: 0, frame_rate: FRAME_RATE_30 }
    }

    fn stub_au() -> AccessUnit {
        AccessUnit { start_addr: 0x1000, size: 512, pts: 100_000, dts: 99_000, user_data: 0 }
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::ARG.0, 0x8061_0101);
        assert_eq!(errors::SEQ.0, 0x8061_0102);
        assert_eq!(errors::BUSY.0, 0x8061_0103);
        assert_eq!(errors::EMPTY.0, 0x8061_0104);
        assert_eq!(errors::AU.0, 0x8061_0105);
        assert_eq!(errors::PIC.0, 0x8061_0106);
        assert_eq!(errors::FATAL.0, 0x8061_0180);
    }

    #[test]
    fn codec_type_constants_stable() {
        assert_eq!(CODEC_MPEG2, 0);
        assert_eq!(CODEC_AVC, 1);
        assert_eq!(CODEC_MPEG4, 2);
        assert_eq!(CODEC_VC1, 3);
        assert_eq!(CODEC_DIVX, 5);
        assert_eq!(CODEC_JVT, 7);
        assert_eq!(CODEC_MVC, 11);
    }

    #[test]
    fn is_known_codec_accepts_all() {
        for c in [CODEC_MPEG2, CODEC_AVC, CODEC_MPEG4, CODEC_VC1, CODEC_DIVX,
                  CODEC_JVT, CODEC_DIVX3_11, CODEC_MVC, CODEC_MVC2] {
            assert!(is_known_codec(c));
        }
    }

    #[test]
    fn is_known_codec_rejects_unknown() {
        assert!(!is_known_codec(99));
        assert!(!is_known_codec(-1));
    }

    #[test]
    fn frame_rate_constants_stable() {
        assert_eq!(FRAME_RATE_23976, 0);
        assert_eq!(FRAME_RATE_30, 4);
        assert_eq!(FRAME_RATE_60, 7);
    }

    // --- query ---------------------------------------------------

    #[test]
    fn query_attr_returns_18mb_for_avc() {
        let d = StubVdecDecoder::default();
        let attr = cell_vdec_query_attr(&d, CODEC_AVC).unwrap();
        assert!(attr.mem_size >= 0x0100_0000);
    }

    #[test]
    fn query_attr_invalid_codec_is_arg() {
        let d = StubVdecDecoder::default();
        assert_eq!(cell_vdec_query_attr(&d, 99).unwrap_err(), errors::ARG);
    }

    // --- open / close --------------------------------------------

    #[test]
    fn open_returns_handle() {
        let mut m = VdecManager::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        assert_eq!(h, 1);
    }

    #[test]
    fn open_invalid_codec_is_arg() {
        let mut m = VdecManager::default();
        let p = OpenParam { codec_type: 99, ..stub_param(0) };
        assert_eq!(cell_vdec_open(&mut m, p).unwrap_err(), errors::ARG);
    }

    #[test]
    fn open_invalid_frame_rate_is_arg() {
        let mut m = VdecManager::default();
        let p = OpenParam { frame_rate: 99, ..stub_param(CODEC_AVC) };
        assert_eq!(cell_vdec_open(&mut m, p).unwrap_err(), errors::ARG);
    }

    #[test]
    fn close_unknown_is_arg() {
        let mut m = VdecManager::default();
        assert_eq!(cell_vdec_close(&mut m, 999).unwrap_err(), errors::ARG);
    }

    #[test]
    fn close_during_sequence_is_seq() {
        let mut m = VdecManager::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        assert_eq!(cell_vdec_close(&mut m, h).unwrap_err(), errors::SEQ);
    }

    // --- sequence lifecycle --------------------------------------

    #[test]
    fn start_seq_twice_is_seq() {
        let mut m = VdecManager::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        assert_eq!(cell_vdec_start_seq(&mut m, h).unwrap_err(), errors::SEQ);
    }

    #[test]
    fn end_seq_without_start_is_seq() {
        let mut m = VdecManager::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        assert_eq!(cell_vdec_end_seq(&mut m, h).unwrap_err(), errors::SEQ);
    }

    #[test]
    fn start_end_round_trip() {
        let mut m = VdecManager::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        cell_vdec_end_seq(&mut m, h).unwrap();
        cell_vdec_close(&mut m, h).unwrap();
    }

    // --- decode --------------------------------------------------

    #[test]
    fn decode_before_start_is_seq() {
        let mut m = VdecManager::default();
        let mut d = StubVdecDecoder::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        let bytes = vec![0u8; 512];
        assert_eq!(
            cell_vdec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap_err(),
            errors::SEQ,
        );
    }

    #[test]
    fn decode_size_mismatch_is_au() {
        let mut m = VdecManager::default();
        let mut d = StubVdecDecoder::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        let au = AccessUnit { size: 100, ..stub_au() };
        let bytes = vec![0u8; 50];
        assert_eq!(
            cell_vdec_decode_au(&mut m, &mut d, h, au, &bytes).unwrap_err(),
            errors::AU,
        );
    }

    #[test]
    fn decode_queues_picture() {
        let mut m = VdecManager::default();
        let mut d = StubVdecDecoder::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();

        let bytes = vec![0u8; 512];
        cell_vdec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap();
        let pic = cell_vdec_get_picture(&mut m, h).unwrap();
        assert_eq!(pic.width, 640);
        assert_eq!(pic.height, 480);
        assert_eq!(pic.yuv_bytes.len(), (640 * 480) + (640 * 480) / 2);
        assert_eq!(pic.pts, 100_000);
        assert_eq!(pic.dts, 99_000);
    }

    #[test]
    fn get_picture_empty_queue_is_empty() {
        let mut m = VdecManager::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        assert_eq!(cell_vdec_get_picture(&mut m, h).unwrap_err(), errors::EMPTY);
    }

    #[test]
    fn get_pic_item_reports_remaining() {
        let mut m = VdecManager::default();
        let mut d = StubVdecDecoder::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();

        let bytes = vec![0u8; 512];
        for _ in 0..3 {
            cell_vdec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap();
        }
        let (_pic, remaining) = cell_vdec_get_pic_item(&mut m, h).unwrap();
        assert_eq!(remaining, 2);
    }

    #[test]
    fn decode_empty_au_is_empty() {
        let mut m = VdecManager::default();
        let mut d = StubVdecDecoder::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        let au = AccessUnit { size: 0, ..stub_au() };
        assert_eq!(
            cell_vdec_decode_au(&mut m, &mut d, h, au, &[]).unwrap_err(),
            errors::EMPTY,
        );
    }

    // --- frame rate setter ---------------------------------------

    #[test]
    fn set_frame_rate_happy_path() {
        let mut m = VdecManager::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_set_frame_rate(&mut m, h, FRAME_RATE_60).unwrap();
    }

    #[test]
    fn set_frame_rate_bad_value_is_arg() {
        let mut m = VdecManager::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        assert_eq!(
            cell_vdec_set_frame_rate(&mut m, h, 99).unwrap_err(),
            errors::ARG,
        );
    }

    // --- smoke ---------------------------------------------------

    #[test]
    fn full_pipeline_open_start_decode_get_end_close() {
        let mut m = VdecManager::default();
        let mut d = StubVdecDecoder::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        let bytes = vec![0u8; 512];
        cell_vdec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap();
        cell_vdec_get_picture(&mut m, h).unwrap();
        cell_vdec_end_seq(&mut m, h).unwrap();
        cell_vdec_close(&mut m, h).unwrap();
    }

    #[test]
    fn end_seq_clears_pending_pictures() {
        let mut m = VdecManager::default();
        let mut d = StubVdecDecoder::default();
        let h = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        let bytes = vec![0u8; 512];
        cell_vdec_decode_au(&mut m, &mut d, h, stub_au(), &bytes).unwrap();
        cell_vdec_end_seq(&mut m, h).unwrap();
        cell_vdec_start_seq(&mut m, h).unwrap();
        assert_eq!(cell_vdec_get_picture(&mut m, h).unwrap_err(), errors::EMPTY);
    }

    #[test]
    fn multi_handle_isolation() {
        let mut m = VdecManager::default();
        let mut d = StubVdecDecoder::default();
        let h1 = cell_vdec_open(&mut m, stub_param(CODEC_AVC)).unwrap();
        let h2 = cell_vdec_open(&mut m, stub_param(CODEC_MPEG2)).unwrap();
        cell_vdec_start_seq(&mut m, h1).unwrap();
        cell_vdec_start_seq(&mut m, h2).unwrap();
        let bytes = vec![0u8; 512];
        cell_vdec_decode_au(&mut m, &mut d, h1, stub_au(), &bytes).unwrap();
        assert_eq!(cell_vdec_get_picture(&mut m, h2).unwrap_err(), errors::EMPTY);
        cell_vdec_get_picture(&mut m, h1).unwrap();
    }
}
