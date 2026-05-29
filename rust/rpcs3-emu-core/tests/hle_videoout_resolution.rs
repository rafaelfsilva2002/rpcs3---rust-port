//! HLE wave — cellVideoOut routed to rpcs3-hle-cellvideoout.
//!
//! Boots `single_videoout_resolution_v1.self` (a PSL1GHT homebrew calling
//! `cellVideoOutGetResolution(VIDEO_RESOLUTION_720, &res)`) through
//! `EmuCore::run_self`. cellVideoOut's resolution lookup is STATELESS (id ->
//! width/height table), so no EmuCore state field is needed. Pre-wire the
//! return-0 stub never writes the struct (exit 0); once routed, the homebrew
//! reads the real 1280×720 and packs it into the exit code.
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
    p.push("single_videoout_resolution_v1");
    p.push("single_videoout_resolution_v1.self");
    p
}

#[test]
fn videoout_get_resolution_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE videoout] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE videoout] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: the dispatcher routes cellVideoOutGetResolution to
    // rpcs3-hle-cellvideoout (720p = 1280×720) and writes width/height (BE u16)
    // into the guest struct. The homebrew packs (width<<16)|height = 0x050002D0.
    // Pre-wire it was 0 (struct untouched).
    assert_eq!(
        status as u32,
        0x0500_02D0,
        "expected packed 1280×720 (0x050002D0); got 0x{:08x} \
         (0 = unwired stub, 0xBAD = call error)",
        status as u32,
    );
}
