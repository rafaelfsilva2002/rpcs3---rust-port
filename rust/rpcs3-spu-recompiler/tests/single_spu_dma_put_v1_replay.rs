//! R8.1 — first replay-validated DMA PUT fixture integration test
//! (8th oracle). Symmetric to R6.7 A.5 GET (`single_spu_dma_get_v1`)
//! but inverts the DMA direction (LS → EA).
//!
//! Loads `behavior-freeze/fixtures/spu/traces/single_spu_dma_put_v1.jsonl`
//! plus the matching `.spuimg` and `.dmachunk` side-files, runs the
//! full pipeline (parser → per-SPU transformer → SpuProgram builder
//! → R8.1 PUT-replay pre-application → replay × Interpreter +
//! replay × Recompiler), and asserts byte-identical agreement plus
//! the canonical OUT_MBOX sentinel.
//!
//! The PUT-replay path differs from GET-replay in the load-bearing
//! invariant:
//!
//! - **GET-replay**: the captured `.dmachunk` carries the EA source
//!   bytes the SPU received; `apply_mfc_dma_pre_replay` WRITES them
//!   into LS so the SPU's subsequent reads observe the captured
//!   data (no real EA in replay).
//! - **PUT-replay**: the captured `.dmachunk` carries the LS source
//!   bytes the SPU PRODUCED at dispatch time;
//!   `apply_mfc_dma_pre_replay` ASSERTS that the SPU's LS bytes at
//!   `[lsa..lsa+size]` match the captured chunk. Any divergence
//!   (the Rust SPU bytecode generates different output than the
//!   capture run) is a real correctness gap, surfaced as the new
//!   `MfcReplayError::PutLsBytesMismatch`.
//!
//! The canonical OUT_MBOX sentinel `0xC0FFEECA` proves the SPU
//! reached the post-PUT path (i.e., the rdch ch24 unblocked, which
//! means the MFC tag completed, which in replay means the PUT-
//! assert + tag-stat queue push happened cleanly).

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, CapturedEvent, InterpreterExecutor,
    SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

/// Canonical OUT_MBOX sentinel — proves the SPU reached the
/// post-PUT path. The PPU side computes a separate `ea_status`
/// (= sum_of_ea ^ 0xCAFEBABE = 0xCAFEA57E for the canonical
/// counting pattern), but that lives outside the SPU runtime and
/// is verified by the host-side acceptance criteria documented in
/// the fixture's README + notes.md.
const CANONICAL_SENTINEL: u32 = 0xC0FF_EECA;

const FIXTURE_NAME: &str = "single_spu_dma_put_v1";

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
fn r8_1_single_spu_dma_put_v1_replay_validated_byte_identical() {
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

    // ===== 3. R8.1 acceptance criteria. =====
    // Exactly 1 spu_mfc_cmd event with cmd=0x20 (PUT), tag=3, size=128.
    let mfc_cmd_events: Vec<&SpuMfcCmdEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuMfcCmd(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(mfc_cmd_events.len(), 1, "exactly 1 spu_mfc_cmd expected");
    let mfc_cmd = mfc_cmd_events[0];
    assert_eq!(mfc_cmd.cmd, 0x20, "expected PUT cmd 0x20, got 0x{:x}", mfc_cmd.cmd);
    assert_eq!(mfc_cmd.tag, 3);
    assert_eq!(mfc_cmd.size, 128);
    assert_eq!(mfc_cmd.eah, 0);
    assert_eq!(mfc_cmd.lsa, 0x10000);

    // Exactly 1 mfc_dma_complete for tag=3.
    let dma_complete_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::MfcDmaComplete(c) if c.tag == 3))
        .count();
    assert_eq!(dma_complete_count, 1);

    // Exactly 1 spu_wrch ch28 with the sentinel.
    let out_mbox_events: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 28 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(
        out_mbox_events,
        vec![CANONICAL_SENTINEL],
        "captured ch28 must carry the post-PUT sentinel",
    );

    // Exactly 1 spu_stop with code 0x101.
    let stop_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuStop(s) if s.stop_code == 0x101))
        .count();
    assert_eq!(stop_count, 1);

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

    // ===== 6. Build SpuProgram from the .spuimg side-file. =====
    //
    // Seed r3 with the PSL1GHT arg0 EA. The trace doesn't capture
    // initial GPRs, but lv2 sets r3 = thread_args.arg0 (a u64) at
    // sysSpuThreadInitialize before entry_pc runs. SPU calling
    // convention: a 64-bit scalar parameter is placed in one r128
    // with lane 0 = high 32, lane 1 = low 32. The PPU side does
    // `thread_args.arg0 = (uint64_t)(uintptr_t)ea_buf`, so high 32
    // is zero and low 32 holds the EA pointer. The SPU's
    // `(uint32_t)spu_id` then shuffles lane 1 into the preferred
    // slot (lane 0) for the subsequent ch18 wrch.
    //
    // Without this override, lane 1 starts at 0, the shuffle moves
    // 0 into lane 0, and the final-state ExpectGprWord{reg:3,lane:0,
    // value:0x10011180} fails. The GET fixture overwrites r3 with
    // `tag_stat` by exit so it doesn't need this; PUT keeps the
    // original EA in r3 through stop 0x101.
    //
    // We derive the EA from the captured `spu_mfc_cmd.eal` (the SPU
    // wrote ea into ch18 from r3 — same value, byte-identical).
    let r3_initial: u128 = (mfc_cmd.eal as u128) << 64;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    assert!(image_path.exists(), ".spuimg missing");
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder must succeed")
        .with_initial_gpr(3, r3_initial);

    // ===== 7. R8.1 — apply PUT-replay pre-application. =====
    //
    // `apply_mfc_dma_pre_replay` runs BEFORE the SPU executes, so
    // for PUT it cannot inspect dispatch-time LS (the SPU hasn't
    // written the source bytes yet). The helper now routes PUT
    // through `process_mfc_cmd_pre_replay`, which validates the
    // chunk SHA + size via the A.3 loader and registers the
    // in-flight tag (so subsequent `mfc_dma_complete` + `rdch ch24`
    // resolve correctly), but defers the LS-bytes assertion.
    //
    // The deferred assertion is performed POST-replay in step 11
    // below, against the SPU's final LS — sufficient for the
    // canonical fixture (the SPU writes the source pattern into
    // LS[0x10000..0x10080] and never touches it again before stop
    // 0x101). A future R-phase that drives the state machine
    // in-line with the executor would restore the dispatch-time
    // contract; for now the post-replay check + diff_snapshots
    // cross-backend agreement + sentinel collectively gate
    // correctness.
    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed");
    assert_eq!(plan.dispatched_get_count, 1, "exactly 1 MFC dispatch (PUT)");
    assert_eq!(plan.tag_stat_queue.len(), 1);
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

    // ===== 10b. R8.1 — Deferred PUT LS-bytes verification. =====
    //
    // `apply_mfc_dma_pre_replay` skipped the dispatch-time assert
    // (impossible before the SPU runs). Now that both backends have
    // executed the SPU, the final LS at [lsa..lsa+size] MUST match
    // the captured chunk byte-for-byte. This is the load-bearing
    // PUT correctness gate.
    //
    // Why this works for the canonical fixture: after the PUT
    // dispatch (step 6 wrch ch21), the SPU only does ch22/ch23
    // wrch + ch24 rdch + ch28 wrch + stop — none of those touch
    // LS. So final LS at the dispatch region == dispatch-time LS.
    //
    // We do this check on BOTH backends (they already agree per
    // diff_snapshots above, but explicit > implicit).
    let captured_chunk =
        resolve_dma_chunk_side_file(&trace_path, &dma_dir, &mfc_cmd.ea_chunk_sha256, Some(mfc_cmd.size as usize))
            .expect("captured chunk must resolve");
    assert_eq!(captured_chunk.len(), mfc_cmd.size as usize);
    let lo = mfc_cmd.lsa as usize;
    let hi = lo + mfc_cmd.size as usize;
    assert_eq!(
        &interp.final_snapshot.ls[lo..hi],
        captured_chunk.as_slice(),
        "Interpreter final LS at [0x{:x}..0x{:x}] must match captured PUT chunk",
        lo, hi,
    );
    assert_eq!(
        &jit.final_snapshot.ls[lo..hi],
        captured_chunk.as_slice(),
        "Recompiler final LS at [0x{:x}..0x{:x}] must match captured PUT chunk",
        lo, hi,
    );

    // ===== 11. Canonical OUT_MBOX sentinel. =====
    //
    // Same R5.9e.7 PpuPopOutMbox synthetic drain as the GET test:
    // after stop 0x101, the lv2 kernel reads OUT_MBOX as the
    // group-exit status, which the transformer models with a
    // synthetic pop event. So `final_snapshot.channels.out_mbox`
    // is `None` post-drain. The sentinel is verified via the
    // captured JSONL event check above plus the diff_snapshots
    // contract.
    assert_eq!(
        interp.final_snapshot.channels.out_mbox,
        None,
        "Interpreter OUT_MBOX must be None after post-stop drain",
    );
    assert_eq!(
        jit.final_snapshot.channels.out_mbox,
        None,
        "Recompiler OUT_MBOX must be None after post-stop drain",
    );

    // ===== 12. Sanity-print. =====
    eprintln!(
        "[R8.1 SUCCESS] {FIXTURE_NAME} replay-validated:\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         spu_mfc_cmd cmd=0x{:x} tag={} size={} lsa=0x{:x} ea=0x{:x}\n  \
         ea_chunk_sha256={}\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         OUT_MBOX sentinel = 0x{CANONICAL_SENTINEL:08x}\n  \
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
