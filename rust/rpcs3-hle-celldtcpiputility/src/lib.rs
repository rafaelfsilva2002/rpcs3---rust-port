//! Rust port of `rpcs3/Emu/Cell/Modules/cellDtcpIpUtility.cpp`.
//!
//! The C++ source (100 lines) declares 13 PRX entries under the module
//! name `cellDtcpIpUtility`. Every body is a stub that logs
//! `UNIMPLEMENTED_FUNC` and returns `CELL_OK`. DTCP-IP (Digital
//! Transmission Content Protection over IP) is the copy-protection
//! scheme used when a PS3 streams protected media to a home-AV
//! receiver — it involves:
//!
//!  1. Initializing the library (Initialize ↔ Finalize).
//!  2. Activating the device with a license authority
//!     (Activate ↔ CheckActivation / SuspendActivationForDebug).
//!  3. Opening a session and running a DTCP sequence
//!     (Open ↔ Close, StartSequence ↔ StopSequence).
//!  4. Shuffling ciphertext / plaintext around
//!     (SetEncryptedData, GetDecryptedData, Read, Seek).
//!
//! The Rust port preserves the happy-path `CELL_OK` semantics of the C++
//! stub and adds FSM enforcement so callers that skip steps get a named
//! error instead of silent success. Facility `0x8002_D0__` is reserved
//! in this port for DTCP-IP placeholder errors; the real firmware codes
//! are not committed in RPCS3's source tree yet.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:4 / cpp:84.
pub const MODULE_NAME: &str = "cellDtcpIpUtility";

/// Registered entry points in the exact REG_FUNC order at cpp:86-98.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellDtcpIpRead",
    "cellDtcpIpFinalize",
    "cellDtcpIpActivate",
    "cellDtcpIpOpen",
    "cellDtcpIpCheckActivation",
    "cellDtcpIpInitialize",
    "cellDtcpIpGetDecryptedData",
    "cellDtcpIpStopSequence",
    "cellDtcpIpSeek",
    "cellDtcpIpStartSequence",
    "cellDtcpIpSetEncryptedData",
    "cellDtcpIpClose",
    "cellDtcpIpSuspendActivationForDebug",
];

// --- Error codes (placeholder facility 0x8002_D0__) ---------------------
//
// The C++ source commits no named error codes. The values below are
// internal placeholders used purely to enforce FSM invariants from the
// Rust side — happy-path returns preserve the C++ `CELL_OK`.

pub const CELL_DTCPIP_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_D001);
pub const CELL_DTCPIP_ERROR_REINITIALIZED: CellError = CellError(0x8002_D002);
pub const CELL_DTCPIP_ERROR_NOT_ACTIVATED: CellError = CellError(0x8002_D003);
pub const CELL_DTCPIP_ERROR_ALREADY_ACTIVATED: CellError = CellError(0x8002_D004);
pub const CELL_DTCPIP_ERROR_NOT_OPEN: CellError = CellError(0x8002_D005);
pub const CELL_DTCPIP_ERROR_ALREADY_OPEN: CellError = CellError(0x8002_D006);
pub const CELL_DTCPIP_ERROR_NOT_IN_SEQUENCE: CellError = CellError(0x8002_D007);
pub const CELL_DTCPIP_ERROR_ALREADY_IN_SEQUENCE: CellError = CellError(0x8002_D008);
pub const CELL_DTCPIP_ERROR_INVALID_PARAMETER: CellError = CellError(0x8002_D009);
pub const CELL_DTCPIP_ERROR_NO_DATA: CellError = CellError(0x8002_D00A);
pub const CELL_DTCPIP_ERROR_SUSPENDED: CellError = CellError(0x8002_D00B);
pub const CELL_DTCPIP_ERROR_FINALIZED: CellError = CellError(0x8002_D00C);

// --- FSM -----------------------------------------------------------------

/// Library-level state. Initialize/Finalize toggle this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    Uninitialized,
    Initialized,
    Finalized,
}

/// Device activation state. Activate transitions `NotActivated →
/// Activated`; SuspendActivationForDebug transitions either live state
/// to `Suspended`. The real firmware drives this via a license
/// authority round-trip; the port just flips the flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationState {
    NotActivated,
    Activated,
    Suspended,
}

/// Session lifecycle within a single Open/Close window.
///
/// `Closed` → `Open` via `open` → `InSequence` via `start_sequence` →
/// `Open` via `stop_sequence` → `Closed` via `close`. The sequence can
/// be restarted on the same open session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Closed,
    Open,
    InSequence,
}

/// Byte count copied by the most recent `read`, plus the number of
/// bytes that remained in the decrypted buffer afterwards. Used by
/// callers so they can poll `read()` until the tail drains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOutcome {
    pub bytes_read: usize,
    pub remaining: usize,
}

/// HLE state for a single DTCP-IP context.
///
/// This is a per-PRX singleton in the real firmware; the port keeps the
/// same shape — one `DtcpIp` per guest process is what the stub assumes.
#[derive(Debug, Default)]
pub struct DtcpIp {
    module_state: Option<ModuleState>,
    activation_state: Option<ActivationState>,
    session_state: Option<SessionState>,
    encrypted: Vec<u8>,
    decrypted: Vec<u8>,
    read_pos: u64,
    // Counters — useful for tests that want to assert an entry point was
    // invoked without inspecting side effects.
    initialize_calls: u32,
    finalize_calls: u32,
    activate_calls: u32,
    check_activation_calls: u32,
    suspend_calls: u32,
    open_calls: u32,
    close_calls: u32,
    start_sequence_calls: u32,
    stop_sequence_calls: u32,
    set_encrypted_calls: u32,
    get_decrypted_calls: u32,
    read_calls: u32,
    seek_calls: u32,
}

impl DtcpIp {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            module_state: None,
            activation_state: None,
            session_state: None,
            encrypted: Vec::new(),
            decrypted: Vec::new(),
            read_pos: 0,
            initialize_calls: 0,
            finalize_calls: 0,
            activate_calls: 0,
            check_activation_calls: 0,
            suspend_calls: 0,
            open_calls: 0,
            close_calls: 0,
            start_sequence_calls: 0,
            stop_sequence_calls: 0,
            set_encrypted_calls: 0,
            get_decrypted_calls: 0,
            read_calls: 0,
            seek_calls: 0,
        }
    }

    // --- accessors ---

    #[must_use]
    pub fn module_state(&self) -> ModuleState {
        self.module_state.unwrap_or(ModuleState::Uninitialized)
    }

    #[must_use]
    pub fn activation_state(&self) -> ActivationState {
        self.activation_state
            .unwrap_or(ActivationState::NotActivated)
    }

    #[must_use]
    pub fn session_state(&self) -> SessionState {
        self.session_state.unwrap_or(SessionState::Closed)
    }

    #[must_use]
    pub fn read_pos(&self) -> u64 {
        self.read_pos
    }

    #[must_use]
    pub fn decrypted_len(&self) -> usize {
        self.decrypted.len()
    }

    #[must_use]
    pub fn encrypted_len(&self) -> usize {
        self.encrypted.len()
    }

    #[must_use]
    pub fn initialize_calls(&self) -> u32 {
        self.initialize_calls
    }
    #[must_use]
    pub fn finalize_calls(&self) -> u32 {
        self.finalize_calls
    }
    #[must_use]
    pub fn activate_calls(&self) -> u32 {
        self.activate_calls
    }
    #[must_use]
    pub fn check_activation_calls(&self) -> u32 {
        self.check_activation_calls
    }
    #[must_use]
    pub fn suspend_calls(&self) -> u32 {
        self.suspend_calls
    }
    #[must_use]
    pub fn open_calls(&self) -> u32 {
        self.open_calls
    }
    #[must_use]
    pub fn close_calls(&self) -> u32 {
        self.close_calls
    }
    #[must_use]
    pub fn start_sequence_calls(&self) -> u32 {
        self.start_sequence_calls
    }
    #[must_use]
    pub fn stop_sequence_calls(&self) -> u32 {
        self.stop_sequence_calls
    }
    #[must_use]
    pub fn set_encrypted_calls(&self) -> u32 {
        self.set_encrypted_calls
    }
    #[must_use]
    pub fn get_decrypted_calls(&self) -> u32 {
        self.get_decrypted_calls
    }
    #[must_use]
    pub fn read_calls(&self) -> u32 {
        self.read_calls
    }
    #[must_use]
    pub fn seek_calls(&self) -> u32 {
        self.seek_calls
    }

    // --- guards ---

    fn require_initialized(&self) -> Result<(), CellError> {
        match self.module_state() {
            ModuleState::Uninitialized => Err(CELL_DTCPIP_ERROR_NOT_INITIALIZED),
            ModuleState::Finalized => Err(CELL_DTCPIP_ERROR_FINALIZED),
            ModuleState::Initialized => Ok(()),
        }
    }

    fn require_activated(&self) -> Result<(), CellError> {
        self.require_initialized()?;
        match self.activation_state() {
            ActivationState::Activated => Ok(()),
            ActivationState::Suspended => Err(CELL_DTCPIP_ERROR_SUSPENDED),
            ActivationState::NotActivated => Err(CELL_DTCPIP_ERROR_NOT_ACTIVATED),
        }
    }

    fn require_open(&self) -> Result<(), CellError> {
        self.require_activated()?;
        match self.session_state() {
            SessionState::Closed => Err(CELL_DTCPIP_ERROR_NOT_OPEN),
            _ => Ok(()),
        }
    }

    fn require_in_sequence(&self) -> Result<(), CellError> {
        self.require_open()?;
        if self.session_state() == SessionState::InSequence {
            Ok(())
        } else {
            Err(CELL_DTCPIP_ERROR_NOT_IN_SEQUENCE)
        }
    }

    // --- entry points ---

    /// `cellDtcpIpInitialize` (cpp:36-40). Transitions
    /// `Uninitialized → Initialized`; rejects double-init with
    /// `REINITIALIZED` and any attempt on a previously finalized
    /// library with `FINALIZED` (matching sceAd-family precedent).
    pub fn initialize(&mut self) -> Result<(), CellError> {
        match self.module_state() {
            ModuleState::Uninitialized => {
                self.module_state = Some(ModuleState::Initialized);
                self.initialize_calls = self.initialize_calls.saturating_add(1);
                Ok(())
            }
            ModuleState::Initialized => Err(CELL_DTCPIP_ERROR_REINITIALIZED),
            ModuleState::Finalized => Err(CELL_DTCPIP_ERROR_FINALIZED),
        }
    }

    /// `cellDtcpIpFinalize` (cpp:12-16). Closes any live session and
    /// marks the library as `Finalized`. `Uninitialized → NOT_INITIALIZED`.
    pub fn finalize(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        // Implicitly closes any live session + clears activation, matching
        // firmware that tears everything down on finalize.
        self.session_state = Some(SessionState::Closed);
        self.activation_state = Some(ActivationState::NotActivated);
        self.encrypted.clear();
        self.decrypted.clear();
        self.read_pos = 0;
        self.module_state = Some(ModuleState::Finalized);
        self.finalize_calls = self.finalize_calls.saturating_add(1);
        Ok(())
    }

    /// `cellDtcpIpActivate` (cpp:18-22). Requires the library to be up.
    /// A second activation returns `ALREADY_ACTIVATED`; a resumed /
    /// suspended library is rejected until
    /// [`Self::resume_activation`] lifts the suspend flag.
    pub fn activate(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        match self.activation_state() {
            ActivationState::NotActivated => {
                self.activation_state = Some(ActivationState::Activated);
                self.activate_calls = self.activate_calls.saturating_add(1);
                Ok(())
            }
            ActivationState::Activated => Err(CELL_DTCPIP_ERROR_ALREADY_ACTIVATED),
            ActivationState::Suspended => Err(CELL_DTCPIP_ERROR_SUSPENDED),
        }
    }

    /// `cellDtcpIpCheckActivation` (cpp:30-34). Returns `Ok(true)` if
    /// the device is currently activated, `Ok(false)` otherwise. Like
    /// every other entry point, requires the library to be initialized.
    pub fn check_activation(&mut self) -> Result<bool, CellError> {
        self.require_initialized()?;
        self.check_activation_calls = self.check_activation_calls.saturating_add(1);
        Ok(self.activation_state() == ActivationState::Activated)
    }

    /// `cellDtcpIpSuspendActivationForDebug` (cpp:78-82). Flips an
    /// already-activated library to `Suspended`; the real firmware also
    /// emits a diagnostic log line. Sessions are torn down so the game
    /// cannot keep streaming while the license is paused.
    pub fn suspend_activation_for_debug(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        match self.activation_state() {
            ActivationState::Activated => {
                self.activation_state = Some(ActivationState::Suspended);
                self.session_state = Some(SessionState::Closed);
                self.encrypted.clear();
                self.decrypted.clear();
                self.read_pos = 0;
                self.suspend_calls = self.suspend_calls.saturating_add(1);
                Ok(())
            }
            ActivationState::Suspended => Err(CELL_DTCPIP_ERROR_SUSPENDED),
            ActivationState::NotActivated => Err(CELL_DTCPIP_ERROR_NOT_ACTIVATED),
        }
    }

    /// Lift a debug-suspend back to `Activated`. Rust-only helper; the
    /// firmware drops back via a license round-trip.
    pub fn resume_activation(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        if self.activation_state() != ActivationState::Suspended {
            return Err(CELL_DTCPIP_ERROR_NOT_ACTIVATED);
        }
        self.activation_state = Some(ActivationState::Activated);
        Ok(())
    }

    /// `cellDtcpIpOpen` (cpp:24-28). Opens a session;
    /// `ALREADY_OPEN` on a live session.
    pub fn open(&mut self) -> Result<(), CellError> {
        self.require_activated()?;
        match self.session_state() {
            SessionState::Closed => {
                self.session_state = Some(SessionState::Open);
                self.open_calls = self.open_calls.saturating_add(1);
                Ok(())
            }
            SessionState::Open | SessionState::InSequence => {
                Err(CELL_DTCPIP_ERROR_ALREADY_OPEN)
            }
        }
    }

    /// `cellDtcpIpClose` (cpp:72-76). Tears down an `Open` or
    /// `InSequence` session. Clears any buffered data + read cursor.
    pub fn close(&mut self) -> Result<(), CellError> {
        self.require_activated()?;
        if self.session_state() == SessionState::Closed {
            return Err(CELL_DTCPIP_ERROR_NOT_OPEN);
        }
        self.session_state = Some(SessionState::Closed);
        self.encrypted.clear();
        self.decrypted.clear();
        self.read_pos = 0;
        self.close_calls = self.close_calls.saturating_add(1);
        Ok(())
    }

    /// `cellDtcpIpStartSequence` (cpp:60-64). Requires an open session
    /// in the `Open` state; rejects if a sequence is already running.
    pub fn start_sequence(&mut self) -> Result<(), CellError> {
        self.require_open()?;
        match self.session_state() {
            SessionState::Open => {
                self.session_state = Some(SessionState::InSequence);
                self.read_pos = 0;
                self.start_sequence_calls = self.start_sequence_calls.saturating_add(1);
                Ok(())
            }
            SessionState::InSequence => Err(CELL_DTCPIP_ERROR_ALREADY_IN_SEQUENCE),
            SessionState::Closed => unreachable!(),
        }
    }

    /// `cellDtcpIpStopSequence` (cpp:48-52). Requires `InSequence`;
    /// drops the decrypted buffer + read cursor and falls back to
    /// `Open`.
    pub fn stop_sequence(&mut self) -> Result<(), CellError> {
        self.require_in_sequence()?;
        self.session_state = Some(SessionState::Open);
        self.decrypted.clear();
        self.encrypted.clear();
        self.read_pos = 0;
        self.stop_sequence_calls = self.stop_sequence_calls.saturating_add(1);
        Ok(())
    }

    /// `cellDtcpIpSetEncryptedData` (cpp:66-70). Caller hands the
    /// library a slab of ciphertext; the real firmware would queue it
    /// for the SPU decryption pipeline. The port stores it for the
    /// stubbed `Read` path to drain.
    pub fn set_encrypted_data(&mut self, data: &[u8]) -> Result<(), CellError> {
        self.require_in_sequence()?;
        if data.is_empty() {
            return Err(CELL_DTCPIP_ERROR_INVALID_PARAMETER);
        }
        self.encrypted.extend_from_slice(data);
        // Since the stub has no real decryptor, model an identity
        // transform — decrypted buffer gets the same bytes. Tests can
        // still exercise the full read/seek cursor arithmetic this way.
        self.decrypted.extend_from_slice(data);
        self.set_encrypted_calls = self.set_encrypted_calls.saturating_add(1);
        Ok(())
    }

    /// `cellDtcpIpGetDecryptedData` (cpp:42-46). Returns the full
    /// decrypted buffer accumulated so far, leaving it in place.
    pub fn get_decrypted_data(&mut self) -> Result<&[u8], CellError> {
        self.require_in_sequence()?;
        self.get_decrypted_calls = self.get_decrypted_calls.saturating_add(1);
        if self.decrypted.is_empty() {
            Err(CELL_DTCPIP_ERROR_NO_DATA)
        } else {
            Ok(&self.decrypted)
        }
    }

    /// `cellDtcpIpRead` (cpp:6-10). Copies up to `buf.len()` decrypted
    /// bytes into `buf`, advancing the read cursor. Returns the byte
    /// count actually copied + the number of bytes remaining. At EOF
    /// returns `NO_DATA` rather than a zero-byte read.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<ReadOutcome, CellError> {
        self.require_in_sequence()?;
        if buf.is_empty() {
            return Err(CELL_DTCPIP_ERROR_INVALID_PARAMETER);
        }
        let start = self.read_pos as usize;
        if start >= self.decrypted.len() {
            return Err(CELL_DTCPIP_ERROR_NO_DATA);
        }
        let available = self.decrypted.len() - start;
        let n = core::cmp::min(buf.len(), available);
        buf[..n].copy_from_slice(&self.decrypted[start..start + n]);
        self.read_pos += n as u64;
        self.read_calls = self.read_calls.saturating_add(1);
        Ok(ReadOutcome {
            bytes_read: n,
            remaining: self.decrypted.len() - (start + n),
        })
    }

    /// `cellDtcpIpSeek` (cpp:54-58). Absolute seek — rejects cursors
    /// past the decrypted buffer's end with `INVALID_PARAMETER`.
    pub fn seek(&mut self, offset: u64) -> Result<(), CellError> {
        self.require_in_sequence()?;
        if offset as usize > self.decrypted.len() {
            return Err(CELL_DTCPIP_ERROR_INVALID_PARAMETER);
        }
        self.read_pos = offset;
        self.seek_calls = self.seek_calls.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bring_up_to_in_sequence() -> DtcpIp {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        d.open().unwrap();
        d.start_sequence().unwrap();
        d
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellDtcpIpUtility");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 13);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellDtcpIpRead");
        assert_eq!(REGISTERED_ENTRY_POINTS[5], "cellDtcpIpInitialize");
        assert_eq!(
            REGISTERED_ENTRY_POINTS[12],
            "cellDtcpIpSuspendActivationForDebug"
        );
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_DTCPIP_ERROR_NOT_INITIALIZED.0, 0x8002_D001);
        assert_eq!(CELL_DTCPIP_ERROR_REINITIALIZED.0, 0x8002_D002);
        assert_eq!(CELL_DTCPIP_ERROR_NOT_ACTIVATED.0, 0x8002_D003);
        assert_eq!(CELL_DTCPIP_ERROR_ALREADY_ACTIVATED.0, 0x8002_D004);
        assert_eq!(CELL_DTCPIP_ERROR_NOT_OPEN.0, 0x8002_D005);
        assert_eq!(CELL_DTCPIP_ERROR_ALREADY_OPEN.0, 0x8002_D006);
        assert_eq!(CELL_DTCPIP_ERROR_NOT_IN_SEQUENCE.0, 0x8002_D007);
        assert_eq!(CELL_DTCPIP_ERROR_ALREADY_IN_SEQUENCE.0, 0x8002_D008);
        assert_eq!(CELL_DTCPIP_ERROR_INVALID_PARAMETER.0, 0x8002_D009);
        assert_eq!(CELL_DTCPIP_ERROR_NO_DATA.0, 0x8002_D00A);
        assert_eq!(CELL_DTCPIP_ERROR_SUSPENDED.0, 0x8002_D00B);
        assert_eq!(CELL_DTCPIP_ERROR_FINALIZED.0, 0x8002_D00C);
    }

    #[test]
    fn starts_uninitialized_unactivated_closed() {
        let d = DtcpIp::new();
        assert_eq!(d.module_state(), ModuleState::Uninitialized);
        assert_eq!(d.activation_state(), ActivationState::NotActivated);
        assert_eq!(d.session_state(), SessionState::Closed);
    }

    #[test]
    fn initialize_transitions_to_initialized() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        assert_eq!(d.module_state(), ModuleState::Initialized);
        assert_eq!(d.initialize_calls(), 1);
    }

    #[test]
    fn double_initialize_is_reinitialized() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        assert_eq!(d.initialize(), Err(CELL_DTCPIP_ERROR_REINITIALIZED));
    }

    #[test]
    fn finalize_without_init_is_not_initialized() {
        let mut d = DtcpIp::new();
        assert_eq!(d.finalize(), Err(CELL_DTCPIP_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn finalize_marks_terminal_and_blocks_reinit() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.finalize().unwrap();
        assert_eq!(d.module_state(), ModuleState::Finalized);
        assert_eq!(d.initialize(), Err(CELL_DTCPIP_ERROR_FINALIZED));
    }

    #[test]
    fn activate_requires_initialized() {
        let mut d = DtcpIp::new();
        assert_eq!(d.activate(), Err(CELL_DTCPIP_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn activate_transitions_to_activated() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        assert_eq!(d.activation_state(), ActivationState::Activated);
    }

    #[test]
    fn double_activate_is_already_activated() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        assert_eq!(d.activate(), Err(CELL_DTCPIP_ERROR_ALREADY_ACTIVATED));
    }

    #[test]
    fn check_activation_reports_current_state() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        assert!(!d.check_activation().unwrap());
        d.activate().unwrap();
        assert!(d.check_activation().unwrap());
    }

    #[test]
    fn suspend_requires_activated() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        assert_eq!(
            d.suspend_activation_for_debug(),
            Err(CELL_DTCPIP_ERROR_NOT_ACTIVATED)
        );
    }

    #[test]
    fn suspend_clears_session_and_blocks_open() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        d.open().unwrap();
        d.suspend_activation_for_debug().unwrap();
        assert_eq!(d.activation_state(), ActivationState::Suspended);
        assert_eq!(d.session_state(), SessionState::Closed);
        assert_eq!(d.open(), Err(CELL_DTCPIP_ERROR_SUSPENDED));
    }

    #[test]
    fn resume_activation_restores_activated() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        d.suspend_activation_for_debug().unwrap();
        d.resume_activation().unwrap();
        assert_eq!(d.activation_state(), ActivationState::Activated);
    }

    #[test]
    fn open_requires_activated() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        assert_eq!(d.open(), Err(CELL_DTCPIP_ERROR_NOT_ACTIVATED));
    }

    #[test]
    fn double_open_is_already_open() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        d.open().unwrap();
        assert_eq!(d.open(), Err(CELL_DTCPIP_ERROR_ALREADY_OPEN));
    }

    #[test]
    fn close_without_open_is_not_open() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        assert_eq!(d.close(), Err(CELL_DTCPIP_ERROR_NOT_OPEN));
    }

    #[test]
    fn start_sequence_requires_open() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        assert_eq!(d.start_sequence(), Err(CELL_DTCPIP_ERROR_NOT_OPEN));
    }

    #[test]
    fn double_start_sequence_is_already_in_sequence() {
        let mut d = bring_up_to_in_sequence();
        assert_eq!(
            d.start_sequence(),
            Err(CELL_DTCPIP_ERROR_ALREADY_IN_SEQUENCE)
        );
    }

    #[test]
    fn stop_sequence_requires_in_sequence() {
        let mut d = DtcpIp::new();
        d.initialize().unwrap();
        d.activate().unwrap();
        d.open().unwrap();
        assert_eq!(
            d.stop_sequence(),
            Err(CELL_DTCPIP_ERROR_NOT_IN_SEQUENCE)
        );
    }

    #[test]
    fn stop_sequence_returns_to_open_and_restart_works() {
        let mut d = bring_up_to_in_sequence();
        d.set_encrypted_data(&[1, 2, 3]).unwrap();
        d.stop_sequence().unwrap();
        assert_eq!(d.session_state(), SessionState::Open);
        assert_eq!(d.decrypted_len(), 0);
        d.start_sequence().unwrap();
        assert_eq!(d.session_state(), SessionState::InSequence);
    }

    #[test]
    fn set_encrypted_data_empty_is_invalid_parameter() {
        let mut d = bring_up_to_in_sequence();
        assert_eq!(
            d.set_encrypted_data(&[]),
            Err(CELL_DTCPIP_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn set_encrypted_data_populates_decrypted_mirror() {
        let mut d = bring_up_to_in_sequence();
        d.set_encrypted_data(&[0xAA, 0xBB, 0xCC]).unwrap();
        assert_eq!(d.encrypted_len(), 3);
        assert_eq!(d.decrypted_len(), 3);
    }

    #[test]
    fn get_decrypted_data_no_data_when_empty() {
        let mut d = bring_up_to_in_sequence();
        assert_eq!(
            d.get_decrypted_data(),
            Err(CELL_DTCPIP_ERROR_NO_DATA)
        );
    }

    #[test]
    fn read_advances_cursor_and_reports_remaining() {
        let mut d = bring_up_to_in_sequence();
        d.set_encrypted_data(&[1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let mut buf = [0u8; 4];
        let out = d.read(&mut buf).unwrap();
        assert_eq!(out.bytes_read, 4);
        assert_eq!(out.remaining, 4);
        assert_eq!(buf, [1, 2, 3, 4]);
        assert_eq!(d.read_pos(), 4);
        let out2 = d.read(&mut buf).unwrap();
        assert_eq!(out2.bytes_read, 4);
        assert_eq!(out2.remaining, 0);
        assert_eq!(buf, [5, 6, 7, 8]);
    }

    #[test]
    fn read_at_eof_is_no_data() {
        let mut d = bring_up_to_in_sequence();
        d.set_encrypted_data(&[1, 2, 3]).unwrap();
        let mut buf = [0u8; 8];
        let _ = d.read(&mut buf).unwrap();
        assert_eq!(d.read(&mut buf), Err(CELL_DTCPIP_ERROR_NO_DATA));
    }

    #[test]
    fn read_empty_buffer_is_invalid_parameter() {
        let mut d = bring_up_to_in_sequence();
        d.set_encrypted_data(&[1, 2]).unwrap();
        let mut empty = [0u8; 0];
        assert_eq!(
            d.read(&mut empty),
            Err(CELL_DTCPIP_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn seek_past_end_is_invalid_parameter() {
        let mut d = bring_up_to_in_sequence();
        d.set_encrypted_data(&[1, 2, 3]).unwrap();
        assert_eq!(d.seek(10), Err(CELL_DTCPIP_ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn seek_to_middle_affects_next_read() {
        let mut d = bring_up_to_in_sequence();
        d.set_encrypted_data(&[10, 20, 30, 40, 50]).unwrap();
        d.seek(3).unwrap();
        let mut buf = [0u8; 8];
        let out = d.read(&mut buf).unwrap();
        assert_eq!(out.bytes_read, 2);
        assert_eq!(&buf[..2], &[40, 50]);
    }

    #[test]
    fn close_after_in_sequence_clears_state() {
        let mut d = bring_up_to_in_sequence();
        d.set_encrypted_data(&[1, 2, 3]).unwrap();
        d.close().unwrap();
        assert_eq!(d.session_state(), SessionState::Closed);
        assert_eq!(d.decrypted_len(), 0);
        assert_eq!(d.read_pos(), 0);
    }

    #[test]
    fn ops_on_uninitialized_are_rejected() {
        let mut d = DtcpIp::new();
        assert_eq!(d.open(), Err(CELL_DTCPIP_ERROR_NOT_INITIALIZED));
        assert_eq!(
            d.start_sequence(),
            Err(CELL_DTCPIP_ERROR_NOT_INITIALIZED)
        );
        assert_eq!(d.close(), Err(CELL_DTCPIP_ERROR_NOT_INITIALIZED));
        let mut buf = [0u8; 4];
        assert_eq!(d.read(&mut buf), Err(CELL_DTCPIP_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn full_dtcpip_lifecycle_smoke() {
        let mut d = DtcpIp::new();
        // 1. Spin the library up + activate the device.
        d.initialize().unwrap();
        d.activate().unwrap();
        assert!(d.check_activation().unwrap());

        // 2. Open a session and start a DTCP sequence.
        d.open().unwrap();
        d.start_sequence().unwrap();

        // 3. Push two slabs of ciphertext + drain via read+seek.
        d.set_encrypted_data(&[0x01, 0x02, 0x03, 0x04]).unwrap();
        d.set_encrypted_data(&[0x05, 0x06, 0x07, 0x08]).unwrap();
        assert_eq!(d.decrypted_len(), 8);

        let mut buf = [0u8; 3];
        let r1 = d.read(&mut buf).unwrap();
        assert_eq!(r1.bytes_read, 3);
        assert_eq!(&buf, &[0x01, 0x02, 0x03]);

        // 4. Seek to the tail, read the remainder.
        d.seek(5).unwrap();
        let out = d.read(&mut buf).unwrap();
        assert_eq!(out.bytes_read, 3);
        assert_eq!(&buf, &[0x06, 0x07, 0x08]);
        assert_eq!(out.remaining, 0);

        // 5. Wind everything down in the correct order.
        d.stop_sequence().unwrap();
        d.close().unwrap();
        d.finalize().unwrap();
        assert_eq!(d.module_state(), ModuleState::Finalized);

        // 6. Counters reflect the exact dispatch trace.
        assert_eq!(d.initialize_calls(), 1);
        assert_eq!(d.activate_calls(), 1);
        assert_eq!(d.check_activation_calls(), 1);
        assert_eq!(d.open_calls(), 1);
        assert_eq!(d.start_sequence_calls(), 1);
        assert_eq!(d.set_encrypted_calls(), 2);
        assert_eq!(d.read_calls(), 2);
        assert_eq!(d.seek_calls(), 1);
        assert_eq!(d.stop_sequence_calls(), 1);
        assert_eq!(d.close_calls(), 1);
        assert_eq!(d.finalize_calls(), 1);
    }
}
