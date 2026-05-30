//! GPU backend — REAL libgcm surface + clear, executed into a framebuffer.
//!
//! Runs `single_gcm_clearsurf_v1.self` (rsxSetSurface 16x16 A8R8G8B8 ->
//! rsxSetClearColor(0xAABBCCDD) -> rsxClearSurface(0xF3)), reads the captured
//! NV4097 stream from the cellGcm context (R13.2 path), decodes it with
//! `replay_gcm`, then runs `rpcs3_rsx_render::execute_clear` over the decoded
//! state and checks the rendered framebuffer pixel-for-pixel. A clear writes a
//! constant color, so the rendered surface is byte-exact with RPCS3.
//!
//! Skips gracefully when the `.self` is absent (built via Docker PSL1GHT).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_render::execute_clear;
use rpcs3_rsx_state::replay_gcm_state;

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_clearsurf_v1");
    p.push("single_gcm_clearsurf_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn real_clear_renders_byte_exact_framebuffer() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!("[rsx-clearsurf] skip: {} not present", path.display());
        return;
    }

    let bytes = std::fs::read(&path).expect("read .self");
    let mut core = EmuCore::new();
    core.step_budget = 5_000_000;
    core.permissive_unknown_syscalls = false;
    let report = core.run_self(&bytes).expect("run_self");
    assert_eq!(report.exit_status.status, 0xC0DE, "fixture must reach 0xC0DE");
    assert_ne!(core.gcm_context_addr, 0, "cellGcm context must be initialized");

    // Capture [begin..current) from the real cellGcm context (R13.2 path).
    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    let put = current - begin;
    assert!(put > 0 && put % 4 == 0, "non-empty whole-word stream");
    let mut stream = vec![0u8; put as usize];
    core.mem.read(begin, &mut stream).expect("read command buffer");

    // Decode the REAL libgcm stream -> final RSX register state.
    let state = replay_gcm_state(&stream, put).expect("replay libgcm stream");
    let surface = state.surface();
    assert_eq!(surface.color_format, 8, "A8R8G8B8");
    assert_eq!(surface.clip, (16, 16), "16x16 surface clip");
    assert_eq!(surface.color_offset[0], 0);
    assert_eq!(surface.color_pitch[0], 64);
    assert_eq!(state.clear_surface_mask(), 0xF3, "ClearSurface(color+Z+S)");
    assert_eq!(state.color_clear_value(), 0xAABB_CCDD);

    // Execute the clear into a 16x16x4 reference framebuffer.
    let mut fb = vec![0u8; 16 * 16 * 4];
    let painted = execute_clear(&state, &mut fb);
    assert_eq!(painted, 256, "16x16 = 256 pixels cleared");

    // Byte-exact: every pixel is the clear color (A8R8G8B8 big-endian).
    for px in fb.chunks_exact(4) {
        assert_eq!(px, [0xAA, 0xBB, 0xCC, 0xDD], "clear color 0xAABBCCDD");
    }
    eprintln!("[rsx-clearsurf] rendered 16x16 framebuffer; 256 px == 0xAABBCCDD");
}
