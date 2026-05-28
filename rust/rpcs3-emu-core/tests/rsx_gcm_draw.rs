//! R13.3 — first REAL libgcm DRAW_ARRAYS captured through the full
//! cellGcm path.
//!
//! Runs `single_gcm_draw_v1.self`, which extends the R13.2 fixture
//! (`single_gcm_emit_v1`) by adding a single
//! `rsxDrawVertexArray(ctx, GCM_TYPE_TRIANGLES, 0, 3)` call. PSL1GHT
//! librsx expands this inline into the NV4097_SET_BEGIN_END(5) +
//! NV4097_DRAW_ARRAYS + NV4097_SET_BEGIN_END(0) sequence; the
//! DrawTracker in `rpcs3-rsx-state` recognises that pattern as a
//! complete `DrawCall`.
//!
//! We capture `[context.begin .. context.current)` straight from
//! EmuCore memory (the R12.11a path) and assert the snapshot decodes
//! to a non-empty `draw_calls` list — the load-bearing assertion that
//! the FULL command-stream layer (decode → state → DrawTracker) now
//! cycles cleanly on REAL libgcm bytes coming through the REAL
//! cellGcm-init'd context.
//!
//! Skips gracefully when the `.self` is absent.

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_state::{replay_gcm, DrawKind, MethodEffect};

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_draw_v1");
    p.push("single_gcm_draw_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn draw_call_captured_via_real_cellgcm_context() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.3] skip: {} not present (build via Docker PSL1GHT)",
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
        "fixture must run rsxInit + clear + draw + label to completion"
    );

    assert_ne!(
        core.gcm_context_addr, 0,
        "cellGcm HLE must have initialized the context"
    );

    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    assert!(
        current > begin,
        "rsxDrawVertexArray must have emitted commands beyond just clear"
    );
    let put_bytes = current - begin;
    assert_eq!(put_bytes % 4, 0);

    let mut stream = vec![0u8; put_bytes as usize];
    core.mem
        .read(begin, &mut stream)
        .expect("read command buffer [begin..current)");

    let snap = replay_gcm(&stream, put_bytes).expect("replay real libgcm stream");

    // ClearSurface(0xF3) still present (came before the draw).
    assert!(
        snap.effects
            .iter()
            .any(|e| matches!(e, MethodEffect::ClearSurface(0xF3))),
        "stream should still contain ClearSurface(0xF3); effects={:?}",
        snap.effects
    );

    // NEW: a real DrawCall produced by rsxDrawVertexArray —
    // primitive = GCM_TYPE_TRIANGLES (5), kind = Arrays,
    // ranges contains (first=0, count=3).
    assert!(
        !snap.draw_calls.is_empty(),
        "rsxDrawVertexArray must produce a DrawCall in the snapshot; \
         effects={:?}",
        snap.effects
    );
    let draw = &snap.draw_calls[0];
    assert_eq!(draw.primitive, 5, "GCM_TYPE_TRIANGLES = 5");
    assert_eq!(draw.kind, DrawKind::Arrays);
    assert!(
        draw.ranges.iter().any(|&(f, c)| f == 0 && c == 3),
        "expected (first=0, count=3) range; got {:?}",
        draw.ranges
    );

    eprintln!(
        "[R13.3] captured {} words ({} bytes) from REAL cellGcm context \
         @ begin=0x{:08x} current=0x{:08x}; effects={} draw_calls={} \
         first_draw={{primitive={}, kind={:?}, ranges={:?}}}",
        put_bytes / 4,
        put_bytes,
        begin,
        current,
        snap.effects.len(),
        snap.draw_calls.len(),
        draw.primitive,
        draw.kind,
        draw.ranges,
    );
}
