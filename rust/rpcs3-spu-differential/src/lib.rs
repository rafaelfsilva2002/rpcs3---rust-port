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
use rpcs3_spu_thread::{SpuThread, SPU_GPR_COUNT, SPU_LS_SIZE};

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
}

impl SpuProgram {
    #[must_use]
    pub fn new(entry_pc: u32, max_steps: u64) -> Self {
        Self { entry_pc, segments: Vec::new(), max_steps }
    }

    /// Append a code/data segment. Returns `self` for builder chaining.
    #[must_use]
    pub fn with_segment(mut self, lsa: u32, data: Vec<u8>) -> Self {
        self.segments.push(SpuSegment { lsa, data });
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
    /// Channel state is *not* propagated from the JIT — the JIT does
    /// not codegen channel ops yet, so any channel-touching instruction
    /// triggers exactly the partial fallback that lands here, with
    /// channels at their default (empty) state. If the JIT ever gains
    /// channel codegen, the channel state will need to flow through
    /// here too; today it's a non-issue by construction.
    #[must_use]
    pub fn resume_from_state(
        &self,
        gpr: &[u128; SPU_GPR_COUNT],
        ls: &[u8; SPU_LS_SIZE],
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
}

impl SpuDiff {
    #[must_use]
    pub fn is_identical(&self) -> bool {
        self.pc_match
            && self.channel_counts_match
            && self.gpr_mismatches.is_empty()
            && self.ls_total_diff_bytes == 0
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
        };
        let mut sb = SpuStateSnapshot {
            pc: 0,
            gpr: Box::new([0u128; SPU_GPR_COUNT]),
            ls: Box::new([0u8; SPU_LS_SIZE]),
            channel_counts: ChannelCounts::default(),
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
}
