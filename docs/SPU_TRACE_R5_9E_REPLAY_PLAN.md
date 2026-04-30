# R5.9e Multi-SPU Replay + SPU Image Capture — DONE (R5.9e.7 closes the arc)

**Status: R5.9e arc COMPLETE.** All seven sub-phases landed:

- R5.9e.1 (schema doc) **DONE — 2026-04-28**
- R5.9e.2 (parser support) **DONE — 2026-04-28**
- R5.9e.3 writer-emit + R5.9e.3-fix (writer side-file contract) **DONE — 2026-04-28**
- R5.9e.4 (`SpuProgram` builder from captured image) **DONE — 2026-04-28**
- R5.9e.5 (per-SPU sequential replay orchestrator) **DONE — 2026-04-29**
- R5.9e.6 (recompiler replay over synthetic per-SPU fixture; cross-backend `diff_snapshots(...).is_identical()` byte-exact on the canonical mailbox_command_protocol synthetic) **DONE — 2026-04-29**
- **R5.9e.7 (first replay-validated fixture commit: `single_spu_mailbox_v1`)** — **DONE — 2026-04-29**. CC0 PSL1GHT homebrew built via from-source `ps3toolchain` in a Docker container; captured through the R5.9c + R5.9e.3 writer; replay-validated end-to-end with `diff_snapshots(InterpreterExecutor, RecompilerExecutor).is_identical()`. `REPLAY_VALIDATED_TRACE_EXISTS` flag flipped `False` → `True`. Three general engine fixes co-landed (transformer initial-state inference for race-free single-round captures; lv2 stop-0x101/0x102 OUT_MBOX-drain modeling; `SpuProgram.initial_gpr_overrides` for PS3 SPU r1=0x3FFF0 SP convention).

**The R5.9e arc was the project's load-bearing path from "synthetic-only replay" to "real captured trace as validation oracle". That oracle now exists.**

## Replay-validated vs diagnostic-only — the canonical separation

Two distinct categories of trace live in this project. They have different commit locations, different acceptance criteria, and different roles:

| Category | Path | Role | Acceptance criteria | Examples |
|---|---|---|---|---|
| **Replay-validated fixture** | `behavior-freeze/fixtures/spu/traces/<name>.jsonl` (+ `.notes.md` + `behavior-freeze/fixtures/spu/images/<sha>.spuimg`) | Load-bearing oracle. Cross-backend byte-identical contract. Regression sentinel for the entire SPU stack. | All 5 § F.3 criteria met: parse + transform + replay × Interpreter + replay × Recompiler with `diff_snapshots(...).is_identical()` on the final snapshot. `REPLAY_VALIDATED_TRACE_EXISTS` reflects the existence of ≥ 1 such fixture. | **`single_spu_mailbox_v1`** (the only one today; canonical from R5.9e.7). |
| **Diagnostic-only trace** | `rust/rpcs3-spu-differential/tests/data/<name>.jsonl` (or per-test scratch dirs) | Surfaces gaps in coverage as they're hit. NOT promoted to byte-identical contract. Used by `#[ignore]`d diagnostic tests. | Parse + transform validated; replay attempt may legitimately fail at a documented blocker (DMA, unimplemented opcode, etc.). | `spurs_test_v3_real.jsonl` (R5.9d-era), `spurs_test_v4_real.jsonl` (R5.10a..p ISA-coverage iteration). Both DMA-bound at the protocol layer per § D.1 — replay can NOT progress past the MFC boundary. |

**Why the separation matters:**

- The `behavior-freeze/fixtures/spu/traces/` directory is a permission gate. Anything in it is a contract — every backend must agree byte-identically. Adding a non-replay-validated trace there would break the contract by definition.
- The diagnostic-only path under `tests/data/` lets the project retain coverage signals (e.g. "v4 surfaces MFC_LSA channel gap at pc=0x74C") without committing to a replay contract that the trace can't honor.
- `REPLAY_VALIDATED_TRACE_EXISTS = True` (in `behavior-freeze/harness/check_trace_fixtures.py`) is the single gate-flip that signals the project has its first oracle. R5.9e.7 flipped it; future iterations either preserve the flag (additive new fixtures) or document explicit reasons for any retraction.

This doc enumerates the decisions made in R5.9e to take real captured multi-SPU traces from "parse + transform validated" (the R5.9d milestone) through to "replay-validated against both Interpreter and Recompiler executors" (the R5.9e.7 milestone that flipped `REPLAY_VALIDATED_TRACE_EXISTS` to `True` and committed the first real-trace fixture to `behavior-freeze/fixtures/spu/traces/`).

**Cross-references:**
- Wire format (single-SPU + R5.9c multi-SPU writer): [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md).
- R5.9 multi-SPU plan (parser/transformer): [`SPU_TRACE_R5_9_MULTISPU_PLAN.md`](./SPU_TRACE_R5_9_MULTISPU_PLAN.md).
- C++ writer impl: [`../rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`](../rpcs3/Emu/Cell/SPUTraceJsonl.h).
- Patches: [`patches/spu_trace_jsonl_scaffolding.patch`](./patches/spu_trace_jsonl_scaffolding.patch), [`patches/spu_trace_jsonl_runtime_hooks.patch`](./patches/spu_trace_jsonl_runtime_hooks.patch).
- Replay engine: `rust/rpcs3-spu-differential/src/lib.rs` `replay_trace<E: SpuExecutor>(...)`.
- Status: [`PROJECT_STATUS.md`](./PROJECT_STATUS.md) §§ "R5.9c writer-emit landed" + "R5.9d diagnostic flip landed".

R5.9d landed parse + per-SPU transform on the real `spurs_test.self` trace (6 distinct `target_spu` `lv2_id`s, 40,042 events, all `seq`-monotonic). Replay was NOT exercised. R5.9e closes that gap.

---

## A. SPU image capture

### A.1 Where does the SPU bytecode go: JSONL event vs side-file?

**Recommendation: side-files, content-addressed by SHA-256.**

Pros of embedding in JSONL as one event per SPU:
- Self-contained — one trace = one file. Maintainer-friendly.
- Always in `seq` order, naturally bound to the rest of the timeline.
- Fits the existing parser: another `kind` variant.

Cons that disqualify the embedded approach:
- **Size.** A captured 256 KB local store base64-encoded inflates ~1.33×. With 6 SPUs in spurs_test, that's ~2.0 MB of base64 padding added to a ~4.85 MB trace — ~40% bloat for content the parser doesn't even need to read line-by-line.
- **Mixing binary into a text-line format breaks the "one event per line" ergonomics.** A 350 KB JSON line is not human-greppable, breaks line-counting tools, and produces fragile diffs.
- **No deduplication.** If two SPUs share the same `.spucore.elf` segment (typical for SPURS workers), the trace contains the bytes twice.
- **No lazy load.** Parser would always read every byte of every image even when running tests that only need parse + transform (R5.9d).

Side-files solve all four. Trace remains line-oriented and small; images live as standalone files; identical images dedup naturally; replay engine reads only the images it actually needs.

### A.2 How does the JSONL reference a side-file?

A new `kind = "spu_image"` event, emitted at SPU thread creation BEFORE any other event for that SPU:

```jsonc
{
  "seq": <u64>,
  "side": "spu",                         // captured-from-SPU-side; same `side` discrimination as other SPU events
  "kind": "spu_image",
  "target_spu": <u32>,                   // the lv2_id (R5.9c convention)
  "image_sha256": "<64-hex>",            // SHA-256 of the captured image bytes
  "load_addr": <u32>,                    // base address in LS where the image was loaded; usually 0
  "size": <u32>,                         // size in bytes captured (multiple of 4)
  "entry_pc": <u32>                      // entry point the SPU started at; needed for SpuProgram.entry_pc
}
```

**Why every field is load-bearing:**
- `target_spu` — ties this event to the per-SPU group the transformer produces.
- `image_sha256` — lookup key for the side-file. Content-addressed = automatic dedup AND tamper detection.
- `load_addr` — usually 0 but Raw SPU mode can load images at non-zero offsets; capturing it makes replay deterministic.
- `size` — explicit so the parser can validate `image_sha256` matches without reading the side-file twice.
- `entry_pc` — `SpuProgram.entry_pc`. Cannot be derived from the trace's first SPU event because that PC may already be inside a function body (e.g., after a sync barrier).

**Where the side-file lives:**
- Beside the trace, in a parallel directory: `<trace>.images/<sha256>.spuimg`. Example: `behavior-freeze/fixtures/spu/traces/foo.jsonl` + `behavior-freeze/fixtures/spu/traces/foo.images/abc123…cd.spuimg`.
- Across multiple traces, identical images can share a single content-addressed directory: `behavior-freeze/fixtures/spu/images/abc123…cd.spuimg`. The trace's `.notes.md` documents which path style is in use.
- Recommendation for R5.9e.3 (writer): emit alongside the trace at `<trace_path>.images/`. Recommendation for R5.9e.7 (committed fixtures): centralize at `behavior-freeze/fixtures/spu/images/` to keep the per-trace dirs sparse.

### A.3 Capture full LS or just text/code?

**Capture the full 256 KB LS at thread-creation time.**

Reasons against capturing only text/code:
- The boundary between code and data isn't precisely known to the writer at runtime — RPCS3 sees the LS as a flat byte array. Heuristics (e.g., "stop at first 0x00 0x00 0x00 0x00 run") are wrong on real workloads (zero-padded jump tables, BSS-style zero data).
- Self-modifying code inside the data range would silently break with a code-only capture.
- 256 KB × 6 SPUs = 1.5 MB raw, ~512 KB compressed. Acceptable for the kinds of workloads we capture.

Reasons against capturing the full LS at every event:
- Quadratic explosion: per-instruction LS dumps × millions of instructions = gigabytes per trace. Useless for the replay model R5.9e targets.
- Real workloads don't self-modify often. Mitigation in A.5.

### A.4 How to identify / hash the image?

**SHA-256 of the bytes-as-captured.** Not SHA-1 (collision concerns over the 10-year horizon). Not MD5. Not a custom hash.

Trade-offs:
- 64 hex chars per image. ~2× larger than SHA-1's 40, but still negligible per trace (6 SPUs = 384 bytes of hex strings).
- Standard tooling (`sha256sum`, `python -c "import hashlib;…"`) computes it without extra deps.
- Reverse lookups: `find <dir> -name '<hash>.spuimg'` is O(1) given the content-addressed layout.

### A.5 Self-modifying code (SMC)

**Out of scope for R5.9e.1–.7.** SMC breaks the "one image per SPU per trace" assumption — the replay engine would need a series of `spu_image_patch` events (or per-instruction LS snapshots) and a way to re-warm the JIT after each patch.

Mitigation: at R5.9e.2 (parser support), ADD a strict invariant — the parser MUST detect SMC indicators and reject with `TraceParseError::UnsupportedSelfModifyingCode { target_spu, event_index }`. SMC indicators:
- Any `spu_wrch` to `MFC_RdAtomicStat` (= `dmacb` triggering a DMA into the SPU's own LS).
- Any `dsync` or `sync` that a future writer extension might surface as a side-channel event.

The R5.9c writer doesn't capture DMA at all, so the v1 detection is heuristic. A future R5.9f could lift this restriction.

### A.6 Legality / license preservation

The image bytes are part of a homebrew binary. The fixtures-dir hard rule (already in [`behavior-freeze/fixtures/spu/traces/README.md`](../behavior-freeze/fixtures/spu/traces/README.md)) extends to images:

- Only homebrews authored by the user OR explicitly redistributable (public-domain, CC0, BSD/MIT/Apache, LGPL with attribution) belong in `behavior-freeze/fixtures/spu/images/`.
- `spurs_test.self` is RPCS3-source-bundled (RPCS3 itself is GPLv2+); its image bytes therefore qualify for `behavior-freeze/fixtures/spu/images/` IF a `.notes.md` documents the chain-of-custody (RPCS3 commit + capture command + R5.9c sha256s).
- Commercial PS3 game extraction is FORBIDDEN, full stop. Same rule that already covers `.jsonl` fixtures.

The gate (`check_trace_fixtures.py`) extends to require: every `.jsonl` referencing an `image_sha256` MUST have either a sibling `<sha256>.spuimg` file OR a `.notes.md` line `external-image: <sha256> @ <path>` documenting where the image lives. No silent missing-image refs.

---

## B. Mapping captured trace → `SpuProgram`

### B.1 What `SpuProgram` needs

The current `SpuProgram` shape (per [`rpcs3-spu-differential/src/lib.rs`](../rust/rpcs3-spu-differential/src/lib.rs)):
- `entry_pc: u32` — start PC.
- `code: Vec<u8>` (or equivalent) — instruction bytes loaded into LS.
- `data_segments: …` — pre-initialized data regions.
- `max_steps: usize` — replay budget.

The captured `spu_image` event provides everything except `data_segments` and `max_steps`. The image bytes ARE the LS content; there's no separate "data" because LS is unified.

### B.2 `entry_pc` source of truth

**`spu_image.entry_pc`**, NOT "first event's PC".

Pre-R5.9e instinct says "use the first SPU event's PC as entry_pc". That's wrong because the first event's PC may be inside a function reached after some setup (e.g., `init_runtime() → main_loop()` and the first capturable event is in `main_loop`). Replaying from `main_loop` skips the init.

The writer captures `entry_pc` directly from the SPU thread's startup state at thread-creation time. R5.9e.3 (writer extension) is responsible for sourcing this from RPCS3's `spu_thread::start_pc` or equivalent.

### B.3 `max_steps` derivation

**Heuristic: 4× the number of SPU-side events for that SPU's filtered subsequence, capped at 100M.**

Rationale:
- Real workloads execute many SPU instructions between adjacent observable events (most ALU ops are not captured). 4× is a back-of-envelope guard against runaway loops while leaving slack.
- 100M cap prevents pathological traces from running forever.
- Per-SPU. The transformer already groups events per SPU, so this is local to each replay invocation.

### B.4 `final_state` → expected stop / GPRs / channels

The `final_state` event for SPU N becomes the terminal assertion for that SPU's replay:
- Replay must reach `Finished{stop_code}` matching the trace's `spu_stop` event (already part of how `replay_trace` ends).
- After `Finished`, compare `report.final_snapshot.channels` against `final_state.channels`.
- For each `(reg, value)` in `final_state.gpr_lane_zero`, assert `report.final_snapshot.gpr[reg]._u32[0]` (or the lane-0 equivalent depending on backend) matches.

The current `replay_trace` already emits `ExpectGprWord` and `ExpectChannelState` from the transformer's output. R5.9e doesn't change that contract — it just ensures the per-SPU input fed to `replay_trace` includes all the assertion `TraceEvent`s the transformer already produces.

### B.5 Multiple SPUs with identical or different images

- **Identical** (e.g., all 6 SPURS workers loading the same `pat.spucore.elf`): the parser sees 6 `spu_image` events with the same `image_sha256`. Side-file dedup means only one `.spuimg` file on disk. Builder constructs N `SpuProgram` instances each pointing to the same byte buffer. No code change needed.
- **Different** (e.g., one SPU runs `ipc.spucore.elf` and another runs `pat.spucore.elf`): different `image_sha256` per SPU. Side-files differ. Builder constructs distinct `SpuProgram`s. Per-SPU sequential replay handles this naturally.

### B.6 New error variants for the builder

Add to `TraceTransformError` (or a new `SpuProgramBuildError` if scope-creep is undesirable):
- `MissingImageForSpu { target_spu }` — SPU has events but no `spu_image` event. Parse + transform succeed; replay can't proceed.
- `ImageHashMismatch { target_spu, expected, actual }` — `.spuimg` file content doesn't match the declared `image_sha256`. Tamper detection.
- `ImageFileMissing { target_spu, sha256, expected_path }` — declared image but no file on disk.

R5.9e.4 (builder) raises these; R5.9e.5 (replay engine) propagates them.

---

## C. Replay engine

### C.1 Per-SPU sequential first; lockstep deferred

**Recommendation: ship per-SPU sequential replay in R5.9e.5; defer lockstep to R5.9f if needed.**

Per-SPU sequential:
- For each `(target_spu, Vec<TraceEvent>)` in the per-SPU map, build a `SpuProgram`, instantiate one `SpuExecutor`, call `replay_trace(executor, program, events)`. Loop over SPUs.
- Cross-SPU mailbox correlation is preserved IF the trace records the PPU-side push/pop events (which the R5.9c writer does — `record_ppu_push_inmbox` and friends). When SPU A's per-SPU replay encounters a `PpuPushInMbox{value}` event in its filtered subsequence, the replay engine treats it as "PPU pushed this value into MY in_mbox" and the SPU consumes it on its next `rdch`. The fact that SPU B's `wrch` produced that value isn't visible to SPU A's replay, but SPU A doesn't need to know — the push event is the abstract barrier.
- Cost: one full-state SPU executor per SPU, sequentially. Memory bounded; runtime is proportional to total SPU instructions across all SPUs.

Lockstep:
- Maintain N executors simultaneously. Use the global `seq` order to pick which executor to step next.
- Required IFF a real workload reveals a divergence the per-SPU model can't catch (e.g., one SPU's `wrch` value is computed from a value it `rdch`'d in the same `seq` cycle from another SPU's `wrch` — extremely tight inter-SPU coupling).
- Cost: a `MultiSpuLockstepDriver` mirroring the existing `SpuPpuLockstepDriver`. ~500 lines of Rust + new tests.
- Defer until empirically motivated.

### C.2 Feeding `Vec<TraceEvent>` into `SpuExecutor`

The R5.9b transformer already produces `BTreeMap<u32, Vec<TraceEvent>>`. R5.9e.5's replay driver iterates this map:

```rust
pub fn replay_per_spu_traces<E: SpuExecutor + Default>(
    per_spu: &BTreeMap<u32, Vec<TraceEvent>>,
    programs: &BTreeMap<u32, SpuProgram>,
) -> Result<BTreeMap<u32, TraceReplayReport>, ReplayError> {
    let mut reports = BTreeMap::new();
    for (target_spu, events) in per_spu {
        let prog = programs.get(target_spu)
            .ok_or(ReplayError::MissingProgram { target_spu: *target_spu })?;
        let mut backend = E::default();
        let report = replay_trace(&mut backend, prog.clone(), events)?;
        reports.insert(*target_spu, report);
    }
    Ok(reports)
}
```

This is the single new public function R5.9e.5 adds. Strictly additive; the existing single-SPU `replay_trace` is untouched and continues to work for `R5_6_REFERENCE_JSONL`.

### C.3 Are PPU-side events sufficient for replay without a real PPU?

**Yes.** The trace records `PpuPushInMbox`, `PpuPopOutMbox`, `PpuSignal` as boundary events; the replay engine treats them as oracle inputs (it just plays the captured value into the SPU's mailbox/signal at the right point in seq order). No PPU runtime is needed because the PPU's behavior is entirely encoded in the trace.

The only cases where a real PPU would be needed:
- DMA where the PPU sets up data in main memory and the SPU reads from it via `mfc`-issued MMIO. R5.9e doesn't capture DMA, so these traces fail at SMC detection (A.5) or get an `UnsupportedDma` error.
- Interrupt handlers that fire asynchronously based on PPU-side events not in the trace. Out of scope.

### C.4 Snapshot comparison strategy

Per-SPU. Each `report.final_snapshot` is compared against the trace's `final_state` event for that SPU:
- `gpr_lane_zero` entries: exact equality.
- `channels` snapshot: exact equality (in_mbox, out_mbox, out_intr_mbox, snr1, snr2).

If any field mismatches, `replay_trace` already returns a structured error pointing to the divergent field; R5.9e.5 just bubbles that up with a per-SPU prefix.

---

## D. Scope limits

R5.9e ships with these UNSUPPORTED categories. Each gets a documented rejection at parse / transform / replay layer with a specific error.

### D.1 DMA capture

**Not in trace.** Any homebrew that issues `dmacb`/`dmaqu`/etc to move data between LS and main memory cannot replay correctly because the main-memory side is not modeled.

R5.9e.2 (parser) detects: any `spu_wrch` event with `channel == MFC_Cmd (== 21)` raises `UnsupportedDmaInTrace { target_spu, event_index }`. Strict.

### D.2 Shared LS / inter-SPU communication via DMA

Same root cause as D.1. SPU A reads from SPU B's LS via DMA-to-LS. Same rejection path.

### D.3 Real scheduling

**Not modeled.** Replay is sequential per the captured order; it does NOT model the PPU's preemption of an SPU mid-instruction or the atomic-cache scheduler in RPCS3. Acceptable because (a) the trace already encodes the observable order, and (b) replay-time scheduling differences would surface as different `seq` orderings, which the trace records faithfully.

### D.4 spurs_test specifically

**ISA-coverage phase complete; v4 has reached the DMA/MFC boundary documented in § D.1 + this section's pre-R5.10a prediction.** **R5.10p (2026-04-30): post-R5.10o blocker authoritatively classified as DMA command present + unsupported replay boundary. v4 SPU at pc=0x74C..0x07A8 executes a complete MFC GET DMA setup-and-issue sequence (full 28 MFC WRCH + 4 RDCH in `.spuimg`; 4 distinct MFC_Cmd dispatches; first runtime-reached at pc=0x079C). v4 has exited replay-valid scope per R5.9e.2 § D.1.** The progression is:

| Date | Blocker pc | inst (hex) | Mnemonic | Status |
|---|---|---|---|---|
| 2026-04-29 (R5.9e.5/.6) | 0x850 | 0x33FF2E08 | LQR | Identified as ISA gap |
| 2026-04-29 (R5.10a)     | 0x850 | 0x33FF2E08 | LQR | Decoded: class B (decoder + interpreter both gap) |
| 2026-04-29 (R5.10b)     | 0x854 | 0x3EE00085 | **CDD** | LQR landed; SPU advances 1 instr; CDD is now the first gap |
| 2026-04-29 (R5.10c)     | 0x854 | 0x3EE00085 | **CDD** | Decoded: RI7-form, `cdd r5, r1, 0`. Class B. C-family scope mapped: 8 opcodes total (CBX/CHX/CWX/CDX RR + CBD/CHD/CWD/CDD RI7); 4 used in v4 (CBX×2, CBD×3, CHD×3, CWD×5, CDD×2 = 15 instances). All ra=1 (stack pointer) — typical compiler "insert into stack frame" pattern. C++ ref: `SPUInterpreter.cpp:931`. Pure compute, no side effects. Implementation hint for R5.10d: a single `InsertControl { rt, ra, mode, granularity }` variant covers all 8 family members in ~30 lines. |
| 2026-04-29 (R5.10d)     | 0x864 | 0x32880003 | (was misreported as `0x328AB003` in the R5.10d summary; decimal 847,773,699 = `0x32880003`) | Full C-family (all 8 opcodes) landed in decoder + interpreter via single `InsertControl { rt, ra, source: ImmI7\|RegRb, granularity: B\|H\|W\|D }` variant + 8-arm parameterized step. Family covers `p11 ∈ {0x1D4..0x1D7, 0x1F4..0x1F7}`. v4 SPU advances 4 instructions past CDD (0x854 → 0x864). JIT codegen unchanged (R5 partial fallback). |
| 2026-04-29 (R5.10e)     | 0x864 | 0x32880003 | **FSMBI** | Decoded: RI16-form, `fsmbi r3, 0x1000` (top-9 = `0x065`; `inst >> 21 = 0x194`). Class B (decoder + interpreter both gap; JIT inherits via R5 partial fallback). FSM-family scope: 4 opcodes total (FSM RR-word p11=0x1B4 — already in Rust + 6 v4 uses; FSMH RR-halfword p11=0x1B5 — gap, 0 v4 uses; FSMB RR-byte p11=0x1B6 — gap, 8 v4 uses; FSMBI RI16-byte p9=0x065 — **gap, 8 v4 uses**, this opcode). Pure compute (i16 → 16-byte mask), no side effects. C++ ref: `SPUInterpreter.cpp:1671`. R5.10d summary errata recorded: blocker hex was wrong by decimal-misconversion; opcode is FSM-family extension, NOT a new opcode family. |
| 2026-04-29 (R5.10f)     | 0x868 | 0x23FF2B02 | **STQR** (Store Quadword Relative; top-9 = `0x047`) | Remaining FSM-family (FSMH p11=0x1B5, FSMB p11=0x1B6, FSMBI p9=0x065) landed in decoder + interpreter. Decoder: existing FSM 0x1B4 preserved as `Unary` (JIT codegen path untouched); FSMH/FSMB added to `is_unary_rr_11bit`; new `FormSelectMaskImm { rt, imm16 }` variant for FSMBI. Interpreter: 3 new arms with exact-byte mask construction matching C++ semantics; 9 interpreter + 2 decoder unit tests added. JIT codegen unchanged (FSMH/FSMB/FSMBI all route through R5 partial fallback; recompiler tests stay 139). v4 SPU advances 1 instruction past FSMBI; new blocker is STQR (RI16-form sibling of LQR, p9=0x047). Diagnose-then-implement R5.10g/h is the natural next step — STQR is structurally simpler than FSM-family because it's a direct mirror of the already-implemented R5.10b LQR. |
| 2026-04-29 (R5.10g)     | 0x86C | 0x16080183 | candidate **ANDBI** (And Byte Immediate, top-8=`0x16`, RI10-form; defer to R5.10h for authoritative decoding) | STQR landed in decoder + interpreter as direct mirror of R5.10b LQR. Confirmed up-front via C++ side-by-side ([`SPUInterpreter.cpp:1634`](../rpcs3/Emu/Cell/SPUInterpreter.cpp#L1634) STQR vs `:1690` LQR — identical `spu_ls_target(pc, imm16)` address, only direction differs). Decoder: new `StoreRel { rt, target_pc }` variant kept separate from R5.10b's `LoadRel` (preserves existing test surface). Dispatch arm at p9=`0x047`. Interpreter: 6-line arm using `write_qword_be`. 1 decoder + 4 interpreter tests added. JIT codegen unchanged (recompiler stays 139). v4 SPU advances 1 instruction; new blocker `0x16080183` (decimal 369,623,427) at pc=`0x86C`, top-8=`0x16` likely ANDBI (RI10 byte immediate). |
| 2026-04-29 (R5.10h)     | 0x86C | 0x16080183 | **ANDBI** (And Byte Immediate, top-8=`0x16`, RI10-form) | Decoded: `andbi r3, r3, 0x20` (rt=3, ra=3, i8=0x20 per RPCS3 `bf_t<u32, 14, 8>`). Class B-with-caveat. C++ ref: [`SPUInterpreter.cpp:1775`](../rpcs3/Emu/Cell/SPUInterpreter.cpp#L1775) — pure `gpr[rt] = gpr[ra] & broadcast8(i8)`. **Two pre-existing latent issues uncovered**: (1) interpreter has no byte-imm arms at all (gap surfaces here as the v4 `Unimplemented` line); (2) decoder's i8 extraction at `lib.rs:545` uses `(raw >> 16) & 0xFF` but should be `(raw >> 14) & 0xFF` per RPCS3 — silent today (no end-to-end byte-imm path with non-zero i8 is exercised), but for the v4 ANDBI sites the decoder produces `i8=0x08` instead of `0x20`. Family scope mapped: 6 byte-imm opcodes (ORBI/ANDBI/XORBI/CGTBI/CLGTBI/CEQBI). v4 has 18 byte-imm instances across ANDBI(14)/CLGTBI(2)/CEQBI(2). Wider RI10 ALU interpreter gap also includes SFI(14)/CLGTI(7)/ANDHI(2) = 23 future blockers — out of R5.10h family scope, deferred to R5.10j. R5.10i (implement byte-imm in interpreter + fix decoder i8) is the natural fix slice; R5.10i MUST couple both changes to prevent JIT-vs-interpreter divergence. JIT codegen for byte-imm already exists at [`jit.rs:1118-1132`](../rust/rpcs3-spu-recompiler/src/jit.rs#L1118) and inherits the decoder's wrong i8. |
| 2026-04-29 (R5.10i)     | 0x6F0 | 0x5C07C1A0 | **CLGTI** (Compare Logical [unsigned] Greater-Than Immediate, word-imm RI10, top-8=`0x5C`) | Full byte-immediate RI10 family (ORBI/ANDBI/XORBI/CGTBI/CLGTBI/CEQBI) landed in interpreter; decoder i8 extraction bug fixed (`(raw>>16)`→`(raw>>14)`). 1 JIT differential regression test added that proves end-to-end correctness with non-zero `i8` (would have failed pre-fix because JIT received wrong byte from decoder). Two pre-existing JIT unit tests at `jit.rs:3304+3320` had been encoding instructions with the same buggy `<< 16` shift; their LOCAL test-encoding helpers were realigned to `<< 14` (TEST-encoding fix only — JIT codegen at `jit.rs:1500..1567` untouched). v4 SPU now takes a previously-unreachable code path (because byte-imm masks/compares produce correct values that feed downstream branches) and BACKWARDS-jumps to pc=0x6F0 where it hits CLGTI. CLGTI is one of the future-blockers explicitly predicted in R5.10h's "Wider RI10 interpreter gap" table (7 v4 instances). R5.10j should diagnose-then-implement CLGTI — likely as part of a wider word-imm slice (CLGTI + SFI + ANDHI = 23 v4 instances). |
| 2026-04-29 (R5.10j)     | 0x6F0 | 0x5C07C1A0 | **CLGTI** (decoded: `clgti r32, r3, 31`; rt=32, ra=3, si10=0x01F=31) | Decode-only iteration. Class **A** (decoder OK, JIT OK, ONLY interpreter arm missing). C++ ref: [`SPUInterpreter.cpp:1862`](../rpcs3/Emu/Cell/SPUInterpreter.cpp#L1862) — pure per-word unsigned compare via XOR-with-0x80000000 trick. **Full wider RI10 ALU family mapped**: 18 opcodes total = 9 implemented + 5 Class-A interpreter-only gaps (CLGTI 7 v4 / SFI 14 v4 / MPYI 0 / MPYUI 0 / AHI 0 = 21 v4 instances) + 4 Class-B triple-gaps in decoder + JIT + interpreter (ORHI/SFHI/ANDHI/XORHI; only ANDHI has v4 use, 2 instances). All 7 CLGTI v4 sites use immediate `0x0F` or `0x1F` — typical compiler threshold/bound check pattern (likely feeds BRZ/BRNZ branches that gate yet-more-unreachable code paths). **R5.10h prediction empirically validated**: that doc's "Wider RI10 interpreter gap" table called CLGTI + SFI + ANDHI as future blockers; R5.10i unblocked byte-imm and v4 flowed straight to CLGTI as predicted. Recommended R5.10k: Class-A subfamily (5 opcodes, 21 v4 instances unblocked, NO decoder/JIT changes — interpreter-only additions matching R5.10g/R5.10i precedent of preserving already-working layers). Defer Class-B halfword bitops to a later slice when ANDHI surfaces as runtime-reached blocker. |
| 2026-04-29 (R5.10k)     | 0x72C | 0x3FBF0E96 | **ROTQMBYI** (top-11=`0x1FD`, Rotate Quadword Bytes Mask Immediate, RI7-form quadword shift-imm) | Class-A wider-RI10 subfamily landed: CLGTI 0x5C, SFI 0x0C, AHI 0x1D, MPYI 0x74, MPYUI 0x75 — all 5 in interpreter only (decoder + JIT already had these primaries). 9 interpreter unit tests + 2 JIT differential regression tests added (Class-A coverage + MPYI/MPYUI signedness divergence). v4 SPU advances **15 instructions** past CLGTI (pc 0x6F0 → 0x72C) — the +60-byte jump covers CLGTI plus ~14 successors, several of which are SFI/AHI/MPYI/MPYUI siblings now executable in the same slice (cascading unblock pattern). New blocker `ROTQMBYI` is sibling of ROTQBYI 0x1FC / SHLQBYI 0x1FF / SHLQBII 0x1FB which the decoder already knows; 0x1FD itself is decoder + interpreter gap (RPCS3 ref: [`SPUOpcodes.h:185`](../rpcs3/Emu/Cell/SPUOpcodes.h#L185)). R5.10l should diagnose-then-implement, possibly bundling with the rest of the quadword-shift-imm family if other gaps are mapped. |
| 2026-04-29 (R5.10l)     | 0x72C | 0x3FBF0E96 | **ROTQMBYI** (decoded: `rotqmbyi r22, r29, 0x7C`) | Decode-only iteration. Class **B** (decoder + interpreter both gap; JIT inherits via R5 partial fallback). C++ ref: [`SPUInterpreter.cpp:981`](../rpcs3/Emu/Cell/SPUInterpreter.cpp#L981) — pure 128-bit byte-shift-right with zero-fill, mask `(0 - imm7) & 0x1F`. **Full 15-opcode quadword shift/rotate family mapped** in 4 sub-shapes (RR-bit-of-byte 3, RR-bit 3, RR-byte 3, RI7-bit 3, RI7-byte 3); v4 uses 4 opcodes (ROTQBY 1, ROTQBYI 16, ROTQMBYI 2, SHLQBYI 9 = 28 instances). **Third pre-existing latent labeling bug uncovered**: `encode::shlqbyi` at [`lib.rs:2445`](../rust/rpcs3-spu-interpreter/src/lib.rs#L2445) packs `0x1FB` but SHLQBYI is `0x1FF` in C++ (and `0x1FB` is SHLQBII). Silent today (no end-to-end SHLQBYI/SHLQBII path is exercised — v4 has 9 SHLQBYI sites but execution hasn't reached them yet); will surface immediately once v4 advances past ROTQMBYI. **R5.10m must couple ROTQMBYI implementation with SHLQBYI/SHLQBII fix** — same diagnose-then-couple pattern as R5.10h→R5.10i decoder-i8 fix. R5.10m JIT differential regression test should exercise both ROTQMBYI (via partial fallback) AND SHLQBYI (post-fix) end-to-end. |
| 2026-04-30 (R5.10p)     | 0x74C | 0x21A00818 | **WRCH ch16 (MFC_LSA)** — classified `DMA command present + unsupported replay boundary` | Decode-only iteration. Disasm of pc=0x720..0x07AC shows the textbook SPU MFC GET DMA sequence: ch16 LSA + ch17 EAH + ch18 EAL + ch19 Size + ch20 TagID + **ch21 Cmd=0x40 (GET) at pc=0x079C** + ch22 WrTagMask + ch23 WrTagUpdate + RDCH ch24 RdTagStat (blocking-wait at pc=0x07A8) + LQA r4, [0x3FFE0] (consumes the just-DMA'd data at pc=0x07AC). v4 image has 28 MFC WRCH + 4 MFC RDCH, 4 distinct MFC_Cmd dispatches, ZERO non-MFC channel ops. JSONL trace v4 has 40046 spu_wrch events but 100% are ch28 (SPU_WrOutMbox) — R5.9c writer captures ONLY ch28; the MFC channel writes are invisible to the trace. R5.9e.2 `UnsupportedDmaInTrace` parse-time gate doesn't fire (no spu_wrch ch21 events captured); current diagnostic surfaces at runtime in the interpreter ("wrch: unknown channel"). C++ ref [`SPUThread.cpp:6244+`](../rpcs3/Emu/Cell/SPUThread.cpp#L6244): ch16-20/22-23 are pure register stores to `ch_mfc_cmd.{lsa,eah,eal,size,tag}`; ch21 dispatches DMA via `do_mfc()`/PPU vm:: accessors; ch24 blocks until completion. **R5.10p empirically validates the R5.9e.2 § D.1 + § D.4 prediction**: with full ISA coverage now in place, the v4 path advances to exactly the DMA boundary that doc predicted. Three structural-decision options for next step: (A) mock MFC + fake-success ch21/ch24 (~40 lines, but produces silent fake-success), (B) document v4 as DMA-bound + pivot to R5.9e.7 single-SPU non-DMA homebrew (recommended, matches plan), (C) begin R5.9f writer+parser+replay DMA-oracle model (multi-week new phase). Recommendation: pause R5.10 series at R5.10p — phase boundary cleanly closed. |
| 2026-04-30 (R5.10o)     | **0x74C** | **0x21A00818** | **WRCH ch16 (MFC_LSA)** — qualitative shift: NOT an opcode coverage gap; this is the DMA/MFC channel layer R5.9e.2 deferred | LQA + STQA bundle landed in decoder + interpreter, closing the entire RI16 qword L/S family (4/4 opcodes done). 2 new SpuInstKind variants `LoadAbs`/`StoreAbs` (kept distinct from `LoadRel`/`StoreRel` for semantic clarity — `Rel` would lie about absolute addressing). Decoder + interpreter arms ~6 lines each at primaries 0x061 (LQA) and 0x041 (STQA); reuses `read_qword_be`/`write_qword_be` helpers. Encode helpers `encode::lqa(rt, imm16)` + `encode::stqa(rt, imm16)` added. 4 decoder + 7 interpreter + 1 JIT differential tests; explicit anti-regression locks LQR/STQR PC-relative semantics in BOTH layers. JIT codegen unchanged (LoadAbs/StoreAbs are new variants → wildcard → R5 partial fallback). v4 SPU advances 6 instructions past STQA (only the 1st STQA at 0x734 reached; 2nd STQA at 0x764 + 5 LQA sites NOT reached this iteration — execution hit MFC channel 16 first at pc=0x74C). **Major milestone: ISA-coverage phase complete for the linear v4 path through pc=0x74C.** Next iteration (R5.10p) needs to handle MFC channel coverage OR decide that v4 has exited replay-valid scope per R5.9e.2 § D.1. |
| 2026-04-29 (R5.10n)     | 0x734 | 0x20FFFA09 | **STQA** (decoded: `stqa r9, [0x3FFD0]`; top-9=`0x041`, RI16 absolute-store) | Decode-only iteration. Class **B** for STQA (decoder + interpreter both gap; JIT inherits via R5 partial fallback). C++ ref: [`SPUInterpreter.cpp:1594`](../rpcs3/Emu/Cell/SPUInterpreter.cpp#L1594) — pure `LS[(i16<<2) & 0x3FFF0] = gpr[rt]`. **LQA (sibling absolute-load, top-9=`0x061`) is also a triple-gap with 5 v4 sites** at pc=0x07AC..0x0824 immediately after the 2 STQA sites — the 7 instances form a top-of-LS save/restore prologue-epilogue (STQA writes r9/r20 at targets `0x3FFD0`/`0x3FFB0`; LQA reads r4/r38/r45/r46/r50 at `0x3FFE0`/`0x3FFF0`). RI16 qword load/store family fully mapped (4 opcodes, 49 v4 instances total: LQR 30 + STQR 12 + LQA 5 + STQA 2 = 42 already covered + 7 gap). **R5.10o = implement STQA+LQA bundle** (mirror pair, same encoding shape, ~12 lines decoder+interpreter, 2-arm 9-bit dispatch addition, no JIT change). The pre-existing comment at [`rust/rpcs3-spu-interpreter/src/lib.rs:10`](../rust/rpcs3-spu-interpreter/src/lib.rs#L10) explicitly noted absolute LQA/STQA was deferred — R5.10o resolves that TODO. |
| 2026-04-29 (R5.10m)     | 0x734 | 0x20FFFA09 | **STQA** (Store Quadword Absolute, RI16-form, top-9=`0x041`, magn=2 → 11-bit slots `0x104..0x107`; sibling of LQA `0x061`) | ROTQMBYI implemented (decoder primary `0x1FD` added to AluImm7 set; interpreter arm with byte-shift-right-zero-fill semantics). **SHLQBYI/SHLQBII labeling bug FIXED in 4 places**: (1) decoder routes both 0x1FB and 0x1FF correctly without changing variant tag (both stay `AluImm7`; backend discriminates), (2) interpreter at 0x1FB rewritten to bit-shift (SHLQBII per RPCS3), (3) interpreter at 0x1FF NEW byte-shift arm (the real SHLQBYI), (4) encode helpers fixed: `encode::shlqbyi` repacks at 0x1FF, NEW `encode::shlqbii` at 0x1FB, NEW `encode::rotqmbyi` at 0x1FD. 2 decoder + 8 interpreter + 2 JIT differential tests added (incl. anti-regression locking primary `0x1FF` for SHLQBYI and the 0x1FB-vs-0x1FF distinct-result property). v4 advances 2 instructions past ROTQMBYI; **the latent SHLQBYI gap (9 v4 sites) was successfully prevented from surfacing as the next blocker** — execution flowed past the ROTQMBYI region into a different opcode family (STQA). New blocker `STQA` is RI16 absolute-address store; sibling pair of LQA. Likely Class B. R5.10n diagnose target. |

R5.10a opcode coverage diagnosis decoded the R5.9e.5 v4 divergence:

- `target_spu=256, event_index=0, pc=0x850, inst=0x33FF2E08` (NOT `0x33FFE748` — my earlier R5.9e.5/.6 summaries had a decimal→hex misconversion; the authoritative hex is `0x33FF2E08` = 872,361,480 = the value the diagnostic actually prints).
- Decoded mnemonic: **LQR** (Load Quadword Relative). Per RPCS3 C++ `SPUInterpreter.cpp:1690`: `spu.gpr[op.rt] = spu._ref<v128>(spu_ls_target(spu.pc, op.i16));`. Pure load from LS to GPR; no channels, no DMA, no FP, no atomics, no branches. Deterministic.
- 30 LQR instances exist in the v4 `.spuimg` (out of 391 non-zero instructions = 7.7% of code). The SPU hits the FIRST one at `pc=0x850` (second executed instruction after `pc=0x848` `ila` and `pc=0x84C` `hbrr`-as-NOP).
- Classification: **B** — opcode is simple but BOTH the Rust decoder (`rpcs3-spu-decoder/src/lib.rs::classify` returns `Unclassified`; `p9=0x67` is not in the dispatch table) AND the Rust interpreter (`rpcs3-spu-interpreter/src/lib.rs` has no `LQR`/`0x67`/`0x19F` arm) need it added. JIT can fall back to the interpreter via R5 partial fallback once interpreter coverage lands.

This **contradicts the earlier prediction** in this section that v4 would fail with `UnsupportedDmaInTrace` (R5.9e.2 § D.1). The actual failure mode under R5.9e.5 is `SpuExecError { Unimplemented opcode }` because the SPU stack runs out of ISA coverage before any DMA is issued. R5.10b (implement LQR) is the natural unblocker; after LQR lands, the SPU will advance further into spurs_test code and likely surface the next missing opcode (per the disasm window, `pc=0x854` = `CDD` and `pc=0x858` = `CWD` are also currently absent — but these are downstream of LQR, not concurrent blockers).

Trace v4 stays diagnostic-only either way. Even with full ISA coverage, the homebrew is multi-SPU SPURS workers using DMA for cross-SPU coordination; the per-SPU sequential replay model (R5.9e.5) doesn't simulate cross-SPU shared state, so v4 would still surface a different divergence further along execution. **R5.9e.7's first replay-validated fixture remains gated on a license-clean single-SPU homebrew — same conclusion as before, but the ISA-coverage finding is a smaller, sharper next step than DMA emulation.**

---



**Empirical finding (post-R5.9e.2, 2026-04-28):** the v3 trace from spurs_test passes the R5.9e.2 parser cleanly. Two reasons:
1. The R5.9c writer doesn't yet emit `spu_image` events — `spu_image` is NOT mandatory at parse time (the schema permits legacy traces without images), so no `MissingImageForSpu` parse-time error fires.
2. The R5.9c writer instruments `set_ch_value` only for `SPU_WrOutMbox` (channel 28), NOT for `MFC_Cmd` (channel 21). DMA dispatches happen via `MFC_Cmd` writes, but those writes are invisible to the current trace because the hooks aren't there. The R5.9e.2 DMA gate fires on captured `spu_wrch` to channel 21 — and the v3 trace has zero such events.

So under R5.9e.5 (replay engine), the v3 trace reaches the replay stage and fails with `MissingImageForSpu { target_spu }` for each of the 6 SPUs (no `spu_image` events at all). After R5.9e.3 ships writer-side `spu_image` emission and the v3 trace is re-captured, the replay would advance further. Whether DMA actually surfaces depends on whether a future writer iteration hooks `MFC_Cmd` writes — likely yes, at which point the v3 trace's replay would fail at the first DMA write with `UnsupportedDmaInTrace`.

This is the documented case for "real-trace v3 stays diagnostic". `tests/real_trace_diagnostic.rs` will evolve in R5.9e.5 to also test that the replay engine produces the appropriate "won't replay this trace" error (whichever of `MissingImageForSpu` / `UnsupportedDmaInTrace` is the most-specific available failure mode at the time).

The R5.9e.7 commit-fixture milestone DOES NOT use spurs_test. It uses a dedicated single-SPU homebrew (R5.9e.7 sub-deliverable: identify or author one).

### D.5 What R5.9e MUST reject

A trace is "R5.9e-replay-valid" if and only if every condition holds:
1. Every `target_spu` referenced anywhere in the trace has a `spu_image` event.
2. Every `spu_image`'s side-file is present and hash-matches.
3. No SMC indicators (per A.5).
4. No DMA `spu_wrch` to `MFC_Cmd` (per D.1).
5. No events with kinds outside the documented set.

Any violation: replay engine rejects with the matching error variant. Replay does NOT proceed partially.

---

## E. Test plan

### E.1 Synthetic SPU image fixture (R5.9e.4 prerequisite)

A minimal hand-crafted SPU program (10–20 instructions) loaded into LS at offset 0, executes a deterministic computation, stops at `stop 0xD5`. Used to exercise:
- `SpuProgram` builder from a captured-style image (via a hand-rolled `spu_image` event in a synthetic JSONL).
- Per-SPU replay with one SPU group containing one `spu_image` + a few `spu_*` events + `final_state`.

This fixture stays SYNTHETIC and lives alongside `R5_6_REFERENCE_JSONL` in `trace_fmt.rs` as a `pub const R5_9E_SYNTHETIC_JSONL_WITH_IMAGE`. Hand-derived; explicit.

### E.2 First REAL one-SPU homebrew capture (R5.9e.7 deliverable)

**This is the load-bearing fixture.** A single homebrew that:
- Authors / public-domain (license-clean for redistribution).
- Creates exactly ONE SPU thread.
- Does mailbox handshake (PPU push, SPU rdch, SPU compute, SPU wrch, PPU pop).
- Stops with a known stop_code.
- Does NO DMA.

Captured under R5.9c rpcs3.exe + R5.9e.3-extended writer that emits `spu_image` events. The `.jsonl` + `.images/<sha>.spuimg` pair becomes `behavior-freeze/fixtures/spu/traces/<homebrew>.jsonl` + sibling directory. Companion `.notes.md` documents capture command + RPCS3 commit + scaffolding/runtime hooks sha256s + replay reports for both Interpreter and Recompiler executors.

Authoring or sourcing this homebrew is a separate task — the existing local homebrew survey (per PROJECT_STATUS.md § "Single-SPU homebrew search") found nothing usable. R5.9e.7 may need a Cell SDK install or a new homebrew authoring iteration.

### E.3 Multi-SPU parse/transform diagnostic (extension of R5.9d)

`tests/real_trace_diagnostic.rs` gains a fourth `#[ignore]`d test:
- `diagnostic_real_trace_v3_replay_rejects_dma` — feeds the v3 trace through R5.9e.5's replay engine, expects `UnsupportedDmaInTrace { target_spu: …, event_index: … }` on the first DMA `wrch`. Documents the v3 trace's "stuck at parse+transform; replay needs a different homebrew" status.

### E.4 Replay × Interpreter (R5.9e.5)

New lib tests in `rpcs3-spu-differential`:
- `replay_per_spu_synthetic_one_spu` — feeds the synthetic fixture (E.1) through `replay_per_spu_traces<InterpreterExecutor>`, asserts one report with `Finished{0xD5}` and expected GPRs.
- `replay_per_spu_synthetic_two_spu` — extension of E.1 with two SPUs running the SAME image (dedup case). Two reports, both `Finished`.

### E.5 Replay × Recompiler (R5.9e.6)

Mirror of E.4 against `RecompilerExecutor`. Differential goal: same report shape, byte-exact final state. New lib tests in `rpcs3-spu-recompiler`:
- `replay_per_spu_synthetic_one_spu_jit` — same fixture, JIT backend.
- `replay_per_spu_synthetic_two_spu_jit` — same.

If Interpreter and Recompiler agree on E.4 + E.5, that's the strongest correctness signal R5.9e can provide.

### E.6 #[ignore] policy

- Synthetic tests (E.4, E.5): NOT ignored. Run on every `cargo test --workspace --lib`.
- Real-trace replay diagnostics (E.3, future homebrew capture diagnostics): `#[ignore]`d until R5.9e.7's first-fixture commit makes them tractable for CI.
- After R5.9e.7: the first-fixture replay tests ARE active. Real-trace v3 replay diagnostic stays `#[ignore]`d (it's the documented unsupported-DMA case).

---

## F. Migration

### F.1 Keep `R5_6_REFERENCE_JSONL` passing

The synthetic round-trip fixture has NO `spu_image` event. Per the A.4 / B.6 rule, the parser MUST allow traces without `spu_image` events — only the BUILDER for `SpuProgram` cares about images, and the existing `R5_6_REFERENCE_JSONL` test uses the hand-derived `mailbox_command_protocol_program()` directly, not a builder.

Concretely:
- `parse_jsonl_trace`: unchanged. `spu_image` is recognized but NOT required.
- `captured_events_to_traces_per_spu`: unchanged. Per-SPU groups carry image refs as a side-channel field, but the existing `Vec<TraceEvent>` output excludes `spu_image` (it's not a behavior-tracing event; it's metadata).
- `replay_per_spu_traces`: NEW function. Requires programs to be supplied externally. The legacy `replay_trace` continues to take `SpuProgram` directly and works for `R5_6_REFERENCE_JSONL` unchanged.

Round-trip equivalence test (`transform_round_trip_matches_canonical_r5_6_trace`) continues to pass byte-exact.

### F.2 spurs_test_v3 stays diagnostic local-only

- The trace v3 file at `tests/data/spurs_test_v3_real_trimmed.jsonl` lacks `spu_image` events (R5.9c writer doesn't emit them; R5.9e.3 will add them).
- After R5.9e.2 lands, the parser still accepts the v3 trace cleanly (no `spu_image` is allowed).
- After R5.9e.5 lands, replay on the v3 trace fails with `UnsupportedDmaInTrace` (D.1) — that's the documented failure path, captured by E.3.
- The trace remains untracked; `behavior-freeze/fixtures/spu/traces/` continues to hold only `README.md`.

### F.3 When can `REPLAY_VALIDATED_TRACE_EXISTS` flip to True?

Strict criteria, all required:
1. R5.9e.7's first single-SPU homebrew capture is committed to `behavior-freeze/fixtures/spu/traces/<homebrew>.jsonl` + sibling `.images/`.
2. `<homebrew>.notes.md` documents origin / license / capture command / RPCS3 commit / patches sha256.
3. Two integration tests pass against that fixture: replay × Interpreter AND replay × Recompiler.
4. Both report `Finished{stop_code}` matching the trace's `spu_stop`.
5. `final_state` GPR + channel assertions match for both backends.

`check_trace_fixtures.py` flips its `REPLAY_VALIDATED_TRACE_EXISTS` constant to `True` only when all five hold.

**STATUS (2026-04-29): all five criteria met by `single_spu_mailbox_v1`. Flag flipped to `True`.** See [`behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.notes.md`](../behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.notes.md) for full provenance and the engine-side fixes that landed alongside.

### F.4 Naming convention for the first committed fixture

Reserved naming: `single_spu_mailbox_v1.jsonl`. Rationale:
- `single_spu` indicates the topology (rules out spurs_test confusion).
- `mailbox` indicates which hooks are exercised (push_inmbox + rdch + wrch + pop_outmbox).
- `v1` reserves room for `v2` capturing additional features (signals, multi-mailbox, DMA when supported).

Sibling: `single_spu_mailbox_v1.notes.md`. Image side-file landed at the centralized layout per § B.2 / F.4 recommendation: `behavior-freeze/fixtures/spu/images/<sha256>.spuimg` (NOT the per-trace `<trace>.images/` alternate). The R5.9e.7 implementer chose this layout; the `.notes.md` records the decision and the chosen sha256 (`68cf203b…abac43`).

When R5.9e.7 commits this fixture, the existing v2/v3 diagnostic traces stay separate (under `tests/data/`) — the fixture dir holds only replay-validated assets.

---

## R5.9e subphases

| Subphase | Scope | Independently valuable | Depends on |
|---|---|---|---|
| **R5.9e.1** ✅ DONE 2026-04-28 | Schema design for `spu_image` event + `image_sha256` field. New section in [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md) — "R5.9e.1 — SPU image metadata + side-file layout (replay prerequisite)" — formalizing the wire format: 5 required fields (`target_spu`, `image_sha256`, `load_addr`, `size`, `entry_pc`); both per-trace (`<trace>.images/<sha>.spuimg`) and centralized (`behavior-freeze/fixtures/spu/images/<sha>.spuimg`) side-file layouts; 8 invariant rules (hash integrity, side-file required for replay only, multi-SPU dedup OK, no inline bytes, raw byte content, license rules, ordering, no silent fallback); 7 unsupported-cases categories with explicit error variant names that R5.9e.2+ will implement (`UnsupportedDmaInTrace`, `UnsupportedSelfModifyingCode`, `DuplicateSpuImage`, `MissingImageForSpu`, `BadImageSize`, etc.); cross-trace consequences for `R5_6_REFERENCE_JSONL` (no image; parse-only path), `spurs_test_v3` (will get `MissingImageForSpu` once R5.9e.5 lands), and the eventual first replay-validated fixture (`single_spu_mailbox_v1`). Field-level definitions table extended with rows for `image_sha256`, `load_addr`, `size`, `entry_pc`. **No code, no patch, no test, no fixture changed.** | yes — locks the wire format before R5.9e.2/.3 implement parse/writer | nothing |
| **R5.9e.2** ✅ DONE 2026-04-28 | Parser support added in `rust/rpcs3-spu-differential/src/trace_fmt.rs`: `SpuImageEvent` struct (mandatory `target_spu`, `image_sha256`, `load_addr`, `size`, `entry_pc`); `CapturedEvent::SpuImage` variant + accessor extensions (`seq`/`side`/`target_spu`/`required_side`/`kind_label`/new `is_spu_executed`); 7 new `TraceParseError` variants (`DuplicateSpuImage`, `ImageEventOutOfOrder`, `BadImageHash`, `BadImageSize`, `BadImageLoadAddr`, `BadImageEntryPc`, `UnsupportedDmaInTrace`); `validate_spu_image_event` helper; per-SPU walk extended with image-uniqueness + image-ordering tracking (PPU events for the same target do NOT count as "executed", so PPU can act on a not-yet-running SPU); DMA detection on `spu_wrch` to channel 21 (`MFC_Cmd`); transformer's per-SPU state machine adds `SpuImage(_) => {}` arm so images are metadata-only and don't perturb state. Re-export `SpuImageEvent` in `lib.rs`. **`spu_image` is NOT mandatory** — `R5_6_REFERENCE_JSONL` continues parsing unchanged; the v3 spurs_test trace continues passing parse+transform (the R5.9d milestone is preserved). 10 new contract tests cover positive parse, hash/size/load_addr rejection, image out-of-order, duplicate image, DMA detection, the deliberate non-detection of single-channel SMC (deferred to R5.9f when writer surfaces side-channel events), legacy reference parse, and transformer image-passthrough. **SMC detection: not implemented in R5.9e.2** — single-channel signature isn't reliably distinguishable from generic DMA without observing the full `MFC_LSA`/`MFC_Size`/`MFC_Cmd` register sequence; SMC is a strict subset of DMA, so the DMA gate already rejects SMC-bearing traces. **No C++, no patch, no fixture, no `.spuimg` changes.** Test counts: differential lib 79 passed (was 69; +10), workspace 5484 passed (was 5474; +10), diagnostic suite 3 ignored unchanged, gates green (sha256s of both patches preserved at R5.9c values). Diagnostic `--ignored` continues to pass on v3 real trace because the current writer (R5.9c) does NOT emit `spu_image` events AND does NOT instrument `MFC_Cmd` — so neither gate fires on the v3 trace. | yes (parser-side gating for replay-incompatible traces; first metadata-only event variant landed without breaking R5.9d) | R5.9e.1 |
| **R5.9e.3 writer-emit** ✅ DONE 2026-04-28 (re-capture BLOCKED) | Writer extension: `record_spu_image(target_spu, ls_bytes, size, load_addr, entry_pc)` added to `SPUTraceJsonl.{h,cpp}`. Snapshot LS bytes (full 256 KB), compute SHA-256 via `mbedtls_sha256_ret` (already in emucore via `Crypto/sha256.cpp`), write content-addressed `.spuimg` side-file at `<trace_path>.images/<sha>.spuimg` (skipped if same-name same-size file exists), emit JSONL `spu_image` event. Per-target_spu dedup via `std::unordered_set<u32> m_emitted_images` (guarded by `m_write_mutex`) — re-entered `cpu_task` is a no-op. Lock contract preserved (`m_write_mutex` taken before `next_seq()`). Runtime hook in `cpu_task`, AFTER `pc &= 0x3fffc;` so entry_pc is the clean instruction address. Both patches re-touched: scaffolding sha256 `d4873c358d8ce8be8a6e9976a49ec0516a4abab2522546dffcea8497509a09ac` (was `2baebca5…91149`; +7,887 bytes — new method body + side-file helper + sha helper); runtime hooks sha256 `8f253d7d207793266eb3a81e809c73731a8e565757a9d2c40fa944a88266663a` (was `3ee7a861…2bed39`; +new hunk + 6 hunk-header offset updates). New gate invariant 8 (`check_spu_image_api_wiring`) added: scaffolding MUST declare + define `record_spu_image`, runtime hooks MUST call it. `git apply --check` / forward / reverse round-trip validated against post-scaffolding + master baselines. Build via `R:\.claude\build_full.bat`: `rpcs3.exe` produced (63,764,992 bytes at 2026-04-28 19:37; +7,168 bytes vs R5.9c's 63,757,824). emucore.vcxproj compiled clean with same 2 pre-existing warnings (`getenv` C4996 + `TraceFinalGuard` C4530). 9 errors all in `rpcs3_test.vcxproj` for missing gtest NuGet — pre-existing, non-blocking. **Re-capture BLOCKED**: this session attempted both `--version` smoke and `rpcs3.exe --headless ... spurs_test.self` re-capture; the permission hook denied both, citing the prior R5.9c spurs_test denial as still in effect. The writer code/patches/build are correct; trace v4 must be produced manually by the user (same workflow that yielded v3 in R5.9d). Until the user produces the trace, `tests/data/spurs_test_v4_real.jsonl` + `tests/data/spurs_test_v4_real.images/<sha>.spuimg` don't exist locally; R5.9e.4 (builder) and the diagnostic flip can't proceed yet. | yes (writer side is real-captures-compatible) | R5.9e.1, R5.9e.2 (parser must accept the new event before traces appear in the wild) |
| **R5.9e.4** ✅ DONE 2026-04-28 | `SpuProgram` builder landed in new module [`rust/rpcs3-spu-differential/src/spu_image_loader.rs`](../rust/rpcs3-spu-differential/src/spu_image_loader.rs). Public API: `build_spu_program_from_captured_image(image_path, image: &SpuImageEvent, max_steps: u64) -> Result<SpuProgram, SpuProgramBuildError>`. Reads `.spuimg` bytes, hash-validates against `image.image_sha256` via `sha2` crate (already in workspace lockfile; added as direct dep), populates `SpuProgram` with one segment at `image.load_addr` carrying the bytes plus `entry_pc` and `max_steps`. New error enum `SpuProgramBuildError` with 8 structured variants (`ImageFileMissing`, `ImageIo`, `ImageHashMismatch`, `ImageSizeMismatch`, `ImageTooLarge`, `BadImageAlignment`, `BadImageBounds`, `BadEntryPc`) + Display/Error impls. Validation order is cheapest-first (metadata → file existence → length → SHA → build). 8 unit tests cover positive build, missing file, hash mismatch, size mismatch, bad load_addr alignment, bad bounds (`load_addr + size > 256 KiB`), bad entry_pc (unaligned + out-of-range), and the load_addr placement guarantee. 1 new `#[ignore]`d v4 diagnostic test (`diagnostic_real_trace_v4_builds_spu_program_from_image`) builds 6 `SpuProgram` instances from the real R5.9e.3-captured trace's 6 `spu_image` events; all 6 resolve to the SAME `.spuimg` side-file (content-addressed dedup; 6 SPURS workers load the same `.spucore.elf`); each produces a `SpuProgram` whose `validate()` passes. New dev-dep `tempfile` for hermetic test scratch dirs. **No C++, no patches, no parser/transformer changes, no replay touched.** Test counts: differential lib 87 (+8), workspace 5492 (+8), diagnostic suite 7 ignored (+1 new v4 builder test); all 7 pass under `--ignored`. | yes (synthetic + real-trace builder validated; replay can now land independently) | R5.9e.2 |
| **R5.9e.5** ✅ DONE 2026-04-29 | Per-SPU sequential replay landed in new module [`rust/rpcs3-spu-differential/src/per_spu_replay.rs`](../rust/rpcs3-spu-differential/src/per_spu_replay.rs). Public API: (a) `replay_per_spu_traces_with<E: SpuExecutor, F: FnMut(u32) -> E>(per_spu, programs, make_executor)` — caller-provided factory closure for executor-per-SPU; (b) `replay_per_spu_traces<E: SpuExecutor + Default>(per_spu, programs)` — convenience wrapper that uses `E::default()` per SPU. Both return `BTreeMap<u32, TraceReplayReport>` keyed on `target_spu`. New error enum `MultiSpuReplayError` with 3 variants — `MissingProgram { target_spu }`, `ExtraProgram { target_spu }`, `ReplayFailed { target_spu, source: TraceReplayError }` — all carrying `target_spu` so per-SPU diagnostics are precise. Pre-flight bijection check (every trace SPU has program; every program SPU has trace) runs BEFORE any replay; first SPU's executor is built and torn down before next SPU starts (no shared state). 6 unit tests cover positive single-SPU, positive two-SPU (both using mailbox_command_protocol fixture), MissingProgram pre-flight, ExtraProgram pre-flight, ReplayFailed surfaces target_spu (using stop-only program with mismatched expected stop_code → UnexpectedSpuState), and factory-variant invokes closure once per SPU in sorted order. 1 new `#[ignore]`d v4 diagnostic (`diagnostic_real_trace_v4_per_spu_replay_attempt`) wires the FULL pipeline end-to-end (parser → per-SPU transformer → builder → orchestrator → replay_trace<InterpreterExecutor>) — surfaces a clean **SPU interpreter divergence** at `target_spu=256, event_index=0, kind=SpuExecError { message: "Unimplemented { inst: 872361480, pc: 2128, reason: \"opcode not in iteration-1 subset\" }" }`. The divergence is at the SPU INSTRUCTION-EXECUTION layer (instruction 0x33FFE748 not yet implemented in the iteration-1 interpreter subset), NOT at the trace/replay protocol layer — the orchestrator wiring is correct; what's missing is broader SPU opcode coverage (separate scope from R5.9e). The diagnostic test prints the divergence and passes. **Lockstep multi-SPU NOT implemented** — deferred to R5.9f if motivated by a real workload that the per-SPU sequential model can't capture. **No C++, no patches, no parser/transformer/builder semantics changed.** Test counts: differential lib 93 (+6), workspace 5498 (+6), diagnostic suite 8 ignored (+1 new); all 8 pass under `--ignored`. | yes (synthetic replay validated end-to-end; real-trace v4 replay surfaces interpreter coverage as the next blocker) | R5.9e.4 |
| **R5.9e.6** ✅ DONE 2026-04-29 | Recompiler replay over the per-SPU sequential orchestrator added as 4 new tests in [`rust/rpcs3-spu-recompiler/src/lib.rs`](../rust/rpcs3-spu-recompiler/src/lib.rs): `r5_9e_6_per_spu_replay_recompiler_single_spu_mailbox_protocol` (single SPU at target_spu=42, JIT backend, 16 records + Finished{0xD5}), `r5_9e_6_per_spu_replay_recompiler_two_spus_mailbox_protocol` (two SPUs at {7,42}, factory-variant tracks per-SPU invocation order), `r5_9e_6_interpreter_and_recompiler_reports_match` (load-bearing differential — same per-SPU input through both backends, asserts `final_event_kind` / `records.len()` / `total_steps` match AND `diff_snapshots(...).is_identical()` for `final_snapshot` byte-exact agreement), and `r5_9e_6_recompiler_missing_program_error_preserves_target_spu` (`MissingProgram` pre-flight gate fires on JIT backend too, confirming bijection check is backend-agnostic). **Real-trace v4 NOT re-exercised here**: the `Unimplemented opcode 0x33FFE748 @ pc=0x850` divergence already surfaced under R5.9e.5 is in the SPU ISA layer (common to both Interpreter and Recompiler — JIT falls back to Interpreter on unimplemented opcodes via R5 partial fallback), so re-running it under JIT would just emit the same diagnostic. **No changes to `replay_per_spu_traces_with` / `replay_per_spu_traces`** — orchestrator delegates correctly to whatever `SpuExecutor` the factory produces. **No C++, no patches, no parser/transformer/builder semantics changed.** Test counts: recompiler release **139 passed** (was 135; +4), workspace **5502** (was 5498; +4); both `behavior-freeze` gates exit 0; sha256s preserved. | yes (full Interpreter≡Recompiler differential coverage on synthetic; the orchestrator works on any `SpuExecutor`) | R5.9e.5 |
| **R5.9e.7** ✅ DONE 2026-04-29 | First replay-validated SPU trace fixture LANDED. User authorized path **P2** (ps3toolchain build-from-source) executed via Docker `debian:bookworm-slim` container `ps3-build`: built binutils 2.43.1 + gcc 14.2.0 (PPU + SPU) + newlib 4.4.0 + PSL1GHT (~1.3 GB toolchain). Authored CC0 PSL1GHT homebrew at [`behavior-freeze/fixtures/spu/sources/single_spu_mailbox_v1/`](../behavior-freeze/fixtures/spu/sources/single_spu_mailbox_v1/) (PPU `main.c` loader + SPU `spu_mailbox.c` minimal kernel — 6 SPU instructions, `-nostartfiles -nostdlib -Wl,--entry,main` to skip crt0/newlib that pulls in ROTQBY etc. outside the iteration-1 SPU subset; inlined exit `spu_writech(SPU_WrOutMbox, reply); spu_stop(0x101)` to avoid libsputhread). Captured via `enable_autoexit_and_capture.cmd` which auto-patches `R:\bin\config\config.yml` to enable "Exit RPCS3 when process finishes: true" (without it the trace writer's destructor never runs and the JSONL stays 0 bytes — silent failure mode). Captured artifacts staged at [`behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.jsonl`](../behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.jsonl) (5 events, 1.1 KB) + [`behavior-freeze/fixtures/spu/images/68cf203b…abac43.spuimg`](../behavior-freeze/fixtures/spu/images/) (262 KB centralized layout per § F.4). Three engine-side fixes co-landed to bridge captured-vs-replay semantics gaps (all general — not single-fixture hacks): (1) **transformer initial-state inference** (`infer_initial_state` in `trace_fmt.rs`) for race-free single-round captures where PPU writes mailbox before SPU runs and RPCS3 omits the implicit initial park; (2) **lv2 stop-0x101/0x102 OUT_MBOX-drain modeling** (synthetic `PpuPopOutMbox` injection in `transform_single_spu_subset`) reflecting the kernel's group-exit-status semantics that the captured `final_state` has already absorbed; (3) **`SpuProgram.initial_gpr_overrides`** field + `with_initial_gpr` builder + override-application in both `InterpreterExecutor::execute` and `RecompilerExecutor::try_jit_run`; `build_spu_program_from_captured_image` sets gpr[1] preferred-slot = 0x3FFF0 to match `spu_thread::cpu_init` ([`SPUThread.cpp:1342`](../rpcs3/Emu/Cell/SPUThread.cpp#L1342)). Acceptance gate test [`rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_v1_replay.rs`](../rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_v1_replay.rs) drives the FULL pipeline (parse → transform → build → replay × Interpreter + replay × Recompiler) and asserts `diff_snapshots(...).is_identical()` — **PASSES**. `total_steps` legitimately differs (interp=5, jit=9) because the JIT counts dispatcher iterations + JIT prefix steps while the interpreter counts retired instructions; canonical `diff_snapshots` excludes step counts (PC, GPRs, LS, channels, park_state are the byte-identical contract). `behavior-freeze/harness/check_trace_fixtures.py` flag `REPLAY_VALIDATED_TRACE_EXISTS` flipped `False` → `True` (the gate-flip moment R5.9e.7 was scoped to deliver). Workspace gates green, no regressions. See [`single_spu_mailbox_v1.notes.md`](../behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.notes.md) for full provenance + decoded trace. | yes (first replay-validated fixture; the project's load-bearing oracle finally exists) | R5.9e.5 + R5.9e.6 (DONE) + license-clean single-SPU homebrew (authored CC0 in this iteration via Docker-built ps3toolchain). |

R5.9e.1 is the smallest unit and the highest-value first step: it locks the wire format without committing to writer or replay implementation. The rest of the chain has clear dependencies that make sequential implementation feasible.

---

## Risks

1. **DMA dependency in spurs_test_v3** — likely makes the existing real-trace v3 unreplayable under R5.9e. **Mitigation:** explicitly target single-SPU homebrew first; v3 stays diagnostic per F.2. R5.9e is not "make the existing trace replay" — it's "make replay possible at all".
2. **Self-modifying code** — breaks the single-image-per-SPU assumption. **Mitigation:** R5.9e.2 detects + rejects (A.5). Defer SMC support to a future R5.9f or beyond.
3. **Per-SPU sequential losing cross-SPU mailbox correlation** — rare in practice (most homebrew has the PPU as the mailbox arbiter, which records the push/pop in the trace), but not impossible. **Mitigation:** ship per-SPU sequential first; build lockstep only if a real workload exposes a divergence. Document the failure mode in E.3 if it surfaces.
4. **Side-file dependency in CI** — fixture commits include `.spuimg` files. **Mitigation:** content-addressed layout under `behavior-freeze/fixtures/spu/images/`; gate validates each `.jsonl`'s image refs resolve. Repository size grows; acceptable for the kinds of small homebrew used here.
5. **License creep into images** — the same redistributable check applies to `.spuimg` as to `.jsonl`. **Mitigation:** gate `.notes.md` for license markers. Reject commercial-game-derived images at the gate level.
6. **Single-SPU homebrew sourcing** — the local survey found no usable single-SPU homebrew. R5.9e.7 may stall pending Cell SDK install or new homebrew authoring. **Mitigation:** R5.9e.5 + R5.9e.6 are independently valuable on the synthetic fixture; the fixture commit (R5.9e.7) is the only step that REQUIRES a real homebrew.
7. **Patch re-touch coupling in R5.9e.3** — both patches change again, sha256s shift. **Mitigation:** the existing `regen_scaffolding_patch.py` and `apply_r59c_to_R_drive.py` helper scripts already handle the workflow; R5.9e.3 extends them to add a `record_spu_image` arg-threading edit. `check_patch_separation.py` invariant 7 (R5.9c target_spu emit) continues to apply unchanged; a new invariant 8 may be added requiring `record_spu_image` to take the same `target_spu` argument.

---

## What stays blocked

- Replay-validated fixture: until R5.9e.5 + R5.9e.6 + a license-clean single-SPU homebrew all exist, `REPLAY_VALIDATED_TRACE_EXISTS = False`.
- Real-trace v3 replay: blocked by DMA dependency in spurs_test (D.4). Stays diagnostic local-only.
- Cell SDK / PS3 toolchain: out of scope for autonomous installation — same pre-existing blocker that prevents a single-SPU homebrew from being authored from this session.

---

## Cross-cutting checklist (when implementation starts)

This is a one-page checklist for the implementer of R5.9e, NOT a current-iteration action.

- [ ] R5.9e.1: schema doc updated in `SPU_TRACE_CAPTURE.md` to add `spu_image` event + `image_sha256` field on every SPU-side timeline. Side-file layout documented.
- [ ] R5.9e.2: parser updated; new error variants (`UnsupportedDmaInTrace`, `UnsupportedSelfModifyingCode`); tests for positive parse, DMA rejection, SMC rejection, R5_6_REFERENCE_JSONL still passes.
- [ ] R5.9e.3: `SPUTraceJsonl.{h,cpp}` extended with `record_spu_image(target_spu, image_sha256, load_addr, size, entry_pc)`. Side-file write helper. Runtime hook at SPU thread creation. Both patches regenerated; sha256 shift documented; `check_patch_separation.py` invariant 8 added if needed.
- [ ] R5.9e.4: `SpuProgram` builder; tests for hash mismatch, missing file, R5.9e.5 prereq.
- [ ] R5.9e.5: `replay_per_spu_traces` + `MissingProgramForSpu`; synthetic fixture (E.1) replay × Interpreter passes; real-trace v3 diagnostic flipped to "expect UnsupportedDmaInTrace".
- [ ] R5.9e.6: synthetic fixture replay × Recompiler passes; differential equivalence asserted.
- [ ] R5.9e.7: first single-SPU homebrew captured + committed to `behavior-freeze/fixtures/spu/traces/single_spu_mailbox_v1.jsonl` + sibling `.images/`; `.notes.md` populated; `REPLAY_VALIDATED_TRACE_EXISTS` flipped to True; integration tests for replay × Interpreter AND replay × Recompiler land.
