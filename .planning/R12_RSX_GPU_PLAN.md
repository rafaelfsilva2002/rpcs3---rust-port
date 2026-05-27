# R12 — RSX / GPU subsystem

**Status:** PLAN + in-progress (2026-05-27).
**Predecessor:** R11 (PPU interpreter) closed.
**Honest scope note:** RSX is the single largest RPCS3 subsystem.
A full byte-exact port (command processor + ~hundreds of NV4097
method handlers + vertex/fragment shader decompilation + texture
decode + framebuffer management + a Vulkan/GL backend) is a
multi-month, multi-thousand-LOC effort that cannot complete in
one session. R12 builds it the project way: start with the
tractable, behavior-freezable foundation and slice upward,
gating each commit. The GPU backend (actual rendering) is the
far end and is explicitly out of near-term scope — we target the
**command-processing + state layers** that are pure, testable,
and unblock everything above them.

## What already exists (scaffolds, audited 2026-05-27)

- `rpcs3-rsx-gl-common` (72), `gl-decompiler`, `vk-decompiler`,
  `gsframe` (79), `surface-store` (209), `vertex-data` (178),
  `texture-cache-types` (246) — type definitions / partial.
- `rpcs3-hle-cellgcm` (658), `cellgcmsys` (222) — HLE PRX
  contract dispatchers, NOT a real command ring.
- **MISSING: the RSX command processor core** — FIFO parser,
  method register file, method dispatch. R12 builds this.

## Wave sequence (command/state layers; backend deferred)

| Slice | Crate / area | Scope |
|---|---|---|
| R12.1 | new `rpcs3-rsx-fifo` | GCM FIFO command decoder — walk the command buffer, decode headers (increment / non-increment method, JUMP/CALL/RET/NOP), emit (method, arg) sequence |
| R12.2 | `rpcs3-rsx-fifo` | DMA control model (PUT/GET pointers) + run-until-PUT loop |
| R12.3 | new `rpcs3-rsx-state` | RSX method register file (the ~0x10000/4 method address space) + typed accessors for the common register groups |
| R12.4 | `rpcs3-rsx-state` | method dispatch skeleton — route decoded (method,arg) into the register file + recognize method groups (NV4097 set-state, NV0039 buffer-copy, etc.) |
| R12.5 | state | draw-command recognition (BEGIN/END, draw-arrays/draw-index) → emit a structured DrawCall record (no rendering, just the captured intent) |
| R12.6+ | (deferred) | vertex/fragment program decode, texture decode, surface/framebuffer, GPU backend — the giant tail |

## Conventions

- One slice per commit. Each: code + tests + canonical gate
  (`cargo test --workspace --tests --release`, ≥ current block
  count, 0 fail).
- New crates added to `rust/Cargo.toml` workspace members.
- Behavior-freeze: the FIFO decoder + state layer are pure
  functions over a command-buffer byte array — directly testable
  without a GPU, and a natural fit for capture/replay oracles
  later (a captured GCM command stream → expected method writes).

## FIFO command encoding reference (NV / RSX)

Command words are u32 big-endian in the ring buffer. Decode of a
header word `cmd`:
- `(cmd & 0xe0030003) == 0x00000000 && count != 0` → increment
  method: count = (cmd>>18)&0x7ff, method = cmd & 0x3ffc, then
  `count` args follow; method address advances by 4 per arg.
- `(cmd & 0xe0030003) == 0x40000000` → non-increment method:
  same count/method, but all args go to the same method.
- `(cmd & 0xe0000003) == 0x20000000` → OLD JUMP: GET = cmd & 0x1ffffffc.
- `(cmd & 0xe0000003) == 0x00000002` → CALL: push GET+4, GET = cmd & 0x1ffffffc.
- `cmd == 0x00020000` → RETURN: GET = call-stack pop.
- `(cmd & 0x60000000) == 0x60000000` (or count==0 sentinel) → NOP.

(Exact masks cross-checked against RPCS3 `rsx::FIFO::fifo_engine`
during R12.1.)

## Validation status

- R12.1: (in progress)
