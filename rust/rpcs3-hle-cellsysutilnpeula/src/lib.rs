//! Rust port of `rpcs3/Emu/Cell/Modules/cellSysutilNpEula.cpp`.
//!
//! 3 PRX entries under the module name `cellSysutilNpEula` — the NP
//! EULA confirmation dialog that games use to gate first-launch
//! agreement to Sony's network terms:
//!
//!  1. `sceNpEulaCheckEulaStatus` — async "have I accepted this
//!     communication id's EULA already?" query.
//!  2. `sceNpEulaAbort` — cancels any running check/show dialog.
//!  3. `sceNpEulaShowCurrentEula` — pops the EULA accept dialog.
//!
//! The C++ stub (103 lines) doesn't actually render a dialog — it
//! queues a deferred callback reporting `ALREADY_ACCEPTED` so games
//! like Resistance 3 / Uncharted 2 don't hang on the EULA prompt. The
//! Rust port preserves that exact semantics + adds FSM enforcement
//! (check and show paths are mutually exclusive) and parameter
//! validation.
//!
//! REG_FUNC order at cpp:100-102.
//! Module name byte-exact at cpp:7 / cpp:98.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:7 / cpp:98.
pub const MODULE_NAME: &str = "cellSysutilNpEula";

/// REG_FUNC order at cpp:100-102.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpEulaCheckEulaStatus",
    "sceNpEulaAbort",
    "sceNpEulaShowCurrentEula",
];

// --- Error codes (byte-exact sceNp.h) -----------------------------------
//
// Facility `0x8002_E5__` with three sub-ranges:
//  - Base errors `00..05`.
//  - EULA lookup errors `A0..A1`.
//  - CONF (sceNpEulaConf) errors `B0..B6`.

pub const SCE_NP_EULA_ERROR_UNKNOWN: CellError = CellError(0x8002_E500);
pub const SCE_NP_EULA_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_E501);
pub const SCE_NP_EULA_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_E502);
pub const SCE_NP_EULA_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_E503);
pub const SCE_NP_EULA_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_E504);
pub const SCE_NP_EULA_ERROR_BUSY: CellError = CellError(0x8002_E505);
pub const SCE_NP_EULA_ERROR_EULA_NOT_FOUND: CellError = CellError(0x8002_E5A0);
pub const SCE_NP_EULA_ERROR_NET_OUT_OF_MEMORY: CellError = CellError(0x8002_E5A1);
pub const SCE_NP_EULA_ERROR_CONF_FORMAT: CellError = CellError(0x8002_E5B0);
pub const SCE_NP_EULA_ERROR_CONF_INVALID_FILENAME: CellError = CellError(0x8002_E5B1);
pub const SCE_NP_EULA_ERROR_CONF_TOO_MANY_EULA_FILES: CellError = CellError(0x8002_E5B2);
pub const SCE_NP_EULA_ERROR_CONF_INVALID_LANGUAGE: CellError = CellError(0x8002_E5B3);
pub const SCE_NP_EULA_ERROR_CONF_INVALID_COUNTRY: CellError = CellError(0x8002_E5B4);
pub const SCE_NP_EULA_ERROR_CONF_INVALID_NPCOMMID: CellError = CellError(0x8002_E5B5);
pub const SCE_NP_EULA_ERROR_CONF_INVALID_EULA_VERSION: CellError = CellError(0x8002_E5B6);

// --- EULA status (byte-exact cpp:9-17) ----------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum SceNpEulaStatus {
    Unknown = 0,
    Accepted = 1,
    AlreadyAccepted = 2,
    Rejected = 3,
    Aborted = 4,
    Error = 5,
}

impl Default for SceNpEulaStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

impl SceNpEulaStatus {
    #[must_use]
    pub const fn as_u32(self) -> u32 {
        self as u32
    }

    #[must_use]
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Unknown),
            1 => Some(Self::Accepted),
            2 => Some(Self::AlreadyAccepted),
            3 => Some(Self::Rejected),
            4 => Some(Self::Aborted),
            5 => Some(Self::Error),
            _ => None,
        }
    }
}

/// The fixed EULA version the stub publishes (`1` — see cpp:51
/// `cbFunc(cb_ppu, cb_infos.status, CELL_OK, 1, cbFuncArg)`).
pub const STUB_EULA_VERSION: u32 = 1;

// --- Data mirrors -------------------------------------------------------

/// Mirror of `SceNpCommunicationId` — the 9-byte NP communication id
/// games declare up front (e.g. `"NPWR00001"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct SceNpCommunicationId {
    pub data: [u8; 9],
    pub term: u8,
    pub num: i32,
}

impl Default for SceNpCommunicationId {
    fn default() -> Self {
        Self {
            data: [0; 9],
            term: 0,
            num: 0,
        }
    }
}

/// Which entry point queued a deferred callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EulaCallbackKind {
    CheckEulaStatus,
    ShowCurrentEula,
}

/// One queued callback — the stub delivers `(status, errorCode,
/// version, userdata)` for `CheckEulaStatus` and a generic sysutil
/// callback for `ShowCurrentEula` (the Rust port tracks both via the
/// same struct for ergonomics).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EulaDeferredCallback {
    pub kind: EulaCallbackKind,
    pub status: SceNpEulaStatus,
    pub error_code: CellError,
    pub version: u32,
    pub userdata: u64,
}

// --- Manager ------------------------------------------------------------

/// Mirror of `sceNpEulaCallbacksRegistered` (cpp:22-27). The port
/// models the atomic flags as a plain boolean because every public
/// method takes `&mut self`.
#[derive(Debug, Default)]
pub struct SysutilNpEula {
    status: SceNpEulaStatus,
    check_eula_status_registered: bool,
    show_current_eula_registered: bool,
    pending_callbacks: Vec<EulaDeferredCallback>,
    check_eula_status_calls: u32,
    abort_calls: u32,
    show_current_eula_calls: u32,
}

impl SysutilNpEula {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn status(&self) -> SceNpEulaStatus {
        self.status
    }

    #[must_use]
    pub fn check_eula_status_registered(&self) -> bool {
        self.check_eula_status_registered
    }

    #[must_use]
    pub fn show_current_eula_registered(&self) -> bool {
        self.show_current_eula_registered
    }

    #[must_use]
    pub fn pending_callback_count(&self) -> usize {
        self.pending_callbacks.len()
    }

    pub fn drain_callbacks(&mut self) -> Vec<EulaDeferredCallback> {
        core::mem::take(&mut self.pending_callbacks)
    }

    #[must_use]
    pub fn check_eula_status_calls(&self) -> u32 {
        self.check_eula_status_calls
    }
    #[must_use]
    pub fn abort_calls(&self) -> u32 {
        self.abort_calls
    }
    #[must_use]
    pub fn show_current_eula_calls(&self) -> u32 {
        self.show_current_eula_calls
    }

    fn either_callback_registered(&self) -> bool {
        self.check_eula_status_registered || self.show_current_eula_registered
    }

    /// `sceNpEulaCheckEulaStatus` (cpp:29-57). Queues a deferred
    /// callback reporting the current status (defaults to
    /// `AlreadyAccepted` per cpp:46).
    ///
    /// Returns `INVALID_ARGUMENT` on null inputs, `ALREADY_INITIALIZED`
    /// if either callback path is already live.
    pub fn check_eula_status(
        &mut self,
        communication_id: Option<&SceNpCommunicationId>,
        cb_addr: u64,
        cb_userdata: u64,
    ) -> Result<(), CellError> {
        self.check_eula_status_calls = self.check_eula_status_calls.saturating_add(1);
        if communication_id.is_none() || cb_addr == 0 {
            return Err(SCE_NP_EULA_ERROR_INVALID_ARGUMENT);
        }
        if self.either_callback_registered() {
            return Err(SCE_NP_EULA_ERROR_ALREADY_INITIALIZED);
        }
        self.check_eula_status_registered = true;
        // cpp:46 — stub always reports "already accepted" so games
        // proceed past the EULA gate.
        self.status = SceNpEulaStatus::AlreadyAccepted;
        self.pending_callbacks.push(EulaDeferredCallback {
            kind: EulaCallbackKind::CheckEulaStatus,
            status: self.status,
            error_code: CellError::OK,
            version: STUB_EULA_VERSION,
            userdata: cb_userdata,
        });
        Ok(())
    }

    /// Simulates the sysutil scheduler firing the deferred callback
    /// queued by `check_eula_status` — the C++ callback body clears
    /// `sceNpEulaCheckEulaStatus_callback_registered` at cpp:52.
    /// Callers drain the queue with this after inspecting the
    /// payload.
    pub fn deliver_pending(&mut self) -> Vec<EulaDeferredCallback> {
        let cbs = self.drain_callbacks();
        for cb in &cbs {
            match cb.kind {
                EulaCallbackKind::CheckEulaStatus => {
                    self.check_eula_status_registered = false;
                }
                EulaCallbackKind::ShowCurrentEula => {
                    self.show_current_eula_registered = false;
                }
            }
        }
        cbs
    }

    /// `sceNpEulaAbort` (cpp:59-74). Requires one of the callbacks
    /// to be live — otherwise `NOT_INITIALIZED`.
    pub fn abort(&mut self) -> Result<(), CellError> {
        self.abort_calls = self.abort_calls.saturating_add(1);
        if !self.either_callback_registered() {
            return Err(SCE_NP_EULA_ERROR_NOT_INITIALIZED);
        }
        // cpp:71 — firmware flips the reported status to Aborted
        // without actually tearing the dialog down.
        self.status = SceNpEulaStatus::Aborted;
        Ok(())
    }

    /// `sceNpEulaShowCurrentEula` (cpp:77-96). Null inputs →
    /// `INVALID_ARGUMENT`; concurrent callback → `ALREADY_INITIALIZED`.
    /// The stub body just marks the callback registered — no deferred
    /// notification is queued (C++ comment cpp:93 says "Unknown
    /// parameters").
    pub fn show_current_eula(
        &mut self,
        communication_id: Option<&SceNpCommunicationId>,
        cb_addr: u64,
    ) -> Result<(), CellError> {
        self.show_current_eula_calls =
            self.show_current_eula_calls.saturating_add(1);
        if communication_id.is_none() || cb_addr == 0 {
            return Err(SCE_NP_EULA_ERROR_INVALID_ARGUMENT);
        }
        if self.either_callback_registered() {
            return Err(SCE_NP_EULA_ERROR_ALREADY_INITIALIZED);
        }
        self.show_current_eula_registered = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_comm_id() -> SceNpCommunicationId {
        let mut id = SceNpCommunicationId::default();
        id.data.copy_from_slice(b"NPWR00001");
        id.num = 1;
        id
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellSysutilNpEula");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(
            REGISTERED_ENTRY_POINTS,
            &[
                "sceNpEulaCheckEulaStatus",
                "sceNpEulaAbort",
                "sceNpEulaShowCurrentEula",
            ]
        );
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(SCE_NP_EULA_ERROR_UNKNOWN.0, 0x8002_E500);
        assert_eq!(SCE_NP_EULA_ERROR_INVALID_ARGUMENT.0, 0x8002_E501);
        assert_eq!(SCE_NP_EULA_ERROR_NOT_INITIALIZED.0, 0x8002_E502);
        assert_eq!(SCE_NP_EULA_ERROR_ALREADY_INITIALIZED.0, 0x8002_E503);
        assert_eq!(SCE_NP_EULA_ERROR_OUT_OF_MEMORY.0, 0x8002_E504);
        assert_eq!(SCE_NP_EULA_ERROR_BUSY.0, 0x8002_E505);
        assert_eq!(SCE_NP_EULA_ERROR_EULA_NOT_FOUND.0, 0x8002_E5A0);
        assert_eq!(SCE_NP_EULA_ERROR_NET_OUT_OF_MEMORY.0, 0x8002_E5A1);
        assert_eq!(SCE_NP_EULA_ERROR_CONF_FORMAT.0, 0x8002_E5B0);
        assert_eq!(SCE_NP_EULA_ERROR_CONF_INVALID_FILENAME.0, 0x8002_E5B1);
        assert_eq!(SCE_NP_EULA_ERROR_CONF_TOO_MANY_EULA_FILES.0, 0x8002_E5B2);
        assert_eq!(SCE_NP_EULA_ERROR_CONF_INVALID_LANGUAGE.0, 0x8002_E5B3);
        assert_eq!(SCE_NP_EULA_ERROR_CONF_INVALID_COUNTRY.0, 0x8002_E5B4);
        assert_eq!(SCE_NP_EULA_ERROR_CONF_INVALID_NPCOMMID.0, 0x8002_E5B5);
        assert_eq!(SCE_NP_EULA_ERROR_CONF_INVALID_EULA_VERSION.0, 0x8002_E5B6);
    }

    #[test]
    fn error_code_sub_range_gaps() {
        // Base 00-05, gap, EULA A0-A1, gap, CONF B0-B6.
        assert_eq!(
            SCE_NP_EULA_ERROR_EULA_NOT_FOUND.0 - SCE_NP_EULA_ERROR_BUSY.0,
            0x9B
        );
        assert_eq!(
            SCE_NP_EULA_ERROR_CONF_FORMAT.0 - SCE_NP_EULA_ERROR_NET_OUT_OF_MEMORY.0,
            0xF
        );
    }

    #[test]
    fn status_enum_byte_exact() {
        assert_eq!(SceNpEulaStatus::Unknown.as_u32(), 0);
        assert_eq!(SceNpEulaStatus::Accepted.as_u32(), 1);
        assert_eq!(SceNpEulaStatus::AlreadyAccepted.as_u32(), 2);
        assert_eq!(SceNpEulaStatus::Rejected.as_u32(), 3);
        assert_eq!(SceNpEulaStatus::Aborted.as_u32(), 4);
        assert_eq!(SceNpEulaStatus::Error.as_u32(), 5);
        assert_eq!(SceNpEulaStatus::from_u32(6), None);
    }

    #[test]
    fn status_roundtrip() {
        for v in 0..=5 {
            let s = SceNpEulaStatus::from_u32(v).unwrap();
            assert_eq!(s.as_u32(), v);
        }
    }

    #[test]
    fn stub_eula_version_byte_exact() {
        assert_eq!(STUB_EULA_VERSION, 1);
    }

    #[test]
    fn starts_unknown_and_no_callbacks() {
        let e = SysutilNpEula::new();
        assert_eq!(e.status(), SceNpEulaStatus::Unknown);
        assert!(!e.check_eula_status_registered());
        assert!(!e.show_current_eula_registered());
    }

    #[test]
    fn check_eula_status_null_comm_id_is_invalid() {
        let mut e = SysutilNpEula::new();
        assert_eq!(
            e.check_eula_status(None, 1, 0),
            Err(SCE_NP_EULA_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn check_eula_status_null_cb_is_invalid() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        assert_eq!(
            e.check_eula_status(Some(&id), 0, 0),
            Err(SCE_NP_EULA_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn check_eula_status_happy_path_queues_already_accepted() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        e.check_eula_status(Some(&id), 0xABCD, 0x1234).unwrap();
        assert!(e.check_eula_status_registered());
        assert_eq!(e.status(), SceNpEulaStatus::AlreadyAccepted);
        let cbs = e.drain_callbacks();
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].kind, EulaCallbackKind::CheckEulaStatus);
        assert_eq!(cbs[0].status, SceNpEulaStatus::AlreadyAccepted);
        assert_eq!(cbs[0].error_code, CellError::OK);
        assert_eq!(cbs[0].version, 1);
        assert_eq!(cbs[0].userdata, 0x1234);
    }

    #[test]
    fn check_eula_status_twice_is_already_initialized() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        e.check_eula_status(Some(&id), 1, 0).unwrap();
        assert_eq!(
            e.check_eula_status(Some(&id), 1, 0),
            Err(SCE_NP_EULA_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn deliver_pending_clears_check_registration() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        e.check_eula_status(Some(&id), 1, 0).unwrap();
        assert!(e.check_eula_status_registered());
        let cbs = e.deliver_pending();
        assert_eq!(cbs.len(), 1);
        assert!(!e.check_eula_status_registered());
        // Now we can issue another check.
        e.check_eula_status(Some(&id), 1, 0).unwrap();
    }

    #[test]
    fn abort_without_registration_is_not_initialized() {
        let mut e = SysutilNpEula::new();
        assert_eq!(e.abort(), Err(SCE_NP_EULA_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn abort_flips_status_to_aborted() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        e.check_eula_status(Some(&id), 1, 0).unwrap();
        e.abort().unwrap();
        assert_eq!(e.status(), SceNpEulaStatus::Aborted);
        // Registration still live — firmware comment cpp:70 clarifies
        // abort only changes the reported status, doesn't unregister.
        assert!(e.check_eula_status_registered());
    }

    #[test]
    fn show_current_eula_null_comm_id_is_invalid() {
        let mut e = SysutilNpEula::new();
        assert_eq!(
            e.show_current_eula(None, 1),
            Err(SCE_NP_EULA_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn show_current_eula_null_cb_is_invalid() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        assert_eq!(
            e.show_current_eula(Some(&id), 0),
            Err(SCE_NP_EULA_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn show_current_eula_happy_path_registers_flag() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        e.show_current_eula(Some(&id), 1).unwrap();
        assert!(e.show_current_eula_registered());
        // Stub doesn't queue a callback (cpp:93 comment).
        assert_eq!(e.pending_callback_count(), 0);
    }

    #[test]
    fn show_blocks_subsequent_check() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        e.show_current_eula(Some(&id), 1).unwrap();
        assert_eq!(
            e.check_eula_status(Some(&id), 1, 0),
            Err(SCE_NP_EULA_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn check_blocks_subsequent_show() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        e.check_eula_status(Some(&id), 1, 0).unwrap();
        assert_eq!(
            e.show_current_eula(Some(&id), 1),
            Err(SCE_NP_EULA_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn abort_works_from_show_path() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        e.show_current_eula(Some(&id), 1).unwrap();
        e.abort().unwrap();
        assert_eq!(e.status(), SceNpEulaStatus::Aborted);
    }

    #[test]
    fn counters_track_dispatch() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();
        // Invalid-arg doesn't skip the counter increment — matches
        // C++ which logs the call before returning.
        let _ = e.check_eula_status(None, 1, 0);
        e.check_eula_status(Some(&id), 1, 0).unwrap();
        e.deliver_pending();
        e.show_current_eula(Some(&id), 1).unwrap();
        e.abort().unwrap();
        assert_eq!(e.check_eula_status_calls(), 2);
        assert_eq!(e.show_current_eula_calls(), 1);
        assert_eq!(e.abort_calls(), 1);
    }

    #[test]
    fn full_npeula_lifecycle_smoke() {
        let mut e = SysutilNpEula::new();
        let id = sample_comm_id();

        // 1. Check + deliver ALREADY_ACCEPTED — the common happy path.
        e.check_eula_status(Some(&id), 0x8000_0000, 0xDEAD).unwrap();
        assert_eq!(e.status(), SceNpEulaStatus::AlreadyAccepted);
        let cbs = e.deliver_pending();
        assert_eq!(cbs[0].status, SceNpEulaStatus::AlreadyAccepted);
        assert_eq!(cbs[0].userdata, 0xDEAD);

        // 2. Show can now run because the check path was delivered.
        e.show_current_eula(Some(&id), 0x8000_0001).unwrap();
        assert!(e.show_current_eula_registered());

        // 3. Abort flips status + clears via deliver_pending (the
        //    firmware's own sysutil callback would drain the show
        //    registration in a real run; the stub doesn't queue one
        //    so we manually clear via deliver_pending no-op).
        e.abort().unwrap();
        assert_eq!(e.status(), SceNpEulaStatus::Aborted);

        // Counter trace.
        assert_eq!(e.check_eula_status_calls(), 1);
        assert_eq!(e.show_current_eula_calls(), 1);
        assert_eq!(e.abort_calls(), 1);
    }
}
