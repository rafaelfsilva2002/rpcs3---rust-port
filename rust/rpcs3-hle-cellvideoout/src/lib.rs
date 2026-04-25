//! `rpcs3-hle-cellvideoout` — display mode negotiation HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellVideoOut.cpp`. Right after
//! `cellGcmInit`, every game calls `cellVideoOutConfigure` (or
//! `GetResolution`) to negotiate the output resolution with the TV.
//! Our HLE tracks the active config + reports the TV capabilities
//! the emulator user has chosen.
//!
//! ## Entry points covered
//!
//! | HLE function                              | Rust wrapper                       |
//! |-------------------------------------------|------------------------------------|
//! | `cellVideoOutGetState`                    | [`cell_video_out_get_state`]       |
//! | `cellVideoOutGetResolution`               | [`cell_video_out_get_resolution`]  |
//! | `cellVideoOutConfigure`                   | [`cell_video_out_configure`]       |
//! | `cellVideoOutGetConfiguration`            | [`cell_video_out_get_configuration`]|
//! | `cellVideoOutGetDeviceInfo`               | [`cell_video_out_get_device_info`] |
//! | `cellVideoOutGetNumberOfDevice`           | [`cell_video_out_get_number_of_device`] |
//! | `cellVideoOutGetResolutionAvailability`   | [`cell_video_out_get_resolution_availability`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellVideoOut.h:8-16
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_IMPLEMENTED: CellError = CellError(0x8002_B220);
    pub const ILLEGAL_CONFIGURATION: CellError = CellError(0x8002_B221);
    pub const ILLEGAL_PARAMETER: CellError = CellError(0x8002_B222);
    pub const PARAMETER_OUT_OF_RANGE: CellError = CellError(0x8002_B223);
    pub const DEVICE_NOT_FOUND: CellError = CellError(0x8002_B224);
    pub const UNSUPPORTED_VIDEO_OUT: CellError = CellError(0x8002_B225);
    pub const UNSUPPORTED_DISPLAY_MODE: CellError = CellError(0x8002_B226);
    pub const CONDITION_BUSY: CellError = CellError(0x8002_B227);
    pub const VALUE_IS_NOT_SET: CellError = CellError(0x8002_B228);
}

// =====================================================================
// Display port IDs + resolution IDs
// =====================================================================

pub const PRIMARY: u32 = 0;
pub const SECONDARY: u32 = 1;

pub const RESOLUTION_UNDEFINED: u8 = 0;
pub const RESOLUTION_1080: u8 = 1;       // 1920×1080
pub const RESOLUTION_720: u8 = 2;        // 1280×720
pub const RESOLUTION_480: u8 = 4;        // 720×480
pub const RESOLUTION_576: u8 = 5;        // 720×576
pub const RESOLUTION_1600X1080: u8 = 0x0A;
pub const RESOLUTION_1440X1080: u8 = 0x0B;
pub const RESOLUTION_1280X1080: u8 = 0x0C;
pub const RESOLUTION_960X1080: u8 = 0x0D;

/// Scan modes.
pub const SCAN_MODE_INTERLACE: u8 = 0;
pub const SCAN_MODE_PROGRESSIVE: u8 = 1;

/// Aspect ratios.
pub const ASPECT_AUTO: u8 = 0;
pub const ASPECT_4_3: u8 = 1;
pub const ASPECT_16_9: u8 = 2;

/// Color formats (pixel format enum, X8R8G8B8 is the standard).
pub const COLOR_FORMAT_X8R8G8B8: u8 = 0;
pub const COLOR_FORMAT_X8B8G8R8: u8 = 1;
pub const COLOR_FORMAT_R16G16B16X16_FLOAT: u8 = 2;

pub const MAX_BUFFER_COUNT: u32 = 8;

// =====================================================================
// Resolution table
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub id: u8,
    pub width: u32,
    pub height: u32,
}

pub const fn resolution_for_id(id: u8) -> Option<Resolution> {
    Some(match id {
        RESOLUTION_1080 => Resolution { id, width: 1920, height: 1080 },
        RESOLUTION_720 => Resolution { id, width: 1280, height: 720 },
        RESOLUTION_480 => Resolution { id, width: 720, height: 480 },
        RESOLUTION_576 => Resolution { id, width: 720, height: 576 },
        RESOLUTION_1600X1080 => Resolution { id, width: 1600, height: 1080 },
        RESOLUTION_1440X1080 => Resolution { id, width: 1440, height: 1080 },
        RESOLUTION_1280X1080 => Resolution { id, width: 1280, height: 1080 },
        RESOLUTION_960X1080 => Resolution { id, width: 960, height: 1080 },
        _ => return None,
    })
}

// =====================================================================
// Device + state
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoOutConfiguration {
    pub resolution_id: u8,
    pub format: u8,
    pub aspect: u8,
    pub pitch: u32,
}

impl Default for VideoOutConfiguration {
    fn default() -> Self {
        Self {
            resolution_id: RESOLUTION_720,
            format: COLOR_FORMAT_X8R8G8B8,
            aspect: ASPECT_16_9,
            pitch: 1280 * 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoOutState {
    Enabled,
    Disabled,
    DeepSleep,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoOutInfo {
    pub state: VideoOutState,
    pub color_space: u8,
    pub display_mode_count: u8,
    pub resolution_id: u8,
    pub scan_mode: u8,
    pub aspect: u8,
}

#[derive(Debug, Clone)]
pub struct VideoOutManager {
    pub configs: [VideoOutConfiguration; 2],
    pub states: [VideoOutState; 2],
    pub supported_resolutions: Vec<u8>,
}

impl Default for VideoOutManager {
    fn default() -> Self {
        Self {
            configs: [VideoOutConfiguration::default(), VideoOutConfiguration::default()],
            states: [VideoOutState::Enabled, VideoOutState::Disabled],
            supported_resolutions: vec![
                RESOLUTION_720,
                RESOLUTION_1080,
                RESOLUTION_480,
                RESOLUTION_576,
            ],
        }
    }
}

// =====================================================================
// Syscalls
// =====================================================================

fn check_port(port: u32) -> Result<(), CellError> {
    if port == PRIMARY || port == SECONDARY {
        Ok(())
    } else {
        Err(errors::PARAMETER_OUT_OF_RANGE)
    }
}

/// `cellVideoOutGetNumberOfDevice(videoOut)` — always 1 for primary,
/// 0 for secondary (PS3 ships with one HDMI + one AV MULTI, but
/// secondary is considered optional hardware).
#[must_use]
pub fn cell_video_out_get_number_of_device(port: u32) -> Result<i32, CellError> {
    check_port(port)?;
    Ok(if port == PRIMARY { 1 } else { 0 })
}

/// `cellVideoOutGetState(videoOut, deviceIndex, state)`.
#[must_use]
pub fn cell_video_out_get_state(
    m: &VideoOutManager,
    port: u32,
    device_index: u32,
) -> Result<VideoOutInfo, CellError> {
    check_port(port)?;
    if device_index != 0 {
        return Err(errors::DEVICE_NOT_FOUND);
    }
    let state = m.states[port as usize];
    let cfg = m.configs[port as usize];
    Ok(VideoOutInfo {
        state,
        color_space: 1,
        display_mode_count: m.supported_resolutions.len() as u8,
        resolution_id: cfg.resolution_id,
        scan_mode: SCAN_MODE_PROGRESSIVE,
        aspect: cfg.aspect,
    })
}

/// `cellVideoOutGetResolution(resolutionId, resolution_out)`.
#[must_use]
pub fn cell_video_out_get_resolution(resolution_id: u8) -> Result<Resolution, CellError> {
    resolution_for_id(resolution_id).ok_or(errors::UNSUPPORTED_DISPLAY_MODE)
}

/// `cellVideoOutConfigure(videoOut, config, option, waitForEvent)`.
#[must_use]
pub fn cell_video_out_configure(
    m: &mut VideoOutManager,
    port: u32,
    config: VideoOutConfiguration,
) -> Result<(), CellError> {
    check_port(port)?;

    if !m.supported_resolutions.contains(&config.resolution_id) {
        return Err(errors::UNSUPPORTED_DISPLAY_MODE);
    }
    match config.format {
        COLOR_FORMAT_X8R8G8B8
        | COLOR_FORMAT_X8B8G8R8
        | COLOR_FORMAT_R16G16B16X16_FLOAT => {}
        _ => return Err(errors::ILLEGAL_PARAMETER),
    }
    match config.aspect {
        ASPECT_AUTO | ASPECT_4_3 | ASPECT_16_9 => {}
        _ => return Err(errors::ILLEGAL_PARAMETER),
    }

    // Validate pitch: must be >= resolution width * 4 (4 bpp) and
    // multiple of 64 for tiled frame buffers.
    let res = resolution_for_id(config.resolution_id).ok_or(errors::UNSUPPORTED_DISPLAY_MODE)?;
    let bpp = match config.format {
        COLOR_FORMAT_R16G16B16X16_FLOAT => 8,
        _ => 4,
    };
    let min_pitch = res.width * bpp;
    if config.pitch < min_pitch {
        return Err(errors::ILLEGAL_PARAMETER);
    }
    if config.pitch % 64 != 0 {
        return Err(errors::ILLEGAL_PARAMETER);
    }

    m.configs[port as usize] = config;
    Ok(())
}

/// `cellVideoOutGetConfiguration(videoOut, config_out, option)`.
#[must_use]
pub fn cell_video_out_get_configuration(
    m: &VideoOutManager,
    port: u32,
) -> Result<VideoOutConfiguration, CellError> {
    check_port(port)?;
    Ok(m.configs[port as usize])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceInfo {
    pub port_type: u8,
    pub color_space: u8,
    pub latency: u16,
    pub resolution_count: u8,
}

#[must_use]
pub fn cell_video_out_get_device_info(
    m: &VideoOutManager,
    port: u32,
) -> Result<DeviceInfo, CellError> {
    check_port(port)?;
    if m.states[port as usize] == VideoOutState::Disabled {
        return Err(errors::DEVICE_NOT_FOUND);
    }
    Ok(DeviceInfo {
        port_type: 0,   // HDMI
        color_space: 1,
        latency: 0,
        resolution_count: m.supported_resolutions.len() as u8,
    })
}

/// `cellVideoOutGetResolutionAvailability(videoOut, resolutionId, aspect, option)`.
/// Returns 1 if the combination is available, 0 otherwise.
#[must_use]
pub fn cell_video_out_get_resolution_availability(
    m: &VideoOutManager,
    port: u32,
    resolution_id: u8,
    aspect: u8,
) -> Result<i32, CellError> {
    check_port(port)?;
    if !matches!(aspect, ASPECT_AUTO | ASPECT_4_3 | ASPECT_16_9) {
        return Err(errors::ILLEGAL_PARAMETER);
    }
    Ok(if m.supported_resolutions.contains(&resolution_id) { 1 } else { 0 })
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr() -> VideoOutManager { VideoOutManager::default() }

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::NOT_IMPLEMENTED.0, 0x8002_B220);
        assert_eq!(errors::ILLEGAL_PARAMETER.0, 0x8002_B222);
        assert_eq!(errors::DEVICE_NOT_FOUND.0, 0x8002_B224);
        assert_eq!(errors::UNSUPPORTED_DISPLAY_MODE.0, 0x8002_B226);
        assert_eq!(errors::CONDITION_BUSY.0, 0x8002_B227);
    }

    #[test]
    fn resolution_1080_is_1920x1080() {
        let r = resolution_for_id(RESOLUTION_1080).unwrap();
        assert_eq!(r.width, 1920);
        assert_eq!(r.height, 1080);
    }

    #[test]
    fn resolution_720_is_1280x720() {
        let r = resolution_for_id(RESOLUTION_720).unwrap();
        assert_eq!(r.width, 1280);
        assert_eq!(r.height, 720);
    }

    #[test]
    fn resolution_unknown_id_returns_none() {
        assert!(resolution_for_id(0xFF).is_none());
    }

    // --- primary / secondary port handling ------------------------

    #[test]
    fn number_of_devices_primary_is_one() {
        assert_eq!(cell_video_out_get_number_of_device(PRIMARY).unwrap(), 1);
    }

    #[test]
    fn number_of_devices_secondary_is_zero() {
        assert_eq!(cell_video_out_get_number_of_device(SECONDARY).unwrap(), 0);
    }

    #[test]
    fn bad_port_out_of_range() {
        assert_eq!(
            cell_video_out_get_number_of_device(99).unwrap_err(),
            errors::PARAMETER_OUT_OF_RANGE,
        );
    }

    // --- state ----------------------------------------------------

    #[test]
    fn primary_starts_enabled() {
        let m = mgr();
        let info = cell_video_out_get_state(&m, PRIMARY, 0).unwrap();
        assert_eq!(info.state, VideoOutState::Enabled);
    }

    #[test]
    fn secondary_starts_disabled() {
        let m = mgr();
        let info = cell_video_out_get_state(&m, SECONDARY, 0).unwrap();
        assert_eq!(info.state, VideoOutState::Disabled);
    }

    #[test]
    fn state_device_index_nonzero_is_not_found() {
        let m = mgr();
        assert_eq!(
            cell_video_out_get_state(&m, PRIMARY, 1).unwrap_err(),
            errors::DEVICE_NOT_FOUND,
        );
    }

    // --- configure ------------------------------------------------

    #[test]
    fn configure_1080_progressive_16_9() {
        let mut m = mgr();
        let cfg = VideoOutConfiguration {
            resolution_id: RESOLUTION_1080,
            format: COLOR_FORMAT_X8R8G8B8,
            aspect: ASPECT_16_9,
            pitch: 1920 * 4,
        };
        cell_video_out_configure(&mut m, PRIMARY, cfg).unwrap();
        let stored = cell_video_out_get_configuration(&m, PRIMARY).unwrap();
        assert_eq!(stored.resolution_id, RESOLUTION_1080);
    }

    #[test]
    fn configure_unsupported_resolution_errors() {
        let mut m = mgr();
        let cfg = VideoOutConfiguration {
            resolution_id: 0xFF,
            ..VideoOutConfiguration::default()
        };
        assert_eq!(
            cell_video_out_configure(&mut m, PRIMARY, cfg).unwrap_err(),
            errors::UNSUPPORTED_DISPLAY_MODE,
        );
    }

    #[test]
    fn configure_pitch_below_min_errors() {
        let mut m = mgr();
        let cfg = VideoOutConfiguration {
            resolution_id: RESOLUTION_1080,
            pitch: 100,
            ..VideoOutConfiguration::default()
        };
        assert_eq!(
            cell_video_out_configure(&mut m, PRIMARY, cfg).unwrap_err(),
            errors::ILLEGAL_PARAMETER,
        );
    }

    #[test]
    fn configure_pitch_not_multiple_of_64_errors() {
        let mut m = mgr();
        let cfg = VideoOutConfiguration {
            resolution_id: RESOLUTION_720,
            pitch: 1280 * 4 + 1,
            ..VideoOutConfiguration::default()
        };
        assert_eq!(
            cell_video_out_configure(&mut m, PRIMARY, cfg).unwrap_err(),
            errors::ILLEGAL_PARAMETER,
        );
    }

    #[test]
    fn configure_bad_format_errors() {
        let mut m = mgr();
        let cfg = VideoOutConfiguration {
            format: 99,
            ..VideoOutConfiguration::default()
        };
        assert_eq!(
            cell_video_out_configure(&mut m, PRIMARY, cfg).unwrap_err(),
            errors::ILLEGAL_PARAMETER,
        );
    }

    #[test]
    fn configure_bad_aspect_errors() {
        let mut m = mgr();
        let cfg = VideoOutConfiguration {
            aspect: 99,
            ..VideoOutConfiguration::default()
        };
        assert_eq!(
            cell_video_out_configure(&mut m, PRIMARY, cfg).unwrap_err(),
            errors::ILLEGAL_PARAMETER,
        );
    }

    #[test]
    fn configure_fp16_uses_8_bpp_pitch_check() {
        // R16G16B16X16_FLOAT is 8 bytes per pixel, so pitch must be
        // at least width * 8.
        let mut m = mgr();
        let cfg = VideoOutConfiguration {
            resolution_id: RESOLUTION_720,
            format: COLOR_FORMAT_R16G16B16X16_FLOAT,
            aspect: ASPECT_16_9,
            pitch: 1280 * 4,  // too small for FP16
        };
        assert_eq!(
            cell_video_out_configure(&mut m, PRIMARY, cfg).unwrap_err(),
            errors::ILLEGAL_PARAMETER,
        );

        let cfg = VideoOutConfiguration { pitch: 1280 * 8, ..cfg };
        cell_video_out_configure(&mut m, PRIMARY, cfg).unwrap();
    }

    // --- availability ---------------------------------------------

    #[test]
    fn resolution_availability_supported_combo_is_1() {
        let m = mgr();
        assert_eq!(
            cell_video_out_get_resolution_availability(&m, PRIMARY, RESOLUTION_1080, ASPECT_16_9).unwrap(),
            1,
        );
    }

    #[test]
    fn resolution_availability_unsupported_is_0() {
        let m = mgr();
        assert_eq!(
            cell_video_out_get_resolution_availability(&m, PRIMARY, RESOLUTION_1280X1080, ASPECT_16_9).unwrap(),
            0,
        );
    }

    #[test]
    fn resolution_availability_bad_aspect_errors() {
        let m = mgr();
        assert_eq!(
            cell_video_out_get_resolution_availability(&m, PRIMARY, RESOLUTION_720, 99).unwrap_err(),
            errors::ILLEGAL_PARAMETER,
        );
    }

    // --- device info ----------------------------------------------

    #[test]
    fn device_info_primary_returns_hdmi() {
        let m = mgr();
        let info = cell_video_out_get_device_info(&m, PRIMARY).unwrap();
        assert_eq!(info.port_type, 0);
        assert_eq!(info.resolution_count, 4);
    }

    #[test]
    fn device_info_secondary_is_device_not_found() {
        let m = mgr();
        assert_eq!(
            cell_video_out_get_device_info(&m, SECONDARY).unwrap_err(),
            errors::DEVICE_NOT_FOUND,
        );
    }

    #[test]
    fn cell_video_out_get_resolution_rejects_unknown_id() {
        assert_eq!(
            cell_video_out_get_resolution(0xFF).unwrap_err(),
            errors::UNSUPPORTED_DISPLAY_MODE,
        );
    }
}
