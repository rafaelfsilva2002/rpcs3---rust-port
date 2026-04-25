//! Rust port of `rpcs3/Emu/Cell/Modules/sceNpUtil.cpp` — PS3 NP utility
//! (bandwidth test) HLE.
//!
//! 4 entries: `Init/Start`, `GetStatus`, `Shutdown`, `Abort`. Upstream stub
//! spawns a fake worker thread that sleeps 100ms and writes
//! `upload_bps = download_bps = 100_000_000.0` (100 Mbps) to the result
//! struct on shutdown — preserved byte-exact.
//!
//! No actual `cellHttp` traffic is generated (cpp:32 TODO). The thread model
//! is collapsed in this port to a synchronous state machine — tests drive it
//! by calling `tick()` to advance the fake worker progress.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use core::mem::size_of;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNpUtil";

/// 4 FNIDs in exact REG_FUNC order (cpp:148-151).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpUtilBandwidthTestInitStart",
    "sceNpUtilBandwidthTestShutdown",
    "sceNpUtilBandwidthTestGetStatus",
    "sceNpUtilBandwidthTestAbort",
];

// ---------------------------------------------------------------------------
// Errors — re-exported from sceNp.h facility 0x8002_AA__.
// ---------------------------------------------------------------------------

pub const SCE_NP_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_AA01);
pub const SCE_NP_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_AA02);

// ---------------------------------------------------------------------------
// Bandwidth test status (header byte-exact).
// ---------------------------------------------------------------------------

pub const SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_NONE: u32 = 0;
pub const SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_RUNNING: u32 = 1;
pub const SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_FINISHED: u32 = 2;

/// Hard-coded fake bandwidth result from cpp:42-43.
pub const FAKE_UPLOAD_BPS: f64 = 100_000_000.0;
pub const FAKE_DOWNLOAD_BPS: f64 = 100_000_000.0;
/// cpp:36 — fake test ends after 100 ticks of 1ms each.
pub const FAKE_TEST_TICKS: u32 = 100;

// ---------------------------------------------------------------------------
// Wire struct.
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct SceNpUtilBandwidthTestResult {
    pub upload_bps: f64,
    pub download_bps: f64,
    pub result: i32,
    pub _padding: [u8; 4],
}
const _: () = assert!(size_of::<SceNpUtilBandwidthTestResult>() == 24);

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct BandwidthTest {
    pub status: u32,
    pub abort_requested: bool,
    pub shutdown_requested: bool,
    pub finished: bool,
    pub fake_sleep_count: u32,
    pub test_result: SceNpUtilBandwidthTestResult,
}

impl Default for BandwidthTest {
    fn default() -> Self {
        Self {
            status: SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_RUNNING,
            abort_requested: false,
            shutdown_requested: false,
            finished: false,
            fake_sleep_count: 0,
            test_result: SceNpUtilBandwidthTestResult::default(),
        }
    }
}

impl BandwidthTest {
    /// One tick of the fake worker (cpp:25-40 loop body). Advances state by
    /// one 1ms iteration and finalizes when count exceeds 100 OR when abort/
    /// shutdown is requested.
    pub fn tick(&mut self) {
        if self.finished {
            return;
        }
        if self.abort_requested || self.shutdown_requested {
            self.finalize();
            return;
        }
        self.fake_sleep_count = self.fake_sleep_count.saturating_add(1);
        if self.fake_sleep_count > FAKE_TEST_TICKS {
            self.finalize();
        }
    }

    /// Run the fake worker to completion (drains all 100 ticks).
    pub fn run_to_completion(&mut self) {
        while !self.finished {
            self.tick();
        }
    }

    fn finalize(&mut self) {
        // Matches cpp:42-46 — write fake result + transition to FINISHED.
        self.test_result.upload_bps = FAKE_UPLOAD_BPS;
        self.test_result.download_bps = FAKE_DOWNLOAD_BPS;
        self.test_result.result = 0; // CELL_OK
        self.status = SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_FINISHED;
        self.finished = true;
    }
}

#[derive(Debug, Default)]
pub struct SceNpUtil {
    /// `bandwidth_test_thread` — None means uninitialized.
    pub test: Option<BandwidthTest>,
    pub last_init_prio: u32,
    pub last_init_stack: u32,

    pub init_start_calls: u64,
    pub get_status_calls: u64,
    pub shutdown_calls: u64,
    pub abort_calls: u64,
}

impl SceNpUtil {
    pub fn new() -> Self {
        Self::default()
    }

    /// `sceNpUtilBandwidthTestInitStart(prio, stack)` — cpp:66-81.
    pub fn bandwidth_test_init_start(
        &mut self,
        prio: u32,
        stack: u32,
    ) -> Result<(), CellError> {
        self.init_start_calls = self.init_start_calls.saturating_add(1);
        if self.test.is_some() {
            return Err(SCE_NP_ERROR_ALREADY_INITIALIZED);
        }
        self.last_init_prio = prio;
        self.last_init_stack = stack;
        self.test = Some(BandwidthTest::default());
        Ok(())
    }

    /// `sceNpUtilBandwidthTestGetStatus()` — cpp:83-96. Returns `Ok(status)`
    /// (`not_an_error(status)` em C++).
    pub fn bandwidth_test_get_status(&mut self) -> Result<u32, CellError> {
        self.get_status_calls = self.get_status_calls.saturating_add(1);
        match &self.test {
            Some(t) => Ok(t.status),
            None => Err(SCE_NP_ERROR_NOT_INITIALIZED),
        }
    }

    /// `sceNpUtilBandwidthTestShutdown(result)` — cpp:98-125. Sets shutdown
    /// flag, runs worker to completion, copies result if `result_out` is
    /// `Some`, then drops the thread.
    pub fn bandwidth_test_shutdown(
        &mut self,
        result_out: Option<&mut SceNpUtilBandwidthTestResult>,
    ) -> Result<(), CellError> {
        self.shutdown_calls = self.shutdown_calls.saturating_add(1);
        let test = self
            .test
            .as_mut()
            .ok_or(SCE_NP_ERROR_NOT_INITIALIZED)?;
        test.shutdown_requested = true;
        // cpp:112-115 spin loop on `finished` — collapsed to run_to_completion.
        test.run_to_completion();
        if let Some(slot) = result_out {
            *slot = test.test_result;
        }
        // cpp:122 join_thread() drops the unique_ptr.
        self.test = None;
        Ok(())
    }

    /// `sceNpUtilBandwidthTestAbort()` — cpp:127-144. Sets abort flag (does
    /// not finalize on its own — caller must call shutdown).
    pub fn bandwidth_test_abort(&mut self) -> Result<(), CellError> {
        self.abort_calls = self.abort_calls.saturating_add(1);
        let test = self
            .test
            .as_mut()
            .ok_or(SCE_NP_ERROR_NOT_INITIALIZED)?;
        test.abort_requested = true;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sceNpUtil");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 4);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "sceNpUtilBandwidthTestInitStart");
        assert_eq!(REGISTERED_ENTRY_POINTS[3], "sceNpUtilBandwidthTestAbort");
    }

    #[test]
    fn status_constants_byte_exact() {
        assert_eq!(SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_NONE, 0);
        assert_eq!(SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_RUNNING, 1);
        assert_eq!(SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_FINISHED, 2);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(SCE_NP_ERROR_NOT_INITIALIZED.0, 0x8002_AA01);
        assert_eq!(SCE_NP_ERROR_ALREADY_INITIALIZED.0, 0x8002_AA02);
    }

    #[test]
    fn fake_bandwidth_constants_match_cpp() {
        assert_eq!(FAKE_UPLOAD_BPS, 100_000_000.0);
        assert_eq!(FAKE_DOWNLOAD_BPS, 100_000_000.0);
        assert_eq!(FAKE_TEST_TICKS, 100);
    }

    #[test]
    fn result_struct_size() {
        // 8+8+4+4 = 24 bytes.
        assert_eq!(size_of::<SceNpUtilBandwidthTestResult>(), 24);
    }

    #[test]
    fn get_status_without_init_fails() {
        let mut m = SceNpUtil::new();
        assert_eq!(
            m.bandwidth_test_get_status(),
            Err(SCE_NP_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn shutdown_without_init_fails() {
        let mut m = SceNpUtil::new();
        assert_eq!(
            m.bandwidth_test_shutdown(None),
            Err(SCE_NP_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn abort_without_init_fails() {
        let mut m = SceNpUtil::new();
        assert_eq!(
            m.bandwidth_test_abort(),
            Err(SCE_NP_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn init_start_double_returns_already_initialized() {
        let mut m = SceNpUtil::new();
        m.bandwidth_test_init_start(512, 0x4000).unwrap();
        assert_eq!(
            m.bandwidth_test_init_start(512, 0x4000),
            Err(SCE_NP_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn init_start_captures_prio_and_stack() {
        let mut m = SceNpUtil::new();
        m.bandwidth_test_init_start(0xCAFE, 0xBEEF).unwrap();
        assert_eq!(m.last_init_prio, 0xCAFE);
        assert_eq!(m.last_init_stack, 0xBEEF);
    }

    #[test]
    fn fresh_test_starts_running() {
        let mut m = SceNpUtil::new();
        m.bandwidth_test_init_start(0, 0).unwrap();
        assert_eq!(
            m.bandwidth_test_get_status().unwrap(),
            SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_RUNNING
        );
    }

    #[test]
    fn tick_advances_count_until_100() {
        let mut t = BandwidthTest::default();
        for i in 1..=100 {
            t.tick();
            assert_eq!(t.fake_sleep_count, i);
            assert!(!t.finished);
        }
        // 101st tick triggers finalize (count > 100).
        t.tick();
        assert!(t.finished);
        assert_eq!(t.status, SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_FINISHED);
    }

    #[test]
    fn run_to_completion_writes_fake_bandwidth() {
        let mut t = BandwidthTest::default();
        t.run_to_completion();
        assert!(t.finished);
        assert_eq!(t.test_result.upload_bps, FAKE_UPLOAD_BPS);
        assert_eq!(t.test_result.download_bps, FAKE_DOWNLOAD_BPS);
        assert_eq!(t.test_result.result, 0);
        assert_eq!(t.status, SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_FINISHED);
    }

    #[test]
    fn abort_short_circuits_test() {
        let mut t = BandwidthTest::default();
        t.tick(); // count=1
        t.abort_requested = true;
        t.tick(); // finalizes immediately
        assert!(t.finished);
        // Result still populated even on abort (cpp:42 unconditional).
        assert_eq!(t.test_result.upload_bps, FAKE_UPLOAD_BPS);
    }

    #[test]
    fn shutdown_short_circuits_test() {
        let mut t = BandwidthTest::default();
        t.shutdown_requested = true;
        t.tick();
        assert!(t.finished);
    }

    #[test]
    fn shutdown_writes_result_when_provided() {
        let mut m = SceNpUtil::new();
        m.bandwidth_test_init_start(0, 0).unwrap();
        let mut result = SceNpUtilBandwidthTestResult::default();
        m.bandwidth_test_shutdown(Some(&mut result)).unwrap();
        assert_eq!(result.upload_bps, FAKE_UPLOAD_BPS);
        assert_eq!(result.download_bps, FAKE_DOWNLOAD_BPS);
        assert_eq!(result.result, 0);
        // Test thread joined.
        assert!(m.test.is_none());
        // Subsequent get_status fails.
        assert_eq!(
            m.bandwidth_test_get_status(),
            Err(SCE_NP_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn shutdown_with_null_result_still_joins() {
        let mut m = SceNpUtil::new();
        m.bandwidth_test_init_start(0, 0).unwrap();
        m.bandwidth_test_shutdown(None).unwrap();
        assert!(m.test.is_none());
    }

    #[test]
    fn re_init_after_shutdown_works() {
        let mut m = SceNpUtil::new();
        m.bandwidth_test_init_start(100, 0x1000).unwrap();
        m.bandwidth_test_shutdown(None).unwrap();
        // Can re-init.
        m.bandwidth_test_init_start(200, 0x2000).unwrap();
        assert_eq!(m.last_init_prio, 200);
        assert_eq!(m.last_init_stack, 0x2000);
    }

    #[test]
    fn abort_then_shutdown_flow() {
        let mut m = SceNpUtil::new();
        m.bandwidth_test_init_start(0, 0).unwrap();
        m.bandwidth_test_abort().unwrap();
        // Status check after abort but before shutdown — still Running.
        // (abort sets a flag; finalize only happens on next tick or shutdown.)
        assert_eq!(
            m.bandwidth_test_get_status().unwrap(),
            SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_RUNNING
        );
        // Shutdown finalizes + joins.
        let mut result = SceNpUtilBandwidthTestResult::default();
        m.bandwidth_test_shutdown(Some(&mut result)).unwrap();
        assert_eq!(result.upload_bps, FAKE_UPLOAD_BPS);
    }

    #[test]
    fn counters_track_invocations() {
        let mut m = SceNpUtil::new();
        let _ = m.bandwidth_test_get_status(); // 1
        m.bandwidth_test_init_start(0, 0).unwrap(); // 1
        let _ = m.bandwidth_test_get_status(); // 2
        m.bandwidth_test_abort().unwrap(); // 1
        m.bandwidth_test_shutdown(None).unwrap(); // 1
        assert_eq!(m.init_start_calls, 1);
        assert_eq!(m.get_status_calls, 2);
        assert_eq!(m.abort_calls, 1);
        assert_eq!(m.shutdown_calls, 1);
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut m = SceNpUtil::new();
        // Pre-init: all ops fail
        assert!(m.bandwidth_test_get_status().is_err());
        assert!(m.bandwidth_test_shutdown(None).is_err());
        assert!(m.bandwidth_test_abort().is_err());

        // Init
        m.bandwidth_test_init_start(0x100, 0x4000).unwrap();
        assert_eq!(
            m.bandwidth_test_get_status().unwrap(),
            SCE_NP_UTIL_BANDWIDTH_TEST_STATUS_RUNNING
        );

        // Abort midway
        m.bandwidth_test_abort().unwrap();

        // Shutdown captures result
        let mut result = SceNpUtilBandwidthTestResult::default();
        m.bandwidth_test_shutdown(Some(&mut result)).unwrap();
        assert_eq!(result.upload_bps, 100_000_000.0);
        assert_eq!(result.download_bps, 100_000_000.0);
        assert_eq!(result.result, 0);

        // Post-shutdown: re-init OK
        m.bandwidth_test_init_start(0x200, 0x8000).unwrap();
        m.bandwidth_test_shutdown(None).unwrap();
    }
}
