//! `rpcs3-hle-cellprint` — PS3 printer utility HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellPrint.cpp` (165 linhas).  The
//! module implements printer support via `cellPrintUtility`.  The RPCS3
//! port is all `UNIMPLEMENTED_FUNC` stubs but honours the async
//! callback pattern: each `cellPrint*Async*` registers a sysutil
//! callback that fires on the PPU.  The Rust port captures the
//! lifecycle FSM + job/page state + accumulated byte counters so
//! higher layers can exercise the full print pipeline.
//!
//! ## Entry points covered
//!
//! | C++ function                  | Rust wrapper                      |
//! |-------------------------------|-----------------------------------|
//! | `cellSysutilPrintInit/Shutdown` | [`Print::init`] / [`Print::shutdown`] |
//! | `cellPrintLoadAsync` / `LoadAsync2` | [`Print::load_async`] / [`Print::load_async2`] |
//! | `cellPrintUnloadAsync`        | [`Print::unload_async`]           |
//! | `cellPrintGetStatus`          | [`Print::get_status`]             |
//! | `cellPrintOpenConfig`         | [`Print::open_config`]            |
//! | `cellPrintGetPrintableArea`   | [`Print::get_printable_area`]     |
//! | `cellPrintStartJob` / `EndJob` / `CancelJob` | [`Print::start_job`] / [`Print::end_job`] / [`Print::cancel_job`] |
//! | `cellPrintStartPage` / `EndPage` | [`Print::start_page`] / [`Print::end_page`] |
//! | `cellPrintSendBand`           | [`Print::send_band`]              |

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellPrint.cpp:8-18
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const INTERNAL:            CellError = CellError(0x8002_C401);
    pub const NO_MEMORY:           CellError = CellError(0x8002_C402);
    pub const PRINTER_NOT_FOUND:   CellError = CellError(0x8002_C403);
    pub const INVALID_PARAM:       CellError = CellError(0x8002_C404);
    pub const INVALID_FUNCTION:    CellError = CellError(0x8002_C405);
    pub const NOT_SUPPORT:         CellError = CellError(0x8002_C406);
    pub const OCCURRED:            CellError = CellError(0x8002_C407);
    pub const CANCELED_BY_PRINTER: CellError = CellError(0x8002_C408);
}

// =====================================================================
// Constants
// =====================================================================

/// Default printable-area width (pixels).  PS3 firmware typically
/// returns a 5,100×6,600 (8.5"×11" at 600 DPI) area; we pick the
/// conservative value that real printers advertise.
pub const DEFAULT_PRINTABLE_WIDTH:  i32 = 5100;
pub const DEFAULT_PRINTABLE_HEIGHT: i32 = 6600;

/// Registered color formats.  These match the PS3 header
/// (`CELL_PRINT_COLORFMT_*`) — 0 = 24-bit RGB, 1 = grayscale, 2 = RGBA.
pub const CELL_PRINT_COLORFMT_RGB:       i32 = 0;
pub const CELL_PRINT_COLORFMT_GRAYSCALE: i32 = 1;
pub const CELL_PRINT_COLORFMT_RGBA:      i32 = 2;

// =====================================================================
// Status / lifecycle
// =====================================================================

/// Observable state of the printer utility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintState {
    Uninitialized,
    /// After `cellSysutilPrintInit`.
    Idle,
    /// After `cellPrintLoadAsync*`.
    Loaded,
    /// After `cellPrintStartJob`.
    JobActive,
    /// After `cellPrintStartPage`.
    PageActive,
    /// After `cellPrintCancelJob` — waits for `EndJob`.
    Cancelled,
}

impl Default for PrintState { fn default() -> Self { Self::Uninitialized } }

/// Mirror of `CellPrintStatus` (cellPrint.cpp:26-32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PrintStatus {
    pub status: i32,
    pub error_status: i32,
    pub continue_enabled: i32,
}

/// Mirror of `CellPrintLoadParam`.  `mode` is opaque — stored as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoadParam {
    pub mode: u32,
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Clone, Default)]
pub struct Print {
    pub state: PrintState,
    pub load_param: Option<LoadParam>,
    pub current_job_total_pages: i32,
    pub current_color_format: i32,
    pub pages_started: u32,
    pub pages_finished: u32,
    pub bands_sent: u32,
    pub total_bytes_sent: u64,
    pub async_callbacks_pending: Vec<AsyncCallback>,
}

/// Queued async callback — sysutil would invoke each entry in FIFO
/// order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsyncCallback {
    pub function: u32,
    pub userdata: u32,
    pub result: i32,
    pub kind: CallbackKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackKind {
    LoadAsync,
    LoadAsync2,
    UnloadAsync,
    OpenConfig,
}

impl Print {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    // ---- Init / Shutdown -------------------------------------------

    /// Port of `cellSysutilPrintInit`.
    pub fn init(&mut self) -> Result<(), CellError> {
        if self.state != PrintState::Uninitialized {
            return Err(errors::INVALID_FUNCTION);
        }
        self.state = PrintState::Idle;
        Ok(())
    }

    /// Port of `cellSysutilPrintShutdown`.
    pub fn shutdown(&mut self) -> Result<(), CellError> {
        if self.state == PrintState::Uninitialized {
            return Err(errors::INVALID_FUNCTION);
        }
        *self = Self::default();
        Ok(())
    }

    // ---- Load / Unload ----------------------------------------------

    /// Port of `cellPrintLoadAsync`.  Enqueues the callback and
    /// transitions Idle → Loaded.  `container` is captured for
    /// debugging but ignored.
    pub fn load_async(
        &mut self,
        function: u32,
        userdata: u32,
        param: LoadParam,
        _container: u32,
    ) -> Result<(), CellError> {
        if self.state != PrintState::Idle { return Err(errors::INVALID_FUNCTION); }
        self.load_param = Some(param);
        self.state = PrintState::Loaded;
        self.async_callbacks_pending.push(AsyncCallback {
            function, userdata, result: 0, kind: CallbackKind::LoadAsync,
        });
        Ok(())
    }

    /// Port of `cellPrintLoadAsync2`.  Same as `LoadAsync` but with a
    /// distinct callback `kind` for tests.
    pub fn load_async2(
        &mut self,
        function: u32,
        userdata: u32,
        param: LoadParam,
    ) -> Result<(), CellError> {
        if self.state != PrintState::Idle { return Err(errors::INVALID_FUNCTION); }
        self.load_param = Some(param);
        self.state = PrintState::Loaded;
        self.async_callbacks_pending.push(AsyncCallback {
            function, userdata, result: 0, kind: CallbackKind::LoadAsync2,
        });
        Ok(())
    }

    /// Port of `cellPrintUnloadAsync`.
    pub fn unload_async(&mut self, function: u32, userdata: u32) -> Result<(), CellError> {
        if self.state != PrintState::Loaded { return Err(errors::INVALID_FUNCTION); }
        self.state = PrintState::Idle;
        self.load_param = None;
        self.async_callbacks_pending.push(AsyncCallback {
            function, userdata, result: 0, kind: CallbackKind::UnloadAsync,
        });
        Ok(())
    }

    /// Port of `cellPrintOpenConfig`.
    pub fn open_config(&mut self, function: u32, userdata: u32) -> Result<(), CellError> {
        if self.state != PrintState::Loaded { return Err(errors::INVALID_FUNCTION); }
        self.async_callbacks_pending.push(AsyncCallback {
            function, userdata, result: 0, kind: CallbackKind::OpenConfig,
        });
        Ok(())
    }

    /// Drain the pending-callback queue.  Returns the callbacks in
    /// FIFO order — callers iterate and invoke each one.
    #[must_use]
    pub fn drain_callbacks(&mut self) -> Vec<AsyncCallback> {
        core::mem::take(&mut self.async_callbacks_pending)
    }

    // ---- Query ------------------------------------------------------

    /// Port of `cellPrintGetStatus`.  C++ stub returns CELL_OK without
    /// writing; the Rust port surfaces the current state as a
    /// [`PrintStatus`] so callers can verify the lifecycle.
    #[must_use]
    pub fn get_status(&self) -> PrintStatus {
        PrintStatus {
            status: match self.state {
                PrintState::Uninitialized => -1,
                PrintState::Idle => 0,
                PrintState::Loaded => 1,
                PrintState::JobActive => 2,
                PrintState::PageActive => 3,
                PrintState::Cancelled => 4,
            },
            error_status: 0,
            continue_enabled: if self.state == PrintState::Cancelled { 0 } else { 1 },
        }
    }

    /// Port of `cellPrintGetPrintableArea`.  Returns the default
    /// dimensions — real printers fill in their actual area.
    #[must_use]
    pub fn get_printable_area(&self) -> (i32, i32) {
        (DEFAULT_PRINTABLE_WIDTH, DEFAULT_PRINTABLE_HEIGHT)
    }

    // ---- Job lifecycle ---------------------------------------------

    /// Port of `cellPrintStartJob`.
    ///
    /// # Errors
    /// * [`errors::INVALID_PARAM`] if `total_page <= 0` or the color
    ///   format is unknown.
    /// * [`errors::INVALID_FUNCTION`] if not in `Loaded` state.
    pub fn start_job(&mut self, total_page: i32, color_format: i32) -> Result<(), CellError> {
        if self.state != PrintState::Loaded { return Err(errors::INVALID_FUNCTION); }
        if total_page <= 0 { return Err(errors::INVALID_PARAM); }
        if !matches!(color_format, 0 | 1 | 2) {
            return Err(errors::INVALID_PARAM);
        }
        self.current_job_total_pages = total_page;
        self.current_color_format = color_format;
        self.pages_started = 0;
        self.pages_finished = 0;
        self.bands_sent = 0;
        self.total_bytes_sent = 0;
        self.state = PrintState::JobActive;
        Ok(())
    }

    /// Port of `cellPrintEndJob`.  Accepts both `JobActive` and
    /// `Cancelled`.
    pub fn end_job(&mut self) -> Result<(), CellError> {
        if !matches!(self.state, PrintState::JobActive | PrintState::Cancelled) {
            return Err(errors::INVALID_FUNCTION);
        }
        self.state = PrintState::Loaded;
        Ok(())
    }

    /// Port of `cellPrintCancelJob`.
    pub fn cancel_job(&mut self) -> Result<(), CellError> {
        if !matches!(self.state, PrintState::JobActive | PrintState::PageActive) {
            return Err(errors::INVALID_FUNCTION);
        }
        self.state = PrintState::Cancelled;
        Ok(())
    }

    /// Port of `cellPrintStartPage`.
    pub fn start_page(&mut self) -> Result<(), CellError> {
        if self.state != PrintState::JobActive { return Err(errors::INVALID_FUNCTION); }
        if self.pages_started >= self.current_job_total_pages as u32 {
            return Err(errors::INVALID_FUNCTION);
        }
        self.pages_started += 1;
        self.state = PrintState::PageActive;
        Ok(())
    }

    /// Port of `cellPrintEndPage`.
    pub fn end_page(&mut self) -> Result<(), CellError> {
        if self.state != PrintState::PageActive { return Err(errors::INVALID_FUNCTION); }
        self.pages_finished += 1;
        self.state = PrintState::JobActive;
        Ok(())
    }

    /// Port of `cellPrintSendBand`.  Accumulates the band's bytes and
    /// returns the total consumed.
    pub fn send_band(&mut self, buff_size: i32) -> Result<i32, CellError> {
        if self.state != PrintState::PageActive { return Err(errors::INVALID_FUNCTION); }
        if buff_size < 0 { return Err(errors::INVALID_PARAM); }
        self.bands_sent = self.bands_sent.saturating_add(1);
        self.total_bytes_sent = self.total_bytes_sent.saturating_add(buff_size as u64);
        Ok(buff_size)
    }
}

// =====================================================================
// Registry — 14 REG_FUNCs under `cellPrintUtility`
// =====================================================================

pub const MODULE_NAME: &str = "cellPrintUtility";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellSysutilPrintInit",
    "cellSysutilPrintShutdown",
    "cellPrintLoadAsync",
    "cellPrintLoadAsync2",
    "cellPrintUnloadAsync",
    "cellPrintGetStatus",
    "cellPrintOpenConfig",
    "cellPrintGetPrintableArea",
    "cellPrintStartJob",
    "cellPrintEndJob",
    "cellPrintCancelJob",
    "cellPrintStartPage",
    "cellPrintEndPage",
    "cellPrintSendBand",
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
        assert_eq!(errors::INTERNAL.0,            0x8002_C401);
        assert_eq!(errors::NO_MEMORY.0,           0x8002_C402);
        assert_eq!(errors::PRINTER_NOT_FOUND.0,   0x8002_C403);
        assert_eq!(errors::INVALID_PARAM.0,       0x8002_C404);
        assert_eq!(errors::INVALID_FUNCTION.0,    0x8002_C405);
        assert_eq!(errors::NOT_SUPPORT.0,         0x8002_C406);
        assert_eq!(errors::OCCURRED.0,            0x8002_C407);
        assert_eq!(errors::CANCELED_BY_PRINTER.0, 0x8002_C408);
    }

    #[test]
    fn error_codes_contiguous() {
        assert_eq!(errors::CANCELED_BY_PRINTER.0 - errors::INTERNAL.0, 7);
    }

    #[test]
    fn color_format_constants() {
        assert_eq!(CELL_PRINT_COLORFMT_RGB, 0);
        assert_eq!(CELL_PRINT_COLORFMT_GRAYSCALE, 1);
        assert_eq!(CELL_PRINT_COLORFMT_RGBA, 2);
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellPrintUtility");
    }

    #[test]
    fn registry_has_14_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 14);
    }

    #[test]
    fn registry_first_and_last_match_cpp() {
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellSysutilPrintInit");
        assert_eq!(REGISTERED_ENTRY_POINTS[13], "cellPrintSendBand");
    }

    // ---- Init / Shutdown --------------------------------------------

    #[test]
    fn init_transitions_to_idle() {
        let mut p = Print::new();
        p.init().unwrap();
        assert_eq!(p.state, PrintState::Idle);
    }

    #[test]
    fn init_twice_is_invalid_function() {
        let mut p = Print::new();
        p.init().unwrap();
        assert_eq!(p.init().unwrap_err(), errors::INVALID_FUNCTION);
    }

    #[test]
    fn shutdown_without_init_is_invalid_function() {
        let mut p = Print::new();
        assert_eq!(p.shutdown().unwrap_err(), errors::INVALID_FUNCTION);
    }

    #[test]
    fn shutdown_resets_everything() {
        let mut p = Print::new();
        p.init().unwrap();
        p.shutdown().unwrap();
        assert_eq!(p.state, PrintState::Uninitialized);
    }

    // ---- Load / Unload ----------------------------------------------

    #[test]
    fn load_async_requires_idle() {
        let mut p = Print::new();
        assert_eq!(
            p.load_async(0x1000, 0x2000, LoadParam::default(), 0).unwrap_err(),
            errors::INVALID_FUNCTION,
        );
    }

    #[test]
    fn load_async_transitions_to_loaded() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0x1000, 0x2000, LoadParam { mode: 1 }, 0).unwrap();
        assert_eq!(p.state, PrintState::Loaded);
        assert_eq!(p.load_param, Some(LoadParam { mode: 1 }));
    }

    #[test]
    fn load_async_queues_callback() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0xABCD, 0x1234, LoadParam::default(), 0).unwrap();
        let cbs = p.drain_callbacks();
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].kind, CallbackKind::LoadAsync);
        assert_eq!(cbs[0].function, 0xABCD);
        assert_eq!(cbs[0].userdata, 0x1234);
    }

    #[test]
    fn load_async2_distinct_kind() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async2(0xABCD, 0x1234, LoadParam::default()).unwrap();
        let cbs = p.drain_callbacks();
        assert_eq!(cbs[0].kind, CallbackKind::LoadAsync2);
    }

    #[test]
    fn unload_async_transitions_to_idle() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0x1000, 0x2000, LoadParam::default(), 0).unwrap();
        p.unload_async(0x3000, 0x4000).unwrap();
        assert_eq!(p.state, PrintState::Idle);
        assert_eq!(p.load_param, None);
    }

    #[test]
    fn unload_async_requires_loaded() {
        let mut p = Print::new();
        assert_eq!(
            p.unload_async(0, 0).unwrap_err(),
            errors::INVALID_FUNCTION,
        );
    }

    #[test]
    fn open_config_queues_callback() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.open_config(0x5000, 0x6000).unwrap();
        let cbs = p.drain_callbacks();
        assert!(cbs.iter().any(|c| c.kind == CallbackKind::OpenConfig));
    }

    // ---- Job / Page -------------------------------------------------

    #[test]
    fn start_job_rejects_zero_pages() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        assert_eq!(p.start_job(0, 0).unwrap_err(), errors::INVALID_PARAM);
    }

    #[test]
    fn start_job_rejects_unknown_color_format() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        assert_eq!(p.start_job(1, 99).unwrap_err(), errors::INVALID_PARAM);
    }

    #[test]
    fn start_job_transitions_to_job_active() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(5, CELL_PRINT_COLORFMT_RGB).unwrap();
        assert_eq!(p.state, PrintState::JobActive);
        assert_eq!(p.current_job_total_pages, 5);
    }

    #[test]
    fn start_page_increments_pages_started() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(3, 0).unwrap();
        p.start_page().unwrap();
        assert_eq!(p.pages_started, 1);
        assert_eq!(p.state, PrintState::PageActive);
    }

    #[test]
    fn start_page_rejects_after_total_pages() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(1, 0).unwrap();
        p.start_page().unwrap();
        p.end_page().unwrap();
        assert_eq!(p.start_page().unwrap_err(), errors::INVALID_FUNCTION);
    }

    #[test]
    fn end_page_increments_finished_and_returns_to_job() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(3, 0).unwrap();
        p.start_page().unwrap();
        p.end_page().unwrap();
        assert_eq!(p.pages_finished, 1);
        assert_eq!(p.state, PrintState::JobActive);
    }

    #[test]
    fn send_band_requires_page_active() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(1, 0).unwrap();
        assert_eq!(p.send_band(100).unwrap_err(), errors::INVALID_FUNCTION);
    }

    #[test]
    fn send_band_accumulates_bytes() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(1, 0).unwrap();
        p.start_page().unwrap();
        p.send_band(1024).unwrap();
        p.send_band(2048).unwrap();
        assert_eq!(p.bands_sent, 2);
        assert_eq!(p.total_bytes_sent, 1024 + 2048);
    }

    #[test]
    fn send_band_negative_size_is_invalid_param() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(1, 0).unwrap();
        p.start_page().unwrap();
        assert_eq!(p.send_band(-1).unwrap_err(), errors::INVALID_PARAM);
    }

    #[test]
    fn cancel_job_from_job_active() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(3, 0).unwrap();
        p.cancel_job().unwrap();
        assert_eq!(p.state, PrintState::Cancelled);
    }

    #[test]
    fn cancel_job_from_page_active() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(3, 0).unwrap();
        p.start_page().unwrap();
        p.cancel_job().unwrap();
        assert_eq!(p.state, PrintState::Cancelled);
    }

    #[test]
    fn end_job_accepts_cancelled() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(3, 0).unwrap();
        p.cancel_job().unwrap();
        p.end_job().unwrap();
        assert_eq!(p.state, PrintState::Loaded);
    }

    #[test]
    fn get_status_reflects_state() {
        let mut p = Print::new();
        assert_eq!(p.get_status().status, -1);
        p.init().unwrap();
        assert_eq!(p.get_status().status, 0);
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        assert_eq!(p.get_status().status, 1);
    }

    #[test]
    fn get_status_cancelled_sets_continue_disabled() {
        let mut p = Print::new();
        p.init().unwrap();
        p.load_async(0, 0, LoadParam::default(), 0).unwrap();
        p.start_job(1, 0).unwrap();
        p.cancel_job().unwrap();
        assert_eq!(p.get_status().continue_enabled, 0);
    }

    #[test]
    fn get_printable_area_returns_defaults() {
        let p = Print::new();
        assert_eq!(
            p.get_printable_area(),
            (DEFAULT_PRINTABLE_WIDTH, DEFAULT_PRINTABLE_HEIGHT),
        );
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_cellprint_lifecycle_smoke() {
        let mut p = Print::new();

        // 1. Init + Load.
        p.init().unwrap();
        p.load_async(0xCB, 0xCD, LoadParam { mode: 1 }, 0).unwrap();
        let cbs = p.drain_callbacks();
        assert_eq!(cbs.len(), 1);

        // 2. Open config + drain callback.
        p.open_config(0xA, 0xB).unwrap();
        assert_eq!(p.drain_callbacks().len(), 1);

        // 3. Start a 2-page RGB job.
        p.start_job(2, CELL_PRINT_COLORFMT_RGB).unwrap();

        // 4. Page 1: 3 bands.
        p.start_page().unwrap();
        p.send_band(1024).unwrap();
        p.send_band(2048).unwrap();
        p.send_band(4096).unwrap();
        p.end_page().unwrap();
        assert_eq!(p.bands_sent, 3);
        assert_eq!(p.total_bytes_sent, 7168);

        // 5. Page 2: 1 band.
        p.start_page().unwrap();
        p.send_band(512).unwrap();
        p.end_page().unwrap();

        // 6. End job.
        p.end_job().unwrap();
        assert_eq!(p.state, PrintState::Loaded);
        assert_eq!(p.pages_finished, 2);

        // 7. Unload + shutdown.
        p.unload_async(0, 0).unwrap();
        p.shutdown().unwrap();
        assert_eq!(p.state, PrintState::Uninitialized);
    }
}
