# LICENSE — CC0 1.0 Universal (Public Domain Dedication)

The author dedicates this work to the public domain under the
[CC0 1.0 Universal](https://creativecommons.org/publicdomain/zero/1.0/)
Public Domain Dedication.

To the extent possible under law, the author has waived all copyright
and related or neighboring rights to this work. This work is published
from: Brazil.

You can copy, modify, distribute, and perform the work, even for
commercial purposes, all without asking permission.

---

This fixture (`single_spu_branch_loop_v1`) is part of the
RPCS3 → Rust port behavior-freeze test suite.

## Files covered

- `main.c` — PPU-side loader.
- `spu/spu_branch_loop.c` — SPU-side compute kernel.
- `Makefile` — build script.
- `capture_trace.cmd` / `enable_autoexit_and_capture.cmd` — RPCS3 capture
  helpers (Windows `.cmd` wrappers; not load-bearing for replay).
- `notes_template.md` — internal scratch; the canonical provenance
  doc for the trace lives at
  `behavior-freeze/fixtures/spu/traces/single_spu_branch_loop_v1.notes.md`.
- `README.md` — fixture overview + build instructions.

## Provenance

Authored 2026-04-29 by the RPCS3-Rust-port project for R5.11
(oracle suite expansion, post-R5 closure).

The captured trace + `.spuimg` side-file + replay-test live under
`behavior-freeze/fixtures/spu/{traces,images}/` and
`rust/rpcs3-spu-recompiler/tests/`; they are derivative works of
this source under the same CC0 dedication.
