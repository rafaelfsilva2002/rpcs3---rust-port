//! `rpcs3-spu-recompiler` — scaffold for the Rust SPU recompiler.
//!
//! ## Status
//!
//! **Scaffold only.** No code generation yet. The default
//! [`RecompilerExecutor`] runs every SPU program by:
//!
//! 1. Decoding it into a [`SpuFunction`](rpcs3_spu_decoder::SpuFunction)
//!    once via [`rpcs3_spu_decoder::decode_function`].
//! 2. Caching the decoded function (proves the function-cache shape
//!    before a real JIT cache lands).
//! 3. Executing via the interpreter backend
//!    ([`rpcs3_spu_differential::InterpreterExecutor`]).
//!
//! This means **today** the recompiler is byte-identical to the
//! interpreter on every input — by construction. That's the entire
//! point: it lets us wire up `--backend recompiler` in `spu-runner`
//! and prove the diff harness is happy *before* introducing any code-
//! generation backend.
//!
//! When a real backend (Cranelift, per `SPU_RECOMPILER_PLAN.md` D1)
//! is approved, it slots in inside `execute()` between steps 2 and 3:
//! "if every block in the function is JIT-compilable, run the JIT;
//! otherwise fall back to the interpreter."
//!
//! ## Public API
//!
//! - [`RecompilerExecutor`] — implements [`SpuExecutor`].
//! - [`function_cache_size`] / [`clear_function_cache`] — for tests
//!   that want to inspect cache behavior.

use std::collections::HashMap;
use std::sync::Mutex;

use rpcs3_spu_decoder::{decode_function, decode_inst, DecodeError, SpuFunction, SpuInstKind};
use rpcs3_spu_differential::{
    error_result, ChannelCounts, ExecutionStopReason, InterpreterExecutor, SpuExecutionResult,
    SpuExecutor, SpuProgram, SpuStateSnapshot,
};

pub mod jit;
use jit::{
    CompiledFunction, JitBackend, JitState,
    JIT_OUTCOME_STOP, JIT_OUTCOME_CONTINUE_TO, JIT_OUTCOME_UNKNOWN_OPCODE,
    JIT_OUTCOME_STALL,
};
use rpcs3_spu_thread::SpuChannels;

/// R4b chain-table entry: a stable, lock-free reference to a previously
/// compiled function plus the `ls_hash` snapshot that proved the function
/// was the right one for that pc. The entry is invalidated by a future
/// dispatcher iteration whose target pc has a different `ls_hash` (which
/// catches self-modifying-code style overwrites of the function bytes).
#[derive(Clone, Copy)]
struct ChainEntry {
    /// Raw extern "C" entry function pointer. Stable for the executor's
    /// lifetime — Cranelift never relocates finalised JIT code.
    entry_fn: extern "C" fn(*mut JitState) -> u32,
    /// `hash_ls_around(ls, pc)` at compile time. If it changes, the chain
    /// entry is stale and must not be trusted.
    ls_hash: u64,
    /// Cached instruction count, used for the dispatcher's `total_steps`
    /// budget without re-querying the decoded-function cache.
    function_size: u64,
}

/// R5.1 + R5.2: which dispatcher path triggered a partial fallback.
/// Used by `partial_fallback_to_interpreter` to attribute stats
/// correctly without double-counting.
#[derive(Clone, Copy)]
enum PartialFallbackCause {
    /// `compile_or_fetch` returned `Err` — the function couldn't be
    /// JIT-compiled because at least one instruction was unsupported.
    /// Channel attribution already happened inside `compile_or_fetch`
    /// while the decoded function was in scope.
    CompileFailure,
    /// JIT runtime returned `JIT_OUTCOME_UNKNOWN_OPCODE`. Defensive
    /// escape hatch — current codegen never emits this — but kept
    /// live for safety. The failing instruction is exactly at
    /// `state.pc` because the JIT writes pc before returning.
    RuntimeUnknownOpcode,
    /// R5.2: JIT-emitted channel helper (`spu_helper_rdch` /
    /// `spu_helper_wrch`) returned a non-Ok outcome. The JIT codegen
    /// already wrote `state.pc` to the channel op's pc and returned
    /// `JIT_OUTCOME_STALL`. Bumps `channel_stall_exits` (and the
    /// existing channel attribution stat).
    ChannelStall,
}

/// Result of a chain-table lookup. Hit returns the data needed to call
/// the function directly; Stale and Miss both signal the dispatcher to
/// fall through to the global cache (Stale also evicts the bad entry).
enum ChainLookup {
    Hit {
        entry_fn: extern "C" fn(*mut JitState) -> u32,
        function_size: u64,
    },
    Stale,
    Miss,
}

/// R4c metadata attached to every compiled function: the byte range its
/// SPU code occupies in LS (`code_start..code_end`) plus a hash of those
/// exact bytes captured at compile time. The dispatcher's SMC scan
/// recomputes the hash and invalidates the entry if it diverges — that
/// guarantees we never invoke JIT code whose source instructions have
/// since changed (whether by another execute() with a different program
/// or by the JIT itself writing through `state.ls_ptr`).
#[derive(Clone, Copy)]
struct CompiledMeta {
    /// Inclusive byte address of the first instruction.
    code_start: u32,
    /// Exclusive byte address one past the last instruction (covers
    /// every basic block reachable from `entry`).
    code_end: u32,
    /// `hash_ls_range(ls, code_start, code_end)` at compile time.
    exact_hash: u64,
    /// Cached for symmetry with `ChainEntry`; lets the SMC scan skip
    /// touching the decoded-function cache to find the size again.
    function_size: u64,
}

const SPU_GPR_COUNT: usize = 128;
const SPU_LS_SIZE: usize = 0x40000;
const DEFAULT_MAX_BLOCKS: usize = 2048;

/// Hash key for the function cache. Combines the entry PC and a hash
/// of the LS region we decoded from. Good enough for v0 — the JIT
/// cache will replace this with a content-addressed scheme.
type CacheKey = (u32, u64);

#[derive(Clone)]
struct CachedFunction {
    /// Decoded basic-block graph. Held for inspection / future codegen.
    #[allow(dead_code)]
    function: SpuFunction,
    /// Whether the JIT was able to compile this function.
    /// `Some(_)` means JIT is ready; `None` means we already tried
    /// and bailed — fall back to interpreter without re-attempting.
    jit_compiled: Option<()>,
}

/// SPU recompiler executor. Today this is just `decode + cache +
/// delegate to interpreter`. The trait surface is the same as
/// [`InterpreterExecutor`] so swap-in is automatic.
pub struct RecompilerExecutor {
    interp: InterpreterExecutor,
    /// JIT module + compiled function cache. Mutable because each
    /// `compile` call mutates Cranelift's module state.
    jit: Mutex<JitCache>,
    cache: Mutex<HashMap<CacheKey, CachedFunction>>,
    max_blocks: usize,
}

impl Default for RecompilerExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Owns the Cranelift JIT module + a per-key compiled-function map.
/// Held inside `RecompilerExecutor::jit` behind a Mutex.
struct JitCache {
    backend: JitBackend,
    compiled: HashMap<CacheKey, CompiledFunction>,
    /// R4b: chain table — pc → most-recently-compiled function (with
    /// `ls_hash` guard). Independent of the global `compiled` map so
    /// rehashing doesn't invalidate it (entries hold raw fn pointers,
    /// not `&CompiledFunction`). Persists across `execute()` calls.
    chain: HashMap<u32, ChainEntry>,
    /// R4c: per-compile metadata used by the SMC scan. Keyed identically
    /// to `compiled`. Lifetime is bound to the compiled function — when
    /// the SMC scan invalidates an entry, both maps lose the key.
    compiled_meta: HashMap<CacheKey, CompiledMeta>,
    /// Counts: `(jit_runs, fallback_runs)` for observability.
    pub stats: JitStats,
}

/// Per-executor JIT statistics. Returned by [`RecompilerExecutor::jit_stats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JitStats {
    /// Number of dispatcher iterations that ran a JIT-compiled function.
    /// In R4a, a single `execute()` call may run multiple JIT iterations
    /// when the program contains indirect branches (each indirect target
    /// is its own JIT-compiled function).
    pub jit_runs: u64,
    /// Number of `execute()` calls that fell back to the interpreter
    /// (because some compiled function hit an unsupported opcode).
    pub fallback_runs: u64,
    /// Number of unique functions successfully JIT-compiled across the
    /// executor's lifetime. Equals the size of the JIT cache.
    pub compiled_functions: u64,
    /// JIT cache hits: dispatcher iterations where the entry PC's
    /// function was already compiled and ready to invoke. Only counts
    /// iterations that actually went through the global cache lookup —
    /// R4b chain hits (see `patch_hits`) bypass the global cache and
    /// are not counted here.
    pub cache_hits: u64,
    /// JIT cache misses: dispatcher iterations where compilation was
    /// triggered. `cache_misses == compiled_functions` while no cache
    /// invalidation occurs.
    pub cache_misses: u64,
    /// Number of dispatcher iterations across `execute()` calls.
    /// Invariant: `dispatcher_iterations == patch_hits + patch_misses`.
    pub dispatcher_iterations: u64,
    /// R4b: dispatcher iterations satisfied by the chain table fast-path
    /// (target pc's compiled function reused without re-querying the
    /// global cache). Equal to `dispatcher_bypasses` and `patch_hits` —
    /// kept as a distinct field so plan-doc terminology lines up.
    pub chained_jumps: u64,
    /// R4b: dispatcher iterations where the chain table fast-path skipped
    /// the global cache mutex entirely.
    pub dispatcher_bypasses: u64,
    /// R4b: dispatcher iterations where the chain table held a valid
    /// (matching `ls_hash`) entry for the target pc.
    pub patch_hits: u64,
    /// R4b: dispatcher iterations where the chain table did NOT satisfy
    /// the request (no entry, or `ls_hash` mismatch). These fall through
    /// to the regular global-cache path.
    pub patch_misses: u64,
    /// R4b: dispatcher iterations where the chain table held a stale
    /// entry (matching pc but wrong `ls_hash`) and we evicted it before
    /// falling through. Subset of `patch_misses`.
    pub invalid_chain_guards: u64,
    /// R4c: total compiled-function entries removed because the SMC
    /// scan detected that the LS bytes covering their code range no
    /// longer match the hash captured at compile time.
    pub smc_invalidations: u64,
    /// R4c: chain-table entries evicted by an SMC scan (subset of
    /// `smc_invalidations` — only counts the case where the chain
    /// pointed at the function we just removed from the global cache).
    pub smc_chain_evictions: u64,
    /// R4c: number of times the SMC scan triggered a wholesale flush
    /// of the JIT caches. Stays at 0 in the minimal R4c implementation
    /// (per-entry invalidation always works); reserved as an escape
    /// hatch for future tracking modes that prefer "nuke everything".
    pub smc_full_flushes: u64,
    /// R4c: range checks where the SMC scan recomputed the exact-range
    /// hash and found a mismatch (== `smc_invalidations` while we evict
    /// every stale entry; stays consistent so tests can assert on either).
    pub smc_range_hits: u64,
    /// R4c: range checks where the SMC scan recomputed the exact-range
    /// hash and found it unchanged. The vast majority of scans land here
    /// for non-SMC programs.
    pub smc_range_misses: u64,
    /// R5: dispatcher iterations that exited the JIT mid-program and
    /// handed execution back to the interpreter to resume from the
    /// current `state.pc`. Counts both `JIT_OUTCOME_UNKNOWN_OPCODE`
    /// runtime exits and compile-failure-on-target events. After R5,
    /// `fallback_runs` (full re-run from `program.entry_pc`) should be
    /// 0 in normal operation; this counter replaces it for partial
    /// exits that preserve the JIT-accumulated state.
    pub partial_fallbacks: u64,
    /// R5: subset of `partial_fallbacks` triggered specifically by an
    /// unsupported opcode (compile failure or runtime UNKNOWN_OPCODE).
    /// Equals `partial_fallbacks` while channel-stall handoff is not
    /// implemented; tracked separately so future R5+ can split them.
    pub unknown_opcode_exits: u64,
    /// R5: reserved counter for future channel-stall handoff to the
    /// interpreter. Stays at 0 until channel codegen lands on the JIT
    /// side; included now so test code and docs can refer to a stable
    /// field name.
    pub channel_stall_exits: u64,
    /// R5: number of times the interpreter resumed an in-flight run
    /// (semantically identical to `partial_fallbacks`; kept as a
    /// distinct field so terminology lines up with the resume API).
    pub resumed_interpreter_runs: u64,
    /// R5: cumulative SPU instruction steps executed by the interpreter
    /// after a partial fallback. Useful for `steps_under_partial_fallback
    /// / total_steps` ratios. Steps executed by the JIT prefix are
    /// already accounted for in `dispatcher_iterations`/`jit_runs`.
    pub resumed_interpreter_steps: u64,
    /// R5.1: count of channel ops successfully codegen'd at JIT compile
    /// time. Counts every supported `rchcnt` against a constant-count
    /// channel emitted by the JIT — incremented once per instruction
    /// per compile, not per run. A channel op that fails `supported_check`
    /// (e.g. `rdch`/`wrch`, or `rchcnt` against a variable-count channel)
    /// is **not** counted here; instead it triggers the R5 partial
    /// fallback path and bumps `channel_ops_partial_fallback`.
    pub channel_ops_jitted: u64,
    /// R5.1: subset of `partial_fallbacks` triggered by an unsupported
    /// channel op (i.e., the function being compiled contained a
    /// channel instruction the JIT does not codegen). Distinguishes
    /// "fell back because of channels" from "fell back because of
    /// some other unsupported opcode (e.g., dfa)".
    pub channel_ops_partial_fallback: u64,
}

impl RecompilerExecutor {
    /// Construct a recompiler with the default cache + max-blocks.
    #[must_use]
    pub fn new() -> Self {
        Self {
            interp: InterpreterExecutor::default(),
            jit: Mutex::new(JitCache {
                backend: JitBackend::new(),
                compiled: HashMap::new(),
                chain: HashMap::new(),
                compiled_meta: HashMap::new(),
                stats: JitStats::default(),
            }),
            cache: Mutex::new(HashMap::new()),
            max_blocks: DEFAULT_MAX_BLOCKS,
        }
    }

    /// How many decoded functions currently sit in the cache.
    #[must_use]
    pub fn function_cache_size(&self) -> usize {
        self.cache.lock().expect("cache lock poisoned").len()
    }

    /// Snapshot of JIT statistics. Useful for tests that want to
    /// confirm the JIT path was actually taken (vs. silent fallback).
    #[must_use]
    pub fn jit_stats(&self) -> JitStats {
        self.jit.lock().expect("jit lock poisoned").stats
    }

    /// Drop every cached function. The next `execute` call will
    /// re-decode from scratch.
    pub fn clear_function_cache(&self) {
        self.cache.lock().expect("cache lock poisoned").clear();
        let mut jit = self.jit.lock().expect("jit lock poisoned");
        jit.compiled.clear();
        jit.chain.clear();
        jit.compiled_meta.clear();
    }

    /// Snapshot the current size of the R4b chain table (one entry per
    /// distinct pc the dispatcher has previously executed). Useful for
    /// tests that want to confirm chained-patching state without parsing
    /// the full `JitStats`.
    #[must_use]
    pub fn chain_table_size(&self) -> usize {
        self.jit.lock().expect("jit lock poisoned").chain.len()
    }

    /// R5: partial fallback to the interpreter from the current JIT
    /// state. Builds a snapshot of GPRs + LS + PC from the in-flight
    /// `JitState` and hands execution to `InterpreterExecutor::resume_from_state`.
    ///
    /// Used by the dispatcher loop when:
    ///   - `compile_or_fetch` returns `Err` for the current target pc
    ///     (an unsupported opcode lives inside that function), or
    ///   - the JIT runtime returns `JIT_OUTCOME_UNKNOWN_OPCODE` (the
    ///     reserved escape hatch for codegen paths that decided to
    ///     bail at runtime — currently unused in practice but defensive).
    ///
    /// The returned `SpuExecutionResult.steps_executed` already folds
    /// `total_steps` (JIT prefix) into the interpreter's step count.
    fn partial_fallback_to_interpreter(
        &self,
        state: &JitState,
        ls: &[u8; SPU_LS_SIZE],
        channels: &SpuChannels,
        total_steps: u64,
        program: &SpuProgram,
        cause: PartialFallbackCause,
    ) -> SpuExecutionResult {
        let mut gpr = [0u128; SPU_GPR_COUNT];
        for i in 0..SPU_GPR_COUNT {
            gpr[i] = state.load_gpr(i);
        }
        let resume_pc = state.pc;
        let remaining = program.max_steps.saturating_sub(total_steps);

        // R5.1 + R5.2 attribution:
        //   - `CompileFailure`: `compile_or_fetch` already bumped
        //     `channel_ops_partial_fallback`; nothing else here.
        //   - `RuntimeUnknownOpcode`: rare/defensive — decode the one
        //     instruction at resume_pc and bump if it's a channel op.
        //   - `ChannelStall`: the JIT-emitted helper returned non-Ok;
        //     the instruction at resume_pc IS a channel op by
        //     construction, so always bump both
        //     `channel_ops_partial_fallback` and `channel_stall_exits`.
        let (channel_attr_bump, stall_bump) = match cause {
            PartialFallbackCause::CompileFailure => (false, false),
            PartialFallbackCause::RuntimeUnknownOpcode => (decode_inst_at(ls, resume_pc), false),
            PartialFallbackCause::ChannelStall => (true, true),
        };

        let result = self.interp.resume_from_state(
            &gpr,
            ls,
            channels,
            resume_pc,
            remaining,
            total_steps,
        );

        let interp_steps = result.steps_executed.saturating_sub(total_steps);
        {
            let mut jit = self.jit.lock().expect("jit lock poisoned");
            jit.stats.partial_fallbacks += 1;
            jit.stats.unknown_opcode_exits += 1;
            jit.stats.resumed_interpreter_runs += 1;
            jit.stats.resumed_interpreter_steps =
                jit.stats.resumed_interpreter_steps.saturating_add(interp_steps);
            if channel_attr_bump {
                jit.stats.channel_ops_partial_fallback += 1;
            }
            if stall_bump {
                jit.stats.channel_stall_exits += 1;
            }
        }

        result
    }


    /// R4a dispatcher loop with R4b chained-patching fast-path.
    /// Each iteration:
    ///   1. R4b: look up state.pc in the chain table. On a hit with
    ///      matching `ls_hash`, call the cached entry_fn directly —
    ///      bypassing the global cache lookup entirely.
    ///   2. Otherwise, fall back to R4a behavior: compile-or-fetch
    ///      against the global cache, then call.
    ///   3. Branch on the outcome. CONTINUE_TO loops back; STOP returns;
    ///      UNKNOWN_OPCODE means fall back to interpreter for the rest.
    ///
    /// Returns `Some(result)` when execution finished cleanly via JIT
    /// (Stop). Returns `None` if any compile attempt failed up-front,
    /// signalling the caller to use the full interpreter path.
    fn try_jit_run(
        &self,
        program: &SpuProgram,
    ) -> Option<SpuExecutionResult> {
        // Allocate the persistent LS buffer + JIT state for this run.
        // Both live until the dispatcher exits.
        let mut ls = Box::new([0u8; SPU_LS_SIZE]);
        for seg in &program.segments {
            let start = seg.lsa as usize;
            let end = start + seg.data.len();
            if end <= SPU_LS_SIZE {
                ls[start..end].copy_from_slice(&seg.data);
            }
        }
        // R5.2: per-execute SpuChannels owned by the dispatcher. JIT
        // codegen for `rdch`/`wrch` calls runtime helpers that mutate
        // this state via `state.channels_ptr`. On partial fallback, we
        // hand the same SpuChannels to the interpreter so the resume
        // sees the JIT-side mutations.
        let mut channels = Box::new(SpuChannels::default());

        let mut state = JitState::new();
        state.pc = program.entry_pc & 0x3FFFC;
        state.ls_ptr = ls.as_mut_ptr();
        state.channels_ptr = &mut *channels as *mut SpuChannels;
        for &(reg, value) in &program.initial_gpr_overrides {
            if (reg as usize) < state.gpr_lanes.len() {
                state.store_gpr(reg as usize, value);
            }
        }
        // R6.7 C.4 — feed the pre-populated tag-stat queue into the
        // JIT's channels (same field on the heap-pinned SpuChannels
        // that codegen mutates via state.channels_ptr). Empty queue
        // for non-DMA programs.
        channels.mfc_tag_stat_queue.extend(
            program.initial_mfc_tag_stat_queue.iter().copied(),
        );
        // R8.5d D.6 — seed the per-tag stall mask so the JIT's
        // `rdch ch25 MFC_RdListStallStat` codegen path returns the
        // captured value destructively (mirrors the Interpreter
        // backend in rpcs3-spu-differential::lib.rs). Zero for
        // non-stall traces.
        channels.mfc_list_stall_mask = program.initial_mfc_list_stall_mask;

        let max_iterations = (program.max_steps.max(1)) as usize;
        let mut total_steps: u64 = 0;

        for _iter in 0..max_iterations {
            // R4c: scan for self-modifying code before consulting any
            // cache. If the LS bytes covering a previously compiled
            // function changed (whether the JIT itself overwrote them
            // last iter, or a different `execute()` call ran a different
            // program at the same pc), the scan invalidates the entry
            // here so the chain/global lookups below see only fresh
            // state. No-op when nothing has changed.
            self.smc_scan(&ls);

            let target_pc = state.pc;
            // Compute ls_hash once per iteration. Used both for the
            // chain-table guard AND (on miss) the global cache key.
            let cur_hash = hash_ls_around(ls.as_ref(), target_pc);

            // R4b chain-table fast-path. On hit (with matching ls_hash)
            // we skip the global cache lookup and call the compiled
            // function directly. On stale hit / miss, fall through.
            let chain_lookup = self.chain_lookup(target_pc, cur_hash);
            let (entry_fn, function_size) = match chain_lookup {
                ChainLookup::Hit { entry_fn, function_size } => (entry_fn, function_size),
                ChainLookup::Stale | ChainLookup::Miss => {
                    let compile_result = self.compile_or_fetch(target_pc, &ls);
                    let (compiled_ptr, function_size) = match compile_result {
                        Ok((ptr, size)) => (ptr, size),
                        Err(_) => {
                            // R5 partial fallback: target function has an
                            // unsupported opcode in its body. Hand the
                            // current state (gprs, ls, pc=target_pc) to
                            // the interpreter and let it continue from
                            // there — no need to re-run the JIT prefix.
                            // R5.1 attribution: channel-related cause was
                            // already bumped inside `compile_or_fetch`.
                            return Some(self.partial_fallback_to_interpreter(
                                &state, &ls, &channels, total_steps, program,
                                PartialFallbackCause::CompileFailure,
                            ));
                        }
                    };
                    // Read the entry fn pointer + install into the chain
                    // table for next time. The pointer-deref happens
                    // before any further global-cache mutations, so it's
                    // safe with the existing invariant (single-threaded
                    // dispatcher).
                    let entry_fn = unsafe { (*compiled_ptr).entry_fn() };
                    self.chain_install(target_pc, entry_fn, cur_hash, function_size);
                    (entry_fn, function_size)
                }
            };

            {
                let mut jit = self.jit.lock().expect("jit lock poisoned");
                jit.stats.dispatcher_iterations += 1;
                jit.stats.jit_runs += 1;
            }

            // SAFETY: entry_fn points at finalised Cranelift JIT code in
            // the JITModule held by self.jit; the module is not dropped
            // while the executor is alive. The JIT reads/writes only
            // `state` (with its ls_ptr that points at our `ls` buffer)
            // for the duration of this call.
            let outcome = entry_fn(&mut state as *mut JitState);
            total_steps = total_steps.saturating_add(function_size);

            match outcome {
                JIT_OUTCOME_STOP => {
                    let code = state.stop_code;
                    return Some(self.build_result(&mut state, &ls, &channels, total_steps,
                        ExecutionStopReason::Stop(code)));
                }
                JIT_OUTCOME_CONTINUE_TO => {
                    // state.pc is the new target; loop.
                    if total_steps >= program.max_steps {
                        return Some(self.build_result(&mut state, &ls, &channels, total_steps,
                            ExecutionStopReason::MaxStepsExceeded));
                    }
                    continue;
                }
                JIT_OUTCOME_UNKNOWN_OPCODE => {
                    // R5 partial fallback: JIT runtime hit an opcode it
                    // refuses to handle (defensive — supported_check
                    // catches this at compile time, but we keep the
                    // path live as a safety net). Hand to interpreter
                    // from current state.pc.
                    return Some(self.partial_fallback_to_interpreter(
                        &state, &ls, &channels, total_steps, program,
                        PartialFallbackCause::RuntimeUnknownOpcode,
                    ));
                }
                JIT_OUTCOME_STALL => {
                    // R5.2: JIT codegen for rdch/wrch hit a Stall or
                    // BadChannel return from the runtime helper. The
                    // helper already left state.pc at the channel op's
                    // pc and the SpuChannels in their pre-call state
                    // (read/write check capacity before mutating). We
                    // hand control to the interpreter so it can either
                    // surface the stall (Ok(StepOutcome::ChannelStall))
                    // or produce the same Error::Unimplemented as a
                    // pure interpreter run would.
                    return Some(self.partial_fallback_to_interpreter(
                        &state, &ls, &channels, total_steps, program,
                        PartialFallbackCause::ChannelStall,
                    ));
                }
                _ => {
                    return Some(self.build_result(&mut state, &ls, &channels, total_steps,
                        ExecutionStopReason::Error(format!(
                            "unknown JIT outcome {outcome}"
                        ))));
                }
            }
        }

        // R5: every former "hit_unsupported" path now returns a result
        // via `partial_fallback_to_interpreter` from inside the loop, so
        // reaching here means we exhausted `max_iterations` without
        // hitting Stop. Surface that as MaxStepsExceeded.
        Some(self.build_result(&mut state, &ls, &channels, total_steps,
            ExecutionStopReason::MaxStepsExceeded))
    }

    /// R4b: look up the chain table for `target_pc` and validate against
    /// `cur_hash`. Returns `Hit` (skip the global cache), `Stale` (entry
    /// existed but ls_hash diverged — evicted, fall through to global
    /// cache), or `Miss` (no entry — fall through). Stats are updated
    /// inside this function.
    fn chain_lookup(&self, target_pc: u32, cur_hash: u64) -> ChainLookup {
        let mut jit = self.jit.lock().expect("jit lock poisoned");
        match jit.chain.get(&target_pc).copied() {
            Some(entry) if entry.ls_hash == cur_hash => {
                jit.stats.chained_jumps += 1;
                jit.stats.dispatcher_bypasses += 1;
                jit.stats.patch_hits += 1;
                ChainLookup::Hit {
                    entry_fn: entry.entry_fn,
                    function_size: entry.function_size,
                }
            }
            Some(_) => {
                // ls_hash mismatch → stale entry (could be SMC, could be
                // an aliased pc from a different program). Evict and let
                // the caller fall through to the global cache.
                jit.chain.remove(&target_pc);
                jit.stats.invalid_chain_guards += 1;
                jit.stats.patch_misses += 1;
                ChainLookup::Stale
            }
            None => {
                jit.stats.patch_misses += 1;
                ChainLookup::Miss
            }
        }
    }

    /// R4b: install / refresh the chain entry for `target_pc` after a
    /// successful global-cache lookup or compile. Subsequent dispatcher
    /// iterations targeting the same pc will skip the global cache as
    /// long as `ls_hash` continues to match.
    fn chain_install(
        &self,
        target_pc: u32,
        entry_fn: extern "C" fn(*mut JitState) -> u32,
        ls_hash: u64,
        function_size: u64,
    ) {
        let mut jit = self.jit.lock().expect("jit lock poisoned");
        jit.chain.insert(target_pc, ChainEntry { entry_fn, ls_hash, function_size });
    }

    /// Look up the JIT-compiled function for entry PC `pc` against
    /// the current LS contents. On miss, decode + compile + cache.
    /// Returns a stable pointer into the JitCache map (callers must
    /// not hold the cache lock while invoking the function — Cranelift
    /// JIT modules don't relocate code once finalised, so the raw
    /// pointer survives across cache mutations).
    fn compile_or_fetch(
        &self,
        pc: u32,
        ls: &[u8; SPU_LS_SIZE],
    ) -> Result<(*const CompiledFunction, u64), DecodeError> {
        let key = (pc, hash_ls_around(ls.as_ref(), pc));

        // Fast path: cache hit. Take the lock only long enough to
        // bump stats and compute the pointer — release before calling.
        {
            let mut jit = self.jit.lock().expect("jit lock poisoned");
            let cached_ptr = jit.compiled.get(&key).map(|c| c as *const CompiledFunction);
            if let Some(ptr) = cached_ptr {
                jit.stats.cache_hits += 1;
                drop(jit);
                let size = self.function_size(pc, ls);
                return Ok((ptr, size));
            }
        }

        // Decode and compile.
        let func = decode_function(ls, pc, self.max_blocks)?;
        let function_size = func.instruction_count() as u64;

        // R4c: figure out the function's exact byte range. The decoder
        // gives us a graph of basic blocks; the range we care about for
        // SMC detection is `[min(block.start_pc), max(block.end_pc))`.
        // For a contiguous function this is [entry, entry + size*4); for
        // a graph with gaps (rare in SPU homebrew) it may cover a few
        // non-code bytes — that's safe (still detects writes anywhere
        // inside the function), just slightly conservative.
        let (code_start, code_end) = code_range_of(&func);
        let exact_hash = hash_ls_range(ls.as_ref(), code_start, code_end);

        // Cache the decoded function as well (so subsequent diff
        // tooling and the harness can inspect it).
        self.cache.lock().expect("cache lock poisoned").entry(key).or_insert(
            CachedFunction { function: func.clone(), jit_compiled: None }
        );

        // R5.1: count the channel ops that the just-decoded function
        // contains — every one of them passed `supported_check` (else
        // backend.compile would fail), so the count is exactly the
        // number of channel ops the JIT will emit code for.
        let channel_ops_in_func: u64 = func.blocks.values()
            .flat_map(|b| b.instructions.iter())
            .filter(|i| matches!(i.kind, SpuInstKind::Channel { .. }))
            .count() as u64;

        // R5.1: pre-compute "did the function contain any channel op
        // at all?" so we can attribute a compile failure to channels
        // without re-decoding. Used below in the compile-error path.
        let has_channel_op = channel_ops_in_func > 0;

        let mut jit = self.jit.lock().expect("jit lock poisoned");
        jit.stats.cache_misses += 1;
        let compiled = jit.backend.compile(&func).map_err(|_e| {
            // R5.1: bump the channel-attribution stat at compile-failure
            // time. The dispatcher's R5 partial fallback path is the
            // 1:1 consumer of this Err — one Err here means one
            // partial fallback above. If the function carries any
            // channel op (whether that op is the cause or not), we
            // count this as channel-related; coarse but predictable.
            if has_channel_op {
                jit.stats.channel_ops_partial_fallback += 1;
            }
            DecodeError::BadEntryPc(pc)
        })?;
        jit.stats.compiled_functions += 1;
        jit.stats.channel_ops_jitted = jit.stats.channel_ops_jitted
            .saturating_add(channel_ops_in_func);
        let entry = jit.compiled.entry(key).or_insert(compiled);
        let ptr: *const CompiledFunction = entry;
        // R4c: install the code-range metadata used by `smc_scan`.
        jit.compiled_meta.insert(
            key,
            CompiledMeta { code_start, code_end, exact_hash, function_size },
        );
        Ok((ptr, function_size))
    }

    /// R4c: walk every compiled function and check whether its source
    /// bytes still match the hash captured at compile time. Any entry
    /// whose hash diverges is invalidated atomically: it leaves
    /// `compiled`, `compiled_meta`, the decoded `cache`, and (when the
    /// chain still points at the about-to-be-evicted function) the
    /// chain table. Stats:
    ///   - `smc_range_hits`        += entries whose bytes changed
    ///   - `smc_range_misses`      += entries whose bytes were intact
    ///   - `smc_invalidations`     += entries we removed
    ///   - `smc_chain_evictions`   += chain entries dropped because they
    ///                                pointed at a now-evicted function
    ///
    /// Conservative by construction: when the scan can't tell whether
    /// the chain entry corresponded to the invalidated function (e.g.
    /// the chain happens to be empty), we leave the chain alone. The
    /// next `chain_lookup` will catch the issue via its own `ls_hash`
    /// guard, so byte-exactness is preserved either way.
    fn smc_scan(&self, ls: &[u8; SPU_LS_SIZE]) {
        // Phase 1: snapshot stale keys under a short read-style hold.
        let stale: Vec<(CacheKey, extern "C" fn(*mut JitState) -> u32)>;
        let mut range_misses: u64 = 0;
        {
            let jit = self.jit.lock().expect("jit lock poisoned");
            let mut buf = Vec::new();
            for (key, meta) in &jit.compiled_meta {
                let cur = hash_ls_range(ls.as_ref(), meta.code_start, meta.code_end);
                if cur != meta.exact_hash {
                    if let Some(cf) = jit.compiled.get(key) {
                        buf.push((*key, cf.entry_fn()));
                    }
                } else {
                    range_misses += 1;
                }
            }
            stale = buf;
        }

        if stale.is_empty() {
            // Fast path: nothing to invalidate. Just bump the misses.
            if range_misses > 0 {
                let mut jit = self.jit.lock().expect("jit lock poisoned");
                jit.stats.smc_range_misses += range_misses;
            }
            return;
        }

        // Phase 2: drop the stale entries from compiled + meta + chain
        // (where applicable).
        {
            let mut jit = self.jit.lock().expect("jit lock poisoned");
            jit.stats.smc_range_misses += range_misses;
            for (key, evicted_fn) in &stale {
                jit.compiled.remove(key);
                jit.compiled_meta.remove(key);
                jit.stats.smc_invalidations += 1;
                jit.stats.smc_range_hits += 1;
                // Only evict the chain entry if it actually points at
                // the function we just removed. A different (pc, hash)
                // compile may still own the chain slot.
                let evicted_addr = *evicted_fn as usize;
                if let Some(chain_entry) = jit.chain.get(&key.0).copied() {
                    if chain_entry.entry_fn as usize == evicted_addr {
                        jit.chain.remove(&key.0);
                        jit.stats.smc_chain_evictions += 1;
                    }
                }
            }
        }

        // Phase 3: drop the matching decoded-function cache entries —
        // a separate Mutex from `jit`, so we keep this out of the
        // critical section above.
        let mut decoded = self.cache.lock().expect("cache lock poisoned");
        for (key, _) in &stale {
            decoded.remove(key);
        }
    }

    /// Inspect the size of the compiled-meta map. Tests use this to
    /// confirm the SMC scan actually evicted entries.
    #[must_use]
    pub fn compiled_meta_size(&self) -> usize {
        self.jit.lock().expect("jit lock poisoned").compiled_meta.len()
    }

    /// Fast-path size estimate without re-decoding (uses the decode
    /// cache from `decode_with_cache` if available; otherwise re-decodes).
    fn function_size(&self, pc: u32, ls: &[u8; SPU_LS_SIZE]) -> u64 {
        let key = (pc, hash_ls_around(ls.as_ref(), pc));
        if let Some(entry) = self.cache.lock().expect("cache lock poisoned").get(&key) {
            return entry.function.instruction_count() as u64;
        }
        // Should not happen — cache is populated alongside JIT compile.
        decode_function(ls, pc, self.max_blocks)
            .map(|f| f.instruction_count() as u64)
            .unwrap_or(0)
    }

    fn build_result(
        &self,
        state: &mut JitState,
        ls: &[u8; SPU_LS_SIZE],
        channels: &SpuChannels,
        total_steps: u64,
        stop_reason: ExecutionStopReason,
    ) -> SpuExecutionResult {
        // Defensive: null both pointers before returning. The
        // SpuChannels and LS Boxes are owned by `try_jit_run` and will
        // be dropped after this method returns; we don't want the
        // dangling pointers to outlive their backing storage in
        // `state`.
        state.ls_ptr = std::ptr::null_mut();
        state.channels_ptr = std::ptr::null_mut();

        let mut gpr = Box::new([0u128; SPU_GPR_COUNT]);
        for i in 0..SPU_GPR_COUNT {
            gpr[i] = state.load_gpr(i);
        }
        let mut ls_box = Box::new([0u8; SPU_LS_SIZE]);
        ls_box.copy_from_slice(ls.as_ref());

        // R5.3: derive ChannelCounts from the live SpuChannels — same
        // formula as `rpcs3_spu_differential::snapshot_from_thread`.
        // Without this the snapshot would always report all-zero
        // counts even when the JIT helpers mutated mailbox/snr state,
        // breaking byte-exact equivalence vs the interpreter.
        let channel_counts = ChannelCounts {
            in_mbox_depth: channels.in_mbox.is_some() as u32,
            out_mbox_depth: channels.out_mbox.is_some() as u32,
            out_intr_mbox_depth: channels.out_intr_mbox.is_some() as u32,
            signal1_pending: channels.snr[0] != 0,
            signal2_pending: channels.snr[1] != 0,
        };

        SpuExecutionResult {
            steps_executed: total_steps,
            stop_reason,
            final_state: SpuStateSnapshot {
                pc: state.pc,
                gpr,
                ls: ls_box,
                channel_counts,
                // R5.4a: build_result is reached only on clean exits
                // (STOP, MaxStepsExceeded). The JIT itself never parks
                // — parking is a property of the interpreter's
                // ChannelStall path, which routes through
                // `partial_fallback_to_interpreter` and returns the
                // interpreter's own result (with `park_state` already
                // populated by `snapshot_from_thread`). So here it's
                // always None.
                park_state: None,
                // R5.4b: clone the live SpuChannels into the snapshot
                // so callers can drive the wake → resume cycle. Even
                // on clean STOPs the channels may have been mutated
                // by JIT helpers (rdch/wrch/rchcnt) and we need to
                // expose that state.
                channels: channels.clone(),
            },
        }
    }
}

impl SpuExecutor for RecompilerExecutor {
    fn execute(&mut self, program: &SpuProgram) -> SpuExecutionResult {
        if let Err(e) = program.validate() {
            return error_result(e.to_string());
        }

        // Run the dispatcher loop. Returns Some(result) on clean Stop,
        // None when JIT compilation failed (unsupported opcode).
        if let Some(r) = self.try_jit_run(program) {
            return r;
        }

        self.jit.lock().expect("jit lock poisoned").stats.fallback_runs += 1;
        self.interp.execute(program)
    }

    fn backend_name(&self) -> &'static str {
        "recompiler-jit"
    }
}

/// Quick non-cryptographic hash of the 256-byte window around `entry`.
/// Cheap, stable, sufficient for cache invalidation in v0.
fn hash_ls_around(ls: &[u8], entry: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let start = (entry as usize).min(ls.len().saturating_sub(256));
    let end = (start + 256).min(ls.len());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ls[start..end].hash(&mut h);
    h.finish()
}

/// R4c: hash an arbitrary `[start, end)` byte range of LS. Used by
/// the SMC scan to validate that a function's source bytes are still
/// the ones the JIT was compiled from. Bounds-clamped so we never
/// panic on empty/inverted ranges; an empty range hashes the empty
/// slice (consistently zero across calls).
fn hash_ls_range(ls: &[u8], start: u32, end: u32) -> u64 {
    use std::hash::{Hash, Hasher};
    let s = (start as usize).min(ls.len());
    let e = (end as usize).min(ls.len()).max(s);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ls[s..e].hash(&mut h);
    h.finish()
}

/// R5.1: peek at the single SPU instruction at `pc` in `ls` and
/// classify it as "channel op" (rdch / wrch / rchcnt). Returns
/// `false` if the read would go out of bounds, decode produces
/// anything other than `SpuInstKind::Channel { .. }`, or `pc` is
/// misaligned. Cheap — used only on the partial-fallback slow path
/// to attribute the cause.
fn decode_inst_at(ls: &[u8; SPU_LS_SIZE], pc: u32) -> bool {
    let p = pc as usize;
    if p + 4 > SPU_LS_SIZE || pc & 0x3 != 0 {
        return false;
    }
    let raw = u32::from_be_bytes([ls[p], ls[p + 1], ls[p + 2], ls[p + 3]]);
    matches!(decode_inst(raw, pc).kind, SpuInstKind::Channel { .. })
}

/// R4c: compute the byte range covered by every block in `func`.
/// Returns `(min_block.start_pc, max_block.end_pc)`. Empty function
/// would map to `(entry, entry)` — degenerate but harmless to hash.
fn code_range_of(func: &SpuFunction) -> (u32, u32) {
    let mut start = func.entry;
    let mut end = func.entry;
    let mut seen = false;
    for block in func.blocks.values() {
        if !seen {
            start = block.start_pc;
            end = block.end_pc;
            seen = true;
        } else {
            if block.start_pc < start { start = block.start_pc; }
            if block.end_pc > end { end = block.end_pc; }
        }
    }
    (start, end)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rpcs3_spu_differential::{diff_snapshots, run_and_diff};

    /// `il rt, imm; stop 0` as a single segment.
    fn il_stop_program(rt: u32, imm: i16) -> SpuProgram {
        let il = ((0x081u32 & 0x1FF) << 23) | ((imm as u16 as u32 & 0xFFFF) << 7) | (rt & 0x7F);
        let stop = 0u32;
        let mut bytes = Vec::with_capacity(8);
        bytes.extend_from_slice(&il.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn recompiler_executes_il_stop_via_interpreter_delegate() {
        let mut exec = RecompilerExecutor::new();
        let r = exec.execute(&il_stop_program(3, 0x1234));
        assert_eq!(r.steps_executed, 2);
        assert_eq!(r.stop_reason, ExecutionStopReason::Stop(0));
        assert_eq!(r.final_state.gpr[3], 0x00001234_00001234_00001234_00001234);
    }

    #[test]
    fn recompiler_caches_decoded_functions() {
        let mut exec = RecompilerExecutor::new();
        assert_eq!(exec.function_cache_size(), 0);
        exec.execute(&il_stop_program(3, 0));
        assert_eq!(exec.function_cache_size(), 1);
        // Same program → cache hit, no new entry.
        exec.execute(&il_stop_program(3, 0));
        assert_eq!(exec.function_cache_size(), 1);
        // Different program at the same entry_pc → R4c's SMC scan
        // detects the prior compile is stale (different bytes occupy
        // the same address), evicts it, then the new program compiles
        // fresh. Net cache size stays at 1 — that's R4c hygiene, not
        // a regression.
        exec.execute(&il_stop_program(5, 0));
        assert_eq!(exec.function_cache_size(), 1,
                   "R4c: stale entry from prog #1 must be evicted before prog #2 compiles");
        let stats = exec.jit_stats();
        assert!(stats.smc_invalidations >= 1,
                "R4c smc_scan must have invalidated the prior compile");
    }

    #[test]
    fn clear_cache_drops_entries() {
        let mut exec = RecompilerExecutor::new();
        exec.execute(&il_stop_program(3, 0));
        assert_eq!(exec.function_cache_size(), 1);
        exec.clear_function_cache();
        assert_eq!(exec.function_cache_size(), 0);
    }

    #[test]
    fn recompiler_byte_matches_interpreter_for_il_stop() {
        let prog = il_stop_program(7, 0x55);
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (a, b, diff) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert_eq!(a.stop_reason, b.stop_reason);
        assert_eq!(a.steps_executed, b.steps_executed);
        assert!(diff.is_identical(), "diff: {diff:?}");
    }

    #[test]
    fn recompiler_byte_matches_interpreter_across_three_programs() {
        let progs = [
            il_stop_program(1, -1),
            il_stop_program(15, 0x7FFF),
            il_stop_program(127, 0),
        ];
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for p in &progs {
            let r1 = interp.execute(p);
            let r2 = recomp.execute(p);
            let d = diff_snapshots(&r1.final_state, &r2.final_state);
            assert!(d.is_identical(), "program {:?} diff: {d:?}", p.entry_pc);
        }
    }

    #[test]
    fn invalid_program_propagates_as_error() {
        let mut exec = RecompilerExecutor::new();
        let bad = SpuProgram::new(0x101, 10); // unaligned entry
        let r = exec.execute(&bad);
        match r.stop_reason {
            ExecutionStopReason::Error(_) => {}
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn backend_name_is_distinct_from_interpreter() {
        let exec = RecompilerExecutor::new();
        assert_ne!(exec.backend_name(), "interpreter");
        assert_eq!(exec.backend_name(), "recompiler-jit");
    }

    #[test]
    fn jit_stats_track_runs_and_compilations() {
        let mut exec = RecompilerExecutor::new();
        let stats0 = exec.jit_stats();
        assert_eq!(stats0, JitStats::default());

        // First run of il_stop → JIT-compilable, runs via JIT.
        exec.execute(&il_stop_program(3, 0x1234));
        let stats1 = exec.jit_stats();
        assert_eq!(stats1.compiled_functions, 1);
        assert_eq!(stats1.jit_runs, 1);
        assert_eq!(stats1.fallback_runs, 0);

        // Second run of same program → JIT cache hit, no new compile.
        exec.execute(&il_stop_program(3, 0x1234));
        let stats2 = exec.jit_stats();
        assert_eq!(stats2.compiled_functions, 1, "should not recompile");
        assert_eq!(stats2.jit_runs, 2);
    }

    /// Build the full `synthetic_loop.elf` program inline (no ELF
    /// parsing) and assert it runs entirely through the JIT —
    /// proving the multi-block compile path (entry block → loop
    /// body → conditional brnz → back-edge br → exit stop) all
    /// works end-to-end.
    fn loop_program() -> SpuProgram {
        let il    = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let ila   = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let a     = |rt: u32, ra: u32, rb: u32| (0x0C0u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ai    = |rt: u32, ra: u32, imm: u32| (0x1Cu32 << 24) | ((imm & 0x3FF) << 14) | (ra << 7) | rt;
        let ceqi  = |rt: u32, ra: u32, imm: u32| (0x7Cu32 << 24) | ((imm & 0x3FF) << 14) | (ra << 7) | rt;
        let brnz  = |rt: u32, off: u32| (0x042u32 << 23) | ((off & 0xFFFF) << 7) | rt;
        let br    = |off: u32| (0x064u32 << 23) | ((off & 0xFFFF) << 7);
        let _ = il;

        let code: [u32; 8] = [
            ila(3, 0),
            ila(4, 1),
            a(3, 3, 4),
            ai(4, 4, 1),
            ceqi(5, 4, 11),
            brnz(5, 2),
            br(((-4i32) as u32) & 0xFFFF),
            0u32, // stop 0x55? No, that's stop 0. Adjust:
        ];
        let mut bytes = Vec::with_capacity(8 * 4);
        for w in code.iter().take(7) {
            bytes.extend_from_slice(&w.to_be_bytes());
        }
        let stop = 0x55u32 & 0x3FFF; // primary 0x000, code 0x55
        bytes.extend_from_slice(&stop.to_be_bytes());

        SpuProgram::new(0x100, 1000).with_segment(0x100, bytes)
    }

    #[test]
    fn jit_runs_full_loop_program_no_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&loop_program());

        // Should be a clean Stop(0x55) with r3 = 55 broadcast.
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0x55));
        assert_eq!(result.final_state.gpr[3],
                   0x00000037_00000037_00000037_00000037);

        // Crucially: every step happened via the JIT, no interpreter
        // fallback was needed.
        let stats = exec.jit_stats();
        assert_eq!(stats.compiled_functions, 1, "expected 1 compiled function");
        assert_eq!(stats.jit_runs, 1, "expected JIT to handle the run");
        assert_eq!(stats.fallback_runs, 0, "expected NO interpreter fallback");
    }

    #[test]
    fn jit_runs_full_loop_program_byte_identical_to_interpreter() {
        let prog = loop_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (a, b, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert_eq!(a.stop_reason, b.stop_reason);
        assert!(d.is_identical(),
                "JIT diverged from interpreter on loop:\n{d:?}");
    }

    /// Mirrors `synthetic_arith.elf`: exercises il, a, sf, shli,
    /// xor, or, and. Now that the JIT supports shli + the RR
    /// bitwise family, this should run 100% through the JIT.
    fn arith_program() -> SpuProgram {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let a    = |rt: u32, ra: u32, rb: u32| (0x0C0u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let sf   = |rt: u32, ra: u32, rb: u32| (0x040u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let and  = |rt: u32, ra: u32, rb: u32| (0x0C1u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let or   = |rt: u32, ra: u32, rb: u32| (0x041u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let xor  = |rt: u32, ra: u32, rb: u32| (0x241u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let shli = |rt: u32, ra: u32, imm7: u32| (0x07Bu32 << 21) | ((imm7 & 0x7F) << 14) | (ra << 7) | rt;
        let stop = 0x0042u32 & 0x3FFF;

        let code: [u32; 9] = [
            il(3, 5),
            il(4, 3),
            a(5, 3, 4),
            sf(6, 4, 3),  // sf rt,ra,rb = rb-ra → r3-r4 = 2
            shli(7, 3, 2),
            xor(8, 3, 4),
            or(9, 3, 4),
            and(10, 3, 4),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn jit_runs_full_arith_program_no_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&arith_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0x42));

        // Verify each ALU op landed in its expected register.
        assert_eq!(result.final_state.gpr[5],  0x00000008_00000008_00000008_00000008); // 5+3
        assert_eq!(result.final_state.gpr[6],  0x00000002_00000002_00000002_00000002); // 5-3
        assert_eq!(result.final_state.gpr[7],  0x00000014_00000014_00000014_00000014); // 5<<2
        assert_eq!(result.final_state.gpr[8],  0x00000006_00000006_00000006_00000006); // 5^3
        assert_eq!(result.final_state.gpr[9],  0x00000007_00000007_00000007_00000007); // 5|3
        assert_eq!(result.final_state.gpr[10], 0x00000001_00000001_00000001_00000001); // 5&3

        // Crucially: every step happened via the JIT.
        let stats = exec.jit_stats();
        assert_eq!(stats.compiled_functions, 1);
        assert_eq!(stats.jit_runs, 1);
        assert_eq!(stats.fallback_runs, 0,
                   "synthetic_arith should be 100% JIT-compilable now");
    }

    #[test]
    fn jit_runs_full_arith_program_byte_identical_to_interpreter() {
        let prog = arith_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(), "JIT diverged from interpreter on arith:\n{d:?}");
    }

    /// R5.10o — end-to-end regression for LQA + STQA via partial
    /// fallback. The JIT has no codegen for the absolute-address
    /// `0x041`/`0x061` primaries (the SpuInstKind is the new
    /// `LoadAbs`/`StoreAbs` variant, which hits the JIT's wildcard
    /// `_ =>` arm in supported_check). Both opcodes route through R5
    /// partial fallback to the interpreter. This test asserts that
    /// the JIT and interpreter end up byte-identical after a full
    /// store-then-load round-trip at the top of LS — the same
    /// save/restore pattern observed in the v4 trace at pc=0x734
    /// (STQA r9) and pc=0x07AC..0x0824 (LQA r4/r38/etc).
    #[test]
    fn jit_lqa_stqa_byte_identical_to_interpreter_via_partial_fallback() {
        let il    = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        // RI16 abs store: primary 0x041 (top-9), imm16 at bits 7..22.
        let stqa = |rt: u32, imm16: i16| -> u32 {
            let i = (imm16 as u16 as u32) & 0xFFFF;
            (0x041u32 << 23) | (i << 7) | (rt & 0x7F)
        };
        // RI16 abs load: primary 0x061.
        let lqa = |rt: u32, imm16: i16| -> u32 {
            let i = (imm16 as u16 as u32) & 0xFFFF;
            (0x061u32 << 23) | (i << 7) | (rt & 0x7F)
        };
        let stop  = 0x0055u32;

        // il r3, 0x55AA               → r3 = 0x000055AA broadcast across 4 lanes.
        // stqa r3, -12 (target 0x3FFD0) → write r3 to top-of-LS.
        // lqa  r4, -12                → load same 16 bytes back into r4.
        // stop 0x55.
        // Post-execution invariant: r4 == r3 (round-trip).
        let code: [u32; 4] = [
            il(3, 0x55AA),
            stqa(3, -12),
            lqa(4, -12),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (a, b, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert_eq!(a.stop_reason, b.stop_reason);
        assert_eq!(a.stop_reason, ExecutionStopReason::Stop(0x55));
        assert!(
            d.is_identical(),
            "LQA/STQA JIT-vs-interpreter divergence (the JIT must \
             partial-fallback correctly to the new interpreter arms):\n{d:?}",
        );

        // Round-trip invariant: r4 == r3 == [0,0,0x55,0xAA] per lane.
        let expected: u128 = (0x000055AAu128 << 96)
                           | (0x000055AAu128 << 64)
                           | (0x000055AAu128 << 32)
                           | 0x000055AAu128;
        assert_eq!(a.final_state.gpr[3], expected, "il r3, 0x55AA");
        assert_eq!(a.final_state.gpr[4], expected, "lqa→stqa round-trip preserves bytes");
    }

    /// R5.10m — end-to-end regression for ROTQMBYI through partial
    /// fallback. The JIT has no codegen for `0x1FD`, so it marks the
    /// instruction Unsupported and the dispatcher hands control to
    /// the interpreter at the same pc. The interpreter's new R5.10m
    /// arm produces the correct byte-shift-right result. This test
    /// guards (a) the partial fallback path stays alive for ROTQMBYI,
    /// AND (b) the interpreter matches the expected mask result on a
    /// non-trivial input.
    #[test]
    fn jit_rotqmbyi_byte_identical_to_interpreter() {
        let il    = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        // rotqmbyi rt, ra, imm7: primary 0x1FD at MSB-0 bits 0..10.
        let rotqmbyi = |rt: u32, ra: u32, imm7: i8| -> u32 {
            let imm = (imm7 as u8 as u32) & 0x7F;
            (0x1FDu32 << 21) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
        };
        let stop  = 0x0055u32;

        // il r3, 0x1234 → r3 = 0x00001234 broadcast across 4 lanes.
        // rotqmbyi r4, r3, -4 → r4 = r3 >> 32 bits (= shift right 4
        //   bytes in SPU BE) with zero fill. For our broadcast input
        //   each byte pattern repeats; result is non-trivial enough to
        //   detect a wrong shift count.
        let code: [u32; 3] = [
            il(3, 0x1234),
            rotqmbyi(4, 3, -4),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (a, b, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert_eq!(a.stop_reason, b.stop_reason);
        assert_eq!(a.stop_reason, ExecutionStopReason::Stop(0x55));
        assert!(
            d.is_identical(),
            "ROTQMBYI JIT/interpreter divergence (the JIT must \
             partial-fallback correctly to the new interpreter arm):\n{d:?}",
        );

        // Spot-check the expected r4 byte pattern: r3 = 0x00001234 in
        // each word lane → all 16 bytes of r3 are
        // [0,0,0x12,0x34, 0,0,0x12,0x34, ...]. Right-shift by 4 bytes
        // (zero-fill) → 4 leading zeros, then bytes 0..11 of original.
        let r3_bytes = a.final_state.gpr[3].to_be_bytes();
        let expected_r4 = {
            let mut e = [0u8; 16];
            for i in 4..16 { e[i] = r3_bytes[i - 4]; }
            u128::from_be_bytes(e)
        };
        assert_eq!(a.final_state.gpr[4], expected_r4, "ROTQMBYI mask wrong");
    }

    /// R5.10m — end-to-end regression for SHLQBYI at the corrected
    /// primary `0x1FF`. Pre-R5.10m the encoder packed `0x1FB` (which
    /// is SHLQBII) and the interpreter handled `0x1FB` as byte-shift,
    /// so the two layers silently agreed on a wrong-vs-RPCS3 wire
    /// format. Post-R5.10m: encoder packs `0x1FF`, interpreter handles
    /// `0x1FF` as byte-shift, JIT codegen at `0x1FF` already does
    /// byte-shift — all three layers now agree on the correct primary.
    /// This test exercises the full pipeline.
    #[test]
    fn jit_shlqbyi_byte_identical_to_interpreter_at_primary_0x1ff() {
        let il    = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let shlqbyi = |rt: u32, ra: u32, imm7: i8| -> u32 {
            let imm = (imm7 as u8 as u32) & 0x7F;
            (0x1FFu32 << 21) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
        };
        let stop  = 0x0055u32;

        // il r3, 0x00FF → r3 = 0x000000FF broadcast.
        // shlqbyi r4, r3, 3 → byte-shift left by 3 bytes (zero-fill
        //   right tail).
        let code: [u32; 3] = [
            il(3, 0x00FF),
            shlqbyi(4, 3, 3),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(
            d.is_identical(),
            "SHLQBYI at primary 0x1FF must produce identical results \
             via JIT codegen and interpreter:\n{d:?}",
        );
    }

    /// R5.10k — end-to-end regression for the wider RI10 Class-A
    /// subfamily. Runs `il r3, 32; clgti r4, r3, 31; sfi r5, r3, 100;
    /// ahi r6, r3, -3; mpyi r7, r3, 7; mpyui r8, r3, 7; stop 0x55`
    /// via BOTH backends and asserts byte-identical state.
    ///
    /// Pre-R5.10k the interpreter had no arm for any of these 5
    /// opcodes and would fail with `Unimplemented`, while the JIT
    /// silently computed correct results — the diff would be a
    /// run-finish vs run-error mismatch, not a value mismatch. This
    /// test guards both: (a) interpreter now executes them, AND
    /// (b) the values it produces match the JIT byte-for-byte.
    #[test]
    fn jit_class_a_ri10_byte_identical_to_interpreter() {
        let il    = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        // Word-imm RI10 packer. imm10 sign-extended; only low 10 bits
        // of the 16-bit immediate matter.
        let pack8 = |p8: u32, rt: u32, ra: u32, imm10: i16| -> u32 {
            let imm = (imm10 as u32) & 0x3FF;
            ((p8 & 0xFF) << 24) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
        };
        let clgti = |rt: u32, ra: u32, imm: i16| pack8(0x5C, rt, ra, imm);
        let sfi   = |rt: u32, ra: u32, imm: i16| pack8(0x0C, rt, ra, imm);
        let ahi   = |rt: u32, ra: u32, imm: i16| pack8(0x1D, rt, ra, imm);
        let mpyi  = |rt: u32, ra: u32, imm: i16| pack8(0x74, rt, ra, imm);
        let mpyui = |rt: u32, ra: u32, imm: i16| pack8(0x75, rt, ra, imm);
        let stop  = 0x0055u32;

        let code: [u32; 7] = [
            il(3, 32),         // r3 = 0x20 broadcast (lanes = 32)
            clgti(4, 3, 31),   // r4 = 0xFFFFFFFF per lane (32 > 31 unsigned)
            sfi(5, 3, 100),    // r5 = (100 - 32) = 68 per lane
            ahi(6, 3, -3),     // r6 = halfword(0x0020 + 0xFFFD) = 0x001D per halfword
            mpyi(7, 3, 7),     // r7 = (low_i16(32) * 7) = 224 per word
            mpyui(8, 3, 7),    // r8 = (low_u16(32) * 7) = 224 per word
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (a, b, d) = run_and_diff(&mut interp, &mut recomp, &prog);

        assert_eq!(a.stop_reason, b.stop_reason);
        assert_eq!(a.stop_reason, ExecutionStopReason::Stop(0x55));
        assert!(
            d.is_identical(),
            "Class-A RI10 subfamily JIT/interpreter divergence:\n{d:?}",
        );

        // Spot-check expected lane values to lock semantics.
        assert_eq!(a.final_state.gpr[4], u128::from_be_bytes([0xFF; 16]),
                   "clgti 32 > 31 unsigned must be all-ones");
        // Each word lane = 68. In SPU big-endian: bytes [0,0,0,68] per word.
        let expect_5: u128 = (68u128 << 96) | (68u128 << 64) | (68u128 << 32) | 68u128;
        assert_eq!(a.final_state.gpr[5], expect_5, "sfi 100 - 32 = 68");
        // mpyi 32*7 = 224 per word. mpyui same value (32 fits in low u16).
        let expect_word_224: u128 = (224u128 << 96) | (224u128 << 64) | (224u128 << 32) | 224u128;
        assert_eq!(a.final_state.gpr[7], expect_word_224, "mpyi 32*7 = 224");
        assert_eq!(a.final_state.gpr[8], expect_word_224, "mpyui 32*7 = 224");
    }

    /// R5.10k — separate JIT differential test for MPYI/MPYUI
    /// signedness divergence: when low halfword has high bit set,
    /// signed and unsigned variants must produce DIFFERENT results
    /// AND each backend must agree with the other on its variant's
    /// result. Pre-R5.10k the interpreter rejected both, so any
    /// codegen bug in the JIT for these specific paths was invisible.
    #[test]
    fn jit_mpyi_vs_mpyui_signedness_byte_identical_to_interpreter() {
        let il    = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let pack8 = |p8: u32, rt: u32, ra: u32, imm10: i16| -> u32 {
            let imm = (imm10 as u32) & 0x3FF;
            ((p8 & 0xFF) << 24) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
        };
        let mpyi  = |rt: u32, ra: u32, imm: i16| pack8(0x74, rt, ra, imm);
        let mpyui = |rt: u32, ra: u32, imm: i16| pack8(0x75, rt, ra, imm);
        let stop  = 0x0055u32;

        // r3 = 0xFFFF broadcast → low halfword of each word lane is
        // 0xFFFF (= -1 signed, 65535 unsigned).
        let code: [u32; 4] = [
            il(3, 0xFFFF),
            mpyi(4, 3, 3),     // signed: -1 * 3 = -3 = 0xFFFFFFFD
            mpyui(5, 3, 3),    // unsigned: 65535 * 3 = 196605 = 0x0002FFFD
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (a, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(), "mpyi/mpyui differential failed:\n{d:?}");

        let expect_signed:   u128 = (0xFFFFFFFDu128 << 96) | (0xFFFFFFFDu128 << 64) | (0xFFFFFFFDu128 << 32) | 0xFFFFFFFDu128;
        let expect_unsigned: u128 = (0x0002FFFDu128 << 96) | (0x0002FFFDu128 << 64) | (0x0002FFFDu128 << 32) | 0x0002FFFDu128;
        assert_eq!(a.final_state.gpr[4], expect_signed,   "mpyi -1*3 = -3");
        assert_eq!(a.final_state.gpr[5], expect_unsigned, "mpyui 65535*3 = 196605");
    }

    /// R5.10i — end-to-end regression for the decoder i8 extraction
    /// fix. Runs `il r3, 0xFFFF; andbi r4, r3, 0x20; stop 0x55` via
    /// BOTH backends and asserts byte-identical state.
    ///
    /// Before R5.10i the decoder extracted i8 from bits 16..23 instead
    /// of bits 14..21 — for `andbi rt, ra, 0x20` the JIT received
    /// `imm10 = 0x08` and produced `r4 = 0x08…08` per byte, while the
    /// interpreter (after R5.10i) computes `r4 = 0x20…20`. The
    /// diff would surface as a 16-byte mismatch in gpr[4]. This test
    /// guards the coupled decoder fix + interpreter arm.
    #[test]
    fn jit_andbi_byte_identical_to_interpreter_with_nonzero_i8() {
        // il    rt=3, imm=0xFFFF                → r3 = 0xFFFFFFFF broadcast
        // andbi rt=4, ra=3, i8=0x20             → r4 = 0x20 in every byte
        // stop  0x55
        let il    = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        // andbi: 8-bit primary 0x16, i8 in bits 14..21, ra in 7..13, rt in 0..6.
        let andbi = |rt: u32, ra: u32, i8: u32| (0x16u32 << 24) | ((i8 & 0xFF) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F);
        let stop  = 0x0055u32;

        let code: [u32; 3] = [
            il(3, 0xFFFF),
            andbi(4, 3, 0x20),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (a, b, d) = run_and_diff(&mut interp, &mut recomp, &prog);

        // Both backends must reach the same stop code.
        assert_eq!(a.stop_reason, b.stop_reason);
        assert_eq!(a.stop_reason, ExecutionStopReason::Stop(0x55));

        // The byte-exact result must match: r4 = [0x20; 16] per RPCS3.
        // Pre-R5.10i this would fail because the JIT used i8=0x08.
        assert!(
            d.is_identical(),
            "JIT diverged from interpreter on byte-imm with non-zero i8 — \
             this means the decoder i8 extraction or the interpreter byte-imm \
             arm regressed:\n{d:?}",
        );
        assert_eq!(
            a.final_state.gpr[4],
            u128::from_be_bytes([0x20; 16]),
            "andbi r4, r3, 0x20 must mask each byte to 0x20",
        );
    }

    /// Mirrors `synthetic_loadstore.elf`: round-trip through LS via stqd+lqd.
    fn loadstore_program() -> SpuProgram {
        let il = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let ila = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let stqd = |rt: u32, ra: u32, imm10: u32|
            (0x24u32 << 24) | ((imm10 & 0x3FF) << 14) | (ra << 7) | rt;
        let lqd = |rt: u32, ra: u32, imm10: u32|
            (0x34u32 << 24) | ((imm10 & 0x3FF) << 14) | (ra << 7) | rt;
        let stop = 0xABu32 & 0x3FFF;

        let code: [u32; 5] = [
            il(3, 0x5A5A),
            ila(4, 0x40),
            stqd(3, 4, 1),     // stqd r3 → LSA 0x50
            lqd(5, 4, 1),      // lqd r5 ← LSA 0x50
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn jit_runs_full_loadstore_program_no_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&loadstore_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0xAB));
        // r5 = 0x5A5A broadcast, loaded back from the LS write.
        assert_eq!(result.final_state.gpr[5],
                   0x00005A5A_00005A5A_00005A5A_00005A5A);
        // The LS at 0x50 should have the stored qword.
        let stored = u128::from_be_bytes(
            result.final_state.ls[0x50..0x60].try_into().unwrap()
        );
        assert_eq!(stored, 0x00005A5A_00005A5A_00005A5A_00005A5A);

        let stats = exec.jit_stats();
        assert_eq!(stats.compiled_functions, 1);
        assert_eq!(stats.jit_runs, 1);
        assert_eq!(stats.fallback_runs, 0,
                   "loadstore should be 100% JIT-compilable now");
    }

    #[test]
    fn jit_runs_full_loadstore_program_byte_identical_to_interpreter() {
        let prog = loadstore_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(), "JIT diverged from interpreter on loadstore:\n{d:?}");
    }

    /// Mirrors `synthetic_orx_collapse.elf`: il + il + ah + orx + stop.
    fn orx_collapse_program() -> SpuProgram {
        let il  = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let ah  = |rt: u32, ra: u32, rb: u32| (0x0C8u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let orx = |rt: u32, ra: u32| (0x1F0u32 << 21) | (ra << 7) | rt;
        let stop = 0xCCu32 & 0x3FFF;
        let code: [u32; 5] = [il(3, 0x1234), il(4, 0x5678), ah(5, 3, 4), orx(6, 5), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn jit_runs_full_orx_collapse_program_no_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&orx_collapse_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0xCC));
        // r5 = 0x000068AC × 4; orx ORs all word lanes → 0x000068AC in lane 0.
        assert_eq!(result.final_state.gpr[6],
                   0x000068AC_00000000_00000000_00000000);
        let stats = exec.jit_stats();
        assert_eq!(stats.fallback_runs, 0,
                   "orx_collapse should be 100% JIT-compilable now");
    }

    #[test]
    fn jit_runs_full_orx_collapse_byte_identical_to_interpreter() {
        let prog = orx_collapse_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(), "JIT diverged from interpreter on orx_collapse:\n{d:?}");
    }

    /// Mirrors `synthetic_halfword_shifts.elf`: ilh + shlhi + rothmi + rothi + stop.
    fn halfword_shifts_program() -> SpuProgram {
        let ilh   = |rt: u32, imm: u16| ((0x083u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let shlhi = |rt: u32, ra: u32, imm7: u32| (0x07Fu32 << 21) | ((imm7 & 0x7F) << 14) | (ra << 7) | rt;
        let rothmi = |rt: u32, ra: u32, imm7: i8| (0x07Du32 << 21) | (((imm7 as u32) & 0x7F) << 14) | (ra << 7) | rt;
        let rothi = |rt: u32, ra: u32, imm7: u32| (0x07Cu32 << 21) | ((imm7 & 0x7F) << 14) | (ra << 7) | rt;
        let stop = 0xDDu32 & 0x3FFF;
        let code: [u32; 5] = [
            ilh(3, 0x00FF),       // r3: each halfword = 0x00FF
            shlhi(4, 3, 4),       // r4 each halfword = 0x00FF << 4 = 0x0FF0
            rothmi(5, 3, -4),     // r5 each halfword = 0x00FF >> 4 = 0x000F
            rothi(6, 3, 8),       // r6 each halfword = rotate-left 0x00FF by 8 = 0xFF00
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn jit_runs_full_halfword_shifts_no_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&halfword_shifts_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0xDD));
        // Each lane = 0x0FF0_0FF0 (both halfwords = 0x0FF0).
        assert_eq!(result.final_state.gpr[4],
                   0x0FF00FF0_0FF00FF0_0FF00FF0_0FF00FF0);
        assert_eq!(result.final_state.gpr[5],
                   0x000F000F_000F000F_000F000F_000F000F);
        assert_eq!(result.final_state.gpr[6],
                   0xFF00FF00_FF00FF00_FF00FF00_FF00FF00);
        let stats = exec.jit_stats();
        assert_eq!(stats.fallback_runs, 0,
                   "halfword_shifts should be 100% JIT-compilable now");
    }

    #[test]
    fn jit_runs_halfword_shifts_byte_identical_to_interpreter() {
        let prog = halfword_shifts_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(), "JIT diverged from interpreter on halfword_shifts:\n{d:?}");
    }

    /// Mirrors `synthetic_float_dot.elf`: chain of fa+fm producing 8.0.
    fn float_dot_program() -> SpuProgram {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let shli = |rt: u32, ra: u32, imm7: u32| (0x07Bu32 << 21) | ((imm7 & 0x7F) << 14) | (ra << 7) | rt;
        let a    = |rt: u32, ra: u32, rb: u32| (0x0C0u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let fa   = |rt: u32, ra: u32, rb: u32| (0x2C4u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let fm   = |rt: u32, ra: u32, rb: u32| (0x2C6u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let stop = 0x66u32 & 0x3FFF;
        let code: [u32; 7] = [
            il(3, 0x4000),    // r3 each lane = 0x00004000
            shli(3, 3, 16),   // r3 each lane = 0x40000000 (= float 2.0)
            a(4, 3, 0),       // r4 = r3 (r0 is always 0 by SPU convention)
            fa(5, 3, 4),      // r5 = 2.0 + 2.0 = 4.0
            fm(6, 3, 4),      // r6 = 2.0 * 2.0 = 4.0
            fa(7, 5, 6),      // r7 = 4.0 + 4.0 = 8.0 (= 0x41000000)
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn jit_runs_full_float_dot_no_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&float_dot_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0x66));
        // r7 = 8.0 (= 0x41000000) broadcast.
        assert_eq!(result.final_state.gpr[7],
                   0x41000000_41000000_41000000_41000000);
        let stats = exec.jit_stats();
        assert_eq!(stats.fallback_runs, 0,
                   "float_dot should be 100% JIT-compilable now");
    }

    #[test]
    fn jit_runs_float_dot_byte_identical_to_interpreter() {
        let prog = float_dot_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(), "JIT diverged from interpreter on float_dot:\n{d:?}");
    }

    /// Fibonacci computed via SPU instructions. Same shape as a real
    /// inner loop: counter + ceqi + brnz exit condition + 3 register
    /// moves + back-edge br. After 10 iterations r3 holds fib(10) = 55.
    ///
    /// Used both for "is the JIT correct on a non-trivial program?"
    /// and for the benchmark below.
    fn fibonacci_program() -> SpuProgram {
        let ila  = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let a    = |rt: u32, ra: u32, rb: u32| (0x0C0u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ai   = |rt: u32, ra: u32, imm: u32| (0x1Cu32 << 24) | ((imm & 0x3FF) << 14) | (ra << 7) | rt;
        let ceqi = |rt: u32, ra: u32, imm: u32| (0x7Cu32 << 24) | ((imm & 0x3FF) << 14) | (ra << 7) | rt;
        let brnz = |rt: u32, off: u32| (0x042u32 << 23) | ((off & 0xFFFF) << 7) | rt;
        let br   = |off: u32| (0x064u32 << 23) | ((off & 0xFFFF) << 7);
        let stop = 0xFBu32 & 0x3FFF;

        // Layout (PC offsets from entry 0x100):
        //   0x100 ila r3, 0          ; current
        //   0x104 ila r4, 1          ; next
        //   0x108 ila r5, 0          ; counter
        //   0x10C: loop top
        //   0x10C ceqi r7, r5, 10    ; r7 = (counter == 10) mask
        //   0x110 brnz r7, exit      ; +5 words → 0x124
        //   0x114 a r8, r3, r4       ; tmp = current + next
        //   0x118 a r3, r4, r0       ; current = next
        //   0x11C a r4, r8, r0       ; next = tmp
        //   0x120 ai r5, r5, 1       ; counter++
        //   *missing back-branch at 0x124, fixme below*
        //
        // br -6 from 0x124 lands at 0x10C (loop top). We need the br
        // BEFORE the stop. So:
        //   0x124 br -6              ; back to loop top 0x10C
        //   0x128 stop 0xFB
        // brnz exit needs to skip the br: brnz r7, +5 word jumps to 0x124+0=0x128? Let me re-count.
        //   0x110 brnz r7, X → target = 0x110 + X*4
        //   We want target = 0x128 → X = (0x128 - 0x110) / 4 = 0x18 / 4 = 6.
        //
        // Let me re-layout with brnz +6:
        let code: [u32; 11] = [
            ila(3, 0),                  // 0x100  current = 0
            ila(4, 1),                  // 0x104  next = 1
            ila(5, 0),                  // 0x108  counter = 0
            ceqi(7, 5, 10),             // 0x10C  loop top: r7 = (counter==10)
            brnz(7, 6),                 // 0x110  if r7 ≠ 0 → exit at 0x128
            a(8, 3, 4),                 // 0x114  tmp = current + next
            a(3, 4, 0),                 // 0x118  current = next  (r0 = 0)
            a(4, 8, 0),                 // 0x11C  next = tmp
            ai(5, 5, 1),                // 0x120  counter++
            br(((-6i32) as u32) & 0xFFFF), // 0x124  back to 0x10C
            stop,                       // 0x128  exit
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 1000).with_segment(0x100, bytes)
    }

    #[test]
    fn jit_runs_full_fibonacci_no_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&fibonacci_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0xFB));
        // fib(10) = 55. Each lane should hold 55 (broadcast).
        assert_eq!(
            result.final_state.gpr[3],
            0x00000037_00000037_00000037_00000037,
            "fib(10) should be 55"
        );
        let stats = exec.jit_stats();
        assert_eq!(stats.fallback_runs, 0,
                   "fibonacci should be 100% JIT-compilable");
    }

    #[test]
    fn jit_runs_fibonacci_byte_identical_to_interpreter() {
        let prog = fibonacci_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(),
                "JIT diverged from interpreter on fibonacci:\n{d:?}");
    }

    /// Sum of squares: 1² + 2² + ... + 10² = 385. Exercises mpyu in a
    /// real loop. r3 holds the accumulator; r4 the counter.
    fn sum_of_squares_program() -> SpuProgram {
        let ila  = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let a    = |rt: u32, ra: u32, rb: u32| (0x0C0u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ai   = |rt: u32, ra: u32, imm: u32| (0x1Cu32 << 24) | ((imm & 0x3FF) << 14) | (ra << 7) | rt;
        let ceqi = |rt: u32, ra: u32, imm: u32| (0x7Cu32 << 24) | ((imm & 0x3FF) << 14) | (ra << 7) | rt;
        let mpyu = |rt: u32, ra: u32, rb: u32| (0x3CCu32 << 21) | (rb << 14) | (ra << 7) | rt;
        let brnz = |rt: u32, off: u32| (0x042u32 << 23) | ((off & 0xFFFF) << 7) | rt;
        let br   = |off: u32| (0x064u32 << 23) | ((off & 0xFFFF) << 7);
        let stop = 0xAAu32 & 0x3FFF;

        // Layout (PC offsets from entry 0x100):
        //   0x100 ila r3, 0          ; sum
        //   0x104 ila r4, 1          ; counter i = 1
        //   0x108: loop top
        //   0x108 ceqi r5, r4, 11    ; r5 = (i == 11)
        //   0x10C brnz r5, exit      ; → 0x120 = +5 words from 0x10C
        //   0x110 mpyu r6, r4, r4    ; r6 = i × i (per lane)
        //   0x114 a r3, r3, r6       ; sum += r6
        //   0x118 ai r4, r4, 1       ; i++
        //   0x11C br -5              ; → 0x108
        //   0x120 stop 0xAA
        let code: [u32; 9] = [
            ila(3, 0),
            ila(4, 1),
            ceqi(5, 4, 11),                  // 0x108
            brnz(5, 5),                      // 0x10C  → 0x120
            mpyu(6, 4, 4),                   // 0x110
            a(3, 3, 6),                      // 0x114
            ai(4, 4, 1),                     // 0x118
            br(((-5i32) as u32) & 0xFFFF),   // 0x11C  → 0x108
            stop,                            // 0x120
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 1000).with_segment(0x100, bytes)
    }

    #[test]
    fn jit_runs_full_sum_of_squares_no_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&sum_of_squares_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0xAA));
        // 1²+2²+...+10² = 385 = 0x181 broadcast.
        assert_eq!(
            result.final_state.gpr[3],
            0x00000181_00000181_00000181_00000181,
            "sum_of_squares should produce 385 = 0x181 broadcast"
        );
        let stats = exec.jit_stats();
        assert_eq!(stats.fallback_runs, 0,
                   "sum_of_squares should be 100% JIT-compilable");
    }

    #[test]
    fn jit_runs_sum_of_squares_byte_identical_to_interpreter() {
        let prog = sum_of_squares_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(),
                "JIT diverged from interpreter on sum_of_squares:\n{d:?}");
    }

    /// R4a: subroutine call/return via brsl + bi.
    /// Mirrors `synthetic_brsl_ret.elf`. With R4a's dispatcher loop,
    /// this runs **100% via JIT** with TWO compiled functions:
    ///   1. Entry function (0x100): il + brsl → jumps to subroutine block.
    ///   2. Subroutine block (0x110): ai + bi → CONTINUE_TO (link).
    ///   3. Continuation function (0x108): stop.
    /// Dispatcher iterates twice: once on the entry function,
    /// then a second compile-or-fetch for the indirect target 0x108.
    fn brsl_ret_program() -> SpuProgram {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let brsl = |rt: u32, off: u32| (0x066u32 << 23) | ((off & 0xFFFF) << 7) | rt;
        let ai   = |rt: u32, ra: u32, imm: u32| (0x1Cu32 << 24) | ((imm & 0x3FF) << 14) | (ra << 7) | rt;
        let bi   = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        let nop  = 0x4020_0000u32;
        let stop = 0x99u32 & 0x3FFF;

        // 0x100  il r3, 10            ; argument
        // 0x104  brsl r5, +3           ; → 0x110, link r5 = 0x108
        // 0x108  stop 0x99             ; final stop after return
        // 0x10C  nop                   ; padding
        // 0x110  ai r3, r3, 7          ; subroutine: r3 += 7 → 17
        // 0x114  bi r5                 ; return via link
        let code: [u32; 6] = [
            il(3, 10),
            brsl(5, 3),
            stop,
            nop,
            ai(3, 3, 7),
            bi(5),
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn r4a_jit_runs_brsl_ret_zero_fallback() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&brsl_ret_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0x99));
        // r3 = 10 + 7 = 17 broadcast.
        assert_eq!(
            result.final_state.gpr[3],
            0x00000011_00000011_00000011_00000011,
            "subroutine should add 7 to r3 → 17"
        );

        let stats = exec.jit_stats();
        assert_eq!(stats.fallback_runs, 0,
                   "R4a brsl_ret must not fall back to interpreter");
        // Two distinct functions: entry (0x100) and continuation (0x108).
        assert_eq!(stats.compiled_functions, 2,
                   "expected 2 compiled functions (entry + continuation)");
        assert_eq!(stats.cache_misses, 2);
        assert_eq!(stats.jit_runs, 2);
        assert_eq!(stats.dispatcher_iterations, 2);
    }

    #[test]
    fn r4a_brsl_ret_byte_identical_to_interpreter() {
        let prog = brsl_ret_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let (_, _, d) = run_and_diff(&mut interp, &mut recomp, &prog);
        assert!(d.is_identical(),
                "JIT diverged from interpreter on brsl_ret:\n{d:?}");
    }

    #[test]
    fn r4a_dispatcher_caches_continuation_function() {
        // Run brsl_ret twice. With R4b, the second run hits the *chain
        // table* (not the global cache) for both functions — chained_jumps
        // accounts for the continuation function being reached without
        // an additional compile-or-fetch dispatch.
        let mut exec = RecompilerExecutor::new();
        exec.execute(&brsl_ret_program());
        let stats_after_first = exec.jit_stats();
        assert_eq!(stats_after_first.compiled_functions, 2);
        assert_eq!(stats_after_first.cache_misses, 2);
        assert_eq!(stats_after_first.cache_hits, 0);
        // First run: both iterations populate the chain table (patch
        // misses) and install entries — no chain hits yet.
        assert_eq!(stats_after_first.patch_hits, 0);
        assert_eq!(stats_after_first.patch_misses, 2);

        exec.execute(&brsl_ret_program());
        let stats_after_second = exec.jit_stats();
        assert_eq!(stats_after_second.compiled_functions, 2,
                   "no new functions should be compiled on second run");
        // R4b: the second run satisfies both iterations from the chain
        // table — chained_jumps += 2. The global cache is *not* hit
        // because we never reach `compile_or_fetch` on the chained path.
        assert_eq!(stats_after_second.cache_hits, 0,
                   "R4b chain-bypass should leave global cache_hits unchanged");
        assert_eq!(stats_after_second.chained_jumps, 2,
                   "second run must satisfy both iterations from the chain table");
        assert_eq!(stats_after_second.dispatcher_bypasses, 2);
        assert_eq!(stats_after_second.patch_hits, 2);
        // patch_misses still 2 from the first run; second run added zero.
        assert_eq!(stats_after_second.patch_misses, 2);
        assert_eq!(stats_after_second.fallback_runs, 0);
    }

    #[test]
    fn r4a_indirect_branch_target_resolved_dynamically() {
        // Build a program where the indirect target depends on a
        // computed value. r4 = 0x108 + 0x4 (or similar manipulation),
        // then bi r4 jumps to a stop at 0x10C — proving the dispatcher
        // re-compiles for whichever PC the JIT writes.
        let il = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let ila = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let bi = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        let stop = 0x77u32 & 0x3FFF;

        // 0x100 ila r4, 0x10C  ; target
        // 0x104 bi r4           ; → 0x10C
        // 0x108 stop 0xAA       ; (NOT executed)
        // 0x10C stop 0x77       ; (executed)
        let code: [u32; 4] = [
            ila(4, 0x10C),
            bi(4),
            0xAAu32 & 0x3FFF,  // stop 0xAA
            stop,
        ];
        let _ = il;
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&prog);
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0x77));
        assert_eq!(exec.jit_stats().fallback_runs, 0);
    }

    #[test]
    fn r4a_dispatcher_caps_iterations_to_max_steps() {
        // Self-loop via bi. Without a bound, the dispatcher would
        // run forever; with max_steps it must terminate cleanly.
        let ila = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let bi = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        let code: [u32; 2] = [
            ila(4, 0x100),  // r4 = 0x100 (entry)
            bi(4),          // bi r4 → loops back to 0x100 forever
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 50).with_segment(0x100, bytes);

        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&prog);
        // Must terminate with MaxStepsExceeded, not infinite-loop.
        assert!(matches!(result.stop_reason, ExecutionStopReason::MaxStepsExceeded));
    }

    /// Wall-clock comparison between interpreter and JIT for the loop
    /// fixture. Reports the speedup ratio. JIT compile cost is excluded
    /// from the timed region — we warm up by running once before the
    /// loop. This documents the actual end-to-end benefit of the JIT
    /// for hot code, not just compile latency.
    ///
    /// The assertion is intentionally loose (JIT must be at least as
    /// fast as interpreter on average) so the test stays stable across
    /// CI machines / Cranelift versions. The test prints concrete
    /// numbers to stderr so a human can read the actual ratio.
    #[test]
    fn jit_outperforms_interpreter_on_hot_loop() {
        use std::time::Instant;

        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;

        let prog = loop_program();

        // Warm up both backends so any one-time costs (Cranelift compile,
        // CPU caches, allocator) don't pollute the measurement.
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }

        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();

        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();

        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        eprintln!(
            "[bench] {} runs of synthetic_loop:\n  interpreter: {:?}\n  JIT:         {:?}\n  speedup:     {:.2}×",
            MEASURED_RUNS, interp_time, jit_time, speedup
        );

        // The recompiler should at minimum keep up with the interpreter
        // on a 5-block, 8-instruction loop running 10× per call.
        // If JIT is dramatically slower, something is wrong (e.g. we're
        // re-compiling per call instead of caching).
        assert!(
            speedup >= 0.5,
            "JIT should be at least half as fast as interpreter; got {speedup:.2}×"
        );

        // After all those runs, JitStats should show 1 compile + many runs.
        let stats = recomp.jit_stats();
        assert_eq!(stats.compiled_functions, 1,
                   "expected 1 cached compilation");
        assert!(stats.jit_runs >= (WARMUP_RUNS + MEASURED_RUNS) as u64,
                   "expected at least {} JIT runs, got {}",
                   WARMUP_RUNS + MEASURED_RUNS, stats.jit_runs);
        assert_eq!(stats.fallback_runs, 0,
                   "loop program should never fall back");
    }

    /// R4a benchmark: subroutine call/return via brsl + bi. Pre-R4a
    /// this ALWAYS fell back to interpreter (function had `bi` →
    /// pre-flight rejected). Post-R4a it stays in JIT-land via the
    /// dispatcher, with cached compilation across calls.
    #[test]
    fn r4a_benchmark_brsl_ret_jit_vs_interpreter() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;

        let prog = brsl_ret_program();

        // Pre-R4a equivalent: the interpreter — measures the path that
        // would have been taken before R4a's dispatcher loop existed.
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }

        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();

        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();

        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        eprintln!(
            "[bench-R4a] {} runs of brsl_ret (subroutine call+return):\n  interpreter: {:?}\n  JIT (R4a):   {:?}\n  speedup:     {:.2}×",
            MEASURED_RUNS, interp_time, jit_time, speedup
        );

        // After warm + measured, the JIT cache should hold both
        // functions (entry + continuation).
        let stats = recomp.jit_stats();
        assert_eq!(stats.compiled_functions, 2);
        assert_eq!(stats.fallback_runs, 0);
        // Hits dominate misses: 5 warmup + 200 measured = 205 calls
        // × 2 dispatcher iters each = 410 total. First 2 iters compile
        // (patch misses + cache misses), the remaining 408 are now
        // R4b chain hits (patch_hits / chained_jumps), bypassing the
        // global cache entirely.
        assert!(stats.chained_jumps >= 408,
                "expected ≥408 R4b chained_jumps across {} executions, got {}",
                WARMUP_RUNS + MEASURED_RUNS, stats.chained_jumps);
        assert_eq!(stats.cache_misses, 2);
        // Global cache_hits is 0 — every post-compile iteration is a
        // chain hit before the global cache is queried.
        assert_eq!(stats.cache_hits, 0,
                   "R4b should bypass the global cache after first compile");
    }

    // =================================================================
    // R4b — chained patching seguro: tests
    // =================================================================
    //
    // Safety contract:
    //   - The chain table only short-circuits when ls_hash matches.
    //   - On any mismatch (e.g. SMC, aliased pc from a different
    //     program), the entry is evicted and execution falls back to
    //     the standard global-cache path. Byte-exact equivalence with
    //     the interpreter must hold either way.

    /// First-execute paths must always be patch_misses — there is
    /// nothing in the chain table yet, so the dispatcher cannot bypass
    /// the global cache. Stats invariants:
    ///   patch_hits == 0, patch_misses == dispatcher_iterations,
    ///   chained_jumps == 0, dispatcher_bypasses == 0.
    #[test]
    fn r4b_chain_refuses_when_not_compiled() {
        let mut exec = RecompilerExecutor::new();
        let result = exec.execute(&brsl_ret_program());
        assert_eq!(result.stop_reason, ExecutionStopReason::Stop(0x99));
        let stats = exec.jit_stats();
        assert_eq!(stats.dispatcher_iterations, 2);
        assert_eq!(stats.patch_hits, 0,
                   "fresh executor: chain table must be empty on first run");
        assert_eq!(stats.chained_jumps, 0);
        assert_eq!(stats.dispatcher_bypasses, 0);
        assert_eq!(stats.patch_misses, 2,
                   "both dispatcher iterations must miss the chain table");
        // After the first run, the chain table holds 2 entries (entry +
        // continuation). Both will be hits on the second run.
        assert_eq!(exec.chain_table_size(), 2);
        assert_eq!(stats.invalid_chain_guards, 0);
    }

    /// SMC-style: poison the LS bytes around an entry between executions
    /// so `ls_hash` changes. Byte-exactness vs the interpreter must be
    /// preserved. Pre-R4c the eviction path was the R4b chain guard
    /// (`invalid_chain_guards += 1`); with R4c the dispatcher's SMC
    /// scan catches the stale compile *before* `chain_lookup` runs and
    /// evicts both `compiled` and `chain` — so the safety signal moves
    /// from `invalid_chain_guards` to `smc_invalidations` /
    /// `smc_chain_evictions`. Either path proves the chain refused to
    /// run stale code; the test now accepts both.
    #[test]
    fn r4b_chain_refuses_when_ls_hash_changes() {
        // Build two programs that share entry_pc=0x100 but have
        // different bytes around it — the second should NOT reuse the
        // first program's chain entry.
        fn il_stop(rt: u32, imm: i16) -> SpuProgram {
            let il = ((0x081u32 & 0x1FF) << 23) | ((imm as u16 as u32 & 0xFFFF) << 7) | (rt & 0x7F);
            let stop = 0u32;
            let mut bytes = Vec::with_capacity(8);
            bytes.extend_from_slice(&il.to_be_bytes());
            bytes.extend_from_slice(&stop.to_be_bytes());
            SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
        }
        let prog_a = il_stop(3, 0x1234);
        let prog_b = il_stop(3, 0x5678); // different imm → different bytes
        let mut exec = RecompilerExecutor::new();

        // First run installs (pc=0x100, hash_a) into the chain.
        exec.execute(&prog_a);
        let s1 = exec.jit_stats();
        assert_eq!(s1.invalid_chain_guards, 0);
        assert_eq!(s1.patch_hits, 0);
        assert_eq!(s1.patch_misses, 1);

        // Second run targets pc=0x100 but ls_hash differs. The chain
        // table holds a stale entry → invalid_chain_guards += 1, then
        // the global cache is consulted (cache miss, fresh compile).
        let r2 = exec.execute(&prog_b);
        assert_eq!(r2.stop_reason, ExecutionStopReason::Stop(0));
        // Result must remain byte-exact vs interpreter.
        let mut interp = InterpreterExecutor::default();
        let r_interp = interp.execute(&prog_b);
        let d = diff_snapshots(&r_interp.final_state, &r2.final_state);
        assert!(d.is_identical(),
                "R4b stale-chain fallback must preserve byte-exactness: {d:?}");

        let s2 = exec.jit_stats();
        // Either R4b path (invalid_chain_guards) or R4c path
        // (smc_chain_evictions) must have caught the stale chain.
        // R4c runs first, so smc_chain_evictions is the path we hit
        // today; the assert keeps the OR so the test stays meaningful
        // if the order ever changes.
        let evicted = s2.invalid_chain_guards + s2.smc_chain_evictions;
        assert!(evicted >= 1,
                "stale chain entry must be detected by either R4b or R4c path; \
                 got invalid_chain_guards={}, smc_chain_evictions={}",
                s2.invalid_chain_guards, s2.smc_chain_evictions);
        assert_eq!(s2.patch_hits, 0,
                   "stale entry must NOT count as patch_hit");
        assert!(s2.smc_invalidations >= 1,
                "R4c must have invalidated the prior compile from the SMC scan");
        // Two compiled functions across the two distinct (pc, ls_hash)
        // keys (the first invalidated by R4c, the second freshly
        // compiled — `compiled_functions` is monotonically increasing
        // per compile, not a snapshot).
        assert_eq!(s2.compiled_functions, 2);
        assert_eq!(s2.cache_misses, 2);
        assert_eq!(s2.fallback_runs, 0);
    }

    /// brsl_ret 2nd-execution: chain table must satisfy both dispatcher
    /// iterations on the second `execute()` call. This is the canonical
    /// R4b acceptance criterion — chained_jumps > 0 across calls.
    #[test]
    fn r4b_brsl_ret_second_execution_chains() {
        let mut exec = RecompilerExecutor::new();

        // Run #1: warms the chain table.
        exec.execute(&brsl_ret_program());
        let s1 = exec.jit_stats();
        assert_eq!(s1.chained_jumps, 0, "first run cannot chain");
        assert_eq!(s1.compiled_functions, 2);

        // Run #2: every dispatcher iteration must be a chain hit.
        exec.execute(&brsl_ret_program());
        let s2 = exec.jit_stats();
        assert!(s2.chained_jumps > 0,
                "second execution must produce chained_jumps > 0");
        assert_eq!(s2.chained_jumps - s1.chained_jumps, 2,
                   "second run should chain both dispatcher iterations");
        assert_eq!(s2.dispatcher_bypasses - s1.dispatcher_bypasses, 2);
        assert_eq!(s2.patch_hits - s1.patch_hits, 2);
        // No new compiles, no new global cache hits or misses.
        assert_eq!(s2.compiled_functions, 2);
        assert_eq!(s2.cache_hits, 0);
        assert_eq!(s2.cache_misses, 2);
    }

    /// R4b equivalence: synthetic_loop must remain byte-exact across
    /// repeated executions even with the chain table active. We run
    /// the same program 10 times and verify the JIT state matches the
    /// interpreter every iteration; chained_jumps must accumulate.
    #[test]
    fn r4b_synthetic_loop_equivalence_across_repeated_runs() {
        let prog = loop_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();

        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: JIT diverged from interpreter: {d:?}");
        }
        let stats = recomp.jit_stats();
        assert!(stats.chained_jumps >= 9,
                "expected ≥9 chained jumps after 10 runs (1 warmup + 9 chain hits), got {}",
                stats.chained_jumps);
        assert_eq!(stats.compiled_functions, 1,
                   "loop program: only one function — no recompiles allowed");
        assert_eq!(stats.invalid_chain_guards, 0,
                   "no SMC: invalid_chain_guards must remain 0");
    }

    /// R4b equivalence: brsl_ret must remain byte-exact across repeats.
    #[test]
    fn r4b_brsl_ret_equivalence_across_repeated_runs() {
        let prog = brsl_ret_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();

        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: JIT diverged from interpreter: {d:?}");
        }
        let stats = recomp.jit_stats();
        // 10 runs × 2 dispatcher iters/run = 20 total. First 2 are
        // patch misses (compile), remaining 18 should be chain hits.
        assert!(stats.chained_jumps >= 18,
                "expected ≥18 chained jumps after 10 brsl_ret runs, got {}",
                stats.chained_jumps);
    }

    /// R4b equivalence: fibonacci_program — single-function loop with
    /// 11 dispatcher iterations? No — only 1 dispatcher iter (no
    /// indirect branches). The chain table accumulates 1 entry; runs
    /// 2..N hit the chain.
    #[test]
    fn r4b_fibonacci_equivalence_across_repeated_runs() {
        let prog = fibonacci_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();

        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: JIT diverged from interpreter on fibonacci: {d:?}");
        }
        let stats = recomp.jit_stats();
        assert!(stats.chained_jumps >= 9,
                "expected ≥9 chained jumps after 10 fibonacci runs, got {}",
                stats.chained_jumps);
        assert_eq!(stats.compiled_functions, 1);
    }

    /// R4b equivalence: sum_of_squares — same shape as fibonacci.
    #[test]
    fn r4b_sum_of_squares_equivalence_across_repeated_runs() {
        let prog = sum_of_squares_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();

        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: JIT diverged from interpreter on sum_of_squares: {d:?}");
        }
        let stats = recomp.jit_stats();
        assert!(stats.chained_jumps >= 9,
                "expected ≥9 chained jumps after 10 sum_of_squares runs, got {}",
                stats.chained_jumps);
        assert_eq!(stats.compiled_functions, 1);
    }

    /// R4b safety: clearing the cache must clear the chain table too.
    #[test]
    fn r4b_clear_cache_drops_chain_table() {
        let mut exec = RecompilerExecutor::new();
        exec.execute(&brsl_ret_program());
        assert_eq!(exec.chain_table_size(), 2);
        exec.clear_function_cache();
        assert_eq!(exec.chain_table_size(), 0,
                   "clear_function_cache must purge the chain table");
        // Stats counters are NOT cleared (they're cumulative observability).
        // But the next run must repopulate the chain — i.e. patch_hits
        // does not increase on iteration 1 of the post-clear run.
        let pre_chained = exec.jit_stats().chained_jumps;
        exec.execute(&brsl_ret_program());
        let post_chained = exec.jit_stats().chained_jumps;
        assert_eq!(post_chained, pre_chained,
                   "post-clear run must not chain (chain table was empty)");
    }

    /// R4b benchmark: synthetic_loop hot loop with chain table active.
    /// Reports JIT vs interpreter timing + chain stats.
    #[test]
    fn r4b_benchmark_synthetic_loop() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = loop_program();

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();

        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        let stats = recomp.jit_stats();
        eprintln!(
            "[bench-R4b] synthetic_loop {} runs:\n  interpreter: {:?}\n  JIT (R4b):   {:?}\n  speedup:     {:.2}×\n  chained_jumps: {}\n  dispatcher_bypasses: {}",
            MEASURED_RUNS, interp_time, jit_time, speedup,
            stats.chained_jumps, stats.dispatcher_bypasses,
        );
        // 5 warmup + 200 measured = 205 dispatcher iters; first one is
        // a patch miss, rest are chain hits.
        assert!(stats.chained_jumps >= 204,
                "expected ≥204 chained_jumps, got {}", stats.chained_jumps);
        assert_eq!(stats.compiled_functions, 1);
    }

    /// R4b benchmark: brsl_ret with chain table active. Compares both
    /// total wall-clock and the share of dispatcher iters that bypassed
    /// the global cache.
    #[test]
    fn r4b_benchmark_brsl_ret() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = brsl_ret_program();

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();

        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        let stats = recomp.jit_stats();
        eprintln!(
            "[bench-R4b] brsl_ret (call+return) {} runs:\n  interpreter: {:?}\n  JIT (R4b):   {:?}\n  speedup:     {:.2}×\n  chained_jumps: {}\n  dispatcher_bypasses: {}\n  cache_misses: {}",
            MEASURED_RUNS, interp_time, jit_time, speedup,
            stats.chained_jumps, stats.dispatcher_bypasses, stats.cache_misses,
        );
        assert_eq!(stats.compiled_functions, 2);
        assert!(stats.chained_jumps >= 408,
                "expected ≥408 chained_jumps across 205 runs × 2 iters, got {}",
                stats.chained_jumps);
    }

    /// R4b benchmark: fibonacci_program. Single-function tight loop;
    /// the JIT compiles once and the chain table satisfies every
    /// subsequent execute() call.
    #[test]
    fn r4b_benchmark_fibonacci() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = fibonacci_program();

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();

        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        let stats = recomp.jit_stats();
        eprintln!(
            "[bench-R4b] fibonacci {} runs:\n  interpreter: {:?}\n  JIT (R4b):   {:?}\n  speedup:     {:.2}×\n  chained_jumps: {}",
            MEASURED_RUNS, interp_time, jit_time, speedup, stats.chained_jumps,
        );
        assert!(stats.chained_jumps >= 204,
                "expected ≥204 chained_jumps, got {}", stats.chained_jumps);
        assert_eq!(stats.compiled_functions, 1);
    }

    /// R4b benchmark: sum_of_squares_program.
    #[test]
    fn r4b_benchmark_sum_of_squares() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = sum_of_squares_program();

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();

        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        let stats = recomp.jit_stats();
        eprintln!(
            "[bench-R4b] sum_of_squares {} runs:\n  interpreter: {:?}\n  JIT (R4b):   {:?}\n  speedup:     {:.2}×\n  chained_jumps: {}",
            MEASURED_RUNS, interp_time, jit_time, speedup, stats.chained_jumps,
        );
        assert!(stats.chained_jumps >= 204,
                "expected ≥204 chained_jumps, got {}", stats.chained_jumps);
        assert_eq!(stats.compiled_functions, 1);
    }

    // =================================================================
    // R4c — SMC / cache invalidation: tests
    // =================================================================
    //
    // Safety contract:
    //   - Every dispatcher iteration runs `smc_scan` *before* touching
    //     the chain or global cache. Any compiled function whose source
    //     bytes diverged from its compile-time hash is evicted from
    //     `compiled` + `compiled_meta` + `cache`. The chain is evicted
    //     too, but only when its `entry_fn` matches the function we
    //     just removed (preserves R4b chain hits for unrelated pcs).
    //   - Result must remain byte-exact vs the interpreter regardless
    //     of how many invalidations happen during a run.

    /// Non-SMC programs MUST never trigger an invalidation. The scan
    /// runs every iteration but only walks `compiled_meta` and never
    /// finds a mismatch (because no instruction byte was modified).
    /// `smc_range_misses` accumulates while `smc_invalidations` and
    /// `smc_chain_evictions` stay at 0.
    ///
    /// Counting note: the scan runs at the *start* of every dispatcher
    /// iteration. Run #1's scan walks an empty meta map (no misses);
    /// from run #2 onward each iter's scan finds the loop_program's
    /// 1 entry → +1 miss per iter. After N runs of loop_program, the
    /// total is exactly `N - 1` misses.
    #[test]
    fn r4c_no_smc_means_no_invalidations() {
        let mut exec = RecompilerExecutor::new();
        for _ in 0..10 { exec.execute(&loop_program()); }
        let stats = exec.jit_stats();
        assert_eq!(stats.smc_invalidations, 0,
                   "non-SMC program must produce 0 SMC invalidations");
        assert_eq!(stats.smc_chain_evictions, 0);
        assert_eq!(stats.smc_full_flushes, 0);
        // 10 runs × 1 dispatcher iter per run = 10 iters; first scan
        // sees an empty map, so misses == 9.
        assert_eq!(stats.smc_range_misses, 9,
                   "smc_range_misses must accumulate from iter #2 onward; got {}",
                   stats.smc_range_misses);
        assert_eq!(stats.smc_range_hits, 0);
    }

    /// Direct SMC test: same entry_pc, two distinct programs run on the
    /// same executor. The second `execute()` carries different bytes at
    /// the entry, the SMC scan detects the prior compile is stale, and
    /// the result must reflect the *new* program — proving stale code
    /// did NOT execute.
    #[test]
    fn r4c_smc_detected_across_executions_at_same_pc() {
        // prog_a: stop with code 0xAA
        // prog_b: stop with code 0xBB (different bytes at 0x100)
        let stop_prog = |code: u32| {
            let stop = code & 0x3FFF;
            let bytes = stop.to_be_bytes();
            SpuProgram::new(0x100, 100).with_segment(0x100, bytes.to_vec())
        };
        let prog_a = stop_prog(0xAA);
        let prog_b = stop_prog(0xBB);

        let mut exec = RecompilerExecutor::new();
        let r_a = exec.execute(&prog_a);
        assert_eq!(r_a.stop_reason, ExecutionStopReason::Stop(0xAA));
        let s_a = exec.jit_stats();
        assert_eq!(s_a.smc_invalidations, 0);

        let r_b = exec.execute(&prog_b);
        // Critical: result must be Stop(0xBB), not Stop(0xAA). Stale
        // JIT code would still produce 0xAA — that would be a bug.
        assert_eq!(r_b.stop_reason, ExecutionStopReason::Stop(0xBB),
                   "R4c must invalidate stale compile and run new program");
        let s_b = exec.jit_stats();
        assert!(s_b.smc_invalidations >= 1,
                "smc_scan must have invalidated the stale prog_a compile, got {}",
                s_b.smc_invalidations);
    }

    /// Negative-space test: an SPU `stqd` that writes to LS *outside*
    /// any compiled function's code range must NOT trigger an SMC
    /// invalidation. The program is split across two functions (forced
    /// by an indirect branch) so the dispatcher runs `smc_scan` after
    /// the write but before the second function executes — that's the
    /// observation point where we'd notice a false positive.
    ///
    /// Layout (single LS, single execute() call):
    /// ```text
    /// 0x100  ila r4, 0          ; function A
    /// 0x104  stqd r5, 32(r4)    ;   writes 16 bytes at LS[0x200]
    /// 0x108  ila r6, 0x114      ;
    /// 0x10C  bi r6              ;   indirect → CONTINUE_TO 0x114
    /// 0x110  nop                ; padding
    /// 0x114  stop 0xCC          ; function B
    /// ```
    /// A's code_range is `[0x100, 0x110)`; B's is `[0x114, 0x118)`.
    /// The stqd target `0x200` is well outside both, so the SMC scan
    /// at iter #2's start must find both entries unchanged.
    #[test]
    fn r4c_writes_outside_compiled_ranges_do_not_invalidate() {
        let ila  = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let stqd = |rt: u32, ra: u32, imm10: u32|
            (0x24u32 << 24) | ((imm10 & 0x3FF) << 14) | (ra << 7) | rt;
        let bi   = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        let nop  = 0x4020_0000u32;
        let stop = 0xCCu32 & 0x3FFF;
        let code: [u32; 6] = [
            ila(4, 0),               // 0x100  r4 = 0
            stqd(5, 4, 32),          // 0x104  LS[0 + 32*16] = LS[0x200] ← r5 (zero)
            ila(6, 0x114),           // 0x108  r6 = 0x114
            bi(6),                   // 0x10C  → CONTINUE_TO 0x114
            nop,                     // 0x110  padding (not in any block)
            stop,                    // 0x114  function B
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut exec = RecompilerExecutor::new();
        let r = exec.execute(&prog);
        assert_eq!(r.stop_reason, ExecutionStopReason::Stop(0xCC));
        let s = exec.jit_stats();
        assert_eq!(s.compiled_functions, 2,
                   "expected 2 compiled funcs (A at 0x100, B at 0x114)");
        assert_eq!(s.smc_invalidations, 0,
                   "stqd to LS[0x200] is outside both code ranges; \
                    smc_scan must not invalidate anything");
        assert_eq!(s.smc_chain_evictions, 0);
        // The scan at iter #2's start should have walked function A's
        // entry (function B isn't compiled yet at that scan point) and
        // confirmed it's still intact.
        assert!(s.smc_range_misses >= 1,
                "expected ≥1 range miss from the iter-#2 scan, got {}",
                s.smc_range_misses);
        // Byte-exact vs interpreter.
        let mut interp = InterpreterExecutor::default();
        let r_interp = interp.execute(&prog);
        let d = diff_snapshots(&r_interp.final_state, &r.final_state);
        assert!(d.is_identical(),
                "JIT must remain byte-exact when writing outside code ranges: {d:?}");
    }

    /// Across-program SMC where the chain table is the witness: prog_a
    /// runs (chain installs entry_a for pc 0x100), prog_b runs at the
    /// same pc with different bytes. The R4c scan invalidates entry_a
    /// from `compiled` AND `chain` (because chain still pointed at the
    /// just-evicted fn pointer), so `smc_chain_evictions >= 1`.
    #[test]
    fn r4c_smc_evicts_chain_when_pointing_at_invalidated_fn() {
        let stop_prog = |code: u32| {
            let stop = code & 0x3FFF;
            let bytes = stop.to_be_bytes();
            SpuProgram::new(0x100, 100).with_segment(0x100, bytes.to_vec())
        };
        let mut exec = RecompilerExecutor::new();
        exec.execute(&stop_prog(0xAA));
        // Run again with same program → chain hits.
        exec.execute(&stop_prog(0xAA));
        let s_warm = exec.jit_stats();
        assert!(s_warm.chained_jumps >= 1,
                "second run of identical prog must hit the chain");
        // Now poison: run a different program at the same pc.
        exec.execute(&stop_prog(0xBB));
        let s_smc = exec.jit_stats();
        assert!(s_smc.smc_chain_evictions >= 1,
                "chain pointing at stale prog_a must be evicted by SMC scan, got {}",
                s_smc.smc_chain_evictions);
    }

    /// Equivalence: synthetic_loop must remain byte-exact vs interpreter
    /// across many repeats with R4c active. The scan runs every iter
    /// but never finds an SMC mismatch (clean program), so results must
    /// be identical to a fresh interpreter run.
    #[test]
    fn r4c_synthetic_loop_equivalence_across_repeated_runs() {
        let prog = loop_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: JIT diverged from interpreter under R4c: {d:?}");
        }
        let s = recomp.jit_stats();
        assert_eq!(s.smc_invalidations, 0);
        assert!(s.chained_jumps >= 9, "R4b chain still active: got {}", s.chained_jumps);
    }

    /// Equivalence: brsl_ret with R4c enabled.
    #[test]
    fn r4c_brsl_ret_equivalence_across_repeated_runs() {
        let prog = brsl_ret_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: brsl_ret JIT diverged under R4c: {d:?}");
        }
        let s = recomp.jit_stats();
        assert_eq!(s.smc_invalidations, 0);
        assert!(s.chained_jumps >= 18);
    }

    /// Equivalence: fibonacci with R4c enabled.
    #[test]
    fn r4c_fibonacci_equivalence_across_repeated_runs() {
        let prog = fibonacci_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: fibonacci JIT diverged under R4c: {d:?}");
        }
        let s = recomp.jit_stats();
        assert_eq!(s.smc_invalidations, 0);
    }

    /// Equivalence: sum_of_squares with R4c enabled.
    #[test]
    fn r4c_sum_of_squares_equivalence_across_repeated_runs() {
        let prog = sum_of_squares_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: sum_of_squares JIT diverged under R4c: {d:?}");
        }
        let s = recomp.jit_stats();
        assert_eq!(s.smc_invalidations, 0);
    }

    /// Stats invariant: for a non-SMC program every scan recompute
    /// produces a `smc_range_misses`, never a `smc_range_hits`. We can't
    /// equate `smc_range_misses` with `dispatcher_iterations` directly
    /// because the scan runs *before* the per-iter compile, so the
    /// very first iter (or iter that triggers a fresh compile) sees an
    /// empty / smaller meta map. The invariant we *can* check is
    /// `smc_range_hits == 0` and `smc_invalidations == 0`.
    #[test]
    fn r4c_stats_invariant_no_smc_means_only_misses() {
        let mut exec = RecompilerExecutor::new();
        for _ in 0..3 { exec.execute(&loop_program()); }
        let s = exec.jit_stats();
        assert_eq!(s.smc_range_hits, 0,
                   "no SMC: every scan recompute must be a miss");
        assert_eq!(s.smc_invalidations, 0);
        // After 3 runs we should have at least 1 miss accumulated
        // (runs 2 & 3 each scan 1 entry → 2 misses).
        assert!(s.smc_range_misses >= 2,
                "expected ≥2 range misses across 3 runs, got {}",
                s.smc_range_misses);
    }

    /// Compiled-meta map must shrink in lock-step with the global
    /// cache when SMC invalidates an entry.
    #[test]
    fn r4c_compiled_meta_size_tracks_compiled_size() {
        let mut exec = RecompilerExecutor::new();
        exec.execute(&brsl_ret_program());
        // 2 compiled functions → 2 meta entries.
        assert_eq!(exec.compiled_meta_size(), 2);
        exec.clear_function_cache();
        assert_eq!(exec.compiled_meta_size(), 0);
    }

    // =================================================================
    // R4c benchmarks: same shape as R4b, used to verify safety scan
    // doesn't materially regress performance.
    // =================================================================

    #[test]
    fn r4c_benchmark_synthetic_loop() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = loop_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();
        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        let s = recomp.jit_stats();
        eprintln!(
            "[bench-R4c] synthetic_loop {} runs:\n  interpreter: {:?}\n  JIT (R4c):   {:?}\n  speedup:     {:.2}×\n  smc_invalidations: {}\n  smc_range_misses: {}\n  chained_jumps: {}",
            MEASURED_RUNS, interp_time, jit_time, speedup,
            s.smc_invalidations, s.smc_range_misses, s.chained_jumps,
        );
        assert_eq!(s.smc_invalidations, 0);
    }

    #[test]
    fn r4c_benchmark_brsl_ret() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = brsl_ret_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();
        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        let s = recomp.jit_stats();
        eprintln!(
            "[bench-R4c] brsl_ret {} runs:\n  interpreter: {:?}\n  JIT (R4c):   {:?}\n  speedup:     {:.2}×\n  smc_invalidations: {}\n  smc_range_misses: {}\n  chained_jumps: {}",
            MEASURED_RUNS, interp_time, jit_time, speedup,
            s.smc_invalidations, s.smc_range_misses, s.chained_jumps,
        );
        assert_eq!(s.smc_invalidations, 0);
    }

    #[test]
    fn r4c_benchmark_fibonacci() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = fibonacci_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();
        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        let s = recomp.jit_stats();
        eprintln!(
            "[bench-R4c] fibonacci {} runs:\n  interpreter: {:?}\n  JIT (R4c):   {:?}\n  speedup:     {:.2}×\n  smc_invalidations: {}\n  smc_range_misses: {}",
            MEASURED_RUNS, interp_time, jit_time, speedup,
            s.smc_invalidations, s.smc_range_misses,
        );
        assert_eq!(s.smc_invalidations, 0);
    }

    #[test]
    fn r4c_benchmark_sum_of_squares() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = sum_of_squares_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let jit_time = t0.elapsed();
        let speedup = interp_time.as_secs_f64() / jit_time.as_secs_f64().max(1e-9);
        let s = recomp.jit_stats();
        eprintln!(
            "[bench-R4c] sum_of_squares {} runs:\n  interpreter: {:?}\n  JIT (R4c):   {:?}\n  speedup:     {:.2}×\n  smc_invalidations: {}\n  smc_range_misses: {}",
            MEASURED_RUNS, interp_time, jit_time, speedup,
            s.smc_invalidations, s.smc_range_misses,
        );
        assert_eq!(s.smc_invalidations, 0);
    }

    // =================================================================
    // R5 — Interpreter resume from JitState (partial fallback): tests
    // =================================================================
    //
    // Safety contract:
    //   - When the JIT cannot continue (compile failure on a target
    //     function or runtime UNKNOWN_OPCODE), the dispatcher hands the
    //     in-flight `JitState` (gprs + ls + pc) to the interpreter via
    //     `resume_from_state`. The interpreter continues from `state.pc`
    //     — no re-run from `program.entry_pc`.
    //   - End result must be byte-exact vs an interpreter-only run of
    //     the same program.
    //   - Stats: `partial_fallbacks` and `unknown_opcode_exits` bump;
    //     `fallback_runs` (full fallback from entry) stays 0.
    //
    // The trigger we use is `rchcnt rt, ch` — primary 0x00F. Decoded
    // by `rpcs3-spu-decoder` as `SpuInstKind::Channel { kind: Count }`.
    // The JIT does **not** codegen channel ops (no `Channel` arm in
    // `supported_check`), so the function containing it fails to
    // compile. The interpreter handles `rchcnt` (returns the channel's
    // current count, 0 by default) without stalling.

    /// Build a 2-function program where:
    ///   - Function A (0x100): `ila r4, 0x12345; ila r6, 0x110; bi r6;`
    ///     — supported by JIT. After JIT runs A, state has r4=0x12345
    ///     and r6=0x110, and CONTINUE_TO 0x110.
    ///   - Function B (0x110): `rchcnt r3, 28; ai r3, r3, 100; stop 0x77;`
    ///     — first instruction is unsupported by JIT, so compile_or_fetch
    ///     returns Err. Dispatcher must hand to interpreter from pc=0x110
    ///     with r4/r6 already populated.
    ///
    /// Padding `nop` at 0x10C keeps the indirect-branch target outside
    /// the decoded extent of function A so the decoder follows only
    /// `bi r6` and stops there.
    fn r5_partial_fallback_program() -> SpuProgram {
        let ila    = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let bi     = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        // R5.3 update: `rchcnt` is now fully JIT-codegen'd (R5.1
        // const-1 fast-path + R5.3 helper for variable channels), so
        // it no longer triggers a partial fallback. We use `dfa`
        // (primary 0x2CC, double-precision add) instead — neither the
        // JIT codegen nor the Rust interpreter implements double-
        // precision float, so both produce `Error::Unimplemented` and
        // the byte-exact diff still passes via the partial-fallback
        // bridge.
        let dfa    = |rt: u32, ra: u32, rb: u32|
            (0x2CCu32 << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | rt;
        let nop    = 0x4020_0000u32;
        let stop   = 0x77u32 & 0x3FFF;

        let code: [u32; 7] = [
            ila(4, 0x12345),    // 0x100  r4 = 0x12345 (proves JIT prefix ran)
            ila(6, 0x110),      // 0x104  r6 = 0x110
            bi(6),              // 0x108  → CONTINUE_TO 0x110 (function A ends)
            nop,                // 0x10C  padding (decoder doesn't follow)
            dfa(3, 3, 3),       // 0x110  function B: UNSUPPORTED by JIT *and* interpreter
            ila(3, 0xABC),      // 0x114  (never reached — interpreter hits Unimplemented)
            stop,               // 0x118
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        SpuProgram::new(0x100, 100).with_segment(0x100, bytes)
    }

    #[test]
    fn r5_partial_fallback_unknown_opcode_resumes_correctly() {
        let prog = r5_partial_fallback_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();

        let result_interp = interp.execute(&prog);
        let result_recomp = recomp.execute(&prog);

        // Result must be byte-exact vs interpreter-only.
        let d = diff_snapshots(&result_interp.final_state, &result_recomp.final_state);
        assert!(d.is_identical(),
                "R5 partial fallback diverged from interpreter-only: {d:?}");
        assert_eq!(result_interp.stop_reason, result_recomp.stop_reason);

        // R5.3 update: dfa is unsupported by both JIT and interpreter,
        // so both produce `ExecutionStopReason::Error("Unimplemented...")`.
        // The contract still holds: byte-exact equivalence + JIT prefix
        // state preserved through the partial-fallback boundary.
        match &result_recomp.stop_reason {
            ExecutionStopReason::Error(msg) => {
                assert!(msg.contains("Unimplemented"),
                        "expected dfa Unimplemented, got: {msg}");
            }
            other => panic!("expected Error, got {other:?}"),
        }

        // r4 = 0x12345 broadcast — proves JIT prefix ran AND its state
        // was preserved into the interpreter resume even though the
        // interpreter immediately bailed at the dfa.
        assert_eq!(
            result_recomp.final_state.gpr[4],
            0x00012345_00012345_00012345_00012345,
            "r4 must reflect the JIT-prefix's `ila r4, 0x12345`",
        );

        // Stats: partial fallback fired exactly once; full fallback never.
        let s = recomp.jit_stats();
        assert_eq!(s.partial_fallbacks, 1,
                   "expected exactly one partial fallback");
        assert_eq!(s.unknown_opcode_exits, 1);
        assert_eq!(s.resumed_interpreter_runs, 1);
        assert_eq!(s.fallback_runs, 0,
                   "full fallback must NOT fire — partial fallback handles it");
        // The dispatcher ran function A once via JIT before falling back.
        assert_eq!(s.compiled_functions, 1,
                   "only function A should have been JIT-compiled");
        assert_eq!(s.jit_runs, 1);
    }

    /// R5 with a store before the fallback: function A does `stqd`
    /// to LS[0x200] before the indirect branch. The interpreter must
    /// see those LS bytes in resume.
    #[test]
    fn r5_partial_fallback_preserves_jit_ls_writes() {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let ila  = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let stqd = |rt: u32, ra: u32, imm10: u32|
            (0x24u32 << 24) | ((imm10 & 0x3FF) << 14) | (ra << 7) | rt;
        let bi     = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        // R5.3 update: rchcnt is now JIT-supported, so we use `dfa`
        // (double-precision add) as the unsupported trigger.
        let dfa  = |rt: u32, ra: u32, rb: u32|
            (0x2CCu32 << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | rt;
        let nop  = 0x4020_0000u32;
        let stop = 0x88u32 & 0x3FFF;

        // Layout (single segment starting at 0x100). stqd's effective
        // address is `(ra.preferred + imm10 * 16) & 0x3FFF0`, so we
        // use `ila r4, 0x40` and `imm10 = 16` → effective LS addr =
        // 0x40 + 16*16 = 0x140 + 0xC0 = 0x200... actually 0x40 + 256 =
        // 0x140? Let me state it directly: imm10 of 0x10 (=16 decimal)
        // gives 0x40 + 16*16 = 0x40 + 0x100 = 0x140. We pick 0x140
        // because it's outside the program code (which ends at 0x124),
        // proving the JIT store landed and the interpreter saw it.
        //
        //   0x100 il   r3, 0x5A5A          ; r3 = 0x5A5A broadcast
        //   0x104 ila  r4, 0x40            ; r4 = 0x40 broadcast
        //   0x108 stqd r3, 16(r4)          ; LS[0x40 + 16*16] = LS[0x140]
        //   0x10C ila  r6, 0x118           ; target for indirect branch
        //   0x110 bi   r6                  ; → CONTINUE_TO 0x118
        //   0x114 nop                      ; padding
        //   0x118 rchcnt r5, 28            ; UNSUPPORTED → partial fallback
        //   0x11C lqd  r7, 16(r4)          ; r7 = LS[0x140] (must see JIT's store)
        //   0x120 stop 0x88
        // Layout:
        //   0x100 il   r3, 0x5A5A          ; JIT
        //   0x104 ila  r4, 0x40            ; JIT
        //   0x108 stqd r3, 16(r4)          ; JIT — LS[0x140] = 0x5A5A
        //   0x10C ila  r6, 0x118           ; JIT
        //   0x110 bi   r6                  ; → CONTINUE_TO 0x118
        //   0x114 nop                      ; padding
        //   0x118 dfa  r5, r5, r5          ; UNSUPPORTED → partial fallback
        //   0x11C nop                      ; never reached
        //   0x120 stop 0x88                ; never reached
        let code: [u32; 9] = [
            il(3, 0x5A5A),
            ila(4, 0x40),
            stqd(3, 4, 16),
            ila(6, 0x118),
            bi(6),
            nop,
            dfa(5, 5, 5),
            nop,
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 200).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5 partial fallback with LS write diverged: {d:?}");
        // R5.3: dfa unsupported by both → both produce Error.
        match &r_recomp.stop_reason {
            ExecutionStopReason::Error(_) => {}
            other => panic!("expected Error from dfa, got {other:?}"),
        }

        // The LS at 0x140 should hold the stored qword — proving the
        // JIT prefix's stqd survived through the partial-fallback
        // boundary even though the interpreter immediately bailed at
        // the dfa. Address: 0x40 + 16*16 = 0x140, outside the code
        // segment (which ends at 0x124), so the byte check is
        // unambiguous.
        let stored = u128::from_be_bytes(
            r_recomp.final_state.ls[0x140..0x150].try_into().unwrap()
        );
        assert_eq!(stored, 0x00005A5A_00005A5A_00005A5A_00005A5A,
                   "LS[0x140] must reflect the JIT prefix's stqd");

        let s = recomp.jit_stats();
        assert_eq!(s.partial_fallbacks, 1);
        assert_eq!(s.fallback_runs, 0);
    }

    /// R5: PC at resume must be the JIT-exit pc, not program.entry_pc.
    /// We exercise this by having function A do `bi` to a far target;
    /// the resume must continue from that target, not redo function A.
    ///
    /// Without preserved pc, the interpreter would re-run from 0x100
    /// and the final state would either diverge (re-running ila r4)
    /// or, by accident, look the same (idempotent). To force a
    /// divergence-on-bug, we use a counter the JIT increments and the
    /// interpreter increments again — only PC-correct resume gives
    /// counter == 1; full re-run would give counter == 2.
    #[test]
    fn r5_partial_fallback_preserves_pc_no_redo_of_jit_prefix() {
        let ila    = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let bi     = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        // R5.3 update: rchcnt is now JIT-supported. Use `dfa` instead.
        let dfa    = |rt: u32, ra: u32, rb: u32|
            (0x2CCu32 << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | rt;
        let nop    = 0x4020_0000u32;
        let _ = 0x99u32; // stop unused now (interpreter errors before reaching it)

        // 0x100 ila r3, 0xCAFE            ; r3 = 0xCAFE (proves JIT ran)
        // 0x104 ila r4, 0x110             ; target
        // 0x108 bi r4                     ; → 0x110 (function A done)
        // 0x10C nop                       ; padding
        // 0x110 dfa r5, r5, r5            ; UNSUPPORTED — partial fallback
        // 0x114 nop                       ; never reached
        // 0x118 nop                       ; never reached
        let code: [u32; 7] = [
            ila(3, 0xCAFE),
            ila(4, 0x110),
            bi(4),
            nop,
            dfa(5, 5, 5),
            nop,
            nop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut recomp = RecompilerExecutor::new();
        let r = recomp.execute(&prog);

        // The interpreter resume hits dfa at 0x110 immediately and
        // returns Err(Unimplemented). Result is Error, with state.pc
        // left at 0x110 (where the dfa lives).
        match &r.stop_reason {
            ExecutionStopReason::Error(_) => {}
            other => panic!("expected Error, got {other:?}"),
        }
        // r3 must reflect the JIT prefix's `ila r3, 0xCAFE`. If the
        // interpreter had re-run from entry_pc=0x100 (full fallback)
        // r3 would also be 0xCAFE — so this assertion alone doesn't
        // distinguish the paths. The robust signal is the stats:
        // partial_fallbacks bumped, fallback_runs stayed 0.
        assert_eq!(
            r.final_state.gpr[3],
            0x0000CAFE_0000CAFE_0000CAFE_0000CAFE,
            "r3 must reflect the JIT prefix's `ila r3, 0xCAFE`",
        );
        let s = recomp.jit_stats();
        assert_eq!(s.partial_fallbacks, 1,
                   "partial fallback path must have been used");
        assert_eq!(s.fallback_runs, 0,
                   "full fallback (from entry_pc) must NOT have been used");
    }

    /// R5: a 100% JIT-supported program must NOT trigger any fallback,
    /// neither partial nor full. Smoke check that introducing R5
    /// didn't accidentally make the dispatcher pessimistic.
    #[test]
    fn r5_no_fallback_on_fully_jit_supported_program() {
        let mut exec = RecompilerExecutor::new();
        // Run all the existing 100%-JIT fixtures one after another.
        exec.execute(&loop_program());
        exec.execute(&arith_program());
        exec.execute(&loadstore_program());
        exec.execute(&fibonacci_program());
        exec.execute(&sum_of_squares_program());

        let s = exec.jit_stats();
        assert_eq!(s.partial_fallbacks, 0,
                   "fully JIT-supported programs must not trigger partial fallback");
        assert_eq!(s.unknown_opcode_exits, 0);
        assert_eq!(s.resumed_interpreter_runs, 0);
        assert_eq!(s.resumed_interpreter_steps, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    /// R5: equivalence test — repeating the partial-fallback program
    /// 10 times must produce the same byte-exact result as 10
    /// interpreter-only runs.
    #[test]
    fn r5_partial_fallback_equivalence_across_repeated_runs() {
        let prog = r5_partial_fallback_program();
        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: R5 partial fallback diverged: {d:?}");
        }
        let s = recomp.jit_stats();
        // Each of the 10 runs triggers exactly one partial fallback.
        assert_eq!(s.partial_fallbacks, 10);
        // Function A is compiled once; subsequent runs hit the cache or
        // the chain and skip recompile.
        assert_eq!(s.compiled_functions, 1);
        assert_eq!(s.fallback_runs, 0);
    }

    /// R5 benchmark: time partial fallback vs interpreter-only on the
    /// same program. The point isn't to claim a speedup (the suffix
    /// runs interpreter either way) but to verify (a) partial fallback
    /// is no slower than interpreter-only on the suffix, and (b) the
    /// JIT prefix amortizes across repeated runs.
    #[test]
    fn r5_benchmark_partial_fallback_vs_interpreter() {
        use std::time::Instant;
        const WARMUP_RUNS: u32 = 5;
        const MEASURED_RUNS: u32 = 200;
        let prog = r5_partial_fallback_program();

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        for _ in 0..WARMUP_RUNS {
            interp.execute(&prog);
            recomp.execute(&prog);
        }
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { interp.execute(&prog); }
        let interp_time = t0.elapsed();
        let t0 = Instant::now();
        for _ in 0..MEASURED_RUNS { recomp.execute(&prog); }
        let recomp_time = t0.elapsed();

        let ratio = interp_time.as_secs_f64() / recomp_time.as_secs_f64().max(1e-9);
        let s = recomp.jit_stats();
        eprintln!(
            "[bench-R5] partial-fallback program {} runs:\n  interpreter:        {:?}\n  JIT+partial-fallback: {:?}\n  ratio:              {:.2}× (interpreter / JIT-with-resume)\n  partial_fallbacks:  {}\n  resumed_interp_steps: {}",
            MEASURED_RUNS, interp_time, recomp_time, ratio,
            s.partial_fallbacks, s.resumed_interpreter_steps,
        );
        // Validate stats: every measured run + warmup triggers one
        // partial fallback.
        assert_eq!(
            s.partial_fallbacks,
            (WARMUP_RUNS + MEASURED_RUNS) as u64,
            "expected one partial fallback per execute() call",
        );
        assert_eq!(s.fallback_runs, 0);
    }

    // =================================================================
    // R5.1 — Channel ops partial codegen: tests
    // =================================================================
    //
    // Safety contract (re-stated for this layer):
    //   - `rchcnt` against the 7 constant-count channels
    //     (SPU_RDEVENTSTAT, SPU_WREVENTMASK, SPU_WREVENTACK, SPU_WRDEC,
    //      SPU_RDDEC, SPU_RDEVENTMASK, SPU_RDMACHSTAT) is JIT-codegen'd
    //     directly: rt = [1, 0, 0, 0] in lane order. PC advances at the
    //     block boundary like any other non-terminator instruction.
    //   - Every other channel form (`rchcnt` against a variable-count
    //     channel; `rdch` / `wrch` against any channel) keeps the
    //     pre-R5.1 behavior: `supported_check` rejects the function,
    //     `compile_or_fetch` returns Err, the dispatcher hands control
    //     to the interpreter via R5 partial fallback. No fake values.
    //   - Stats: a successful compile that contains channel ops bumps
    //     `channel_ops_jitted` by the number of channel instructions
    //     the JIT emitted code for. A partial fallback whose triggering
    //     instruction is a channel op bumps `channel_ops_partial_fallback`.

    /// `rchcnt rt, ch` against a constant-count channel must run 100%
    /// via JIT — no compile failure, no partial fallback, byte-exact
    /// vs interpreter. We use SPU_RDMACHSTAT (channel 23) so the
    /// interpreter sees count=1 and the JIT emits the same.
    #[test]
    fn r5_1_rchcnt_const_channel_runs_via_jit() {
        let il     = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xCCu32 & 0x3FFF;

        // 0x100 il r3, 0xFFFF        ; r3 = 0xFFFFFFFF broadcast (so we
        //                              can prove rchcnt overwrote it)
        // 0x104 rchcnt r3, 23        ; SPU_RDMACHSTAT — const-1 channel
        // 0x108 stop 0xCC
        let code: [u32; 3] = [
            il(3, 0xFFFF),
            rchcnt(3, 23),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.1 rchcnt JIT diverged from interpreter: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0xCC));
        // r3 lane 0 = 1 (count); lanes 1..3 = 0 — overwrites the
        // 0xFFFFFFFF broadcast we set up.
        assert_eq!(
            r_recomp.final_state.gpr[3],
            0x00000001_00000000_00000000_00000000,
            "rchcnt against const-1 channel must produce [1, 0, 0, 0] lanes",
        );

        // Stats: 1 channel op codegen'd; ZERO partial fallback.
        let s = recomp.jit_stats();
        assert_eq!(s.channel_ops_jitted, 1,
                   "supported channel op should be codegen'd, not fall back");
        assert_eq!(s.channel_ops_partial_fallback, 0);
        assert_eq!(s.partial_fallbacks, 0,
                   "rchcnt against const-channel must NOT trigger partial fallback");
        assert_eq!(s.fallback_runs, 0);
    }

    /// All 7 const-count channels must be accepted by the JIT.
    #[test]
    fn r5_1_all_seven_const_channels_supported() {
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0x99u32 & 0x3FFF;
        // The 7 const-count channels per rpcs3_spu_thread::ch::*:
        //   0=RDEVENTSTAT, 1=WREVENTMASK, 2=WREVENTACK,
        //   7=WRDEC,       8=RDDEC,       22=RDEVENTMASK,
        //   23=RDMACHSTAT
        let const_channels: [u32; 7] = [0, 1, 2, 7, 8, 22, 23];

        for ch in const_channels {
            let code = vec![rchcnt(3, ch), stop];
            let mut bytes = Vec::with_capacity(code.len() * 4);
            for w in &code { bytes.extend_from_slice(&w.to_be_bytes()); }
            let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

            let mut recomp = RecompilerExecutor::new();
            let r = recomp.execute(&prog);
            assert_eq!(r.stop_reason, ExecutionStopReason::Stop(0x99),
                       "channel {} should reach stop via JIT", ch);
            let s = recomp.jit_stats();
            assert_eq!(s.partial_fallbacks, 0,
                       "channel {} must run via JIT, not partial fallback", ch);
            assert_eq!(s.channel_ops_jitted, 1, "channel {} should be jitted", ch);
        }
    }

    /// R5.3 update (was R5.1's "variable channels always fallback"):
    /// `rchcnt` against SPU_RDINMBOX (channel 29) is now JIT-codegen'd
    /// via `spu_helper_rchcnt`. The helper queries the live
    /// `SpuChannels::count(29)` (returns 0 for empty mailbox) and
    /// writes `[0, 0, 0, 0]` to `gpr[rt]`. No partial fallback.
    #[test]
    fn r5_1_rchcnt_variable_channel_falls_to_partial_fallback() {
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0x55u32 & 0x3FFF;
        let code: [u32; 2] = [rchcnt(3, 29), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.3 variable-channel rchcnt must be byte-exact: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0x55));

        let s = recomp.jit_stats();
        // R5.3: variable channel is now JIT-codegen'd via helper.
        assert_eq!(s.partial_fallbacks, 0,
                   "R5.3: variable rchcnt is now JITed — no fallback");
        assert_eq!(s.channel_ops_partial_fallback, 0);
        assert_eq!(s.fallback_runs, 0);
        assert_eq!(s.channel_ops_jitted, 1,
                   "R5.3: rchcnt against variable channel is codegen'd");
    }

    /// R5.2 (was R5.1's "always fallback" assertion): `wrch` against
    /// SPU_WREVENTMASK (channel 1, never stalls — pure write to
    /// `event_mask`) is now JIT-codegen'd via the runtime helper.
    /// Programs that only write to non-stalling channels stay 100%
    /// in JIT-land.
    #[test]
    fn r5_1_wrch_falls_to_partial_fallback() {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0x77u32 & 0x3FFF;

        // 0x100 il r3, 0x42         ; r3 = 0x42 (the value to wrch)
        // 0x104 wrch ch=1, r3       ; SPU_WREVENTMASK — JIT helper, never stalls
        // 0x108 stop 0x77
        let code: [u32; 3] = [il(3, 0x42), wrch(3, 1), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(), "wrch JIT must be byte-exact vs interpreter: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0x77));

        let s = recomp.jit_stats();
        // R5.2 reality: wrch is JIT-codegen'd via helper, no fallback.
        assert_eq!(s.partial_fallbacks, 0,
                   "R5.2: non-stalling wrch must run via JIT helper, no fallback");
        assert_eq!(s.channel_ops_partial_fallback, 0);
        assert_eq!(s.channel_stall_exits, 0);
        // The wrch counts toward `channel_ops_jitted` (compile-time stat
        // bumped per channel instruction in the compiled function).
        assert_eq!(s.channel_ops_jitted, 1);
        assert_eq!(s.fallback_runs, 0);
    }

    /// R5.2 (was R5.1's "function B falls back due to wrch"): the
    /// mixed `rchcnt + bi + wrch` program now runs 100% via JIT
    /// because wrch ch=1 (SPU_WREVENTMASK) is helper-JITed and
    /// doesn't stall. We keep the test under the R5.1 name to make
    /// the regression bar explicit: the previous fallback path is
    /// gone, replaced by a clean compile-and-execute.
    #[test]
    fn r5_1_mixed_const_channel_jit_then_wrch_partial_fallback() {
        let ila    = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let bi     = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let wrch   = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let nop    = 0x4020_0000u32;
        let stop   = 0xAAu32 & 0x3FFF;

        // 0x100 rchcnt r3, 23      ; r3 = [1,0,0,0] (JIT)
        // 0x104 ila    r6, 0x110   ; r6 = 0x110
        // 0x108 bi     r6          ; → CONTINUE_TO 0x110 (function A ends)
        // 0x10C nop                ; padding
        // 0x110 ila    r4, 0x99    ; r4 = 0x99 (this triggers compile-failure
        //                            on function B because of the wrch below,
        //                            so this ila runs in the interpreter)
        // 0x114 wrch   ch=1, r4    ; SPU_WREVENTMASK = 0x99 (interpreter)
        // 0x118 stop 0xAA
        let code: [u32; 7] = [
            rchcnt(3, 23),
            ila(6, 0x110),
            bi(6),
            nop,
            ila(4, 0x99),
            wrch(4, 1),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(), "R5.2 mixed program must be byte-exact: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0xAA));
        // r3 reflects function A's rchcnt; r4 reflects the ila before
        // the wrch in function B.
        assert_eq!(
            r_recomp.final_state.gpr[3],
            0x00000001_00000000_00000000_00000000,
            "r3 must reflect rchcnt's [1, 0, 0, 0] lane layout",
        );

        let s = recomp.jit_stats();
        // Both functions compile (function A: rchcnt; function B:
        // ila + wrch). 2 channel ops codegen'd (1 rchcnt + 1 wrch).
        assert_eq!(s.channel_ops_jitted, 2,
                   "rchcnt in A + wrch in B should both be codegen'd");
        assert_eq!(s.compiled_functions, 2);
        // wrch ch=1 doesn't stall — no partial fallback.
        assert_eq!(s.partial_fallbacks, 0,
                   "R5.2: wrch ch=1 helper succeeds, no fallback path");
        assert_eq!(s.channel_ops_partial_fallback, 0);
        assert_eq!(s.channel_stall_exits, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    /// Equivalence: 10 repeats of the const-channel program must all
    /// match the interpreter. Verifies that the codegen is stable
    /// across compile+chain cache hits and that `channel_ops_jitted`
    /// only bumps once (compile-time stat, not per-run).
    #[test]
    fn r5_1_rchcnt_const_channel_equivalence_across_repeats() {
        let il     = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0x88u32 & 0x3FFF;
        let code: [u32; 3] = [il(3, 0xFFFF), rchcnt(3, 8 /* SPU_RDDEC */), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: R5.1 const-channel JIT diverged: {d:?}");
        }
        let s = recomp.jit_stats();
        assert_eq!(s.compiled_functions, 1, "only one compile across repeats");
        assert_eq!(s.channel_ops_jitted, 1,
                   "channel_ops_jitted is a compile-time stat — bumped once");
        assert_eq!(s.partial_fallbacks, 0);
        assert!(s.chained_jumps >= 9,
                "R4b chain still active across repeats: got {}",
                s.chained_jumps);
    }

    /// Sanity: existing 100% JIT-supported fixtures must continue to
    /// produce zero channel-related stats (they don't exercise channels).
    #[test]
    fn r5_1_pre_existing_fixtures_have_no_channel_ops_or_fallback() {
        let mut exec = RecompilerExecutor::new();
        exec.execute(&loop_program());
        exec.execute(&arith_program());
        exec.execute(&loadstore_program());
        exec.execute(&fibonacci_program());
        exec.execute(&sum_of_squares_program());
        exec.execute(&brsl_ret_program());

        let s = exec.jit_stats();
        assert_eq!(s.channel_ops_jitted, 0,
                   "existing fixtures don't use channel ops");
        assert_eq!(s.channel_ops_partial_fallback, 0);
        assert_eq!(s.partial_fallbacks, 0,
                   "existing fixtures must not trigger partial fallback");
        assert_eq!(s.fallback_runs, 0);
    }

    // =================================================================
    // R5.2 — Channel ops via runtime helpers: tests
    // =================================================================
    //
    // Layer-on-top-of contract:
    //   - rdch / wrch are now JIT-codegen'd via `spu_helper_rdch` /
    //     `spu_helper_wrch` runtime helpers operating on the live
    //     SpuChannels owned by the dispatcher.
    //   - Successful (Ok) ops stay 100% in JIT-land — no fallback.
    //   - Stall / BadChannel returns from the helper trigger
    //     `JIT_OUTCOME_STALL`, which routes to R5 partial fallback
    //     with `state.pc` at the channel op and `channels` propagated.
    //   - The interpreter resume sees the same SpuChannels state the
    //     JIT was working with — channel-stall is handled identically
    //     to a pure interpreter run.

    /// `wrch ch=1` (SPU_WREVENTMASK) followed by `rdch ch=22`
    /// (SPU_RDEVENTMASK) round-trip: writes a value, reads it back,
    /// must produce byte-exact same gpr layout as the interpreter.
    /// Both ops are JITed; the program runs entirely under JIT
    /// codegen with zero fallback.
    #[test]
    fn r5_2_wrch_event_mask_rdch_round_trip_via_jit() {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xC1u32 & 0x3FFF;

        // 0x100 il   r3, 0x4242     ; r3 = 0x4242 broadcast
        // 0x104 wrch ch=1, r3       ; SPU_WREVENTMASK = 0x4242 (helper)
        // 0x108 rdch r5, ch=22      ; r5 lane 0 = 0x4242 (helper read)
        // 0x10C stop 0xC1
        let code: [u32; 4] = [il(3, 0x4242), wrch(3, 1), rdch(5, 22), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.2 wrch+rdch round-trip must be byte-exact: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0xC1));
        // r5 lane 0 = 0x4242 (the value we wrch'd into event_mask, then
        // read via rdch). Lanes 1..3 are zero per the helper layout.
        assert_eq!(
            r_recomp.final_state.gpr[5],
            0x00004242_00000000_00000000_00000000,
            "rdch lane 0 should equal the wrch'd event_mask",
        );

        let s = recomp.jit_stats();
        assert_eq!(s.channel_ops_jitted, 2,
                   "both wrch and rdch should be codegen'd via helper");
        assert_eq!(s.partial_fallbacks, 0,
                   "non-stalling ops stay in JIT-land");
        assert_eq!(s.channel_stall_exits, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    /// `wrch ch=28` (SPU_WROUTMBOX) twice — first succeeds, second
    /// stalls because mailbox is full. The JIT helper returns Stall,
    /// JIT exits with `JIT_OUTCOME_STALL`, R5 hands to interpreter
    /// which produces `ExecutionStopReason::ChannelStall`. Result
    /// must match interpreter-only execution.
    #[test]
    fn r5_2_wrch_outmbox_second_write_stalls_falls_to_partial_fallback() {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xCAu32 & 0x3FFF;

        // 0x100 il   r3, 0xAA       ; r3 = 0xAA
        // 0x104 wrch ch=28, r3      ; SPU_WROUTMBOX = 0xAA (helper Ok)
        // 0x108 il   r4, 0xBB       ; r4 = 0xBB
        // 0x10C wrch ch=28, r4      ; STALLS — mbox already full
        //                            (helper returns Stall → JIT exits
        //                            JIT_OUTCOME_STALL → R5 fallback)
        // 0x110 stop 0xCA           ; (never reached — interpreter sees
        //                            ChannelStall and stops there)
        let code: [u32; 5] = [il(3, 0xAA), wrch(3, 28), il(4, 0xBB), wrch(4, 28), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.2 stall path must be byte-exact vs interpreter: {d:?}");
        // Both should report ChannelStall on channel 28, write side.
        assert_eq!(r_recomp.stop_reason,
                   ExecutionStopReason::ChannelStall { channel: 28, is_write: true });
        assert_eq!(r_interp.stop_reason, r_recomp.stop_reason);

        let s = recomp.jit_stats();
        // Both wrch instructions get codegen'd at compile time.
        assert_eq!(s.channel_ops_jitted, 2);
        // The second wrch call hits Stall at runtime → JIT_OUTCOME_STALL
        // → R5 partial fallback.
        assert_eq!(s.partial_fallbacks, 1);
        assert_eq!(s.channel_stall_exits, 1,
                   "stall must be attributed to channel_stall_exits");
        assert_eq!(s.channel_ops_partial_fallback, 1);
        assert_eq!(s.fallback_runs, 0,
                   "no full fallback — partial fallback handles channel stall");
    }

    /// `rdch ch=29` (SPU_RDINMBOX) on an empty mailbox: helper returns
    /// Stall on the first read. Must reproduce interpreter's
    /// ChannelStall reason.
    #[test]
    fn r5_2_rdch_empty_inmbox_stalls() {
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xCBu32 & 0x3FFF;

        // 0x100 rdch r3, ch=29      ; SPU_RDINMBOX — empty by default,
        //                            helper returns Stall, R5 fallback
        // 0x104 stop 0xCB           ; not reached
        let code: [u32; 2] = [rdch(3, 29), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.2 rdch stall must be byte-exact: {d:?}");
        assert_eq!(r_recomp.stop_reason,
                   ExecutionStopReason::ChannelStall { channel: 29, is_write: false });
        assert_eq!(r_interp.stop_reason, r_recomp.stop_reason);

        let s = recomp.jit_stats();
        assert_eq!(s.channel_ops_jitted, 1);
        assert_eq!(s.partial_fallbacks, 1);
        assert_eq!(s.channel_stall_exits, 1);
        assert_eq!(s.fallback_runs, 0);
    }

    /// `wrch ch=2` (SPU_WREVENTACK) clears bits in event_stat.
    /// Tests the side-effect path of a write helper: helper mutates
    /// SpuChannels via `channels.write` which transitively modifies
    /// event_stat via the bit-clear.
    #[test]
    fn r5_2_wrch_event_ack_clears_event_stat_bits() {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xACu32 & 0x3FFF;

        // 0x100 il   r3, 0x00FF        ; r3 = 0x00FF
        // 0x104 wrch ch=1, r3           ; event_mask = 0x00FF
        // 0x108 il   r4, 0x000F        ; r4 = 0x000F
        // 0x10C wrch ch=2, r4           ; event_stat &= !0x000F (no-op since 0)
        // 0x110 stop 0xAC
        let code: [u32; 5] = [
            il(3, 0x00FF), wrch(3, 1),
            il(4, 0x000F), wrch(4, 2),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(), "R5.2 wrch event_ack diverged: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0xAC));

        let s = recomp.jit_stats();
        assert_eq!(s.channel_ops_jitted, 2);
        assert_eq!(s.partial_fallbacks, 0);
        assert_eq!(s.channel_stall_exits, 0);
    }

    /// Mixed: the JIT prefix wrch-es a value, then a far indirect
    /// branch into a function whose first opcode is `rdch ch=29`
    /// (empty in-mbox → stall). Verify the interpreter resume sees
    /// the JIT-mutated SpuChannels (event_mask still set) and stalls
    /// at the rdch with PC preserved.
    #[test]
    fn r5_2_jit_mutates_channels_then_resume_sees_them() {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let ila  = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let bi   = |ra: u32| (0x1A8u32 << 21) | ((ra & 0x7F) << 7);
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let nop  = 0x4020_0000u32;
        let stop = 0x55u32 & 0x3FFF;

        // 0x100 il   r3, 0x1234     ; r3 = 0x1234
        // 0x104 wrch ch=1, r3       ; event_mask = 0x1234 (JIT helper Ok)
        // 0x108 ila  r6, 0x118      ; r6 = 0x118 (target)
        // 0x10C bi   r6             ; CONTINUE_TO 0x118 (function A ends)
        // 0x110 nop                 ; padding
        // 0x114 nop                 ; padding
        // 0x118 rdch r5, ch=29      ; SPU_RDINMBOX empty → STALL
        // 0x11C stop 0x55
        let code: [u32; 8] = [
            il(3, 0x1234), wrch(3, 1),
            ila(6, 0x118), bi(6),
            nop, nop,
            rdch(5, 29), stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.2 mutation+stall must be byte-exact: {d:?}");
        assert_eq!(r_recomp.stop_reason,
                   ExecutionStopReason::ChannelStall { channel: 29, is_write: false });
        // r3 from JIT prefix preserved across the boundary.
        assert_eq!(
            r_recomp.final_state.gpr[3],
            0x00001234_00001234_00001234_00001234,
            "r3 (JIT prefix) must be preserved through partial fallback",
        );

        let s = recomp.jit_stats();
        assert_eq!(s.channel_ops_jitted, 2,
                   "wrch in A + rdch in B both codegen'd");
        assert_eq!(s.compiled_functions, 2);
        assert_eq!(s.partial_fallbacks, 1);
        assert_eq!(s.channel_stall_exits, 1);
        assert_eq!(s.fallback_runs, 0);
    }

    /// Equivalence: 10× the round-trip program. `channel_ops_jitted`
    /// is a compile-time stat, so it bumps once per unique compile;
    /// the chain caches the function across reps.
    #[test]
    fn r5_2_wrch_rdch_equivalence_across_repeats() {
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0x42u32 & 0x3FFF;
        let code: [u32; 4] = [il(3, 0x9999), wrch(3, 1), rdch(5, 22), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: R5.2 rdch/wrch JIT diverged: {d:?}");
        }
        let s = recomp.jit_stats();
        assert_eq!(s.compiled_functions, 1, "single compile across reps");
        assert_eq!(s.channel_ops_jitted, 2, "wrch+rdch counted once at compile");
        assert_eq!(s.partial_fallbacks, 0,
                   "non-stalling channel ops stay in JIT-land");
        assert_eq!(s.fallback_runs, 0);
    }

    /// Fixtures that don't exercise channels must still pass with all
    /// channel stats at zero, as they did pre-R5.2.
    #[test]
    fn r5_2_pre_existing_fixtures_unchanged() {
        let mut exec = RecompilerExecutor::new();
        exec.execute(&loop_program());
        exec.execute(&arith_program());
        exec.execute(&loadstore_program());
        exec.execute(&fibonacci_program());
        exec.execute(&sum_of_squares_program());
        exec.execute(&brsl_ret_program());

        let s = exec.jit_stats();
        assert_eq!(s.channel_ops_jitted, 0);
        assert_eq!(s.channel_ops_partial_fallback, 0);
        assert_eq!(s.channel_stall_exits, 0);
        assert_eq!(s.partial_fallbacks, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    // =================================================================
    // R5.3 — rchcnt against variable-count channels via runtime helper
    // =================================================================
    //
    // Layer-on-top-of contract (from R5.2):
    //   - rchcnt const-1 channel (R5.1): direct codegen, no helper.
    //   - rchcnt variable channel (R5.3): `spu_helper_rchcnt` calls
    //     `SpuChannels::count(channel)` and writes the lane layout
    //     `[count, 0, 0, 0]` to gpr[rt]. Never stalls (count is a
    //     query). Bad channels return BadChannel → R5 partial fallback.
    //   - All R5.2 rdch/wrch behaviour preserved.

    /// `rchcnt ch=29` (SPU_RDINMBOX) on empty mailbox: count = 0.
    /// Helper writes [0, 0, 0, 0]. No fallback.
    #[test]
    fn r5_3_rchcnt_inmbox_empty_returns_zero_via_jit() {
        let il     = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop   = 0xD0u32 & 0x3FFF;

        // 0x100 il r3, 0xFFFF        ; r3 broadcast 0xFFFFFFFF (must be overwritten)
        // 0x104 rchcnt r3, 29        ; SPU_RDINMBOX — count = 0 (empty)
        // 0x108 stop 0xD0
        let code: [u32; 3] = [il(3, 0xFFFF), rchcnt(3, 29), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.3 rchcnt-29 empty must be byte-exact: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0xD0));
        // r3 = [0, 0, 0, 0] — count is 0 (empty mbox), padded zeros.
        assert_eq!(
            r_recomp.final_state.gpr[3],
            0x00000000_00000000_00000000_00000000,
            "rchcnt empty inmbox must produce all-zero lanes",
        );

        let s = recomp.jit_stats();
        assert_eq!(s.channel_ops_jitted, 1);
        assert_eq!(s.partial_fallbacks, 0,
                   "R5.3: rchcnt variable channel runs via helper, no fallback");
        assert_eq!(s.channel_stall_exits, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    /// `rchcnt ch=28` (SPU_WROUTMBOX) cycle: empty (count=1, free) → wrch
    /// → full (count=0, no free slot). Tests that the helper sees the
    /// JIT-mutated SpuChannels state.
    #[test]
    fn r5_3_rchcnt_outmbox_count_changes_after_wrch_via_jit() {
        let il     = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch   = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop   = 0xD1u32 & 0x3FFF;

        // 0x100 rchcnt r4, 28        ; before wrch: out_mbox empty → count = 1
        // 0x104 il     r3, 0xAA      ; r3 = 0xAA
        // 0x108 wrch   r3, 28        ; out_mbox = 0xAA
        // 0x10C rchcnt r5, 28        ; after wrch: out_mbox full → count = 0
        // 0x110 stop 0xD1
        let code: [u32; 5] = [
            rchcnt(4, 28),
            il(3, 0xAA),
            wrch(3, 28),
            rchcnt(5, 28),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.3 rchcnt-28 cycle must be byte-exact: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0xD1));
        // r4 lane 0 = 1 (free slot before wrch).
        assert_eq!(
            r_recomp.final_state.gpr[4] >> 96,
            1u128,
            "rchcnt before wrch must report 1 (free slot)",
        );
        // r5 lane 0 = 0 (no free slot after wrch).
        assert_eq!(
            r_recomp.final_state.gpr[5] >> 96,
            0u128,
            "rchcnt after wrch must report 0 (mbox full)",
        );

        let s = recomp.jit_stats();
        // 2× rchcnt + 1× wrch = 3 channel ops codegen'd.
        assert_eq!(s.channel_ops_jitted, 3);
        assert_eq!(s.partial_fallbacks, 0);
        assert_eq!(s.channel_stall_exits, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    /// `rchcnt ch=3` (SPU_RDSIGNOTIFY1) on no signal pending: count = 0.
    /// Tests another variable-count channel.
    #[test]
    fn r5_3_rchcnt_signotify_no_signal_via_jit() {
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop   = 0xD2u32 & 0x3FFF;

        let code: [u32; 2] = [rchcnt(3, 3 /* SPU_RDSIGNOTIFY1 */), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.3 rchcnt SNR1 must be byte-exact: {d:?}");
        assert_eq!(r_recomp.final_state.gpr[3] >> 96, 0u128,
                   "no signal pending → count = 0");

        let s = recomp.jit_stats();
        assert_eq!(s.channel_ops_jitted, 1);
        assert_eq!(s.partial_fallbacks, 0);
    }

    /// `rchcnt` against a bad channel (e.g. 100): helper returns
    /// BadChannel → JIT exits via STALL → R5 partial fallback. The
    /// interpreter resume reproduces the BadChannel via
    /// `Error::Unimplemented`.
    #[test]
    fn r5_3_rchcnt_bad_channel_falls_to_partial_fallback() {
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop   = 0xDEu32 & 0x3FFF;

        // Channel 100 is not in SpuChannels::count — returns BadChannel.
        let code: [u32; 2] = [rchcnt(3, 100), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.3 rchcnt bad-channel must be byte-exact: {d:?}");
        match &r_recomp.stop_reason {
            ExecutionStopReason::Error(msg) => {
                assert!(msg.contains("Unimplemented"),
                        "expected rchcnt Unimplemented, got: {msg}");
            }
            other => panic!("expected Error from bad channel, got {other:?}"),
        }
        assert_eq!(r_interp.stop_reason, r_recomp.stop_reason);

        let s = recomp.jit_stats();
        // The rchcnt was codegen'd (counts at compile time, even though
        // runtime returned BadChannel).
        assert_eq!(s.channel_ops_jitted, 1);
        // Helper returned BadChannel → JIT exits STALL → partial fallback.
        assert_eq!(s.partial_fallbacks, 1);
        assert_eq!(s.channel_stall_exits, 1,
                   "BadChannel from rchcnt helper attributed to channel_stall_exits");
        assert_eq!(s.channel_ops_partial_fallback, 1);
        assert_eq!(s.fallback_runs, 0);
        let _ = stop;  // unreachable — kept for clarity.
    }

    /// Mixed program: wrch (R5.2) sets event_mask, rchcnt (R5.3 helper)
    /// reads count of variable channel, rdch (R5.2) reads back the
    /// value. Confirms all three helpers cooperate without fallback.
    #[test]
    fn r5_3_mixed_wrch_rchcnt_rdch_byte_exact() {
        let il     = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch   = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let rdch   = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop   = 0xD3u32 & 0x3FFF;

        // 0x100 il   r3, 0x1234        ; r3 = 0x1234
        // 0x104 wrch ch=1, r3          ; event_mask = 0x1234 (R5.2 helper)
        // 0x108 rchcnt r4, 22          ; SPU_RDEVENTMASK count = 1 (R5.1 const-1)
        // 0x10C rchcnt r5, 28          ; SPU_WROUTMBOX count = 1 (R5.3 helper)
        // 0x110 rdch r6, 22            ; read event_mask = 0x1234 (R5.2 helper)
        // 0x114 stop 0xD3
        let code: [u32; 6] = [
            il(3, 0x1234),
            wrch(3, 1),
            rchcnt(4, 22),
            rchcnt(5, 28),
            rdch(6, 22),
            stop,
        ];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.3 mixed wrch/rchcnt/rdch must be byte-exact: {d:?}");
        assert_eq!(r_recomp.stop_reason, ExecutionStopReason::Stop(0xD3));
        // r6 lane 0 = 0x1234 (read back from event_mask).
        assert_eq!(r_recomp.final_state.gpr[6] >> 96, 0x1234u128);

        let s = recomp.jit_stats();
        // 4 channel ops codegen'd: wrch + rchcnt const + rchcnt var + rdch.
        assert_eq!(s.channel_ops_jitted, 4);
        assert_eq!(s.partial_fallbacks, 0,
                   "all helpers succeed — no fallback");
        assert_eq!(s.channel_stall_exits, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    /// 10× repeat of the variable-rchcnt program: chain/cache stay
    /// consistent, no stale state.
    #[test]
    fn r5_3_rchcnt_variable_equivalence_across_repeats() {
        let il     = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop   = 0xD4u32 & 0x3FFF;
        let code: [u32; 3] = [il(3, 0xFFFF), rchcnt(3, 29), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let baseline = interp.execute(&prog);
        for i in 0..10 {
            let r = recomp.execute(&prog);
            let d = diff_snapshots(&baseline.final_state, &r.final_state);
            assert!(d.is_identical(),
                    "iter {i}: R5.3 rchcnt variable diverged: {d:?}");
        }
        let s = recomp.jit_stats();
        assert_eq!(s.compiled_functions, 1, "single compile across reps");
        assert_eq!(s.channel_ops_jitted, 1, "rchcnt counted once at compile");
        assert_eq!(s.partial_fallbacks, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    /// Pre-existing fixtures must keep zero channel stats.
    #[test]
    fn r5_3_pre_existing_fixtures_unchanged() {
        let mut exec = RecompilerExecutor::new();
        exec.execute(&loop_program());
        exec.execute(&fibonacci_program());
        exec.execute(&sum_of_squares_program());
        exec.execute(&brsl_ret_program());
        let s = exec.jit_stats();
        assert_eq!(s.channel_ops_jitted, 0);
        assert_eq!(s.channel_ops_partial_fallback, 0);
        assert_eq!(s.channel_stall_exits, 0);
        assert_eq!(s.partial_fallbacks, 0);
        assert_eq!(s.fallback_runs, 0);
    }

    // =================================================================
    // R5.4a — Channel parking propagation through partial fallback
    // =================================================================
    //
    // The JIT-side helpers don't park directly — `JIT_OUTCOME_STALL`
    // routes through `partial_fallback_to_interpreter` and the
    // interpreter's own step() sets `park_state` on `ChannelStall`.
    // Then `snapshot_from_thread` carries park_state into the
    // recompiler's final result. End-to-end equivalence with an
    // interpreter-only run must include park_state.

    /// `rdch ch=29` on empty in_mbox: stall via R5.2 helper → R5
    /// partial fallback → interpreter parks → snapshot reports
    /// park_state.
    #[test]
    fn r5_4a_rdch_stall_propagates_park_state_through_jit() {
        use rpcs3_spu_thread::{SpuParkReason, SpuParkState};
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xCEu32 & 0x3FFF;
        let code: [u32; 2] = [rdch(3, 29), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.4a rdch stall must be byte-exact incl. park_state: {d:?}");
        assert_eq!(r_recomp.stop_reason,
                   ExecutionStopReason::ChannelStall { channel: 29, is_write: false });

        // Both backends agree on park_state.
        let expected = Some(SpuParkState {
            pc: 0x100,
            reason: SpuParkReason::ChannelRead { channel: 29 },
        });
        assert_eq!(r_recomp.final_state.park_state, expected,
                   "recompiler must propagate park_state from interpreter resume");
        assert_eq!(r_interp.final_state.park_state, expected);
    }

    /// `wrch ch=28` when out_mbox is full: stall via R5.2 helper →
    /// partial fallback → interpreter parks with ChannelWrite reason.
    #[test]
    fn r5_4a_wrch_stall_propagates_park_state_through_jit() {
        use rpcs3_spu_thread::{SpuParkReason, SpuParkState};
        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xCFu32 & 0x3FFF;

        // 0x100 il   r3, 0xAA       ; r3 = 0xAA
        // 0x104 wrch ch=28, r3      ; success — out_mbox = 0xAA
        // 0x108 il   r4, 0xBB       ; r4 = 0xBB
        // 0x10C wrch ch=28, r4      ; STALL — mbox full → park
        // 0x110 stop 0xCF
        let code: [u32; 5] = [il(3, 0xAA), wrch(3, 28), il(4, 0xBB), wrch(4, 28), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.4a wrch stall must be byte-exact incl. park_state: {d:?}");
        assert_eq!(r_recomp.stop_reason,
                   ExecutionStopReason::ChannelStall { channel: 28, is_write: true });

        let expected = Some(SpuParkState {
            pc: 0x10C,
            reason: SpuParkReason::ChannelWrite { channel: 28 },
        });
        assert_eq!(r_recomp.final_state.park_state, expected,
                   "park PC must be the second wrch (0x10C), not the first");
        assert_eq!(r_interp.final_state.park_state, expected);
    }

    /// Bad channel: helper returns BadChannel → interpreter
    /// reproduces `Error::Unimplemented` → park_state is NOT set
    /// (only true stalls park). Demonstrates that BadChannel and
    /// Stall are semantically distinguished.
    #[test]
    fn r5_4a_bad_channel_does_not_park() {
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xD7u32 & 0x3FFF;
        let code: [u32; 2] = [rdch(3, 100), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut interp = InterpreterExecutor::default();
        let mut recomp = RecompilerExecutor::new();
        let r_interp = interp.execute(&prog);
        let r_recomp = recomp.execute(&prog);

        let d = diff_snapshots(&r_interp.final_state, &r_recomp.final_state);
        assert!(d.is_identical(),
                "R5.4a bad channel must be byte-exact: {d:?}");

        // Bad channel → Error, not ChannelStall.
        match &r_recomp.stop_reason {
            ExecutionStopReason::Error(_) => {}
            other => panic!("expected Error from bad channel, got {other:?}"),
        }

        // park_state must be None — only WouldStall parks, not BadChannel.
        assert_eq!(r_recomp.final_state.park_state, None,
                   "BadChannel must NOT park");
        assert_eq!(r_interp.final_state.park_state, None);
    }

    /// Programs that don't stall on channels must NOT park.
    /// Sanity for the entire R5.4a model: parking is opt-in via
    /// WouldStall, not a side effect of channel ops in general.
    #[test]
    fn r5_4a_non_stalling_program_has_no_park_state() {
        let rchcnt = |rt: u32, ch: u32| (0x00Fu32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xD8u32 & 0x3FFF;
        // rchcnt against a const-1 channel — never stalls.
        let code: [u32; 2] = [rchcnt(3, 23), stop];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut recomp = RecompilerExecutor::new();
        let r = recomp.execute(&prog);
        assert_eq!(r.stop_reason, ExecutionStopReason::Stop(0xD8));
        assert_eq!(r.final_state.park_state, None,
                   "rchcnt const-1 success must not produce park_state");
    }

    /// Pre-existing fixtures (no channel ops) must produce park_state
    /// = None — preserves byte-exact equivalence carried over from
    /// earlier waves.
    #[test]
    fn r5_4a_pre_existing_fixtures_have_no_park_state() {
        let mut exec = RecompilerExecutor::new();
        let r1 = exec.execute(&loop_program());
        let r2 = exec.execute(&fibonacci_program());
        let r3 = exec.execute(&sum_of_squares_program());
        let r4 = exec.execute(&brsl_ret_program());
        for (label, r) in [("loop", r1), ("fib", r2), ("sumsq", r3), ("brsl", r4)] {
            assert_eq!(r.final_state.park_state, None,
                       "{label}: park_state must be None for non-channel programs");
        }
    }

    // =====================================================================
    // R5.4b — wake + resume after JIT stall
    //
    // End-to-end flow verified here:
    //   1. JIT runs program until rdch/wrch hits WouldStall.
    //   2. R5 partial fallback returns SpuExecutionResult with both
    //      `park_state = Some(..)` and `channels = (live state)`.
    //   3. PPU side mutates a clone of `channels` (push in_mbox / drain
    //      out_mbox / signal snr) and calls a wake helper on a SpuThread
    //      reconstructed from the snapshot. Wake returns Ready { pc }.
    //   4. Caller resumes via `InterpreterExecutor::resume_from_state`
    //      with the updated channels, starting at `pc`. The channel op
    //      now succeeds, the program runs to Stop.
    //   5. Final snapshot is byte-exact vs an interpreter run that did
    //      the same manual injection at the same point.
    // =====================================================================

    /// rdch SPU_RDINMBOX stall via JIT → wake by pushing in_mbox →
    /// resume from park.pc → consumes value → byte-exact vs interp.
    #[test]
    fn r5_4b_jit_rdch_stall_wake_and_resume() {
        use rpcs3_spu_differential::diff_snapshots;
        use rpcs3_spu_thread::{SpuParkReason, SpuParkState, SpuThread, SpuWakeResult};

        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop_imm = 0xCEu32 & 0x3FFF;
        let code: [u32; 2] = [rdch(3, 29), stop_imm];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes.clone());

        // ---- A) JIT stall → snapshot has park_state + channels ----
        let mut recomp = RecompilerExecutor::new();
        let r_stall = recomp.execute(&prog);
        assert_eq!(r_stall.stop_reason,
                   ExecutionStopReason::ChannelStall { channel: 29, is_write: false });
        let park = r_stall.final_state.park_state.expect("must have park_state");
        assert_eq!(park, SpuParkState {
            pc: 0x100,
            reason: SpuParkReason::ChannelRead { channel: 29 },
        });
        // Channels snapshot exposed for wake.
        let stalled_channels = r_stall.final_state.channels.clone();
        assert!(stalled_channels.in_mbox.is_none(),
                "in_mbox must be empty at stall");

        // ---- B) PPU wake on a SpuThread reconstructed from snapshot --
        let mut spu = SpuThread::new(0);
        spu.channels = stalled_channels.clone();
        spu.park_state = Some(park);
        let wake = spu.ppu_push_inmbox_and_try_wake(0xC0FFEE12);
        let wake_pc = match wake {
            SpuWakeResult::Ready { pc } => pc,
            other => panic!("expected Ready, got {other:?}"),
        };
        assert_eq!(wake_pc, park.pc);
        assert!(!spu.is_parked());
        // The wake helper updated `spu.channels`, which we'll feed
        // forward into resume.
        let resumed_channels = spu.channels.clone();

        // ---- C) Resume from park.pc via InterpreterExecutor ------
        let interp = InterpreterExecutor::default();
        let r_resume = interp.resume_from_state(
            r_stall.final_state.gpr.as_ref(),
            r_stall.final_state.ls.as_ref(),
            &resumed_channels,
            wake_pc,
            100,
            r_stall.steps_executed,
        );
        assert_eq!(r_resume.stop_reason, ExecutionStopReason::Stop(0xCE));
        assert!(r_resume.final_state.park_state.is_none(),
                "after wake+resume park must be cleared");
        // Final pc lands on stop instruction (rdch advances to 0x104).
        assert_eq!(r_resume.final_state.pc, 0x104);

        // ---- D) Byte-exact vs interpreter-only flow with same wake -
        let mut interp_only = InterpreterExecutor::default();
        let r_interp_full = interp_only.execute(&prog);
        // r_interp_full stalls at the same point; resume the same way.
        let r_interp_full_resume = {
            let mut t = SpuThread::new(0);
            t.channels = r_interp_full.final_state.channels.clone();
            t.park_state = r_interp_full.final_state.park_state;
            let _ = t.ppu_push_inmbox_and_try_wake(0xC0FFEE12);
            let resumed = t.channels.clone();
            interp.resume_from_state(
                r_interp_full.final_state.gpr.as_ref(),
                r_interp_full.final_state.ls.as_ref(),
                &resumed,
                park.pc,
                100,
                r_interp_full.steps_executed,
            )
        };
        let d = diff_snapshots(&r_resume.final_state, &r_interp_full_resume.final_state);
        assert!(d.is_identical(),
                "JIT-stall→wake→resume must equal interp→stall→wake→resume: {d:?}");
    }

    /// wrch SPU_WROUTMBOX stall via JIT → wake by draining out_mbox →
    /// resume from park.pc → wrch writes new value.
    #[test]
    fn r5_4b_jit_wrch_stall_wake_and_resume() {
        use rpcs3_spu_differential::diff_snapshots;
        use rpcs3_spu_thread::{SpuParkReason, SpuParkState, SpuThread, SpuWakeResult};

        let il   = |rt: u32, imm: u16| ((0x081u32 & 0x1FF) << 23) | ((imm as u32) << 7) | rt;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop_imm = 0xCFu32 & 0x3FFF;
        // 0x100 il   r3, 0xAA
        // 0x104 wrch ch=28, r3   ; success — out_mbox = 0xAA
        // 0x108 il   r4, 0xBB
        // 0x10C wrch ch=28, r4   ; STALL — mbox full
        // 0x110 stop 0xCF
        let code: [u32; 5] = [il(3, 0xAA), wrch(3, 28), il(4, 0xBB), wrch(4, 28), stop_imm];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut recomp = RecompilerExecutor::new();
        let r_stall = recomp.execute(&prog);
        assert_eq!(r_stall.stop_reason,
                   ExecutionStopReason::ChannelStall { channel: 28, is_write: true });
        let park = r_stall.final_state.park_state.expect("must have park_state");
        assert_eq!(park, SpuParkState {
            pc: 0x10C,
            reason: SpuParkReason::ChannelWrite { channel: 28 },
        });
        let stalled_channels = r_stall.final_state.channels.clone();
        assert_eq!(stalled_channels.out_mbox, Some(0xAA),
                   "out_mbox holds the first wrch's value at stall");

        // PPU drains out_mbox, then wake.
        let mut spu = SpuThread::new(0);
        spu.channels = stalled_channels.clone();
        spu.park_state = Some(park);
        let (drained, wake) = spu.ppu_pop_outmbox_and_try_wake();
        assert_eq!(drained, Some(0xAA));
        let wake_pc = match wake {
            SpuWakeResult::Ready { pc } => pc,
            other => panic!("expected Ready, got {other:?}"),
        };
        assert_eq!(wake_pc, 0x10C);
        let resumed_channels = spu.channels.clone();

        let interp = InterpreterExecutor::default();
        let r_resume = interp.resume_from_state(
            r_stall.final_state.gpr.as_ref(),
            r_stall.final_state.ls.as_ref(),
            &resumed_channels,
            wake_pc,
            100,
            r_stall.steps_executed,
        );
        assert_eq!(r_resume.stop_reason, ExecutionStopReason::Stop(0xCF));
        assert_eq!(r_resume.final_state.channels.out_mbox, Some(0xBB),
                   "second wrch must have written 0xBB after wake");
        assert!(r_resume.final_state.park_state.is_none());

        // Compare to a pure-interpreter run that did the same dance.
        let mut interp_only = InterpreterExecutor::default();
        let r_interp_full = interp_only.execute(&prog);
        let r_interp_full_resume = {
            let mut t = SpuThread::new(0);
            t.channels = r_interp_full.final_state.channels.clone();
            t.park_state = r_interp_full.final_state.park_state;
            let (_, _) = t.ppu_pop_outmbox_and_try_wake();
            let resumed = t.channels.clone();
            interp.resume_from_state(
                r_interp_full.final_state.gpr.as_ref(),
                r_interp_full.final_state.ls.as_ref(),
                &resumed,
                park.pc,
                100,
                r_interp_full.steps_executed,
            )
        };
        let d = diff_snapshots(&r_resume.final_state, &r_interp_full_resume.final_state);
        assert!(d.is_identical(),
                "JIT wrch wake+resume must equal interp wake+resume: {d:?}");
    }

    /// Wake with unsatisfied condition → StillBlocked → resume must
    /// re-park (interpreter sees the same WouldStall again at the same
    /// pc). Demonstrates the wake API does NOT fake success.
    #[test]
    fn r5_4b_jit_wrong_wake_keeps_thread_blocked() {
        use rpcs3_spu_thread::{SpuThread, SpuWakeResult};

        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop_imm = 0xD0u32 & 0x3FFF;
        let code: [u32; 2] = [rdch(3, 29), stop_imm];
        let mut bytes = Vec::with_capacity(code.len() * 4);
        for w in code { bytes.extend_from_slice(&w.to_be_bytes()); }
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut recomp = RecompilerExecutor::new();
        let r_stall = recomp.execute(&prog);
        let park = r_stall.final_state.park_state.expect("park_state must be set");
        let stalled_channels = r_stall.final_state.channels.clone();

        // PPU sends a signal — wrong channel for an RDINMBOX park.
        let mut spu = SpuThread::new(0);
        spu.channels = stalled_channels;
        spu.park_state = Some(park);
        let wake = spu.signal_and_try_wake(0, 0xFF);
        assert_eq!(wake, SpuWakeResult::StillBlocked);
        assert!(spu.is_parked(), "thread must remain parked");

        // Resume anyway — interpreter must re-stall on the same rdch
        // because in_mbox is still empty.
        let interp = InterpreterExecutor::default();
        let r_resume = interp.resume_from_state(
            r_stall.final_state.gpr.as_ref(),
            r_stall.final_state.ls.as_ref(),
            &spu.channels,
            park.pc,
            100,
            r_stall.steps_executed,
        );
        assert_eq!(r_resume.stop_reason,
                   ExecutionStopReason::ChannelStall { channel: 29, is_write: false });
        let park_again = r_resume.final_state.park_state
            .expect("re-stall must re-park at same pc");
        assert_eq!(park_again.pc, park.pc, "park pc must be stable across re-stall");
    }

    // =====================================================================
    // R5.4c — SpuSingleThreadExecutor over the JIT backend
    //
    // Same park → wake → resume cycle as the differential-side tests,
    // but the FIRST run goes through the recompiler (exercises the JIT
    // stall → R5 partial fallback → snapshot path) before the
    // executor classifies the result as Parked. Resume continues
    // through `InterpreterExecutor::resume_from_state` per R5.4c
    // contract.
    // =====================================================================

    /// JIT runs `rdch r3, RDINMBOX(29); ai r4, r3, 1; stop 0xA1` →
    /// stalls on rdch → executor reports Parked → wake via
    /// ppu_push_inmbox_and_try_wake → resume via executor → Finished.
    #[test]
    fn r5_4c_executor_via_jit_rdch_full_cycle() {
        use rpcs3_spu_differential::{SpuExecEvent, SpuSingleThreadExecutor};
        use rpcs3_spu_thread::{SpuParkReason, SpuThread, SpuWakeResult};

        let rdch = (0x00Du32 << 21) | ((29 & 0x7F) << 7) | 3;
        let ai   = ((0x1Cu32 & 0xFF) << 24) | ((1u32 & 0x3FF) << 14) | ((3 & 0x7F) << 7) | 4;
        let stop = 0xA1u32 & 0x3FFF;
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&rdch.to_be_bytes());
        bytes.extend_from_slice(&ai.to_be_bytes());
        bytes.extend_from_slice(&stop.to_be_bytes());
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = RecompilerExecutor::new();
        let mut exec = SpuSingleThreadExecutor::new();

        let ev1 = exec.run_until_event(&mut backend, &prog);
        let (parked_pc, parked_snapshot, parked_steps) = match ev1 {
            SpuExecEvent::Parked { pc, reason, snapshot, steps } => {
                assert_eq!(reason, SpuParkReason::ChannelRead { channel: 29 });
                (pc, snapshot, steps)
            }
            other => panic!("expected Parked, got {other:?}"),
        };
        assert_eq!(parked_pc, 0x100,
                   "JIT path must produce the same park PC as the interpreter path");

        let mut shadow = SpuThread::new(0);
        shadow.channels = parked_snapshot.channels.clone();
        shadow.park_state = parked_snapshot.park_state;
        let wake_pc = match shadow.ppu_push_inmbox_and_try_wake(0x88) {
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
        match ev2 {
            SpuExecEvent::Finished { stop_code, snapshot, steps } => {
                assert_eq!(stop_code, 0xA1);
                assert!(steps > parked_steps);
                assert_eq!(snapshot.gpr[3] >> 96, 0x88u128);
                assert_eq!(snapshot.gpr[4] >> 96, 0x89u128);
                assert_eq!(snapshot.park_state, None);
                assert_eq!(snapshot.channels.in_mbox, None);
            }
            other => panic!("expected Finished, got {other:?}"),
        }
    }

    /// JIT runs `il r3,0x1111; wrch r3,OUT(28); il r3,0xCAFE; wrch r3,OUT(28); stop 0xB2` →
    /// second wrch stalls → executor reports Parked → drain via
    /// ppu_pop_outmbox_and_try_wake → resume via executor → Finished
    /// with new value in out_mbox.
    #[test]
    fn r5_4c_executor_via_jit_wrch_full_cycle() {
        use rpcs3_spu_differential::{SpuExecEvent, SpuSingleThreadExecutor};
        use rpcs3_spu_thread::{SpuParkReason, SpuThread, SpuWakeResult};

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

        let mut backend = RecompilerExecutor::new();
        let mut exec = SpuSingleThreadExecutor::new();

        let ev1 = exec.run_until_event(&mut backend, &prog);
        let (parked_pc, parked_snapshot, parked_steps) = match ev1 {
            SpuExecEvent::Parked { pc, reason, snapshot, steps } => {
                assert_eq!(reason, SpuParkReason::ChannelWrite { channel: 28 });
                (pc, snapshot, steps)
            }
            other => panic!("expected Parked, got {other:?}"),
        };
        assert_eq!(parked_pc, 0x10C, "park PC must be the second wrch (0x10C)");
        assert_eq!(parked_snapshot.channels.out_mbox, Some(0x1111));

        let mut shadow = SpuThread::new(0);
        shadow.channels = parked_snapshot.channels.clone();
        shadow.park_state = parked_snapshot.park_state;
        let (drained, wake) = shadow.ppu_pop_outmbox_and_try_wake();
        assert_eq!(drained, Some(0x1111));
        let wake_pc = match wake {
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
        match ev2 {
            SpuExecEvent::Finished { stop_code, snapshot, .. } => {
                assert_eq!(stop_code, 0xB2);
                assert_eq!(snapshot.channels.out_mbox, Some(0xFFFFCAFE),
                           "il sign-extends 0xCAFE to 0xFFFFCAFE");
                assert_eq!(snapshot.park_state, None);
            }
            other => panic!("expected Finished, got {other:?}"),
        }
    }

    /// Pre-existing fixtures (loop, fib, sumsq, brsl) must finish with
    /// SpuExecEvent::Finished, never Parked. Confirms R5.4c does not
    /// regress non-channel programs.
    #[test]
    fn r5_4c_executor_existing_fixtures_still_finish() {
        use rpcs3_spu_differential::{SpuExecEvent, SpuSingleThreadExecutor};

        let mut backend = RecompilerExecutor::new();
        let mut exec = SpuSingleThreadExecutor::new();

        for (label, prog) in [
            ("loop", loop_program()),
            ("fib", fibonacci_program()),
            ("sumsq", sum_of_squares_program()),
            ("brsl", brsl_ret_program()),
        ] {
            match exec.run_until_event(&mut backend, &prog) {
                SpuExecEvent::Finished { snapshot, .. } => {
                    assert_eq!(snapshot.park_state, None,
                               "{label}: park_state must be None");
                }
                other => panic!("{label}: expected Finished, got {other:?}"),
            }
        }
    }

    // =====================================================================
    // R5.4e — SpuPpuLockstepDriver over the JIT backend
    //
    // Drives a full PPU↔SPU script through `RecompilerExecutor` to
    // validate that the lockstep harness composes with the JIT path
    // (initial run goes through JIT; resume after wake still goes
    // through interpreter per R5.4c contract).
    // =====================================================================

    /// JIT runs `rdch r3, IN(29); ai r4, r3, 1; wrch r4, OUT(28); stop 0xA1` →
    /// SPU parks on rdch (JIT helper returns Stall, R5 partial fallback
    /// produces the Parked event the lockstep driver classifies). PPU
    /// pushes 41, wake fires, resume runs the rest, SPU finishes,
    /// PPU pops 42.
    #[test]
    fn r5_4e_lockstep_via_jit_rdch_handshake() {
        use rpcs3_spu_differential::{
            LockstepError, PpuAction, SpuEventKind, SpuPpuLockstepDriver,
        };
        use rpcs3_spu_thread::SpuParkReason;

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
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = RecompilerExecutor::new();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let trace: Result<_, LockstepError> = driver.run_script(&[
            PpuAction::ExpectPark {
                reason: SpuParkReason::ChannelRead { channel: 29 },
            },
            PpuAction::PushInMbox(41),
            PpuAction::ExpectFinished { stop_code: 0xA1 },
            PpuAction::PopOutMbox { expect: Some(42) },
        ]);
        let trace = trace.expect("JIT-side lockstep script must succeed");

        assert!(matches!(
            trace.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xA1 }
        ));
        assert_eq!(trace.final_snapshot.channels.in_mbox, None);
        assert_eq!(trace.final_snapshot.channels.out_mbox, None);
        assert_eq!(trace.final_snapshot.park_state, None);
    }

    /// JIT runs `il r3,0x1111; wrch r3,OUT(28); il r3,0x2222; wrch r3,OUT(28); stop 0xB2` →
    /// second wrch parks on full out_mbox via R5.2 helper Stall →
    /// R5 partial fallback produces Parked event → PPU drains and
    /// resumes through interpreter (R5.4c) → JIT-byte-exact result.
    #[test]
    fn r5_4e_lockstep_via_jit_wrch_backpressure() {
        use rpcs3_spu_differential::{
            PpuAction, SpuEventKind, SpuPpuLockstepDriver,
        };
        use rpcs3_spu_thread::SpuParkReason;

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

        let mut backend = RecompilerExecutor::new();
        let mut driver = SpuPpuLockstepDriver::new(&mut backend, prog);

        let trace = driver
            .run_script(&[
                PpuAction::ExpectPark {
                    reason: SpuParkReason::ChannelWrite { channel: 28 },
                },
                PpuAction::PopOutMbox { expect: Some(0x1111) },
                PpuAction::ExpectFinished { stop_code: 0xB2 },
                PpuAction::PopOutMbox { expect: Some(0x2222) },
            ])
            .expect("JIT-side wrch backpressure script must succeed");

        assert!(matches!(
            trace.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xB2 }
        ));
        assert_eq!(trace.final_snapshot.channels.out_mbox, None);
    }

    /// R5.5 — JIT-backend smoke test for the trace replay layer.
    /// Drives the rdch INMBOX handshake script through
    /// `RecompilerExecutor`. Initial run goes through JIT (channel
    /// helper returns Stall → R5 partial fallback produces Parked
    /// event the trace replay engine consumes); resume after wake
    /// goes through interpreter per R5.4c contract — documented as
    /// a R5.4d follow-up, not a correctness issue here.
    #[test]
    fn r5_5_trace_replay_jit_backend_smoke() {
        use rpcs3_spu_differential::{
            replay_trace, SpuEventKind, SpuWakeResultKind, TraceEvent,
            TraceReplayErrorKind,
        };
        use rpcs3_spu_thread::SpuParkReason;

        // Same shape as r5_4e_lockstep_via_jit_rdch_handshake but
        // expressed as a TraceEvent script.
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
        let prog = SpuProgram::new(0x100, 100).with_segment(0x100, bytes);

        let mut backend = RecompilerExecutor::new();
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
                TraceEvent::ExpectGprWord { reg: 4, lane: 0, value: 42 },
                TraceEvent::ExpectChannelState {
                    in_mbox: None,
                    out_mbox: None,
                    out_intr_mbox: None,
                    snr1: 0,
                    snr2: 0,
                },
            ],
        )
        .map_err(|e| {
            // If this fails, surface the kind with the event index.
            let _: TraceReplayErrorKind = e.kind.clone();
            e
        })
        .expect("JIT-backend trace replay must succeed");

        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xA1 }
        ));
        // Smoke check: summary contains expected text.
        let summary = report.summary();
        assert!(summary.contains("6 events processed"));
        assert!(summary.contains("Finished"));
    }

    /// R5.6 — JIT-backend run of the synthetic homebrew-like mailbox
    /// command protocol fixture. Exercises:
    ///   - rdch INMBOX park (JIT helper Stall → R5 partial fallback).
    ///   - wrch OUTMBOX backpressure park.
    ///   - PPU-side wake via push (in_mbox) and pop (out_mbox).
    ///   - branch + loop in the SPU code (brnz, br) — must remain
    ///     correct under JIT codegen.
    ///   - halt sentinel (cmd 0xFF → ceq → brnz → stop 0xD5).
    /// Resume after wake still goes through the interpreter per R5.4c
    /// contract — JIT-side resume is R5.4d, deferred.
    #[test]
    fn r5_6_trace_replay_mailbox_command_protocol_jit() {
        use rpcs3_spu_differential::{
            mailbox_command_protocol_program, mailbox_command_protocol_trace,
            replay_trace, SpuEventKind, FIXTURE_NAME_MAILBOX_PROTOCOL,
        };

        let prog = mailbox_command_protocol_program();
        let trace = mailbox_command_protocol_trace();

        let mut backend = RecompilerExecutor::new();
        let report = replay_trace(&mut backend, prog, &trace)
            .expect("mailbox command protocol must replay cleanly on JIT backend");

        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xD5 }
        ));
        assert_eq!(report.records.len(), 16);
        assert_eq!(report.final_snapshot.park_state, None);
        assert_eq!(report.final_snapshot.channels.in_mbox, None);
        assert_eq!(report.final_snapshot.channels.out_mbox, None);

        // Labeled summary mentions fixture name + final stop code so a
        // multi-trace run log is readable.
        let labeled = report.summary_with_label(FIXTURE_NAME_MAILBOX_PROTOCOL);
        assert!(labeled.contains(FIXTURE_NAME_MAILBOX_PROTOCOL));
        assert!(labeled.contains("0xd5"));
        assert!(labeled.contains("[15]"), "all 16 events must be listed");
    }

    /// R5.8 A.1+A.2 — JIT-backend smoke test for the JSONL capture
    /// pipeline. Parses the public reference JSONL fixture from
    /// `rpcs3-spu-differential::trace_fmt`, transforms it to a
    /// `Vec<TraceEvent>`, and replays through `RecompilerExecutor`.
    /// Initial run goes through JIT (channel helper Stalls →
    /// R5 partial fallback produces Parked events the trace replay
    /// engine consumes); resume after wake still uses interpreter
    /// per R5.4c contract — documented limitation, not a correctness
    /// issue here.
    ///
    /// This is the JIT-side end of the round-trip: parse → transform
    /// → replay through JIT must produce the same `Finished` report
    /// as the interpreter-side
    /// `replay_transformed_trace_through_interpreter` test in the
    /// differential crate.
    #[test]
    fn r5_8_jsonl_pipeline_jit_replay_smoke() {
        use rpcs3_spu_differential::{
            captured_events_to_trace, mailbox_command_protocol_program, parse_jsonl_trace,
            replay_trace, SpuEventKind, R5_6_REFERENCE_JSONL,
        };

        let events = parse_jsonl_trace(R5_6_REFERENCE_JSONL).expect("parse must succeed");
        assert_eq!(events.len(), 24, "reference fixture has 24 captured events");
        let trace = captured_events_to_trace(&events).expect("transform must succeed");
        assert_eq!(trace.len(), 16, "transformer must produce 16 R5.5 TraceEvents");

        let mut backend = RecompilerExecutor::new();
        let report = replay_trace(&mut backend, mailbox_command_protocol_program(), &trace)
            .expect("JIT-backend trace replay must succeed");

        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xD5 }
        ));
        assert_eq!(report.records.len(), 16);
        assert_eq!(report.final_snapshot.park_state, None);
        assert_eq!(report.final_snapshot.channels.in_mbox, None);
        assert_eq!(report.final_snapshot.channels.out_mbox, None);
    }

    // ---------------------------------------------------------------
    // R5.9e.6 — Recompiler replay over the per-SPU sequential
    // orchestrator on synthetic fixtures. Mirrors the interpreter
    // tests in `rpcs3_spu_differential::per_spu_replay::tests` but
    // with `RecompilerExecutor` as the backend. Differential goal:
    // Interpreter and Recompiler must agree on the per-SPU
    // `TraceReplayReport` for synthetic-supported paths.
    //
    // Real-trace v4 is NOT exercised here — its current divergence
    // (`Unimplemented opcode 0x33FFE748 @ pc=0x850`) is in the
    // interpreter's iteration-1 ISA subset and would block the JIT
    // path with the same root cause; surfacing it on R5.9e.6 would
    // re-emit the same diagnostic the R5.9e.5 v4 test already prints.
    // ---------------------------------------------------------------

    /// R5.9e.6 — single SPU at `target_spu=42` running the canonical
    /// mailbox-command-protocol fixture through the JIT backend via
    /// the per-SPU orchestrator. If `replay_per_spu_traces_with`
    /// correctly delegates to `replay_trace`, the result is the same
    /// `TraceReplayReport` the JIT-side R5.6 test already produces.
    #[test]
    fn r5_9e_6_per_spu_replay_recompiler_single_spu_mailbox_protocol() {
        use std::collections::BTreeMap;

        use rpcs3_spu_differential::{
            mailbox_command_protocol_program, mailbox_command_protocol_trace,
            replay_per_spu_traces_with, SpuEventKind,
        };

        let mut per_spu = BTreeMap::new();
        per_spu.insert(42u32, mailbox_command_protocol_trace());

        let mut programs = BTreeMap::new();
        programs.insert(42u32, mailbox_command_protocol_program());

        let reports = replay_per_spu_traces_with(&per_spu, &programs, |_| {
            RecompilerExecutor::new()
        })
        .expect("synthetic single-SPU JIT replay must succeed");

        assert_eq!(reports.len(), 1);
        let report = &reports[&42u32];
        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xD5 }
        ));
        assert_eq!(report.records.len(), 16);
        assert_eq!(report.final_snapshot.park_state, None);
        assert_eq!(report.final_snapshot.channels.in_mbox, None);
        assert_eq!(report.final_snapshot.channels.out_mbox, None);
    }

    /// R5.9e.6 — two SPUs at `target_spu=7` and `target_spu=42`, both
    /// running the same canonical fixture, replayed via the per-SPU
    /// orchestrator with a fresh `RecompilerExecutor` per SPU.
    /// Verifies (a) the orchestrator runs both, (b) iteration order
    /// is sorted by `target_spu`, (c) per-SPU JIT executors are
    /// independent (no JIT cache leak between SPUs).
    #[test]
    fn r5_9e_6_per_spu_replay_recompiler_two_spus_mailbox_protocol() {
        use std::collections::BTreeMap;

        use rpcs3_spu_differential::{
            mailbox_command_protocol_program, mailbox_command_protocol_trace,
            replay_per_spu_traces_with, SpuEventKind,
        };

        let mut per_spu = BTreeMap::new();
        per_spu.insert(7u32, mailbox_command_protocol_trace());
        per_spu.insert(42u32, mailbox_command_protocol_trace());

        let mut programs = BTreeMap::new();
        programs.insert(7u32, mailbox_command_protocol_program());
        programs.insert(42u32, mailbox_command_protocol_program());

        // Track per-SPU factory invocations so we can assert order.
        let mut seen: Vec<u32> = Vec::new();
        let reports = replay_per_spu_traces_with(&per_spu, &programs, |tgt| {
            seen.push(tgt);
            RecompilerExecutor::new()
        })
        .expect("synthetic two-SPU JIT replay must succeed");

        assert_eq!(reports.len(), 2);
        assert_eq!(
            seen,
            vec![7u32, 42u32],
            "factory must be called once per SPU in BTreeMap-sorted order"
        );
        for (tgt, report) in &reports {
            assert!(
                matches!(
                    report.final_event_kind,
                    SpuEventKind::Finished { stop_code: 0xD5 }
                ),
                "JIT replay for target_spu={tgt} should finish with stop_code 0xD5"
            );
            assert_eq!(report.records.len(), 16);
            assert_eq!(report.final_snapshot.park_state, None);
        }
    }

    /// R5.9e.6 — load-bearing differential check: feed the IDENTICAL
    /// per-SPU set through both Interpreter and Recompiler, compare
    /// reports for byte-exact agreement on the synthetic supported
    /// path. If this test ever diverges, the JIT codegen has drifted
    /// from the interpreter oracle on a path the iteration-1 ISA
    /// subset already covers — a real correctness regression.
    ///
    /// Equality covers: SPU final event kind, record count, total
    /// steps, and the `final_snapshot` (channels + park state). GPRs
    /// are checked via `diff_snapshots` (the R5.4c+ canonical helper).
    #[test]
    fn r5_9e_6_interpreter_and_recompiler_reports_match() {
        use std::collections::BTreeMap;

        use rpcs3_spu_differential::{
            diff_snapshots, mailbox_command_protocol_program, mailbox_command_protocol_trace,
            replay_per_spu_traces, replay_per_spu_traces_with, InterpreterExecutor,
        };

        let mut per_spu = BTreeMap::new();
        per_spu.insert(7u32, mailbox_command_protocol_trace());
        per_spu.insert(42u32, mailbox_command_protocol_trace());

        let mut programs = BTreeMap::new();
        programs.insert(7u32, mailbox_command_protocol_program());
        programs.insert(42u32, mailbox_command_protocol_program());

        let interp_reports =
            replay_per_spu_traces::<InterpreterExecutor>(&per_spu, &programs)
                .expect("interpreter run must succeed");
        let jit_reports =
            replay_per_spu_traces_with(&per_spu, &programs, |_| RecompilerExecutor::new())
                .expect("JIT run must succeed");

        // Same key set in same order.
        let interp_keys: Vec<u32> = interp_reports.keys().copied().collect();
        let jit_keys: Vec<u32> = jit_reports.keys().copied().collect();
        assert_eq!(interp_keys, jit_keys, "per-SPU key set must agree");

        for &tgt in &interp_keys {
            let i = &interp_reports[&tgt];
            let j = &jit_reports[&tgt];

            assert_eq!(
                format!("{:?}", i.final_event_kind),
                format!("{:?}", j.final_event_kind),
                "final_event_kind must match for target_spu={tgt}",
            );
            assert_eq!(
                i.records.len(),
                j.records.len(),
                "record count must match for target_spu={tgt}",
            );
            assert_eq!(
                i.total_steps, j.total_steps,
                "total_steps must match for target_spu={tgt} (interp={}, jit={})",
                i.total_steps, j.total_steps,
            );

            // Final snapshot byte-exact agreement via the canonical
            // diff helper. `is_identical` is the compound predicate:
            // pc, channels, GPRs, LS bytes, park state ALL match.
            let diff = diff_snapshots(&i.final_snapshot, &j.final_snapshot);
            assert!(
                diff.is_identical(),
                "final_snapshot differs for target_spu={tgt}: {diff:?}",
            );
        }
    }

    /// R5.9e.6 — per-SPU orchestrator's `MissingProgram` pre-flight
    /// gate also fires when the executor backend is the JIT. The
    /// orchestrator's bijection check is backend-agnostic, but
    /// re-asserting it here locks in the contract and prevents a
    /// future "Recompiler-only" regression where pre-flight is
    /// accidentally moved post-replay.
    #[test]
    fn r5_9e_6_recompiler_missing_program_error_preserves_target_spu() {
        use std::collections::BTreeMap;

        use rpcs3_spu_differential::{
            mailbox_command_protocol_trace, replay_per_spu_traces_with, MultiSpuReplayError,
            SpuProgram,
        };

        let mut per_spu = BTreeMap::new();
        per_spu.insert(13u32, mailbox_command_protocol_trace());
        let programs: BTreeMap<u32, SpuProgram> = BTreeMap::new();

        let err = replay_per_spu_traces_with(&per_spu, &programs, |_| RecompilerExecutor::new())
            .expect_err("missing program must reject pre-flight even on JIT backend");
        match err {
            MultiSpuReplayError::MissingProgram { target_spu } => {
                assert_eq!(target_spu, 13);
            }
            other => panic!("expected MissingProgram, got {other:?}"),
        }
    }
}
