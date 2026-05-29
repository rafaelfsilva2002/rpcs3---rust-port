//! HLE wave — cellVideoOutGetState routed to rpcs3-hle-cellvideoout.
//!
//! Boots `single_videoout_state_v1.self` (a PSL1GHT homebrew calling
//! `cellVideoOutGetState(PRIMARY, 0, &state)`) through `EmuCore::run_self`. Sixth
//! cellVideoOut function — reuses the VideoOutManager field; only the GetState
//! NID is new. Pre-wire the return-0 stub never writes the struct; once routed,
//! the primary's state (enabled, 720p) lands.
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
    p.push("single_videoout_state_v1");
    p.push("single_videoout_state_v1.self");
    p
}

#[test]
fn videoout_get_state_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE videoout-state] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE videoout-state] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: the primary is enabled at 720p, so colorSpace=1 and
    // displayMode.resolution=2 => (1<<8)|2 = 0x102. Pre-wire it was 0.
    assert_eq!(
        status as u32,
        0x0102,
        "expected colorSpace+resolution 0x102; got 0x{:08x} \
         (0 = unwired stub, 0xBAD = call error)",
        status as u32,
    );
}
