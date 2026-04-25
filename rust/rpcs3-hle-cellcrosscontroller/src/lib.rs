//! Rust port of `rpcs3/Emu/Cell/Modules/cellCrossController.cpp`.
//!
//! The C++ source (192 lines) declares exactly 2 PRX-registered functions
//! under the module name `cellCrossController`:
//!
//!  - `cellCrossControllerInitialize` (visible REG_FUNC at cpp:187)
//!  - `finish_callback` (hidden REG_HIDDEN_FUNC at cpp:190)
//!
//! The initializer installs the user callback, forwards package metadata
//! onto a message-dialog, and spawns a helper thread that would (in the
//! real firmware) hand-shake with a paired PS Vita. The hidden callback
//! is only invoked when the user cancels the msg-dialog — mirroring
//! cpp:120 `ensure(cc.callback && button_type == CELL_MSGDIALOG_BUTTON_ESCAPE)`.
//!
//! This port follows the validation cascade in
//! `cellCrossControllerInitialize` (cpp:126-181) byte-for-byte: the order
//! of error codes below matches the order of checks in the C++, so a
//! fuzzer hitting multiple invalid fields at once surfaces the same error
//! as the firmware would.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Module name byte-exact at cpp:185 `DECLARE(...)("cellCrossController", ...)`.
pub const MODULE_NAME: &str = "cellCrossController";

/// Registered entry point names in REG_FUNC / REG_HIDDEN_FUNC order
/// (cpp:187, cpp:190). `finish_callback` is registered as a hidden helper
/// so that the `CellMsgDialogCallback` vector supplied by the firmware has
/// a stable PPU address to target.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellCrossControllerInitialize",
    "finish_callback",
];

// --- Error codes (byte-exact cellCrossController.h:3-19) ----------------
//
// Facility `0x8002_CD__`, non-contiguous:
//  - `80`..=`81` — user-level cancel / network failure
//  - `90`..=`9A` — programmer / resource errors (tight cluster)
//  - `A0`        — internal fatal

pub const CELL_CROSS_CONTROLLER_ERROR_CANCEL: CellError = CellError(0x8002_CD80);
pub const CELL_CROSS_CONTROLLER_ERROR_NETWORK: CellError = CellError(0x8002_CD81);
pub const CELL_CROSS_CONTROLLER_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_CD90);
pub const CELL_CROSS_CONTROLLER_ERROR_FATAL: CellError = CellError(0x8002_CD91);
pub const CELL_CROSS_CONTROLLER_ERROR_INVALID_PKG_FILENAME: CellError = CellError(0x8002_CD92);
pub const CELL_CROSS_CONTROLLER_ERROR_INVALID_SIG_FILENAME: CellError = CellError(0x8002_CD93);
pub const CELL_CROSS_CONTROLLER_ERROR_INVALID_ICON_FILENAME: CellError = CellError(0x8002_CD94);
pub const CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE: CellError = CellError(0x8002_CD95);
pub const CELL_CROSS_CONTROLLER_ERROR_PKG_FILE_OPEN: CellError = CellError(0x8002_CD96);
pub const CELL_CROSS_CONTROLLER_ERROR_SIG_FILE_OPEN: CellError = CellError(0x8002_CD97);
pub const CELL_CROSS_CONTROLLER_ERROR_ICON_FILE_OPEN: CellError = CellError(0x8002_CD98);
pub const CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE: CellError = CellError(0x8002_CD99);
pub const CELL_CROSS_CONTROLLER_ERROR_INVALID_PKG_FILE: CellError = CellError(0x8002_CD9A);
pub const CELL_CROSS_CONTROLLER_ERROR_INTERNAL: CellError = CellError(0x8002_CDA0);

// --- Status constants (cellCrossController.h:21-25) ---------------------

pub const CELL_CROSS_CONTROLLER_STATUS_INITIALIZED: i32 = 1;
pub const CELL_CROSS_CONTROLLER_STATUS_FINALIZED: i32 = 2;

// --- String length caps (cellCrossController.h:27-34) -------------------

pub const CELL_CROSS_CONTROLLER_PKG_APP_VER_LEN: usize = 6; // e.g. "01.00"
pub const CELL_CROSS_CONTROLLER_PKG_TITLE_ID_LEN: usize = 10;
pub const CELL_CROSS_CONTROLLER_PKG_TITLE_LEN: usize = 52;
pub const CELL_CROSS_CONTROLLER_PARAM_FILE_NAME_LEN: usize = 255;

// --- Msg-dialog button sentinels (matches cellMsgDialog.h) --------------
//
// The port only cares about `ESCAPE`; keeping the rest as named
// constants documents the expected peer-ABI.

pub const CELL_MSGDIALOG_BUTTON_NONE: i32 = -1;
pub const CELL_MSGDIALOG_BUTTON_INVALID: i32 = 0;
pub const CELL_MSGDIALOG_BUTTON_OK: i32 = 1;
pub const CELL_MSGDIALOG_BUTTON_YES: i32 = 1;
pub const CELL_MSGDIALOG_BUTTON_NO: i32 = 2;
pub const CELL_MSGDIALOG_BUTTON_ESCAPE: i32 = 3;

// --- Data mirrors --------------------------------------------------------

/// Mirror of `CellCrossControllerParam` (cellCrossController.h:37-43).
///
/// The C++ struct is four firmware pointers; the port uses owned
/// `String`s because the host has no guest-pointer indirection. The
/// caller passes `None` to represent a null pointer — matching the
/// `!pParam->pPackageFileName` null checks at cpp:154/159/164.
#[derive(Debug, Default, Clone)]
pub struct CellCrossControllerParam {
    pub package_file_name: Option<String>,
    pub signature_file_name: Option<String>,
    pub icon_file_name: Option<String>,
}

/// Mirror of `CellCrossControllerPackageInfo` (cellCrossController.h:45-50).
#[derive(Debug, Default, Clone)]
pub struct CellCrossControllerPackageInfo {
    pub title: Option<String>,
    pub title_id: Option<String>,
    pub app_ver: Option<String>,
}

/// Lifecycle of the singleton `cross_controller` (cpp:42-113).
///
/// `Uninitialized` → `Initialized` after the init thread spawns, and
/// `Finalized` once the thread returns (or the user cancels the dialog).
/// The firmware never re-enters `Uninitialized` from `Finalized` —
/// callers must allocate a fresh instance. The port mirrors that: once
/// finalized, `initialize` returns `INVALID_STATE`, matching the
/// `cc.status == CELL_CROSS_CONTROLLER_STATUS_INITIALIZED` check at
/// cpp:142 extended for post-finalize attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossControllerPhase {
    Uninitialized,
    Initialized,
    Finalized,
}

/// Result delivered to the user callback through `sysutil_register_cb`.
/// The port drains these via [`CrossController::drain_callbacks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrossControllerCallback {
    /// Status value passed in the C++ callback's first `s32` argument —
    /// either `CELL_CROSS_CONTROLLER_STATUS_INITIALIZED` (cpp:79) or
    /// `CELL_CROSS_CONTROLLER_STATUS_FINALIZED` (cpp:57/122).
    pub status: i32,
    /// Second `s32`: `CELL_OK` (on success) or a cancel / internal error
    /// on abort paths.
    pub error_code: CellError,
    /// Opaque user-data originally passed at init time (cpp:47).
    pub userdata: u64,
}

// --- Validation helpers --------------------------------------------------

/// Reproduces the `memchr(ptr, '\0', LEN + 1)` length-cap check used at
/// cpp:154/159/164/169-171. Returns `true` iff `s` has a NUL terminator
/// at or before byte `cap`, i.e. `s.len() <= cap`.
#[must_use]
pub const fn fits_in_cap(s: &str, cap: usize) -> bool {
    s.len() <= cap
}

/// Returns the first error the C++ cascade would surface for the given
/// package name, or `Ok(())` if it is acceptable. Factored out so
/// callers and the full `initialize()` share the exact same precedence.
fn validate_package_param(param: &CellCrossControllerParam) -> Result<(), CellError> {
    match &param.package_file_name {
        None => Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_PKG_FILENAME),
        Some(s) if !fits_in_cap(s, CELL_CROSS_CONTROLLER_PARAM_FILE_NAME_LEN) => {
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_PKG_FILENAME)
        }
        _ => Ok(()),
    }?;
    match &param.signature_file_name {
        None => Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_SIG_FILENAME),
        Some(s) if !fits_in_cap(s, CELL_CROSS_CONTROLLER_PARAM_FILE_NAME_LEN) => {
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_SIG_FILENAME)
        }
        _ => Ok(()),
    }?;
    match &param.icon_file_name {
        None => Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_ICON_FILENAME),
        Some(s) if !fits_in_cap(s, CELL_CROSS_CONTROLLER_PARAM_FILE_NAME_LEN) => {
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_ICON_FILENAME)
        }
        _ => Ok(()),
    }
}

/// Reproduces the bulk `INVALID_VALUE` check at cpp:169-175: every
/// package-info field must be present, NUL-terminated within its cap,
/// and the caller must supply a non-null callback.
fn validate_package_info(
    info: &CellCrossControllerPackageInfo,
    has_cb: bool,
) -> Result<(), CellError> {
    let fits = |field: &Option<String>, cap: usize| -> bool {
        field.as_deref().is_some_and(|s| fits_in_cap(s, cap))
    };
    let ok = fits(&info.app_ver, CELL_CROSS_CONTROLLER_PKG_APP_VER_LEN)
        && fits(&info.title_id, CELL_CROSS_CONTROLLER_PKG_TITLE_ID_LEN)
        && fits(&info.title, CELL_CROSS_CONTROLLER_PKG_TITLE_LEN)
        && has_cb;
    if ok {
        Ok(())
    } else {
        Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE)
    }
}

// --- Manager -------------------------------------------------------------

/// HLE singleton mirroring `struct cross_controller` at cpp:42-113.
/// The port tracks the lifecycle phase, the last installed callback
/// address + userdata, and the deferred-callback queue.
#[derive(Debug, Default)]
pub struct CrossController {
    phase: Option<CrossControllerPhase>,
    callback_addr: u64,
    userdata: u64,
    pending_callbacks: Vec<CrossControllerCallback>,
    thread_running: bool,
    dialog_open: bool,
}

impl CrossController {
    /// Construct a fresh singleton in `Uninitialized` state.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            phase: None,
            callback_addr: 0,
            userdata: 0,
            pending_callbacks: Vec::new(),
            thread_running: false,
            dialog_open: false,
        }
    }

    /// Current lifecycle phase; `Uninitialized` by default.
    #[must_use]
    pub fn phase(&self) -> CrossControllerPhase {
        self.phase.unwrap_or(CrossControllerPhase::Uninitialized)
    }

    #[must_use]
    pub fn is_thread_running(&self) -> bool {
        self.thread_running
    }

    #[must_use]
    pub fn is_dialog_open(&self) -> bool {
        self.dialog_open
    }

    #[must_use]
    pub fn installed_callback(&self) -> u64 {
        self.callback_addr
    }

    #[must_use]
    pub fn pending_callback_count(&self) -> usize {
        self.pending_callbacks.len()
    }

    pub fn drain_callbacks(&mut self) -> Vec<CrossControllerCallback> {
        core::mem::take(&mut self.pending_callbacks)
    }

    fn queue(&mut self, status: i32, error_code: CellError) {
        self.pending_callbacks.push(CrossControllerCallback {
            status,
            error_code,
            userdata: self.userdata,
        });
    }

    /// `cellCrossControllerInitialize` (cpp:126-182).
    ///
    /// Runs the cpp:142-181 validation cascade in order:
    ///
    ///  1. Already initialized → `INVALID_STATE` (cpp:144).
    ///  2. `pParam` or `pPkgInfo` null → `INVALID_VALUE` (cpp:149).
    ///  3. Package-name variants → `INVALID_PKG_FILENAME`,
    ///     `INVALID_SIG_FILENAME`, `INVALID_ICON_FILENAME`
    ///     (cpp:154/159/164).
    ///  4. Any pkg-info field null / over cap OR missing callback →
    ///     `INVALID_VALUE` (cpp:169-175).
    ///  5. Installs the callback, queues the `INITIALIZED` deferred
    ///     notification, and flips the phase (cpp:177-181).
    ///
    /// `dialog_open` is always whether the spawned dialog would currently
    /// be on-screen — used in tandem with [`Self::finish_callback`] to
    /// mirror the cpp:115-124 cancel path.
    pub fn initialize(
        &mut self,
        param: Option<&CellCrossControllerParam>,
        info: Option<&CellCrossControllerPackageInfo>,
        callback_addr: u64,
        userdata: u64,
    ) -> Result<(), CellError> {
        if self.phase() != CrossControllerPhase::Uninitialized {
            return Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE);
        }
        let Some(param) = param else {
            return Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE);
        };
        let Some(info) = info else {
            return Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE);
        };
        validate_package_param(param)?;
        validate_package_info(info, callback_addr != 0)?;

        self.callback_addr = callback_addr;
        self.userdata = userdata;
        self.phase = Some(CrossControllerPhase::Initialized);
        self.thread_running = true;
        self.dialog_open = true;
        self.queue(CELL_CROSS_CONTROLLER_STATUS_INITIALIZED, CellError::OK);
        Ok(())
    }

    /// `finish_callback` (cpp:115-124). Called by the msg-dialog runtime
    /// when the user presses Circle to cancel the pairing prompt. The
    /// firmware `ensure()` at cpp:120 rejects anything that isn't
    /// `CELL_MSGDIALOG_BUTTON_ESCAPE` + an active callback; the port
    /// returns the `INVALID_STATE` error for the same conditions instead
    /// of panicking.
    pub fn finish_callback(&mut self, button_type: i32) -> Result<(), CellError> {
        if button_type != CELL_MSGDIALOG_BUTTON_ESCAPE {
            return Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE);
        }
        if self.callback_addr == 0 || self.phase() != CrossControllerPhase::Initialized {
            return Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE);
        }
        // Deliver FINALIZED + CANCEL, stop the helper thread, mark the
        // singleton terminal.
        self.queue(
            CELL_CROSS_CONTROLLER_STATUS_FINALIZED,
            CELL_CROSS_CONTROLLER_ERROR_CANCEL,
        );
        self.thread_running = false;
        self.dialog_open = false;
        self.phase = Some(CrossControllerPhase::Finalized);
        Ok(())
    }

    /// Test-only helper: simulate the `on_connection_established` path
    /// at cpp:49-60 where the (stubbed) PS Vita hand-shake succeeds with
    /// a given status. Closes the dialog, delivers `FINALIZED` with
    /// `CELL_OK` (or the supplied error), and marks the singleton
    /// terminal.
    pub fn deliver_connection_result(&mut self, status: CellError) -> Result<(), CellError> {
        if self.phase() != CrossControllerPhase::Initialized {
            return Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE);
        }
        self.queue(CELL_CROSS_CONTROLLER_STATUS_FINALIZED, status);
        self.thread_running = false;
        self.dialog_open = false;
        self.phase = Some(CrossControllerPhase::Finalized);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    fn make_param() -> CellCrossControllerParam {
        CellCrossControllerParam {
            package_file_name: Some("game.pkg".to_string()),
            signature_file_name: Some("game.sig".to_string()),
            icon_file_name: Some("icon.png".to_string()),
        }
    }

    fn make_info() -> CellCrossControllerPackageInfo {
        CellCrossControllerPackageInfo {
            title: Some("Cat Simulator 5".to_string()),
            title_id: Some("NEKO12345".to_string()),
            app_ver: Some("01.00".to_string()),
        }
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellCrossController");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(
            REGISTERED_ENTRY_POINTS,
            &["cellCrossControllerInitialize", "finish_callback"]
        );
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_CANCEL.0, 0x8002_CD80);
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_NETWORK.0, 0x8002_CD81);
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_OUT_OF_MEMORY.0, 0x8002_CD90);
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_FATAL.0, 0x8002_CD91);
        assert_eq!(
            CELL_CROSS_CONTROLLER_ERROR_INVALID_PKG_FILENAME.0,
            0x8002_CD92
        );
        assert_eq!(
            CELL_CROSS_CONTROLLER_ERROR_INVALID_SIG_FILENAME.0,
            0x8002_CD93
        );
        assert_eq!(
            CELL_CROSS_CONTROLLER_ERROR_INVALID_ICON_FILENAME.0,
            0x8002_CD94
        );
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE.0, 0x8002_CD95);
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_PKG_FILE_OPEN.0, 0x8002_CD96);
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_SIG_FILE_OPEN.0, 0x8002_CD97);
        assert_eq!(
            CELL_CROSS_CONTROLLER_ERROR_ICON_FILE_OPEN.0,
            0x8002_CD98
        );
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE.0, 0x8002_CD99);
        assert_eq!(
            CELL_CROSS_CONTROLLER_ERROR_INVALID_PKG_FILE.0,
            0x8002_CD9A
        );
        assert_eq!(CELL_CROSS_CONTROLLER_ERROR_INTERNAL.0, 0x8002_CDA0);
    }

    #[test]
    fn error_codes_gap_structure_preserved() {
        // cpp header has a deliberate gap at 0x82..0x8F and another at
        // 0x9B..0x9F. Touching this test should feel weird — gaps are a
        // firmware contract.
        assert!(CELL_CROSS_CONTROLLER_ERROR_OUT_OF_MEMORY.0 > CELL_CROSS_CONTROLLER_ERROR_NETWORK.0);
        assert_eq!(
            CELL_CROSS_CONTROLLER_ERROR_OUT_OF_MEMORY.0 - CELL_CROSS_CONTROLLER_ERROR_NETWORK.0,
            0xF
        );
        assert_eq!(
            CELL_CROSS_CONTROLLER_ERROR_INTERNAL.0 - CELL_CROSS_CONTROLLER_ERROR_INVALID_PKG_FILE.0,
            0x6
        );
    }

    #[test]
    fn status_and_length_constants_byte_exact() {
        assert_eq!(CELL_CROSS_CONTROLLER_STATUS_INITIALIZED, 1);
        assert_eq!(CELL_CROSS_CONTROLLER_STATUS_FINALIZED, 2);
        assert_eq!(CELL_CROSS_CONTROLLER_PKG_APP_VER_LEN, 6);
        assert_eq!(CELL_CROSS_CONTROLLER_PKG_TITLE_ID_LEN, 10);
        assert_eq!(CELL_CROSS_CONTROLLER_PKG_TITLE_LEN, 52);
        assert_eq!(CELL_CROSS_CONTROLLER_PARAM_FILE_NAME_LEN, 255);
    }

    #[test]
    fn fits_in_cap_boundary_values() {
        assert!(fits_in_cap("", 0));
        assert!(fits_in_cap("abc", 3));
        assert!(!fits_in_cap("abcd", 3));
        assert!(fits_in_cap("01.00", CELL_CROSS_CONTROLLER_PKG_APP_VER_LEN));
        assert!(!fits_in_cap(
            "01.00.00",
            CELL_CROSS_CONTROLLER_PKG_APP_VER_LEN
        ));
    }

    #[test]
    fn new_starts_uninitialized() {
        let cc = CrossController::new();
        assert_eq!(cc.phase(), CrossControllerPhase::Uninitialized);
        assert_eq!(cc.pending_callback_count(), 0);
        assert!(!cc.is_thread_running());
        assert!(!cc.is_dialog_open());
    }

    #[test]
    fn initialize_happy_path_queues_callback() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();
        cc.initialize(Some(&param), Some(&info), 0xC0FFEE_C0FFEE_u64, 0xDEAD_BEEF)
            .unwrap();
        assert_eq!(cc.phase(), CrossControllerPhase::Initialized);
        assert_eq!(cc.installed_callback(), 0xC0FFEE_C0FFEE);
        assert!(cc.is_thread_running());
        assert!(cc.is_dialog_open());
        let cbs = cc.drain_callbacks();
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].status, CELL_CROSS_CONTROLLER_STATUS_INITIALIZED);
        assert_eq!(cbs[0].error_code, CellError::OK);
        assert_eq!(cbs[0].userdata, 0xDEAD_BEEF);
    }

    #[test]
    fn initialize_twice_is_invalid_state() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();
        cc.initialize(Some(&param), Some(&info), 1, 0).unwrap();
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE)
        );
    }

    #[test]
    fn initialize_null_param_is_invalid_value() {
        let mut cc = CrossController::new();
        let info = make_info();
        assert_eq!(
            cc.initialize(None, Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_null_pkg_info_is_invalid_value() {
        let mut cc = CrossController::new();
        let param = make_param();
        assert_eq!(
            cc.initialize(Some(&param), None, 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_null_package_filename_is_pkg_filename_error() {
        let mut cc = CrossController::new();
        let mut param = make_param();
        param.package_file_name = None;
        let info = make_info();
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_PKG_FILENAME)
        );
    }

    #[test]
    fn initialize_oversize_signature_is_sig_filename_error() {
        let mut cc = CrossController::new();
        let mut param = make_param();
        param.signature_file_name = Some("a".repeat(CELL_CROSS_CONTROLLER_PARAM_FILE_NAME_LEN + 1));
        let info = make_info();
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_SIG_FILENAME)
        );
    }

    #[test]
    fn initialize_null_icon_is_icon_filename_error() {
        let mut cc = CrossController::new();
        let mut param = make_param();
        param.icon_file_name = None;
        let info = make_info();
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_ICON_FILENAME)
        );
    }

    #[test]
    fn initialize_oversize_title_is_invalid_value() {
        let mut cc = CrossController::new();
        let param = make_param();
        let mut info = make_info();
        info.title = Some("x".repeat(CELL_CROSS_CONTROLLER_PKG_TITLE_LEN + 1));
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_oversize_title_id_is_invalid_value() {
        let mut cc = CrossController::new();
        let param = make_param();
        let mut info = make_info();
        info.title_id = Some("x".repeat(CELL_CROSS_CONTROLLER_PKG_TITLE_ID_LEN + 1));
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_oversize_app_ver_is_invalid_value() {
        let mut cc = CrossController::new();
        let param = make_param();
        let mut info = make_info();
        info.app_ver = Some("01.00.99".to_string());
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_null_callback_is_invalid_value() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 0, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_VALUE)
        );
    }

    #[test]
    fn initialize_boundary_title_lengths_accepted() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = CellCrossControllerPackageInfo {
            title: Some("a".repeat(CELL_CROSS_CONTROLLER_PKG_TITLE_LEN)),
            title_id: Some("b".repeat(CELL_CROSS_CONTROLLER_PKG_TITLE_ID_LEN)),
            app_ver: Some("c".repeat(CELL_CROSS_CONTROLLER_PKG_APP_VER_LEN)),
        };
        cc.initialize(Some(&param), Some(&info), 42, 0).unwrap();
        assert_eq!(cc.phase(), CrossControllerPhase::Initialized);
    }

    #[test]
    fn finish_callback_non_escape_is_invalid_state() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();
        cc.initialize(Some(&param), Some(&info), 1, 0).unwrap();
        cc.drain_callbacks();
        assert_eq!(
            cc.finish_callback(CELL_MSGDIALOG_BUTTON_OK),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE)
        );
        // Did not prematurely finalize:
        assert_eq!(cc.phase(), CrossControllerPhase::Initialized);
    }

    #[test]
    fn finish_callback_without_init_is_invalid_state() {
        let mut cc = CrossController::new();
        assert_eq!(
            cc.finish_callback(CELL_MSGDIALOG_BUTTON_ESCAPE),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE)
        );
    }

    #[test]
    fn finish_callback_escape_delivers_cancel_and_finalizes() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();
        cc.initialize(Some(&param), Some(&info), 0xABCD, 0x1234)
            .unwrap();
        cc.drain_callbacks();
        cc.finish_callback(CELL_MSGDIALOG_BUTTON_ESCAPE).unwrap();
        let cbs = cc.drain_callbacks();
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].status, CELL_CROSS_CONTROLLER_STATUS_FINALIZED);
        assert_eq!(cbs[0].error_code, CELL_CROSS_CONTROLLER_ERROR_CANCEL);
        assert_eq!(cbs[0].userdata, 0x1234);
        assert_eq!(cc.phase(), CrossControllerPhase::Finalized);
        assert!(!cc.is_thread_running());
        assert!(!cc.is_dialog_open());
    }

    #[test]
    fn finalize_prevents_reinit() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();
        cc.initialize(Some(&param), Some(&info), 1, 0).unwrap();
        cc.finish_callback(CELL_MSGDIALOG_BUTTON_ESCAPE).unwrap();
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE)
        );
    }

    #[test]
    fn deliver_connection_result_ok_path() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();
        cc.initialize(Some(&param), Some(&info), 0xA, 0xB).unwrap();
        cc.drain_callbacks();
        cc.deliver_connection_result(CellError::OK).unwrap();
        let cbs = cc.drain_callbacks();
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].status, CELL_CROSS_CONTROLLER_STATUS_FINALIZED);
        assert_eq!(cbs[0].error_code, CellError::OK);
        assert_eq!(cc.phase(), CrossControllerPhase::Finalized);
    }

    #[test]
    fn deliver_connection_result_internal_error_path() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();
        cc.initialize(Some(&param), Some(&info), 1, 0).unwrap();
        cc.drain_callbacks();
        cc.deliver_connection_result(CELL_CROSS_CONTROLLER_ERROR_INTERNAL)
            .unwrap();
        let cbs = cc.drain_callbacks();
        assert_eq!(cbs[0].error_code, CELL_CROSS_CONTROLLER_ERROR_INTERNAL);
    }

    #[test]
    fn deliver_connection_result_without_init_rejected() {
        let mut cc = CrossController::new();
        assert_eq!(
            cc.deliver_connection_result(CellError::OK),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE)
        );
    }

    #[test]
    fn full_crosscontroller_lifecycle_smoke() {
        let mut cc = CrossController::new();
        let param = make_param();
        let info = make_info();

        // 1. Happy-path init: thread + dialog come up, INITIALIZED
        //    callback is queued.
        cc.initialize(Some(&param), Some(&info), 0x8000_0000, 0xFEEDFACE)
            .unwrap();
        assert_eq!(cc.pending_callback_count(), 1);

        // 2. Simulate connection success via the stubbed Vita hand-shake
        //    at cpp:49-60. The FINALIZED+OK deferred notify is queued.
        cc.deliver_connection_result(CellError::OK).unwrap();

        let cbs = cc.drain_callbacks();
        assert_eq!(cbs.len(), 2);
        assert_eq!(cbs[0].status, CELL_CROSS_CONTROLLER_STATUS_INITIALIZED);
        assert_eq!(cbs[0].error_code, CellError::OK);
        assert_eq!(cbs[1].status, CELL_CROSS_CONTROLLER_STATUS_FINALIZED);
        assert_eq!(cbs[1].error_code, CellError::OK);
        assert!(cbs.iter().all(|c| c.userdata == 0xFEEDFACE));

        // 3. Terminal state rejects re-init and further callbacks.
        assert_eq!(
            cc.initialize(Some(&param), Some(&info), 1, 0),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE)
        );
        assert_eq!(
            cc.finish_callback(CELL_MSGDIALOG_BUTTON_ESCAPE),
            Err(CELL_CROSS_CONTROLLER_ERROR_INVALID_STATE)
        );
    }
}
