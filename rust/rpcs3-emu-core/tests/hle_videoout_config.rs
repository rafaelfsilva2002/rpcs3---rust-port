//! HLE wave — cellVideoOutGetConfiguration routed to rpcs3-hle-cellvideoout.
//!
//! Boots `single_videoout_config_v1.self` (a PSL1GHT homebrew calling
//! `cellVideoOutGetConfiguration(PRIMARY, &cfg, NULL)`) through
//! `EmuCore::run_self`. Fourth cellVideoOut function — reuses the VideoOutManager
//! field; only the GetConfiguration NID is new. Pre-wire the return-0 stub never
//! writes the struct (resolution=0); once routed, the primary config (720p)
//! lands and cfg.resolution = 2.
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
    p.push("single_videoout_config_v1");
    p.push("single_videoout_config_v1.self");
    p
}

#[test]
fn videoout_get_configuration_via_real_homebrew() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[HLE videoout-config] skip: {} not present (build via Docker PSL1GHT)",
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
        "[HLE videoout-config] exit_status = {status} (0x{:08x})",
        status as u32,
    );

    // Post-wire: the primary port's default config is 720p, so cfg.resolution =
    // VIDEO_RESOLUTION_720 (2). Pre-wire it was 0 (struct untouched).
    assert_eq!(
        status, 2,
        "expected 720p resolution id (2); got {status} \
         (0 = unwired stub, 0xBAD = call error)"
    );
}
