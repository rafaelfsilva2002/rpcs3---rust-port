//! R13.5e — REAL libgcm `TextureDescriptor` captured through the full cellGcm
//! path.
//!
//! Runs `single_gcm_texture_v1.self` (rsxInit + `rsxLoadTexture` of a 256x128
//! A8R8G8B8 texture into unit 0 at offset 0x200000 + `rsxTextureControl` enable
//! + frame label), captures `[context.begin .. current)` from EmuCore memory,
//! and decodes via `replay_gcm`. Asserts `RsxSnapshot.textures[0]` reflects the
//! `gcmTexture` the fixture set — second Camada-B descriptor struct validated
//! against REAL libgcm bytes (after the surface descriptor in R13.5c).
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_state::{replay_gcm, TextureDimension};

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_texture_v1");
    p.push("single_gcm_texture_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn texture_descriptor_captured_via_real_cellgcm_context() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.5e] skip: {} not present (build via Docker PSL1GHT)",
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
        "fixture must run rsxInit + rsxLoadTexture + control + label to completion"
    );
    assert_ne!(core.gcm_context_addr, 0, "cellGcm HLE must have initialized the context");

    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    assert!(current > begin, "rsxLoadTexture must have emitted commands");
    let put_bytes = current - begin;
    assert_eq!(put_bytes % 4, 0);

    let mut stream = vec![0u8; put_bytes as usize];
    core.mem
        .read(begin, &mut stream)
        .expect("read command buffer [begin..current)");

    let snap = replay_gcm(&stream, put_bytes).expect("replay real libgcm stream");

    assert!(
        !snap.textures.is_empty(),
        "rsxLoadTexture must populate a TextureDescriptor; textures={:?}",
        snap.textures
    );
    let (unit, t) = snap.textures[0];
    eprintln!("[R13.5e] {} words: texture unit {} = {:?}", put_bytes / 4, unit, t);

    assert_eq!(unit, 0, "texture bound to unit 0");
    // Directly-mapped fields (gcmTexture -> SET_TEXTURE_* methods -> descriptor):
    assert_eq!(t.width, 256, "IMAGE_RECT width");
    assert_eq!(t.height, 128, "IMAGE_RECT height");
    assert_eq!(t.offset, 0x0020_0000, "SET_TEXTURE_OFFSET");
    assert_eq!(t.dimension, TextureDimension::TwoD, "GCM_TEXTURE_DIMS_2D");
    assert!(!t.cubemap, "cubemap was GCM_FALSE");
    // Format-register sub-fields, pinned against REAL libgcm output:
    assert_eq!(t.format_code, 0xA5, "A8R8G8B8 (0x85) + GCM_TEXTURE_FORMAT_LIN/NRM bits");
    assert_eq!(t.location, 1, "GCM_LOCATION_RSX = DMA-context code 1");
    assert_eq!(t.mipmap_levels, 1, "mipmap = 1 level");
    // border (FORMAT bit 3) reads true — set by the GCM_TEXTURE_FORMAT flags;
    // pinned as the faithful decode of the real libgcm bytes.
    assert!(t.border, "FORMAT bit 3 set by the GCM_TEXTURE_FORMAT flags");
}
