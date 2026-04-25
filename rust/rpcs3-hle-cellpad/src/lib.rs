//! `rpcs3-hle-cellpad` — controller-input HLE layer.
//!
//! Ports the game-facing entry points from
//! `rpcs3/Emu/Cell/Modules/cellPad.cpp`. The actual hardware backend
//! (DualShock, Xbox pad, keyboard, …) is out of scope: this crate
//! talks to the emu core via the [`PadBackend`] trait and surfaces
//! the classic `CellPadData` / `CellPadInfo2` shapes that games read.
//!
//! ## Syscalls covered
//!
//! | HLE function                  | Rust wrapper                |
//! |-------------------------------|-----------------------------|
//! | `cellPadInit`                 | [`cell_pad_init`]           |
//! | `cellPadEnd`                  | [`cell_pad_end`]            |
//! | `cellPadGetInfo2`             | [`cell_pad_get_info2`]      |
//! | `cellPadGetData`              | [`cell_pad_get_data`]       |
//! | `cellPadSetPortSetting`       | [`cell_pad_set_port_setting`] |
//! | `cellPadClearBuf`             | [`cell_pad_clear_buf`]      |
//!
//! ## Frozen constants (from `pad_types.h`)
//!
//! * `MAX_PORT_NUM = 7`
//! * `MAX_CODES = 64`
//! * `MAX_PADS = 127`

use rpcs3_emu_types::CellError;

// =====================================================================
// Result codes
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const FATAL: CellError = CellError(0x8012_1101);
    pub const INVALID_PARAMETER: CellError = CellError(0x8012_1102);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8012_1103);
    pub const UNINITIALIZED: CellError = CellError(0x8012_1104);
    pub const RESOURCE_ALLOCATION_FAILED: CellError = CellError(0x8012_1105);
    pub const DATA_READ_FAILED: CellError = CellError(0x8012_1106);
    pub const NO_DEVICE: CellError = CellError(0x8012_1107);
    pub const UNSUPPORTED_GAMEPAD: CellError = CellError(0x8012_1108);
    pub const TOO_MANY_DEVICES: CellError = CellError(0x8012_1109);
    pub const BUSY: CellError = CellError(0x8012_110A);
}

// ---- Layout constants ----------------------------------------------

pub const MAX_PORT_NUM: usize = 7;
pub const MAX_CODES: usize = 64;
pub const MAX_PADS: usize = 127;

// ---- Status bits ---------------------------------------------------

pub const STATUS_DISCONNECTED: u32 = 0x0000_0000;
pub const STATUS_CONNECTED: u32 = 0x0000_0001;
pub const STATUS_ASSIGN_CHANGES: u32 = 0x0000_0002;
pub const STATUS_CUSTOM_CONTROLLER: u32 = 0x0000_0004;

// ---- Digital1 (d-pad + system) ------------------------------------

pub const CTRL_SELECT: u16 = 0x0001;
pub const CTRL_L3: u16 = 0x0002;
pub const CTRL_R3: u16 = 0x0004;
pub const CTRL_START: u16 = 0x0008;
pub const CTRL_UP: u16 = 0x0010;
pub const CTRL_RIGHT: u16 = 0x0020;
pub const CTRL_DOWN: u16 = 0x0040;
pub const CTRL_LEFT: u16 = 0x0080;
pub const CTRL_PS: u16 = 0x0100;

// ---- Digital2 (face buttons + triggers) ---------------------------

pub const CTRL_L2: u16 = 0x0001;
pub const CTRL_R2: u16 = 0x0002;
pub const CTRL_L1: u16 = 0x0004;
pub const CTRL_R1: u16 = 0x0008;
pub const CTRL_TRIANGLE: u16 = 0x0010;
pub const CTRL_CIRCLE: u16 = 0x0020;
pub const CTRL_CROSS: u16 = 0x0040;
pub const CTRL_SQUARE: u16 = 0x0080;

// ---- Button buffer offsets (index into CellPadData.button[]) -----

pub const BTN_OFFSET_DIGITAL1: usize = 2;
pub const BTN_OFFSET_DIGITAL2: usize = 3;
pub const BTN_OFFSET_ANALOG_RIGHT_X: usize = 4;
pub const BTN_OFFSET_ANALOG_RIGHT_Y: usize = 5;
pub const BTN_OFFSET_ANALOG_LEFT_X: usize = 6;
pub const BTN_OFFSET_ANALOG_LEFT_Y: usize = 7;
pub const BTN_OFFSET_PRESS_RIGHT: usize = 8;
pub const BTN_OFFSET_PRESS_LEFT: usize = 9;
pub const BTN_OFFSET_PRESS_UP: usize = 10;
pub const BTN_OFFSET_PRESS_DOWN: usize = 11;
pub const BTN_OFFSET_PRESS_TRIANGLE: usize = 12;
pub const BTN_OFFSET_PRESS_CIRCLE: usize = 13;
pub const BTN_OFFSET_PRESS_CROSS: usize = 14;
pub const BTN_OFFSET_PRESS_SQUARE: usize = 15;
pub const BTN_OFFSET_PRESS_L1: usize = 16;
pub const BTN_OFFSET_PRESS_R1: usize = 17;
pub const BTN_OFFSET_PRESS_L2: usize = 18;
pub const BTN_OFFSET_PRESS_R2: usize = 19;

// =====================================================================
// Data shapes
// =====================================================================

/// Snapshot of buttons + analog axes for one pad. Matches the
/// `CellPadData` the guest reads: 64 u16 slots, big-endian on-wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadData {
    /// `len` — number of valid entries (usually 24 for standard pad).
    pub len: i32,
    pub button: [u16; MAX_CODES],
}

impl Default for PadData {
    fn default() -> Self {
        Self { len: 24, button: [0; MAX_CODES] }
    }
}

/// Friendlier button bitmap → [`PadData`] helper for tests and
/// backends: set `digital1` / `digital2` bitmasks + analog axes,
/// receive a fully-populated struct.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ButtonState {
    pub digital1: u16,
    pub digital2: u16,
    pub left_stick_x: u8,
    pub left_stick_y: u8,
    pub right_stick_x: u8,
    pub right_stick_y: u8,
    /// Per-button analog pressure 0..=255. Indices match the
    /// `BTN_OFFSET_PRESS_*` constants (starting at 8).
    pub press: [u8; 12],
}

impl ButtonState {
    /// Render this state into a `PadData` buffer.
    #[must_use]
    pub fn render(&self) -> PadData {
        let mut d = PadData::default();
        d.button[BTN_OFFSET_DIGITAL1] = self.digital1;
        d.button[BTN_OFFSET_DIGITAL2] = self.digital2;
        d.button[BTN_OFFSET_ANALOG_LEFT_X] = u16::from(self.left_stick_x);
        d.button[BTN_OFFSET_ANALOG_LEFT_Y] = u16::from(self.left_stick_y);
        d.button[BTN_OFFSET_ANALOG_RIGHT_X] = u16::from(self.right_stick_x);
        d.button[BTN_OFFSET_ANALOG_RIGHT_Y] = u16::from(self.right_stick_y);
        for (i, &p) in self.press.iter().enumerate() {
            d.button[BTN_OFFSET_PRESS_RIGHT + i] = u16::from(p);
        }
        d
    }
}

/// `CellPadInfo2` — per-port metadata returned by `cellPadGetInfo2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PadInfo2 {
    pub max_connect: u32,
    pub now_connect: u32,
    pub system_info: u32,
    pub port_status: [u32; MAX_PORT_NUM],
    pub port_setting: [u32; MAX_PORT_NUM],
    pub device_capability: [u32; MAX_PORT_NUM],
    pub device_type: [u32; MAX_PORT_NUM],
}

impl Default for PadInfo2 {
    fn default() -> Self {
        Self {
            max_connect: MAX_PORT_NUM as u32,
            now_connect: 0,
            system_info: 0,
            port_status: [0; MAX_PORT_NUM],
            port_setting: [0; MAX_PORT_NUM],
            device_capability: [0; MAX_PORT_NUM],
            device_type: [0; MAX_PORT_NUM],
        }
    }
}

// =====================================================================
// Backend trait
// =====================================================================

/// Hardware-facing backend. The emu core implements this over the
/// user's real controller handler (DualShock4, keyboard, …); tests
/// use [`TestPadBackend`].
pub trait PadBackend {
    /// Is the pad at `port_no` currently connected?
    fn is_connected(&self, port_no: usize) -> bool;

    /// Read the latest button snapshot for `port_no`. Called every
    /// frame by `cellPadGetData`.
    fn read(&self, port_no: usize) -> Option<ButtonState>;

    /// Device capability bits (reported in `CellPadInfo2`).
    fn capability(&self, port_no: usize) -> u32 {
        let _ = port_no;
        0
    }

    /// Device-type ordinal (DualShock, SixAxis, Move, …).
    fn device_type(&self, port_no: usize) -> u32 {
        let _ = port_no;
        0
    }
}

// =====================================================================
// Pad-manager state — owned by the emu core, passed into wrappers
// =====================================================================

#[derive(Debug)]
pub struct PadManager {
    initialized: bool,
    port_setting: [u32; MAX_PORT_NUM],
    /// Buffer-latch: when `cellPadClearBuf` is called, the next
    /// `cellPadGetData` returns an all-zero snapshot regardless of
    /// backend state.
    buf_cleared: [bool; MAX_PORT_NUM],
}

impl Default for PadManager {
    fn default() -> Self {
        Self {
            initialized: false,
            port_setting: [0; MAX_PORT_NUM],
            buf_cleared: [false; MAX_PORT_NUM],
        }
    }
}

// =====================================================================
// Syscalls
// =====================================================================

fn ensure_init(m: &PadManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::UNINITIALIZED) }
}

fn check_port(port_no: usize) -> Result<(), CellError> {
    if port_no >= MAX_PORT_NUM {
        Err(errors::INVALID_PARAMETER)
    } else {
        Ok(())
    }
}

/// `cellPadInit(max_connect)` — must be the first call on the HLE.
#[must_use]
pub fn cell_pad_init(m: &mut PadManager, max_connect: u32) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::ALREADY_INITIALIZED);
    }
    if max_connect == 0 || max_connect as usize > MAX_PORT_NUM {
        return Err(errors::INVALID_PARAMETER);
    }
    m.initialized = true;
    Ok(())
}

/// `cellPadEnd()` — release the HLE.
#[must_use]
pub fn cell_pad_end(m: &mut PadManager) -> Result<(), CellError> {
    ensure_init(m)?;
    *m = PadManager::default();
    Ok(())
}

/// `cellPadGetInfo2(info_out)`.
#[must_use]
pub fn cell_pad_get_info2<B: PadBackend + ?Sized>(
    m: &PadManager,
    backend: &B,
) -> Result<PadInfo2, CellError> {
    ensure_init(m)?;
    let mut info = PadInfo2::default();
    let mut now = 0;
    for port in 0..MAX_PORT_NUM {
        if backend.is_connected(port) {
            info.port_status[port] = STATUS_CONNECTED;
            info.device_capability[port] = backend.capability(port);
            info.device_type[port] = backend.device_type(port);
            now += 1;
        }
        info.port_setting[port] = m.port_setting[port];
    }
    info.now_connect = now;
    Ok(info)
}

/// `cellPadGetData(port_no, data_out)`.
#[must_use]
pub fn cell_pad_get_data<B: PadBackend + ?Sized>(
    m: &mut PadManager,
    backend: &B,
    port_no: usize,
) -> Result<PadData, CellError> {
    ensure_init(m)?;
    check_port(port_no)?;
    if !backend.is_connected(port_no) {
        return Err(errors::NO_DEVICE);
    }

    // One-shot latch: `cellPadClearBuf` forces the next read to be
    // zeroed, then resets the latch.
    if m.buf_cleared[port_no] {
        m.buf_cleared[port_no] = false;
        return Ok(PadData { len: 0, ..PadData::default() });
    }

    let state = backend.read(port_no).ok_or(errors::DATA_READ_FAILED)?;
    Ok(state.render())
}

/// `cellPadSetPortSetting(port_no, setting)`.
#[must_use]
pub fn cell_pad_set_port_setting(
    m: &mut PadManager,
    port_no: usize,
    setting: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    check_port(port_no)?;
    m.port_setting[port_no] = setting;
    Ok(())
}

/// `cellPadClearBuf(port_no)` — zero the next snapshot read.
#[must_use]
pub fn cell_pad_clear_buf(m: &mut PadManager, port_no: usize) -> Result<(), CellError> {
    ensure_init(m)?;
    check_port(port_no)?;
    m.buf_cleared[port_no] = true;
    Ok(())
}

// =====================================================================
// Reference backend — in-memory, used by tests
// =====================================================================

#[derive(Debug, Default)]
pub struct TestPadBackend {
    pub ports: [Option<ButtonState>; MAX_PORT_NUM],
}

impl PadBackend for TestPadBackend {
    fn is_connected(&self, port_no: usize) -> bool {
        self.ports.get(port_no).is_some_and(|p| p.is_some())
    }
    fn read(&self, port_no: usize) -> Option<ButtonState> {
        self.ports.get(port_no).copied().flatten()
    }
    fn capability(&self, _port_no: usize) -> u32 {
        // CELL_PAD_CAPABILITY_PS3_CONFORMITY | PRESSURE_SENSITIVE_BUTTONS |
        // SENSOR_MODE | HP_ANALOG_STICK | ACTUATOR — composite value used
        // by the default DualShock-style backend.
        0x1F
    }
    fn device_type(&self, _port_no: usize) -> u32 {
        // CELL_PAD_DEV_TYPE_STANDARD
        0
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn mgr_with_pad(port: usize, state: ButtonState) -> (PadManager, TestPadBackend) {
        let mut m = PadManager::default();
        cell_pad_init(&mut m, MAX_PORT_NUM as u32).unwrap();
        let mut b = TestPadBackend::default();
        b.ports[port] = Some(state);
        (m, b)
    }

    // --- constants -------------------------------------------------

    #[test]
    fn error_codes_match_cellPad_h() {
        assert_eq!(errors::FATAL.0, 0x8012_1101);
        assert_eq!(errors::INVALID_PARAMETER.0, 0x8012_1102);
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8012_1103);
        assert_eq!(errors::UNINITIALIZED.0, 0x8012_1104);
        assert_eq!(errors::NO_DEVICE.0, 0x8012_1107);
        assert_eq!(errors::BUSY.0, 0x8012_110A);
    }

    #[test]
    fn digital_button_constants_frozen() {
        assert_eq!(CTRL_SELECT, 0x01);
        assert_eq!(CTRL_START, 0x08);
        assert_eq!(CTRL_UP, 0x10);
        assert_eq!(CTRL_LEFT, 0x80);
        assert_eq!(CTRL_PS, 0x0100);
        assert_eq!(CTRL_CROSS, 0x40);
        assert_eq!(CTRL_SQUARE, 0x80);
    }

    #[test]
    fn layout_constants() {
        assert_eq!(MAX_PORT_NUM, 7);
        assert_eq!(MAX_CODES, 64);
        assert_eq!(MAX_PADS, 127);
    }

    // --- init / end -----------------------------------------------

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = PadManager::default();
        cell_pad_init(&mut m, 4).unwrap();
        assert_eq!(cell_pad_init(&mut m, 4).unwrap_err(), errors::ALREADY_INITIALIZED);
    }

    #[test]
    fn init_rejects_zero_and_over_max() {
        let mut m = PadManager::default();
        assert_eq!(cell_pad_init(&mut m, 0).unwrap_err(), errors::INVALID_PARAMETER);
        assert_eq!(cell_pad_init(&mut m, 99).unwrap_err(), errors::INVALID_PARAMETER);
    }

    #[test]
    fn end_without_init_is_uninitialized() {
        let mut m = PadManager::default();
        assert_eq!(cell_pad_end(&mut m).unwrap_err(), errors::UNINITIALIZED);
    }

    #[test]
    fn end_after_init_succeeds_and_resets() {
        let mut m = PadManager::default();
        cell_pad_init(&mut m, 4).unwrap();
        cell_pad_end(&mut m).unwrap();
        // Second end is uninitialized.
        assert_eq!(cell_pad_end(&mut m).unwrap_err(), errors::UNINITIALIZED);
    }

    // --- info -----------------------------------------------------

    #[test]
    fn info2_reports_connected_ports() {
        let (m, b) = mgr_with_pad(0, ButtonState::default());
        let info = cell_pad_get_info2(&m, &b).unwrap();
        assert_eq!(info.now_connect, 1);
        assert_eq!(info.port_status[0] & STATUS_CONNECTED, STATUS_CONNECTED);
        assert_eq!(info.port_status[1], 0);
        assert_eq!(info.max_connect, MAX_PORT_NUM as u32);
    }

    #[test]
    fn info2_without_init_is_uninitialized() {
        let m = PadManager::default();
        let b = TestPadBackend::default();
        assert_eq!(cell_pad_get_info2(&m, &b).unwrap_err(), errors::UNINITIALIZED);
    }

    // --- get_data -------------------------------------------------

    #[test]
    fn get_data_returns_rendered_state() {
        let bs = ButtonState {
            digital1: CTRL_UP | CTRL_START,
            digital2: CTRL_CROSS,
            left_stick_x: 128,
            ..ButtonState::default()
        };
        let (mut m, b) = mgr_with_pad(2, bs);
        let d = cell_pad_get_data(&mut m, &b, 2).unwrap();
        assert_eq!(d.button[BTN_OFFSET_DIGITAL1], (CTRL_UP | CTRL_START) as u16);
        assert_eq!(d.button[BTN_OFFSET_DIGITAL2], CTRL_CROSS as u16);
        assert_eq!(d.button[BTN_OFFSET_ANALOG_LEFT_X], 128);
        assert_eq!(d.len, 24);
    }

    #[test]
    fn get_data_on_disconnected_port_is_no_device() {
        let mut m = PadManager::default();
        cell_pad_init(&mut m, 4).unwrap();
        let b = TestPadBackend::default();
        assert_eq!(cell_pad_get_data(&mut m, &b, 0).unwrap_err(), errors::NO_DEVICE);
    }

    #[test]
    fn get_data_with_bad_port_is_invalid_parameter() {
        let (mut m, b) = mgr_with_pad(0, ButtonState::default());
        assert_eq!(
            cell_pad_get_data(&mut m, &b, 99).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    // --- clear_buf latch -------------------------------------------

    #[test]
    fn clear_buf_forces_next_read_empty() {
        let bs = ButtonState { digital1: CTRL_UP, ..ButtonState::default() };
        let (mut m, b) = mgr_with_pad(0, bs);

        cell_pad_clear_buf(&mut m, 0).unwrap();
        let d = cell_pad_get_data(&mut m, &b, 0).unwrap();
        assert_eq!(d.len, 0);
        assert_eq!(d.button[BTN_OFFSET_DIGITAL1], 0, "cleared snapshot must be zero");

        // Subsequent reads return live state again.
        let d = cell_pad_get_data(&mut m, &b, 0).unwrap();
        assert_eq!(d.button[BTN_OFFSET_DIGITAL1], CTRL_UP);
    }

    #[test]
    fn clear_buf_bad_port_is_invalid_parameter() {
        let mut m = PadManager::default();
        cell_pad_init(&mut m, 4).unwrap();
        assert_eq!(
            cell_pad_clear_buf(&mut m, 42).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    // --- port settings --------------------------------------------

    #[test]
    fn set_port_setting_round_trips_via_info2() {
        let (mut m, b) = mgr_with_pad(3, ButtonState::default());
        cell_pad_set_port_setting(&mut m, 3, 0xCAFE_BABE).unwrap();
        let info = cell_pad_get_info2(&m, &b).unwrap();
        assert_eq!(info.port_setting[3], 0xCAFE_BABE);
        assert_eq!(info.port_setting[0], 0);
    }

    #[test]
    fn set_port_setting_on_bad_port_is_invalid_parameter() {
        let mut m = PadManager::default();
        cell_pad_init(&mut m, 4).unwrap();
        assert_eq!(
            cell_pad_set_port_setting(&mut m, 99, 0).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    // --- ButtonState rendering ------------------------------------

    #[test]
    fn press_values_land_in_correct_offsets() {
        let bs = ButtonState {
            press: [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120],
            ..ButtonState::default()
        };
        let d = bs.render();
        assert_eq!(d.button[BTN_OFFSET_PRESS_RIGHT], 10);
        assert_eq!(d.button[BTN_OFFSET_PRESS_CROSS], 70);
        assert_eq!(d.button[BTN_OFFSET_PRESS_R2], 120);
    }

    #[test]
    fn analog_sticks_populate_correct_slots() {
        let bs = ButtonState {
            left_stick_x: 1,
            left_stick_y: 2,
            right_stick_x: 3,
            right_stick_y: 4,
            ..ButtonState::default()
        };
        let d = bs.render();
        assert_eq!(d.button[BTN_OFFSET_ANALOG_LEFT_X], 1);
        assert_eq!(d.button[BTN_OFFSET_ANALOG_LEFT_Y], 2);
        assert_eq!(d.button[BTN_OFFSET_ANALOG_RIGHT_X], 3);
        assert_eq!(d.button[BTN_OFFSET_ANALOG_RIGHT_Y], 4);
    }

    #[test]
    fn multi_port_reads_independent() {
        let mut m = PadManager::default();
        cell_pad_init(&mut m, MAX_PORT_NUM as u32).unwrap();
        let mut b = TestPadBackend::default();
        b.ports[0] = Some(ButtonState { digital1: CTRL_UP, ..ButtonState::default() });
        b.ports[1] = Some(ButtonState { digital1: CTRL_DOWN, ..ButtonState::default() });

        assert_eq!(
            cell_pad_get_data(&mut m, &b, 0).unwrap().button[BTN_OFFSET_DIGITAL1],
            CTRL_UP,
        );
        assert_eq!(
            cell_pad_get_data(&mut m, &b, 1).unwrap().button[BTN_OFFSET_DIGITAL1],
            CTRL_DOWN,
        );
    }

    #[test]
    fn paddata_default_is_24_entries() {
        let d = PadData::default();
        assert_eq!(d.len, 24);
        assert_eq!(d.button.iter().all(|&b| b == 0), true);
    }
}
