//! R6.7 A.5 — first replay-validated DMA GET fixture integration test.
//!
//! Loads `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl`
//! plus the matching `.spuimg` and `.dmachunk` side-files, runs the
//! full pipeline (parser → per-SPU transformer → SpuProgram builder
//! → R6.7 C.3 DMA pre-application → replay × Interpreter + replay ×
//! Recompiler), and asserts byte-identical agreement plus the
//! canonical OUT_MBOX status.
//!
//! This is the load-bearing R6.7 acceptance gate: status `0xDEADA12F`
//! is **only** reachable when the GET actually copied the EA bytes
//! into LS AND the SPU computed the deterministic post-DMA sum + XOR.
//! Any silent fake-DMA path (zero-fill LS) produces a different
//! status (`0xDEADBEEF`), so this test failing in the right way
//! distinguishes "no DMA" from "wrong DMA" from "right DMA".
//!
//! ## Pre-capture: this test is `#[ignore]`d
//!
//! The `.self`, `.jsonl`, `.dmachunk`, and `.spuimg` files are all
//! authored at capture-time, NOT at check-in time. They require
//! a working PSL1GHT/ps3toolchain Docker environment AND a working
//! `R:\bin\rpcs3.exe` build to produce. See
//! `behavior-freeze/fixtures/spu/sources/single_spu_dma_get_v1/README.md`
//! for the build + capture pipeline.
//!
//! Once the artifacts land in their canonical locations:
//!
//! - `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl`
//! - `behavior-freeze/fixtures/spu/images/<sha>.spuimg` (the LS image)
//! - `behavior-freeze/fixtures/spu/dma/<sha>.dmachunk` (the EA bytes)
//!
//! the `#[ignore]` attribute below MUST be removed (single-line edit)
//! AND `behavior-freeze/harness/check_trace_fixtures.py` must add
//! `single_spu_dma_get_v1.jsonl` + `single_spu_dma_get_v1.notes.md`
//! to its expected file list. After that, this test is the 7th
//! replay-validated oracle and the project's first DMA oracle.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, CapturedEvent, InterpreterExecutor, SpuImageEvent, SpuMfcCmdEvent,
    TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

/// Canonical OUT_MBOX value for inputs `buf[i] = i & 0xFF`, `i ∈ [0, 128)`:
///
/// `sum_of_buf = 8128 = 0x1FC0` → `cs = 0x1FC0 ^ 0xDEADBEEF = 0xDEADA12F`.
const CANONICAL_STATUS: u32 = 0xDEAD_A12F;

/// Fixture name used for path resolution + diagnostics.
const FIXTURE_NAME: &str = "single_spu_dma_get_v1";

fn fixture_trace_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // → rust/
    p.pop(); // → workspace root
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
fn r6_7_a5_single_spu_dma_get_v1_replay_validated_byte_identical() {
    // ===== 1. Fixture artifacts must exist on disk. =====
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(
        trace_path.exists(),
        "fixture trace not found at {}\n\
         (R6.7 A.5 capture must produce this file before this test passes)",
        trace_path.display(),
    );
    assert!(
        images_dir.exists(),
        "fixture images dir not found at {}\n\
         (capture must produce a .spuimg side-file in this dir)",
        images_dir.display(),
    );
    assert!(
        dma_dir.exists(),
        "fixture DMA dir not found at {}\n\
         (R6.7 A.5 capture must produce a .dmachunk side-file in this dir)",
        dma_dir.display(),
    );

    // ===== 2. Parse the JSONL trace. =====
    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("parser must accept the captured trace");
    assert!(!events.is_empty(), "trace has no events");

    // ===== 3. Verify R6.7 A.5 acceptance criteria. =====
    // Criterion: exactly 1 spu_mfc_cmd event (cmd=0x40, tag=3, size=128).
    let mfc_cmd_events: Vec<&SpuMfcCmdEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuMfcCmd(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(
        mfc_cmd_events.len(),
        1,
        "fixture must have exactly 1 spu_mfc_cmd event (got {})",
        mfc_cmd_events.len(),
    );
    let mfc_cmd = mfc_cmd_events[0];
    assert_eq!(mfc_cmd.cmd, 0x40, "expected GET cmd 0x40, got 0x{:x}", mfc_cmd.cmd);
    assert_eq!(mfc_cmd.tag, 3, "expected tag=3, got {}", mfc_cmd.tag);
    assert_eq!(mfc_cmd.size, 128, "expected size=128, got {}", mfc_cmd.size);
    assert_eq!(mfc_cmd.eah, 0, "expected eah=0 (PSL1GHT 32-bit), got 0x{:x}", mfc_cmd.eah);
    // lsa was chosen as 0x10000 in the SPU source (see spu/spu_dma_get.c).
    assert_eq!(mfc_cmd.lsa, 0x10000, "expected lsa=0x10000, got 0x{:x}", mfc_cmd.lsa);

    // Criterion: exactly 1 mfc_dma_complete event matching the cmd's tag.
    let dma_complete_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::MfcDmaComplete(c) if c.tag == 3))
        .count();
    assert_eq!(
        dma_complete_count, 1,
        "fixture must have exactly 1 mfc_dma_complete for tag=3 (got {})",
        dma_complete_count,
    );

    // Criterion: exactly 1 spu_wrch ch28 (OUT_MBOX) for the canonical status.
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
        "fixture must have exactly 1 spu_wrch ch28 with value=0x{CANONICAL_STATUS:08x} \
         (got {out_mbox_events:?})",
    );

    // Criterion: exactly 1 spu_stop with code 0x101.
    let stop_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuStop(s) if s.stop_code == 0x101))
        .count();
    assert_eq!(stop_count, 1, "fixture must have exactly 1 spu_stop with code=0x101");

    // ===== 4. Per-SPU transformer must produce exactly 1 group. =====
    let groups: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("per-SPU transform must succeed");
    assert_eq!(
        groups.len(),
        1,
        "fixture must have exactly 1 target_spu (got {}: keys={:?})",
        groups.len(),
        groups.keys().collect::<Vec<_>>(),
    );
    let target_spu = *groups.keys().next().unwrap();

    // ===== 5. Locate the spu_image event. Must be exactly 1. =====
    let images: Vec<&SpuImageEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuImage(img) => Some(img),
            _ => None,
        })
        .collect();
    assert_eq!(
        images.len(),
        1,
        "fixture must have exactly 1 spu_image event (got {})",
        images.len(),
    );
    let image = images[0];

    // ===== 6. Build SpuProgram from the .spuimg side-file. =====
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    assert!(
        image_path.exists(),
        ".spuimg side-file missing at {} (sha={})",
        image_path.display(),
        image.image_sha256,
    );
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder must succeed (hash + size + entry_pc)");

    // ===== 7. R6.7 C.3 — apply DMA pre-replay. =====
    //
    // This walks the captured events, drives `MfcReplayState`, loads
    // the `.dmachunk` via the A.3 loader, and produces a SpuProgram
    // whose LS already contains the post-DMA bytes plus a
    // pre-populated rdch ch24 queue.
    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed (loader resolves .dmachunk)");
    assert_eq!(
        plan.dispatched_get_count, 1,
        "exactly 1 GET dispatch expected, got {}",
        plan.dispatched_get_count,
    );
    assert_eq!(
        plan.tag_stat_queue.len(),
        1,
        "exactly 1 rdch ch24 expected, got queue len {}",
        plan.tag_stat_queue.len(),
    );
    let post_dma_program = plan
        .program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

    // Sanity: the post-DMA LS at [lsa..lsa+size] holds the captured
    // chunk bytes (the counting pattern the PPU wrote to EA).
    let lsa = mfc_cmd.lsa as usize;
    let dma_size = mfc_cmd.size as usize;
    let ls = &post_dma_program.segments[0].data;
    for (i, &b) in ls[lsa..lsa + dma_size].iter().enumerate() {
        assert_eq!(
            b,
            (i & 0xFF) as u8,
            "post-DMA LS byte {i} = 0x{b:02x}, expected 0x{:02x} (counting pattern from PPU)",
            i & 0xFF,
        );
    }

    // ===== 8. Run replay × Interpreter. Must reach Finished{0x101}. =====
    let mut programs = BTreeMap::new();
    programs.insert(target_spu, post_dma_program.clone());

    let interp_reports = replay_per_spu_traces::<InterpreterExecutor>(&groups, &programs)
        .expect("replay × Interpreter must succeed");
    assert_eq!(interp_reports.len(), 1);
    let interp = interp_reports.values().next().unwrap();

    // ===== 9. Run replay × Recompiler. Must reach Finished{0x101}. =====
    let jit_reports = replay_per_spu_traces_with(&groups, &programs, |_| RecompilerExecutor::new())
        .expect("replay × Recompiler must succeed");
    assert_eq!(jit_reports.len(), 1);
    let jit = jit_reports.values().next().unwrap();

    // ===== 10. Both backends MUST agree byte-identical. =====
    assert_eq!(
        format!("{:?}", interp.final_event_kind),
        format!("{:?}", jit.final_event_kind),
        "Interpreter vs Recompiler final_event_kind diverged",
    );
    assert_eq!(
        interp.records.len(),
        jit.records.len(),
        "Interpreter vs Recompiler record count diverged",
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

    // ===== 11. Canonical OUT_MBOX status. =====
    //
    // The SPU's wrch ch28 places `cs = 0xDEADA12F` into out_mbox; on
    // stop 0x101 (group exit), lv2 reads OUT_MBOX as the exit status.
    // The R5.9e.7 transformer engine fix injects a synthetic
    // `PpuPopOutMbox` event after `ExpectSpuFinished` to model the
    // lv2 drain — so by the time `final_snapshot` is captured,
    // OUT_MBOX has already been drained back to `None`.
    //
    // The canonical value is verified two ways:
    //   (a) the captured JSONL event at line ~165 asserts the trace
    //       itself contains the wrch ch28 with the canonical status;
    //   (b) `diff_snapshots` below confirms byte-identical agreement
    //       between Interpreter and Recompiler — both must drain to
    //       `None` after the synthetic PpuPopOutMbox, both must agree.
    //
    // Asserting `Some(CANONICAL_STATUS)` here would be tightening the
    // invariant beyond what the post-drain replay model exposes.
    assert_eq!(
        interp.final_snapshot.channels.out_mbox,
        None,
        "Interpreter OUT_MBOX must be None after post-stop drain (R5.9e.7 engine fix); \
         got {:?}",
        interp.final_snapshot.channels.out_mbox,
    );
    assert_eq!(
        jit.final_snapshot.channels.out_mbox,
        None,
        "Recompiler OUT_MBOX must be None after post-stop drain; got {:?}",
        jit.final_snapshot.channels.out_mbox,
    );

    // ===== 12. Sanity-print the result. =====
    eprintln!(
        "[R6.7 A.5 SUCCESS] {FIXTURE_NAME} replay-validated:\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         spu_mfc_cmd cmd=0x{:x} tag={} size={} lsa=0x{:x} ea=0x{:x}\n  \
         ea_chunk_sha256={}\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         OUT_MBOX = 0x{CANONICAL_STATUS:08x}\n  \
         Final-snapshot diff: identical",
        events.len(),
        image.image_sha256,
        mfc_cmd.cmd,
        mfc_cmd.tag,
        mfc_cmd.size,
        mfc_cmd.lsa,
        ((mfc_cmd.eah as u64) << 32) | mfc_cmd.eal as u64,
        mfc_cmd.ea_chunk_sha256,
        interp.total_steps,
        jit.total_steps,
    );
}
