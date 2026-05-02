# single_spu_loadstore_v1.notes.md

R5.11b — fourth replay-validated SPU oracle fixture (post-R5
closure, post-R5.11 oracle suite expansion). Captured 2026-04-29
from RPCS3 against a CC0 PSL1GHT homebrew authored for this
purpose, then replayed end-to-end with byte-identical agreement
on the final SpuStateSnapshot across InterpreterExecutor and
RecompilerExecutor.

This fixture is the **first replay-validated trace exercising the
SPU Local Store load/store path** — `stqd`/`lqd` with displacement
addressing relative to r1, and the standard Cell BE
quadword-of-word-insert/extract pattern (`cwd`/`shufb`/`stqd` for
stores, `lqd`/`rotqby` for loads). It exposed three real general
bugs in the Rust SPU stack that had been latent because the
synthetic-fixture suite never exercised these patterns end-to-end.

## Origem do homebrew

Autoral. CC0 1.0 (public domain). Source committed at
`behavior-freeze/fixtures/spu/sources/single_spu_loadstore_v1/`
with LICENSE.md.

Comportamento (uma linha): PPU pushes `seed = 0x10` to IN_MBOX;
SPU stores 8 deterministic words `(seed << 4) | i` for i=0..7
into a 32-byte stack-allocated `volatile uint32_t buffer[8]`;
SPU sums-reads the 8 words back; SPU writes the checksum
(= `8*0x100 + 28 = 0x81C = 2076`) to OUT_MBOX; halts via
stop 0x101.

The `volatile` qualifier is load-bearing — without it, GCC -O2
keeps the values in registers across both loops and skips LS
access entirely.

## Toolchain

Reuses the same from-source `ps3toolchain` Docker container
`ps3-build` setup as R5.9e.7 / R5.11 fixtures.

Build command (in container):

```
docker cp single_spu_loadstore_v1 ps3-build:/tmp/
docker exec ps3-build bash -c \
  'cd /tmp/single_spu_loadstore_v1 && \
   PS3DEV=/opt/ps3dev PSL1GHT=/opt/ps3dev/psl1ght \
   PATH=$PS3DEV/bin:$PS3DEV/ppu/bin:$PS3DEV/spu/bin:$PATH \
   make'
docker cp ps3-build:/tmp/single_spu_loadstore_v1/single_spu_loadstore_v1.self build/
```

SPU side compiled with `-O2 -Wall -nostartfiles -nostdlib
-Wl,--entry,main`.

## Decoded SPU code (spu-objdump)

Store loop top at pc=0x18; load loop top at pc=0x54; stop at pc=0x80.
The store-side body (pc=0x18..0x44) emits:
- `lqd $11, -32($10)` — load existing 16-byte aligned quadword
- `cwd $7, 0($10)` — generate word-insert mask (R5.10d)
- `shufb $12, $6, $11, $7` — merge new word into the quadword (RRR-form)
- `stqd $12, -32($10)` — store back

The load-side body (pc=0x54..0x78) emits:
- `lqd $18, -32($17)` — load 16-byte quadword
- `rotqby $20, $18, $19` — RR-form byte-rotate (the gap that surfaced
  R5.11b's first general fix below)
- `a $21, $21, $20` — sum-accumulate

All non-mailbox/stop instructions in the produced binary are
within the iteration-1 SPU subset post-R5.11b.

## RPCS3 + capture

Same R5.9c + R5.9e.3 trace writer used for the prior 3 fixtures.
C++ patches preserved unchanged at R5 closure — sha256
`d65aec91…ae1aba1c` (scaffolding) + `8f253d7d…66663a` (runtime hooks).

Captured artifacts staged at:
- `behavior-freeze/fixtures/spu/traces/single_spu_loadstore_v1.jsonl`
  (5 events, ~1.1 KB)
- `behavior-freeze/fixtures/spu/images/24bd144f88c413903a85554c3b78262655717a3681519c5fa47d6eba36ae90d2.spuimg`
  (262 KB, centralized layout)

## Trace contents (5 events)

```
seq 0: ppu_push_inmbox     target_spu=256 value=16 (= seed = 0x10)
seq 1: spu_image           sha=24bd144f...90d2 load=0x0 size=0x40000 entry_pc=0x0
seq 2: spu_wrch  ch28      target_spu=256 pc=124 value=2076 (= 0x81C = checksum)
seq 3: spu_stop            target_spu=256 pc=128 stop_code=0x101
seq 4: final_state         target_spu=256
                           gpr={r1=262064 (SP-64), r2=28, r3=8, r4=262128,
                                r5=0xFFFFFFFF, r6=263, r7=269554195 (= 0x10111213,
                                cwd output of last iter), r8=16, r9=256,
                                r10=262156, r11..r21 various}
                           channels={in_mbox=null, out_mbox=null,
                                     out_intr_mbox=null, snr1=0, snr2=0}
```

## Acceptance criteria (per traces/README.md)

| # | Criterion | Status |
|---|-----------|--------|
| 1 | Origem CC0 / license-clean | ✅ |
| 2 | Boota no RPCS3 instrumentado (R5.9c+R5.9e.3) | ✅ |
| 3 | Cria SPU thread group | ✅ |
| 4 | Exerce LS load/store + mailbox handshake | ✅ first such fixture |
| 5 | Sem DMA (zero ch21) | ✅ |
| 6 | `.jsonl` real, nunca editado | ✅ |
| 7 | Companion `.notes.md` | ✅ this file |
| 8 | `.spuimg` no layout centralizado | ✅ |
| 9 | Pipeline Rust passa com `diff_snapshots(...).is_identical()` | ✅ verified |
| 10 | Acceptance test commitado | ✅ `single_spu_loadstore_v1_replay.rs` |

## Replay-validation

Status: ✅ parser ok / transformer ok / interp replay ok / JIT
replay ok / cross-backend snapshot diff identical.

`total_steps`: interp=188, jit=188 (RECOMPILER FALLS BACK TO
INTERPRETER on the unsupported `rotqby` opcode + `cwd`/`shufb`
RRR-form codegen — R5 partial-fallback path. Cross-backend
agreement is via the fallback, not via independent JIT codegen.
This is fine for byte-identical contract.)

## Engine-side fixes co-landed (3 general, none single-fixture)

This fixture exposed three real general bugs in the Rust SPU
stack that had been silently latent:

### Fix #1 — `rotqby` (RR-form, opcode 0x1DC) added to interpreter

GCC -O2 emits `rotqby` (the RR-form, register-indexed sibling of
the already-implemented `rotqbyi` immediate variant 0x1FC) for
runtime-indexed extraction of a 4-byte slot from a 16-byte
aligned LS load. The SPU interpreter had `rotqbyi` but not
`rotqby`. Implementation is a textbook Cell BE op (same byte-
rotate semantics as `rotqbyi`; shift count from rb's preferred-
slot low 4 bits instead of imm7). Added in
`rust/rpcs3-spu-interpreter/src/lib.rs` step dispatch, plus
`encode::rotqby` helper, plus 2 unit tests
(`rotqby_rotates_quadword_left_by_rb_low_nibble`,
`rotqby_modulo_16_bytes_in_rb_preferred_slot`).

### Fix #2 — `cwd` / `cbd` / `chd` / `cdd` / `cbx`/`chx`/`cwx`/`cdx` default mask byte order

The Rust C-family insert-control ops had the two halves of the
default 16-byte mask SWAPPED relative to RPCS3's actual semantics.
Real RPCS3 emits a default mask of `0x10..0x1F` linear in SPU
big-endian byte order (`from64(0x18191A1B1C1D1E1F,
0x1011121314151617)` reads as `[0x10..0x17, 0x18..0x1F]` when
viewed as SPU bytes 0..15). The Rust impl had it reversed
(`[0x18..0x1F, 0x10..0x17]`).

This was a self-consistent bug: the existing 6 unit tests
asserted against the (wrong) Rust mask, so they passed in
isolation but diverged from real captured traces. Caught
end-to-end here by `single_spu_loadstore_v1`'s captured r7
final value = `0x10111213` (the cwd output of the last store
iteration with `p_byte=12`).

Fix corrects the default mask in the dispatch arm + updates the
6 affected unit tests
(`cdd_generates_low_doubleword_insert_mask`,
`cdd_generates_high_doubleword_insert_mask`,
`cwd_generates_word_insert_mask`,
`chd_generates_halfword_insert_mask`,
`cbd_generates_byte_insert_mask`,
`cbx_uses_rb_plus_ra_source`,
`cdd_real_v4_inst_at_pc_854`) to assert against the corrected
mask. Plus a new test
(`cwd_loadstore_v1_final_iter_matches_captured_trace`) that
specifically validates against the captured r7 = 0x10111213.

### Fix #3 — RRR-form `rt` / `rc` field positions in `pack_rrr` + dispatch

The Rust `pack_rrr` encoder placed `rt` (target) at bits 25..31
and `rc` (4th source) at bits 4..10. Real SPU encoding has them
reversed — `rt` at bits 4..10, `rc` at bits 25..31. The dispatch
arms for `selb` (0x8), `shufb` (0xB), `fma` (0xE), `fnms` (0xD),
`fms` (0xF) all extracted them with the wrong positions, mirroring
the encoder bug and making the executor self-consistent against
synthetic-encoded fixtures.

Caught end-to-end here by `single_spu_loadstore_v1`'s captured
`shufb $12, $6, $11, $7` (= 0xB182C307): real SPU has $12 in
bits 4..10 (rt) and $7 in bits 25..31 (rc/mask). The pre-fix
dispatch swapped them, making shufb write to r7 (wiping the
cwd output) instead of r12, and using r12 (= 0, uninitialized)
as the byte-permutation mask (which produces an all-zeros
output — exactly what we observed). Fix corrects pack_rrr +
all 5 RRR-form dispatch arms (selb, shufb, fma, fnms, fms).
Existing RRR unit tests pass transparently because both encode
and decode were swapped consistently — fixing both is
behaviour-preserving for any test that doesn't pin specific
bit positions.

These three fixes are all GENERAL — not single-fixture. They
correct the Rust SPU stack to match real Cell BE / RPCS3
semantics. They stay landed for any future fixture or R6 work.

## Stability

Once committed, this trace is a regression sentinel. The captured
`.spuimg` hash
`24bd144f88c413903a85554c3b78262655717a3681519c5fa47d6eba36ae90d2`
is the canonical content-address; toolchain output drift would
shift the hash and the right path is a new `vN+1` fixture.
