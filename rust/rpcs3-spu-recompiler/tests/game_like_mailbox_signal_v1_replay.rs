//! R6.6 — sixth replay-validated fixture integration test.
//!
//! Loads `behavior-freeze/fixtures/spu/traces/game_like_mailbox_signal_v1.jsonl`
//! and the corresponding `.spuimg` side-file, runs the full pipeline
//! (parser → per-SPU transformer → builder → replay × Interpreter +
//! replay × Recompiler), and asserts byte-identical agreement.
//!
//! This is the first "game-like" fixture combining FIVE bridge code
//! paths in a single SPU program:
//!   - `rdch ch29` (IN_MBOX, R5.9e.7 path)
//!   - `stqd` / `lqd` via `volatile uint32_t buf[16]` (R5.11b LS
//!     load/store)
//!   - branch + loop ISA across two mix loops (R5.11 branch_loop)
//!   - `rdch ch3` (SNR1, R5.11/R6.3c signal)
//!   - StallRead between IN_MBOX consumption and SNR1 read (R6.4b
//!     multi-round persistent-handle path — surfaces because PPU
//!     calls `sysUsleep(100ms)` between WriteMb and WriteSignal)
//!
//! All five run in ONE execution. Byte-identical agreement here
//! implies the bridge handles all five simultaneously without
//! cross-path interaction bugs. The canonical OUT_MBOX value
//! (`0x051A03C9` for inputs `seed=0x21`, `sig=0x07`) is a
//! load-bearing assertion: it bit-encodes whether the SPU
//! consumed BOTH PPU inputs in order AND ran both mix loops
//! correctly. Any divergence between Interpreter and Recompiler
//! on any of the five paths would surface as a different value
//! and fail this test.

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
    p.push("game_like_mailbox_signal_v1.jsonl");
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
fn r6_6_game_like_mailbox_signal_v1_replay_validated_byte_identical() {
    // ===== 1. Fixture must exist on disk. =====
    let trace_path = fixture_trace_path();
    let images_dir = fixture_images_dir();

    assert!(trace_path.exists(), "trace not found at {}", trace_path.display());
    assert!(images_dir.exists(), "images dir not found at {}", images_dir.display());

    // ===== 2. Parse the JSONL trace. =====
    let raw = std::fs::read_to_string(&trace_path).expect("read trace");
    let events = parse_jsonl_trace(&raw).expect("parser must accept the captured trace");
    assert!(!events.is_empty(), "trace has no events");

    // ===== 3. Acceptance criteria. =====
    // (1) Zero DMA — no `spu_wrch ch21` (MFC_Cmd) events.
    let mfc_cmd_events: Vec<_> = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuWrch(w) if w.channel == 21))
        .collect();
    assert!(
        mfc_cmd_events.is_empty(),
        "fixture must NOT contain any spu_wrch ch21 events; found {}",
        mfc_cmd_events.len(),
    );

    // (2) ≥1 ppu_push_inmbox event (round 1 PPU input).
    let in_mbox_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::PpuPushInmbox(_)))
        .count();
    assert!(
        in_mbox_count >= 1,
        "must have ≥1 ppu_push_inmbox event; got {in_mbox_count}",
    );

    // (3) ≥1 ppu_signal event (round 2 PPU input via SNR1).
    let signal_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::PpuSignal(_)))
        .count();
    assert!(
        signal_count >= 1,
        "must have ≥1 ppu_signal event; got {signal_count}",
    );

    // (4) ≥1 spu_park event — the StallRead actually fired.
    // This is the load-bearing distinguishing feature versus a
    // race-free trace (where PPU writes both inputs before SPU
    // dispatches).  Without `sysUsleep` in the PPU side, the
    // park wouldn't happen and the persistent-handle re-entry
    // path wouldn't be exercised.
    let park_count = events
        .iter()
        .filter(|ev| matches!(ev, CapturedEvent::SpuPark(_)))
        .count();
    assert!(
        park_count >= 1,
        "must have ≥1 spu_park event (proves sysUsleep forces real stall); got {park_count}",
    );

    // (5) Exactly 1 spu_wrch ch28 (the final OUT_MBOX write).
    let out_mbox_writes: Vec<_> = events
        .iter()
        .filter_map(|ev| {
            if let CapturedEvent::SpuWrch(w) = ev {
                if w.channel == 28 { Some(w) } else { None }
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        out_mbox_writes.len(),
        1,
        "must have exactly 1 OUT_MBOX write; got {}",
        out_mbox_writes.len(),
    );

    // (6) The OUT_MBOX value must equal 0x051A03C9 — canonical for
    // inputs seed=0x21, sig=0x07. See README.md for the reference
    // Python computation. Any deviation here means the SPU did not
    // consume BOTH inputs in the right order AND run both mix loops
    // correctly — a corruption this test catches.
    assert_eq!(
        out_mbox_writes[0].value, 0x051A03C9,
        "OUT_MBOX must be 0x051A03C9 (canonical); got 0x{:x}",
        out_mbox_writes[0].value,
    );

    // (7) Stop with code 0x101 (SYS_SPU_THREAD_STOP_GROUP_EXIT).
    let stop_events: Vec<_> = events
        .iter()
        .filter_map(|ev| {
            if let CapturedEvent::SpuStop(s) = ev { Some(s) } else { None }
        })
        .collect();
    assert_eq!(stop_events.len(), 1, "must have exactly 1 stop event");
    assert_eq!(
        stop_events[0].stop_code, 0x101,
        "stop code must be 0x101; got 0x{:x}",
        stop_events[0].stop_code,
    );

    // ===== 4. Per-SPU transformer. =====
    let groups: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("per-SPU transform must succeed");
    assert_eq!(groups.len(), 1, "exactly 1 target_spu");

    // ===== 5. spu_image. =====
    let images: Vec<&SpuImageEvent> = events
        .iter()
        .filter_map(|ev| {
            if let CapturedEvent::SpuImage(img) = ev { Some(img) } else { None }
        })
        .collect();
    assert_eq!(images.len(), 1);
    let image = images[0];

    // ===== 6. Build SpuProgram. =====
    let image_path = images_dir.join(format!("{}.spuimg", image.image_sha256));
    assert!(
        image_path.exists(),
        ".spuimg side-file missing at {}",
        image_path.display(),
    );
    let program = build_spu_program_from_captured_image(&image_path, image, 100_000_000)
        .expect("builder must succeed");

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

    // ===== 9. Byte-identical agreement. =====
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
        "Both backends must report >0 steps",
    );
    let diff = diff_snapshots(&interp.final_snapshot, &jit.final_snapshot);
    assert!(
        diff.is_identical(),
        "Interpreter vs Recompiler final_snapshot diverged: {diff:?}",
    );

    eprintln!(
        "[R6.6 SUCCESS] game_like_mailbox_signal_v1 replay-validated:\n  \
         target_spu={}\n  \
         events={} (in_mbox={}, signal={}, park={}, out_mbox={})\n  \
         spu_image sha={}\n  \
         OUT_MBOX = 0x{:x} (canonical = 0x051A03C9)\n  \
         interp.total_steps={} jit.total_steps={}\n  \
         Final-snapshot diff: identical",
        groups.keys().next().unwrap(),
        events.len(),
        in_mbox_count,
        signal_count,
        park_count,
        out_mbox_writes.len(),
        image.image_sha256,
        out_mbox_writes[0].value,
        interp.total_steps,
        jit.total_steps,
    );
}
