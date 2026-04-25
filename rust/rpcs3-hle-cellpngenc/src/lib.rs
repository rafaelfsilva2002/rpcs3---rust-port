//! `rpcs3-hle-cellpngenc` — PNG encoder HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellPngEnc.cpp`. Mirrors the
//! `cellJpgEnc` shape with PNG-specific extras: compression levels 0-9,
//! 5 filter types, ancillary chunks (PLTE/tRNS/sRGB/gAMA/etc), and
//! explicit SPU enable toggle.
//!
//! ## Entry points covered
//!
//! | HLE function                          | Rust wrapper                        |
//! |---------------------------------------|-------------------------------------|
//! | `cellPngEncQueryAttr`                 | [`query_attr`]                      |
//! | `cellPngEncOpen` / `OpenEx`           | [`PngEnc::open`]                    |
//! | `cellPngEncClose`                     | [`PngEnc::close`]                   |
//! | `cellPngEncEncodePicture`             | [`PngEnc::encode_picture`]          |
//! | `cellPngEncWaitForOutput`             | [`PngEnc::wait_for_output`]         |
//! | `cellPngEncReset`                     | [`PngEnc::reset`]                   |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellPngEnc.h:4-16
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ARG: CellError = CellError(0x8061_1291);
    pub const SEQ: CellError = CellError(0x8061_1292);
    pub const BUSY: CellError = CellError(0x8061_1293);
    pub const EMPTY: CellError = CellError(0x8061_1294);
    pub const RESET: CellError = CellError(0x8061_1295);
    pub const FATAL: CellError = CellError(0x8061_1296);

    pub const STREAM_ABORT: CellError = CellError(0x8061_12A1);
    pub const STREAM_SKIP: CellError = CellError(0x8061_12A2);
    pub const STREAM_OVERFLOW: CellError = CellError(0x8061_12A3);
    pub const STREAM_FILE_OPEN: CellError = CellError(0x8061_12A4);
}

// =====================================================================
// Color space (cellPngEnc.h:18-26)
// =====================================================================

pub const CS_GRAYSCALE: u32 = 1;
pub const CS_RGB: u32 = 2;
pub const CS_PALETTE: u32 = 4;
pub const CS_GRAYSCALE_ALPHA: u32 = 9;
pub const CS_RGBA: u32 = 10;
pub const CS_ARGB: u32 = 20;

#[must_use]
pub fn is_known_color_space(cs: u32) -> bool {
    matches!(cs, CS_GRAYSCALE | CS_RGB | CS_PALETTE | CS_GRAYSCALE_ALPHA | CS_RGBA | CS_ARGB)
}

#[must_use]
pub fn bytes_per_pixel(cs: u32) -> Option<u32> {
    Some(match cs {
        CS_GRAYSCALE | CS_PALETTE => 1,
        CS_GRAYSCALE_ALPHA => 2,
        CS_RGB => 3,
        CS_RGBA | CS_ARGB => 4,
        _ => return None,
    })
}

// =====================================================================
// Compression level (cellPngEnc.h:28-40) — auto-numbered 0..=9
// =====================================================================

pub const COMPR_LEVEL_0: u32 = 0;
pub const COMPR_LEVEL_9: u32 = 9;
pub const COMPR_LEVEL_MIN: u32 = COMPR_LEVEL_0;
pub const COMPR_LEVEL_MAX: u32 = COMPR_LEVEL_9;

#[must_use]
pub fn is_known_compr_level(l: u32) -> bool {
    l <= COMPR_LEVEL_MAX
}

// =====================================================================
// Filter type (cellPngEnc.h:42-50) — bitmask, ALL = 0xF8
// =====================================================================

pub const FILTER_TYPE_NONE: u32 = 0x08;
pub const FILTER_TYPE_SUB: u32 = 0x10;
pub const FILTER_TYPE_UP: u32 = 0x20;
pub const FILTER_TYPE_AVG: u32 = 0x40;
pub const FILTER_TYPE_PAETH: u32 = 0x80;
pub const FILTER_TYPE_ALL: u32 = 0xF8;

#[must_use]
pub fn is_known_filter_bits(f: u32) -> bool {
    // At least one filter bit must be set; no bits outside ALL are allowed.
    f != 0 && (f & !FILTER_TYPE_ALL) == 0
}

// =====================================================================
// Chunk types (cellPngEnc.h:52-71) — 17 values, auto-numbered
// =====================================================================

pub const CHUNK_TYPE_PLTE: u32 = 0;
pub const CHUNK_TYPE_TRNS: u32 = 1;
pub const CHUNK_TYPE_CHRM: u32 = 2;
pub const CHUNK_TYPE_GAMA: u32 = 3;
pub const CHUNK_TYPE_ICCP: u32 = 4;
pub const CHUNK_TYPE_SBIT: u32 = 5;
pub const CHUNK_TYPE_SRGB: u32 = 6;
pub const CHUNK_TYPE_TEXT: u32 = 7;
pub const CHUNK_TYPE_BKGD: u32 = 8;
pub const CHUNK_TYPE_HIST: u32 = 9;
pub const CHUNK_TYPE_PHYS: u32 = 10;
pub const CHUNK_TYPE_SPLT: u32 = 11;
pub const CHUNK_TYPE_TIME: u32 = 12;
pub const CHUNK_TYPE_OFFS: u32 = 13;
pub const CHUNK_TYPE_PCAL: u32 = 14;
pub const CHUNK_TYPE_SCAL: u32 = 15;
pub const CHUNK_TYPE_UNKNOWN: u32 = 16;

#[must_use]
pub fn is_known_chunk_type(c: u32) -> bool {
    c <= CHUNK_TYPE_UNKNOWN
}

// =====================================================================
// Output location + limits
// =====================================================================

pub const LOCATION_FILE: u32 = 0;
pub const LOCATION_BUFFER: u32 = 1;

#[must_use]
pub fn is_known_location(l: u32) -> bool {
    matches!(l, LOCATION_FILE | LOCATION_BUFFER)
}

pub const MAX_WIDTH: u32 = 4096;
pub const MAX_HEIGHT: u32 = 4096;
pub const MAX_HANDLES: u32 = 16;

pub const MAX_CMD_QUEUE_DEPTH: u8 = 8;

/// Library version the real lib reports via `CellPngEncAttr`.
pub const VERSION_UPPER: u32 = 0x0001;
pub const VERSION_LOWER: u32 = 0x0000;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    pub max_width: u32,
    pub max_height: u32,
    pub max_bit_depth: u32, // 8 or 16
    pub enable_spu: bool,
    pub add_mem_size: u32,
}

impl Config {
    fn validate(&self) -> Result<(), CellError> {
        if self.max_width == 0 || self.max_width > MAX_WIDTH {
            return Err(errors::ARG);
        }
        if self.max_height == 0 || self.max_height > MAX_HEIGHT {
            return Err(errors::ARG);
        }
        if !matches!(self.max_bit_depth, 8 | 16) {
            return Err(errors::ARG);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub mem_size: u32,
    pub cmd_queue_depth: u8,
    pub version_upper: u32,
    pub version_lower: u32,
}

/// `cellPngEncQueryAttr(config, attr)`. Returns the memory footprint
/// needed for the configured encoder.
pub fn query_attr(config: &Config) -> Result<Attr, CellError> {
    config.validate()?;
    // Raw buffer at 4 bytes/pixel + zlib scratch (~2x) + addMemSize game-supplied.
    let raw = u64::from(config.max_width)
        * u64::from(config.max_height)
        * u64::from(config.max_bit_depth / 8)
        * 4;
    let mem_size = (raw * 3 / 2 + u64::from(config.add_mem_size) + 4096) as u32;
    Ok(Attr { mem_size, cmd_queue_depth: 4, version_upper: VERSION_UPPER, version_lower: VERSION_LOWER })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub pitch_width: u32,
    pub color_space: u32,
    pub bit_depth: u32,
    pub packed_pixel: bool,
    pub picture_addr: u32,
    pub user_data: u64,
}

impl Picture {
    fn validate(&self, config: &Config) -> Result<(), CellError> {
        if self.width == 0 || self.width > config.max_width {
            return Err(errors::ARG);
        }
        if self.height == 0 || self.height > config.max_height {
            return Err(errors::ARG);
        }
        if self.pitch_width < self.width {
            return Err(errors::ARG);
        }
        if !is_known_color_space(self.color_space) {
            return Err(errors::ARG);
        }
        if !matches!(self.bit_depth, 1 | 2 | 4 | 8 | 16) {
            return Err(errors::ARG);
        }
        if self.bit_depth > config.max_bit_depth {
            return Err(errors::ARG);
        }
        if self.picture_addr == 0 {
            return Err(errors::ARG);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeParam {
    pub enable_spu: bool,
    pub encode_color_space: u32,
    pub compression_level: u32,
    pub filter_type: u32,
    pub ancillary_chunks: Vec<u32>, // CHUNK_TYPE_* values
}

impl EncodeParam {
    fn validate(&self) -> Result<(), CellError> {
        if !is_known_color_space(self.encode_color_space) {
            return Err(errors::ARG);
        }
        if !is_known_compr_level(self.compression_level) {
            return Err(errors::ARG);
        }
        if !is_known_filter_bits(self.filter_type) {
            return Err(errors::ARG);
        }
        for &c in &self.ancillary_chunks {
            if !is_known_chunk_type(c) {
                return Err(errors::ARG);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputParam {
    pub location: u32,
    pub stream_file_name: String, // non-empty when LOCATION_FILE
    pub stream_addr: u32,         // non-zero when LOCATION_BUFFER
    pub limit_size: u32,
}

impl OutputParam {
    fn validate(&self) -> Result<(), CellError> {
        if !is_known_location(self.location) {
            return Err(errors::ARG);
        }
        match self.location {
            LOCATION_FILE => {
                if self.stream_file_name.is_empty() {
                    return Err(errors::STREAM_FILE_OPEN);
                }
            }
            LOCATION_BUFFER => {
                if self.stream_addr == 0 {
                    return Err(errors::ARG);
                }
                if self.limit_size == 0 {
                    return Err(errors::STREAM_OVERFLOW);
                }
            }
            _ => return Err(errors::ARG),
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub state: i32,
    pub location: u32,
    pub stream_file_name: String,
    pub stream_addr: u32,
    pub limit_size: u32,
    pub stream_size: u32,
    pub processed_line: u32,
    pub user_data: u64,
}

// =====================================================================
// Handle / encoder state
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
    config: Config,
    last_stream: Option<StreamInfo>,
}

#[derive(Clone, Debug, Default)]
pub struct PngEnc {
    handles: Vec<Handle>,
    next_id: u32,
}

impl PngEnc {
    #[must_use]
    pub fn new() -> Self {
        Self { next_id: 1, ..Default::default() }
    }

    pub fn open(&mut self, config: Config) -> Result<u32, CellError> {
        config.validate()?;
        if self.handles.len() >= MAX_HANDLES as usize {
            return Err(errors::FATAL);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(errors::FATAL)?;
        self.handles.push(Handle { id, state: HandleState::Idle, config, last_stream: None });
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

    pub fn encode_picture(
        &mut self,
        id: u32,
        picture: Picture,
        param: EncodeParam,
        output: OutputParam,
    ) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::Idle {
            return Err(errors::BUSY);
        }
        picture.validate(&self.handles[idx].config)?;
        param.validate()?;
        output.validate()?;
        self.handles[idx].state = HandleState::Encoding;
        Ok(())
    }

    /// Test hook: simulate the async worker completing the encode.
    pub fn complete_encode(
        &mut self,
        id: u32,
        stream_size: u32,
        processed_line: u32,
    ) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::Encoding {
            return Err(errors::SEQ);
        }
        self.handles[idx].state = HandleState::HasOutput;
        self.handles[idx].last_stream = Some(StreamInfo {
            state: 0,
            location: LOCATION_FILE,
            stream_file_name: String::new(),
            stream_addr: 0,
            limit_size: 0,
            stream_size,
            processed_line,
            user_data: 0,
        });
        Ok(())
    }

    pub fn wait_for_output(&mut self, id: u32) -> Result<StreamInfo, CellError> {
        let idx = self.handle_idx(id)?;
        match self.handles[idx].state {
            HandleState::HasOutput => {
                let info = self.handles[idx].last_stream.take().ok_or(errors::SEQ)?;
                self.handles[idx].state = HandleState::Idle;
                Ok(info)
            }
            HandleState::Encoding => Err(errors::BUSY),
            HandleState::Idle => Err(errors::EMPTY),
        }
    }

    pub fn reset(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state == HandleState::Idle {
            return Err(errors::RESET);
        }
        self.handles[idx].state = HandleState::Idle;
        self.handles[idx].last_stream = None;
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

    fn ok_config() -> Config {
        Config { max_width: 1920, max_height: 1080, max_bit_depth: 8, enable_spu: false, add_mem_size: 0 }
    }

    fn ok_picture() -> Picture {
        Picture {
            width: 1280,
            height: 720,
            pitch_width: 1280,
            color_space: CS_RGBA,
            bit_depth: 8,
            packed_pixel: true,
            picture_addr: 0x10_0000,
            user_data: 0xDEAD_BEEF,
        }
    }

    fn ok_encode_param() -> EncodeParam {
        EncodeParam {
            enable_spu: false,
            encode_color_space: CS_RGBA,
            compression_level: 6,
            filter_type: FILTER_TYPE_SUB,
            ancillary_chunks: vec![CHUNK_TYPE_SRGB, CHUNK_TYPE_GAMA],
        }
    }

    fn ok_output_file() -> OutputParam {
        OutputParam {
            location: LOCATION_FILE,
            stream_file_name: "/dev_hdd0/photo/out.png".into(),
            stream_addr: 0,
            limit_size: 0,
        }
    }

    fn ok_output_buffer() -> OutputParam {
        OutputParam {
            location: LOCATION_BUFFER,
            stream_file_name: String::new(),
            stream_addr: 0x2000,
            limit_size: 1024 * 1024,
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::ARG.0, 0x8061_1291);
        assert_eq!(errors::SEQ.0, 0x8061_1292);
        assert_eq!(errors::BUSY.0, 0x8061_1293);
        assert_eq!(errors::EMPTY.0, 0x8061_1294);
        assert_eq!(errors::RESET.0, 0x8061_1295);
        assert_eq!(errors::FATAL.0, 0x8061_1296);
        assert_eq!(errors::STREAM_ABORT.0, 0x8061_12A1);
        assert_eq!(errors::STREAM_SKIP.0, 0x8061_12A2);
        assert_eq!(errors::STREAM_OVERFLOW.0, 0x8061_12A3);
        assert_eq!(errors::STREAM_FILE_OPEN.0, 0x8061_12A4);
    }

    #[test]
    fn color_space_constants_stable() {
        assert_eq!(CS_GRAYSCALE, 1);
        assert_eq!(CS_RGB, 2);
        assert_eq!(CS_PALETTE, 4);
        assert_eq!(CS_GRAYSCALE_ALPHA, 9);
        assert_eq!(CS_RGBA, 10);
        assert_eq!(CS_ARGB, 20);
    }

    #[test]
    fn compression_level_range_stable() {
        assert_eq!(COMPR_LEVEL_0, 0);
        assert_eq!(COMPR_LEVEL_9, 9);
        assert_eq!(COMPR_LEVEL_MIN, 0);
        assert_eq!(COMPR_LEVEL_MAX, 9);
        for l in 0..=9 {
            assert!(is_known_compr_level(l));
        }
        assert!(!is_known_compr_level(10));
    }

    #[test]
    fn filter_type_bitmask_stable() {
        assert_eq!(FILTER_TYPE_NONE, 0x08);
        assert_eq!(FILTER_TYPE_SUB, 0x10);
        assert_eq!(FILTER_TYPE_UP, 0x20);
        assert_eq!(FILTER_TYPE_AVG, 0x40);
        assert_eq!(FILTER_TYPE_PAETH, 0x80);
        assert_eq!(FILTER_TYPE_ALL, 0xF8);
    }

    #[test]
    fn is_known_filter_bits_logic() {
        assert!(is_known_filter_bits(FILTER_TYPE_SUB));
        assert!(is_known_filter_bits(FILTER_TYPE_ALL));
        assert!(is_known_filter_bits(FILTER_TYPE_SUB | FILTER_TYPE_UP | FILTER_TYPE_AVG));
        assert!(!is_known_filter_bits(0));
        assert!(!is_known_filter_bits(0x01)); // bit outside ALL mask
        assert!(!is_known_filter_bits(0xFF)); // has 0x07 low bits outside ALL
    }

    #[test]
    fn chunk_type_enum_stable() {
        assert_eq!(CHUNK_TYPE_PLTE, 0);
        assert_eq!(CHUNK_TYPE_TRNS, 1);
        assert_eq!(CHUNK_TYPE_GAMA, 3);
        assert_eq!(CHUNK_TYPE_SRGB, 6);
        assert_eq!(CHUNK_TYPE_TIME, 12);
        assert_eq!(CHUNK_TYPE_SCAL, 15);
        assert_eq!(CHUNK_TYPE_UNKNOWN, 16);
    }

    #[test]
    fn location_constants_stable() {
        assert_eq!(LOCATION_FILE, 0);
        assert_eq!(LOCATION_BUFFER, 1);
    }

    #[test]
    fn limits_stable() {
        assert_eq!(MAX_WIDTH, 4096);
        assert_eq!(MAX_HEIGHT, 4096);
        assert_eq!(MAX_HANDLES, 16);
    }

    #[test]
    fn bytes_per_pixel_matches_color_space() {
        assert_eq!(bytes_per_pixel(CS_GRAYSCALE), Some(1));
        assert_eq!(bytes_per_pixel(CS_PALETTE), Some(1));
        assert_eq!(bytes_per_pixel(CS_GRAYSCALE_ALPHA), Some(2));
        assert_eq!(bytes_per_pixel(CS_RGB), Some(3));
        assert_eq!(bytes_per_pixel(CS_RGBA), Some(4));
        assert_eq!(bytes_per_pixel(CS_ARGB), Some(4));
        assert_eq!(bytes_per_pixel(99), None);
    }

    #[test]
    fn config_validate_happy_path() {
        ok_config().validate().unwrap();
    }

    #[test]
    fn config_validate_zero_dims_rejected() {
        let mut c = ok_config();
        c.max_width = 0;
        assert_eq!(c.validate(), Err(errors::ARG));
    }

    #[test]
    fn config_validate_oversize_dims_rejected() {
        let mut c = ok_config();
        c.max_height = MAX_HEIGHT + 1;
        assert_eq!(c.validate(), Err(errors::ARG));
    }

    #[test]
    fn config_validate_bad_bit_depth_rejected() {
        let mut c = ok_config();
        c.max_bit_depth = 4;
        assert_eq!(c.validate(), Err(errors::ARG));
    }

    #[test]
    fn query_attr_returns_sensible_mem_size() {
        let attr = query_attr(&ok_config()).unwrap();
        assert!(attr.mem_size > 0);
        assert_eq!(attr.version_upper, VERSION_UPPER);
        assert_eq!(attr.version_lower, VERSION_LOWER);
    }

    #[test]
    fn query_attr_add_mem_size_included() {
        let mut c = ok_config();
        c.add_mem_size = 100_000;
        let attr = query_attr(&c).unwrap();
        assert!(attr.mem_size >= 100_000);
    }

    #[test]
    fn query_attr_bad_config_rejected() {
        let mut c = ok_config();
        c.max_bit_depth = 99;
        assert_eq!(query_attr(&c), Err(errors::ARG));
    }

    #[test]
    fn open_allocates_incrementing_ids() {
        let mut e = PngEnc::new();
        let a = e.open(ok_config()).unwrap();
        let b = e.open(ok_config()).unwrap();
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(e.handle_count(), 2);
    }

    #[test]
    fn open_bad_config_rejected() {
        let mut e = PngEnc::new();
        let mut c = ok_config();
        c.max_width = 0;
        assert_eq!(e.open(c), Err(errors::ARG));
    }

    #[test]
    fn open_exceeds_max_handles_rejected() {
        let mut e = PngEnc::new();
        for _ in 0..MAX_HANDLES {
            e.open(ok_config()).unwrap();
        }
        assert_eq!(e.open(ok_config()), Err(errors::FATAL));
    }

    #[test]
    fn close_bad_id_rejected() {
        let mut e = PngEnc::new();
        assert_eq!(e.close(999), Err(errors::ARG));
    }

    #[test]
    fn close_while_encoding_is_busy() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()).unwrap();
        assert_eq!(e.close(h), Err(errors::BUSY));
    }

    #[test]
    fn picture_validate_zero_dims_rejected() {
        let c = ok_config();
        let mut p = ok_picture();
        p.width = 0;
        assert_eq!(p.validate(&c), Err(errors::ARG));
    }

    #[test]
    fn picture_validate_over_max_rejected() {
        let c = ok_config();
        let mut p = ok_picture();
        p.height = c.max_height + 1;
        assert_eq!(p.validate(&c), Err(errors::ARG));
    }

    #[test]
    fn picture_validate_pitch_less_than_width_rejected() {
        let c = ok_config();
        let mut p = ok_picture();
        p.pitch_width = p.width - 1;
        assert_eq!(p.validate(&c), Err(errors::ARG));
    }

    #[test]
    fn picture_validate_bit_depth_over_max_rejected() {
        let c = ok_config(); // max 8
        let mut p = ok_picture();
        p.bit_depth = 16;
        assert_eq!(p.validate(&c), Err(errors::ARG));
    }

    #[test]
    fn picture_validate_bad_bit_depth_rejected() {
        let c = ok_config();
        let mut p = ok_picture();
        p.bit_depth = 3;
        assert_eq!(p.validate(&c), Err(errors::ARG));
    }

    #[test]
    fn picture_validate_null_addr_rejected() {
        let c = ok_config();
        let mut p = ok_picture();
        p.picture_addr = 0;
        assert_eq!(p.validate(&c), Err(errors::ARG));
    }

    #[test]
    fn encode_param_validate_bad_compression_rejected() {
        let mut p = ok_encode_param();
        p.compression_level = 10;
        assert_eq!(p.validate(), Err(errors::ARG));
    }

    #[test]
    fn encode_param_validate_empty_filter_rejected() {
        let mut p = ok_encode_param();
        p.filter_type = 0;
        assert_eq!(p.validate(), Err(errors::ARG));
    }

    #[test]
    fn encode_param_validate_unknown_chunk_rejected() {
        let mut p = ok_encode_param();
        p.ancillary_chunks.push(99);
        assert_eq!(p.validate(), Err(errors::ARG));
    }

    #[test]
    fn output_param_file_missing_name_is_stream_file_open() {
        let mut o = ok_output_file();
        o.stream_file_name.clear();
        assert_eq!(o.validate(), Err(errors::STREAM_FILE_OPEN));
    }

    #[test]
    fn output_param_buffer_null_addr_rejected() {
        let mut o = ok_output_buffer();
        o.stream_addr = 0;
        assert_eq!(o.validate(), Err(errors::ARG));
    }

    #[test]
    fn output_param_buffer_zero_limit_is_stream_overflow() {
        let mut o = ok_output_buffer();
        o.limit_size = 0;
        assert_eq!(o.validate(), Err(errors::STREAM_OVERFLOW));
    }

    #[test]
    fn encode_picture_happy_path_file() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_file()).unwrap();
    }

    #[test]
    fn encode_picture_happy_path_buffer() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()).unwrap();
    }

    #[test]
    fn encode_picture_while_encoding_is_busy() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()).unwrap();
        assert_eq!(
            e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()),
            Err(errors::BUSY)
        );
    }

    #[test]
    fn wait_for_output_idle_is_empty() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        assert_eq!(e.wait_for_output(h).err(), Some(errors::EMPTY));
    }

    #[test]
    fn wait_for_output_during_encode_is_busy() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()).unwrap();
        assert_eq!(e.wait_for_output(h).err(), Some(errors::BUSY));
    }

    #[test]
    fn wait_for_output_after_complete_returns_stream_info() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()).unwrap();
        e.complete_encode(h, 80_000, 720).unwrap();
        let info = e.wait_for_output(h).unwrap();
        assert_eq!(info.stream_size, 80_000);
        assert_eq!(info.processed_line, 720);
        // After consuming: back to Idle.
        assert_eq!(e.wait_for_output(h).err(), Some(errors::EMPTY));
    }

    #[test]
    fn complete_encode_without_encoding_is_seq() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        assert_eq!(e.complete_encode(h, 0, 0), Err(errors::SEQ));
    }

    #[test]
    fn reset_idle_is_reset_error() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        assert_eq!(e.reset(h), Err(errors::RESET));
    }

    #[test]
    fn reset_during_encode_cancels() {
        let mut e = PngEnc::new();
        let h = e.open(ok_config()).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()).unwrap();
        e.reset(h).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()).unwrap();
    }

    #[test]
    fn encode_with_16bit_picture_allowed_when_config_allows() {
        let mut e = PngEnc::new();
        let mut c = ok_config();
        c.max_bit_depth = 16;
        let h = e.open(c).unwrap();
        let mut p = ok_picture();
        p.bit_depth = 16;
        e.encode_picture(h, p, ok_encode_param(), ok_output_buffer()).unwrap();
    }

    #[test]
    fn encode_with_palette_accepts_1_2_4_8_bit() {
        let c = ok_config();
        for depth in [1, 2, 4, 8] {
            let mut p = ok_picture();
            p.color_space = CS_PALETTE;
            p.bit_depth = depth;
            p.validate(&c).unwrap();
        }
    }

    #[test]
    fn full_png_enc_lifecycle_smoke() {
        let mut e = PngEnc::new();
        let attr = query_attr(&ok_config()).unwrap();
        assert!(attr.mem_size > 0);
        let h = e.open(ok_config()).unwrap();
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_file()).unwrap();
        e.complete_encode(h, 50_000, 720).unwrap();
        let info = e.wait_for_output(h).unwrap();
        assert_eq!(info.stream_size, 50_000);
        // Second encode to buffer.
        e.encode_picture(h, ok_picture(), ok_encode_param(), ok_output_buffer()).unwrap();
        e.reset(h).unwrap();
        e.close(h).unwrap();
        assert_eq!(e.handle_count(), 0);
    }
}
