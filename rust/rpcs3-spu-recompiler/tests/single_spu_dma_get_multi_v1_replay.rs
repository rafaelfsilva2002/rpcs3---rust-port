//! R8.2 — first multi-DMA replay-validated fixture (9th oracle).
//! Two queued MFC GETs (distinct tags 3 + 5, distinct EAs,
//! distinct sizes 128 + 64, distinct LSAs 0x10000 + 0x10100) +
//! ALL wait mode. Exercises multi-tag in-flight state on top of
//! the 8-oracle baseline without any engine-side code changes.
//!
//! Loads `behavior-freeze/fixtures/spu/traces/
//! single_spu_dma_get_multi_v1.jsonl` plus the matching
//! `.spuimg` and two `.dmachunk` side-files, runs the full
//! pipeline (parser → per-SPU transformer → SpuProgram builder
//! → R6.7 C.3 pre-replay both GETs → replay × Interpreter +
//! replay × Recompiler), and asserts byte-identical agreement
//! plus the canonical OUT_MBOX status `0xE12DEA4E`.
//!
//! The R8.2 contract layers on top of R6.7 A.5:
//!
//! - **Two `spu_mfc_cmd` events** (cmd=0x40, tags 3 + 5,
//!   distinct LSAs / EAs / sizes).
//! - **Two `mfc_dma_complete` events** (matching tags + sizes).
//! - **WrTagMask = 0x28** (= (1<<3) | (1<<5)), multi-bit.
//! - **WrTagUpdate = ALL** (= 2); RdTagStat returns 0x28 only
//!   after both completions fire.
//! - **OUT_MBOX = 0xE12DEA4E** = ((sum1 << 16) | sum2) ^ 0xFEEDFACE
//!   with sum1 = 0x1FC0 (counting pattern), sum2 = 0x1080
//!   (constant 0x42).

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, CapturedEvent, InterpreterExecutor,
    SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

/// Canonical multi-GET OUT_MBOX status. Computed as
/// `((sum1 << 16) | sum2) ^ 0xFEEDFACE` with sum1 = 0x1FC0 +
/// sum2 = 0x1080 → combined = 0x1FC0_1080 → status = 0xE12D_EA4E.
const CANONICAL_STATUS: u32 = 0xE12D_EA4E;

const FIXTURE_NAME: &str = "single_spu_dma_get_multi_v1";

const TAG_1: u32 = 3;
const TAG_2: u32 = 5;
const SIZE_1: u32 = 128;
const SIZE_2: u32 = 64;
const LSA_1: u32 = 0x10000;
const LSA_2: u32 = 0x10100;
const WAIT_MASK: u32 = (1u32 << TAG_1) | (1u32 << TAG_2); // = 0x28

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
fn r8_2_single_spu_dma_get_multi_v1_replay_validated_byte_identical() {
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

    // ===== 3. R8.2 acceptance criteria. =====
    // Exactly 2 spu_mfc_cmd events with cmd=0x40 (GET).
    let mfc_cmd_events: Vec<&SpuMfcCmdEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuMfcCmd(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(
        mfc_cmd_events.len(),
        2,
        "exactly 2 spu_mfc_cmd events expected (got {})",
        mfc_cmd_events.len(),
    );

    // First dispatch: tag 3, size 128, LSA 0x10000.
    let cmd1 = mfc_cmd_events[0];
    assert_eq!(cmd1.cmd, 0x40, "expected GET cmd 0x40, got 0x{:x}", cmd1.cmd);
    assert_eq!(cmd1.tag, TAG_1);
    assert_eq!(cmd1.size, SIZE_1);
    assert_eq!(cmd1.eah, 0);
    assert_eq!(cmd1.lsa, LSA_1);

    // Second dispatch: tag 5, size 64, LSA 0x10100.
    let cmd2 = mfc_cmd_events[1];
    assert_eq!(cmd2.cmd, 0x40);
    assert_eq!(cmd2.tag, TAG_2);
    assert_eq!(cmd2.size, SIZE_2);
    assert_eq!(cmd2.eah, 0);
    assert_eq!(cmd2.lsa, LSA_2);

    // EAs are distinct (different buffers in PPU BSS).
    assert_ne!(cmd1.eal, cmd2.eal, "EA1 and EA2 must be distinct");
    // Chunks may or may not share SHA (R8.2 picks distinct patterns
    // so they end up distinct: counting pattern vs constant 0x42).
    assert_ne!(
        cmd1.ea_chunk_sha256, cmd2.ea_chunk_sha256,
        "R8.2 fixture uses distinct chunk patterns",
    );

    // Exactly 2 mfc_dma_complete events, one per tag.
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

    // Exactly 1 spu_wrch ch28 carrying the canonical status.
    let out_mbox_events: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 28 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(
        out_mbox_events,
        vec![CANONICAL_STATUS],
        "captured ch28 must carry the canonical multi-GET status",
    );

    // Exactly 1 spu_stop with code 0x101.
    let stop_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuStop(s) if s.stop_code == 0x101))
        .count();
    assert_eq!(stop_count, 1);

    // Sanity-check the WrTagMask / WrTagUpdate / RdTagStat sequence.
    // The trace transformer drops these as pure context, but we
    // verify the raw events to lock the multi-tag ALL semantics.
    let wrch_22: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 22 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(wrch_22, vec![WAIT_MASK], "ch22 = 0x28 (tag mask)");

    let wrch_23: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 23 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(wrch_23, vec![2], "ch23 = ALL");

    let rdch_24: Vec<Option<u32>> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuRdch(r) if r.channel == 24 => Some(r.value),
            _ => None,
        })
        .collect();
    assert_eq!(rdch_24, vec![Some(WAIT_MASK)], "ch24 rdch returns 0x28");

    // ===== 4. Per-SPU transformer. =====
    let groups: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("per-SPU transform must succeed");
    assert_eq!(groups.len(), 1, "exactly 1 target_spu");
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
    //
    // PSL1GHT lv2 places thread_args.arg0 → SPU r3 (as u64) and
    // arg1 → SPU r4. The u64 occupies lanes 0+1 of the 128-bit
    // register (high 32 → lane 0, low 32 → lane 1). The SPU's
    // `(uint32_t)spu_id` / `(uint32_t)arg` extracts lane 1 (low 32)
    // via a shuffle into the preferred slot.
    //
    // Without seeding, lane 1 starts at 0 → final-state
    // ExpectGprWord for r3 / r4 fails. R8.2 keeps EA1 in r3 and
    // EA2 in r4 across the function, so both need seeding.
    let r3_initial: u128 = (cmd1.eal as u128) << 64;
    let r4_initial: u128 = (cmd2.eal as u128) << 64;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    assert!(image_path.exists(), ".spuimg missing");
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder must succeed")
        .with_initial_gpr(3, r3_initial)
        .with_initial_gpr(4, r4_initial);

    // ===== 7. R6.7 C.3 — apply DMA pre-replay (both GETs). =====
    //
    // For GET, `apply_mfc_dma_pre_replay` writes each captured
    // .dmachunk into LS at the captured (lsa, size). For R8.2 the
    // helper walks the trace and applies BOTH GETs in order; the
    // tag-stat queue ends up with a single 0x28 value (ALL mode
    // pops once after both completions fire).
    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed");
    assert_eq!(
        plan.dispatched_get_count, 2,
        "exactly 2 MFC dispatches (both GETs)",
    );
    assert_eq!(
        plan.tag_stat_queue.len(),
        1,
        "ALL mode pops the mask exactly once after both completions",
    );
    let post_dma_program = plan
        .program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

    // Sanity: the post-DMA LS holds the captured bytes at BOTH
    // LSAs (counting pattern at LSA_1, constant 0x42 at LSA_2).
    let ls = &post_dma_program.segments[0].data;
    for (i, &b) in ls[LSA_1 as usize..(LSA_1 + SIZE_1) as usize]
        .iter()
        .enumerate()
    {
        assert_eq!(
            b,
            (i & 0xFF) as u8,
            "LS @ LSA_1[{i}] = 0x{b:02x}, expected counting pattern",
        );
    }
    for (i, &b) in ls[LSA_2 as usize..(LSA_2 + SIZE_2) as usize]
        .iter()
        .enumerate()
    {
        assert_eq!(b, 0x42, "LS @ LSA_2[{i}] = 0x{b:02x}, expected 0x42 constant");
    }

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
    //
    // Redundant for GET (the chunks are written by the pre-replay
    // helper) but exercised here to mirror the R8.1 PUT post-replay
    // verification shape and lock the contract that the SPU's
    // bytecode does NOT corrupt the GET destination regions
    // between dispatch and stop.
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

    for (name, snap) in [("Interpreter", &interp.final_snapshot), ("Recompiler", &jit.final_snapshot)] {
        let lo1 = LSA_1 as usize;
        let hi1 = lo1 + SIZE_1 as usize;
        assert_eq!(
            &snap.ls[lo1..hi1],
            chunk1.as_slice(),
            "{name} final LS @ [LSA_1..+SIZE_1] must match chunk1",
        );
        let lo2 = LSA_2 as usize;
        let hi2 = lo2 + SIZE_2 as usize;
        assert_eq!(
            &snap.ls[lo2..hi2],
            chunk2.as_slice(),
            "{name} final LS @ [LSA_2..+SIZE_2] must match chunk2",
        );
    }

    // ===== 11. Canonical OUT_MBOX status — drained post-stop. =====
    //
    // Same R5.9e.7 PpuPopOutMbox synthetic drain: after stop 0x101,
    // the transformer injects a drain so `final_snapshot.channels.
    // out_mbox` reads as None. The canonical 0xE12DEA4E value is
    // verified by the captured JSONL ch28 wrch above + the
    // diff_snapshots contract.
    assert_eq!(
        interp.final_snapshot.channels.out_mbox, None,
        "Interpreter OUT_MBOX must be None after post-stop drain",
    );
    assert_eq!(
        jit.final_snapshot.channels.out_mbox, None,
        "Recompiler OUT_MBOX must be None after post-stop drain",
    );

    // ===== 12. Sanity-print. =====
    eprintln!(
        "[R8.2 SUCCESS] {FIXTURE_NAME} replay-validated:\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         GET #1: cmd=0x{:x} tag={} size={} lsa=0x{:x} ea=0x{:x} chunk={}\n  \
         GET #2: cmd=0x{:x} tag={} size={} lsa=0x{:x} ea=0x{:x} chunk={}\n  \
         WrTagMask=0x{WAIT_MASK:x} WrTagUpdate=ALL RdTagStat=0x{WAIT_MASK:x}\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         OUT_MBOX = 0x{CANONICAL_STATUS:08x}\n  \
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
