# single_spu_loadstore_v1

R5.11b — fourth replay-validated SPU oracle fixture (post-R5
closure, post-R5.11). Exercises the **SPU Local Store load/store
path** (lqd/stqd against r1-relative offsets) on top of the same
race-free single-round IN_MBOX→OUT_MBOX→stop-0x101 shape used by
`single_spu_mailbox_v1`, `single_spu_branch_loop_v1`, and
`single_spu_signal_v1`.

## What it covers

- IN_MBOX read (ch29 / `rdch SPU_RdInMbox`) — same as the mailbox
  fixtures.
- **Stack-allocated `volatile uint32_t buffer[8]` writeback to LS**
  (forces stqd at r1-relative offsets).
- **LS read-back into registers** (forces lqd).
- 8-word sum-checksum.
- OUT_MBOX write (ch28 / `wrch SPU_WrOutMbox`).
- `stop 0x101` (`SYS_SPU_THREAD_STOP_GROUP_EXIT`); lv2 reads
  OUT_MBOX as the group-exit status.

The `volatile` qualifier is load-bearing — without it, GCC -O2
keeps the values in registers across both loops and skips LS
access entirely, defeating the fixture's purpose.

## Build

Same workflow as sibling R5.11 fixtures (Docker container
`ps3-build` with from-source `ps3toolchain`):

```bash
docker cp single_spu_loadstore_v1 ps3-build:/tmp/
docker exec ps3-build bash -c \
  'cd /tmp/single_spu_loadstore_v1 && \
   PS3DEV=/opt/ps3dev PSL1GHT=/opt/ps3dev/psl1ght \
   PATH=$PS3DEV/bin:$PS3DEV/ppu/bin:$PS3DEV/spu/bin:$PATH \
   make'
docker cp ps3-build:/tmp/single_spu_loadstore_v1/single_spu_loadstore_v1.self build/
```

## Capture

Same workflow as sibling fixtures:

```bash
RPCS3_SPU_TRACE_JSONL=/tmp/single_spu_loadstore_v1.jsonl \
  /r/bin/rpcs3.exe --headless \
  /path/to/build/single_spu_loadstore_v1.self
```

Or double-click `enable_autoexit_and_capture.cmd` from Windows Explorer.

## Provenance

Authored 2026-04-29. CC0 1.0 (public domain). See LICENSE.md.

The `build/` subdir (containing the compiled `.self` and `.elf`)
is gitignored per the project's reproducible-build policy —
re-running `make` from these sources produces equivalent
artifacts. The replay-validated trace + `.spuimg` side-file at
`behavior-freeze/fixtures/spu/{traces,images}/` are the
load-bearing artifacts and ARE tracked.
