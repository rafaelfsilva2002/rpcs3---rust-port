//! `rpcs3-hle-cellgem` — PlayStation Move tracking HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellGem.cpp`. The API manages up to 4
//! Move controllers tracked by the PlayStation Eye camera:
//!
//! 1. `Init(attribute)` — allocate resources, up to `max_connect` pads.
//! 2. `Prepare{Camera,VideoConvert}` — configure exposure / output format.
//! 3. `TrackHues(hues[])` — assign an RGB hue to each controller.
//! 4. `Update{Start,Finish}` — consume a camera frame, refresh pose.
//! 5. `GetState/GetInertialState/GetImageState/GetStatusFlags` — query.
//! 6. `SetRumble/EnableMagnetometer/InvalidateCalibration` — actuation.
//! 7. `End` — teardown.
//!
//! ## Entry points covered
//!
//! | HLE function                           | Rust wrapper                          |
//! |----------------------------------------|---------------------------------------|
//! | `cellGemInit`                          | [`GemManager::init`]                  |
//! | `cellGemEnd`                           | [`GemManager::end`]                   |
//! | `cellGemGetInfo`                       | [`GemManager::info`]                  |
//! | `cellGemGetMemorySize`                 | [`GemManager::required_memory`]       |
//! | `cellGemPrepareCamera`                 | [`GemManager::prepare_camera`]        |
//! | `cellGemPrepareVideoConvert`           | [`GemManager::prepare_video_convert`] |
//! | `cellGemUpdateStart`/`UpdateFinish`    | [`GemManager::update_start`]/`finish` |
//! | `cellGemGetState`                      | [`GemManager::get_state`]             |
//! | `cellGemGetInertialState`              | [`GemManager::get_inertial_state`]    |
//! | `cellGemTrackHues`                     | [`GemManager::track_hues`]            |
//! | `cellGemSetRumble`                     | [`GemManager::set_rumble`]            |
//! | `cellGemEnableMagnetometer`            | [`GemManager::enable_magnetometer`]   |
//! | `cellGemInvalidateCalibration`         | [`GemManager::invalidate_calibration`]|
//! | `cellGemGetStatusFlags`                | [`GemManager::status_flags`]          |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellGem.h:8-21
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const RESOURCE_ALLOCATION_FAILED: CellError = CellError(0x8012_1801);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8012_1802);
    pub const UNINITIALIZED: CellError = CellError(0x8012_1803);
    pub const INVALID_PARAMETER: CellError = CellError(0x8012_1804);
    pub const INVALID_ALIGNMENT: CellError = CellError(0x8012_1805);
    pub const UPDATE_NOT_FINISHED: CellError = CellError(0x8012_1806);
    pub const UPDATE_NOT_STARTED: CellError = CellError(0x8012_1807);
    pub const CONVERT_NOT_FINISHED: CellError = CellError(0x8012_1808);
    pub const CONVERT_NOT_STARTED: CellError = CellError(0x8012_1809);
    pub const WRITE_NOT_FINISHED: CellError = CellError(0x8012_180A);
    pub const NOT_A_HUE: CellError = CellError(0x8012_180B);
}

// =====================================================================
// Runtime statuses / constants (byte-exact with cellGem.h:24-166)
// =====================================================================

pub const SPHERE_RADIUS_MM: f32 = 22.5;

// Runtime statuses — positive values returned by query APIs.
pub const NOT_CONNECTED: u32 = 1;
pub const SPHERE_NOT_CALIBRATED: u32 = 2;
pub const SPHERE_CALIBRATING: u32 = 3;
pub const COMPUTING_AVAILABLE_COLORS: u32 = 4;
pub const HUE_NOT_SET: u32 = 5;
pub const NO_VIDEO: u32 = 6;
pub const TIME_OUT_OF_RANGE: u32 = 7;
pub const NOT_CALIBRATED: u32 = 8;
pub const NO_EXTERNAL_PORT_DEVICE: u32 = 9;

// Info status flags.
pub const STATUS_DISCONNECTED: u32 = 0;
pub const STATUS_READY: u32 = 1;

// Digital button bits.
pub const CTRL_SELECT: u16 = 1 << 0;
pub const CTRL_T: u16 = 1 << 1;
pub const CTRL_MOVE: u16 = 1 << 2;
pub const CTRL_START: u16 = 1 << 3;
pub const CTRL_TRIANGLE: u16 = 1 << 4;
pub const CTRL_CIRCLE: u16 = 1 << 5;
pub const CTRL_CROSS: u16 = 1 << 6;
pub const CTRL_SQUARE: u16 = 1 << 7;

// Extension port status bits.
pub const EXT_CONNECTED: u32 = 1 << 0;
pub const EXT_EXT0: u32 = 1 << 1;
pub const EXT_EXT1: u32 = 1 << 2;

// Extension port sizes.
pub const EXTERNAL_PORT_DEVICE_INFO_SIZE: u32 = 38;
pub const EXTERNAL_PORT_OUTPUT_SIZE: u32 = 40;

// Camera exposure limits.
pub const MIN_CAMERA_EXPOSURE: u32 = 40;
pub const MAX_CAMERA_EXPOSURE: u32 = 511;

// GetState / GetInertialState time flags.
pub const STATE_FLAG_CURRENT_TIME: i32 = 0;
pub const STATE_FLAG_LATEST_IMAGE_TIME: i32 = 1;
pub const STATE_FLAG_TIMESTAMP: i32 = 2;

pub const INERTIAL_STATE_FLAG_LATEST: i32 = 0;
pub const INERTIAL_STATE_FLAG_PREVIOUS: i32 = 1;
pub const INERTIAL_STATE_FLAG_NEXT: i32 = 2;

// Hue sentinel values (high byte indicates sentinel).
pub const DONT_TRACK_HUE: u32 = 2 << 24;
pub const DONT_CARE_HUE: u32 = 4 << 24;
pub const DONT_CHANGE_HUE: u32 = 8 << 24;

// Status flag bit masks.
pub const FLAG_CALIBRATION_OCCURRED: u64 = 1 << 0;
pub const FLAG_CALIBRATION_SUCCEEDED: u64 = 1 << 1;
pub const FLAG_CALIBRATION_FAILED_CANT_FIND_SPHERE: u64 = 1 << 2;
pub const FLAG_CALIBRATION_FAILED_MOTION_DETECTED: u64 = 1 << 3;
pub const FLAG_CALIBRATION_FAILED_BRIGHT_LIGHTING: u64 = 1 << 4;
pub const FLAG_LIGHTING_CHANGED: u64 = 1 << 7;
pub const FLAG_WRONG_FOV: u64 = 1 << 8;
pub const ALL_FLAGS: u64 = 0xffff_ffff_ffff_ffff;

// Tracking flags.
pub const TRACKING_FLAG_POSITION_TRACKED: u32 = 1 << 0;
pub const TRACKING_FLAG_VISIBLE: u32 = 1 << 1;

// General limits.
pub const LATENCY_OFFSET: i32 = -22000;
pub const MAX_NUM: u32 = 4;
pub const VERSION: u32 = 2;

// Video conversion flags.
pub const AUTO_WHITE_BALANCE: u32 = 0x1;
pub const GAMMA_BOOST: u32 = 0x2;
pub const COMBINE_PREVIOUS_INPUT_FRAME: u32 = 0x4;
pub const FILTER_OUTLIER_PIXELS: u32 = 0x8;

// Video conversion output formats.
pub const NO_VIDEO_OUTPUT: i32 = 1;
pub const RGBA_640X480: i32 = 2;
pub const YUV_640X480: i32 = 3;
pub const YUV422_640X480: i32 = 4;
pub const YUV411_640X480: i32 = 5;
pub const RGBA_320X240: i32 = 6;
pub const BAYER_RESTORED: i32 = 7;
pub const BAYER_RESTORED_RGGB: i32 = 8;
pub const BAYER_RESTORED_RASTERIZED: i32 = 9;

#[must_use]
pub fn is_known_video_format(fmt: i32) -> bool {
    (NO_VIDEO_OUTPUT..=BAYER_RESTORED_RASTERIZED).contains(&fmt)
}

// External device IDs.
pub const SHARP_SHOOTER_DEVICE_ID: u32 = 0x8081;
pub const RACING_WHEEL_DEVICE_ID: u32 = 0x8101;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GemAttribute {
    pub version: u32,
    pub max_connect: u32,
    pub memory_ptr: u32,
    pub spurs_addr: u32,
    pub spu_priorities: [u8; 8],
}

impl GemAttribute {
    fn validate(&self) -> Result<(), CellError> {
        if self.version != VERSION {
            return Err(errors::INVALID_PARAMETER);
        }
        if self.max_connect == 0 || self.max_connect > MAX_NUM {
            return Err(errors::INVALID_PARAMETER);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct GemState {
    pub pos: Vec3,
    pub vel: Vec3,
    pub accel: Vec3,
    pub quat: Quat,
    pub angular_velocity: Vec3,
    pub tracking_flags: u32,
    pub timestamp: u64,
    pub buttons: u16,
    pub analog_t: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct GemInertialState {
    pub accel: Vec3,
    pub gyro: Vec3,
    pub temperature: f32,
    pub timestamp: u64,
    pub buttons: u16,
    pub analog_t: u8,
    pub counter: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct GemImageState {
    pub frame_timestamp: u64,
    pub timestamp: u64,
    pub visible: bool,
    pub r: f32,  // sphere radius estimate in pixels
    pub u: f32,  // image-plane u
    pub v: f32,  // image-plane v
    pub projectionx: f32,
    pub projectiony: f32,
    pub distance: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GemInfo {
    pub max_connect: u32,
    pub connected: u32,
    pub info: [u32; MAX_NUM as usize],     // STATUS_* per controller
    pub status: [u32; MAX_NUM as usize],   // runtime status codes
    pub ext_status: [u32; MAX_NUM as usize],
    pub ext_id: [u32; MAX_NUM as usize],
    pub port_status: [u32; MAX_NUM as usize],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VideoConvertAttribute {
    pub format: i32,
    pub flags: u32,
    pub gain: u32,
    pub red_gain: u32,
    pub green_gain: u32,
    pub blue_gain: u32,
    pub buffer_memory: u32,
    pub video_data_out: u32,
}

// =====================================================================
// GemManager
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateState {
    Idle,
    Started,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConvertState {
    Idle,
    Started,
}

#[derive(Clone, Debug)]
pub struct GemController {
    pub connected: bool,
    pub calibrated: bool,
    pub magnetometer_enabled: bool,
    pub hue: u32,
    pub rumble: u8,
    pub state: GemState,
    pub inertial: GemInertialState,
    pub image: GemImageState,
    pub ext_status: u32,
    pub ext_id: u32,
}

impl Default for GemController {
    fn default() -> Self {
        Self {
            connected: false,
            calibrated: false,
            magnetometer_enabled: false,
            hue: DONT_CARE_HUE,
            rumble: 0,
            state: GemState::default(),
            inertial: GemInertialState::default(),
            image: GemImageState::default(),
            ext_status: 0,
            ext_id: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GemManager {
    initialized: bool,
    max_connect: u32,
    attr: Option<GemAttribute>,
    camera_prepared: bool,
    camera_exposure: u32,
    camera_gain: f32,
    convert: ConvertState,
    convert_attr: VideoConvertAttribute,
    update: UpdateState,
    controllers: [GemController; MAX_NUM as usize],
    status_flags: u64,
}

impl GemManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            initialized: false,
            max_connect: 0,
            attr: None,
            camera_prepared: false,
            camera_exposure: 0,
            camera_gain: 1.0,
            convert: ConvertState::Idle,
            convert_attr: VideoConvertAttribute::default(),
            update: UpdateState::Idle,
            controllers: core::array::from_fn(|_| GemController::default()),
            status_flags: 0,
        }
    }

    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// `cellGemGetMemorySize(max_connect)`. Must match C++ constant
    /// formula: 8 * max_connect * MB (the real lib reserves some extra
    /// buffers, but games only check the value is non-zero and aligned).
    pub fn required_memory(max_connect: u32) -> Result<u32, CellError> {
        if max_connect == 0 || max_connect > MAX_NUM {
            return Err(errors::INVALID_PARAMETER);
        }
        Ok(max_connect * 8 * 1024 * 1024)
    }

    // ----------------- Lifecycle -----------------

    pub fn init(&mut self, attr: GemAttribute) -> Result<(), CellError> {
        if self.initialized {
            return Err(errors::ALREADY_INITIALIZED);
        }
        attr.validate()?;
        self.max_connect = attr.max_connect;
        self.attr = Some(attr);
        self.initialized = true;
        self.camera_prepared = false;
        self.convert = ConvertState::Idle;
        self.update = UpdateState::Idle;
        self.status_flags = 0;
        for c in &mut self.controllers {
            *c = GemController::default();
        }
        Ok(())
    }

    pub fn end(&mut self) -> Result<(), CellError> {
        if !self.initialized {
            return Err(errors::UNINITIALIZED);
        }
        self.initialized = false;
        self.max_connect = 0;
        self.attr = None;
        self.camera_prepared = false;
        self.convert = ConvertState::Idle;
        self.update = UpdateState::Idle;
        Ok(())
    }

    // ----------------- Camera / video convert -----------------

    pub fn prepare_camera(&mut self, max_exposure: u32, image_quality: f32) -> Result<(), CellError> {
        self.require_init()?;
        if !(MIN_CAMERA_EXPOSURE..=MAX_CAMERA_EXPOSURE).contains(&max_exposure) {
            return Err(errors::INVALID_PARAMETER);
        }
        if !(0.0..=1.0).contains(&image_quality) {
            return Err(errors::INVALID_PARAMETER);
        }
        self.camera_exposure = max_exposure;
        self.camera_gain = image_quality;
        self.camera_prepared = true;
        Ok(())
    }

    pub fn prepare_video_convert(&mut self, cfg: VideoConvertAttribute) -> Result<(), CellError> {
        self.require_init()?;
        if self.convert == ConvertState::Started {
            return Err(errors::CONVERT_NOT_FINISHED);
        }
        if !is_known_video_format(cfg.format) {
            return Err(errors::INVALID_PARAMETER);
        }
        // 16-byte alignment rule for video buffers.
        if cfg.buffer_memory != 0 && cfg.buffer_memory % 16 != 0 {
            return Err(errors::INVALID_ALIGNMENT);
        }
        if cfg.video_data_out != 0 && cfg.video_data_out % 16 != 0 {
            return Err(errors::INVALID_ALIGNMENT);
        }
        self.convert_attr = cfg;
        self.convert = ConvertState::Started;
        Ok(())
    }

    pub fn finish_video_convert(&mut self) -> Result<(), CellError> {
        if self.convert != ConvertState::Started {
            return Err(errors::CONVERT_NOT_STARTED);
        }
        self.convert = ConvertState::Idle;
        Ok(())
    }

    // ----------------- Update loop -----------------

    pub fn update_start(&mut self, camera_frame_timestamp: u64) -> Result<(), CellError> {
        self.require_init()?;
        if !self.camera_prepared {
            return Err(errors::UPDATE_NOT_STARTED);
        }
        if self.update == UpdateState::Started {
            return Err(errors::UPDATE_NOT_FINISHED);
        }
        for c in &mut self.controllers {
            c.image.frame_timestamp = camera_frame_timestamp;
        }
        self.update = UpdateState::Started;
        Ok(())
    }

    pub fn update_finish(&mut self) -> Result<u32, CellError> {
        if self.update != UpdateState::Started {
            return Err(errors::UPDATE_NOT_STARTED);
        }
        self.update = UpdateState::Idle;
        Ok(self.controllers.iter().filter(|c| c.connected).count() as u32)
    }

    // ----------------- Controller state -----------------

    pub fn info(&self) -> Result<GemInfo, CellError> {
        self.require_init()?;
        let mut info = [0u32; MAX_NUM as usize];
        let mut status = [0u32; MAX_NUM as usize];
        let mut ext_status = [0u32; MAX_NUM as usize];
        let mut ext_id = [0u32; MAX_NUM as usize];
        let mut port_status = [0u32; MAX_NUM as usize];
        let mut connected = 0u32;
        for (i, c) in self.controllers.iter().enumerate() {
            if c.connected {
                info[i] = STATUS_READY;
                connected += 1;
                port_status[i] = EXT_CONNECTED;
                ext_status[i] = c.ext_status;
                ext_id[i] = c.ext_id;
            } else {
                info[i] = STATUS_DISCONNECTED;
            }
            status[i] = if c.connected {
                if c.calibrated { STATUS_READY } else { NOT_CALIBRATED }
            } else {
                NOT_CONNECTED
            };
        }
        Ok(GemInfo { max_connect: self.max_connect, connected, info, status, ext_status, ext_id, port_status })
    }

    pub fn get_state(&self, controller: u32, _flag: i32) -> Result<GemState, CellError> {
        self.require_init()?;
        let c = self.controller(controller)?;
        if !c.connected {
            return Err(errors::INVALID_PARAMETER);
        }
        Ok(c.state)
    }

    pub fn get_inertial_state(&self, controller: u32, _flag: i32) -> Result<GemInertialState, CellError> {
        self.require_init()?;
        let c = self.controller(controller)?;
        if !c.connected {
            return Err(errors::INVALID_PARAMETER);
        }
        Ok(c.inertial)
    }

    pub fn get_image_state(&self, controller: u32) -> Result<GemImageState, CellError> {
        self.require_init()?;
        let c = self.controller(controller)?;
        Ok(c.image)
    }

    // ----------------- Hues / rumble / calibration -----------------

    pub fn track_hues(&mut self, hues: &[u32]) -> Result<(), CellError> {
        self.require_init()?;
        if hues.len() > self.max_connect as usize {
            return Err(errors::INVALID_PARAMETER);
        }
        for (i, &hue) in hues.iter().enumerate() {
            // Sentinels are valid; any real hue must fit in 0..=0xFFFFFF (RGB).
            if !matches!(hue, DONT_TRACK_HUE | DONT_CARE_HUE | DONT_CHANGE_HUE) && hue > 0x00FF_FFFF {
                return Err(errors::NOT_A_HUE);
            }
            if hue != DONT_CHANGE_HUE {
                self.controllers[i].hue = hue;
            }
        }
        Ok(())
    }

    pub fn set_rumble(&mut self, controller: u32, intensity: u8) -> Result<(), CellError> {
        self.require_init()?;
        let idx = controller as usize;
        if idx >= self.max_connect as usize {
            return Err(errors::INVALID_PARAMETER);
        }
        self.controllers[idx].rumble = intensity;
        Ok(())
    }

    pub fn enable_magnetometer(&mut self, controller: u32, enable: bool) -> Result<(), CellError> {
        self.require_init()?;
        let idx = controller as usize;
        if idx >= self.max_connect as usize {
            return Err(errors::INVALID_PARAMETER);
        }
        self.controllers[idx].magnetometer_enabled = enable;
        Ok(())
    }

    pub fn invalidate_calibration(&mut self, controller: u32) -> Result<(), CellError> {
        self.require_init()?;
        let idx = controller as usize;
        if idx >= self.max_connect as usize {
            return Err(errors::INVALID_PARAMETER);
        }
        self.controllers[idx].calibrated = false;
        Ok(())
    }

    pub fn status_flags(&self, controller: u32, mask: u64) -> Result<u64, CellError> {
        self.require_init()?;
        let _c = self.controller(controller)?;
        Ok(self.status_flags & mask)
    }

    pub fn clear_status_flags(&mut self, controller: u32, mask: u64) -> Result<(), CellError> {
        self.require_init()?;
        let idx = controller as usize;
        if idx >= self.max_connect as usize {
            return Err(errors::INVALID_PARAMETER);
        }
        self.status_flags &= !mask;
        Ok(())
    }

    // ----------------- Test hooks (reference backend) -----------------

    /// Inject a controller state — the real lib gets this from the camera
    /// and IMU; tests drive it directly.
    pub fn inject_controller(&mut self, idx: u32, controller: GemController) -> Result<(), CellError> {
        self.require_init()?;
        let idx = idx as usize;
        if idx >= self.max_connect as usize {
            return Err(errors::INVALID_PARAMETER);
        }
        self.controllers[idx] = controller;
        Ok(())
    }

    pub fn inject_status_flags(&mut self, flags: u64) -> Result<(), CellError> {
        self.require_init()?;
        self.status_flags |= flags;
        Ok(())
    }

    // ----------------- Helpers -----------------

    fn require_init(&self) -> Result<(), CellError> {
        if self.initialized { Ok(()) } else { Err(errors::UNINITIALIZED) }
    }

    fn controller(&self, idx: u32) -> Result<&GemController, CellError> {
        let idx = idx as usize;
        if idx >= self.max_connect as usize {
            return Err(errors::INVALID_PARAMETER);
        }
        Ok(&self.controllers[idx])
    }
}

impl Default for GemManager {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_attribute(max_connect: u32) -> GemAttribute {
        GemAttribute {
            version: VERSION,
            max_connect,
            memory_ptr: 0,
            spurs_addr: 0,
            spu_priorities: [0; 8],
        }
    }

    fn initialized_manager(max_connect: u32) -> GemManager {
        let mut m = GemManager::new();
        m.init(ok_attribute(max_connect)).unwrap();
        m
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::RESOURCE_ALLOCATION_FAILED.0, 0x8012_1801);
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8012_1802);
        assert_eq!(errors::UNINITIALIZED.0, 0x8012_1803);
        assert_eq!(errors::INVALID_PARAMETER.0, 0x8012_1804);
        assert_eq!(errors::INVALID_ALIGNMENT.0, 0x8012_1805);
        assert_eq!(errors::UPDATE_NOT_FINISHED.0, 0x8012_1806);
        assert_eq!(errors::UPDATE_NOT_STARTED.0, 0x8012_1807);
        assert_eq!(errors::CONVERT_NOT_FINISHED.0, 0x8012_1808);
        assert_eq!(errors::CONVERT_NOT_STARTED.0, 0x8012_1809);
        assert_eq!(errors::WRITE_NOT_FINISHED.0, 0x8012_180A);
        assert_eq!(errors::NOT_A_HUE.0, 0x8012_180B);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(MAX_NUM, 4);
        assert_eq!(VERSION, 2);
        assert_eq!(LATENCY_OFFSET, -22000);
        assert_eq!(MIN_CAMERA_EXPOSURE, 40);
        assert_eq!(MAX_CAMERA_EXPOSURE, 511);
        assert!((SPHERE_RADIUS_MM - 22.5).abs() < f32::EPSILON);
    }

    #[test]
    fn control_bits_match_pad() {
        assert_eq!(CTRL_SELECT, 0x01);
        assert_eq!(CTRL_T, 0x02);
        assert_eq!(CTRL_MOVE, 0x04);
        assert_eq!(CTRL_START, 0x08);
        assert_eq!(CTRL_TRIANGLE, 0x10);
        assert_eq!(CTRL_CIRCLE, 0x20);
        assert_eq!(CTRL_CROSS, 0x40);
        assert_eq!(CTRL_SQUARE, 0x80);
    }

    #[test]
    fn external_ids_stable() {
        assert_eq!(SHARP_SHOOTER_DEVICE_ID, 0x8081);
        assert_eq!(RACING_WHEEL_DEVICE_ID, 0x8101);
    }

    #[test]
    fn video_format_enum_stable() {
        assert_eq!(NO_VIDEO_OUTPUT, 1);
        assert_eq!(RGBA_640X480, 2);
        assert_eq!(YUV_640X480, 3);
        assert_eq!(BAYER_RESTORED_RASTERIZED, 9);
    }

    #[test]
    fn hue_sentinels_stable() {
        assert_eq!(DONT_TRACK_HUE, 0x0200_0000);
        assert_eq!(DONT_CARE_HUE, 0x0400_0000);
        assert_eq!(DONT_CHANGE_HUE, 0x0800_0000);
    }

    #[test]
    fn tracking_flags_stable() {
        assert_eq!(TRACKING_FLAG_POSITION_TRACKED, 1);
        assert_eq!(TRACKING_FLAG_VISIBLE, 2);
    }

    #[test]
    fn required_memory_formula() {
        assert_eq!(GemManager::required_memory(1), Ok(8 * 1024 * 1024));
        assert_eq!(GemManager::required_memory(4), Ok(32 * 1024 * 1024));
        assert_eq!(GemManager::required_memory(0), Err(errors::INVALID_PARAMETER));
        assert_eq!(GemManager::required_memory(5), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn init_happy_path() {
        let mut m = GemManager::new();
        m.init(ok_attribute(4)).unwrap();
        assert!(m.is_initialized());
    }

    #[test]
    fn init_wrong_version_rejected() {
        let mut m = GemManager::new();
        let mut a = ok_attribute(4);
        a.version = 99;
        assert_eq!(m.init(a), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn init_zero_max_connect_rejected() {
        let mut m = GemManager::new();
        let a = ok_attribute(0);
        assert_eq!(m.init(a), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn init_over_max_rejected() {
        let mut m = GemManager::new();
        let a = ok_attribute(MAX_NUM + 1);
        assert_eq!(m.init(a), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = initialized_manager(2);
        assert_eq!(m.init(ok_attribute(2)), Err(errors::ALREADY_INITIALIZED));
    }

    #[test]
    fn end_without_init_is_uninitialized() {
        let mut m = GemManager::new();
        assert_eq!(m.end(), Err(errors::UNINITIALIZED));
    }

    #[test]
    fn end_after_init_ok() {
        let mut m = initialized_manager(1);
        m.end().unwrap();
        assert!(!m.is_initialized());
    }

    #[test]
    fn prepare_camera_happy_path() {
        let mut m = initialized_manager(1);
        m.prepare_camera(200, 0.5).unwrap();
    }

    #[test]
    fn prepare_camera_exposure_too_low_rejected() {
        let mut m = initialized_manager(1);
        assert_eq!(m.prepare_camera(10, 0.5), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn prepare_camera_exposure_too_high_rejected() {
        let mut m = initialized_manager(1);
        assert_eq!(m.prepare_camera(1000, 0.5), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn prepare_camera_without_init_is_uninitialized() {
        let mut m = GemManager::new();
        assert_eq!(m.prepare_camera(100, 0.5), Err(errors::UNINITIALIZED));
    }

    #[test]
    fn prepare_video_convert_happy_path() {
        let mut m = initialized_manager(1);
        let attr = VideoConvertAttribute { format: RGBA_640X480, buffer_memory: 0x1000, video_data_out: 0x2000, ..Default::default() };
        m.prepare_video_convert(attr).unwrap();
        m.finish_video_convert().unwrap();
    }

    #[test]
    fn prepare_video_convert_unknown_format_rejected() {
        let mut m = initialized_manager(1);
        let attr = VideoConvertAttribute { format: 99, ..Default::default() };
        assert_eq!(m.prepare_video_convert(attr), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn prepare_video_convert_misaligned_buffer_rejected() {
        let mut m = initialized_manager(1);
        let attr = VideoConvertAttribute { format: RGBA_640X480, buffer_memory: 0x1007, ..Default::default() };
        assert_eq!(m.prepare_video_convert(attr), Err(errors::INVALID_ALIGNMENT));
    }

    #[test]
    fn prepare_video_convert_twice_is_not_finished() {
        let mut m = initialized_manager(1);
        m.prepare_video_convert(VideoConvertAttribute { format: RGBA_640X480, ..Default::default() }).unwrap();
        assert_eq!(
            m.prepare_video_convert(VideoConvertAttribute { format: RGBA_640X480, ..Default::default() }),
            Err(errors::CONVERT_NOT_FINISHED)
        );
    }

    #[test]
    fn finish_video_convert_without_start_is_not_started() {
        let mut m = initialized_manager(1);
        assert_eq!(m.finish_video_convert(), Err(errors::CONVERT_NOT_STARTED));
    }

    #[test]
    fn update_start_requires_camera_prep() {
        let mut m = initialized_manager(1);
        assert_eq!(m.update_start(0), Err(errors::UPDATE_NOT_STARTED));
    }

    #[test]
    fn update_start_finish_round_trip() {
        let mut m = initialized_manager(2);
        m.prepare_camera(200, 0.5).unwrap();
        let mut c = GemController::default();
        c.connected = true;
        m.inject_controller(0, c).unwrap();
        m.update_start(12345).unwrap();
        assert_eq!(m.update_finish(), Ok(1));
    }

    #[test]
    fn update_start_twice_is_not_finished() {
        let mut m = initialized_manager(1);
        m.prepare_camera(200, 0.5).unwrap();
        m.update_start(1).unwrap();
        assert_eq!(m.update_start(2), Err(errors::UPDATE_NOT_FINISHED));
    }

    #[test]
    fn update_finish_without_start_is_not_started() {
        let mut m = initialized_manager(1);
        assert_eq!(m.update_finish(), Err(errors::UPDATE_NOT_STARTED));
    }

    #[test]
    fn info_reports_disconnected_by_default() {
        let m = initialized_manager(4);
        let info = m.info().unwrap();
        assert_eq!(info.max_connect, 4);
        assert_eq!(info.connected, 0);
        assert!(info.info.iter().all(|&s| s == STATUS_DISCONNECTED));
        assert!(info.status.iter().all(|&s| s == NOT_CONNECTED));
    }

    #[test]
    fn info_reports_connected_after_inject() {
        let mut m = initialized_manager(2);
        let mut c = GemController::default();
        c.connected = true;
        c.calibrated = true;
        m.inject_controller(0, c).unwrap();
        let info = m.info().unwrap();
        assert_eq!(info.connected, 1);
        assert_eq!(info.info[0], STATUS_READY);
        assert_eq!(info.info[1], STATUS_DISCONNECTED);
        assert_eq!(info.status[0], STATUS_READY);
        assert_eq!(info.status[1], NOT_CONNECTED);
    }

    #[test]
    fn get_state_disconnected_controller_is_invalid_param() {
        let m = initialized_manager(1);
        assert_eq!(m.get_state(0, STATE_FLAG_CURRENT_TIME), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn get_state_returns_injected_state() {
        let mut m = initialized_manager(1);
        let mut c = GemController::default();
        c.connected = true;
        c.state.buttons = CTRL_MOVE | CTRL_T;
        c.state.analog_t = 200;
        m.inject_controller(0, c).unwrap();
        let state = m.get_state(0, STATE_FLAG_CURRENT_TIME).unwrap();
        assert_eq!(state.buttons, CTRL_MOVE | CTRL_T);
        assert_eq!(state.analog_t, 200);
    }

    #[test]
    fn get_state_out_of_range_is_invalid_param() {
        let m = initialized_manager(1);
        assert_eq!(m.get_state(99, STATE_FLAG_CURRENT_TIME), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn track_hues_too_many_is_invalid_param() {
        let mut m = initialized_manager(1);
        assert_eq!(m.track_hues(&[0x00FF_0000, 0x0000_FF00]), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn track_hues_accepts_sentinels_and_rgb() {
        let mut m = initialized_manager(4);
        m.track_hues(&[0x00FF_0000, DONT_TRACK_HUE, DONT_CARE_HUE, DONT_CHANGE_HUE]).unwrap();
        // DONT_CHANGE_HUE must not overwrite default.
        assert_eq!(m.controllers[3].hue, DONT_CARE_HUE);
    }

    #[test]
    fn track_hues_rejects_out_of_range_rgb() {
        let mut m = initialized_manager(1);
        assert_eq!(m.track_hues(&[0x0100_0000]), Err(errors::NOT_A_HUE));
    }

    #[test]
    fn set_rumble_happy_path() {
        let mut m = initialized_manager(1);
        m.set_rumble(0, 128).unwrap();
        assert_eq!(m.controllers[0].rumble, 128);
    }

    #[test]
    fn set_rumble_out_of_range_rejected() {
        let mut m = initialized_manager(1);
        assert_eq!(m.set_rumble(5, 10), Err(errors::INVALID_PARAMETER));
    }

    #[test]
    fn enable_magnetometer_toggle() {
        let mut m = initialized_manager(1);
        m.enable_magnetometer(0, true).unwrap();
        assert!(m.controllers[0].magnetometer_enabled);
        m.enable_magnetometer(0, false).unwrap();
        assert!(!m.controllers[0].magnetometer_enabled);
    }

    #[test]
    fn invalidate_calibration_clears_flag() {
        let mut m = initialized_manager(1);
        let mut c = GemController::default();
        c.connected = true;
        c.calibrated = true;
        m.inject_controller(0, c).unwrap();
        m.invalidate_calibration(0).unwrap();
        assert!(!m.controllers[0].calibrated);
    }

    #[test]
    fn status_flags_mask_and_clear() {
        let mut m = initialized_manager(1);
        m.inject_status_flags(FLAG_CALIBRATION_OCCURRED | FLAG_LIGHTING_CHANGED).unwrap();
        assert_eq!(
            m.status_flags(0, FLAG_CALIBRATION_OCCURRED | FLAG_WRONG_FOV).unwrap(),
            FLAG_CALIBRATION_OCCURRED
        );
        m.clear_status_flags(0, FLAG_CALIBRATION_OCCURRED).unwrap();
        assert_eq!(m.status_flags(0, ALL_FLAGS).unwrap(), FLAG_LIGHTING_CHANGED);
    }

    #[test]
    fn full_cycle_smoke() {
        let mut m = GemManager::new();
        assert!(GemManager::required_memory(2).is_ok());
        m.init(ok_attribute(2)).unwrap();
        m.prepare_camera(200, 0.5).unwrap();
        m.prepare_video_convert(VideoConvertAttribute { format: RGBA_640X480, ..Default::default() }).unwrap();
        let mut c = GemController::default();
        c.connected = true;
        c.calibrated = true;
        m.inject_controller(0, c).unwrap();
        m.track_hues(&[0x00FF_0000, DONT_CARE_HUE]).unwrap();
        m.set_rumble(0, 64).unwrap();
        m.enable_magnetometer(0, true).unwrap();
        m.update_start(1000).unwrap();
        assert_eq!(m.update_finish(), Ok(1));
        m.finish_video_convert().unwrap();
        m.end().unwrap();
    }
}
