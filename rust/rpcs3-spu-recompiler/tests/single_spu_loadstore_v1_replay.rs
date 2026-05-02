//! R5.11b — fourth replay-validated fixture integration test.
//!
//! Loads `behavior-freeze/fixtures/spu/traces/single_spu_loadstore_v1.jsonl`
//! and the corresponding `.spuimg` side-file, runs the full pipeline
//! (parser → per-SPU transformer → builder → replay × Interpreter +
//! replay × Recompiler), and asserts byte-identical agreement.
//!
//! The fixture exercises the **SPU Local Store load/store path** —
//! `stqd`/`lqd` against r1-relative offsets via a stack-allocated
//! `volatile uint32_t buffer[8]`. GCC -O2 emits the standard
//! Cell BE pattern for runtime-indexed slot access:
//! `lqd` → `cwd` → `shufb` → `stqd` for stores; `lqd` → `rotqby`
//! (RR-form, the byte count comes from a register) for loads.
//!
//! R5.11b co-landed the interpreter implementation of `rotqby`
//! (opcode 0x1DC, RR-form sibling of the already-implemented
//! `rotqbyi` at 0x1FC) — a textbook Cell BE op needed by any
//! GCC-emitted runtime-indexed quadword extraction. The fix is
//! GENERAL, not single-fixture; it stays landed for any future
//! signal-bearing or array-bearing fixture and for R6.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    build_spu_program_from_captured_image, captured_events_to_traces_per_spu, diff_snapshots,
    parse_jsonl_trace, replay_per_spu_traces, replay_per_spu_traces_with, CapturedEvent,
    InterpreterExecutor, SpuImageEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

fn fixture_trace_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("spu");
    p.push("traces");
    p.push("single_spu_loadstore_v1.jsonl");
    p
}

fn fixture_images_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("spu");
    p.push("images");
    p
}

#[test]
fn r5_11b_single_spu_loadstore_v1_replay_validated_byte_identical() {
    // ===== 1. Fixture must exist on disk. =====
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();

    assert!(
        trace_path.exists(),
        "fixture trace not found at {}",
        trace_path.display(),
    );
    assert!(
        images_dir.exists(),
        "fixture images dir not found at {}",
        images_dir.display(),
    );

    // ===== 2. Parse the JSONL trace. =====
    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("parser must accept the captured trace");
    assert!(!events.is_empty(), "trace has no events");

    // ===== 3. Verify acceptance criteria. =====
    // No DMA.
    let mfc_cmd_events: Vec<_> = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuWrch(w) if w.channel == 21))
        .collect();
    assert!(
        mfc_cmd_events.is_empty(),
        "fixture must NOT contain any spu_wrch ch21 (MFC_Cmd) events; \
         found {} — fixture would be DMA-bound",
        mfc_cmd_events.len(),
    );

    // ≥ 1 OUT_MBOX write — the LS-checksum result.
    let out_mbox_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuWrch(w) if w.channel == 28))
        .count();
    assert!(
        out_mbox_count >= 1,
        "fixture must have ≥1 spu_wrch ch28 (OUT_MBOX) event; got {out_mbox_count}",
    );

    // ===== 4. Per-SPU transformer must produce exactly 1 group. =====
    let groups: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("per-SPU transform must succeed");
    assert_eq!(
        groups.len(),
        1,
        "fixture must have exactly 1 target_spu (got {})",
        groups.len(),
    );

    // ===== 5. Locate the spu_image event. =====
    let images: Vec<&SpuImageEvent> = events
        .iter()
        .filter_map(|ev| {
            if let CapturedEvent::SpuImage(img) = ev {
                Some(img)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        images.len(),
        1,
        "fixture must have exactly 1 spu_image event (got {})",
        images.len(),
    );
    let image = images[0];

    // ===== 6. Build SpuProgram. =====
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    assert!(
        image_path.exists(),
        ".spuimg side-file missing at {} (sha={})",
        image_path.display(),
        image.image_sha256,
    );
    let program = build_spu_program_from_captured_image(&image_path, image, 100_000_000)
        .expect("builder must succeed");

    // ===== 7. Replay × Interpreter. =====
    let mut programs = BTreeMap::new();
    programs.insert(*groups.keys().next().unwrap(), program.clone());

    let interp_reports = replay_per_spu_traces::<InterpreterExecutor>(&groups, &programs)
        .expect("replay × Interpreter must succeed");
    let interp = interp_reports.values().next().unwrap();

    // ===== 8. Replay × Recompiler. =====
    let jit_reports =
        replay_per_spu_traces_with(&groups, &programs, |_| RecompilerExecutor::new())
            .expect("replay × Recompiler must succeed");
    let jit = jit_reports.values().next().unwrap();

    // ===== 9. Byte-identical agreement. =====
    assert_eq!(
        format!("{:?}", interp.final_event_kind),
        format!("{:?}", jit.final_event_kind),
        "Interpreter vs Recompiler final_event_kind diverged",
    );
    assert_eq!(
        interp.records.len(),
        jit.records.len(),
        "record count diverged",
    );
    assert!(
        interp.total_steps > 0 && jit.total_steps > 0,
        "Both backends must report >0 steps; got interp={} jit={}",
        interp.total_steps, jit.total_steps,
    );
    let diff = diff_snapshots(&interp.final_snapshot, &jit.final_snapshot);
    assert!(
        diff.is_identical(),
        "Interpreter vs Recompiler final_snapshot diverged: {diff:?}",
    );

    eprintln!(
        "[R5.11b SUCCESS] single_spu_loadstore_v1 replay-validated:\n  \
         target_spu={}\n  \
         events={} (out_mbox_count={})\n  \
         spu_image sha={}\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         Final-snapshot diff: identical",
        groups.keys().next().unwrap(),
        events.len(),
        out_mbox_count,
        image.image_sha256,
        interp.total_steps,
        jit.total_steps,
    );
}
