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

## Validation status — command/state layer CLOSED 2026-05-27

| Slice | Commit | Crate | Scope |
|---|---|---|---|
| R12.1 | `820e3a650` | new `rpcs3-rsx-fifo` | FIFO command decoder (header → method writes / jump / call / return / nop) |
| R12.2 | `0d9ab06f2` | `rpcs3-rsx-fifo` | FifoEngine — DMA control PUT/GET + call stack + run-until-PUT |
| R12.3 | `6f0bd593a` | new `rpcs3-rsx-state` | method register file `[u32;0x4000]` + FIFO-write apply + typed accessors |
| R12.4 | `276e52a2f` | `rpcs3-rsx-state` | method-group classify + MethodEffect (semaphore/clear/begin-end) |
| R12.5 | `0f17214e5` | `rpcs3-rsx-state` | DrawTracker — BEGIN/END + DRAW_ARRAYS/INDEX → DrawCall |

**Result:** the RSX command/state pipeline is complete and pure:
GCM command stream (BE bytes) → `FifoEngine::run` → `(reg,arg)`
writes → `RsxState` register file + `MethodEffect` control events +
`DrawTracker` draw calls. ~36 inline tests across the two new
crates; workspace gate 270 result blocks, 0 fail. Fully testable
without a GPU — the natural shape of a GCM-stream replay oracle.

## Validation status — Camada B (resource descriptors) CLOSED 2026-05-27

| Slice | Commit | Scope |
|---|---|---|
| R12.6 | `a026382a6` | vertex attribute format parsing — `decode_vertex_format`, `VertexAttribute`, `RsxState::vertex_attribute` |
| R12.7 | `93b6eb725` | index buffer descriptor — `IndexType`, `IndexArray`, `RsxState::index_array` |
| R12.8 | `abbc94ffa` | texture descriptor (parse only) — `TextureDescriptor`, `RsxState::texture`/`texture_enabled` |
| R12.9 | `c66df45b6` | surface / render-target descriptor — `SurfaceTargets`, `SurfaceDescriptor`, `RsxState::surface` |

**Result:** the `RsxState` register file now decodes into every
structured resource a draw references — vertex attributes, index
buffer, textures (16 units), surfaces (MRT + zeta). All pure
register-word transforms, `None`/empty for disabled state, each
with FIFO-pipeline integration tests. 39 `rpcs3-rsx-state` lib
tests; workspace gate 270, 0 fail. The descriptors are the input
a future GPU backend (Camada E) would consume, and are themselves
behavior-freezable from a captured GCM stream.

Texture *format classification* and *pixel decode* (vs the
structural parse here) remain in Camada D.

## Validation status — GCM replay oracles 2026-05-27

Precise nomenclature (provenance tiers):

| Slice | Commit | Name (precise) |
|---|---|---|
| R12.10a | `7e8b4cd9d` | **authored golden stream oracle** (Tier 1 — hand-authored hex) |
| R12.10b | `b57839b75` | **GCM command builder + emit/decode round-trip oracle** (Tier 2 — emitted stream; producer-side. NOT "captured") |
| R12.11 | (in progress) | **real captured stream infrastructure** (Tier 3) |

`rpcs3-rsx-gcm::GcmContext` (Tier 2) emits a stream byte-identical to
the R12.10a golden; both decode through `replay_gcm` → `RsxSnapshot`.
Three RSX crates form a deterministic triangle:
`rpcs3-rsx-gcm` (produce) → bytes → `rpcs3-rsx-fifo` (decode) →
`rpcs3-rsx-state` (`replay_gcm` → `RsxSnapshot`).

## R12.11 — real captured stream infrastructure (Tier 3)

Goal: get GCM bytes produced by a real PSL1GHT fixture's libgcm
(not authored, not our emitter), via a minimal cellGcm HLE rather
than a C++ capture writer — we already have the Rust producer
(GcmContext) and decoder, so the cellGcm path is the natural one.

Success criterion: PSL1GHT fixture calls cellGcm-like API →
EmuCore cellGcm HLE produces/exposes the command buffer → emitted
stream → `replay_gcm` → expected `RsxSnapshot`.

Sub-slices:
- **R12.11a** (in-Rust, no Docker): the capture *mechanism* — a
  `GcmControl` command-buffer model (base / PUT / GET / IO offset)
  + a `capture_command_buffer(mem_image, control)` path that reads
  `[base+GET .. base+PUT)` out of a memory image. Validated by
  driving `GcmContext` writes into a mock memory region, setting
  PUT, capturing, and replaying → the exact same snapshot. This is
  the byte-snapshot logic the real fixture capture reuses.
- **R12.11b** (needs Docker + EmuCore run): a CC0 PSL1GHT fixture
  using libgcm (cellGcmInit + clear + flush); EmuCore wires
  cellGcmInit (command-buffer region) + reads the buffer the
  homebrew's inline libgcm wrote, via R12.11a's capture path; feed
  `replay_gcm` → assert.

Note: PSL1GHT libgcm command emission is mostly *inline* (writes
words directly into the context's command buffer in guest memory),
so capture = read `[GET..PUT)` from memory after flush — NOT
per-command HLE interception. The few real PRX calls (cellGcmInit,
flip, control register) are the HLE surface.

## Deferred — the GPU-backend giant tail (out of near-term scope)

These need an actual GPU backend and are months of work; they do
NOT fit the byte-exact behavior-freeze model (rendering varies by
hardware/driver):
- Vertex/fragment shader decompilation (→ GLSL/SPIR-V). Scaffolds
  exist (`rsx-gl-decompiler`, `rsx-vk-decompiler`).
- Texture decode (swizzled/compressed formats).
- Surface / render-target / framebuffer management
  (`rsx-surface-store` scaffold).
- Vulkan / OpenGL backend (actual rendering).
- Display / VBlank / flip.

A future direction could capture GCM command streams from real
homebrew (via the existing Docker pipeline) and promote the
decode→state→drawcall pipeline to replay oracles — the tractable
behavior-freeze target — while the rendering backend remains a
separate, large undertaking.
