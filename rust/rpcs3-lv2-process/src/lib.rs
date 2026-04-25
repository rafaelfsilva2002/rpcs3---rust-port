//! `rpcs3-lv2-process` — process-level LV2 syscalls.
//!
//! Ports `rpcs3/Emu/Cell/lv2/sys_process.cpp` — the family of
//! `sys_process_*` syscalls that a PS3 title calls early in boot.
//!
//! ## What this crate does
//!
//! Provides pure-ish Rust functions that implement each syscall. They
//! do not touch threads directly; instead, exit-class syscalls return
//! a [`SyscallResult::Exit`] variant that the caller (the emulator
//! core) dispatches by stopping the current `ppu_thread`.
//!
//! ## Iteration scope
//!
//! * `sys_process_getpid` → always 1 (matches C++ at sys_process.cpp:68-72)
//! * `sys_process_getppid` → always 0 (sys_process.cpp:74-78)
//! * `sys_process_get_sdk_version` → reads global SDK version, writes to
//!   guest pointer (placeholder: caller must wire up the write)
//! * `sys_process_exit3`, `_sys_process_exit`, `_sys_process_exit2`
//!   → request process termination with given status
//! * `sys_process_get_number_of_object` → enum-dispatched object count
//!   (stub implementation; counts are plugged in by the caller)
//!
//! ## Object type constants
//!
//! `SYS_*_OBJECT` values from `sys_process.cpp:90-111`. Frozen here
//! because games pass them as literal u32s — reordering breaks.

use rpcs3_emu_types::CellError;

// =====================================================================
// Object type identifiers (sys_process_get_number_of_object / get_id)
// =====================================================================

/// Object-type IDs passed to `sys_process_get_number_of_object` and
/// `sys_process_get_id`. Values anchored against `sys_process.cpp`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectType {
    Mem = 0x08,
    Mutex = 0x85,
    Cond = 0x86,
    RwLock = 0x88,
    IntrTag = 0x0A,
    IntrServiceHandle = 0x0B,
    EventQueue = 0x8D,
    EventPort = 0x0E,
    Trace = 0x21,
    SpuImage = 0x22,
    Prx = 0x23,
    SpuPort = 0x24,
    LwMutex = 0x95,
    Timer = 0x11,
    Semaphore = 0x96,
    FsFd = 0x73,
    LwCond = 0x97,
    EventFlag = 0x98,
    Overlay = 0x9F,
}

impl ObjectType {
    /// Parse a raw u32 from guest code. Unknown values return `None`,
    /// which the caller reports as `CELL_EINVAL`.
    #[must_use]
    pub fn from_u32(v: u32) -> Option<Self> {
        Some(match v {
            0x08 => Self::Mem,
            0x85 => Self::Mutex,
            0x86 => Self::Cond,
            0x88 => Self::RwLock,
            0x0A => Self::IntrTag,
            0x0B => Self::IntrServiceHandle,
            0x8D => Self::EventQueue,
            0x0E => Self::EventPort,
            0x21 => Self::Trace,
            0x22 => Self::SpuImage,
            0x23 => Self::Prx,
            0x24 => Self::SpuPort,
            0x95 => Self::LwMutex,
            0x11 => Self::Timer,
            0x96 => Self::Semaphore,
            0x73 => Self::FsFd,
            0x97 => Self::LwCond,
            0x98 => Self::EventFlag,
            0x9F => Self::Overlay,
            _ => return None,
        })
    }
}

// =====================================================================
// Syscall return type
// =====================================================================

/// Uniform return type for all syscalls in this crate.
///
/// * `Ok(r0_value)` — syscall returned normally with this value as r0.
/// * `Err(cell)` — syscall returned a Cell error (set r0 = code).
/// * `Exit { status }` — process is requesting termination. The emu
///   core must stop all threads and return `status` to the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyscallResult {
    Ok(u64),
    Err(CellError),
    Exit { status: i32 },
}

impl SyscallResult {
    #[must_use]
    pub fn ok_u64(v: u64) -> Self {
        Self::Ok(v)
    }
    #[must_use]
    pub fn ok_s32(v: i32) -> Self {
        Self::Ok(v as i64 as u64)
    }
    #[must_use]
    pub fn err(e: CellError) -> Self {
        Self::Err(e)
    }
    #[must_use]
    pub fn exit(status: i32) -> Self {
        Self::Exit { status }
    }
}

// =====================================================================
// Out-of-crate state (injected by the emu core)
// =====================================================================

/// Information the process syscalls need that lives outside this
/// crate — plugged in by the emulator core.
pub trait ProcessState {
    /// Get the current process id (always 1 in RPCS3 — see
    /// `process_getpid()` at sys_process.cpp:62-66).
    fn pid(&self) -> i32 {
        1
    }

    /// Get the parent PID (always 0).
    fn ppid(&self) -> i32 {
        0
    }

    /// SDK version reported to the game. C++ reads from
    /// `g_ps3_process_info.sdk_ver`; emulators typically seed this
    /// from the ELF's `sys_process_param` segment.
    fn sdk_version(&self) -> u32;

    /// Count objects of the given type. The emu core walks the IDM
    /// for the requested object family; we stub it by returning 0
    /// when the caller hasn't overridden this method.
    fn object_count(&self, _kind: ObjectType) -> u32 {
        0
    }
}

/// Zero-state stub useful for tests that don't need a full emu core.
#[derive(Debug, Default, Clone)]
pub struct TestProcessState {
    pub sdk_version: u32,
    pub overrides: std::collections::HashMap<ObjectType, u32>,
}

impl ProcessState for TestProcessState {
    fn sdk_version(&self) -> u32 {
        self.sdk_version
    }
    fn object_count(&self, kind: ObjectType) -> u32 {
        self.overrides.get(&kind).copied().unwrap_or(0)
    }
}

// =====================================================================
// Syscalls
// =====================================================================

/// `sys_process_getpid()` → PID. Mirrors sys_process.cpp:68.
#[must_use]
pub fn sys_process_getpid<S: ProcessState + ?Sized>(state: &S) -> SyscallResult {
    SyscallResult::ok_s32(state.pid())
}

/// `sys_process_getppid()` → parent PID. Mirrors sys_process.cpp:74.
#[must_use]
pub fn sys_process_getppid<S: ProcessState + ?Sized>(state: &S) -> SyscallResult {
    SyscallResult::ok_s32(state.ppid())
}

/// `sys_process_get_sdk_version(pid, version_out)`.
///
/// Mirrors sys_process.cpp:293. Returns the SDK version through
/// `version_out` in the guest address space; since this crate doesn't
/// own memory, we return the value and let the caller write it.
#[must_use]
pub fn sys_process_get_sdk_version<S: ProcessState + ?Sized>(
    state: &S,
    pid: u32,
) -> Result<u32, CellError> {
    // RPCS3 accepts any pid and returns the current process's SDK
    // version (process_getpid() always returns 1, and the global
    // g_ps3_process_info.sdk_ver is the only one tracked).
    let _ = pid;
    Ok(state.sdk_version())
}

/// `_sys_process_exit(status, arg2, arg3)` — request process exit.
/// Mirrors sys_process.cpp:345. `arg2`/`arg3` are currently ignored
/// by RPCS3 (marked TODO there); we preserve the signature for ABI.
#[must_use]
pub fn _sys_process_exit(status: i32, _arg2: u32, _arg3: u32) -> SyscallResult {
    SyscallResult::exit(status)
}

/// `sys_process_exit3(status)` — thin wrapper over _sys_process_exit.
/// Mirrors sys_process.cpp:519 (calls `_sys_process_exit(status, 0, 0)`).
#[must_use]
pub fn sys_process_exit3(status: i32) -> SyscallResult {
    _sys_process_exit(status, 0, 0)
}

/// `_sys_process_exit2(status, arg_ptr, arg_size, arg4)`. Same effect
/// as `_sys_process_exit` for our purposes; the extended args are
/// passed through to debugger hooks in the C++ code.
#[must_use]
pub fn _sys_process_exit2(status: i32, _arg_ptr: u32, _arg_size: u32, _arg4: u32) -> SyscallResult {
    SyscallResult::exit(status)
}

/// `sys_process_get_number_of_object(object, nump_out)`.
/// Mirrors sys_process.cpp:86. Returns the count (in-value) and an
/// error code; caller writes the count to the guest pointer.
#[must_use]
pub fn sys_process_get_number_of_object<S: ProcessState + ?Sized>(
    state: &S,
    object: u32,
) -> Result<u32, CellError> {
    let Some(kind) = ObjectType::from_u32(object) else {
        return Err(CellError::EINVAL);
    };
    Ok(state.object_count(kind))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- getpid / getppid ------------------------------------------

    #[test]
    fn getpid_returns_one() {
        let s = TestProcessState::default();
        assert_eq!(sys_process_getpid(&s), SyscallResult::ok_s32(1));
    }

    #[test]
    fn getppid_returns_zero() {
        let s = TestProcessState::default();
        assert_eq!(sys_process_getppid(&s), SyscallResult::ok_s32(0));
    }

    // -- get_sdk_version -------------------------------------------

    #[test]
    fn get_sdk_version_returns_state_value() {
        let s = TestProcessState { sdk_version: 0x00340000, ..Default::default() };
        assert_eq!(sys_process_get_sdk_version(&s, 1), Ok(0x00340000));
    }

    #[test]
    fn get_sdk_version_ignores_pid_parameter() {
        // C++ uses the global regardless of the pid argument.
        let s = TestProcessState { sdk_version: 0x00360000, ..Default::default() };
        assert_eq!(sys_process_get_sdk_version(&s, 0xDEAD), Ok(0x00360000));
        assert_eq!(sys_process_get_sdk_version(&s, 0), Ok(0x00360000));
    }

    // -- exit family ----------------------------------------------

    #[test]
    fn exit3_propagates_status() {
        match sys_process_exit3(42) {
            SyscallResult::Exit { status } => assert_eq!(status, 42),
            other => panic!("expected Exit, got {other:?}"),
        }
    }

    #[test]
    fn exit3_with_negative_status() {
        assert_eq!(sys_process_exit3(-1), SyscallResult::Exit { status: -1 });
    }

    #[test]
    fn underscore_exit_passes_through_args() {
        // arg2/arg3 are currently no-ops in RPCS3 — must still accept them.
        assert_eq!(_sys_process_exit(7, 0xDEAD, 0xBEEF), SyscallResult::Exit { status: 7 });
    }

    #[test]
    fn underscore_exit2_matches_exit() {
        // Semantically equivalent for our purposes.
        assert_eq!(_sys_process_exit2(3, 0, 0, 0), SyscallResult::Exit { status: 3 });
    }

    // -- ObjectType mapping ----------------------------------------

    #[test]
    fn object_type_known_values() {
        assert_eq!(ObjectType::from_u32(0x08), Some(ObjectType::Mem));
        assert_eq!(ObjectType::from_u32(0x85), Some(ObjectType::Mutex));
        assert_eq!(ObjectType::from_u32(0x73), Some(ObjectType::FsFd));
        assert_eq!(ObjectType::from_u32(0x9F), Some(ObjectType::Overlay));
    }

    #[test]
    fn object_type_unknown_is_none() {
        assert!(ObjectType::from_u32(0xFFFF).is_none());
        assert!(ObjectType::from_u32(0).is_none());
    }

    #[test]
    fn object_type_repr_values_are_frozen() {
        assert_eq!(ObjectType::Mem as u32, 0x08);
        assert_eq!(ObjectType::Mutex as u32, 0x85);
        assert_eq!(ObjectType::FsFd as u32, 0x73);
        assert_eq!(ObjectType::EventFlag as u32, 0x98);
    }

    // -- get_number_of_object -------------------------------------

    #[test]
    fn get_number_of_object_returns_state_count() {
        let mut s = TestProcessState::default();
        s.overrides.insert(ObjectType::Mem, 7);
        s.overrides.insert(ObjectType::Mutex, 3);
        assert_eq!(sys_process_get_number_of_object(&s, 0x08), Ok(7));
        assert_eq!(sys_process_get_number_of_object(&s, 0x85), Ok(3));
        // Unspecified types default to 0.
        assert_eq!(sys_process_get_number_of_object(&s, 0x86), Ok(0));
    }

    #[test]
    fn get_number_of_object_invalid_kind_is_einval() {
        let s = TestProcessState::default();
        assert_eq!(
            sys_process_get_number_of_object(&s, 0xFFFF),
            Err(CellError::EINVAL)
        );
    }

    // -- SyscallResult helpers -------------------------------------

    #[test]
    fn syscall_result_ok_s32_extends_correctly() {
        // -1 as s32 should encode as u64::MAX (sign-extended).
        assert_eq!(SyscallResult::ok_s32(-1), SyscallResult::Ok(u64::MAX));
        assert_eq!(SyscallResult::ok_s32(42), SyscallResult::Ok(42));
    }
}
