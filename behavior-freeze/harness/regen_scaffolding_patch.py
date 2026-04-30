#!/usr/bin/env python3
"""Regenerate docs/patches/spu_trace_jsonl_scaffolding.patch from the
current state of rpcs3/Emu/Cell/SPUTraceJsonl.{h,cpp}.

Triggered by R5.9c writer change (target_spu emission). The wiring
sections (CMakeLists.txt, emucore.vcxproj, emucore.vcxproj.filters) are
unchanged in R5.9c and are copied verbatim from the previous patch.
The two new-file sections (SPUTraceJsonl.h and SPUTraceJsonl.cpp) are
regenerated from the live working-tree files.

Usage:
  python behavior-freeze/harness/regen_scaffolding_patch.py [--dry-run]

Run from the repo root. Writes the new patch to
`docs/patches/spu_trace_jsonl_scaffolding.patch` (overwrites).
"""

from __future__ import annotations

import hashlib
import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
PATCH_PATH = REPO_ROOT / "docs" / "patches" / "spu_trace_jsonl_scaffolding.patch"

H_PATH = REPO_ROOT / "rpcs3" / "Emu" / "Cell" / "SPUTraceJsonl.h"
CPP_PATH = REPO_ROOT / "rpcs3" / "Emu" / "Cell" / "SPUTraceJsonl.cpp"

# Order of files in the regenerated patch.
NEW_FILE_TARGETS = (
    ("rpcs3/Emu/Cell/SPUTraceJsonl.cpp", CPP_PATH),
    ("rpcs3/Emu/Cell/SPUTraceJsonl.h", H_PATH),
)


def git_blob_hash(data: bytes) -> str:
    """Compute git blob sha1: sha1("blob <len>\\0<content>")."""
    header = f"blob {len(data)}\0".encode("ascii")
    return hashlib.sha1(header + data).hexdigest()


def render_new_file_section(rel_path: str, file_path: Path) -> str:
    raw = file_path.read_bytes()
    blob = git_blob_hash(raw)
    # Use the working-tree text (decoded) for the patch lines; binary
    # files would need a different format but our files are pure ASCII.
    text = raw.decode("utf-8")
    lines = text.split("\n")
    # split() leaves an empty string at the end if the file ended with
    # a newline; drop it so we don't emit a spurious "+" line. Track
    # the `\ No newline at end of file` marker if absent.
    trailing_newline = text.endswith("\n")
    if trailing_newline:
        lines = lines[:-1]
    line_count = len(lines)
    plus_lines = "\n".join("+" + l for l in lines)
    no_newline_marker = "" if trailing_newline else "\n\\ No newline at end of file"
    return (
        f"diff --git a/{rel_path} b/{rel_path}\n"
        f"new file mode 100644\n"
        f"index 000000000..{blob[:9]}\n"
        f"--- /dev/null\n"
        f"+++ b/{rel_path}\n"
        f"@@ -0,0 +1,{line_count} @@\n"
        f"{plus_lines}{no_newline_marker}\n"
    )


def extract_wiring_sections(old_patch: str) -> list[str]:
    """Return the list of file-diff sections in the old patch that are
    NOT for the two SPUTraceJsonl files. Each section is the raw
    `diff --git ...` block, ending right before the next `diff --git`.
    """
    sections = []
    cur: list[str] = []
    for line in old_patch.splitlines(keepends=True):
        if line.startswith("diff --git "):
            if cur:
                sections.append("".join(cur))
            cur = [line]
        else:
            cur.append(line)
    if cur:
        sections.append("".join(cur))

    skip_targets = {
        "rpcs3/Emu/Cell/SPUTraceJsonl.cpp",
        "rpcs3/Emu/Cell/SPUTraceJsonl.h",
    }
    keep = []
    for s in sections:
        m = re.match(r"diff --git a/(\S+) b/", s)
        if m and m.group(1) in skip_targets:
            continue
        keep.append(s)
    return keep


def main() -> int:
    dry_run = "--dry-run" in sys.argv

    old = PATCH_PATH.read_text(encoding="utf-8")
    wiring = extract_wiring_sections(old)

    # New-file sections (SPUTraceJsonl.cpp first, then .h, matching the
    # original patch's ordering).
    new_file_sections = [
        render_new_file_section(rel_path, p) for rel_path, p in NEW_FILE_TARGETS
    ]

    # Reassemble: original patch order was:
    #   1. rpcs3/Emu/CMakeLists.txt
    #   2. rpcs3/Emu/Cell/SPUTraceJsonl.cpp
    #   3. rpcs3/Emu/Cell/SPUTraceJsonl.h
    #   4. rpcs3/emucore.vcxproj
    #   5. rpcs3/emucore.vcxproj.filters
    # The new-file blocks slot in between #1 and #4. We rely on
    # extract_wiring_sections preserving the original order of the
    # wiring sections.
    if len(wiring) != 3:
        print(
            f"ERROR: expected 3 wiring sections (CMakeLists + 2 vcxproj), "
            f"got {len(wiring)}",
            file=sys.stderr,
        )
        return 1

    # wiring[0] = CMakeLists.txt, wiring[1] = emucore.vcxproj,
    # wiring[2] = emucore.vcxproj.filters.
    # Preserve that. Insert new-file sections after CMakeLists.
    #
    # Pre-existing drift fix: the working-tree CMakeLists.txt (and the
    # `R5.8 A.3` source comments in SPUTraceJsonl.{h,cpp}) use em-dash
    # `—` (U+2014) but the legacy v1/v2 scaffolding patch had two
    # ASCII hyphens `--`. Apply the substitution here so the
    # regenerated patch round-trips byte-exact against the working
    # tree. Idempotent: matches `R5.8 A.3 -- ` only, which the new
    # text never contains.
    cmakelists = wiring[0].replace("R5.8 A.3 -- ", "R5.8 A.3 — ")
    out = cmakelists + "".join(new_file_sections) + wiring[1] + wiring[2]

    if dry_run:
        sys.stdout.write(out)
        return 0

    PATCH_PATH.write_text(out, encoding="utf-8", newline="\n")
    new_sha = hashlib.sha256(PATCH_PATH.read_bytes()).hexdigest()
    print(f"OK: regenerated {PATCH_PATH.relative_to(REPO_ROOT)}")
    print(f"  - bytes: {PATCH_PATH.stat().st_size}")
    print(f"  - sha256: {new_sha}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
