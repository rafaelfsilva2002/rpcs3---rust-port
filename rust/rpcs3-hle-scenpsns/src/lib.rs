//! Rust port of `rpcs3/Emu/Cell/Modules/sceNpSns.cpp` — PS3 Sony NP SNS
//! (Facebook integration) HLE surface.
//!
//! Upstream registers 11 entries on the `sceNpSns` PRX and keeps a singleton
//! `sce_np_sns_manager { is_initialized }` with handle ID tracking via the
//! emulator's `idm`. Real observable behaviour preserved:
//!
//! * `Init`: rejects double-init (`ALREADY_INITIALIZED`), rejects null params,
//!   sets `is_initialized = true`.
//! * `Term`: requires prior `Init` (`NOT_INITIALIZED`), clears flag.
//! * `CreateHandle`: requires init, rejects null output, assigns handle from
//!   `[1..=HANDLE_SLOT_MAX]`; exhaustion → `EXCEEDS_MAX`.
//! * `DestroyHandle` / `AbortHandle` / `LoadThrottle` / `StreamPublish`:
//!   handle ∈ `(INVALID_HANDLE=0, HANDLE_SLOT_MAX=4]`; unknown → `UNKNOWN_HANDLE`.
//! * `GetAccessToken` / `GetLongAccessToken`: multi-step cascade
//!   (null param/result/fb_app_id → INVALID_ARGUMENT, !init → NOT_INITIALIZED,
//!    bad handle → INVALID_ARGUMENT, unknown handle → UNKNOWN_HANDLE,
//!    PSN offline → NOT_SIGN_IN via not_an_error).
//! * `CheckThrottle` / `CheckConfig`: null/init guards only.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use core::mem::size_of;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNpSns";

/// 11 FNIDs in exact `REG_FUNC` order (cpp:322-332).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpSnsFbInit",
    "sceNpSnsFbTerm",
    "sceNpSnsFbCreateHandle",
    "sceNpSnsFbDestroyHandle",
    "sceNpSnsFbAbortHandle",
    "sceNpSnsFbGetAccessToken",
    "sceNpSnsFbGetLongAccessToken",
    "sceNpSnsFbStreamPublish",
    "sceNpSnsFbCheckThrottle",
    "sceNpSnsFbCheckConfig",
    "sceNpSnsFbLoadThrottle",
];

// ---------------------------------------------------------------------------
// Errors — byte-exact `sceNpSnsError` (0x8002_45__).
// ---------------------------------------------------------------------------

pub const SCE_NP_SNS_ERROR_UNKNOWN: CellError = CellError(0x8002_4501);
pub const SCE_NP_SNS_ERROR_NOT_SIGN_IN: CellError = CellError(0x8002_4502);
pub const SCE_NP_SNS_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_4503);
pub const SCE_NP_SNS_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_4504);
pub const SCE_NP_SNS_ERROR_SHUTDOWN: CellError = CellError(0x8002_4505);
pub const SCE_NP_SNS_ERROR_BUSY: CellError = CellError(0x8002_4506);

pub const SCE_NP_SNS_FB_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_4511);
pub const SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_4512);
pub const SCE_NP_SNS_FB_ERROR_EXCEEDS_MAX: CellError = CellError(0x8002_4513);
pub const SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE: CellError = CellError(0x8002_4514);
pub const SCE_NP_SNS_FB_ERROR_ABORTED: CellError = CellError(0x8002_4515);
pub const SCE_NP_SNS_FB_ERROR_ALREADY_ABORTED: CellError = CellError(0x8002_4516);
pub const SCE_NP_SNS_FB_ERROR_CONFIG_DISABLED: CellError = CellError(0x8002_4517);
pub const SCE_NP_SNS_FB_ERROR_FBSERVER_ERROR_RESPONSE: CellError = CellError(0x8002_4518);
pub const SCE_NP_SNS_FB_ERROR_THROTTLE_CLOSED: CellError = CellError(0x8002_4519);
pub const SCE_NP_SNS_FB_ERROR_OPERATION_INTERVAL_VIOLATION: CellError = CellError(0x8002_451A);
pub const SCE_NP_SNS_FB_ERROR_UNLOADED_THROTTLE: CellError = CellError(0x8002_451B);
pub const SCE_NP_SNS_FB_ERROR_ACCESS_NOT_ALLOWED: CellError = CellError(0x8002_451C);

// ---------------------------------------------------------------------------
// Constants (header).
// ---------------------------------------------------------------------------

pub const SCE_NP_SNS_FB_ACCESS_TOKEN_PARAM_OPTIONS_SILENT: u32 = 0x0000_0001;

pub const SCE_NP_SNS_FB_INVALID_HANDLE: u32 = 0;
pub const SCE_NP_SNS_FB_HANDLE_SLOT_MAX: u32 = 4;
pub const SCE_NP_SNS_FB_PERMISSIONS_LENGTH_MAX: usize = 255;
pub const SCE_NP_SNS_FB_ACCESS_TOKEN_LENGTH_MAX: usize = 255;
pub const SCE_NP_SNS_FB_LONG_ACCESS_TOKEN_LENGTH_MAX: usize = 4096;

// ---------------------------------------------------------------------------
// Wire structs.
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SceNpSnsFbInitParams {
    pub pool: u32,
    pub pool_size: u32,
}
const _: () = assert!(size_of::<SceNpSnsFbInitParams>() == 8);

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SceNpSnsFbAccessTokenParam {
    pub fb_app_id: u64,
    pub permissions: [u8; SCE_NP_SNS_FB_PERMISSIONS_LENGTH_MAX + 1],
    pub options: u32,
}

impl Default for SceNpSnsFbAccessTokenParam {
    fn default() -> Self {
        Self {
            fb_app_id: 0,
            permissions: [0; SCE_NP_SNS_FB_PERMISSIONS_LENGTH_MAX + 1],
            options: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// PSN status plug (upstream `np::np_handler::get_psn_status`).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PsnStatus {
    Offline,
    Online,
}

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct SceNpSns {
    pub is_initialized: bool,
    /// Live handle set. Values are `[1..=HANDLE_SLOT_MAX]`.
    pub handles: Vec<u32>,
    pub next_handle_id: u32,
    pub psn_status: PsnStatus,

    // Per-entry counters — 11 entries.
    pub fb_init_calls: u64,
    pub fb_term_calls: u64,
    pub fb_create_handle_calls: u64,
    pub fb_destroy_handle_calls: u64,
    pub fb_abort_handle_calls: u64,
    pub fb_get_access_token_calls: u64,
    pub fb_get_long_access_token_calls: u64,
    pub fb_stream_publish_calls: u64,
    pub fb_check_throttle_calls: u64,
    pub fb_check_config_calls: u64,
    pub fb_load_throttle_calls: u64,
}

impl Default for SceNpSns {
    fn default() -> Self {
        Self {
            is_initialized: false,
            handles: Vec::new(),
            next_handle_id: 1, // idm::id_base = 1
            psn_status: PsnStatus::Online,
            fb_init_calls: 0,
            fb_term_calls: 0,
            fb_create_handle_calls: 0,
            fb_destroy_handle_calls: 0,
            fb_abort_handle_calls: 0,
            fb_get_access_token_calls: 0,
            fb_get_long_access_token_calls: 0,
            fb_stream_publish_calls: 0,
            fb_check_throttle_calls: 0,
            fb_check_config_calls: 0,
            fb_load_throttle_calls: 0,
        }
    }
}

impl SceNpSns {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_psn_status(&mut self, status: PsnStatus) {
        self.psn_status = status;
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        if !self.is_initialized {
            Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED)
        } else {
            Ok(())
        }
    }

    /// Validates `handle ∈ (INVALID..=HANDLE_SLOT_MAX)`. Upstream uses
    /// `handle == INVALID || handle > HANDLE_SLOT_MAX` for the rejection.
    fn require_handle_in_range(handle: u32) -> Result<(), CellError> {
        if handle == SCE_NP_SNS_FB_INVALID_HANDLE || handle > SCE_NP_SNS_FB_HANDLE_SLOT_MAX {
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        } else {
            Ok(())
        }
    }

    fn require_handle_alive(&self, handle: u32) -> Result<(), CellError> {
        if self.handles.contains(&handle) {
            Ok(())
        } else {
            Err(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE)
        }
    }

    /// `sceNpSnsFbInit(params)` — cpp:42-63. Rejects double-init first,
    /// then null params.
    pub fn fb_init(
        &mut self,
        params: Option<&SceNpSnsFbInitParams>,
    ) -> Result<(), CellError> {
        self.fb_init_calls = self.fb_init_calls.saturating_add(1);
        if self.is_initialized {
            return Err(SCE_NP_SNS_FB_ERROR_ALREADY_INITIALIZED);
        }
        if params.is_none() {
            return Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT);
        }
        self.is_initialized = true;
        Ok(())
    }

    /// `sceNpSnsFbTerm()` — cpp:65-79.
    pub fn fb_term(&mut self) -> Result<(), CellError> {
        self.fb_term_calls = self.fb_term_calls.saturating_add(1);
        self.require_initialized()?;
        self.is_initialized = false;
        Ok(())
    }

    /// `sceNpSnsFbCreateHandle(handle)` — cpp:81-104. Ordering preserved:
    /// init check before null check; allocation before slot-exhaustion check.
    pub fn fb_create_handle(
        &mut self,
        handle_out: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.fb_create_handle_calls = self.fb_create_handle_calls.saturating_add(1);
        self.require_initialized()?;
        let slot = handle_out.ok_or(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)?;
        // Simulate `idm::make<sns_fb_handle_t>()`.
        let id = self.next_handle_id;
        self.next_handle_id = self.next_handle_id.saturating_add(1);
        *slot = id;
        if id == SCE_NP_SNS_FB_INVALID_HANDLE || id > SCE_NP_SNS_FB_HANDLE_SLOT_MAX {
            return Err(SCE_NP_SNS_FB_ERROR_EXCEEDS_MAX);
        }
        self.handles.push(id);
        Ok(())
    }

    /// `sceNpSnsFbDestroyHandle(handle)` — cpp:106-126.
    pub fn fb_destroy_handle(&mut self, handle: u32) -> Result<(), CellError> {
        self.fb_destroy_handle_calls = self.fb_destroy_handle_calls.saturating_add(1);
        self.require_initialized()?;
        Self::require_handle_in_range(handle)?;
        let before = self.handles.len();
        self.handles.retain(|h| *h != handle);
        if self.handles.len() == before {
            return Err(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE);
        }
        Ok(())
    }

    /// `sceNpSnsFbAbortHandle(handle)` — cpp:128-152.
    pub fn fb_abort_handle(&mut self, handle: u32) -> Result<(), CellError> {
        self.fb_abort_handle_calls = self.fb_abort_handle_calls.saturating_add(1);
        self.require_initialized()?;
        Self::require_handle_in_range(handle)?;
        self.require_handle_alive(handle)?;
        // TODO upstream — stub kept.
        Ok(())
    }

    /// `sceNpSnsFbGetAccessToken(handle, param, result)` — cpp:154-192.
    /// Cascade order preserved: (1) param/result/fb_app_id null → INVALID,
    /// (2) !init → NOT_INITIALIZED, (3) handle range → INVALID, (4) unknown →
    /// UNKNOWN_HANDLE, (5) PSN offline → NOT_SIGN_IN.
    pub fn fb_get_access_token(
        &mut self,
        handle: u32,
        param: Option<&SceNpSnsFbAccessTokenParam>,
        result_non_null: bool,
    ) -> Result<(), CellError> {
        self.fb_get_access_token_calls = self.fb_get_access_token_calls.saturating_add(1);
        let p = param.ok_or(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)?;
        if !result_non_null {
            return Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT);
        }
        if p.fb_app_id == 0 {
            return Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT);
        }
        self.require_initialized()?;
        Self::require_handle_in_range(handle)?;
        self.require_handle_alive(handle)?;
        if self.psn_status == PsnStatus::Offline {
            return Err(SCE_NP_SNS_ERROR_NOT_SIGN_IN);
        }
        Ok(())
    }

    /// `sceNpSnsFbGetLongAccessToken(handle, param, result)` — cpp:281-317.
    /// Same cascade as `GetAccessToken`.
    pub fn fb_get_long_access_token(
        &mut self,
        handle: u32,
        param: Option<&SceNpSnsFbAccessTokenParam>,
        result_non_null: bool,
    ) -> Result<(), CellError> {
        self.fb_get_long_access_token_calls =
            self.fb_get_long_access_token_calls.saturating_add(1);
        let p = param.ok_or(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)?;
        if !result_non_null {
            return Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT);
        }
        if p.fb_app_id == 0 {
            return Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT);
        }
        self.require_initialized()?;
        Self::require_handle_in_range(handle)?;
        self.require_handle_alive(handle)?;
        if self.psn_status == PsnStatus::Offline {
            return Err(SCE_NP_SNS_ERROR_NOT_SIGN_IN);
        }
        Ok(())
    }

    /// `sceNpSnsFbStreamPublish(handle, ...)` — cpp:194-221. Doesn't check
    /// init; only handle range + alive.
    pub fn fb_stream_publish(&mut self, handle: u32) -> Result<(), CellError> {
        self.fb_stream_publish_calls = self.fb_stream_publish_calls.saturating_add(1);
        Self::require_handle_in_range(handle)?;
        self.require_handle_alive(handle)?;
        Ok(())
    }

    /// `sceNpSnsFbCheckThrottle(arg0)` — cpp:223-238.
    pub fn fb_check_throttle(&mut self, arg0_non_null: bool) -> Result<(), CellError> {
        self.fb_check_throttle_calls = self.fb_check_throttle_calls.saturating_add(1);
        if !arg0_non_null {
            return Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT);
        }
        self.require_initialized()?;
        Ok(())
    }

    /// `sceNpSnsFbCheckConfig(arg0)` — cpp:240-250. Note: upstream does NOT
    /// check arg0 null, only the init flag. Preserved.
    pub fn fb_check_config(&mut self) -> Result<(), CellError> {
        self.fb_check_config_calls = self.fb_check_config_calls.saturating_add(1);
        self.require_initialized()?;
        Ok(())
    }

    /// `sceNpSnsFbLoadThrottle(handle)` — cpp:252-279. Same handle check as
    /// StreamPublish (no init check).
    pub fn fb_load_throttle(&mut self, handle: u32) -> Result<(), CellError> {
        self.fb_load_throttle_calls = self.fb_load_throttle_calls.saturating_add(1);
        Self::require_handle_in_range(handle)?;
        self.require_handle_alive(handle)?;
        Ok(())
    }
}

// Suppress unused warning for String import (kept for future API shaping).
#[allow(dead_code)]
fn _keep_string(_: String) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sceNpSns");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 11);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "sceNpSnsFbInit");
        assert_eq!(REGISTERED_ENTRY_POINTS[10], "sceNpSnsFbLoadThrottle");
    }

    #[test]
    fn error_codes_byte_exact() {
        // General block
        assert_eq!(SCE_NP_SNS_ERROR_UNKNOWN.0, 0x8002_4501);
        assert_eq!(SCE_NP_SNS_ERROR_NOT_SIGN_IN.0, 0x8002_4502);
        assert_eq!(SCE_NP_SNS_ERROR_INVALID_ARGUMENT.0, 0x8002_4503);
        assert_eq!(SCE_NP_SNS_ERROR_OUT_OF_MEMORY.0, 0x8002_4504);
        assert_eq!(SCE_NP_SNS_ERROR_SHUTDOWN.0, 0x8002_4505);
        assert_eq!(SCE_NP_SNS_ERROR_BUSY.0, 0x8002_4506);
        // FB block
        assert_eq!(SCE_NP_SNS_FB_ERROR_ALREADY_INITIALIZED.0, 0x8002_4511);
        assert_eq!(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED.0, 0x8002_4512);
        assert_eq!(SCE_NP_SNS_FB_ERROR_EXCEEDS_MAX.0, 0x8002_4513);
        assert_eq!(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE.0, 0x8002_4514);
        assert_eq!(SCE_NP_SNS_FB_ERROR_ABORTED.0, 0x8002_4515);
        assert_eq!(SCE_NP_SNS_FB_ERROR_ALREADY_ABORTED.0, 0x8002_4516);
        assert_eq!(SCE_NP_SNS_FB_ERROR_CONFIG_DISABLED.0, 0x8002_4517);
        assert_eq!(SCE_NP_SNS_FB_ERROR_FBSERVER_ERROR_RESPONSE.0, 0x8002_4518);
        assert_eq!(SCE_NP_SNS_FB_ERROR_THROTTLE_CLOSED.0, 0x8002_4519);
        assert_eq!(SCE_NP_SNS_FB_ERROR_OPERATION_INTERVAL_VIOLATION.0, 0x8002_451A);
        assert_eq!(SCE_NP_SNS_FB_ERROR_UNLOADED_THROTTLE.0, 0x8002_451B);
        assert_eq!(SCE_NP_SNS_FB_ERROR_ACCESS_NOT_ALLOWED.0, 0x8002_451C);
    }

    #[test]
    fn constants() {
        assert_eq!(SCE_NP_SNS_FB_INVALID_HANDLE, 0);
        assert_eq!(SCE_NP_SNS_FB_HANDLE_SLOT_MAX, 4);
        assert_eq!(SCE_NP_SNS_FB_PERMISSIONS_LENGTH_MAX, 255);
        assert_eq!(SCE_NP_SNS_FB_ACCESS_TOKEN_LENGTH_MAX, 255);
        assert_eq!(SCE_NP_SNS_FB_LONG_ACCESS_TOKEN_LENGTH_MAX, 4096);
        assert_eq!(SCE_NP_SNS_FB_ACCESS_TOKEN_PARAM_OPTIONS_SILENT, 1);
    }

    #[test]
    fn init_double_init_rejected() {
        let mut m = SceNpSns::new();
        let params = SceNpSnsFbInitParams::default();
        m.fb_init(Some(&params)).unwrap();
        assert!(m.is_initialized);
        assert_eq!(
            m.fb_init(Some(&params)),
            Err(SCE_NP_SNS_FB_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn init_null_params_when_not_init_is_invalid() {
        let mut m = SceNpSns::new();
        assert_eq!(m.fb_init(None), Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn init_already_initialized_takes_precedence_over_null() {
        // cpp:48-56: init check comes BEFORE null params check.
        let mut m = SceNpSns::new();
        let params = SceNpSnsFbInitParams::default();
        m.fb_init(Some(&params)).unwrap();
        assert_eq!(
            m.fb_init(None),
            Err(SCE_NP_SNS_FB_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn term_without_init_is_error() {
        let mut m = SceNpSns::new();
        assert_eq!(m.fb_term(), Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn term_clears_flag() {
        let mut m = SceNpSns::new();
        let params = SceNpSnsFbInitParams::default();
        m.fb_init(Some(&params)).unwrap();
        m.fb_term().unwrap();
        assert!(!m.is_initialized);
    }

    #[test]
    fn create_handle_requires_init() {
        let mut m = SceNpSns::new();
        let mut h = 0u32;
        assert_eq!(
            m.fb_create_handle(Some(&mut h)),
            Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn create_handle_null_rejected_after_init_check() {
        let mut m = SceNpSns::new();
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        assert_eq!(
            m.fb_create_handle(None),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn create_handle_assigns_monotonic_ids() {
        let mut m = SceNpSns::new();
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        let mut h = 0u32;
        for expected in 1..=SCE_NP_SNS_FB_HANDLE_SLOT_MAX {
            m.fb_create_handle(Some(&mut h)).unwrap();
            assert_eq!(h, expected);
        }
        // Next attempt → slot overflow → EXCEEDS_MAX.
        let res = m.fb_create_handle(Some(&mut h));
        assert_eq!(res, Err(SCE_NP_SNS_FB_ERROR_EXCEEDS_MAX));
        // h WAS written (cpp:97 writes before the check cpp:98).
        assert_eq!(h, SCE_NP_SNS_FB_HANDLE_SLOT_MAX + 1);
    }

    #[test]
    fn destroy_handle_rejects_invalid_and_unknown() {
        let mut m = SceNpSns::new();
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        // INVALID_HANDLE=0 → INVALID_ARGUMENT.
        assert_eq!(
            m.fb_destroy_handle(0),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        // > SLOT_MAX → INVALID_ARGUMENT.
        assert_eq!(
            m.fb_destroy_handle(SCE_NP_SNS_FB_HANDLE_SLOT_MAX + 1),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        // In-range but not live → UNKNOWN_HANDLE.
        assert_eq!(
            m.fb_destroy_handle(3),
            Err(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE)
        );
        // Create one, destroy works.
        let mut h = 0u32;
        m.fb_create_handle(Some(&mut h)).unwrap();
        m.fb_destroy_handle(h).unwrap();
        assert!(!m.handles.contains(&h));
    }

    #[test]
    fn destroy_handle_without_init_is_not_initialized() {
        let mut m = SceNpSns::new();
        assert_eq!(
            m.fb_destroy_handle(1),
            Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn abort_handle_validation_cascade() {
        let mut m = SceNpSns::new();
        assert_eq!(
            m.fb_abort_handle(1),
            Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED)
        );
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        assert_eq!(
            m.fb_abort_handle(0),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.fb_abort_handle(99),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.fb_abort_handle(2),
            Err(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE)
        );
        let mut h = 0u32;
        m.fb_create_handle(Some(&mut h)).unwrap();
        m.fb_abort_handle(h).unwrap();
    }

    #[test]
    fn get_access_token_full_cascade() {
        let mut m = SceNpSns::new();
        // param null
        assert_eq!(
            m.fb_get_access_token(1, None, true),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        let mut p = SceNpSnsFbAccessTokenParam::default();
        p.fb_app_id = 42;
        // result null
        assert_eq!(
            m.fb_get_access_token(1, Some(&p), false),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        // fb_app_id == 0
        let p_zero = SceNpSnsFbAccessTokenParam::default();
        assert_eq!(
            m.fb_get_access_token(1, Some(&p_zero), true),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        // !initialized
        assert_eq!(
            m.fb_get_access_token(1, Some(&p), true),
            Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED)
        );
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        // invalid handle range
        assert_eq!(
            m.fb_get_access_token(0, Some(&p), true),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        // unknown handle
        assert_eq!(
            m.fb_get_access_token(2, Some(&p), true),
            Err(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE)
        );
        // Create handle
        let mut h = 0u32;
        m.fb_create_handle(Some(&mut h)).unwrap();
        // offline
        m.set_psn_status(PsnStatus::Offline);
        assert_eq!(
            m.fb_get_access_token(h, Some(&p), true),
            Err(SCE_NP_SNS_ERROR_NOT_SIGN_IN)
        );
        // online ok
        m.set_psn_status(PsnStatus::Online);
        m.fb_get_access_token(h, Some(&p), true).unwrap();
    }

    #[test]
    fn get_long_access_token_shares_cascade() {
        let mut m = SceNpSns::new();
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        let mut h = 0u32;
        m.fb_create_handle(Some(&mut h)).unwrap();
        let mut p = SceNpSnsFbAccessTokenParam::default();
        p.fb_app_id = 1;
        m.fb_get_long_access_token(h, Some(&p), true).unwrap();
        m.set_psn_status(PsnStatus::Offline);
        assert_eq!(
            m.fb_get_long_access_token(h, Some(&p), true),
            Err(SCE_NP_SNS_ERROR_NOT_SIGN_IN)
        );
    }

    #[test]
    fn stream_publish_and_load_throttle_no_init_check() {
        let mut m = SceNpSns::new();
        // Not initialized — still errors on handle check, NOT on init.
        assert_eq!(
            m.fb_stream_publish(0),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.fb_stream_publish(2),
            Err(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE)
        );
        assert_eq!(
            m.fb_load_throttle(0),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.fb_load_throttle(2),
            Err(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE)
        );
        // With an alive handle (after init for CreateHandle):
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        let mut h = 0u32;
        m.fb_create_handle(Some(&mut h)).unwrap();
        m.fb_stream_publish(h).unwrap();
        m.fb_load_throttle(h).unwrap();
    }

    #[test]
    fn check_throttle_null_then_init_cascade() {
        let mut m = SceNpSns::new();
        assert_eq!(
            m.fb_check_throttle(false),
            Err(SCE_NP_SNS_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.fb_check_throttle(true),
            Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED)
        );
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        m.fb_check_throttle(true).unwrap();
    }

    #[test]
    fn check_config_only_init_guard() {
        let mut m = SceNpSns::new();
        assert_eq!(
            m.fb_check_config(),
            Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED)
        );
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        m.fb_check_config().unwrap();
    }

    #[test]
    fn counters_track_all_entries() {
        let mut m = SceNpSns::new();
        let _ = m.fb_init(None);
        m.fb_init(Some(&SceNpSnsFbInitParams::default())).unwrap();
        let mut h = 0u32;
        m.fb_create_handle(Some(&mut h)).unwrap();
        let _ = m.fb_abort_handle(h);
        let mut p = SceNpSnsFbAccessTokenParam::default();
        p.fb_app_id = 1;
        let _ = m.fb_get_access_token(h, Some(&p), true);
        let _ = m.fb_get_long_access_token(h, Some(&p), true);
        let _ = m.fb_stream_publish(h);
        let _ = m.fb_check_throttle(true);
        let _ = m.fb_check_config();
        let _ = m.fb_load_throttle(h);
        let _ = m.fb_destroy_handle(h);
        let _ = m.fb_term();

        assert!(m.fb_init_calls >= 2); // one failed + one success
        assert_eq!(m.fb_term_calls, 1);
        assert_eq!(m.fb_create_handle_calls, 1);
        assert_eq!(m.fb_destroy_handle_calls, 1);
        assert_eq!(m.fb_abort_handle_calls, 1);
        assert_eq!(m.fb_get_access_token_calls, 1);
        assert_eq!(m.fb_get_long_access_token_calls, 1);
        assert_eq!(m.fb_stream_publish_calls, 1);
        assert_eq!(m.fb_check_throttle_calls, 1);
        assert_eq!(m.fb_check_config_calls, 1);
        assert_eq!(m.fb_load_throttle_calls, 1);
    }

    #[test]
    fn full_sns_lifecycle_smoke() {
        let mut m = SceNpSns::new();
        // Init
        m.fb_init(Some(&SceNpSnsFbInitParams { pool: 0x1000, pool_size: 0x4000 })).unwrap();
        // Create 4 handles (max).
        let mut ids: [u32; 4] = [0; 4];
        for s in ids.iter_mut() {
            m.fb_create_handle(Some(s)).unwrap();
        }
        assert_eq!(ids, [1, 2, 3, 4]);
        // 5th should fail.
        let mut extra = 0u32;
        assert_eq!(
            m.fb_create_handle(Some(&mut extra)),
            Err(SCE_NP_SNS_FB_ERROR_EXCEEDS_MAX)
        );
        assert_eq!(extra, 5); // Still written.
        // Use handle 2.
        let mut p = SceNpSnsFbAccessTokenParam::default();
        p.fb_app_id = 0x12_3456;
        m.fb_get_access_token(2, Some(&p), true).unwrap();
        m.fb_stream_publish(2).unwrap();
        m.fb_load_throttle(2).unwrap();
        m.fb_check_throttle(true).unwrap();
        m.fb_check_config().unwrap();
        // Destroy half.
        m.fb_destroy_handle(1).unwrap();
        m.fb_destroy_handle(3).unwrap();
        // Destroying again fails.
        assert_eq!(
            m.fb_destroy_handle(1),
            Err(SCE_NP_SNS_FB_ERROR_UNKNOWN_HANDLE)
        );
        // Abort the remaining ones.
        m.fb_abort_handle(2).unwrap();
        m.fb_abort_handle(4).unwrap();
        // Term.
        m.fb_term().unwrap();
        // Post-term: everything requires init.
        assert_eq!(
            m.fb_check_config(),
            Err(SCE_NP_SNS_FB_ERROR_NOT_INITIALIZED)
        );
    }
}
