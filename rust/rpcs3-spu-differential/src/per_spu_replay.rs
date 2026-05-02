//! R5.9e.5 — Per-SPU sequential replay orchestrator.
//!
//! Bridges three pieces that already exist independently:
//! 1. [`captured_events_to_traces_per_spu`] (R5.9b) — produces a
//!    `BTreeMap<u32, Vec<TraceEvent>>` keyed on `target_spu`.
//! 2. [`build_spu_program_from_captured_image`] (R5.9e.4) — produces
//!    one [`SpuProgram`] per SPU from its `.spuimg` side-file.
//! 3. [`replay_trace`] (pre-existing) — runs a single `Vec<TraceEvent>`
//!    against a single [`SpuExecutor`] backend.
//!
//! The orchestrator is **sequential**: each SPU runs to completion (or
//! to a replay error) on its own fresh executor instance before the
//! next SPU starts. No state is shared between SPU runs; cross-SPU
//! mailbox correlation is implicit in the per-SPU `Vec<TraceEvent>`
//! (the R5.9b transformer already records PPU push/pop events in each
//! SPU's filtered subsequence).
//!
//! **Lockstep is NOT implemented** in R5.9e.5. A future R5.9f could
//! add a `MultiSpuLockstepDriver` mirroring `SpuPpuLockstepDriver`,
//! but per-SPU sequential is the v1 because (a) most homebrews route
//! cross-SPU mailbox traffic through the PPU (which IS recorded), and
//! (b) sequential replay is simpler and lets failures be diagnosed
//! one SPU at a time.
//!
//! Design references: `docs/SPU_TRACE_R5_9E_REPLAY_PLAN.md` § C.1
//! (per-SPU sequential first), § C.2 (`replay_per_spu_traces` API
//! shape).
//!
//! [`captured_events_to_traces_per_spu`]: crate::captured_events_to_traces_per_spu
//! [`build_spu_program_from_captured_image`]: crate::build_spu_program_from_captured_image
//! [`replay_trace`]: crate::replay_trace
//! [`SpuExecutor`]: crate::SpuExecutor
//! [`SpuProgram`]: crate::SpuProgram

use std::collections::BTreeMap;

use crate::{
    replay_trace, SpuExecutor, SpuProgram, TraceEvent, TraceReplayError, TraceReplayReport,
};

/// Errors from [`replay_per_spu_traces_with`] /
/// [`replay_per_spu_traces`]. Every variant carries the `target_spu`
/// at which the failure surfaced — callers can use this to print
/// per-SPU diagnostics or to skip a single bad SPU and continue with
/// others (R5.9e.5 itself does NOT continue past a failure; the
/// `target_spu` field is for the caller's reporting).
#[derive(Debug, Clone)]
pub enum MultiSpuReplayError {
    /// A trace exists for `target_spu` but no `SpuProgram` was
    /// supplied. Most common cause: caller forgot to invoke
    /// `build_spu_program_from_captured_image` for one of the
    /// `spu_image` events the parser surfaced.
    MissingProgram { target_spu: u32 },
    /// A program was supplied for `target_spu` but the trace map
    /// contains no events for it. Most common cause: stale program
    /// from a previous capture attempt left in the map.
    ExtraProgram { target_spu: u32 },
    /// `replay_trace` itself surfaced an error for this SPU. The
    /// underlying error carries `event_index` + `kind`; this wrapper
    /// adds the `target_spu` so the caller can distinguish "SPU 5
    /// diverged" from "SPU 12 diverged" without having to thread
    /// SPU identity through every diagnostic.
    ReplayFailed {
        target_spu: u32,
        source: TraceReplayError,
    },
}

impl std::fmt::Display for MultiSpuReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingProgram { target_spu } => write!(
                f,
                "multi-SPU replay error: trace contains events for target_spu={target_spu} but no SpuProgram was supplied"
            ),
            Self::ExtraProgram { target_spu } => write!(
                f,
                "multi-SPU replay error: SpuProgram supplied for target_spu={target_spu} but trace contains no events for it"
            ),
            Self::ReplayFailed { target_spu, source } => write!(
                f,
                "multi-SPU replay error at target_spu={target_spu}: {source}"
            ),
        }
    }
}

impl std::error::Error for MultiSpuReplayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ReplayFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Run [`replay_trace`] once per `target_spu` in `per_spu`, using a
/// caller-provided `make_executor` factory to mint a fresh executor
/// for each SPU. Returns a `BTreeMap` keyed on `target_spu` with one
/// [`TraceReplayReport`] per SPU on success.
///
/// **Pre-flight** — both directions of the trace↔program bijection
/// are checked BEFORE any replay runs:
/// - Every `target_spu` in `per_spu` must have a matching entry in
///   `programs` (else [`MultiSpuReplayError::MissingProgram`]).
/// - Every `target_spu` in `programs` must have a matching entry in
///   `per_spu` (else [`MultiSpuReplayError::ExtraProgram`]).
///
/// Strict bijection is the simplest contract that's easy to reason
/// about. A future iteration could relax this (e.g., allow extra
/// programs that simply aren't replayed), but the strict form catches
/// the most likely caller bug first.
///
/// **Sequential** — SPUs run in `BTreeMap` iteration order (= sorted
/// by `target_spu`). The first SPU's executor is built and torn down
/// before the second SPU's executor is built. No state is shared.
///
/// **First failure halts** — if `replay_trace` returns `Err` for any
/// SPU, this function returns immediately with
/// [`MultiSpuReplayError::ReplayFailed`] carrying the offending
/// `target_spu`. Callers that want "best-effort, report all failures"
/// semantics must call `replay_per_spu_traces_with` per-SPU
/// themselves.
///
/// Cloning of `SpuProgram` happens once per SPU to satisfy
/// `replay_trace`'s by-value `program` parameter; the cost is `O(LS)`
/// per SPU which is small for the 256 KB local store.
pub fn replay_per_spu_traces_with<E, F>(
    per_spu: &BTreeMap<u32, Vec<TraceEvent>>,
    programs: &BTreeMap<u32, SpuProgram>,
    mut make_executor: F,
) -> Result<BTreeMap<u32, TraceReplayReport>, MultiSpuReplayError>
where
    E: SpuExecutor,
    F: FnMut(u32) -> E,
{
    // Pre-flight bijection check, deterministic order (BTreeMap keys
    // are sorted) so error messages are stable across runs.
    for &tgt in per_spu.keys() {
        if !programs.contains_key(&tgt) {
            return Err(MultiSpuReplayError::MissingProgram { target_spu: tgt });
        }
    }
    for &tgt in programs.keys() {
        if !per_spu.contains_key(&tgt) {
            return Err(MultiSpuReplayError::ExtraProgram { target_spu: tgt });
        }
    }

    let mut reports: BTreeMap<u32, TraceReplayReport> = BTreeMap::new();
    for (target_spu, events) in per_spu {
        // Pre-flight guaranteed presence; using `expect` here so a
        // future map-mutation regression surfaces immediately.
        let program = programs
            .get(target_spu)
            .expect("pre-flight bijection check guarantees presence")
            .clone();
        let mut backend = make_executor(*target_spu);
        let report = replay_trace(&mut backend, program, events).map_err(|source| {
            MultiSpuReplayError::ReplayFailed {
                target_spu: *target_spu,
                source,
            }
        })?;
        reports.insert(*target_spu, report);
    }
    Ok(reports)
}

/// Convenience wrapper over [`replay_per_spu_traces_with`] for
/// executors that implement [`Default`]. The factory is
/// `|_| E::default()`.
///
/// Use the explicit `replay_per_spu_traces_with` form if your
/// executor needs per-SPU configuration (e.g., a recompiler that
/// caches compiled programs per `target_spu`).
pub fn replay_per_spu_traces<E>(
    per_spu: &BTreeMap<u32, Vec<TraceEvent>>,
    programs: &BTreeMap<u32, SpuProgram>,
) -> Result<BTreeMap<u32, TraceReplayReport>, MultiSpuReplayError>
where
    E: SpuExecutor + Default,
{
    replay_per_spu_traces_with(per_spu, programs, |_target_spu| E::default())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        replay_per_spu_traces, replay_per_spu_traces_with, MultiSpuReplayError,
    };
    use crate::{
        mailbox_command_protocol_program, mailbox_command_protocol_trace, InterpreterExecutor,
        SpuEventKind, SpuProgram, TraceEvent, TraceReplayErrorKind,
    };

    /// Build a tiny "stop_code 0xD5" SPU program for failure-mode
    /// tests. The program is exactly one instruction (`stop 0xD5`) at
    /// `entry_pc=0x100`. When replayed, the SPU finishes immediately
    /// with `stop_code=0xD5`.
    fn stop_only_program(stop_code: u32) -> SpuProgram {
        let stop = stop_code & 0x3FFF;
        let bytes = stop.to_be_bytes().to_vec();
        SpuProgram::new(0x100, 10).with_segment(0x100, bytes)
    }

    /// Single SPU at `target_spu=42` running the canonical
    /// mailbox-command-protocol fixture. The fixture is the load-
    /// bearing R5.6 reference; if it replays successfully under the
    /// per-SPU API, the orchestrator is correctly delegating to
    /// `replay_trace`.
    #[test]
    fn per_spu_replay_single_spu_synthetic_interpreter() {
        let mut per_spu = BTreeMap::new();
        per_spu.insert(42u32, mailbox_command_protocol_trace());

        let mut programs = BTreeMap::new();
        programs.insert(42u32, mailbox_command_protocol_program());

        let reports = replay_per_spu_traces::<InterpreterExecutor>(&per_spu, &programs)
            .expect("synthetic single-SPU replay must succeed");

        assert_eq!(reports.len(), 1);
        let report = &reports[&42u32];
        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xD5 }
        ));
        assert_eq!(report.records.len(), 16);
    }

    /// Two SPUs at different `target_spu`s, both running the same
    /// canonical fixture. Verifies (a) the orchestrator runs both,
    /// (b) iteration order is deterministic (BTreeMap sorted), and
    /// (c) per-SPU executors are independent (no state leak between
    /// runs).
    #[test]
    fn per_spu_replay_two_spus_synthetic_interpreter() {
        let mut per_spu = BTreeMap::new();
        per_spu.insert(7u32, mailbox_command_protocol_trace());
        per_spu.insert(42u32, mailbox_command_protocol_trace());

        let mut programs = BTreeMap::new();
        programs.insert(7u32, mailbox_command_protocol_program());
        programs.insert(42u32, mailbox_command_protocol_program());

        let reports = replay_per_spu_traces::<InterpreterExecutor>(&per_spu, &programs)
            .expect("synthetic two-SPU replay must succeed");

        assert_eq!(reports.len(), 2);
        for (tgt, report) in &reports {
            assert!(
                matches!(
                    report.final_event_kind,
                    SpuEventKind::Finished { stop_code: 0xD5 }
                ),
                "SPU {tgt} should finish with stop_code 0xD5"
            );
            assert_eq!(report.records.len(), 16);
        }

        // Iteration order is sorted (BTreeMap contract); verify the
        // returned map preserves the key set verbatim.
        let keys: Vec<u32> = reports.keys().copied().collect();
        assert_eq!(keys, vec![7u32, 42u32]);
    }

    /// A trace for `target_spu=1` but the programs map is empty —
    /// must return MissingProgram pre-flight, BEFORE any replay runs.
    #[test]
    fn per_spu_replay_rejects_missing_program() {
        let mut per_spu = BTreeMap::new();
        per_spu.insert(1u32, mailbox_command_protocol_trace());
        let programs: BTreeMap<u32, SpuProgram> = BTreeMap::new();

        let err = replay_per_spu_traces::<InterpreterExecutor>(&per_spu, &programs)
            .expect_err("missing program must reject pre-flight");
        match err {
            MultiSpuReplayError::MissingProgram { target_spu } => assert_eq!(target_spu, 1),
            other => panic!("expected MissingProgram, got {other:?}"),
        }
    }

    /// A program for `target_spu=99` but the trace map has no events
    /// for it — must return ExtraProgram pre-flight.
    #[test]
    fn per_spu_replay_rejects_extra_program() {
        let mut per_spu = BTreeMap::new();
        per_spu.insert(1u32, mailbox_command_protocol_trace());

        let mut programs = BTreeMap::new();
        programs.insert(1u32, mailbox_command_protocol_program());
        // Stale program for an SPU the trace knows nothing about.
        programs.insert(99u32, mailbox_command_protocol_program());

        let err = replay_per_spu_traces::<InterpreterExecutor>(&per_spu, &programs)
            .expect_err("extra program must reject pre-flight");
        match err {
            MultiSpuReplayError::ExtraProgram { target_spu } => assert_eq!(target_spu, 99),
            other => panic!("expected ExtraProgram, got {other:?}"),
        }
    }

    /// Replay error MUST carry `target_spu`. We construct a trace that
    /// expects `Finished{stop_code: 0xAA}` but the program actually
    /// finishes with `0xD5` — `replay_trace` surfaces an
    /// `UnexpectedSpuState`. The orchestrator wraps it in
    /// `ReplayFailed { target_spu: 42, source: ... }`.
    #[test]
    fn per_spu_replay_reports_target_spu_on_failure() {
        let bad_trace = vec![TraceEvent::ExpectSpuFinished { stop_code: 0xAA }];

        let mut per_spu = BTreeMap::new();
        per_spu.insert(42u32, bad_trace);

        let mut programs = BTreeMap::new();
        programs.insert(42u32, stop_only_program(0xD5));

        let err = replay_per_spu_traces::<InterpreterExecutor>(&per_spu, &programs)
            .expect_err("expected_stop_code mismatch must surface as ReplayFailed");
        match err {
            MultiSpuReplayError::ReplayFailed { target_spu, source } => {
                assert_eq!(target_spu, 42, "error must carry the failing target_spu");
                // The underlying TraceReplayError points at the first
                // event (index 0) where the divergence surfaced.
                assert_eq!(source.event_index, 0);
                // The kind is UnexpectedSpuState because the SPU has
                // already Finished{0xD5} when the trace expects
                // Finished{0xAA}.
                assert!(
                    matches!(source.kind, TraceReplayErrorKind::UnexpectedSpuState { .. }),
                    "expected UnexpectedSpuState, got {:?}",
                    source.kind
                );
            }
            other => panic!("expected ReplayFailed, got {other:?}"),
        }
    }

    /// `replay_per_spu_traces_with` lets the caller supply a custom
    /// factory closure. Verify that the closure is invoked once per
    /// SPU with the correct `target_spu` argument, and that the
    /// returned executors drive the replay correctly.
    #[test]
    fn per_spu_replay_with_factory_invokes_closure_per_spu() {
        let mut per_spu = BTreeMap::new();
        per_spu.insert(7u32, mailbox_command_protocol_trace());
        per_spu.insert(42u32, mailbox_command_protocol_trace());

        let mut programs = BTreeMap::new();
        programs.insert(7u32, mailbox_command_protocol_program());
        programs.insert(42u32, mailbox_command_protocol_program());

        let mut seen: Vec<u32> = Vec::new();
        let reports = replay_per_spu_traces_with(&per_spu, &programs, |tgt| {
            seen.push(tgt);
            InterpreterExecutor::default()
        })
        .expect("factory variant must succeed on synthetic two-SPU input");

        assert_eq!(reports.len(), 2);
        assert_eq!(seen, vec![7u32, 42u32], "factory must be called once per SPU in sorted order");
    }
}
