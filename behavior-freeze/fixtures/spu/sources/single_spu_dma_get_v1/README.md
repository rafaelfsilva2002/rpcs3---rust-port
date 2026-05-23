# single_spu_dma_get_v1

R6.7 A.5 first replay-validated DMA GET fixture. Single-SPU PSL1GHT
homebrew that exercises the full MFC GET dispatch path (ch16-25)
with a deterministic CC0 input/output pair.

## Current status

**REPLAY-VALIDATED — 7th oracle (R6.7 A.5 LANDED 2026-05-03).**

Built artifacts now present + committed:

- `build/single_spu_dma_get_v1.self` (real PSL1GHT build via the
  `rpcs3-ps3dev-toolchain:local` Docker image)
- Trace JSONL in
  `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl`
- `.dmachunk` side-file in `behavior-freeze/fixtures/spu/dma/`
- `.spuimg` side-file in `behavior-freeze/fixtures/spu/images/`
- `.notes.md` companion documenting provenance + engine fixes

Replay test `single_spu_dma_get_v1_replay` (formerly `#[ignore]`d)
is enabled and green; the fixture is part of the R6 closure gate
set and runs byte-identical across `InterpreterExecutor` and
`RecompilerExecutor`. The runtime bridge delegates this fixture
end-to-end (R7.2 — see `docs/PROJECT_STATUS.md` § 9.1 +
`docs/SPU_DMA_MFC_R6_7_DESIGN.md` § 19 closure).

The build-and-capture procedure below remains useful for re-capture
or for authoring new DMA fixtures; the explicit "BLOCKED" status
header that lived here at A.5 authoring time is preserved as
historical context.

## Status (historical pre-landing snapshot — 2026-05-02)

**A.5 status as of 2026-05-02: BLOCKED on real-binary capture.**

Source files (`main.c`, `spu/spu_dma_get.c`, `Makefile`,
`LICENSE.md`, this `README.md`) are committed. The build artifacts
(`build/single_spu_dma_get_v1.self`, captured `.jsonl` trace,
`.dmachunk` side-file, `.spuimg` LS image, `.notes.md`) are NOT
committed yet because they require:

1. A working PSL1GHT/ps3toolchain environment (the
   `rpcs3-ps3dev-toolchain:local` Docker image, ~2.43 GB —
   built once via R6.4b-toolchain).
2. A working RPCS3 build (`R:\bin\rpcs3.exe` from the R6.5b
   build, ~64 MB).

When both are available, the build + capture + replay-test
pipeline below produces the missing artifacts. **NO synthetic /
hand-edited JSONL or `.dmachunk` will be committed under any
circumstance** — they must come from a real RPCS3 capture
(R6.7 A.1 writer extension) of this fixture's actual `.self` to
qualify as an oracle.

## Behaviour

PPU side:
1. Allocates a 128-byte buffer (`ea_buf`) in BSS, aligned to a
   128-byte boundary.
2. Fills the buffer with a counting pattern: `ea_buf[i] = i & 0xFF`
   for `i ∈ [0, 128)`.
3. Passes the buffer's effective address to the SPU via
   `sysSpuThreadArgument.arg[0]` (which lv2 plumbs to the SPU's
   `int main(uint64_t spu_id, uint64_t arg)` second parameter).
4. Calls `sysSpuThreadGroupStart` and `sysSpuThreadGroupJoin`,
   prints the result.

SPU side:
1. Receives the EA in the `arg` parameter (low 32 bits).
2. Runs the canonical MFC GET sequence:
   - `wrch ch16, 0x10000`              (LSA — destination in LS)
   - `wrch ch17, 0`                    (EAH — always 0 in PSL1GHT)
   - `wrch ch18, ea`                   (EAL — runtime-supplied)
   - `wrch ch19, 128`                  (Size)
   - `wrch ch20, 3`                    (TagID)
   - `wrch ch21, 0x40`                 (MFC_Cmd = GET)
   - `wrch ch22, 0x8`                  (WrTagMask = 1<<3)
   - `wrch ch23, 2`                    (WrTagUpdate = ALL)
   - `rdch ch24`                       (RdTagStat — returns 0x8 once GET completes)
3. Reads the 128 GET'd bytes from LS at `[0x10000..0x10080]`.
4. Sums the bytes (deterministic), XORs with `0xDEADBEEF`.
5. Writes the result to `OUT_MBOX` (ch28), halts via `stop 0x101`.

## Canonical computation

The fixture is fully deterministic. Inputs:

- PPU buffer: `buf[i] = i & 0xFF` for `i ∈ [0, 128)`.

The sum-of-bytes is `0 + 1 + 2 + ... + 127 = 128*127/2 = 8128 =
0x1FC0`. The XOR mix is:

```python
sum_of_buf = sum(i & 0xFF for i in range(128))     # 8128 == 0x1FC0
status     = sum_of_buf ^ 0xDEADBEEF                # 0xDEADA12F
```

So `OUT_MBOX = 0xDEADA12F`, and the joined `status` reported by
lv2's group-exit handler equals this value.

**Expected TTY output (RPCS3 OFF, no trace):**

```
[dma_get_v1] OK cause=0x1 status=0xdeada12f
```

## Why this fixture is the load-bearing R6.7 oracle

Status `0xDEADA12F` is **only** reachable when:

1. The MFC GET dispatch ACTUALLY copied the EA bytes into LS
   (any silent fake-DMA path that zero-fills LS produces `0 ^
   0xDEADBEEF = 0xDEADBEEF`, a different status).
2. The SPU computed the deterministic post-DMA sum + XOR.
3. The OUT_MBOX wrch + stop 0x101 fired in order.

Each of these is a separate bridge / replay path under test. A
single bit-flip in any of them changes the status.

## Build (when R:/ + Docker available)

From the project root, with the `rpcs3-ps3dev-toolchain:local`
Docker image present:

```bash
docker run --rm -v "$PWD":/work \
  -w /work/behavior-freeze/fixtures/spu/sources/single_spu_dma_get_v1 \
  rpcs3-ps3dev-toolchain:local \
  bash -lc 'make clean && make V=1'
```

Output: `single_spu_dma_get_v1.self` in this directory. Move to
`build/single_spu_dma_get_v1.self` once verified.

## Capture trace (when R:\bin\rpcs3.exe available)

After RPCS3 OFF reproduces the canonical TTY, capture with R6.7
writer extension active:

```cmd
set RPCS3_SPU_TRACE_JSONL=behavior-freeze\fixtures\spu\traces\single_spu_dma_get_v1.jsonl
R:\bin\rpcs3.exe --no-gui ^
   behavior-freeze\fixtures\spu\sources\single_spu_dma_get_v1\build\single_spu_dma_get_v1.self
```

The capture produces:

- `behavior-freeze/fixtures/spu/traces/single_spu_dma_get_v1.jsonl` — JSONL events (~15-20 events expected).
- `<jsonl>.dma/<sha>.dmachunk` — the captured EA bytes (128 bytes,
  must hash to the SHA-256 referenced in the JSONL `spu_mfc_cmd`
  event).
- `<jsonl>.images/<sha>.spuimg` — full SPU LS at thread-create.

## Move artifacts to canonical locations

```bash
# .spuimg → behavior-freeze/fixtures/spu/images/<sha>.spuimg
mv <jsonl>.images/*.spuimg behavior-freeze/fixtures/spu/images/

# .dmachunk → behavior-freeze/fixtures/spu/dma/<sha>.dmachunk (NEW)
mkdir -p behavior-freeze/fixtures/spu/dma
mv <jsonl>.dma/*.dmachunk behavior-freeze/fixtures/spu/dma/

# .self into build/ subdir of this fixture source
mkdir -p build
mv ../single_spu_dma_get_v1.self build/
```

## Replay test (currently `#[ignore]`)

`rust/rpcs3-spu-recompiler/tests/single_spu_dma_get_v1_replay.rs`
mirrors the 6 existing oracle replay tests:

- Parses the JSONL via `parse_jsonl_trace`.
- Builds the SPU program from `.spuimg` via
  `build_spu_program_from_captured_image`.
- Runs `apply_mfc_dma_pre_replay` to inject DMA into LS + populate
  the rdch ch24 queue (uses A.3 loader + A.4 state machine + C.3
  helper from R6.7).
- Replays through Interpreter AND Recompiler via
  `replay_per_spu_traces_with`.
- Asserts:
  - `Finished{stop_code: 0x101}` on both backends.
  - `diff_snapshots(interp, jit).is_identical() == true`.
  - The SPU's OUT_MBOX (drained as group-exit status) equals
    `0xDEADA12F`.
  - `mfc_cmd_events.len() == 1` (exactly one `spu_mfc_cmd` event,
    matching cmd=0x40, tag=3, size=128).

The test is `#[ignore]`d until the JSONL trace + `.dmachunk` land
in their canonical locations. **Once the artifacts are committed
the `#[ignore]` line must be removed** (single-line edit).

## Hard limits

- **GET only** (cmd 0x40). No PUT, list, atomic, barrier-flagged.
- **EAH = 0** (PSL1GHT 32-bit user space).
- **128-byte transfer** (16-byte aligned, well within the 16 KiB
  R6.7 cap).
- **Single tag** (tag=3). No multi-tag fan-in.
- **Single-SPU thread group**.
- **Zero PPU↔SPU mailbox / signal traffic** (just the GET +
  OUT_MBOX exit path). Distinct from `single_spu_mailbox_v1` /
  `single_spu_signal_v1` etc. — this is the pure-DMA oracle.
