//! Guest-PPU callback oracle — cellSysutil RegisterCallback + CheckCallback
//! driving `EmuCore::call_guest_function` against a real PSL1GHT homebrew.
//!
//! The first behavior-freeze fixture where emu-core CALLS BACK INTO guest code.
//! The test pre-seeds one pending system event (status 0x0101) — a deterministic
//! stand-in for the system event source — then boots the homebrew, which
//! registers a callback and calls CheckCallback. emu-core drains the event and
//! invokes the guest callback, which records the status; main returns 0x600D iff
//! the callback ran with the right status (0xBAD0 otherwise).
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_sysutil_callback_v1");
    p.push("single_sysutil_callback_v1.self");
    p
}

#[test]
fn sysutil_callback_invoked_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE sysutil-cb] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    // Pre-seed one pending system event (status = 0x0101). Deterministic
    // stand-in for the real system event source; persists into the run.
    core.sysutil_queue.push(0x0101, 0);

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE sysutil-cb] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // 0x600D = the guest callback ran and observed status 0x0101 (proving the
    // HLE -> guest -> HLE re-entry). 0xBAD0 = callback never invoked.
    assert_eq!(
        status as u32,
        0x600D,
        "expected 0x600D (guest callback invoked with status 0x0101); got 0x{:08x}",
        status as u32,
    );
}

/// Negative control — SAME binary, but the host does NOT pre-seed an event.
/// CheckCallback drains an empty queue, so the guest callback never runs and
/// `g_observed` stays 0 -> main returns 0xBAD0. Proves the 0x600D above came
/// from the re-entrant dispatch, not from the binary unconditionally succeeding.
#[test]
fn sysutil_callback_not_invoked_without_event() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE sysutil-cb-neg] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    // No pre-seeded event: the queue is empty.

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE sysutil-cb-neg] exit_status = {status} (0x{:08x})",
        status as u32,
    );
    assert_eq!(
        status as u32,
        0xBAD0,
        "expected 0xBAD0 (no event -> callback never runs); got 0x{:08x}",
        status as u32,
    );
}
