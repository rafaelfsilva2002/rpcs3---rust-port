//! Rust port of `cellFsSdata*` entry points from
//! `rpcs3/Emu/Cell/Modules/cellFs.cpp`.
//!
//! SDATA is a Sony-specific encrypted container layered on top of MSELF or a
//! plain file descriptor.  The HLE surface exposes three public functions:
//!
//! * [`cell_fs_sdata_open`]          — opens a path in read-only mode and
//!   delegates to `cellFsOpen` with a 2-word SDATA header argument.
//! * [`cell_fs_sdata_open_by_fd`]   — opens an SDATA file that is already
//!   inside an MSELF container (identified by the PPU file descriptor
//!   `mself_fd`) at the requested byte `offset`.
//! * [`cell_fs_sdata_open_with_version`] — stub (C++ returns `CELL_OK` after a
//!   "unimplemented" log line).
//!
//! The real C++ implementation of `cellFsSdataOpenByFd` serialises an
//! `lv2_file_op_09` control block and issues `sys_fs_fcntl(mself_fd,
//! 0x80000009, ctrl, 0x40)` against the `sys_fs` LV2 service.  In Rust we
//! mirror the validation and field layout so upper layers can drive the same
//! syscall without pulling in the fs backend.

use rpcs3_emu_types::CellError;

// ---------------------------------------------------------------------------
// Constants (byte-exact vs cellFs.cpp)
// ---------------------------------------------------------------------------

/// Only flag accepted by `cellFsSdataOpen`.  `CELL_FS_O_RDONLY` is `0` in the
/// original header (`rpcs3/Emu/Cell/lv2/sys_fs.h`).
pub const CELL_FS_O_RDONLY: i32 = 0o0;

/// First word of the SDATA header argument passed to `cellFsOpen` for
/// `cellFsSdataOpen`.
pub const SDATA_HEADER_ARG1: u32 = 0x180;

/// Second word of the SDATA header argument passed to `cellFsOpen` for
/// `cellFsSdataOpen`.
pub const SDATA_HEADER_ARG2: u32 = 0x10;

/// Size (in bytes) of the SDATA header argument blob — two 32-bit big-endian
/// words.
pub const SDATA_HEADER_ARG_SIZE: u64 = 8;

/// `sys_fs_fcntl` op code used by `cellFsSdataOpenByFd`.
pub const LV2_FILE_OP_SDATA_OPEN_BY_FD: u32 = 0x8000_0009;

/// Byte size of the `lv2_file_op_09` control block consumed by the fcntl call.
pub const LV2_FILE_OP_09_SIZE: u32 = 0x40;

/// Intentionally "wrong" vtable that the C++ port sets inside the ctrl block
/// (see comment in `cellFs.cpp` around the `_vtable` assignment).
pub const SDATA_VTABLE1: u32 = 0xFA88_0000;

/// Second "vtable"-shaped pointer stored inside the ctrl block.
pub const SDATA_VTABLE2: u32 = 0xFA88_0020;

/// Inclusive lower bound for the MSELF file descriptor — descriptors 0..=2
/// collide with the PPU standard streams.
pub const MSELF_FD_MIN: u32 = 3;

/// Inclusive upper bound for the MSELF file descriptor.
pub const MSELF_FD_MAX: u32 = 255;

/// Sentinel the C++ side writes into `*sdata_fd` when the call fails *after*
/// the initial `!sdata_fd` check but *before* success.  Represented here as
/// the two's-complement of `-1` in a 32-bit register.
pub const SDATA_FD_INVALID: u32 = 0xFFFF_FFFF;

// ---------------------------------------------------------------------------
// SDATA control block (mirror of `lv2_file_op_09`)
// ---------------------------------------------------------------------------

/// Control block assembled by `cellFsSdataOpenByFd` and passed to the
/// `sys_fs_fcntl` opcode `0x80000009`.
///
/// The layout mirrors `lv2_file_op_09` in
/// `rpcs3/Emu/Cell/lv2/sys_fs.h`: two "vtable" addresses sandwiching the
/// operation metadata, then argument bytes, then the out-parameters that
/// `sys_fs` fills in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SdataCtrl {
    pub vtable1: u32,
    pub op: u32,
    pub fd: u32,
    pub offset: u64,
    pub vtable2: u32,
    pub arg1: u32,
    pub arg2: u32,
    pub arg_ptr: u32,
    pub arg_size: u32,
    pub out_code: i32,
    pub out_fd: u32,
}

impl SdataCtrl {
    /// Build the ctrl block exactly as the C++ side populates it *before*
    /// dispatching the fcntl call.
    #[must_use]
    pub fn new(mself_fd: u32, offset: u64, arg_ptr: u32, arg_size: u32) -> Self {
        Self {
            vtable1: SDATA_VTABLE1,
            op: LV2_FILE_OP_SDATA_OPEN_BY_FD,
            fd: mself_fd,
            offset,
            vtable2: SDATA_VTABLE2,
            arg1: SDATA_HEADER_ARG1,
            arg2: SDATA_HEADER_ARG2,
            arg_ptr,
            arg_size,
            out_code: 0,
            out_fd: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// `cellFsSdataOpen`
// ---------------------------------------------------------------------------

/// Request produced by [`cell_fs_sdata_open`].  Callers are expected to
/// forward it to `cellFsOpen` with the embedded flag, header argument, and
/// argument size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SdataOpenRequest {
    pub flags: i32,
    pub arg1: u32,
    pub arg2: u32,
    pub arg_size: u64,
}

impl Default for SdataOpenRequest {
    fn default() -> Self {
        Self {
            flags: CELL_FS_O_RDONLY,
            arg1: SDATA_HEADER_ARG1,
            arg2: SDATA_HEADER_ARG2,
            arg_size: SDATA_HEADER_ARG_SIZE,
        }
    }
}

/// Validate arguments for `cellFsSdataOpen` and return the follow-up
/// `cellFsOpen` request that the caller should dispatch.
///
/// Ported from [cellFs.cpp:45-55]:
/// ```cpp
/// if (flags != CELL_FS_O_RDONLY) return CELL_EINVAL;
/// return cellFsOpen(ppu, path, CELL_FS_O_RDONLY, fd,
///                   vm::make_var<be_t<u32>[2]>({0x180, 0x10}), 8);
/// ```
///
/// # Errors
/// * [`CellError::EINVAL`] if `flags != CELL_FS_O_RDONLY`.
pub fn cell_fs_sdata_open(flags: i32) -> Result<SdataOpenRequest, CellError> {
    if flags != CELL_FS_O_RDONLY {
        return Err(CellError::EINVAL);
    }
    Ok(SdataOpenRequest::default())
}

// ---------------------------------------------------------------------------
// `cellFsSdataOpenByFd`
// ---------------------------------------------------------------------------

/// Planned fcntl invocation produced by [`cell_fs_sdata_open_by_fd`].
///
/// The caller pushes `ctrl` to LV2 via
/// `sys_fs_fcntl(mself_fd, op, &ctrl, ctrl_size)` — the arguments are
/// precomputed here so the surface remains testable without touching the real
/// syscall stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdataOpenByFdPlan {
    pub mself_fd: u32,
    pub op: u32,
    pub ctrl: SdataCtrl,
    pub ctrl_size: u32,
}

/// Argument-validation half of `cellFsSdataOpenByFd` — produces the fcntl
/// plan that the caller must execute.  The post-fcntl fixup (reading
/// `ctrl.out_code`/`ctrl.out_fd`) lives in
/// [`finish_sdata_open_by_fd`].
///
/// Ported from [cellFs.cpp:526-569].
///
/// # Errors
/// * [`CellError::EFAULT`] if the caller passes a null `sdata_fd`
///   destination (modelled by `has_sdata_fd = false`).
/// * [`CellError::EBADF`]  if `mself_fd` is outside `3..=255`.
/// * [`CellError::EINVAL`] if any flag bit is set (C++ side only accepts
///   `flags == 0`).
pub fn cell_fs_sdata_open_by_fd(
    has_sdata_fd: bool,
    mself_fd: u32,
    flags: i32,
    offset: u64,
    arg_ptr: u32,
    arg_size: u64,
) -> Result<SdataOpenByFdPlan, CellError> {
    if !has_sdata_fd {
        return Err(CellError::EFAULT);
    }
    if !(MSELF_FD_MIN..=MSELF_FD_MAX).contains(&mself_fd) {
        return Err(CellError::EBADF);
    }
    if flags != 0 {
        return Err(CellError::EINVAL);
    }

    let arg_size_u32 = u32::try_from(arg_size).unwrap_or(u32::MAX);
    Ok(SdataOpenByFdPlan {
        mself_fd,
        op: LV2_FILE_OP_SDATA_OPEN_BY_FD,
        ctrl: SdataCtrl::new(mself_fd, offset, arg_ptr, arg_size_u32),
        ctrl_size: LV2_FILE_OP_09_SIZE,
    })
}

/// Outcome of `sys_fs_fcntl` translated to the surface the caller exposes as
/// `cellFsSdataOpenByFd`.
///
/// The C++ side distinguishes three paths:
///
/// 1. `fcntl` returns a non-zero error → return `not_an_error(rc)` (the
///    original errno is surfaced verbatim, **not** wrapped as a
///    `CellError`).
/// 2. `fcntl` returns 0 but `ctrl.out_code` is non-zero → return
///    `CellError(ctrl.out_code)`.
/// 3. Both are zero → write `ctrl.out_fd` into `*sdata_fd` and return
///    `CELL_OK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SdataOpenByFdOutcome {
    /// fcntl itself failed; `rc` is the raw syscall errno from
    /// `sys_fs_fcntl`, surfaced to the guest unchanged.
    FcntlError(i32),
    /// fcntl succeeded but the control block reported a failure; `code` is
    /// the encoded `CellError`.
    CtrlError(CellError),
    /// Happy path — caller should write `sdata_fd` into `*sdata_fd`.
    Opened { sdata_fd: u32 },
}

/// Post-fcntl fixup for `cellFsSdataOpenByFd`.  Reads `ctrl.out_code`/
/// `ctrl.out_fd` from the ctrl block that LV2 filled in and returns the
/// outcome the guest should observe.
#[must_use]
pub fn finish_sdata_open_by_fd(fcntl_rc: i32, ctrl: &SdataCtrl) -> SdataOpenByFdOutcome {
    if fcntl_rc != 0 {
        return SdataOpenByFdOutcome::FcntlError(fcntl_rc);
    }
    if ctrl.out_code != 0 {
        return SdataOpenByFdOutcome::CtrlError(CellError(ctrl.out_code as u32));
    }
    SdataOpenByFdOutcome::Opened { sdata_fd: ctrl.out_fd }
}

// ---------------------------------------------------------------------------
// `cellFsSdataOpenWithVersion`
// ---------------------------------------------------------------------------

/// `cellFsSdataOpenWithVersion` is marked `UNIMPLEMENTED_FUNC` in the C++
/// source and returns `CELL_OK` after a stub log line.  We preserve the exact
/// observable behaviour.
#[must_use]
pub fn cell_fs_sdata_open_with_version() -> Result<(), CellError> {
    Ok(())
}

// ---------------------------------------------------------------------------
// Small in-memory fd registry — mirrors the cellFs fd-table pattern so tests
// can exercise full open/close cycles without touching the real VFS.
// ---------------------------------------------------------------------------

/// Very small handle registry.  The real `sys_fs` layer owns the fd table;
/// this helper exists so tests (and unit exercises of upper layers) can drive
/// the port end-to-end without LV2.
#[derive(Debug, Default)]
pub struct SdataFdRegistry {
    entries: Vec<SdataFdEntry>,
    next: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SdataFdEntry {
    pub sdata_fd: u32,
    pub mself_fd: u32,
    pub offset: u64,
    pub closed: bool,
}

impl SdataFdRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self { entries: Vec::new(), next: 3 }
    }

    /// Allocate an fd for the outcome of an `OpenByFd` plan.
    ///
    /// # Errors
    /// * [`CellError::ENOMEM`] if the internal counter cannot advance.
    pub fn register(&mut self, mself_fd: u32, offset: u64) -> Result<SdataFdEntry, CellError> {
        let sdata_fd = self.next;
        self.next = self.next.checked_add(1).ok_or(CellError::ENOMEM)?;
        let entry = SdataFdEntry { sdata_fd, mself_fd, offset, closed: false };
        self.entries.push(entry);
        Ok(entry)
    }

    #[must_use]
    pub fn get(&self, sdata_fd: u32) -> Option<&SdataFdEntry> {
        self.entries.iter().find(|e| e.sdata_fd == sdata_fd)
    }

    /// Close an outstanding fd.
    ///
    /// # Errors
    /// * [`CellError::EBADF`] if `sdata_fd` was never registered.
    /// * [`CellError::EBADF`] if the fd is already closed (matches sys_fs).
    pub fn close(&mut self, sdata_fd: u32) -> Result<(), CellError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|e| e.sdata_fd == sdata_fd)
            .ok_or(CellError::EBADF)?;
        if entry.closed {
            return Err(CellError::EBADF);
        }
        entry.closed = true;
        Ok(())
    }

    #[must_use]
    pub fn open_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.closed).count()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants --------------------------------------------------------

    #[test]
    fn header_constants_byte_exact() {
        assert_eq!(CELL_FS_O_RDONLY, 0);
        assert_eq!(SDATA_HEADER_ARG1, 0x180);
        assert_eq!(SDATA_HEADER_ARG2, 0x10);
        assert_eq!(SDATA_HEADER_ARG_SIZE, 8);
    }

    #[test]
    fn fcntl_constants_byte_exact() {
        assert_eq!(LV2_FILE_OP_SDATA_OPEN_BY_FD, 0x8000_0009);
        assert_eq!(LV2_FILE_OP_09_SIZE, 0x40);
        assert_eq!(SDATA_VTABLE1, 0xFA88_0000);
        assert_eq!(SDATA_VTABLE2, 0xFA88_0020);
    }

    #[test]
    fn mself_fd_range_matches_cpp() {
        assert_eq!(MSELF_FD_MIN, 3);
        assert_eq!(MSELF_FD_MAX, 255);
    }

    #[test]
    fn sdata_fd_invalid_sentinel_matches_cpp() {
        assert_eq!(SDATA_FD_INVALID, 0xFFFF_FFFF);
        // `*sdata_fd = -1;` in C++ sign-extends to 32-bit, so unsigned 0xFFFF_FFFF.
        assert_eq!(SDATA_FD_INVALID as i32, -1);
    }

    #[test]
    fn cell_error_codes_byte_exact() {
        assert_eq!(CellError::EINVAL.0, 0x8001_0002);
        assert_eq!(CellError::EFAULT.0, 0x8001_000D);
        assert_eq!(CellError::EBADF.0, 0x8001_002A);
    }

    // ---- cellFsSdataOpen --------------------------------------------------

    #[test]
    fn sdata_open_happy_path_builds_default_request() {
        let req = cell_fs_sdata_open(CELL_FS_O_RDONLY).unwrap();
        assert_eq!(req.flags, 0);
        assert_eq!(req.arg1, 0x180);
        assert_eq!(req.arg2, 0x10);
        assert_eq!(req.arg_size, 8);
    }

    #[test]
    fn sdata_open_rejects_write_flag() {
        assert_eq!(cell_fs_sdata_open(0o1).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn sdata_open_rejects_rdwr_flag() {
        assert_eq!(cell_fs_sdata_open(0o2).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn sdata_open_rejects_negative_flag() {
        assert_eq!(cell_fs_sdata_open(-1).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn sdata_open_rejects_create_flag() {
        // CELL_FS_O_CREAT = 0o100 in sys_fs.h
        assert_eq!(cell_fs_sdata_open(0o100).unwrap_err(), CellError::EINVAL);
    }

    #[test]
    fn sdata_open_request_default_matches_cpp_vm_make_var() {
        let req = SdataOpenRequest::default();
        assert_eq!(req, SdataOpenRequest {
            flags: 0,
            arg1: 0x180,
            arg2: 0x10,
            arg_size: 8,
        });
    }

    // ---- cellFsSdataOpenByFd ---------------------------------------------

    #[test]
    fn open_by_fd_rejects_null_sdata_fd() {
        let err = cell_fs_sdata_open_by_fd(false, 10, 0, 0, 0x1000, 0).unwrap_err();
        assert_eq!(err, CellError::EFAULT);
    }

    #[test]
    fn open_by_fd_rejects_mself_fd_too_low() {
        // 0, 1, 2 all reserved for stdio.
        for fd in 0..=2u32 {
            let err = cell_fs_sdata_open_by_fd(true, fd, 0, 0, 0, 0).unwrap_err();
            assert_eq!(err, CellError::EBADF, "fd {fd} should be EBADF");
        }
    }

    #[test]
    fn open_by_fd_rejects_mself_fd_too_high() {
        let err = cell_fs_sdata_open_by_fd(true, 256, 0, 0, 0, 0).unwrap_err();
        assert_eq!(err, CellError::EBADF);
        let err = cell_fs_sdata_open_by_fd(true, u32::MAX, 0, 0, 0, 0).unwrap_err();
        assert_eq!(err, CellError::EBADF);
    }

    #[test]
    fn open_by_fd_accepts_boundary_fds() {
        assert!(cell_fs_sdata_open_by_fd(true, MSELF_FD_MIN, 0, 0, 0, 0).is_ok());
        assert!(cell_fs_sdata_open_by_fd(true, MSELF_FD_MAX, 0, 0, 0, 0).is_ok());
    }

    #[test]
    fn open_by_fd_rejects_nonzero_flags() {
        let err = cell_fs_sdata_open_by_fd(true, 5, 1, 0, 0, 0).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
        let err = cell_fs_sdata_open_by_fd(true, 5, 0o100, 0, 0, 0).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
        let err = cell_fs_sdata_open_by_fd(true, 5, -1, 0, 0, 0).unwrap_err();
        assert_eq!(err, CellError::EINVAL);
    }

    #[test]
    fn open_by_fd_happy_path_builds_ctrl_block() {
        let plan = cell_fs_sdata_open_by_fd(true, 7, 0, 0xDEAD_BEEF, 0x4000_0000, 64).unwrap();
        assert_eq!(plan.mself_fd, 7);
        assert_eq!(plan.op, LV2_FILE_OP_SDATA_OPEN_BY_FD);
        assert_eq!(plan.ctrl_size, LV2_FILE_OP_09_SIZE);
        assert_eq!(plan.ctrl.vtable1, SDATA_VTABLE1);
        assert_eq!(plan.ctrl.vtable2, SDATA_VTABLE2);
        assert_eq!(plan.ctrl.op, LV2_FILE_OP_SDATA_OPEN_BY_FD);
        assert_eq!(plan.ctrl.fd, 7);
        assert_eq!(plan.ctrl.offset, 0xDEAD_BEEF);
        assert_eq!(plan.ctrl.arg1, 0x180);
        assert_eq!(plan.ctrl.arg2, 0x10);
        assert_eq!(plan.ctrl.arg_ptr, 0x4000_0000);
        assert_eq!(plan.ctrl.arg_size, 64);
    }

    #[test]
    fn open_by_fd_truncates_huge_arg_size() {
        let plan = cell_fs_sdata_open_by_fd(true, 10, 0, 0, 0, u64::MAX).unwrap();
        assert_eq!(plan.ctrl.arg_size, u32::MAX);
    }

    #[test]
    fn open_by_fd_zero_offset_ok() {
        let plan = cell_fs_sdata_open_by_fd(true, 42, 0, 0, 0x1234_5678, 8).unwrap();
        assert_eq!(plan.ctrl.offset, 0);
    }

    // ---- validation ordering matches cellFs.cpp --------------------------

    #[test]
    fn open_by_fd_validation_order_null_then_fd_then_flags() {
        // null sdata_fd beats bad fd + bad flags
        assert_eq!(
            cell_fs_sdata_open_by_fd(false, 0, 1, 0, 0, 0).unwrap_err(),
            CellError::EFAULT,
        );
        // bad fd beats bad flags (flags=1 but fd=2 → EBADF not EINVAL)
        assert_eq!(
            cell_fs_sdata_open_by_fd(true, 2, 1, 0, 0, 0).unwrap_err(),
            CellError::EBADF,
        );
    }

    // ---- ctrl block ------------------------------------------------------

    #[test]
    fn ctrl_new_prepopulates_all_fields() {
        let ctrl = SdataCtrl::new(9, 0xABCD, 0x1000, 32);
        assert_eq!(ctrl.vtable1, SDATA_VTABLE1);
        assert_eq!(ctrl.vtable2, SDATA_VTABLE2);
        assert_eq!(ctrl.op, LV2_FILE_OP_SDATA_OPEN_BY_FD);
        assert_eq!(ctrl.fd, 9);
        assert_eq!(ctrl.offset, 0xABCD);
        assert_eq!(ctrl.arg1, 0x180);
        assert_eq!(ctrl.arg2, 0x10);
        assert_eq!(ctrl.arg_ptr, 0x1000);
        assert_eq!(ctrl.arg_size, 32);
        assert_eq!(ctrl.out_code, 0);
        assert_eq!(ctrl.out_fd, 0);
    }

    // ---- finish_sdata_open_by_fd ----------------------------------------

    #[test]
    fn finish_fcntl_error_is_passed_through_unwrapped() {
        let ctrl = SdataCtrl::new(5, 0, 0, 0);
        let outcome = finish_sdata_open_by_fd(-5, &ctrl);
        assert!(matches!(outcome, SdataOpenByFdOutcome::FcntlError(-5)));
    }

    #[test]
    fn finish_ctrl_error_wraps_out_code() {
        let mut ctrl = SdataCtrl::new(5, 0, 0, 0);
        ctrl.out_code = 0x8001_000F_u32 as i32;
        let outcome = finish_sdata_open_by_fd(0, &ctrl);
        match outcome {
            SdataOpenByFdOutcome::CtrlError(e) => assert_eq!(e.0, 0x8001_000F),
            _ => panic!("expected CtrlError"),
        }
    }

    #[test]
    fn finish_happy_path_surfaces_out_fd() {
        let mut ctrl = SdataCtrl::new(5, 0, 0, 0);
        ctrl.out_fd = 42;
        let outcome = finish_sdata_open_by_fd(0, &ctrl);
        assert!(matches!(outcome, SdataOpenByFdOutcome::Opened { sdata_fd: 42 }));
    }

    #[test]
    fn finish_fcntl_error_beats_ctrl_error() {
        let mut ctrl = SdataCtrl::new(5, 0, 0, 0);
        ctrl.out_code = 1;
        ctrl.out_fd = 99;
        // fcntl error is checked first in C++ — ctrl.out_code must be
        // ignored here.
        let outcome = finish_sdata_open_by_fd(-2, &ctrl);
        assert!(matches!(outcome, SdataOpenByFdOutcome::FcntlError(-2)));
    }

    #[test]
    fn finish_ctrl_error_beats_successful_out_fd() {
        let mut ctrl = SdataCtrl::new(5, 0, 0, 0);
        ctrl.out_code = 123;
        ctrl.out_fd = 77;
        let outcome = finish_sdata_open_by_fd(0, &ctrl);
        assert!(matches!(outcome, SdataOpenByFdOutcome::CtrlError(_)));
    }

    // ---- cellFsSdataOpenWithVersion --------------------------------------

    #[test]
    fn open_with_version_stubs_to_cell_ok() {
        assert!(cell_fs_sdata_open_with_version().is_ok());
    }

    // ---- fd registry ------------------------------------------------------

    #[test]
    fn registry_starts_at_fd_3() {
        let mut reg = SdataFdRegistry::new();
        let entry = reg.register(10, 0).unwrap();
        assert_eq!(entry.sdata_fd, 3);
    }

    #[test]
    fn registry_allocates_monotonically() {
        let mut reg = SdataFdRegistry::new();
        let a = reg.register(10, 0).unwrap();
        let b = reg.register(11, 0).unwrap();
        let c = reg.register(12, 0).unwrap();
        assert_eq!((a.sdata_fd, b.sdata_fd, c.sdata_fd), (3, 4, 5));
    }

    #[test]
    fn registry_get_returns_entry_after_register() {
        let mut reg = SdataFdRegistry::new();
        let e = reg.register(42, 0x1000).unwrap();
        let back = reg.get(e.sdata_fd).copied().unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn registry_get_returns_none_for_unknown() {
        let reg = SdataFdRegistry::new();
        assert!(reg.get(999).is_none());
    }

    #[test]
    fn registry_close_marks_entry_closed() {
        let mut reg = SdataFdRegistry::new();
        let e = reg.register(5, 0).unwrap();
        assert_eq!(reg.open_count(), 1);
        reg.close(e.sdata_fd).unwrap();
        assert_eq!(reg.open_count(), 0);
        assert!(reg.get(e.sdata_fd).unwrap().closed);
    }

    #[test]
    fn registry_double_close_is_ebadf() {
        let mut reg = SdataFdRegistry::new();
        let e = reg.register(5, 0).unwrap();
        reg.close(e.sdata_fd).unwrap();
        assert_eq!(reg.close(e.sdata_fd).unwrap_err(), CellError::EBADF);
    }

    #[test]
    fn registry_close_unknown_is_ebadf() {
        let mut reg = SdataFdRegistry::new();
        assert_eq!(reg.close(7).unwrap_err(), CellError::EBADF);
    }

    #[test]
    fn registry_open_count_tracks_live_entries() {
        let mut reg = SdataFdRegistry::new();
        reg.register(5, 0).unwrap();
        reg.register(6, 0).unwrap();
        reg.register(7, 0).unwrap();
        assert_eq!(reg.open_count(), 3);
        let second = reg.entries[1].sdata_fd;
        reg.close(second).unwrap();
        assert_eq!(reg.open_count(), 2);
    }

    // ---- full smoke lifecycle --------------------------------------------

    #[test]
    fn full_sdata_lifecycle_smoke() {
        // cellFsSdataOpen validates + defers to cellFsOpen.
        let open = cell_fs_sdata_open(CELL_FS_O_RDONLY).unwrap();
        assert_eq!(open.flags, 0);

        // cellFsSdataOpenByFd builds the fcntl plan.
        let plan = cell_fs_sdata_open_by_fd(true, 7, 0, 0x1000, 0x4000_0000, 128).unwrap();
        let mut ctrl = plan.ctrl;
        // Simulate a successful fcntl round-trip — sys_fs writes an out_fd.
        ctrl.out_fd = 33;
        let outcome = finish_sdata_open_by_fd(0, &ctrl);
        let sdata_fd = match outcome {
            SdataOpenByFdOutcome::Opened { sdata_fd } => sdata_fd,
            _ => panic!("expected Opened"),
        };
        assert_eq!(sdata_fd, 33);

        // Record the fd in the registry so close() can later find it.
        let mut reg = SdataFdRegistry::new();
        let entry = reg.register(plan.mself_fd, plan.ctrl.offset).unwrap();
        assert_eq!(entry.mself_fd, 7);
        assert_eq!(entry.offset, 0x1000);

        // Graceful close.
        reg.close(entry.sdata_fd).unwrap();
        assert_eq!(reg.open_count(), 0);

        // Stub entry point never fails.
        assert!(cell_fs_sdata_open_with_version().is_ok());
    }
}
