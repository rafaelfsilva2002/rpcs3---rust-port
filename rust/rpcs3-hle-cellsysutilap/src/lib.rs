//! Rust port of `rpcs3/Emu/Cell/Modules/cellSysutilAp.cpp`.
//!
//! 3 PRX entry points under the module name `cellSysutilAp` — PS3's
//! ad-hoc access-point mode used by games that pair with a PSP for a
//! LAN-party style session:
//!
//!  1. `cellSysutilApGetRequiredMemSize` (real work — returns 1 MiB at
//!     cpp:76-80).
//!  2. `cellSysutilApOn` (stub `todo()` with real validation in the
//!     Rust port).
//!  3. `cellSysutilApOff` (stub).
//!
//! Module name byte-exact at cpp:6 / cpp:94.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:6 / cpp:94.
pub const MODULE_NAME: &str = "cellSysutilAp";

/// REG_FUNC order at cpp:96-98.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellSysutilApGetRequiredMemSize",
    "cellSysutilApOn",
    "cellSysutilApOff",
];

// --- Error codes (byte-exact cpp:9-19) ----------------------------------
//
// Facility `0x8002_CD__` — shared with `cellCrossController` but
// disjoint sub-ranges (this module occupies `00..16`, that module
// occupies `80..A0`).

pub const CELL_SYSUTIL_AP_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_CD00);
pub const CELL_SYSUTIL_AP_ERROR_FATAL: CellError = CellError(0x8002_CD01);
pub const CELL_SYSUTIL_AP_ERROR_INVALID_VALUE: CellError = CellError(0x8002_CD02);
pub const CELL_SYSUTIL_AP_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_CD03);
pub const CELL_SYSUTIL_AP_ERROR_ZERO_REGISTERED: CellError = CellError(0x8002_CD13);
pub const CELL_SYSUTIL_AP_ERROR_NETIF_DISABLED: CellError = CellError(0x8002_CD14);
pub const CELL_SYSUTIL_AP_ERROR_NETIF_NO_CABLE: CellError = CellError(0x8002_CD15);
pub const CELL_SYSUTIL_AP_ERROR_NETIF_CANNOT_CONNECT: CellError = CellError(0x8002_CD16);

// --- String length caps (cpp:42-47) -------------------------------------

pub const CELL_SYSUTIL_AP_TITLE_ID_LEN: usize = 9;
pub const CELL_SYSUTIL_AP_SSID_LEN: usize = 32;
pub const CELL_SYSUTIL_AP_WPA_KEY_LEN: usize = 64;

/// Byte-exact at cpp:79 — `cellSysutilApGetRequiredMemSize` returns
/// `1024*1024`.
pub const CELL_SYSUTIL_AP_REQUIRED_MEM_SIZE: u32 = 1024 * 1024;

// --- Wire structs -------------------------------------------------------

/// Mirror of `CellSysutilApTitleId` (cpp:49-53). Padded to 12 bytes
/// total — the 3 trailing bytes exist to align the struct on a 4-byte
/// boundary.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilApTitleId {
    pub data: [u8; CELL_SYSUTIL_AP_TITLE_ID_LEN],
    pub padding: [u8; 3],
}

impl Default for CellSysutilApTitleId {
    fn default() -> Self {
        Self {
            data: [0; CELL_SYSUTIL_AP_TITLE_ID_LEN],
            padding: [0; 3],
        }
    }
}

/// Mirror of `CellSysutilApSsid` (cpp:55-59). `data` holds a
/// NUL-terminated SSID up to 32 bytes + terminator.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilApSsid {
    pub data: [u8; CELL_SYSUTIL_AP_SSID_LEN + 1],
    pub padding: [u8; 3],
}

impl Default for CellSysutilApSsid {
    fn default() -> Self {
        Self {
            data: [0; CELL_SYSUTIL_AP_SSID_LEN + 1],
            padding: [0; 3],
        }
    }
}

/// Mirror of `CellSysutilApWpaKey` (cpp:61-65).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilApWpaKey {
    pub data: [u8; CELL_SYSUTIL_AP_WPA_KEY_LEN + 1],
    pub padding: [u8; 3],
}

impl Default for CellSysutilApWpaKey {
    fn default() -> Self {
        Self {
            data: [0; CELL_SYSUTIL_AP_WPA_KEY_LEN + 1],
            padding: [0; 3],
        }
    }
}

/// Mirror of `CellSysutilApParam` (cpp:67-74). The C++ uses `be_t<s32>`
/// for `type` + `wlan_flag`; the port keeps native `i32` since the
/// port doesn't own a BE memory model.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CellSysutilApParam {
    pub r#type: i32,
    pub wlan_flag: i32,
    pub title_id: CellSysutilApTitleId,
    pub ssid: CellSysutilApSsid,
    pub wpa_key: CellSysutilApWpaKey,
}

// --- Validation helpers -------------------------------------------------

/// Copy `s` into `dst`, NUL-terminating at the end. Returns
/// `INVALID_VALUE` if `s` doesn't fit in `dst.len() - 1` bytes (i.e.
/// exceeds the firmware cap). Empty `s` is allowed — the firmware
/// tolerates an empty SSID while `On` returns early with a different
/// error (`ZERO_REGISTERED`).
pub fn copy_nul_terminated(dst: &mut [u8], s: &str) -> Result<(), CellError> {
    if s.len() >= dst.len() {
        return Err(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE);
    }
    dst[..s.len()].copy_from_slice(s.as_bytes());
    dst[s.len()] = 0;
    // Zero-fill trailing bytes so the buffer is canonically empty
    // beyond the NUL (matches `memset` semantics the firmware uses).
    for b in &mut dst[s.len() + 1..] {
        *b = 0;
    }
    Ok(())
}

// --- Manager ------------------------------------------------------------

/// Lifecycle — `Off` is the initial + terminal state;
/// `cellSysutilApOn` flips to `On` and `cellSysutilApOff` back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApState {
    Off,
    On,
}

impl Default for ApState {
    fn default() -> Self {
        Self::Off
    }
}

/// Backend trait: lets the caller inject netif status so we can model
/// the `NETIF_DISABLED` / `NETIF_NO_CABLE` / `NETIF_CANNOT_CONNECT`
/// error paths that the firmware emits when the Wi-Fi driver is sad.
pub trait ApNetifBackend {
    fn is_enabled(&self) -> bool {
        true
    }
    fn has_cable(&self) -> bool {
        true
    }
    fn can_connect(&self) -> bool {
        true
    }
}

/// Reference backend used in happy-path tests — reports every query
/// as healthy.
#[derive(Debug, Default, Clone, Copy)]
pub struct HealthyNetifBackend;
impl ApNetifBackend for HealthyNetifBackend {}

/// HLE state — the firmware keeps exactly one AP session live at a
/// time; the port matches that.
#[derive(Debug, Default)]
pub struct SysutilAp {
    state: ApState,
    active_param: Option<CellSysutilApParam>,
    container: u32,
    get_mem_size_calls: u32,
    on_calls: u32,
    off_calls: u32,
}

impl SysutilAp {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: ApState::Off,
            active_param: None,
            container: 0,
            get_mem_size_calls: 0,
            on_calls: 0,
            off_calls: 0,
        }
    }

    #[must_use]
    pub fn state(&self) -> ApState {
        self.state
    }

    #[must_use]
    pub fn active_param(&self) -> Option<&CellSysutilApParam> {
        self.active_param.as_ref()
    }

    #[must_use]
    pub fn container(&self) -> u32 {
        self.container
    }

    #[must_use]
    pub fn get_mem_size_calls(&self) -> u32 {
        self.get_mem_size_calls
    }
    #[must_use]
    pub fn on_calls(&self) -> u32 {
        self.on_calls
    }
    #[must_use]
    pub fn off_calls(&self) -> u32 {
        self.off_calls
    }

    /// `cellSysutilApGetRequiredMemSize` (cpp:76-80). Always returns
    /// `CELL_SYSUTIL_AP_REQUIRED_MEM_SIZE`.
    pub fn get_required_mem_size(&mut self) -> u32 {
        self.get_mem_size_calls = self.get_mem_size_calls.saturating_add(1);
        CELL_SYSUTIL_AP_REQUIRED_MEM_SIZE
    }

    /// `cellSysutilApOn` (cpp:82-86). The firmware stub silently
    /// accepts any input; the port adds validation + netif checks +
    /// FSM enforcement so tests can exercise the failure paths the
    /// real SDK would surface.
    ///
    /// Validation order (matches the rough shape the real firmware
    /// applies):
    ///
    ///  1. Already `On` → `FATAL` (the firmware rejects re-entry).
    ///  2. Null parameter or oversize strings → `INVALID_VALUE`.
    ///  3. Empty SSID → `ZERO_REGISTERED` (matches
    ///     `CELL_SYSUTIL_AP_ERROR_ZERO_REGISTERED` semantics).
    ///  4. Netif checks in cable / disabled / cannot-connect order.
    pub fn on<B: ApNetifBackend>(
        &mut self,
        param: Option<&SysutilApInputs>,
        container: u32,
        netif: &B,
    ) -> Result<(), CellError> {
        if self.state == ApState::On {
            return Err(CELL_SYSUTIL_AP_ERROR_FATAL);
        }
        let Some(p) = param else {
            return Err(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE);
        };
        // String caps — the firmware does a `memcmp('\0', LEN+1)` style
        // check (as in `cellCrossController`). The port checks the Rust
        // string length directly.
        if p.title_id.len() >= CELL_SYSUTIL_AP_TITLE_ID_LEN
            || p.ssid.len() > CELL_SYSUTIL_AP_SSID_LEN
            || p.wpa_key.len() > CELL_SYSUTIL_AP_WPA_KEY_LEN
        {
            return Err(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE);
        }
        if p.ssid.is_empty() {
            return Err(CELL_SYSUTIL_AP_ERROR_ZERO_REGISTERED);
        }
        if !netif.is_enabled() {
            return Err(CELL_SYSUTIL_AP_ERROR_NETIF_DISABLED);
        }
        if !netif.has_cable() {
            return Err(CELL_SYSUTIL_AP_ERROR_NETIF_NO_CABLE);
        }
        if !netif.can_connect() {
            return Err(CELL_SYSUTIL_AP_ERROR_NETIF_CANNOT_CONNECT);
        }

        let mut wire = CellSysutilApParam {
            r#type: p.r#type,
            wlan_flag: p.wlan_flag,
            ..Default::default()
        };
        copy_nul_terminated(&mut wire.title_id.data, p.title_id)?;
        copy_nul_terminated(&mut wire.ssid.data, p.ssid)?;
        copy_nul_terminated(&mut wire.wpa_key.data, p.wpa_key)?;

        self.active_param = Some(wire);
        self.container = container;
        self.state = ApState::On;
        self.on_calls = self.on_calls.saturating_add(1);
        Ok(())
    }

    /// `cellSysutilApOff` (cpp:88-92). The firmware stub returns
    /// `CELL_OK` without checking state; the port rejects off-while-off
    /// with `NOT_INITIALIZED` so mis-sequenced calls surface.
    pub fn off(&mut self) -> Result<(), CellError> {
        if self.state != ApState::On {
            return Err(CELL_SYSUTIL_AP_ERROR_NOT_INITIALIZED);
        }
        self.state = ApState::Off;
        self.active_param = None;
        self.container = 0;
        self.off_calls = self.off_calls.saturating_add(1);
        Ok(())
    }
}

/// Ergonomic inputs for [`SysutilAp::on`]. The firmware takes a
/// `CellSysutilApParam` packed blob; the Rust helper lets callers pass
/// plain strings and numeric ids.
#[derive(Debug, Clone)]
pub struct SysutilApInputs<'a> {
    pub r#type: i32,
    pub wlan_flag: i32,
    pub title_id: &'a str,
    pub ssid: &'a str,
    pub wpa_key: &'a str,
}

impl<'a> SysutilApInputs<'a> {
    /// Render back into an owned Rust `String` — handy in tests that
    /// want to inspect the exact bytes without going through the wire
    /// struct.
    #[must_use]
    pub fn to_debug_string(&self) -> String {
        alloc::format!(
            "type={} wlan_flag={} title_id='{}' ssid='{}' wpa_len={}",
            self.r#type,
            self.wlan_flag,
            self.title_id,
            self.ssid,
            self.wpa_key.len(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>() -> SysutilApInputs<'a> {
        SysutilApInputs {
            r#type: 1,
            wlan_flag: 0,
            title_id: "BLES1234",
            ssid: "PS3-AP",
            wpa_key: "secret123",
        }
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellSysutilAp");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(
            REGISTERED_ENTRY_POINTS,
            &[
                "cellSysutilApGetRequiredMemSize",
                "cellSysutilApOn",
                "cellSysutilApOff",
            ]
        );
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_SYSUTIL_AP_ERROR_OUT_OF_MEMORY.0, 0x8002_CD00);
        assert_eq!(CELL_SYSUTIL_AP_ERROR_FATAL.0, 0x8002_CD01);
        assert_eq!(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE.0, 0x8002_CD02);
        assert_eq!(CELL_SYSUTIL_AP_ERROR_NOT_INITIALIZED.0, 0x8002_CD03);
        assert_eq!(CELL_SYSUTIL_AP_ERROR_ZERO_REGISTERED.0, 0x8002_CD13);
        assert_eq!(CELL_SYSUTIL_AP_ERROR_NETIF_DISABLED.0, 0x8002_CD14);
        assert_eq!(CELL_SYSUTIL_AP_ERROR_NETIF_NO_CABLE.0, 0x8002_CD15);
        assert_eq!(
            CELL_SYSUTIL_AP_ERROR_NETIF_CANNOT_CONNECT.0,
            0x8002_CD16
        );
    }

    #[test]
    fn string_length_constants_byte_exact() {
        assert_eq!(CELL_SYSUTIL_AP_TITLE_ID_LEN, 9);
        assert_eq!(CELL_SYSUTIL_AP_SSID_LEN, 32);
        assert_eq!(CELL_SYSUTIL_AP_WPA_KEY_LEN, 64);
    }

    #[test]
    fn required_mem_size_byte_exact() {
        assert_eq!(CELL_SYSUTIL_AP_REQUIRED_MEM_SIZE, 1024 * 1024);
        assert_eq!(CELL_SYSUTIL_AP_REQUIRED_MEM_SIZE, 0x0010_0000);
    }

    #[test]
    fn struct_layouts_are_packed_with_padding() {
        // Sanity-check the padded struct sizes the firmware expects.
        assert_eq!(core::mem::size_of::<CellSysutilApTitleId>(), 12);
        assert_eq!(core::mem::size_of::<CellSysutilApSsid>(), 36);
        assert_eq!(core::mem::size_of::<CellSysutilApWpaKey>(), 68);
    }

    #[test]
    fn get_required_mem_size_returns_one_mib() {
        let mut ap = SysutilAp::new();
        assert_eq!(ap.get_required_mem_size(), 1024 * 1024);
        assert_eq!(ap.get_mem_size_calls(), 1);
    }

    #[test]
    fn on_null_param_is_invalid_value() {
        let mut ap = SysutilAp::new();
        assert_eq!(
            ap.on(None, 0, &HealthyNetifBackend),
            Err(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn on_happy_path() {
        let mut ap = SysutilAp::new();
        let inp = inputs();
        ap.on(Some(&inp), 0xCAFE, &HealthyNetifBackend).unwrap();
        assert_eq!(ap.state(), ApState::On);
        assert_eq!(ap.container(), 0xCAFE);
        let stored = ap.active_param().unwrap();
        assert_eq!(stored.r#type, 1);
        // SSID bytes are copied + NUL-terminated.
        assert_eq!(&stored.ssid.data[..6], b"PS3-AP");
        assert_eq!(stored.ssid.data[6], 0);
    }

    #[test]
    fn on_while_already_on_is_fatal() {
        let mut ap = SysutilAp::new();
        let inp = inputs();
        ap.on(Some(&inp), 0, &HealthyNetifBackend).unwrap();
        assert_eq!(
            ap.on(Some(&inp), 0, &HealthyNetifBackend),
            Err(CELL_SYSUTIL_AP_ERROR_FATAL)
        );
    }

    #[test]
    fn on_oversize_title_id_is_invalid_value() {
        let mut ap = SysutilAp::new();
        let huge = "X".repeat(CELL_SYSUTIL_AP_TITLE_ID_LEN + 1);
        let inp = SysutilApInputs {
            title_id: &huge,
            ..inputs()
        };
        assert_eq!(
            ap.on(Some(&inp), 0, &HealthyNetifBackend),
            Err(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn on_oversize_ssid_is_invalid_value() {
        let mut ap = SysutilAp::new();
        let huge = "S".repeat(CELL_SYSUTIL_AP_SSID_LEN + 1);
        let inp = SysutilApInputs {
            ssid: &huge,
            ..inputs()
        };
        assert_eq!(
            ap.on(Some(&inp), 0, &HealthyNetifBackend),
            Err(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn on_oversize_wpa_key_is_invalid_value() {
        let mut ap = SysutilAp::new();
        let huge = "K".repeat(CELL_SYSUTIL_AP_WPA_KEY_LEN + 1);
        let inp = SysutilApInputs {
            wpa_key: &huge,
            ..inputs()
        };
        assert_eq!(
            ap.on(Some(&inp), 0, &HealthyNetifBackend),
            Err(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn on_empty_ssid_is_zero_registered() {
        let mut ap = SysutilAp::new();
        let inp = SysutilApInputs {
            ssid: "",
            ..inputs()
        };
        assert_eq!(
            ap.on(Some(&inp), 0, &HealthyNetifBackend),
            Err(CELL_SYSUTIL_AP_ERROR_ZERO_REGISTERED)
        );
    }

    #[test]
    fn on_netif_disabled() {
        struct DisabledNetif;
        impl ApNetifBackend for DisabledNetif {
            fn is_enabled(&self) -> bool {
                false
            }
        }
        let mut ap = SysutilAp::new();
        let inp = inputs();
        assert_eq!(
            ap.on(Some(&inp), 0, &DisabledNetif),
            Err(CELL_SYSUTIL_AP_ERROR_NETIF_DISABLED)
        );
    }

    #[test]
    fn on_netif_no_cable() {
        struct NoCableNetif;
        impl ApNetifBackend for NoCableNetif {
            fn has_cable(&self) -> bool {
                false
            }
        }
        let mut ap = SysutilAp::new();
        let inp = inputs();
        assert_eq!(
            ap.on(Some(&inp), 0, &NoCableNetif),
            Err(CELL_SYSUTIL_AP_ERROR_NETIF_NO_CABLE)
        );
    }

    #[test]
    fn on_netif_cannot_connect() {
        struct FlakyNetif;
        impl ApNetifBackend for FlakyNetif {
            fn can_connect(&self) -> bool {
                false
            }
        }
        let mut ap = SysutilAp::new();
        let inp = inputs();
        assert_eq!(
            ap.on(Some(&inp), 0, &FlakyNetif),
            Err(CELL_SYSUTIL_AP_ERROR_NETIF_CANNOT_CONNECT)
        );
    }

    #[test]
    fn off_when_off_is_not_initialized() {
        let mut ap = SysutilAp::new();
        assert_eq!(ap.off(), Err(CELL_SYSUTIL_AP_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn off_happy_path_clears_state() {
        let mut ap = SysutilAp::new();
        let inp = inputs();
        ap.on(Some(&inp), 0xBEEF, &HealthyNetifBackend).unwrap();
        ap.off().unwrap();
        assert_eq!(ap.state(), ApState::Off);
        assert!(ap.active_param().is_none());
        assert_eq!(ap.container(), 0);
    }

    #[test]
    fn copy_nul_terminated_happy_path() {
        let mut buf = [0xFFu8; 10];
        copy_nul_terminated(&mut buf, "hello").unwrap();
        assert_eq!(&buf[..5], b"hello");
        assert_eq!(buf[5], 0);
        // Trailing bytes zero-filled.
        assert!(buf[6..].iter().all(|&b| b == 0));
    }

    #[test]
    fn copy_nul_terminated_oversize_rejected() {
        let mut buf = [0u8; 5];
        assert_eq!(
            copy_nul_terminated(&mut buf, "oversized"),
            Err(CELL_SYSUTIL_AP_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn reinit_after_off_allowed() {
        let mut ap = SysutilAp::new();
        let inp = inputs();
        ap.on(Some(&inp), 1, &HealthyNetifBackend).unwrap();
        ap.off().unwrap();
        ap.on(Some(&inp), 2, &HealthyNetifBackend).unwrap();
        assert_eq!(ap.container(), 2);
    }

    #[test]
    fn full_sysutilap_lifecycle_smoke() {
        let mut ap = SysutilAp::new();

        // 1. Query required mem.
        assert_eq!(ap.get_required_mem_size(), 1 << 20);

        // 2. Happy-path turn on.
        let inp = inputs();
        ap.on(Some(&inp), 0xFACE, &HealthyNetifBackend).unwrap();
        assert_eq!(ap.state(), ApState::On);

        // 3. Verify stored param fields.
        let p = ap.active_param().unwrap();
        assert_eq!(p.r#type, 1);
        assert_eq!(p.wlan_flag, 0);
        assert_eq!(&p.title_id.data[..8], b"BLES1234");
        assert_eq!(&p.ssid.data[..6], b"PS3-AP");

        // 4. Second On rejected.
        assert_eq!(
            ap.on(Some(&inp), 0, &HealthyNetifBackend),
            Err(CELL_SYSUTIL_AP_ERROR_FATAL)
        );

        // 5. Off + counters.
        ap.off().unwrap();
        assert_eq!(ap.state(), ApState::Off);
        assert_eq!(ap.get_mem_size_calls(), 1);
        assert_eq!(ap.on_calls(), 1);
        assert_eq!(ap.off_calls(), 1);
    }
}
