//! R6.4b-replay — fifth replay-validated fixture integration test.
//!
//! Loads `behavior-freeze/fixtures/spu/traces/single_spu_mailbox_multi_v1.jsonl`
//! and the corresponding `.spuimg` side-file, runs the full pipeline
//! (parser → per-SPU transformer → builder → replay × Interpreter +
//! replay × Recompiler), and asserts byte-identical agreement.
//!
//! The fixture exercises a **two-PPU-input** workload: the PPU sends
//! one value via IN_MBOX (ch29, `sysSpuThreadWriteMb`) and a second
//! value via SNR1 (ch3, `sysSpuThreadWriteSignal`). The SPU reads
//! both, combines them into a single OUT_MBOX value, and stops with
//! `0x101`. Canonical inputs `cmd1=0x100`, `cmd2=0x200` produce
//! `OUT_MBOX = (cmd2 + 0xB2) + (cmd1 + 0xA1) = 0x453`.
//!
//! Why this fixture exists in addition to mailbox/branch_loop/
//! loadstore/signal: it's the first fixture that uses BOTH
//! `ppu_push_inmbox` AND `ppu_signal` on the same SPU thread. The
//! per-SPU transformer must merge these two PPU-side input streams
//! into a single ordered TraceEvent vector that the replay engine
//! can drive end-to-end. Capturing this from a real RPCS3 binary
//! (R6.4b-toolchain landed `.self`) ensures the writer + transformer
//! handle the mixed-channel case identically across Interpreter and
//! Recompiler backends.
//!
//! Why this matters for the C++↔Rust SPU bridge (R6.4b): the bridge
//! delegates SPU execution to the Rust interpreter+recompiler. If
//! they ever diverge on this trace, the bridge would silently
//! produce wrong results for any workload that uses both channels.
//! This test is the regression sentinel for that.

use std::collections::BTreeMap;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    build_spu_program_from_captured_image, captured_events_to_traces_per_spu, diff_snapshots,
    parse_jsonl_trace, replay_per_spu_traces, replay_per_spu_traces_with, CapturedEvent,
    InterpreterExecutor, SpuImageEvent, TraceEvent,
};
use rpcs3_spu_recompiler::RecompilerExecutor;

fn fixture_trace_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("behavior-freeze");
    p.push("fixtures");
    p.push("spu");
    p.push("traces");
    p.push("single_spu_mailbox_multi_v1.jsonl");
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

#[test]
fn r6_4b_single_spu_mailbox_multi_v1_replay_validated_byte_identical() {
    // ===== 1. Fixture must exist on disk. =====
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();

    assert!(
        trace_path.exists(),
        "fixture trace not found at {}",
        trace_path.display(),
    );
    assert!(
        images_dir.exists(),
        "fixture images dir not found at {}",
        images_dir.display(),
    );

    // ===== 2. Parse the JSONL trace. =====
    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("parser must accept the captured trace");
    assert!(!events.is_empty(), "trace has no events");

    // ===== 3. Verify acceptance criteria. =====
    // Acceptance #1: NO DMA. Zero spu_wrch ch21 (MFC_Cmd) events.
    let mfc_cmd_events: Vec<_> = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuWrch(w) if w.channel == 21))
        .collect();
    assert!(
        mfc_cmd_events.is_empty(),
        "fixture must NOT contain any spu_wrch ch21 (MFC_Cmd) events; \
         found {} — fixture would be DMA-bound",
        mfc_cmd_events.len(),
    );

    // Acceptance #2: ≥1 ppu_push_inmbox event (round 1 input via IN_MBOX).
    let in_mbox_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::PpuPushInmbox(_)))
        .count();
    assert!(
        in_mbox_count >= 1,
        "mailbox_multi fixture must have ≥1 ppu_push_inmbox event; got {in_mbox_count}",
    );

    // Acceptance #3: ≥1 ppu_signal event (round 2 input via SNR1).
    // This is the load-bearing distinguishing feature versus the
    // single-channel fixtures (mailbox_v1 has 0 signals; signal_v1
    // has 0 mailbox pushes; this fixture is the only one that
    // captures both PPU input paths in a single trace).
    let signal_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::PpuSignal(_)))
        .count();
    assert!(
        signal_count >= 1,
        "mailbox_multi fixture must have ≥1 ppu_signal event; got {signal_count}",
    );

    // Acceptance #4: ≥1 OUT_MBOX write — the SPU's combined reply.
    let out_mbox_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuWrch(w) if w.channel == 28))
        .count();
    assert!(
        out_mbox_count >= 1,
        "fixture must have ≥1 spu_wrch ch28 (OUT_MBOX) event; got {out_mbox_count}",
    );

    // Acceptance #5: stop instruction with code 0x101
    // (SYS_SPU_THREAD_STOP_GROUP_EXIT). The lv2 kernel reads
    // OUT_MBOX as the group-exit status when this stop fires.
    let stop_events: Vec<_> = events
        .iter()
        .filter_map(|ev| {
            if let CapturedEvent::SpuStop(s) = ev {
                Some(s)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        stop_events.len(),
        1,
        "fixture must have exactly 1 spu_stop event (got {})",
        stop_events.len(),
    );
    assert_eq!(
        stop_events[0].stop_code, 0x101,
        "stop_code must be 0x101 (SYS_SPU_THREAD_STOP_GROUP_EXIT); got 0x{:x}",
        stop_events[0].stop_code,
    );

    // Acceptance #6: the OUT_MBOX value must be 0x453, the canonical
    // expected reply for inputs cmd1=0x100, cmd2=0x200:
    //   partial = cmd1 + 0xA1 = 0x1A1
    //   reply   = (cmd2 + 0xB2) + partial = 0x2B2 + 0x1A1 = 0x453
    // Any deviation here means the SPU did not consume both inputs
    // in the expected order — a corruption the test catches.
    let out_mbox_writes: Vec<_> = events
        .iter()
        .filter_map(|ev| {
            if let CapturedEvent::SpuWrch(w) = ev {
                if w.channel == 28 {
                    Some(w)
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        out_mbox_writes.len(),
        1,
        "fixture must have exactly 1 OUT_MBOX write (got {})",
        out_mbox_writes.len(),
    );
    assert_eq!(
        out_mbox_writes[0].value, 0x453,
        "OUT_MBOX value must be 0x453 (= 0x1A1 + 0x2B2); got 0x{:x}",
        out_mbox_writes[0].value,
    );

    // ===== 4. Per-SPU transformer must produce exactly 1 group. =====
    let groups: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("per-SPU transform must succeed");
    assert_eq!(
        groups.len(),
        1,
        "fixture must have exactly 1 target_spu (got {})",
        groups.len(),
    );

    // ===== 5. Locate the spu_image event. =====
    let images: Vec<&SpuImageEvent> = events
        .iter()
        .filter_map(|ev| {
            if let CapturedEvent::SpuImage(img) = ev {
                Some(img)
            } else {
                None
            }
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
    let program = build_spu_program_from_captured_image(&image_path, image, 100_000_000)
        .expect("builder must succeed (hash + size + entry_pc)");

    // ===== 7. Replay × Interpreter. =====
    let mut programs = BTreeMap::new();
    programs.insert(*groups.keys().next().unwrap(), program.clone());

    let interp_reports = replay_per_spu_traces::<InterpreterExecutor>(&groups, &programs)
        .expect("replay × Interpreter must succeed");
    let interp = interp_reports.values().next().unwrap();

    // ===== 8. Replay × Recompiler. =====
    let jit_reports =
        replay_per_spu_traces_with(&groups, &programs, |_| RecompilerExecutor::new())
            .expect("replay × Recompiler must succeed");
    let jit = jit_reports.values().next().unwrap();

    // ===== 9. Byte-identical agreement on final state. =====
    assert_eq!(
        format!("{:?}", interp.final_event_kind),
        format!("{:?}", jit.final_event_kind),
        "Interpreter vs Recompiler final_event_kind diverged",
    );
    assert_eq!(
        interp.records.len(),
        jit.records.len(),
        "record count diverged",
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

    eprintln!(
        "[R6.4b SUCCESS] single_spu_mailbox_multi_v1 replay-validated:\n  \
         target_spu={}\n  \
         events={} (in_mbox={}, signal={}, out_mbox={})\n  \
         spu_image sha={}\n  \
         OUT_MBOX = 0x{:x} (canonical = 0x453)\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         Final-snapshot diff: identical",
        groups.keys().next().unwrap(),
        events.len(),
        in_mbox_count,
        signal_count,
        out_mbox_count,
        image.image_sha256,
        out_mbox_writes[0].value,
        interp.total_steps,
        jit.total_steps,
    );
}
