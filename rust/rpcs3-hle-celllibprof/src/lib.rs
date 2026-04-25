//! Rust port of `rpcs3/Emu/Cell/Modules/cellLibprof.cpp`.
//!
//! 4 PRX entries under the module name `cellLibprof` — the user-trace
//! profiler: games call it to register named probe points that external
//! tooling (SN Tuner / DECI traces) can latch on to. The C++ bodies are
//! `UNIMPLEMENTED_FUNC` stubs that return `CELL_OK`; the real firmware
//! wires these into a kernel-side ring buffer.
//!
//! REG_FUNC order at cpp:32-35:
//!
//!  1. `cellUserTraceInit`
//!  2. `cellUserTraceRegister`
//!  3. `cellUserTraceUnregister`
//!  4. `cellUserTraceTerminate`
//!
//! Module name byte-exact at cpp:4 / cpp:30.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:4 / cpp:30.
pub const MODULE_NAME: &str = "cellLibprof";

/// REG_FUNC order at cpp:32-35.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellUserTraceInit",
    "cellUserTraceRegister",
    "cellUserTraceUnregister",
    "cellUserTraceTerminate",
];

// --- Error codes (placeholder facility 0x8002_D4__) ---------------------
//
// The C++ source commits no named error codes; the values below enforce
// the Rust FSM without altering the C++ happy-path `CELL_OK` return.

pub const CELL_LIBPROF_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_D401);
pub const CELL_LIBPROF_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_D402);
pub const CELL_LIBPROF_ERROR_PROBE_NOT_FOUND: CellError = CellError(0x8002_D403);
pub const CELL_LIBPROF_ERROR_PROBE_ALREADY_EXISTS: CellError = CellError(0x8002_D404);
pub const CELL_LIBPROF_ERROR_INVALID_PARAMETER: CellError = CellError(0x8002_D405);
pub const CELL_LIBPROF_ERROR_TERMINATED: CellError = CellError(0x8002_D406);

/// Max concurrent registered probes — a soft cap the port applies so
/// registrations don't grow the vector unbounded in fuzzed tests.
pub const CELL_LIBPROF_MAX_PROBES: usize = 256;

/// Max bytes (including any terminator) in a probe name — matches the
/// common Sony user-trace convention of a short ASCII identifier.
pub const CELL_LIBPROF_MAX_NAME_LEN: usize = 64;

// --- FSM + probe model --------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    Uninitialized,
    Initialized,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceProbe {
    pub probe_id: u32,
    pub name: String,
}

// --- Manager ------------------------------------------------------------

#[derive(Debug, Default)]
pub struct Libprof {
    state: Option<ModuleState>,
    probes: Vec<TraceProbe>,
    init_calls: u32,
    register_calls: u32,
    unregister_calls: u32,
    terminate_calls: u32,
}

impl Libprof {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn state(&self) -> ModuleState {
        self.state.unwrap_or(ModuleState::Uninitialized)
    }

    #[must_use]
    pub fn probe_count(&self) -> usize {
        self.probes.len()
    }

    #[must_use]
    pub fn init_calls(&self) -> u32 {
        self.init_calls
    }
    #[must_use]
    pub fn register_calls(&self) -> u32 {
        self.register_calls
    }
    #[must_use]
    pub fn unregister_calls(&self) -> u32 {
        self.unregister_calls
    }
    #[must_use]
    pub fn terminate_calls(&self) -> u32 {
        self.terminate_calls
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        match self.state() {
            ModuleState::Initialized => Ok(()),
            ModuleState::Terminated => Err(CELL_LIBPROF_ERROR_TERMINATED),
            ModuleState::Uninitialized => Err(CELL_LIBPROF_ERROR_NOT_INITIALIZED),
        }
    }

    /// `cellUserTraceInit` (cpp:6-10).
    pub fn init(&mut self) -> Result<(), CellError> {
        match self.state() {
            ModuleState::Uninitialized => {
                self.state = Some(ModuleState::Initialized);
                self.init_calls = self.init_calls.saturating_add(1);
                Ok(())
            }
            ModuleState::Initialized => Err(CELL_LIBPROF_ERROR_ALREADY_INITIALIZED),
            ModuleState::Terminated => Err(CELL_LIBPROF_ERROR_TERMINATED),
        }
    }

    /// `cellUserTraceRegister` (cpp:12-16). Rejects `probe_id == 0`,
    /// overlong / empty names, and probes past the cap.
    pub fn register(
        &mut self,
        probe_id: u32,
        name: impl Into<String>,
    ) -> Result<(), CellError> {
        self.require_initialized()?;
        if probe_id == 0 {
            return Err(CELL_LIBPROF_ERROR_INVALID_PARAMETER);
        }
        let name = name.into();
        if name.is_empty() || name.len() > CELL_LIBPROF_MAX_NAME_LEN {
            return Err(CELL_LIBPROF_ERROR_INVALID_PARAMETER);
        }
        if self.probes.iter().any(|p| p.probe_id == probe_id) {
            return Err(CELL_LIBPROF_ERROR_PROBE_ALREADY_EXISTS);
        }
        if self.probes.len() >= CELL_LIBPROF_MAX_PROBES {
            return Err(CELL_LIBPROF_ERROR_INVALID_PARAMETER);
        }
        self.probes.push(TraceProbe { probe_id, name });
        self.register_calls = self.register_calls.saturating_add(1);
        Ok(())
    }

    /// `cellUserTraceUnregister` (cpp:18-22).
    pub fn unregister(&mut self, probe_id: u32) -> Result<(), CellError> {
        self.require_initialized()?;
        let pos = self
            .probes
            .iter()
            .position(|p| p.probe_id == probe_id)
            .ok_or(CELL_LIBPROF_ERROR_PROBE_NOT_FOUND)?;
        self.probes.swap_remove(pos);
        self.unregister_calls = self.unregister_calls.saturating_add(1);
        Ok(())
    }

    /// `cellUserTraceTerminate` (cpp:24-28). Drops every live probe and
    /// marks the module terminal (subsequent `init` returns
    /// `TERMINATED`).
    pub fn terminate(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        self.probes.clear();
        self.state = Some(ModuleState::Terminated);
        self.terminate_calls = self.terminate_calls.saturating_add(1);
        Ok(())
    }

    /// Test helper — lookup a probe by id. Returns `None` if missing.
    #[must_use]
    pub fn find_probe(&self, probe_id: u32) -> Option<&TraceProbe> {
        self.probes.iter().find(|p| p.probe_id == probe_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bring_up() -> Libprof {
        let mut l = Libprof::new();
        l.init().unwrap();
        l
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellLibprof");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(
            REGISTERED_ENTRY_POINTS,
            &[
                "cellUserTraceInit",
                "cellUserTraceRegister",
                "cellUserTraceUnregister",
                "cellUserTraceTerminate",
            ]
        );
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_LIBPROF_ERROR_NOT_INITIALIZED.0, 0x8002_D401);
        assert_eq!(CELL_LIBPROF_ERROR_ALREADY_INITIALIZED.0, 0x8002_D402);
        assert_eq!(CELL_LIBPROF_ERROR_PROBE_NOT_FOUND.0, 0x8002_D403);
        assert_eq!(CELL_LIBPROF_ERROR_PROBE_ALREADY_EXISTS.0, 0x8002_D404);
        assert_eq!(CELL_LIBPROF_ERROR_INVALID_PARAMETER.0, 0x8002_D405);
        assert_eq!(CELL_LIBPROF_ERROR_TERMINATED.0, 0x8002_D406);
    }

    #[test]
    fn constants_byte_exact() {
        assert_eq!(CELL_LIBPROF_MAX_PROBES, 256);
        assert_eq!(CELL_LIBPROF_MAX_NAME_LEN, 64);
    }

    #[test]
    fn starts_uninitialized() {
        let l = Libprof::new();
        assert_eq!(l.state(), ModuleState::Uninitialized);
        assert_eq!(l.probe_count(), 0);
    }

    #[test]
    fn init_happy_path() {
        let mut l = Libprof::new();
        l.init().unwrap();
        assert_eq!(l.state(), ModuleState::Initialized);
    }

    #[test]
    fn double_init_is_already_initialized() {
        let mut l = bring_up();
        assert_eq!(l.init(), Err(CELL_LIBPROF_ERROR_ALREADY_INITIALIZED));
    }

    #[test]
    fn init_after_terminate_is_terminated() {
        let mut l = bring_up();
        l.terminate().unwrap();
        assert_eq!(l.init(), Err(CELL_LIBPROF_ERROR_TERMINATED));
    }

    #[test]
    fn register_without_init_is_not_initialized() {
        let mut l = Libprof::new();
        assert_eq!(
            l.register(1, "a"),
            Err(CELL_LIBPROF_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn register_zero_id_is_invalid() {
        let mut l = bring_up();
        assert_eq!(
            l.register(0, "a"),
            Err(CELL_LIBPROF_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn register_empty_name_is_invalid() {
        let mut l = bring_up();
        assert_eq!(
            l.register(1, ""),
            Err(CELL_LIBPROF_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn register_oversize_name_is_invalid() {
        let mut l = bring_up();
        let huge = "a".repeat(CELL_LIBPROF_MAX_NAME_LEN + 1);
        assert_eq!(
            l.register(1, huge),
            Err(CELL_LIBPROF_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn register_duplicate_rejected() {
        let mut l = bring_up();
        l.register(1, "probe").unwrap();
        assert_eq!(
            l.register(1, "other"),
            Err(CELL_LIBPROF_ERROR_PROBE_ALREADY_EXISTS)
        );
    }

    #[test]
    fn register_past_cap_rejected() {
        let mut l = bring_up();
        for i in 1..=CELL_LIBPROF_MAX_PROBES as u32 {
            l.register(i, "n").unwrap();
        }
        let extra_id = (CELL_LIBPROF_MAX_PROBES + 1) as u32;
        assert_eq!(
            l.register(extra_id, "n"),
            Err(CELL_LIBPROF_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn unregister_unknown_is_not_found() {
        let mut l = bring_up();
        assert_eq!(
            l.unregister(99),
            Err(CELL_LIBPROF_ERROR_PROBE_NOT_FOUND)
        );
    }

    #[test]
    fn unregister_roundtrip() {
        let mut l = bring_up();
        l.register(1, "a").unwrap();
        l.register(2, "b").unwrap();
        l.unregister(1).unwrap();
        assert_eq!(l.probe_count(), 1);
        assert!(l.find_probe(1).is_none());
        assert!(l.find_probe(2).is_some());
    }

    #[test]
    fn terminate_without_init_is_not_initialized() {
        let mut l = Libprof::new();
        assert_eq!(l.terminate(), Err(CELL_LIBPROF_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn terminate_clears_probes() {
        let mut l = bring_up();
        l.register(1, "a").unwrap();
        l.register(2, "b").unwrap();
        l.terminate().unwrap();
        assert_eq!(l.probe_count(), 0);
        assert_eq!(l.state(), ModuleState::Terminated);
    }

    #[test]
    fn operations_after_terminate_rejected() {
        let mut l = bring_up();
        l.terminate().unwrap();
        assert_eq!(l.register(1, "a"), Err(CELL_LIBPROF_ERROR_TERMINATED));
        assert_eq!(l.unregister(1), Err(CELL_LIBPROF_ERROR_TERMINATED));
        assert_eq!(l.terminate(), Err(CELL_LIBPROF_ERROR_TERMINATED));
    }

    #[test]
    fn name_at_boundary_accepted() {
        let mut l = bring_up();
        let name = "x".repeat(CELL_LIBPROF_MAX_NAME_LEN);
        l.register(1, name.clone()).unwrap();
        assert_eq!(l.find_probe(1).unwrap().name, name);
    }

    #[test]
    fn full_libprof_lifecycle_smoke() {
        let mut l = Libprof::new();
        l.init().unwrap();

        // Register three probes.
        l.register(1, "enter_world").unwrap();
        l.register(2, "player_dies").unwrap();
        l.register(3, "frame_begin").unwrap();
        assert_eq!(l.probe_count(), 3);

        // Drop one, verify others persist.
        l.unregister(2).unwrap();
        assert!(l.find_probe(2).is_none());
        assert!(l.find_probe(1).is_some());
        assert!(l.find_probe(3).is_some());

        // Terminate drops everything.
        l.terminate().unwrap();
        assert_eq!(l.probe_count(), 0);

        // Counter trace.
        assert_eq!(l.init_calls(), 1);
        assert_eq!(l.register_calls(), 3);
        assert_eq!(l.unregister_calls(), 1);
        assert_eq!(l.terminate_calls(), 1);

        // Post-terminate is sealed.
        assert_eq!(l.init(), Err(CELL_LIBPROF_ERROR_TERMINATED));
    }
}
