//! R8.3c — first IMMEDIATE-wait-mode oracle (12th oracle).
//! Two queued GETs + TWO ch24 reads with `WrTagUpdate =
//! IMMEDIATE` (= 0) and distinct masks. Captures real RPCS3
//! behavior for IMMEDIATE / clearing semantics.
//!
//! Captured invariants (confirmed empirically):
//! - First IMMEDIATE read (mask 0x08): ts1 = 0x08
//! - Second IMMEDIATE read (mask 0x28): ts2 = 0x28
//!
//! `ts2 == 0x28` proves that RPCS3 does NOT clear bits from
//! `completed_tags` on IMMEDIATE read (Cell BE canonical
//! semantic). If clearing had happened, ts2 would be 0x20 or
//! 0x00.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, CapturedEvent, InterpreterExecutor,
    SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

const CANONICAL_STATUS: u32 = 0xDD16_4A9E;
const TAG_STAT_1: u32 = 0x08;
const TAG_STAT_2: u32 = 0x28;

const FIXTURE_NAME: &str = "single_spu_dma_tag_immediate_v1";

const TAG_1: u32 = 3;
const TAG_2: u32 = 5;
const SIZE_1: u32 = 128;
const SIZE_2: u32 = 64;
const LSA_1: u32 = 0x10000;
const LSA_2: u32 = 0x10100;
const MASK_1: u32 = 1u32 << TAG_1;                   // 0x08
const MASK_FULL: u32 = (1u32 << TAG_1) | (1u32 << TAG_2); // 0x28
const IMMEDIATE_MODE: u32 = 0;

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
fn r8_3c_single_spu_dma_tag_immediate_v1_replay_validated_byte_identical() {
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(trace_path.exists());
    assert!(images_dir.exists());
    assert!(dma_dir.exists());

    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("parser must accept the captured trace");
    assert!(!events.is_empty());

    let mfc_cmd_events: Vec<&SpuMfcCmdEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuMfcCmd(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(mfc_cmd_events.len(), 2);
    let cmd1 = mfc_cmd_events[0];
    let cmd2 = mfc_cmd_events[1];
    assert_eq!(cmd1.tag, TAG_1);
    assert_eq!(cmd2.tag, TAG_2);

    let dma_completes: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::MfcDmaComplete(c) => Some((c.tag, c.transferred_bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(dma_completes.len(), 2);

    // **Load-bearing R8.3c invariants:** TWO ch22 writes with
    // distinct masks, TWO ch23 writes (both IMMEDIATE = 0),
    // TWO ch24 reads (captured values).
    let wrch_22: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 22 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(
        wrch_22,
        vec![MASK_1, MASK_FULL],
        "two distinct masks (0x08, 0x28)",
    );

    let wrch_23: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 23 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(
        wrch_23,
        vec![IMMEDIATE_MODE, IMMEDIATE_MODE],
        "both reads use IMMEDIATE mode (= 0) — load-bearing R8.3c invariant",
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
        vec![Some(TAG_STAT_1), Some(TAG_STAT_2)],
        "ts2 = 0x28 (full mask) proves IMMEDIATE does NOT clear \
         completed_tags on read — Cell BE / R8.3b persistent semantic \
         carries through to IMMEDIATE mode unchanged",
    );

    let out_mbox_events: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 28 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(out_mbox_events, vec![CANONICAL_STATUS]);

    let stop_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuStop(s) if s.stop_code == 0x101))
        .count();
    assert_eq!(stop_count, 1);

    let groups: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("per-SPU transform must succeed");
    assert_eq!(groups.len(), 1);
    let target_spu = *groups.keys().next().unwrap();

    let images: Vec<&SpuImageEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuImage(img) => Some(img),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 1);
    let image = images[0];

    let r3_initial: u128 = (cmd1.eal as u128) << 64;
    let r4_initial: u128 = (cmd2.eal as u128) << 64;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder must succeed")
        .with_initial_gpr(3, r3_initial)
        .with_initial_gpr(4, r4_initial);

    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed");
    assert_eq!(plan.dispatched_get_count, 2);

    let post_dma_program = plan
        .program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

    let mut programs = BTreeMap::new();
    programs.insert(target_spu, post_dma_program.clone());

    let interp_reports = replay_per_spu_traces::<InterpreterExecutor>(&groups, &programs)
        .expect("replay × Interpreter must succeed");
    let interp = interp_reports.values().next().unwrap();

    let jit_reports = replay_per_spu_traces_with(&groups, &programs, |_| RecompilerExecutor::new())
        .expect("replay × Recompiler must succeed");
    let jit = jit_reports.values().next().unwrap();

    assert_eq!(
        format!("{:?}", interp.final_event_kind),
        format!("{:?}", jit.final_event_kind),
    );
    let diff = diff_snapshots(&interp.final_snapshot, &jit.final_snapshot);
    assert!(diff.is_identical(), "diff_snapshots: {diff:?}");

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
        assert_eq!(&snap.ls[lo1..hi1], chunk1.as_slice(), "{name} LS@LSA_1");
        let lo2 = LSA_2 as usize;
        let hi2 = lo2 + SIZE_2 as usize;
        assert_eq!(&snap.ls[lo2..hi2], chunk2.as_slice(), "{name} LS@LSA_2");
    }

    assert_eq!(interp.final_snapshot.channels.out_mbox, None);
    assert_eq!(jit.final_snapshot.channels.out_mbox, None);

    eprintln!(
        "[R8.3c SUCCESS] {FIXTURE_NAME} replay-validated:\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         Read #1: mask=0x{MASK_1:x} IMMEDIATE -> 0x{TAG_STAT_1:x}\n  \
         Read #2: mask=0x{MASK_FULL:x} IMMEDIATE -> 0x{TAG_STAT_2:x} \
         (tag 3 bit retained → IMMEDIATE doesn't clear)\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         OUT_MBOX = 0x{CANONICAL_STATUS:08x}\n  \
         Final-snapshot diff: identical",
        events.len(),
        image.image_sha256,
        interp.total_steps,
        jit.total_steps,
    );
}
