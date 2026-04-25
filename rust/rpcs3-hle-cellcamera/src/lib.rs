//! `rpcs3-hle-cellcamera` — PlayStation Eye / EyeToy camera HLE.
//!
//! Ports the entry-point subset of
//! `rpcs3/Emu/Cell/Modules/cellCamera.cpp`. The runtime tracks a
//! single camera slot, its open/start state, and a key/value
//! attribute store the game uses to tune gain/exposure/white-balance.
//! Frame data comes from a [`CameraBackend`] trait the emu core
//! plugs in.
//!
//! ## Entry points covered
//!
//! | HLE function                  | Rust wrapper                    |
//! |-------------------------------|---------------------------------|
//! | `cellCameraInit`              | [`cell_camera_init`]            |
//! | `cellCameraEnd`               | [`cell_camera_end`]             |
//! | `cellCameraOpen`              | [`cell_camera_open`]            |
//! | `cellCameraClose`             | [`cell_camera_close`]           |
//! | `cellCameraStart`             | [`cell_camera_start`]           |
//! | `cellCameraStop`              | [`cell_camera_stop`]            |
//! | `cellCameraGetAttribute`      | [`cell_camera_get_attribute`]   |
//! | `cellCameraSetAttribute`      | [`cell_camera_set_attribute`]   |
//! | `cellCameraRead`              | [`cell_camera_read`]            |
//! | `cellCameraGetType`           | [`cell_camera_get_type`]        |
//! | `cellCameraIsAttached`        | [`cell_camera_is_attached`]     |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellCamera.h:14-28
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ALREADY_INIT: CellError = CellError(0x8014_0801);
    pub const NOT_INIT: CellError = CellError(0x8014_0803);
    pub const PARAM: CellError = CellError(0x8014_0804);
    pub const ALREADY_OPEN: CellError = CellError(0x8014_0805);
    pub const NOT_OPEN: CellError = CellError(0x8014_0806);
    pub const DEVICE_NOT_FOUND: CellError = CellError(0x8014_0807);
    pub const DEVICE_DEACTIVATED: CellError = CellError(0x8014_0808);
    pub const NOT_STARTED: CellError = CellError(0x8014_0809);
    pub const FORMAT_UNKNOWN: CellError = CellError(0x8014_080A);
    pub const RESOLUTION_UNKNOWN: CellError = CellError(0x8014_080B);
    pub const BAD_FRAMERATE: CellError = CellError(0x8014_080C);
    pub const TIMEOUT: CellError = CellError(0x8014_080D);
    pub const BUSY: CellError = CellError(0x8014_080E);
    pub const FATAL: CellError = CellError(0x8014_080F);
    pub const MUTEX: CellError = CellError(0x8014_0810);
}

// =====================================================================
// Enum constants (camera.h:227-303)
// =====================================================================

pub const TYPE_UNKNOWN: i32 = 0;
pub const TYPE_EYETOY: i32 = 1;
pub const TYPE_EYETOY2: i32 = 2;
pub const TYPE_USBVIDEOCLASS: i32 = 3;

pub const FORMAT_UNKNOWN: i32 = 0;
pub const FORMAT_JPG: i32 = 1;
pub const FORMAT_RAW8: i32 = 2;
pub const FORMAT_YUV422: i32 = 3;
pub const FORMAT_RAW10: i32 = 4;
pub const FORMAT_RGBA: i32 = 5;
pub const FORMAT_YUV420: i32 = 6;
pub const FORMAT_V_Y1_U_Y0: i32 = 7;

pub const RESOLUTION_UNKNOWN: i32 = 0;
pub const RESOLUTION_VGA: i32 = 1;
pub const RESOLUTION_QVGA: i32 = 2;
pub const RESOLUTION_WGA: i32 = 3;
pub const RESOLUTION_SPECIFIED_WH: i32 = 4;

// Attribute keys.
pub const ATTR_GAIN: i32 = 0;
pub const ATTR_REDBLUEGAIN: i32 = 1;
pub const ATTR_SATURATION: i32 = 2;
pub const ATTR_EXPOSURE: i32 = 3;
pub const ATTR_BRIGHTNESS: i32 = 4;
pub const ATTR_AEC: i32 = 5;
pub const ATTR_AGC: i32 = 6;
pub const ATTR_AWB: i32 = 7;
pub const ATTR_ABC: i32 = 8;
pub const ATTR_LED: i32 = 9;
pub const ATTR_AUDIOGAIN: i32 = 10;
pub const ATTR_QS: i32 = 11;
pub const ATTR_GAMMA: i32 = 20;
pub const ATTR_DENOISE: i32 = 23;

pub const ATTR_FORMATCAP: i32 = 100;
pub const ATTR_FORMATINDEX: i32 = 101;
pub const ATTR_UNKNOWN: i32 = 500;

pub const MAX_CAMERAS: usize = 2;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraInfo {
    pub format: i32,
    pub resolution: i32,
    pub framerate: u32,
    pub buffer_size: u32,
}

impl Default for CameraInfo {
    fn default() -> Self {
        Self {
            format: FORMAT_YUV422,
            resolution: RESOLUTION_VGA,
            framerate: 30,
            buffer_size: 640 * 480 * 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOutcome {
    pub frame_number: u64,
    pub bytes_read: u32,
}

// =====================================================================
// Backend trait
// =====================================================================

pub trait CameraBackend {
    fn attached(&self) -> bool;
    fn camera_type(&self) -> i32;
    fn read_frame(&mut self, out: &mut [u8]) -> Result<u32, CellError>;
}

/// No-camera backend — every call reports not-attached.
#[derive(Debug, Default)]
pub struct NullCameraBackend;
impl CameraBackend for NullCameraBackend {
    fn attached(&self) -> bool { false }
    fn camera_type(&self) -> i32 { TYPE_UNKNOWN }
    fn read_frame(&mut self, _: &mut [u8]) -> Result<u32, CellError> {
        Err(errors::DEVICE_NOT_FOUND)
    }
}

/// In-memory backend that emits canned RGBA frames for tests.
#[derive(Debug, Clone)]
pub struct TestCameraBackend {
    pub device_type: i32,
    pub frame_bytes: Vec<u8>,
    pub frames_read: u64,
}

impl TestCameraBackend {
    pub fn eyetoy_with_frame(bytes: Vec<u8>) -> Self {
        Self { device_type: TYPE_EYETOY, frame_bytes: bytes, frames_read: 0 }
    }
}

impl CameraBackend for TestCameraBackend {
    fn attached(&self) -> bool { true }
    fn camera_type(&self) -> i32 { self.device_type }
    fn read_frame(&mut self, out: &mut [u8]) -> Result<u32, CellError> {
        let n = self.frame_bytes.len().min(out.len());
        out[..n].copy_from_slice(&self.frame_bytes[..n]);
        self.frames_read += 1;
        Ok(n as u32)
    }
}

// =====================================================================
// State machine
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraState {
    Closed,
    Open,
    Running,
}

#[derive(Debug)]
pub struct CameraManager {
    initialized: bool,
    state: CameraState,
    info: CameraInfo,
    attributes: std::collections::BTreeMap<i32, (i32, i32)>,
    frame_count: u64,
}

impl Default for CameraManager {
    fn default() -> Self {
        Self {
            initialized: false,
            state: CameraState::Closed,
            info: CameraInfo::default(),
            attributes: std::collections::BTreeMap::new(),
            frame_count: 0,
        }
    }
}

// =====================================================================
// Validation helpers
// =====================================================================

fn ensure_init(m: &CameraManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::NOT_INIT) }
}

fn ensure_open(m: &CameraManager) -> Result<(), CellError> {
    ensure_init(m)?;
    if m.state == CameraState::Closed {
        Err(errors::NOT_OPEN)
    } else {
        Ok(())
    }
}

fn validate_info(info: &CameraInfo) -> Result<(), CellError> {
    if !matches!(
        info.format,
        FORMAT_JPG | FORMAT_RAW8 | FORMAT_YUV422 | FORMAT_RAW10
        | FORMAT_RGBA | FORMAT_YUV420 | FORMAT_V_Y1_U_Y0,
    ) {
        return Err(errors::FORMAT_UNKNOWN);
    }
    if !matches!(
        info.resolution,
        RESOLUTION_VGA | RESOLUTION_QVGA | RESOLUTION_WGA | RESOLUTION_SPECIFIED_WH,
    ) {
        return Err(errors::RESOLUTION_UNKNOWN);
    }
    if info.framerate == 0 || info.framerate > 60 {
        return Err(errors::BAD_FRAMERATE);
    }
    Ok(())
}

// =====================================================================
// Syscalls
// =====================================================================

#[must_use]
pub fn cell_camera_init(m: &mut CameraManager) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::ALREADY_INIT);
    }
    m.initialized = true;
    Ok(())
}

#[must_use]
pub fn cell_camera_end(m: &mut CameraManager) -> Result<(), CellError> {
    ensure_init(m)?;
    *m = CameraManager::default();
    Ok(())
}

#[must_use]
pub fn cell_camera_is_attached<B: CameraBackend + ?Sized>(
    m: &CameraManager,
    backend: &B,
) -> Result<bool, CellError> {
    ensure_init(m)?;
    Ok(backend.attached())
}

#[must_use]
pub fn cell_camera_get_type<B: CameraBackend + ?Sized>(
    m: &CameraManager,
    backend: &B,
) -> Result<i32, CellError> {
    ensure_init(m)?;
    if !backend.attached() {
        return Err(errors::DEVICE_NOT_FOUND);
    }
    Ok(backend.camera_type())
}

#[must_use]
pub fn cell_camera_open<B: CameraBackend + ?Sized>(
    m: &mut CameraManager,
    backend: &B,
    info: CameraInfo,
) -> Result<(), CellError> {
    ensure_init(m)?;
    if m.state != CameraState::Closed {
        return Err(errors::ALREADY_OPEN);
    }
    if !backend.attached() {
        return Err(errors::DEVICE_NOT_FOUND);
    }
    validate_info(&info)?;
    m.info = info;
    m.state = CameraState::Open;
    Ok(())
}

#[must_use]
pub fn cell_camera_close(m: &mut CameraManager) -> Result<(), CellError> {
    ensure_open(m)?;
    m.state = CameraState::Closed;
    Ok(())
}

#[must_use]
pub fn cell_camera_start(m: &mut CameraManager) -> Result<(), CellError> {
    ensure_open(m)?;
    m.state = CameraState::Running;
    Ok(())
}

#[must_use]
pub fn cell_camera_stop(m: &mut CameraManager) -> Result<(), CellError> {
    ensure_init(m)?;
    if m.state != CameraState::Running {
        return Err(errors::NOT_STARTED);
    }
    m.state = CameraState::Open;
    Ok(())
}

#[must_use]
pub fn cell_camera_set_attribute(
    m: &mut CameraManager,
    attr: i32,
    value_lo: i32,
    value_hi: i32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    if attr < 0 || attr >= ATTR_UNKNOWN {
        return Err(errors::PARAM);
    }
    m.attributes.insert(attr, (value_lo, value_hi));
    Ok(())
}

#[must_use]
pub fn cell_camera_get_attribute(
    m: &CameraManager,
    attr: i32,
) -> Result<(i32, i32), CellError> {
    ensure_init(m)?;
    if attr < 0 || attr >= ATTR_UNKNOWN {
        return Err(errors::PARAM);
    }
    Ok(m.attributes.get(&attr).copied().unwrap_or((0, 0)))
}

/// `cellCameraRead(frame_num_out, bytes_read_out, buf)`.
#[must_use]
pub fn cell_camera_read<B: CameraBackend + ?Sized>(
    m: &mut CameraManager,
    backend: &mut B,
    buf: &mut [u8],
) -> Result<ReadOutcome, CellError> {
    ensure_open(m)?;
    if m.state != CameraState::Running {
        return Err(errors::NOT_STARTED);
    }
    if !backend.attached() {
        return Err(errors::DEVICE_DEACTIVATED);
    }
    let bytes = backend.read_frame(buf)?;
    m.frame_count += 1;
    Ok(ReadOutcome { frame_number: m.frame_count, bytes_read: bytes })
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> CameraManager {
        let mut m = CameraManager::default();
        cell_camera_init(&mut m).unwrap();
        m
    }

    fn eyetoy() -> TestCameraBackend {
        TestCameraBackend::eyetoy_with_frame(vec![0x42u8; 1024])
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::ALREADY_INIT.0, 0x8014_0801);
        assert_eq!(errors::NOT_INIT.0, 0x8014_0803);
        assert_eq!(errors::DEVICE_NOT_FOUND.0, 0x8014_0807);
        assert_eq!(errors::NOT_STARTED.0, 0x8014_0809);
        assert_eq!(errors::MUTEX.0, 0x8014_0810);
    }

    #[test]
    fn type_enum_values_match() {
        assert_eq!(TYPE_UNKNOWN, 0);
        assert_eq!(TYPE_EYETOY, 1);
        assert_eq!(TYPE_EYETOY2, 2);
        assert_eq!(TYPE_USBVIDEOCLASS, 3);
    }

    #[test]
    fn format_enum_values_match() {
        assert_eq!(FORMAT_UNKNOWN, 0);
        assert_eq!(FORMAT_JPG, 1);
        assert_eq!(FORMAT_YUV422, 3);
        assert_eq!(FORMAT_RGBA, 5);
        assert_eq!(FORMAT_V_Y1_U_Y0, 7);
    }

    #[test]
    fn resolution_enum_values_match() {
        assert_eq!(RESOLUTION_UNKNOWN, 0);
        assert_eq!(RESOLUTION_VGA, 1);
        assert_eq!(RESOLUTION_QVGA, 2);
        assert_eq!(RESOLUTION_SPECIFIED_WH, 4);
    }

    // --- init / end ----------------------------------------------

    #[test]
    fn init_twice_is_already_init() {
        let mut m = CameraManager::default();
        cell_camera_init(&mut m).unwrap();
        assert_eq!(cell_camera_init(&mut m).unwrap_err(), errors::ALREADY_INIT);
    }

    #[test]
    fn ops_without_init_are_not_init() {
        let mut m = CameraManager::default();
        let b = eyetoy();
        assert_eq!(cell_camera_is_attached(&m, &b).unwrap_err(), errors::NOT_INIT);
        assert_eq!(cell_camera_get_type(&m, &b).unwrap_err(), errors::NOT_INIT);
        assert_eq!(
            cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap_err(),
            errors::NOT_INIT,
        );
    }

    #[test]
    fn end_resets_state() {
        let mut m = init();
        let b = eyetoy();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        cell_camera_end(&mut m).unwrap();
        // Reopening requires re-init.
        assert_eq!(
            cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap_err(),
            errors::NOT_INIT,
        );
    }

    // --- is_attached / get_type ----------------------------------

    #[test]
    fn is_attached_reflects_backend() {
        let m = init();
        assert_eq!(cell_camera_is_attached(&m, &eyetoy()).unwrap(), true);
        assert_eq!(cell_camera_is_attached(&m, &NullCameraBackend).unwrap(), false);
    }

    #[test]
    fn get_type_not_attached_is_device_not_found() {
        let m = init();
        assert_eq!(
            cell_camera_get_type(&m, &NullCameraBackend).unwrap_err(),
            errors::DEVICE_NOT_FOUND,
        );
    }

    #[test]
    fn get_type_returns_backend_type() {
        let m = init();
        assert_eq!(cell_camera_get_type(&m, &eyetoy()).unwrap(), TYPE_EYETOY);
    }

    // --- open / close / start / stop ------------------------------

    #[test]
    fn open_close_round_trip() {
        let mut m = init();
        let b = eyetoy();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        cell_camera_close(&mut m).unwrap();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
    }

    #[test]
    fn open_twice_is_already_open() {
        let mut m = init();
        let b = eyetoy();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        assert_eq!(
            cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap_err(),
            errors::ALREADY_OPEN,
        );
    }

    #[test]
    fn open_without_attached_is_device_not_found() {
        let mut m = init();
        assert_eq!(
            cell_camera_open(&mut m, &NullCameraBackend, CameraInfo::default()).unwrap_err(),
            errors::DEVICE_NOT_FOUND,
        );
    }

    #[test]
    fn open_rejects_unknown_format() {
        let mut m = init();
        let b = eyetoy();
        let info = CameraInfo { format: 99, ..CameraInfo::default() };
        assert_eq!(
            cell_camera_open(&mut m, &b, info).unwrap_err(),
            errors::FORMAT_UNKNOWN,
        );
    }

    #[test]
    fn open_rejects_unknown_resolution() {
        let mut m = init();
        let b = eyetoy();
        let info = CameraInfo { resolution: 99, ..CameraInfo::default() };
        assert_eq!(
            cell_camera_open(&mut m, &b, info).unwrap_err(),
            errors::RESOLUTION_UNKNOWN,
        );
    }

    #[test]
    fn open_rejects_zero_or_over_60_framerate() {
        let mut m = init();
        let b = eyetoy();
        let info = CameraInfo { framerate: 0, ..CameraInfo::default() };
        assert_eq!(
            cell_camera_open(&mut m, &b, info).unwrap_err(),
            errors::BAD_FRAMERATE,
        );
        let info = CameraInfo { framerate: 120, ..CameraInfo::default() };
        assert_eq!(
            cell_camera_open(&mut m, &b, info).unwrap_err(),
            errors::BAD_FRAMERATE,
        );
    }

    #[test]
    fn close_when_not_open_is_not_open() {
        let mut m = init();
        assert_eq!(cell_camera_close(&mut m).unwrap_err(), errors::NOT_OPEN);
    }

    #[test]
    fn start_stop_transitions_state() {
        let mut m = init();
        let b = eyetoy();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        cell_camera_start(&mut m).unwrap();
        assert_eq!(m.state, CameraState::Running);
        cell_camera_stop(&mut m).unwrap();
        assert_eq!(m.state, CameraState::Open);
    }

    #[test]
    fn stop_when_not_started_is_not_started() {
        let mut m = init();
        let b = eyetoy();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        assert_eq!(cell_camera_stop(&mut m).unwrap_err(), errors::NOT_STARTED);
    }

    // --- attributes -----------------------------------------------

    #[test]
    fn set_then_get_attribute_round_trips() {
        let mut m = init();
        cell_camera_set_attribute(&mut m, ATTR_EXPOSURE, 120, 0).unwrap();
        assert_eq!(
            cell_camera_get_attribute(&m, ATTR_EXPOSURE).unwrap(),
            (120, 0),
        );
    }

    #[test]
    fn get_unset_attribute_returns_zero() {
        let m = init();
        assert_eq!(cell_camera_get_attribute(&m, ATTR_GAIN).unwrap(), (0, 0));
    }

    #[test]
    fn set_unknown_attribute_is_param() {
        let mut m = init();
        assert_eq!(
            cell_camera_set_attribute(&mut m, ATTR_UNKNOWN, 0, 0).unwrap_err(),
            errors::PARAM,
        );
        assert_eq!(
            cell_camera_set_attribute(&mut m, -1, 0, 0).unwrap_err(),
            errors::PARAM,
        );
    }

    // --- read -----------------------------------------------------

    #[test]
    fn read_before_start_is_not_started() {
        let mut m = init();
        let mut b = eyetoy();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        let mut buf = vec![0u8; 16];
        assert_eq!(
            cell_camera_read(&mut m, &mut b, &mut buf).unwrap_err(),
            errors::NOT_STARTED,
        );
    }

    #[test]
    fn read_increments_frame_counter() {
        let mut m = init();
        let mut b = eyetoy();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        cell_camera_start(&mut m).unwrap();
        let mut buf = vec![0u8; 256];
        let r1 = cell_camera_read(&mut m, &mut b, &mut buf).unwrap();
        let r2 = cell_camera_read(&mut m, &mut b, &mut buf).unwrap();
        assert_eq!(r1.frame_number, 1);
        assert_eq!(r2.frame_number, 2);
        assert_eq!(r1.bytes_read, 256);
        assert_eq!(buf[0], 0x42);
    }

    #[test]
    fn read_when_device_detached_is_device_deactivated() {
        let mut m = init();
        let mut b = eyetoy();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        cell_camera_start(&mut m).unwrap();

        // Swap to null backend to simulate hot-unplug.
        let mut disconnected = NullCameraBackend;
        let mut buf = vec![0u8; 16];
        assert_eq!(
            cell_camera_read(&mut m, &mut disconnected, &mut buf).unwrap_err(),
            errors::DEVICE_DEACTIVATED,
        );
    }

    #[test]
    fn full_lifecycle_init_open_start_read_stop_close_end() {
        let mut m = CameraManager::default();
        let mut b = eyetoy();
        cell_camera_init(&mut m).unwrap();
        cell_camera_open(&mut m, &b, CameraInfo::default()).unwrap();
        cell_camera_start(&mut m).unwrap();
        let mut buf = vec![0u8; 32];
        cell_camera_read(&mut m, &mut b, &mut buf).unwrap();
        cell_camera_stop(&mut m).unwrap();
        cell_camera_close(&mut m).unwrap();
        cell_camera_end(&mut m).unwrap();
    }
}
