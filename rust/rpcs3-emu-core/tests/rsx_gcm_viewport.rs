//! R13.5d — REAL libgcm VIEWPORT state registers captured through the full
//! cellGcm path.
//!
//! Runs `single_gcm_viewport_v1.self` (rsxInit + `rsxSetViewport(0,0,640,480,…)`
//! + label), captures `[context.begin .. current)` from EmuCore memory. Unlike
//! the surface/texture/draw slices, the viewport is a *SetState* register group
//! NOT exposed in `RsxSnapshot` — so instead of `replay_gcm` we decode the
//! stream straight into an `RsxState` (`run_and_apply`) and read the
//! `SET_VIEWPORT_HORIZONTAL/VERTICAL` registers directly, validating their
//! encoding against REAL libgcm bytes.
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_fifo::FifoEngine;
use rpcs3_rsx_state::{RsxState, VIEWPORT_HORIZONTAL, VIEWPORT_VERTICAL};

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_viewport_v1");
    p.push("single_gcm_viewport_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn viewport_registers_captured_via_real_cellgcm_context() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.5d] skip: {} not present (build via Docker PSL1GHT)",
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
        "fixture must run rsxInit + rsxSetViewport + label to completion"
    );
    assert_ne!(core.gcm_context_addr, 0, "cellGcm HLE must have initialized the context");

    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    assert!(current > begin, "rsxSetViewport must have emitted commands");
    let put_bytes = current - begin;
    assert_eq!(put_bytes % 4, 0);

    let mut stream = vec![0u8; put_bytes as usize];
    core.mem
        .read(begin, &mut stream)
        .expect("read command buffer [begin..current)");

    // Viewport is SetState (not in RsxSnapshot) — decode the raw stream into an
    // RsxState and read the registers directly.
    let mut state = RsxState::new();
    let mut engine = FifoEngine::new(0, put_bytes);
    state
        .run_and_apply(&mut engine, &stream)
        .expect("apply real libgcm stream to RsxState");

    let h = state.read(VIEWPORT_HORIZONTAL);
    let v = state.read(VIEWPORT_VERTICAL);
    eprintln!(
        "[R13.5d] {} words: VIEWPORT_HORIZONTAL=0x{h:08x} VERTICAL=0x{v:08x}",
        put_bytes / 4,
    );

    // SET_VIEWPORT_HORIZONTAL/VERTICAL pack (origin | size<<16). 640x480 viewport
    // at origin (0,0): width/height in the high 16 bits. (Pinned after observing
    // the real libgcm encoding.)
    assert_eq!(h >> 16, 640, "viewport width (HORIZONTAL high 16)");
    assert_eq!(h & 0xFFFF, 0, "viewport x origin (HORIZONTAL low 16)");
    assert_eq!(v >> 16, 480, "viewport height (VERTICAL high 16)");
    assert_eq!(v & 0xFFFF, 0, "viewport y origin (VERTICAL low 16)");
}
