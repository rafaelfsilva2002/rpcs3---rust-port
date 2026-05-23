# behavior-freeze/docs/

This directory contains **only path-locked operational stubs**. Anything
that does not have an external system depending on the exact path
`behavior-freeze/docs/<file>` was moved to
[`docs/history/`](../../docs/history/) on 2026-05-22 (the "Option B
consolidation").

## What lives here

| File | Why this path is locked |
|---|---|
| `AUTONOMOUS_LOG.md` | Claude Code hooks defined in `.claude/settings.local.json` (Stop event + SessionStart event) reference this exact path: the Stop hook appends `<timestamp> turn ended` lines via shell `>>`, and the SessionStart hook reads the last 5 lines via `tail -5` to surface them as additional context to the next session. Moving the file would require updating both hook commands. |
| `SPU_RECOMPILER_PLAN.md` | The Rust source `rust/rpcs3-spu-recompiler/src/lib.rs` carries a doc-comment that references this exact path. Moving the file would require updating the Rust source comment. |

## What was moved (and to where)

Five legacy R4-era behavior-freeze contract docs were moved to
`docs/history/` on 2026-05-22:

| Old path | New path |
|---|---|
| `behavior-freeze/docs/INVENTORY.md` | `docs/history/INVENTORY.md` |
| `behavior-freeze/docs/DECISIONS.md` | `docs/history/DECISIONS.md` |
| `behavior-freeze/docs/DEFERRED.md` | `docs/history/DEFERRED.md` |
| `behavior-freeze/docs/BACKLOG_RESIDUAL.md` | `docs/history/BACKLOG_RESIDUAL.md` |
| `behavior-freeze/docs/HOMEBREW_PLAN.md` | `docs/history/HOMEBREW_PLAN.md` |

Rationale: all 5 carried explicit point-in-time disclaimers redirecting
readers to current `docs/PROJECT_STATUS.md`, and had no external
path dependency. Keeping them inside `behavior-freeze/docs/` made the
project look like it had two parallel docs folders. The consolidation
preserves the historical content (verbatim) at the new path under
`docs/history/`.

## Discovery rule

- Looking for **current project status / phase / roadmap**? Read
  [`docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md).
- Looking for an **older snapshot / archived plan**? Browse
  [`docs/history/`](../../docs/history/).
- Looking for **pre-R4b verbatim state**? Browse
  [`historico/pre-r4b-2026-04-25/`](../../historico/pre-r4b-2026-04-25/).

## Hard rules

- ❌ Do NOT add new files here unless they have a real external
  path dependency (Claude Code hook, Rust source doc-comment,
  CI config that references the exact path, etc.).
- ❌ Do NOT move `AUTONOMOUS_LOG.md` or `SPU_RECOMPILER_PLAN.md`
  without updating the dependent system (hook commands or Rust
  source comment).
- ✅ If a new docs artifact is needed, prefer creating it under
  `docs/` (live) or `docs/history/` (archive).
