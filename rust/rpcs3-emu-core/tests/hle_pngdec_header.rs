//! HLE backlog — cellPngDec header parse (callback-driven). Boots
//! `single_pngdec_header_v1.self` (Create -> Open(BUFFER) -> ReadHeader on an
//! embedded 320x240 RGB PNG; Create/Open drive the guest cbCtrlMalloc callback).
//! Exit 0xC0DE iff the parsed header is {320, 240, 3 comp, RGB, depth 8}.

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
    p.push("single_pngdec_header_v1");
    p.push("single_pngdec_header_v1.self");
    p
}

#[test]
fn pngdec_read_header_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE pngdec] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!("[HLE pngdec] exit_status = {status} (0x{:08x})", status as u32);

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (parsed PNG header 320x240/3/RGB/8); got 0x{:08x}",
        status as u32,
    );
}
