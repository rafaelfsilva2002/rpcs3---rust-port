//! HLE backlog — cellPad GetInfo2. Boots `single_pad_info_v1.self` (ioPadInit(7)
//! -> ioPadGetInfo2). Headless emu-core has no host pad handler, so the result
//! is deterministic: max=7, connected=0. Exit 0xC0DE iff both match.
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("hle");
    p.push("sources");
    p.push("single_pad_info_v1");
    p.push("single_pad_info_v1.self");
    p
}

#[test]
fn pad_get_info2_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE pad] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!("[HLE pad] exit_status = {status} (0x{:08x})", status as u32);

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (max==7 && connected==0); got 0x{:08x}",
        status as u32,
    );
}
