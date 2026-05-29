//! HLE wave — cellNetCtlGetInfo (MTU) routed to rpcs3-hle-cellnetctl.
//!
//! Boots `single_netctl_mtu_v1.self` (a PSL1GHT homebrew calling
//! `cellNetCtlInit()` then `cellNetCtlGetInfo(NET_CTL_INFO_MTU, &info)`) through
//! `EmuCore::run_self`. Reuses the NetCtlManager field + connected backend from
//! the get-state fixture; only the GetInfo NID is new. Pre-wire the return-0 stub
//! never writes the union (mtu=0); once routed, the crate returns Mtu(1500).
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
    p.push("single_netctl_mtu_v1");
    p.push("single_netctl_mtu_v1.self");
    p
}

#[test]
fn netctl_get_info_mtu_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE netctl-mtu] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE netctl-mtu] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: cellNetCtlGetInfo(MTU) returns the default MTU 1500, written as
    // BE u32 into the union (mtu @offset 0). Pre-wire it was 0 (union untouched).
    assert_eq!(
        status, 1500,
        "expected default MTU 1500; got {status} \
         (0 = unwired/OUT untouched, 0xBAD = call error)"
    );
}
