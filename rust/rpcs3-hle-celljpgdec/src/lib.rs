//! `rpcs3-hle-celljpgdec` — JPEG decoder HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellJpgDec.cpp`. The PS3 ships a
//! baseline-JPEG decoder with fixed-point IDCT; games call through
//! `Create → Open → ReadHeader → SetParameter → Decode → Close`.
//!
//! ## Entry points covered
//!
//! | HLE function                    | Rust wrapper                        |
//! |---------------------------------|-------------------------------------|
//! | `cellJpgDecCreate`              | [`JpgDec::create`]                  |
//! | `cellJpgDecDestroy`             | [`JpgDec::destroy`]                 |
//! | `cellJpgDecOpen`                | [`JpgDec::open`]                    |
//! | `cellJpgDecClose`               | [`JpgDec::close`]                   |
//! | `cellJpgDecReadHeader`          | [`JpgDec::read_header`]             |
//! | `cellJpgDecSetParameter`        | [`JpgDec::set_parameter`]           |
//! | `cellJpgDecDecodeData`          | [`JpgDec::decode_data`]             |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellJpgDec.h:3-15
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const HEADER: CellError = CellError(0x8061_1101);
    pub const STREAM_FORMAT: CellError = CellError(0x8061_1102);
    pub const ARG: CellError = CellError(0x8061_1103);
    pub const SEQ: CellError = CellError(0x8061_1104);
    pub const BUSY: CellError = CellError(0x8061_1105);
    pub const FATAL: CellError = CellError(0x8061_1106);
    pub const OPEN_FILE: CellError = CellError(0x8061_1107);
    pub const SPU_UNSUPPORT: CellError = CellError(0x8061_1108);
    pub const CB_PARAM: CellError = CellError(0x8061_1109);
}

// =====================================================================
// Color space / output mode / status / source (cellJpgDec.h:17-46)
// =====================================================================

pub const CS_UNKNOWN: u32 = 0;
pub const CS_GRAYSCALE: u32 = 1;
pub const CS_RGB: u32 = 2;
pub const CS_YCBCR: u32 = 3;
pub const CS_RGBA: u32 = 10;
pub const CS_UPSAMPLE_ONLY: u32 = 11;
pub const CS_ARGB: u32 = 20;
pub const CS_GRAYSCALE_TO_ALPHA_RGBA: u32 = 40;
pub const CS_GRAYSCALE_TO_ALPHA_ARGB: u32 = 41;

#[must_use]
pub fn is_known_color_space(cs: u32) -> bool {
    matches!(
        cs,
        CS_UNKNOWN
            | CS_GRAYSCALE
            | CS_RGB
            | CS_YCBCR
            | CS_RGBA
            | CS_UPSAMPLE_ONLY
            | CS_ARGB
            | CS_GRAYSCALE_TO_ALPHA_RGBA
            | CS_GRAYSCALE_TO_ALPHA_ARGB
    )
}

/// Number of bytes per output pixel for a given output color space.
#[must_use]
pub fn bytes_per_pixel(cs: u32) -> Option<u32> {
    Some(match cs {
        CS_GRAYSCALE => 1,
        CS_RGB | CS_YCBCR => 3,
        CS_RGBA | CS_ARGB | CS_GRAYSCALE_TO_ALPHA_RGBA | CS_GRAYSCALE_TO_ALPHA_ARGB => 4,
        _ => return None,
    })
}

pub const SRC_FILE: u32 = 0;
pub const SRC_BUFFER: u32 = 1;

#[must_use]
pub fn is_known_source(s: u32) -> bool {
    matches!(s, SRC_FILE | SRC_BUFFER)
}

pub const DEC_STATUS_FINISH: u32 = 0;
pub const DEC_STATUS_STOP: u32 = 1;

pub const OUT_TOP_TO_BOTTOM: u32 = 0;
pub const OUT_BOTTOM_TO_TOP: u32 = 1;

#[must_use]
pub fn is_known_output_mode(m: u32) -> bool {
    matches!(m, OUT_TOP_TO_BOTTOM | OUT_BOTTOM_TO_TOP)
}

// =====================================================================
// JPEG magic bytes + limits
// =====================================================================

/// JPEG files start with `FF D8` (SOI — start of image).
pub const MAGIC_SOI: &[u8; 2] = &[0xFF, 0xD8];

/// Max dimensions supported by the real decoder (per SDK docs: 8k × 8k).
pub const MAX_WIDTH: u32 = 8192;
pub const MAX_HEIGHT: u32 = 8192;

/// Maximum simultaneous handles — C++ `CellJpgDecSubHandle::id_count`.
pub const MAX_HANDLES: u32 = 1023;

/// `CellJpgDecSubHandle::id_base`.
pub const HANDLE_ID_BASE: u32 = 1;

// =====================================================================
// Structs
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Info {
    pub image_width: u32,
    pub image_height: u32,
    pub num_components: u32,
    pub color_space: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Src {
    pub src_select: u32,
    pub file_name: String,
    pub file_offset: u64,
    pub file_size: u32,
    pub stream_ptr: u32,
    pub stream_size: u32,
    pub spu_thread_enable: u32,
}

impl Src {
    fn validate(&self) -> Result<(), CellError> {
        if !is_known_source(self.src_select) {
            return Err(errors::ARG);
        }
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
                if self.stream_size == 0 || self.stream_ptr == 0 {
                    return Err(errors::ARG);
                }
            }
            _ => return Err(errors::ARG),
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InParam {
    pub command_ptr: u32,
    pub down_scale: u32,
    pub method: u32,
    pub output_mode: u32,
    pub output_color_space: u32,
    pub output_color_alpha: u8,
}

impl InParam {
    fn validate(&self) -> Result<(), CellError> {
        if ![1, 2, 4, 8].contains(&self.down_scale) {
            return Err(errors::ARG);
        }
        if !is_known_output_mode(self.output_mode) {
            return Err(errors::ARG);
        }
        if !is_known_color_space(self.output_color_space) {
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
    pub output_mode: u32,
    pub output_color_space: u32,
    pub down_scale: u32,
    pub use_memory_space: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DataOutInfo {
    pub mean: f32,
    pub output_lines: u32,
    pub status: u32,
}

// =====================================================================
// Handle + decoder state
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
enum HandleState {
    Opened,
    HeaderRead,
    Configured,
    Decoding,
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
pub struct JpgDec {
    mem_space_allocated: u32,
    handles: Vec<Handle>,
    next_id: u32,
}

impl JpgDec {
    #[must_use]
    pub fn new() -> Self {
        Self { next_id: HANDLE_ID_BASE, ..Default::default() }
    }

    /// `cellJpgDecCreate(mainHandle, threadParam, extParam)`. We track
    /// only the init-space accounting that games query via `OpnInfo`.
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

    /// `cellJpgDecOpen(mainHandle, subHandle, src, openInfo)`. Allocates
    /// a per-stream handle.
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

    /// `cellJpgDecReadHeader(sub, info)`. Tests supply a byte slice that
    /// must start with FF D8 (SOI). Real lib parses the APPn/DHT/SOF0
    /// chunks — we inject the parsed Info directly.
    pub fn read_header(&mut self, id: u32, header_bytes: &[u8], info: Info) -> Result<Info, CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::Opened {
            return Err(errors::SEQ);
        }
        if header_bytes.len() < 2 || &header_bytes[..2] != MAGIC_SOI {
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
        if info.num_components == 0 || info.num_components > 4 {
            return Err(errors::STREAM_FORMAT);
        }
        self.handles[idx].info = Some(info);
        self.handles[idx].state = HandleState::HeaderRead;
        Ok(info)
    }

    /// Byte-exact port of RPCS3 `cellJpgDecReadHeader`'s manual header parse
    /// (cellJpgDec.cpp:146-178): validate the SOI+APP0 (`FF D8 FF E0`) and the
    /// "JFIF" tag, walk the segment chain to the `FF C0` SOF0 marker, then read
    /// width/height from it. NOTE the faithful quirks: the segment length uses
    /// `buffer[i]*0xFF + buffer[i+1]` (RPCS3's `*0xFF`, not `*0x100`), while the
    /// SOF0 width/height use `*0x100`; numComponents is hardcoded 3 and
    /// colorSpace `CELL_JPG_RGB`. Out-of-range reads return `HEADER` (RPCS3 would
    /// over-read; same result for valid input).
    pub fn parse_header(bytes: &[u8]) -> Result<Info, CellError> {
        let n = bytes.len();
        if n < 10
            || bytes[0] != 0xFF
            || bytes[1] != 0xD8
            || bytes[2] != 0xFF
            || bytes[3] != 0xE0
            || &bytes[6..10] != b"JFIF"
        {
            return Err(errors::HEADER);
        }
        let mut i: usize = 4;
        let mut block_length = bytes[i] as usize * 0xFF + bytes[i + 1] as usize;
        loop {
            i = i.wrapping_add(block_length);
            if i + 1 >= n || bytes[i] != 0xFF {
                return Err(errors::HEADER);
            }
            if bytes[i + 1] == 0xC0 {
                break; // SOF0 — start of frame
            }
            i += 2;
            if i + 1 >= n {
                return Err(errors::HEADER);
            }
            block_length = bytes[i] as usize * 0xFF + bytes[i + 1] as usize;
        }
        if i + 8 >= n {
            return Err(errors::HEADER);
        }
        Ok(Info {
            image_width: u32::from(bytes[i + 7]) * 0x100 + u32::from(bytes[i + 8]),
            image_height: u32::from(bytes[i + 5]) * 0x100 + u32::from(bytes[i + 6]),
            num_components: 3,
            color_space: CS_RGB,
        })
    }

    /// The (stream_ptr, stream_size) of an opened BUFFER-source handle — lets the
    /// emu-core ReadHeader arm fetch the JPEG bytes from guest memory.
    #[must_use]
    pub fn stream_window(&self, id: u32) -> Option<(u32, u32)> {
        let h = self.handles.iter().find(|h| h.id == id)?;
        Some((h.src.stream_ptr, h.src.stream_size))
    }

    /// The `OutParam` set by `set_parameter` for a handle — the emu-core
    /// DecodeData arm reads the output color space / mode from it.
    #[must_use]
    pub fn out_param_for(&self, id: u32) -> Option<OutParam> {
        self.handles.iter().find(|h| h.id == id)?.out_param
    }

    /// `cellJpgDecSetParameter(sub, inParam, outParam)`.
    pub fn set_parameter(&mut self, id: u32, in_param: InParam) -> Result<OutParam, CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::HeaderRead {
            return Err(errors::SEQ);
        }
        in_param.validate()?;
        let info = self.handles[idx].info.ok_or(errors::SEQ)?;
        let bpp = bytes_per_pixel(in_param.output_color_space).ok_or(errors::ARG)?;
        let output_width = info.image_width / in_param.down_scale;
        let output_height = info.image_height / in_param.down_scale;
        if output_width == 0 || output_height == 0 {
            return Err(errors::ARG);
        }
        let out = OutParam {
            output_width_byte: u64::from(output_width) * u64::from(bpp),
            output_width,
            output_height,
            output_components: bpp,
            output_mode: in_param.output_mode,
            output_color_space: in_param.output_color_space,
            down_scale: in_param.down_scale,
            use_memory_space: u64::from(output_width) as u32 * u64::from(bpp) as u32 * output_height,
        };
        self.handles[idx].in_param = Some(in_param);
        self.handles[idx].out_param = Some(out);
        self.handles[idx].state = HandleState::Configured;
        Ok(out)
    }

    /// `cellJpgDecDecodeData(sub, dataOut, dataCtrlParam, dataOutInfo)`.
    /// Returns the output info (mean, output_lines, status).
    pub fn decode_data(&mut self, id: u32) -> Result<DataOutInfo, CellError> {
        let idx = self.handle_idx(id)?;
        if self.handles[idx].state != HandleState::Configured {
            return Err(errors::SEQ);
        }
        self.handles[idx].state = HandleState::Decoding;
        let out = self.handles[idx].out_param.ok_or(errors::SEQ)?;
        // After decode, state transitions back to Configured for re-decode.
        self.handles[idx].state = HandleState::Configured;
        Ok(DataOutInfo { mean: 0.0, output_lines: out.output_height, status: DEC_STATUS_FINISH })
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
            stream_size: 4096,
            spu_thread_enable: 0,
        }
    }

    fn file_src() -> Src {
        Src {
            src_select: SRC_FILE,
            file_name: "/dev_hdd0/photo/test.jpg".into(),
            file_offset: 0,
            file_size: 4096,
            stream_ptr: 0,
            stream_size: 0,
            spu_thread_enable: 0,
        }
    }

    fn ok_info() -> Info {
        Info { image_width: 1024, image_height: 768, num_components: 3, color_space: CS_YCBCR }
    }

    fn ok_in_param() -> InParam {
        InParam {
            command_ptr: 0,
            down_scale: 1,
            method: 0,
            output_mode: OUT_TOP_TO_BOTTOM,
            output_color_space: CS_RGBA,
            output_color_alpha: 0xFF,
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::HEADER.0, 0x8061_1101);
        assert_eq!(errors::STREAM_FORMAT.0, 0x8061_1102);
        assert_eq!(errors::ARG.0, 0x8061_1103);
        assert_eq!(errors::SEQ.0, 0x8061_1104);
        assert_eq!(errors::BUSY.0, 0x8061_1105);
        assert_eq!(errors::FATAL.0, 0x8061_1106);
        assert_eq!(errors::OPEN_FILE.0, 0x8061_1107);
        assert_eq!(errors::SPU_UNSUPPORT.0, 0x8061_1108);
        assert_eq!(errors::CB_PARAM.0, 0x8061_1109);
    }

    #[test]
    fn color_space_constants_stable() {
        assert_eq!(CS_UNKNOWN, 0);
        assert_eq!(CS_GRAYSCALE, 1);
        assert_eq!(CS_RGB, 2);
        assert_eq!(CS_YCBCR, 3);
        assert_eq!(CS_RGBA, 10);
        assert_eq!(CS_UPSAMPLE_ONLY, 11);
        assert_eq!(CS_ARGB, 20);
        assert_eq!(CS_GRAYSCALE_TO_ALPHA_RGBA, 40);
        assert_eq!(CS_GRAYSCALE_TO_ALPHA_ARGB, 41);
    }

    #[test]
    fn src_and_mode_constants_stable() {
        assert_eq!(SRC_FILE, 0);
        assert_eq!(SRC_BUFFER, 1);
        assert_eq!(OUT_TOP_TO_BOTTOM, 0);
        assert_eq!(OUT_BOTTOM_TO_TOP, 1);
        assert_eq!(DEC_STATUS_FINISH, 0);
        assert_eq!(DEC_STATUS_STOP, 1);
    }

    #[test]
    fn handle_constants_stable() {
        assert_eq!(HANDLE_ID_BASE, 1);
        assert_eq!(MAX_HANDLES, 1023);
    }

    #[test]
    fn jpg_limits_stable() {
        assert_eq!(MAX_WIDTH, 8192);
        assert_eq!(MAX_HEIGHT, 8192);
        assert_eq!(MAGIC_SOI, &[0xFF, 0xD8]);
    }

    #[test]
    fn bytes_per_pixel_maps() {
        assert_eq!(bytes_per_pixel(CS_GRAYSCALE), Some(1));
        assert_eq!(bytes_per_pixel(CS_RGB), Some(3));
        assert_eq!(bytes_per_pixel(CS_YCBCR), Some(3));
        assert_eq!(bytes_per_pixel(CS_RGBA), Some(4));
        assert_eq!(bytes_per_pixel(CS_ARGB), Some(4));
        assert_eq!(bytes_per_pixel(CS_GRAYSCALE_TO_ALPHA_RGBA), Some(4));
        assert_eq!(bytes_per_pixel(99), None);
    }

    #[test]
    fn create_happy_path() {
        let mut d = JpgDec::new();
        d.create(4 * 1024 * 1024).unwrap();
        assert_eq!(d.init_space_allocated(), 4 * 1024 * 1024);
    }

    #[test]
    fn create_zero_space_rejected() {
        let mut d = JpgDec::new();
        assert_eq!(d.create(0), Err(errors::ARG));
    }

    #[test]
    fn create_twice_is_busy() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        assert_eq!(d.create(1024), Err(errors::BUSY));
    }

    #[test]
    fn destroy_without_create_is_seq() {
        let mut d = JpgDec::new();
        assert_eq!(d.destroy(), Err(errors::SEQ));
    }

    #[test]
    fn destroy_with_open_handles_is_busy() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        d.open(buffer_src()).unwrap();
        assert_eq!(d.destroy(), Err(errors::BUSY));
    }

    #[test]
    fn open_before_create_is_seq() {
        let mut d = JpgDec::new();
        assert_eq!(d.open(buffer_src()), Err(errors::SEQ));
    }

    #[test]
    fn open_bad_src_select_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let mut s = buffer_src();
        s.src_select = 99;
        assert_eq!(d.open(s), Err(errors::ARG));
    }

    #[test]
    fn open_file_empty_name_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let mut s = file_src();
        s.file_name.clear();
        assert_eq!(d.open(s), Err(errors::OPEN_FILE));
    }

    #[test]
    fn open_buffer_null_ptr_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let mut s = buffer_src();
        s.stream_ptr = 0;
        assert_eq!(d.open(s), Err(errors::ARG));
    }

    #[test]
    fn open_buffer_zero_size_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let mut s = buffer_src();
        s.stream_size = 0;
        assert_eq!(d.open(s), Err(errors::ARG));
    }

    #[test]
    fn open_allocates_incrementing_ids() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let a = d.open(buffer_src()).unwrap();
        let b = d.open(buffer_src()).unwrap();
        assert_eq!(a, HANDLE_ID_BASE);
        assert_eq!(b, a + 1);
    }

    #[test]
    fn close_bad_id_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        assert_eq!(d.close(999), Err(errors::ARG));
    }

    #[test]
    fn read_header_bad_magic_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        assert_eq!(d.read_header(h, &[0x12, 0x34], ok_info()), Err(errors::HEADER));
    }

    #[test]
    fn read_header_too_short_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        assert_eq!(d.read_header(h, &[0xFF], ok_info()), Err(errors::HEADER));
    }

    #[test]
    fn read_header_zero_dims_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.image_width = 0;
        assert_eq!(d.read_header(h, MAGIC_SOI, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_over_max_dims_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.image_width = MAX_WIDTH + 1;
        assert_eq!(d.read_header(h, MAGIC_SOI, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_unknown_color_space_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.color_space = 99;
        assert_eq!(d.read_header(h, MAGIC_SOI, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_bad_component_count_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let mut info = ok_info();
        info.num_components = 5;
        assert_eq!(d.read_header(h, MAGIC_SOI, info), Err(errors::STREAM_FORMAT));
    }

    #[test]
    fn read_header_happy_path() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        let info = d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        assert_eq!(info.image_width, 1024);
    }

    #[test]
    fn read_header_twice_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        assert_eq!(d.read_header(h, MAGIC_SOI, ok_info()), Err(errors::SEQ));
    }

    #[test]
    fn set_parameter_before_header_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        assert_eq!(d.set_parameter(h, ok_in_param()).err(), Some(errors::SEQ));
    }

    #[test]
    fn set_parameter_bad_downscale_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.down_scale = 3;
        assert_eq!(d.set_parameter(h, p).err(), Some(errors::ARG));
    }

    #[test]
    fn set_parameter_bad_output_mode_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.output_mode = 99;
        assert_eq!(d.set_parameter(h, p).err(), Some(errors::ARG));
    }

    #[test]
    fn set_parameter_unknown_color_space_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.output_color_space = 99;
        assert_eq!(d.set_parameter(h, p).err(), Some(errors::ARG));
    }

    #[test]
    fn set_parameter_downscale_applies_to_output() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.down_scale = 4;
        let out = d.set_parameter(h, p).unwrap();
        assert_eq!(out.output_width, 256);
        assert_eq!(out.output_height, 192);
        assert_eq!(out.output_components, 4);
        assert_eq!(out.output_width_byte, 256 * 4);
    }

    #[test]
    fn set_parameter_output_rgb_3bpp() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        let mut p = ok_in_param();
        p.output_color_space = CS_RGB;
        let out = d.set_parameter(h, p).unwrap();
        assert_eq!(out.output_components, 3);
    }

    #[test]
    fn decode_data_before_setparam_rejected() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        assert_eq!(d.decode_data(h).err(), Some(errors::SEQ));
    }

    #[test]
    fn decode_data_happy_path() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        d.set_parameter(h, ok_in_param()).unwrap();
        let out = d.decode_data(h).unwrap();
        assert_eq!(out.output_lines, 768);
        assert_eq!(out.status, DEC_STATUS_FINISH);
    }

    #[test]
    fn decode_data_can_repeat_after_success() {
        let mut d = JpgDec::new();
        d.create(1024).unwrap();
        let h = d.open(buffer_src()).unwrap();
        d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        d.set_parameter(h, ok_in_param()).unwrap();
        d.decode_data(h).unwrap();
        // Re-decoding same config is allowed — state transitions back.
        d.decode_data(h).unwrap();
    }

    #[test]
    fn full_jpg_lifecycle_smoke() {
        let mut d = JpgDec::new();
        d.create(2 * 1024 * 1024).unwrap();
        let h = d.open(file_src()).unwrap();
        let info = d.read_header(h, MAGIC_SOI, ok_info()).unwrap();
        assert_eq!(info.num_components, 3);
        let mut p = ok_in_param();
        p.output_color_space = CS_RGBA;
        p.down_scale = 2;
        let out = d.set_parameter(h, p).unwrap();
        assert_eq!(out.output_width, 512);
        let data = d.decode_data(h).unwrap();
        assert_eq!(data.status, DEC_STATUS_FINISH);
        d.close(h).unwrap();
        assert_eq!(d.handle_count(), 0);
        d.destroy().unwrap();
    }

    /// Minimal JFIF: SOI + APP0(len 16) + SOF0(h=240, w=320) — exercises the
    /// byte-exact `parse_header` (segment walk + SOF0 extraction).
    const MINI_JPG: &[u8] = &[
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
        0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, // APP0 (16-byte segment)
        0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0xF0, 0x01, 0x40, 0x03, // SOF0 h=240 w=320
        0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF, 0xD9,
    ];

    #[test]
    fn parse_header_extracts_sof0_dimensions() {
        let info = JpgDec::parse_header(MINI_JPG).unwrap();
        assert_eq!(info.image_width, 320);
        assert_eq!(info.image_height, 240);
        assert_eq!(info.num_components, 3);
        assert_eq!(info.color_space, CS_RGB);
    }

    #[test]
    fn parse_header_rejects_non_jfif() {
        assert_eq!(JpgDec::parse_header(&[0xFF, 0xD8, 0x00]), Err(errors::HEADER));
        assert_eq!(JpgDec::parse_header(MAGIC_SOI), Err(errors::HEADER));
    }
}
