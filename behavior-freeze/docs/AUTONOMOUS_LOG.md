# Deprecated workflow stub (active append-only log)

This file is kept at this exact path because **`.claude/settings.local.json`
hooks depend on it**:

- The `Stop` hook appends a `"turn ended"` timestamp to this file at the
  end of every turn.
- The `SessionStart` hook reads the last 5 lines of this file via
  `tail -5` to seed each new session with prior-turn context.

Do not delete or rename this file, and do not move it. The hooks need
both the path and the file's append-only nature.

**Do not** use this file as the current project status — newer
`turn ended` timestamps will accumulate below this header from now on,
but they are session telemetry, not status content.

- **Current authoritative status:** [`../../docs/PROJECT_STATUS.md`](../../docs/PROJECT_STATUS.md)
- **Full historical log (all 230+ iterations of chronological detail) preserved at:** [`../../historico/pre-r4b-2026-04-25/AUTONOMOUS_LOG.md`](../../historico/pre-r4b-2026-04-25/AUTONOMOUS_LOG.md)

---

## Session telemetry (appended by hooks)
