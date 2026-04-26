# Architectural Decisions Log

**Last updated:** 2026-04-24 (decision content); count snapshots below
record the state at decision time and are intentionally **not**
back-edited. For the current verified status, see
[`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md).

**Format:** ADR-style (title, decision, status, rationale, consequence).
Decisions here are the load-bearing ones — anything that, if reversed,
would require redoing significant work.

> Note: numbers like "230 crates / 5165 tests" inside individual ADRs
> describe the project state at the moment the decision was recorded.
> They are point-in-time snapshots, not current-status claims. Refer to
> `../../docs/PROJECT_STATUS.md` for the current numbers.

---

## ADR-001 — Behavior-freeze first

- **Decision:** Replicate observable RPCS3 contracts byte-for-byte before any optimization, refactor, or modernization.
- **Status:** Active. 230 crates / 5165 tests delivered under this rule.
- **Rationale:** RPCS3 is the oracle for "what PS3 does." Any deviation we can't justify byte-for-byte risks games that work upstream not working here. This is non-negotiable for hardware-emulation correctness.
- **Consequence:** We accept some "ugly" code (e.g. preserving cpp's quirks like `bcdDevice = pid` in usb_vfs, or the `0xABADCAFE` spinlock sentinel). Cleanup is post-freeze, never pre-freeze.

---

## ADR-002 — Preserve observable RPCS3-style contracts

- **Decision:** Public API of every Rust crate mirrors the cpp class / namespace shape it ports. No "improved" or "Rustified" interfaces during behavior-freeze.
- **Status:** Active.
- **Rationale:** Lets a future C++ frontend link against our Rust crates via FFI with minimal glue. Lets a maintainer cross-reference cpp ↔ rust line-by-line.
- **Consequence:** Some Rust idiom is sacrificed (e.g. `Option<&str>` instead of `Result<&str, Error>`, sentinel-value error codes instead of typed errors). Future Rustification waves can address this.

---

## ADR-003 — Inventory P0/P1/P2 drives prioritization

- **Decision:** [`INVENTORY.md`](INVENTORY.md) classifies modules as P0 (critical path), P1 (important), P2 (nice-to-have). Work proceeds P0 → P1 → P2.
- **Status:** Active.
- **Rationale:** Without explicit prioritization, port work tends to drift to easy wins instead of structurally important ones.
- **Consequence:** 100% Cell/Modules coverage was achieved before any RSX runtime work, even though some P2 RSX helpers are smaller / easier than P0 HLE modules.

---

## ADR-004 — `compare_run.py` is the differential gate

- **Decision:** Any change to a ported crate must pass `compare_run.py` against RPCS3 C++ output (when fixtures exist).
- **Status:** Infrastructure ready, real fixture-driven runs blocked on items 3-5 in [`DEFERRED.md`](DEFERRED.md).
- **Rationale:** Unit tests prove the crate compiles and matches a hand-written oracle. Differential runs prove the crate matches the **real** RPCS3 binary on the **real** workload. Different and complementary signals.
- **Consequence:** Tests passing != ready to ship; differential runs are the final gate.

---

## ADR-005 — Zero-regression rule

- **Decision:** No commit lands if `cargo test --workspace --lib` doesn't stay green.
- **Status:** Active. **0 regressions across 229 iterations.**
- **Rationale:** Long-running ports drift if regressions accumulate. Forcing zero-regression every iter keeps the system shippable at every commit.
- **Consequence:** Sometimes an "obvious" refactor is rejected because it breaks a downstream test in a way that's not worth fixing yet. That's a feature.

---

## ADR-006 — Ship-of-Theseus incremental replacement

- **Decision:** Replace RPCS3 module-by-module, never as a big-bang rewrite.
- **Status:** Active. Each iter = one module port.
- **Rationale:** Big-bang rewrites famously fail (Joel Spolsky's "Things You Should Never Do"). Incremental replacement preserves the working system at every step.
- **Consequence:** Some inefficiency (each crate re-implements its own small helpers instead of sharing utilities). Acceptable — sharing comes after the surface is locked.

---

## ADR-007 — No big-bang rewrite

- **Decision:** Reject any proposal that requires "rewriting RPCS3 from scratch in Rust" as a single deliverable.
- **Status:** Active.
- **Rationale:** See ADR-006. Restated explicitly because the temptation to "just rewrite the JIT in pure Rust" recurs.
- **Consequence:** Line-by-line ports of the original C++ runtime giants (`PPUTranslator.cpp`, `PPUAnalyser.cpp`, `PPUModule.cpp`, `RSXThread.cpp`, `VKGSRender.cpp`, `rpcs3qt/`, etc.) remain explicitly out of scope (see [`DEFERRED.md`](DEFERRED.md)) until someone signs up for a multi-week dedicated wave. Incremental Rust replacements ARE allowed when test-gated and tracked in [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md) — that's how the SPU recompiler crate (`rust/rpcs3-spu-recompiler`) was built post-2026-04-24 (Cranelift JIT, R1..R4c). Each layer was added in isolation, byte-exact validated against the Rust SPU interpreter, never as a big bang. Big-bang remains forbidden; incremental replacement is the approved path.

---

## ADR-008 — Rust is the default language

- **Decision:** New ports are written in Rust unless a measurable benefit justifies otherwise.
- **Status:** Active.
- **Rationale:** Rust gives us memory safety + cargo + great no_std support + good MSVC interop. Predictable choice = lower decision overhead per iter.
- **Consequence:** All 230 crates are Rust. Workspace is a Cargo workspace. CI / tooling assumes Rust.

---

## ADR-009 — Zig only with measurable benefit

- **Decision:** Zig is allowed for hot paths IF a benchmark shows a measurable benefit over the equivalent Rust port.
- **Status:** Active. **Zero Zig crates committed in this wave** — none of the porting hit the "measurable benefit" bar.
- **Rationale:** Zig has cleaner FFI to C and lower overhead in some scenarios. But mixing two languages doubles tooling cost. Allow only when the benefit is real and measurable.
- **Consequence:** The port is currently 100% Rust + the original C++ source. The SPU recompiler hot path is now in pure Rust via Cranelift (`rust/rpcs3-spu-recompiler`, R1..R4c) and benchmarks vs the Rust interpreter — no Zig was needed there. Door remains open for Zig in future hot-path work (e.g. RSX command stream emitter, PPU JIT, anywhere the Rust+Cranelift stack hits a measurable ceiling).

---

## ADR-010 — Append-only autonomous log

- **Decision:** [`AUTONOMOUS_LOG.md`](AUTONOMOUS_LOG.md) is append-only. Never delete, compact, or rewrite past entries. Truncation == loss of audit trail.
- **Status:** Active. 1689 lines, 229 iter entries preserved.
- **Rationale:** Audit trail is what lets a reviewer say "show me why the port of cellAtracMulti made decision X." Compacting it loses that.
- **Consequence:** The file grows unbounded. Acceptable — text is cheap.

---

## ADR-011 — "Plan substantially complete" ≠ "runtime complete"

- **Decision:** Use precise language: "plan substantially complete" means the **port plan as a documentation/scope artifact** is closed, NOT that the **runtime emulator** is finished.
- **Status:** Active.
- **Rationale:** Confusion between these two ends careers (and projects). The plan describes scope; the runtime is what users boot games with. We have the former; we explicitly do NOT have the latter.
- **Consequence:** README, CURRENT_STATE, and CHECKLIST all repeat this clarification verbatim. For the current status doc, see [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md). The pre-cleanup CURRENT_STATE.md snapshot is at [`../../historico/pre-r4b-2026-04-25/CURRENT_STATE.md`](../../historico/pre-r4b-2026-04-25/CURRENT_STATE.md).

---

## ADR-012 — Frozen baseline at 230 crates / 5165 tests / 229 iters / 0 regressions

- **Decision:** This is the documental baseline ("freeze") for this wave. Subsequent work is measured against this baseline.
- **Status:** Frozen 2026-04-24.
- **Rationale:** Without a baseline, "we're making progress" is unfalsifiable. The frozen numbers + the snapshot files (`PLAN_FREEZE_2026-04-24.md`, `CHECKLIST_FREEZE_2026-04-24.md`, `CURRENT_STATE_2026-04-24.md`) make the baseline reproducible. Those snapshot files now live in [`../../historico/pre-r4b-2026-04-25/`](../../historico/pre-r4b-2026-04-25/) (kept verbatim, never edited).
- **Consequence:** Future work updates the counters in the current status doc [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md) or it's not landed. The freeze snapshots are kept as historical reference and never edited.
