//! R13.2 — REAL libgcm capture through the cellGcm-init'd context.
//!
//! Runs `single_gcm_emit_v1.self` which calls the real `rsxInit` (now
//! unblocked by R13.1's cellGcm HLE) and then emits commands via
//! PSL1GHT librsx (`rsxSetClearColor` / `rsxClearSurface` /
//! `rsxSetWriteCommandLabel`) into the cellGcm-init'd context. We then
//! read `[begin .. current)` straight out of EmuCore guest memory —
//! the R12.11a `capture_command_buffer` path applied to the REAL
//! cellGcm context — and decode it with `rpcs3_rsx_state::replay_gcm`.
//!
//! Difference from R12.11b (`rsx_capture_smoke.rs`): that fixture set
//! up a `gcmContextData` BY HAND over a static buffer to dodge the
//! cellGcm HLE, and dumped the words via `sysTtyWrite`. R13.2 uses the
//! REAL cellGcm init path AND reads the context-buffer directly from
//! emulator memory — closing the R12.11b → R13 advance: same byte
//! origin (real PSL1GHT librsx), now through the FULL cellGcm path.
//!
//! Skips gracefully when the `.self` is absent (built locally via the
//! Docker PSL1GHT toolchain).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_state::{replay_gcm, MethodEffect};

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_emit_v1");
    p.push("single_gcm_emit_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn emitted_stream_captured_via_real_cellgcm_context() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.2] skip: {} not present (build via Docker PSL1GHT)",
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
        "fixture must run rsxInit + emission to completion and return 0xC0DE"
    );

    // The cellGcm HLE (R13.1) must have populated the context pointer.
    assert_ne!(
        core.gcm_context_addr, 0,
        "cellGcm HLE must have initialized the context"
    );

    // CellGcmContextData layout (GCM.h:26): begin / end / current /
    // callback (4 BE u32). PSL1GHT librsx writes inline NV4097 words to
    // *context->current and bumps context->current.
    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    assert!(
        current >= begin,
        "context.current ({current:#x}) must be >= begin ({begin:#x})"
    );

    let put_bytes = current - begin;
    assert!(
        put_bytes > 0,
        "REAL libgcm emission must have advanced context.current past \
         begin (begin={begin:#x} current={current:#x}) — empty stream \
         means the emission path didn't write"
    );
    assert_eq!(put_bytes % 4, 0, "GCM stream must be whole 32-bit words");

    // Read the live stream straight out of guest memory (R12.11a path).
    let mut stream = vec![0u8; put_bytes as usize];
    core.mem
        .read(begin, &mut stream)
        .expect("read command buffer [begin..current)");

    // Decode the REAL captured stream through the frozen decoder.
    let snap = replay_gcm(&stream, put_bytes).expect("replay real libgcm stream");

    // PSL1GHT's `rsxClearSurface(ctx, 0xF3)` emits NV4097_CLEAR_SURFACE
    // with the 0xF3 mask (color + depth + stencil) — the load-bearing
    // assertion: the REAL libgcm bytes coming out of the REAL cellGcm
    // context decode to the expected effect.
    assert!(
        snap.effects
            .iter()
            .any(|e| matches!(e, MethodEffect::ClearSurface(0xF3))),
        "REAL libgcm stream should contain ClearSurface(0xF3); effects={:?}",
        snap.effects
    );

    eprintln!(
        "[R13.2] captured {} words ({} bytes) from REAL cellGcm context \
         @ begin=0x{:08x} current=0x{:08x}; effects={} draw_calls={}",
        put_bytes / 4,
        put_bytes,
        begin,
        current,
        snap.effects.len(),
        snap.draw_calls.len()
    );
}
