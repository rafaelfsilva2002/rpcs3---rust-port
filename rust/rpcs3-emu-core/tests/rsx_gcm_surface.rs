//! R13.5c — REAL libgcm `SurfaceDescriptor` captured through the full cellGcm
//! path.
//!
//! Runs `single_gcm_surface_v1.self` (rsxInit + `rsxSetSurface` of a 640x480
//! A8R8G8B8 color / Z24S8 depth surface with distinctive offsets + pitches +
//! a frame label), captures `[context.begin .. current)` straight from EmuCore
//! memory (the R12.11a path), and decodes via `replay_gcm`. Asserts the
//! `RsxSnapshot.surface` descriptor reflects the `gcmSurface` the fixture set —
//! the first validation of a whole Camada-B descriptor struct against REAL
//! libgcm bytes through the real cellGcm-init'd context.
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_state::replay_gcm;

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_surface_v1");
    p.push("single_gcm_surface_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn surface_descriptor_captured_via_real_cellgcm_context() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.5c] skip: {} not present (build via Docker PSL1GHT)",
            path.display()
        );
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let report = core.run_self(&bytes).expect("run_self");

    assert_eq!(
        report.exit_status.status, 0xC0DE,
        "fixture must run rsxInit + rsxSetSurface + label to completion"
    );
    assert_ne!(core.gcm_context_addr, 0, "cellGcm HLE must have initialized the context");

    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    assert!(current > begin, "rsxSetSurface must have emitted commands");
    let put_bytes = current - begin;
    assert_eq!(put_bytes % 4, 0);

    let mut stream = vec![0u8; put_bytes as usize];
    core.mem
        .read(begin, &mut stream)
        .expect("read command buffer [begin..current)");

    let snap = replay_gcm(&stream, put_bytes).expect("replay real libgcm stream");
    let s = snap.surface;

    eprintln!(
        "[R13.5c] {} words: surface = {:?}",
        put_bytes / 4,
        s,
    );

    // Directly-mapped fields (gcmSurface -> SET_SURFACE_* methods -> descriptor):
    assert_eq!(s.clip, (640, 480), "surface clip should be width x height");
    assert_eq!(s.color_offset[0], 0x0001_0000, "color slot A offset");
    assert_eq!(s.color_pitch[0], 2560, "color slot A pitch (640*4)");
    assert_eq!(s.zeta_offset, 0x0020_0000, "zeta (depth) offset");
    assert_eq!(s.zeta_pitch, 2560, "zeta pitch (640*4)");
    assert_eq!(
        s.targets.active_count(),
        1,
        "GCM_SURFACE_TARGET_0 = exactly 1 active color slot"
    );
    // Format codes (GCM enum-encoded), confirmed against real libgcm output:
    assert_eq!(s.color_format, 8, "GCM_SURFACE_A8R8G8B8 = color format 8");
    assert_eq!(s.depth_format, 2, "GCM_SURFACE_ZETA_Z24S8 = depth format 2");
    assert_eq!(s.antialias, 0, "GCM_SURFACE_CENTER_1 = antialias 0");
}
