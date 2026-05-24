//! `rpcs3-spu-decoder` — pure SPU instruction decoder + basic-block
//! analysis.
//!
//! Produces the input that every SPU executor backend consumes:
//! a [`SpuFunction`] = a graph of [`SpuBasicBlock`]s, each holding a
//! linear list of [`SpuInstruction`]s and a list of successor PCs.
//!
//! ## What this crate does NOT do
//!
//! - No execution (that's the interpreter / recompiler's job).
//! - No codegen.
//! - No register-flow analysis (R4 in `SPU_RECOMPILER_PLAN.md`).
//! - No constant propagation (R4).
//!
//! The decoder is intentionally *opaque* about semantics: it tags each
//! instruction with a [`SpuInstKind`] and lets backends decide what to
//! do with it. Every backend is free to walk the same `SpuFunction`
//! and emit interpreter dispatch / Cranelift IR / LLVM IR / etc.

#![allow(missing_docs)]

use std::collections::BTreeMap;

const SPU_LS_SIZE: usize = 0x40000;
const SPU_LS_MASK: u32 = (SPU_LS_SIZE - 1) as u32;

// =====================================================================
// Bit-field helpers (MSB=0 numbering, matches SPU ISA spec)
// =====================================================================

#[inline]
const fn bits(inst: u32, pos: u32, nb: u32) -> u32 {
    (inst >> (32 - pos - nb)) & ((1 << nb) - 1)
}

#[inline] fn rt(inst: u32) -> u8 { bits(inst, 25, 7) as u8 }
#[inline] fn ra(inst: u32) -> u8 { bits(inst, 18, 7) as u8 }
#[inline] fn rb(inst: u32) -> u8 { bits(inst, 11, 7) as u8 }
#[inline] fn rc_rrr(inst: u32) -> u8 { bits(inst, 4, 7) as u8 }
#[inline] fn rt_rrr(inst: u32) -> u8 { bits(inst, 25, 7) as u8 }

#[inline]
fn i7_signed(inst: u32) -> i8 {
    let v = bits(inst, 11, 7) as u32;
    if v & 0x40 != 0 { (v | 0xFFFF_FF80) as i8 } else { v as i8 }
}

#[inline]
fn i10_signed(inst: u32) -> i16 {
    let v = bits(inst, 8, 10) as u32;
    if v & 0x200 != 0 { (v | 0xFFFF_FC00) as i16 } else { v as i16 }
}

#[inline]
fn i16_unsigned(inst: u32) -> u16 {
    bits(inst, 9, 16) as u16
}

#[inline]
fn i16_signed(inst: u32) -> i16 {
    bits(inst, 9, 16) as i16
}

#[inline]
#[allow(dead_code)]
fn i18_unsigned(inst: u32) -> u32 {
    bits(inst, 7, 18)
}

// =====================================================================
// Public types
// =====================================================================

/// A decoded SPU instruction at a specific PC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuInstruction {
    /// PC where this instruction lives (in local store).
    pub pc: u32,
    /// Raw 32-bit instruction word, big-endian as stored on the SPU.
    pub raw: u32,
    /// Decoded semantic kind.
    pub kind: SpuInstKind,
}

/// What family the instruction belongs to. The recompiler dispatches
/// codegen based on this; the interpreter ignores it (it dispatches
/// on `raw` directly).
///
/// Variants are coarse on purpose. A full opcode table would be
/// duplicating the interpreter's match arms; keeping the families
/// coarse means the decoder ships in ~500 lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum SpuInstKind {
    /// `stop` / `stopd` — halts execution with a 14-bit code.
    Stop { code: u32 },
    /// `nop` (0x201), `lnop` (0x001), `sync` (0x002), `dsync` (0x003).
    Nop,
    /// `il`, `ila`, `ilh`, `ilhu`, `iohl` — broadcast immediate load.
    LoadImm { rt: u8 },
    /// 11-bit RR-form ALU: `op rt, ra, rb`.
    AluRr { rt: u8, ra: u8, rb: u8 },
    /// RI10-form (8-bit primary): `op rt, ra, imm10`.
    AluImm { rt: u8, ra: u8, imm10: i16 },
    /// RI7-form (11-bit primary): `op rt, ra, imm7`.
    AluImm7 { rt: u8, ra: u8, imm7: i8 },
    /// 4-bit primary RRR-form: `op rt, ra, rb, rc`.
    Rrr { rt: u8, ra: u8, rb: u8, rc: u8 },
    /// Indexed load `lqx rt, ra, rb` or store `stqx rt, ra, rb`.
    LoadStoreIndexed { rt: u8, ra: u8, rb: u8, is_store: bool },
    /// D-form load `lqd rt, imm10*16(ra)` or store `stqd`.
    LoadStoreDForm { rt: u8, ra: u8, offset: i16, is_store: bool },
    /// R5.10b — PC-relative load `lqr rt, imm16` (RI16-form, primary 0x67).
    /// `target_pc` is pre-computed as `(pc + (imm16 << 2)) & 0x3FFF0` —
    /// matches RPCS3 C++ `spu_ls_target(pc, imm16)` (LS-mask + 16-byte
    /// align). Used by `lqr` today; `stqr` (primary 0x47) is NOT covered
    /// by this variant in R5.10b — adding STQR is a separate slice.
    /// Compare with `LoadStoreDForm` (register-base) and `LoadStoreIndexed`
    /// (register+register): this is the third address-mode for qword
    /// load/store, the PC-relative one.
    LoadRel { rt: u8, target_pc: u32 },
    /// R5.10g — `stqr rt, imm16` (Store Quadword PC-Relative, primary
    /// `p9 = 0x047`, RI16-form). Direct mirror of `LoadRel`/LQR (R5.10b)
    /// — same `target_pc = (pc + (imm16 << 2)) & 0x3FFF0` computation,
    /// just `LS[target..target+16] = gpr[rt]` instead of the read.
    /// **Pure store** — no channels, no DMA, no FP, no atomics, no
    /// branches. C++ ref: `rpcs3/Emu/Cell/SPUInterpreter.cpp:1634`.
    StoreRel { rt: u8, target_pc: u32 },
    /// R5.10o — `lqa rt, imm16` (Load Quadword Absolute, primary
    /// `p9 = 0x061`, RI16-form). Like `LoadRel` (LQR) but with PC=0
    /// in the address calc: `target_pc = (imm16 << 2) & 0x3FFF0`
    /// (no PC contribution, hence "Absolute"). The `Rel` variant
    /// would be a semantic lie, so a distinct `LoadAbs` is used; the
    /// JIT/interpreter discriminate on the raw 9-bit primary anyway.
    /// **Pure LS access** — no channels, no DMA, no FP, no atomics,
    /// no branches. C++ ref: `rpcs3/Emu/Cell/SPUInterpreter.cpp:1648`.
    LoadAbs { rt: u8, target_pc: u32 },
    /// R5.10o — `stqa rt, imm16` (Store Quadword Absolute, primary
    /// `p9 = 0x041`, RI16-form). Mirror of `LoadAbs` with reversed
    /// direction. Same `target_pc = (imm16 << 2) & 0x3FFF0` formula
    /// (PC=0). **Pure store**. C++ ref:
    /// `rpcs3/Emu/Cell/SPUInterpreter.cpp:1594`.
    StoreAbs { rt: u8, target_pc: u32 },
    /// R5.10f — `fsmbi rt, imm16` (Form Select Mask for Bytes Immediate,
    /// `p9 = 0x065`, RI16-form). Builds a 16-byte mask from the 16-bit
    /// immediate: byte `k` of `rt` becomes `0xFF` iff bit `(15 - k)` of
    /// `imm16` is set, else `0x00`. **Pure compute** — no `ra`/`rb`/LS
    /// access, no channels, no DMA, no FP, no atomics, no branches.
    /// Distinct from the RR-form FSMB (`p11 = 0x1B6`, source = ra-bits)
    /// only in where the 16 source bits come from. C++ ref:
    /// `rpcs3/Emu/Cell/SPUInterpreter.cpp:1671`.
    FormSelectMaskImm { rt: u8, imm16: u16 },
    /// R5.10d — C-family insert-control opcodes (8 mnemonics):
    /// CBX/CHX/CWX/CDX (primaries 0x1D4..0x1D7, RR-form, source RegRb)
    /// and CBD/CHD/CWD/CDD (primaries 0x1F4..0x1F7, RI7-form, source
    /// ImmI7). All 8 build a 16-byte `shufb` mask that, when used with
    /// a subsequent `shufb`, inserts `granularity` bytes (1/2/4/8) of
    /// source A's preferred slot into source B at a runtime-computed
    /// byte offset within the destination qword. **Pure compute**: no
    /// channels, no DMA, no FP, no atomics, no LS access. C++ refs in
    /// `rpcs3/Emu/Cell/SPUInterpreter.cpp` lines 772 (CBX) through 942
    /// (CDD).
    InsertControl {
        rt: u8,
        ra: u8,
        source: InsertControlSource,
        granularity: InsertGranularity,
    },
    /// Channel access: `rdch`, `wrch`, `rchcnt`. `rt`/`channel` slots
    /// are the same as ra/rt in encoding; we surface them flat.
    Channel { rt: u8, channel: u32, kind: ChannelOp },
    /// RR unary: `op rt, ra`. Used by clz/cntb/xs*h/xshw/xswd/orx/fsm/
    /// frest/frsqest/orx and similar single-source ops.
    Unary { rt: u8, ra: u8 },
    /// Convert with scale (RI8): `op rt, ra, scale`.
    Convert { rt: u8, ra: u8, scale: u8 },
    /// Direct branch `br` / `bra` — unconditional, always taken.
    BranchDirect { target: u32 },
    /// Conditional branch `brnz` / `brz` — taken if condition on rt
    /// holds at runtime; fall-through otherwise.
    BranchCond { rt: u8, target: u32 },
    /// Branch+link relative `brsl` — writes link to rt, then jumps.
    BranchDirectLink { rt: u8, target: u32 },
    /// Indirect branch `bi` / `iret` — target read from ra at runtime.
    BranchIndirect { ra: u8 },
    /// Indirect with link `bisl` — writes link to rt.
    BranchIndirectLink { rt: u8, ra: u8 },
    /// Conditional indirect `biz` / `binz` / `bihz` / `bihnz` — branch
    /// to ra if condition on rt holds at runtime.
    BranchIndirectCond { rt: u8, ra: u8 },
    /// Branch hint `hbr` / `hbra` / `hbrr` — interpreter NOP.
    BranchHint,
    /// We recognise the encoding but haven't classified it yet.
    /// Backends should emit fallback (interpreter) for these.
    Unclassified,
}

/// Which channel-access flavour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum ChannelOp {
    Read,
    Write,
    ReadCount,
}

/// R5.10d — How a C-family insert-control opcode forms its address.
/// `RegRb` is the RR-form (CBX/CHX/CWX/CDX); `ImmI7` is the RI7-form
/// (CBD/CHD/CWD/CDD). The two forms share the same mask-construction
/// logic — only the address source differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum InsertControlSource {
    /// CBD/CHD/CWD/CDD — addr = `gpr[ra]_lane0 + sext(imm7)`.
    ImmI7 { imm7: i8 },
    /// CBX/CHX/CWX/CDX — addr = `gpr[ra]_lane0 + gpr[rb]_lane0`.
    RegRb { rb: u8 },
}

/// R5.10d — How many bytes of source A get inserted at the computed
/// position. The granularity also dictates the alignment mask applied
/// to the address: `Byte` uses `& 0xF`, `Halfword` uses `& 0xE`,
/// `Word` uses `& 0xC`, `Doubleword` uses `& 0x8`. Mirrors the
/// `_u8`/`_u16`/`_u32`/`_u64` writes in the RPCS3 C++ implementations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum InsertGranularity {
    /// CBX/CBD — 1 byte, alignment mask `0xF`.
    Byte,
    /// CHX/CHD — 2 bytes, alignment mask `0xE`.
    Halfword,
    /// CWX/CWD — 4 bytes, alignment mask `0xC`.
    Word,
    /// CDX/CDD — 8 bytes, alignment mask `0x8`.
    Doubleword,
}

/// Where a basic block sends control next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockTerminator {
    /// `stop` / `stopd` — no successor.
    Stop { code: u32 },
    /// Unconditional direct jump to a known PC inside LS.
    UncondDirect { target: u32 },
    /// Unconditional but indirect — successor PC unknown at decode.
    UncondIndirect,
    /// Two-way branch: `taken` and `fall_through` both point inside LS.
    CondDirect { taken: u32, fall_through: u32 },
    /// Two-way indirect (e.g. `biz`): one fall-through, one indirect.
    CondIndirect { fall_through: u32 },
    /// Decoder ran into an unrecognised opcode; the block ends here
    /// to be safe. Backends MUST treat it as a fallback exit.
    UnknownOpcode { fall_through: u32 },
    /// Decoder hit the end of the explored region without finding a
    /// terminator. Recovery: fall through to the next PC.
    FellThroughLimit { fall_through: u32 },
}

impl BlockTerminator {
    /// All directly-known successor PCs (excludes indirect targets,
    /// which the runtime resolves).
    pub fn direct_successors(&self) -> Vec<u32> {
        match self {
            Self::Stop { .. } | Self::UncondIndirect => Vec::new(),
            Self::UncondDirect { target } => vec![*target],
            Self::CondDirect { taken, fall_through } => vec![*taken, *fall_through],
            Self::CondIndirect { fall_through }
            | Self::UnknownOpcode { fall_through }
            | Self::FellThroughLimit { fall_through } => vec![*fall_through],
        }
    }
}

/// One basic block: a maximal straight-line region from a known entry
/// PC up to (and including) a control-flow terminator.
#[derive(Debug, Clone)]
pub struct SpuBasicBlock {
    /// PC of the first instruction.
    pub start_pc: u32,
    /// PC of the last instruction + 4 (one past the last).
    pub end_pc: u32,
    /// All instructions in this block, in execution order.
    pub instructions: Vec<SpuInstruction>,
    /// How control leaves this block.
    pub terminator: BlockTerminator,
}

/// A complete decoded SPU function: graph of basic blocks reachable
/// from the entry PC.
#[derive(Debug, Clone)]
pub struct SpuFunction {
    /// Entry PC the worklist started from.
    pub entry: u32,
    /// All discovered blocks, keyed by `start_pc` for cheap lookup.
    pub blocks: BTreeMap<u32, SpuBasicBlock>,
}

impl SpuFunction {
    /// Number of distinct basic blocks discovered.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Total instructions across all blocks.
    pub fn instruction_count(&self) -> usize {
        self.blocks.values().map(|b| b.instructions.len()).sum()
    }

    /// All terminator successors across the function — handy for
    /// validating that every branch lands inside a known block.
    pub fn all_direct_successors(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for b in self.blocks.values() {
            out.extend(b.terminator.direct_successors());
        }
        out
    }
}

// =====================================================================
// Decoder errors
// =====================================================================

/// Error from [`decode_function`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Entry PC outside LS or not 4-byte aligned.
    BadEntryPc(u32),
    /// `ls` slice does not cover the requested PC.
    OutOfBounds { pc: u32 },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadEntryPc(pc) => write!(f, "entry PC 0x{pc:x} invalid"),
            Self::OutOfBounds { pc } => write!(f, "PC 0x{pc:x} outside LS slice"),
        }
    }
}

impl std::error::Error for DecodeError {}

// =====================================================================
// Per-instruction decode
// =====================================================================

/// Decode one 32-bit SPU instruction word at PC `pc`.
#[must_use]
pub fn decode_inst(raw: u32, pc: u32) -> SpuInstruction {
    let kind = classify(raw, pc);
    SpuInstruction { pc, raw, kind }
}

fn classify(raw: u32, pc: u32) -> SpuInstKind {
    // ---- 11-bit primary opcode dispatch ----
    let p11 = bits(raw, 0, 11);

    // stop / stopd / sync / dsync / nop / lnop
    match p11 {
        0x000 => return SpuInstKind::Stop { code: bits(raw, 18, 14) },
        0x001 | 0x002 | 0x003 | 0x201 => return SpuInstKind::Nop,
        // STOPD = stop & signal, behaves like stop 0
        0x140 => return SpuInstKind::Stop { code: 0 },
        _ => {}
    }

    // Branch family (11-bit)
    match p11 {
        0x1A8 => return SpuInstKind::BranchIndirect { ra: ra(raw) },
        0x1AA => return SpuInstKind::BranchIndirect { ra: ra(raw) }, // iret
        0x1A9 => return SpuInstKind::BranchIndirectLink { rt: rt(raw), ra: ra(raw) },
        0x1AC => return SpuInstKind::BranchHint, // hbr
        0x128 | 0x129 | 0x12A | 0x12B => {
            return SpuInstKind::BranchIndirectCond { rt: rt(raw), ra: ra(raw) };
        }
        _ => {}
    }

    // RR-form ALU/bitwise/compare/shift (11-bit primary, 0-magn group)
    if is_alu_rr_11bit(p11) {
        return SpuInstKind::AluRr { rt: rt(raw), ra: ra(raw), rb: rb(raw) };
    }

    // RR-form unary (11-bit primary, single source)
    if is_unary_rr_11bit(p11) {
        return SpuInstKind::Unary { rt: rt(raw), ra: ra(raw) };
    }

    // R5.10d — C-family insert-control (8 opcodes).
    // RR-form (CBX/CHX/CWX/CDX) and RI7-form (CBD/CHD/CWD/CDD) share
    // the same `InsertControl` variant; the form is encoded in the
    // `source` field. Granularity is derived from the low 2 bits of
    // the 11-bit primary.
    if matches!(p11, 0x1D4 | 0x1D5 | 0x1D6 | 0x1D7 | 0x1F4 | 0x1F5 | 0x1F6 | 0x1F7) {
        let granularity = match p11 & 0x3 {
            0x0 => InsertGranularity::Byte,
            0x1 => InsertGranularity::Halfword,
            0x2 => InsertGranularity::Word,
            0x3 => InsertGranularity::Doubleword,
            _ => unreachable!(),
        };
        let source = if (p11 & 0x020) != 0 {
            // 0x1Fx — RI7-form (CBD/CHD/CWD/CDD)
            InsertControlSource::ImmI7 { imm7: i7_signed(raw) }
        } else {
            // 0x1Dx — RR-form (CBX/CHX/CWX/CDX)
            InsertControlSource::RegRb { rb: rb(raw) }
        };
        return SpuInstKind::InsertControl {
            rt: rt(raw),
            ra: ra(raw),
            source,
            granularity,
        };
    }

    // Channel access (11-bit primary)
    match p11 {
        0x00D => return SpuInstKind::Channel { rt: rt(raw), channel: ra(raw) as u32 & 0x7F, kind: ChannelOp::Read },
        0x10D => return SpuInstKind::Channel { rt: rt(raw), channel: ra(raw) as u32 & 0x7F, kind: ChannelOp::Write },
        0x00F => return SpuInstKind::Channel { rt: rt(raw), channel: ra(raw) as u32 & 0x7F, kind: ChannelOp::ReadCount },
        _ => {}
    }

    // Indexed load/store (11-bit primary)
    match p11 {
        0x1C4 => return SpuInstKind::LoadStoreIndexed { rt: rt(raw), ra: ra(raw), rb: rb(raw), is_store: false },
        0x144 => return SpuInstKind::LoadStoreIndexed { rt: rt(raw), ra: ra(raw), rb: rb(raw), is_store: true },
        _ => {}
    }

    // RR-form shifts (11-bit primary): word + halfword.
    if matches!(p11,
        0x05B | 0x058 | 0x059 | 0x05A   // word: shl/rot/rotm/rotma
        | 0x05F | 0x05C | 0x05D | 0x05E // halfword: shlh/roth/rothm/rotmah
    ) {
        return SpuInstKind::AluRr { rt: rt(raw), ra: ra(raw), rb: rb(raw) };
    }

    // RI7 shift immediates (11-bit primary): word + halfword + quadword.
    if matches!(p11,
        0x078 | 0x079 | 0x07A | 0x07B  // word shifts (roti/rotmi/rotmai/shli)
        | 0x07C | 0x07D | 0x07E | 0x07F  // halfword shifts (rothi/rothmi/rotmahi/shlhi)
        // Quadword shift-imm family. Per RPCS3 SPUOpcodes.h:
        //   0x1FB = SHLQBII (bit-shift left immediate)
        //   0x1FC = ROTQBYI (byte-rotate immediate)
        //   0x1FD = ROTQMBYI (byte-shift-right-with-zero-fill immediate, R5.10m)
        //   0x1FF = SHLQBYI (byte-shift left immediate)
        // ROTQBII (0x1F8) and ROTQMBII (0x1F9) are NOT yet covered (no v4 use).
        | 0x1FB | 0x1FC | 0x1FD | 0x1FF
    ) {
        return SpuInstKind::AluImm7 { rt: rt(raw), ra: ra(raw), imm7: i7_signed(raw) };
    }

    // ---- 9-bit primary (branches direct + immediate ALU) ----
    let p9 = bits(raw, 0, 9);
    match p9 {
        0x064 => {
            // br i16 — relative
            return SpuInstKind::BranchDirect {
                target: branch_relative_target(pc, i16_signed(raw)),
            };
        }
        0x060 => {
            // bra i16 — absolute
            return SpuInstKind::BranchDirect {
                target: (i16_signed(raw) as u32).wrapping_mul(4) & SPU_LS_MASK,
            };
        }
        0x066 => {
            // brsl rt, i16
            return SpuInstKind::BranchDirectLink {
                rt: rt(raw),
                target: branch_relative_target(pc, i16_signed(raw)),
            };
        }
        0x067 => {
            // R5.10b — lqr rt, imm16 — PC-relative qword load.
            // Target = (pc + (imm16 << 2)) & 0x3FFF0 mirroring the C++
            // `spu_ls_target(pc, imm16)` semantics: LS-mask AND 16-byte
            // align (the bottom 4 bits of the address are forced to 0
            // because qwords are aligned). Distinct from
            // `branch_relative_target` (which only LS-masks).
            let imm = i16_signed(raw) as i32;
            let target = ((pc as i32).wrapping_add(imm.wrapping_mul(4)) as u32) & 0x3FFF0;
            return SpuInstKind::LoadRel {
                rt: rt(raw),
                target_pc: target,
            };
        }
        // R5.10g — stqr rt, imm16 — PC-relative qword store. Direct
        // mirror of LQR (0x067) — same `spu_ls_target` computation,
        // just store-direction. Variant kept separate from `LoadRel` to
        // preserve R5.10b's existing test surface.
        0x047 => {
            let imm = i16_signed(raw) as i32;
            let target = ((pc as i32).wrapping_add(imm.wrapping_mul(4)) as u32) & 0x3FFF0;
            return SpuInstKind::StoreRel {
                rt: rt(raw),
                target_pc: target,
            };
        }
        // R5.10o — lqa rt, imm16 — Load Quadword Absolute. Same
        // `spu_ls_target` formula as LQR but with PC=0:
        //   target = (imm16 << 2) & 0x3FFF0
        // Distinct variant `LoadAbs` to avoid the misleading `Rel`
        // name; same numeric layout.
        0x061 => {
            let imm = i16_signed(raw) as i32;
            let target = (imm.wrapping_mul(4) as u32) & 0x3FFF0;
            return SpuInstKind::LoadAbs {
                rt: rt(raw),
                target_pc: target,
            };
        }
        // R5.10o — stqa rt, imm16 — Store Quadword Absolute. Mirror
        // of LQA above with store direction.
        0x041 => {
            let imm = i16_signed(raw) as i32;
            let target = (imm.wrapping_mul(4) as u32) & 0x3FFF0;
            return SpuInstKind::StoreAbs {
                rt: rt(raw),
                target_pc: target,
            };
        }
        0x042 | 0x040 => {
            // brnz/brz rt, i16
            return SpuInstKind::BranchCond {
                rt: rt(raw),
                target: branch_relative_target(pc, i16_signed(raw)),
            };
        }
        // il / ilh / ilhu / iohl
        0x081 | 0x082 | 0x083 | 0x0C1 => {
            let _ = i16_unsigned(raw); // imm extracted by backend if needed
            return SpuInstKind::LoadImm { rt: rt(raw) };
        }
        // R5.10f — fsmbi rt, imm16 (Form Select Mask for Bytes Immediate).
        // Distinct from FSM/FSMH/FSMB (RR-form, p11=0x1B4..0x1B6) — the
        // 16 source bits come from `imm16` instead of `ra`'s preferred
        // slot. Pure compute; backend (interpreter) discriminates by the
        // variant tag.
        0x065 => {
            return SpuInstKind::FormSelectMaskImm {
                rt: rt(raw),
                imm16: i16_unsigned(raw),
            };
        }
        _ => {}
    }

    // ---- 10-bit primary (convert ops with 8-bit scale) ----
    let p10 = bits(raw, 0, 10);
    if matches!(p10, 0x1D8 | 0x1D9 | 0x1DA | 0x1DB) {
        return SpuInstKind::Convert {
            rt: rt(raw),
            ra: ra(raw),
            scale: bits(raw, 10, 8) as u8,
        };
    }

    // ---- 4-bit primary (RRR-form: selb, shufb, fma, fnms, fms) ----
    let p4 = bits(raw, 0, 4);
    if matches!(p4, 0x8 | 0xB | 0xE | 0xD | 0xF) {
        return SpuInstKind::Rrr {
            rt: rt_rrr(raw),
            ra: ra(raw),
            rb: rb(raw),
            rc: rc_rrr(raw),
        };
    }

    // ---- 8-bit primary (D-form load/store + immediate ALU) ----
    let p8 = bits(raw, 0, 8);
    match p8 {
        0x34 => return SpuInstKind::LoadStoreDForm { rt: rt(raw), ra: ra(raw), offset: i10_signed(raw), is_store: false },
        0x24 => return SpuInstKind::LoadStoreDForm { rt: rt(raw), ra: ra(raw), offset: i10_signed(raw), is_store: true },
        // immediate ALU: word arith/cmp + halfword arith/cmp + multiplies.
        0x14 | 0x04 | 0x44 | 0x1C | 0x7C | 0x4C | 0x5C
        | 0x7D | 0x4D | 0x5D
        | 0x74 | 0x75
        | 0x1D | 0x0C => {
            return SpuInstKind::AluImm { rt: rt(raw), ra: ra(raw), imm10: i10_signed(raw) };
        }
        // byte-immediate ops: orbi/andbi/xorbi/cgtbi/clgtbi/ceqbi.
        // Per RPCS3 `bf_t<u32, 14, 8> i8` (SPUOpcodes.h), the 8-bit
        // immediate occupies LSB-0 bits 14..21 (= MSB-0 bits 10..17),
        // matching `(raw >> 14) & 0xFF`. R5.10i corrected the prior
        // `(raw >> 16) & 0xFF` extraction which was off by 2 bits and
        // produced silently-wrong values for any non-zero `i8` (the
        // interpreter had no byte-imm arm to surface the bug, but the
        // JIT inherited it via `imm10`). We sign-extend to i16 so the
        // existing `AluImm { imm10 }` carrier still works.
        0x06 | 0x16 | 0x46 | 0x4E | 0x5E | 0x7E => {
            let imm8 = ((raw >> 14) & 0xFF) as u8 as i8;
            return SpuInstKind::AluImm { rt: rt(raw), ra: ra(raw), imm10: imm8 as i16 };
        }
        _ => {}
    }

    // ---- 7-bit primary: ila + branch hints ----
    let p7 = bits(raw, 0, 7);
    match p7 {
        0x21 => return SpuInstKind::LoadImm { rt: rt(raw) }, // ila rt, i18
        0x08 | 0x09 => return SpuInstKind::BranchHint,        // hbra / hbrr
        _ => {}
    }

    SpuInstKind::Unclassified
}

#[inline]
fn branch_relative_target(pc: u32, imm16: i16) -> u32 {
    let off = (imm16 as i64).wrapping_mul(4);
    ((pc as i64).wrapping_add(off) as u32) & SPU_LS_MASK
}

fn is_alu_rr_11bit(p11: u32) -> bool {
    matches!(
        p11,
        // word add / sub / and / or / xor / nor / nand / eqv / andc / orc
        // (canonical SPU primaries; matches both rpcs3 C++ and our interpreter
        // post-2026-04-25 opcode-canonicalisation fix)
        0x0C0 | 0x040 | 0x0C1 | 0x041 | 0x241 | 0x049 | 0x0C9 | 0x249 | 0x2C1 | 0x2C9
        // halfword add / sub / mul
        | 0x0C8 | 0x048 | 0x3C4 | 0x3CC
        // compares (word + halfword + byte)
        | 0x3C0 | 0x240 | 0x2C0 | 0x3C8 | 0x3D0 | 0x248 | 0x250 | 0x2C8 | 0x2D0
        // float compares + arith
        | 0x2C2 | 0x2CA | 0x3C2 | 0x3CA | 0x2C4 | 0x2C5 | 0x2C6
        // carry / borrow generate
        | 0x0C2 | 0x042
    )
}

fn is_unary_rr_11bit(p11: u32) -> bool {
    matches!(
        p11,
        // sign-extend variants
        0x2A5 | 0x2A6 | 0x2AE | 0x2B4 | 0x2B6
        // form-select-mask: word (FSM, R5.x), halfword (FSMH, R5.10f),
        // byte (FSMB, R5.10f). Same RR-unary shape; semantics differ
        // only in granularity. Interpreter discriminates on p11.
        | 0x1B4 | 0x1B5 | 0x1B6
        // reciprocal estimates
        | 0x1B8 | 0x1B9
        // or-across
        | 0x1F0
    )
}

// =====================================================================
// Block-level analysis
// =====================================================================

/// Decode the function reachable from `entry_pc` in `ls` (a 256 KB
/// local-store slice). Two-pass walk:
///
/// 1. Pass 1: scan reachable instructions to collect every branch
///    target (the set of "block leaders").
/// 2. Pass 2: cut basic blocks at every leader. A block ends when it
///    hits its own terminator OR the start of another leader (fall-
///    through into a block that's also a branch target).
///
/// `max_blocks` caps recursion so a runaway dispatcher cannot blow up
/// memory. `2048` is a sane default for any single SPU function.
pub fn decode_function(
    ls: &[u8],
    entry_pc: u32,
    max_blocks: usize,
) -> Result<SpuFunction, DecodeError> {
    if entry_pc as usize >= ls.len() || entry_pc & 0x3 != 0 {
        return Err(DecodeError::BadEntryPc(entry_pc));
    }

    // Pass 1: collect all PCs that are branch leaders (entry +
    // every direct successor reachable via DFS).
    let leaders = collect_block_leaders(ls, entry_pc, max_blocks)?;

    // Pass 2: cut blocks. Each leader gets its own block; a block
    // ends at its own terminator OR when it crosses another leader.
    let mut blocks: BTreeMap<u32, SpuBasicBlock> = BTreeMap::new();
    for &start in &leaders {
        if blocks.len() >= max_blocks { break; }
        let block = decode_block_until(ls, start, &leaders)?;
        blocks.insert(start, block);
    }

    Ok(SpuFunction { entry: entry_pc, blocks })
}

/// Two-pass leader collection. Walks reachable instructions linearly
/// from `entry`, then chases every branch target found. Returns the
/// union sorted by PC.
fn collect_block_leaders(
    ls: &[u8],
    entry: u32,
    max_blocks: usize,
) -> Result<std::collections::BTreeSet<u32>, DecodeError> {
    use std::collections::BTreeSet;

    let mut leaders: BTreeSet<u32> = BTreeSet::new();
    let mut worklist: Vec<u32> = vec![entry];
    let mut visited: BTreeSet<u32> = BTreeSet::new();

    while let Some(start) = worklist.pop() {
        if !visited.insert(start) {
            continue;
        }
        if leaders.len() >= max_blocks {
            break;
        }
        leaders.insert(start);

        // Walk linearly from `start` until we hit a terminator. Every
        // direct successor we discover becomes a new leader candidate.
        let block = decode_block(ls, start)?;
        for succ in block.terminator.direct_successors() {
            if !visited.contains(&succ) {
                worklist.push(succ);
            }
        }
    }
    Ok(leaders)
}

/// Decode a block starting at `start`, but cut early if we cross
/// another leader PC (so leaders never overlap).
fn decode_block_until(
    ls: &[u8],
    start: u32,
    leaders: &std::collections::BTreeSet<u32>,
) -> Result<SpuBasicBlock, DecodeError> {
    let mut instructions = Vec::with_capacity(16);
    let mut pc = start;
    loop {
        let off = pc as usize;
        if off + 4 > ls.len() {
            return Ok(SpuBasicBlock {
                start_pc: start,
                end_pc: pc,
                instructions,
                terminator: BlockTerminator::FellThroughLimit { fall_through: pc },
            });
        }
        let raw = u32::from_be_bytes([ls[off], ls[off+1], ls[off+2], ls[off+3]]);
        let inst = decode_inst(raw, pc);
        let next_pc = pc.wrapping_add(4) & SPU_LS_MASK;

        // If the next PC is *another* leader, this block falls through
        // into it — cut here without emitting our own terminator.
        // The instruction at `pc` is included (it's not a branch, just
        // the last straight-line op before the boundary).
        let term_for_inst = block_terminator_for(&inst.kind, next_pc);
        instructions.push(inst);

        if let Some(t) = term_for_inst {
            return Ok(SpuBasicBlock {
                start_pc: start,
                end_pc: next_pc,
                instructions,
                terminator: t,
            });
        }

        if next_pc != start && leaders.contains(&next_pc) {
            return Ok(SpuBasicBlock {
                start_pc: start,
                end_pc: next_pc,
                instructions,
                terminator: BlockTerminator::UncondDirect { target: next_pc },
            });
        }
        pc = next_pc;

        if instructions.len() > (SPU_LS_SIZE / 4) {
            return Ok(SpuBasicBlock {
                start_pc: start,
                end_pc: pc,
                instructions,
                terminator: BlockTerminator::FellThroughLimit { fall_through: pc },
            });
        }
    }
}

/// Decode a single basic block starting at `start`. Stops at the first
/// terminator (branch, stop, unknown opcode) or when LS runs out.
pub fn decode_block(ls: &[u8], start: u32) -> Result<SpuBasicBlock, DecodeError> {
    let mut instructions = Vec::with_capacity(16);
    let mut pc = start;
    loop {
        let off = pc as usize;
        if off + 4 > ls.len() {
            // Out of LS — block ends here, "fall-through" points
            // where the next instruction would have been.
            let term = BlockTerminator::FellThroughLimit { fall_through: pc };
            return Ok(SpuBasicBlock {
                start_pc: start,
                end_pc: pc,
                instructions,
                terminator: term,
            });
        }
        let raw = u32::from_be_bytes([ls[off], ls[off+1], ls[off+2], ls[off+3]]);
        let inst = decode_inst(raw, pc);

        // Check terminator BEFORE pushing — we always push the
        // terminator instruction itself, but stop right after.
        let next_pc = pc.wrapping_add(4) & SPU_LS_MASK;
        let term = block_terminator_for(&inst.kind, next_pc);
        instructions.push(inst);

        if let Some(t) = term {
            return Ok(SpuBasicBlock {
                start_pc: start,
                end_pc: next_pc,
                instructions,
                terminator: t,
            });
        }
        pc = next_pc;

        // Defensive: don't loop forever on a 256 KB LS.
        if instructions.len() > (SPU_LS_SIZE / 4) {
            return Ok(SpuBasicBlock {
                start_pc: start,
                end_pc: pc,
                instructions,
                terminator: BlockTerminator::FellThroughLimit { fall_through: pc },
            });
        }
    }
}

fn block_terminator_for(kind: &SpuInstKind, fall_through: u32) -> Option<BlockTerminator> {
    match kind {
        SpuInstKind::Stop { code } => Some(BlockTerminator::Stop { code: *code }),
        SpuInstKind::BranchDirect { target } => {
            Some(BlockTerminator::UncondDirect { target: *target })
        }
        SpuInstKind::BranchDirectLink { target, .. } => {
            // brsl: control DOES leave; the link register provides a
            // way back at runtime, but the next instruction at
            // `fall_through` is never executed in straight-line flow.
            // For block analysis we treat brsl like an unconditional
            // direct branch; the recompiler tracks the return path
            // separately via the link reg.
            Some(BlockTerminator::UncondDirect { target: *target })
        }
        SpuInstKind::BranchCond { target, .. } => {
            Some(BlockTerminator::CondDirect { taken: *target, fall_through })
        }
        SpuInstKind::BranchIndirect { .. } => Some(BlockTerminator::UncondIndirect),
        SpuInstKind::BranchIndirectLink { .. } => Some(BlockTerminator::UncondIndirect),
        SpuInstKind::BranchIndirectCond { .. } => {
            Some(BlockTerminator::CondIndirect { fall_through })
        }
        // Branch hints don't end blocks (they're NOPs semantically).
        SpuInstKind::BranchHint => None,
        SpuInstKind::Unclassified => Some(BlockTerminator::UnknownOpcode { fall_through }),
        _ => None,
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn write_be(buf: &mut Vec<u8>, w: u32) {
        buf.extend_from_slice(&w.to_be_bytes());
    }

    /// Helper: build a tiny LS containing `code` starting at `entry`.
    fn make_ls(entry: u32, code: &[u32]) -> Vec<u8> {
        let mut ls = vec![0u8; SPU_LS_SIZE];
        let mut buf = Vec::new();
        for w in code { write_be(&mut buf, *w); }
        ls[entry as usize..entry as usize + buf.len()].copy_from_slice(&buf);
        ls
    }

    // --- Single-instruction decode ------------------------------

    #[test]
    fn stop_decodes_with_code() {
        let i = decode_inst(0x0000_1234 & 0x3FFF, 0x100);
        match i.kind {
            SpuInstKind::Stop { code } => assert_eq!(code, 0x1234),
            other => panic!("expected Stop, got {other:?}"),
        }
    }

    #[test]
    fn nop_lnop_sync_dsync_all_decode_to_nop() {
        // primary 0x001 / 0x002 / 0x003 / 0x201 — placed at top 11 bits.
        for opcode in [0x001u32, 0x002, 0x003, 0x201] {
            let raw = opcode << 21;
            let i = decode_inst(raw, 0);
            assert_eq!(i.kind, SpuInstKind::Nop, "opcode 0x{opcode:x}");
        }
    }

    #[test]
    fn br_decodes_to_direct_target() {
        // br +4 (4 words = 16 bytes) from PC=0x100 → target 0x110.
        // primary 0x064, imm16 at bits 9..24.
        let raw = (0x064u32 << 23) | ((4u32 & 0xFFFF) << 7);
        let i = decode_inst(raw, 0x100);
        match i.kind {
            SpuInstKind::BranchDirect { target } => assert_eq!(target, 0x110),
            other => panic!("expected BranchDirect, got {other:?}"),
        }
    }

    #[test]
    fn brnz_decodes_to_cond_branch() {
        let raw = (0x042u32 << 23) | ((2u32 & 0xFFFF) << 7) | 5; // rt=5, +2 words
        let i = decode_inst(raw, 0x100);
        match i.kind {
            SpuInstKind::BranchCond { rt, target } => {
                assert_eq!(rt, 5);
                assert_eq!(target, 0x108);
            }
            other => panic!("expected BranchCond, got {other:?}"),
        }
    }

    #[test]
    fn bi_decodes_to_indirect() {
        // bi r4 — primary 0x1A8 (RR-unary, ra=4)
        let raw = (0x1A8u32 << 21) | (4u32 << 7);
        let i = decode_inst(raw, 0);
        match i.kind {
            SpuInstKind::BranchIndirect { ra } => assert_eq!(ra, 4),
            other => panic!("expected BranchIndirect, got {other:?}"),
        }
    }

    #[test]
    fn decode_lqr_pc_relative_negative_offset() {
        // R5.10b regression — the exact instruction surfaced by the
        // R5.9e.5 v4 diagnostic at pc=0x850 in the spurs_test image:
        //   inst   = 0x33FF2E08
        //   rt     = 8
        //   imm16  = 0xFE5C (signed -420)
        //   target = (0x850 + (-420 << 2)) & 0x3FFF0
        //          = (0x850 - 0x690) & 0x3FFF0 = 0x1C0
        //
        // Pre-R5.10b this returned `Unclassified`; post-R5.10b it
        // resolves to `LoadRel { rt: 8, target_pc: 0x1C0 }`.
        let i = decode_inst(0x33FF2E08, 0x850);
        match i.kind {
            SpuInstKind::LoadRel { rt, target_pc } => {
                assert_eq!(rt, 8, "rt is bottom 7 bits of inst");
                assert_eq!(target_pc, 0x1C0, "PC-relative target");
            }
            other => panic!("expected LoadRel, got {other:?}"),
        }
        assert!(
            !matches!(i.kind, SpuInstKind::Unclassified),
            "LQR must no longer be Unclassified",
        );
    }

    #[test]
    fn decode_cdd_real_v4_opcode() {
        // R5.10d regression — the exact instruction surfaced by the
        // R5.10b v4 diagnostic at pc=0x854 in the spurs_test image:
        //   inst   = 0x3EE00085
        //   p11    = 0x1F7 (CDD, RI7-form, granularity Doubleword)
        //   rt     = 5
        //   ra     = 1
        //   imm7   = 0
        //
        // Pre-R5.10d this returned `Unclassified`; post-R5.10d it
        // resolves to InsertControl with the matching fields.
        let i = decode_inst(0x3EE00085, 0x854);
        match i.kind {
            SpuInstKind::InsertControl {
                rt,
                ra,
                source: InsertControlSource::ImmI7 { imm7 },
                granularity: InsertGranularity::Doubleword,
            } => {
                assert_eq!(rt, 5);
                assert_eq!(ra, 1);
                assert_eq!(imm7, 0);
            }
            other => panic!("expected InsertControl(CDD), got {other:?}"),
        }
        assert!(
            !matches!(i.kind, SpuInstKind::Unclassified),
            "CDD must no longer be Unclassified",
        );
    }

    #[test]
    fn decode_full_c_family_classification() {
        // Lock-in: every one of the 8 C-family primaries decodes to
        // `InsertControl` with the right granularity + source form.
        // Encoding: 11-bit primary at bits 0..10 = top 11 bits of the
        // 32-bit instruction word. We use rt=4, ra=2 throughout, and
        // rb=6 (RR-form) or imm7=3 (RI7-form) so any field-extraction
        // bug surfaces.
        struct Case {
            primary: u32,
            granularity: InsertGranularity,
            is_imm: bool,
        }
        let cases = [
            Case { primary: 0x1D4, granularity: InsertGranularity::Byte,       is_imm: false }, // CBX
            Case { primary: 0x1D5, granularity: InsertGranularity::Halfword,   is_imm: false }, // CHX
            Case { primary: 0x1D6, granularity: InsertGranularity::Word,       is_imm: false }, // CWX
            Case { primary: 0x1D7, granularity: InsertGranularity::Doubleword, is_imm: false }, // CDX
            Case { primary: 0x1F4, granularity: InsertGranularity::Byte,       is_imm: true  }, // CBD
            Case { primary: 0x1F5, granularity: InsertGranularity::Halfword,   is_imm: true  }, // CHD
            Case { primary: 0x1F6, granularity: InsertGranularity::Word,       is_imm: true  }, // CWD
            Case { primary: 0x1F7, granularity: InsertGranularity::Doubleword, is_imm: true  }, // CDD
        ];
        for c in &cases {
            // Build inst: primary << 21 | rb<<14 | ra<<7 | rt.
            // For RI7 form, the rb-slot holds the 7-bit immediate.
            let body = if c.is_imm { 3u32 << 14 } else { 6u32 << 14 };
            let raw = (c.primary << 21) | body | (2u32 << 7) | 4u32;
            let i = decode_inst(raw, 0);
            match i.kind {
                SpuInstKind::InsertControl { rt, ra, source, granularity } => {
                    assert_eq!(rt, 4, "primary=0x{:x}", c.primary);
                    assert_eq!(ra, 2, "primary=0x{:x}", c.primary);
                    assert_eq!(granularity, c.granularity, "primary=0x{:x}", c.primary);
                    match (c.is_imm, source) {
                        (true, InsertControlSource::ImmI7 { imm7 }) => {
                            assert_eq!(imm7, 3, "primary=0x{:x}", c.primary);
                        }
                        (false, InsertControlSource::RegRb { rb }) => {
                            assert_eq!(rb, 6, "primary=0x{:x}", c.primary);
                        }
                        (true, _) => panic!("expected ImmI7 for primary 0x{:x}", c.primary),
                        (false, _) => panic!("expected RegRb for primary 0x{:x}", c.primary),
                    }
                }
                other => panic!("primary 0x{:x} expected InsertControl, got {other:?}", c.primary),
            }
        }
    }

    #[test]
    fn decode_stqa_real_v4_opcode() {
        // R5.10o regression — exact instruction surfaced by the
        // R5.10m v4 diagnostic at pc=0x734 in the spurs_test image:
        //   inst   = 0x20FFFA09 (decimal 553_646_601)
        //   p9     = 0x041 (STQA, RI16 absolute store)
        //   rt     = 9
        //   imm16  = 0xFFF4 (signed -12)
        //   target = (-12 << 2) & 0x3FFF0
        //          = (-48) & 0x3FFF0
        //          = 0xFFFFFFD0 & 0x3FFF0
        //          = 0x3FFD0
        // Pre-R5.10o this returned `Unclassified`; post-R5.10o it
        // resolves to `StoreAbs { rt: 9, target_pc: 0x3FFD0 }`.
        let i = decode_inst(0x20FFFA09, 0x734);
        match i.kind {
            SpuInstKind::StoreAbs { rt, target_pc } => {
                assert_eq!(rt, 9);
                assert_eq!(target_pc, 0x3FFD0);
            }
            other => panic!("expected StoreAbs(STQA), got {other:?}"),
        }
    }

    #[test]
    fn decode_lqa_absolute_negative_offset() {
        // R5.10o regression — one of the 5 v4 LQA sites:
        //   pc=0x07AC inst=0x30FFFC04 lqa r4, imm16=-8
        //   target = (-8 << 2) & 0x3FFF0 = 0xFFFFFFE0 & 0x3FFF0 = 0x3FFE0
        let i = decode_inst(0x30FFFC04, 0x07AC);
        match i.kind {
            SpuInstKind::LoadAbs { rt, target_pc } => {
                assert_eq!(rt, 4);
                assert_eq!(target_pc, 0x3FFE0);
            }
            other => panic!("expected LoadAbs(LQA), got {other:?}"),
        }
    }

    #[test]
    fn decode_lqa_stqa_target_independent_of_pc() {
        // R5.10o anti-regression: LQA / STQA must use PC=0 (absolute)
        // — the same imm16 must produce the SAME target regardless of
        // the pc parameter passed to decode_inst. Anti-regression
        // against accidentally pc-adding the imm16 like LQR/STQR do.
        let raw_lqa  = (0x061u32 << 23) | ((0x0010u16 as u32) << 7) | 5;  // lqa r5, +0x10
        let raw_stqa = (0x041u32 << 23) | ((0x0010u16 as u32) << 7) | 6;  // stqa r6, +0x10
        // Expected target is purely from imm16: (0x10 << 2) & 0x3FFF0 = 0x40.
        for pc in &[0x0u32, 0x100, 0x1234, 0x3FFF8] {
            let il = decode_inst(raw_lqa, *pc);
            let is = decode_inst(raw_stqa, *pc);
            match il.kind {
                SpuInstKind::LoadAbs { target_pc, .. } => {
                    assert_eq!(target_pc, 0x40, "LQA target must be PC-independent (pc=0x{pc:x})");
                }
                other => panic!("LQA pc=0x{pc:x}: {other:?}"),
            }
            match is.kind {
                SpuInstKind::StoreAbs { target_pc, .. } => {
                    assert_eq!(target_pc, 0x40, "STQA target must be PC-independent (pc=0x{pc:x})");
                }
                other => panic!("STQA pc=0x{pc:x}: {other:?}"),
            }
        }
    }

    #[test]
    fn decode_lqr_stqr_remain_pc_relative_after_lqa_stqa_landing() {
        // R5.10o anti-regression: LQR (0x067) and STQR (0x047) must
        // STILL be PC-relative — landing LQA/STQA must NOT have
        // accidentally changed their dispatch to absolute.
        let raw_lqr = (0x067u32 << 23) | ((0x0010u16 as u32) << 7) | 5; // lqr r5, +0x10
        let raw_stqr = (0x047u32 << 23) | ((0x0010u16 as u32) << 7) | 6; // stqr r6, +0x10
        // For PC=0x100, target = (0x100 + (0x10 << 2)) & 0x3FFF0 = 0x140.
        let il = decode_inst(raw_lqr, 0x100);
        let is = decode_inst(raw_stqr, 0x100);
        match il.kind {
            SpuInstKind::LoadRel { target_pc, .. } => {
                assert_eq!(target_pc, 0x140, "LQR must remain PC-relative");
            }
            other => panic!("LQR regressed: {other:?}"),
        }
        match is.kind {
            SpuInstKind::StoreRel { target_pc, .. } => {
                assert_eq!(target_pc, 0x140, "STQR must remain PC-relative");
            }
            other => panic!("STQR regressed: {other:?}"),
        }
    }

    #[test]
    fn decode_rotqmbyi_real_v4_opcode() {
        // R5.10m regression — exact instruction surfaced by the
        // R5.10k v4 diagnostic at pc=0x72C in the spurs_test image:
        //   inst   = 0x3FBF0E96 (decimal 1_069_485_718)
        //   p11    = 0x1FD (ROTQMBYI)
        //   rt     = 22, ra = 29, imm7 = 0x7C (= -4 signed)
        // Pre-R5.10m this returned `Unclassified`; post-R5.10m it
        // resolves to `AluImm7 { rt:22, ra:29, imm7:-4 }` (the i7
        // helper sign-extends; backend discriminates ROTQMBYI vs
        // ROTQBYI/SHLQBYI/SHLQBII on the raw primary).
        let i = decode_inst(0x3FBF0E96, 0x72C);
        match i.kind {
            SpuInstKind::AluImm7 { rt, ra, imm7 } => {
                assert_eq!(rt, 22);
                assert_eq!(ra, 29);
                assert_eq!(imm7, -4, "imm7 0x7C signed = -4");
            }
            other => panic!("expected AluImm7(ROTQMBYI), got {other:?}"),
        }
    }

    #[test]
    fn decode_quadword_shift_family_primaries_resolve_to_aluimm7() {
        // R5.10m: lock-in for the 4 RI7-form quadword shift primaries
        // covered today (SHLQBII 0x1FB, ROTQBYI 0x1FC, ROTQMBYI 0x1FD,
        // SHLQBYI 0x1FF). Each must resolve to AluImm7 (the backend
        // discriminates by raw primary). ROTQBII (0x1F8) and ROTQMBII
        // (0x1F9) are intentionally NOT in the set yet (no v4 use)
        // and must remain Unclassified to avoid silent dispatch.
        for &p11 in &[0x1FBu32, 0x1FC, 0x1FD, 0x1FF] {
            let raw = (p11 << 21) | (5u32 << 14) | (3u32 << 7) | 7u32; // imm7=5, ra=3, rt=7
            let i = decode_inst(raw, 0);
            assert!(
                matches!(i.kind, SpuInstKind::AluImm7 { .. }),
                "p11=0x{p11:03x} must decode to AluImm7, got {:?}", i.kind,
            );
        }
        // Anti-regression: 0x1F8 ROTQBII and 0x1F9 ROTQMBII must NOT
        // be silently accepted. They currently lack a Rust impl.
        for &p11 in &[0x1F8u32, 0x1F9] {
            let raw = (p11 << 21) | (5u32 << 14);
            let i = decode_inst(raw, 0);
            assert!(
                matches!(i.kind, SpuInstKind::Unclassified),
                "p11=0x{p11:03x} must remain Unclassified pre-implementation; got {:?}", i.kind,
            );
        }
    }

    #[test]
    fn decode_andbi_real_v4_opcode_extracts_i8_from_bits_14_21() {
        // R5.10i regression — the exact instruction the R5.10g v4
        // diagnostic surfaced at pc=0x86C in the spurs_test image:
        //   inst   = 0x16080183 (= decimal 369_623_427)
        //   p8     = 0x16 (ANDBI, RI10 byte-immediate)
        //   rt     = 3
        //   ra     = 3
        //   i8     = 0x20 per RPCS3 `bf_t<u32, 14, 8>`
        //            (= LSB-0 bits 14..21 = `(raw >> 14) & 0xFF`)
        //
        // Pre-R5.10i the decoder used `(raw >> 16) & 0xFF` and produced
        // `imm10 = 0x08` — silently wrong (no interpreter byte-imm arm
        // existed to surface a differential mismatch, but the JIT
        // inherited the wrong byte). Post-R5.10i it produces
        // `imm10 = 0x20` matching the C++ reference.
        let i = decode_inst(0x16080183, 0x86C);
        match i.kind {
            SpuInstKind::AluImm { rt, ra, imm10 } => {
                assert_eq!(rt, 3);
                assert_eq!(ra, 3);
                assert_eq!(
                    imm10, 0x20,
                    "i8 must come from bits 14..21, NOT 16..23 (regression-lock)",
                );
            }
            other => panic!("expected AluImm(ANDBI), got {other:?}"),
        }
    }

    #[test]
    fn decode_byte_imm_family_extracts_i8_correctly() {
        // Spot-check the off-by-2-bits regression across all 6 byte-imm
        // primaries with a non-zero `i8` that differs between the right
        // and the wrong extraction. Build the inst by hand: primary at
        // bits 0..7 (MSB-0), i8 at bits 10..17 (MSB-0 = LSB-0 14..21),
        // ra at bits 18..24, rt at bits 25..31. Choose i8=0xA5 because
        // every bit is set/cleared distinctly so any 2-bit shift would
        // produce a visibly different value.
        for &p8 in &[0x06u32, 0x16, 0x46, 0x4E, 0x5E, 0x7E] {
            let i8_field: u32 = 0xA5;
            let raw = (p8 << 24) | (i8_field << 14) | (4u32 << 7) | 5u32; // ra=4, rt=5
            let i = decode_inst(raw, 0);
            match i.kind {
                SpuInstKind::AluImm { rt, ra, imm10 } => {
                    assert_eq!(rt, 5,  "p8=0x{p8:02X}");
                    assert_eq!(ra, 4,  "p8=0x{p8:02X}");
                    // 0xA5 sign-extended via i8 → i16 = 0xFFA5 (-91).
                    let expected = 0xA5u8 as i8 as i16;
                    assert_eq!(
                        imm10, expected,
                        "p8=0x{p8:02X}: i8 extraction must take bits 14..21 (= 0xA5)",
                    );
                }
                other => panic!("p8=0x{p8:02X} expected AluImm, got {other:?}"),
            }
        }
    }

    #[test]
    fn decode_stqr_real_v4_opcode() {
        // R5.10g regression — exact instruction surfaced by the R5.10f
        // v4 diagnostic at pc=0x868 in the spurs_test image:
        //   inst   = 0x23FF2B02 (= decimal 603_925_250, what the
        //            `--ignored` real_trace_diagnostic literally prints)
        //   p9     = 0x047 (STQR, RI16-form)
        //   rt     = 2
        //   imm16  = 0xFE56 (signed -426)
        //   target = (pc + (imm16<<2)) & 0x3FFF0
        //          = (0x868 + (-426 * 4)) & 0x3FFF0
        //          = (0x868 - 0x6A8)     & 0x3FFF0
        //          = 0x1C0               & 0x3FFF0
        //          = 0x1C0
        //
        // Pre-R5.10g this returned `Unclassified`; post-R5.10g it
        // resolves to StoreRel with the matching fields.
        let i = decode_inst(0x23FF2B02, 0x868);
        match i.kind {
            SpuInstKind::StoreRel { rt, target_pc } => {
                assert_eq!(rt, 2);
                assert_eq!(target_pc, 0x1C0);
            }
            other => panic!("expected StoreRel(STQR), got {other:?}"),
        }
        assert!(
            !matches!(i.kind, SpuInstKind::Unclassified),
            "STQR must no longer be Unclassified",
        );
    }

    #[test]
    fn decode_fsmbi_real_v4_opcode() {
        // R5.10f regression — the exact instruction surfaced by the
        // R5.10e v4 diagnostic at pc=0x864 in the spurs_test image:
        //   inst   = 0x32880003 (= decimal 847_773_699, what the
        //            `--ignored` real_trace_diagnostic literally prints)
        //   p9     = 0x065 (FSMBI, RI16-form)
        //   rt     = 3
        //   imm16  = 0x1000
        //
        // Pre-R5.10f this returned `Unclassified`; post-R5.10f it
        // resolves to FormSelectMaskImm with the matching fields.
        let i = decode_inst(0x32880003, 0x864);
        match i.kind {
            SpuInstKind::FormSelectMaskImm { rt, imm16 } => {
                assert_eq!(rt, 3);
                assert_eq!(imm16, 0x1000);
            }
            other => panic!("expected FormSelectMaskImm(FSMBI), got {other:?}"),
        }
        assert!(
            !matches!(i.kind, SpuInstKind::Unclassified),
            "FSMBI must no longer be Unclassified",
        );
    }

    #[test]
    fn decode_fsm_family_rr_classification() {
        // Lock-in: FSM (0x1B4), FSMH (0x1B5), FSMB (0x1B6) all decode
        // to `Unary { rt, ra }` (preserving the JIT codegen pathway for
        // the already-supported FSM at 0x1B4). The interpreter
        // discriminates on the 11-bit primary; the JIT marks 0x1B5/0x1B6
        // as Unsupported and routes them through R5 partial fallback.
        for &p11 in &[0x1B4u32, 0x1B5, 0x1B6] {
            let raw = (p11 << 21) | (3u32 << 7) | 5u32; // rt=5, ra=3
            let i = decode_inst(raw, 0);
            match i.kind {
                SpuInstKind::Unary { rt, ra } => {
                    assert_eq!(rt, 5, "p11=0x{p11:03x}");
                    assert_eq!(ra, 3, "p11=0x{p11:03x}");
                }
                other => panic!("p11=0x{p11:03x} expected Unary, got {other:?}"),
            }
            assert!(
                !matches!(i.kind, SpuInstKind::Unclassified),
                "p11=0x{p11:03x} must not be Unclassified",
            );
        }
    }

    #[test]
    fn bisl_decodes_to_indirect_link() {
        let raw = (0x1A9u32 << 21) | (4u32 << 7) | 7; // rt=7, ra=4
        let i = decode_inst(raw, 0);
        match i.kind {
            SpuInstKind::BranchIndirectLink { rt, ra } => {
                assert_eq!(rt, 7); assert_eq!(ra, 4);
            }
            other => panic!("expected BranchIndirectLink, got {other:?}"),
        }
    }

    #[test]
    fn brsl_decodes_to_direct_link() {
        // brsl rt=2, +3 words — primary 0x066
        let raw = (0x066u32 << 23) | ((3u32 & 0xFFFF) << 7) | 2;
        let i = decode_inst(raw, 0x100);
        match i.kind {
            SpuInstKind::BranchDirectLink { rt, target } => {
                assert_eq!(rt, 2);
                assert_eq!(target, 0x10C);
            }
            other => panic!("expected BranchDirectLink, got {other:?}"),
        }
    }

    #[test]
    fn hbr_hbra_hbrr_all_decode_to_branch_hint() {
        // hbr (11-bit 0x1AC)
        let raw = 0x1ACu32 << 21;
        assert_eq!(decode_inst(raw, 0).kind, SpuInstKind::BranchHint);
        // hbra (7-bit 0x08)
        let raw = 0x08u32 << 25;
        assert_eq!(decode_inst(raw, 0).kind, SpuInstKind::BranchHint);
        // hbrr (7-bit 0x09)
        let raw = 0x09u32 << 25;
        assert_eq!(decode_inst(raw, 0).kind, SpuInstKind::BranchHint);
    }

    #[test]
    fn alu_rr_classifies_correctly() {
        // a rt, ra, rb — canonical SPU primary 0xC0
        let raw = (0x0C0u32 << 21) | (5u32 << 14) | (4u32 << 7) | 3;
        let i = decode_inst(raw, 0);
        match i.kind {
            SpuInstKind::AluRr { rt, ra, rb } => {
                assert_eq!(rt, 3); assert_eq!(ra, 4); assert_eq!(rb, 5);
            }
            other => panic!("expected AluRr, got {other:?}"),
        }
    }

    #[test]
    fn lqd_classifies_as_d_form_load() {
        // lqd r3, +1*16(r4) — primary 0x34, imm10=1, ra=4, rt=3
        let raw = (0x34u32 << 24) | ((1u32 & 0x3FF) << 14) | (4u32 << 7) | 3;
        let i = decode_inst(raw, 0);
        match i.kind {
            SpuInstKind::LoadStoreDForm { rt, ra, offset, is_store } => {
                assert_eq!(rt, 3); assert_eq!(ra, 4);
                assert_eq!(offset, 1); assert!(!is_store);
            }
            other => panic!("expected LoadStoreDForm, got {other:?}"),
        }
    }

    #[test]
    fn unrecognised_opcode_is_unclassified() {
        let raw = 0x0100_0000u32; // not in the iter-1 subset
        assert_eq!(decode_inst(raw, 0).kind, SpuInstKind::Unclassified);
    }

    // --- Block-level analysis -----------------------------------

    #[test]
    fn block_ends_at_stop() {
        let ls = make_ls(0x100, &[
            (0x081u32 << 23) | (0x1234u32 << 7) | 3, // il r3, 0x1234
            0,                                        // stop 0
        ]);
        let block = decode_block(&ls, 0x100).unwrap();
        assert_eq!(block.start_pc, 0x100);
        assert_eq!(block.end_pc, 0x108);
        assert_eq!(block.instructions.len(), 2);
        assert_eq!(block.terminator, BlockTerminator::Stop { code: 0 });
    }

    #[test]
    fn block_ends_at_unconditional_branch() {
        let ls = make_ls(0x100, &[
            (0x081u32 << 23) | (1u32 << 7) | 3,        // il r3, 1
            (0x064u32 << 23) | ((2u32 & 0xFFFF) << 7), // br +2 words → 0x10C
        ]);
        let block = decode_block(&ls, 0x100).unwrap();
        assert_eq!(block.terminator, BlockTerminator::UncondDirect { target: 0x10C });
    }

    #[test]
    fn block_with_branch_hint_does_not_end() {
        let ls = make_ls(0x100, &[
            0x1ACu32 << 21, // hbr — should NOT end block
            0,              // stop 0
        ]);
        let block = decode_block(&ls, 0x100).unwrap();
        assert_eq!(block.instructions.len(), 2);
        assert_eq!(block.terminator, BlockTerminator::Stop { code: 0 });
    }

    // --- Function-level decoding (matches our committed fixtures) -

    /// Mirrors `synthetic_loop.elf`: 8 instructions; should produce
    /// 2 blocks (entry + the loop body re-entered via back-edge),
    /// or 3 blocks depending on how brnz fall-through is counted.
    fn build_loop_program() -> Vec<u8> {
        let il = |rt: u32, imm: u16| (0x081u32 << 23) | ((imm as u32) << 7) | rt;
        let ila = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let a = |rt: u32, ra: u32, rb: u32| (0x0C0u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ai = |rt: u32, ra: u32, imm: u16| (0x1Cu32 << 24) | ((imm as u32 & 0x3FF) << 14) | (ra << 7) | rt;
        let ceqi = |rt: u32, ra: u32, imm: u16| (0x7Cu32 << 24) | ((imm as u32 & 0x3FF) << 14) | (ra << 7) | rt;
        let brnz = |rt: u32, off: i16| (0x042u32 << 23) | ((off as u16 as u32 & 0xFFFF) << 7) | rt;
        let br = |off: i16| (0x064u32 << 23) | ((off as u16 as u32 & 0xFFFF) << 7);
        let _ = il;
        make_ls(0x100, &[
            ila(3, 0),         // 0x100
            ila(4, 1),         // 0x104
            a(3, 3, 4),        // 0x108: loop top
            ai(4, 4, 1),       // 0x10C
            ceqi(5, 4, 11),    // 0x110
            brnz(5, 2),        // 0x114: brnz r5, +2 → 0x11C
            br(-4),            // 0x118: br -4 → 0x108
            0,                 // 0x11C: stop 0
        ])
    }

    #[test]
    fn loop_program_decodes_with_back_edge() {
        let ls = build_loop_program();
        let func = decode_function(&ls, 0x100, 32).unwrap();

        // Expected blocks: 0x100 (setup), 0x108 (loop body), 0x11C (stop).
        let starts: Vec<u32> = func.blocks.keys().copied().collect();
        assert!(starts.contains(&0x100), "blocks: {starts:?}");
        assert!(starts.contains(&0x108), "blocks: {starts:?}");
        assert!(starts.contains(&0x11C), "blocks: {starts:?}");

        // The brnz at 0x114 should produce a 2-way successor; the
        // br at 0x118 should produce 1-way to 0x108 (back-edge).
        let block_108 = &func.blocks[&0x108];
        match &block_108.terminator {
            BlockTerminator::UncondDirect { target } => assert_eq!(*target, 0x108),
            BlockTerminator::CondDirect { .. } => {
                // also acceptable depending on where brnz cuts
            }
            other => panic!("unexpected terminator at 0x108: {other:?}"),
        }
    }

    #[test]
    fn unknown_opcode_terminates_block_safely() {
        let ls = make_ls(0x100, &[
            (0x081u32 << 23) | (1u32 << 7) | 3,  // il
            0x0100_0000,                          // unclassified
            0,                                    // stop
        ]);
        let block = decode_block(&ls, 0x100).unwrap();
        // Should end at the unclassified opcode, not at stop.
        assert_eq!(block.instructions.len(), 2);
        match block.terminator {
            BlockTerminator::UnknownOpcode { fall_through } => {
                assert_eq!(fall_through, 0x108);
            }
            other => panic!("expected UnknownOpcode, got {other:?}"),
        }
    }

    #[test]
    fn entry_pc_unaligned_returns_error() {
        let ls = vec![0u8; 0x1000];
        let r = decode_function(&ls, 0x101, 32);
        assert_eq!(r.unwrap_err(), DecodeError::BadEntryPc(0x101));
    }

    #[test]
    fn entry_pc_out_of_range_returns_error() {
        let ls = vec![0u8; 0x100];
        let r = decode_function(&ls, 0x200, 32);
        assert_eq!(r.unwrap_err(), DecodeError::BadEntryPc(0x200));
    }

    #[test]
    fn function_aggregate_helpers() {
        let ls = build_loop_program();
        let func = decode_function(&ls, 0x100, 32).unwrap();
        assert!(func.block_count() >= 3);
        assert_eq!(func.instruction_count(), 8); // exact match: 8 SPU insts
        let succs = func.all_direct_successors();
        // Setup block falls through into 0x108; loop body branches
        // back to 0x108; brnz fall-through hits 0x118 then br to 0x108
        // and brnz-taken goes to 0x11C.
        assert!(succs.contains(&0x108));
        assert!(succs.contains(&0x11C));
    }

    #[test]
    fn max_blocks_caps_exploration() {
        // Build a chain of 100 forward jumps, each landing at the next
        // 4-byte slot via `br +0`. With max_blocks=5 we should stop early.
        let mut code = Vec::new();
        for _ in 0..100 {
            code.push((0x064u32 << 23) | ((1u32 & 0xFFFF) << 7)); // br +1 word
        }
        let ls = make_ls(0x100, &code);
        let func = decode_function(&ls, 0x100, 5).unwrap();
        assert!(func.block_count() <= 5);
    }
}
