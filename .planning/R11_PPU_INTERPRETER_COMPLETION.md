# R11 — PPU interpreter completion

**Status:** PLAN + in-progress (2026-05-26).
**Goal:** finish the `rpcs3-ppu-interpreter` from the iteration-7
subset to full Cell BE PPU ISA coverage.
**Predecessor:** R10 (LV2 sync) closed.

## Baseline (audited 2026-05-26)

~75-85 instructions implemented (by dynamic frequency this already
covers most of what normal scalar code runs; the gaps are FP
breadth, indexed load/store, VMX, and system ops). Infra is
present — `PpuThread` has `gpr[32]`, `fpr[32] f64`, `vr[32] u128`,
`cr`, `xer`, `fpscr`, `lr`, `ctr`. No architectural blocker.

Tests are INLINE in `src/lib.rs` (136 `#[test]` at baseline), not
a `tests/` dir. Each wave adds handlers + inline tests.

## Wave sequence

| Wave | Family | ~Ops | ~LOC | Notes |
|---|---|---|---|---|
| R11.1 | FP arithmetic | 8-14 | 60 | fmadd/fmsub/fnmadd/fnmsub (s+d), fsel, fsqrt(s), fre(s), frsqrte(s) |
| R11.2 | FP convert + compare + status | 12 | 80 | fctiw/z, fctid/z, fcfid, frsp, fcmpu, fcmpo, mffs, mtfsf |
| R11.3 | Indexed load/store (P31 X-form) | 34 | 100 | lwzx/ldx/stfsx family + byte-reversed |
| R11.4 | ALU overflow (OE) + CR ops + barriers | 20 | 80 | addo/subfo/mulo, mcrf, crand/cror/etc, sync/isync |
| R11.5 | String / multiple | 6 | 50 | lmw/stmw/lswi/stswi/lswx/stswx |
| --- | **"PPU scalar complete" milestone** | | | |
| R11.6 | VMX integer | ~100 | 500 | add/sub/min/max/cmp/shift/pack/splat/merge/mul |
| R11.7 | VMX FP + load/store + misc | ~60 | 350 | lvx/stvx, vcfsx/vctsxs, vrefp, splat-imm, etc. |
| --- | **"PPU + VMX complete" milestone** | | | |
| R11.8 | System / supervisor | ~30 | 100 | SPR(TBR/PVR), MSR, atomic (lwarx/stwcx.), cache/TLB (mostly user-mode stubs) |

## Conventions

- One wave per commit. Each: handlers + inline `#[test]` + canonical
  gate (`cargo test --workspace --tests --release`, must stay
  ≥ current block count, 0 fail).
- Follow existing FP pattern: compute, write reg, call
  `fpscr_update_from_result`. Rc-bit / CR1 update only if the
  existing arms do it (they currently don't — match that).
- Single-precision ops round via `as f32 as f64`.
- Rust `f64::mul_add(c, b)` = fused multiply-add (single rounding)
  = PPC fmadd semantics.

## Open questions (from audit, deferred unless they bite)

1. VSCR — reuse FPSCR bits vs add `vscr: u32`. Decide at R11.6.
2. FPSCR rounding-mode (RN) — Rust f64 has no rounding control;
   accept as a documented limitation unless a fixture needs it.
3. Supervisor ops (MSR/TLB) — stub as no-op vs Unimplemented.
   Lean no-op for user-mode binaries.

## Validation status — R11 CLOSED 2026-05-26

| Slice | Commit | lib tests | Scope |
|---|---|---|---|
| R11.1 | `3f8f51215` | 147 | FP arithmetic (fmadd family, fsel, fsqrt/fre/frsqrte) |
| R11.2 | `7db2403a2` | 155 | FP convert/compare/status (fctiw/fcfid/fcmpu/mffs/mtfsf) |
| R11.3 | `27a988928` | 162 | indexed load/store (P31 X-form, ~34 ops) |
| R11.4 | `e0c5a1b8b` | 169 | CR-logical + mcrf + barriers |
| R11.5 | `3bb18b154` | 172 | string/multiple — **PPU SCALAR COMPLETE** |
| R11.6a | `2eb33b2b1` | 178 | VMX integer add/sub (modulo + saturating) |
| R11.6b | `72df1913f` | 183 | VMX min/max |
| R11.6c | `e62747dc4` | 188 | VMX compares (eq+gt, +CR6) |
| R11.6d | `4966801e0` | 195 | VMX shift/rotate + merge + splat |
| R11.7a | `2820c0dac` | 201 | VMX float (minmax/est/convert/compares) |
| R11.7b | `ad58100da` | 205 | VMX load/store (lvx/stvx/element/lvsl) |
| R11.6e | `32b6d6355` | 212 | VMX pack/unpack/multiply + vsel/vsldoi |
| R11.8 | `f938877a2` | 216 | system: atomic/mftb/msr/cache |

**Result:** PPU interpreter functionally complete — full scalar
ISA (integer/FP/branch/CR/load-store all forms) + VMX/AltiVec
(~120 vector ops) + system ops. 216 lib tests (from 136
baseline); workspace gate held at 268 result blocks, 0 fail
across all 13 commits.

**Deferred (none block user-mode PPC execution):**
- R11.4b: OE-enabled arithmetic overflow tracking (addo/subfo/
  mulldo etc.) — low frequency.
- mftb real monotonic clock (currently stub → 0).
- supervisor TLB/SLB ops, rfid.
- VMX: vmsum* multiply-sum family, vcmpbfp, crypto (vcipher),
  exotic packs (vpkpx) — rare in practice.
