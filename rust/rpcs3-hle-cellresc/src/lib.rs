//! `rpcs3-hle-cellresc` — PS3 video rescaling HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellResc.cpp` (410 linhas).  cellResc
//! handles upscaling between PS3 resolutions (720x480, 720x576 PAL,
//! 1280x720, 1920x1080) with PAL temporal mode options.  The Rust
//! port captures the Init/Exit FSM, bufferMode + palTemporal + flip
//! compatibility checks, aspect ratio, flip status state, and the
//! GCM surface → RESC source routing.
//!
//! ## Entry points covered
//!
//! | C++ function                                   | Rust wrapper                       |
//! |------------------------------------------------|------------------------------------|
//! | `cellRescInit` / `cellRescExit`                | [`Resc::init`] / [`Resc::exit`]    |
//! | `cellRescVideoOutResolutionId2RescBufferMode`  | [`Resc::video_out_resolution_id_to_buffer_mode`] |
//! | `cellRescSetDsts`                              | [`Resc::set_dsts`]                 |
//! | `cellRescSetDisplayMode`                       | [`Resc::set_display_mode`]         |
//! | `cellRescAdjustAspectRatio`                    | [`Resc::adjust_aspect_ratio`]      |
//! | `cellRescSetPalInterpolateDropFlexRatio`       | [`Resc::set_pal_interpolate_drop_flex_ratio`] |
//! | `cellRescGetBufferSize`                        | [`Resc::get_buffer_size`]          |
//! | `cellRescGetNumColorBuffers`                   | [`Resc::get_num_color_buffers`]    |
//! | `cellRescSetSrc`                               | [`Resc::set_src`]                  |
//! | `cellRescSetConvertAndFlip`                    | [`Resc::set_convert_and_flip`]     |
//! | `cellRescSetWaitFlip`                          | [`Resc::set_wait_flip`]            |
//! | `cellRescSetBufferAddress`                     | [`Resc::set_buffer_address`]       |
//! | `cellRescSetFlipHandler` / `cellRescSetVBlankHandler` | [`Resc::set_flip_handler`] / [`Resc::set_vblank_handler`] |
//! | `cellRescResetFlipStatus` / `cellRescGetFlipStatus` | [`Resc::reset_flip_status`] / [`Resc::get_flip_status`] |
//! | `cellRescGetRegisterCount` / `SetRegisterCount`| [`Resc::get_register_count`] / [`Resc::set_register_count`] |
//! | `cellRescCreateInterlaceTable`                 | [`Resc::create_interlace_table`]   |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellResc.h:5-12
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_INITIALIZED:   CellError = CellError(0x8021_0301);
    pub const REINITIALIZED:     CellError = CellError(0x8021_0302);
    pub const BAD_ALIGNMENT:     CellError = CellError(0x8021_0303);
    pub const BAD_ARGUMENT:      CellError = CellError(0x8021_0304);
    pub const LESS_MEMORY:       CellError = CellError(0x8021_0305);
    pub const GCM_FLIP_QUE_FULL: CellError = CellError(0x8021_0306);
    pub const BAD_COMBINATION:   CellError = CellError(0x8021_0307);
    pub const X308:              CellError = CellError(0x8021_0308);
}

// =====================================================================
// Constants — byte-exact with cellResc.h
// =====================================================================

pub const CELL_RESC_720X480:   u32 = 0x1;
pub const CELL_RESC_720X576:   u32 = 0x2;
pub const CELL_RESC_1280X720:  u32 = 0x4;
pub const CELL_RESC_1920X1080: u32 = 0x8;

/// Mask of all known buffer modes — equivalent to OR of the 4 bits.
pub const CELL_RESC_BUFFER_MODE_MASK: u32 =
    CELL_RESC_720X480 | CELL_RESC_720X576 | CELL_RESC_1280X720 | CELL_RESC_1920X1080;

pub const CELL_RESC_PAL_60_DROP:                      u32 = 1;
pub const CELL_RESC_PAL_60_INTERPOLATE:               u32 = 2;
pub const CELL_RESC_PAL_60_INTERPOLATE_30_DROP:       u32 = 3;
pub const CELL_RESC_PAL_60_INTERPOLATE_DROP_FLEXIBLE: u32 = 4;
pub const CELL_RESC_PAL_60_FOR_HSYNC:                 u32 = 5;

pub const CELL_RESC_DISPLAY_VSYNC: u32 = 0;
pub const CELL_RESC_DISPLAY_HSYNC: u32 = 1;

pub const CELL_VIDEO_OUT_RESOLUTION_576: u32 = 11;

/// Max `size` field accepted by `cellRescInit` (cpp:44 `> 28`).
pub const MAX_INIT_CONFIG_SIZE: u32 = 28;

// =====================================================================
// Config struct + manager
// =====================================================================

/// Mirror of `CellRescInitConfig`.  Stored verbatim after validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RescInitConfig {
    pub size: u32,
    pub resource_policy: u32,
    pub support_modes: u32,
    pub ratio_mode: u32,
    pub pal_temporal_mode: u32,
    pub interlace_mode: u32,
    pub flip_mode: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RescDsts {
    pub format: u32,
    pub pitch: u32,
    pub heightAlign: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Resc {
    pub is_initialized: bool,
    pub config: RescInitConfig,
    pub buffer_mode: u32,
    pub aspect_horizontal: f32,
    pub aspect_vertical: f32,
    pub pal_interpolate_drop_flex_ratio: f32,
    pub flip_handler: Option<u32>,
    pub vblank_handler: Option<u32>,
    pub flip_status: i32,
    pub register_count: i32,
    pub flip_counter: u64,
    pub src_slots: [Option<u32>; 8],
}

impl Resc {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    fn require_init(&self) -> Result<(), CellError> {
        if !self.is_initialized { return Err(errors::NOT_INITIALIZED); }
        Ok(())
    }

    /// Port of `cellRescInit` (cpp:33-62).
    ///
    /// # Errors
    /// * [`errors::REINITIALIZED`] if already init'd.
    /// * [`errors::BAD_ARGUMENT`] if `size > 28` or config_valid is false.
    pub fn init(&mut self, config_valid: bool, config: RescInitConfig) -> Result<(), CellError> {
        if self.is_initialized { return Err(errors::REINITIALIZED); }
        if !config_valid || config.size > MAX_INIT_CONFIG_SIZE {
            return Err(errors::BAD_ARGUMENT);
        }
        self.config = config;
        self.is_initialized = true;
        Ok(())
    }

    /// Port of `cellRescExit` (cpp:64-70).  Clears the init flag.
    pub fn exit(&mut self) {
        self.is_initialized = false;
    }

    /// Port of `cellRescVideoOutResolutionId2RescBufferMode`.
    ///
    /// # Errors
    /// * [`errors::BAD_ARGUMENT`] if `resolution_id == 0` or `> 11`, or
    ///   if `buffer_mode_ptr_valid` is false.
    pub fn video_out_resolution_id_to_buffer_mode(
        &self,
        resolution_id: u32,
        buffer_mode_ptr_valid: bool,
    ) -> Result<(), CellError> {
        if !buffer_mode_ptr_valid || resolution_id == 0 || resolution_id > CELL_VIDEO_OUT_RESOLUTION_576 {
            return Err(errors::BAD_ARGUMENT);
        }
        Ok(())
    }

    /// Port of `cellRescSetDsts` (cpp:84-101).
    pub fn set_dsts(&self, buffer_mode: u32, dsts_valid: bool) -> Result<(), CellError> {
        self.require_init()?;
        if !dsts_valid || buffer_mode == 0 || buffer_mode > CELL_RESC_1920X1080 {
            return Err(errors::BAD_ARGUMENT);
        }
        Ok(())
    }

    /// Port of `cellRescSetDisplayMode` (cpp:103-145).
    ///
    /// Validates bufferMode against `support_modes` mask and enforces
    /// the PAL-temporal × flip-mode compatibility matrix for
    /// `CELL_RESC_720x576` (PAL).
    pub fn set_display_mode(&mut self, buffer_mode: u32) -> Result<(), CellError> {
        self.require_init()?;
        if buffer_mode == 0 || buffer_mode > CELL_RESC_1920X1080 {
            return Err(errors::BAD_ARGUMENT);
        }
        if (self.config.support_modes & buffer_mode) == 0 {
            return Err(errors::BAD_ARGUMENT);
        }
        if buffer_mode == CELL_RESC_720X576 {
            let pal = self.config.pal_temporal_mode;
            let flip = self.config.flip_mode;
            // cpp:125 check: pal in INTERPOLATE modes or PAL_60_DROP.
            let is_interpolate_or_drop =
                pal.wrapping_sub(CELL_RESC_PAL_60_INTERPOLATE) <= CELL_RESC_PAL_60_INTERPOLATE
                    || pal == CELL_RESC_PAL_60_DROP;
            if is_interpolate_or_drop && flip == CELL_RESC_DISPLAY_HSYNC {
                return Err(errors::BAD_COMBINATION);
            }
            if pal == CELL_RESC_PAL_60_FOR_HSYNC && flip == CELL_RESC_DISPLAY_VSYNC {
                return Err(errors::BAD_COMBINATION);
            }
        }
        self.buffer_mode = buffer_mode;
        Ok(())
    }

    /// Port of `cellRescAdjustAspectRatio`.
    pub fn adjust_aspect_ratio(&mut self, horizontal: f32, vertical: f32) -> Result<(), CellError> {
        self.require_init()?;
        self.aspect_horizontal = horizontal;
        self.aspect_vertical = vertical;
        Ok(())
    }

    /// Port of `cellRescSetPalInterpolateDropFlexRatio`.
    pub fn set_pal_interpolate_drop_flex_ratio(&mut self, ratio: f32) -> Result<(), CellError> {
        self.require_init()?;
        if !(0.0..=1.0).contains(&ratio) {
            return Err(errors::BAD_ARGUMENT);
        }
        self.pal_interpolate_drop_flex_ratio = ratio;
        Ok(())
    }

    /// Port of `cellRescGetBufferSize`.
    ///
    /// # Errors
    /// * [`errors::NOT_INITIALIZED`] if RESC hasn't been init'd.
    pub fn get_buffer_size(&self) -> Result<(i32, i32, i32), CellError> {
        self.require_init()?;
        // C++ stub returns CELL_OK with values unspecified; Rust
        // returns synthetic but deterministic values so tests can
        // assert they're written.
        Ok((
            self.get_num_color_buffers(self.buffer_mode, self.config.pal_temporal_mode, 0) * 0x10_0000,
            0x10_0000,
            0x10_0000,
        ))
    }

    /// Port of `cellRescGetNumColorBuffers` (cpp:219-...).  Different
    /// PAL modes require different buffer counts; 1080p never needs
    /// more than the base.
    #[must_use]
    pub fn get_num_color_buffers(&self, dst_mode: u32, pal_temporal_mode: u32, _reserved: i32) -> i32 {
        if dst_mode == CELL_RESC_720X576 {
            match pal_temporal_mode {
                CELL_RESC_PAL_60_INTERPOLATE => 6,
                CELL_RESC_PAL_60_INTERPOLATE_30_DROP => 6,
                CELL_RESC_PAL_60_INTERPOLATE_DROP_FLEXIBLE => 6,
                CELL_RESC_PAL_60_DROP => 3,
                CELL_RESC_PAL_60_FOR_HSYNC => 2,
                _ => 2,
            }
        } else {
            2
        }
    }

    /// Port of `cellRescSetSrc`.
    ///
    /// # Errors
    /// * [`errors::BAD_ARGUMENT`] if `idx` is out of range (0..=7).
    pub fn set_src(&mut self, idx: i32, src_valid: bool, src_addr: u32) -> Result<(), CellError> {
        self.require_init()?;
        if !src_valid || !(0..=7).contains(&idx) {
            return Err(errors::BAD_ARGUMENT);
        }
        self.src_slots[idx as usize] = Some(src_addr);
        Ok(())
    }

    /// Port of `cellRescSetConvertAndFlip`.  Increments the flip
    /// counter and returns how many flips have happened.
    pub fn set_convert_and_flip(&mut self, idx: i32) -> Result<u64, CellError> {
        self.require_init()?;
        if !(0..=7).contains(&idx) { return Err(errors::BAD_ARGUMENT); }
        if self.src_slots[idx as usize].is_none() {
            return Err(errors::BAD_ARGUMENT);
        }
        self.flip_counter = self.flip_counter.saturating_add(1);
        self.flip_status = 0; // pending flip
        Ok(self.flip_counter)
    }

    /// Port of `cellRescSetWaitFlip`.  Marks the pending flip as done.
    pub fn set_wait_flip(&mut self) -> Result<(), CellError> {
        self.require_init()?;
        self.flip_status = 1;
        Ok(())
    }

    /// Port of `cellRescSetBufferAddress`.
    pub fn set_buffer_address(
        &self,
        color_valid: bool,
        vertex_valid: bool,
        fragment_valid: bool,
    ) -> Result<(), CellError> {
        self.require_init()?;
        if !color_valid || !vertex_valid || !fragment_valid {
            return Err(errors::BAD_ARGUMENT);
        }
        Ok(())
    }

    /// Port of `cellRescSetFlipHandler`.
    pub fn set_flip_handler(&mut self, handler: u32) { self.flip_handler = if handler == 0 { None } else { Some(handler) }; }

    /// Port of `cellRescSetVBlankHandler`.
    pub fn set_vblank_handler(&mut self, handler: u32) { self.vblank_handler = if handler == 0 { None } else { Some(handler) }; }

    /// Port of `cellRescResetFlipStatus`.  C++ sets status back to 1.
    pub fn reset_flip_status(&mut self) { self.flip_status = 1; }

    /// Port of `cellRescGetFlipStatus`.
    #[must_use]
    pub fn get_flip_status(&self) -> i32 { self.flip_status }

    /// Port of `cellRescGetRegisterCount`.
    #[must_use]
    pub fn get_register_count(&self) -> i32 { self.register_count }

    /// Port of `cellRescSetRegisterCount`.
    pub fn set_register_count(&mut self, count: i32) { self.register_count = count; }

    /// Port of `cellRescCreateInterlaceTable`.
    ///
    /// # Errors
    /// * [`errors::BAD_ARGUMENT`] if `length <= 0`, `src_h <= 0.0`, or
    ///   `ea_addr_valid` is false.
    pub fn create_interlace_table(
        &self,
        ea_addr_valid: bool,
        src_h: f32,
        _depth: u32,
        length: i32,
    ) -> Result<(), CellError> {
        self.require_init()?;
        if !ea_addr_valid || length <= 0 || src_h <= 0.0 {
            return Err(errors::BAD_ARGUMENT);
        }
        Ok(())
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn base_config() -> RescInitConfig {
        RescInitConfig {
            size: 28,
            resource_policy: 0,
            support_modes: CELL_RESC_BUFFER_MODE_MASK,
            ratio_mode: 0,
            pal_temporal_mode: 0,
            interlace_mode: 0,
            flip_mode: CELL_RESC_DISPLAY_VSYNC,
        }
    }

    // ---- constants ---------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::NOT_INITIALIZED.0,   0x8021_0301);
        assert_eq!(errors::REINITIALIZED.0,     0x8021_0302);
        assert_eq!(errors::BAD_ALIGNMENT.0,     0x8021_0303);
        assert_eq!(errors::BAD_ARGUMENT.0,      0x8021_0304);
        assert_eq!(errors::LESS_MEMORY.0,       0x8021_0305);
        assert_eq!(errors::GCM_FLIP_QUE_FULL.0, 0x8021_0306);
        assert_eq!(errors::BAD_COMBINATION.0,   0x8021_0307);
        assert_eq!(errors::X308.0,              0x8021_0308);
    }

    #[test]
    fn buffer_mode_constants() {
        assert_eq!(CELL_RESC_720X480, 0x1);
        assert_eq!(CELL_RESC_720X576, 0x2);
        assert_eq!(CELL_RESC_1280X720, 0x4);
        assert_eq!(CELL_RESC_1920X1080, 0x8);
        assert_eq!(CELL_RESC_BUFFER_MODE_MASK, 0xF);
    }

    #[test]
    fn pal_temporal_constants() {
        assert_eq!(CELL_RESC_PAL_60_DROP, 1);
        assert_eq!(CELL_RESC_PAL_60_INTERPOLATE, 2);
        assert_eq!(CELL_RESC_PAL_60_INTERPOLATE_30_DROP, 3);
        assert_eq!(CELL_RESC_PAL_60_INTERPOLATE_DROP_FLEXIBLE, 4);
        assert_eq!(CELL_RESC_PAL_60_FOR_HSYNC, 5);
    }

    #[test]
    fn max_init_config_size_byte_exact() {
        assert_eq!(MAX_INIT_CONFIG_SIZE, 28);
    }

    // ---- init / exit -------------------------------------------------

    #[test]
    fn init_happy_path() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert!(r.is_initialized);
    }

    #[test]
    fn init_reinitialized_is_error() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.init(true, base_config()).unwrap_err(), errors::REINITIALIZED);
    }

    #[test]
    fn init_null_config_is_bad_arg() {
        let mut r = Resc::new();
        assert_eq!(r.init(false, base_config()).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn init_oversize_config_is_bad_arg() {
        let mut r = Resc::new();
        let mut cfg = base_config();
        cfg.size = 29;
        assert_eq!(r.init(true, cfg).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn exit_clears_flag() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        r.exit();
        assert!(!r.is_initialized);
    }

    // ---- resolution / bufferMode validation -------------------------

    #[test]
    fn resolution_id_zero_is_bad_arg() {
        let r = Resc::new();
        assert_eq!(
            r.video_out_resolution_id_to_buffer_mode(0, true).unwrap_err(),
            errors::BAD_ARGUMENT,
        );
    }

    #[test]
    fn resolution_id_too_high_is_bad_arg() {
        let r = Resc::new();
        assert_eq!(
            r.video_out_resolution_id_to_buffer_mode(12, true).unwrap_err(),
            errors::BAD_ARGUMENT,
        );
    }

    #[test]
    fn resolution_id_null_ptr_is_bad_arg() {
        let r = Resc::new();
        assert_eq!(
            r.video_out_resolution_id_to_buffer_mode(1, false).unwrap_err(),
            errors::BAD_ARGUMENT,
        );
    }

    #[test]
    fn resolution_id_valid_range_ok() {
        let r = Resc::new();
        for id in 1..=CELL_VIDEO_OUT_RESOLUTION_576 {
            assert!(r.video_out_resolution_id_to_buffer_mode(id, true).is_ok());
        }
    }

    // ---- set_dsts ----------------------------------------------------

    #[test]
    fn set_dsts_without_init_is_not_initialized() {
        let r = Resc::new();
        assert_eq!(r.set_dsts(CELL_RESC_1280X720, true).unwrap_err(), errors::NOT_INITIALIZED);
    }

    #[test]
    fn set_dsts_null_dsts_is_bad_arg() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.set_dsts(CELL_RESC_1280X720, false).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn set_dsts_zero_mode_is_bad_arg() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.set_dsts(0, true).unwrap_err(), errors::BAD_ARGUMENT);
    }

    // ---- set_display_mode -------------------------------------------

    #[test]
    fn set_display_mode_unsupported_is_bad_arg() {
        let mut r = Resc::new();
        let mut cfg = base_config();
        cfg.support_modes = CELL_RESC_720X480; // only SD supported
        r.init(true, cfg).unwrap();
        assert_eq!(r.set_display_mode(CELL_RESC_1920X1080).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn set_display_mode_supported_ok() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        r.set_display_mode(CELL_RESC_1920X1080).unwrap();
        assert_eq!(r.buffer_mode, CELL_RESC_1920X1080);
    }

    #[test]
    fn set_display_mode_pal_interpolate_hsync_is_bad_combination() {
        let mut r = Resc::new();
        let mut cfg = base_config();
        cfg.pal_temporal_mode = CELL_RESC_PAL_60_INTERPOLATE;
        cfg.flip_mode = CELL_RESC_DISPLAY_HSYNC;
        r.init(true, cfg).unwrap();
        assert_eq!(r.set_display_mode(CELL_RESC_720X576).unwrap_err(), errors::BAD_COMBINATION);
    }

    #[test]
    fn set_display_mode_pal_60_drop_hsync_is_bad_combination() {
        let mut r = Resc::new();
        let mut cfg = base_config();
        cfg.pal_temporal_mode = CELL_RESC_PAL_60_DROP;
        cfg.flip_mode = CELL_RESC_DISPLAY_HSYNC;
        r.init(true, cfg).unwrap();
        assert_eq!(r.set_display_mode(CELL_RESC_720X576).unwrap_err(), errors::BAD_COMBINATION);
    }

    #[test]
    fn set_display_mode_pal_for_hsync_vsync_is_bad_combination() {
        let mut r = Resc::new();
        let mut cfg = base_config();
        cfg.pal_temporal_mode = CELL_RESC_PAL_60_FOR_HSYNC;
        cfg.flip_mode = CELL_RESC_DISPLAY_VSYNC;
        r.init(true, cfg).unwrap();
        assert_eq!(r.set_display_mode(CELL_RESC_720X576).unwrap_err(), errors::BAD_COMBINATION);
    }

    #[test]
    fn set_display_mode_pal_interpolate_vsync_ok() {
        let mut r = Resc::new();
        let mut cfg = base_config();
        cfg.pal_temporal_mode = CELL_RESC_PAL_60_INTERPOLATE;
        cfg.flip_mode = CELL_RESC_DISPLAY_VSYNC;
        r.init(true, cfg).unwrap();
        r.set_display_mode(CELL_RESC_720X576).unwrap();
    }

    #[test]
    fn set_display_mode_non_pal_buffer_skips_pal_checks() {
        let mut r = Resc::new();
        let mut cfg = base_config();
        cfg.pal_temporal_mode = CELL_RESC_PAL_60_INTERPOLATE;
        cfg.flip_mode = CELL_RESC_DISPLAY_HSYNC; // would be bad for 576 but 1080p skips
        r.init(true, cfg).unwrap();
        r.set_display_mode(CELL_RESC_1920X1080).unwrap();
    }

    // ---- aspect ratio + pal flex ratio ------------------------------

    #[test]
    fn adjust_aspect_ratio_stores_values() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        r.adjust_aspect_ratio(16.0, 9.0).unwrap();
        assert_eq!(r.aspect_horizontal, 16.0);
        assert_eq!(r.aspect_vertical, 9.0);
    }

    #[test]
    fn pal_flex_ratio_accepts_unit_range() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        r.set_pal_interpolate_drop_flex_ratio(0.5).unwrap();
        assert_eq!(r.pal_interpolate_drop_flex_ratio, 0.5);
    }

    #[test]
    fn pal_flex_ratio_rejects_out_of_range() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.set_pal_interpolate_drop_flex_ratio(1.5).unwrap_err(), errors::BAD_ARGUMENT);
        assert_eq!(r.set_pal_interpolate_drop_flex_ratio(-0.1).unwrap_err(), errors::BAD_ARGUMENT);
    }

    // ---- num color buffers ------------------------------------------

    #[test]
    fn num_color_buffers_pal_interpolate_is_6() {
        let r = Resc::new();
        assert_eq!(r.get_num_color_buffers(CELL_RESC_720X576, CELL_RESC_PAL_60_INTERPOLATE, 0), 6);
    }

    #[test]
    fn num_color_buffers_pal_drop_is_3() {
        let r = Resc::new();
        assert_eq!(r.get_num_color_buffers(CELL_RESC_720X576, CELL_RESC_PAL_60_DROP, 0), 3);
    }

    #[test]
    fn num_color_buffers_1080p_is_2() {
        let r = Resc::new();
        assert_eq!(r.get_num_color_buffers(CELL_RESC_1920X1080, CELL_RESC_PAL_60_INTERPOLATE, 0), 2);
    }

    // ---- src slots / flip -------------------------------------------

    #[test]
    fn set_src_oob_is_bad_arg() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.set_src(8, true, 0x1000).unwrap_err(), errors::BAD_ARGUMENT);
        assert_eq!(r.set_src(-1, true, 0x1000).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn set_src_valid_stores() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        r.set_src(3, true, 0x5000).unwrap();
        assert_eq!(r.src_slots[3], Some(0x5000));
    }

    #[test]
    fn convert_and_flip_increments_counter() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        r.set_src(0, true, 0x1000).unwrap();
        let n1 = r.set_convert_and_flip(0).unwrap();
        let n2 = r.set_convert_and_flip(0).unwrap();
        assert_eq!((n1, n2), (1, 2));
    }

    #[test]
    fn convert_and_flip_unset_src_is_bad_arg() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.set_convert_and_flip(0).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn flip_handlers_roundtrip() {
        let mut r = Resc::new();
        r.set_flip_handler(0x1000);
        r.set_vblank_handler(0x2000);
        assert_eq!(r.flip_handler, Some(0x1000));
        assert_eq!(r.vblank_handler, Some(0x2000));
        // Zero clears.
        r.set_flip_handler(0);
        r.set_vblank_handler(0);
        assert!(r.flip_handler.is_none());
        assert!(r.vblank_handler.is_none());
    }

    #[test]
    fn flip_status_lifecycle() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        r.set_src(0, true, 0x1000).unwrap();
        r.set_convert_and_flip(0).unwrap();
        assert_eq!(r.get_flip_status(), 0); // pending
        r.set_wait_flip().unwrap();
        assert_eq!(r.get_flip_status(), 1); // done
        r.reset_flip_status();
        assert_eq!(r.get_flip_status(), 1); // reset sets back to 1
    }

    // ---- register count + interlace table ---------------------------

    #[test]
    fn register_count_roundtrip() {
        let mut r = Resc::new();
        r.set_register_count(512);
        assert_eq!(r.get_register_count(), 512);
    }

    #[test]
    fn interlace_table_null_ea_is_bad_arg() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.create_interlace_table(false, 1.0, 0, 10).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn interlace_table_zero_length_is_bad_arg() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.create_interlace_table(true, 1.0, 0, 0).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn interlace_table_zero_src_h_is_bad_arg() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        assert_eq!(r.create_interlace_table(true, 0.0, 0, 10).unwrap_err(), errors::BAD_ARGUMENT);
    }

    #[test]
    fn interlace_table_valid_ok() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();
        r.create_interlace_table(true, 480.0, 0, 1024).unwrap();
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_cellresc_lifecycle_smoke() {
        let mut r = Resc::new();
        r.init(true, base_config()).unwrap();

        // 1. Set display mode to 1080p.
        r.set_display_mode(CELL_RESC_1920X1080).unwrap();
        assert_eq!(r.buffer_mode, CELL_RESC_1920X1080);

        // 2. Aspect ratio + PAL flex ratio.
        r.adjust_aspect_ratio(16.0, 9.0).unwrap();
        r.set_pal_interpolate_drop_flex_ratio(0.75).unwrap();

        // 3. Register SRC slot + flip.
        r.set_src(0, true, 0xDEAD_0000).unwrap();
        r.set_src(1, true, 0xBEEF_0000).unwrap();
        let count = r.set_convert_and_flip(0).unwrap();
        assert_eq!(count, 1);
        assert_eq!(r.get_flip_status(), 0);
        r.set_wait_flip().unwrap();
        assert_eq!(r.get_flip_status(), 1);

        // 4. Buffer size query.
        let (col, vtx, frag) = r.get_buffer_size().unwrap();
        assert!(col > 0 && vtx > 0 && frag > 0);

        // 5. Flip/VBlank handlers.
        r.set_flip_handler(0x1000);
        r.set_vblank_handler(0x2000);

        // 6. Register count.
        r.set_register_count(256);
        assert_eq!(r.get_register_count(), 256);

        // 7. Exit + reinit fails no more.
        r.exit();
        assert!(!r.is_initialized);
        r.init(true, base_config()).unwrap(); // ok after exit
    }
}
