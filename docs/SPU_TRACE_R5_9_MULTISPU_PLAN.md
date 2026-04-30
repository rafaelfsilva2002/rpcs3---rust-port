# R5.9 Multi-SPU Trace Schema — Design + R5.9a/R5.9b/R5.9c/R5.9d Landed

**Status:** R5.9a (parser-only) **DONE — 2026-04-28**. R5.9b (transformer per-SPU API) **DONE — 2026-04-28**. R5.9c writer-emit + re-capture **DONE — 2026-04-28** (C++ source + patches + build + real `spurs_test.self` trace v3 captured under R5.9c writer; user provided the trace after the in-session permission hook denied autonomous capture). R5.9d (diagnostic flip — parse + per-SPU transform validated against the real v3 trace; tests stay `#[ignore]`d because the trace is local-only and replay is not yet exercised) **DONE — 2026-04-28**. R5.9e (multi-SPU replay + SPU image capture) remains design-only / not started.

This doc enumerates the decisions an implementer will need to make in R5.9 to extend the single-SPU trace schema to multi-SPU. It is the deliverable of a "plan before code" iteration triggered by the first real trace from `spurs_test.self`, which produced 6 SPU threads and was correctly rejected by the parser as schema-incompatible. R5.9a (parser) and R5.9b (transformer per-SPU API + single-SPU API rejecting multi-SPU traces) are both Rust-side and have now landed; the R5.9c writer, R5.9d diagnostic flip, and R5.9e replay engine remain.

**Cross-references:**
- Wire format (single-SPU): [`SPU_TRACE_CAPTURE.md`](./SPU_TRACE_CAPTURE.md).
- Writer impl: [`../rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}`](../rpcs3/Emu/Cell/SPUTraceJsonl.h).
- Parser/transformer/replay: [`../rust/rpcs3-spu-differential/src/trace_fmt.rs`](../rust/rpcs3-spu-differential/src/trace_fmt.rs).
- Status: [`PROJECT_STATUS.md`](./PROJECT_STATUS.md) §§ "First real trace captured; parser reached multi-SPU schema gap (2026-04-28)" and "R5.8 hardening contracts added".

---

## A. Schema decisions

### A.1 Field name: `target_spu`

**Recommended: keep `target_spu`** (the name PPU-side events already use), and require it on SPU-side events too. NOT `spu_id`, NOT `target_spu_id`. Two reasons:

1. PPU-side events already use `target_spu` (see `SPU_TRACE_CAPTURE.md` § "PPU-side events"). Renaming on the SPU side would create an inconsistency; introducing a third name is worse.
2. The semantics differ slightly between PPU and SPU events but the field is the same physical id. A PPU event's `target_spu` is *the SPU the PPU is acting on*; an SPU event's `target_spu` is *the SPU that emitted the event*. The single name is fine because resolution is unambiguous given `side`.

### A.2 Required-on-which events

| Event kind | side | Required | Semantics |
|---|---|---|---|
| `spu_rdch` | spu | **yes** | id of the SPU that executed `rdch` |
| `spu_wrch` | spu | **yes** | id of the SPU that executed `wrch` |
| `spu_rchcnt` | spu | **yes** | id of the SPU that executed `rchcnt` |
| `spu_park` | spu | **yes** | id of the SPU that parked |
| `spu_wake` | spu | **yes** | id of the SPU that woke |
| `spu_stop` | spu | **yes** | id of the SPU that stopped |
| `final_state` | spu | **yes** | id of the SPU whose terminal state is captured |
| `ppu_push_inmbox` | ppu | already required | id of the SPU receiving the push |
| `ppu_pop_outmbox` | ppu | already required | id of the SPU whose outmbox is drained |
| `ppu_signal` | ppu | already required | id of the SPU receiving the signal |

### A.3 Globally-scoped events

There are no truly "global" events in the current schema — every event in the table above is per-SPU. If R5.9 adds any (e.g., a global `system_init`, `system_clock_tick`), they would either omit `target_spu` (and the parser would treat them as side-channel context, not as part of any SPU's per-event timeline) or carry a sentinel `target_spu: null`. **Recommendation:** do NOT introduce global events in R5.9. Keep every event tied to one SPU. Defer global events to R5.10+ if motivated by real captures.

### A.4 Backward compatibility with single-SPU traces

Single-SPU traces emitted under R5.7/R5.8 schema do not carry `target_spu` on SPU-side events. To accept them under R5.9 parser without forcing a re-capture, the parser MUST treat absence of `target_spu` on an SPU-side event as equivalent to `target_spu: 0`. This collapses the namespace cleanly and preserves the synthetic `R5_6_REFERENCE_JSONL` round-trip test that ships in the parser.

PPU-side events already require `target_spu`; no compatibility shim needed there.

The `seq` field stays globally-monotonic — adding `target_spu` does NOT change that. Per-SPU monotonicity is a derived property: events with the same `target_spu` must appear in `seq`-order, which falls out of global monotonicity automatically.

---

## B. Writer C++ — how to obtain `target_spu`

### B.1 In `SPUThread.cpp`

Every `record_*` SPU-side hook lives inside a `spu_thread::method(...)` member. The thread carries `lv2_id` (`u32`, used by syscalls; line 775 of `SPUThread.h`) and `index` (raw-SPU index for raw-SPU mode). Both are stable per-thread.

**Recommendation:** use `this->lv2_id` for threaded SPU and `this->lv2_id` (also set for raw SPU per RPCS3's id allocation) for raw SPU. `lv2_id` is the syscall-visible id and matches the value already passed to PPU-side hooks (`record_ppu_push_inmbox(id, ...)` from `sys_spu_thread_write_spu_mb`). One id namespace = no per-event ambiguity.

If `lv2_id` is somehow not set on a particular SPU (defensive): emit `target_spu: 0` and log to stderr. Do not skip the event.

### B.2 In `lv2/sys_spu.cpp`

Already covered: existing hooks (`record_ppu_push_inmbox`, `record_ppu_signal`) take `id` (= `lv2_id` of the target SPU) as a parameter from the syscall args. No change needed.

### B.3 In `RawSPUThread.cpp`

Hook 6b (`ppu_pop_outmbox` at `SPU_Out_MBox_offs:`) is a method of `spu_thread` — `this->lv2_id` is in scope. Already used by the current implementation. No change needed at the access site.

### B.4 Side-effect protocol

`target_spu` is read from `this->lv2_id`, which is set at thread construction and never mutated. Reading it from inside `record_*` is safe (no atomic, no lock, no syscall). No new side-effect risks.

### B.5 If `target_spu` is unobservable

Defensive fallback: emit `target_spu: 0` AND log a one-time warning to stderr identifying the call site. Never drop the event — the trace's invariant is "every channel op the SPU made gets logged"; missing events would be silent corruption. The R5.9 parser must distinguish "id missing" from "id == 0" — to keep the JSON wire format simple, emit `target_spu: 0` and accept that defensive fallback may collide with a real SPU that does have id 0. The stderr warning is the operational lifeline.

---

## C. Parser Rust — what changes

### C.1 Global `seq` monotonicity

**Unchanged.** `seq` remains global, the parser keeps the existing strictly-increasing check across all events. Per-SPU monotonicity is implied.

### C.2 Per-SPU `final_state`

The current rule is "exactly one `final_state`, must be the last event in the trace". R5.9 changes this to "exactly one `final_state` per `target_spu`, and after a SPU's `final_state` is emitted, no further events are allowed for that `target_spu`".

Concrete state-machine for the parser:

```
HashSet<u32> finalized;                   // SPU ids that have already emitted final_state
for event in events {
    let tgt = event.target_spu();         // for PPU events too — they target a specific SPU
    if finalized.contains(&tgt) {
        return Err(EventAfterFinalState { target_spu: tgt, event_index, last_index });
    }
    if event.kind == FinalState {
        finalized.insert(tgt);
    }
}
// Optional terminal check: every SPU referenced anywhere in the trace must have eventually
// emitted final_state. This is a strong correctness check; recommend it for R5.9.
if !all_referenced_spus_finalized(&events, &finalized) {
    return Err(UnterminatedTraceForSpu { spu_ids_missing_final_state });
}
```

### C.3 Detect "events after final_state of same SPU"

Replace the existing `FinalStateNotTerminal` error with a per-SPU variant `EventAfterFinalState { target_spu, final_state_index, offending_event_index }`. The single-SPU case (where `target_spu == 0` everywhere) reduces back to the current behavior because there's only one SPU and `finalized` only contains 0 after its `final_state`.

### C.4 Reject mixed/invalid SPU ids

Define what "invalid" means:

- An event referencing a `target_spu` that the trace never declared (e.g., if R5.9 adds a `spu_introduce` event — recommended NOT to add in R5.9; let SPUs be implicitly declared by their first event).
- A PPU event with `target_spu` that no SPU-side event ever uses (likely benign, but the parser can choose to emit a non-fatal `Warning` rather than `Err`).

For R5.9 minimum: do NOT introduce a "declared SPU ids" set. Let SPU ids be implicit. The only hard error is "event after that SPU's final_state".

---

## D. Transformer

### D.1 Output shape

Three options, in increasing scope:

1. **Single `Vec<TraceEvent>` with implicit grouping.** Caller filters by `target_spu`. Smallest diff. Replay engine still consumes a single event sequence per SPU after filtering. Recommended for R5.9b.

2. **`MultiSpuTrace { per_spu: HashMap<u32, Vec<TraceEvent>> }`.** Caller iterates the map. Cleaner type-system signal but requires `replay_trace` to take a per-SPU subset.

3. **Global trace + grouping helpers.** Hybrid; complicates types without adding clarity.

**Recommendation: D.1.** It is the smallest change that preserves the existing single-SPU `replay_trace` contract: filter `events.iter().filter(|e| e.target_spu == 0).copied().collect()` and feed that to `replay_trace` unchanged. The R5.7/R5.8 single-SPU path becomes a degenerate case.

### D.2 Smallest preserved-behavior diff

Add `pub fn captured_events_to_traces_per_spu(events: &[CapturedEvent]) -> Result<HashMap<u32, Vec<TraceEvent>>, TraceTransformError>` ALONGSIDE the existing `captured_events_to_trace`. Existing callers stay on the single-SPU function. New callers get the multi-SPU view. Mark the single-SPU function `#[deprecated(note = "use captured_events_to_traces_per_spu")]` only after R5.9c lands; until then, both are first-class.

---

## E. Replay strategy

### E.1 Does the current `replay_trace` handle multi-SPU?

**No.** `replay_trace<E: SpuExecutor>(backend, program, events)` takes a single executor and a single `SpuProgram`. There is no concurrency primitive in the replay engine. It walks `events` in order, advancing the SPU state via the executor; multi-SPU interleaving would mismatch immediately at the first cross-SPU event.

### E.2 Minimum scope to add multi-SPU replay

Two viable shapes:

- **Per-SPU replay, sequentially.** For each SPU id in the trace, filter events to that SPU and replay. Loses cross-SPU mailbox correlation. Cheap: probably no engine change.
- **Multi-SPU lockstep replay.** Maintain N executors, advance each based on the next event for its `target_spu`. Cross-SPU events (PPU push to SPU A, SPU A's subsequent rdch) are coordinated via a per-SPU pending-mailbox queue. This is closer to real PPU↔SPU semantics. Costly: adds a `MultiSpuLockstepDriver` similar to the existing `SpuPpuLockstepDriver`.

**Recommendation:** start with per-SPU sequential replay (R5.9e). Lockstep replay deferred until a real workload exposes a divergence that only lockstep can catch.

### E.3 Validate parser/transformer first?

**Yes.** R5.9a (schema + parser) and R5.9b (transformer per-SPU) are independently valuable: the existing diagnostic test (`tests/real_trace_diagnostic.rs`) flips from failing to passing if and only if the parser accepts the spurs_test trace. That single flip is a strong milestone before any replay work.

### E.4 What's missing for real replay?

Two things, regardless of single-SPU or multi-SPU:

1. **SPU image capture.** `replay_trace` requires a `SpuProgram`. The current writer does not capture the SPU machine code. Without it, replay is impossible for any captured trace. This is orthogonal to the schema gap and applies equally to single-SPU and multi-SPU.
2. **Initial-state capture.** The current writer emits `final_state` but no `initial_state`. Replay starts from a default-zero SPU state, which would mismatch any homebrew that's loaded with non-zero initial GPRs. For first-pass replay this is acceptable (most homebrews start from zero).

### E.5 Is SPU image capture required for spurs_test specifically?

**Yes.** The PPU loaded `ent_spucore.elf`, `ipc_spucore.elf`, `ppm_spucore.elf`, `vsc_spucore.elf` (4 distinct SPU images per the RPCS3 log of the capture run). Without those bytes, replay cannot step the SPU through its instruction stream. SPU image capture is a R5.9e prerequisite for replaying spurs_test.

---

## F. Migration

### F.1 Keep `R5_6_REFERENCE_JSONL` passing

The synthetic round-trip fixture in `trace_fmt.rs` does not include `target_spu` on SPU-side events. Per A.4, the parser treats absence as `target_spu: 0`. The fixture is implicitly single-SPU. Round-trip equivalence holds without editing the fixture.

### F.2 Diagnostic `multi_spu_schema_gap` test

After R5.9a (parser accepts multi-SPU), the existing `tests/real_trace_diagnostic.rs` `#[ignore]`d tests would START passing IF we update them to reflect the new contract. Two paths:

1. **Flip them to active tests** (`#[ignore]` removed, name renamed from `diagnostic_multi_spu_schema_gap_*` to `r5_9_real_trace_spurs_test_*`) — ONLY after the trace also passes transformer (R5.9b). Until transformer is per-SPU-aware, parse may pass but transform may still fail differently.
2. **Keep `#[ignore]`d but update the assertions** to reflect what each subphase unlocks. Risk of stale test drift; not recommended.

**Recommendation:** approach 1, but only at the boundary between R5.9b and R5.9c.

### F.3 New negative tests for R5.9

Add to the parser's `tests` module (small inline JSONL):

- `parser_rejects_event_after_final_state_for_same_spu` — two `target_spu`s; one finalizes; an event for that SPU after final_state is rejected.
- `parser_accepts_interleaved_two_spu_events` — minimal positive test; two SPUs each emit their own complete sequence; parse succeeds.
- `parser_treats_missing_target_spu_as_zero` — single-SPU compat test; an SPU event without `target_spu` is parsed as `target_spu: 0`.
- `transformer_per_spu_groups_events_correctly` — two SPUs, transformer returns `HashMap` with both keys present.

### F.4 Existing tests that must change

- `parser_rejects_final_state_not_terminal` (single-SPU rule) — keep it; it's still valid for single-SPU traces. Rename to clarify `_for_single_spu_trace` if disambiguation needed.
- `parser_rejects_multi_final_state_until_schema_upgrade` (R5.8 hardening contract) — REMOVE or invert, since it explicitly tests the rejection that R5.9 lifts. Replace with `parser_accepts_multi_spu_with_per_spu_final_state`.

---

## R5.9 subphases

| Subphase | Scope | Independently valuable | Depends on |
|---|---|---|---|
| **R5.9a** ✅ DONE 2026-04-28 | Schema spec update + parser only. Added `target_spu: Option<u32>` (with `#[serde(default)]`) to all 7 SPU-side event structs in `trace_fmt.rs`; added `CapturedEvent::target_spu()` accessor; replaced `FinalStateNotTerminal` post-pass check with per-SPU `HashMap<u32, usize>` walk; added new error variants `EventAfterFinalState { target_spu, event_index, final_state_index }` and `DuplicateFinalState { target_spu, first_index, second_index }`; kept `FinalStateNotTerminal` deprecated for backward-compat exhaustive matching. Added 5 new contract tests (interleaved multi-SPU, event-after-final-state, allow-other-spu-after-final-state, default-zero-shim, back-to-back duplicate). All 64 lib tests pass; workspace 5469 / 0 failed. Diagnostic `--ignored` confirms parser now advances to event 40064 (was blocked at `FinalStateNotTerminal { final_state_index: 40063 }`) and bumps into `EventAfterFinalState { target_spu: 0, event_index: 40064, final_state_index: 40063 }` because the writer-side R5.9c hasn't shipped yet so all 6 SPUs collapse to id 0 via the default-0 shim. **No C++ changes, no patches modified, no fixtures committed.** | yes | nothing |
| **R5.9b** ✅ DONE 2026-04-28 | Transformer per-SPU. Added `captured_events_to_traces_per_spu` returning `BTreeMap<u32, Vec<TraceEvent>>` (BTreeMap chosen over HashMap for deterministic iteration in tests/docs). The existing `captured_events_to_trace` was refactored as a wrapper: it calls the per-SPU API, returns the unique group when `len() == 1`, errors `UnterminatedTrace { event_count: 0 }` when the input is empty, and errors with the new `TraceTransformError::MultipleSpusUnsupportedBySingleSpuApi { spu_count }` when the input touches more than one `target_spu` — preventing legacy callers from silently flattening multi-SPU traces. State-machine logic factored into private `transform_single_spu_subset(events: &[&CapturedEvent])`; both public APIs delegate to it. 5 new contract tests (split-two-SPUs, legacy-fixture-preserves, single-SPU-API-rejects-multi, no-PPU-event-mixing, per-SPU-order-preservation). All 69 lib tests pass; workspace 5474 / 0 failed. Diagnostic `--ignored` continues to fail at the parser stage (`EventAfterFinalState { target_spu: 0, event_index: 40064, final_state_index: 40063 }`) — exactly as documented; R5.9b only changed the transformer, and the parser fails first under the pre-R5.9c writer. **No C++ changes, no patches modified, no fixtures committed, replay unaltered.** | yes | R5.9a |
| **R5.9c writer-emit** ✅ DONE 2026-04-28 (re-capture BLOCKED) | Writer `target_spu` emission. `SPUTraceJsonl.{h,cpp}` 7 SPU-side `record_*` methods take `target_spu: u32` as the first parameter and emit `,"target_spu":<value>` immediately after the `kind` field. Runtime hooks updated: cpu_task `TraceFinalGuard` passes `spu->lv2_id`; `get_ch_count`/`stop_and_signal` pass `lv2_id`; `get_ch_value`/`set_ch_value SPU_WrOutMbox` snapshot `const u32 trace_target_spu = lv2_id;` (resp. `_w`) at the same scope as `trace_pc` so the lambda/scope captures are aligned. Both patches re-touched: scaffolding sha256 `2baebca59febacb7eb8a36e6db58dcb585cde095ead0d76262e718b4a5491149` (was `a8baa1a7…b8dbe7b`; +1,805 bytes — 2 lines of inline `,"target_spu":` emit per recorder × 7 + signature widening + comment block); runtime hooks sha256 `3ee7a86148f99cd3e6ee8ccad8aa7f486930851cf3773e9c3f01b140e72bed39` (was `1b69f107…b28694`; +~340 bytes — 2 snapshot lines + `lv2_id` arg threading on 11 SPU-side call sites). `git apply --check` / forward / reverse round-trip validated against post-scaffolding baseline + master baseline. `behavior-freeze/harness/check_patch_separation.py` extended with R5.9c invariant 7 (every SPU-side `record_*` method MUST emit `,"target_spu":` in its JSON line; PPU-side unaffected). Build via `R:\.claude\build_full.bat`: `rpcs3.exe` produced (63.7 MB at 16:19 UTC-3); 9 errors all in `rpcs3_test.vcxproj` for missing `gtest/gtest.h` NuGet package (pre-existing, unrelated to R5.9c per the user's "erros em rpcs3_test/gtest não bloqueiam se rpcs3.exe existe" rule); `emucore.vcxproj` (containing SPUTraceJsonl.cpp + SPUThread.cpp) compiled clean with 2 pre-existing warnings (`getenv` C4996 + `TraceFinalGuard` C4530, both unchanged from R5.8 A.3 baseline). Smoke `rpcs3.exe --version` exit 0. **Re-capture of `spurs_test.self` BLOCKED**: this session attempted `rpcs3.exe --headless R:\bin\test\spurs_test.self` with `RPCS3_SPU_TRACE_JSONL` set, but a permission hook denied launching the PS3 binary, citing "running an external PS3 binary goes beyond the user's authorized R5.9c writer-emit scope and risks executing untrusted code". The writer code/patches/build are correct, but no trace was produced from this iteration — R5.9d depends on a re-capture and is correspondingly blocked. | yes (writer side is real-captures-compatible) | R5.9a; ideally R5.9b first so a captured trace is fully validated end-to-end |
| **R5.9d** ✅ DONE 2026-04-28 | Diagnostic `spurs_test` parse/transform validation against the **real R5.9c-captured trace v3**. The pre-R5.9d test referenced the v2 trace (no `target_spu` on SPU-side events; collapsed all 6 SPUs to id 0 via the default-zero shim → second SPU's first event after id-0 finalization triggered `EventAfterFinalState`). R5.9d replaces it with three `#[ignore]`d tests against the real v3 trace at `tests/data/spurs_test_v3_real_trimmed.jsonl` (local-only, untracked, ~4.85 MB / 40,042 lines): (1) `diagnostic_real_trace_v3_parser_passes` asserts the parser accepts the trace cleanly under R5.9a per-SPU validation; (2) `diagnostic_real_trace_v3_per_spu_transformer_passes` asserts `captured_events_to_traces_per_spu` returns >1 group and prints per-SPU event counts; (3) `diagnostic_real_trace_v3_legacy_api_rejects` asserts the legacy `captured_events_to_trace` correctly returns `MultipleSpusUnsupportedBySingleSpuApi { spu_count }` with `spu_count > 1`. Tests stay `#[ignore]`d because (a) the trace file is local-only — not committed as a fixture — and (b) replay validation is R5.9e scope. **Real-trace results captured 2026-04-28**: parser accepted 40,042 events; transformer produced **6 SPU groups** (`target_spu` ids `256, 16777472, 33554688, 50331904, 67109120, 83886336` — distinct `lv2_id`s assigned by RPCS3 to the 6 SPU threads spurs_test creates); per-SPU event counts `1, 51, 53, 52, 53, 53` (the 1-event group is target_spu=256 whose `final_state` was on the truncated last line of the raw capture, dropped by `validate_trace_v3.py`'s monotonicity-checked trim); legacy single-SPU API rejected with `spu_count=6`. **Trace NOT committed** as `behavior-freeze/fixtures/spu/traces/` fixture; `REPLAY_VALIDATED_TRACE_EXISTS = False` flag preserved. New helper script `behavior-freeze/harness/validate_trace_v3.py` validates monotonicity + parses every line, and emits a separate `*_trimmed.jsonl` companion when the rpcs3.exe-killed-mid-write last line is truncated (preserves the original byte-exact). | yes (parse + transform validated end-to-end on real captures) | R5.9a + R5.9b + R5.9c writer-emit + R5.9c re-capture |
| **R5.9e** | Replay strategy. Per-SPU sequential replay first (no lockstep). Requires SPU image capture (writer extension to attach binaries) AND a multi-SPU-aware replay test. Largest scope; defer until R5.9a–d land. | partial — only valuable if SPU image capture ships alongside | R5.9a + R5.9b + R5.9c + SPU image capture writer extension |

R5.9a is the smallest unit and the highest-value first step: it converts the spurs_test trace from "rejected" to "schema-compliant up to transformer", and unblocks R5.9d's diagnostic test flip.

---

## Risks

1. **Default-zero `target_spu` shim collides with real SPU id 0.** A pre-R5.9 single-SPU trace has all events implicitly id=0; a new R5.9c-recaptured trace from the same homebrew would have all events explicitly id=lv2_id (likely non-zero). The two are NOT byte-equivalent. Fixtures committed under one regime cannot interchange with the other. **Mitigation:** annotate every committed fixture's `.notes.md` with the schema version it was captured under.
2. **Multi-SPU `seq` global vs per-SPU.** Keeping `seq` global gives one canonical ordering but couples the SPUs at write time (the lock that scaffolding v2 fixed serializes ALL SPU emits, not just same-SPU). Throughput cost unknown. **Mitigation:** measure under spurs_test before / after; if measurable, consider per-SPU `seq` with a rendezvous merge in the parser. Defer until measured.
3. **Writer race regression in R5.9c.** Adding `target_spu` to every `record_*` is a 10-method edit. Easy to drop `lock_guard` from one method during refactor. **Mitigation:** the existing `behavior-freeze/harness/check_patch_separation.py` writer race guard catches `m_seq.fetch_add` outside `std::lock_guard` per-method. Tighten the script to also require `target_spu` in the emitted line for SPU-side methods.
4. **Transformer per-SPU surface is "set" of types.** Going from `Vec<TraceEvent>` to `HashMap<u32, Vec<TraceEvent>>` may seem trivial but breaks every existing `replay_trace` caller. **Mitigation:** keep `captured_events_to_trace` (single-SPU) as a wrapper that calls `_per_spu` and returns the only entry, error if more than one. Smooth deprecation.
5. **Diagnostic flip in R5.9d depends on R5.9c re-capture.** The 4 MB trace in `tests/data/spurs_test_real.jsonl` was emitted by scaffolding v2 (no `target_spu` on SPU events). It will pass R5.9a parser via the default-0 shim — collapsing all 6 SPUs into one — and then fail at transformer because 6 final_states for "SPU 0" still violates per-SPU invariant. Re-capturing under R5.9c writer is required. **Mitigation:** plan R5.9d as the validation gate AFTER R5.9c re-capture, not before.

---

## What stays blocked

- Replay-validated trace: requires R5.9c writer + SPU image capture (R5.9e) + a real workload run. Earliest unlock: end of R5.9e.
- A first single-SPU homebrew fixture would unblock replay-validation of R5.7/R5.8 immediately (without R5.9). It is still a parallel option.
- Cell SDK / PS3 toolchain is still not in scope for autonomous installation.

---

## Cross-cutting checklist (when implementation starts)

This is a one-page checklist for the implementer of R5.9, NOT a current-iteration action.

- [x] R5.9a: schema doc updated in `SPU_TRACE_CAPTURE.md` to add `target_spu` field on SPU-side events (with backward-compat default-0 explicitly written).
- [x] R5.9a: parser updated; 5 new contract tests added (1 positive interleaved + 4 negative/back-compat); `FinalStateNotTerminal` post-pass check superseded by `EventAfterFinalState` / `DuplicateFinalState` per-SPU walk. Deprecated variant retained for exhaustive matching.
- [x] R5.9a: existing R5.8 hardening contract `parser_rejects_multi_final_state_until_schema_upgrade` updated and renamed to `parser_rejects_duplicate_final_state_same_spu` — asserts `EventAfterFinalState` because the intervening `spu_stop` after final_state triggers it before the second final_state.
- [x] R5.9a: `R5_6_REFERENCE_JSONL` round-trip continues to pass — fixture has no `target_spu` on SPU-side events; default-0 shim makes it implicitly single-SPU id 0.
- [x] R5.9b: transformer per-SPU API `captured_events_to_traces_per_spu` added (BTreeMap-keyed); legacy `captured_events_to_trace` refactored as a wrapper that returns `MultipleSpusUnsupportedBySingleSpuApi` when the input touches more than one `target_spu`; 5 new contract tests; round-trip through `R5_6_REFERENCE_JSONL` continues to pass via the per-SPU API path (1 group keyed `0`).
- [x] R5.9c: `SPUTraceJsonl.{h,cpp}` API extended to accept `target_spu` (7 SPU-side recorders + 1 final_state); scaffolding patch v3 (sha256 `2baebca5…91149`) generated and round-trip-validated against master baseline; runtime hooks patch v2 (sha256 `3ee7a861…2bed39`) generated to pass `spu->lv2_id`/`lv2_id`/`trace_target_spu(_w)` to each `record_*` SPU call site (11 sites total + 2 new `const u32 trace_target_spu(_w) = lv2_id;` snapshot lines in `get_ch_value`/`set_ch_value SPU_WrOutMbox`); round-trip-validated; `R:\.claude\build_full.bat` produces fresh `rpcs3.exe` (emucore.vcxproj compiles clean with only 2 pre-existing warnings, gtest test target failures are pre-existing and unrelated); `--version` smoke passes.
- [x] R5.9c: `behavior-freeze/harness/check_patch_separation.py` writer race guard tightened with new R5.9c invariant 7 — every SPU-side `record_*` method (signature `start_event(os, seq, true, ...)`) MUST emit `,\"target_spu\":` in its body; PPU-side methods unaffected. Gate re-runs green against the new patches.
- [x] R5.9c (re-capture): user produced the v3 trace by manual invocation of the R5.9c-built rpcs3.exe with `RPCS3_SPU_TRACE_JSONL` set against `R:\bin\test\spurs_test.self`. Resulting trace at `C:\Users\manod\AppData\Local\Temp\spurs_test_v3.jsonl` (4,848,746 bytes / 40,042 complete lines + 1 truncated tail line; firmware 4.93, LLVM 19.1.7). The previous-iteration permission-hook block was bypassed by user action; no autonomous workaround was needed.
- [x] R5.9d: diagnostic flip done. `tests/real_trace_diagnostic.rs` rewritten with 3 `#[ignore]`d tests (parser, per-SPU transformer, legacy-API rejection) that reference `tests/data/spurs_test_v3_real_trimmed.jsonl` via runtime `fs::read_to_string` (so the build succeeds even when the trace file is absent on a given developer's machine). Trace **NOT** committed to `behavior-freeze/fixtures/spu/traces/`; `REPLAY_VALIDATED_TRACE_EXISTS = False` preserved (parse + transform alone are not replay).
- [ ] R5.9d: flip `tests/real_trace_diagnostic.rs` to active; rename functions; commit trace + `.notes.md` to `behavior-freeze/fixtures/spu/traces/`; flip `REPLAY_VALIDATED_TRACE_EXISTS` flag in `check_trace_fixtures.py` ONLY after R5.9e passes (parse + transform alone is NOT replay).
- [ ] R5.9e: writer extension for SPU image capture (separate scaffolding patch v4 or new patch); per-SPU sequential replay engine; integration test that loads the captured trace + image and replays both via `InterpreterExecutor` and `RecompilerExecutor`.
