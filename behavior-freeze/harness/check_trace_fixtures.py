#!/usr/bin/env python3
"""R5.8 hardening gate: trace fixture directory invariants.

Enforces the rules documented in
`behavior-freeze/fixtures/spu/traces/README.md`:

  1. Only `README.md` may exist alone in the directory.
  2. Every `.jsonl` (or `.jsonl.gz`) MUST have a paired `.notes.md`
     companion documenting origin, license, capture command, RPCS3
     commit, scaffolding+runtime patch sha256s, and replay results.
  3. While `replay-validated` has not been achieved (per
     PROJECT_STATUS.md), no `.jsonl` should be committed yet — the
     directory is expected to contain only `README.md`.

This is a regression guard, not a one-time check. Run from CI or a
manual `just check` recipe before any commit that touches this dir.

Exit codes:
  0 — invariants hold.
  1 — at least one violation. Stderr lists every violation.

Manual run:
  python behavior-freeze/harness/check_trace_fixtures.py
"""

from __future__ import annotations

import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
TRACE_DIR = REPO_ROOT / "behavior-freeze" / "fixtures" / "spu" / "traces"

# Toggle to True only when at least one trace has been replay-validated
# end-to-end (parse + transform + replay × Interpreter + replay × Recompiler).
# Until then, the directory is expected to contain only README.md.
#
# Flipped True at R5.9e.7 (2026-04-29) when single_spu_mailbox_v1
# became the first replay-validated SPU trace fixture: captured from
# RPCS3 against a CC0 PSL1GHT homebrew, parser/transformer/replay
# accepted byte-identically across InterpreterExecutor and
# RecompilerExecutor. See:
# `rust/rpcs3-spu-recompiler/tests/single_spu_mailbox_v1_replay.rs`
# and the matching `.notes.md` next to the JSONL.
REPLAY_VALIDATED_TRACE_EXISTS = True


def main() -> int:
    if not TRACE_DIR.is_dir():
        print(f"ERROR: trace directory missing: {TRACE_DIR}", file=sys.stderr)
        return 1

    violations: list[str] = []
    files = sorted(TRACE_DIR.iterdir())
    names = [f.name for f in files if f.is_file()]

    if "README.md" not in names:
        violations.append("missing README.md in trace fixture directory")

    jsonl_files = [n for n in names if n.endswith(".jsonl") or n.endswith(".jsonl.gz")]
    notes_files = {n for n in names if n.endswith(".notes.md")}

    if not REPLAY_VALIDATED_TRACE_EXISTS and jsonl_files:
        violations.append(
            "no replay-validated trace exists yet (per PROJECT_STATUS.md), "
            "but the following .jsonl files were found: "
            f"{jsonl_files}. Either flip REPLAY_VALIDATED_TRACE_EXISTS in this "
            "script (after replay × Interpreter AND replay × Recompiler both "
            "pass against the trace) or remove the files."
        )

    for jf in jsonl_files:
        stem = jf.removesuffix(".jsonl.gz").removesuffix(".jsonl")
        notes_name = f"{stem}.notes.md"
        if notes_name not in notes_files:
            violations.append(
                f"trace fixture '{jf}' has no companion '{notes_name}'. "
                "Every .jsonl MUST have a .notes.md per the directory README."
            )

    extras = set(names) - {"README.md"} - set(jsonl_files) - notes_files
    if extras:
        violations.append(
            f"unexpected files in trace fixture directory: {sorted(extras)}. "
            "Only README.md, *.jsonl, *.jsonl.gz, and *.notes.md are allowed."
        )

    if violations:
        print(f"FAIL: {len(violations)} violation(s) in {TRACE_DIR}", file=sys.stderr)
        for v in violations:
            print(f"  - {v}", file=sys.stderr)
        return 1

    print(f"OK: {TRACE_DIR.relative_to(REPO_ROOT)} satisfies the contract")
    print(f"  - REPLAY_VALIDATED_TRACE_EXISTS = {REPLAY_VALIDATED_TRACE_EXISTS}")
    print(f"  - files: {names}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
