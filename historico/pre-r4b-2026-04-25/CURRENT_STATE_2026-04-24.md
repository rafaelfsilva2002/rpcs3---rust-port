# Current State — RPCS3 → Rust Port

**Last updated:** 2026-04-24
**Frozen baseline:** [`PLAN_FREEZE_2026-04-24.md`](PLAN_FREEZE_2026-04-24.md), [`CHECKLIST_FREEZE_2026-04-24.md`](CHECKLIST_FREEZE_2026-04-24.md), [`CURRENT_STATE_2026-04-24.md`](CURRENT_STATE_2026-04-24.md)

## Numbers

- **230** crates (Cargo workspace)
- **5165** passing deterministic tests
- **229** autonomous iterations consecutive
- **0** regressions across the entire session

## Methodology

- **behavior-freeze first** — replicate observable RPCS3 contracts byte-for-byte before any optimization
- **`compare_run.py`** is the differential gate (see [`harness/compare_run.py`](../harness/compare_run.py))
- **inventory P0/P1/P2** drives prioritization (see [`INVENTORY.md`](INVENTORY.md))
- **Ship-of-Theseus incremental** — module-by-module replacement, never big-bang rewrite
- **zero-regression rule** — every change keeps the full workspace green
- **append-only autonomous log** — see [`AUTONOMOUS_LOG.md`](AUTONOMOUS_LOG.md), 1689 lines / 229 iterations

## Language strategy

- **Rust is the default** for all new ports
- **Zig only enters with measurable benefit** (none committed in this wave; option remains open for hot paths)

## Plan status

**Plan substantially complete.**

> ⚠️ **IMPORTANT CLARIFICATION:** "Plan substantially complete" does **NOT** mean "complete runtime parity with RPCS3."
>
> What is complete: the **port plan as a documentation/scope artifact** — every viable byte-exact port from `Cell/Modules/`, `Audio/`, `Io/`, `Loader/`, `RSX/` (helpers), `NP/`, LV2 syscalls, and HLE modules has been delivered.
>
> What is **not** complete: the **runtime emulator** — the giants (SPU/PPU Recompilers, PPU Translator, RSX Thread, VKGSRender, System.cpp, Qt UI) remain explicitly out of scope and would each be a multi-week dedicated project. Contract stubs exist in `rpcs3-ppu-interpreter` / `rpcs3-spu-interpreter` / `rpcs3-ppu-thread` / `rpcs3-spu-thread` to satisfy the behavior-freeze wave's needs.

## Next phase (execution)

Move from "module surface coverage" to "execution against real targets":

- **Real fixtures** — at least one open-source PPU homebrew (e.g. ps3autotests) committed and reproducible
- **Homebrew differential validation** — run our crates alongside RPCS3 C++ on the same fixture, diff log + frame hash + WAV
- **Save/load real validation** — exercise cellSavedata against real save data + delete/load cycles
- **Sentinel commercial title** — pick one as canonical regression sentinel (avoids drift)
- **Performance/RAM/VRAM profiling** — only after correctness baseline is locked

See [`ROADMAP.md`](ROADMAP.md) for the full phase list.

## See also

- [`CHECKLIST.md`](CHECKLIST.md) — operational checklist with per-wave status
- [`BACKLOG_RESIDUAL.md`](BACKLOG_RESIDUAL.md) — small remaining pieces by category
- [`DEFERRED.md`](DEFERRED.md) — explicitly deferred items (with reason / required input / unblock condition)
- [`DECISIONS.md`](DECISIONS.md) — architectural decisions log
- [`ROADMAP.md`](ROADMAP.md) — next-phase plan
