//! `rpcs3-hle-cellmouse` — USB mouse input HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellMouse.cpp`. Companion to
//! `rpcs3-hle-cellkb`: same skeleton but per-frame delta x/y + button
//! bitmap + wheel, instead of keycode arrays.
//!
//! ## Entry points covered
//!
//! | HLE function                  | Rust wrapper                |
//! |-------------------------------|-----------------------------|
//! | `cellMouseInit`               | [`cell_mouse_init`]         |
//! | `cellMouseEnd`                | [`cell_mouse_end`]          |
//! | `cellMouseGetInfo`            | [`cell_mouse_get_info`]     |
//! | `cellMouseGetData`            | [`cell_mouse_get_data`]     |
//! | `cellMouseGetDataList`        | [`cell_mouse_get_data_list`]|
//! | `cellMouseGetRawData`         | [`cell_mouse_get_raw_data`] |
//! | `cellMouseClearBuf`           | [`cell_mouse_clear_buf`]    |
//! | `cellMouseSetTabletRotation`  | [`cell_mouse_set_tablet_rotation`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellMouse.h:4-13
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const FATAL: CellError = CellError(0x8012_1201);
    pub const INVALID_PARAMETER: CellError = CellError(0x8012_1202);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8012_1203);
    pub const UNINITIALIZED: CellError = CellError(0x8012_1204);
    pub const RESOURCE_ALLOCATION_FAILED: CellError = CellError(0x8012_1205);
    pub const DATA_READ_FAILED: CellError = CellError(0x8012_1206);
    pub const NO_DEVICE: CellError = CellError(0x8012_1207);
    pub const SYS_SETTING_FAILED: CellError = CellError(0x8012_1208);
}

// =====================================================================
// Layout / button constants
// =====================================================================

pub const MAX_MICE: usize = 127;
pub const MAX_DATA_LIST_NUM: usize = 8;
pub const MAX_CODES: usize = 64;

/// Button bitmap (`MouseData.buttons`).
pub const BTN_LEFT: u8 = 0x01;
pub const BTN_RIGHT: u8 = 0x02;
pub const BTN_MIDDLE: u8 = 0x04;
pub const BTN_4: u8 = 0x08;
pub const BTN_5: u8 = 0x10;

pub const TABLET_ROTATION_0: u32 = 0;
pub const TABLET_ROTATION_90: u32 = 1;
pub const TABLET_ROTATION_180: u32 = 2;
pub const TABLET_ROTATION_270: u32 = 3;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseInfo {
    pub max_connect: u32,
    pub now_connect: u32,
    pub info: u32,
    pub vendor_id: [u16; 8],
    pub product_id: [u16; 8],
    pub status: [u8; 8],
}

/// One atomic mouse delta snapshot (matches `CellMouseData`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseData {
    /// Valid data length in buf (0..MAX_CODES).
    pub update: u32,
    pub buttons: u8,
    pub x_axis: i8,
    pub y_axis: i8,
    pub wheel: i8,
    pub tilt: i8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MouseDataList {
    pub list_num: u32,
    pub list: [MouseData; MAX_DATA_LIST_NUM],
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MouseRawData {
    pub len: i32,
    pub data: Vec<u8>,
}

// =====================================================================
// Backend
// =====================================================================

pub trait MouseBackend {
    fn is_connected(&self, port: usize) -> bool;
    fn vendor_id(&self, port: usize) -> u16 { let _ = port; 0 }
    fn product_id(&self, port: usize) -> u16 { let _ = port; 0 }
    /// Pop the next pending delta for `port`, or return `None`.
    fn pop_data(&mut self, port: usize) -> Option<MouseData>;
    /// HID raw data (advanced interfaces — optional).
    fn raw_data(&mut self, port: usize) -> MouseRawData {
        let _ = port;
        MouseRawData::default()
    }
}

#[derive(Debug, Default)]
pub struct NullMouseBackend;
impl MouseBackend for NullMouseBackend {
    fn is_connected(&self, _: usize) -> bool { false }
    fn pop_data(&mut self, _: usize) -> Option<MouseData> { None }
}

/// In-memory scripted backend for tests.
#[derive(Debug, Default)]
pub struct TestMouseBackend {
    pub connected: Vec<bool>,
    pub queue: std::collections::VecDeque<MouseData>,
    pub vendor: u16,
    pub product: u16,
    pub raw_queue: std::collections::VecDeque<Vec<u8>>,
}

impl TestMouseBackend {
    pub fn with_mouse_at(port: usize) -> Self {
        let mut v = vec![false; port + 1];
        v[port] = true;
        Self {
            connected: v,
            vendor: 0x045E,  // Microsoft
            product: 0x0084, // IntelliMouse
            ..Default::default()
        }
    }
}

impl MouseBackend for TestMouseBackend {
    fn is_connected(&self, port: usize) -> bool {
        self.connected.get(port).copied().unwrap_or(false)
    }
    fn vendor_id(&self, _: usize) -> u16 { self.vendor }
    fn product_id(&self, _: usize) -> u16 { self.product }
    fn pop_data(&mut self, _: usize) -> Option<MouseData> {
        self.queue.pop_front()
    }
    fn raw_data(&mut self, _: usize) -> MouseRawData {
        let bytes = self.raw_queue.pop_front().unwrap_or_default();
        MouseRawData { len: bytes.len() as i32, data: bytes }
    }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug)]
pub struct MouseManager {
    initialized: bool,
    clear_latched: Vec<bool>,
    tablet_rotation: Vec<u32>,
}

impl Default for MouseManager {
    fn default() -> Self {
        Self {
            initialized: false,
            clear_latched: vec![false; MAX_MICE],
            tablet_rotation: vec![TABLET_ROTATION_0; MAX_MICE],
        }
    }
}

fn ensure_init(m: &MouseManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::UNINITIALIZED) }
}

fn check_port(port: usize) -> Result<(), CellError> {
    if port < MAX_MICE { Ok(()) } else { Err(errors::INVALID_PARAMETER) }
}

// =====================================================================
// Syscalls
// =====================================================================

#[must_use]
pub fn cell_mouse_init(m: &mut MouseManager, max_connect: u32) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::ALREADY_INITIALIZED);
    }
    if max_connect == 0 || max_connect as usize > MAX_MICE {
        return Err(errors::INVALID_PARAMETER);
    }
    m.initialized = true;
    Ok(())
}

#[must_use]
pub fn cell_mouse_end(m: &mut MouseManager) -> Result<(), CellError> {
    ensure_init(m)?;
    *m = MouseManager::default();
    Ok(())
}

#[must_use]
pub fn cell_mouse_get_info<B: MouseBackend + ?Sized>(
    m: &MouseManager,
    backend: &B,
) -> Result<MouseInfo, CellError> {
    ensure_init(m)?;
    let mut info = MouseInfo::default();
    info.max_connect = MAX_MICE as u32;
    for p in 0..8 {
        if backend.is_connected(p) {
            info.status[p] = 1;
            info.vendor_id[p] = backend.vendor_id(p);
            info.product_id[p] = backend.product_id(p);
            info.now_connect += 1;
        }
    }
    Ok(info)
}

#[must_use]
pub fn cell_mouse_get_data<B: MouseBackend + ?Sized>(
    m: &mut MouseManager,
    backend: &mut B,
    port: usize,
) -> Result<MouseData, CellError> {
    ensure_init(m)?;
    check_port(port)?;
    if !backend.is_connected(port) {
        return Err(errors::NO_DEVICE);
    }
    if m.clear_latched[port] {
        m.clear_latched[port] = false;
        return Ok(MouseData::default());
    }
    let mut data = backend.pop_data(port).unwrap_or_default();
    // Apply tablet rotation to delta X/Y.
    let rot = m.tablet_rotation[port];
    if rot != TABLET_ROTATION_0 {
        let (x, y) = (data.x_axis, data.y_axis);
        match rot {
            TABLET_ROTATION_90 => { data.x_axis = -y; data.y_axis = x; }
            TABLET_ROTATION_180 => { data.x_axis = -x; data.y_axis = -y; }
            TABLET_ROTATION_270 => { data.x_axis = y; data.y_axis = -x; }
            _ => {}
        }
    }
    data.update = 1;
    Ok(data)
}

/// `cellMouseGetDataList(port, list_out)` — drains up to
/// `MAX_DATA_LIST_NUM` queued snapshots at once.
#[must_use]
pub fn cell_mouse_get_data_list<B: MouseBackend + ?Sized>(
    m: &mut MouseManager,
    backend: &mut B,
    port: usize,
) -> Result<MouseDataList, CellError> {
    ensure_init(m)?;
    check_port(port)?;
    if !backend.is_connected(port) {
        return Err(errors::NO_DEVICE);
    }
    let mut list = MouseDataList::default();
    if m.clear_latched[port] {
        m.clear_latched[port] = false;
        return Ok(list);
    }
    for i in 0..MAX_DATA_LIST_NUM {
        if let Some(d) = backend.pop_data(port) {
            list.list[i] = d;
            list.list[i].update = 1;
            list.list_num += 1;
        } else {
            break;
        }
    }
    Ok(list)
}

#[must_use]
pub fn cell_mouse_get_raw_data<B: MouseBackend + ?Sized>(
    m: &MouseManager,
    backend: &mut B,
    port: usize,
) -> Result<MouseRawData, CellError> {
    ensure_init(m)?;
    check_port(port)?;
    if !backend.is_connected(port) {
        return Err(errors::NO_DEVICE);
    }
    Ok(backend.raw_data(port))
}

#[must_use]
pub fn cell_mouse_clear_buf(m: &mut MouseManager, port: usize) -> Result<(), CellError> {
    ensure_init(m)?;
    check_port(port)?;
    m.clear_latched[port] = true;
    Ok(())
}

#[must_use]
pub fn cell_mouse_set_tablet_rotation(
    m: &mut MouseManager,
    port: usize,
    rotation: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    check_port(port)?;
    if !matches!(rotation, TABLET_ROTATION_0 | TABLET_ROTATION_90 | TABLET_ROTATION_180 | TABLET_ROTATION_270) {
        return Err(errors::INVALID_PARAMETER);
    }
    m.tablet_rotation[port] = rotation;
    Ok(())
}

/// Test helper.
#[must_use]
pub fn get_tablet_rotation(m: &MouseManager, port: usize) -> Option<u32> {
    m.tablet_rotation.get(port).copied()
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> (MouseManager, TestMouseBackend) {
        let mut m = MouseManager::default();
        cell_mouse_init(&mut m, 4).unwrap();
        (m, TestMouseBackend::with_mouse_at(0))
    }

    fn data(x: i8, y: i8, buttons: u8) -> MouseData {
        MouseData { update: 0, buttons, x_axis: x, y_axis: y, wheel: 0, tilt: 0 }
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::FATAL.0, 0x8012_1201);
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8012_1203);
        assert_eq!(errors::NO_DEVICE.0, 0x8012_1207);
    }

    #[test]
    fn layout_constants_match() {
        assert_eq!(MAX_MICE, 127);
        assert_eq!(MAX_DATA_LIST_NUM, 8);
        assert_eq!(MAX_CODES, 64);
    }

    #[test]
    fn button_bitmap_values() {
        assert_eq!(BTN_LEFT, 0x01);
        assert_eq!(BTN_RIGHT, 0x02);
        assert_eq!(BTN_MIDDLE, 0x04);
        assert_eq!(BTN_4, 0x08);
        assert_eq!(BTN_5, 0x10);
    }

    // --- init / end ---------------------------------------------

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = MouseManager::default();
        cell_mouse_init(&mut m, 4).unwrap();
        assert_eq!(cell_mouse_init(&mut m, 4).unwrap_err(), errors::ALREADY_INITIALIZED);
    }

    #[test]
    fn init_rejects_zero_and_over_max() {
        let mut m = MouseManager::default();
        assert_eq!(cell_mouse_init(&mut m, 0).unwrap_err(), errors::INVALID_PARAMETER);
        assert_eq!(cell_mouse_init(&mut m, 200).unwrap_err(), errors::INVALID_PARAMETER);
    }

    #[test]
    fn end_without_init_is_uninitialized() {
        let mut m = MouseManager::default();
        assert_eq!(cell_mouse_end(&mut m).unwrap_err(), errors::UNINITIALIZED);
    }

    // --- info -----------------------------------------------------

    #[test]
    fn get_info_reports_vendor_product_for_connected_port() {
        let (m, b) = init();
        let info = cell_mouse_get_info(&m, &b).unwrap();
        assert_eq!(info.now_connect, 1);
        assert_eq!(info.vendor_id[0], 0x045E);
        assert_eq!(info.product_id[0], 0x0084);
    }

    // --- get_data ------------------------------------------------

    #[test]
    fn get_data_on_disconnected_port_is_no_device() {
        let mut m = MouseManager::default();
        cell_mouse_init(&mut m, 4).unwrap();
        let mut b = NullMouseBackend;
        assert_eq!(
            cell_mouse_get_data(&mut m, &mut b, 0).unwrap_err(),
            errors::NO_DEVICE,
        );
    }

    #[test]
    fn get_data_returns_backend_frame() {
        let (mut m, mut b) = init();
        b.queue.push_back(data(5, -3, BTN_LEFT));
        let d = cell_mouse_get_data(&mut m, &mut b, 0).unwrap();
        assert_eq!(d.x_axis, 5);
        assert_eq!(d.y_axis, -3);
        assert_eq!(d.buttons, BTN_LEFT);
        assert_eq!(d.update, 1);
    }

    #[test]
    fn get_data_empty_queue_returns_zero_frame() {
        let (mut m, mut b) = init();
        let d = cell_mouse_get_data(&mut m, &mut b, 0).unwrap();
        assert_eq!(d.x_axis, 0);
        assert_eq!(d.y_axis, 0);
        assert_eq!(d.buttons, 0);
    }

    // --- tablet rotation -----------------------------------------

    #[test]
    fn tablet_rotation_90_swaps_and_negates_x() {
        let (mut m, mut b) = init();
        cell_mouse_set_tablet_rotation(&mut m, 0, TABLET_ROTATION_90).unwrap();
        b.queue.push_back(data(10, 20, 0));
        let d = cell_mouse_get_data(&mut m, &mut b, 0).unwrap();
        assert_eq!(d.x_axis, -20);
        assert_eq!(d.y_axis, 10);
    }

    #[test]
    fn tablet_rotation_180_negates_both() {
        let (mut m, mut b) = init();
        cell_mouse_set_tablet_rotation(&mut m, 0, TABLET_ROTATION_180).unwrap();
        b.queue.push_back(data(7, -4, 0));
        let d = cell_mouse_get_data(&mut m, &mut b, 0).unwrap();
        assert_eq!(d.x_axis, -7);
        assert_eq!(d.y_axis, 4);
    }

    #[test]
    fn tablet_rotation_270_inverse_of_90() {
        let (mut m, mut b) = init();
        cell_mouse_set_tablet_rotation(&mut m, 0, TABLET_ROTATION_270).unwrap();
        b.queue.push_back(data(10, 20, 0));
        let d = cell_mouse_get_data(&mut m, &mut b, 0).unwrap();
        assert_eq!(d.x_axis, 20);
        assert_eq!(d.y_axis, -10);
    }

    #[test]
    fn tablet_rotation_bad_value_invalid() {
        let (mut m, _) = init();
        assert_eq!(
            cell_mouse_set_tablet_rotation(&mut m, 0, 99).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn tablet_rotation_per_port_isolation() {
        let (mut m, _) = init();
        cell_mouse_set_tablet_rotation(&mut m, 0, TABLET_ROTATION_90).unwrap();
        cell_mouse_set_tablet_rotation(&mut m, 1, TABLET_ROTATION_180).unwrap();
        assert_eq!(get_tablet_rotation(&m, 0), Some(TABLET_ROTATION_90));
        assert_eq!(get_tablet_rotation(&m, 1), Some(TABLET_ROTATION_180));
    }

    // --- data_list ------------------------------------------------

    #[test]
    fn get_data_list_drains_up_to_8() {
        let (mut m, mut b) = init();
        for i in 0..10 {
            b.queue.push_back(data(i, 0, 0));
        }
        let list = cell_mouse_get_data_list(&mut m, &mut b, 0).unwrap();
        assert_eq!(list.list_num, MAX_DATA_LIST_NUM as u32);
        assert_eq!(list.list[0].x_axis, 0);
        assert_eq!(list.list[7].x_axis, 7);
    }

    #[test]
    fn get_data_list_empty_returns_list_num_zero() {
        let (mut m, mut b) = init();
        let list = cell_mouse_get_data_list(&mut m, &mut b, 0).unwrap();
        assert_eq!(list.list_num, 0);
    }

    // --- clear_buf -----------------------------------------------

    #[test]
    fn clear_buf_next_get_data_is_empty() {
        let (mut m, mut b) = init();
        b.queue.push_back(data(5, 5, BTN_LEFT));
        cell_mouse_clear_buf(&mut m, 0).unwrap();
        let d = cell_mouse_get_data(&mut m, &mut b, 0).unwrap();
        assert_eq!(d.x_axis, 0);
        assert_eq!(d.buttons, 0);
        // Next read delivers queued frame.
        let d = cell_mouse_get_data(&mut m, &mut b, 0).unwrap();
        assert_eq!(d.x_axis, 5);
    }

    #[test]
    fn clear_buf_applies_to_data_list_too() {
        let (mut m, mut b) = init();
        b.queue.push_back(data(1, 1, 0));
        b.queue.push_back(data(2, 2, 0));
        cell_mouse_clear_buf(&mut m, 0).unwrap();
        let list = cell_mouse_get_data_list(&mut m, &mut b, 0).unwrap();
        assert_eq!(list.list_num, 0);
    }

    // --- raw data -------------------------------------------------

    #[test]
    fn get_raw_data_returns_backend_queue() {
        let (mut m, mut b) = init();
        b.raw_queue.push_back(vec![0x01, 0x02, 0x03, 0x04]);
        let r = cell_mouse_get_raw_data(&m, &mut b, 0).unwrap();
        assert_eq!(r.len, 4);
        assert_eq!(r.data, vec![0x01, 0x02, 0x03, 0x04]);
    }

    #[test]
    fn get_raw_data_on_null_backend_is_no_device() {
        let mut m = MouseManager::default();
        cell_mouse_init(&mut m, 4).unwrap();
        let mut b = NullMouseBackend;
        assert_eq!(
            cell_mouse_get_raw_data(&m, &mut b, 0).unwrap_err(),
            errors::NO_DEVICE,
        );
    }

    // --- bad port / not init --------------------------------------

    #[test]
    fn bad_port_on_all_ops_is_invalid_parameter() {
        let (mut m, mut b) = init();
        assert_eq!(cell_mouse_get_data(&mut m, &mut b, 999).unwrap_err(), errors::INVALID_PARAMETER);
        assert_eq!(cell_mouse_get_data_list(&mut m, &mut b, 999).unwrap_err(), errors::INVALID_PARAMETER);
        assert_eq!(cell_mouse_clear_buf(&mut m, 999).unwrap_err(), errors::INVALID_PARAMETER);
        assert_eq!(cell_mouse_set_tablet_rotation(&mut m, 999, 0).unwrap_err(), errors::INVALID_PARAMETER);
    }

    #[test]
    fn ops_without_init_are_uninitialized() {
        let mut m = MouseManager::default();
        let b = TestMouseBackend::with_mouse_at(0);
        assert_eq!(cell_mouse_get_info(&m, &b).unwrap_err(), errors::UNINITIALIZED);
    }

    // --- lifecycle -----------------------------------------------

    #[test]
    fn full_lifecycle_init_get_data_end() {
        let mut m = MouseManager::default();
        cell_mouse_init(&mut m, 4).unwrap();
        let mut b = TestMouseBackend::with_mouse_at(0);
        b.queue.push_back(data(1, 2, BTN_LEFT));
        cell_mouse_get_data(&mut m, &mut b, 0).unwrap();
        cell_mouse_end(&mut m).unwrap();
    }
}
