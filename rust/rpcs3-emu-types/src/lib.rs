//! `rpcs3-emu-types` — observable ABI enums and Cell error codes.
//!
//! These types are the **wire contract** shared between C++ and Rust
//! portions of the emulator. Ordinals must remain stable forever:
//! serialized savestates, game-side error codes, and any FFI bridge
//! with C++ depend on them.
//!
//! ## Sources
//!
//! | Enum              | C++ source                              |
//! |-------------------|-----------------------------------------|
//! | `GameBootResult`  | `rpcs3/Emu/System.h:42`                 |
//! | `SystemState`     | `rpcs3/Emu/System.h:30`                 |
//! | `CpuFlag`         | `rpcs3/Emu/CPU/CPUThread.h:14`          |
//! | Cell errno values | `rpcs3/Emu/Cell/ErrorCodes.h` (various) |
//!
//! ## Invariants
//!
//! * Every `#[repr(u32)]` — matches C++ `enum class ... : u32`.
//! * Ordinals compile-time-asserted via `const _: () = assert!(...)`.
//! * Only ADD values at the end. Never reorder or renumber.

// =====================================================================
// GameBootResult — result of Emulator::BootGame / Emulator::Load
// =====================================================================

/// Mirrors `enum class game_boot_result : u32` at `rpcs3/Emu/System.h:42-62`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameBootResult {
    NoErrors = 0,
    GenericError = 1,
    NothingToBoot = 2,
    WrongDiscLocation = 3,
    InvalidFileOrFolder = 4,
    InvalidBdvdFolder = 5,
    InstallFailed = 6,
    DecryptionError = 7,
    FileCreationError = 8,
    FirmwareMissing = 9,
    FirmwareVersion = 10,
    UnsupportedDiscType = 11,
    SavestateCorrupted = 12,
    SavestateVersionUnsupported = 13,
    StillRunning = 14,
    AlreadyAdded = 15,
    CurrentlyRestricted = 16,
    DatabaseConfigMissing = 17,
}

impl GameBootResult {
    /// Mirrors `is_error(res)` at `rpcs3/Emu/System.h:64-67`.
    #[must_use]
    pub const fn is_error(self) -> bool {
        !matches!(self, GameBootResult::NoErrors)
    }

    /// Human-readable message, matches `rpcs3/Emu/System.cpp:139-161`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            GameBootResult::NoErrors => "",
            GameBootResult::GenericError => "generic error",
            GameBootResult::NothingToBoot => "nothing to boot",
            GameBootResult::WrongDiscLocation => "wrong disc location",
            GameBootResult::InvalidFileOrFolder => "invalid file or folder",
            GameBootResult::InvalidBdvdFolder => "invalid bdvd folder",
            GameBootResult::InstallFailed => "install failed",
            GameBootResult::DecryptionError => "decryption error",
            GameBootResult::FileCreationError => "file creation error",
            GameBootResult::FirmwareMissing => "firmware missing",
            GameBootResult::FirmwareVersion => "firmware version mismatch",
            GameBootResult::UnsupportedDiscType => "unsupported disc type",
            GameBootResult::SavestateCorrupted => "savestate corrupted",
            GameBootResult::SavestateVersionUnsupported => "savestate version unsupported",
            GameBootResult::StillRunning => "still running",
            GameBootResult::AlreadyAdded => "already added",
            GameBootResult::CurrentlyRestricted => "currently restricted",
            GameBootResult::DatabaseConfigMissing => "database config missing",
        }
    }
}

// Compile-time assertions: ordinals are load-bearing. Any change here
// is a breaking ABI change that must be done deliberately.
const _: () = {
    assert!(GameBootResult::NoErrors as u32 == 0);
    assert!(GameBootResult::DatabaseConfigMissing as u32 == 17);
};

// =====================================================================
// SystemState — lifecycle state of the Emulator singleton
// =====================================================================

/// Mirrors `enum class system_state : u32` at `rpcs3/Emu/System.h:30-40`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SystemState {
    Stopped = 0,
    Loading = 1,
    Stopping = 2,
    Running = 3,
    Paused = 4,
    /// Paused but cannot resume (post-error paused state).
    Frozen = 5,
    Ready = 6,
    Starting = 7,
}

const _: () = {
    assert!(SystemState::Stopped as u32 == 0);
    assert!(SystemState::Starting as u32 == 7);
};

// =====================================================================
// CpuFlag — per-thread scheduler/control bits (PPU + SPU)
// =====================================================================

/// Mirrors `enum class cpu_flag : u32` at `rpcs3/Emu/CPU/CPUThread.h:14-38`.
///
/// These are bit ORDINALS (used in a bitset), not bit masks. A value
/// `n` corresponds to mask `1u32 << n`.
///
/// **ABI-critical**: ordering must match `CPUThread.h` exactly. Any drift
/// silently breaks savestate serialization and scheduler logic.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpuFlag {
    /// Thread not running (HLE, initial state).
    Stop = 0,
    /// Irreversible exit.
    Exit = 1,
    /// Waiting state set by the thread itself.
    Wait = 2,
    /// Thread cannot properly return after next `check_state()`.
    Temp = 3,
    /// Thread suspended by suspend_all technique.
    Pause = 4,
    /// Thread suspended.
    Suspend = 5,
    /// Callback return requested.
    Ret = 6,
    /// Thread must complete the syscall after deserialization.
    Again = 7,
    /// Thread received a signal (HLE).
    Signal = 8,
    /// Thread must unlock memory mutex.
    Memory = 9,
    /// Thread has postponed work.
    Pending = 10,
    /// Thread needs to recheck if there is pending work before removing `Pending`.
    PendingRecheck = 11,
    /// Notification-only flag; does not change other state.
    Notify = 12,
    /// Thread asked to yield its execution time.
    Yield = 13,
    /// Thread asked to preempt all CPU threads.
    Preempt = 14,
    /// Request the thread to exit.
    ReqExit = 15,
    /// Emulation paused (debugger).
    DbgGlobalPause = 16,
    /// Thread paused (debugger).
    DbgPause = 17,
    /// Thread forced to pause after one instruction (debugger).
    DbgStep = 18,
}

impl CpuFlag {
    /// Bit mask for the flag: `1u32 << ordinal`.
    #[must_use]
    pub const fn mask(self) -> u32 {
        1u32 << (self as u32)
    }

    /// The `is_stopped` bitmask used by `CPUThread.h:41-44`:
    /// `stop | exit | again | req_exit`.
    #[must_use]
    pub const fn stopped_mask() -> u32 {
        CpuFlag::Stop.mask()
            | CpuFlag::Exit.mask()
            | CpuFlag::Again.mask()
            | CpuFlag::ReqExit.mask()
    }

    /// The `is_paused` bitmask used by `CPUThread.h:47-50`:
    /// `suspend | dbg_global_pause | dbg_pause`.
    /// Note: a thread that is stopped is NOT paused per the C++ helper.
    #[must_use]
    pub const fn paused_mask() -> u32 {
        CpuFlag::Suspend.mask()
            | CpuFlag::DbgGlobalPause.mask()
            | CpuFlag::DbgPause.mask()
    }
}

/// Test whether a state bitset indicates a stopped thread.
/// Matches `::is_stopped(state)` at `CPUThread.h:41`.
#[must_use]
pub const fn is_stopped(state: u32) -> bool {
    (state & CpuFlag::stopped_mask()) != 0
}

/// Test whether a state bitset indicates a paused thread.
/// Matches `::is_paused(state)` at `CPUThread.h:47`. A thread that is
/// stopped returns `false` here (stopped dominates paused).
#[must_use]
pub const fn is_paused(state: u32) -> bool {
    ((state & CpuFlag::paused_mask()) != 0) && !is_stopped(state)
}

const _: () = {
    assert!(CpuFlag::Stop as u32 == 0);
    assert!(CpuFlag::Exit as u32 == 1);
    assert!(CpuFlag::Wait as u32 == 2);
    assert!(CpuFlag::Temp as u32 == 3);
    assert!(CpuFlag::Pause as u32 == 4);
    assert!(CpuFlag::Suspend as u32 == 5);
    assert!(CpuFlag::Ret as u32 == 6);
    assert!(CpuFlag::Again as u32 == 7);
    assert!(CpuFlag::Signal as u32 == 8);
    assert!(CpuFlag::Memory as u32 == 9);
    assert!(CpuFlag::Pending as u32 == 10);
    assert!(CpuFlag::PendingRecheck as u32 == 11);
    assert!(CpuFlag::Notify as u32 == 12);
    assert!(CpuFlag::Yield as u32 == 13);
    assert!(CpuFlag::Preempt as u32 == 14);
    assert!(CpuFlag::ReqExit as u32 == 15);
    assert!(CpuFlag::DbgGlobalPause as u32 == 16);
    assert!(CpuFlag::DbgPause as u32 == 17);
    assert!(CpuFlag::DbgStep as u32 == 18);

    assert!(CpuFlag::Stop.mask() == 0x0000_0001);
    assert!(CpuFlag::Exit.mask() == 0x0000_0002);
    assert!(CpuFlag::Pause.mask() == 0x0000_0010);
    assert!(CpuFlag::DbgStep.mask() == 0x0004_0000);
};

// =====================================================================
// Cell errno — common return codes observable to game code
// =====================================================================

/// A Cell errno value (mirrors the C++ `CellError` wrapper).
///
/// The high bit (`0x8001_0000`) marks these as Cell facility errors,
/// common across `sys_fs`, `cellGame`, `cellAudio`, etc.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellError(pub u32);

impl CellError {
    pub const OK: Self = Self(0);
    // Generic POSIX-style errors (0x8001_0001+)
    pub const EAGAIN: Self = Self(0x8001_0001);
    pub const EINVAL: Self = Self(0x8001_0002);
    pub const ENOSYS: Self = Self(0x8001_0003);
    pub const ENOMEM: Self = Self(0x8001_0004);
    pub const ESRCH: Self = Self(0x8001_0005);
    pub const ENOENT: Self = Self(0x8001_0006);
    pub const ENOEXEC: Self = Self(0x8001_0007);
    pub const EDEADLK: Self = Self(0x8001_0008);
    pub const EPERM: Self = Self(0x8001_0009);
    pub const EBUSY: Self = Self(0x8001_000A);
    pub const ETIMEDOUT: Self = Self(0x8001_000B);
    pub const EABORT: Self = Self(0x8001_000C);
    pub const EFAULT: Self = Self(0x8001_000D);
    pub const ESTAT: Self = Self(0x8001_000F); // "stat" — generic status
    pub const EALIGN: Self = Self(0x8001_0010);
    pub const EKRESOURCE: Self = Self(0x8001_0011);
    pub const EISDIR: Self = Self(0x8001_0012);
    pub const ECANCELED: Self = Self(0x8001_0013);
    pub const EEXIST: Self = Self(0x8001_0014);
    pub const EISCONN: Self = Self(0x8001_0015);
    pub const ENOTCONN: Self = Self(0x8001_0016);
    pub const EAUTHFAIL: Self = Self(0x8001_0017);
    pub const ENOTMSELF: Self = Self(0x8001_0018);
    pub const ESYSVER: Self = Self(0x8001_0019);
    pub const EAUTHFATAL: Self = Self(0x8001_001A);
    pub const EDOM: Self = Self(0x8001_001B);
    pub const ERANGE: Self = Self(0x8001_001C);
    pub const EILSEQ: Self = Self(0x8001_001D);
    pub const EFPOS: Self = Self(0x8001_001E);
    pub const EINTR: Self = Self(0x8001_001F);
    pub const EFBIG: Self = Self(0x8001_0020);
    pub const EMLINK: Self = Self(0x8001_0021);
    pub const ENFILE: Self = Self(0x8001_0022);
    pub const ENOSPC: Self = Self(0x8001_0023);
    pub const ENOTTY: Self = Self(0x8001_0024);
    pub const EPIPE: Self = Self(0x8001_0025);
    pub const EROFS: Self = Self(0x8001_0026);
    pub const ESPIPE: Self = Self(0x8001_0027);
    pub const E2BIG: Self = Self(0x8001_0028);
    pub const EACCES: Self = Self(0x8001_0029);
    pub const EBADF: Self = Self(0x8001_002A);
    pub const EIO: Self = Self(0x8001_002B);
    pub const EMFILE: Self = Self(0x8001_002C);
    pub const ENODEV: Self = Self(0x8001_002D);
    pub const ENOTDIR: Self = Self(0x8001_002E);
    pub const ENXIO: Self = Self(0x8001_002F);
    pub const EXDEV: Self = Self(0x8001_0030);
    pub const EBADMSG: Self = Self(0x8001_0031);
    pub const EINPROGRESS: Self = Self(0x8001_0032);
    pub const EMSGSIZE: Self = Self(0x8001_0033);
    pub const ENAMETOOLONG: Self = Self(0x8001_0034);
    pub const ENOLCK: Self = Self(0x8001_0035);
    pub const ENOTEMPTY: Self = Self(0x8001_0036);
    pub const EUNSUP: Self = Self(0x8001_0037);

    #[must_use]
    pub const fn is_ok(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn is_error(self) -> bool {
        self.0 != 0
    }
}

impl From<u32> for CellError {
    fn from(v: u32) -> Self {
        Self(v)
    }
}

impl From<CellError> for u32 {
    fn from(c: CellError) -> Self {
        c.0
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- GameBootResult --------------------------------------------

    #[test]
    fn game_boot_result_ordinals_frozen() {
        assert_eq!(GameBootResult::NoErrors as u32, 0);
        assert_eq!(GameBootResult::GenericError as u32, 1);
        assert_eq!(GameBootResult::DecryptionError as u32, 7);
        assert_eq!(GameBootResult::StillRunning as u32, 14);
        assert_eq!(GameBootResult::DatabaseConfigMissing as u32, 17);
    }

    #[test]
    fn game_boot_result_is_error_contract() {
        assert!(!GameBootResult::NoErrors.is_error());
        assert!(GameBootResult::GenericError.is_error());
        assert!(GameBootResult::DatabaseConfigMissing.is_error());
    }

    #[test]
    fn game_boot_result_all_have_messages() {
        // Enumerate all 18 variants; every one must have a non-panicking
        // as_str(). Iterating enum by hand since Rust doesn't give us
        // that for free.
        let all = [
            GameBootResult::NoErrors,
            GameBootResult::GenericError,
            GameBootResult::NothingToBoot,
            GameBootResult::WrongDiscLocation,
            GameBootResult::InvalidFileOrFolder,
            GameBootResult::InvalidBdvdFolder,
            GameBootResult::InstallFailed,
            GameBootResult::DecryptionError,
            GameBootResult::FileCreationError,
            GameBootResult::FirmwareMissing,
            GameBootResult::FirmwareVersion,
            GameBootResult::UnsupportedDiscType,
            GameBootResult::SavestateCorrupted,
            GameBootResult::SavestateVersionUnsupported,
            GameBootResult::StillRunning,
            GameBootResult::AlreadyAdded,
            GameBootResult::CurrentlyRestricted,
            GameBootResult::DatabaseConfigMissing,
        ];
        assert_eq!(all.len(), 18);
        for v in all {
            let _s = v.as_str(); // must not panic
        }
    }

    // -- SystemState -----------------------------------------------

    #[test]
    fn system_state_ordinals_frozen() {
        assert_eq!(SystemState::Stopped as u32, 0);
        assert_eq!(SystemState::Loading as u32, 1);
        assert_eq!(SystemState::Stopping as u32, 2);
        assert_eq!(SystemState::Running as u32, 3);
        assert_eq!(SystemState::Paused as u32, 4);
        assert_eq!(SystemState::Frozen as u32, 5);
        assert_eq!(SystemState::Ready as u32, 6);
        assert_eq!(SystemState::Starting as u32, 7);
    }

    // -- CpuFlag ---------------------------------------------------

    #[test]
    fn cpu_flag_full_ordinals_frozen() {
        // Anchor every value against rpcs3/Emu/CPU/CPUThread.h:14-38.
        assert_eq!(CpuFlag::Stop as u32, 0);
        assert_eq!(CpuFlag::Exit as u32, 1);
        assert_eq!(CpuFlag::Wait as u32, 2);
        assert_eq!(CpuFlag::Temp as u32, 3);
        assert_eq!(CpuFlag::Pause as u32, 4);
        assert_eq!(CpuFlag::Suspend as u32, 5);
        assert_eq!(CpuFlag::Ret as u32, 6);
        assert_eq!(CpuFlag::Again as u32, 7);
        assert_eq!(CpuFlag::Signal as u32, 8);
        assert_eq!(CpuFlag::Memory as u32, 9);
        assert_eq!(CpuFlag::Pending as u32, 10);
        assert_eq!(CpuFlag::PendingRecheck as u32, 11);
        assert_eq!(CpuFlag::Notify as u32, 12);
        assert_eq!(CpuFlag::Yield as u32, 13);
        assert_eq!(CpuFlag::Preempt as u32, 14);
        assert_eq!(CpuFlag::ReqExit as u32, 15);
        assert_eq!(CpuFlag::DbgGlobalPause as u32, 16);
        assert_eq!(CpuFlag::DbgPause as u32, 17);
        assert_eq!(CpuFlag::DbgStep as u32, 18);
    }

    #[test]
    fn cpu_flag_mask_is_shift() {
        assert_eq!(CpuFlag::Stop.mask(), 0x0000_0001);
        assert_eq!(CpuFlag::Exit.mask(), 0x0000_0002);
        assert_eq!(CpuFlag::Wait.mask(), 0x0000_0004);
        assert_eq!(CpuFlag::Temp.mask(), 0x0000_0008);
        assert_eq!(CpuFlag::Pause.mask(), 0x0000_0010);
        assert_eq!(CpuFlag::Suspend.mask(), 0x0000_0020);
        assert_eq!(CpuFlag::Yield.mask(), 0x0000_2000);
        assert_eq!(CpuFlag::DbgStep.mask(), 0x0004_0000);
    }

    #[test]
    fn is_stopped_matches_c_helper() {
        // stop | exit | again | req_exit → stopped
        assert!(is_stopped(CpuFlag::Stop.mask()));
        assert!(is_stopped(CpuFlag::Exit.mask()));
        assert!(is_stopped(CpuFlag::Again.mask()));
        assert!(is_stopped(CpuFlag::ReqExit.mask()));
        assert!(is_stopped(CpuFlag::Stop.mask() | CpuFlag::Wait.mask()));

        // pause/suspend/dbg_* alone are NOT stopped
        assert!(!is_stopped(CpuFlag::Pause.mask()));
        assert!(!is_stopped(CpuFlag::Suspend.mask()));
        assert!(!is_stopped(CpuFlag::DbgPause.mask()));
        assert!(!is_stopped(CpuFlag::Wait.mask()));
        assert!(!is_stopped(0));
    }

    #[test]
    fn is_paused_matches_c_helper() {
        // suspend | dbg_global_pause | dbg_pause → paused, IF not stopped
        assert!(is_paused(CpuFlag::Suspend.mask()));
        assert!(is_paused(CpuFlag::DbgGlobalPause.mask()));
        assert!(is_paused(CpuFlag::DbgPause.mask()));

        // stopped dominates paused
        assert!(!is_paused(CpuFlag::Suspend.mask() | CpuFlag::Exit.mask()));
        assert!(!is_paused(CpuFlag::DbgPause.mask() | CpuFlag::Stop.mask()));

        // Pause flag alone is NOT the same as Suspend — `is_paused` looks
        // at suspend/dbg_* only. Matches CPUThread.h:49 exactly.
        assert!(!is_paused(CpuFlag::Pause.mask()));
        assert!(!is_paused(0));
    }

    // -- CellError -------------------------------------------------

    #[test]
    fn cell_error_ok_is_zero() {
        assert_eq!(CellError::OK.0, 0);
        assert!(CellError::OK.is_ok());
        assert!(!CellError::OK.is_error());
    }

    #[test]
    fn cell_error_enoent_value_is_frozen() {
        assert_eq!(CellError::ENOENT.0, 0x8001_0006);
        assert!(CellError::ENOENT.is_error());
    }

    #[test]
    fn cell_error_all_have_high_bit() {
        // Every non-OK error must carry the 0x80010000 "Cell facility" bit.
        let errors = [
            CellError::EAGAIN,
            CellError::EINVAL,
            CellError::ENOMEM,
            CellError::ENOENT,
            CellError::EACCES,
            CellError::EIO,
            CellError::EISDIR,
            CellError::ENOTDIR,
            CellError::EEXIST,
            CellError::ENOTEMPTY,
        ];
        for e in errors {
            assert_eq!(e.0 & 0xFFFF_0000, 0x8001_0000, "error {:?} missing facility bit", e);
        }
    }

    #[test]
    fn cell_error_round_trip_u32() {
        let e = CellError::EBADF;
        let raw: u32 = e.into();
        assert_eq!(raw, 0x8001_002A);
        let back: CellError = raw.into();
        assert_eq!(back, CellError::EBADF);
    }

    #[test]
    fn cell_error_ordering_is_by_raw_value() {
        // BTreeMap/sort order must be stable.
        let mut errs = vec![CellError::ENOENT, CellError::OK, CellError::EIO, CellError::EACCES];
        errs.sort();
        assert_eq!(
            errs,
            vec![
                CellError::OK,
                CellError::ENOENT,
                CellError::EACCES,
                CellError::EIO,
            ]
        );
    }
}
