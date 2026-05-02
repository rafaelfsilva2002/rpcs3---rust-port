//! R6.0 — C-ABI bindings around the Rust SPU stack.
//!
//! This crate exposes a minimal, panic-safe C-callable surface that
//! the RPCS3 C++ side will use (in R6.1+) to delegate cooperative
//! SPU thread execution to the Rust interpreter. R6.0 is purely
//! Rust-side scaffolding — no C++ patch consumes this crate yet.
//!
//! # Conventions
//!
//! - All entry points are `extern "C"` and panic-safe via
//!   [`std::panic::catch_unwind`]: a Rust panic surfaces as a
//!   non-zero return code rather than unwinding across the FFI
//!   boundary (which is UB).
//! - Integer return codes: 0 on success, non-zero on error. The
//!   typed [`RustSpuOutcome`] enum is the return type for run/step.
//! - GPRs are passed as 16-byte arrays in SPU big-endian byte order,
//!   matching the on-disk wire format used by the trace writer +
//!   replay engine. This decouples the FFI from host endianness.
//! - Each handle is single-threaded. Multi-threaded SPU bridges are
//!   R6.6 scope (one handle per cooperative SPU thread, lifetime
//!   owned by the C++ `spu_thread`).
//!
//! # Memory ownership
//!
//! [`rust_spu_new`] returns a `*mut RustSpu` allocated via
//! `Box::leak`. The caller MUST eventually pass it back to
//! [`rust_spu_drop`]. Never call any other entry point with a
//! null or already-dropped handle — those are UB at the FFI
//! boundary, but each entry point still null-checks the handle
//! and returns an error code instead of dereferencing.

#![allow(non_camel_case_types)]

use std::panic::{self, AssertUnwindSafe};

use rpcs3_spu_interpreter::{run_n, StepOutcome};
use rpcs3_spu_thread::{SpuThread, SPU_LS_SIZE};

// =====================================================================
// Public types
// =====================================================================

/// Opaque handle. The C side sees only `*mut RustSpu`.
pub struct RustSpu {
    spu: SpuThread,
}

/// Run/step outcome surfaced to the C side.
///
/// `code_or_channel` semantics by variant (filled by run/step):
/// - [`RustSpuOutcome::Continue`]: 0 (max steps reached, SPU still
///   running but the run window expired)
/// - [`RustSpuOutcome::Stop`]: the 14-bit stop code (e.g. 0x101 for
///   `SYS_SPU_THREAD_STOP_GROUP_EXIT`)
/// - [`RustSpuOutcome::StallRead`] / [`RustSpuOutcome::StallWrite`]:
///   the channel index that stalled (29 = IN_MBOX, 28 = OUT_MBOX, 3 = SNR1, etc.)
/// - [`RustSpuOutcome::Error`]: the program counter at which the
///   error fired (an unsupported opcode or LS-OOB read).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RustSpuOutcome {
    Continue = 0,
    Stop = 1,
    StallRead = 2,
    StallWrite = 3,
    Error = 4,
}

// =====================================================================
// Internal helpers
// =====================================================================

/// Catches any panic and returns the supplied error code instead of
/// unwinding across the FFI boundary.
fn guard<F: FnOnce() -> i32>(f: F, panic_code: i32) -> i32 {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok(rc) => rc,
        Err(_) => panic_code,
    }
}

/// Same as [`guard`] but for run/step entry points that return
/// [`RustSpuOutcome`]. On panic, returns `Error` and writes 0xFFFF_FFFF
/// to `out_code` if non-null.
fn guard_outcome<F: FnOnce() -> (RustSpuOutcome, u32)>(
    f: F,
    out_code: *mut u32,
) -> RustSpuOutcome {
    match panic::catch_unwind(AssertUnwindSafe(f)) {
        Ok((outcome, code)) => {
            if !out_code.is_null() {
                unsafe { *out_code = code };
            }
            outcome
        }
        Err(_) => {
            if !out_code.is_null() {
                unsafe { *out_code = 0xFFFF_FFFF };
            }
            RustSpuOutcome::Error
        }
    }
}

#[inline]
unsafe fn handle_mut<'a>(h: *mut RustSpu) -> Option<&'a mut RustSpu> {
    h.as_mut()
}

// =====================================================================
// Lifecycle
// =====================================================================

/// Allocate a fresh SPU executor instance. Returns null on
/// allocation failure (currently unreachable; `Box::new` panics
/// instead, which we catch and return null for).
///
/// # Safety
///
/// The returned pointer must be passed to [`rust_spu_drop`]
/// exactly once. Passing it to any other entry point after drop
/// is UB.
#[no_mangle]
pub extern "C" fn rust_spu_new() -> *mut RustSpu {
    panic::catch_unwind(AssertUnwindSafe(|| {
        let h = Box::new(RustSpu {
            spu: SpuThread::new(0),
        });
        Box::into_raw(h)
    }))
    .unwrap_or(std::ptr::null_mut())
}

/// Drop the executor and free its backing memory. Null pointer is a
/// no-op (matches `free(NULL)`).
///
/// # Safety
///
/// `h` must be a pointer previously returned by [`rust_spu_new`],
/// and not yet passed to this function.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_drop(h: *mut RustSpu) {
    if h.is_null() {
        return;
    }
    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        drop(Box::from_raw(h));
    }));
}

// =====================================================================
// Initial-state setup
// =====================================================================

/// Copy `size` bytes from `bytes` into the executor's Local Store
/// at offset 0.
///
/// Returns 0 on success; -1 on null handle / null bytes; -2 if
/// `size > SPU_LS_SIZE` (256 KiB).
///
/// # Safety
///
/// `bytes` must point to at least `size` readable bytes. `h` must
/// be a live handle.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_load_ls(
    h: *mut RustSpu,
    bytes: *const u8,
    size: u32,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            if bytes.is_null() {
                return -1;
            }
            if size as usize > SPU_LS_SIZE {
                return -2;
            }
            let slice = std::slice::from_raw_parts(bytes, size as usize);
            if !h.spu.ls_write(0, slice) {
                return -3;
            }
            0
        },
        -100,
    )
}

/// Set GPR `reg` (0..127) to the 128-bit value in `bytes` (16 bytes,
/// SPU big-endian byte order — `bytes[0]` = preferred-slot MSB).
///
/// Returns 0 on success; -1 on null handle / null bytes; -2 on
/// `reg >= 128`.
///
/// # Safety
///
/// `bytes` must point to at least 16 readable bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_set_gpr(
    h: *mut RustSpu,
    reg: u32,
    bytes: *const u8,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            if bytes.is_null() {
                return -1;
            }
            if reg as usize >= h.spu.gpr.len() {
                return -2;
            }
            let mut buf = [0u8; 16];
            buf.copy_from_slice(std::slice::from_raw_parts(bytes, 16));
            h.spu.gpr[reg as usize] = u128::from_be_bytes(buf);
            0
        },
        -100,
    )
}

/// Set the program counter (instruction pointer). The provided
/// `pc` is silently masked to 4-byte alignment + 256 KiB LS bounds.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_set_pc(h: *mut RustSpu, pc: u32) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            h.spu.pc = pc & 0x3FFFC;
            0
        },
        -100,
    )
}

// =====================================================================
// Channel ops (PPU side)
// =====================================================================

/// Push `value` into the SPU's IN_MBOX (channel 29). Returns 0 on
/// success, -1 on null handle.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_push_inmbox(
    h: *mut RustSpu,
    value: u32,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            let _ = h.spu.channels.ppu_push_inmbox(value);
            0
        },
        -100,
    )
}

/// Pop a value from the SPU's OUT_MBOX (channel 28). Writes the
/// drained value to `*out_value`. Returns 0 on success (mailbox
/// had a value), 1 if the mailbox was empty, -1 on null pointers.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_pop_outmbox(
    h: *mut RustSpu,
    out_value: *mut u32,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            if out_value.is_null() {
                return -1;
            }
            match h.spu.channels.ppu_pop_outmbox() {
                Some(v) => {
                    *out_value = v;
                    0
                }
                None => 1,
            }
        },
        -100,
    )
}

/// Send a signal to SNR `slot` (0 = SNR1, 1 = SNR2). The OR-merge
/// behavior matches Cell BE semantics: if a value is already pending,
/// `value` is OR-merged. Returns 0 on success, -2 on slot >= 2.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_signal(
    h: *mut RustSpu,
    slot: u32,
    value: u32,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            if slot >= 2 {
                return -2;
            }
            let _ = h.spu.channels.signal(slot as usize, value);
            0
        },
        -100,
    )
}

// =====================================================================
// Run / step
// =====================================================================

/// Run up to `max_steps` instructions. Returns the outcome that
/// caused the run to halt:
///
/// - [`RustSpuOutcome::Continue`] (0): max_steps reached, SPU still
///   running. `*out_code` set to 0. `*out_steps` set to `max_steps`.
/// - [`RustSpuOutcome::Stop`] (1): SPU executed a `stop` instruction.
///   `*out_code` set to the 14-bit stop code.
/// - [`RustSpuOutcome::StallRead`] (2) / [`RustSpuOutcome::StallWrite`] (3):
///   SPU parked on a blocking channel op. `*out_code` set to the
///   channel index.
/// - [`RustSpuOutcome::Error`] (4): unsupported opcode / LS-OOB.
///   `*out_code` set to the program counter at which the error fired.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_run_until_event(
    h: *mut RustSpu,
    max_steps: u32,
    out_code: *mut u32,
    out_steps: *mut u32,
) -> RustSpuOutcome {
    guard_outcome(
        || {
            let Some(h) = handle_mut(h) else {
                return (RustSpuOutcome::Error, 0xFFFF_FFFF);
            };
            match run_n(&mut h.spu, max_steps as usize) {
                Ok((steps, outcome)) => {
                    if !out_steps.is_null() {
                        *out_steps = steps as u32;
                    }
                    match outcome {
                        StepOutcome::Continue => (RustSpuOutcome::Continue, 0),
                        StepOutcome::Stop(code) => (RustSpuOutcome::Stop, code),
                        StepOutcome::ChannelStall {
                            channel,
                            is_write: false,
                        } => (RustSpuOutcome::StallRead, channel),
                        StepOutcome::ChannelStall {
                            channel,
                            is_write: true,
                        } => (RustSpuOutcome::StallWrite, channel),
                    }
                }
                Err(_e) => {
                    if !out_steps.is_null() {
                        *out_steps = 0;
                    }
                    (RustSpuOutcome::Error, h.spu.pc)
                }
            }
        },
        out_code,
    )
}

/// Single-instruction step. Convenience wrapper around
/// [`rust_spu_run_until_event`] with `max_steps = 1`.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_step(
    h: *mut RustSpu,
    out_code: *mut u32,
) -> RustSpuOutcome {
    let mut steps = 0u32;
    rust_spu_run_until_event(h, 1, out_code, &mut steps as *mut u32)
}

// =====================================================================
// Query state
// =====================================================================

/// Read the current PC. Returns 0 on success, -1 on null pointers.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_get_pc(
    h: *mut RustSpu,
    out_pc: *mut u32,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            if out_pc.is_null() {
                return -1;
            }
            *out_pc = h.spu.pc;
            0
        },
        -100,
    )
}

/// Read GPR `reg` (0..127) into `bytes` (16 bytes, BE order).
///
/// # Safety
///
/// `bytes` must point to at least 16 writable bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_get_gpr(
    h: *mut RustSpu,
    reg: u32,
    bytes: *mut u8,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            if bytes.is_null() {
                return -1;
            }
            if reg as usize >= h.spu.gpr.len() {
                return -2;
            }
            let be = h.spu.gpr[reg as usize].to_be_bytes();
            std::ptr::copy_nonoverlapping(be.as_ptr(), bytes, 16);
            0
        },
        -100,
    )
}

/// Read up to `size` bytes from LS offset 0 into `bytes`.
/// `size` must be `<= 256 KiB`.
///
/// # Safety
///
/// `bytes` must point to at least `size` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_get_ls(
    h: *mut RustSpu,
    bytes: *mut u8,
    size: u32,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            if bytes.is_null() {
                return -1;
            }
            if size as usize > SPU_LS_SIZE {
                return -2;
            }
            let Some(view) = h.spu.ls_read(0, size as usize) else {
                return -3;
            };
            std::ptr::copy_nonoverlapping(view.as_ptr(), bytes, size as usize);
            0
        },
        -100,
    )
}

/// Read the SPU's parked PC (if any). Returns 0 on success and
/// writes the parked PC to `*out_pc`. Returns 1 if not parked
/// (out_pc unchanged). Returns -1 on null pointers.
#[no_mangle]
pub unsafe extern "C" fn rust_spu_get_park_pc(
    h: *mut RustSpu,
    out_pc: *mut u32,
) -> i32 {
    guard(
        || {
            let Some(h) = handle_mut(h) else { return -1 };
            if out_pc.is_null() {
                return -1;
            }
            match h.spu.park_state {
                Some(p) => {
                    *out_pc = p.pc;
                    0
                }
                None => 1,
            }
        },
        -100,
    )
}

// Re-export internal helpers used by tests (kept private to the
// crate; not part of the C-ABI surface).
#[doc(hidden)]
pub mod __test_support {
    pub use rpcs3_spu_thread::ch::*;
    pub use rpcs3_spu_thread::SpuParkReason;
}

#[cfg(test)]
mod tests;
