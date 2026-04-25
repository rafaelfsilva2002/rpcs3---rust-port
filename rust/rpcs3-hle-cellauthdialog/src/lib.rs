//! `rpcs3-hle-cellauthdialog` — PS3 authentication-dialog utility HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellAuthDialog.cpp` (66 linhas).  The
//! firmware dialog used to prompt a player for PSN credentials.  The
//! RPCS3 port is stubs after a trivial argument check; the Rust port
//! adds an observable Open → Abort/Close FSM so higher layers can
//! exercise the lifecycle and verify byte-exact error codes.
//!
//! ## Entry points covered
//!
//! | C++ function            | Rust wrapper                  |
//! |-------------------------|-------------------------------|
//! | `cellAuthDialogOpen`    | [`AuthDialog::open`]          |
//! | `cellAuthDialogAbort`   | [`AuthDialog::abort`]         |
//! | `cellAuthDialogClose`   | [`AuthDialog::close`]         |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellAuthDialog.cpp:9-12
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    /// Undocumented error observed at offset 0x201.
    pub const UNKNOWN_201: CellError = CellError(0x8002_D201);
    /// `cellAuthDialogOpen` with `arg1 == 0` returns this exact code
    /// (cpp:35-36).
    pub const ARG1_IS_ZERO: CellError = CellError(0x8002_D202);
    /// Error returned if Abort/Close are called before the dialog has
    /// been initialized (cpp:46, 56 comments).
    pub const UNKNOWN_203: CellError = CellError(0x8002_D203);
}

// =====================================================================
// Lifecycle FSM
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthDialogState {
    /// No open dialog.  Abort/Close in this state return `UNKNOWN_203`.
    Idle,
    /// `cellAuthDialogOpen` succeeded — dialog is visible.
    Open,
    /// `cellAuthDialogAbort` was called while open.
    Aborted,
    /// `cellAuthDialogClose` was called while open.
    Closed,
}

impl Default for AuthDialogState {
    fn default() -> Self { Self::Idle }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuthDialog {
    pub state: AuthDialogState,
    /// `arg1` captured on the most recent successful `Open`.
    pub last_arg1: u64,
    /// How many times Open returned ARG1_IS_ZERO.
    pub rejected_opens: u32,
}

impl AuthDialog {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `cellAuthDialogOpen` (cpp:31-39).  The C++ source
    /// explicitly logs "arg1 is s64 but the check is for == 0 instead
    /// of >= 0", so negative values are accepted.
    ///
    /// # Errors
    /// * [`errors::ARG1_IS_ZERO`] if `arg1 == 0`.
    pub fn open(&mut self, arg1: u64) -> Result<(), CellError> {
        if arg1 == 0 {
            self.rejected_opens = self.rejected_opens.saturating_add(1);
            return Err(errors::ARG1_IS_ZERO);
        }
        self.last_arg1 = arg1;
        self.state = AuthDialogState::Open;
        Ok(())
    }

    /// Port of `cellAuthDialogAbort` (cpp:41-49).  The C++ stub always
    /// returns `CELL_OK`; the Rust port mirrors the firmware intent
    /// (per the commented-out guard on cpp:46) by returning
    /// `UNKNOWN_203` when called from `Idle`.
    ///
    /// # Errors
    /// * [`errors::UNKNOWN_203`] if no dialog is open.
    pub fn abort(&mut self) -> Result<(), CellError> {
        if self.state != AuthDialogState::Open {
            return Err(errors::UNKNOWN_203);
        }
        self.state = AuthDialogState::Aborted;
        Ok(())
    }

    /// Port of `cellAuthDialogClose` (cpp:51-59).  Same pattern as
    /// [`AuthDialog::abort`].
    ///
    /// # Errors
    /// * [`errors::UNKNOWN_203`] if no dialog is open.
    pub fn close(&mut self) -> Result<(), CellError> {
        if self.state != AuthDialogState::Open {
            return Err(errors::UNKNOWN_203);
        }
        self.state = AuthDialogState::Closed;
        Ok(())
    }

    /// Convenience: reset to `Idle` after a terminal state.  The
    /// firmware lets the caller re-open the dialog after abort/close.
    pub fn reset(&mut self) {
        self.state = AuthDialogState::Idle;
    }
}

// =====================================================================
// Registry
// =====================================================================

pub const MODULE_NAME: &str = "cellAuthDialogUtility";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellAuthDialogOpen",
    "cellAuthDialogAbort",
    "cellAuthDialogClose",
];

#[must_use]
pub fn is_registered(name: &str) -> bool {
    REGISTERED_ENTRY_POINTS.contains(&name)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::UNKNOWN_201.0,  0x8002_D201);
        assert_eq!(errors::ARG1_IS_ZERO.0, 0x8002_D202);
        assert_eq!(errors::UNKNOWN_203.0,  0x8002_D203);
    }

    #[test]
    fn error_codes_contiguous() {
        assert_eq!(errors::ARG1_IS_ZERO.0 - errors::UNKNOWN_201.0, 1);
        assert_eq!(errors::UNKNOWN_203.0 - errors::ARG1_IS_ZERO.0, 1);
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellAuthDialogUtility");
    }

    #[test]
    fn registry_has_three_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 3);
    }

    #[test]
    fn registry_order_matches_cpp() {
        // cpp:63-65.
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellAuthDialogOpen");
        assert_eq!(REGISTERED_ENTRY_POINTS[1], "cellAuthDialogAbort");
        assert_eq!(REGISTERED_ENTRY_POINTS[2], "cellAuthDialogClose");
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("cellAuthDialogUnknown"));
    }

    // ---- open -------------------------------------------------------

    #[test]
    fn open_with_zero_is_arg1_is_zero() {
        let mut d = AuthDialog::new();
        assert_eq!(d.open(0).unwrap_err(), errors::ARG1_IS_ZERO);
        assert_eq!(d.state, AuthDialogState::Idle);
        assert_eq!(d.rejected_opens, 1);
    }

    #[test]
    fn open_with_nonzero_transitions_to_open() {
        let mut d = AuthDialog::new();
        d.open(0x1234_5678).unwrap();
        assert_eq!(d.state, AuthDialogState::Open);
        assert_eq!(d.last_arg1, 0x1234_5678);
    }

    #[test]
    fn open_accepts_max_u64() {
        let mut d = AuthDialog::new();
        d.open(u64::MAX).unwrap();
        assert_eq!(d.last_arg1, u64::MAX);
    }

    #[test]
    fn open_accepts_sign_bit() {
        // C++ comment: "arg1 is s64 but the check is for == 0 instead
        // of >= 0", so negative-as-u64 values are accepted.
        let mut d = AuthDialog::new();
        d.open(0x8000_0000_0000_0000).unwrap();
        assert_eq!(d.state, AuthDialogState::Open);
    }

    #[test]
    fn open_rejected_counter_counts_zero_attempts() {
        let mut d = AuthDialog::new();
        assert_eq!(d.open(0).unwrap_err(), errors::ARG1_IS_ZERO);
        assert_eq!(d.open(0).unwrap_err(), errors::ARG1_IS_ZERO);
        assert_eq!(d.open(0).unwrap_err(), errors::ARG1_IS_ZERO);
        assert_eq!(d.rejected_opens, 3);
    }

    // ---- abort ------------------------------------------------------

    #[test]
    fn abort_while_idle_is_unknown_203() {
        let mut d = AuthDialog::new();
        assert_eq!(d.abort().unwrap_err(), errors::UNKNOWN_203);
    }

    #[test]
    fn abort_open_dialog_transitions_to_aborted() {
        let mut d = AuthDialog::new();
        d.open(1).unwrap();
        d.abort().unwrap();
        assert_eq!(d.state, AuthDialogState::Aborted);
    }

    #[test]
    fn abort_after_close_is_unknown_203() {
        let mut d = AuthDialog::new();
        d.open(1).unwrap();
        d.close().unwrap();
        assert_eq!(d.abort().unwrap_err(), errors::UNKNOWN_203);
    }

    #[test]
    fn abort_after_abort_is_unknown_203() {
        let mut d = AuthDialog::new();
        d.open(1).unwrap();
        d.abort().unwrap();
        assert_eq!(d.abort().unwrap_err(), errors::UNKNOWN_203);
    }

    // ---- close ------------------------------------------------------

    #[test]
    fn close_while_idle_is_unknown_203() {
        let mut d = AuthDialog::new();
        assert_eq!(d.close().unwrap_err(), errors::UNKNOWN_203);
    }

    #[test]
    fn close_open_dialog_transitions_to_closed() {
        let mut d = AuthDialog::new();
        d.open(1).unwrap();
        d.close().unwrap();
        assert_eq!(d.state, AuthDialogState::Closed);
    }

    #[test]
    fn close_after_abort_is_unknown_203() {
        let mut d = AuthDialog::new();
        d.open(1).unwrap();
        d.abort().unwrap();
        assert_eq!(d.close().unwrap_err(), errors::UNKNOWN_203);
    }

    // ---- reset ------------------------------------------------------

    #[test]
    fn reset_clears_state_to_idle() {
        let mut d = AuthDialog::new();
        d.open(1).unwrap();
        d.close().unwrap();
        d.reset();
        assert_eq!(d.state, AuthDialogState::Idle);
    }

    #[test]
    fn reset_allows_reopen() {
        let mut d = AuthDialog::new();
        d.open(1).unwrap();
        d.abort().unwrap();
        d.reset();
        d.open(2).unwrap();
        assert_eq!(d.state, AuthDialogState::Open);
        assert_eq!(d.last_arg1, 2);
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_authdialog_lifecycle_smoke() {
        let mut d = AuthDialog::new();

        // 1. Zero-arg open fails.
        assert_eq!(d.open(0).unwrap_err(), errors::ARG1_IS_ZERO);
        assert_eq!(d.rejected_opens, 1);

        // 2. Valid open.
        d.open(0xDEAD_BEEF).unwrap();
        assert_eq!(d.state, AuthDialogState::Open);

        // 3. User cancels → abort.
        d.abort().unwrap();
        assert_eq!(d.state, AuthDialogState::Aborted);

        // 4. Subsequent abort/close in terminal state = UNKNOWN_203.
        assert_eq!(d.abort().unwrap_err(), errors::UNKNOWN_203);
        assert_eq!(d.close().unwrap_err(), errors::UNKNOWN_203);

        // 5. Reset + reopen.
        d.reset();
        d.open(0xCAFE).unwrap();
        d.close().unwrap();
        assert_eq!(d.state, AuthDialogState::Closed);
    }
}
