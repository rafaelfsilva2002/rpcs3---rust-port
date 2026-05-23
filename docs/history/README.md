# docs/history/

**Single archive location** for project documentation snapshots, deprecated
plans, and point-in-time stubs that redirect readers to current
authoritative docs.

## What lives here

| File | Original location | Why archived |
|---|---|---|
| [`PROJECT_STATUS_R5_ARCHIVE.md`](./PROJECT_STATUS_R5_ARCHIVE.md) | `docs/PROJECT_STATUS.md` (R5 closure 2026-04-29) | Long-form R4/R5 iteration log + R5.11 / R5.11b additive expansions, moved out of live `PROJECT_STATUS.md` at R6 closure 2026-05-03. |
| [`INVENTORY.md`](./INVENTORY.md) | `behavior-freeze/docs/INVENTORY.md` | P0/P1/P2 contract inventory (stable facts per crate) from R4-era behavior-freeze contract. Moved 2026-05-22. |
| [`DECISIONS.md`](./DECISIONS.md) | `behavior-freeze/docs/DECISIONS.md` | Architectural decision records (ADR-style) with explicit "point-in-time" header. Moved 2026-05-22. |
| [`DEFERRED.md`](./DEFERRED.md) | `behavior-freeze/docs/DEFERRED.md` | List of items deferred at R4-era behavior-freeze; cross-references current `PROJECT_STATUS.md`. Moved 2026-05-22. |
| [`BACKLOG_RESIDUAL.md`](./BACKLOG_RESIDUAL.md) | `behavior-freeze/docs/BACKLOG_RESIDUAL.md` | Residual backlog from R4-era cleanup; explicitly stamped point-in-time. Moved 2026-05-22. |
| [`HOMEBREW_PLAN.md`](./HOMEBREW_PLAN.md) | `behavior-freeze/docs/HOMEBREW_PLAN.md` | P1..P5 homebrew fixture plan. Referenced from `docs/SPU_TRACE_CAPTURE.md` and one fixture README. Moved 2026-05-22. |

## What does NOT live here

- **Live authoritative docs.** Those stay in `docs/` at the top level.
  Current source of truth is `docs/PROJECT_STATUS.md`.
- **Pre-R4b verbatim snapshots.** Those live at
  [`historico/pre-r4b-2026-04-25/`](../../historico/pre-r4b-2026-04-25/)
  (Portuguese-named directory, even older archive layer).
- **Path-locked operational stubs.** Two files MUST remain at
  `behavior-freeze/docs/` because external systems reference the exact
  path:
  - `AUTONOMOUS_LOG.md` — Claude Code Stop / SessionStart hooks
    (defined in `.claude/settings.local.json`) `tail` and append to
    this exact path.
  - `SPU_RECOMPILER_PLAN.md` — referenced from a doc-comment in
    `rust/rpcs3-spu-recompiler/src/lib.rs`.

  See `behavior-freeze/docs/README.md` for the lock-in rationale.

## Discovery rule

If you're looking for "the current state of X", **don't read this
directory** — read `docs/PROJECT_STATUS.md`. Files here are
intentionally outdated snapshots preserved for historical context
and forensic debugging.

## Hard rules

- ✅ Files moved here MUST have an explicit point-in-time marker or
  redirect-to-live-doc footer (most already do).
- ❌ Never re-edit archived files in a way that pretends they
  represent current state.
- ❌ Never delete a file from here without recording the move in
  `docs/PROJECT_STATUS.md` or the relevant cleanup commit.
