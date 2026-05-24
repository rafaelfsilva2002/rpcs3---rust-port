//! R8.5e E.6 — first MFC PUTL **stall-and-notify** (`sb & 0x80`)
//! replay-validated oracle (20th oracle). Symmetric inverse of
//! R8.5d D.6 (`single_spu_dma_getl_stall_v1`): a single MFC
//! PUTL dispatch (cmd=0x24) with three elements writes data
//! FROM SPU LS TO RPCS3 EA via the same 8-byte BE descriptor
//! format, with element 1 carrying sb=0x80 to trigger the
//! transfer-then-stall handshake (Cell BE Sec. 12.5).
//!
//! Captured invariants (R8.5e E.5 → E.6):
//! - PUTL cmd=0x24 with 5 additive fields populated (descriptor
//!   sha + 3 element chunks + sizes [128,64,96] + 3 eals).
//! - Element 0 source: 128 B counting pattern at
//!   LS[0x10000..0x10080] (.dmachunk SHA shared with GETL +
//!   GET + PUT + getl_stall pool — perfect dedup).
//! - Element 1 source: 64 B constant 0x42 at
//!   LS[0x10080..0x100C0] — transferred BEFORE the stall
//!   raises (Cell BE Sec. 12.5).
//! - Element 2 source: 96 B constant 0x11 at
//!   LS[0x100C0..0x10120] — transferred only after the SPU's
//!   ch26 ack.
//! - Exactly 1 `spu_rdch` on ch25 with value=0x08 (= 1 << tag).
//! - Exactly 1 `spu_wrch` on ch26 with value=3 (tag id, NOT mask).
//! - `mfc_dma_complete.transferred_bytes = 288` (= 128+64+96),
//!   confirming Cell BE Sec. 12.5 transfer-then-stall.
//! - SPU writes FIXED sentinel `0xC0FFEEC3` to OUT_MBOX (the
//!   PPU computes `ea_status = 0xA12FDC1E` post-join from EA
//!   reads — both halves of the canonical TTY).
//!
//! PUTL replay semantics differ from GETL on LS handling
//! (mirrors PUTL_v1's R8.4e contract):
//! - GETL: replay COPIES chunk bytes into LS at cumulative
//!   offset (the SPU's reads after `wrch ch21` see them).
//! - PUTL: replay does NOT mutate LS. The SPU's own bytecode
//!   already populated LS[lsa..lsa+sum(ts)] with the source
//!   bytes BEFORE the `wrch ch21` dispatch — replay verifies
//!   the dispatch-time SPU LS matches the captured chunk
//!   bytes POST-execution (the SPU's interpreter walk has by
//!   then re-derived the same LS content via the captured
//!   spu_image + bytecode).
//!
//! Plumbing chain validated end-to-end:
//! - R8.5b: writer captures ch25/ch26 events as Schema A
//!   spu_rdch/spu_wrch.
//! - R8.5c: replay state machine drives `process_mfc_list_cmd`
//!   transfer-then-stall + `process_spu_rdch_list_stall_stat`
//!   + `process_spu_wrch_list_stall_ack`, both with
//!   `is_putl=true` semantics.
//! - R8.5d D.2: C++ bridge runtime resumes via
//!   `bridge_dma_list_stall_ack_callback` with `is_put=true`.
//! - R8.5d D.6: `SpuProgram::with_mfc_list_stall_mask` +
//!   `DmaPreReplayPlan::initial_list_stall_mask` route the
//!   captured ch25 value to the SPU's runtime mask register.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    apply_mfc_dma_pre_replay, build_spu_program_from_captured_image,
    captured_events_to_traces_per_spu, diff_snapshots, parse_jsonl_trace, replay_per_spu_traces,
    replay_per_spu_traces_with, resolve_dma_chunk_side_file, resolve_dma_listdesc_side_file,
    CapturedEvent, InterpreterExecutor, SpuImageEvent, SpuMfcCmdEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

const SPU_SENTINEL: u32 = 0xC0FF_EEC3;
const FIXTURE_NAME: &str = "single_spu_dma_putl_stall_v1";

const TAG: u32 = 3;
const LSA_SRC_BASE: u32 = 0x10000;
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
fn r8_5e_e6_single_spu_dma_putl_stall_v1_replay_validated_byte_identical() {
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();
    let dma_dir = fixture_dma_dir();

    assert!(trace_path.exists(), "trace missing at {}", trace_path.display());
    assert!(images_dir.exists());
    assert!(dma_dir.exists());

    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw)
        .expect("R8.5e E.6: parser must accept PUTL stall-and-notify trace");
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
    assert_eq!(cmd.size, DESCRIPTOR_SIZE, "size = 3 elements * 8 bytes");
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
    assert_eq!(sizes[2], EL_SIZE_3);
    // All 3 element EAs distinct (no aliasing).
    assert_ne!(eals[0], eals[1]);
    assert_ne!(eals[1], eals[2]);
    assert_ne!(eals[0], eals[2]);

    // R8.5e E.6 — stall handshake: exactly 1 ch25 read (returns
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

    // ch28 carries the FIXED SPU sentinel (NOT a computed status
    // — for PUTL the SPU has no post-DMA reason to read back
    // from LS, so the post-PUTL outcome split: SPU emits a fixed
    // sentinel via OUT_MBOX; PPU computes ea_status from EA
    // buffer sums post-join).
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
        .expect("transform must succeed for PUTL stall-and-notify");
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

    // Seed GPRs from PSL1GHT arg packing (same packing as
    // getl_stall_v1):
    //   arg0 (spu_id u64) = (EA1 << 32) | EA2  →  r3 = (EA1 << 96) | (EA2 << 64)
    //   arg1 (arg    u64) = EA3 << 32          →  r4 = EA3 << 96
    let r3_initial: u128 = ((eals[0] as u128) << 96) | ((eals[1] as u128) << 64);
    let r4_initial: u128 = (eals[2] as u128) << 96;
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    let program = build_spu_program_from_captured_image(&image_path, image, 1_000_000)
        .expect("builder must succeed")
        .with_initial_gpr(3, r3_initial)
        .with_initial_gpr(4, r4_initial);

    // R8.5c — pre-replay walks the PUTL via process_mfc_list_cmd
    // (PUTL branch leaves LS untouched — `is_putl=true`).
    // The SPU's own bytecode populates LS during the replay run.
    // R8.5d D.6 — pre-replay records the captured ch25 stall mask
    // BEFORE its destructive read consumes it; D.6 plumbs the
    // value through `SpuProgram::with_mfc_list_stall_mask` so the
    // SPU's `rdch ch25` during replay returns the same value.
    let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &dma_dir, program)
        .expect("apply_mfc_dma_pre_replay must succeed for PUTL stall-and-notify");
    assert_eq!(plan.dispatched_get_count, 1, "exactly 1 MFC dispatch (1 PUTL = 1 cmd)");
    assert_eq!(plan.tag_stat_queue.len(), 1);
    assert_eq!(plan.initial_list_stall_mask, 1u32 << TAG);

    let post_dma_program = plan
        .program
        .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect())
        .with_mfc_list_stall_mask(plan.initial_list_stall_mask);

    // PUTL semantics: do NOT pre-assert LS source bytes pre-
    // replay — the SPU's own bytecode populates them at runtime
    // (unlike GETL where pre-replay copies chunk bytes into LS).

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

    // Post-replay: verify SPU's final LS at all 3 source
    // regions matches the captured chunk bytes. The SPU's
    // bytecode wrote those bytes BEFORE the PUTL dispatch;
    // after the entire replay walk, LS at the source range
    // must still hold them (PUTL does not modify LS).
    let lo1 = LSA_SRC_BASE as usize;
    let hi1 = lo1 + EL_SIZE_1 as usize;
    let lo2 = hi1;
    let hi2 = lo2 + EL_SIZE_2 as usize;
    let lo3 = hi2;
    let hi3 = lo3 + EL_SIZE_3 as usize;
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
            "{name} final LS @ element 1 source (stalled — LS[lsa+128..lsa+192])"
        );
        assert_eq!(
            &snap.ls[lo3..hi3],
            chunk_2.as_slice(),
            "{name} final LS @ element 2 source (post-ack — LS[lsa+192..lsa+288])"
        );
    }

    assert_eq!(interp.final_snapshot.channels.out_mbox, None);
    assert_eq!(jit.final_snapshot.channels.out_mbox, None);

    eprintln!(
        "[R8.5e E.6 SUCCESS] {FIXTURE_NAME} replay-validated (20th oracle):\n  \
         target_spu={target_spu}\n  \
         events={}\n  \
         spu_image sha={}\n  \
         PUTL cmd=0x24 tag={} size={} (descriptor bytes)\n  \
         descriptor sha={}\n  \
         element 0 src: size={} sha={} ea=0x{:x}\n  \
         element 1 src: size={} sha={} ea=0x{:x} [sb=0x80 STALL]\n  \
         element 2 src: size={} sha={} ea=0x{:x}\n  \
         ch25 stall_mask=0x{:x} → ch26 ack tag={}\n  \
         transferred_bytes (sum ts of all 3 elements) = {}\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         SPU sentinel = 0x{SPU_SENTINEL:08x} (PPU computes ea_status=0xA12FDC1E)\n  \
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
