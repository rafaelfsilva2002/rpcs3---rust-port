#!/usr/bin/env python3
"""One-shot helper: bring R:'s rpcs3/ tree from pre-R5.9c (scaffolding v2 +
runtime hooks v1) to R5.9c (scaffolding v3 + runtime hooks v2) so the
build .bat picks up the writer changes.

Idempotent: if R: already has R5.9c content, the script is a no-op.

Two parts:
  1. Copy SPUTraceJsonl.{h,cpp} from C: tree to R: tree (R5.9c writer).
  2. Surgical edits to R:'s SPUThread.cpp:
     - Add `const u32 trace_target_spu   = lv2_id;` snapshot in get_ch_value.
     - Add `const u32 trace_target_spu_w = lv2_id;` snapshot in
       set_ch_value SPU_WrOutMbox.
     - Prepend `lv2_id, ` / `spu->lv2_id, ` / `trace_target_spu, ` /
       `trace_target_spu_w, ` to each `record_spu_*` and
       `record_final_state` call so the R5.9c writer signature is
       satisfied.

Run from the repo root:
  python behavior-freeze/harness/apply_r59c_to_R_drive.py
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path


C_ROOT = Path(__file__).resolve().parent.parent.parent
R_ROOT = Path("R:/")

C_HEADER = C_ROOT / "rpcs3" / "Emu" / "Cell" / "SPUTraceJsonl.h"
C_IMPL = C_ROOT / "rpcs3" / "Emu" / "Cell" / "SPUTraceJsonl.cpp"
R_HEADER = R_ROOT / "rpcs3" / "Emu" / "Cell" / "SPUTraceJsonl.h"
R_IMPL = R_ROOT / "rpcs3" / "Emu" / "Cell" / "SPUTraceJsonl.cpp"
R_SPU_THREAD = R_ROOT / "rpcs3" / "Emu" / "Cell" / "SPUThread.cpp"

# Each entry is (old, new). Idempotent: if `old` is absent (because
# `new` is already in place), the substitution is skipped.
SPU_THREAD_EDITS = [
    # 1. record_final_state in TraceFinalGuard (cpu_task)
    (
        "tracer.record_final_state(gprs, ch);",
        "tracer.record_final_state(spu->lv2_id, gprs, ch);",
    ),
    # 2. record_spu_rchcnt in get_ch_count
    (
        "tracer.record_spu_rchcnt(pc, ch, result);",
        "tracer.record_spu_rchcnt(lv2_id, pc, ch, result);",
    ),
    # 3a. Insert trace_target_spu snapshot in get_ch_value (after trace_pc)
    (
        "\tauto& trace_tracer = rpcs3::spu_trace::TraceWriter::instance();\n"
        "\tconst u32 trace_pc = pc;\n"
        "\tbool trace_did_stall = false;",
        "\tauto& trace_tracer = rpcs3::spu_trace::TraceWriter::instance();\n"
        "\tconst u32 trace_pc = pc;\n"
        "\tconst u32 trace_target_spu = lv2_id;\n"
        "\tbool trace_did_stall = false;",
    ),
    # 3b. record_spu_rdch entry-stall
    (
        "trace_tracer.record_spu_rdch(trace_pc, ch, std::nullopt, /*would_stall=*/true);",
        "trace_tracer.record_spu_rdch(trace_target_spu, trace_pc, ch, std::nullopt, /*would_stall=*/true);",
    ),
    # 3c. record_spu_park (read)
    (
        "trace_tracer.record_spu_park(trace_pc,\n"
        "\t\t\t\t\trpcs3::spu_trace::ParkReason::ChannelRead, ch, std::nullopt);",
        "trace_tracer.record_spu_park(trace_target_spu, trace_pc,\n"
        "\t\t\t\t\trpcs3::spu_trace::ParkReason::ChannelRead, ch, std::nullopt);",
    ),
    # 3d. record_spu_wake (read)
    (
        "trace_tracer.record_spu_wake(trace_pc);",
        "trace_tracer.record_spu_wake(trace_target_spu, trace_pc);",
    ),
    # 3e. record_spu_rdch post-pop
    (
        "trace_tracer.record_spu_rdch(trace_pc, ch,\n"
        "\t\t\t\tstatic_cast<u32>(out), /*would_stall=*/false);",
        "trace_tracer.record_spu_rdch(trace_target_spu, trace_pc, ch,\n"
        "\t\t\t\tstatic_cast<u32>(out), /*would_stall=*/false);",
    ),
    # 4a. Insert trace_target_spu_w snapshot in set_ch_value SPU_WrOutMbox
    (
        "\t\tauto& trace_tracer_w = rpcs3::spu_trace::TraceWriter::instance();\n"
        "\t\tconst u32 trace_pc_w = pc;\n"
        "\t\tconst bool trace_would_stall_w = ch_out_mbox.get_count() != 0;",
        "\t\tauto& trace_tracer_w = rpcs3::spu_trace::TraceWriter::instance();\n"
        "\t\tconst u32 trace_pc_w = pc;\n"
        "\t\tconst u32 trace_target_spu_w = lv2_id;\n"
        "\t\tconst bool trace_would_stall_w = ch_out_mbox.get_count() != 0;",
    ),
    # 4b. record_spu_wrch entry
    (
        "trace_tracer_w.record_spu_wrch(trace_pc_w, ch, value, trace_would_stall_w);",
        "trace_tracer_w.record_spu_wrch(trace_target_spu_w, trace_pc_w, ch, value, trace_would_stall_w);",
    ),
    # 4c. record_spu_park (write)
    (
        "trace_tracer_w.record_spu_park(trace_pc_w,\n"
        "\t\t\t\t\trpcs3::spu_trace::ParkReason::ChannelWrite, ch, std::nullopt);",
        "trace_tracer_w.record_spu_park(trace_target_spu_w, trace_pc_w,\n"
        "\t\t\t\t\trpcs3::spu_trace::ParkReason::ChannelWrite, ch, std::nullopt);",
    ),
    # 5a. record_spu_wake (write)
    (
        "trace_tracer_w.record_spu_wake(trace_pc_w);",
        "trace_tracer_w.record_spu_wake(trace_target_spu_w, trace_pc_w);",
    ),
    # 5b. record_spu_wrch post-commit
    (
        "trace_tracer_w.record_spu_wrch(trace_pc_w, ch, value, /*would_stall=*/false);",
        "trace_tracer_w.record_spu_wrch(trace_target_spu_w, trace_pc_w, ch, value, /*would_stall=*/false);",
    ),
    # 6. record_spu_stop in stop_and_signal
    (
        "tracer.record_spu_stop(pc, code);",
        "tracer.record_spu_stop(lv2_id, pc, code);",
    ),
    # R5.9e.3 — Insert the spu_image hook in cpu_task right after
    # `pc &= 0x3fffc;` (which strips the status bit so entry_pc is the
    # raw SPU instruction address). The hook calls record_spu_image
    # which captures LS bytes, computes SHA-256, writes the side-file
    # at <trace>.images/<sha>.spuimg, and emits the JSONL spu_image
    # event. Writer dedupes per target_spu so re-entered cpu_task
    # (pause/resume) is a no-op. Idempotent: skipped if already in
    # place.
    (
        "\tpc &= 0x3fffc;\n\n\tstd::fesetround(FE_TOWARDZERO);",
        "\tpc &= 0x3fffc;\n\n"
        "\t// R5.9e.3 trace hook: emit spu_image once per SPU thread at thread entry,\n"
        "\t// AFTER pc &= 0x3fffc strips the status/interrupt bit so entry_pc is the\n"
        "\t// raw instruction address. Writer dedupes per target_spu so re-entries\n"
        "\t// of cpu_task (pause/resume) are no-ops; the SPU's image is captured\n"
        "\t// exactly once per trace session.\n"
        "\t{\n"
        "\t\tauto& tracer = rpcs3::spu_trace::TraceWriter::instance();\n"
        "\t\tif (tracer.enabled())\n"
        "\t\t{\n"
        "\t\t\ttracer.record_spu_image(lv2_id, ls, SPU_LS_SIZE,\n"
        "\t\t\t\t/*load_addr=*/0u, /*entry_pc=*/pc);\n"
        "\t\t}\n"
        "\t}\n\n"
        "\tstd::fesetround(FE_TOWARDZERO);",
    ),
]


def main() -> int:
    if not R_ROOT.exists():
        print(f"ERROR: R: drive root not found at {R_ROOT}", file=sys.stderr)
        return 1

    print("Step 1: copy SPUTraceJsonl.{h,cpp} from C: tree to R: tree")
    shutil.copy2(C_HEADER, R_HEADER)
    shutil.copy2(C_IMPL, R_IMPL)
    print(f"  - copied {C_HEADER.name} ({C_HEADER.stat().st_size} bytes)")
    print(f"  - copied {C_IMPL.name} ({C_IMPL.stat().st_size} bytes)")

    print("Step 2: surgical edits to R:'s SPUThread.cpp")
    text = R_SPU_THREAD.read_text(encoding="utf-8")
    applied = 0
    skipped = 0
    for old, new in SPU_THREAD_EDITS:
        if new in text:
            skipped += 1
            continue
        if old not in text:
            print(
                f"  ERROR: anchor not found for edit:\n"
                f"  --- old (first 80 chars):\n  {old[:80]}\n"
                f"  --- new (first 80 chars):\n  {new[:80]}",
                file=sys.stderr,
            )
            return 2
        text = text.replace(old, new, 1)
        applied += 1

    R_SPU_THREAD.write_text(text, encoding="utf-8", newline="\n")
    print(f"  - applied {applied} edit(s), skipped {skipped} (already R5.9c)")

    # Quick sanity: every record_spu_*/record_final_state call must
    # contain `lv2_id` (or a `trace_target_spu*` derived from it).
    bad = []
    for line in text.splitlines():
        s = line.strip()
        if not (
            s.startswith("tracer.record_spu_")
            or s.startswith("trace_tracer.record_spu_")
            or s.startswith("trace_tracer_w.record_spu_")
            or s.startswith("tracer.record_final_state(")
        ):
            continue
        if (
            "lv2_id" not in line
            and "trace_target_spu" not in line
            and "spu->lv2_id" not in line
        ):
            bad.append(line)
    if bad:
        print("  ERROR: post-edit, the following SPU-side call sites lack target_spu:", file=sys.stderr)
        for b in bad:
            print(f"    {b}", file=sys.stderr)
        return 3

    print("OK: R: drive synced to R5.9c")
    return 0


if __name__ == "__main__":
    sys.exit(main())
