#!/usr/bin/env python3
"""R5.9d diagnostic helper: validate monotonicity + JSON-parsability of
the real `spurs_test_v3.jsonl` trace before handing it to the Rust
parser.

Read-only against the source file. If the file's trailing line is
truncated (a known artifact of killing rpcs3.exe mid-write), this
script writes a separate `*_trimmed.jsonl` copy with the bad line
dropped. The original is NEVER modified — both files coexist so the
trim's provenance is auditable.

Run from the repo root:
  python behavior-freeze/harness/validate_trace_v3.py
"""

from __future__ import annotations

import json
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
SRC = REPO_ROOT / "rust" / "rpcs3-spu-differential" / "tests" / "data" / "spurs_test_v3_real.jsonl"
TRIMMED = SRC.with_name(SRC.stem + "_trimmed.jsonl")


def main() -> int:
    if not SRC.is_file():
        print(f"ERROR: source trace missing: {SRC}", file=sys.stderr)
        return 1

    raw = SRC.read_bytes()
    print(f"Source: {SRC.relative_to(REPO_ROOT)}")
    print(f"  - bytes: {len(raw)}")

    # Decode and split by LF. Surface CRLF without normalizing — RPCS3
    # writer emits LF only, so any CR would be a regression.
    text = raw.decode("utf-8", errors="replace")
    if "\r\n" in text:
        print("  - WARNING: file contains CRLF — RPCS3 writer should emit LF only", file=sys.stderr)

    # Split lossless: includes trailing partial line if no final LF.
    lines = text.split("\n")
    # Conventionally a trailing newline produces an empty trailing
    # element; drop it for the line accounting.
    has_trailing_lf = text.endswith("\n")
    if has_trailing_lf:
        line_count = len(lines) - 1
    else:
        line_count = len(lines)
    print(f"  - lines: {line_count} (trailing LF: {has_trailing_lf})")

    # Check trailing line for completeness. A complete event line ends
    # with `}` so we use that as a coarse marker before invoking the
    # full JSON parser.
    last_idx = line_count - 1
    last_raw = lines[last_idx] if not has_trailing_lf else lines[last_idx]
    last_complete = last_raw.endswith("}")
    print(f"  - last line ends with '}}': {last_complete}")

    bad_lines: list[tuple[int, str]] = []
    prev_seq: int | None = None
    monotonic_ok = True
    parse_ok = True
    last_ok_seq: int | None = None
    last_ok_idx: int | None = None

    for idx in range(line_count):
        line = lines[idx]
        if not line.strip():
            continue
        if line.lstrip().startswith("#"):
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError as e:
            parse_ok = False
            bad_lines.append((idx, f"JSON parse error at offset {e.pos}: {e.msg}"))
            continue
        seq = obj.get("seq")
        if not isinstance(seq, int):
            bad_lines.append((idx, f"seq missing or non-integer: {seq!r}"))
            continue
        if prev_seq is not None and seq <= prev_seq:
            monotonic_ok = False
            bad_lines.append(
                (idx, f"non-monotonic seq: prev={prev_seq}, got={seq}")
            )
        prev_seq = seq
        last_ok_seq = seq
        last_ok_idx = idx

    print(f"  - JSON-parsable lines (excluding comments/blanks): {line_count - len(bad_lines)}")
    print(f"  - last good seq: {last_ok_seq} at line index {last_ok_idx}")
    print(f"  - monotonic: {monotonic_ok}")
    print(f"  - parse_ok (every non-comment/non-blank parses): {parse_ok}")

    for idx, msg in bad_lines[:5]:
        print(f"    BAD line[{idx}]: {msg}", file=sys.stderr)
    if len(bad_lines) > 5:
        print(f"    ... ({len(bad_lines) - 5} more)", file=sys.stderr)

    # If the last line is truncated AND it's the only bad line, write a
    # trimmed companion file. Anything broader than that is reported
    # but not auto-trimmed — preserves the user's "NÃO editar o trace"
    # rule by keeping the source intact and only producing an explicit
    # trimmed COPY.
    only_last_truncated = (
        len(bad_lines) == 1
        and bad_lines[0][0] == line_count - 1
        and not last_complete
    )

    if only_last_truncated:
        print(
            f"\nLast line is truncated mid-JSON (rpcs3.exe killed mid-write)."
            f"\nWriting trimmed copy: {TRIMMED.relative_to(REPO_ROOT)}"
        )
        # Re-emit lines 0..(last_complete_idx) preserving original
        # newlines. We trim by writing exactly the byte slice up to and
        # including the LAST `\n` before the truncated line.
        offset_of_last_lf = raw.rfind(b"\n")
        # If the file ends with `\n` (trailing newline), then there's
        # no truncation — we wouldn't be here. So there's content after
        # the last `\n` and that's the truncated tail.
        if offset_of_last_lf < 0:
            print("  ERROR: no LF found in file; cannot trim safely", file=sys.stderr)
            return 2
        # +1 to include the LF terminator of the last GOOD line.
        TRIMMED.write_bytes(raw[: offset_of_last_lf + 1])
        print(f"  - trimmed bytes: {TRIMMED.stat().st_size}")
        # Re-count lines in the trimmed file for the report.
        trimmed_lines = TRIMMED.read_bytes().count(b"\n")
        print(f"  - trimmed line count: {trimmed_lines}")
        return 0

    if bad_lines:
        print(f"\nFAIL: {len(bad_lines)} bad line(s); trimmed copy NOT generated automatically", file=sys.stderr)
        return 3

    print("OK: all lines parse + seq monotonic")
    return 0


if __name__ == "__main__":
    sys.exit(main())
