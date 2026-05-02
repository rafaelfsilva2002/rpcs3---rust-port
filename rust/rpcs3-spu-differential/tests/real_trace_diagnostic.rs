//! R5.9d diagnostic: parse + per-SPU transformer validation against
//! the real `spurs_test.self` trace captured under R5.9c writer
//! (`target_spu` emitted on every SPU-side event).
//!
//! **Status of these tests:** they are `#[ignore]`d. Two reasons:
//!   1. The trace file is **local-only** — not committed to the
//!      repository (4.8 MB, untracked under
//!      `tests/data/spurs_test_v3_real_trimmed.jsonl`). Each developer
//!      who wants to run them must produce the trace from their own
//!      RPCS3 build (see capture instructions below).
//!   2. **Replay is NOT exercised here.** R5.9d only validates parse
//!      + per-SPU group classification. Replaying a multi-SPU trace
//!      requires (a) per-SPU sequential or lockstep replay engine
//!      (R5.9e) AND (b) writer-side SPU image capture (also R5.9e).
//!      Neither exists yet, so committing this trace as a
//!      replay-validated fixture would be premature.
//!
//! ## R5.9d historical context (replaces the pre-R5.9d diagnostic)
//!
//! Pre-R5.9d, this same test file held two `#[ignore]`d tests
//! (`diagnostic_multi_spu_schema_gap_{parser,transformer}`) that
//! asserted on a R5.9 schema gap surfaced by the v2 trace from
//! scaffolding v2 + runtime hooks v1. After R5.9a (parser per-SPU
//! validation), the v2 trace's failure mode shifted from
//! `FinalStateNotTerminal` to `EventAfterFinalState { target_spu: 0,
//! event_index: 40064 }` because the v2 writer did NOT emit
//! `target_spu` and all 6 SPUs collapsed to id 0 via the
//! default-zero compatibility shim. R5.9c re-touched both patches
//! to make the writer emit `target_spu` (= source SPU's `lv2_id`),
//! producing a v3 trace where the parser per-SPU walk and the R5.9b
//! transformer per-SPU API both succeed.
//!
//! ## How to capture the v3 trace
//!
//! Prerequisites: RPCS3 build with R5.9c scaffolding patch
//! (sha256 `2baebca5…91149`) + runtime hooks patch
//! (sha256 `3ee7a861…2bed39`) applied. RPCS3 firmware 4.93 +
//! `R:\bin\test\spurs_test.self` available.
//!
//! Capture command (Windows PowerShell):
//!
//! ```powershell
//! $env:RPCS3_SPU_TRACE_JSONL = "C:\Users\manod\AppData\Local\Temp\spurs_test_v3.jsonl"
//! & R:\bin\rpcs3.exe --headless R:\bin\test\spurs_test.self
//! # Wait ~5–10s for the homebrew to reach SPU code, then close RPCS3.
//! ```
//!
//! Then, from the repo root, normalize and validate:
//!
//! ```bash
//! cp "C:/Users/manod/AppData/Local/Temp/spurs_test_v3.jsonl" \
//!    rust/rpcs3-spu-differential/tests/data/spurs_test_v3_real.jsonl
//! python behavior-freeze/harness/validate_trace_v3.py
//! # Produces tests/data/spurs_test_v3_real_trimmed.jsonl when the
//! # last line is truncated mid-JSON (rpcs3.exe killed mid-write).
//! ```
//!
//! Finally:
//!
//! ```bash
//! cargo test -p rpcs3-spu-differential --test real_trace_diagnostic -- --ignored --nocapture
//! ```
//!
//! See `docs/PROJECT_STATUS.md` § "R5.9c writer-emit landed" and
//! § "R5.9d diagnostic flip" for the full pipeline narrative.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rpcs3_spu_differential::{
    build_spu_program_from_captured_image, captured_events_to_trace,
    captured_events_to_traces_per_spu, parse_jsonl_trace, replay_per_spu_traces, CapturedEvent,
    InterpreterExecutor, MultiSpuReplayError, SpuProgram, TraceEvent, TraceTransformError,
};

/// Path to the trimmed v3 trace, relative to the crate manifest dir.
/// The trimmed copy is what `validate_trace_v3.py` produces from the
/// raw RPCS3 capture (drops the truncated last line that rpcs3.exe
/// leaves when killed mid-write — see § "How to capture" above).
fn trace_v3_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/data/spurs_test_v3_real_trimmed.jsonl");
    p
}

/// Read the v3 trace, surfacing a helpful "how to capture" message if
/// it isn't present locally. Used by all three R5.9d diagnostics.
fn read_trace_v3() -> String {
    let p = trace_v3_path();
    fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "real trace v3 not found at {}: {e}\n\n\
             This test is local-only — see the file-level doc comment for capture\n\
             instructions (run rpcs3.exe with RPCS3_SPU_TRACE_JSONL set, then run\n\
             `python behavior-freeze/harness/validate_trace_v3.py` to produce the\n\
             trimmed copy). The trace is intentionally NOT committed as a fixture\n\
             until R5.9e (multi-SPU replay) is implemented.",
            p.display(),
        )
    })
}

/// Parser must accept the R5.9c trace cleanly. The R5.9a per-SPU
/// `final_state` walk collapses gracefully when each SPU has its own
/// `target_spu`; no `EventAfterFinalState` / `DuplicateFinalState`
/// errors are expected.
#[test]
#[ignore = "real trace v3 is local-only; run with --ignored after R5.9c capture"]
fn diagnostic_real_trace_v3_parser_passes() {
    let s = read_trace_v3();
    let events = parse_jsonl_trace(&s)
        .expect("R5.9c real trace must parse cleanly under the R5.9a parser");
    assert!(
        events.len() > 1000,
        "real trace should have many events; got {}",
        events.len(),
    );
    assert!(
        events.len() < 100_000,
        "but not absurdly many; got {}",
        events.len(),
    );
    println!("parser accepted {} events", events.len());
}

/// R5.9b per-SPU transformer must produce more than one group on the
/// real trace (spurs_test runs 6 SPU threads per the RPCS3 log). Each
/// group is a Vec<TraceEvent> ready for replay (replay is NOT
/// exercised here — R5.9e scope).
#[test]
#[ignore = "real trace v3 is local-only; run with --ignored after R5.9c capture"]
fn diagnostic_real_trace_v3_per_spu_transformer_passes() {
    let s = read_trace_v3();
    let events = parse_jsonl_trace(&s).expect("parse must succeed before transform");
    let groups: BTreeMap<u32, Vec<TraceEvent>> = captured_events_to_traces_per_spu(&events)
        .expect("R5.9c real trace must transform under the per-SPU API");

    assert!(
        groups.len() > 1,
        "spurs_test is multi-SPU; expected >1 group, got {}",
        groups.len(),
    );

    println!("per-SPU groups (target_spu → trace event count):");
    for (id, trace) in &groups {
        println!("  target_spu={id} → {} TraceEvent(s)", trace.len());
    }
    println!("total SPU groups: {}", groups.len());
}

/// Defensive contract: the legacy single-SPU API
/// (`captured_events_to_trace`) MUST refuse a multi-SPU real trace
/// rather than silently flatten it. This is the load-bearing safety
/// property R5.9b added — pre-R5.9b a caller would have produced a
/// nonsense flattened timeline; under R5.9b the call returns
/// `MultipleSpusUnsupportedBySingleSpuApi`.
#[test]
#[ignore = "real trace v3 is local-only; run with --ignored after R5.9c capture"]
fn diagnostic_real_trace_v3_legacy_api_rejects() {
    let s = read_trace_v3();
    let events = parse_jsonl_trace(&s).expect("parse must succeed before transform");
    let err = captured_events_to_trace(&events)
        .expect_err("single-SPU API must refuse multi-SPU real trace");
    match err {
        TraceTransformError::MultipleSpusUnsupportedBySingleSpuApi { spu_count } => {
            assert!(
                spu_count > 1,
                "expected multiple SPUs, got spu_count={spu_count}",
            );
            println!(
                "legacy single-SPU API correctly rejected with spu_count={spu_count}",
            );
        }
        other => panic!(
            "expected MultipleSpusUnsupportedBySingleSpuApi, got {other:?}",
        ),
    }
}

// ---------------------------------------------------------------------
// R5.9e.3-fix diagnostics — real trace v4 from R5.9e.3 writer
// (target_spu + spu_image emission). The v4 trace has 6 spu_image
// events (one per SPU lv2_id), all referencing the same SHA-256 image
// (content-addressed dedup); the side-file resolves to a single
// `.spuimg` of 262,144 bytes whose SHA matches.
//
// These tests stay #[ignore]d for the same reasons as v3: the trace
// is local-only (untracked) and replay isn't exercised here. They
// validate that R5.9e.2 parser + R5.9e.b transformer accept the new
// `spu_image` events without regression and that the per-SPU group
// count remains 6.
// ---------------------------------------------------------------------

fn trace_v4_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests/data/spurs_test_v4_real_trimmed.jsonl");
    p
}

fn read_trace_v4() -> String {
    let p = trace_v4_path();
    fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "real trace v4 not found at {}: {e}\n\n\
             This test is local-only. Capture under R5.9e.3-fix rpcs3.exe:\n\
               $env:RPCS3_SPU_TRACE_JSONL=\"$env:TEMP\\spurs_test_v4.jsonl\"\n\
               R:\\bin\\rpcs3.exe --headless R:\\bin\\test\\spurs_test.self\n\
             Then copy the trace + .images/ side-files into\n\
               tests/data/spurs_test_v4_real.jsonl\n\
               tests/data/spurs_test_v4_real.images/<sha>.spuimg\n\
             and trim the truncated tail line into\n\
               tests/data/spurs_test_v4_real_trimmed.jsonl",
            p.display(),
        )
    })
}

#[test]
#[ignore = "real trace v4 is local-only; run with --ignored after R5.9e.3-fix capture"]
fn diagnostic_real_trace_v4_parser_passes_with_spu_image() {
    let s = read_trace_v4();
    let events = parse_jsonl_trace(&s)
        .expect("R5.9e.3-fix trace must parse cleanly under R5.9e.2 parser");

    // The v4 trace contains spu_image events alongside the executed
    // events. Count both and report.
    let total = events.len();
    let image_count = events
        .iter()
        .filter(|e| matches!(e, rpcs3_spu_differential::CapturedEvent::SpuImage(_)))
        .count();
    assert!(total > 1000, "expected many events; got {total}");
    assert_eq!(
        image_count, 6,
        "v4 trace has 6 SPU threads; expected 6 spu_image events, got {image_count}"
    );
    println!(
        "v4 parser ok: {total} events total, {image_count} are spu_image (one per SPU lv2_id)"
    );
}

#[test]
#[ignore = "real trace v4 is local-only; run with --ignored after R5.9e.3-fix capture"]
fn diagnostic_real_trace_v4_per_spu_transformer_passes() {
    let s = read_trace_v4();
    let events = parse_jsonl_trace(&s).expect("parse must succeed before transform");
    let groups: BTreeMap<u32, Vec<TraceEvent>> = captured_events_to_traces_per_spu(&events)
        .expect("R5.9e.3-fix trace must transform under the per-SPU API");

    assert!(
        groups.len() > 1,
        "spurs_test is multi-SPU; expected >1 group, got {}",
        groups.len(),
    );
    assert_eq!(groups.len(), 6, "expected 6 SPU groups (one per lv2_id)");

    println!("v4 per-SPU groups (target_spu → trace event count):");
    for (id, trace) in &groups {
        println!("  target_spu={id} → {} TraceEvent(s)", trace.len());
    }
    println!("v4 total SPU groups: {}", groups.len());

    // The transformer ignores spu_image events (they're metadata-only),
    // so the per-SPU TraceEvent counts should match v3's structure
    // closely (modulo the truncation artifact moving by one event).
}

#[test]
#[ignore = "real trace v4 is local-only; run with --ignored after R5.9e.3-fix capture"]
fn diagnostic_real_trace_v4_legacy_api_rejects() {
    let s = read_trace_v4();
    let events = parse_jsonl_trace(&s).expect("parse must succeed before transform");
    let err = captured_events_to_trace(&events)
        .expect_err("single-SPU API must refuse multi-SPU real trace");
    match err {
        TraceTransformError::MultipleSpusUnsupportedBySingleSpuApi { spu_count } => {
            assert_eq!(spu_count, 6, "expected 6 SPUs in v4, got {spu_count}");
            println!(
                "v4 legacy single-SPU API correctly rejected with spu_count={spu_count}"
            );
        }
        other => panic!(
            "expected MultipleSpusUnsupportedBySingleSpuApi, got {other:?}",
        ),
    }
}

/// R5.9e.4 diagnostic: feed each SPU's `spu_image` event from the v4
/// trace through `build_spu_program_from_captured_image`, resolving
/// the side-file via the sibling `.images/` directory. Asserts the
/// builder accepts the real captured image and produces a `SpuProgram`
/// with the expected entry_pc, segment count, and segment data.
///
/// Side-file dedup: the v4 trace has 6 `spu_image` events all
/// referencing the same SHA-256 (the 6 SPURS workers load the same
/// `.spucore.elf`). The builder is called once per event; all 6 calls
/// resolve to the same on-disk `.spuimg` file (content-addressed).
/// Each call returns an independent `SpuProgram` with the same code
/// segment but with the per-SPU `entry_pc` from its event.
///
/// Replay is NOT exercised here — that's R5.9e.5+ scope. This test
/// only validates that the builder produces a valid `SpuProgram` from
/// real captured data.
#[test]
#[ignore = "real trace v4 is local-only; run with --ignored after R5.9e.3-fix capture"]
fn diagnostic_real_trace_v4_builds_spu_program_from_image() {
    let s = read_trace_v4();
    let events = parse_jsonl_trace(&s).expect("parse must succeed before builder");

    // Resolve `.images/` directory beside the trimmed trace.
    let mut images_dir = trace_v4_path();
    // trace_v4_path() returns ".../spurs_test_v4_real_trimmed.jsonl";
    // the side-files live in ".../spurs_test_v4_real.images/" because
    // they were produced from the raw (un-trimmed) JSONL by the
    // RPCS3 writer.
    images_dir.set_file_name("spurs_test_v4_real.images");
    assert!(
        images_dir.is_dir(),
        "v4 .images/ dir not found at {} — see test doc-comment for capture instructions",
        images_dir.display(),
    );

    let mut built = 0usize;
    let mut shas_seen = std::collections::HashSet::new();

    for ev in &events {
        let CapturedEvent::SpuImage(img) = ev else { continue };

        // Resolve the side-file by SHA-named lookup (content-addressed).
        let image_path = images_dir.join(format!("{}.spuimg", img.image_sha256));
        assert!(
            image_path.is_file(),
            "side-file missing for target_spu={} sha={} (expected at {})",
            img.target_spu,
            img.image_sha256,
            image_path.display(),
        );

        // Build with a generous max_steps; the value is propagated but
        // never exercised by the builder itself.
        let prog = build_spu_program_from_captured_image(&image_path, img, 10_000_000)
            .expect("real captured image must build under R5.9e.4");

        assert_eq!(prog.entry_pc, img.entry_pc);
        assert_eq!(prog.segments.len(), 1, "exactly one segment per image");
        assert_eq!(prog.segments[0].lsa, img.load_addr);
        assert_eq!(prog.segments[0].data.len(), img.size as usize);

        // Cross-check that the produced program passes the existing
        // SpuProgram::validate (LS-bounds + entry_pc alignment).
        prog.validate().expect("built SpuProgram must validate");

        built += 1;
        shas_seen.insert(img.image_sha256.clone());
    }

    assert_eq!(built, 6, "spurs_test v4 has 6 spu_image events, got {built}");
    assert_eq!(
        shas_seen.len(),
        1,
        "spurs_test workers share the same image; expected 1 unique SHA, got {}",
        shas_seen.len(),
    );
    println!(
        "v4 builder ok: {built} SpuProgram(s) built from {} unique side-file(s)",
        shas_seen.len(),
    );
}

/// R5.9e.5 diagnostic: end-to-end replay attempt on the real v4
/// trace. Wires R5.9b's per-SPU transformer + R5.9e.4's per-SPU
/// program builder + R5.9e.5's per-SPU replay orchestrator + the
/// existing single-SPU `replay_trace<InterpreterExecutor>`.
///
/// The expected outcome is **failure**, not success — the v4 trace
/// captures DMA-heavy SPURS workers, and R5.9e's scope explicitly
/// rejects DMA replay (see SPU_TRACE_R5_9E_REPLAY_PLAN.md § D.1 +
/// § D.4). The point of this diagnostic is to (a) confirm the
/// pipeline wiring works end-to-end, (b) surface the EXACT
/// divergence so future work can target a fix, and (c) lock in the
/// expectation that v4 stays diagnostic-only (NOT a fixture). If
/// this test ever starts passing, that's a real-trace milestone
/// worth promoting to a committed fixture — and that promotion is
/// R5.9e.7's job, not this test's.
///
/// Test policy: print the actual error and pass. We do NOT assert a
/// specific failure mode because the divergence point may shift as
/// the SPU interpreter / replay engine evolves; the documented
/// invariant is "v4 doesn't replay-validate yet", not "v4 fails at
/// exactly event N with kind X".
#[test]
#[ignore = "real trace v4 is local-only; run with --ignored after R5.9e.3-fix capture"]
fn diagnostic_real_trace_v4_per_spu_replay_attempt() {
    let s = read_trace_v4();
    let events = parse_jsonl_trace(&s).expect("parse must succeed");
    let per_spu: BTreeMap<u32, Vec<TraceEvent>> =
        captured_events_to_traces_per_spu(&events).expect("transform must succeed");

    // Build per-SPU programs by resolving each spu_image event's
    // side-file. Same logic as the R5.9e.4 diagnostic, factored out.
    let mut images_dir = trace_v4_path();
    images_dir.set_file_name("spurs_test_v4_real.images");
    assert!(
        images_dir.is_dir(),
        "v4 .images/ dir not found at {} — see test doc-comment for capture instructions",
        images_dir.display(),
    );

    let mut programs: BTreeMap<u32, SpuProgram> = BTreeMap::new();
    for ev in &events {
        let CapturedEvent::SpuImage(img) = ev else { continue };
        let image_path = images_dir.join(format!("{}.spuimg", img.image_sha256));
        let prog = build_spu_program_from_captured_image(&image_path, img, 100_000_000)
            .expect("v4 image must build (covered by R5.9e.4 diagnostic)");
        programs.insert(img.target_spu, prog);
    }

    println!(
        "v4 replay attempt: {} SPUs in trace, {} SpuPrograms built",
        per_spu.len(),
        programs.len(),
    );

    // Per-SPU sequential replay. The expected outcome is failure;
    // print the real error and pass the test.
    let result = replay_per_spu_traces::<InterpreterExecutor>(&per_spu, &programs);
    match result {
        Ok(reports) => {
            // If this branch is reached, the v4 trace replay-validated
            // end-to-end against the Interpreter. That's a milestone:
            // promote to R5.9e.6 (Recompiler differential) and then
            // R5.9e.7 (commit fixture).
            println!(
                "*** v4 replay UNEXPECTEDLY SUCCEEDED *** \
                 {} per-SPU reports produced. This is a milestone — \
                 verify the SPU stop_codes + final snapshots match the \
                 captured final_state events, then promote to R5.9e.6 / R5.9e.7.",
                reports.len(),
            );
            for (tgt, report) in &reports {
                println!(
                    "  target_spu={tgt}: final={:?}, total_steps={}, records={}",
                    report.final_event_kind,
                    report.total_steps,
                    report.records.len(),
                );
            }
        }
        Err(MultiSpuReplayError::ReplayFailed { target_spu, source }) => {
            println!(
                "v4 replay diagnostic divergence (expected per § D.1 / § D.4): \
                 target_spu={target_spu}, event_index={}, kind={:?}",
                source.event_index, source.kind,
            );
            // Do NOT assert a specific kind — the divergence may shift
            // as the SPU stack evolves. The documented invariant is
            // "v4 doesn't replay-validate", which this branch satisfies.
        }
        Err(MultiSpuReplayError::MissingProgram { target_spu }) => {
            panic!(
                "missing program for target_spu={target_spu} — should have been built from spu_image events"
            );
        }
        Err(MultiSpuReplayError::ExtraProgram { target_spu }) => {
            panic!(
                "extra program for target_spu={target_spu} — programs map has SPUs the trace doesn't know about"
            );
        }
    }
}
