//! Rust port of `rpcs3/Emu/Cell/Modules/libad_async.cpp`.
//!
//! Async counterpart of `libad_core`. The C++ source (51 lines) declares
//! 6 PRX entry points under the module name `libad_async`; every body is a
//! stub that logs `UNIMPLEMENTED_FUNC` and returns `CELL_OK`.
//!
//! Module name is `libad_async` byte-exact at cpp:4 `LOG_CHANNEL` and
//! cpp:42 `DECLARE(ppu_module_manager::libad_async)("libad_async", ...)`.
//!
//! REG_FUNC order at cpp:44-49:
//!
//!  1. `sceAdAsyncOpenContext`
//!  2. `sceAdAsyncConnectContext`
//!  3. `sceAdAsyncSpaceOpen`
//!  4. `sceAdAsyncFlushReports`
//!  5. `sceAdAsyncSpaceClose`
//!  6. `sceAdAsyncCloseContext`
//!
//! Like the sync libad_core port, every successful call returns `CELL_OK`
//! (preserving C++ happy-path); FSM enforcement is layered on top for the
//! Rust side so mis-sequenced calls surface an error instead of silent
//! success. Unlike libad_core, every call issues a *request id* which is
//! paired with a completion callback in [`AdAsync::drain_callbacks`] —
//! matching the async firmware convention where the PRX posts a result to
//! a user-supplied callback once the real operation finishes.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Module name byte-exact at cpp:4 / cpp:42.
pub const MODULE_NAME: &str = "libad_async";

/// Ordered entry points as registered via `REG_FUNC` at cpp:44-49.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceAdAsyncOpenContext",
    "sceAdAsyncConnectContext",
    "sceAdAsyncSpaceOpen",
    "sceAdAsyncFlushReports",
    "sceAdAsyncSpaceClose",
    "sceAdAsyncCloseContext",
];

// --- Error codes ---------------------------------------------------------
//
// C++ commits no named error codes (stub). Facility `0x8002_E2__` is
// reserved in this port for the async sceAd surface (parallel to
// `0x8002_E1__` for libad_core).

/// Any async call issued before `open_context`.
pub const SCE_AD_ASYNC_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_E201);
/// `open_context` issued twice without an intervening close.
pub const SCE_AD_ASYNC_ERROR_ALREADY_OPEN: CellError = CellError(0x8002_E202);
/// `space_open` / `flush_reports` invoked while the context is not
/// connected.
pub const SCE_AD_ASYNC_ERROR_NOT_CONNECTED: CellError = CellError(0x8002_E203);
/// `connect_context` called twice without a close.
pub const SCE_AD_ASYNC_ERROR_ALREADY_CONNECTED: CellError = CellError(0x8002_E204);
/// Any call against a context already closed (terminal state).
pub const SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED: CellError = CellError(0x8002_E205);
/// `space_close` or re-open for a space id that was never opened.
pub const SCE_AD_ASYNC_ERROR_SPACE_NOT_FOUND: CellError = CellError(0x8002_E206);
/// `space_open` for a space id already opened.
pub const SCE_AD_ASYNC_ERROR_SPACE_ALREADY_OPEN: CellError = CellError(0x8002_E207);
/// `space_close` for an id that isn't currently open (already closed).
pub const SCE_AD_ASYNC_ERROR_SPACE_NOT_OPEN: CellError = CellError(0x8002_E208);

// --- FSM ----------------------------------------------------------------

/// Context lifecycle — parallel to [`crate::libad_core::AdContextState`]
/// but separate since the async surface has its own bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdAsyncContextState {
    Uninitialized,
    Open,
    Connected,
    Closed,
}

/// Which entry point produced a callback. Used by
/// [`AdAsync::drain_callbacks`] consumers to route completions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdAsyncCallbackKind {
    OpenContext,
    ConnectContext,
    SpaceOpen,
    FlushReports,
    SpaceClose,
    CloseContext,
}

/// One queued completion. `request_id` matches the id returned by the
/// originating call so the consumer can correlate callbacks to requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdAsyncCallback {
    pub kind: AdAsyncCallbackKind,
    pub request_id: u32,
    pub result: CellError,
    pub userdata: u64,
}

/// An ad report queued by the game and drained through `flush_reports`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdAsyncReport {
    pub space_id: u32,
    pub event: u32,
    pub timestamp: u64,
}

/// HLE state for one libad_async context.
#[derive(Debug, Default)]
pub struct AdAsync {
    state: Option<AdAsyncContextState>,
    // Spaces currently open. (id, userdata). The SDK identifies spaces by
    // an opaque handle; here we use the game-supplied id directly.
    open_spaces: Vec<u32>,
    pending_callbacks: Vec<AdAsyncCallback>,
    reports: Vec<AdAsyncReport>,
    next_request_id: u32,
    // Counters per entry point, handy for verifying a lifecycle drove each
    // surface exactly once.
    open_context_calls: u32,
    connect_context_calls: u32,
    space_open_calls: u32,
    flush_reports_calls: u32,
    space_close_calls: u32,
    close_context_calls: u32,
}

impl AdAsync {
    /// Construct an empty async context manager.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: None,
            open_spaces: Vec::new(),
            pending_callbacks: Vec::new(),
            reports: Vec::new(),
            next_request_id: 1,
            open_context_calls: 0,
            connect_context_calls: 0,
            space_open_calls: 0,
            flush_reports_calls: 0,
            space_close_calls: 0,
            close_context_calls: 0,
        }
    }

    #[must_use]
    pub fn state(&self) -> AdAsyncContextState {
        self.state.unwrap_or(AdAsyncContextState::Uninitialized)
    }

    #[must_use]
    pub fn pending_callback_count(&self) -> usize {
        self.pending_callbacks.len()
    }

    #[must_use]
    pub fn queued_report_count(&self) -> usize {
        self.reports.len()
    }

    #[must_use]
    pub fn open_space_count(&self) -> usize {
        self.open_spaces.len()
    }

    #[must_use]
    pub fn is_space_open(&self, space_id: u32) -> bool {
        self.open_spaces.iter().any(|&id| id == space_id)
    }

    // --- counters ---

    #[must_use]
    pub fn open_context_calls(&self) -> u32 {
        self.open_context_calls
    }
    #[must_use]
    pub fn connect_context_calls(&self) -> u32 {
        self.connect_context_calls
    }
    #[must_use]
    pub fn space_open_calls(&self) -> u32 {
        self.space_open_calls
    }
    #[must_use]
    pub fn flush_reports_calls(&self) -> u32 {
        self.flush_reports_calls
    }
    #[must_use]
    pub fn space_close_calls(&self) -> u32 {
        self.space_close_calls
    }
    #[must_use]
    pub fn close_context_calls(&self) -> u32 {
        self.close_context_calls
    }

    // --- helpers ---

    fn alloc_request_id(&mut self) -> u32 {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        id
    }

    fn queue_callback(
        &mut self,
        kind: AdAsyncCallbackKind,
        request_id: u32,
        result: CellError,
        userdata: u64,
    ) {
        self.pending_callbacks.push(AdAsyncCallback {
            kind,
            request_id,
            result,
            userdata,
        });
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        match self.state() {
            AdAsyncContextState::Uninitialized => Err(SCE_AD_ASYNC_ERROR_NOT_INITIALIZED),
            AdAsyncContextState::Closed => Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED),
            _ => Ok(()),
        }
    }

    fn require_connected(&self) -> Result<(), CellError> {
        match self.state() {
            AdAsyncContextState::Uninitialized => Err(SCE_AD_ASYNC_ERROR_NOT_INITIALIZED),
            AdAsyncContextState::Closed => Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED),
            AdAsyncContextState::Open => Err(SCE_AD_ASYNC_ERROR_NOT_CONNECTED),
            AdAsyncContextState::Connected => Ok(()),
        }
    }

    // --- entry points ---

    /// `sceAdAsyncOpenContext` (cpp:6-10). Queues an `OpenContext`
    /// completion and transitions `Uninitialized → Open`. Returns the
    /// allocated request id.
    pub fn open_context(&mut self, userdata: u64) -> Result<u32, CellError> {
        match self.state() {
            AdAsyncContextState::Uninitialized => {
                self.state = Some(AdAsyncContextState::Open);
                self.open_context_calls = self.open_context_calls.saturating_add(1);
                let id = self.alloc_request_id();
                self.queue_callback(AdAsyncCallbackKind::OpenContext, id, CellError::OK, userdata);
                Ok(id)
            }
            AdAsyncContextState::Open | AdAsyncContextState::Connected => {
                Err(SCE_AD_ASYNC_ERROR_ALREADY_OPEN)
            }
            AdAsyncContextState::Closed => Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED),
        }
    }

    /// `sceAdAsyncConnectContext` (cpp:12-16). `Open → Connected`.
    pub fn connect_context(&mut self, userdata: u64) -> Result<u32, CellError> {
        match self.state() {
            AdAsyncContextState::Uninitialized => Err(SCE_AD_ASYNC_ERROR_NOT_INITIALIZED),
            AdAsyncContextState::Open => {
                self.state = Some(AdAsyncContextState::Connected);
                self.connect_context_calls = self.connect_context_calls.saturating_add(1);
                let id = self.alloc_request_id();
                self.queue_callback(
                    AdAsyncCallbackKind::ConnectContext,
                    id,
                    CellError::OK,
                    userdata,
                );
                Ok(id)
            }
            AdAsyncContextState::Connected => Err(SCE_AD_ASYNC_ERROR_ALREADY_CONNECTED),
            AdAsyncContextState::Closed => Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED),
        }
    }

    /// `sceAdAsyncSpaceOpen` (cpp:18-22). Opens an ad space by id.
    /// Requires an active connection.
    pub fn space_open(&mut self, space_id: u32, userdata: u64) -> Result<u32, CellError> {
        self.require_connected()?;
        if self.is_space_open(space_id) {
            return Err(SCE_AD_ASYNC_ERROR_SPACE_ALREADY_OPEN);
        }
        self.open_spaces.push(space_id);
        self.space_open_calls = self.space_open_calls.saturating_add(1);
        let id = self.alloc_request_id();
        self.queue_callback(AdAsyncCallbackKind::SpaceOpen, id, CellError::OK, userdata);
        Ok(id)
    }

    /// `sceAdAsyncFlushReports` (cpp:24-28). Drains queued reports via
    /// the completion callback and returns them directly to the caller.
    pub fn flush_reports(
        &mut self,
        userdata: u64,
    ) -> Result<(u32, Vec<AdAsyncReport>), CellError> {
        self.require_connected()?;
        self.flush_reports_calls = self.flush_reports_calls.saturating_add(1);
        let reports = core::mem::take(&mut self.reports);
        let id = self.alloc_request_id();
        self.queue_callback(
            AdAsyncCallbackKind::FlushReports,
            id,
            CellError::OK,
            userdata,
        );
        Ok((id, reports))
    }

    /// `sceAdAsyncSpaceClose` (cpp:30-34). Closes an ad space by id.
    /// Errors if the space was never opened or has already been closed.
    pub fn space_close(&mut self, space_id: u32, userdata: u64) -> Result<u32, CellError> {
        self.require_initialized()?;
        let pos = self.open_spaces.iter().position(|&id| id == space_id);
        let Some(pos) = pos else {
            return Err(SCE_AD_ASYNC_ERROR_SPACE_NOT_OPEN);
        };
        self.open_spaces.remove(pos);
        self.space_close_calls = self.space_close_calls.saturating_add(1);
        let id = self.alloc_request_id();
        self.queue_callback(AdAsyncCallbackKind::SpaceClose, id, CellError::OK, userdata);
        Ok(id)
    }

    /// `sceAdAsyncCloseContext` (cpp:36-40). Terminal transition. All
    /// open spaces are implicitly closed (but no per-space SpaceClose
    /// callback is queued — the SDK sends a single CloseContext
    /// completion).
    pub fn close_context(&mut self, userdata: u64) -> Result<u32, CellError> {
        self.require_initialized()?;
        self.open_spaces.clear();
        self.state = Some(AdAsyncContextState::Closed);
        self.close_context_calls = self.close_context_calls.saturating_add(1);
        let id = self.alloc_request_id();
        self.queue_callback(
            AdAsyncCallbackKind::CloseContext,
            id,
            CellError::OK,
            userdata,
        );
        Ok(id)
    }

    /// Drain the pending completion callbacks in FIFO order.
    pub fn drain_callbacks(&mut self) -> Vec<AdAsyncCallback> {
        core::mem::take(&mut self.pending_callbacks)
    }

    // --- harness helpers (not exposed as FNIDs) ---

    /// Queue a report that a future `flush_reports` will drain.
    pub fn enqueue_report(&mut self, report: AdAsyncReport) -> Result<(), CellError> {
        self.require_connected()?;
        self.reports.push(report);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "libad_async");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(
            REGISTERED_ENTRY_POINTS,
            &[
                "sceAdAsyncOpenContext",
                "sceAdAsyncConnectContext",
                "sceAdAsyncSpaceOpen",
                "sceAdAsyncFlushReports",
                "sceAdAsyncSpaceClose",
                "sceAdAsyncCloseContext",
            ]
        );
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 6);
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(SCE_AD_ASYNC_ERROR_NOT_INITIALIZED.0, 0x8002_E201);
        assert_eq!(SCE_AD_ASYNC_ERROR_ALREADY_OPEN.0, 0x8002_E202);
        assert_eq!(SCE_AD_ASYNC_ERROR_NOT_CONNECTED.0, 0x8002_E203);
        assert_eq!(SCE_AD_ASYNC_ERROR_ALREADY_CONNECTED.0, 0x8002_E204);
        assert_eq!(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED.0, 0x8002_E205);
        assert_eq!(SCE_AD_ASYNC_ERROR_SPACE_NOT_FOUND.0, 0x8002_E206);
        assert_eq!(SCE_AD_ASYNC_ERROR_SPACE_ALREADY_OPEN.0, 0x8002_E207);
        assert_eq!(SCE_AD_ASYNC_ERROR_SPACE_NOT_OPEN.0, 0x8002_E208);
    }

    #[test]
    fn new_starts_uninitialized() {
        let ad = AdAsync::new();
        assert_eq!(ad.state(), AdAsyncContextState::Uninitialized);
        assert_eq!(ad.pending_callback_count(), 0);
        assert_eq!(ad.open_space_count(), 0);
    }

    #[test]
    fn open_queues_callback_and_allocs_id() {
        let mut ad = AdAsync::new();
        let id = ad.open_context(0xDEAD_BEEF).unwrap();
        assert_eq!(id, 1);
        assert_eq!(ad.state(), AdAsyncContextState::Open);
        assert_eq!(ad.pending_callback_count(), 1);
        let cbs = ad.drain_callbacks();
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].kind, AdAsyncCallbackKind::OpenContext);
        assert_eq!(cbs[0].request_id, id);
        assert_eq!(cbs[0].result, CellError::OK);
        assert_eq!(cbs[0].userdata, 0xDEAD_BEEF);
        assert_eq!(ad.pending_callback_count(), 0);
    }

    #[test]
    fn double_open_is_already_open() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        assert_eq!(ad.open_context(0), Err(SCE_AD_ASYNC_ERROR_ALREADY_OPEN));
    }

    #[test]
    fn connect_requires_open() {
        let mut ad = AdAsync::new();
        assert_eq!(ad.connect_context(0), Err(SCE_AD_ASYNC_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn connect_queues_callback() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        let id = ad.connect_context(0x12345).unwrap();
        assert_eq!(id, 2);
        assert_eq!(ad.state(), AdAsyncContextState::Connected);
        let cbs = ad.drain_callbacks();
        assert_eq!(cbs.len(), 2);
        assert_eq!(cbs[1].kind, AdAsyncCallbackKind::ConnectContext);
        assert_eq!(cbs[1].userdata, 0x12345);
    }

    #[test]
    fn double_connect_is_already_connected() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        assert_eq!(ad.connect_context(0), Err(SCE_AD_ASYNC_ERROR_ALREADY_CONNECTED));
    }

    #[test]
    fn space_open_requires_connected() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        assert_eq!(ad.space_open(7, 0), Err(SCE_AD_ASYNC_ERROR_NOT_CONNECTED));
    }

    #[test]
    fn space_open_tracks_id() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        ad.space_open(10, 0xAA).unwrap();
        ad.space_open(20, 0xBB).unwrap();
        assert_eq!(ad.open_space_count(), 2);
        assert!(ad.is_space_open(10));
        assert!(ad.is_space_open(20));
        assert!(!ad.is_space_open(30));
    }

    #[test]
    fn space_open_duplicate_is_error() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        ad.space_open(10, 0).unwrap();
        assert_eq!(
            ad.space_open(10, 0),
            Err(SCE_AD_ASYNC_ERROR_SPACE_ALREADY_OPEN)
        );
    }

    #[test]
    fn space_close_unknown_is_not_open() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        assert_eq!(ad.space_close(99, 0), Err(SCE_AD_ASYNC_ERROR_SPACE_NOT_OPEN));
    }

    #[test]
    fn space_close_removes_from_open_list() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        ad.space_open(10, 0).unwrap();
        ad.space_open(20, 0).unwrap();
        ad.space_close(10, 0).unwrap();
        assert_eq!(ad.open_space_count(), 1);
        assert!(!ad.is_space_open(10));
        assert!(ad.is_space_open(20));
    }

    #[test]
    fn flush_reports_requires_connected() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        assert_eq!(ad.flush_reports(0), Err(SCE_AD_ASYNC_ERROR_NOT_CONNECTED));
    }

    #[test]
    fn flush_reports_drains_queue() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        ad.enqueue_report(AdAsyncReport {
            space_id: 10,
            event: 1,
            timestamp: 100,
        })
        .unwrap();
        ad.enqueue_report(AdAsyncReport {
            space_id: 20,
            event: 2,
            timestamp: 200,
        })
        .unwrap();
        assert_eq!(ad.queued_report_count(), 2);
        let (req, reports) = ad.flush_reports(0xCAFE).unwrap();
        assert_eq!(reports.len(), 2);
        assert_eq!(ad.queued_report_count(), 0);
        let cbs = ad.drain_callbacks();
        let flush = cbs.iter().find(|c| c.request_id == req).unwrap();
        assert_eq!(flush.kind, AdAsyncCallbackKind::FlushReports);
        assert_eq!(flush.userdata, 0xCAFE);
    }

    #[test]
    fn close_context_clears_open_spaces() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        ad.space_open(1, 0).unwrap();
        ad.space_open(2, 0).unwrap();
        ad.close_context(0).unwrap();
        assert_eq!(ad.state(), AdAsyncContextState::Closed);
        assert_eq!(ad.open_space_count(), 0);
    }

    #[test]
    fn close_before_open_is_not_initialized() {
        let mut ad = AdAsync::new();
        assert_eq!(ad.close_context(0), Err(SCE_AD_ASYNC_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn double_close_is_context_closed() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.close_context(0).unwrap();
        assert_eq!(ad.close_context(0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
    }

    #[test]
    fn reopen_after_close_is_rejected() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.close_context(0).unwrap();
        assert_eq!(ad.open_context(0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
    }

    #[test]
    fn space_ops_after_close_are_context_closed() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        ad.close_context(0).unwrap();
        assert_eq!(ad.space_open(1, 0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.space_close(1, 0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.flush_reports(0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
    }

    #[test]
    fn request_ids_are_monotonic() {
        let mut ad = AdAsync::new();
        let a = ad.open_context(0).unwrap();
        let b = ad.connect_context(0).unwrap();
        let c = ad.space_open(1, 0).unwrap();
        let d = ad.space_open(2, 0).unwrap();
        assert!(a < b);
        assert!(b < c);
        assert!(c < d);
    }

    #[test]
    fn counters_match_call_count() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        ad.connect_context(0).unwrap();
        ad.space_open(1, 0).unwrap();
        ad.space_open(2, 0).unwrap();
        ad.flush_reports(0).unwrap();
        ad.space_close(1, 0).unwrap();
        ad.close_context(0).unwrap();
        assert_eq!(ad.open_context_calls(), 1);
        assert_eq!(ad.connect_context_calls(), 1);
        assert_eq!(ad.space_open_calls(), 2);
        assert_eq!(ad.flush_reports_calls(), 1);
        assert_eq!(ad.space_close_calls(), 1);
        assert_eq!(ad.close_context_calls(), 1);
    }

    #[test]
    fn callbacks_preserve_fifo_order() {
        let mut ad = AdAsync::new();
        ad.open_context(0x1).unwrap();
        ad.connect_context(0x2).unwrap();
        ad.space_open(1, 0x3).unwrap();
        let cbs = ad.drain_callbacks();
        assert_eq!(cbs.len(), 3);
        assert_eq!(cbs[0].kind, AdAsyncCallbackKind::OpenContext);
        assert_eq!(cbs[0].userdata, 0x1);
        assert_eq!(cbs[1].kind, AdAsyncCallbackKind::ConnectContext);
        assert_eq!(cbs[1].userdata, 0x2);
        assert_eq!(cbs[2].kind, AdAsyncCallbackKind::SpaceOpen);
        assert_eq!(cbs[2].userdata, 0x3);
    }

    #[test]
    fn drain_callbacks_empties_queue() {
        let mut ad = AdAsync::new();
        ad.open_context(0).unwrap();
        assert_eq!(ad.pending_callback_count(), 1);
        let first = ad.drain_callbacks();
        assert_eq!(first.len(), 1);
        assert_eq!(ad.pending_callback_count(), 0);
        let again = ad.drain_callbacks();
        assert!(again.is_empty());
    }

    #[test]
    fn full_libad_async_lifecycle_smoke() {
        let mut ad = AdAsync::new();

        // 1. Open (sets state Open, queues OpenContext cb).
        let req_open = ad.open_context(0x1001).unwrap();

        // 2. Connect (sets state Connected, queues ConnectContext cb).
        let req_conn = ad.connect_context(0x1002).unwrap();

        // 3. Open two ad spaces.
        let req_s1 = ad.space_open(1000, 0x2000).unwrap();
        let req_s2 = ad.space_open(2000, 0x2001).unwrap();
        assert_eq!(ad.open_space_count(), 2);

        // 4. Queue reports + flush.
        ad.enqueue_report(AdAsyncReport {
            space_id: 1000,
            event: 1, // impression
            timestamp: 100,
        })
        .unwrap();
        ad.enqueue_report(AdAsyncReport {
            space_id: 2000,
            event: 2, // click
            timestamp: 200,
        })
        .unwrap();
        let (req_flush, reports) = ad.flush_reports(0x3000).unwrap();
        assert_eq!(reports.len(), 2);

        // 5. Close one space explicitly.
        let req_sc = ad.space_close(1000, 0x4000).unwrap();
        assert_eq!(ad.open_space_count(), 1);
        assert!(!ad.is_space_open(1000));
        assert!(ad.is_space_open(2000));

        // 6. Close context (drops remaining open spaces).
        let req_close = ad.close_context(0x5000).unwrap();
        assert_eq!(ad.state(), AdAsyncContextState::Closed);
        assert_eq!(ad.open_space_count(), 0);

        // 7. Drain the completion queue — 7 callbacks, each with monotonic
        //    request id in the order they were issued.
        let cbs = ad.drain_callbacks();
        assert_eq!(cbs.len(), 7);
        let ids: alloc::vec::Vec<u32> = cbs.iter().map(|c| c.request_id).collect();
        assert_eq!(
            ids,
            alloc::vec![
                req_open, req_conn, req_s1, req_s2, req_flush, req_sc, req_close
            ]
        );
        assert_eq!(cbs[0].kind, AdAsyncCallbackKind::OpenContext);
        assert_eq!(cbs[1].kind, AdAsyncCallbackKind::ConnectContext);
        assert_eq!(cbs[2].kind, AdAsyncCallbackKind::SpaceOpen);
        assert_eq!(cbs[3].kind, AdAsyncCallbackKind::SpaceOpen);
        assert_eq!(cbs[4].kind, AdAsyncCallbackKind::FlushReports);
        assert_eq!(cbs[5].kind, AdAsyncCallbackKind::SpaceClose);
        assert_eq!(cbs[6].kind, AdAsyncCallbackKind::CloseContext);
        // Every callback result is CELL_OK — mirrors C++ stub return value.
        assert!(cbs.iter().all(|c| c.result == CellError::OK));

        // 8. Post-close, every surface rejects cleanly.
        assert_eq!(ad.open_context(0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.connect_context(0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.space_open(3000, 0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.space_close(3000, 0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.flush_reports(0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
        assert_eq!(ad.close_context(0), Err(SCE_AD_ASYNC_ERROR_CONTEXT_CLOSED));
    }
}
