//! R6.0 — exercise the C-ABI surface from Rust as if it were C.
//!
//! Each test uses raw pointers + `unsafe` blocks (no `&mut Box<...>`
//! shortcuts) to validate that the FFI surface holds when invoked
//! the way the C++ side will invoke it in R6.1+. The 4 replay-
//! validated oracle fixtures are the truth source for SPU semantics
//! — these tests build a minimum-viable program in LS and verify
//! the FFI's I/O matches the existing replay engine's output.

use super::*;

/// Minimal handcrafted SPU program: `il r3, 7; stop 0xC3`.
/// `il` (immediate load): primary 0x081, RI16-form. Loads `value`
/// (sign-extended 16-bit) into all 4 lanes of `rt`.
fn build_il_stop_program() -> Vec<u8> {
    let il_r3_7: u32 = (0x081_u32 << 23) | ((7u32 & 0xFFFF) << 7) | 3;
    let stop_c3: u32 = 0xC3_u32 & 0x3FFF;
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&il_r3_7.to_be_bytes());
    bytes.extend_from_slice(&stop_c3.to_be_bytes());
    bytes
}

/// Lifecycle: alloc + drop. Repeats to make sure we don't leak the
/// `Box` somehow.
#[test]
fn rust_spu_new_drop_lifecycle() {
    for _ in 0..16 {
        unsafe {
            let h = rust_spu_new();
            assert!(!h.is_null(), "rust_spu_new must return non-null");
            rust_spu_drop(h);
        }
    }
}

/// Drop on null is a no-op (matches `free(NULL)` semantics).
#[test]
fn rust_spu_drop_null_is_noop() {
    unsafe {
        rust_spu_drop(std::ptr::null_mut());
    }
}

/// Every entry point must safely return an error on null handle
/// instead of dereferencing.
#[test]
fn rust_spu_entry_points_reject_null_handle() {
    unsafe {
        let mut tmp_u32 = 0u32;
        let mut tmp_buf = [0u8; 16];
        let null = std::ptr::null_mut::<RustSpu>();

        assert_eq!(rust_spu_load_ls(null, [0u8; 4].as_ptr(), 4), -1);
        assert_eq!(rust_spu_set_gpr(null, 1, [0u8; 16].as_ptr()), -1);
        assert_eq!(rust_spu_set_pc(null, 0), -1);
        assert_eq!(rust_spu_push_inmbox(null, 0), -1);
        assert_eq!(rust_spu_pop_outmbox(null, &mut tmp_u32 as *mut u32), -1);
        assert_eq!(rust_spu_signal(null, 0, 0), -1);
        assert_eq!(rust_spu_get_pc(null, &mut tmp_u32 as *mut u32), -1);
        assert_eq!(rust_spu_get_gpr(null, 0, tmp_buf.as_mut_ptr()), -1);
        assert_eq!(rust_spu_get_ls(null, tmp_buf.as_mut_ptr(), 16), -1);
        assert_eq!(rust_spu_get_park_pc(null, &mut tmp_u32 as *mut u32), -1);

        let mut steps = 0u32;
        let outcome = rust_spu_run_until_event(
            null,
            10,
            &mut tmp_u32 as *mut u32,
            &mut steps as *mut u32,
        );
        assert_eq!(outcome, RustSpuOutcome::Error);
    }
}

/// Bounds checks: reg out-of-range, size > LS, etc.
#[test]
fn rust_spu_bounds_checks() {
    unsafe {
        let h = rust_spu_new();

        // GPR reg >= 128 → -2.
        assert_eq!(rust_spu_set_gpr(h, 128, [0u8; 16].as_ptr()), -2);
        let mut buf = [0u8; 16];
        assert_eq!(rust_spu_get_gpr(h, 128, buf.as_mut_ptr()), -2);

        // LS size > 256 KiB → -2.
        let big = vec![0u8; SPU_LS_SIZE + 1];
        assert_eq!(rust_spu_load_ls(h, big.as_ptr(), big.len() as u32), -2);
        assert_eq!(rust_spu_get_ls(h, buf.as_mut_ptr(), (SPU_LS_SIZE + 1) as u32), -2);

        // Signal slot >= 2 → -2.
        assert_eq!(rust_spu_signal(h, 2, 0xDEADBEEF), -2);

        rust_spu_drop(h);
    }
}

/// Set GPR via FFI, read it back, verify byte-identical roundtrip.
#[test]
fn rust_spu_gpr_set_get_roundtrip() {
    unsafe {
        let h = rust_spu_new();

        // 16-byte BE pattern.
        let want: [u8; 16] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
            0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
        ];

        assert_eq!(rust_spu_set_gpr(h, 5, want.as_ptr()), 0);

        let mut got = [0u8; 16];
        assert_eq!(rust_spu_get_gpr(h, 5, got.as_mut_ptr()), 0);
        assert_eq!(got, want, "GPR roundtrip must be byte-identical");

        rust_spu_drop(h);
    }
}

/// Load a tiny SPU program via FFI, run it, check it stops with the
/// expected code and the expected GPR value.
#[test]
fn rust_spu_run_il_stop_program() {
    unsafe {
        let h = rust_spu_new();

        let bytes = build_il_stop_program();
        assert_eq!(
            rust_spu_load_ls(h, bytes.as_ptr(), bytes.len() as u32),
            0,
            "load_ls"
        );

        // PC=0 (entry).
        assert_eq!(rust_spu_set_pc(h, 0), 0);

        let mut code = 0u32;
        let mut steps = 0u32;
        let outcome = rust_spu_run_until_event(
            h,
            100,
            &mut code as *mut u32,
            &mut steps as *mut u32,
        );

        assert_eq!(outcome, RustSpuOutcome::Stop, "must hit stop");
        assert_eq!(code, 0xC3, "stop code must be 0xC3");
        assert_eq!(steps, 2, "il + stop = 2 steps");

        // Verify r3 = 7 (preferred slot = top 32 bits of u128).
        let mut r3_be = [0u8; 16];
        assert_eq!(rust_spu_get_gpr(h, 3, r3_be.as_mut_ptr()), 0);
        let lane0 = u32::from_be_bytes([r3_be[0], r3_be[1], r3_be[2], r3_be[3]]);
        assert_eq!(lane0, 7, "r3 lane 0 must equal the loaded immediate");

        rust_spu_drop(h);
    }
}

/// Drive a mailbox handshake through the FFI: SPU does
/// `rdch ch29; wrch ch28, r3; stop 0x101`. PPU pushes a value,
/// expects SPU to park on rdch, then sees the same value come back
/// out via OUT_MBOX after wake.
#[test]
fn rust_spu_mailbox_handshake_via_ffi() {
    unsafe {
        let h = rust_spu_new();

        // Build SPU program: rdch r3,ch29; wrch ch28,r3; stop 0x101.
        // rdch: primary 0x00D, RR-like. ra=channel index (=29), rt=3.
        let rdch: u32 = (0x00D_u32 << 21) | ((29 & 0x7F) << 7) | 3;
        // wrch: primary 0x10D, ra=channel (28), rt=3.
        let wrch: u32 = (0x10D_u32 << 21) | ((28 & 0x7F) << 7) | 3;
        let stop: u32 = 0x101 & 0x3FFF;

        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());

        assert_eq!(rust_spu_load_ls(h, bytes.as_ptr(), bytes.len() as u32), 0);
        assert_eq!(rust_spu_set_pc(h, 0), 0);

        // First run: SPU should park on rdch (channel 29 empty).
        let mut code = 0u32;
        let mut steps = 0u32;
        let outcome = rust_spu_run_until_event(
            h,
            100,
            &mut code as *mut u32,
            &mut steps as *mut u32,
        );
        assert_eq!(outcome, RustSpuOutcome::StallRead, "must park on rdch");
        assert_eq!(code, 29, "stalled on channel 29 (IN_MBOX)");

        // Verify park_pc = 0 (rdch instruction).
        let mut park_pc = u32::MAX;
        assert_eq!(rust_spu_get_park_pc(h, &mut park_pc as *mut u32), 0);
        assert_eq!(park_pc, 0);

        // PPU pushes 0xCAFEBEEF.
        assert_eq!(rust_spu_push_inmbox(h, 0xCAFE_BEEF), 0);

        // Second run: SPU resumes from park, reads, writes,
        // stops with 0x101.
        let outcome = rust_spu_run_until_event(
            h,
            100,
            &mut code as *mut u32,
            &mut steps as *mut u32,
        );
        assert_eq!(outcome, RustSpuOutcome::Stop, "must hit stop");
        assert_eq!(code, 0x101, "stop code = 0x101 (group exit)");

        // PPU drains OUT_MBOX, expects 0xCAFEBEEF.
        let mut popped = 0u32;
        assert_eq!(rust_spu_pop_outmbox(h, &mut popped as *mut u32), 0);
        assert_eq!(popped, 0xCAFE_BEEF, "OUT_MBOX must echo the pushed value");

        rust_spu_drop(h);
    }
}

/// Same shape as the mailbox handshake test, but using a signal
/// (`rdch ch3` for SPU_RdSigNotify1) instead of IN_MBOX.
/// Verifies the SNR1-blocking semantics fix from R5.11 lands
/// through the FFI surface.
#[test]
fn rust_spu_signal_handshake_via_ffi() {
    unsafe {
        let h = rust_spu_new();

        // rdch r3, ch3 (SNR1); wrch ch28, r3; stop 0x101.
        let rdch: u32 = (0x00D_u32 << 21) | ((3 & 0x7F) << 7) | 3;
        let wrch: u32 = (0x10D_u32 << 21) | ((28 & 0x7F) << 7) | 3;
        let stop: u32 = 0x101 & 0x3FFF;

        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());

        assert_eq!(rust_spu_load_ls(h, bytes.as_ptr(), bytes.len() as u32), 0);
        assert_eq!(rust_spu_set_pc(h, 0), 0);

        let mut code = 0u32;
        let mut steps = 0u32;
        let outcome = rust_spu_run_until_event(h, 100, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::StallRead);
        assert_eq!(code, 3, "stalled on SNR1 (channel 3)");

        // PPU sends signal slot 0.
        assert_eq!(rust_spu_signal(h, 0, 0xDEAD_BEEF), 0);

        let outcome = rust_spu_run_until_event(h, 100, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::Stop);
        assert_eq!(code, 0x101);

        let mut popped = 0u32;
        assert_eq!(rust_spu_pop_outmbox(h, &mut popped), 0);
        assert_eq!(popped, 0xDEAD_BEEF);

        rust_spu_drop(h);
    }
}

/// LS roundtrip: write program bytes, read them back, verify equal.
#[test]
fn rust_spu_ls_load_read_roundtrip() {
    unsafe {
        let h = rust_spu_new();

        let mut want = vec![0u8; 256];
        for (i, b) in want.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }

        assert_eq!(rust_spu_load_ls(h, want.as_ptr(), want.len() as u32), 0);

        let mut got = vec![0u8; 256];
        assert_eq!(rust_spu_get_ls(h, got.as_mut_ptr(), got.len() as u32), 0);

        assert_eq!(got, want);

        rust_spu_drop(h);
    }
}

/// Step (max_steps = 1) must execute exactly one instruction and
/// return Continue (since the program isn't done after one step).
#[test]
fn rust_spu_step_advances_one_instruction() {
    unsafe {
        let h = rust_spu_new();

        let bytes = build_il_stop_program();
        assert_eq!(rust_spu_load_ls(h, bytes.as_ptr(), bytes.len() as u32), 0);
        assert_eq!(rust_spu_set_pc(h, 0), 0);

        // Step 1: il r3, 7 — pc advances to 4, no stop.
        let mut code = 0u32;
        let outcome = rust_spu_step(h, &mut code);
        assert_eq!(outcome, RustSpuOutcome::Continue);

        let mut pc = 0u32;
        assert_eq!(rust_spu_get_pc(h, &mut pc), 0);
        assert_eq!(pc, 4, "pc must advance by 4 after il");

        // Step 2: stop 0xC3.
        let outcome = rust_spu_step(h, &mut code);
        assert_eq!(outcome, RustSpuOutcome::Stop);
        assert_eq!(code, 0xC3);

        rust_spu_drop(h);
    }
}

/// Unsupported opcode at PC must surface as `Error`. We use the
/// 11-bit primary 0x008 which the interpreter explicitly notes as
/// "unused" (a fully-zero instruction body with primary 0x008
/// won't match any dispatch arm and falls through to
/// `Error::Unimplemented`).
#[test]
fn rust_spu_error_on_unsupported_opcode() {
    unsafe {
        let h = rust_spu_new();

        // Top-11 = 0x008 (unused per R5.10m comment), rest = 0.
        let bad: u32 = 0x008 << 21;
        let bad_be = bad.to_be_bytes();
        assert_eq!(rust_spu_load_ls(h, bad_be.as_ptr(), 4), 0);
        assert_eq!(rust_spu_set_pc(h, 0), 0);

        let mut code = 0u32;
        let mut steps = 0u32;
        let outcome = rust_spu_run_until_event(h, 100, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::Error);
        assert_eq!(code, 0, "error PC must equal the failing pc (= 0)");

        rust_spu_drop(h);
    }
}

/// R6.5b — OUT_MBOX backpressure (StallWrite ch28) handling.
///
/// FFI-level acceptance gate for the bridge's StallWrite ch28
/// resolution. A real-binary fixture for this is not feasible in
/// PSL1GHT cooperative-thread context: the lv2 syscall surface
/// has no PPU-side path to drain a cooperative SPU's OUT_MBOX
/// during execution, so a SPU that writes ch28 twice without an
/// intervening stop would deadlock both the C++ executor's
/// `push_wait` and any naive bridge implementation. This FFI test
/// drives the same SPU instruction sequence the bridge's
/// `try_delegate_execution()` multi-round loop sees: the program
/// writes OUT_MBOX twice (with a stop in between is not allowed
/// because stop terminates the SPU; instead the program writes,
/// writes again, stops). Between the two writes the executor
/// surfaces `StallWrite code=28`. The test then drains and
/// resumes, reproducing the bridge's depth-1 overwrite semantics
/// at the FFI layer.
///
/// Program (3 instructions):
/// ```
///   pc= 0:  il r3, 0xAAA      ; r3 = 0xAAA (load immediate, sign-extended)
///   pc= 4:  wrch ch28, r3     ; OUT_MBOX = 0xAAA  → succeeds
///                             ; (next wrch will stall)
///   pc= 8:  il r3, 0x555      ; r3 = 0x555 (overwrites the prev value)
///   pc=12:  wrch ch28, r3     ; OUT_MBOX = 0x555  → STALLS
///                             ; (Rust mailbox still holds 0xAAA)
///   pc=16:  stop 0xC5
/// ```
///
/// The test verifies:
///   1. First run: 3 instructions execute (il, wrch, il), then
///      the second wrch stalls → `StallWrite code=28`.
///   2. Pop Rust's OUT_MBOX → drains 0xAAA (first value, never
///      reached the consumer-equivalent in the FFI test, but
///      matches what the bridge would push to RPCS3's ch_out_mbox
///      via `set_value`).
///   3. Run again on SAME handle → the stalled wrch retries with
///      Rust's OUT_MBOX now empty, succeeds, then the stop fires.
///   4. Pop Rust's OUT_MBOX → drains 0x555.
///   5. Outcome = Stop, code = 0xC5.
///
/// If a future bridge implementation drops the handle on
/// StallWrite (the R6.4b "any other Stall = fall back" behavior),
/// run-2 would start fresh from PC=0 and re-emit 0xAAA, never
/// reaching 0x555 — a detectable corruption this test catches.
#[test]
fn rust_spu_outmbox_backpressure_via_ffi() {
    unsafe {
        let h = rust_spu_new();

        // Encode the 5-instruction program. il is RI16-form (primary
        // 0x081, 16-bit imm sign-extended into all 4 lanes of rt).
        // Same encoding helpers as `build_il_stop_program` and the
        // mailbox handshake tests use.
        let il_r3_aaa: u32 = (0x081_u32 << 23) | ((0xAAA_u32 & 0xFFFF) << 7) | 3;
        let wrch_28:  u32 = (0x10D_u32 << 21) | ((28 & 0x7F) << 7) | 3;
        let il_r3_555: u32 = (0x081_u32 << 23) | ((0x555_u32 & 0xFFFF) << 7) | 3;
        let stop_c5:  u32 = 0xC5_u32 & 0x3FFF;

        let mut bytes = Vec::with_capacity(20);
        bytes.extend_from_slice(&il_r3_aaa.to_be_bytes());  // pc=0
        bytes.extend_from_slice(&wrch_28.to_be_bytes());    // pc=4
        bytes.extend_from_slice(&il_r3_555.to_be_bytes());  // pc=8
        bytes.extend_from_slice(&wrch_28.to_be_bytes());    // pc=12
        bytes.extend_from_slice(&stop_c5.to_be_bytes());    // pc=16

        assert_eq!(rust_spu_load_ls(h, bytes.as_ptr(), bytes.len() as u32), 0);
        assert_eq!(rust_spu_set_pc(h, 0), 0);

        // First run: il/wrch/il succeed, the second wrch stalls.
        // The stalled instruction itself counts as a "step" attempt
        // (matches the mailbox_multi_round test's convention where
        // a stall on the 3rd instruction reports steps=3).
        let mut code = 0u32;
        let mut steps = 0u32;
        let outcome = rust_spu_run_until_event(h, 100, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::StallWrite, "must stall on second wrch");
        assert_eq!(code, 28, "stalled on OUT_MBOX (channel 28)");
        assert_eq!(steps, 4, "il + wrch + il + wrch(stalled) = 4 steps");

        // The bridge's recovery: pop Rust's OUT_MBOX (drains the
        // intermediate value) and continue.
        let mut popped = 0u32;
        assert_eq!(rust_spu_pop_outmbox(h, &mut popped), 0);
        assert_eq!(popped, 0xAAA, "drained value must equal first wrch value (0xAAA)");
        // OUT_MBOX must be empty now.
        assert_eq!(
            rust_spu_pop_outmbox(h, &mut popped), 1,
            "OUT_MBOX must be empty after the drain",
        );

        // Second run on the SAME handle: stalled wrch retries with
        // empty Rust OUT_MBOX, succeeds, then stop fires.
        let outcome = rust_spu_run_until_event(h, 100, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::Stop, "must hit stop after resume");
        assert_eq!(code, 0xC5, "stop code = 0xC5");
        // Steps in run 2: the stalled wrch retries (1) + stop (1) = 2.
        assert_eq!(steps, 2, "run 2 must consume the retried wrch + stop");

        // Pop the second value.
        assert_eq!(rust_spu_pop_outmbox(h, &mut popped), 0);
        assert_eq!(popped, 0x555, "second drained value must equal second wrch value (0x555)");

        // Final PC at the stop instruction.
        let mut final_pc = 0u32;
        assert_eq!(rust_spu_get_pc(h, &mut final_pc), 0);
        assert_eq!(final_pc, 16, "final pc must be at the stop instruction");

        rust_spu_drop(h);
    }
}

/// R6.4b-pre — multi-round mailbox handshake on the SAME handle.
///
/// FFI-level dual of the
/// `behavior-freeze/fixtures/spu/sources/single_spu_mailbox_multi_v1`
/// .self fixture. The fixture's `.self` is not yet built (no
/// PSL1GHT toolchain in the current dev env), so this test serves
/// as the **executable** acceptance gate for R6.4b: any
/// persistent-handle implementation that wires
/// `try_delegate_execution()` to keep `rust_spu_t*` alive across
/// `cpu_task` re-entries should pass this test deterministically.
///
/// Program (5 instructions, one quad each):
/// ```
///   pc= 0:  rdch r3, ch29   ; round 1: read IN_MBOX
///   pc= 4:  wrch ch28, r3   ; round 1: echo to OUT_MBOX
///   pc= 8:  rdch r3, ch29   ; round 2: read IN_MBOX (STALLS here)
///   pc=12:  wrch ch28, r3   ; round 2: echo to OUT_MBOX
///   pc=16:  stop 0x101
/// ```
///
/// The test verifies:
///   1. First run after pushing only round 1 → StallRead at pc=8
///      (channel = 29). PC parks; first OUT_MBOX populated.
///   2. PPU drains OUT_MBOX (must equal round-1 push).
///   3. PPU pushes round 2.
///   4. Second run on SAME handle → Stop with code 0x101.
///   5. PPU drains OUT_MBOX (must equal round-2 push).
///   6. Total steps consumed = 5 (3 in run-1 + 2 in run-2).
///
/// If a future bridge implementation (R6.4b) drops the handle on
/// the first StallRead and rebuilds it on re-entry, the second
/// run starts from PC=0 again, re-reads IN_MBOX (which now has
/// round 2's value) and emits round-1's value to OUT_MBOX — a
/// detectable corruption that this test catches.
#[test]
fn rust_spu_mailbox_multi_round_via_ffi() {
    unsafe {
        let h = rust_spu_new();

        // Encode rdch r3, ch29; wrch ch28, r3; rdch r3, ch29;
        // wrch ch28, r3; stop 0x101.
        let rdch_29: u32 = (0x00D_u32 << 21) | ((29 & 0x7F) << 7) | 3;
        let wrch_28: u32 = (0x10D_u32 << 21) | ((28 & 0x7F) << 7) | 3;
        let stop_101: u32 = 0x101 & 0x3FFF;

        let mut bytes = Vec::with_capacity(20);
        bytes.extend_from_slice(&rdch_29.to_be_bytes());   // pc=0
        bytes.extend_from_slice(&wrch_28.to_be_bytes());   // pc=4
        bytes.extend_from_slice(&rdch_29.to_be_bytes());   // pc=8
        bytes.extend_from_slice(&wrch_28.to_be_bytes());   // pc=12
        bytes.extend_from_slice(&stop_101.to_be_bytes());  // pc=16

        assert_eq!(rust_spu_load_ls(h, bytes.as_ptr(), bytes.len() as u32), 0);
        assert_eq!(rust_spu_set_pc(h, 0), 0);

        // Round 1: PPU pushes first command.
        const ROUND1: u32 = 0x1111_AAAA;
        const ROUND2: u32 = 0x2222_BBBB;
        assert_eq!(rust_spu_push_inmbox(h, ROUND1), 0);

        // First run: rdch (consume ROUND1), wrch (push to OUT_MBOX),
        // rdch (STALL — IN_MBOX now empty).
        let mut code = 0u32;
        let mut steps = 0u32;
        let outcome = rust_spu_run_until_event(h, 100, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::StallRead, "must park on second rdch");
        assert_eq!(code, 29, "stalled on IN_MBOX (channel 29)");
        assert_eq!(steps, 3, "consumed exactly 3 instructions before stall");

        // Park PC must be at pc=8 (the second rdch).
        let mut park_pc = u32::MAX;
        assert_eq!(rust_spu_get_park_pc(h, &mut park_pc), 0);
        assert_eq!(park_pc, 8);

        // PPU drains OUT_MBOX — must contain round 1's value.
        let mut popped = 0u32;
        assert_eq!(rust_spu_pop_outmbox(h, &mut popped), 0);
        assert_eq!(popped, ROUND1, "round 1 OUT_MBOX must echo ROUND1");
        // OUT_MBOX must be empty after the drain.
        assert_eq!(
            rust_spu_pop_outmbox(h, &mut popped), 1,
            "OUT_MBOX must be empty after the drain"
        );

        // R6.4b-pre invariant check: handle must be the SAME instance
        // across runs. Verified by getting GPR/PC and confirming
        // they reflect post-run-1 state, not initial state.
        let mut gpr3 = [0u8; 16];
        assert_eq!(rust_spu_get_gpr(h, 3, gpr3.as_mut_ptr()), 0);
        // r3 holds ROUND1 in the preferred slot (BE bytes 0..4).
        let r3_preferred = u32::from_be_bytes([gpr3[0], gpr3[1], gpr3[2], gpr3[3]]);
        assert_eq!(r3_preferred, ROUND1, "r3 must hold round 1's value across the stall");

        // PPU pushes round 2.
        assert_eq!(rust_spu_push_inmbox(h, ROUND2), 0);

        // Second run on the SAME handle: rdch resumes from park,
        // consumes ROUND2; wrch pushes to OUT_MBOX; stop 0x101.
        let outcome = rust_spu_run_until_event(h, 100, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::Stop, "must hit stop after resume");
        assert_eq!(code, 0x101, "stop code = 0x101 (group exit)");
        assert_eq!(steps, 3, "consumed rdch + wrch + stop = 3 steps in run 2");

        // PPU drains OUT_MBOX — must contain round 2's value.
        assert_eq!(rust_spu_pop_outmbox(h, &mut popped), 0);
        assert_eq!(popped, ROUND2, "round 2 OUT_MBOX must echo ROUND2");

        // Final PC must be past the stop instruction.
        let mut final_pc = 0u32;
        assert_eq!(rust_spu_get_pc(h, &mut final_pc), 0);
        assert_eq!(final_pc, 16, "final pc must be at the stop instruction");

        rust_spu_drop(h);
    }
}

/// R6.4 — Continue + resume on the same handle.
/// Demonstrates the FFI contract that supports persistent-handle
/// re-entry: a single `rust_spu_t*` can survive multiple
/// `rust_spu_run_until_event` calls, each picking up where the
/// previous left off. PC, GPRs, LS, channels all persist.
///
/// Not yet exercised by `try_delegate_execution()` in C++ (R6.4a
/// is stateless: any non-Stop outcome ⇒ drop handle + fall back).
/// R6.4b will use this pattern to keep the handle alive across
/// `cpu_task` re-entries.
#[test]
fn rust_spu_continue_then_resume_on_same_handle() {
    unsafe {
        let h = rust_spu_new();

        // Build a 4-instruction program: 3 × `il r3, 0` (no-op-ish)
        // + `stop 0xC4`. With `max_steps = 2`, the first run should
        // reach Continue. With another `max_steps = 10` on the same
        // handle, we must complete and hit Stop.
        let il_r3_0: u32 = (0x081_u32 << 23) | ((0u32 & 0xFFFF) << 7) | 3;
        let stop_c4: u32 = 0xC4_u32 & 0x3FFF;

        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&il_r3_0.to_be_bytes());  // pc=0
        bytes.extend_from_slice(&il_r3_0.to_be_bytes());  // pc=4
        bytes.extend_from_slice(&il_r3_0.to_be_bytes());  // pc=8
        bytes.extend_from_slice(&stop_c4.to_be_bytes());  // pc=12

        assert_eq!(rust_spu_load_ls(h, bytes.as_ptr(), bytes.len() as u32), 0);
        assert_eq!(rust_spu_set_pc(h, 0), 0);

        // First run: budget=2 should hit Continue with pc advanced.
        let mut code = 0u32;
        let mut steps = 0u32;
        let outcome = rust_spu_run_until_event(h, 2, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::Continue, "must hit budget");
        assert_eq!(steps, 2, "must report exactly the budget consumed");

        let mut pc1 = 0u32;
        assert_eq!(rust_spu_get_pc(h, &mut pc1), 0);
        assert_eq!(pc1, 8, "after 2 il instructions, pc must be 8");

        // Second run on the SAME handle: budget=10 picks up at pc=8
        // and completes the program.
        let outcome = rust_spu_run_until_event(h, 10, &mut code, &mut steps);
        assert_eq!(outcome, RustSpuOutcome::Stop, "must hit stop after resume");
        assert_eq!(code, 0xC4, "stop code carried across resume");
        // 1 il @ pc=8, then stop @ pc=12 → 2 steps consumed
        assert_eq!(steps, 2);

        let mut pc2 = 0u32;
        assert_eq!(rust_spu_get_pc(h, &mut pc2), 0);
        assert_eq!(pc2, 12, "final pc must be at the stop instruction");

        rust_spu_drop(h);
    }
}
