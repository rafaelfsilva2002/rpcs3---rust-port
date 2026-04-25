//! `rpcs3-hle-sys-rsxaudio-user` — PS3 RSX audio subsystem user-mode HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sys_rsxaudio_.cpp` (72 linhas).  Every
//! entry point in the C++ file is marked `UNIMPLEMENTED_FUNC` and
//! returns `CELL_OK`; what the Rust port adds is the **observable
//! lifecycle** (`Uninitialized → Initialized → ConnectionOpen →
//! Prepared → Running`) so higher layers can exercise the state
//! machine in tests without needing the real RSX audio backend.
//!
//! ## Entry points covered
//!
//! | C++ function                           | Rust wrapper                       |
//! |----------------------------------------|------------------------------------|
//! | `sys_rsxaudio_initialize`              | [`RsxAudio::initialize`]           |
//! | `sys_rsxaudio_finalize`                | [`RsxAudio::finalize`]             |
//! | `sys_rsxaudio_create_connection`       | [`RsxAudio::create_connection`]    |
//! | `sys_rsxaudio_close_connection`        | [`RsxAudio::close_connection`]     |
//! | `sys_rsxaudio_import_shared_memory`    | [`RsxAudio::import_shared_memory`] |
//! | `sys_rsxaudio_unimport_shared_memory`  | [`RsxAudio::unimport_shared_memory`]|
//! | `sys_rsxaudio_prepare_process`         | [`RsxAudio::prepare_process`]      |
//! | `sys_rsxaudio_start_process`           | [`RsxAudio::start_process`]        |
//! | `sys_rsxaudio_stop_process`            | [`RsxAudio::stop_process`]         |

extern crate alloc;

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes
// =====================================================================

pub const CELL_EINVAL: CellError = CellError(0x8001_0002);

// =====================================================================
// Lifecycle state
// =====================================================================

/// Observable state of the RSX audio subsystem — matches the
/// documented firmware lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RsxAudioState {
    /// `sys_rsxaudio_initialize` not yet called.
    Uninitialized,
    /// After `initialize` — no active connection.
    Initialized,
    /// After `create_connection` — ready to import shared memory.
    ConnectionOpen,
    /// After `import_shared_memory` + `prepare_process` — ready to run.
    Prepared,
    /// After `start_process` — audio is flowing.
    Running,
}

// =====================================================================
// Manager
// =====================================================================

/// Mirror of the RSX audio subsystem's observable state.  Every entry
/// point is a stub in the C++ port but the state machine below
/// captures the ordering invariants the firmware enforces at the LV2
/// level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxAudio {
    pub state: RsxAudioState,
    pub shm_imported: bool,
}

impl Default for RsxAudio {
    fn default() -> Self {
        Self { state: RsxAudioState::Uninitialized, shm_imported: false }
    }
}

impl RsxAudio {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `sys_rsxaudio_initialize`.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if already initialised.
    pub fn initialize(&mut self) -> Result<(), CellError> {
        if self.state != RsxAudioState::Uninitialized {
            return Err(CELL_EINVAL);
        }
        self.state = RsxAudioState::Initialized;
        Ok(())
    }

    /// Port of `sys_rsxaudio_finalize`.
    ///
    /// # Errors
    /// * [`CELL_EINVAL`] if not initialised or still running/prepared.
    pub fn finalize(&mut self) -> Result<(), CellError> {
        if self.state == RsxAudioState::Uninitialized {
            return Err(CELL_EINVAL);
        }
        if matches!(self.state, RsxAudioState::Prepared | RsxAudioState::Running) {
            return Err(CELL_EINVAL);
        }
        self.state = RsxAudioState::Uninitialized;
        self.shm_imported = false;
        Ok(())
    }

    /// Port of `sys_rsxaudio_create_connection`.  Requires `Initialized`.
    pub fn create_connection(&mut self) -> Result<(), CellError> {
        if self.state != RsxAudioState::Initialized {
            return Err(CELL_EINVAL);
        }
        self.state = RsxAudioState::ConnectionOpen;
        Ok(())
    }

    /// Port of `sys_rsxaudio_close_connection`.  Requires
    /// `ConnectionOpen` or `Prepared` (the firmware allows close after
    /// prepare but before start).
    pub fn close_connection(&mut self) -> Result<(), CellError> {
        match self.state {
            RsxAudioState::ConnectionOpen | RsxAudioState::Prepared => {
                self.state = RsxAudioState::Initialized;
                self.shm_imported = false;
                Ok(())
            }
            _ => Err(CELL_EINVAL),
        }
    }

    /// Port of `sys_rsxaudio_import_shared_memory`.  Requires
    /// `ConnectionOpen`.
    pub fn import_shared_memory(&mut self) -> Result<(), CellError> {
        if self.state != RsxAudioState::ConnectionOpen {
            return Err(CELL_EINVAL);
        }
        if self.shm_imported {
            return Err(CELL_EINVAL);
        }
        self.shm_imported = true;
        Ok(())
    }

    /// Port of `sys_rsxaudio_unimport_shared_memory`.  Requires the
    /// shared memory to have been imported.
    pub fn unimport_shared_memory(&mut self) -> Result<(), CellError> {
        if !self.shm_imported {
            return Err(CELL_EINVAL);
        }
        if matches!(self.state, RsxAudioState::Prepared | RsxAudioState::Running) {
            return Err(CELL_EINVAL);
        }
        self.shm_imported = false;
        Ok(())
    }

    /// Port of `sys_rsxaudio_prepare_process`.  Requires imported SHM
    /// + `ConnectionOpen`.
    pub fn prepare_process(&mut self) -> Result<(), CellError> {
        if self.state != RsxAudioState::ConnectionOpen || !self.shm_imported {
            return Err(CELL_EINVAL);
        }
        self.state = RsxAudioState::Prepared;
        Ok(())
    }

    /// Port of `sys_rsxaudio_start_process`.  Requires `Prepared`.
    pub fn start_process(&mut self) -> Result<(), CellError> {
        if self.state != RsxAudioState::Prepared {
            return Err(CELL_EINVAL);
        }
        self.state = RsxAudioState::Running;
        Ok(())
    }

    /// Port of `sys_rsxaudio_stop_process`.  Requires `Running`.
    pub fn stop_process(&mut self) -> Result<(), CellError> {
        if self.state != RsxAudioState::Running {
            return Err(CELL_EINVAL);
        }
        self.state = RsxAudioState::Prepared;
        Ok(())
    }
}

// =====================================================================
// Registry
// =====================================================================

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sys_rsxaudio_close_connection",
    "sys_rsxaudio_create_connection",
    "sys_rsxaudio_finalize",
    "sys_rsxaudio_import_shared_memory",
    "sys_rsxaudio_initialize",
    "sys_rsxaudio_prepare_process",
    "sys_rsxaudio_start_process",
    "sys_rsxaudio_stop_process",
    "sys_rsxaudio_unimport_shared_memory",
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
    fn registry_has_nine_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 9);
    }

    #[test]
    fn registry_alphabetical_order_matches_cpp() {
        // sys_rsxaudio_.cpp:62-70 REG_FUNC order is alphabetical.
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "sys_rsxaudio_close_connection");
        assert_eq!(REGISTERED_ENTRY_POINTS[1], "sys_rsxaudio_create_connection");
        assert_eq!(REGISTERED_ENTRY_POINTS[4], "sys_rsxaudio_initialize");
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("sys_rsxaudio_nonexistent"));
    }

    // ---- initialize / finalize --------------------------------------

    #[test]
    fn initialize_from_uninitialized_transitions_to_initialized() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        assert_eq!(r.state, RsxAudioState::Initialized);
    }

    #[test]
    fn initialize_twice_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        assert_eq!(r.initialize().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn finalize_without_initialize_is_einval() {
        let mut r = RsxAudio::new();
        assert_eq!(r.finalize().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn finalize_from_initialized_returns_to_uninitialized() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.finalize().unwrap();
        assert_eq!(r.state, RsxAudioState::Uninitialized);
    }

    #[test]
    fn finalize_while_running_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        r.start_process().unwrap();
        assert_eq!(r.finalize().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn finalize_while_prepared_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        assert_eq!(r.finalize().unwrap_err(), CELL_EINVAL);
    }

    // ---- create / close connection ----------------------------------

    #[test]
    fn create_connection_without_initialize_is_einval() {
        let mut r = RsxAudio::new();
        assert_eq!(r.create_connection().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn create_connection_after_initialize_ok() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        assert_eq!(r.state, RsxAudioState::ConnectionOpen);
    }

    #[test]
    fn create_connection_twice_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        assert_eq!(r.create_connection().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn close_connection_without_open_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        assert_eq!(r.close_connection().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn close_connection_from_open_returns_to_initialized() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.close_connection().unwrap();
        assert_eq!(r.state, RsxAudioState::Initialized);
    }

    #[test]
    fn close_connection_from_prepared_also_ok() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        r.close_connection().unwrap();
        assert_eq!(r.state, RsxAudioState::Initialized);
        assert!(!r.shm_imported);
    }

    #[test]
    fn close_connection_while_running_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        r.start_process().unwrap();
        assert_eq!(r.close_connection().unwrap_err(), CELL_EINVAL);
    }

    // ---- import / unimport SHM --------------------------------------

    #[test]
    fn import_shm_without_connection_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        assert_eq!(r.import_shared_memory().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn import_shm_twice_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        assert_eq!(r.import_shared_memory().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn unimport_without_import_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        assert_eq!(r.unimport_shared_memory().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn unimport_while_running_is_einval() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        r.start_process().unwrap();
        assert_eq!(r.unimport_shared_memory().unwrap_err(), CELL_EINVAL);
    }

    // ---- prepare / start / stop -------------------------------------

    #[test]
    fn prepare_requires_imported_shm() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        assert_eq!(r.prepare_process().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn prepare_after_import_transitions_to_prepared() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        assert_eq!(r.state, RsxAudioState::Prepared);
    }

    #[test]
    fn start_requires_prepared() {
        let mut r = RsxAudio::new();
        assert_eq!(r.start_process().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn start_after_prepare_transitions_to_running() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        r.start_process().unwrap();
        assert_eq!(r.state, RsxAudioState::Running);
    }

    #[test]
    fn stop_without_running_is_einval() {
        let mut r = RsxAudio::new();
        assert_eq!(r.stop_process().unwrap_err(), CELL_EINVAL);
    }

    #[test]
    fn stop_from_running_returns_to_prepared() {
        let mut r = RsxAudio::new();
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        r.start_process().unwrap();
        r.stop_process().unwrap();
        assert_eq!(r.state, RsxAudioState::Prepared);
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_rsxaudio_lifecycle_smoke() {
        let mut r = RsxAudio::new();
        assert_eq!(r.state, RsxAudioState::Uninitialized);

        // Happy path: init → create → import → prepare → start → stop → unimport → close → finalize.
        r.initialize().unwrap();
        r.create_connection().unwrap();
        r.import_shared_memory().unwrap();
        r.prepare_process().unwrap();
        r.start_process().unwrap();
        assert_eq!(r.state, RsxAudioState::Running);

        r.stop_process().unwrap();
        r.close_connection().unwrap();
        // close_connection from Prepared clears shm_imported, so we can finalize directly.
        r.finalize().unwrap();

        assert_eq!(r.state, RsxAudioState::Uninitialized);
        assert!(!r.shm_imported);
    }
}
