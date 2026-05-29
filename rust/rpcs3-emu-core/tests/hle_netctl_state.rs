//! HLE wave — cellNetCtl init + get-state routed to rpcs3-hle-cellnetctl.
//!
//! Boots `single_netctl_state_v1.self` (a PSL1GHT homebrew calling
//! `cellNetCtlInit()` then `cellNetCtlGetState(&state)`) through
//! `EmuCore::run_self`. Stateful manager (`NetCtlManager` field) + a fixed
//! connected-network provider (`StubConnectedBackend`). Pre-wire both stubs
//! return 0 and never write the OUT pointer (state keeps its 0x55 sentinel);
//! once routed, the runtime reports IPOBTAINED (3).
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
    p.push("single_netctl_state_v1");
    p.push("single_netctl_state_v1.self");
    p
}

#[test]
fn netctl_init_get_state_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE netctl] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE netctl] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: cellNetCtlInit flips initialized; cellNetCtlGetState with the
    // staged StubConnectedBackend reports CELL_NET_CTL_STATE_IPOBTAINED (3),
    // written to the OUT pointer. Pre-wire it was 0x55 (sentinel, OUT untouched).
    assert_eq!(
        status, 3,
        "expected CELL_NET_CTL_STATE_IPOBTAINED (3); got {status} \
         (0x55 = unwired/OUT untouched, 0xBAD = call error)"
    );
}
