//! R8.4c — first MFC GETL list-DMA replay-validated oracle
//! (13th oracle). Promotes the R8.4b capture-only fixture
//! single_spu_dma_getl_v1 to a full replay-validated oracle:
//! the trace's 2-element list descriptor is loaded from the
//! new `.dmalistdesc` side-file, each element's data chunk
//! from the existing `.dmachunk` pool, both elements land in
//! LS at cumulative offsets, and the SPU's final OUT_MBOX
//! status matches the canonical `0xDF1EEA5A`.
//!
//! Captured invariants (R8.4b → R8.4c):
//! - GETL cmd=0x44 with 5 additive fields populated.
//! - Element 0: 128 B counting pattern at LS[0x10000..0x10080].
//! - Element 1: 64 B constant 0x42 at LS[0x10080..0x100C0].
//! - mfc_dma_complete transferred_bytes = 192 (= sum of ts).
//! - SPU sums both regions: `((sum1 << 16) | sum2) ^
//!   0xC0DEFADA = 0xDF1EEA5A`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, resolve_dma_listdesc_side_file,
    CapturedEvent, InterpreterExecutor, SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

const CANONICAL_STATUS: u32 = 0xDF1E_EA5A;
const FIXTURE_NAME: &str = "single_spu_dma_getl_v1";

const TAG: u32 = 3;
const LSA_DEST_BASE: u32 = 0x10000;
const DESCRIPTOR_SIZE: u32 = 16;  // 2 elements × 8 bytes
const ELEMENT_COUNT: usize = 2;
const EL_SIZE_1: u32 = 128;
const EL_SIZE_2: u32 = 64;

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
fn r8_4c_single_spu_dma_getl_v1_replay_validated_byte_identical() {
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(trace_path.exists(), "trace missing at {}", trace_path.display());
    assert!(images_dir.exists());
    assert!(dma_dir.exists());

    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("R8.4c: parser must accept GETL trace");
    assert!(!events.is_empty());

    // Exactly 1 spu_mfc_cmd event with cmd=0x44 GETL.
    let mfc_cmds: Vec<&SpuMfcCmdEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuMfcCmd(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(mfc_cmds.len(), 1);
    let cmd = mfc_cmds[0];
    assert_eq!(cmd.cmd, 0x44, "GETL cmd");
    assert_eq!(cmd.tag, TAG);
    assert_eq!(cmd.size, DESCRIPTOR_SIZE, "size = 2 elements * 8 bytes");
    assert_eq!(cmd.lsa, LSA_DEST_BASE);
    assert_eq!(cmd.eah, 0);

    // Additive list fields all present + consistent.
    let desc_sha = cmd.descriptor_sha256.as_deref().expect("descriptor_sha256");
    let desc_size = cmd.descriptor_size.expect("descriptor_size");
    let elements = cmd.element_chunks.as_deref().expect("element_chunks");
    let sizes = cmd.element_sizes.as_deref().expect("element_sizes");
    let eals = cmd.element_eals.as_deref().expect("element_eals");

    assert_eq!(desc_size, DESCRIPTOR_SIZE);
    assert_eq!(elements.len(), ELEMENT_COUNT);
    assert_eq!(sizes.len(), ELEMENT_COUNT);
    assert_eq!(eals.len(), ELEMENT_COUNT);
    assert_eq!(sizes[0], EL_SIZE_1);
    assert_eq!(sizes[1], EL_SIZE_2);
    assert_ne!(eals[0], eals[1], "elements point to distinct EA buffers");

    // Exactly 1 mfc_dma_complete; transferred_bytes = sum(ts).
    let dma_completes: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::MfcDmaComplete(c) => Some((c.tag, c.transferred_bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(dma_completes, vec![(TAG, EL_SIZE_1 + EL_SIZE_2)]);

    // ch22/ch23/ch24 standard wait (mask=0x08, ALL → 0x08).
    let wrch_22: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 22 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(wrch_22, vec![1u32 << TAG]);

    let wrch_23: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 23 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(wrch_23, vec![2u32], "ALL mode");

    let rdch_24: Vec<Option<u32>> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuRdch(r) if r.channel == 24 => Some(r.value),
            _ => None,
        })
        .collect();
    assert_eq!(rdch_24, vec![Some(1u32 << TAG)]);

    // ch28 carries canonical status.
    let out_mbox: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 28 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(out_mbox, vec![CANONICAL_STATUS]);

    let stop_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuStop(s) if s.stop_code == 0x101))
        .count();
    assert_eq!(stop_count, 1);

    // Side-files exist and content-address verifies (load both
    // directly, then run the full pipeline).
    let desc_bytes = resolve_dma_listdesc_side_file(
        &trace_path, &dma_dir, desc_sha, Some(DESCRIPTOR_SIZE as usize),
    )
    .expect("descriptor side-file must resolve");
    assert_eq!(desc_bytes.len(), DESCRIPTOR_SIZE as usize);

    let chunk_0 = resolve_dma_chunk_side_file(
        &trace_path, &dma_dir, &elements[0], Some(EL_SIZE_1 as usize),
    )
    .expect("element 0 chunk must resolve");
    let chunk_1 = resolve_dma_chunk_side_file(
        &trace_path, &dma_dir, &elements[1], Some(EL_SIZE_2 as usize),
    )
    .expect("element 1 chunk must resolve");
    assert_eq!(chunk_0.len(), EL_SIZE_1 as usize);
    assert_eq!(chunk_1.len(), EL_SIZE_2 as usize);

    // Per-SPU transformer.
    let groups: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("transform must succeed for GETL");
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

    // Seed r3 = EA1 + r4 = EA2 (PSL1GHT arg0 + arg1).
    let r3_initial: u128 = (eals[0] as u128) << 64;
    let r4_initial: u128 = (eals[1] as u128) << 64;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder must succeed")
        .with_initial_gpr(3, r3_initial)
        .with_initial_gpr(4, r4_initial);

    // R8.4c — pre-replay walks the GETL via the new path
    // (process_mfc_list_cmd through process_mfc_cmd_pre_replay).
    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed for GETL");
    assert_eq!(plan.dispatched_get_count, 1, "exactly 1 MFC dispatch (1 GETL = 1 cmd)");
    assert_eq!(plan.tag_stat_queue.len(), 1);

    let post_dma_program = plan
        .program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

    // Sanity check: pre-replay LS state has element 0 + element 1
    // landed at their cumulative offsets.
    let ls = &post_dma_program.segments[0].data;
    let lo1 = LSA_DEST_BASE as usize;
    let hi1 = lo1 + EL_SIZE_1 as usize;
    assert_eq!(&ls[lo1..hi1], chunk_0.as_slice(), "element 0 at LSA_DEST_BASE");
    let lo2 = hi1;
    let hi2 = lo2 + EL_SIZE_2 as usize;
    assert_eq!(&ls[lo2..hi2], chunk_1.as_slice(), "element 1 at cumulative offset");

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

    // Post-replay verify final LS regions for both backends.
    for (name, snap) in [
        ("Interpreter", &interp.final_snapshot),
        ("Recompiler", &jit.final_snapshot),
    ] {
        assert_eq!(&snap.ls[lo1..hi1], chunk_0.as_slice(), "{name} final LS @ element 0");
        assert_eq!(&snap.ls[lo2..hi2], chunk_1.as_slice(), "{name} final LS @ element 1");
    }

    assert_eq!(interp.final_snapshot.channels.out_mbox, None);
    assert_eq!(jit.final_snapshot.channels.out_mbox, None);

    eprintln!(
        "[R8.4c SUCCESS] {FIXTURE_NAME} replay-validated (13th oracle):\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         GETL cmd=0x44 tag={} size={} (descriptor bytes)\n  \
         descriptor sha={}\n  \
         element 0: size={} sha={} ea=0x{:x}\n  \
         element 1: size={} sha={} ea=0x{:x}\n  \
         transferred_bytes (sum ts) = {}\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         OUT_MBOX = 0x{CANONICAL_STATUS:08x}\n  \
         Final-snapshot diff: identical",
        events.len(),
        image.image_sha256,
        cmd.tag, cmd.size,
        desc_sha,
        sizes[0], elements[0], eals[0],
        sizes[1], elements[1], eals[1],
        EL_SIZE_1 + EL_SIZE_2,
        interp.total_steps,
        jit.total_steps,
    );
}
