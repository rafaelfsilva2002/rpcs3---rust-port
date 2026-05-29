//! R13.5b — REAL libgcm INDEXED draw captured through the full cellGcm path.
//!
//! Runs `single_gcm_indexdraw_v1.self` (rsxInit + `rsxDrawIndexArray`
//! (GCM_TYPE_TRIANGLES, offset 0x10000, count 3, 16-bit indices, RSX) + label),
//! captures `[context.begin .. current)` from EmuCore memory, and decodes via
//! `replay_gcm`. Asserts the snapshot contains a `DrawKind::Indexed` DrawCall
//! plus the parsed `IndexArray` descriptor — first indexed-draw path validated
//! against REAL libgcm bytes (the draw oracle so far only covered
//! `DrawKind::Arrays`).
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
    p.push("single_gcm_indexdraw_v1");
    p.push("single_gcm_indexdraw_v1.self");
    p
}

fn read_be_u32(core: &EmuCore, addr: u32) -> u32 {
    let mut b = [0u8; 4];
    core.mem.read(addr, &mut b).expect("read gcm struct");
    u32::from_be_bytes(b)
}

#[test]
fn indexed_draw_captured_via_real_cellgcm_context() {
    let path = fixture_self();
    if !path.exists() {
        eprintln!(
            "[R13.5b] skip: {} not present (build via Docker PSL1GHT)",
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
        "fixture must run rsxInit + rsxDrawIndexArray + label to completion"
    );
    assert_ne!(core.gcm_context_addr, 0, "cellGcm HLE must have initialized the context");

    let begin = read_be_u32(&core, core.gcm_context_addr);
    let current = read_be_u32(&core, core.gcm_context_addr + 8);
    assert!(current > begin, "rsxDrawIndexArray must have emitted commands");
    let put_bytes = current - begin;
    assert_eq!(put_bytes % 4, 0);

    let mut stream = vec![0u8; put_bytes as usize];
    core.mem
        .read(begin, &mut stream)
        .expect("read command buffer [begin..current)");

    let snap = replay_gcm(&stream, put_bytes).expect("replay real libgcm stream");

    eprintln!(
        "[R13.5b] {} words: draw_calls={:?} index_array={:?}",
        put_bytes / 4,
        snap.draw_calls,
        snap.index_array,
    );

    assert!(
        !snap.draw_calls.is_empty(),
        "rsxDrawIndexArray must produce a DrawCall; effects={:?}",
        snap.effects
    );
    let draw = &snap.draw_calls[0];
    assert_eq!(draw.kind, DrawKind::Indexed, "indexed draw");
    assert_eq!(draw.primitive, 5, "GCM_TYPE_TRIANGLES = 5");
    assert!(
        draw.ranges.iter().any(|&(f, c)| f == 0 && c == 3),
        "expected (first=0, count=3) range; got {:?}",
        draw.ranges
    );

    // IndexArray descriptor, pinned against REAL libgcm output:
    assert_eq!(snap.index_array.address, 0x0001_0000, "SET_INDEX_ARRAY_ADDRESS");
    assert_eq!(snap.index_array.location, 0, "GCM_LOCATION_RSX = 0");
    assert_eq!(
        format!("{:?}", snap.index_array.index_type),
        "U16",
        "GCM_INDEX_TYPE_16B → U16 element type",
    );
}
