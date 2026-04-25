//! `rpcs3-hle-libmedi` — PS3 Mediator telemetry library HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/libmedi.cpp` (79 linhas).  Every
//! entry point is an `UNIMPLEMENTED_FUNC` stub in RPCS3; the Rust port
//! adds an observable lifecycle FSM so higher layers can exercise the
//! Create → GetUserInfo/GetProviderUrl → Sign/PostReports → FlushCache
//! → Close flow the firmware would enforce.
//!
//! ## Entry points covered
//!
//! | C++ function                         | Rust wrapper                        |
//! |--------------------------------------|-------------------------------------|
//! | `cellMediatorCreateContext`          | [`Mediator::create_context`]        |
//! | `cellMediatorCloseContext`           | [`Mediator::close_context`]         |
//! | `cellMediatorGetStatus`              | [`Mediator::get_status`]            |
//! | `cellMediatorGetProviderUrl`         | [`Mediator::get_provider_url`]      |
//! | `cellMediatorGetUserInfo`            | [`Mediator::get_user_info`]         |
//! | `cellMediatorGetSignatureLength`     | [`Mediator::get_signature_length`]  |
//! | `cellMediatorSign`                   | [`Mediator::sign`]                  |
//! | `cellMediatorPostReports`            | [`Mediator::post_reports`]          |
//! | `cellMediatorReliablePostReports`    | [`Mediator::reliable_post_reports`] |
//! | `cellMediatorFlushCache`             | [`Mediator::flush_cache`]           |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Constants
// =====================================================================

/// Fixed signature length advertised by `cellMediatorGetSignatureLength`.
/// The firmware stub returns `CELL_OK` without writing — higher layers
/// want to know the PS3's actual 256-byte signature size, so the port
/// exposes that here.
pub const MEDIATOR_SIGNATURE_LENGTH: u32 = 256;

// =====================================================================
// Lifecycle state
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediatorState {
    /// `cellMediatorCreateContext` has not been called.
    Uninitialized,
    /// Context is open; queries and signing are permitted.
    Open,
    /// A `cellMediatorClose` call is in flight / completed.
    Closed,
}

// =====================================================================
// Manager
// =====================================================================

/// Mirror of the `libmedi` observable state.  The firmware singleton
/// holds a single Mediator context (the C++ stub doesn't actually
/// store anything); the port preserves the open/closed FSM and tracks
/// a few counters so tests can verify operation ordering.
#[derive(Debug, Clone, Default)]
pub struct Mediator {
    pub state: MediatorState,
    pub reports_posted: u32,
    pub reliable_reports_posted: u32,
    pub cache_flushed_count: u32,
}

impl Default for MediatorState {
    fn default() -> Self { Self::Uninitialized }
}

impl Mediator {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `cellMediatorCreateContext` — transitions
    /// `Uninitialized → Open`.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if already open or closed (no re-init without
    ///   destroying the manager first).
    pub fn create_context(&mut self) -> Result<(), CellError> {
        if self.state != MediatorState::Uninitialized {
            return Err(CELL_EINVAL);
        }
        self.state = MediatorState::Open;
        Ok(())
    }

    /// Port of `cellMediatorCloseContext` — transitions `Open → Closed`.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if not currently open.
    pub fn close_context(&mut self) -> Result<(), CellError> {
        if self.state != MediatorState::Open {
            return Err(CELL_EINVAL);
        }
        self.state = MediatorState::Closed;
        Ok(())
    }

    /// Generic "operation requires open" guard — most Mediator calls
    /// have the same precondition.
    fn require_open(&self) -> Result<(), CellError> {
        if self.state != MediatorState::Open {
            return Err(CELL_EINVAL);
        }
        Ok(())
    }

    /// Port of `cellMediatorGetStatus` — returns the current state as
    /// a byte-exact-friendly integer.
    pub fn get_status(&self) -> Result<MediatorState, CellError> {
        // C++ stub doesn't check for open; we surface the state as
        // read-only without validation.
        Ok(self.state)
    }

    /// Port of `cellMediatorGetProviderUrl` — stub returns `CELL_OK`;
    /// port returns a static default URL.
    pub fn get_provider_url(&self) -> Result<&'static str, CellError> {
        self.require_open()?;
        Ok("https://mediator.ps3.sony.net/")
    }

    /// Port of `cellMediatorGetUserInfo`.
    pub fn get_user_info(&self) -> Result<(), CellError> { self.require_open() }

    /// Port of `cellMediatorGetSignatureLength`.
    pub fn get_signature_length(&self) -> Result<u32, CellError> {
        self.require_open()?;
        Ok(MEDIATOR_SIGNATURE_LENGTH)
    }

    /// Port of `cellMediatorSign`.
    pub fn sign(&self) -> Result<(), CellError> { self.require_open() }

    /// Port of `cellMediatorPostReports` — bumps the best-effort counter.
    pub fn post_reports(&mut self) -> Result<(), CellError> {
        self.require_open()?;
        self.reports_posted = self.reports_posted.saturating_add(1);
        Ok(())
    }

    /// Port of `cellMediatorReliablePostReports` — bumps the reliable
    /// counter.
    pub fn reliable_post_reports(&mut self) -> Result<(), CellError> {
        self.require_open()?;
        self.reliable_reports_posted = self.reliable_reports_posted.saturating_add(1);
        Ok(())
    }

    /// Port of `cellMediatorFlushCache` — bumps the flush counter.
    pub fn flush_cache(&mut self) -> Result<(), CellError> {
        self.require_open()?;
        self.cache_flushed_count = self.cache_flushed_count.saturating_add(1);
        Ok(())
    }
}

// =====================================================================
// Registry
// =====================================================================

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellMediatorCloseContext",
    "cellMediatorCreateContext",
    "cellMediatorFlushCache",
    "cellMediatorGetProviderUrl",
    "cellMediatorGetSignatureLength",
    "cellMediatorGetStatus",
    "cellMediatorGetUserInfo",
    "cellMediatorPostReports",
    "cellMediatorReliablePostReports",
    "cellMediatorSign",
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
    fn cell_einval_byte_exact() {
        assert_eq!(CELL_EINVAL.0, 0x8001_0002);
    }

    #[test]
    fn signature_length_byte_exact() {
        assert_eq!(MEDIATOR_SIGNATURE_LENGTH, 256);
    }

    // ---- state machine ----------------------------------------------

    #[test]
    fn fresh_mediator_is_uninitialized() {
        let m = Mediator::new();
        assert_eq!(m.state, MediatorState::Uninitialized);
    }

    #[test]
    fn create_context_transitions_to_open() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        assert_eq!(m.state, MediatorState::Open);
    }

    #[test]
    fn create_twice_is_einval() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        assert_eq!(m.create_context().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn close_without_create_is_einval() {
        let mut m = Mediator::new();
        assert_eq!(m.close_context().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn close_after_open_transitions_to_closed() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        m.close_context().unwrap();
        assert_eq!(m.state, MediatorState::Closed);
    }

    #[test]
    fn double_close_is_einval() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        m.close_context().unwrap();
        assert_eq!(m.close_context().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn cannot_reopen_after_close() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        m.close_context().unwrap();
        // The firmware requires a fresh manager after close.
        assert_eq!(m.create_context().unwrap_err(), CELL_EINVAL);
    }

    // ---- operation guards -------------------------------------------

    #[test]
    fn sign_requires_open() {
        let m = Mediator::new();
        assert_eq!(m.sign().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn provider_url_requires_open() {
        let m = Mediator::new();
        assert!(m.get_provider_url().is_err());
    }

    #[test]
    fn get_provider_url_when_open() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        assert_eq!(m.get_provider_url().unwrap(), "https://mediator.ps3.sony.net/");
    }

    #[test]
    fn get_user_info_requires_open() {
        let m = Mediator::new();
        assert_eq!(m.get_user_info().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn get_signature_length_when_open() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        assert_eq!(m.get_signature_length().unwrap(), 256);
    }

    #[test]
    fn get_signature_length_when_closed_is_einval() {
        let m = Mediator::new();
        assert_eq!(m.get_signature_length().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn get_status_always_returns_state() {
        // get_status is the one call that doesn't require Open —
        // matches C++ stub behaviour (returns CELL_OK unconditionally).
        let m = Mediator::new();
        assert_eq!(m.get_status().unwrap(), MediatorState::Uninitialized);
        let mut m = Mediator::new();
        m.create_context().unwrap();
        assert_eq!(m.get_status().unwrap(), MediatorState::Open);
    }

    // ---- counters ---------------------------------------------------

    #[test]
    fn post_reports_increments_counter() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        m.post_reports().unwrap();
        m.post_reports().unwrap();
        m.post_reports().unwrap();
        assert_eq!(m.reports_posted, 3);
    }

    #[test]
    fn reliable_post_reports_separate_counter() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        m.post_reports().unwrap();
        m.reliable_post_reports().unwrap();
        m.reliable_post_reports().unwrap();
        assert_eq!(m.reports_posted, 1);
        assert_eq!(m.reliable_reports_posted, 2);
    }

    #[test]
    fn flush_cache_increments_counter() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        m.flush_cache().unwrap();
        m.flush_cache().unwrap();
        assert_eq!(m.cache_flushed_count, 2);
    }

    #[test]
    fn counters_rejected_when_not_open() {
        let mut m = Mediator::new();
        assert_eq!(m.post_reports().unwrap_err(), CELL_EINVAL);
        assert_eq!(m.reliable_post_reports().unwrap_err(), CELL_EINVAL);
        assert_eq!(m.flush_cache().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn counters_rejected_after_close() {
        let mut m = Mediator::new();
        m.create_context().unwrap();
        m.close_context().unwrap();
        assert_eq!(m.post_reports().unwrap_err(), CELL_EINVAL);
    }

    // ---- registry ---------------------------------------------------

    #[test]
    fn registry_has_ten_entries() {
        // cpp:69-78 has exactly 10 REG_FUNC calls.
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 10);
    }

    #[test]
    fn registry_alphabetical_order_matches_cpp() {
        // The C++ REG_FUNC block is alphabetical by function name.
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellMediatorCloseContext");
        assert_eq!(REGISTERED_ENTRY_POINTS[1], "cellMediatorCreateContext");
        assert_eq!(REGISTERED_ENTRY_POINTS[9], "cellMediatorSign");
    }

    #[test]
    fn registry_is_registered_helper() {
        assert!(is_registered("cellMediatorCreateContext"));
        assert!(!is_registered("cellMediatorMissing"));
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_libmedi_lifecycle_smoke() {
        let mut m = Mediator::new();
        assert_eq!(m.state, MediatorState::Uninitialized);

        // 1. Pre-open: every operation except get_status fails.
        assert!(m.sign().is_err());
        assert!(m.post_reports().is_err());
        assert_eq!(m.get_status().unwrap(), MediatorState::Uninitialized);

        // 2. Create context.
        m.create_context().unwrap();
        assert_eq!(m.state, MediatorState::Open);

        // 3. Query provider + user info + signature length.
        assert_eq!(m.get_provider_url().unwrap(), "https://mediator.ps3.sony.net/");
        m.get_user_info().unwrap();
        assert_eq!(m.get_signature_length().unwrap(), 256);

        // 4. Sign + post some reports.
        m.sign().unwrap();
        m.post_reports().unwrap();
        m.reliable_post_reports().unwrap();
        m.reliable_post_reports().unwrap();

        // 5. Flush the cache.
        m.flush_cache().unwrap();

        // 6. Close.
        m.close_context().unwrap();
        assert_eq!(m.state, MediatorState::Closed);

        // 7. All ops fail after close.
        assert!(m.sign().is_err());

        // 8. Counters preserved.
        assert_eq!(m.reports_posted, 1);
        assert_eq!(m.reliable_reports_posted, 2);
        assert_eq!(m.cache_flushed_count, 1);
    }
}
