//! HLE wave — sys_net inet_addr routed to rpcs3-hle-sys-net-user.
//!
//! Boots `single_net_inet_addr_v1.self` (a PSL1GHT homebrew calling
//! `inet_addr("1.2.3.4")`) through `EmuCore::run_self`. On real PS3 firmware
//! sys_net_inet_addr is a STUB that unconditionally returns INET_ADDR_NONE
//! (0xFFFFFFFF); the crate mirrors this byte-exact. Pre-wire the return-0 stub
//! gives 0; once routed, r3=0xFFFFFFFF and the homebrew returns 1.
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
    p.push("single_net_inet_addr_v1");
    p.push("single_net_inet_addr_v1.self");
    p
}

#[test]
fn net_inet_addr_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE inet_addr] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE inet_addr] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: sys_net inet_addr returns INET_ADDR_NONE (0xFFFFFFFF) per the
    // firmware stub, so the homebrew returns 1. Pre-wire it was 0.
    assert_eq!(
        status, 1,
        "expected 1 (inet_addr==0xFFFFFFFF firmware stub); got {status} \
         (0 = unwired stub / inlined)"
    );
}
