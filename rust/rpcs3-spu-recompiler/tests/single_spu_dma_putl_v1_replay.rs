//! R8.4e — first MFC PUTL list-DMA replay-validated oracle
//! (14th oracle). Symmetric inverse of R8.4c GETL: a single
//! MFC PUTL dispatch (cmd=0x24) with two elements writes data
//! FROM SPU LS TO RPCS3 EA via the same 8-byte BE descriptor
//! format.
//!
//! Captured invariants:
//! - PUTL cmd=0x24 with 5 additive fields populated.
//! - Element 0 source: 128 B counting pattern at
//!   LS[0x10000..0x10080] (.dmachunk SHA shared with GETL +
//!   GET + PUT pool — perfect dedup).
//! - Element 1 source: 64 B constant 0x42 at
//!   LS[0x10080..0x100C0] (.dmachunk SHA shared with R8.2+
//!   pool).
//! - mfc_dma_complete transferred_bytes = 192 (= sum of ts).
//! - SPU writes fixed sentinel `0xC0FFEEBA` to OUT_MBOX (the
//!   PPU computes `ea_status = 0xA12FDA7E` post-join from EA
//!   reads — both halves of the canonical TTY).
//!
//! PUTL replay semantics differ from GETL on LS handling:
//! - GETL: replay COPIES chunk bytes into LS at cumulative
//!   offset (the SPU's reads after `wrch ch21` see them).
//! - PUTL: replay does NOT mutate LS. The SPU's own bytecode
//!   already populated `LS[lsa..lsa+sum(ts)]` with the source
//!   bytes BEFORE the `wrch ch21` dispatch — replay verifies
//!   the dispatch-time SPU LS matches the captured chunk bytes
//!   POST-execution (the SPU's interpreter walk has by then
//!   re-derived the same LS content via the captured spu_image
//!   + bytecode).

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, resolve_dma_listdesc_side_file,
    CapturedEvent, InterpreterExecutor, SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

const SPU_SENTINEL: u32 = 0xC0FF_EEBA;
const FIXTURE_NAME: &str = "single_spu_dma_putl_v1";

const TAG: u32 = 3;
const LSA_SRC_BASE: u32 = 0x10000;
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
fn r8_4e_single_spu_dma_putl_v1_replay_validated_byte_identical() {
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(trace_path.exists(), "trace missing at {}", trace_path.display());
    assert!(images_dir.exists());
    assert!(dma_dir.exists());

    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("R8.4e: parser must accept PUTL trace");
    assert!(!events.is_empty());

    // Exactly 1 spu_mfc_cmd event with cmd=0x24 PUTL.
    let mfc_cmds: Vec<&SpuMfcCmdEvent> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuMfcCmd(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(mfc_cmds.len(), 1);
    let cmd = mfc_cmds[0];
    assert_eq!(cmd.cmd, 0x24, "PUTL cmd");
    assert_eq!(cmd.tag, TAG);
    assert_eq!(cmd.size, DESCRIPTOR_SIZE, "size = 2 elements * 8 bytes");
    assert_eq!(cmd.lsa, LSA_SRC_BASE);
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

    // ch28 carries the SPU sentinel `0xC0FFEEBA`. The PPU's
    // ea_status (`0xA12FDA7E`) lives OUTSIDE the SPU trace —
    // it's computed by the PPU post-join.
    let out_mbox: Vec<u32> = events
        .iter()
        .filter_map(|ev| match ev {
            CapturedEvent::SpuWrch(w) if w.channel == 28 => Some(w.value),
            _ => None,
        })
        .collect();
    assert_eq!(out_mbox, vec![SPU_SENTINEL]);

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
        captured_events_to_traces_per_spu(&events).expect("transform must succeed for PUTL");
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

    // R8.4e — pre-replay walks the PUTL via process_mfc_list_cmd
    // (PUTL branch leaves LS untouched; only validates side-files).
    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed for PUTL");
    assert_eq!(plan.dispatched_get_count, 1, "exactly 1 MFC dispatch (1 PUTL = 1 cmd)");
    assert_eq!(plan.tag_stat_queue.len(), 1);

    let post_dma_program = plan
        .program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

    // Sanity: PUTL did NOT pre-populate LS source (unlike GETL).
    // The SPU's own bytecode populates LS during execution.
    // We DO NOT assert chunk bytes exist at LS[lsa..] pre-replay
    // — that would be GETL semantics.

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

    // Post-replay: verify SPU's final LS at the source regions
    // matches the captured chunk bytes. The SPU's bytecode wrote
    // those bytes BEFORE the PUTL dispatch; after the entire
    // replay walk, LS at the source range must still hold them.
    let lo1 = LSA_SRC_BASE as usize;
    let hi1 = lo1 + EL_SIZE_1 as usize;
    let lo2 = hi1;
    let hi2 = lo2 + EL_SIZE_2 as usize;
    for (name, snap) in [
        ("Interpreter", &interp.final_snapshot),
        ("Recompiler", &jit.final_snapshot),
    ] {
        assert_eq!(
            &snap.ls[lo1..hi1],
            chunk_0.as_slice(),
            "{name} final LS @ element 0 source (LS[lsa..lsa+128])"
        );
        assert_eq!(
            &snap.ls[lo2..hi2],
            chunk_1.as_slice(),
            "{name} final LS @ element 1 source (LS[lsa+128..lsa+192])"
        );
    }

    assert_eq!(interp.final_snapshot.channels.out_mbox, None);
    assert_eq!(jit.final_snapshot.channels.out_mbox, None);

    eprintln!(
        "[R8.4e SUCCESS] {FIXTURE_NAME} replay-validated (14th oracle):\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         PUTL cmd=0x24 tag={} size={} (descriptor bytes)\n  \
         descriptor sha={}\n  \
         element 0 src: size={} sha={} ea=0x{:x}\n  \
         element 1 src: size={} sha={} ea=0x{:x}\n  \
         transferred_bytes (sum ts) = {}\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         SPU sentinel (OUT_MBOX) = 0x{SPU_SENTINEL:08x}\n  \
         (PPU-computed ea_status = 0xA12FDA7E — outside SPU trace)\n  \
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
