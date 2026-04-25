//! `rpcs3-hle-cellgifdec` — GIF decoder HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellGifDec.cpp` + `cellGifDec.h` (924
//! lines total).  Covers the full GIF89a/GIF87a Logical Screen
//! Descriptor parser plus the Create → Open → ReadHeader → SetParameter
//! → DecodeData → Close → Destroy state machine.
//!
//! ## Entry points covered
//!
//! | HLE function                     | Rust wrapper                    |
//! |----------------------------------|---------------------------------|
//! | `cellGifDecCreate` / `ExtCreate` | [`GifDec::create`]              |
//! | `cellGifDecOpen` / `ExtOpen`     | [`GifDec::open`]                |
//! | `cellGifDecReadHeader`           | [`GifDec::read_header`]         |
//! | `cellGifDecSetParameter`         | [`GifDec::set_parameter`]       |
//! | `cellGifDecDecodeData`           | [`GifDec::decode`]              |
//! | `cellGifDecClose`                | [`GifDec::close`]               |
//! | `cellGifDecDestroy`              | [`GifDec::destroy`]             |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellGifDec.h:6-16
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const OPEN_FILE:     CellError = CellError(0x8061_1300);
    pub const STREAM_FORMAT: CellError = CellError(0x8061_1301);
    pub const SEQ:           CellError = CellError(0x8061_1302);
    pub const ARG:           CellError = CellError(0x8061_1303);
    pub const FATAL:         CellError = CellError(0x8061_1304);
    pub const SPU_UNSUPPORT: CellError = CellError(0x8061_1305);
    pub const SPU_ERROR:     CellError = CellError(0x8061_1306);
    pub const CB_PARAM:      CellError = CellError(0x8061_1307);
}

// =====================================================================
// Enum ordinals — byte-exact with cellGifDec.h
// =====================================================================

pub const SRC_FILE:   i32 = 0;
pub const SRC_BUFFER: i32 = 1;

pub const SPU_THREAD_DISABLE: i32 = 0;
pub const SPU_THREAD_ENABLE:  i32 = 1;

pub const RECORD_TYPE_IMAGE_DESC: i32 = 1;
pub const RECORD_TYPE_EXTENSION:  i32 = 2;
pub const RECORD_TYPE_TERMINATE:  i32 = 3;

pub const COLORSPACE_RGBA: i32 = 10;
pub const COLORSPACE_ARGB: i32 = 20;

pub const COMMAND_CONTINUE: i32 = 0;
pub const COMMAND_STOP:     i32 = 1;

pub const DEC_STATUS_FINISH: i32 = 0;
pub const DEC_STATUS_STOP:   i32 = 1;

pub const BUFFER_MODE_LINE: i32 = 1;

pub const SPU_MODE_RECEIVE_EVENT:     i32 = 0;
pub const SPU_MODE_TRYRECEIVE_EVENT:  i32 = 1;

pub const INTERLACE_NO: i32 = 0;
pub const INTERLACE_YES: i32 = 1;

// ---- GIF signature constants ----

/// "GIF8" big-endian at bytes 0..4 of every valid GIF file.
pub const GIF_MAGIC_GIF8_BE: u32 = 0x4749_4638;

/// "9a" little-endian at bytes 4..6 of GIF89a.
pub const GIF_TRAILER_89A_LE: u16 = 0x6139; // '9' = 0x39, 'a' = 0x61

/// "7a" little-endian at bytes 4..6 of GIF87a.
pub const GIF_TRAILER_87A_LE: u16 = 0x6137;

/// Logical Screen Descriptor is 6 bytes signature + 7 bytes screen data.
pub const GIF_HEADER_SIZE: usize = 13;

/// Max outstanding sub-streams one decoder can own.  C++ manager maps 1:1
/// with `GifStream` heap allocations; we cap to keep tests deterministic.
pub const MAX_SUBSTREAMS: usize = 1023;

/// Base id for sub-stream allocation (matching `idm::make_ptr` default).
pub const SUBSTREAM_ID_BASE: u32 = 1;

// =====================================================================
// Public structs — byte-exact mirrors of cellGifDec.h
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GifInfo {
    pub s_width: u32,
    pub s_height: u32,
    pub s_global_color_table_flag: u32,
    pub s_color_resolution: u32,
    pub s_sort_flag: u32,
    pub s_size_of_global_color_table: u32,
    pub s_background_color: u32,
    pub s_pixel_aspect_ratio: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GifOutParam {
    pub output_width_byte: u64,
    pub output_width: u32,
    pub output_height: u32,
    pub output_components: u32,
    pub output_bit_depth: u32,
    pub output_color_space: i32,
    pub use_memory_space: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GifSrc {
    File { name: alloc::string::String, offset: i64, size: u32 },
    Buffer { addr: u32, size: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GifInParam {
    pub color_space: i32,
    pub output_color_alpha1: u8,
    pub output_color_alpha2: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GifDataOutInfo {
    pub record_type: i32,
    pub extension_label: u8,
    pub status: i32,
}

// =====================================================================
// Handle-state FSM
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamState {
    Opened,
    HeaderRead,
    Configured,
    Decoded,
}

#[derive(Debug, Clone)]
pub struct GifStream {
    pub id: u32,
    pub src: GifSrc,
    pub info: GifInfo,
    pub out_param: GifOutParam,
    pub state: StreamState,
}

// =====================================================================
// Decoder manager
// =====================================================================

#[derive(Debug, Default, Clone)]
pub struct GifDec {
    created: bool,
    streams: Vec<GifStream>,
    next_id: u32,
}

impl GifDec {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    #[must_use]
    pub fn is_created(&self) -> bool { self.created }

    /// Port of `cellGifDecCreate` / `cellGifDecExtCreate`.
    ///
    /// # Errors
    /// * [`errors::ARG`] if `main_handle_valid` is false.
    /// * [`errors::SEQ`] if already created (C++ doesn't check but the
    ///   mirror stays strict so higher layers trip on double create).
    pub fn create(&mut self, main_handle_valid: bool) -> Result<(), CellError> {
        if !main_handle_valid {
            return Err(errors::ARG);
        }
        if self.created {
            return Err(errors::SEQ);
        }
        self.created = true;
        self.next_id = SUBSTREAM_ID_BASE;
        Ok(())
    }

    /// Port of `cellGifDecDestroy`.
    ///
    /// # Errors
    /// * [`errors::ARG`] if `main_handle_valid` is false.
    /// * [`errors::SEQ`] if not created.
    pub fn destroy(&mut self, main_handle_valid: bool) -> Result<(), CellError> {
        if !main_handle_valid {
            return Err(errors::ARG);
        }
        if !self.created {
            return Err(errors::SEQ);
        }
        self.streams.clear();
        self.created = false;
        Ok(())
    }

    /// Port of `cellGifDecOpen`.  The C++ implementation allocates a
    /// `GifStream` via `idm::make_ptr` and stores the decoded size +
    /// source descriptor in it; we allocate a handle id and return it.
    ///
    /// # Errors
    /// * [`errors::ARG`]   if any required pointer is null / `src` is invalid.
    /// * [`errors::SEQ`]   if the decoder wasn't created.
    /// * [`errors::FATAL`] if `MAX_SUBSTREAMS` is exhausted.
    /// * [`errors::OPEN_FILE`] if a file-source carries an empty name.
    pub fn open(
        &mut self,
        main_handle_valid: bool,
        sub_handle_out_valid: bool,
        src: GifSrc,
    ) -> Result<u32, CellError> {
        if !main_handle_valid {
            return Err(errors::ARG);
        }
        if !self.created {
            return Err(errors::SEQ);
        }
        if !sub_handle_out_valid {
            return Err(errors::ARG);
        }
        match &src {
            GifSrc::File { name, size, .. } => {
                if name.is_empty() { return Err(errors::OPEN_FILE); }
                if *size == 0 { return Err(errors::ARG); }
            }
            GifSrc::Buffer { addr, size } => {
                if *addr == 0 || *size == 0 { return Err(errors::ARG); }
            }
        }
        if self.streams.len() >= MAX_SUBSTREAMS {
            return Err(errors::FATAL);
        }
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).ok_or(errors::FATAL)?;
        self.streams.push(GifStream {
            id,
            src,
            info: GifInfo::default(),
            out_param: GifOutParam::default(),
            state: StreamState::Opened,
        });
        Ok(id)
    }

    /// Parse a 13-byte Logical Screen Descriptor and populate the
    /// stream's [`GifInfo`].  Port of `cellGifDecReadHeader`.
    ///
    /// # Errors
    /// * [`errors::ARG`]   for null handles / info pointer.
    /// * [`errors::SEQ`]   if the decoder wasn't created or the sub-stream
    ///                     is not in the [`StreamState::Opened`] state.
    /// * [`errors::STREAM_FORMAT`] if `header.len() < 13`, or the first
    ///                     4 bytes are not `GIF8`, or the trailer is not
    ///                     `87a` / `89a`.
    pub fn read_header(
        &mut self,
        main_handle_valid: bool,
        sub_handle: u32,
        info_out_valid: bool,
        header: &[u8],
    ) -> Result<GifInfo, CellError> {
        if !main_handle_valid { return Err(errors::ARG); }
        if !self.created { return Err(errors::SEQ); }
        // C++ returns ARG for sub_handle==0 (null ptr).  Higher-layer
        // mirror uses the range check + lookup.
        let stream = self.streams.iter_mut().find(|s| s.id == sub_handle)
            .ok_or(errors::ARG)?;
        if stream.state != StreamState::Opened { return Err(errors::SEQ); }
        if !info_out_valid { return Err(errors::ARG); }

        let info = parse_gif_header(header)?;
        stream.info = info;
        stream.state = StreamState::HeaderRead;
        Ok(info)
    }

    /// Port of `cellGifDecSetParameter`.  Computes the canonical
    /// `outputWidthByte = (SWidth * SColorResolution * 3) / 8` and sets
    /// `outputComponents = 4` (only RGBA / ARGB are accepted).
    ///
    /// # Errors
    /// * [`errors::ARG`] for bad handles / null pointers / unknown color
    ///                  space.
    /// * [`errors::SEQ`] if the header wasn't read yet.
    pub fn set_parameter(
        &mut self,
        main_handle_valid: bool,
        sub_handle: u32,
        out_param_valid: bool,
        in_param: GifInParam,
    ) -> Result<GifOutParam, CellError> {
        if !main_handle_valid { return Err(errors::ARG); }
        if !self.created { return Err(errors::SEQ); }
        let stream = self.streams.iter_mut().find(|s| s.id == sub_handle)
            .ok_or(errors::ARG)?;
        if !out_param_valid { return Err(errors::ARG); }
        if stream.state == StreamState::Opened { return Err(errors::SEQ); }

        let components = match in_param.color_space {
            COLORSPACE_RGBA | COLORSPACE_ARGB => 4,
            _ => return Err(errors::ARG),
        };
        let out = GifOutParam {
            output_width_byte: (u64::from(stream.info.s_width)
                * u64::from(stream.info.s_color_resolution) * 3) / 8,
            output_width:  stream.info.s_width,
            output_height: stream.info.s_height,
            output_components: components,
            output_bit_depth: 0,
            output_color_space: in_param.color_space,
            use_memory_space: 0,
        };
        stream.out_param = out;
        stream.state = StreamState::Configured;
        Ok(out)
    }

    /// Port of `cellGifDecDecodeData`.  The C++ routine runs LZW
    /// decompression; in the port we validate the lifecycle and return
    /// the `DataOutInfo` the firmware would emit after a successful pass.
    ///
    /// # Errors
    /// * [`errors::ARG`] / [`errors::SEQ`] mirror `set_parameter`.
    /// * [`errors::CB_PARAM`] if `command == STOP` but `dataOutInfo` was
    ///                         null.
    pub fn decode(
        &mut self,
        main_handle_valid: bool,
        sub_handle: u32,
        data_out_info_valid: bool,
        command: i32,
    ) -> Result<GifDataOutInfo, CellError> {
        if !main_handle_valid { return Err(errors::ARG); }
        if !self.created { return Err(errors::SEQ); }
        let stream = self.streams.iter_mut().find(|s| s.id == sub_handle)
            .ok_or(errors::ARG)?;
        if !data_out_info_valid { return Err(errors::ARG); }
        if stream.state != StreamState::Configured { return Err(errors::SEQ); }

        let status = match command {
            COMMAND_CONTINUE => DEC_STATUS_FINISH,
            COMMAND_STOP     => DEC_STATUS_STOP,
            _ => return Err(errors::CB_PARAM),
        };
        stream.state = StreamState::Decoded;
        Ok(GifDataOutInfo {
            record_type: RECORD_TYPE_IMAGE_DESC,
            extension_label: 0,
            status,
        })
    }

    /// Port of `cellGifDecClose`.  Removes the sub-stream from the table.
    ///
    /// # Errors
    /// * [`errors::ARG`] for unknown handles.
    /// * [`errors::SEQ`] if the decoder isn't created.
    pub fn close(&mut self, main_handle_valid: bool, sub_handle: u32) -> Result<(), CellError> {
        if !main_handle_valid { return Err(errors::ARG); }
        if !self.created { return Err(errors::SEQ); }
        let pos = self.streams.iter().position(|s| s.id == sub_handle)
            .ok_or(errors::ARG)?;
        self.streams.swap_remove(pos);
        Ok(())
    }

    #[must_use]
    pub fn stream_state(&self, id: u32) -> Option<StreamState> {
        self.streams.iter().find(|s| s.id == id).map(|s| s.state)
    }

    #[must_use]
    pub fn stream_info(&self, id: u32) -> Option<GifInfo> {
        self.streams.iter().find(|s| s.id == id).map(|s| s.info)
    }

    #[must_use]
    pub fn stream_out_param(&self, id: u32) -> Option<GifOutParam> {
        self.streams.iter().find(|s| s.id == id).map(|s| s.out_param)
    }

    #[must_use]
    pub fn stream_count(&self) -> usize { self.streams.len() }
}

// =====================================================================
// Header parser — byte-exact port of the 13-byte LSD block
// =====================================================================

/// Parse a GIF Logical Screen Descriptor from `buffer`.  Mirror of
/// cellGifDec.cpp:292-307.
///
/// # Errors
/// [`errors::STREAM_FORMAT`] if the buffer is too short, the magic is
/// wrong, or the version trailer isn't `87a`/`89a`.
pub fn parse_gif_header(buffer: &[u8]) -> Result<GifInfo, CellError> {
    if buffer.len() < GIF_HEADER_SIZE {
        return Err(errors::STREAM_FORMAT);
    }
    let magic = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
    if magic != GIF_MAGIC_GIF8_BE {
        return Err(errors::STREAM_FORMAT);
    }
    let trailer = u16::from_le_bytes([buffer[4], buffer[5]]);
    if trailer != GIF_TRAILER_89A_LE && trailer != GIF_TRAILER_87A_LE {
        return Err(errors::STREAM_FORMAT);
    }
    let packed = buffer[10];
    Ok(GifInfo {
        s_width:  u32::from(buffer[6]) + u32::from(buffer[7]) * 0x100,
        s_height: u32::from(buffer[8]) + u32::from(buffer[9]) * 0x100,
        s_global_color_table_flag:   u32::from(packed >> 7),
        s_color_resolution:          u32::from((packed >> 4) & 7) + 1,
        s_sort_flag:                 u32::from((packed >> 3) & 1),
        s_size_of_global_color_table: u32::from(packed & 7) + 1,
        s_background_color:   u32::from(buffer[11]),
        s_pixel_aspect_ratio: u32::from(buffer[12]),
    })
}

extern crate alloc;

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn gif89a_100x50() -> [u8; 13] {
        // magic=0x47494638 ('GIF8'), trailer=0x6139 ('9a')
        // width = 100 (0x64 0x00), height = 50 (0x32 0x00)
        // packed = 0b1_111_0_010 = 0xF2
        //   GCT flag = 1, color resolution = 7+1=8, sort flag = 0, GCT size = 2+1=3
        [
            b'G', b'I', b'F', b'8',      // 0x47494638
            b'9', b'a',                  // little-endian 0x6139
            0x64, 0x00,                  // width LE = 100
            0x32, 0x00,                  // height LE = 50
            0xF2,                        // packed
            0x00,                        // background color
            0x00,                        // pixel aspect ratio
        ]
    }
    fn gif87a_16x16_minimal() -> [u8; 13] {
        [
            b'G', b'I', b'F', b'8',
            b'7', b'a',                  // 0x6137
            0x10, 0x00, 0x10, 0x00,
            0x00, 0x00, 0x00,
        ]
    }

    // ---- constants ---------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::OPEN_FILE.0,     0x8061_1300);
        assert_eq!(errors::STREAM_FORMAT.0, 0x8061_1301);
        assert_eq!(errors::SEQ.0,           0x8061_1302);
        assert_eq!(errors::ARG.0,           0x8061_1303);
        assert_eq!(errors::FATAL.0,         0x8061_1304);
        assert_eq!(errors::SPU_UNSUPPORT.0, 0x8061_1305);
        assert_eq!(errors::SPU_ERROR.0,     0x8061_1306);
        assert_eq!(errors::CB_PARAM.0,      0x8061_1307);
    }

    #[test]
    fn enum_ordinals_match_cpp() {
        assert_eq!(SRC_FILE, 0);
        assert_eq!(SRC_BUFFER, 1);
        assert_eq!(COLORSPACE_RGBA, 10);
        assert_eq!(COLORSPACE_ARGB, 20);
        assert_eq!(RECORD_TYPE_IMAGE_DESC, 1);
        assert_eq!(RECORD_TYPE_TERMINATE, 3);
        assert_eq!(COMMAND_CONTINUE, 0);
        assert_eq!(COMMAND_STOP, 1);
        assert_eq!(DEC_STATUS_FINISH, 0);
        assert_eq!(DEC_STATUS_STOP, 1);
        assert_eq!(BUFFER_MODE_LINE, 1);
        assert_eq!(SPU_MODE_TRYRECEIVE_EVENT, 1);
        assert_eq!(INTERLACE_YES, 1);
    }

    #[test]
    fn magic_constants_byte_exact() {
        assert_eq!(GIF_MAGIC_GIF8_BE, 0x4749_4638);
        assert_eq!(GIF_TRAILER_89A_LE, 0x6139);
        assert_eq!(GIF_TRAILER_87A_LE, 0x6137);
        assert_eq!(GIF_HEADER_SIZE, 13);
    }

    // ---- parse_gif_header -------------------------------------------

    #[test]
    fn parse_header_89a_happy_path() {
        let buf = gif89a_100x50();
        let info = parse_gif_header(&buf).unwrap();
        assert_eq!(info.s_width, 100);
        assert_eq!(info.s_height, 50);
        assert_eq!(info.s_global_color_table_flag, 1);
        assert_eq!(info.s_color_resolution, 8); // (0xF2>>4)&7 + 1 = 7+1
        assert_eq!(info.s_sort_flag, 0);
        assert_eq!(info.s_size_of_global_color_table, 3); // (0xF2 & 7) + 1 = 2+1
        assert_eq!(info.s_background_color, 0);
        assert_eq!(info.s_pixel_aspect_ratio, 0);
    }

    #[test]
    fn parse_header_87a_happy_path() {
        let buf = gif87a_16x16_minimal();
        let info = parse_gif_header(&buf).unwrap();
        assert_eq!(info.s_width, 16);
        assert_eq!(info.s_height, 16);
        assert_eq!(info.s_global_color_table_flag, 0);
        // packed = 0x00 → color_resolution = 0 + 1 = 1
        assert_eq!(info.s_color_resolution, 1);
        assert_eq!(info.s_size_of_global_color_table, 1);
    }

    #[test]
    fn parse_header_too_short_is_stream_format() {
        assert_eq!(parse_gif_header(&[0; 12]).unwrap_err(), errors::STREAM_FORMAT);
        assert_eq!(parse_gif_header(&[]).unwrap_err(), errors::STREAM_FORMAT);
    }

    #[test]
    fn parse_header_bad_magic_is_stream_format() {
        let mut buf = gif89a_100x50();
        buf[0] = b'X';
        assert_eq!(parse_gif_header(&buf).unwrap_err(), errors::STREAM_FORMAT);
    }

    #[test]
    fn parse_header_bad_trailer_is_stream_format() {
        let mut buf = gif89a_100x50();
        buf[4] = b'8';
        buf[5] = b'a'; // "8a" → neither 87a nor 89a
        assert_eq!(parse_gif_header(&buf).unwrap_err(), errors::STREAM_FORMAT);
    }

    #[test]
    fn parse_header_width_little_endian_high_byte() {
        // width = 0x0102 = 258
        let mut buf = gif89a_100x50();
        buf[6] = 0x02;
        buf[7] = 0x01;
        let info = parse_gif_header(&buf).unwrap();
        assert_eq!(info.s_width, 258);
    }

    #[test]
    fn parse_header_packed_field_bits() {
        // packed = 0b0_010_1_101 = 0x2D
        //   GCT=0, color_res=2+1=3, sort=1, GCT_size=5+1=6
        let mut buf = gif89a_100x50();
        buf[10] = 0x2D;
        let info = parse_gif_header(&buf).unwrap();
        assert_eq!(info.s_global_color_table_flag, 0);
        assert_eq!(info.s_color_resolution, 3);
        assert_eq!(info.s_sort_flag, 1);
        assert_eq!(info.s_size_of_global_color_table, 6);
    }

    #[test]
    fn parse_header_accepts_extra_trailing_bytes() {
        let mut buf = Vec::from(gif89a_100x50());
        buf.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
        parse_gif_header(&buf).unwrap();
    }

    // ---- create / destroy -------------------------------------------

    #[test]
    fn create_happy_path() {
        let mut d = GifDec::new();
        assert!(!d.is_created());
        d.create(true).unwrap();
        assert!(d.is_created());
    }

    #[test]
    fn create_null_handle_is_arg() {
        let mut d = GifDec::new();
        assert_eq!(d.create(false).unwrap_err(), errors::ARG);
    }

    #[test]
    fn double_create_is_seq() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        assert_eq!(d.create(true).unwrap_err(), errors::SEQ);
    }

    #[test]
    fn destroy_before_create_is_seq() {
        let mut d = GifDec::new();
        assert_eq!(d.destroy(true).unwrap_err(), errors::SEQ);
    }

    #[test]
    fn destroy_null_handle_is_arg() {
        let mut d = GifDec::new();
        assert_eq!(d.destroy(false).unwrap_err(), errors::ARG);
    }

    #[test]
    fn destroy_clears_streams() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.destroy(true).unwrap();
        assert_eq!(d.stream_count(), 0);
        // after re-create, ids restart
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        assert_eq!(id, SUBSTREAM_ID_BASE);
    }

    // ---- open --------------------------------------------------------

    #[test]
    fn open_requires_create() {
        let mut d = GifDec::new();
        assert_eq!(
            d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap_err(),
            errors::SEQ,
        );
    }

    #[test]
    fn open_null_main_handle_is_arg() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        assert_eq!(
            d.open(false, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap_err(),
            errors::ARG,
        );
    }

    #[test]
    fn open_null_sub_handle_is_arg() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        assert_eq!(
            d.open(true, false, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap_err(),
            errors::ARG,
        );
    }

    #[test]
    fn open_file_empty_name_is_open_file() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        assert_eq!(
            d.open(true, true, GifSrc::File { name: alloc::string::String::new(), offset: 0, size: 13 }).unwrap_err(),
            errors::OPEN_FILE,
        );
    }

    #[test]
    fn open_file_zero_size_is_arg() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        assert_eq!(
            d.open(true, true, GifSrc::File { name: alloc::string::String::from("x.gif"), offset: 0, size: 0 }).unwrap_err(),
            errors::ARG,
        );
    }

    #[test]
    fn open_buffer_zero_addr_is_arg() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        assert_eq!(
            d.open(true, true, GifSrc::Buffer { addr: 0, size: 13 }).unwrap_err(),
            errors::ARG,
        );
    }

    #[test]
    fn open_buffer_zero_size_is_arg() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        assert_eq!(
            d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 0 }).unwrap_err(),
            errors::ARG,
        );
    }

    #[test]
    fn open_allocates_ids_monotonically() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let a = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        let b = d.open(true, true, GifSrc::Buffer { addr: 0x2000, size: 13 }).unwrap();
        assert!(b > a);
    }

    // ---- read_header ------------------------------------------------

    #[test]
    fn read_header_happy_path() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        let info = d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        assert_eq!(info.s_width, 100);
        assert_eq!(d.stream_state(id), Some(StreamState::HeaderRead));
    }

    #[test]
    fn read_header_twice_is_seq() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        assert_eq!(
            d.read_header(true, id, true, &gif89a_100x50()).unwrap_err(),
            errors::SEQ,
        );
    }

    #[test]
    fn read_header_bad_magic_is_stream_format() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        let mut bad = gif89a_100x50();
        bad[0] = 0;
        assert_eq!(
            d.read_header(true, id, true, &bad).unwrap_err(),
            errors::STREAM_FORMAT,
        );
    }

    #[test]
    fn read_header_unknown_substream_is_arg() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        assert_eq!(
            d.read_header(true, 999, true, &gif89a_100x50()).unwrap_err(),
            errors::ARG,
        );
    }

    // ---- set_parameter -----------------------------------------------

    #[test]
    fn set_parameter_before_header_is_seq() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        assert_eq!(
            d.set_parameter(true, id, true,
                GifInParam { color_space: COLORSPACE_RGBA, output_color_alpha1: 0, output_color_alpha2: 0 }).unwrap_err(),
            errors::SEQ,
        );
    }

    #[test]
    fn set_parameter_unknown_color_space_is_arg() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        assert_eq!(
            d.set_parameter(true, id, true,
                GifInParam { color_space: 2, output_color_alpha1: 0, output_color_alpha2: 0 }).unwrap_err(),
            errors::ARG,
        );
    }

    #[test]
    fn set_parameter_rgba_output_components_is_4() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        let out = d.set_parameter(true, id, true,
            GifInParam { color_space: COLORSPACE_RGBA, output_color_alpha1: 0xFF, output_color_alpha2: 0x00 }).unwrap();
        assert_eq!(out.output_components, 4);
        assert_eq!(out.output_color_space, COLORSPACE_RGBA);
        assert_eq!(out.output_width, 100);
        assert_eq!(out.output_height, 50);
    }

    #[test]
    fn set_parameter_width_byte_formula() {
        // SWidth=100, SColorResolution=8 → (100 * 8 * 3) / 8 = 300
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        let out = d.set_parameter(true, id, true,
            GifInParam { color_space: COLORSPACE_ARGB, output_color_alpha1: 0, output_color_alpha2: 0 }).unwrap();
        assert_eq!(out.output_width_byte, 300);
    }

    // ---- decode ------------------------------------------------------

    #[test]
    fn decode_requires_configured_state() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        assert_eq!(
            d.decode(true, id, true, COMMAND_CONTINUE).unwrap_err(),
            errors::SEQ,
        );
    }

    #[test]
    fn decode_continue_returns_finish() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        d.set_parameter(true, id, true,
            GifInParam { color_space: COLORSPACE_RGBA, output_color_alpha1: 0, output_color_alpha2: 0 }).unwrap();
        let out = d.decode(true, id, true, COMMAND_CONTINUE).unwrap();
        assert_eq!(out.status, DEC_STATUS_FINISH);
        assert_eq!(out.record_type, RECORD_TYPE_IMAGE_DESC);
        assert_eq!(d.stream_state(id), Some(StreamState::Decoded));
    }

    #[test]
    fn decode_stop_returns_stop() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        d.set_parameter(true, id, true,
            GifInParam { color_space: COLORSPACE_RGBA, output_color_alpha1: 0, output_color_alpha2: 0 }).unwrap();
        let out = d.decode(true, id, true, COMMAND_STOP).unwrap();
        assert_eq!(out.status, DEC_STATUS_STOP);
    }

    #[test]
    fn decode_unknown_command_is_cb_param() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        d.set_parameter(true, id, true,
            GifInParam { color_space: COLORSPACE_RGBA, output_color_alpha1: 0, output_color_alpha2: 0 }).unwrap();
        assert_eq!(
            d.decode(true, id, true, 42).unwrap_err(),
            errors::CB_PARAM,
        );
    }

    // ---- close -------------------------------------------------------

    #[test]
    fn close_removes_stream() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();
        d.close(true, id).unwrap();
        assert_eq!(d.stream_count(), 0);
        // Re-close is ARG now.
        assert_eq!(d.close(true, id).unwrap_err(), errors::ARG);
    }

    // ---- lifecycle smoke --------------------------------------------

    #[test]
    fn full_gif_lifecycle_smoke() {
        let mut d = GifDec::new();
        d.create(true).unwrap();
        let id = d.open(true, true, GifSrc::Buffer { addr: 0x1000, size: 13 }).unwrap();

        // Read a GIF89a 100x50 header.
        let info = d.read_header(true, id, true, &gif89a_100x50()).unwrap();
        assert_eq!(info.s_width, 100);

        // Configure for RGBA output.
        let out = d.set_parameter(true, id, true,
            GifInParam { color_space: COLORSPACE_RGBA, output_color_alpha1: 0xFF, output_color_alpha2: 0 }).unwrap();
        assert_eq!(out.output_components, 4);
        assert_eq!(out.output_width_byte, 300);

        // Decode continue.
        let do_info = d.decode(true, id, true, COMMAND_CONTINUE).unwrap();
        assert_eq!(do_info.status, DEC_STATUS_FINISH);

        // Tear down.
        d.close(true, id).unwrap();
        d.destroy(true).unwrap();
        assert!(!d.is_created());
    }
}
