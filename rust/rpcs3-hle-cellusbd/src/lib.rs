//! `rpcs3-hle-cellusbd` — USB device driver subsystem HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellUsbd.cpp`. The USBD runtime
//! manages **LDDs** (Logical Device Drivers) — game-registered
//! handlers for USB vendor/product IDs — and the **pipes** bound to
//! individual endpoints of a connected device. Games use this for
//! third-party peripherals: dance mats, guitars, cameras, webcams.
//!
//! ## Entry points covered
//!
//! | HLE function                  | Rust wrapper                    |
//! |-------------------------------|---------------------------------|
//! | `cellUsbdInit`                | [`cell_usbd_init`]              |
//! | `cellUsbdEnd`                 | [`cell_usbd_end`]               |
//! | `cellUsbdRegisterLdd`         | [`cell_usbd_register_ldd`]      |
//! | `cellUsbdUnregisterLdd`       | [`cell_usbd_unregister_ldd`]    |
//! | `cellUsbdOpenPipe`            | [`cell_usbd_open_pipe`]         |
//! | `cellUsbdClosePipe`           | [`cell_usbd_close_pipe`]        |
//! | `cellUsbdGetDeviceSpeed`      | [`cell_usbd_get_device_speed`]  |
//! | `cellUsbdGetDeviceDescriptor` | [`cell_usbd_get_device_descriptor`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellUsbd.h:7-25
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_INITIALIZED: CellError = CellError(0x8011_0001);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8011_0002);
    pub const NO_MEMORY: CellError = CellError(0x8011_0003);
    pub const INVALID_PARAM: CellError = CellError(0x8011_0004);
    pub const INVALID_TRANSFER_TYPE: CellError = CellError(0x8011_0005);
    pub const LDD_ALREADY_REGISTERED: CellError = CellError(0x8011_0006);
    pub const LDD_NOT_ALLOCATED: CellError = CellError(0x8011_0007);
    pub const LDD_NOT_RELEASED: CellError = CellError(0x8011_0008);
    pub const LDD_NOT_FOUND: CellError = CellError(0x8011_0009);
    pub const DEVICE_NOT_FOUND: CellError = CellError(0x8011_000A);
    pub const PIPE_NOT_ALLOCATED: CellError = CellError(0x8011_000B);
    pub const PIPE_NOT_RELEASED: CellError = CellError(0x8011_000C);
    pub const PIPE_NOT_FOUND: CellError = CellError(0x8011_000D);
    pub const IOREQ_NOT_ALLOCATED: CellError = CellError(0x8011_000E);
    pub const IOREQ_NOT_RELEASED: CellError = CellError(0x8011_000F);
    pub const IOREQ_NOT_FOUND: CellError = CellError(0x8011_0010);
    pub const CANNOT_GET_DESCRIPTOR: CellError = CellError(0x8011_0011);
    pub const FATAL: CellError = CellError(0x8011_00FF);
}

// =====================================================================
// Transfer completion codes
// =====================================================================

pub const HC_CC_NOERR: u32 = 0x0;
pub const EHCI_CC_MISSMF: u32 = 0x10;
pub const EHCI_CC_XACT: u32 = 0x20;
pub const EHCI_CC_BABBLE: u32 = 0x30;
pub const EHCI_CC_DATABUF: u32 = 0x40;
pub const EHCI_CC_HALTED: u32 = 0x50;

// =====================================================================
// Device speed + transfer type
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceSpeed {
    /// USB 1.1 low-speed (1.5 Mbit/s).
    Low,
    /// USB 1.1 full-speed (12 Mbit/s).
    Full,
    /// USB 2.0 (480 Mbit/s).
    High,
}

impl DeviceSpeed {
    #[must_use]
    pub fn as_u32(self) -> u32 {
        match self {
            Self::Low => 1,
            Self::Full => 2,
            Self::High => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferType {
    Control = 0,
    Isochronous = 1,
    Bulk = 2,
    Interrupt = 3,
}

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LddInfo {
    /// USB vendor id (e.g. 0x054C = Sony).
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
    /// Game-supplied name pointer (opaque u32).
    pub name_addr: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub vendor_id: u16,
    pub product_id: u16,
    pub speed: DeviceSpeed,
    pub class: u8,
    pub subclass: u8,
    pub protocol: u8,
    pub manufacturer: String,
    pub product: String,
}

// =====================================================================
// Backend trait — host USB enumeration source
// =====================================================================

pub trait UsbBackend {
    /// List devices currently attached to the host that match a given
    /// vendor/product id pair (product == 0 acts as wildcard).
    fn find_devices(&self, vendor: u16, product: u16) -> Vec<DeviceDescriptor>;

    /// Look up a specific device by its runtime handle.
    fn descriptor(&self, device_handle: u32) -> Option<DeviceDescriptor>;
}

/// Reference backend: exposes a fixed vector of devices.
#[derive(Debug, Default)]
pub struct FixedUsbBackend {
    pub devices: Vec<DeviceDescriptor>,
}

impl UsbBackend for FixedUsbBackend {
    fn find_devices(&self, vendor: u16, product: u16) -> Vec<DeviceDescriptor> {
        self.devices
            .iter()
            .filter(|d| d.vendor_id == vendor && (product == 0 || d.product_id == product))
            .cloned()
            .collect()
    }
    fn descriptor(&self, device_handle: u32) -> Option<DeviceDescriptor> {
        let idx = device_handle as usize;
        self.devices.get(idx).cloned()
    }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Default)]
pub struct UsbdManager {
    initialized: bool,
    next_ldd_id: u32,
    ldds: std::collections::BTreeMap<u32, LddInfo>,
    next_pipe_id: u32,
    pipes: std::collections::BTreeMap<u32, Pipe>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pipe {
    device_handle: u32,
    endpoint: u8,
    transfer_type: TransferType,
}

// =====================================================================
// Syscalls
// =====================================================================

fn ensure_init(m: &UsbdManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::NOT_INITIALIZED) }
}

/// `cellUsbdInit()`.
#[must_use]
pub fn cell_usbd_init(m: &mut UsbdManager) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::ALREADY_INITIALIZED);
    }
    m.initialized = true;
    Ok(())
}

/// `cellUsbdEnd()`.
#[must_use]
pub fn cell_usbd_end(m: &mut UsbdManager) -> Result<(), CellError> {
    ensure_init(m)?;
    // Must release all pipes and LDDs before End.
    if !m.pipes.is_empty() {
        return Err(errors::PIPE_NOT_RELEASED);
    }
    if !m.ldds.is_empty() {
        return Err(errors::LDD_NOT_RELEASED);
    }
    m.initialized = false;
    Ok(())
}

/// `cellUsbdRegisterLdd(name, params)`.
#[must_use]
pub fn cell_usbd_register_ldd(
    m: &mut UsbdManager,
    info: LddInfo,
) -> Result<u32, CellError> {
    ensure_init(m)?;
    if info.vendor_id == 0 && info.product_id == 0 {
        return Err(errors::INVALID_PARAM);
    }
    // Same vendor+product cannot be registered twice.
    if m.ldds.values().any(|l| l.vendor_id == info.vendor_id && l.product_id == info.product_id) {
        return Err(errors::LDD_ALREADY_REGISTERED);
    }
    m.next_ldd_id += 1;
    let id = m.next_ldd_id;
    m.ldds.insert(id, info);
    Ok(id)
}

/// `cellUsbdUnregisterLdd(handle)`.
#[must_use]
pub fn cell_usbd_unregister_ldd(m: &mut UsbdManager, ldd_id: u32) -> Result<(), CellError> {
    ensure_init(m)?;
    if m.ldds.remove(&ldd_id).is_none() {
        return Err(errors::LDD_NOT_FOUND);
    }
    Ok(())
}

/// `cellUsbdOpenPipe(device_handle, endpoint_descriptor)`.
#[must_use]
pub fn cell_usbd_open_pipe(
    m: &mut UsbdManager,
    device_handle: u32,
    endpoint: u8,
    transfer_type: TransferType,
) -> Result<u32, CellError> {
    ensure_init(m)?;
    if endpoint > 0x0F {
        // USB endpoint numbers are 4 bits.
        return Err(errors::INVALID_PARAM);
    }
    m.next_pipe_id += 1;
    let id = m.next_pipe_id;
    m.pipes.insert(id, Pipe { device_handle, endpoint, transfer_type });
    Ok(id)
}

/// `cellUsbdClosePipe(pipe_handle)`.
#[must_use]
pub fn cell_usbd_close_pipe(m: &mut UsbdManager, pipe_id: u32) -> Result<(), CellError> {
    ensure_init(m)?;
    if m.pipes.remove(&pipe_id).is_none() {
        return Err(errors::PIPE_NOT_FOUND);
    }
    Ok(())
}

/// `cellUsbdGetDeviceSpeed(device_handle)`.
#[must_use]
pub fn cell_usbd_get_device_speed<B: UsbBackend + ?Sized>(
    m: &UsbdManager,
    backend: &B,
    device_handle: u32,
) -> Result<u32, CellError> {
    ensure_init(m)?;
    let d = backend
        .descriptor(device_handle)
        .ok_or(errors::DEVICE_NOT_FOUND)?;
    Ok(d.speed.as_u32())
}

/// `cellUsbdGetDeviceDescriptor(device_handle)`.
#[must_use]
pub fn cell_usbd_get_device_descriptor<B: UsbBackend + ?Sized>(
    m: &UsbdManager,
    backend: &B,
    device_handle: u32,
) -> Result<DeviceDescriptor, CellError> {
    ensure_init(m)?;
    backend
        .descriptor(device_handle)
        .ok_or(errors::CANNOT_GET_DESCRIPTOR)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_device() -> DeviceDescriptor {
        DeviceDescriptor {
            vendor_id: 0x054C,
            product_id: 0x0268,  // DualShock 3
            speed: DeviceSpeed::Full,
            class: 0,
            subclass: 0,
            protocol: 0,
            manufacturer: "Sony".into(),
            product: "DualShock 3".into(),
        }
    }

    fn backend() -> FixedUsbBackend {
        FixedUsbBackend { devices: vec![test_device()] }
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::NOT_INITIALIZED.0, 0x8011_0001);
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8011_0002);
        assert_eq!(errors::LDD_ALREADY_REGISTERED.0, 0x8011_0006);
        assert_eq!(errors::DEVICE_NOT_FOUND.0, 0x8011_000A);
        assert_eq!(errors::FATAL.0, 0x8011_00FF);
    }

    #[test]
    fn tcc_constants_match_cpp() {
        assert_eq!(HC_CC_NOERR, 0x0);
        assert_eq!(EHCI_CC_MISSMF, 0x10);
        assert_eq!(EHCI_CC_XACT, 0x20);
        assert_eq!(EHCI_CC_BABBLE, 0x30);
        assert_eq!(EHCI_CC_DATABUF, 0x40);
        assert_eq!(EHCI_CC_HALTED, 0x50);
    }

    // --- init / end ----------------------------------------------

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        assert_eq!(cell_usbd_init(&mut m).unwrap_err(), errors::ALREADY_INITIALIZED);
    }

    #[test]
    fn end_without_init_is_not_initialized() {
        let mut m = UsbdManager::default();
        assert_eq!(cell_usbd_end(&mut m).unwrap_err(), errors::NOT_INITIALIZED);
    }

    #[test]
    fn end_with_open_pipes_is_pipe_not_released() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        cell_usbd_open_pipe(&mut m, 0, 1, TransferType::Bulk).unwrap();
        assert_eq!(cell_usbd_end(&mut m).unwrap_err(), errors::PIPE_NOT_RELEASED);
    }

    #[test]
    fn end_with_registered_ldds_is_ldd_not_released() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        cell_usbd_register_ldd(&mut m, LddInfo { vendor_id: 1, product_id: 2, name_addr: 0 }).unwrap();
        assert_eq!(cell_usbd_end(&mut m).unwrap_err(), errors::LDD_NOT_RELEASED);
    }

    // --- LDD lifecycle --------------------------------------------

    #[test]
    fn register_ldd_returns_handle() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let id = cell_usbd_register_ldd(
            &mut m,
            LddInfo { vendor_id: 0x054C, product_id: 0x0268, name_addr: 0x1000 },
        )
        .unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn register_ldd_duplicate_vid_pid_is_already_registered() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let info = LddInfo { vendor_id: 1, product_id: 2, name_addr: 0 };
        cell_usbd_register_ldd(&mut m, info).unwrap();
        assert_eq!(
            cell_usbd_register_ldd(&mut m, info).unwrap_err(),
            errors::LDD_ALREADY_REGISTERED,
        );
    }

    #[test]
    fn register_ldd_zero_ids_is_invalid_param() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let info = LddInfo { vendor_id: 0, product_id: 0, name_addr: 0 };
        assert_eq!(
            cell_usbd_register_ldd(&mut m, info).unwrap_err(),
            errors::INVALID_PARAM,
        );
    }

    #[test]
    fn unregister_unknown_ldd_is_not_found() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        assert_eq!(
            cell_usbd_unregister_ldd(&mut m, 999).unwrap_err(),
            errors::LDD_NOT_FOUND,
        );
    }

    #[test]
    fn unregister_frees_vid_pid_for_reuse() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let info = LddInfo { vendor_id: 1, product_id: 2, name_addr: 0 };
        let id = cell_usbd_register_ldd(&mut m, info).unwrap();
        cell_usbd_unregister_ldd(&mut m, id).unwrap();
        // Can register the same VID/PID again.
        cell_usbd_register_ldd(&mut m, info).unwrap();
    }

    // --- pipes ----------------------------------------------------

    #[test]
    fn open_pipe_round_trips() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let pipe = cell_usbd_open_pipe(&mut m, 0, 0x81, TransferType::Interrupt).unwrap_err();
        // endpoint 0x81 is > 0x0F → rejected.
        assert_eq!(pipe, errors::INVALID_PARAM);

        let pipe = cell_usbd_open_pipe(&mut m, 0, 1, TransferType::Bulk).unwrap();
        assert_eq!(pipe, 1);
        cell_usbd_close_pipe(&mut m, pipe).unwrap();
    }

    #[test]
    fn close_unknown_pipe_is_not_found() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        assert_eq!(
            cell_usbd_close_pipe(&mut m, 999).unwrap_err(),
            errors::PIPE_NOT_FOUND,
        );
    }

    // --- device queries -------------------------------------------

    #[test]
    fn get_device_speed_returns_backend_speed() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let b = backend();
        assert_eq!(
            cell_usbd_get_device_speed(&m, &b, 0).unwrap(),
            DeviceSpeed::Full.as_u32(),
        );
    }

    #[test]
    fn get_device_speed_unknown_device_is_not_found() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let b = backend();
        assert_eq!(
            cell_usbd_get_device_speed(&m, &b, 999).unwrap_err(),
            errors::DEVICE_NOT_FOUND,
        );
    }

    #[test]
    fn get_device_descriptor_happy_path() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let b = backend();
        let d = cell_usbd_get_device_descriptor(&m, &b, 0).unwrap();
        assert_eq!(d.vendor_id, 0x054C);
        assert_eq!(d.product, "DualShock 3");
    }

    #[test]
    fn get_device_descriptor_missing_is_cannot_get_descriptor() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let b = backend();
        assert_eq!(
            cell_usbd_get_device_descriptor(&m, &b, 42).unwrap_err(),
            errors::CANNOT_GET_DESCRIPTOR,
        );
    }

    // --- backend lookup -------------------------------------------

    #[test]
    fn fixed_backend_find_devices_exact_match() {
        let b = FixedUsbBackend {
            devices: vec![
                test_device(),
                DeviceDescriptor {
                    vendor_id: 0x12BA,
                    product_id: 0x0100,
                    ..test_device()
                },
            ],
        };
        let found = b.find_devices(0x054C, 0x0268);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].vendor_id, 0x054C);
    }

    #[test]
    fn fixed_backend_find_devices_product_wildcard() {
        let b = FixedUsbBackend {
            devices: vec![
                test_device(),
                DeviceDescriptor {
                    vendor_id: 0x054C,
                    product_id: 0x02EA,  // different Sony product
                    ..test_device()
                },
            ],
        };
        let found = b.find_devices(0x054C, 0);
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn device_speed_enum_to_u32_matches_expected() {
        assert_eq!(DeviceSpeed::Low.as_u32(), 1);
        assert_eq!(DeviceSpeed::Full.as_u32(), 2);
        assert_eq!(DeviceSpeed::High.as_u32(), 3);
    }

    // --- not-initialized gate ------------------------------------

    #[test]
    fn register_ldd_without_init_is_not_initialized() {
        let mut m = UsbdManager::default();
        let info = LddInfo { vendor_id: 1, product_id: 2, name_addr: 0 };
        assert_eq!(
            cell_usbd_register_ldd(&mut m, info).unwrap_err(),
            errors::NOT_INITIALIZED,
        );
    }

    #[test]
    fn open_pipe_without_init_is_not_initialized() {
        let mut m = UsbdManager::default();
        assert_eq!(
            cell_usbd_open_pipe(&mut m, 0, 1, TransferType::Bulk).unwrap_err(),
            errors::NOT_INITIALIZED,
        );
    }

    #[test]
    fn full_lifecycle_init_register_pipe_close_unregister_end() {
        let mut m = UsbdManager::default();
        cell_usbd_init(&mut m).unwrap();
        let ldd = cell_usbd_register_ldd(
            &mut m,
            LddInfo { vendor_id: 0x054C, product_id: 0x0268, name_addr: 0 },
        )
        .unwrap();
        let pipe = cell_usbd_open_pipe(&mut m, 0, 2, TransferType::Interrupt).unwrap();
        cell_usbd_close_pipe(&mut m, pipe).unwrap();
        cell_usbd_unregister_ldd(&mut m, ldd).unwrap();
        cell_usbd_end(&mut m).unwrap();
    }
}
