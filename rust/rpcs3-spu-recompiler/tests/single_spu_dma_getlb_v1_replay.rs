//! R8.4f-a — first MFC GETLB list-DMA + barrier replay-validated
//! oracle (15th oracle). Byte-identical to GETL (R8.4c) except
//! cmd=0x45 (GETL | MFC_BARRIER_MASK).
//!
//! Per RPCS3 `do_list_transfer`, the barrier bit is stripped
//! before the per-element copy (`args.cmd & ~0xf`), so the data
//! path is byte-identical to plain GETL. Barrier ordering
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

const CANONICAL_STATUS: u32 = 0xDF1E_EA3B;
const FIXTURE_NAME: &str = "single_spu_dma_getlb_v1";

const TAG: u32 = 3;
const LSA_DEST_BASE: u32 = 0x10000;
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
fn r8_4f_a_single_spu_dma_getlb_v1_replay_validated_byte_identical() {
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(trace_path.exists(), "trace missing at {}", trace_path.display());

    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("R8.4f-a: parser must accept GETLB trace");
    assert!(!events.is_empty());

    let mfc_cmds: Vec<&SpuMfcCmdEvent> = events.iter().filter_map(|ev| match ev {
        CapturedEvent::SpuMfcCmd(m) => Some(m), _ => None,
    }).collect();
    assert_eq!(mfc_cmds.len(), 1);
    let cmd = mfc_cmds[0];
    assert_eq!(cmd.cmd, 0x45, "GETLB cmd code (= GETL | MFC_BARRIER_MASK)");
    assert_eq!(cmd.tag, TAG);
    assert_eq!(cmd.size, DESCRIPTOR_SIZE);
    assert_eq!(cmd.lsa, LSA_DEST_BASE);
    assert_eq!(cmd.eah, 0);

    let desc_sha = cmd.descriptor_sha256.as_deref().expect("descriptor_sha256");
    let desc_size = cmd.descriptor_size.expect("descriptor_size");
    let elements = cmd.element_chunks.as_deref().expect("element_chunks");
    let sizes = cmd.element_sizes.as_deref().expect("element_sizes");
    let eals = cmd.element_eals.as_deref().expect("element_eals");
    assert_eq!(desc_size, DESCRIPTOR_SIZE);
    assert_eq!(elements.len(), ELEMENT_COUNT);
    assert_eq!(sizes, &[EL_SIZE_1, EL_SIZE_2]);
    assert_ne!(eals[0], eals[1]);

    let dma_completes: Vec<(u32, u32)> = events.iter().filter_map(|ev| match ev {
        CapturedEvent::MfcDmaComplete(c) => Some((c.tag, c.transferred_bytes)), _ => None,
    }).collect();
    assert_eq!(dma_completes, vec![(TAG, EL_SIZE_1 + EL_SIZE_2)]);

    let out_mbox: Vec<u32> = events.iter().filter_map(|ev| match ev {
        CapturedEvent::SpuWrch(w) if w.channel == 28 => Some(w.value), _ => None,
    }).collect();
    assert_eq!(out_mbox, vec![CANONICAL_STATUS]);

    let stop_count = events.iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuStop(s) if s.stop_code == 0x101)).count();
    assert_eq!(stop_count, 1);

    // Side-files exist (descriptor + element chunks all dedup
    // with the R8.4b/c GETL canonical pool, since byte patterns
    // are identical).
    let desc_bytes = resolve_dma_listdesc_side_file(&trace_path, &dma_dir, desc_sha, Some(DESCRIPTOR_SIZE as usize))
        .expect("descriptor side-file");
    assert_eq!(desc_bytes.len(), DESCRIPTOR_SIZE as usize);
    let chunk_0 = resolve_dma_chunk_side_file(&trace_path, &dma_dir, &elements[0], Some(EL_SIZE_1 as usize)).expect("chunk 0");
    let chunk_1 = resolve_dma_chunk_side_file(&trace_path, &dma_dir, &elements[1], Some(EL_SIZE_2 as usize)).expect("chunk 1");

    let groups: BTreeMap<u32, Vec<TraceEvent>> = captured_events_to_traces_per_spu(&events).expect("transform");
    assert_eq!(groups.len(), 1);
    let target_spu = *groups.keys().next().unwrap();

    let images: Vec<&SpuImageEvent> = events.iter().filter_map(|ev| match ev {
        CapturedEvent::SpuImage(img) => Some(img), _ => None,
    }).collect();
    assert_eq!(images.len(), 1);
    let image = images[0];

    let r3_initial: u128 = (eals[0] as u128) << 64;
    let r4_initial: u128 = (eals[1] as u128) << 64;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder")
        .with_initial_gpr(3, r3_initial)
        .with_initial_gpr(4, r4_initial);

    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed for GETLB");
    assert_eq!(plan.dispatched_get_count, 1);
    assert_eq!(plan.tag_stat_queue.len(), 1);

    let post_dma_program = plan.program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

    // GETLB has GETL semantics: chunks copied into LS at
    // cumulative offset (= GETL byte-identical path).
    let ls = &post_dma_program.segments[0].data;
    let lo1 = LSA_DEST_BASE as usize;
    let hi1 = lo1 + EL_SIZE_1 as usize;
    let lo2 = hi1;
    let hi2 = lo2 + EL_SIZE_2 as usize;
    assert_eq!(&ls[lo1..hi1], chunk_0.as_slice(), "GETLB element 0 in LS at cumulative offset");
    assert_eq!(&ls[lo2..hi2], chunk_1.as_slice(), "GETLB element 1 in LS at cumulative offset");

    let mut programs = BTreeMap::new();
    programs.insert(target_spu, post_dma_program);

    let interp = replay_per_spu_traces::<InterpreterExecutor>(&groups, &programs)
        .expect("replay × Interpreter"); let interp = interp.values().next().unwrap();
    let jit = replay_per_spu_traces_with(&groups, &programs, |_| RecompilerExecutor::new())
        .expect("replay × Recompiler"); let jit = jit.values().next().unwrap();

    let diff = diff_snapshots(&interp.final_snapshot, &jit.final_snapshot);
    assert!(diff.is_identical(), "diff_snapshots: {diff:?}");

    eprintln!(
        "[R8.4f-a SUCCESS] {FIXTURE_NAME} replay-validated (15th oracle):\n  \
         GETLB cmd=0x45 tag={} size={} (= GETL data path + barrier modifier stripped at do_list_transfer)\n  \
         OUT_MBOX = 0x{CANONICAL_STATUS:08x}\n  \
         Final-snapshot diff: identical",
        cmd.tag, cmd.size,
    );
}
