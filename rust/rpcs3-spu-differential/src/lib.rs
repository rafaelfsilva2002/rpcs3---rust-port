//! `rpcs3-spu-differential` — backend-agnostic SPU execution +
//! differential comparison harness.
//!
//! ## Why this crate exists
//!
//! Before we land an SPU recompiler, we need a stable interface for
//! "execute an SPU program and report the final state" that does NOT
//! lock us into the interpreter as the sole backend. The future
//! `SPUCommonRecompiler.cpp` Rust port will implement [`SpuExecutor`]
//! against the *same* trait, and the differential harness will then
//! diff its output against the interpreter byte-for-byte.
//!
//! ## Public surface
//!
//! - [`SpuExecutor`] — trait every backend implements.
//! - [`SpuProgram`] — input: a list of `(lsa, bytes)` segments + entry PC.
//! - [`SpuExecutionResult`] / [`ExecutionStopReason`] — output.
//! - [`SpuStateSnapshot`] — final {GPR, PC, LS, channel-counts}.
//! - [`InterpreterExecutor`] — interpreter-backed implementation.
//! - [`diff_snapshots`] — compute a structured difference.
//!
//! The shapes are chosen so that adding a recompiler backend later
//! requires zero changes to the harness or to fixture-driven tests.

use rpcs3_spu_interpreter::{run_n, StepOutcome};
use rpcs3_spu_thread::{
    SpuChannels, SpuParkReason, SpuParkState, SpuThread, SPU_GPR_COUNT, SPU_LS_SIZE,
};

// R5.8 A.1 + A.2: JSONL trace capture parser + transformer.
// See `docs/SPU_TRACE_CAPTURE.md` for the wire format spec.
pub mod trace_fmt;
pub use trace_fmt::{
    captured_events_to_trace, captured_events_to_traces_per_spu, parse_jsonl_trace,
    CapturedChannels, CapturedEvent, CapturedGprEntry, CapturedParkReason, CapturedSide,
    FinalStateEvent, MfcDmaCompleteEvent, PpuPopOutmboxEvent, PpuPushInmboxEvent, PpuSignalEvent,
    SpuImageEvent, SpuMfcCmdEvent, SpuParkEvent, SpuRchcntEvent, SpuRdchEvent, SpuStopEvent,
    SpuWakeEvent, SpuWrchEvent, TraceParseError, TraceTransformError, R5_6_REFERENCE_JSONL,
};

pub mod spu_image_loader;
pub use spu_image_loader::{build_spu_program_from_captured_image, SpuProgramBuildError};

// R6.7 A.3 — `.dmachunk` side-file loader. Resolves the per-trace +
// canonical paths, reads the bytes, validates size + SHA-256. Does
// NOT feed bytes into LS or alter the transformer's MFC-rejection
// policy — that's A.4 scope.
pub mod dma_chunk;
pub use dma_chunk::{
    canonical_dma_chunk_path, per_trace_dma_chunk_path, resolve_dma_chunk_side_file,
    DmaChunkLoadError, DMA_CHUNK_EXTENSION, DMA_CHUNK_SIZE_MAX,
};

// R6.7 A.4 — MFC replay state machine for GET-only DMA. Standalone
// infrastructure; consumes captured MFC events + applies `.dmachunk`
// bytes to a caller-supplied LS buffer + computes the rdch ch24
// (RdTagStat) oracle. ACEITO PARCIAL — wiring into the actual SPU
// executor (Interpreter / Recompiler) requires Phase C support for
// MFC channels in `rpcs3-spu-thread`'s `ch::` module; the transformer
// continues to hard-reject MFC traces until that lands.
pub mod mfc_replay;
pub use mfc_replay::{
    MfcInFlight, MfcReplayError, MfcReplayState, MfcTagUpdate, PendingMfcCmd,
};

pub mod per_spu_replay;
pub use per_spu_replay::{
    replay_per_spu_traces, replay_per_spu_traces_with, MultiSpuReplayError,
};

// =====================================================================
// Inputs
// =====================================================================

/// One bit of code or data to be deployed into Local Store before
/// execution starts. `lsa` must be ≤ 256 KB - data.len().
#[derive(Debug, Clone)]
pub struct SpuSegment {
    pub lsa: u32,
    pub data: Vec<u8>,
}

/// A complete SPU program ready to execute on any backend.
#[derive(Debug, Clone)]
pub struct SpuProgram {
    pub entry_pc: u32,
    pub segments: Vec<SpuSegment>,
    pub max_steps: u64,
    /// Optional per-register initial values applied to the SPU
    /// before the first instruction executes. Empty by default
    /// (= all GPRs start zero, matching synthetic / hand-written
    /// fixtures). Populated for replay programs built from real
    /// captured traces, where the PS3 lv2 kernel sets
    /// register state (preferred slot of r1 = top-of-LS - 16,
    /// r3..r6 = sysSpuThreadArgument fields).
    pub initial_gpr_overrides: Vec<(u8, u128)>,
    /// R6.7 C.4 — pre-populated `rdch ch24 (RdTagStat)` queue. Each
    /// `rdch ch24` the SPU executes during replay pops one value
    /// from the front of this queue. Populated by
    /// [`crate::mfc_replay::apply_mfc_dma_pre_replay`] from a
    /// captured trace; left empty for non-DMA programs (the queue
    /// is dormant if the SPU never reads ch24).
    pub initial_mfc_tag_stat_queue: Vec<u32>,
}

impl SpuProgram {
    #[must_use]
    pub fn new(entry_pc: u32, max_steps: u64) -> Self {
        Self {
            entry_pc,
            segments: Vec::new(),
            max_steps,
            initial_gpr_overrides: Vec::new(),
            initial_mfc_tag_stat_queue: Vec::new(),
        }
    }

    /// Append a code/data segment. Returns `self` for builder chaining.
    #[must_use]
    pub fn with_segment(mut self, lsa: u32, data: Vec<u8>) -> Self {
        self.segments.push(SpuSegment { lsa, data });
        self
    }

    /// Append a GPR override applied at thread start (before
    /// `entry_pc` runs). `reg` must be 0..=127 (caller's responsibility
    /// — out-of-range entries are ignored at execute time).
    #[must_use]
    pub fn with_initial_gpr(mut self, reg: u8, value: u128) -> Self {
        self.initial_gpr_overrides.push((reg, value));
        self
    }

    /// R6.7 C.4 — set the pre-populated `rdch ch24` queue for this
    /// program. Each rdch ch24 the SPU performs during replay pops
    /// one value. Used by [`crate::mfc_replay::apply_mfc_dma_pre_replay`]
    /// to plumb the captured tag-stat sequence into the executor.
    #[must_use]
    pub fn with_mfc_tag_stat_queue(mut self, queue: Vec<u32>) -> Self {
        self.initial_mfc_tag_stat_queue = queue;
        self
    }

    /// Validate every segment fits inside LS without overlap and
    /// without crossing the 256 KB boundary. Backends should call
    /// this before execution.
    pub fn validate(&self) -> Result<(), ProgramError> {
        for seg in &self.segments {
            if seg.data.is_empty() {
                continue;
            }
            let start = seg.lsa as usize;
            let end = start
                .checked_add(seg.data.len())
                .ok_or(ProgramError::SegmentOverflow { lsa: seg.lsa })?;
            if end > SPU_LS_SIZE {
                return Err(ProgramError::SegmentOutOfRange {
                    lsa: seg.lsa,
                    size: seg.data.len() as u64,
                });
            }
        }
        if (self.entry_pc as usize) >= SPU_LS_SIZE || self.entry_pc & 0x3 != 0 {
            return Err(ProgramError::BadEntryPc(self.entry_pc));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProgramError {
    SegmentOverflow { lsa: u32 },
    SegmentOutOfRange { lsa: u32, size: u64 },
    BadEntryPc(u32),
}

impl std::fmt::Display for ProgramError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SegmentOverflow { lsa } => {
                write!(f, "segment at lsa=0x{lsa:x} overflows usize")
            }
            Self::SegmentOutOfRange { lsa, size } => {
                write!(f, "segment lsa=0x{lsa:x} size=0x{size:x} exceeds 256 KB LS")
            }
            Self::BadEntryPc(pc) => {
                write!(f, "entry pc=0x{pc:x} not 4-byte aligned or out of range")
            }
        }
    }
}

impl std::error::Error for ProgramError {}

// =====================================================================
// Outputs
// =====================================================================

/// Why execution stopped. Mirrors [`rpcs3_spu_interpreter::StepOutcome`]
/// but lifted into a stable shape that the recompiler can also produce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionStopReason {
    /// `stop` / `stopd` — guest halted with a 14-bit code.
    Stop(u32),
    /// Channel read/write would block. PC has NOT advanced past the
    /// instruction — re-execution will retry.
    ChannelStall { channel: u32, is_write: bool },
    /// `max_steps` exhausted without reaching `Stop`.
    MaxStepsExceeded,
    /// Backend-side error (out-of-bounds LS access, unimplemented
    /// opcode, segment validation failure, etc.).
    Error(String),
}

/// Snapshot of all observable SPU state at the end of execution.
/// This is the canonical comparison surface for differential testing.
#[derive(Debug, Clone)]
pub struct SpuStateSnapshot {
    pub pc: u32,
    pub gpr: Box<[u128; SPU_GPR_COUNT]>,
    /// Full 256 KB local store. Boxed to keep the struct small on the
    /// stack; backends can return this without expensive copies as
    /// long as ownership transfers cleanly.
    pub ls: Box<[u8; SPU_LS_SIZE]>,
    /// In-mailbox / out-mailbox / signal-notification queue depths
    /// after execution, for invariants the recompiler must preserve.
    pub channel_counts: ChannelCounts,
    /// R5.4a: parked state, if the SPU thread ended execution parked
    /// on a channel op. `None` for clean Stop / MaxStepsExceeded /
    /// Error endings, `Some(state)` only after a `ChannelStall`.
    /// Both backends must agree on this for byte-exact equivalence.
    pub park_state: Option<SpuParkState>,
    /// R5.4b: full SpuChannels state at end of execution. Lets the
    /// caller drive the wake → resume cycle (push/pop a mailbox,
    /// `try_resolve_park`, then `resume_from_state` from the parked
    /// PC). Pre-R5.4b only `channel_counts` was exposed, which is
    /// insufficient for resume because counts don't carry mailbox
    /// values or signal payloads.
    pub channels: SpuChannels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChannelCounts {
    pub in_mbox_depth: u32,
    pub out_mbox_depth: u32,
    pub out_intr_mbox_depth: u32,
    pub signal1_pending: bool,
    pub signal2_pending: bool,
}

#[derive(Debug, Clone)]
pub struct SpuExecutionResult {
    pub steps_executed: u64,
    pub stop_reason: ExecutionStopReason,
    pub final_state: SpuStateSnapshot,
}

// =====================================================================
// Backend trait
// =====================================================================

/// Any SPU execution backend (interpreter today; recompiler tomorrow)
/// implements this trait. The trait is intentionally tiny — backends
/// own their own configuration and any internal caches.
///
/// Backends MUST:
/// - Call `program.validate()` and surface errors as `Error(String)`.
/// - Initialise LS to all zeros before deploying segments.
/// - Reset GPRs and PC to the values implied by `program`.
/// - Report `steps_executed` as a count of *retired* instructions
///   (not speculative or chunk counts).
///
/// Backends MAY:
/// - Cache compiled blocks across calls (recompiler).
/// - Accept additional config via their own constructor.
pub trait SpuExecutor {
    /// Execute `program`. Returns the final state and stop reason.
    /// Should not panic — every error is propagated via
    /// [`ExecutionStopReason::Error`].
    fn execute(&mut self, program: &SpuProgram) -> SpuExecutionResult;

    /// A short stable identifier for diff reports ("interpreter",
    /// "recompiler-cranelift", etc.). Default = type name.
    fn backend_name(&self) -> &'static str {
        "anonymous"
    }
}

// =====================================================================
// Interpreter backend
// =====================================================================

/// [`SpuExecutor`] backed by `rpcs3-spu-interpreter`. This is the
/// reference oracle every other backend must match.
#[derive(Default)]
pub struct InterpreterExecutor;

impl SpuExecutor for InterpreterExecutor {
    fn execute(&mut self, program: &SpuProgram) -> SpuExecutionResult {
        if let Err(e) = program.validate() {
            return error_result(e.to_string());
        }

        let mut spu = SpuThread::new(0);
        for seg in &program.segments {
            if !spu.ls_write(seg.lsa, &seg.data) {
                return error_result(format!(
                    "ls_write failed at lsa=0x{:x} size=0x{:x}",
                    seg.lsa,
                    seg.data.len()
                ));
            }
        }
        for &(reg, value) in &program.initial_gpr_overrides {
            if (reg as usize) < spu.gpr.len() {
                spu.gpr[reg as usize] = value;
            }
        }
        // R6.7 C.4 — feed the pre-populated tag-stat queue into the
        // SPU's channels. Each `rdch ch24` during replay pops one
        // value. Empty queue for non-DMA programs (zero overhead).
        spu.channels.mfc_tag_stat_queue.extend(
            program.initial_mfc_tag_stat_queue.iter().copied(),
        );
        spu.pc = program.entry_pc & 0x3FFFC;

        let max = program.max_steps.min(usize::MAX as u64) as usize;
        let (steps, stop_reason) = match run_n(&mut spu, max) {
            Ok((n, StepOutcome::Stop(code))) => (n as u64, ExecutionStopReason::Stop(code)),
            Ok((n, StepOutcome::ChannelStall { channel, is_write })) => {
                (n as u64, ExecutionStopReason::ChannelStall { channel, is_write })
            }
            Ok((n, StepOutcome::Continue)) => (n as u64, ExecutionStopReason::MaxStepsExceeded),
            Err(e) => (0, ExecutionStopReason::Error(format!("{e:?}"))),
        };

        let final_state = snapshot_from_thread(&spu);
        SpuExecutionResult { steps_executed: steps, stop_reason, final_state }
    }

    fn backend_name(&self) -> &'static str {
        "interpreter"
    }
}

impl InterpreterExecutor {
    /// R5: resume an in-flight SPU run from a captured state. Used by
    /// the SPU recompiler's partial-fallback path when the JIT exits
    /// mid-program (compile failure on a target function, or a runtime
    /// `JIT_OUTCOME_UNKNOWN_OPCODE`). The caller passes the GPRs, LS,
    /// and PC already established by the JIT prefix; this method runs
    /// the interpreter from `pc` until Stop / channel stall / budget
    /// exhaustion.
    ///
    /// `prior_steps` is folded into `steps_executed` of the returned
    /// result so callers see a single combined step count covering
    /// both the JIT prefix and the interpreter suffix.
    ///
    /// R5.2 update: now also accepts `channels` so the JIT-mutated
    /// SpuChannels state (event_mask, mailbox slots, snr, etc.) flows
    /// across the partial-fallback boundary. The interpreter sees the
    /// real channel state the JIT was working with, which is required
    /// for channel-stall correctness — the interpreter can re-execute
    /// the same channel op and observe the same outcome it would have
    /// observed in a pure interpreter run.
    #[must_use]
    pub fn resume_from_state(
        &self,
        gpr: &[u128; SPU_GPR_COUNT],
        ls: &[u8; SPU_LS_SIZE],
        channels: &SpuChannels,
        pc: u32,
        max_steps_remaining: u64,
        prior_steps: u64,
    ) -> SpuExecutionResult {
        let mut spu = SpuThread::new(0);
        spu.gpr.copy_from_slice(gpr.as_ref());
        if !spu.ls_write(0, ls.as_ref()) {
            return error_result(
                "resume_from_state: bulk LS write failed".into(),
            );
        }
        spu.channels = channels.clone();
        spu.pc = pc & 0x3FFFC;

        let max = max_steps_remaining.min(usize::MAX as u64) as usize;
        let (steps, stop_reason) = match run_n(&mut spu, max) {
            Ok((n, StepOutcome::Stop(code))) => (n as u64, ExecutionStopReason::Stop(code)),
            Ok((n, StepOutcome::ChannelStall { channel, is_write })) => {
                (n as u64, ExecutionStopReason::ChannelStall { channel, is_write })
            }
            Ok((n, StepOutcome::Continue)) => (n as u64, ExecutionStopReason::MaxStepsExceeded),
            Err(e) => (0, ExecutionStopReason::Error(format!("{e:?}"))),
        };

        let final_state = snapshot_from_thread(&spu);
        SpuExecutionResult {
            steps_executed: prior_steps.saturating_add(steps),
            stop_reason,
            final_state,
        }
    }
}

/// Construct a "this backend errored" result with a default snapshot.
/// Public so other backend crates (e.g. `rpcs3-spu-recompiler`) can
/// produce identical-shape error results.
pub fn error_result(msg: String) -> SpuExecutionResult {
    SpuExecutionResult {
        steps_executed: 0,
        stop_reason: ExecutionStopReason::Error(msg),
        final_state: SpuStateSnapshot {
            pc: 0,
            gpr: Box::new([0u128; SPU_GPR_COUNT]),
            ls: Box::new([0u8; SPU_LS_SIZE]),
            channel_counts: ChannelCounts::default(),
            park_state: None,
            channels: SpuChannels::default(),
        },
    }
}

fn snapshot_from_thread(spu: &SpuThread) -> SpuStateSnapshot {
    let mut gpr = Box::new([0u128; SPU_GPR_COUNT]);
    gpr.copy_from_slice(&spu.gpr);

    let mut ls = Box::new([0u8; SPU_LS_SIZE]);
    if let Some(view) = spu.ls_read(0, SPU_LS_SIZE) {
        ls.copy_from_slice(view);
    }

    // Channel counts derived directly from the public fields on
    // SpuChannels. Mailbox depths are 0 or 1 (single-slot mailboxes).
    // SNR pending bits are not currently observable as separate
    // depth fields — the SPU sees signals fold into the value, so
    // we treat any non-zero SNR slot as "pending".
    let cc = ChannelCounts {
        in_mbox_depth: spu.channels.in_mbox.is_some() as u32,
        out_mbox_depth: spu.channels.out_mbox.is_some() as u32,
        out_intr_mbox_depth: spu.channels.out_intr_mbox.is_some() as u32,
        signal1_pending: spu.channels.snr[0] != 0,
        signal2_pending: spu.channels.snr[1] != 0,
    };

    SpuStateSnapshot {
        pc: spu.pc,
        gpr,
        ls,
        channel_counts: cc,
        // R5.4a: propagate park state into the snapshot so callers
        // can observe a parked SPU thread (interpreter sets this on
        // ChannelStall via `park_on_channel`).
        park_state: spu.park_state,
        // R5.4b: full channel state for the wake → resume cycle.
        channels: spu.channels.clone(),
    }
}

// =====================================================================
// Diff
// =====================================================================

/// Structured difference between two snapshots. `gpr_mismatches` lists
/// `(reg, a_value, b_value)`. `ls_mismatches` lists `(lsa, a_byte,
/// b_byte)` for the first N differing bytes (capped to keep reports
/// readable).
#[derive(Debug, Clone, Default)]
pub struct SpuDiff {
    pub pc_match: bool,
    pub channel_counts_match: bool,
    pub gpr_mismatches: Vec<(usize, u128, u128)>,
    pub ls_mismatches: Vec<(usize, u8, u8)>,
    pub ls_total_diff_bytes: usize,
    /// R5.4a: do both snapshots agree on `park_state`? Becomes false
    /// if one backend parked on a channel op and the other didn't, or
    /// if they parked on different pcs / different reasons.
    pub park_state_match: bool,
    /// R5.4b: do both snapshots agree on the full SpuChannels state?
    /// Stronger than `channel_counts_match` (which only checks
    /// mailbox depths and SNR-pending bits). False if mailbox values
    /// or event_stat/event_mask/decrementer/etc. differ.
    pub channels_match: bool,
}

impl SpuDiff {
    #[must_use]
    pub fn is_identical(&self) -> bool {
        self.pc_match
            && self.channel_counts_match
            && self.gpr_mismatches.is_empty()
            && self.ls_total_diff_bytes == 0
            && self.park_state_match
            && self.channels_match
    }
}

/// Compare two snapshots. Caps `ls_mismatches` reporting to `ls_report_limit`
/// entries (default `64`); `ls_total_diff_bytes` always reports the full count.
#[must_use]
pub fn diff_snapshots(a: &SpuStateSnapshot, b: &SpuStateSnapshot) -> SpuDiff {
    diff_snapshots_with_limit(a, b, 64)
}

#[must_use]
pub fn diff_snapshots_with_limit(
    a: &SpuStateSnapshot,
    b: &SpuStateSnapshot,
    ls_report_limit: usize,
) -> SpuDiff {
    let mut d = SpuDiff::default();
    d.pc_match = a.pc == b.pc;
    d.channel_counts_match = a.channel_counts == b.channel_counts;
    d.park_state_match = a.park_state == b.park_state;
    d.channels_match = a.channels == b.channels;

    for (i, (av, bv)) in a.gpr.iter().zip(b.gpr.iter()).enumerate() {
        if av != bv {
            d.gpr_mismatches.push((i, *av, *bv));
        }
    }

    for (i, (av, bv)) in a.ls.iter().zip(b.ls.iter()).enumerate() {
        if av != bv {
            d.ls_total_diff_bytes += 1;
            if d.ls_mismatches.len() < ls_report_limit {
                d.ls_mismatches.push((i, *av, *bv));
            }
        }
    }

    d
}

/// Convenience: execute the same program on two backends and diff the
/// results. The recompiler will use this directly in its CI.
pub fn run_and_diff(
    a: &mut dyn SpuExecutor,
    b: &mut dyn SpuExecutor,
    program: &SpuProgram,
) -> (SpuExecutionResult, SpuExecutionResult, SpuDiff) {
    let ra = a.execute(program);
    let rb = b.execute(program);
    let diff = diff_snapshots(&ra.final_state, &rb.final_state);
    (ra, rb, diff)
}

// =====================================================================
// R5.4c — single-threaded park / wake / resume executor
// =====================================================================

/// Typed event surfaced by [`SpuSingleThreadExecutor`]. Folds an
/// [`SpuExecutionResult`] into one of four observable outcomes a caller
/// can drive a single-threaded park/wake/resume cycle around.
#[derive(Debug, Clone)]
pub enum SpuExecEvent {
    /// Program reached `stop` cleanly. Cycle is over.
    Finished {
        stop_code: u32,
        snapshot: SpuStateSnapshot,
        steps: u64,
    },
    /// Channel op blocked. The caller MUST mutate the snapshot's
    /// channels (via the wake helpers in `rpcs3_spu_thread`) and call
    /// [`SpuSingleThreadExecutor::resume_after_wake`] with the post-
    /// wake channels and the returned `pc`. Resume must NOT happen if
    /// the wake helper returned `StillBlocked`.
    Parked {
        pc: u32,
        reason: SpuParkReason,
        snapshot: SpuStateSnapshot,
        steps: u64,
    },
    /// Hard error from the underlying backend (BadChannel,
    /// unimplemented opcode, segment validation, etc.). The caller
    /// MUST NOT resume.
    Error {
        message: String,
        snapshot: SpuStateSnapshot,
        steps: u64,
    },
    /// `program.max_steps` exhausted without Stop or Park.
    BudgetExhausted {
        snapshot: SpuStateSnapshot,
        steps: u64,
    },
}

impl SpuExecEvent {
    /// Borrow the snapshot regardless of variant. Convenience for
    /// driver code that wants to inspect channels without moving the
    /// event apart.
    #[must_use]
    pub fn snapshot(&self) -> &SpuStateSnapshot {
        match self {
            Self::Finished { snapshot, .. }
            | Self::Parked { snapshot, .. }
            | Self::Error { snapshot, .. }
            | Self::BudgetExhausted { snapshot, .. } => snapshot,
        }
    }

    /// Cumulative retired-step count at this event.
    #[must_use]
    pub fn steps(&self) -> u64 {
        match self {
            Self::Finished { steps, .. }
            | Self::Parked { steps, .. }
            | Self::Error { steps, .. }
            | Self::BudgetExhausted { steps, .. } => *steps,
        }
    }

    /// Convenience: `true` iff this is a `Parked` event.
    #[must_use]
    pub fn is_parked(&self) -> bool {
        matches!(self, Self::Parked { .. })
    }
}

/// R5.4c thin single-threaded driver around any [`SpuExecutor`].
/// Classifies the backend's [`SpuExecutionResult`] into a typed
/// [`SpuExecEvent`] and provides a `resume_after_wake` entry point
/// that drives the interpreter from the parked PC with caller-supplied
/// post-wake channels.
///
/// **Not concurrent.** No threads, no scheduler, no event loop. The
/// caller drives the cycle explicitly:
///
/// ```ignore
/// let mut exec = SpuSingleThreadExecutor::new();
/// let mut backend = RecompilerExecutor::new();
/// match exec.run_until_event(&mut backend, &program) {
///     SpuExecEvent::Parked { pc, snapshot, steps, .. } => {
///         // Reconstruct a thread to use the wake helpers, mutate
///         // channels, then resume.
///         let mut t = SpuThread::new(0);
///         t.channels = snapshot.channels.clone();
///         t.park_state = snapshot.park_state;
///         let SpuWakeResult::Ready { pc: wake_pc } =
///             t.ppu_push_inmbox_and_try_wake(0x42)
///         else { panic!("expected Ready"); };
///         let next = exec.resume_after_wake(&snapshot, &t.channels, wake_pc, &program, steps);
///         // ...
///     }
///     other => { /* Finished / Error / BudgetExhausted */ }
/// }
/// ```
///
/// Resume always goes through `InterpreterExecutor::resume_from_state`
/// because that is the only public resume API today; future backends
/// may grow their own resume paths and the executor can be retargeted
/// at the trait level then.
#[derive(Default)]
pub struct SpuSingleThreadExecutor {
    interp: InterpreterExecutor,
}

impl SpuSingleThreadExecutor {
    #[must_use]
    pub fn new() -> Self {
        Self { interp: InterpreterExecutor::default() }
    }

    /// Run `program` on `backend` from `program.entry_pc`. Returns
    /// the first observable event.
    pub fn run_until_event<E: SpuExecutor>(
        &mut self,
        backend: &mut E,
        program: &SpuProgram,
    ) -> SpuExecEvent {
        Self::classify(backend.execute(program))
    }

    /// Resume execution from `wake_pc` (which MUST be the value the
    /// caller's wake helper returned via `SpuWakeResult::Ready`). The
    /// snapshot's GPRs / LS are reused; `wake_channels` is the post-
    /// wake channel state the caller built (typically by mutating the
    /// snapshot's channels via the wake helpers).
    ///
    /// `prior_steps` is the cumulative retired-step count from the
    /// event being resumed (i.e. the previous event's `steps`). It
    /// folds into the returned event's `steps` so a chain of
    /// `run → wake → resume` reports a monotonic step count.
    pub fn resume_after_wake(
        &self,
        snapshot: &SpuStateSnapshot,
        wake_channels: &SpuChannels,
        wake_pc: u32,
        program: &SpuProgram,
        prior_steps: u64,
    ) -> SpuExecEvent {
        let remaining = program.max_steps.saturating_sub(prior_steps);
        let result = self.interp.resume_from_state(
            snapshot.gpr.as_ref(),
            snapshot.ls.as_ref(),
            wake_channels,
            wake_pc,
            remaining,
            prior_steps,
        );
        Self::classify(result)
    }

    /// Map an [`SpuExecutionResult`] to the typed event. Pure
    /// classification — no extra state.
    fn classify(result: SpuExecutionResult) -> SpuExecEvent {
        let steps = result.steps_executed;
        match result.stop_reason {
            ExecutionStopReason::Stop(code) => SpuExecEvent::Finished {
                stop_code: code,
                snapshot: result.final_state,
                steps,
            },
            ExecutionStopReason::ChannelStall { .. } => {
                // R5.4a invariant: ChannelStall ⇒ park_state is Some.
                let park = result.final_state.park_state.expect(
                    "R5.4a invariant: ChannelStall must produce park_state",
                );
                SpuExecEvent::Parked {
                    pc: park.pc,
                    reason: park.reason,
                    snapshot: result.final_state,
                    steps,
                }
            }
            ExecutionStopReason::Error(msg) => SpuExecEvent::Error {
                message: msg,
                snapshot: result.final_state,
                steps,
            },
            ExecutionStopReason::MaxStepsExceeded => SpuExecEvent::BudgetExhausted {
                snapshot: result.final_state,
                steps,
            },
        }
    }
}

// =====================================================================
// R5.4e — Synthetic single-threaded PPU↔SPU lockstep driver
// =====================================================================

/// PPU-side action in a [`SpuPpuLockstepDriver`] script. Actions are
/// either side-effect actions that mutate the SPU's channels and may
/// trigger a wake/resume (`PushInMbox`, `PopOutMbox`, `Signal`), or
/// assertions that check the SPU's current state (`ExpectPark`,
/// `ExpectFinished`). Actions execute in order against the driver's
/// current state.
///
/// **Wake semantics** for the side-effect actions:
/// - The driver mutates the current snapshot's channels first.
/// - Then attempts a wake by reconstructing a shadow `SpuThread` from
///   the snapshot's channels + park metadata and calling
///   `try_resolve_park`.
/// - On `SpuWakeResult::Ready`, the driver calls
///   [`SpuSingleThreadExecutor::resume_after_wake`] and runs the SPU
///   until the next event.
/// - On `StillBlocked` or `NotParked`, the driver does NOT resume.
#[derive(Debug, Clone)]
pub enum PpuAction {
    /// Push `value` into `in_mbox` (best-effort). On RDINMBOX park,
    /// triggers wake. On non-parked or other parks, side effect only.
    PushInMbox(u32),
    /// Drain `out_mbox`. If `expect == Some(v)`, the popped value
    /// must equal `v` or the action returns
    /// [`LockstepError::OutMboxMismatch`]. On WROUTMBOX park,
    /// triggers wake.
    PopOutMbox { expect: Option<u32> },
    /// OR `value` into `snr[slot]`. On RDSIGNOTIFY{1,2} park,
    /// triggers wake. R5.11: `read(SPU_RDSIGNOTIFY*)` now matches
    /// Cell BE semantics (returns `WouldStall` when count == 0), so
    /// natural run-until-park on signotify IS reachable — the
    /// `single_spu_signal_v1` fixture exercises this end-to-end.
    Signal { slot: usize, value: u32 },
    /// Assert the SPU is currently parked with `reason`.
    ExpectPark { reason: SpuParkReason },
    /// Assert the SPU has finished cleanly with `stop_code`.
    ExpectFinished { stop_code: u32 },
}

/// Outcome of applying a [`PpuAction`].
#[derive(Debug, Clone)]
pub enum PpuOutcome {
    /// `PushInMbox` / `Signal`: side effect applied; wake attempted.
    WakeTried { wake: SpuWakeResult },
    /// `PopOutMbox`: drained `popped` from out_mbox (`None` if mbox
    /// was already empty); wake attempted afterwards.
    Drained {
        popped: Option<u32>,
        wake: SpuWakeResult,
    },
    /// `ExpectPark` / `ExpectFinished`: assertion passed.
    Asserted,
}

/// Re-export-shaped wake result for callers that don't want to import
/// from `rpcs3_spu_thread`.
pub use rpcs3_spu_thread::SpuWakeResult;

/// Lightweight summary of an [`SpuExecEvent`] for the trace, dropping
/// the (heavy) snapshot. Final snapshot is attached separately to
/// [`LockstepTrace`].
#[derive(Debug, Clone)]
pub enum SpuEventKind {
    Parked { pc: u32, reason: SpuParkReason },
    Finished { stop_code: u32 },
    Error { message: String },
    BudgetExhausted,
}

impl SpuEventKind {
    fn from_event(ev: &SpuExecEvent) -> Self {
        match ev {
            SpuExecEvent::Parked { pc, reason, .. } => {
                Self::Parked { pc: *pc, reason: *reason }
            }
            SpuExecEvent::Finished { stop_code, .. } => {
                Self::Finished { stop_code: *stop_code }
            }
            SpuExecEvent::Error { message, .. } => {
                Self::Error { message: message.clone() }
            }
            SpuExecEvent::BudgetExhausted { .. } => Self::BudgetExhausted,
        }
    }
}

/// One entry in the lockstep trace. Captures both SPU events and PPU
/// actions in execution order so a caller can post-mortem the cycle.
#[derive(Debug, Clone)]
pub enum TraceRecord {
    SpuEvent {
        kind: SpuEventKind,
        steps_at_event: u64,
    },
    PpuAction {
        action: PpuAction,
        outcome: PpuOutcome,
    },
    /// Resume kicked off from `from_pc` (a Ready wake's pc) at
    /// `prior_steps` cumulative steps.
    ResumeStarted {
        from_pc: u32,
        prior_steps: u64,
    },
}

/// Errors surfaced by [`SpuPpuLockstepDriver::apply`] /
/// [`SpuPpuLockstepDriver::run_script`].
#[derive(Debug, Clone)]
pub enum LockstepError {
    /// `ExpectPark` saw a non-parked SPU state.
    ExpectedParkGot {
        expected_reason: SpuParkReason,
        actual: SpuEventKind,
    },
    /// `ExpectFinished` saw a non-finished SPU state.
    ExpectedFinishedGot {
        expected_stop_code: u32,
        actual: SpuEventKind,
    },
    /// `PopOutMbox { expect: Some(v) }` saw a value that didn't match.
    OutMboxMismatch { expected: u32, got: Option<u32> },
    /// SPU surfaced an Error event during execution.
    SpuExecError { message: String, steps: u64 },
    /// `apply` was called against a driver that hasn't been started
    /// (no prior `step_spu` / `run_script` call).
    DriverNotStarted,
}

/// Final result of a lockstep script run.
#[derive(Debug, Clone)]
pub struct LockstepTrace {
    /// Ordered SPU events + PPU actions + resume markers.
    pub records: Vec<TraceRecord>,
    /// SPU's final state at end of script (Parked / Finished / etc.).
    pub final_event_kind: SpuEventKind,
    /// Cumulative retired-step count at end.
    pub total_steps: u64,
    /// Final SPU snapshot (channels, GPRs, LS, park_state).
    pub final_snapshot: SpuStateSnapshot,
}

/// Internal state machine for [`SpuPpuLockstepDriver`].
enum DriverState {
    /// SPU has not run yet. Constructed state.
    NeedsInitialRun,
    /// SPU is parked awaiting a wake. The snapshot's `channels` are
    /// the live channel state; `pc` and `reason` are the park
    /// metadata. Mutating `snapshot.channels` and calling
    /// `try_resolve_park` against a shadow built from these fields
    /// is how the driver attempts wake.
    Parked {
        snapshot: SpuStateSnapshot,
        pc: u32,
        reason: SpuParkReason,
        steps: u64,
    },
    /// SPU has reached a terminal state. Snapshot is the final state
    /// the SPU produced; PPU may still drain `out_mbox` against it.
    Done {
        kind: SpuEventKind,
        snapshot: SpuStateSnapshot,
        steps: u64,
    },
}

/// R5.4e — synthetic single-threaded PPU↔SPU lockstep driver.
///
/// Wraps [`SpuSingleThreadExecutor`] and any [`SpuExecutor`] backend
/// to drive scripted PPU↔SPU mailbox / signal handshakes. Strictly
/// turn-by-turn:
///
/// 1. Caller calls `step_spu` (or `run_script`); SPU runs until its
///    first event (Parked / Finished / Error / BudgetExhausted).
/// 2. Caller applies a [`PpuAction`]:
///    - Side-effect actions (`PushInMbox`, `PopOutMbox`, `Signal`)
///      mutate `snapshot.channels` and try wake. On `Ready`, the
///      driver auto-resumes via
///      [`SpuSingleThreadExecutor::resume_after_wake`] and runs SPU
///      until the next event. On `StillBlocked` / `NotParked`, no
///      resume happens.
///    - Assertion actions (`ExpectPark`, `ExpectFinished`) check
///      the current state without mutating it.
/// 3. Repeat until the script ends or the SPU reaches a terminal
///    state.
///
/// **Not a scheduler.** No threads. No event loop. The PPU side is a
/// scripted sequence; the SPU side is a single deterministic
/// executor. This is a validation harness for byte-exact mailbox /
/// signal behavior, not a runtime.
///
/// **Resume goes through the interpreter** today (R5.4c contract).
/// The initial run can be on any backend — JIT or interpreter — so
/// JIT-stall→park→PPU-action→interpreter-resume cycles are
/// validated end-to-end. A future R5.4d would add a JIT-side resume
/// path.
pub struct SpuPpuLockstepDriver<'b, E: SpuExecutor> {
    backend: &'b mut E,
    program: SpuProgram,
    executor: SpuSingleThreadExecutor,
    state: DriverState,
    records: Vec<TraceRecord>,
}

impl<'b, E: SpuExecutor> SpuPpuLockstepDriver<'b, E> {
    /// Construct a fresh driver. SPU is in `NeedsInitialRun`; call
    /// `step_spu` or `run_script` to start.
    pub fn new(backend: &'b mut E, program: SpuProgram) -> Self {
        Self {
            backend,
            program,
            executor: SpuSingleThreadExecutor::new(),
            state: DriverState::NeedsInitialRun,
            records: Vec::new(),
        }
    }

    /// Whether the SPU is currently parked.
    #[must_use]
    pub fn is_parked(&self) -> bool {
        matches!(self.state, DriverState::Parked { .. })
    }

    /// Whether the SPU has reached a terminal state.
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(self.state, DriverState::Done { .. })
    }

    /// Park metadata, if currently parked.
    #[must_use]
    pub fn park_info(&self) -> Option<(u32, SpuParkReason)> {
        if let DriverState::Parked { pc, reason, .. } = &self.state {
            Some((*pc, *reason))
        } else {
            None
        }
    }

    /// Current SPU event kind (Parked / Finished / Error /
    /// BudgetExhausted), or `None` if the SPU hasn't been started.
    /// Read-only mirror of the driver's internal state — used by the
    /// R5.5 trace replay engine to inspect state between events.
    #[must_use]
    pub fn current_event_kind(&self) -> Option<SpuEventKind> {
        match &self.state {
            DriverState::NeedsInitialRun => None,
            DriverState::Parked { pc, reason, .. } => {
                Some(SpuEventKind::Parked { pc: *pc, reason: *reason })
            }
            DriverState::Done { kind, .. } => Some(kind.clone()),
        }
    }

    /// Borrow the current snapshot (parked or done). Returns `None`
    /// before the initial run.
    #[must_use]
    pub fn current_snapshot(&self) -> Option<&SpuStateSnapshot> {
        match &self.state {
            DriverState::NeedsInitialRun => None,
            DriverState::Parked { snapshot, .. } | DriverState::Done { snapshot, .. } => {
                Some(snapshot)
            }
        }
    }

    /// Cumulative retired-step count at the current state. `0` before
    /// the initial run.
    #[must_use]
    pub fn total_steps(&self) -> u64 {
        match &self.state {
            DriverState::NeedsInitialRun => 0,
            DriverState::Parked { steps, .. } | DriverState::Done { steps, .. } => *steps,
        }
    }

    /// Run SPU until the next event. No-op if already parked or done.
    /// Records an `SpuEvent` trace record.
    pub fn step_spu(&mut self) {
        if !matches!(self.state, DriverState::NeedsInitialRun) {
            return;
        }
        let event = self.executor.run_until_event(&mut *self.backend, &self.program);
        self.records.push(TraceRecord::SpuEvent {
            kind: SpuEventKind::from_event(&event),
            steps_at_event: event.steps(),
        });
        self.transition_from_event(event);
    }

    fn transition_from_event(&mut self, event: SpuExecEvent) {
        self.state = match event {
            SpuExecEvent::Parked { pc, reason, snapshot, steps } => {
                DriverState::Parked { snapshot, pc, reason, steps }
            }
            SpuExecEvent::Finished { stop_code, snapshot, steps } => {
                DriverState::Done {
                    kind: SpuEventKind::Finished { stop_code },
                    snapshot,
                    steps,
                }
            }
            SpuExecEvent::Error { message, snapshot, steps } => {
                DriverState::Done {
                    kind: SpuEventKind::Error { message },
                    snapshot,
                    steps,
                }
            }
            SpuExecEvent::BudgetExhausted { snapshot, steps } => {
                DriverState::Done {
                    kind: SpuEventKind::BudgetExhausted,
                    snapshot,
                    steps,
                }
            }
        };
    }

    /// Mutable access to the current snapshot's channels, regardless
    /// of whether parked or done.
    fn current_channels_mut(&mut self) -> Option<&mut SpuChannels> {
        match &mut self.state {
            DriverState::Parked { snapshot, .. } => Some(&mut snapshot.channels),
            DriverState::Done { snapshot, .. } => Some(&mut snapshot.channels),
            DriverState::NeedsInitialRun => None,
        }
    }

    /// Try to wake the parked SPU using the current snapshot's
    /// channels. Returns `NotParked` if not parked.
    fn try_wake(&self) -> SpuWakeResult {
        if let DriverState::Parked { snapshot, pc, reason, .. } = &self.state {
            let mut shadow = SpuThread::new(0);
            shadow.channels = snapshot.channels.clone();
            shadow.park_state = Some(SpuParkState { pc: *pc, reason: *reason });
            shadow.try_resolve_park()
        } else {
            SpuWakeResult::NotParked
        }
    }

    /// On a `Ready` wake, transition out of Parked, drive
    /// `resume_after_wake`, and record both the `ResumeStarted` and
    /// the resulting `SpuEvent` trace records.
    fn resume_on_ready(&mut self, wake: SpuWakeResult) {
        let SpuWakeResult::Ready { pc: wake_pc } = wake else { return; };
        let old_state = std::mem::replace(&mut self.state, DriverState::NeedsInitialRun);
        let (snapshot, prior_steps) = match old_state {
            DriverState::Parked { snapshot, steps, .. } => (snapshot, steps),
            other => {
                self.state = other;
                return;
            }
        };
        self.records.push(TraceRecord::ResumeStarted {
            from_pc: wake_pc,
            prior_steps,
        });
        let resume_event = self.executor.resume_after_wake(
            &snapshot,
            &snapshot.channels,
            wake_pc,
            &self.program,
            prior_steps,
        );
        self.records.push(TraceRecord::SpuEvent {
            kind: SpuEventKind::from_event(&resume_event),
            steps_at_event: resume_event.steps(),
        });
        self.transition_from_event(resume_event);
    }

    /// Apply a single PPU action against the driver's current state.
    /// Records the action + outcome in the trace; on Ready wake,
    /// auto-resumes and records the post-resume event.
    pub fn apply(&mut self, action: PpuAction) -> Result<PpuOutcome, LockstepError> {
        match action {
            PpuAction::PushInMbox(value) => {
                let _ = self.current_channels_mut()
                    .map(|ch| ch.ppu_push_inmbox(value));
                let wake = self.try_wake();
                let outcome = PpuOutcome::WakeTried { wake };
                self.records.push(TraceRecord::PpuAction {
                    action: PpuAction::PushInMbox(value),
                    outcome: outcome.clone(),
                });
                self.resume_on_ready(wake);
                Ok(outcome)
            }
            PpuAction::PopOutMbox { expect } => {
                let popped = self.current_channels_mut()
                    .and_then(|ch| ch.ppu_pop_outmbox());
                if let Some(exp) = expect {
                    if popped != Some(exp) {
                        return Err(LockstepError::OutMboxMismatch {
                            expected: exp,
                            got: popped,
                        });
                    }
                }
                let wake = self.try_wake();
                let outcome = PpuOutcome::Drained { popped, wake };
                self.records.push(TraceRecord::PpuAction {
                    action: PpuAction::PopOutMbox { expect },
                    outcome: outcome.clone(),
                });
                self.resume_on_ready(wake);
                Ok(outcome)
            }
            PpuAction::Signal { slot, value } => {
                let _ = self.current_channels_mut()
                    .map(|ch| ch.signal(slot, value));
                let wake = self.try_wake();
                let outcome = PpuOutcome::WakeTried { wake };
                self.records.push(TraceRecord::PpuAction {
                    action: PpuAction::Signal { slot, value },
                    outcome: outcome.clone(),
                });
                self.resume_on_ready(wake);
                Ok(outcome)
            }
            PpuAction::ExpectPark { reason } => {
                match &self.state {
                    DriverState::Parked { reason: r, .. } if *r == reason => {
                        self.records.push(TraceRecord::PpuAction {
                            action: PpuAction::ExpectPark { reason },
                            outcome: PpuOutcome::Asserted,
                        });
                        Ok(PpuOutcome::Asserted)
                    }
                    other => Err(LockstepError::ExpectedParkGot {
                        expected_reason: reason,
                        actual: actual_kind(other),
                    }),
                }
            }
            PpuAction::ExpectFinished { stop_code } => {
                match &self.state {
                    DriverState::Done { kind: SpuEventKind::Finished { stop_code: sc }, .. }
                        if *sc == stop_code =>
                    {
                        self.records.push(TraceRecord::PpuAction {
                            action: PpuAction::ExpectFinished { stop_code },
                            outcome: PpuOutcome::Asserted,
                        });
                        Ok(PpuOutcome::Asserted)
                    }
                    other => Err(LockstepError::ExpectedFinishedGot {
                        expected_stop_code: stop_code,
                        actual: actual_kind(other),
                    }),
                }
            }
        }
    }

    /// High-level orchestrator. Calls `step_spu` once if the SPU
    /// hasn't run yet, then applies each action in order. On any
    /// `LockstepError` the trace built so far is dropped — callers
    /// who need partial traces should call `step_spu` / `apply`
    /// directly.
    pub fn run_script(
        &mut self,
        script: &[PpuAction],
    ) -> Result<LockstepTrace, LockstepError> {
        if matches!(self.state, DriverState::NeedsInitialRun) {
            self.step_spu();
            if let DriverState::Done {
                kind: SpuEventKind::Error { message },
                steps,
                ..
            } = &self.state
            {
                return Err(LockstepError::SpuExecError {
                    message: message.clone(),
                    steps: *steps,
                });
            }
        }
        for action in script {
            self.apply(action.clone())?;
        }
        let (final_kind, total_steps, final_snapshot) = match &self.state {
            DriverState::NeedsInitialRun => {
                return Err(LockstepError::DriverNotStarted);
            }
            DriverState::Parked { snapshot, pc, reason, steps } => (
                SpuEventKind::Parked { pc: *pc, reason: *reason },
                *steps,
                snapshot.clone(),
            ),
            DriverState::Done { kind, snapshot, steps } => {
                (kind.clone(), *steps, snapshot.clone())
            }
        };
        Ok(LockstepTrace {
            records: std::mem::take(&mut self.records),
            final_event_kind: final_kind,
            total_steps,
            final_snapshot,
        })
    }
}

fn actual_kind(state: &DriverState) -> SpuEventKind {
    match state {
        DriverState::Parked { pc, reason, .. } => {
            SpuEventKind::Parked { pc: *pc, reason: *reason }
        }
        DriverState::Done { kind, .. } => kind.clone(),
        DriverState::NeedsInitialRun => SpuEventKind::BudgetExhausted, // sentinel
    }
}

// =====================================================================
// R5.5 — Deterministic PPU↔SPU trace replay layer
// =====================================================================
//
// Sits on top of `SpuPpuLockstepDriver` (R5.4e) and adds:
// - Richer event vocabulary (state assertions, channel-state asserts,
//   GPR-lane asserts, kind-only wake matching).
// - Event-indexed error reporting so a failure points at the exact
//   trace event that diverged.
// - Human-readable summary export for debug / trace review.
//
// **Not a scheduler.** No threads, no event loop. The trace is just a
// Vec<TraceEvent> the engine plays back deterministically against the
// underlying `SpuExecutor` backend (interpreter or recompiler).

/// Kind-only mirror of [`SpuWakeResult`] — drops the `pc` payload of
/// `Ready` so trace authors can express "expect wake" without
/// hardcoding the parked PC. Useful because the parked PC is already
/// asserted via [`TraceEvent::ExpectSpuPark { pc, .. }`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuWakeResultKind {
    NotParked,
    StillBlocked,
    Ready,
}

impl SpuWakeResultKind {
    /// Project a real [`SpuWakeResult`] down to its kind.
    #[must_use]
    pub fn from_actual(actual: SpuWakeResult) -> Self {
        match actual {
            SpuWakeResult::NotParked => Self::NotParked,
            SpuWakeResult::StillBlocked => Self::StillBlocked,
            SpuWakeResult::Ready { .. } => Self::Ready,
        }
    }

    fn matches(self, actual: SpuWakeResult) -> bool {
        matches!(
            (self, actual),
            (Self::NotParked, SpuWakeResult::NotParked)
            | (Self::StillBlocked, SpuWakeResult::StillBlocked)
            | (Self::Ready, SpuWakeResult::Ready { .. })
        )
    }
}

/// One step in a deterministic PPU↔SPU trace. Events come in two
/// flavors: side-effect events that drive the SPU forward
/// (`PpuPushInMbox`, `PpuPopOutMbox`, `PpuSignal`) and assertion
/// events that check state without mutating it
/// (`ExpectSpuPark`, `ExpectSpuFinished`, `ExpectGprWord`,
/// `ExpectChannelState`).
#[derive(Debug, Clone)]
pub enum TraceEvent {
    /// Assert SPU is currently parked with `reason`. If `pc` is
    /// `Some`, also assert the parked PC equals it.
    ExpectSpuPark { reason: SpuParkReason, pc: Option<u32> },
    /// PPU pushes `value` to in_mbox; expect wake of `expect_wake` kind.
    PpuPushInMbox { value: u32, expect_wake: SpuWakeResultKind },
    /// PPU drains out_mbox. If `expect` is `Some(v)`, popped value
    /// must equal `v`. If `expect_wake` is `Some(kind)`, wake kind
    /// must match. (Pop is the only PPU-action that has an optional
    /// wake check — its primary purpose is value drain.)
    PpuPopOutMbox {
        expect: Option<u32>,
        expect_wake: Option<SpuWakeResultKind>,
    },
    /// PPU OR-merges `value` into `snr[slot]`; expect wake kind.
    PpuSignal {
        slot: usize,
        value: u32,
        expect_wake: SpuWakeResultKind,
    },
    /// Assert SPU has finished cleanly with `stop_code`.
    ExpectSpuFinished { stop_code: u32 },
    /// Assert `gpr[reg]` lane (0=high, 3=low) equals `value`.
    ExpectGprWord { reg: usize, lane: usize, value: u32 },
    /// Assert exact channel state. `None` for a mailbox field means
    /// "expect empty"; `Some(v)` means "expect Some(v)". SNR fields
    /// are u32 because `SpuChannels::snr` is `[u32; 2]` (no Option).
    ExpectChannelState {
        in_mbox: Option<u32>,
        out_mbox: Option<u32>,
        out_intr_mbox: Option<u32>,
        snr1: u32,
        snr2: u32,
    },
}

/// Outcome of replaying one trace event.
#[derive(Debug, Clone)]
pub enum ReplayOutcome {
    /// Assertion event passed (ExpectSpuPark / ExpectSpuFinished /
    /// ExpectGprWord / ExpectChannelState).
    AssertionPassed,
    /// PPU side-effect event applied; carries the actual wake result
    /// and (for pop) the drained value for trace summary / debugging.
    PpuActionApplied {
        actual_wake: SpuWakeResult,
        popped: Option<u32>,
    },
}

/// One record in a [`TraceReplayReport`], one per trace event.
#[derive(Debug, Clone)]
pub struct TraceReplayRecord {
    pub event_index: usize,
    pub event: TraceEvent,
    pub outcome: ReplayOutcome,
    /// Cumulative retired-step count at the time this event was
    /// processed (BEFORE applying side-effect events; identical for
    /// assertions).
    pub steps_at: u64,
}

/// Final report from a successful [`replay_trace`] run.
#[derive(Debug, Clone)]
pub struct TraceReplayReport {
    pub records: Vec<TraceReplayRecord>,
    pub final_event_kind: SpuEventKind,
    pub total_steps: u64,
    pub final_snapshot: SpuStateSnapshot,
}

impl TraceReplayReport {
    /// Human-readable trace summary: header line + one line per event.
    /// Includes event index, steps-at, event payload, outcome, and
    /// (for the final state) the SPU event kind / total steps.
    #[must_use]
    pub fn summary(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let _ = writeln!(
            out,
            "TraceReplayReport: {n} events processed, final={final_kind:?}, total_steps={steps}",
            n = self.records.len(),
            final_kind = self.final_event_kind,
            steps = self.total_steps,
        );
        for r in &self.records {
            let _ = writeln!(
                out,
                "  [{idx}] steps={steps} event={event:?} outcome={outcome:?}",
                idx = r.event_index,
                steps = r.steps_at,
                event = r.event,
                outcome = r.outcome,
            );
        }
        if let SpuEventKind::Finished { stop_code } = self.final_event_kind {
            let _ = writeln!(out, "  [final] stop_code={stop_code:#x}");
        }
        out
    }

    /// Same as [`summary`] but prepends a `== {label} ==` header line.
    /// Useful for fixture-driven trace replays where multiple traces
    /// flow through the same logging stream and need provenance.
    #[must_use]
    pub fn summary_with_label(&self, label: &str) -> String {
        let mut s = format!("== {label} ==\n");
        s.push_str(&self.summary());
        s
    }
}

/// Errors from [`replay_trace`]. Always carry the failing
/// `event_index` so the caller can locate the divergence in the
/// input trace.
#[derive(Debug, Clone)]
pub struct TraceReplayError {
    pub event_index: usize,
    pub kind: TraceReplayErrorKind,
}

#[derive(Debug, Clone)]
pub enum TraceReplayErrorKind {
    /// Trace expected one SPU state, observed another.
    UnexpectedSpuState { expected: String, actual: SpuEventKind },
    /// Park PC didn't match the trace's expected PC.
    ParkPcMismatch { expected: u32, actual: u32 },
    /// Park reason didn't match.
    ParkReasonMismatch {
        expected: SpuParkReason,
        actual: SpuParkReason,
    },
    /// PPU action's actual wake kind didn't match the expected kind.
    WakeKindMismatch {
        expected: SpuWakeResultKind,
        actual: SpuWakeResult,
    },
    /// PopOutMbox value didn't match expected.
    OutMboxValueMismatch { expected: u32, got: Option<u32> },
    /// ExpectGprWord lane value mismatch.
    GprMismatch {
        reg: usize,
        lane: usize,
        expected: u32,
        got: u32,
    },
    /// ExpectGprWord with `lane >= 4` (only 0..=3 are valid).
    InvalidGprLane { reg: usize, lane: usize },
    /// ExpectChannelState mismatch — `detail` summarizes the diff.
    ChannelStateMismatch { detail: String },
    /// Initial SPU run produced an Error event.
    SpuExecError { message: String },
    /// Driver was not started before processing events. This is an
    /// internal invariant — `replay_trace` always runs `step_spu`
    /// first — but surfaced if reached.
    InitialRunNotStarted,
}

impl std::fmt::Display for TraceReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "trace replay error at event {}: {:?}", self.event_index, self.kind)
    }
}

impl std::error::Error for TraceReplayError {}

/// Deterministic replay of a `&[TraceEvent]` script against any
/// [`SpuExecutor`] backend. Internally constructs a
/// [`SpuPpuLockstepDriver`], runs SPU until first event, then
/// processes each trace event in order, producing a
/// [`TraceReplayReport`] on success or a [`TraceReplayError`] keyed
/// by the offending event index.
///
/// **Determinism** comes from the driver's strict turn-by-turn
/// model: for each side-effect event the SPU runs to a definite
/// next event before the next trace event is processed. Assertion
/// events do not advance state.
pub fn replay_trace<E: SpuExecutor>(
    backend: &mut E,
    program: SpuProgram,
    events: &[TraceEvent],
) -> Result<TraceReplayReport, TraceReplayError> {
    let mut driver = SpuPpuLockstepDriver::new(backend, program);
    driver.step_spu();

    // Surface initial-run errors immediately at event_index 0.
    if let Some(SpuEventKind::Error { message }) = driver.current_event_kind() {
        return Err(TraceReplayError {
            event_index: 0,
            kind: TraceReplayErrorKind::SpuExecError { message },
        });
    }

    let mut records = Vec::with_capacity(events.len());
    for (idx, ev) in events.iter().enumerate() {
        let steps_at = driver.total_steps();
        let outcome = process_trace_event(&mut driver, ev, idx)?;
        records.push(TraceReplayRecord {
            event_index: idx,
            event: ev.clone(),
            outcome,
            steps_at,
        });
    }

    let final_kind = driver
        .current_event_kind()
        .ok_or(TraceReplayError {
            event_index: events.len(),
            kind: TraceReplayErrorKind::InitialRunNotStarted,
        })?;
    let final_snapshot = driver
        .current_snapshot()
        .expect("snapshot exists after step_spu")
        .clone();
    let total_steps = driver.total_steps();

    Ok(TraceReplayReport {
        records,
        final_event_kind: final_kind,
        total_steps,
        final_snapshot,
    })
}

fn process_trace_event<E: SpuExecutor>(
    driver: &mut SpuPpuLockstepDriver<'_, E>,
    event: &TraceEvent,
    event_index: usize,
) -> Result<ReplayOutcome, TraceReplayError> {
    match event {
        TraceEvent::ExpectSpuPark { reason, pc } => {
            let actual = driver.current_event_kind().ok_or(TraceReplayError {
                event_index,
                kind: TraceReplayErrorKind::InitialRunNotStarted,
            })?;
            let SpuEventKind::Parked { pc: actual_pc, reason: actual_reason } = actual
            else {
                return Err(TraceReplayError {
                    event_index,
                    kind: TraceReplayErrorKind::UnexpectedSpuState {
                        expected: format!("Parked({reason:?})"),
                        actual,
                    },
                });
            };
            if actual_reason != *reason {
                return Err(TraceReplayError {
                    event_index,
                    kind: TraceReplayErrorKind::ParkReasonMismatch {
                        expected: *reason,
                        actual: actual_reason,
                    },
                });
            }
            if let Some(expected_pc) = *pc {
                if expected_pc != actual_pc {
                    return Err(TraceReplayError {
                        event_index,
                        kind: TraceReplayErrorKind::ParkPcMismatch {
                            expected: expected_pc,
                            actual: actual_pc,
                        },
                    });
                }
            }
            Ok(ReplayOutcome::AssertionPassed)
        }
        TraceEvent::ExpectSpuFinished { stop_code } => {
            let actual = driver.current_event_kind().ok_or(TraceReplayError {
                event_index,
                kind: TraceReplayErrorKind::InitialRunNotStarted,
            })?;
            match actual {
                SpuEventKind::Finished { stop_code: actual_sc } if actual_sc == *stop_code => {
                    Ok(ReplayOutcome::AssertionPassed)
                }
                other => Err(TraceReplayError {
                    event_index,
                    kind: TraceReplayErrorKind::UnexpectedSpuState {
                        expected: format!("Finished({stop_code:#x})"),
                        actual: other,
                    },
                }),
            }
        }
        TraceEvent::ExpectGprWord { reg, lane, value } => {
            let snap = driver.current_snapshot().ok_or(TraceReplayError {
                event_index,
                kind: TraceReplayErrorKind::InitialRunNotStarted,
            })?;
            if *lane >= 4 {
                return Err(TraceReplayError {
                    event_index,
                    kind: TraceReplayErrorKind::InvalidGprLane { reg: *reg, lane: *lane },
                });
            }
            let lane_val = match *lane {
                0 => (snap.gpr[*reg] >> 96) as u32,
                1 => (snap.gpr[*reg] >> 64) as u32,
                2 => (snap.gpr[*reg] >> 32) as u32,
                3 => snap.gpr[*reg] as u32,
                _ => unreachable!("lane bounds checked above"),
            };
            if lane_val != *value {
                return Err(TraceReplayError {
                    event_index,
                    kind: TraceReplayErrorKind::GprMismatch {
                        reg: *reg,
                        lane: *lane,
                        expected: *value,
                        got: lane_val,
                    },
                });
            }
            Ok(ReplayOutcome::AssertionPassed)
        }
        TraceEvent::ExpectChannelState {
            in_mbox,
            out_mbox,
            out_intr_mbox,
            snr1,
            snr2,
        } => {
            let snap = driver.current_snapshot().ok_or(TraceReplayError {
                event_index,
                kind: TraceReplayErrorKind::InitialRunNotStarted,
            })?;
            let mut detail = String::new();
            use std::fmt::Write;
            if snap.channels.in_mbox != *in_mbox {
                let _ = write!(detail, "in_mbox: expected={in_mbox:?}, got={:?}; ",
                    snap.channels.in_mbox);
            }
            if snap.channels.out_mbox != *out_mbox {
                let _ = write!(detail, "out_mbox: expected={out_mbox:?}, got={:?}; ",
                    snap.channels.out_mbox);
            }
            if snap.channels.out_intr_mbox != *out_intr_mbox {
                let _ = write!(detail, "out_intr_mbox: expected={out_intr_mbox:?}, got={:?}; ",
                    snap.channels.out_intr_mbox);
            }
            if snap.channels.snr[0] != *snr1 {
                let _ = write!(detail, "snr1: expected={snr1}, got={}; ",
                    snap.channels.snr[0]);
            }
            if snap.channels.snr[1] != *snr2 {
                let _ = write!(detail, "snr2: expected={snr2}, got={}; ",
                    snap.channels.snr[1]);
            }
            if !detail.is_empty() {
                return Err(TraceReplayError {
                    event_index,
                    kind: TraceReplayErrorKind::ChannelStateMismatch { detail },
                });
            }
            Ok(ReplayOutcome::AssertionPassed)
        }
        TraceEvent::PpuPushInMbox { value, expect_wake } => {
            apply_with_wake_check(
                driver,
                event_index,
                PpuAction::PushInMbox(*value),
                Some(*expect_wake),
            )
        }
        TraceEvent::PpuPopOutMbox { expect, expect_wake } => {
            apply_with_wake_check(
                driver,
                event_index,
                PpuAction::PopOutMbox { expect: *expect },
                *expect_wake,
            )
        }
        TraceEvent::PpuSignal { slot, value, expect_wake } => {
            apply_with_wake_check(
                driver,
                event_index,
                PpuAction::Signal { slot: *slot, value: *value },
                Some(*expect_wake),
            )
        }
    }
}

fn apply_with_wake_check<E: SpuExecutor>(
    driver: &mut SpuPpuLockstepDriver<'_, E>,
    event_index: usize,
    action: PpuAction,
    expect_wake: Option<SpuWakeResultKind>,
) -> Result<ReplayOutcome, TraceReplayError> {
    let outcome = driver.apply(action).map_err(|e| TraceReplayError {
        event_index,
        kind: lockstep_error_to_replay_kind(e),
    })?;
    let (actual_wake, popped) = match outcome {
        PpuOutcome::WakeTried { wake } => (wake, None),
        PpuOutcome::Drained { wake, popped } => (wake, popped),
        PpuOutcome::Asserted => unreachable!(
            "apply() with side-effect action cannot return Asserted"
        ),
    };
    if let Some(expected) = expect_wake {
        if !expected.matches(actual_wake) {
            return Err(TraceReplayError {
                event_index,
                kind: TraceReplayErrorKind::WakeKindMismatch {
                    expected,
                    actual: actual_wake,
                },
            });
        }
    }
    Ok(ReplayOutcome::PpuActionApplied { actual_wake, popped })
}

fn lockstep_error_to_replay_kind(e: LockstepError) -> TraceReplayErrorKind {
    match e {
        LockstepError::OutMboxMismatch { expected, got } => {
            TraceReplayErrorKind::OutMboxValueMismatch { expected, got }
        }
        LockstepError::ExpectedParkGot { expected_reason, actual } => {
            TraceReplayErrorKind::UnexpectedSpuState {
                expected: format!("Parked({expected_reason:?})"),
                actual,
            }
        }
        LockstepError::ExpectedFinishedGot { expected_stop_code, actual } => {
            TraceReplayErrorKind::UnexpectedSpuState {
                expected: format!("Finished({expected_stop_code:#x})"),
                actual,
            }
        }
        LockstepError::SpuExecError { message, .. } => {
            TraceReplayErrorKind::SpuExecError { message }
        }
        LockstepError::DriverNotStarted => TraceReplayErrorKind::InitialRunNotStarted,
    }
}

// =====================================================================
// R5.6 — Synthetic homebrew-like SPU command-protocol fixture
// =====================================================================
//
// First realistic-shaped PPU↔SPU mailbox protocol expressed as a
// reusable program builder + R5.5 trace literal. The program is NOT a
// real PS3 homebrew dump — it is a synthetic program that mirrors the
// shape of typical SPU command-dispatch loops (read command, dispatch,
// compute, write result, loop until halt sentinel). Used by both
// `rpcs3-spu-differential` (interpreter backend) and
// `rpcs3-spu-recompiler` (JIT backend) tests to validate the full
// R5.5 trace replay engine on a realistic interaction shape.

/// Stable label used in trace summaries / failure reports so multi-
/// trace logs can attribute output to this fixture.
pub const FIXTURE_NAME_MAILBOX_PROTOCOL: &str = "synthetic_mailbox_command_protocol";

/// Build the synthetic mailbox-command-protocol SPU program at
/// entry_pc 0x100 with `max_steps = 200`. Layout:
///
/// ```text
/// 0x100  rdch r3, IN_MBOX(29)    ; read command
/// 0x104  il   r4, 0xFF           ; halt sentinel (sign-extended to 0x000000FF)
/// 0x108  ceq  r5, r3, r4         ; r5 = (r3 == 0xFF) ? all-ones : 0
/// 0x10C  brnz r5, +4 (HALT)      ; if r3 == 0xFF, branch to 0x11C
/// 0x110  ai   r6, r3, 0x29       ; result = command + 0x29
/// 0x114  wrch r6, OUT_MBOX(28)   ; send result (parks if backpressure)
/// 0x118  br   -6 (LOOP)          ; back to 0x100
/// 0x11C  stop 0xD5               ; halt with code 0xD5
/// ```
///
/// **Determinism:** the program byte string is a pure function of
/// the encoder constants. Re-calling this builder produces identical
/// segments — verified by the `r5_6_fixture_is_reproducible` test.
#[must_use]
pub fn mailbox_command_protocol_program() -> SpuProgram {
    let rdch = (0x00Du32 << 21) | ((29u32 & 0x7F) << 7) | 3;
    let il   = ((0x081u32 & 0x1FF) << 23) | ((0xFFu32 & 0xFFFF) << 7) | 4;
    let ceq  = (0x3C0u32 << 21) | ((4u32 & 0x7F) << 14) | ((3u32 & 0x7F) << 7) | 5;
    let brnz = ((0x042u32 & 0x1FF) << 23) | ((4i16 as u16 as u32) << 7) | 5;
    let ai   = ((0x1Cu32 & 0xFF) << 24) | ((0x29u32 & 0x3FF) << 14)
             | ((3u32 & 0x7F) << 7) | 6;
    let wrch = (0x10Du32 << 21) | ((28u32 & 0x7F) << 7) | 6;
    let br_back = ((0x064u32 & 0x1FF) << 23) | (((-6i16) as u16 as u32) << 7);
    let stop = 0xD5u32 & 0x3FFF;

    let mut bytes = Vec::with_capacity(32);
    bytes.extend_from_slice(&rdch.to_be_bytes());
    bytes.extend_from_slice(&il.to_be_bytes());
    bytes.extend_from_slice(&ceq.to_be_bytes());
    bytes.extend_from_slice(&brnz.to_be_bytes());
    bytes.extend_from_slice(&ai.to_be_bytes());
    bytes.extend_from_slice(&wrch.to_be_bytes());
    bytes.extend_from_slice(&br_back.to_be_bytes());
    bytes.extend_from_slice(&stop.to_be_bytes());
    SpuProgram::new(0x100, 200).with_segment(0x100, bytes)
}

/// Build the canonical R5.5 trace for the mailbox-command-protocol
/// fixture. Drives:
///
/// - First park on rdch (in_mbox empty).
/// - Command 1 (`1` → result `0x2A`): clean cycle — push wakes rdch,
///   SPU computes and wrch's, loops, parks rdch again.
/// - Command 2 (`2` → result `0x2B`): exercises wrch backpressure —
///   SPU's wrch parks because out_mbox still holds 0x2A from cmd 1.
///   PPU drains 0x2A; that wake satisfies the wrch park; SPU resumes,
///   writes 0x2B, loops, parks rdch.
/// - Command 0xFF (halt sentinel): ceq matches → brnz branches to
///   HALT → stop 0xD5.
/// - Final cleanup pop drains the residual 0x2B.
///
/// Final asserts verify GPRs and channel state. Total: 16 events.
#[must_use]
pub fn mailbox_command_protocol_trace() -> Vec<TraceEvent> {
    use SpuParkReason::*;
    use SpuWakeResultKind::*;
    vec![
        // [0] Initial park on rdch waiting for first command.
        TraceEvent::ExpectSpuPark {
            reason: ChannelRead { channel: 29 },
            pc: Some(0x100),
        },
        // [1] PPU sends command 1 — wakes the rdch.
        TraceEvent::PpuPushInMbox { value: 1, expect_wake: Ready },
        // [2] After cmd 1 path, SPU loops back to rdch and parks again.
        TraceEvent::ExpectSpuPark {
            reason: ChannelRead { channel: 29 },
            pc: Some(0x100),
        },
        // [3] out_mbox holds 0x2A (1 + 0x29).
        TraceEvent::ExpectChannelState {
            in_mbox: None,
            out_mbox: Some(0x2A),
            out_intr_mbox: None,
            snr1: 0,
            snr2: 0,
        },
        // [4] PPU sends command 2 — triggers backpressure path.
        TraceEvent::PpuPushInMbox { value: 2, expect_wake: Ready },
        // [5] SPU parks at the wrch (0x114) because out_mbox is full.
        TraceEvent::ExpectSpuPark {
            reason: ChannelWrite { channel: 28 },
            pc: Some(0x114),
        },
        // [6] Stalled wrch did NOT mutate out_mbox (R5.4a invariant).
        TraceEvent::ExpectChannelState {
            in_mbox: None,
            out_mbox: Some(0x2A),
            out_intr_mbox: None,
            snr1: 0,
            snr2: 0,
        },
        // [7] PPU drains 0x2A — wake satisfies the wrch park; SPU
        // resumes, writes 0x2B, loops, parks rdch.
        TraceEvent::PpuPopOutMbox {
            expect: Some(0x2A),
            expect_wake: Some(Ready),
        },
        // [8] Now parked at rdch with out_mbox = 0x2B.
        TraceEvent::ExpectSpuPark {
            reason: ChannelRead { channel: 29 },
            pc: Some(0x100),
        },
        // [9] State sanity: out_mbox = 0x2B, in_mbox empty.
        TraceEvent::ExpectChannelState {
            in_mbox: None,
            out_mbox: Some(0x2B),
            out_intr_mbox: None,
            snr1: 0,
            snr2: 0,
        },
        // [10] PPU sends halt sentinel 0xFF.
        TraceEvent::PpuPushInMbox { value: 0xFF, expect_wake: Ready },
        // [11] SPU consumes 0xFF, ceq matches, brnz to HALT, stops.
        TraceEvent::ExpectSpuFinished { stop_code: 0xD5 },
        // [12] Final cleanup: PPU pops residual 0x2B (no wake — done).
        TraceEvent::PpuPopOutMbox {
            expect: Some(0x2B),
            expect_wake: Some(NotParked),
        },
        // [13] All channels drained.
        TraceEvent::ExpectChannelState {
            in_mbox: None,
            out_mbox: None,
            out_intr_mbox: None,
            snr1: 0,
            snr2: 0,
        },
        // [14] Last command consumed lives in r3 lane 0.
        TraceEvent::ExpectGprWord { reg: 3, lane: 0, value: 0xFF },
        // [15] Last computed result (cmd 2 + 0x29 = 0x2B) lives in r6.
        TraceEvent::ExpectGprWord { reg: 6, lane: 0, value: 0x2B },
    ]
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode `il rt, imm16; stop 0` as a code segment.
    fn il_stop_program(rt: u32, imm: i16) -> SpuProgram {
        let il = ((0x081u32 & 0x1FF) << 23) | ((imm as u16 as u32 & 0xFFFF) << 7) | (rt & 0x7F);
        let stop = 0u32; // primary 0x000, code 0
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&il.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn interpreter_runs_il_stop_program() {
        let mut exec = InterpreterExecutor::default();
        let result = exec.execute(&il_stop_program(3, 0x1234));
        assert_eq!(result.steps_executed, 2);
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0));
        // r3 should be 0x1234 broadcast across all four 32-bit lanes.
        assert_eq!(result.final_state.gpr[3], 0x00001234_00001234_00001234_00001234);
        assert_eq!(result.final_state.pc, 0x104);
    }

    #[test]
    fn identical_runs_diff_to_empty() {
        let prog = il_stop_program(5, 42);
        let mut exec = InterpreterExecutor::default();
        let a = exec.execute(&prog);
        let b = exec.execute(&prog);
        let d = diff_snapshots(&a.final_state, &b.final_state);
        assert!(d.is_identical(), "diff: {d:?}");
        assert!(d.gpr_mismatches.is_empty());
        assert_eq!(d.ls_total_diff_bytes, 0);
    }

    #[test]
    fn divergent_register_values_show_up_in_diff() {
        let a = il_stop_program(3, 0x1111);
        let b = il_stop_program(3, 0x2222);
        let mut exec = InterpreterExecutor::default();
        let ra = exec.execute(&a);
        let rb = exec.execute(&b);
        let d = diff_snapshots(&ra.final_state, &rb.final_state);
        assert!(!d.is_identical());
        assert_eq!(d.gpr_mismatches.len(), 1);
        let (reg, _, _) = d.gpr_mismatches[0];
        assert_eq!(reg, 3);
    }

    #[test]
    fn run_and_diff_pairs_two_backends() {
        let prog = il_stop_program(1, 0xABCD_u16 as i16);
        let mut a = InterpreterExecutor::default();
        let mut b = InterpreterExecutor::default();
        let (ra, rb, d) = run_and_diff(&mut a, &mut b, &prog);
        assert_eq!(ra.steps_executed, rb.steps_executed);
        assert_eq!(ra.stop_reason, rb.stop_reason);
        assert!(d.is_identical());
    }

    #[test]
    fn validate_rejects_segment_past_ls() {
        let prog = SpuProgram::new(0x0, 10)
            .with_segment(SPU_LS_SIZE as u32 - 4, vec![0u8; 8]);
        match prog.validate() {
            Err(ProgramError::SegmentOutOfRange { lsa, .. }) => {
                assert_eq!(lsa, SPU_LS_SIZE as u32 - 4);
            }
            other => panic!("expected SegmentOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_unaligned_entry_pc() {
        let prog = SpuProgram::new(0x101, 10);
        match prog.validate() {
            Err(ProgramError::BadEntryPc(0x101)) => {}
            other => panic!("expected BadEntryPc, got {other:?}"),
        }
    }

    #[test]
    fn validate_rejects_entry_pc_out_of_range() {
        let prog = SpuProgram::new(SPU_LS_SIZE as u32, 10);
        match prog.validate() {
            Err(ProgramError::BadEntryPc(_)) => {}
            other => panic!("expected BadEntryPc, got {other:?}"),
        }
    }

    #[test]
    fn invalid_program_surfaces_as_error_result() {
        let prog = SpuProgram::new(0x101, 10); // unaligned entry
        let mut exec = InterpreterExecutor::default();
        let r = exec.execute(&prog);
        match r.stop_reason {
            ExecutionStopReason::Error(msg) => assert!(msg.contains("entry pc")),
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn max_steps_zero_returns_max_steps_exceeded() {
        // il + stop is 2 instructions; max_steps=0 cannot run any.
        let mut prog = il_stop_program(3, 0);
        prog.max_steps = 0;
        let mut exec = InterpreterExecutor::default();
        let r = exec.execute(&prog);
        assert_eq!(r.stop_reason, ExecutionStopReason::MaxStepsExceeded);
        assert_eq!(r.steps_executed, 0);
    }

    #[test]
    fn snapshot_includes_full_256kb_ls() {
        let mut exec = InterpreterExecutor::default();
        let r = exec.execute(&il_stop_program(0, 0));
        assert_eq!(r.final_state.ls.len(), SPU_LS_SIZE);
    }

    #[test]
    fn ls_diff_reports_first_n_bytes_only() {
        // Build two identical-shape snapshots that differ in 100 bytes;
        // diff should report exactly the first 64 (default cap).
        let mut sa = SpuStateSnapshot {
            pc: 0,
            gpr: Box::new([0u128; SPU_GPR_COUNT]),
            ls: Box::new([0u8; SPU_LS_SIZE]),
            channel_counts: ChannelCounts::default(),
            park_state: None,
            channels: SpuChannels::default(),
        };
        let mut sb = SpuStateSnapshot {
            pc: 0,
            gpr: Box::new([0u128; SPU_GPR_COUNT]),
            ls: Box::new([0u8; SPU_LS_SIZE]),
            channel_counts: ChannelCounts::default(),
            park_state: None,
            channels: SpuChannels::default(),
        };
        for i in 0..100 {
            sa.ls[i] = 0xAA;
            sb.ls[i] = 0xBB;
        }
        let d = diff_snapshots(&sa, &sb);
        assert_eq!(d.ls_total_diff_bytes, 100);
        assert_eq!(d.ls_mismatches.len(), 64);
        // First reported byte is offset 0.
        assert_eq!(d.ls_mismatches[0], (0, 0xAA, 0xBB));
    }

    #[test]
    fn backend_name_returned_for_interpreter() {
        let exec = InterpreterExecutor::default();
        assert_eq!(exec.backend_name(), "interpreter");
    }

    /// `il r3, 0x1234; wrch r3, SPU_WROUTMBOX (28); stop 0` —
    /// program writes to outbox then halts. After execution the
    /// snapshot's `out_mbox_depth` should be 1.
    fn il_wrch_stop_program() -> SpuProgram {
        // SPU_WROUTMBOX is channel 28 per RPCS3.
        const CH_OUTMBOX: u32 = 28;
        let il = ((0x081u32 & 0x1FF) << 23) | ((0x1234u32 & 0xFFFF) << 7) | 3;
        // wrch ch, rt: 11-bit primary 0x10D, ra-slot holds channel
        // (we use ra(inst) for the channel index in the interpreter).
        let wrch = ((0x10Du32 & 0x7FF) << 21) | ((CH_OUTMBOX & 0x7F) << 7) | 3;
        let stop = 0u32;
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&il.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn channel_counts_reflect_outbox_write() {
        let prog = il_wrch_stop_program();
        let mut exec = InterpreterExecutor::default();
        let r = exec.execute(&prog);

        // Program halted cleanly (or stalled — both acceptable depending
        // on the channel impl. Check execution actually advanced.)
        assert!(r.steps_executed >= 2);
        // Outbox should now have the wrch'd value pending.
        assert_eq!(
            r.final_state.channel_counts.out_mbox_depth, 1,
            "expected outbox depth 1 after wrch, got {:?}", r.final_state.channel_counts
        );
        // No other channel was touched.
        assert_eq!(r.final_state.channel_counts.in_mbox_depth, 0);
        assert_eq!(r.final_state.channel_counts.out_intr_mbox_depth, 0);
    }

    #[test]
    fn channel_counts_diff_when_one_run_writes_outbox() {
        let with_wrch = il_wrch_stop_program();
        // A version that doesn't write the outbox at all.
        let il_only_stop = {
            let il = ((0x081u32 & 0x1FF) << 23) | ((0x1234u32 & 0xFFFF) << 7) | 3;
            let stop = 0u32;
            let mut bytes = Vec::with_capacity(8);
            bytes.extend_from_slice(&il.to_be_bytes());
            bytes.extend_from_slice(&stop.to_be_bytes());
            SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
        };

        let mut exec = InterpreterExecutor::default();
        let r_with = exec.execute(&with_wrch);
        let r_without = exec.execute(&il_only_stop);

        let d = diff_snapshots(&r_with.final_state, &r_without.final_state);
        // The two runs differ in: PC (different program length),
        // outbox depth (1 vs 0), so diff must surface non-identity.
        assert!(!d.is_identical(), "expected divergence, got identity");
        assert!(!d.channel_counts_match,
                "expected channel_counts mismatch when one run writes outbox");
    }

    // =====================================================================
    // R5.4c — SpuSingleThreadExecutor tests (interpreter backend)
    //
    // Three synthetic blocking fixtures (rdch INMBOX, wrch OUTMBOX,
    // rdch SIGNOTIFY1) plus invariants:
    //   - Park event carries the channel-op PC (not pc+4).
    //   - StillBlocked wake → resume re-stalls at same PC (no advance).
    //   - BadChannel surfaces as Error, NOT Parked.
    //   - Channels survive park → wake → resume byte-exact.
    // =====================================================================

    fn rdch_inmbox_block_then_resume_program() -> SpuProgram {
        // 0x100 rdch r3, RDINMBOX(29)
        // 0x104 ai   r4, r3, 1       (primary 0x1C, ri10 form)
        // 0x108 stop 0xA1
        let rdch = (0x00Du32 << 21) | ((29 & 0x7F) << 7) | 3;
        let ai   = ((0x1Cu32 & 0xFF) << 24) | ((1u32 & 0x3FF) << 14) | ((3 & 0x7F) << 7) | 4;
        let stop = 0xA1u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&ai.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    fn wrch_outmbox_full_then_resume_program() -> SpuProgram {
        // 0x100 il   r3, 0xCAFE
        // 0x104 wrch r3, WROUTMBOX(28)
        // 0x108 stop 0xB2
        let il   = ((0x081u32 & 0x1FF) << 23) | ((0xCAFEu32 & 0xFFFF) << 7) | 3;
        let wrch = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 3;
        let stop = 0xB2u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&il.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    /// End-to-end: rdch on empty INMBOX → executor reports Parked →
    /// caller pushes via wake helper → resume → Finished(0xA1).
    #[test]
    fn executor_rdch_inmbox_park_wake_resume_end_to_end() {
        use rpcs3_spu_thread::SpuWakeResult;

        let prog = rdch_inmbox_block_then_resume_program();
        let mut backend = InterpreterExecutor::default();
        let mut exec = SpuSingleThreadExecutor::new();

        // ---- 1. First run parks at the rdch ---------------------
        let ev1 = exec.run_until_event(&mut backend, &prog);
        let (parked_pc, parked_reason, parked_snapshot, parked_steps) = match ev1 {
            SpuExecEvent::Parked { pc, reason, snapshot, steps } => {
                (pc, reason, snapshot, steps)
            }
            other => panic!("expected Parked, got {other:?}"),
        };
        assert_eq!(parked_pc, 0x100, "park PC must be the rdch's pc, not pc+4");
        assert_eq!(parked_reason, SpuParkReason::ChannelRead { channel: 29 });
        assert!(parked_snapshot.park_state.is_some());
        assert!(parked_snapshot.channels.in_mbox.is_none());

        // ---- 2. Caller does PPU-side wake on a SpuThread shadow -
        let mut shadow = SpuThread::new(0);
        shadow.channels = parked_snapshot.channels.clone();
        shadow.park_state = parked_snapshot.park_state;
        let wake_pc = match shadow.ppu_push_inmbox_and_try_wake(0x29) {
            SpuWakeResult::Ready { pc } => pc,
            other => panic!("expected Ready, got {other:?}"),
        };
        assert_eq!(wake_pc, parked_pc, "wake PC must equal parked PC");
        let wake_channels = shadow.channels.clone();

        // ---- 3. Resume runs from wake_pc, reaches stop ----------
        let ev2 = exec.resume_after_wake(
            &parked_snapshot,
            &wake_channels,
            wake_pc,
            &prog,
            parked_steps,
        );
        let final_snapshot = match ev2 {
            SpuExecEvent::Finished { stop_code, snapshot, steps } => {
                assert_eq!(stop_code, 0xA1);
                assert!(steps > parked_steps,
                        "resume must add steps on top of prior_steps");
                snapshot
            }
            other => panic!("expected Finished, got {other:?}"),
        };

        // r3 holds 0x29 (consumed from in_mbox into preferred slot)
        assert_eq!(final_snapshot.gpr[3] >> 96, 0x29u128);
        // r4 = r3 + 1 = 0x2A in the preferred slot
        assert_eq!(final_snapshot.gpr[4] >> 96, 0x2Au128);
        // park is cleared after Ready resume
        assert_eq!(final_snapshot.park_state, None);
        // mailbox drained
        assert_eq!(final_snapshot.channels.in_mbox, None);
    }

    /// End-to-end: wrch on full OUTMBOX → executor parks → caller drains
    /// out_mbox → wake → resume → Finished(0xB2) with new value in mbox.
    ///
    /// Program lays down a prelude wrch that fills the mbox, then a
    /// second wrch that must stall:
    ///   0x100 il   r3, 0x1111       (pre-fill value)
    ///   0x104 wrch r3, OUTMBOX(28)  (success — out_mbox = 0x1111)
    ///   0x108 il   r3, 0xCAFE       (overwrite r3)
    ///   0x10C wrch r3, OUTMBOX(28)  (STALL — mbox full)
    ///   0x110 stop 0xB2
    #[test]
    fn executor_wrch_outmbox_park_wake_resume_end_to_end() {
        use rpcs3_spu_thread::SpuWakeResult;

        // Reference the helper so it stays exercised by `cargo check`.
        let _ = wrch_outmbox_full_then_resume_program();

        let il_a = ((0x081u32 & 0x1FF) << 23) | ((0x1111u32 & 0xFFFF) << 7) | 3;
        let wrch = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 3;
        let il_b = ((0x081u32 & 0x1FF) << 23) | ((0xCAFEu32 & 0xFFFF) << 7) | 3;
        let stop = 0xB2u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(20);
        bytes.extend_from_slice(&il_a.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&il_b.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let mut exec = SpuSingleThreadExecutor::new();

        // ---- 1. First run parks at the second wrch --------------
        let ev1 = exec.run_until_event(&mut backend, &prog);
        let (parked_pc, parked_reason, parked_snapshot, parked_steps) = match ev1 {
            SpuExecEvent::Parked { pc, reason, snapshot, steps } => {
                (pc, reason, snapshot, steps)
            }
            other => panic!("expected Parked, got {other:?}"),
        };
        assert_eq!(parked_pc, 0x10C, "park PC must be the second wrch (0x10C)");
        assert_eq!(parked_reason, SpuParkReason::ChannelWrite { channel: 28 });
        assert_eq!(parked_snapshot.channels.out_mbox, Some(0x1111),
                   "out_mbox holds the first wrch's value at stall");

        // ---- 2. PPU drains via wake helper ----------------------
        let mut shadow = SpuThread::new(0);
        shadow.channels = parked_snapshot.channels.clone();
        shadow.park_state = parked_snapshot.park_state;
        let (drained, wake_result) = shadow.ppu_pop_outmbox_and_try_wake();
        assert_eq!(drained, Some(0x1111));
        let wake_pc = match wake_result {
            SpuWakeResult::Ready { pc } => pc,
            other => panic!("expected Ready, got {other:?}"),
        };

        // ---- 3. Resume runs the second wrch, reaches stop -------
        let ev2 = exec.resume_after_wake(
            &parked_snapshot,
            &shadow.channels,
            wake_pc,
            &prog,
            parked_steps,
        );
        let final_snapshot = match ev2 {
            SpuExecEvent::Finished { stop_code, snapshot, .. } => {
                assert_eq!(stop_code, 0xB2);
                snapshot
            }
            other => panic!("expected Finished, got {other:?}"),
        };
        // `il` sign-extends the 16-bit immediate. 0xCAFE has bit 15
        // set, so the broadcast value is 0xFFFFCAFE.
        assert_eq!(final_snapshot.channels.out_mbox, Some(0xFFFFCAFE),
                   "after resume, out_mbox holds the second wrch's sign-extended value");
        assert_eq!(final_snapshot.park_state, None);
    }

    /// Wake with unsatisfied condition → caller would NOT call
    /// resume_after_wake (StillBlocked is observable on the helper).
    /// If the caller incorrectly resumes anyway with the unchanged
    /// channels, the executor must re-stall at the same PC (no fake
    /// success, no PC advance).
    #[test]
    fn executor_wake_still_blocked_does_not_resume() {
        use rpcs3_spu_thread::SpuWakeResult;

        let prog = rdch_inmbox_block_then_resume_program();
        let mut backend = InterpreterExecutor::default();
        let mut exec = SpuSingleThreadExecutor::new();

        let ev1 = exec.run_until_event(&mut backend, &prog);
        let (parked_pc, parked_snapshot, parked_steps) = match ev1 {
            SpuExecEvent::Parked { pc, snapshot, steps, .. } => (pc, snapshot, steps),
            other => panic!("expected Parked, got {other:?}"),
        };

        // Wrong wake: signal slot 0 while parked on RDINMBOX.
        let mut shadow = SpuThread::new(0);
        shadow.channels = parked_snapshot.channels.clone();
        shadow.park_state = parked_snapshot.park_state;
        let wake = shadow.signal_and_try_wake(0, 0xFF);
        assert_eq!(wake, SpuWakeResult::StillBlocked);

        // Resume anyway — interpreter must re-stall on the same rdch
        // because in_mbox is still empty.
        let ev2 = exec.resume_after_wake(
            &parked_snapshot,
            &shadow.channels,
            parked_pc,
            &prog,
            parked_steps,
        );
        match ev2 {
            SpuExecEvent::Parked { pc, reason, .. } => {
                assert_eq!(pc, parked_pc, "re-stall must keep park PC stable");
                assert_eq!(reason, SpuParkReason::ChannelRead { channel: 29 });
            }
            other => panic!("expected re-Parked, got {other:?}"),
        }
    }

    /// Park PC must be the channel op's PC, not pc+4 — re-execution
    /// from that PC must observe the same op a second time.
    #[test]
    fn executor_resume_uses_parked_pc_not_pc_plus_4() {
        use rpcs3_spu_thread::SpuWakeResult;

        let prog = rdch_inmbox_block_then_resume_program();
        let mut backend = InterpreterExecutor::default();
        let mut exec = SpuSingleThreadExecutor::new();

        let ev = exec.run_until_event(&mut backend, &prog);
        let (parked_pc, parked_snapshot, parked_steps) = match ev {
            SpuExecEvent::Parked { pc, snapshot, steps, .. } => (pc, snapshot, steps),
            other => panic!("expected Parked, got {other:?}"),
        };
        assert_eq!(parked_pc, 0x100, "park PC == rdch PC, not 0x104");

        let mut shadow = SpuThread::new(0);
        shadow.channels = parked_snapshot.channels.clone();
        shadow.park_state = parked_snapshot.park_state;
        let wake_pc = match shadow.ppu_push_inmbox_and_try_wake(0x55) {
            SpuWakeResult::Ready { pc } => pc,
            other => panic!("expected Ready, got {other:?}"),
        };
        assert_eq!(wake_pc, 0x100,
                   "Ready pc must be the rdch's PC, ready for re-execution");

        let ev2 = exec.resume_after_wake(
            &parked_snapshot,
            &shadow.channels,
            wake_pc,
            &prog,
            parked_steps,
        );
        match ev2 {
            SpuExecEvent::Finished { snapshot, stop_code, .. } => {
                assert_eq!(stop_code, 0xA1);
                // Final PC is 0x108 (the stop's address).
                assert_eq!(snapshot.pc, 0x108,
                           "rdch must have advanced past itself; stop holds pc");
                // r3 holds the wake's payload — proves rdch re-executed.
                assert_eq!(snapshot.gpr[3] >> 96, 0x55u128);
            }
            other => panic!("expected Finished, got {other:?}"),
        }
    }

    /// Channels survive park → wake → resume byte-exact. Specifically:
    /// out_mbox / in_mbox / snr / event_mask state at park is preserved
    /// into the resume snapshot (modulo what the resumed instructions
    /// themselves change).
    #[test]
    fn executor_preserves_channels_across_park_and_resume() {
        use rpcs3_spu_thread::SpuWakeResult;

        let prog = rdch_inmbox_block_then_resume_program();
        let mut backend = InterpreterExecutor::default();
        let mut exec = SpuSingleThreadExecutor::new();

        let ev = exec.run_until_event(&mut backend, &prog);
        let (_parked_pc, parked_snapshot, parked_steps) = match ev {
            SpuExecEvent::Parked { pc, snapshot, steps, .. } => (pc, snapshot, steps),
            other => panic!("expected Parked, got {other:?}"),
        };

        // Snapshot exposed at park: in_mbox empty, out_mbox empty,
        // snr both 0 (untouched by the program prelude).
        assert!(parked_snapshot.channels.in_mbox.is_none());
        assert!(parked_snapshot.channels.out_mbox.is_none());
        assert_eq!(parked_snapshot.channels.snr, [0, 0]);
        assert_eq!(parked_snapshot.channels.event_mask, 0);

        let mut shadow = SpuThread::new(0);
        shadow.channels = parked_snapshot.channels.clone();
        shadow.park_state = parked_snapshot.park_state;
        let wake_pc = match shadow.ppu_push_inmbox_and_try_wake(0x77) {
            SpuWakeResult::Ready { pc } => pc,
            other => panic!("expected Ready, got {other:?}"),
        };

        let ev2 = exec.resume_after_wake(
            &parked_snapshot,
            &shadow.channels,
            wake_pc,
            &prog,
            parked_steps,
        );
        let final_snapshot = match ev2 {
            SpuExecEvent::Finished { snapshot, .. } => snapshot,
            other => panic!("expected Finished, got {other:?}"),
        };

        // Channels NOT touched by the resumed instructions must match.
        assert_eq!(final_snapshot.channels.out_mbox, parked_snapshot.channels.out_mbox);
        assert_eq!(final_snapshot.channels.snr, parked_snapshot.channels.snr);
        assert_eq!(final_snapshot.channels.event_mask,
                   parked_snapshot.channels.event_mask);
        // Channel touched by rdch (in_mbox) is now drained.
        assert_eq!(final_snapshot.channels.in_mbox, None);
    }

    /// `rdch` with channel = 100 (no decoder for that channel) is a
    /// BadChannel error, NOT a parking condition. Executor must
    /// surface `Error`, not `Parked`.
    #[test]
    fn executor_bad_channel_reports_error_not_parked() {
        let rdch = (0x00Du32 << 21) | ((100 & 0x7F) << 7) | 3;
        let stop = 0xD7u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let mut exec = SpuSingleThreadExecutor::new();
        let ev = exec.run_until_event(&mut backend, &prog);
        match ev {
            SpuExecEvent::Error { snapshot, .. } => {
                assert!(snapshot.park_state.is_none(),
                        "BadChannel must NOT park");
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    /// Sanity: the simple il+stop program produces Finished, never
    /// Parked. Confirms classify() doesn't accidentally route clean
    /// runs through the park path.
    #[test]
    fn executor_finished_event_for_simple_stop() {
        let prog = il_stop_program(7, 0x55);
        let mut backend = InterpreterExecutor::default();
        let mut exec = SpuSingleThreadExecutor::new();
        match exec.run_until_event(&mut backend, &prog) {
            SpuExecEvent::Finished { stop_code, snapshot, .. } => {
                assert_eq!(stop_code, 0);
                assert_eq!(snapshot.park_state, None);
            }
            other => panic!("expected Finished, got {other:?}"),
        }
    }

    /// SpuStateSnapshot constructor surfaces both R5.4a `park_state`
    /// and R5.4b `channels` so callers can drive the cycle. Sanity
    /// for the snapshot shape.
    #[test]
    fn snapshot_carries_park_state_and_channels() {
        let snap = SpuStateSnapshot {
            pc: 0x200,
            gpr: Box::new([0u128; SPU_GPR_COUNT]),
            ls: Box::new([0u8; SPU_LS_SIZE]),
            channel_counts: ChannelCounts::default(),
            park_state: Some(SpuParkState {
                pc: 0x200,
                reason: SpuParkReason::ChannelRead { channel: 29 },
            }),
            channels: SpuChannels::default(),
        };
        // park_state and channels are reachable via the public surface.
        assert_eq!(snap.park_state.unwrap().pc, 0x200);
        assert!(snap.channels.in_mbox.is_none());
    }

    // =====================================================================
    // R5.4e — SpuPpuLockstepDriver tests (interpreter backend)
    //
    // Validates the synthetic single-threaded PPU↔SPU driver against
    // four scripted scenarios: rdch INMBOX handshake, wrch OUTMBOX
    // backpressure, ping-pong, and a signotify path that documents
    // why end-to-end via run_until_park is not reachable.
    // =====================================================================

    /// Encode an SPU program: rdch r3, IN(29); ai r4, r3, 1;
    /// wrch r4, OUT(28); stop 0xA1.
    fn rdch_inmbox_handshake_program() -> SpuProgram {
        let rdch = (0x00Du32 << 21) | ((29 & 0x7F) << 7) | 3;
        let ai   = ((0x1Cu32 & 0xFF) << 24) | ((1u32 & 0x3FF) << 14)
                 | ((3 & 0x7F) << 7) | 4;
        let wrch = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 4;
        let stop = 0xA1u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&ai.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    /// rdch INMBOX handshake — full PPU↔SPU script.
    /// SPU parks on rdch, PPU pushes 41 to wake, SPU consumes,
    /// computes 42, writes to out_mbox, stops. PPU pops and asserts.
    #[test]
    fn lockstep_rdch_inmbox_handshake() {
        let mut backend = InterpreterExecutor::default();
        let prog = rdch_inmbox_handshake_program();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let trace = driver
            .run_script(&[
                PpuAction::ExpectPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                },
                PpuAction::PushInMbox(41),
                PpuAction::ExpectFinished { stop_code: 0xA1 },
                PpuAction::PopOutMbox { expect: Some(42) },
            ])
            .expect("script must succeed");

        // Final state.
        assert!(matches!(
            trace.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xA1 }
        ));
        assert_eq!(trace.final_snapshot.park_state, None);
        assert_eq!(trace.final_snapshot.channels.in_mbox, None);
        assert_eq!(trace.final_snapshot.channels.out_mbox, None,
                   "PopOutMbox drained the value");
        // Steps must be monotonic and > 0.
        assert!(trace.total_steps >= 4, "at least rdch, ai, wrch, stop ran");

        // Trace contains exactly one ResumeStarted (after PushInMbox)
        // and 2 SpuEvent records (initial park + post-resume finish).
        let resumes = trace.records.iter()
            .filter(|r| matches!(r, TraceRecord::ResumeStarted { .. }))
            .count();
        let spu_events = trace.records.iter()
            .filter(|r| matches!(r, TraceRecord::SpuEvent { .. }))
            .count();
        assert_eq!(resumes, 1, "exactly one resume after PushInMbox");
        assert_eq!(spu_events, 2, "initial park + post-resume finish");
    }

    /// wrch OUTMBOX backpressure — SPU writes twice to out_mbox; the
    /// second write parks because the first is still pending. PPU
    /// drains, wake fires, second wrch completes.
    #[test]
    fn lockstep_wrch_outmbox_backpressure() {
        // 0x100 il   r3, 0x1111
        // 0x104 wrch r3, OUT(28)    (success, out_mbox = 0x1111)
        // 0x108 il   r3, 0x2222
        // 0x10C wrch r3, OUT(28)    (STALL — mbox full)
        // 0x110 stop 0xB2
        let il_a = ((0x081u32 & 0x1FF) << 23) | ((0x1111u32 & 0xFFFF) << 7) | 3;
        let wrch = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 3;
        let il_b = ((0x081u32 & 0x1FF) << 23) | ((0x2222u32 & 0xFFFF) << 7) | 3;
        let stop = 0xB2u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(20);
        bytes.extend_from_slice(&il_a.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&il_b.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let trace = driver
            .run_script(&[
                PpuAction::ExpectPark {
                    reason: SpuParkReason::ChannelWrite { channel: 28 },
                },
                // Drain the first wrch's value; this also wakes the
                // parked second wrch.
                PpuAction::PopOutMbox { expect: Some(0x1111) },
                PpuAction::ExpectFinished { stop_code: 0xB2 },
                // Drain the second wrch's value — still in out_mbox.
                PpuAction::PopOutMbox { expect: Some(0x2222) },
            ])
            .expect("script must succeed");

        assert!(matches!(
            trace.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xB2 }
        ));
        assert_eq!(trace.final_snapshot.channels.out_mbox, None);

        // Steps must reflect: il, wrch, il (parked here), wrch (resumed),
        // stop.
        assert!(trace.total_steps >= 5);
    }

    /// Bidirectional ping-pong — two complete park/wake cycles. SPU
    /// reads input, writes echo, reads second input, writes second
    /// echo, stops. PPU pushes both inputs and pops both echoes.
    #[test]
    fn lockstep_bidirectional_ping_pong() {
        // 0x100 rdch r3, IN(29)    (parks #1)
        // 0x104 wrch r3, OUT(28)   (writes echo)
        // 0x108 rdch r4, IN(29)    (parks #2)
        // 0x10C wrch r4, OUT(28)   (writes echo)
        // 0x110 stop 0xC3
        let rdch_3 = (0x00Du32 << 21) | ((29 & 0x7F) << 7) | 3;
        let wrch_3 = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 3;
        let rdch_4 = (0x00Du32 << 21) | ((29 & 0x7F) << 7) | 4;
        let wrch_4 = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 4;
        let stop   = 0xC3u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(20);
        bytes.extend_from_slice(&rdch_3.to_be_bytes());
        bytes.extend_from_slice(&wrch_3.to_be_bytes());
        bytes.extend_from_slice(&rdch_4.to_be_bytes());
        bytes.extend_from_slice(&wrch_4.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let trace = driver
            .run_script(&[
                PpuAction::ExpectPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                },
                PpuAction::PushInMbox(0xAA),
                // After push+wake+resume, SPU runs rdch, wrch, then
                // rdch parks again on empty in_mbox.
                PpuAction::ExpectPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                },
                // Drain the first echo before pushing the second
                // input — also pops out_mbox.
                PpuAction::PopOutMbox { expect: Some(0xAA) },
                PpuAction::PushInMbox(0xBB),
                PpuAction::ExpectFinished { stop_code: 0xC3 },
                PpuAction::PopOutMbox { expect: Some(0xBB) },
            ])
            .expect("script must succeed");

        assert!(matches!(
            trace.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xC3 }
        ));
        // Two park cycles → two ResumeStarted records.
        let resumes = trace.records.iter()
            .filter(|r| matches!(r, TraceRecord::ResumeStarted { .. }))
            .count();
        assert_eq!(resumes, 2, "two PushInMbox wakes should fire two resumes");
    }

    /// R5.11: signotify reads NOW park naturally when `snr` is empty,
    /// matching Cell BE semantics. Program of
    /// `rdch r5, RDSIGNOTIFY1; stop 0xC3` parks on the rdch; a
    /// `Signal { slot: 0, value: 0xA5A5 }` action wakes the SPU,
    /// the read returns 0xA5A5, and the SPU runs to Finished.
    /// Validates the natural park-→-Signal-→-resume path
    /// end-to-end through the lockstep driver — same path the
    /// `single_spu_signal_v1` fixture exercises against a real
    /// captured trace.
    #[test]
    fn lockstep_signotify_parks_naturally_and_signal_wakes() {
        let rdch = (0x00Du32 << 21) | ((3 & 0x7F) << 7) | 5;
        let stop = 0xC3u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let trace = driver
            .run_script(&[
                PpuAction::ExpectPark {
                    reason: SpuParkReason::ChannelRead { channel: 3 },
                },
                PpuAction::Signal { slot: 0, value: 0xA5A5 },
                PpuAction::ExpectFinished { stop_code: 0xC3 },
            ])
            .expect("script must succeed");

        assert!(matches!(
            trace.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xC3 }
        ));
        // r5 read snr[0] = 0xA5A5 (the value pushed by the Signal action).
        assert_eq!(trace.final_snapshot.gpr[5] >> 96, 0xA5A5_u128);
        // After read, snr[0] is cleared.
        assert_eq!(trace.final_snapshot.channels.snr[0], 0);
    }

    /// `ExpectPark` against a Done state must error.
    #[test]
    fn lockstep_expect_park_fails_against_finished() {
        // il r3, 1; stop 0
        let il = ((0x081u32 & 0x1FF) << 23) | ((1u32 & 0xFFFF) << 7) | 3;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&il.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let err = driver
            .run_script(&[PpuAction::ExpectPark {
                reason: SpuParkReason::ChannelRead { channel: 29 },
            }])
            .expect_err("must fail — SPU finished, not parked");
        match err {
            LockstepError::ExpectedParkGot { actual, .. } => {
                assert!(matches!(actual, SpuEventKind::Finished { stop_code: 0 }));
            }
            other => panic!("expected ExpectedParkGot, got {other:?}"),
        }
    }

    /// `PopOutMbox { expect: Some(v) }` must error on mismatched value.
    #[test]
    fn lockstep_pop_outmbox_mismatch_errors() {
        // il r3, 7; wrch r3, OUT(28); stop 0
        let il = ((0x081u32 & 0x1FF) << 23) | ((7u32 & 0xFFFF) << 7) | 3;
        let wrch = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 3;
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&il.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let err = driver
            .run_script(&[
                PpuAction::ExpectFinished { stop_code: 0 },
                PpuAction::PopOutMbox { expect: Some(99) },
            ])
            .expect_err("expected mismatch");
        match err {
            LockstepError::OutMboxMismatch { expected: 99, got: Some(7) } => {}
            other => panic!("expected OutMboxMismatch, got {other:?}"),
        }
    }

    /// SPU error during execution propagates as
    /// `LockstepError::SpuExecError`.
    #[test]
    fn lockstep_spu_exec_error_propagates() {
        // rdch r3, 100  (BadChannel — Error path) ; stop
        let rdch = (0x00Du32 << 21) | ((100 & 0x7F) << 7) | 3;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let err = driver
            .run_script(&[])
            .expect_err("BadChannel must surface as SpuExecError");
        assert!(matches!(err, LockstepError::SpuExecError { .. }));
    }

    // =====================================================================
    // R5.5 — Trace replay tests (interpreter backend)
    //
    // Validates the deterministic trace-replay layer over R5.4e
    // lockstep. Covers happy paths (rdch handshake / wrch backpressure /
    // bidirectional ping-pong / GPR + channel state asserts) and
    // failure paths (wrong expected value, wrong park reason).
    // =====================================================================

    /// Encode rdch IN handshake program: `rdch r3,IN(29); ai r4,r3,1;
    /// wrch r4,OUT(28); stop 0xA1`.
    fn replay_rdch_inmbox_program() -> SpuProgram {
        let rdch = (0x00Du32 << 21) | ((29 & 0x7F) << 7) | 3;
        let ai   = ((0x1Cu32 & 0xFF) << 24) | ((1u32 & 0x3FF) << 14)
                 | ((3 & 0x7F) << 7) | 4;
        let wrch = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 4;
        let stop = 0xA1u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(16);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&ai.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    /// Happy path 1: rdch INMBOX handshake script.
    #[test]
    fn trace_replay_rdch_inmbox_handshake() {
        let prog = replay_rdch_inmbox_program();
        let mut backend = InterpreterExecutor::default();
        let report = replay_trace(
            &mut backend,
            prog,
            &[
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                    pc: Some(0x100),
                },
                TraceEvent::PpuPushInMbox {
                    value: 41,
                    expect_wake: SpuWakeResultKind::Ready,
                },
                TraceEvent::ExpectSpuFinished { stop_code: 0xA1 },
                TraceEvent::PpuPopOutMbox {
                    expect: Some(42),
                    expect_wake: Some(SpuWakeResultKind::NotParked),
                },
                TraceEvent::ExpectGprWord { reg: 3, lane: 0, value: 41 },
                TraceEvent::ExpectGprWord { reg: 4, lane: 0, value: 42 },
            ],
        )
        .expect("trace must replay cleanly");

        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xA1 }
        ));
        assert_eq!(report.records.len(), 6);
        assert_eq!(report.final_snapshot.channels.in_mbox, None);
        assert_eq!(report.final_snapshot.channels.out_mbox, None,
                   "PopOutMbox drained the value");
    }

    /// Happy path 2: wrch OUTMBOX backpressure.
    #[test]
    fn trace_replay_wrch_outmbox_backpressure() {
        // 0x100 il   r3, 0x1111
        // 0x104 wrch r3, OUT(28)
        // 0x108 il   r3, 0x2222
        // 0x10C wrch r3, OUT(28)  (parks)
        // 0x110 stop 0xB2
        let il_a = ((0x081u32 & 0x1FF) << 23) | ((0x1111u32 & 0xFFFF) << 7) | 3;
        let wrch = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 3;
        let il_b = ((0x081u32 & 0x1FF) << 23) | ((0x2222u32 & 0xFFFF) << 7) | 3;
        let stop = 0xB2u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(20);
        bytes.extend_from_slice(&il_a.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&il_b.to_be_bytes());
        bytes.extend_from_slice(&wrch.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let report = replay_trace(
            &mut backend,
            prog,
            &[
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelWrite { channel: 28 },
                    pc: Some(0x10C),
                },
                // Drain first value (also wakes the parked wrch).
                TraceEvent::PpuPopOutMbox {
                    expect: Some(0x1111),
                    expect_wake: Some(SpuWakeResultKind::Ready),
                },
                TraceEvent::ExpectSpuFinished { stop_code: 0xB2 },
                // Drain second value.
                TraceEvent::PpuPopOutMbox {
                    expect: Some(0x2222),
                    expect_wake: Some(SpuWakeResultKind::NotParked),
                },
                TraceEvent::ExpectChannelState {
                    in_mbox: None,
                    out_mbox: None,
                    out_intr_mbox: None,
                    snr1: 0,
                    snr2: 0,
                },
            ],
        )
        .expect("trace must replay cleanly");

        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xB2 }
        ));
    }

    /// Happy path 3: bidirectional ping-pong with two park/wake cycles.
    /// Steps must be monotonic.
    #[test]
    fn trace_replay_bidirectional_two_rounds() {
        // 0x100 rdch r3, IN(29)    (parks #1)
        // 0x104 wrch r3, OUT(28)
        // 0x108 rdch r4, IN(29)    (parks #2)
        // 0x10C wrch r4, OUT(28)
        // 0x110 stop 0xC3
        let rdch_3 = (0x00Du32 << 21) | ((29 & 0x7F) << 7) | 3;
        let wrch_3 = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 3;
        let rdch_4 = (0x00Du32 << 21) | ((29 & 0x7F) << 7) | 4;
        let wrch_4 = (0x10Du32 << 21) | ((28 & 0x7F) << 7) | 4;
        let stop   = 0xC3u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(20);
        bytes.extend_from_slice(&rdch_3.to_be_bytes());
        bytes.extend_from_slice(&wrch_3.to_be_bytes());
        bytes.extend_from_slice(&rdch_4.to_be_bytes());
        bytes.extend_from_slice(&wrch_4.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let report = replay_trace(
            &mut backend,
            prog,
            &[
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                    pc: Some(0x100),
                },
                TraceEvent::PpuPushInMbox {
                    value: 0xAA,
                    expect_wake: SpuWakeResultKind::Ready,
                },
                // After resume, SPU runs rdch+wrch then parks again on
                // the second rdch.
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                    pc: Some(0x108),
                },
                // SPU is parked on rdch — popping out_mbox doesn't
                // satisfy the RDINMBOX condition, so wake is
                // StillBlocked. SPU stays parked.
                TraceEvent::PpuPopOutMbox {
                    expect: Some(0xAA),
                    expect_wake: Some(SpuWakeResultKind::StillBlocked),
                },
                TraceEvent::PpuPushInMbox {
                    value: 0xBB,
                    expect_wake: SpuWakeResultKind::Ready,
                },
                TraceEvent::ExpectSpuFinished { stop_code: 0xC3 },
                TraceEvent::PpuPopOutMbox {
                    expect: Some(0xBB),
                    expect_wake: Some(SpuWakeResultKind::NotParked),
                },
            ],
        )
        .expect("trace must replay cleanly");

        // Steps must be monotonic across all records.
        let mut prev = 0u64;
        for r in &report.records {
            assert!(r.steps_at >= prev, "non-monotonic at idx {}", r.event_index);
            prev = r.steps_at;
        }
        assert!(report.total_steps >= prev);
    }

    /// Failure path 1: wrong expected out_mbox value.
    #[test]
    fn trace_replay_rejects_wrong_expected_value() {
        let prog = replay_rdch_inmbox_program();
        let mut backend = InterpreterExecutor::default();
        let err = replay_trace(
            &mut backend,
            prog,
            &[
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                    pc: None,
                },
                TraceEvent::PpuPushInMbox {
                    value: 41,
                    expect_wake: SpuWakeResultKind::Ready,
                },
                TraceEvent::ExpectSpuFinished { stop_code: 0xA1 },
                // Wrong: SPU produced 42, trace claims 99.
                TraceEvent::PpuPopOutMbox {
                    expect: Some(99),
                    expect_wake: Some(SpuWakeResultKind::NotParked),
                },
            ],
        )
        .expect_err("must reject wrong expected value");

        assert_eq!(err.event_index, 3, "error must point at PopOutMbox");
        assert!(matches!(
            err.kind,
            TraceReplayErrorKind::OutMboxValueMismatch {
                expected: 99,
                got: Some(42)
            }
        ), "got {:?}", err.kind);
    }

    /// Failure path 2: wrong expected park reason.
    #[test]
    fn trace_replay_rejects_unexpected_park_reason() {
        let prog = replay_rdch_inmbox_program();
        let mut backend = InterpreterExecutor::default();
        let err = replay_trace(
            &mut backend,
            prog,
            &[
                // Wrong: SPU parks on rdch (ChannelRead {29}), trace
                // claims wrch (ChannelWrite {28}).
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelWrite { channel: 28 },
                    pc: None,
                },
            ],
        )
        .expect_err("must reject wrong park reason");

        assert_eq!(err.event_index, 0);
        assert!(matches!(
            err.kind,
            TraceReplayErrorKind::ParkReasonMismatch {
                expected: SpuParkReason::ChannelWrite { channel: 28 },
                actual: SpuParkReason::ChannelRead { channel: 29 },
            }
        ), "got {:?}", err.kind);
    }

    /// Park PC mismatch is its own variant.
    #[test]
    fn trace_replay_rejects_wrong_parked_pc() {
        let prog = replay_rdch_inmbox_program();
        let mut backend = InterpreterExecutor::default();
        let err = replay_trace(
            &mut backend,
            prog,
            &[
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                    pc: Some(0x999), // wrong; actual is 0x100
                },
            ],
        )
        .expect_err("must reject wrong PC");

        assert_eq!(err.event_index, 0);
        assert!(matches!(
            err.kind,
            TraceReplayErrorKind::ParkPcMismatch { expected: 0x999, actual: 0x100 }
        ));
    }

    /// Wake-kind mismatch must surface as `WakeKindMismatch`. Use a
    /// signal action against an RDINMBOX park — actual wake will be
    /// StillBlocked, but the trace claims Ready.
    #[test]
    fn trace_replay_rejects_wake_kind_mismatch() {
        let prog = replay_rdch_inmbox_program();
        let mut backend = InterpreterExecutor::default();
        let err = replay_trace(
            &mut backend,
            prog,
            &[
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                    pc: None,
                },
                // Wrong: signal slot 0 won't wake an RDINMBOX park.
                TraceEvent::PpuSignal {
                    slot: 0,
                    value: 0xFF,
                    expect_wake: SpuWakeResultKind::Ready,
                },
            ],
        )
        .expect_err("must reject wake-kind mismatch");

        assert_eq!(err.event_index, 1);
        assert!(matches!(
            err.kind,
            TraceReplayErrorKind::WakeKindMismatch {
                expected: SpuWakeResultKind::Ready,
                actual: SpuWakeResult::StillBlocked,
            }
        ), "got {:?}", err.kind);
    }

    /// `replay_trace` summary export contains event index, steps, and
    /// final stop code.
    #[test]
    fn trace_replay_exports_human_readable_summary() {
        let prog = replay_rdch_inmbox_program();
        let mut backend = InterpreterExecutor::default();
        let report = replay_trace(
            &mut backend,
            prog,
            &[
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                    pc: None,
                },
                TraceEvent::PpuPushInMbox {
                    value: 7,
                    expect_wake: SpuWakeResultKind::Ready,
                },
                TraceEvent::ExpectSpuFinished { stop_code: 0xA1 },
            ],
        )
        .expect("trace must replay");

        let summary = report.summary();
        // Header line.
        assert!(summary.contains("3 events processed"));
        assert!(summary.contains("Finished"));
        // Per-event lines with index + steps_at.
        assert!(summary.contains("[0]"));
        assert!(summary.contains("[1]"));
        assert!(summary.contains("[2]"));
        // Final stop code.
        assert!(summary.contains("0xa1"), "summary should include stop code");
    }

    /// `ExpectGprWord` mismatch surfaces as `GprMismatch`.
    #[test]
    fn trace_replay_expect_gpr_word_rejects_wrong_value() {
        let prog = replay_rdch_inmbox_program();
        let mut backend = InterpreterExecutor::default();
        let err = replay_trace(
            &mut backend,
            prog,
            &[
                TraceEvent::ExpectSpuPark {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                    pc: None,
                },
                TraceEvent::PpuPushInMbox {
                    value: 41,
                    expect_wake: SpuWakeResultKind::Ready,
                },
                TraceEvent::ExpectSpuFinished { stop_code: 0xA1 },
                // Wrong: r3 is 41 in lane 0, not 99.
                TraceEvent::ExpectGprWord { reg: 3, lane: 0, value: 99 },
            ],
        )
        .expect_err("must reject wrong GPR value");

        assert_eq!(err.event_index, 3);
        assert!(matches!(
            err.kind,
            TraceReplayErrorKind::GprMismatch {
                reg: 3, lane: 0, expected: 99, got: 41,
            }
        ));
    }

    /// Initial-run BadChannel surfaces as `SpuExecError` at event 0.
    #[test]
    fn trace_replay_initial_bad_channel_surfaces_error() {
        let rdch = (0x00Du32 << 21) | ((100 & 0x7F) << 7) | 3;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = InterpreterExecutor::default();
        let err = replay_trace(&mut backend, prog, &[])
            .expect_err("BadChannel must surface as initial-run error");
        assert_eq!(err.event_index, 0);
        assert!(matches!(err.kind, TraceReplayErrorKind::SpuExecError { .. }));
    }

    // =====================================================================
    // R5.6 — Synthetic homebrew-like mailbox-command-protocol fixture
    // =====================================================================

    /// Run the full mailbox command protocol fixture through the
    /// interpreter backend. Validates: rdch INMBOX park, push cmd 1
    /// → result 0x2A, push cmd 2 → wrch backpressure → drain wakes
    /// wrch → result 0x2B, push 0xFF → halt 0xD5, final cleanup pop,
    /// final GPR + channel state.
    #[test]
    fn r5_6_trace_replay_mailbox_command_protocol_interpreter() {
        let prog = mailbox_command_protocol_program();
        let trace = mailbox_command_protocol_trace();

        let mut backend = InterpreterExecutor::default();
        let report = replay_trace(&mut backend, prog, &trace)
            .expect("mailbox command protocol must replay cleanly on interpreter");

        // Final state — clean halt, channels drained.
        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xD5 }
        ));
        assert_eq!(report.final_snapshot.park_state, None);
        assert_eq!(report.final_snapshot.channels.in_mbox, None);
        assert_eq!(report.final_snapshot.channels.out_mbox, None);
        // 16 trace events, all processed.
        assert_eq!(report.records.len(), 16);
        // Steps must be monotonic across the cycle.
        let mut prev = 0u64;
        for r in &report.records {
            assert!(r.steps_at >= prev,
                    "non-monotonic at idx {} ({} < {})",
                    r.event_index, r.steps_at, prev);
            prev = r.steps_at;
        }
    }

    /// Mutating one expected pop value in the trace must produce a
    /// `OutMboxValueMismatch` keyed at the offending event index.
    /// Reproducibility of failure messages.
    #[test]
    fn r5_6_trace_rejects_wrong_command_result() {
        let prog = mailbox_command_protocol_program();
        let mut trace = mailbox_command_protocol_trace();

        // Mutate event [7] (PpuPopOutMbox cmd-1 result drain) to
        // expect a wrong value. Trace was correct otherwise.
        match &mut trace[7] {
            TraceEvent::PpuPopOutMbox { expect, .. } => {
                *expect = Some(0x99); // wrong; actual SPU produced 0x2A
            }
            _ => panic!("trace[7] must be PpuPopOutMbox"),
        }

        let mut backend = InterpreterExecutor::default();
        let err = replay_trace(&mut backend, prog, &trace)
            .expect_err("trace with wrong expected value must fail");
        assert_eq!(err.event_index, 7);
        assert!(matches!(
            err.kind,
            TraceReplayErrorKind::OutMboxValueMismatch {
                expected: 0x99,
                got: Some(0x2A),
            }
        ), "got {:?}", err.kind);
    }

    /// Fixture builder must be deterministic byte-for-byte. Calling
    /// the program builder twice yields identical segments. Same for
    /// the trace builder.
    #[test]
    fn r5_6_fixture_is_reproducible() {
        let p1 = mailbox_command_protocol_program();
        let p2 = mailbox_command_protocol_program();
        assert_eq!(p1.entry_pc, p2.entry_pc);
        assert_eq!(p1.max_steps, p2.max_steps);
        assert_eq!(p1.segments.len(), p2.segments.len());
        for (s1, s2) in p1.segments.iter().zip(p2.segments.iter()) {
            assert_eq!(s1.lsa, s2.lsa);
            assert_eq!(s1.data, s2.data,
                       "program bytes must be deterministic across calls");
        }

        let t1 = mailbox_command_protocol_trace();
        let t2 = mailbox_command_protocol_trace();
        // TraceEvent doesn't derive PartialEq (it carries Strings, etc.),
        // so compare via Debug formatting — equivalent for these
        // structurally-identical literals.
        assert_eq!(t1.len(), t2.len());
        for (e1, e2) in t1.iter().zip(t2.iter()) {
            assert_eq!(format!("{e1:?}"), format!("{e2:?}"),
                       "trace events must be deterministic across calls");
        }
    }

    /// `summary_with_label` includes the fixture name and per-event
    /// indices (`[0]`, `[1]`, ...). On a successful run the summary
    /// also includes the final stop code. On a failed run, the
    /// `Display` impl on `TraceReplayError` emits the failing event
    /// index — verified separately so failure messages stay usable.
    #[test]
    fn r5_6_trace_summary_mentions_fixture_name_and_event_index() {
        let prog = mailbox_command_protocol_program();
        let trace = mailbox_command_protocol_trace();

        let mut backend = InterpreterExecutor::default();
        let report = replay_trace(&mut backend, prog, &trace).expect("replay ok");

        let labeled = report.summary_with_label(FIXTURE_NAME_MAILBOX_PROTOCOL);
        assert!(labeled.contains(FIXTURE_NAME_MAILBOX_PROTOCOL),
                "summary must mention fixture name");
        // Event indices.
        assert!(labeled.contains("[0]"));
        assert!(labeled.contains("[15]"), "all 16 events must be listed");
        // Final stop code.
        assert!(labeled.contains("0xd5"),
                "summary must include the stop code");

        // Failure-message usability: mutate one event and assert the
        // formatted error mentions the failing event index.
        let mut bad_trace = mailbox_command_protocol_trace();
        if let TraceEvent::ExpectGprWord { value, .. } = &mut bad_trace[14] {
            *value = 0x9999; // wrong
        }
        let mut backend2 = InterpreterExecutor::default();
        let err = replay_trace(
            &mut backend2,
            mailbox_command_protocol_program(),
            &bad_trace,
        )
        .expect_err("must fail");
        let formatted = format!("{}", err);
        assert!(formatted.contains("event 14"),
                "failure message must mention failing event index, got: {formatted}");
    }
}
