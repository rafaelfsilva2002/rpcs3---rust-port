//! R8.4f-b — first MFC PUTLB list-DMA + barrier replay-validated
//! oracle (17th oracle). Byte-identical to PUTL (R8.4e) except
//! cmd=0x25 (PUTL | MFC_BARRIER_MASK).
//!
//! Per RPCS3 `do_list_transfer`, the barrier bit is stripped
//! before the per-element copy (`args.cmd & ~0xf`), so the data
//! path is byte-identical to plain PUTL. Barrier ordering
//! effects on `mfc_barrier` register persistence don't surface
//! in this single-SPU, fresh-tag, single-dispatch fixture.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, resolve_dma_listdesc_side_file,
    CapturedEvent, InterpreterExecutor, SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

const SPU_SENTINEL: u32 = 0xC0FF_EEBB;
const FIXTURE_NAME: &str = "single_spu_dma_putlb_v1";

const TAG: u32 = 3;
const LSA_SRC_BASE: u32 = 0x10000;
const DESCRIPTOR_SIZE: u32 = 16;
const ELEMENT_COUNT: usize = 2;
const EL_SIZE_1: u32 = 128;
const EL_SIZE_2: u32 = 64;

fn fixture_trace_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); p.pop();
    p.push("behavior-freeze"); p.push("fixtures"); p.push("spu"); p.push("traces");
    p.push(format!("{FIXTURE_NAME}.jsonl"));
    p
}
fn fixture_images_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); p.pop();
    p.push("behavior-freeze"); p.push("fixtures"); p.push("spu"); p.push("images");
    p
}
fn fixture_dma_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); p.pop();
    p.push("behavior-freeze"); p.push("fixtures"); p.push("spu"); p.push("dma");
    p
}

#[test]
fn r8_4f_b_single_spu_dma_putlb_v1_replay_validated_byte_identical() {
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(trace_path.exists());

    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("R8.4f-b: parser must accept PUTLB trace");
    assert!(!events.is_empty());

    let mfc_cmds: Vec<&SpuMfcCmdEvent> = events.iter().filter_map(|ev| match ev {
        CapturedEvent::SpuMfcCmd(m) => Some(m), _ => None,
    }).collect();
    assert_eq!(mfc_cmds.len(), 1);
    let cmd = mfc_cmds[0];
    assert_eq!(cmd.cmd, 0x25, "PUTLB cmd code (= PUTL | MFC_BARRIER_MASK)");
    assert_eq!(cmd.tag, TAG);
    assert_eq!(cmd.size, DESCRIPTOR_SIZE);
    assert_eq!(cmd.lsa, LSA_SRC_BASE);

    let desc_sha = cmd.descriptor_sha256.as_deref().expect("descriptor_sha256");
    let elements = cmd.element_chunks.as_deref().expect("element_chunks");
    let sizes = cmd.element_sizes.as_deref().expect("element_sizes");
    let eals = cmd.element_eals.as_deref().expect("element_eals");
    assert_eq!(elements.len(), ELEMENT_COUNT);
    assert_eq!(sizes, &[EL_SIZE_1, EL_SIZE_2]);

    let dma_completes: Vec<(u32, u32)> = events.iter().filter_map(|ev| match ev {
        CapturedEvent::MfcDmaComplete(c) => Some((c.tag, c.transferred_bytes)), _ => None,
    }).collect();
    assert_eq!(dma_completes, vec![(TAG, EL_SIZE_1 + EL_SIZE_2)]);

    let out_mbox: Vec<u32> = events.iter().filter_map(|ev| match ev {
        CapturedEvent::SpuWrch(w) if w.channel == 28 => Some(w.value), _ => None,
    }).collect();
    assert_eq!(out_mbox, vec![SPU_SENTINEL]);

    let _desc_bytes = resolve_dma_listdesc_side_file(&trace_path, &dma_dir, desc_sha, Some(DESCRIPTOR_SIZE as usize)).expect("descriptor");
    let chunk_0 = resolve_dma_chunk_side_file(&trace_path, &dma_dir, &elements[0], Some(EL_SIZE_1 as usize)).expect("chunk 0");
    let chunk_1 = resolve_dma_chunk_side_file(&trace_path, &dma_dir, &elements[1], Some(EL_SIZE_2 as usize)).expect("chunk 1");

    let groups: BTreeMap<u32, Vec<TraceEvent>> = captured_events_to_traces_per_spu(&events).expect("transform");
    let target_spu = *groups.keys().next().unwrap();

    let images: Vec<&SpuImageEvent> = events.iter().filter_map(|ev| match ev {
        CapturedEvent::SpuImage(img) => Some(img), _ => None,
    }).collect();
    let image = images[0];

    let r3_initial: u128 = (eals[0] as u128) << 64;
    let r4_initial: u128 = (eals[1] as u128) << 64;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder")
        .with_initial_gpr(3, r3_initial)
        .with_initial_gpr(4, r4_initial);

    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed for PUTLB");
    assert_eq!(plan.dispatched_get_count, 1);
    assert_eq!(plan.tag_stat_queue.len(), 1);

    let post_dma_program = plan.program.with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

    let mut programs = BTreeMap::new();
    programs.insert(target_spu, post_dma_program);

    let interp = replay_per_spu_traces::<InterpreterExecutor>(&groups, &programs).expect("Interpreter");
    let interp = interp.values().next().unwrap();
    let jit = replay_per_spu_traces_with(&groups, &programs, |_| RecompilerExecutor::new()).expect("Recompiler");
    let jit = jit.values().next().unwrap();

    let diff = diff_snapshots(&interp.final_snapshot, &jit.final_snapshot);
    assert!(diff.is_identical(), "diff: {diff:?}");

    // PUTLB final LS at source range matches the captured chunks
    // (same post-replay invariant as PUTL — SPU's bytecode
    // populated LS source pre-dispatch; remains there after the
    // PUTLB walk since LS is not mutated by the list-PUT path).
    let lo1 = LSA_SRC_BASE as usize;
    let hi1 = lo1 + EL_SIZE_1 as usize;
    let lo2 = hi1;
    let hi2 = lo2 + EL_SIZE_2 as usize;
    for (name, snap) in [
        ("Interpreter", &interp.final_snapshot),
        ("Recompiler", &jit.final_snapshot),
    ] {
        assert_eq!(&snap.ls[lo1..hi1], chunk_0.as_slice(), "{name} final LS @ element 0 src");
        assert_eq!(&snap.ls[lo2..hi2], chunk_1.as_slice(), "{name} final LS @ element 1 src");
    }

    eprintln!(
        "[R8.4f-b SUCCESS] {FIXTURE_NAME} replay-validated (17th oracle):\n  \
         PUTLB cmd=0x25 (= PUTL data path + barrier modifier stripped)\n  \
         OUT_MBOX sentinel = 0x{SPU_SENTINEL:08x}\n  \
         Final-snapshot diff: identical"
    );
}
