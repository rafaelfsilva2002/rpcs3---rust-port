//! HLE backlog — cellJpgDec header parse. Boots `single_jpgdec_header_v1.self`
//! (Create -> Open(BUFFER) -> ReadHeader on an embedded minimal JFIF). Exit
//! 0xC0DE iff the parsed header is {width 320, height 240, 3 comp, RGB}.
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
    p.push("single_jpgdec_header_v1");
    p.push("single_jpgdec_header_v1.self");
    p
}

#[test]
fn jpgdec_read_header_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[HLE jpgdec] skip: {} not present", path.display());
        return;
    }
    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;

    let report = core.run_self(&bytes).expect("run_self");
    let status = report.exit_status.status;
    eprintln!(
        "[HLE jpgdec] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    assert_eq!(
        status as u32,
        0xC0DE,
        "expected 0xC0DE (parsed JPEG header 320x240/3/RGB); got 0x{:08x}",
        status as u32,
    );
}
