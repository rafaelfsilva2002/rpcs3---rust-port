//! R13.5a — REAL libgcm MULTIPLE DrawCalls captured through the full cellGcm
//! path.
//!
//! Runs `single_gcm_multidraw_v1.self` (rsxInit + two `rsxDrawVertexArray`
//! calls: TRIANGLES 0..3 then TRIANGLES 10..6 + label), captures
//! `[context.begin .. current)` from EmuCore memory, and decodes via
//! `replay_gcm`. Asserts the snapshot contains TWO distinct `DrawCall` records —
//! the draw oracle so far only ever produced one, so this validates the
//! multi-draw path against REAL libgcm bytes.
//!
//! Skips gracefully when the `.self` is absent (gitignored; built via Docker).

use std::path::PathBuf;

use rpcs3_emu_core::EmuCore;
use rpcs3_rsx_state::{replay_gcm, DrawKind};

fn fixture_self() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // rust/
    p.pop(); // rpcs3-master/
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("rsx");
    p.push("sources");
    p.push("single_gcm_multidraw_v1");
    p.push("single_gcm_multidraw_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn multiple_draws_captured_via_real_cellgcm_context() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.5a] skip: {} not present (build via Docker PSL1GHT)",
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
        "fixture must run rsxInit + two draws + label to completion"
    );
    assert_ne!(core.gcm_context_addr, 0, "cellGcm HLE must have initialized the context");

    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    assert!(current > begin, "two draws must have emitted commands");
    let put_bytes = current - begin;
    assert_eq!(put_bytes % 4, 0);

    let mut stream = vec![0u8; put_bytes as usize];
    core.mem
        .read(begin, &mut stream)
        .expect("read command buffer [begin..current)");

    let snap = replay_gcm(&stream, put_bytes).expect("replay real libgcm stream");

    eprintln!(
        "[R13.5a] {} words: draw_calls={:?}",
        put_bytes / 4,
        snap.draw_calls,
    );

    assert_eq!(
        snap.draw_calls.len(),
        2,
        "two rsxDrawVertexArray calls must produce TWO DrawCalls; got {:?}",
        snap.draw_calls
    );
    for d in &snap.draw_calls {
        assert_eq!(d.primitive, 5, "GCM_TYPE_TRIANGLES = 5");
        assert_eq!(d.kind, DrawKind::Arrays, "non-indexed draw");
    }
    assert!(
        snap.draw_calls[0].ranges.iter().any(|&(f, c)| f == 0 && c == 3),
        "first draw range (0,3); got {:?}",
        snap.draw_calls[0].ranges
    );
    assert!(
        snap.draw_calls[1].ranges.iter().any(|&(f, c)| f == 10 && c == 6),
        "second draw range (10,6); got {:?}",
        snap.draw_calls[1].ranges
    );
}
