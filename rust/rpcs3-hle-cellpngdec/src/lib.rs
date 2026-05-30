//! `rpcs3-hle-cellpngdec` — PNG decoder HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellPngDec.cpp`. Mirrors the cellJpgDec
//! shape but with PNG-specific metadata (bit depth, interlace, alpha
//! selector, pack flag). Surface: `Create → Open → ReadHeader →
//! SetParameter → DecodeData → Close`.
//!
//! ## Entry points covered
//!
//! | HLE function                    | Rust wrapper                        |
//! |---------------------------------|-------------------------------------|
//! | `cellPngDecCreate`              | [`PngDec::create`]                  |
//! | `cellPngDecDestroy`             | [`PngDec::destroy`]                 |
//! | `cellPngDecOpen`                | [`PngDec::open`]                    |
//! | `cellPngDecClose`               | [`PngDec::close`]                   |
//! | `cellPngDecReadHeader`          | [`PngDec::read_header`]             |
//! | `cellPngDecSetParameter`        | [`PngDec::set_parameter`]           |
//! | `cellPngDecDecodeData`          | [`PngDec::decode_data`]             |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellPngDec.h:13-25
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const HEADER: CellError = CellError(0x8061_1201);
    pub const STREAM_FORMAT: CellError = CellError(0x8061_1202);
    pub const ARG: CellError = CellError(0x8061_1203);
    pub const SEQ: CellError = CellError(0x8061_1204);
    pub const BUSY: CellError = CellError(0x8061_1205);
    pub const FATAL: CellError = CellError(0x8061_1206);
    pub const OPEN_FILE: CellError = CellError(0x8061_1207);
    pub const SPU_UNSUPPORT: CellError = CellError(0x8061_1208);
    pub const SPU_ERROR: CellError = CellError(0x8061_1209);
    pub const CB_PARAM: CellError = CellError(0x8061_120a);
}

// =====================================================================
// Version + enums (cellPngDec.h:7-95)
// =====================================================================

pub const CODEC_VERSION: u32 = 0x0042_0000;

/// PNG signature per RFC 2083 §3.1 — first 8 bytes of every PNG file.
pub const MAGIC_PNG: &[u8; 8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

// Color space constants (byte-exact).
pub const CS_GRAYSCALE: i32 = 1;
pub const CS_RGB: i32 = 2;
pub const CS_PALETTE: i32 = 4;
pub const CS_GRAYSCALE_ALPHA: i32 = 9;
pub const CS_RGBA: i32 = 10;
pub const CS_ARGB: i32 = 20;

#[must_use]
pub fn is_known_color_space(cs: i32) -> bool {
    matches!(cs, CS_GRAYSCALE | CS_RGB | CS_PALETTE | CS_GRAYSCALE_ALPHA | CS_RGBA | CS_ARGB)
}

#[must_use]
pub fn bytes_per_pixel(cs: i32) -> Option<u32> {
    Some(match cs {
        CS_GRAYSCALE | CS_PALETTE => 1,
        CS_GRAYSCALE_ALPHA => 2,
        CS_RGB => 3,
        CS_RGBA | CS_ARGB => 4,
        _ => return None,
    })
}

pub const SPU_THREAD_DISABLE: i32 = 0;
pub const SPU_THREAD_ENABLE: i32 = 1;

pub const SRC_FILE: i32 = 0;
pub const SRC_BUFFER: i32 = 1;

pub const NO_INTERLACE: i32 = 0;
pub const ADAM7_INTERLACE: i32 = 1;

pub const OUT_TOP_TO_BOTTOM: i32 = 0;
pub const OUT_BOTTOM_TO_TOP: i32 = 1;

pub const PACK_1BYTE_PER_NPIXEL: i32 = 0;
pub const PACK_1BYTE_PER_1PIXEL: i32 = 1;

pub const ALPHA_STREAM: i32 = 0;
pub const ALPHA_FIX: i32 = 1;

pub const COMMAND_CONTINUE: i32 = 0;
pub const COMMAND_STOP: i32 = 1;

pub const DEC_STATUS_FINISH: i32 = 0;
pub const DEC_STATUS_STOP: i32 = 1;

pub const BUFFER_MODE_LINE: i32 = 1;

pub const SPU_MODE_RECEIVE_EVENT: i32 = 0;
pub const SPU_MODE_TRYRECEIVE_EVENT: i32 = 1;

// PNG spec-valid bit depths per color space (RFC 2083 §11.2.2).
#[must_use]
pub fn is_valid_bit_depth(cs: i32, depth: u32) -> bool {
    match cs {
        CS_GRAYSCALE => matches!(depth, 1 | 2 | 4 | 8 | 16),
        CS_PALETTE => matches!(depth, 1 | 2 | 4 | 8),
        CS_RGB | CS_RGBA | CS_ARGB | CS_GRAYSCALE_ALPHA => matches!(depth, 8 | 16),
        _ => false,
    }
}

// Chunk-presence flags (commonly queried by games via CellPngDecInfo.chunkInformation).
pub const CHUNK_IHDR: u32 = 1 << 0;
pub const CHUNK_PLTE: u32 = 1 << 1;
pub const CHUNK_IDAT: u32 = 1 << 2;
pub const CHUNK_IEND: u32 = 1 << 3;
pub const CHUNK_tRNS: u32 = 1 << 4;
pub const CHUNK_gAMA: u32 = 1 << 5;

// Handle cap (same pattern as cellJpgDec id pool).
pub const MAX_HANDLES: u32 = 1023;
pub const HANDLE_ID_BASE: u32 = 1;

pub const MAX_WIDTH: u32 = 8192;
pub const MAX_HEIGHT: u32 = 8192;

// =====================================================================
// Structs
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub image_width: u32,
    pub image_height: u32,
    pub num_components: u32,
    pub color_space: i32,
    pub bit_depth: u32,
    pub interlace_method: i32,
    pub chunk_information: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Src {
    pub src_select: i32,
    pub file_name: String,
    pub file_offset: i64,
    pub file_size: u32,
    pub stream_ptr: u32,
    pub stream_size: u32,
    pub spu_thread_enable: i32,
}

impl Src {
    fn validate(&self) -> Result<(), CellError> {
        match self.src_select {
            SRC_FILE => {
                if self.file_name.is_empty() {
                    return Err(errors::OPEN_FILE);
                }
                if self.file_size == 0 {
                    return Err(errors::OPEN_FILE);
                }
            }
            SRC_BUFFER => {
                if self.stream_ptr == 0 || self.stream_size == 0 {
                    return Err(errors::ARG);
                }
            }
            _ => return Err(errors::ARG),
        }
        if !matches!(self.spu_thread_enable, SPU_THREAD_DISABLE | SPU_THREAD_ENABLE) {
            return Err(errors::ARG);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InParam {
    pub output_mode: i32,
    pub output_color_space: i32,
    pub output_bit_depth: u32,
    pub output_packing: i32,
    pub output_alpha_select: i32,
    pub output_color_alpha: u8,
}

impl InParam {
    fn validate(&self) -> Result<(), CellError> {
        if !matches!(self.output_mode, OUT_TOP_TO_BOTTOM | OUT_BOTTOM_TO_TOP) {
            return Err(errors::ARG);
        }
        if !is_known_color_space(self.output_color_space) {
            return Err(errors::ARG);
        }
        if !matches!(self.output_bit_depth, 8 | 16) {
            return Err(errors::ARG);
        }
        if !matches!(self.output_packing, PACK_1BYTE_PER_NPIXEL | PACK_1BYTE_PER_1PIXEL) {
            return Err(errors::ARG);
        }
        if !matches!(self.output_alpha_select, ALPHA_STREAM | ALPHA_FIX) {
            return Err(errors::ARG);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutParam {
    pub output_width_byte: u64,
    pub output_width: u32,
    pub output_height: u32,
    pub output_components: u32,
    pub output_bit_depth: u32,
    pub output_mode: i32,
    pub output_color_space: i32,
    pub use_memory_space: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataOutInfo {
    pub output_lines: u32,
    pub status: i32,
}

// =====================================================================
// Handle + decoder state
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
enum HandleState {
    Opened,
    HeaderRead,
    Configured,
}

#[derive(Clone, Debug)]
struct Handle {
    id: u32,
    state: HandleState,
    info: Option<Info>,
    in_param: Option<InParam>,
    out_param: Option<OutParam>,
    #[allow(dead_code)] // stored for future real-decoder pass
    src: Src,
}

#[derive(Clone, Debug, Default)]
pub struct PngDec {
    mem_space_allocated: u32,
    handles: Vec<Handle>,
    next_id: u32,
}

impl PngDec {
    #[must_use]
    pub fn new() -> Self {
        Self { next_id: HANDLE_ID_BASE, ..Default::default() }
    }

    pub fn create(&mut self, mem_space: u32) -> Result<(), CellError> {
        if self.mem_space_allocated != 0 {
            return Err(errors::BUSY);
        }
        if mem_space == 0 {
            return Err(errors::ARG);
        }
        self.mem_space_allocated = mem_space;
        Ok(())
    }

    pub fn destroy(&mut self) -> Result<(), CellError> {
        if self.mem_space_allocated == 0 {
            return Err(errors::SEQ);
        }
        if !self.handles.is_empty() {
            return Err(errors::BUSY);
        }
        self.mem_space_allocated = 0;
        Ok(())
    }

    pub fn open(&mut self, src: Src) -> Result<u32, CellError> {
        if self.mem_space_allocated == 0 {
            return Err(errors::SEQ);
        }
        src.validate()?;
        if self.handles.len() >= MAX_HANDLES as usize {
            return Err(errors::FATAL);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(errors::FATAL)?;
        self.handles.push(Handle {
            id,
            state: HandleState::Opened,
            info: None,
            in_param: None,
            out_param: None,
            src,
        });
        Ok(id)
    }

    pub fn close(&mut self, id: u32) -> Result<(), CellError> {
        let idx = self.handle_idx(id)?;
        self.handles.remove(idx);
        Ok(())
    }

    pub fn read_header(&mut self, id: u32, header_bytes: &[u8], info: Info) -> Result<Info, CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::Opened {
            return Err(errors::SEQ);
        }
        if header_bytes.len() < MAGIC_PNG.len() || &header_bytes[..MAGIC_PNG.len()] != MAGIC_PNG {
            return Err(errors::HEADER);
        }
        if info.image_width == 0 || info.image_height == 0 {
            return Err(errors::STREAM_FORMAT);
        }
        if info.image_width > MAX_WIDTH || info.image_height > MAX_HEIGHT {
            return Err(errors::STREAM_FORMAT);
        }
        if !is_known_color_space(info.color_space) {
            return Err(errors::STREAM_FORMAT);
        }
        if !is_valid_bit_depth(info.color_space, info.bit_depth) {
            return Err(errors::STREAM_FORMAT);
        }
        if !matches!(info.interlace_method, NO_INTERLACE | ADAM7_INTERLACE) {
            return Err(errors::STREAM_FORMAT);
        }
        if (info.chunk_information & CHUNK_IHDR) == 0 {
            return Err(errors::STREAM_FORMAT);
        }
        self.handles[idx].info = Some(info);
        self.handles[idx].state = HandleState::HeaderRead;
        Ok(info)
    }

    /// Parse a PNG header byte-exact to what RPCS3 `cellPngDecReadHeader` reports
    /// (cellPngDec.cpp:558-564 `pngSetHeader`): the values libpng's
    /// `png_get_image_*` return come straight from the IHDR chunk. Layout: 8-byte
    /// signature, then `[len=13][="IHDR"][width@16][height@20][bitDepth@24]
    /// [colorType@25][compress@26][filter@27][interlace@28]`. numComponents +
    /// colorSpace are derived from colorType (getPngDecColourType / png_get_channels).
    /// `chunk_information` (an ancillary-chunk bitmask in RPCS3) is left 0 — not
    /// part of the IHDR and not inspected by the header oracle.
    pub fn parse_header(bytes: &[u8]) -> Result<Info, CellError> {
        if bytes.len() < 29 || &bytes[..8] != MAGIC_PNG || &bytes[12..16] != b"IHDR" {
            return Err(errors::HEADER);
        }
        let be = |o: usize| {
            u32::from_be_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
        };
        let color_type = bytes[25];
        // png_get_channels (color_type -> channels) + getPngDecColourType.
        let (num_components, color_space) = match color_type {
            0 => (1, CS_GRAYSCALE),
            2 => (3, CS_RGB),
            3 => (1, CS_PALETTE),
            4 => (2, CS_GRAYSCALE_ALPHA),
            6 => (4, CS_RGBA),
            _ => return Err(errors::STREAM_FORMAT),
        };
        Ok(Info {
            image_width: be(16),
            image_height: be(20),
            num_components,
            color_space,
            bit_depth: u32::from(bytes[24]),
            interlace_method: i32::from(bytes[28]),
            chunk_information: 0,
        })
    }

    /// The (stream_ptr, stream_size) of an opened BUFFER-source handle — lets the
    /// emu-core ReadHeader arm fetch the PNG bytes from guest memory.
    #[must_use]
    pub fn stream_window(&self, id: u32) -> Option<(u32, u32)> {
        let h = self.handles.iter().find(|h| h.id == id)?;
        Some((h.src.stream_ptr, h.src.stream_size))
    }

    pub fn set_parameter(&mut self, id: u32, in_param: InParam) -> Result<OutParam, CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::HeaderRead {
            return Err(errors::SEQ);
        }
        in_param.validate()?;
        let info = self.handles[idx].info.ok_or(errors::SEQ)?;
        let bpp = bytes_per_pixel(in_param.output_color_space).ok_or(errors::ARG)?;
        // If bit_depth = 16 we double the pixel stride (PNG stores 16-bit per channel).
        let bpp_adjusted = if in_param.output_bit_depth == 16 { bpp * 2 } else { bpp };
        let output_width = info.image_width;
        let output_height = info.image_height;
        let width_byte = u64::from(output_width) * u64::from(bpp_adjusted);
        let out = OutParam {
            output_width_byte: width_byte,
            output_width,
            output_height,
            output_components: bpp,
            output_bit_depth: in_param.output_bit_depth,
            output_mode: in_param.output_mode,
            output_color_space: in_param.output_color_space,
            use_memory_space: (width_byte * u64::from(output_height)) as u32,
        };
        self.handles[idx].in_param = Some(in_param);
        self.handles[idx].out_param = Some(out);
        self.handles[idx].state = HandleState::Configured;
        Ok(out)
    }

    pub fn decode_data(&mut self, id: u32) -> Result<DataOutInfo, CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::Configured {
            return Err(errors::SEQ);
        }
        let out = self.handles[idx].out_param.ok_or(errors::SEQ)?;
        Ok(DataOutInfo { output_lines: out.output_height, status: DEC_STATUS_FINISH })
    }

    #[must_use]
    pub fn init_space_allocated(&self) -> u32 {
        self.mem_space_allocated
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

    fn buffer_src() -> Src {
        Src {
            src_select: SRC_BUFFER,
            file_name: String::new(),
            file_offset: 0,
            file_size: 0,
            stream_ptr: 0x1000,
            stream_size: 2048,
            spu_thread_enable: SPU_THREAD_DISABLE,
        }
    }

    fn file_src() -> Src {
        Src {
            src_select: SRC_FILE,
            file_name: "/dev_hdd0/photo/test.png".into(),
            file_offset: 0,
            file_size: 4096,
            stream_ptr: 0,
            stream_size: 0,
            spu_thread_enable: SPU_THREAD_DISABLE,
        }
    }

    fn ok_info() -> Info {
        Info {
            image_width: 512,
            image_height: 384,
            num_components: 4,
            color_space: CS_RGBA,
            bit_depth: 8,
            interlace_method: NO_INTERLACE,
            chunk_information: CHUNK_IHDR | CHUNK_IDAT | CHUNK_IEND,
        }
    }

    fn ok_in_param() -> InParam {
        InParam {
            output_mode: OUT_TOP_TO_BOTTOM,
            output_color_space: CS_RGBA,
            output_bit_depth: 8,
            output_packing: PACK_1BYTE_PER_1PIXEL,
            output_alpha_select: ALPHA_STREAM,
            output_color_alpha: 0xFF,
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::HEADER.0, 0x8061_1201);
        assert_eq!(errors::STREAM_FORMAT.0, 0x8061_1202);
        assert_eq!(errors::ARG.0, 0x8061_1203);
        assert_eq!(errors::SEQ.0, 0x8061_1204);
        assert_eq!(errors::BUSY.0, 0x8061_1205);
        assert_eq!(errors::FATAL.0, 0x8061_1206);
        assert_eq!(errors::OPEN_FILE.0, 0x8061_1207);
        assert_eq!(errors::SPU_UNSUPPORT.0, 0x8061_1208);
        assert_eq!(errors::SPU_ERROR.0, 0x8061_1209);
        assert_eq!(errors::CB_PARAM.0, 0x8061_120a);
    }

    #[test]
    fn codec_version_stable() {
        assert_eq!(CODEC_VERSION, 0x0042_0000);
    }

    #[test]
    fn magic_png_stable() {
        assert_eq!(MAGIC_PNG.len(), 8);
        assert_eq!(MAGIC_PNG[0], 0x89);
        assert_eq!(&MAGIC_PNG[1..4], b"PNG");
        assert_eq!(&MAGIC_PNG[4..], &[0x0D, 0x0A, 0x1A, 0x0A]);
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
    fn enum_constants_stable() {
        assert_eq!(SPU_THREAD_DISABLE, 0);
        assert_eq!(SPU_THREAD_ENABLE, 1);
        assert_eq!(SRC_FILE, 0);
        assert_eq!(SRC_BUFFER, 1);
        assert_eq!(NO_INTERLACE, 0);
        assert_eq!(ADAM7_INTERLACE, 1);
        assert_eq!(OUT_TOP_TO_BOTTOM, 0);
        assert_eq!(OUT_BOTTOM_TO_TOP, 1);
        assert_eq!(PACK_1BYTE_PER_NPIXEL, 0);
        assert_eq!(PACK_1BYTE_PER_1PIXEL, 1);
        assert_eq!(ALPHA_STREAM, 0);
        assert_eq!(ALPHA_FIX, 1);
        assert_eq!(COMMAND_CONTINUE, 0);
        assert_eq!(COMMAND_STOP, 1);
        assert_eq!(DEC_STATUS_FINISH, 0);
        assert_eq!(DEC_STATUS_STOP, 1);
        assert_eq!(BUFFER_MODE_LINE, 1);
    }

    #[test]
    fn bytes_per_pixel_covers_all_color_spaces() {
        assert_eq!(bytes_per_pixel(CS_GRAYSCALE), Some(1));
        assert_eq!(bytes_per_pixel(CS_PALETTE), Some(1));
        assert_eq!(bytes_per_pixel(CS_GRAYSCALE_ALPHA), Some(2));
        assert_eq!(bytes_per_pixel(CS_RGB), Some(3));
        assert_eq!(bytes_per_pixel(CS_RGBA), Some(4));
        assert_eq!(bytes_per_pixel(CS_ARGB), Some(4));
        assert_eq!(bytes_per_pixel(99), None);
    }

    #[test]
    fn is_valid_bit_depth_matches_png_spec() {
        for d in [1u32, 2, 4, 8, 16] {
            assert!(is_valid_bit_depth(CS_GRAYSCALE, d));
        }
        for d in [1u32, 2, 4, 8] {
            assert!(is_valid_bit_depth(CS_PALETTE, d));
        }
        assert!(!is_valid_bit_depth(CS_PALETTE, 16));
        for cs in [CS_RGB, CS_RGBA, CS_ARGB, CS_GRAYSCALE_ALPHA] {
            assert!(is_valid_bit_depth(cs, 8));
            assert!(is_valid_bit_depth(cs, 16));
            assert!(!is_valid_bit_depth(cs, 4));
            assert!(!is_valid_bit_depth(cs, 1));
        }
        assert!(!is_valid_bit_depth(99, 8));
    }

    #[test]
    fn chunk_flags_stable() {
        assert_eq!(CHUNK_IHDR, 1);
        assert_eq!(CHUNK_PLTE, 2);
        assert_eq!(CHUNK_IDAT, 4);
        assert_eq!(CHUNK_IEND, 8);
        assert_eq!(CHUNK_tRNS, 16);
        assert_eq!(CHUNK_gAMA, 32);
    }

    #[test]
    fn max_handles_and_dims_stable() {
        assert_eq!(MAX_HANDLES, 1023);
        assert_eq!(HANDLE_ID_BASE, 1);
        assert_eq!(MAX_WIDTH, 8192);
        assert_eq!(MAX_HEIGHT, 8192);
    }

    #[test]
    fn create_zero_space_rejected() {
        let mut d = PngDec::new();
        assert_eq!(d.create(0), Err(errors::ARG));
    }

    #[test]
    fn create_twice_is_busy() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        assert_eq!(d.create(1024), Err(errors::BUSY));
    }

    #[test]
    fn destroy_without_create_is_seq() {
        let mut d = PngDec::new();
        assert_eq!(d.destroy(), Err(errors::SEQ));
    }

    #[test]
    fn destroy_with_handles_is_busy() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        d.open(buffer_src()).unwrap();
        assert_eq!(d.destroy(), Err(errors::BUSY));
    }

    #[test]
    fn open_before_create_is_seq() {
        let mut d = PngDec::new();
        assert_eq!(d.open(buffer_src()), Err(errors::SEQ));
    }

    #[test]
    fn open_bad_src_select_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let mut s = buffer_src();
        s.src_select = 99;
        assert_eq!(d.open(s), Err(errors::ARG));
    }

    #[test]
    fn open_file_empty_name_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let mut s = file_src();
        s.file_name.clear();
        assert_eq!(d.open(s), Err(errors::OPEN_FILE));
    }

    #[test]
    fn open_buffer_null_ptr_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let mut s = buffer_src();
        s.stream_ptr = 0;
        assert_eq!(d.open(s), Err(errors::ARG));
    }

    #[test]
    fn open_bad_spu_thread_enable_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let mut s = buffer_src();
        s.spu_thread_enable = 99;
        assert_eq!(d.open(s), Err(errors::ARG));
    }

    #[test]
    fn open_allocates_incrementing_ids() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let a = d.open(buffer_src()).unwrap();
        let b = d.open(buffer_src()).unwrap();
        assert_eq!(a, HANDLE_ID_BASE);
        assert_eq!(b, a + 1);
    }

    #[test]
    fn close_bad_id_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        assert_eq!(d.close(999), Err(errors::ARG));
    }

    #[test]
    fn read_header_bad_magic_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        assert_eq!(d.read_header(h, &[0xFF; 8], ok_info()), Err(errors::HEADER));
    }

    #[test]
    fn read_header_too_short_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        assert_eq!(d.read_header(h, &MAGIC_PNG[..4], ok_info()), Err(errors::HEADER));
    }

    #[test]
    fn read_header_zero_dims_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.image_height = 0;
        assert_eq!(d.read_header(h, MAGIC_PNG, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_oversize_dims_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.image_width = MAX_WIDTH + 1;
        assert_eq!(d.read_header(h, MAGIC_PNG, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_unknown_color_space_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.color_space = 99;
        assert_eq!(d.read_header(h, MAGIC_PNG, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_invalid_bit_depth_for_palette_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.color_space = CS_PALETTE;
        info.bit_depth = 16; // palette images can't be 16-bit
        assert_eq!(d.read_header(h, MAGIC_PNG, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_missing_ihdr_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.chunk_information = CHUNK_IDAT | CHUNK_IEND;
        assert_eq!(d.read_header(h, MAGIC_PNG, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_bad_interlace_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.interlace_method = 5;
        assert_eq!(d.read_header(h, MAGIC_PNG, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_happy_path() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let info = d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        assert_eq!(info.image_width, 512);
    }

    #[test]
    fn read_header_twice_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        assert_eq!(d.read_header(h, MAGIC_PNG, ok_info()), Err(errors::SEQ));
    }

    #[test]
    fn set_parameter_bad_bit_depth_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.output_bit_depth = 4;
        assert_eq!(d.set_parameter(h, p).err(), Some(errors::ARG));
    }

    #[test]
    fn set_parameter_bad_packing_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.output_packing = 99;
        assert_eq!(d.set_parameter(h, p).err(), Some(errors::ARG));
    }

    #[test]
    fn set_parameter_bad_alpha_select_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.output_alpha_select = 99;
        assert_eq!(d.set_parameter(h, p).err(), Some(errors::ARG));
    }

    #[test]
    fn set_parameter_happy_path_8bit() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        let out = d.set_parameter(h, ok_in_param()).unwrap();
        assert_eq!(out.output_width, 512);
        assert_eq!(out.output_components, 4);
        assert_eq!(out.output_width_byte, 512 * 4);
        assert_eq!(out.use_memory_space, 512 * 4 * 384);
    }

    #[test]
    fn set_parameter_16bit_doubles_width_byte() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.output_bit_depth = 16;
        let out = d.set_parameter(h, p).unwrap();
        assert_eq!(out.output_width_byte, 512 * 4 * 2);
    }

    #[test]
    fn decode_data_before_setparam_rejected() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        assert_eq!(d.decode_data(h).err(), Some(errors::SEQ));
    }

    #[test]
    fn decode_data_happy_path() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        d.set_parameter(h, ok_in_param()).unwrap();
        let out = d.decode_data(h).unwrap();
        assert_eq!(out.output_lines, 384);
        assert_eq!(out.status, DEC_STATUS_FINISH);
    }

    #[test]
    fn grayscale_1bit_accepted() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let info = Info {
            image_width: 64,
            image_height: 64,
            num_components: 1,
            color_space: CS_GRAYSCALE,
            bit_depth: 1,
            interlace_method: NO_INTERLACE,
            chunk_information: CHUNK_IHDR | CHUNK_IDAT | CHUNK_IEND,
        };
        d.read_header(h, MAGIC_PNG, info).unwrap();
    }

    #[test]
    fn adam7_interlace_accepted() {
        let mut d = PngDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.interlace_method = ADAM7_INTERLACE;
        d.read_header(h, MAGIC_PNG, info).unwrap();
    }

    #[test]
    fn full_png_lifecycle_smoke() {
        let mut d = PngDec::new();
        d.create(2 * 1024 * 1024).unwrap();
        let h = d.open(file_src()).unwrap();
        let info = d.read_header(h, MAGIC_PNG, ok_info()).unwrap();
        assert_eq!(info.color_space, CS_RGBA);
        let out = d.set_parameter(h, ok_in_param()).unwrap();
        assert_eq!(out.output_height, 384);
        let data = d.decode_data(h).unwrap();
        assert_eq!(data.status, DEC_STATUS_FINISH);
        d.close(h).unwrap();
        assert_eq!(d.handle_count(), 0);
        d.destroy().unwrap();
    }

    #[test]
    fn parse_header_extracts_ihdr() {
        // 8-byte signature + IHDR(320x240, depth 8, RGB color_type 2).
        let mut png = Vec::new();
        png.extend_from_slice(MAGIC_PNG);
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&320u32.to_be_bytes());
        png.extend_from_slice(&240u32.to_be_bytes());
        png.extend_from_slice(&[8, 2, 0, 0, 0]); // depth, color_type=RGB, comp, filter, interlace
        let info = PngDec::parse_header(&png).unwrap();
        assert_eq!(info.image_width, 320);
        assert_eq!(info.image_height, 240);
        assert_eq!(info.num_components, 3);
        assert_eq!(info.color_space, CS_RGB);
        assert_eq!(info.bit_depth, 8);
    }

    #[test]
    fn parse_header_rejects_non_png() {
        assert_eq!(PngDec::parse_header(&[0u8; 40]), Err(errors::HEADER));
    }
}
