#!/usr/bin/env python3
"""R5.8 hardening gate: patch separation + writer race regression guard.

Enforces three invariants on the two SPU trace patches:

  1. `docs/patches/spu_trace_jsonl_scaffolding.patch` exists.
  2. `docs/patches/spu_trace_jsonl_runtime_hooks.patch` exists.
  3. They are SEPARATE files (different sha256, distinct content).
  4. Scaffolding patch only touches scaffolding files (writer + build
     wiring). It MUST NOT touch the SPU/PPU hot-path source files
     listed in HOT_PATH_FILES — those belong to the runtime hooks
     patch. Mixing the two would violate the documented apply order
     (scaffolding → build → runtime hooks → build → smoke → capture)
     in `docs/patches/README.md`.
  5. Runtime hooks patch MUST NOT touch SPUTraceJsonl.{h,cpp} —
     those belong to the scaffolding patch. Allowed exception: a
     bug in the writer that the runtime hooks revealed; in that
     case the fix lands in scaffolding (regenerated), not in
     runtime hooks.
  6. Writer race regression guard: in the scaffolding patch, every
     `m_seq.fetch_add(...)` call MUST appear inside a function that
     has previously taken `m_write_mutex` via `std::lock_guard` (or
     equivalent). Textual heuristic, not perfect — guards against the
     obvious regression of moving seq allocation back outside the
     lock (the bug fixed in scaffolding v2 after the spurs_test real
     trace surfaced it).
  7. R5.9c target_spu emission guard: every SPU-side `record_*` method
     in the scaffolding patch (i.e., every method calling
     `start_event(os, seq, true, ...)`) MUST also contain
     `,"target_spu":` in its emit body. This is the load-bearing
     invariant that lets multi-SPU traces parse cleanly under R5.9a
     and group correctly under R5.9b. PPU-side methods are unaffected
     (they have always emitted `target_spu`).
  8. R5.9e.3 `record_spu_image` presence guard: scaffolding patch MUST
     declare AND implement `record_spu_image` (with `target_spu` as
     the first parameter, per R5.9c convention). Runtime hooks patch
     MUST contain at least one call site invoking
     `record_spu_image(...)`. Catches the regression of removing
     either side of the writer/runtime contract while leaving the
     other in place. Side-file emit + JSONL emit are validated by
     other invariants (6 lock contract, 7 target_spu emit) — this
     one only checks the API surface is wired end-to-end.

Exit codes:
  0 — all invariants hold.
  1 — at least one violation. Stderr lists every violation.
"""

from __future__ import annotations

import hashlib
import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent.parent
PATCH_DIR = REPO_ROOT / "docs" / "patches"
SCAFFOLDING = PATCH_DIR / "spu_trace_jsonl_scaffolding.patch"
RUNTIME_HOOKS = PATCH_DIR / "spu_trace_jsonl_runtime_hooks.patch"

# R6.1 — optional Rust SPU bridge patch. Tracked separately from the
# pinned trace-writer pair: the bridge patch's content is NOT part of
# the writer's separation contract (different scope), but its sha256
# is pinned so the patch can't drift silently. The pin is OPTIONAL:
# if the file is absent, the gate stays green (R6.0 / pre-R6.1
# states are valid).
RUST_BRIDGE = PATCH_DIR / "spu_rust_bridge.patch"
# R8.1 (2026-05-19) — superseded R7.2 sha
# a1e810264d8d9474018c279606111b543eb3f6b6c5845839382e4a657e220e70
# Extends R7.2's runtime DMA GET dispatch with symmetric PUT
# dispatch (LS → EA, the inverse direction):
#   - bridge_dma_put_callback() — reads `src_ls_ptr` bytes (already
#     populated by the SPU at dispatch time) and writes them to
#     `vm::_ptr<u8>(eal)`. Mirror of bridge_dma_get_callback's
#     read-path but data flows the opposite way; uses the same
#     captured-tag-stat queueing on success.
#   - rust_spu_set_dma_put_callback(handle, &bridge_dma_put_callback,
#     &spu) installed alongside the GET callback on every
#     rust_spu_new in try_delegate_execution(). The refuse_mfc gate
#     is RELAXED whenever EITHER callback is installed (R7.2
#     unchanged); the Rust interpreter routes wrch ch21 by cmd
#     value (0x40 → GET callback, 0x20 → PUT callback, other →
#     MfcUnsupported).
#   - SUCCESS log on every PUT dispatch: "R8.1 DMA PUT dispatched:
#     cmd=0x20 eal=0x... size=N tag=T ... real LS/EA path
#     (vm::_ptr<u8>); tag-stat 1<<T queued for subsequent rdch ch24".
#   - List / atomic / lock-line variants still surface MfcUnsupported
#     via the R7.1 outcome arm — bridge falls back honestly for any
#     path R8.1 does not handle (PUT extension is GET-shape only).
# R8.1 acceptance verified on single_spu_dma_put_v1.self via
# check_triple_symmetry.py --fixture put: bridge OFF and bridge ON
# both produce canonical TTY 0xc0ffeeca / 0xcafea57e; bridge ON
# delegates end-to-end (total_steps >1000, NO MfcUnsupported
# fallback); replay oracle byte-identical (8th oracle).
RUST_BRIDGE_PINNED_SHA256 = (
    "0afda1c6943feb5d98329299a57dd68404095efb0a792839779febed13ab8a7e"
)

# Hot-path source files that runtime hooks edit; scaffolding MUST NOT
# touch any of these.
HOT_PATH_FILES = (
    "rpcs3/Emu/Cell/SPUThread.cpp",
    "rpcs3/Emu/Cell/SPUCommonRecompiler.cpp",
    "rpcs3/Emu/Cell/SPUInterpreter.cpp",
    "rpcs3/Emu/Cell/SPULLVMRecompiler.cpp",
    "rpcs3/Emu/Cell/SPUASMJITRecompiler.cpp",
    "rpcs3/Emu/Cell/RawSPUThread.cpp",
    "rpcs3/Emu/Cell/lv2/sys_spu.cpp",
)

# Scaffolding-only files; runtime hooks MUST NOT touch any of these.
SCAFFOLDING_FILES = (
    "rpcs3/Emu/Cell/SPUTraceJsonl.h",
    "rpcs3/Emu/Cell/SPUTraceJsonl.cpp",
    "rpcs3/Emu/CMakeLists.txt",
    "rpcs3/emucore.vcxproj",
    "rpcs3/emucore.vcxproj.filters",
)


def sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


def patch_touches(patch_text: str, target: str) -> bool:
    """Return True if the patch contains a `diff --git` or +++ header
    referencing the target path. Whitespace-tolerant."""
    pattern = rf"(^diff --git [ab]/{re.escape(target)}|^\+\+\+ b/{re.escape(target)})"
    return bool(re.search(pattern, patch_text, re.MULTILINE))


def check_writer_race_guard(scaffolding_text: str) -> list[str]:
    """Heuristic: every `m_seq.fetch_add` line appearing in the
    scaffolding patch's added (`+`) lines must be in the same `record_*`
    function whose body also contains an added `std::lock_guard` or
    `m_write_mutex` line preceding it. We approximate by collecting all
    added `+` lines from the scaffolding patch's SPUTraceJsonl.cpp
    section and scanning for the pattern.
    """
    violations: list[str] = []

    cpp_section_match = re.search(
        r"^diff --git [ab]/rpcs3/Emu/Cell/SPUTraceJsonl\.cpp.*?(?=^diff --git |\Z)",
        scaffolding_text,
        re.MULTILINE | re.DOTALL,
    )
    if not cpp_section_match:
        violations.append(
            "scaffolding patch: SPUTraceJsonl.cpp section not found — "
            "writer race guard cannot run"
        )
        return violations

    cpp_section = cpp_section_match.group(0)
    added_lines = [
        l[1:] for l in cpp_section.splitlines() if l.startswith("+") and not l.startswith("+++")
    ]
    added_text = "\n".join(added_lines)

    # Count fetch_add and lock_guard occurrences in added content.
    # If fetch_add appears at all, lock_guard must dominate (same number
    # of lock_guard sites as fetch_add sites, since each record_* should
    # have exactly one of each).
    fetch_count = added_text.count("m_seq.fetch_add")
    lock_count = added_text.count("std::lock_guard")
    if fetch_count == 0:
        # Patch may be at a state where fetch_add isn't in added lines
        # (e.g., context lines only). Skip the strong check.
        return violations

    if lock_count < fetch_count:
        violations.append(
            f"writer race regression: scaffolding patch added {fetch_count} "
            f"`m_seq.fetch_add` site(s) but only {lock_count} `std::lock_guard` "
            "site(s). Each `record_*` MUST take `m_write_mutex` BEFORE "
            "calling `next_seq()` / `m_seq.fetch_add()`. This is the bug "
            "fixed in scaffolding v2 after the spurs_test real trace "
            "surfaced non-monotonic seq."
        )

    # Per-method check: for each `record_*` function in the added
    # content, the next_seq()/m_seq.fetch_add call must be preceded by
    # a std::lock_guard acquisition of m_write_mutex.
    #
    # We split by `void TraceWriter::record_` and DROP the pre-record_*
    # prologue (chunk[0]) which contains the `next_seq()` helper itself
    # — the helper does NOT need its own lock; the contract is "callers
    # of next_seq must hold m_write_mutex".
    method_blocks = re.split(
        r"(?=void TraceWriter::record_)", added_text
    )[1:]  # drop prologue

    for block in method_blocks:
        sig = block.splitlines()[0] if block else "?"
        # The block contains either `next_seq()` (the helper-call form) or
        # `m_seq.fetch_add(...)` (the inlined form). Either way, lock must
        # be acquired earlier in the block.
        seq_marker = None
        for needle in ("next_seq()", "m_seq.fetch_add"):
            if needle in block:
                seq_marker = needle
                break
        if seq_marker is None:
            # record_* that doesn't allocate seq is conceptually fine,
            # but in this writer every record_* allocates exactly one.
            # If a future contributor adds a non-emit record_*, ignore.
            continue
        if "std::lock_guard" not in block:
            violations.append(
                f"writer race regression: `{sig.strip()[:80]}` "
                f"calls `{seq_marker}` but does NOT contain `std::lock_guard`. "
                "Add `std::lock_guard<std::mutex> lk(m_write_mutex);` BEFORE "
                "the seq allocation, per scaffolding v2 contract."
            )
            continue
        lock_idx = block.index("std::lock_guard")
        seq_idx = block.index(seq_marker)
        if seq_idx < lock_idx:
            violations.append(
                f"writer race regression: `{sig.strip()[:80]}` "
                f"has `{seq_marker}` BEFORE `std::lock_guard`. The seq must "
                "be allocated under the lock to keep on-disk order matching "
                "alloc order. This is the bug fixed in scaffolding v2."
            )

        # R5.9c invariant: every SPU-side `record_*` method must emit
        # the `target_spu` JSON field. SPU-side methods are identified
        # by `start_event(os, seq, true, ` (the `true` = spu_side flag).
        # PPU-side methods (`false` flag) have always emitted
        # `target_spu` and are excluded. Match the C++ source form
        # `,\"target_spu\":` (backslash-escaped quotes inside a
        # string literal) since that's what appears in the patch text;
        # the JSON the parser consumes is `,"target_spu":` (no backslash)
        # but C++ source needs the escapes.
        if "start_event(os, seq, true," in block and ',\\"target_spu\\":' not in block:
            violations.append(
                f"R5.9c writer regression: SPU-side `{sig.strip()[:80]}` "
                "does NOT emit `\",\"target_spu\":\"` in its JSON line. Every "
                "SPU-side `record_*` method MUST emit `target_spu` so the "
                "Rust R5.9a parser per-SPU `final_state` validation and the "
                "R5.9b transformer per-SPU grouping work on real traces. "
                "Add a `target_spu` parameter to the method signature and "
                "emit `os << \",\\\"target_spu\\\":\"; append_u32(os, target_spu);` "
                "right after `start_event(os, seq, true, \"<kind>\");`."
            )

    return violations


def check_spu_image_api_wiring(scaffolding_text: str, runtime_text: str) -> list[str]:
    """R5.9e.3 invariant 8: scaffolding patch MUST declare + implement
    `record_spu_image`, AND runtime hooks patch MUST contain at least
    one call site invoking it. Either side missing means the writer/
    runtime contract is half-wired — easy to introduce by accident
    when re-touching only one of the two patches.
    """
    violations: list[str] = []

    # The scaffolding patch should contain BOTH the declaration (in
    # the .h section) and the definition (in the .cpp section). We
    # use the declaration form `void record_spu_image(` as the
    # canonical anchor — appears in `.h` once and `.cpp` once.
    scaffolding_added_lines = "\n".join(
        l[1:] for l in scaffolding_text.splitlines()
        if l.startswith("+") and not l.startswith("+++")
    )
    if "void record_spu_image(" not in scaffolding_added_lines:
        violations.append(
            "R5.9e.3 wiring regression: scaffolding patch does NOT declare "
            "`void record_spu_image(...)`. The writer-side API for SPU image "
            "capture is missing; replay engine cannot resolve side-files. "
            "Re-add `record_spu_image` to `SPUTraceJsonl.h`/`.cpp`."
        )
    elif "void TraceWriter::record_spu_image(" not in scaffolding_added_lines:
        violations.append(
            "R5.9e.3 wiring regression: scaffolding patch declares "
            "`record_spu_image` but does NOT define it (no "
            "`void TraceWriter::record_spu_image(` body). Implementation "
            "is missing — the runtime call would fail to link."
        )

    # The runtime hooks patch should call record_spu_image somewhere.
    # We look for `record_spu_image(` in the added (`+`) lines.
    runtime_added_lines = "\n".join(
        l[1:] for l in runtime_text.splitlines()
        if l.startswith("+") and not l.startswith("+++")
    )
    if "record_spu_image(" not in runtime_added_lines:
        violations.append(
            "R5.9e.3 wiring regression: runtime hooks patch does NOT call "
            "`record_spu_image(...)`. The writer would compile, but no SPU "
            "image events would be emitted. Add a call site in `cpu_task` "
            "(or equivalent thread-entry hook) that invokes "
            "`tracer.record_spu_image(lv2_id, ls, SPU_LS_SIZE, "
            "/*load_addr=*/0u, /*entry_pc=*/pc);`."
        )

    return violations


def main() -> int:
    violations: list[str] = []

    for p in (SCAFFOLDING, RUNTIME_HOOKS):
        if not p.is_file():
            violations.append(f"missing patch: {p.relative_to(REPO_ROOT)}")

    if violations:
        for v in violations:
            print(f"  - {v}", file=sys.stderr)
        return 1

    scaffolding_text = SCAFFOLDING.read_text(encoding="utf-8", errors="replace")
    runtime_text = RUNTIME_HOOKS.read_text(encoding="utf-8", errors="replace")

    if sha256(SCAFFOLDING) == sha256(RUNTIME_HOOKS):
        violations.append("scaffolding and runtime hooks patches have identical sha256 — they MUST be separate files")

    # Scaffolding must not touch hot-path files.
    for hp in HOT_PATH_FILES:
        if patch_touches(scaffolding_text, hp):
            violations.append(
                f"scaffolding patch touches hot-path file '{hp}'. "
                "Hot-path edits belong to runtime hooks patch."
            )

    # Runtime hooks must not touch scaffolding-only files.
    for sf in SCAFFOLDING_FILES:
        if patch_touches(runtime_text, sf):
            violations.append(
                f"runtime hooks patch touches scaffolding-only file '{sf}'. "
                "Writer / build wiring edits belong to scaffolding patch. "
                "If a bug in the writer requires a fix, regenerate the "
                "scaffolding patch — do NOT mix into runtime hooks."
            )

    violations.extend(check_writer_race_guard(scaffolding_text))
    violations.extend(check_spu_image_api_wiring(scaffolding_text, runtime_text))

    # R6.1 — optional sha256 pin on the Rust bridge patch. Absent file
    # is fine (pre-R6.1 state); present file with mismatched sha is a
    # violation (= silent drift).
    rust_bridge_sha = None
    if RUST_BRIDGE.is_file():
        rust_bridge_sha = sha256(RUST_BRIDGE)
        if rust_bridge_sha != RUST_BRIDGE_PINNED_SHA256:
            violations.append(
                f"R6.1 sha drift: spu_rust_bridge.patch sha256 = {rust_bridge_sha} "
                f"but pinned = {RUST_BRIDGE_PINNED_SHA256}. Regenerate the patch "
                "(see docs/SPU_RUST_BRIDGE_PATCH.md) and update RUST_BRIDGE_PINNED_SHA256."
            )

    if violations:
        print(f"FAIL: {len(violations)} violation(s)", file=sys.stderr)
        for v in violations:
            print(f"  - {v}", file=sys.stderr)
        return 1

    print("OK: patch separation + writer race guards satisfied")
    print(f"  - scaffolding sha256: {sha256(SCAFFOLDING)}")
    print(f"  - runtime hooks sha256: {sha256(RUNTIME_HOOKS)}")
    if rust_bridge_sha is not None:
        print(f"  - rust bridge sha256: {rust_bridge_sha}")
    else:
        print("  - rust bridge patch: not present (pre-R6.1 state, OK)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
