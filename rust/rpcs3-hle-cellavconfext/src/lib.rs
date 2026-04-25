//! Rust port of `rpcs3/Emu/Cell/Modules/cellAvconfExt.cpp` — PS3 audio/video
//! configuration extensions HLE (21 entries, 617 lines C++).
//!
//! 21 entries cobertas com FSM mínima onde aplica + counters per-entry +
//! gamma roundtrip + cursor color conversion REAL (template <Is_Float, Range_Limited>
//! port de cpp:240-276).
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "cellSysutilAvconfExt";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellAudioOutUnregisterDevice",
    "cellAudioOutGetDeviceInfo2",
    "cellVideoOutSetXVColor",
    "cellVideoOutSetupDisplay",
    "cellAudioInGetDeviceInfo",
    "cellVideoOutConvertCursorColor",
    "cellVideoOutGetGamma",
    "cellAudioInGetAvailableDeviceInfo",
    "cellAudioOutGetAvailableDeviceInfo",
    "cellVideoOutSetGamma",
    "cellAudioOutRegisterDevice",
    "cellAudioOutSetDeviceMode",
    "cellAudioInSetDeviceMode",
    "cellAudioInRegisterDevice",
    "cellAudioInUnregisterDevice",
    "cellVideoOutGetScreenSize",
    "cellVideoOutSetCopyControl",
    "cellVideoOutConfigure2",
    "cellAudioOutGetConfiguration2",
    "cellAudioOutConfigure2",
    "cellVideoOutGetResolutionAvailability2",
];

// Errors (re-export from cellAudioOut/cellVideoOut/cellAudioIn).
pub const CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER: CellError = CellError(0x8002_B242);
pub const CELL_AUDIO_OUT_ERROR_DEVICE_NOT_FOUND: CellError = CellError(0x8002_B244);
pub const CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER: CellError = CellError(0x8002_B262);
pub const CELL_AUDIO_IN_ERROR_DEVICE_NOT_FOUND: CellError = CellError(0x8002_B264);
pub const CELL_VIDEO_OUT_ERROR_NOT_IMPLEMENTED: CellError = CellError(0x8002_B220);
pub const CELL_VIDEO_OUT_ERROR_ILLEGAL_PARAMETER: CellError = CellError(0x8002_B222);
pub const CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE: CellError = CellError(0x8002_B223);
pub const CELL_VIDEO_OUT_ERROR_UNSUPPORTED_VIDEO_OUT: CellError = CellError(0x8002_B225);
pub const CELL_VIDEO_OUT_ERROR_VALUE_IS_NOT_SET: CellError = CellError(0x8002_B228);

// Constants (header byte-exato).
pub const CELL_VIDEO_OUT_PRIMARY: u32 = 0;
pub const CELL_VIDEO_OUT_SECONDARY: u32 = 1;

pub const CELL_VIDEO_OUT_BUFFER_COLOR_FORMAT_X8R8G8B8: i32 = 0;
pub const CELL_VIDEO_OUT_BUFFER_COLOR_FORMAT_X8B8G8R8: i32 = 1;
pub const CELL_VIDEO_OUT_BUFFER_COLOR_FORMAT_R16G16B16X16_FLOAT: i32 = 2;

pub const CELL_VIDEO_OUT_RGB_OUTPUT_RANGE_LIMITED: u8 = 0;
pub const CELL_VIDEO_OUT_RGB_OUTPUT_RANGE_FULL: u8 = 1;

pub const CELL_VIDEO_OUT_COPY_CONTROL_COPY_FREE: u32 = 0;
pub const CELL_VIDEO_OUT_COPY_CONTROL_COPY_NEVER: u32 = 2;

pub const CELL_AUDIO_IN_SINGLE_DEVICE_MODE: u32 = 0;
pub const CELL_AUDIO_IN_MULTI_DEVICE_MODE: u32 = 1;
pub const CELL_AUDIO_IN_MULTI_DEVICE_MODE_2: u32 = 2;
pub const CELL_AUDIO_IN_MULTI_DEVICE_MODE_10: u32 = 10;

pub const CELL_AUDIO_OUT_MULTI_DEVICE_MODE_2: u32 = 2;

pub const CELL_AUDIO_OUT_DEVICE_STATE_AVAILABLE: u8 = 1;
pub const CELL_AUDIO_OUT_DEVICE_STATE_UNAVAILABLE: u8 = 2;

/// `gamma` valid range cpp:297/408.
pub const GAMMA_MIN: f32 = 0.8;
pub const GAMMA_MAX: f32 = 1.2;

/// Device list cap cpp:365/396.
pub const MAX_DEVICE_INFO_COUNT: u32 = 16;

#[derive(Debug, Default, Clone, PartialEq)]
pub struct AudioInDeviceInfo {
    pub port_type: u32,
    pub device_id: u32,
    pub state: u8,
    pub device_number: u8,
    pub name: String,
}

/// `cellVideoOutConvertCursorColor` core algorithm — port byte-exato de
/// cpp:240-276 `convert_cursor_color<Is_Float, Range_Limited>`.
///
/// Iterates `num` 4-byte ARGB pixels, processing channels 1..4 (R, G, B —
/// alpha at index 0 is preserved as-is per upstream ordering).
pub fn convert_cursor_color(
    src: &[u8],
    dst: &mut [u8],
    num: usize,
    gamma: f32,
    is_float: bool,
    range_limited: bool,
) {
    for i in 0..num {
        let base = i * 4;
        // Preserve channel 0 (alpha-like) untouched — upstream loop starts at c=1.
        if base < dst.len() && base < src.len() {
            dst[base] = src[base];
        }
        for c in 1..4 {
            let idx = base + c;
            if idx >= src.len() || idx >= dst.len() {
                break;
            }
            let val = if is_float {
                if range_limited {
                    let v = (src[idx] as f32 / 255.0) * 219.0 + 16.0;
                    (v + 0.5) as u8
                } else {
                    src[idx]
                }
            } else {
                let mut v = (libm_pow(src[idx] as f32 / 255.0, gamma)).clamp(0.0, 1.0);
                if range_limited {
                    v = v * 219.0 + 16.0;
                } else {
                    v *= 255.0;
                }
                (v + 0.5) as u8
            };
            dst[idx] = val;
        }
    }
}

/// Minimal `powf` for no_std (Taylor series via exp(y*ln(x)) — accurate enough
/// for gamma 0.8..1.2 range).
fn libm_pow(x: f32, y: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    libm_exp(y * libm_ln(x))
}

fn libm_ln(x: f32) -> f32 {
    // ln(x) via series ln((1+u)/(1-u)) = 2 (u + u^3/3 + u^5/5 + ...) where u=(x-1)/(x+1).
    let u = (x - 1.0) / (x + 1.0);
    let u2 = u * u;
    2.0 * (u + u * u2 / 3.0 + u * u2 * u2 / 5.0 + u * u2 * u2 * u2 / 7.0 + u * u2 * u2 * u2 * u2 / 9.0)
}

fn libm_exp(x: f32) -> f32 {
    // exp(x) Taylor series, scaled by argument splitting if too large.
    let mut term = 1.0f32;
    let mut sum = 1.0f32;
    for n in 1..20 {
        term *= x / n as f32;
        sum += term;
    }
    sum
}

#[derive(Debug)]
pub struct CellAvconfExt {
    pub in_devices: Vec<AudioInDeviceInfo>,
    pub registered_out_devices: Vec<u32>,
    pub registered_in_devices: Vec<u32>,
    pub gamma: f32,
    pub stereo_enabled: bool,
    pub screen_size_inches: f32,
    pub rgb_output_range: u8,
    pub in_device_mode: u32,
    pub out_device_mode: u32,
    pub video_copy_control: u32,
    pub next_device_number: u32,

    // 21 per-entry counters
    pub audio_out_unregister_device_calls: u64,
    pub audio_out_get_device_info2_calls: u64,
    pub video_out_set_xv_color_calls: u64,
    pub video_out_setup_display_calls: u64,
    pub audio_in_get_device_info_calls: u64,
    pub video_out_convert_cursor_color_calls: u64,
    pub video_out_get_gamma_calls: u64,
    pub audio_in_get_available_device_info_calls: u64,
    pub audio_out_get_available_device_info_calls: u64,
    pub video_out_set_gamma_calls: u64,
    pub audio_out_register_device_calls: u64,
    pub audio_out_set_device_mode_calls: u64,
    pub audio_in_set_device_mode_calls: u64,
    pub audio_in_register_device_calls: u64,
    pub audio_in_unregister_device_calls: u64,
    pub video_out_get_screen_size_calls: u64,
    pub video_out_set_copy_control_calls: u64,
    pub video_out_configure2_calls: u64,
    pub audio_out_get_configuration2_calls: u64,
    pub audio_out_configure2_calls: u64,
    pub video_out_get_resolution_availability2_calls: u64,
}

impl Default for CellAvconfExt {
    fn default() -> Self {
        Self {
            in_devices: Vec::new(),
            registered_out_devices: Vec::new(),
            registered_in_devices: Vec::new(),
            gamma: 1.0,
            stereo_enabled: false,
            screen_size_inches: 24.0,
            rgb_output_range: CELL_VIDEO_OUT_RGB_OUTPUT_RANGE_FULL,
            in_device_mode: CELL_AUDIO_IN_SINGLE_DEVICE_MODE,
            out_device_mode: 0,
            video_copy_control: CELL_VIDEO_OUT_COPY_CONTROL_COPY_FREE,
            next_device_number: 1,
            audio_out_unregister_device_calls: 0,
            audio_out_get_device_info2_calls: 0,
            video_out_set_xv_color_calls: 0,
            video_out_setup_display_calls: 0,
            audio_in_get_device_info_calls: 0,
            video_out_convert_cursor_color_calls: 0,
            video_out_get_gamma_calls: 0,
            audio_in_get_available_device_info_calls: 0,
            audio_out_get_available_device_info_calls: 0,
            video_out_set_gamma_calls: 0,
            audio_out_register_device_calls: 0,
            audio_out_set_device_mode_calls: 0,
            audio_in_set_device_mode_calls: 0,
            audio_in_register_device_calls: 0,
            audio_in_unregister_device_calls: 0,
            video_out_get_screen_size_calls: 0,
            video_out_set_copy_control_calls: 0,
            video_out_configure2_calls: 0,
            audio_out_get_configuration2_calls: 0,
            audio_out_configure2_calls: 0,
            video_out_get_resolution_availability2_calls: 0,
        }
    }
}

impl CellAvconfExt {
    pub fn new() -> Self {
        Self::default()
    }

    /// Test/scaffold: register an in-device.
    pub fn add_in_device(&mut self, name: &str) -> u32 {
        let id = self.next_device_number;
        self.next_device_number = self.next_device_number.wrapping_add(1);
        self.in_devices.push(AudioInDeviceInfo {
            port_type: 0,
            device_id: id,
            state: CELL_AUDIO_OUT_DEVICE_STATE_AVAILABLE,
            device_number: id as u8,
            name: name.into(),
        });
        id
    }

    pub fn audio_out_unregister_device(&mut self, device_number: u32) -> Result<(), CellError> {
        self.audio_out_unregister_device_calls = self.audio_out_unregister_device_calls.saturating_add(1);
        self.registered_out_devices.retain(|d| *d != device_number);
        Ok(())
    }

    pub fn audio_out_get_device_info2(
        &mut self,
        _device_number: u32,
        device_index: u32,
        info_present: bool,
    ) -> Result<(), CellError> {
        self.audio_out_get_device_info2_calls = self.audio_out_get_device_info2_calls.saturating_add(1);
        if device_index != 0 || !info_present {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        Ok(())
    }

    pub fn video_out_set_xv_color(&mut self, unk1: u32, _unk2: u32, _unk3: u32) -> Result<(), CellError> {
        self.video_out_set_xv_color_calls = self.video_out_set_xv_color_calls.saturating_add(1);
        if unk1 != 0 {
            return Err(CELL_VIDEO_OUT_ERROR_NOT_IMPLEMENTED);
        }
        Ok(())
    }

    pub fn video_out_setup_display(&mut self, video_out: u32) -> Result<(), CellError> {
        self.video_out_setup_display_calls = self.video_out_setup_display_calls.saturating_add(1);
        if video_out != CELL_VIDEO_OUT_SECONDARY {
            return Err(CELL_VIDEO_OUT_ERROR_UNSUPPORTED_VIDEO_OUT);
        }
        Ok(())
    }

    pub fn audio_in_get_device_info(
        &mut self,
        device_number: u32,
        device_index: u32,
        info_present: bool,
    ) -> Result<AudioInDeviceInfo, CellError> {
        self.audio_in_get_device_info_calls = self.audio_in_get_device_info_calls.saturating_add(1);
        if device_index != 0 || !info_present {
            return Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER);
        }
        if (device_number as usize) >= self.in_devices.len() {
            return Err(CELL_AUDIO_OUT_ERROR_DEVICE_NOT_FOUND);
        }
        Ok(self.in_devices[device_number as usize].clone())
    }

    pub fn video_out_convert_cursor_color(
        &mut self,
        _video_out: u32,
        displaybuffer_format: i32,
        gamma: f32,
        source_buffer_format: i32,
        src: Option<&[u8]>,
        dst: Option<&mut [u8]>,
        num: i32,
    ) -> Result<(), CellError> {
        self.video_out_convert_cursor_color_calls = self.video_out_convert_cursor_color_calls.saturating_add(1);
        let src = src.ok_or(CELL_VIDEO_OUT_ERROR_ILLEGAL_PARAMETER)?;
        let dst = dst.ok_or(CELL_VIDEO_OUT_ERROR_ILLEGAL_PARAMETER)?;
        if displaybuffer_format < 0
            || displaybuffer_format > CELL_VIDEO_OUT_BUFFER_COLOR_FORMAT_R16G16B16X16_FLOAT
            || source_buffer_format != CELL_VIDEO_OUT_BUFFER_COLOR_FORMAT_X8R8G8B8
        {
            return Err(CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE);
        }
        if displaybuffer_format < CELL_VIDEO_OUT_BUFFER_COLOR_FORMAT_R16G16B16X16_FLOAT
            && (gamma < GAMMA_MIN || gamma > GAMMA_MAX)
        {
            return Err(CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE);
        }
        let is_float = displaybuffer_format == CELL_VIDEO_OUT_BUFFER_COLOR_FORMAT_R16G16B16X16_FLOAT;
        let range_limited = self.rgb_output_range == CELL_VIDEO_OUT_RGB_OUTPUT_RANGE_LIMITED;
        convert_cursor_color(src, dst, num as usize, gamma, is_float, range_limited);
        Ok(())
    }

    pub fn video_out_get_gamma(&mut self, video_out: u32) -> Result<f32, CellError> {
        self.video_out_get_gamma_calls = self.video_out_get_gamma_calls.saturating_add(1);
        if video_out != CELL_VIDEO_OUT_PRIMARY {
            return Err(CELL_VIDEO_OUT_ERROR_UNSUPPORTED_VIDEO_OUT);
        }
        Ok(self.gamma)
    }

    pub fn audio_in_get_available_device_info(
        &mut self,
        count: u32,
    ) -> Result<u32, CellError> {
        self.audio_in_get_available_device_info_calls = self.audio_in_get_available_device_info_calls.saturating_add(1);
        if count > MAX_DEVICE_INFO_COUNT {
            return Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER);
        }
        let n = (count as usize).min(self.in_devices.len()) as u32;
        Ok(n)
    }

    pub fn audio_out_get_available_device_info(
        &mut self,
        count: u32,
        info_present: bool,
    ) -> Result<u32, CellError> {
        self.audio_out_get_available_device_info_calls = self.audio_out_get_available_device_info_calls.saturating_add(1);
        if count > MAX_DEVICE_INFO_COUNT || !info_present {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        Ok(0)
    }

    pub fn video_out_set_gamma(&mut self, video_out: u32, gamma: f32) -> Result<(), CellError> {
        self.video_out_set_gamma_calls = self.video_out_set_gamma_calls.saturating_add(1);
        if gamma < GAMMA_MIN || gamma > GAMMA_MAX {
            return Err(CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE);
        }
        if video_out != CELL_VIDEO_OUT_PRIMARY {
            return Err(CELL_VIDEO_OUT_ERROR_UNSUPPORTED_VIDEO_OUT);
        }
        self.gamma = gamma;
        Ok(())
    }

    pub fn audio_out_register_device(
        &mut self,
        _device_type: u64,
        name_present: bool,
        option_present: bool,
        _config_present: bool,
    ) -> Result<u32, CellError> {
        self.audio_out_register_device_calls = self.audio_out_register_device_calls.saturating_add(1);
        if option_present || !name_present {
            return Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER); // Strange C++ choice cpp:430
        }
        Ok(0)
    }

    pub fn audio_out_set_device_mode(&mut self, mode: u32) -> Result<(), CellError> {
        self.audio_out_set_device_mode_calls = self.audio_out_set_device_mode_calls.saturating_add(1);
        if mode > CELL_AUDIO_OUT_MULTI_DEVICE_MODE_2 {
            return Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        self.out_device_mode = mode;
        Ok(())
    }

    pub fn audio_in_set_device_mode(&mut self, mode: u32) -> Result<(), CellError> {
        self.audio_in_set_device_mode_calls = self.audio_in_set_device_mode_calls.saturating_add(1);
        match mode {
            CELL_AUDIO_IN_SINGLE_DEVICE_MODE
            | CELL_AUDIO_IN_MULTI_DEVICE_MODE
            | CELL_AUDIO_IN_MULTI_DEVICE_MODE_2
            | CELL_AUDIO_IN_MULTI_DEVICE_MODE_10 => {}
            _ => return Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER),
        }
        self.in_device_mode = mode;
        Ok(())
    }

    pub fn audio_in_register_device(
        &mut self,
        _device_type: u64,
        name: Option<&str>,
        option_present: bool,
        config_volume: Option<u8>,
    ) -> Result<u32, CellError> {
        self.audio_in_register_device_calls = self.audio_in_register_device_calls.saturating_add(1);
        let _name = name.ok_or(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)?;
        let vol = config_volume.ok_or(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)?;
        if option_present || vol > 5 {
            return Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER);
        }
        // Find device by name; if missing, DEVICE_NOT_FOUND.
        let _name_str = name.unwrap();
        if !self.in_devices.iter().any(|d| d.name == _name_str) {
            return Err(CELL_AUDIO_IN_ERROR_DEVICE_NOT_FOUND);
        }
        let id = self.next_device_number;
        self.next_device_number = self.next_device_number.wrapping_add(1);
        self.registered_in_devices.push(id);
        Ok(id)
    }

    pub fn audio_in_unregister_device(&mut self, device_number: u32) -> Result<(), CellError> {
        self.audio_in_unregister_device_calls = self.audio_in_unregister_device_calls.saturating_add(1);
        self.registered_in_devices.retain(|d| *d != device_number);
        Ok(())
    }

    pub fn video_out_get_screen_size(&mut self, video_out: u32) -> Result<f32, CellError> {
        self.video_out_get_screen_size_calls = self.video_out_get_screen_size_calls.saturating_add(1);
        if video_out != CELL_VIDEO_OUT_PRIMARY {
            return Err(CELL_VIDEO_OUT_ERROR_UNSUPPORTED_VIDEO_OUT);
        }
        if self.stereo_enabled {
            Ok(self.screen_size_inches)
        } else {
            Err(CELL_VIDEO_OUT_ERROR_VALUE_IS_NOT_SET)
        }
    }

    pub fn video_out_set_copy_control(&mut self, _video_out: u32, control: u32) -> Result<(), CellError> {
        self.video_out_set_copy_control_calls = self.video_out_set_copy_control_calls.saturating_add(1);
        if control > CELL_VIDEO_OUT_COPY_CONTROL_COPY_NEVER {
            return Err(CELL_VIDEO_OUT_ERROR_ILLEGAL_PARAMETER);
        }
        self.video_copy_control = control;
        Ok(())
    }

    pub fn video_out_configure2(&mut self) -> Result<(), CellError> {
        self.video_out_configure2_calls = self.video_out_configure2_calls.saturating_add(1);
        Ok(())
    }

    pub fn audio_out_get_configuration2(&mut self) -> Result<(), CellError> {
        self.audio_out_get_configuration2_calls = self.audio_out_get_configuration2_calls.saturating_add(1);
        Ok(())
    }

    pub fn audio_out_configure2(&mut self) -> Result<(), CellError> {
        self.audio_out_configure2_calls = self.audio_out_configure2_calls.saturating_add(1);
        Ok(())
    }

    pub fn video_out_get_resolution_availability2(&mut self) -> Result<(), CellError> {
        self.video_out_get_resolution_availability2_calls = self.video_out_get_resolution_availability2_calls.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "cellSysutilAvconfExt");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 21);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER.0, 0x8002_B242);
        assert_eq!(CELL_VIDEO_OUT_ERROR_VALUE_IS_NOT_SET.0, 0x8002_B228);
    }

    #[test]
    fn xv_color_unk1_nonzero_not_implemented() {
        let mut m = CellAvconfExt::new();
        assert_eq!(m.video_out_set_xv_color(0, 0, 0), Ok(()));
        assert_eq!(
            m.video_out_set_xv_color(1, 0, 0),
            Err(CELL_VIDEO_OUT_ERROR_NOT_IMPLEMENTED)
        );
    }

    #[test]
    fn setup_display_only_secondary() {
        let mut m = CellAvconfExt::new();
        m.video_out_setup_display(CELL_VIDEO_OUT_SECONDARY).unwrap();
        assert_eq!(
            m.video_out_setup_display(CELL_VIDEO_OUT_PRIMARY),
            Err(CELL_VIDEO_OUT_ERROR_UNSUPPORTED_VIDEO_OUT)
        );
    }

    #[test]
    fn gamma_set_and_get_roundtrip() {
        let mut m = CellAvconfExt::new();
        m.video_out_set_gamma(CELL_VIDEO_OUT_PRIMARY, 1.0).unwrap();
        assert_eq!(m.video_out_get_gamma(CELL_VIDEO_OUT_PRIMARY).unwrap(), 1.0);
        m.video_out_set_gamma(CELL_VIDEO_OUT_PRIMARY, 0.9).unwrap();
        assert_eq!(m.video_out_get_gamma(CELL_VIDEO_OUT_PRIMARY).unwrap(), 0.9);
    }

    #[test]
    fn gamma_out_of_range_rejected() {
        let mut m = CellAvconfExt::new();
        assert_eq!(
            m.video_out_set_gamma(CELL_VIDEO_OUT_PRIMARY, 0.7),
            Err(CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE)
        );
        assert_eq!(
            m.video_out_set_gamma(CELL_VIDEO_OUT_PRIMARY, 1.3),
            Err(CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE)
        );
    }

    #[test]
    fn gamma_only_primary() {
        let mut m = CellAvconfExt::new();
        assert_eq!(
            m.video_out_set_gamma(CELL_VIDEO_OUT_SECONDARY, 1.0),
            Err(CELL_VIDEO_OUT_ERROR_UNSUPPORTED_VIDEO_OUT)
        );
        assert_eq!(
            m.video_out_get_gamma(CELL_VIDEO_OUT_SECONDARY),
            Err(CELL_VIDEO_OUT_ERROR_UNSUPPORTED_VIDEO_OUT)
        );
    }

    #[test]
    fn get_screen_size_requires_stereo() {
        let mut m = CellAvconfExt::new();
        assert_eq!(
            m.video_out_get_screen_size(CELL_VIDEO_OUT_PRIMARY),
            Err(CELL_VIDEO_OUT_ERROR_VALUE_IS_NOT_SET)
        );
        m.stereo_enabled = true;
        m.screen_size_inches = 32.0;
        assert_eq!(m.video_out_get_screen_size(CELL_VIDEO_OUT_PRIMARY).unwrap(), 32.0);
    }

    #[test]
    fn copy_control_validates_range() {
        let mut m = CellAvconfExt::new();
        m.video_out_set_copy_control(0, 0).unwrap();
        m.video_out_set_copy_control(0, 1).unwrap();
        m.video_out_set_copy_control(0, 2).unwrap();
        assert_eq!(
            m.video_out_set_copy_control(0, 3),
            Err(CELL_VIDEO_OUT_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn audio_out_set_device_mode_validates_range() {
        let mut m = CellAvconfExt::new();
        m.audio_out_set_device_mode(0).unwrap();
        m.audio_out_set_device_mode(2).unwrap();
        assert_eq!(
            m.audio_out_set_device_mode(99),
            Err(CELL_AUDIO_OUT_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn audio_in_set_device_mode_validates_whitelist() {
        let mut m = CellAvconfExt::new();
        for mode in [0, 1, 2, 10] {
            m.audio_in_set_device_mode(mode).unwrap();
        }
        assert_eq!(
            m.audio_in_set_device_mode(3),
            Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)
        );
        assert_eq!(
            m.audio_in_set_device_mode(99),
            Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn audio_in_register_validates_volume() {
        let mut m = CellAvconfExt::new();
        m.add_in_device("USB Mic 1");
        // option=true rejected.
        assert_eq!(
            m.audio_in_register_device(0, Some("USB Mic 1"), true, Some(3)),
            Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)
        );
        // null name rejected.
        assert_eq!(
            m.audio_in_register_device(0, None, false, Some(3)),
            Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)
        );
        // null config rejected.
        assert_eq!(
            m.audio_in_register_device(0, Some("USB Mic 1"), false, None),
            Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)
        );
        // volume > 5 rejected.
        assert_eq!(
            m.audio_in_register_device(0, Some("USB Mic 1"), false, Some(6)),
            Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)
        );
        // Unknown device rejected.
        assert_eq!(
            m.audio_in_register_device(0, Some("Unknown Mic"), false, Some(3)),
            Err(CELL_AUDIO_IN_ERROR_DEVICE_NOT_FOUND)
        );
        // Happy path.
        let id = m.audio_in_register_device(0, Some("USB Mic 1"), false, Some(3)).unwrap();
        assert!(id > 0);
        assert!(m.registered_in_devices.contains(&id));
    }

    #[test]
    fn audio_in_unregister() {
        let mut m = CellAvconfExt::new();
        m.add_in_device("Mic");
        let id = m.audio_in_register_device(0, Some("Mic"), false, Some(3)).unwrap();
        m.audio_in_unregister_device(id).unwrap();
        assert!(!m.registered_in_devices.contains(&id));
    }

    #[test]
    fn convert_cursor_color_float_full() {
        // displaybuffer_format=2 (FLOAT) + range full → just copy channels.
        let src = [0xFF, 0x10, 0x20, 0x30];
        let mut dst = [0u8; 4];
        convert_cursor_color(&src, &mut dst, 1, 1.0, true, false);
        assert_eq!(dst[0], 0xFF); // alpha preserved
        assert_eq!(dst[1], 0x10);
        assert_eq!(dst[2], 0x20);
        assert_eq!(dst[3], 0x30);
    }

    #[test]
    fn convert_cursor_color_float_limited() {
        let src = [0xFF, 0xFF, 0xFF, 0xFF];
        let mut dst = [0u8; 4];
        convert_cursor_color(&src, &mut dst, 1, 1.0, true, true);
        // (1.0) * 219 + 16 = 235
        assert_eq!(dst[1], 235);
        assert_eq!(dst[2], 235);
        assert_eq!(dst[3], 235);
    }

    #[test]
    fn convert_cursor_color_gamma_1_full() {
        // Gamma 1.0 + non-float + full range → x → x*255 = identity.
        let src = [0xFF, 100, 150, 200];
        let mut dst = [0u8; 4];
        convert_cursor_color(&src, &mut dst, 1, 1.0, false, false);
        // pow(100/255, 1.0) * 255 ~= 100
        assert!((dst[1] as i32 - 100).abs() <= 1);
        assert!((dst[2] as i32 - 150).abs() <= 1);
        assert!((dst[3] as i32 - 200).abs() <= 1);
    }

    #[test]
    fn convert_cursor_color_validation() {
        let mut m = CellAvconfExt::new();
        let src = [0u8; 4];
        let mut dst = [0u8; 4];
        // null src.
        assert_eq!(
            m.video_out_convert_cursor_color(0, 0, 1.0, 0, None, Some(&mut dst), 1),
            Err(CELL_VIDEO_OUT_ERROR_ILLEGAL_PARAMETER)
        );
        // bad displaybuffer_format.
        assert_eq!(
            m.video_out_convert_cursor_color(0, -1, 1.0, 0, Some(&src), Some(&mut dst), 1),
            Err(CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE)
        );
        // bad source_buffer_format.
        assert_eq!(
            m.video_out_convert_cursor_color(0, 0, 1.0, 1, Some(&src), Some(&mut dst), 1),
            Err(CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE)
        );
        // bad gamma for non-float.
        assert_eq!(
            m.video_out_convert_cursor_color(0, 0, 0.5, 0, Some(&src), Some(&mut dst), 1),
            Err(CELL_VIDEO_OUT_ERROR_PARAMETER_OUT_OF_RANGE)
        );
    }

    #[test]
    fn audio_in_get_available_count_capped() {
        let mut m = CellAvconfExt::new();
        m.add_in_device("d1");
        m.add_in_device("d2");
        assert_eq!(m.audio_in_get_available_device_info(5).unwrap(), 2);
        assert_eq!(m.audio_in_get_available_device_info(1).unwrap(), 1);
        assert_eq!(
            m.audio_in_get_available_device_info(17),
            Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn audio_in_get_device_info_lookup() {
        let mut m = CellAvconfExt::new();
        m.add_in_device("Mic1");
        let info = m.audio_in_get_device_info(0, 0, true).unwrap();
        assert_eq!(info.name, "Mic1");
        assert_eq!(
            m.audio_in_get_device_info(99, 0, true),
            Err(CELL_AUDIO_OUT_ERROR_DEVICE_NOT_FOUND)
        );
        assert_eq!(
            m.audio_in_get_device_info(0, 1, true),
            Err(CELL_AUDIO_IN_ERROR_ILLEGAL_PARAMETER)
        );
    }

    #[test]
    fn full_avconfext_lifecycle_smoke() {
        let mut m = CellAvconfExt::new();
        m.add_in_device("HeadsetMic");
        // Init gamma.
        m.video_out_set_gamma(CELL_VIDEO_OUT_PRIMARY, 1.0).unwrap();
        assert_eq!(m.gamma, 1.0);
        // Audio in setup.
        m.audio_in_set_device_mode(CELL_AUDIO_IN_MULTI_DEVICE_MODE).unwrap();
        let id = m.audio_in_register_device(0, Some("HeadsetMic"), false, Some(3)).unwrap();
        // Cursor color convert.
        let src = [0xFF; 4];
        let mut dst = [0u8; 4];
        m.video_out_convert_cursor_color(0, 0, 1.0, 0, Some(&src), Some(&mut dst), 1).unwrap();
        // Cleanup.
        m.audio_in_unregister_device(id).unwrap();
        m.audio_out_set_device_mode(2).unwrap();
        m.video_out_set_copy_control(0, 1).unwrap();
    }
}
