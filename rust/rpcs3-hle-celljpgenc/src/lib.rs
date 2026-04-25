//! `rpcs3-hle-celljpgenc` — JPEG encoder HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellJpgEnc.cpp`. Games use the
//! encoder to write screenshots or user-generated content to JPEG
//! files. API shape: `Create → Open → EncodePicture → WaitForOutput →
//! Close → Destroy`.
//!
//! ## Entry points covered
//!
//! | HLE function                 | Rust wrapper                     |
//! |------------------------------|----------------------------------|
//! | `cellJpgEncQueryAttr`        | [`query_attr`]                   |
//! | `cellJpgEncOpen`             | [`JpgEnc::open`]                 |
//! | `cellJpgEncClose`            | [`JpgEnc::close`]                |
//! | `cellJpgEncEncodePicture`    | [`JpgEnc::encode_picture`]       |
//! | `cellJpgEncEncodePicture2`   | [`JpgEnc::encode_picture`]       |
//! | `cellJpgEncWaitForOutput`    | [`JpgEnc::wait_for_output`]      |
//! | `cellJpgEncEncodePictureWithAttr` | [`JpgEnc::encode_with_attr`] |
//! | `cellJpgEncReset`            | [`JpgEnc::reset`]                |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellJpgEnc.h:6-19
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ARG: CellError = CellError(0x8061_1191);
    pub const SEQ: CellError = CellError(0x8061_1192);
    pub const BUSY: CellError = CellError(0x8061_1193);
    pub const EMPTY: CellError = CellError(0x8061_1194);
    pub const RESET: CellError = CellError(0x8061_1195);
    pub const FATAL: CellError = CellError(0x8061_1196);

    pub const STREAM_ABORT: CellError = CellError(0x8061_11A1);
    pub const STREAM_SKIP: CellError = CellError(0x8061_11A2);
    pub const STREAM_OVERFLOW: CellError = CellError(0x8061_11A3);
    pub const STREAM_FILE_OPEN: CellError = CellError(0x8061_11A4);
}

// =====================================================================
// Input color space (cellJpgEnc.h:22-28)
// =====================================================================

pub const CS_GRAYSCALE: i32 = 1;
pub const CS_RGB: i32 = 2;
pub const CS_YCBCR: i32 = 3;
pub const CS_RGBA: i32 = 10;
pub const CS_ARGB: i32 = 20;

#[must_use]
pub fn is_known_color_space(cs: i32) -> bool {
    matches!(cs, CS_GRAYSCALE | CS_RGB | CS_YCBCR | CS_RGBA | CS_ARGB)
}

/// Input bytes/pixel for a given input color space.
#[must_use]
pub fn bytes_per_pixel(cs: i32) -> Option<u32> {
    Some(match cs {
        CS_GRAYSCALE => 1,
        CS_RGB | CS_YCBCR => 3,
        CS_RGBA | CS_ARGB => 4,
        _ => return None,
    })
}

// =====================================================================
// Sampling format (cellJpgEnc.h:30-34)
//
// The C++ enum auto-numbers these starting from CELL_JPGENC_SAMPLING_FMT_YCbCr444,
// but the preceding block ends at CELL_JPGENC_COLOR_SPACE_ARGB=20, so the next
// auto-assigned value is 21. We preserve the sequence exactly.
// =====================================================================

pub const SAMPLING_YCBCR_444: i32 = 21;
pub const SAMPLING_YCBCR_422: i32 = 22;
pub const SAMPLING_YCBCR_420: i32 = 23;
pub const SAMPLING_YCBCR_411: i32 = 24;
pub const SAMPLING_FULL: i32 = 25;

#[must_use]
pub fn is_known_sampling(s: i32) -> bool {
    (SAMPLING_YCBCR_444..=SAMPLING_FULL).contains(&s)
}

// =====================================================================
// DCT / compression / output location (cellJpgEnc.h:36-44)
// =====================================================================

pub const DCT_METHOD_QUALITY: i32 = 0;
pub const DCT_METHOD_FAST: i32 = 5;

#[must_use]
pub fn is_known_dct(d: i32) -> bool {
    matches!(d, DCT_METHOD_QUALITY | DCT_METHOD_FAST)
}

// Auto-numbered right after DCT_METHOD_FAST=5.
pub const COMPR_MODE_CONSTANT_QUALITY: i32 = 6;
pub const COMPR_MODE_STREAM_SIZE_LIMIT: i32 = 7;

#[must_use]
pub fn is_known_compr_mode(c: i32) -> bool {
    matches!(c, COMPR_MODE_CONSTANT_QUALITY | COMPR_MODE_STREAM_SIZE_LIMIT)
}

// Auto-numbered right after COMPR_MODE_STREAM_SIZE_LIMIT=7.
pub const LOCATION_FILE: i32 = 8;
pub const LOCATION_BUFFER: i32 = 9;

#[must_use]
pub fn is_known_location(l: i32) -> bool {
    matches!(l, LOCATION_FILE | LOCATION_BUFFER)
}

// =====================================================================
// Limits (observed from C++ encoder)
// =====================================================================

pub const MAX_WIDTH: u32 = 4096;
pub const MAX_HEIGHT: u32 = 4096;
pub const QUALITY_MIN: u32 = 1;
pub const QUALITY_MAX: u32 = 100;
pub const MAX_HANDLES: u32 = 16;

// =====================================================================
// Structs
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub max_width: u32,
    pub max_height: u32,
    pub color_space: i32,
    pub sampling: i32,
    pub dct_method: i32,
    pub compr_mode: i32,
    pub quality: u32, // 1..=100
}

impl Attr {
    fn validate(&self) -> Result<(), CellError> {
        if self.max_width == 0 || self.max_width > MAX_WIDTH {
            return Err(errors::ARG);
        }
        if self.max_height == 0 || self.max_height > MAX_HEIGHT {
            return Err(errors::ARG);
        }
        if !is_known_color_space(self.color_space) {
            return Err(errors::ARG);
        }
        if !is_known_sampling(self.sampling) {
            return Err(errors::ARG);
        }
        if !is_known_dct(self.dct_method) {
            return Err(errors::ARG);
        }
        if !is_known_compr_mode(self.compr_mode) {
            return Err(errors::ARG);
        }
        if !(QUALITY_MIN..=QUALITY_MAX).contains(&self.quality) {
            return Err(errors::ARG);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EncodedInfo {
    pub memory_size: u32,   // bytes allocated by the encoder
    pub enable_au: bool,    // whether per-AU encoding is supported
}

/// `cellJpgEncQueryAttr(attr, encInfo)`. Returns required memory for
/// the given format.
pub fn query_attr(attr: &Attr) -> Result<EncodedInfo, CellError> {
    attr.validate()?;
    let bpp = bytes_per_pixel(attr.color_space).ok_or(errors::ARG)?;
    let raw = u64::from(attr.max_width) * u64::from(attr.max_height) * u64::from(bpp);
    // Encoder needs input + rough 50% headroom for Huffman buffers.
    let memory_size = (raw + raw / 2 + 1024) as u32;
    Ok(EncodedInfo { memory_size, enable_au: attr.sampling == SAMPLING_FULL })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeParam {
    pub input_width: u32,
    pub input_height: u32,
    pub quality: u32,
    pub location: i32,
    pub dst_path: String, // non-empty when LOCATION_FILE
    pub dst_buf_ptr: u32, // non-zero when LOCATION_BUFFER
    pub dst_buf_size: u32,
}

impl EncodeParam {
    fn validate(&self, attr: &Attr) -> Result<(), CellError> {
        if self.input_width == 0 || self.input_width > attr.max_width {
            return Err(errors::ARG);
        }
        if self.input_height == 0 || self.input_height > attr.max_height {
            return Err(errors::ARG);
        }
        if !(QUALITY_MIN..=QUALITY_MAX).contains(&self.quality) {
            return Err(errors::ARG);
        }
        if !is_known_location(self.location) {
            return Err(errors::ARG);
        }
        match self.location {
            LOCATION_FILE => {
                if self.dst_path.is_empty() {
                    return Err(errors::STREAM_FILE_OPEN);
                }
            }
            LOCATION_BUFFER => {
                if self.dst_buf_ptr == 0 || self.dst_buf_size == 0 {
                    return Err(errors::ARG);
                }
            }
            _ => return Err(errors::ARG),
        }
        Ok(())
    }
}

// =====================================================================
// Handle + state
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HandleState {
    Idle,
    Encoding,
    HasOutput,
}

#[derive(Clone, Debug)]
struct Handle {
    id: u32,
    state: HandleState,
    attr: Attr,
    last_bytes_written: u32,
}

#[derive(Clone, Debug, Default)]
pub struct JpgEnc {
    handles: Vec<Handle>,
    next_id: u32,
}

impl JpgEnc {
    #[must_use]
    pub fn new() -> Self {
        Self { next_id: 1, ..Default::default() }
    }

    /// `cellJpgEncOpen(mainHandle, attr, encInfo)`. Allocates a per-encoder handle.
    pub fn open(&mut self, attr: Attr) -> Result<u32, CellError> {
        attr.validate()?;
        if self.handles.len() >= MAX_HANDLES as usize {
            return Err(errors::FATAL);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(errors::FATAL)?;
        self.handles.push(Handle { id, state: HandleState::Idle, attr, last_bytes_written: 0 });
        Ok(id)
    }

    pub fn close(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state == HandleState::Encoding {
            return Err(errors::BUSY);
        }
        self.handles.remove(idx);
        Ok(())
    }

    /// `cellJpgEncEncodePicture(handle, src, quality, location, dst)`.
    /// Kicks off the encode.
    pub fn encode_picture(&mut self, id: u32, param: EncodeParam) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        let attr = self.handles[idx].attr;
        if self.handles[idx].state != HandleState::Idle {
            return Err(errors::BUSY);
        }
        param.validate(&attr)?;
        self.handles[idx].state = HandleState::Encoding;
        Ok(())
    }

    /// Overrides per-encode attributes (quality/compr_mode) when a
    /// game calls `cellJpgEncEncodePictureWithAttr`.
    pub fn encode_with_attr(&mut self, id: u32, param: EncodeParam, attr: Attr) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::Idle {
            return Err(errors::BUSY);
        }
        attr.validate()?;
        param.validate(&attr)?;
        self.handles[idx].attr = attr;
        self.handles[idx].state = HandleState::Encoding;
        Ok(())
    }

    /// Test hook — signal that the encode finished and produced
    /// `bytes_written` bytes of output. Real lib posts this from the
    /// worker thread.
    pub fn complete_encode(&mut self, id: u32, bytes_written: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::Encoding {
            return Err(errors::SEQ);
        }
        self.handles[idx].state = HandleState::HasOutput;
        self.handles[idx].last_bytes_written = bytes_written;
        Ok(())
    }

    /// `cellJpgEncWaitForOutput(handle, streamInfo)`. Returns the byte
    /// count the encoder produced; EMPTY if no encode has finished.
    pub fn wait_for_output(&mut self, id: u32) -> Result<u32, CellError> {
        let idx = self.handle_idx(id)?;
        match self.handles[idx].state {
            HandleState::HasOutput => {
                let bytes = self.handles[idx].last_bytes_written;
                self.handles[idx].state = HandleState::Idle;
                self.handles[idx].last_bytes_written = 0;
                Ok(bytes)
            }
            HandleState::Encoding => Err(errors::BUSY),
            HandleState::Idle => Err(errors::EMPTY),
        }
    }

    /// `cellJpgEncReset(handle)`. Abort any in-progress encode.
    pub fn reset(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state == HandleState::Idle {
            return Err(errors::RESET);
        }
        self.handles[idx].state = HandleState::Idle;
        self.handles[idx].last_bytes_written = 0;
        Ok(())
    }

    #[must_use]
    pub fn handle_count(&self) -> usize {
        self.handles.len()
    }

    fn handle_idx(&self, id: u32) -> Result<usize, CellError> {
        self.handles.iter().position(|h| h.id == id).ok_or(errors::ARG)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_attr() -> Attr {
        Attr {
            max_width: 1920,
            max_height: 1080,
            color_space: CS_RGB,
            sampling: SAMPLING_YCBCR_420,
            dct_method: DCT_METHOD_QUALITY,
            compr_mode: COMPR_MODE_CONSTANT_QUALITY,
            quality: 80,
        }
    }

    fn ok_param(loc: i32) -> EncodeParam {
        match loc {
            LOCATION_FILE => EncodeParam {
                input_width: 1280,
                input_height: 720,
                quality: 80,
                location: LOCATION_FILE,
                dst_path: "/dev_hdd0/photo/out.jpg".into(),
                dst_buf_ptr: 0,
                dst_buf_size: 0,
            },
            _ => EncodeParam {
                input_width: 1280,
                input_height: 720,
                quality: 80,
                location: LOCATION_BUFFER,
                dst_path: String::new(),
                dst_buf_ptr: 0x4000,
                dst_buf_size: 1024 * 1024,
            },
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::ARG.0, 0x8061_1191);
        assert_eq!(errors::SEQ.0, 0x8061_1192);
        assert_eq!(errors::BUSY.0, 0x8061_1193);
        assert_eq!(errors::EMPTY.0, 0x8061_1194);
        assert_eq!(errors::RESET.0, 0x8061_1195);
        assert_eq!(errors::FATAL.0, 0x8061_1196);
        assert_eq!(errors::STREAM_ABORT.0, 0x8061_11A1);
        assert_eq!(errors::STREAM_SKIP.0, 0x8061_11A2);
        assert_eq!(errors::STREAM_OVERFLOW.0, 0x8061_11A3);
        assert_eq!(errors::STREAM_FILE_OPEN.0, 0x8061_11A4);
    }

    #[test]
    fn color_space_constants_stable() {
        assert_eq!(CS_GRAYSCALE, 1);
        assert_eq!(CS_RGB, 2);
        assert_eq!(CS_YCBCR, 3);
        assert_eq!(CS_RGBA, 10);
        assert_eq!(CS_ARGB, 20);
    }

    #[test]
    fn sampling_constants_stable() {
        assert_eq!(SAMPLING_YCBCR_444, 21);
        assert_eq!(SAMPLING_YCBCR_422, 22);
        assert_eq!(SAMPLING_YCBCR_420, 23);
        assert_eq!(SAMPLING_YCBCR_411, 24);
        assert_eq!(SAMPLING_FULL, 25);
    }

    #[test]
    fn dct_and_compr_constants_stable() {
        assert_eq!(DCT_METHOD_QUALITY, 0);
        assert_eq!(DCT_METHOD_FAST, 5);
        assert_eq!(COMPR_MODE_CONSTANT_QUALITY, 6);
        assert_eq!(COMPR_MODE_STREAM_SIZE_LIMIT, 7);
        assert_eq!(LOCATION_FILE, 8);
        assert_eq!(LOCATION_BUFFER, 9);
    }

    #[test]
    fn limits_stable() {
        assert_eq!(MAX_WIDTH, 4096);
        assert_eq!(MAX_HEIGHT, 4096);
        assert_eq!(QUALITY_MIN, 1);
        assert_eq!(QUALITY_MAX, 100);
        assert_eq!(MAX_HANDLES, 16);
    }

    #[test]
    fn bytes_per_pixel_maps() {
        assert_eq!(bytes_per_pixel(CS_GRAYSCALE), Some(1));
        assert_eq!(bytes_per_pixel(CS_RGB), Some(3));
        assert_eq!(bytes_per_pixel(CS_YCBCR), Some(3));
        assert_eq!(bytes_per_pixel(CS_RGBA), Some(4));
        assert_eq!(bytes_per_pixel(CS_ARGB), Some(4));
        assert_eq!(bytes_per_pixel(99), None);
    }

    #[test]
    fn attr_validate_happy_path() {
        ok_attr().validate().unwrap();
    }

    #[test]
    fn attr_validate_rejects_zero_dims() {
        let mut a = ok_attr();
        a.max_width = 0;
        assert_eq!(a.validate(), Err(errors::ARG));
    }

    #[test]
    fn attr_validate_rejects_oversize_dims() {
        let mut a = ok_attr();
        a.max_width = MAX_WIDTH + 1;
        assert_eq!(a.validate(), Err(errors::ARG));
    }

    #[test]
    fn attr_validate_unknown_color_space_rejected() {
        let mut a = ok_attr();
        a.color_space = 99;
        assert_eq!(a.validate(), Err(errors::ARG));
    }

    #[test]
    fn attr_validate_unknown_sampling_rejected() {
        let mut a = ok_attr();
        a.sampling = 99;
        assert_eq!(a.validate(), Err(errors::ARG));
    }

    #[test]
    fn attr_validate_unknown_dct_rejected() {
        let mut a = ok_attr();
        a.dct_method = 2;
        assert_eq!(a.validate(), Err(errors::ARG));
    }

    #[test]
    fn attr_validate_quality_range_rejected() {
        let mut a = ok_attr();
        a.quality = 0;
        assert_eq!(a.validate(), Err(errors::ARG));
        a.quality = 101;
        assert_eq!(a.validate(), Err(errors::ARG));
    }

    #[test]
    fn query_attr_returns_sensible_memory_size() {
        let info = query_attr(&ok_attr()).unwrap();
        assert!(info.memory_size > 1920 * 1080 * 3);
        assert!(!info.enable_au); // 420 sampling
    }

    #[test]
    fn query_attr_full_sampling_enables_au() {
        let mut a = ok_attr();
        a.sampling = SAMPLING_FULL;
        assert!(query_attr(&a).unwrap().enable_au);
    }

    #[test]
    fn open_allocates_incrementing_ids() {
        let mut e = JpgEnc::new();
        let a = e.open(ok_attr()).unwrap();
        let b = e.open(ok_attr()).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(e.handle_count(), 2);
    }

    #[test]
    fn open_bad_attr_rejected() {
        let mut e = JpgEnc::new();
        let mut a = ok_attr();
        a.quality = 200;
        assert_eq!(e.open(a), Err(errors::ARG));
    }

    #[test]
    fn open_exceeds_max_handles_rejected() {
        let mut e = JpgEnc::new();
        for _ in 0..MAX_HANDLES {
            e.open(ok_attr()).unwrap();
        }
        assert_eq!(e.open(ok_attr()), Err(errors::FATAL));
    }

    #[test]
    fn close_bad_id_rejected() {
        let mut e = JpgEnc::new();
        assert_eq!(e.close(999), Err(errors::ARG));
    }

    #[test]
    fn close_during_encode_is_busy() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_BUFFER)).unwrap();
        assert_eq!(e.close(h), Err(errors::BUSY));
    }

    #[test]
    fn encode_picture_file_happy_path() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_FILE)).unwrap();
    }

    #[test]
    fn encode_picture_buffer_happy_path() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_BUFFER)).unwrap();
    }

    #[test]
    fn encode_picture_empty_file_path_is_stream_file_open() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        let mut p = ok_param(LOCATION_FILE);
        p.dst_path.clear();
        assert_eq!(e.encode_picture(h, p), Err(errors::STREAM_FILE_OPEN));
    }

    #[test]
    fn encode_picture_buffer_null_rejected() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        let mut p = ok_param(LOCATION_BUFFER);
        p.dst_buf_ptr = 0;
        assert_eq!(e.encode_picture(h, p), Err(errors::ARG));
    }

    #[test]
    fn encode_picture_oversize_dims_rejected() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        let mut p = ok_param(LOCATION_BUFFER);
        p.input_width = 4096;
        assert_eq!(e.encode_picture(h, p), Err(errors::ARG));
    }

    #[test]
    fn encode_picture_while_encoding_is_busy() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_BUFFER)).unwrap();
        assert_eq!(e.encode_picture(h, ok_param(LOCATION_BUFFER)), Err(errors::BUSY));
    }

    #[test]
    fn wait_for_output_idle_is_empty() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        assert_eq!(e.wait_for_output(h), Err(errors::EMPTY));
    }

    #[test]
    fn wait_for_output_during_encode_is_busy() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_BUFFER)).unwrap();
        assert_eq!(e.wait_for_output(h), Err(errors::BUSY));
    }

    #[test]
    fn wait_for_output_after_complete_returns_bytes() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_BUFFER)).unwrap();
        e.complete_encode(h, 12_345).unwrap();
        assert_eq!(e.wait_for_output(h), Ok(12_345));
        // After consuming, state should be Idle again.
        assert_eq!(e.wait_for_output(h), Err(errors::EMPTY));
    }

    #[test]
    fn complete_without_encode_is_seq() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        assert_eq!(e.complete_encode(h, 0), Err(errors::SEQ));
    }

    #[test]
    fn reset_idle_is_reset_error() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        assert_eq!(e.reset(h), Err(errors::RESET));
    }

    #[test]
    fn reset_during_encode_cancels() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_BUFFER)).unwrap();
        e.reset(h).unwrap();
        // After reset can start a new encode.
        e.encode_picture(h, ok_param(LOCATION_BUFFER)).unwrap();
    }

    #[test]
    fn encode_with_attr_allows_attr_swap() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        let mut new_attr = ok_attr();
        new_attr.quality = 50;
        new_attr.compr_mode = COMPR_MODE_STREAM_SIZE_LIMIT;
        e.encode_with_attr(h, ok_param(LOCATION_BUFFER), new_attr).unwrap();
    }

    #[test]
    fn encode_with_attr_bad_attr_rejected() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        let mut new_attr = ok_attr();
        new_attr.max_width = 0;
        assert_eq!(
            e.encode_with_attr(h, ok_param(LOCATION_BUFFER), new_attr),
            Err(errors::ARG)
        );
    }

    #[test]
    fn encode_with_attr_while_encoding_is_busy() {
        let mut e = JpgEnc::new();
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_BUFFER)).unwrap();
        assert_eq!(
            e.encode_with_attr(h, ok_param(LOCATION_BUFFER), ok_attr()),
            Err(errors::BUSY)
        );
    }

    #[test]
    fn is_known_sampling_helper() {
        for s in [SAMPLING_YCBCR_444, SAMPLING_YCBCR_422, SAMPLING_YCBCR_420, SAMPLING_YCBCR_411, SAMPLING_FULL] {
            assert!(is_known_sampling(s));
        }
        assert!(!is_known_sampling(20));
        assert!(!is_known_sampling(26));
    }

    #[test]
    fn is_known_location_helper() {
        assert!(is_known_location(LOCATION_FILE));
        assert!(is_known_location(LOCATION_BUFFER));
        assert!(!is_known_location(0));
        assert!(!is_known_location(99));
    }

    #[test]
    fn full_jpg_enc_lifecycle_smoke() {
        let mut e = JpgEnc::new();
        let info = query_attr(&ok_attr()).unwrap();
        assert!(info.memory_size > 0);
        let h = e.open(ok_attr()).unwrap();
        e.encode_picture(h, ok_param(LOCATION_FILE)).unwrap();
        e.complete_encode(h, 50_000).unwrap();
        assert_eq!(e.wait_for_output(h), Ok(50_000));
        // Second encode with different attrs.
        e.encode_with_attr(
            h,
            ok_param(LOCATION_BUFFER),
            Attr { quality: 90, ..ok_attr() },
        )
        .unwrap();
        e.reset(h).unwrap();
        e.close(h).unwrap();
        assert_eq!(e.handle_count(), 0);
    }
}
