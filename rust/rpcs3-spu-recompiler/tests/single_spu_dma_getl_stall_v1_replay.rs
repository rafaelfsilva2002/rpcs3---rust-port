//! R8.5d D.6 — first MFC GETL **stall-and-notify** (`sb & 0x80`)
//! replay-validated oracle (19th oracle). Promotes the R8.5d D.5
//! capture-only fixture `single_spu_dma_getl_stall_v1` to a full
//! replay-validated oracle: the trace's 3-element list descriptor
//! is loaded from the new `.dmalistdesc` side-file, each element's
//! data chunk is content-addressed via the existing pool (with
//! 2/3 elements deduped against GETL_v1), the stall-and-notify
//! handshake is replayed via R8.5c's `process_spu_rdch_list_stall_stat`
//! + `process_spu_wrch_list_stall_ack`, and the SPU's final
//! OUT_MBOX status matches the canonical `0xDF1EEC3A`.
//!
//! Captured invariants (R8.5d D.5 → D.6):
//! - GETL cmd=0x44 with 5 additive fields populated (descriptor
//!   sha + 3 element chunks + sizes [128,64,96] + 3 eals).
//! - Element 0: 128 B counting pattern at LS[0x10000..0x10080].
//! - Element 1:  64 B constant 0x42 at LS[0x10080..0x100C0]
//!   — transferred BEFORE the stall raises (Cell BE Sec. 12.5).
//! - Element 2:  96 B constant 0x11 at LS[0x100C0..0x10120]
//!   — transferred only after the SPU's ch26 ack.
//! - Exactly 1 `spu_rdch` on ch25 with value=0x08 (= 1 << tag).
//! - Exactly 1 `spu_wrch` on ch26 with value=3 (tag id, NOT mask).
//! - `mfc_dma_complete.transferred_bytes = 288` (= 128+64+96).
//! - SPU sums all three regions:
//!   `((sum1 << 16) | (sum2 + sum3)) ^ 0xC0DEFADA = 0xDF1EEC3A`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, resolve_dma_listdesc_side_file,
    CapturedEvent, InterpreterExecutor, SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

const CANONICAL_STATUS: u32 = 0xDF1E_EC3A;
const FIXTURE_NAME: &str = "single_spu_dma_getl_stall_v1";

const TAG: u32 = 3;
const LSA_DEST_BASE: u32 = 0x10000;
const DESCRIPTOR_SIZE: u32 = 24; // 3 elements × 8 bytes
const ELEMENT_COUNT: usize = 3;
const EL_SIZE_1: u32 = 128;
const EL_SIZE_2: u32 = 64;
const EL_SIZE_3: u32 = 96;
const TRANSFERRED_BYTES: u32 = EL_SIZE_1 + EL_SIZE_2 + EL_SIZE_3; // 288

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
fn r8_5d_d6_single_spu_dma_getl_stall_v1_replay_validated_byte_identical() {
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(trace_path.exists(), "trace missing at {}", trace_path.display());
    assert!(images_dir.exists());
    assert!(dma_dir.exists());

    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw)
        .expect("R8.5d D.6: parser must accept GETL stall-and-notify trace");
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
    assert_eq!(cmd.size, DESCRIPTOR_SIZE, "size = 3 elements * 8 bytes");
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
    assert_eq!(sizes[2], EL_SIZE_3);
    // All 3 element EAs distinct (no aliasing).
    assert_ne!(eals[0], eals[1]);
    assert_ne!(eals[1], eals[2]);
    assert_ne!(eals[0], eals[2]);

    // R8.5d D.6 — stall handshake: exactly 1 ch25 read (returns
    // stall mask) + exactly 1 ch26 write (acks tag id, NOT mask).
    let rdch_25: Vec<Option<u32>> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuRdch(r) if r.channel == 25 => Some(r.value),
            _ => None,
        })
        .collect();
    assert_eq!(
        rdch_25,
        vec![Some(1u32 << TAG)],
        "ch25 MFC_RdListStallStat must return per-tag stall mask exactly once"
    );

    let wrch_26: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 26 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(
        wrch_26,
        vec![TAG],
        "ch26 MFC_WrListStallAck must write tag id (NOT bitmask) exactly once"
    );

    // Exactly 1 mfc_dma_complete; transferred_bytes = sum(ts) for
    // ALL 3 elements (Cell BE Sec. 12.5: includes stalled element
    // 1 before stall raise + element 2 after ack resume).
    let dma_completes: Vec<(u32, u32)> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::MfcDmaComplete(c) => Some((c.tag, c.transferred_bytes)),
            _ => None,
        })
        .collect();
    assert_eq!(dma_completes, vec![(TAG, TRANSFERRED_BYTES)]);

    // ch22/ch23/ch24 standard wait (mask=0x08, ALL → 0x08) AFTER
    // the ack (the homebrew waits for full-list tag-stat
    // completion post-resume).
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

    // Side-files exist and content-address verifies.
    let desc_bytes = resolve_dma_listdesc_side_file(
        &trace_path,
        &dma_dir,
        desc_sha,
        Some(DESCRIPTOR_SIZE as usize),
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
    let chunk_2 = resolve_dma_chunk_side_file(
        &trace_path, &dma_dir, &elements[2], Some(EL_SIZE_3 as usize),
    )
    .expect("element 2 chunk must resolve");
    assert_eq!(chunk_0.len(), EL_SIZE_1 as usize);
    assert_eq!(chunk_1.len(), EL_SIZE_2 as usize);
    assert_eq!(chunk_2.len(), EL_SIZE_3 as usize);

    // Per-SPU transformer.
    let groups: BTreeMap<u32, Vec<TraceEvent>> = captured_events_to_traces_per_spu(&events)
        .expect("transform must succeed for GETL stall-and-notify");
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

    // Seed GPRs from PSL1GHT arg packing:
    //   arg0 (spu_id u64) = (EA1 << 32) | EA2  →  r3 = (EA1 << 96) | (EA2 << 64)
    //   arg1 (arg    u64) = EA3 << 32          →  r4 = EA3 << 96
    let r3_initial: u128 = ((eals[0] as u128) << 96) | ((eals[1] as u128) << 64);
    let r4_initial: u128 = (eals[2] as u128) << 96;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder must succeed")
        .with_initial_gpr(3, r3_initial)
        .with_initial_gpr(4, r4_initial);

    // R8.5c — pre-replay walks the GETL via process_mfc_list_cmd,
    // observes element 1's sb=0x80 AFTER transferring it (Cell BE
    // Sec. 12.5), saves a ListDmaPartialProgress + sets
    // list_stall_mask, and then the captured ch25 read consumes
    // the mask, the captured ch26 write resumes the walk to
    // completion. Final transferred_bytes = 288, all 3 elements
    // landed at their cumulative offsets.
    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed for GETL stall-and-notify");
    assert_eq!(plan.dispatched_get_count, 1, "exactly 1 MFC dispatch (1 GETL = 1 cmd)");
    assert_eq!(plan.tag_stat_queue.len(), 1);
    // R8.5d D.6 — pre-replay records the captured ch25 stall mask
    // BEFORE its destructive read consumes it, then the test
    // plumbs it through `SpuProgram::with_mfc_list_stall_mask` so
    // the SPU's `rdch ch25` during replay returns the same value.
    assert_eq!(plan.initial_list_stall_mask, 1u32 << TAG);

    let post_dma_program = plan
        .program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect())
        .with_mfc_list_stall_mask(plan.initial_list_stall_mask);

    // Sanity check: pre-replay LS state has all 3 elements landed
    // at their cumulative offsets.
    let ls = &post_dma_program.segments[0].data;
    let lo1 = LSA_DEST_BASE as usize;
    let hi1 = lo1 + EL_SIZE_1 as usize;
    assert_eq!(&ls[lo1..hi1], chunk_0.as_slice(), "element 0 at LSA_DEST_BASE");
    let lo2 = hi1;
    let hi2 = lo2 + EL_SIZE_2 as usize;
    assert_eq!(
        &ls[lo2..hi2],
        chunk_1.as_slice(),
        "element 1 (stalled) at cumulative offset — Cell BE Sec. 12.5"
    );
    let lo3 = hi2;
    let hi3 = lo3 + EL_SIZE_3 as usize;
    assert_eq!(
        &ls[lo3..hi3],
        chunk_2.as_slice(),
        "element 2 (post-ack resume) at cumulative offset"
    );

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

    // Post-replay verify final LS regions for all 3 elements,
    // both backends.
    for (name, snap) in [
        ("Interpreter", &interp.final_snapshot),
        ("Recompiler", &jit.final_snapshot),
    ] {
        assert_eq!(&snap.ls[lo1..hi1], chunk_0.as_slice(), "{name} final LS @ element 0");
        assert_eq!(&snap.ls[lo2..hi2], chunk_1.as_slice(), "{name} final LS @ element 1");
        assert_eq!(&snap.ls[lo3..hi3], chunk_2.as_slice(), "{name} final LS @ element 2");
    }

    assert_eq!(interp.final_snapshot.channels.out_mbox, None);
    assert_eq!(jit.final_snapshot.channels.out_mbox, None);

    eprintln!(
        "[R8.5d D.6 SUCCESS] {FIXTURE_NAME} replay-validated (19th oracle):\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         GETL cmd=0x44 tag={} size={} (descriptor bytes)\n  \
         descriptor sha={}\n  \
         element 0: size={} sha={} ea=0x{:x}\n  \
         element 1: size={} sha={} ea=0x{:x} [sb=0x80 STALL]\n  \
         element 2: size={} sha={} ea=0x{:x}\n  \
         ch25 stall_mask=0x{:x} → ch26 ack tag={}\n  \
         transferred_bytes (sum ts of all 3 elements) = {}\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         OUT_MBOX = 0x{CANONICAL_STATUS:08x}\n  \
         Final-snapshot diff: identical",
        events.len(),
        image.image_sha256,
        cmd.tag, cmd.size,
        desc_sha,
        sizes[0], elements[0], eals[0],
        sizes[1], elements[1], eals[1],
        sizes[2], elements[2], eals[2],
        1u32 << TAG, TAG,
        TRANSFERRED_BYTES,
        interp.total_steps,
        jit.total_steps,
    );
}
