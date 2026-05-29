//! Guest-PPU callback #2 — cellMsgDialogOpen2 headless-auto-confirm invoking a
//! guest callback via `EmuCore::call_guest_function`.
//!
//! The homebrew opens an OK message dialog with a callback. With no user,
//! emu-core auto-confirms and invokes the guest callback with the default button
//! (OK = 1); the callback records it and main returns 0x600D (0xBAD0 if it never
//! ran). Second API proving the guest-callback unlock.
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
    p.push("single_msgdialog_callback_v1");
    p.push("single_msgdialog_callback_v1.self");
    p
}

#[test]
fn msgdialog_callback_invoked_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE msgdialog-cb] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE msgdialog-cb] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // 0x600D = the dialog callback ran with the OK button (proving HLE -> guest
    // re-entry via cellMsgDialog). 0xBAD0 = callback never invoked.
    assert_eq!(
        status as u32,
        0x600D,
        "expected 0x600D (msgDialog callback invoked with OK); got 0x{:08x}",
        status as u32,
    );
}
