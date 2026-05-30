//! HLE backlog — cellMouse GetInfo. Boots `single_mouse_info_v1.self`
//! (ioMouseInit(127) -> ioMouseGetInfo). Headless = no host mouse → max=127,
//! connected=0.

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
    p.push("single_mouse_info_v1");
    p.push("single_mouse_info_v1.self");
    p
}

#[test]
fn mouse_get_info_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE mouse] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!("[HLE mouse] exit_status = {status} (0x{:08x})", status as u32);
    assert_eq!(status as u32, 0xC0DE, "got 0x{:08x}", status as u32);
}
