//! HLE wave — cellVideoOutGetResolutionAvailability routed to rpcs3-hle-cellvideoout.
//!
//! Boots `single_videoout_resavail_v1.self` (a PSL1GHT homebrew calling
//! `cellVideoOutGetResolutionAvailability(PRIMARY, 720p, 16:9, 0)`) through
//! `EmuCore::run_self`. Third cellVideoOut function — introduces a stateful
//! `VideoOutManager` field whose default `supported_resolutions` includes 720p.
//! Pre-wire the return-0 stub gives 0; once routed, the primary port reports 1.
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
    p.push("single_videoout_resavail_v1");
    p.push("single_videoout_resavail_v1.self");
    p
}

#[test]
fn videoout_get_resolution_availability_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE videoout-resavail] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE videoout-resavail] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: 720p is in the default VideoOutManager supported set, so
    // availability = 1. Pre-wire it was 0 (the return-0 stub).
    assert_eq!(
        status, 1,
        "expected 720p availability 1; got {status} \
         (0 = unwired stub, error code = illegal param)"
    );
}
