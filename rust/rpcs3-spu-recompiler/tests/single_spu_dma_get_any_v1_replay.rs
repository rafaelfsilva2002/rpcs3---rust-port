//! R8.3a — first ANY-wait-mode replay-validated fixture (10th
//! oracle). Mirrors R8.2 multi-DMA (two queued GETs, distinct
//! tags 3 + 5, distinct EAs / sizes 128 + 64 / LSAs 0x10000 +
//! 0x10100) but uses `WrTagUpdate = ANY` (= 1) instead of ALL
//! (= 2). The SPU embeds the actual `RdTagStat` returned value
//! into the canonical OUT_MBOX status via
//! `(tag_stat << 24) ^ 0xBEEFBEAD`, making the oracle robust
//! to any backend choice of ch24 return.
//!
//! Loads `behavior-freeze/fixtures/spu/traces/
//! single_spu_dma_get_any_v1.jsonl`. Captured `ch24 = 0x28`
//! (RPCS3 sync DMA returns the full mask in ANY mode).
//! Canonical status `0x892FAE2D`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, CapturedEvent, InterpreterExecutor,
    SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

/// Canonical multi-GET ANY-mode OUT_MBOX status from the
/// captured trace. Derived as
/// `((sum1 << 16) | sum2) ^ (tag_stat << 24) ^ 0xBEEFBEAD`
/// where sum1 = 0x1FC0, sum2 = 0x1080, tag_stat = 0x28
/// (captured RPCS3 sync-DMA ANY return).
const CANONICAL_STATUS: u32 = 0x892F_AE2D;

/// Captured ch24 returned value (the load-bearing R8.3a
/// invariant — encodes whatever the backend's ANY semantics
/// produced for this trace).
const CAPTURED_TAG_STAT: u32 = 0x28;

const FIXTURE_NAME: &str = "single_spu_dma_get_any_v1";

const TAG_1: u32 = 3;
const TAG_2: u32 = 5;
const SIZE_1: u32 = 128;
const SIZE_2: u32 = 64;
const LSA_1: u32 = 0x10000;
const LSA_2: u32 = 0x10100;
const WAIT_MASK: u32 = (1u32 << TAG_1) | (1u32 << TAG_2); // = 0x28
const ANY_MODE: u32 = 1;

fn fixture_trace_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("spu");
    p.push("traces");
    p.push(format!("{FIXTURE_NAME}.jsonl"));
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

fn fixture_dma_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("spu");
    p.push("dma");
    p
}

#[test]
fn r8_3a_single_spu_dma_get_any_v1_replay_validated_byte_identical() {
    // ===== 1. Fixture artifacts must exist on disk. =====
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(
        trace_path.exists(),
        "fixture trace not found at {}",
        trace_path.display(),
    );
    assert!(images_dir.exists(), "fixture images dir missing");
    assert!(dma_dir.exists(), "fixture DMA dir missing");

    // ===== 2. Parse the JSONL trace. =====
    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("parser must accept the captured trace");
    assert!(!events.is_empty(), "trace has no events");

    // ===== 3. R8.3a acceptance criteria. =====
    // Exactly 2 spu_mfc_cmd events with cmd=0x40 (GET).
    let mfc_cmd_events: Vec<&SpuMfcCmdEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuMfcCmd(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(mfc_cmd_events.len(), 2);

    let cmd1 = mfc_cmd_events[0];
    assert_eq!(cmd1.cmd, 0x40);
    assert_eq!(cmd1.tag, TAG_1);
    assert_eq!(cmd1.size, SIZE_1);
    assert_eq!(cmd1.lsa, LSA_1);

    let cmd2 = mfc_cmd_events[1];
    assert_eq!(cmd2.cmd, 0x40);
    assert_eq!(cmd2.tag, TAG_2);
    assert_eq!(cmd2.size, SIZE_2);
    assert_eq!(cmd2.lsa, LSA_2);

    assert_ne!(cmd1.eal, cmd2.eal);
    assert_ne!(cmd1.ea_chunk_sha256, cmd2.ea_chunk_sha256);

    // Exactly 2 mfc_dma_complete events.
    let dma_completes: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::MfcDmaComplete(c) => Some((c.tag, c.transferred_bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(dma_completes.len(), 2);
    assert!(dma_completes.contains(&(TAG_1, SIZE_1)));
    assert!(dma_completes.contains(&(TAG_2, SIZE_2)));

    // Exactly 1 ch28 wrch with the canonical status.
    let out_mbox_events: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 28 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(out_mbox_events, vec![CANONICAL_STATUS]);

    // Exactly 1 spu_stop with code 0x101.
    let stop_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuStop(s) if s.stop_code == 0x101))
        .count();
    assert_eq!(stop_count, 1);

    // **Load-bearing R8.3a invariants:** ch22, ch23, ch24.
    let wrch_22: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 22 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(wrch_22, vec![WAIT_MASK], "ch22 = 0x28");

    let wrch_23: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 23 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(
        wrch_23,
        vec![ANY_MODE],
        "ch23 must be 1 (ANY mode) — the load-bearing R8.3a invariant",
    );

    let rdch_24: Vec<Option<u32>> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuRdch(r) if r.channel == 24 => Some(r.value),
            _ => None,
        })
        .collect();
    assert_eq!(
        rdch_24,
        vec![Some(CAPTURED_TAG_STAT)],
        "ch24 ANY-mode return MUST match the captured canonical \
         (RPCS3 sync DMA returns full mask 0x28). A different value \
         here means a backend semantic change — re-document the canonical \
         in the .notes.md before bumping this expectation",
    );

    // ===== 4. Per-SPU transformer. =====
    let groups: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("per-SPU transform must succeed");
    assert_eq!(groups.len(), 1);
    let target_spu = *groups.keys().next().unwrap();

    // ===== 5. Locate the spu_image event. =====
    let images: Vec<&SpuImageEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuImage(img) => Some(img),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 1);
    let image = images[0];

    // ===== 6. Build SpuProgram, seed r3 = EA1 + r4 = EA2. =====
    let r3_initial: u128 = (cmd1.eal as u128) << 64;
    let r4_initial: u128 = (cmd2.eal as u128) << 64;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    assert!(image_path.exists(), ".spuimg missing");
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder must succeed")
        .with_initial_gpr(3, r3_initial)
        .with_initial_gpr(4, r4_initial);

    // ===== 7. apply_mfc_dma_pre_replay — ANY mode. =====
    //
    // For ANY mode, `process_rdch_tagstat` validates that the
    // captured value is consistent with the state machine's view:
    // at least one tag in the mask must be completed, and the
    // returned mask must equal `completed_tags & wr_tag_mask`.
    // With both completions captured, both tags are in the
    // completed set; intersect with mask 0x28 → 0x28 ✓.
    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed");
    assert_eq!(plan.dispatched_get_count, 2);
    assert_eq!(plan.tag_stat_queue.len(), 1);
    // Sanity: the captured tag_stat IS what the queue carries.
    let captured_in_queue = plan.tag_stat_queue.iter().next().copied();
    assert_eq!(
        captured_in_queue,
        Some(CAPTURED_TAG_STAT),
        "tag_stat queue must carry the captured ANY return (0x28)",
    );

    let post_dma_program = plan
        .program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

    // ===== 8. Run replay × Interpreter. =====
    let mut programs = BTreeMap::new();
    programs.insert(target_spu, post_dma_program.clone());

    let interp_reports = replay_per_spu_traces::<InterpreterExecutor>(&groups, &programs)
        .expect("replay × Interpreter must succeed");
    assert_eq!(interp_reports.len(), 1);
    let interp = interp_reports.values().next().unwrap();

    // ===== 9. Run replay × Recompiler. =====
    let jit_reports = replay_per_spu_traces_with(&groups, &programs, |_| RecompilerExecutor::new())
        .expect("replay × Recompiler must succeed");
    assert_eq!(jit_reports.len(), 1);
    let jit = jit_reports.values().next().unwrap();

    // ===== 10. Both backends MUST agree byte-identical. =====
    assert_eq!(
        format!("{:?}", interp.final_event_kind),
        format!("{:?}", jit.final_event_kind),
    );
    assert_eq!(interp.records.len(), jit.records.len());
    assert!(interp.total_steps > 0 && jit.total_steps > 0);
    let diff = diff_snapshots(&interp.final_snapshot, &jit.final_snapshot);
    assert!(diff.is_identical(), "diff_snapshots: {diff:?}");

    // ===== 10b. Verify final LS at both regions matches chunks. =====
    let chunk1 = resolve_dma_chunk_side_file(
        &trace_path,
        &dma_dir,
        &cmd1.ea_chunk_sha256,
        Some(SIZE_1 as usize),
    )
    .expect("chunk1 must resolve");
    let chunk2 = resolve_dma_chunk_side_file(
        &trace_path,
        &dma_dir,
        &cmd2.ea_chunk_sha256,
        Some(SIZE_2 as usize),
    )
    .expect("chunk2 must resolve");

    for (name, snap) in [
        ("Interpreter", &interp.final_snapshot),
        ("Recompiler", &jit.final_snapshot),
    ] {
        let lo1 = LSA_1 as usize;
        let hi1 = lo1 + SIZE_1 as usize;
        assert_eq!(
            &snap.ls[lo1..hi1],
            chunk1.as_slice(),
            "{name} final LS @ LSA_1 region must match chunk1",
        );
        let lo2 = LSA_2 as usize;
        let hi2 = lo2 + SIZE_2 as usize;
        assert_eq!(
            &snap.ls[lo2..hi2],
            chunk2.as_slice(),
            "{name} final LS @ LSA_2 region must match chunk2",
        );
    }

    // ===== 11. OUT_MBOX None post-drain (R5.9e.7 synthetic). =====
    assert_eq!(interp.final_snapshot.channels.out_mbox, None);
    assert_eq!(jit.final_snapshot.channels.out_mbox, None);

    // ===== 12. Sanity-print. =====
    eprintln!(
        "[R8.3a SUCCESS] {FIXTURE_NAME} replay-validated:\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         GET #1: cmd=0x{:x} tag={} size={} lsa=0x{:x} ea=0x{:x} chunk={}\n  \
         GET #2: cmd=0x{:x} tag={} size={} lsa=0x{:x} ea=0x{:x} chunk={}\n  \
         WrTagMask=0x{WAIT_MASK:x} WrTagUpdate=ANY(1) RdTagStat=0x{CAPTURED_TAG_STAT:x} (captured)\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         OUT_MBOX = 0x{CANONICAL_STATUS:08x} (= ((sum1 << 16) | sum2) ^ (tag_stat << 24) ^ 0xBEEFBEAD)\n  \
         Final-snapshot diff: identical",
        events.len(),
        image.image_sha256,
        cmd1.cmd,
        cmd1.tag,
        cmd1.size,
        cmd1.lsa,
        ((cmd1.eah as u64) << 32) | cmd1.eal as u64,
        cmd1.ea_chunk_sha256,
        cmd2.cmd,
        cmd2.tag,
        cmd2.size,
        cmd2.lsa,
        ((cmd2.eah as u64) << 32) | cmd2.eal as u64,
        cmd2.ea_chunk_sha256,
        interp.total_steps,
        jit.total_steps,
    );
}
