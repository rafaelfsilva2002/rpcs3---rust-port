//! `rpcs3-hle-cellkb` — USB keyboard input HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellKb.cpp`. Games use this module
//! for on-screen keyboards, text entry, and debug console input.
//! The HLE tracks init state, per-port config (read mode, LED state,
//! key-repeat timers) and a rolling input buffer fed by a host
//! [`KbBackend`].
//!
//! ## Entry points covered
//!
//! | HLE function            | Rust wrapper              |
//! |-------------------------|---------------------------|
//! | `cellKbInit`            | [`cell_kb_init`]          |
//! | `cellKbEnd`             | [`cell_kb_end`]           |
//! | `cellKbGetInfo`         | [`cell_kb_get_info`]      |
//! | `cellKbRead`            | [`cell_kb_read`]          |
//! | `cellKbSetReadMode`     | [`cell_kb_set_read_mode`] |
//! | `cellKbClearBuf`        | [`cell_kb_clear_buf`]     |
//! | `cellKbSetLEDStatus`    | [`cell_kb_set_led_status`]|
//! | `cellKbGetConfiguration`| [`cell_kb_get_configuration`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellKb.h:6-14
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const FATAL: CellError = CellError(0x8012_1001);
    pub const INVALID_PARAMETER: CellError = CellError(0x8012_1002);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8012_1003);
    pub const UNINITIALIZED: CellError = CellError(0x8012_1004);
    pub const RESOURCE_ALLOCATION_FAILED: CellError = CellError(0x8012_1005);
    pub const READ_FAILED: CellError = CellError(0x8012_1006);
    pub const NO_DEVICE: CellError = CellError(0x8012_1007);
    pub const SYS_SETTING_FAILED: CellError = CellError(0x8012_1008);
}

// =====================================================================
// Constants (from Keyboard.h)
// =====================================================================

pub const MAX_KEYBOARDS: usize = 127;
pub const MAX_KEYCODES: usize = 62;

pub const RMODE_INPUTCHAR: u32 = 0;
pub const RMODE_PACKET: u32 = 1;

pub const LED_MODE_MANUAL: u32 = 0;
pub const LED_MODE_AUTO1: u32 = 1;
pub const LED_MODE_AUTO2: u32 = 2;

pub const MAPPING_101: u32 = 0;
pub const MAPPING_106: u32 = 1;
pub const MAPPING_106_KANA: u32 = 2;

pub const RAWDAT_BIT: u16 = 0x8000;
pub const KEYPAD_BIT: u16 = 0x4000;

/// LED bit flags.
pub const LED_NUMLOCK: u32 = 0x01;
pub const LED_CAPSLOCK: u32 = 0x02;
pub const LED_SCROLLLOCK: u32 = 0x04;
pub const LED_COMPOSE: u32 = 0x08;
pub const LED_KANA: u32 = 0x10;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KbInfo {
    pub max_connect: u32,
    pub now_connect: u32,
    pub info: u32,
    pub status: [u8; 8],
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KbData {
    /// Modifier keys (bit-set): bit 0..=3 = Ctrl/Shift/Alt/Win left,
    /// bit 4..=7 = same right.
    pub led: u32,
    pub mkey: u32,
    pub length: i32,
    /// Up to `MAX_KEYCODES` key codes, each 16-bit.
    pub keycode: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KbConfig {
    pub arrange: u32,
    pub read_mode: u32,
    pub code_type: u32,
}

impl Default for KbConfig {
    fn default() -> Self {
        Self {
            arrange: MAPPING_101,
            read_mode: RMODE_INPUTCHAR,
            code_type: 0,
        }
    }
}

// =====================================================================
// Backend trait
// =====================================================================

pub trait KbBackend {
    fn is_connected(&self, port: usize) -> bool;
    /// Current pressed key codes for a port (empty if nothing pressed).
    fn read(&mut self, port: usize) -> Vec<u16>;
    /// Current modifier key bitmask (see KbData.mkey).
    fn modifier(&self, port: usize) -> u32 { let _ = port; 0 }
    /// Current LED bitmask reported by the host keyboard.
    fn led(&self, port: usize) -> u32 { let _ = port; 0 }
}

/// Empty backend — no keyboard connected.
#[derive(Debug, Default)]
pub struct NullKbBackend;
impl KbBackend for NullKbBackend {
    fn is_connected(&self, _: usize) -> bool { false }
    fn read(&mut self, _: usize) -> Vec<u16> { Vec::new() }
}

/// In-memory scripted backend for tests. Attach a keyboard at port 0
/// and feed pre-canned keycode snapshots.
#[derive(Debug, Default)]
pub struct TestKbBackend {
    pub connected: Vec<bool>,
    pub queue: std::collections::VecDeque<Vec<u16>>,
    pub modifier: u32,
    pub led: u32,
}

impl TestKbBackend {
    pub fn with_keyboard_at(port: usize) -> Self {
        let mut v = vec![false; port + 1];
        v[port] = true;
        Self { connected: v, ..Default::default() }
    }
    pub fn push_frame(&mut self, keys: Vec<u16>) {
        self.queue.push_back(keys);
    }
}

impl KbBackend for TestKbBackend {
    fn is_connected(&self, port: usize) -> bool {
        self.connected.get(port).copied().unwrap_or(false)
    }
    fn read(&mut self, _port: usize) -> Vec<u16> {
        self.queue.pop_front().unwrap_or_default()
    }
    fn modifier(&self, _: usize) -> u32 { self.modifier }
    fn led(&self, _: usize) -> u32 { self.led }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug)]
pub struct KbManager {
    initialized: bool,
    configs: Vec<KbConfig>,
    led_state: Vec<u32>,
    /// When true, next read returns an empty frame.
    clear_latched: Vec<bool>,
}

impl Default for KbManager {
    fn default() -> Self {
        Self {
            initialized: false,
            configs: (0..MAX_KEYBOARDS).map(|_| KbConfig::default()).collect(),
            led_state: vec![0; MAX_KEYBOARDS],
            clear_latched: vec![false; MAX_KEYBOARDS],
        }
    }
}

// =====================================================================
// Validation
// =====================================================================

fn ensure_init(m: &KbManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::UNINITIALIZED) }
}

fn check_port(port: usize) -> Result<(), CellError> {
    if port < MAX_KEYBOARDS { Ok(()) } else { Err(errors::INVALID_PARAMETER) }
}

// =====================================================================
// Syscalls
// =====================================================================

/// `cellKbInit(max_connect)`.
#[must_use]
pub fn cell_kb_init(m: &mut KbManager, max_connect: u32) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::ALREADY_INITIALIZED);
    }
    if max_connect == 0 || max_connect as usize > MAX_KEYBOARDS {
        return Err(errors::INVALID_PARAMETER);
    }
    m.initialized = true;
    Ok(())
}

#[must_use]
pub fn cell_kb_end(m: &mut KbManager) -> Result<(), CellError> {
    ensure_init(m)?;
    *m = KbManager::default();
    Ok(())
}

/// `cellKbGetInfo(info_out)`.
#[must_use]
pub fn cell_kb_get_info<B: KbBackend + ?Sized>(
    m: &KbManager,
    backend: &B,
) -> Result<KbInfo, CellError> {
    ensure_init(m)?;
    let mut info = KbInfo::default();
    info.max_connect = MAX_KEYBOARDS as u32;
    for p in 0..MAX_KEYBOARDS.min(8) {
        if backend.is_connected(p) {
            info.status[p] = 1;
            info.now_connect += 1;
        }
    }
    Ok(info)
}

/// `cellKbRead(port, data_out)`.
#[must_use]
pub fn cell_kb_read<B: KbBackend + ?Sized>(
    m: &mut KbManager,
    backend: &mut B,
    port: usize,
) -> Result<KbData, CellError> {
    ensure_init(m)?;
    check_port(port)?;
    if !backend.is_connected(port) {
        return Err(errors::NO_DEVICE);
    }
    // ClearBuf latch: next read returns empty frame.
    if m.clear_latched[port] {
        m.clear_latched[port] = false;
        return Ok(KbData { led: backend.led(port), mkey: 0, length: 0, keycode: Vec::new() });
    }
    let keys = backend.read(port);
    let mut data = KbData {
        led: backend.led(port),
        mkey: backend.modifier(port),
        length: keys.len() as i32,
        keycode: keys,
    };
    if data.keycode.len() > MAX_KEYCODES {
        data.keycode.truncate(MAX_KEYCODES);
        data.length = MAX_KEYCODES as i32;
    }
    Ok(data)
}

/// `cellKbSetReadMode(port, mode)`.
#[must_use]
pub fn cell_kb_set_read_mode(
    m: &mut KbManager,
    port: usize,
    mode: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    check_port(port)?;
    if mode != RMODE_INPUTCHAR && mode != RMODE_PACKET {
        return Err(errors::INVALID_PARAMETER);
    }
    m.configs[port].read_mode = mode;
    Ok(())
}

/// `cellKbGetConfiguration(port, config_out)`.
#[must_use]
pub fn cell_kb_get_configuration(
    m: &KbManager,
    port: usize,
) -> Result<KbConfig, CellError> {
    ensure_init(m)?;
    check_port(port)?;
    Ok(m.configs[port])
}

/// `cellKbClearBuf(port)`.
#[must_use]
pub fn cell_kb_clear_buf(m: &mut KbManager, port: usize) -> Result<(), CellError> {
    ensure_init(m)?;
    check_port(port)?;
    m.clear_latched[port] = true;
    Ok(())
}

/// `cellKbSetLEDStatus(port, led_bitmask)`.
#[must_use]
pub fn cell_kb_set_led_status(
    m: &mut KbManager,
    port: usize,
    led: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    check_port(port)?;
    let allowed = LED_NUMLOCK | LED_CAPSLOCK | LED_SCROLLLOCK | LED_COMPOSE | LED_KANA;
    if led & !allowed != 0 {
        return Err(errors::INVALID_PARAMETER);
    }
    m.led_state[port] = led;
    Ok(())
}

/// Test helper: inspect stored LED bits.
#[must_use]
pub fn get_led_state(m: &KbManager, port: usize) -> Option<u32> {
    m.led_state.get(port).copied()
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> (KbManager, TestKbBackend) {
        let mut m = KbManager::default();
        cell_kb_init(&mut m, 4).unwrap();
        (m, TestKbBackend::with_keyboard_at(0))
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::FATAL.0, 0x8012_1001);
        assert_eq!(errors::INVALID_PARAMETER.0, 0x8012_1002);
        assert_eq!(errors::UNINITIALIZED.0, 0x8012_1004);
        assert_eq!(errors::NO_DEVICE.0, 0x8012_1007);
    }

    #[test]
    fn layout_constants_match_cpp() {
        assert_eq!(MAX_KEYBOARDS, 127);
        assert_eq!(MAX_KEYCODES, 62);
        assert_eq!(RMODE_INPUTCHAR, 0);
        assert_eq!(RMODE_PACKET, 1);
    }

    #[test]
    fn mapping_constants_match_cpp() {
        assert_eq!(MAPPING_101, 0);
        assert_eq!(MAPPING_106, 1);
        assert_eq!(MAPPING_106_KANA, 2);
    }

    // --- init / end ----------------------------------------------

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = KbManager::default();
        cell_kb_init(&mut m, 4).unwrap();
        assert_eq!(cell_kb_init(&mut m, 4).unwrap_err(), errors::ALREADY_INITIALIZED);
    }

    #[test]
    fn init_rejects_zero() {
        let mut m = KbManager::default();
        assert_eq!(cell_kb_init(&mut m, 0).unwrap_err(), errors::INVALID_PARAMETER);
    }

    #[test]
    fn init_rejects_over_max() {
        let mut m = KbManager::default();
        assert_eq!(
            cell_kb_init(&mut m, 200).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn end_without_init_is_uninitialized() {
        let mut m = KbManager::default();
        assert_eq!(cell_kb_end(&mut m).unwrap_err(), errors::UNINITIALIZED);
    }

    // --- get_info ------------------------------------------------

    #[test]
    fn info_reports_connected_keyboards() {
        let (m, b) = init();
        let info = cell_kb_get_info(&m, &b).unwrap();
        assert_eq!(info.now_connect, 1);
        assert_eq!(info.status[0], 1);
    }

    #[test]
    fn info_without_init_is_uninitialized() {
        let m = KbManager::default();
        assert_eq!(
            cell_kb_get_info(&m, &NullKbBackend).unwrap_err(),
            errors::UNINITIALIZED,
        );
    }

    // --- read ----------------------------------------------------

    #[test]
    fn read_on_disconnected_port_is_no_device() {
        let mut m = KbManager::default();
        cell_kb_init(&mut m, 4).unwrap();
        let mut b = NullKbBackend;
        assert_eq!(
            cell_kb_read(&mut m, &mut b, 0).unwrap_err(),
            errors::NO_DEVICE,
        );
    }

    #[test]
    fn read_out_of_range_is_invalid_parameter() {
        let (mut m, mut b) = init();
        assert_eq!(
            cell_kb_read(&mut m, &mut b, 999).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn read_returns_backend_keys() {
        let (mut m, mut b) = init();
        b.push_frame(vec![0x41, 0x42, 0x43]);  // ABC
        let data = cell_kb_read(&mut m, &mut b, 0).unwrap();
        assert_eq!(data.keycode, vec![0x41, 0x42, 0x43]);
        assert_eq!(data.length, 3);
    }

    #[test]
    fn read_truncates_oversized_frame_to_max_keycodes() {
        let (mut m, mut b) = init();
        let big = (0..100u16).collect();
        b.push_frame(big);
        let data = cell_kb_read(&mut m, &mut b, 0).unwrap();
        assert_eq!(data.keycode.len(), MAX_KEYCODES);
        assert_eq!(data.length, MAX_KEYCODES as i32);
    }

    #[test]
    fn read_returns_modifier_and_led_from_backend() {
        let (mut m, mut b) = init();
        b.modifier = 0x03;  // Ctrl+Shift
        b.led = LED_NUMLOCK;
        b.push_frame(vec![0x41]);
        let data = cell_kb_read(&mut m, &mut b, 0).unwrap();
        assert_eq!(data.mkey, 0x03);
        assert_eq!(data.led, LED_NUMLOCK);
    }

    // --- clear_buf latch -----------------------------------------

    #[test]
    fn clear_buf_forces_next_read_empty() {
        let (mut m, mut b) = init();
        b.push_frame(vec![0x41, 0x42]);
        cell_kb_clear_buf(&mut m, 0).unwrap();
        let data = cell_kb_read(&mut m, &mut b, 0).unwrap();
        assert_eq!(data.length, 0);
        // Subsequent read delivers the queued frame.
        let data = cell_kb_read(&mut m, &mut b, 0).unwrap();
        assert_eq!(data.keycode, vec![0x41, 0x42]);
    }

    #[test]
    fn clear_buf_bad_port_is_invalid_parameter() {
        let (mut m, _) = init();
        assert_eq!(
            cell_kb_clear_buf(&mut m, 999).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    // --- configuration / read_mode --------------------------------

    #[test]
    fn get_configuration_default_is_mapping_101_inputchar() {
        let (m, _) = init();
        let cfg = cell_kb_get_configuration(&m, 0).unwrap();
        assert_eq!(cfg.arrange, MAPPING_101);
        assert_eq!(cfg.read_mode, RMODE_INPUTCHAR);
    }

    #[test]
    fn set_read_mode_valid_accepts() {
        let (mut m, _) = init();
        cell_kb_set_read_mode(&mut m, 0, RMODE_PACKET).unwrap();
        assert_eq!(
            cell_kb_get_configuration(&m, 0).unwrap().read_mode,
            RMODE_PACKET,
        );
    }

    #[test]
    fn set_read_mode_invalid_rejected() {
        let (mut m, _) = init();
        assert_eq!(
            cell_kb_set_read_mode(&mut m, 0, 99).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    // --- LED ------------------------------------------------------

    #[test]
    fn set_led_status_all_flags_accepted() {
        let (mut m, _) = init();
        let all = LED_NUMLOCK | LED_CAPSLOCK | LED_SCROLLLOCK | LED_COMPOSE | LED_KANA;
        cell_kb_set_led_status(&mut m, 0, all).unwrap();
        assert_eq!(get_led_state(&m, 0), Some(all));
    }

    #[test]
    fn set_led_status_reserved_bit_is_invalid() {
        let (mut m, _) = init();
        assert_eq!(
            cell_kb_set_led_status(&mut m, 0, 0x100).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn set_led_status_per_port_isolation() {
        let (mut m, _) = init();
        cell_kb_set_led_status(&mut m, 0, LED_NUMLOCK).unwrap();
        cell_kb_set_led_status(&mut m, 1, LED_CAPSLOCK).unwrap();
        assert_eq!(get_led_state(&m, 0), Some(LED_NUMLOCK));
        assert_eq!(get_led_state(&m, 1), Some(LED_CAPSLOCK));
    }

    // --- full lifecycle ------------------------------------------

    #[test]
    fn full_lifecycle_init_read_end() {
        let mut m = KbManager::default();
        cell_kb_init(&mut m, 4).unwrap();
        let mut b = TestKbBackend::with_keyboard_at(0);
        b.push_frame(vec![0x20, 0x20]);  // two spaces
        let data = cell_kb_read(&mut m, &mut b, 0).unwrap();
        assert_eq!(data.length, 2);
        cell_kb_end(&mut m).unwrap();
    }
}
