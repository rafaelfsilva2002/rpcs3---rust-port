//! `rpcs3-ppu-interpreter` — PowerPC 64 instruction interpreter.
//!
//! Ports the fetch/decode/execute cycle from
//! `rpcs3/Emu/Cell/PPUInterpreter.cpp`, opcode-by-opcode.
//!
//! ## Iteration 1 — integer arithmetic + logic subset
//!
//! Goal: execute enough of PowerPC to run a hello-world style ELF
//! that only does scalar arithmetic + a `sc` (handled elsewhere).
//!
//! Supported in this crate version:
//! * `addi`, `addis` — immediate add (with the `ra=0 ⇒ literal 0` quirk)
//! * `mulli` — multiply immediate (signed)
//! * `ori`, `oris`, `xori`, `xoris` — immediate bitwise
//! * `andi.`, `andis.` — immediate bitwise with record
//! * `add`, `subf` (XO-form) — register arithmetic
//! * `and`, `or`, `xor` (XO-form) — register bitwise
//! * `nop` — canonical form `ori r0, r0, 0`
//!
//! Every handler:
//! * advances `ppu.cia` by 4 on success (no branches in this subset)
//! * honours `rc` bit only for opcodes that have it in this subset
//!   (`andi.`, `andis.`); full CR0 update lives in Iteration 2
//! * uses wrapping integer math — PPC doesn't trap on overflow unless
//!   `OE=1`, which we ignore in this subset (all handlers use signed/unsigned
//!   two's-complement wrap, same as a JIT)

use rpcs3_memory_backing::{Error as MemError, SparseBackend};
use rpcs3_ppu_opcodes::{primary, ppu_rotate_mask, PpuOpcode};
use rpcs3_ppu_thread::PpuThread;

/// 32-bit rotate mask used by rlwinm/rlwnm.
/// Returns bits `mb..=me` set (wrap-around if me < mb), rest zero.
fn rotate_mask_32(mb: u32, me: u32) -> u32 {
    // Reuse the 64-bit helper on the low 32 positions.
    ppu_rotate_mask(mb.wrapping_add(32), me.wrapping_add(32)) as u32
}

// =====================================================================
// Errors
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Memory read/write error (page not allocated, wrong flags, …).
    Memory(MemError),
    /// No handler for the given opcode yet in this iteration.
    Unimplemented { inst: u32, cia: u32, reason: &'static str },
}

impl From<MemError> for Error {
    fn from(e: MemError) -> Self {
        Self::Memory(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Memory(e) => write!(f, "memory error: {e}"),
            Error::Unimplemented { inst, cia, reason } => {
                write!(f, "unimplemented opcode 0x{inst:08x} at CIA 0x{cia:08x}: {reason}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// Outcome of a single [`step`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// Ordinary instruction: CIA was already advanced by 4 (or set by
    /// a branch). Caller continues.
    Continue,
    /// The `sc` instruction was hit. CIA is already past it.
    /// By LV2 convention, the syscall number is in `r11`, arguments
    /// in `r3..=r10`, return value goes back into `r3`. The caller
    /// (emu core) is responsible for dispatching; this crate stays
    /// agnostic of the syscall family crates.
    Syscall,
}

// =====================================================================
// Helpers
// =====================================================================

/// Read a 32-bit big-endian instruction word from `addr`. PowerPC
/// stores instructions in memory in big-endian order.
fn read_inst_be(mem: &SparseBackend, addr: u32) -> Result<u32, Error> {
    let mut buf = [0u8; 4];
    mem.read(addr, &mut buf).map_err(Error::from)?;
    Ok(u32::from_be_bytes(buf))
}

/// Return `0` when `ra == 0`, else `ppu.gpr[ra]`. Matches the PPC
/// quirk of `addi`/`addis`: register 0 reads as literal zero only in
/// these instructions.
#[inline]
fn ra_or_zero(ppu: &PpuThread, ra: u32) -> u64 {
    if ra == 0 {
        0
    } else {
        ppu.gpr[ra as usize]
    }
}

/// Update CR0 with the signed-comparison flags of `result` vs 0.
/// CR0 layout (PPC bit numbering):
/// * bit 0: LT (result < 0)
/// * bit 1: GT (result > 0)
/// * bit 2: EQ (result == 0)
/// * bit 3: SO (copy of XER.SO)
///
/// `rpcs3-ppu-thread::CrBits` is laid out as 32 single-bit entries
/// (bits[0..=3] = CR0). We write each bit individually.
fn update_cr0(ppu: &mut PpuThread, result: u64) {
    let signed = result as i64;
    let lt = signed < 0;
    let gt = signed > 0;
    let eq = signed == 0;
    let so = ppu.xer.so;
    ppu.cr.0[0] = u8::from(lt);
    ppu.cr.0[1] = u8::from(gt);
    ppu.cr.0[2] = u8::from(eq);
    ppu.cr.0[3] = u8::from(so);
}

/// Write a comparison result to `CR[crfd]`. Used by cmp/cmpi/cmpl/cmpli.
fn set_cr_field(ppu: &mut PpuThread, crfd: u32, lt: bool, gt: bool, eq: bool) {
    let base = (crfd as usize & 0x7) * 4;
    ppu.cr.0[base] = u8::from(lt);
    ppu.cr.0[base + 1] = u8::from(gt);
    ppu.cr.0[base + 2] = u8::from(eq);
    ppu.cr.0[base + 3] = u8::from(ppu.xer.so);
}

/// Decode the SPR number from the 10-bit `spr` field of mtspr/mfspr.
/// PPC encodes the number with halves swapped:
/// `actual_spr = ((encoded & 0x1F) << 5) | ((encoded >> 5) & 0x1F)`.
fn decode_spr(encoded: u32) -> u32 {
    ((encoded & 0x1F) << 5) | ((encoded >> 5) & 0x1F)
}

/// SPR numbers we know about.
pub mod spr {
    pub const XER: u32 = 1;
    pub const LR: u32 = 8;
    pub const CTR: u32 = 9;
}

// =====================================================================
// Step — one instruction
// =====================================================================

// =====================================================================
// Branch condition evaluation
// =====================================================================
//
// PowerPC BO field (5 bits, numbered BO[0..4]):
//   BO[0] (bit 6 of inst, but "first" in PPC bit numbering):
//     0 = decrement CTR and branch if condition matches
//     1 = don't decrement CTR (BO[1] used for CTR comparison)
//   BO[1]: when BO[0]=0 and we branch if CTR!=0 vs ==0:
//     0 = branch if CTR != 0
//     1 = branch if CTR == 0
//   BO[2]: 0 = check CR[BI]; 1 = skip CR check
//   BO[3]: 0 = branch if CR[BI] = 0; 1 = branch if CR[BI] = 1
//   BO[4]: hint (ignored by interpreters)
//
// Common values:
//   20 (0b10100): branch always (both CTR and CR ignored)
//   12 (0b01100): branch if CR[BI] = 1 (true)
//    4 (0b00100): branch if CR[BI] = 0 (false)
//   16 (0b10000): decrement CTR, branch if CTR != 0
//   18 (0b10010): decrement CTR, branch if CTR == 0

fn branch_condition_taken(ppu: &mut PpuThread, bo: u32, bi: u32) -> bool {
    // BO bit masks match the C++ reference (PPUInterpreter.cpp:3205-3216):
    //   bo0 (& 0x10): 1 = skip CR check
    //   bo1 (& 0x08): expected CR[BI] value when CR check is active
    //   bo2 (& 0x04): 1 = skip CTR decrement
    //   bo3 (& 0x02): CTR sense when decrementing (0 = branch if CTR!=0, 1 = CTR==0)
    let bo0 = (bo & 0x10) != 0;
    let bo1 = (bo & 0x08) != 0;
    let bo2 = (bo & 0x04) != 0;
    let bo3 = (bo & 0x02) != 0;

    if !bo2 {
        ppu.ctr = ppu.ctr.wrapping_sub(1);
    }

    // ctr_ok = bo2 | ((CTR != 0) XOR bo3)
    let ctr_ok = bo2 || ((ppu.ctr != 0) ^ bo3);

    // cond_ok = bo0 | (CR[BI] == bo1)
    let cr_bit = ppu.cr.0[bi as usize % 32] != 0;
    let cond_ok = bo0 || (cr_bit == bo1);

    ctr_ok && cond_ok
}

/// Same as `branch_condition_taken` but without CTR decrement. Used by
/// `bcctr` — decrement-CTR forms are invalid there because the branch
/// target *is* CTR.
fn branch_condition_taken_no_ctr(ppu: &PpuThread, bo: u32, bi: u32) -> bool {
    let bo0 = (bo & 0x10) != 0;
    let bo1 = (bo & 0x08) != 0;
    if bo0 {
        return true;
    }
    let cr_bit = ppu.cr.0[bi as usize % 32] != 0;
    cr_bit == bo1
}

// =====================================================================
// FPSCR helpers — minimal subset covering FPRF + exception summary bits
// =====================================================================
//
// FPSCR is a 32-bit register; PPC uses MSB=0 numbering, so "bit N" in
// the manual is `1 << (31 - N)` in our little-endian representation.
// We expose the important slots so handlers can set them without
// duplicating the shift math.
//
// Bits handled here:
//   FX    (PPC bit 0, mask 0x8000_0000): exception summary — sticky,
//         set whenever any exception bit transitions 0→1.
//   FEX   (PPC bit 1, mask 0x4000_0000): enabled exception summary.
//         Tracks VX | OX | UX | ZX | XX when its enable bit is on.
//   VX    (PPC bit 2, mask 0x2000_0000): invalid op summary (NaN).
//   FPRF  (PPC bits 15..19, mask 0x0001_F000): result class + FPCC.
//         Reset on every arithmetic result.

#[allow(dead_code)]
mod fpscr_bits {
    pub const FX: u32 = 0x8000_0000;
    pub const FEX: u32 = 0x4000_0000;
    pub const VX: u32 = 0x2000_0000;
    pub const FPRF_MASK: u32 = 0x0001_F000;

    // FPRF values — bit 4 is "C" (class), bits 0..3 are FPCC
    // (< > = ?). Using the Power ISA encoding table.
    pub const FPRF_QUIET_NAN: u32 = 0b10001 << 12;     // C=1, ?=1
    pub const FPRF_NEG_INF: u32    = 0b01001 << 12;     // <,  normal-class
    pub const FPRF_NEG_NORMAL: u32 = 0b01000 << 12;
    pub const FPRF_NEG_ZERO: u32   = 0b10010 << 12;     // C=1, =
    pub const FPRF_POS_ZERO: u32   = 0b00010 << 12;
    pub const FPRF_POS_NORMAL: u32 = 0b00100 << 12;
    pub const FPRF_POS_INF: u32    = 0b00101 << 12;
}

fn fpscr_update_from_result(ppu: &mut PpuThread, r: f64) {
    use fpscr_bits::*;

    let mut fp = ppu.fpscr & !FPRF_MASK;

    if r.is_nan() {
        fp |= FPRF_QUIET_NAN;
        // NaN result means invalid-operation summary is sticky.
        if (ppu.fpscr & VX) == 0 {
            fp |= FX;
        }
        fp |= VX;
    } else if r == 0.0 {
        // f64 distinguishes +0/-0 via sign bit.
        fp |= if r.is_sign_negative() { FPRF_NEG_ZERO } else { FPRF_POS_ZERO };
    } else if r.is_infinite() {
        fp |= if r < 0.0 { FPRF_NEG_INF } else { FPRF_POS_INF };
    } else {
        fp |= if r < 0.0 { FPRF_NEG_NORMAL } else { FPRF_POS_NORMAL };
    }

    ppu.fpscr = fp;
}

// =====================================================================
// Vector lane helpers (shared with Altivec opcodes)
// =====================================================================

#[inline]
fn split_lanes(v: u128) -> [u32; 4] {
    let b = v.to_be_bytes();
    [
        u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
        u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
        u32::from_be_bytes([b[12], b[13], b[14], b[15]]),
    ]
}

#[inline]
fn join_lanes(lanes: [u32; 4]) -> u128 {
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&lanes[0].to_be_bytes());
    out[4..8].copy_from_slice(&lanes[1].to_be_bytes());
    out[8..12].copy_from_slice(&lanes[2].to_be_bytes());
    out[12..16].copy_from_slice(&lanes[3].to_be_bytes());
    u128::from_be_bytes(out)
}

// =====================================================================
// Effective-address computation for D-form and DS-form loads/stores
// =====================================================================

#[inline]
fn effective_address_d(ppu: &PpuThread, op: PpuOpcode) -> u32 {
    // EA = (ra==0 ? 0 : gpr[ra]) + simm16
    let base = ra_or_zero(ppu, op.ra());
    base.wrapping_add(op.simm16() as i64 as u64) as u32
}

#[inline]
fn effective_address_ds(ppu: &PpuThread, op: PpuOpcode) -> u32 {
    // EA = (ra==0 ? 0 : gpr[ra]) + (ds << 2)
    let base = ra_or_zero(ppu, op.ra());
    base.wrapping_add((op.ds() as i64 as u64).wrapping_shl(2) >> 2 << 2) as u32
}

#[inline]
fn mem_read_u8(mem: &SparseBackend, addr: u32) -> Result<u8, Error> {
    let mut buf = [0u8; 1];
    mem.read(addr, &mut buf).map_err(Error::from)?;
    Ok(buf[0])
}

#[inline]
fn mem_read_u16_be(mem: &SparseBackend, addr: u32) -> Result<u16, Error> {
    let mut buf = [0u8; 2];
    mem.read(addr, &mut buf).map_err(Error::from)?;
    Ok(u16::from_be_bytes(buf))
}

#[inline]
fn mem_read_u32_be(mem: &SparseBackend, addr: u32) -> Result<u32, Error> {
    let mut buf = [0u8; 4];
    mem.read(addr, &mut buf).map_err(Error::from)?;
    Ok(u32::from_be_bytes(buf))
}

#[inline]
fn mem_read_u64_be(mem: &SparseBackend, addr: u32) -> Result<u64, Error> {
    let mut buf = [0u8; 8];
    mem.read(addr, &mut buf).map_err(Error::from)?;
    Ok(u64::from_be_bytes(buf))
}

#[inline]
fn mem_write_u8(mem: &mut SparseBackend, addr: u32, v: u8) -> Result<(), Error> {
    mem.write(addr, &[v]).map_err(Error::from)
}

#[inline]
fn mem_write_u16_be(mem: &mut SparseBackend, addr: u32, v: u16) -> Result<(), Error> {
    mem.write(addr, &v.to_be_bytes()).map_err(Error::from)
}

#[inline]
fn mem_write_u32_be(mem: &mut SparseBackend, addr: u32, v: u32) -> Result<(), Error> {
    mem.write(addr, &v.to_be_bytes()).map_err(Error::from)
}

#[inline]
fn mem_write_u64_be(mem: &mut SparseBackend, addr: u32, v: u64) -> Result<(), Error> {
    mem.write(addr, &v.to_be_bytes()).map_err(Error::from)
}

// =====================================================================
// DS-form extended opcodes (primary 58 = LD family, 62 = STD family)
// =====================================================================

// ld:  primary 58, XO (bits 30..31) = 0
// lwa: primary 58, XO (bits 30..31) = 2
// ldu: primary 58, XO (bits 30..31) = 1
// std:  primary 62, XO (bits 30..31) = 0
// stdu: primary 62, XO (bits 30..31) = 1

/// Fetch, decode, and execute the instruction at `ppu.cia`.
/// On success, `ppu.cia` is advanced by 4 for non-branch instructions.
pub fn step(ppu: &mut PpuThread, mem: &mut SparseBackend) -> Result<StepOutcome, Error> {
    let cia = ppu.cia;
    let inst = read_inst_be(mem, cia)?;
    let op = PpuOpcode::new(inst);

    match op.main() {
        // ---- D-form immediate arithmetic --------------------------
        primary::ADDI => {
            // rd = (ra=0 ? 0 : gpr[ra]) + simm16
            let sum = ra_or_zero(ppu, op.ra())
                .wrapping_add(op.simm16() as i64 as u64);
            ppu.gpr[op.rd() as usize] = sum;
        }
        primary::ADDIS => {
            // rd = (ra=0 ? 0 : gpr[ra]) + (simm16 << 16)
            let imm = (op.simm16() as i64 as u64).wrapping_shl(16);
            let sum = ra_or_zero(ppu, op.ra()).wrapping_add(imm);
            ppu.gpr[op.rd() as usize] = sum;
        }
        primary::MULLI => {
            // rd = gpr[ra] * simm16 (signed, low 64 bits)
            let a = ppu.gpr[op.ra() as usize] as i64;
            let b = op.simm16() as i64;
            ppu.gpr[op.rd() as usize] = a.wrapping_mul(b) as u64;
        }

        // ---- D-form immediate logic -------------------------------
        // Note: ori/xori/and. use rs (= bits 6..10) as source, store in ra (= bits 11..15).
        primary::ORI => {
            ppu.gpr[op.ra() as usize] = ppu.gpr[op.rs() as usize] | u64::from(op.uimm16());
        }
        primary::ORIS => {
            ppu.gpr[op.ra() as usize] = ppu.gpr[op.rs() as usize] | (u64::from(op.uimm16()) << 16);
        }
        primary::XORI => {
            ppu.gpr[op.ra() as usize] = ppu.gpr[op.rs() as usize] ^ u64::from(op.uimm16());
        }
        primary::XORIS => {
            ppu.gpr[op.ra() as usize] = ppu.gpr[op.rs() as usize] ^ (u64::from(op.uimm16()) << 16);
        }
        primary::ANDI_D => {
            // andi. always sets CR0.
            let r = ppu.gpr[op.rs() as usize] & u64::from(op.uimm16());
            ppu.gpr[op.ra() as usize] = r;
            update_cr0(ppu, r);
        }
        primary::ANDIS_D => {
            let r = ppu.gpr[op.rs() as usize] & (u64::from(op.uimm16()) << 16);
            ppu.gpr[op.ra() as usize] = r;
            update_cr0(ppu, r);
        }

        // ---- D-form compare (cmpi/cmpli, primary 10/11) --------
        primary::CMPI => {
            // cmpi crfD, L, rA, SIMM
            // L=0: 32-bit signed; L=1: 64-bit signed.
            let crfd = op.crfd();
            let l = op.l10();
            let imm = op.simm16() as i64;
            if l == 0 {
                let a = ppu.gpr[op.ra() as usize] as i32 as i64;
                set_cr_field(ppu, crfd, a < imm, a > imm, a == imm);
            } else {
                let a = ppu.gpr[op.ra() as usize] as i64;
                set_cr_field(ppu, crfd, a < imm, a > imm, a == imm);
            }
        }
        primary::CMPLI => {
            // cmpli crfD, L, rA, UIMM (unsigned)
            let crfd = op.crfd();
            let l = op.l10();
            let imm = u64::from(op.uimm16());
            if l == 0 {
                let a = ppu.gpr[op.ra() as usize] as u32 as u64;
                set_cr_field(ppu, crfd, a < imm, a > imm, a == imm);
            } else {
                let a = ppu.gpr[op.ra() as usize];
                set_cr_field(ppu, crfd, a < imm, a > imm, a == imm);
            }
        }

        // ---- XO-form register arithmetic/logic --------------------
        primary::XO_ALU => {
            // Extended opcode is bits 22..30 (9 bits) when OE is encoded
            // separately, or bits 21..30 (10 bits) combined. We match
            // the 10-bit XO directly since Rust only cares about the
            // decoded value. XO sits at bit positions 21..30 of the
            // instruction; equivalently (inst >> 1) & 0x3FF.
            let xo = (inst >> 1) & 0x3FF;
            let rc_bit = op.rc() != 0;

            match xo {
                266 => {
                    // add rd, ra, rb
                    let r = ppu.gpr[op.ra() as usize]
                        .wrapping_add(ppu.gpr[op.rb() as usize]);
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                40 => {
                    // subf rd, ra, rb → rd = rb - ra
                    let r = ppu.gpr[op.rb() as usize]
                        .wrapping_sub(ppu.gpr[op.ra() as usize]);
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                235 => {
                    // mullw — signed low 32×32 → 64 (low word)
                    let a = ppu.gpr[op.ra() as usize] as i32 as i64;
                    let b = ppu.gpr[op.rb() as usize] as i32 as i64;
                    let r = a.wrapping_mul(b) as u64;
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                28 => {
                    // and ra, rs, rb
                    let r = ppu.gpr[op.rs() as usize] & ppu.gpr[op.rb() as usize];
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                444 => {
                    // or ra, rs, rb (canonical `mr` = or rs, rs, rs)
                    let r = ppu.gpr[op.rs() as usize] | ppu.gpr[op.rb() as usize];
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                316 => {
                    // xor ra, rs, rb
                    let r = ppu.gpr[op.rs() as usize] ^ ppu.gpr[op.rb() as usize];
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }

                // ---- X-form compare (cmp / cmpl, primary 31) ----
                0 => {
                    // cmp crfD, L, rA, rB — signed
                    let crfd = op.crfd();
                    let l = op.l10();
                    if l == 0 {
                        let a = ppu.gpr[op.ra() as usize] as i32 as i64;
                        let b = ppu.gpr[op.rb() as usize] as i32 as i64;
                        set_cr_field(ppu, crfd, a < b, a > b, a == b);
                    } else {
                        let a = ppu.gpr[op.ra() as usize] as i64;
                        let b = ppu.gpr[op.rb() as usize] as i64;
                        set_cr_field(ppu, crfd, a < b, a > b, a == b);
                    }
                }
                32 => {
                    // cmpl crfD, L, rA, rB — unsigned
                    let crfd = op.crfd();
                    let l = op.l10();
                    if l == 0 {
                        let a = ppu.gpr[op.ra() as usize] as u32 as u64;
                        let b = ppu.gpr[op.rb() as usize] as u32 as u64;
                        set_cr_field(ppu, crfd, a < b, a > b, a == b);
                    } else {
                        let a = ppu.gpr[op.ra() as usize];
                        let b = ppu.gpr[op.rb() as usize];
                        set_cr_field(ppu, crfd, a < b, a > b, a == b);
                    }
                }

                // ---- Shifts ---------------------------------------
                24 => {
                    // slw ra, rs, rb — shift left word (32-bit, zero-extend to 64)
                    let shift = ppu.gpr[op.rb() as usize] & 0x3F;
                    let r = if shift >= 32 {
                        0
                    } else {
                        ((ppu.gpr[op.rs() as usize] as u32) << shift) as u64
                    };
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                536 => {
                    // srw ra, rs, rb — shift right word logical
                    let shift = ppu.gpr[op.rb() as usize] & 0x3F;
                    let r = if shift >= 32 {
                        0
                    } else {
                        ((ppu.gpr[op.rs() as usize] as u32) >> shift) as u64
                    };
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                27 => {
                    // sld ra, rs, rb — shift left doubleword
                    let shift = ppu.gpr[op.rb() as usize] & 0x7F;
                    let r = if shift >= 64 {
                        0
                    } else {
                        ppu.gpr[op.rs() as usize] << shift
                    };
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                539 => {
                    // srd ra, rs, rb — shift right doubleword logical
                    let shift = ppu.gpr[op.rb() as usize] & 0x7F;
                    let r = if shift >= 64 {
                        0
                    } else {
                        ppu.gpr[op.rs() as usize] >> shift
                    };
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                792 => {
                    // sraw ra, rs, rb — shift right word algebraic, sets XER.CA
                    let shift = ppu.gpr[op.rb() as usize] & 0x3F;
                    let rs = ppu.gpr[op.rs() as usize] as i32;
                    let (r, ca) = if shift >= 32 {
                        // All sign bits. CA = 1 iff rs was negative.
                        let signed = (rs >> 31) as i64;
                        (signed as u64, rs < 0)
                    } else {
                        let shifted = (rs >> shift) as i64 as u64;
                        // CA = 1 if rs negative AND any bit shifted out was 1.
                        let mask = if shift == 0 { 0 } else { (1u32 << shift) - 1 };
                        let ca = rs < 0 && (rs as u32 & mask) != 0;
                        (shifted, ca)
                    };
                    ppu.gpr[op.ra() as usize] = r;
                    ppu.xer.ca = ca;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                824 => {
                    // srawi ra, rs, sh — immediate
                    let shift = op.sh32();
                    let rs = ppu.gpr[op.rs() as usize] as i32;
                    let r = (rs >> shift) as i64 as u64;
                    let ca = if shift == 0 {
                        false
                    } else {
                        let mask = (1u32 << shift) - 1;
                        rs < 0 && (rs as u32 & mask) != 0
                    };
                    ppu.gpr[op.ra() as usize] = r;
                    ppu.xer.ca = ca;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }

                // ---- Unary / extension / count-leading-zeros ----
                104 => {
                    // neg rd, ra → rd = -ra (wrapping two's complement)
                    let r = (ppu.gpr[op.ra() as usize] as i64).wrapping_neg() as u64;
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                954 => {
                    // extsb ra, rs → sign-extend byte
                    let r = (ppu.gpr[op.rs() as usize] as i8) as i64 as u64;
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                922 => {
                    // extsh ra, rs → sign-extend halfword
                    let r = (ppu.gpr[op.rs() as usize] as i16) as i64 as u64;
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                986 => {
                    // extsw ra, rs → sign-extend word
                    let r = (ppu.gpr[op.rs() as usize] as i32) as i64 as u64;
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                26 => {
                    // cntlzw ra, rs → count leading zeros of low 32 bits
                    let v = ppu.gpr[op.rs() as usize] as u32;
                    let r = v.leading_zeros() as u64;
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                58 => {
                    // cntlzd ra, rs → count leading zeros of full 64 bits
                    let v = ppu.gpr[op.rs() as usize];
                    let r = v.leading_zeros() as u64;
                    ppu.gpr[op.ra() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }

                // ---- Multiply-high + divide --------------------
                11 => {
                    // mulhwu rd, ra, rb — unsigned high 32 bits of 32x32 → 64
                    let a = ppu.gpr[op.ra() as usize] as u32 as u64;
                    let b = ppu.gpr[op.rb() as usize] as u32 as u64;
                    let r = ((a * b) >> 32) as u32 as u64;
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                75 => {
                    // mulhw rd, ra, rb — signed high 32 bits of 32x32
                    let a = ppu.gpr[op.ra() as usize] as i32 as i64;
                    let b = ppu.gpr[op.rb() as usize] as i32 as i64;
                    let r = ((a * b) >> 32) as i32 as u32 as u64;
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                9 => {
                    // mulhdu rd, ra, rb — unsigned high 64 bits of 64x64 → 128
                    let a = ppu.gpr[op.ra() as usize] as u128;
                    let b = ppu.gpr[op.rb() as usize] as u128;
                    let r = ((a * b) >> 64) as u64;
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                73 => {
                    // mulhd rd, ra, rb — signed high 64 bits of 64x64
                    let a = ppu.gpr[op.ra() as usize] as i64 as i128;
                    let b = ppu.gpr[op.rb() as usize] as i64 as i128;
                    let r = ((a * b) >> 64) as i64 as u64;
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }
                491 => {
                    // divw rd, ra, rb — signed 32/32, undefined on div by 0
                    let a = ppu.gpr[op.ra() as usize] as i32;
                    let b = ppu.gpr[op.rb() as usize] as i32;
                    let r = if b == 0 || (a == i32::MIN && b == -1) {
                        0 // undefined in PPC; we return 0 (matches RPCS3)
                    } else {
                        a.wrapping_div(b)
                    };
                    ppu.gpr[op.rd() as usize] = r as u32 as u64;
                    if rc_bit {
                        update_cr0(ppu, r as u32 as u64);
                    }
                }
                459 => {
                    // divwu rd, ra, rb — unsigned 32/32
                    let a = ppu.gpr[op.ra() as usize] as u32;
                    let b = ppu.gpr[op.rb() as usize] as u32;
                    let r = if b == 0 { 0 } else { a / b };
                    ppu.gpr[op.rd() as usize] = r as u64;
                    if rc_bit {
                        update_cr0(ppu, r as u64);
                    }
                }
                489 => {
                    // divd rd, ra, rb — signed 64/64
                    let a = ppu.gpr[op.ra() as usize] as i64;
                    let b = ppu.gpr[op.rb() as usize] as i64;
                    let r = if b == 0 || (a == i64::MIN && b == -1) {
                        0
                    } else {
                        a.wrapping_div(b)
                    };
                    ppu.gpr[op.rd() as usize] = r as u64;
                    if rc_bit {
                        update_cr0(ppu, r as u64);
                    }
                }
                457 => {
                    // divdu rd, ra, rb — unsigned 64/64
                    let a = ppu.gpr[op.ra() as usize];
                    let b = ppu.gpr[op.rb() as usize];
                    let r = if b == 0 { 0 } else { a / b };
                    ppu.gpr[op.rd() as usize] = r;
                    if rc_bit {
                        update_cr0(ppu, r);
                    }
                }

                // ---- mtspr / mfspr --------------------------------
                467 => {
                    // mtspr spr, rs
                    let spr_num = decode_spr(op.spr());
                    let value = ppu.gpr[op.rs() as usize];
                    match spr_num {
                        spr::LR => ppu.lr = value,
                        spr::CTR => ppu.ctr = value,
                        spr::XER => {
                            // Pack into XER: SO (bit 31), OV (30), CA (29), cnt (0..7)
                            let v = value as u32;
                            ppu.xer.so = (v & (1 << 31)) != 0;
                            ppu.xer.ov = (v & (1 << 30)) != 0;
                            ppu.xer.ca = (v & (1 << 29)) != 0;
                            ppu.xer.cnt = (v & 0x7F) as u8;
                        }
                        _ => {
                            return Err(Error::Unimplemented {
                                inst,
                                cia,
                                reason: "mtspr: SPR not in iteration-4 subset",
                            });
                        }
                    }
                }
                339 => {
                    // mfspr rd, spr
                    let spr_num = decode_spr(op.spr());
                    let v = match spr_num {
                        spr::LR => ppu.lr,
                        spr::CTR => ppu.ctr,
                        spr::XER => ppu.xer.pack_word(),
                        _ => {
                            return Err(Error::Unimplemented {
                                inst,
                                cia,
                                reason: "mfspr: SPR not in iteration-4 subset",
                            });
                        }
                    };
                    ppu.gpr[op.rd() as usize] = v;
                }

                _ => {
                    return Err(Error::Unimplemented {
                        inst,
                        cia,
                        reason: "XO-form opcode not in iteration-4 subset",
                    });
                }
            }
        }

        // ---- D-form loads (zero-extend to 64 bit) ------------------
        primary::LBZ => {
            let ea = effective_address_d(ppu, op);
            let v = mem_read_u8(mem, ea)?;
            ppu.gpr[op.rd() as usize] = v as u64;
        }
        primary::LHZ => {
            let ea = effective_address_d(ppu, op);
            let v = mem_read_u16_be(mem, ea)?;
            ppu.gpr[op.rd() as usize] = v as u64;
        }
        primary::LHA => {
            let ea = effective_address_d(ppu, op);
            let v = mem_read_u16_be(mem, ea)?;
            // Sign-extend halfword to 64 bits.
            ppu.gpr[op.rd() as usize] = v as i16 as i64 as u64;
        }
        primary::LWZ => {
            let ea = effective_address_d(ppu, op);
            let v = mem_read_u32_be(mem, ea)?;
            ppu.gpr[op.rd() as usize] = v as u64;
        }

        // ---- D-form stores -----------------------------------------
        primary::STB => {
            let ea = effective_address_d(ppu, op);
            mem_write_u8(mem, ea, ppu.gpr[op.rs() as usize] as u8)?;
        }
        primary::STH => {
            let ea = effective_address_d(ppu, op);
            mem_write_u16_be(mem, ea, ppu.gpr[op.rs() as usize] as u16)?;
        }
        primary::STW => {
            let ea = effective_address_d(ppu, op);
            mem_write_u32_be(mem, ea, ppu.gpr[op.rs() as usize] as u32)?;
        }

        // ---- DS-form loads/stores (primary 58 / 62) ---------------
        primary::LD => {
            // XO is in bits 30..31 (low 2 bits). 0 = ld.
            let xo = inst & 0x3;
            match xo {
                0 => {
                    // ld rd, ds(ra)
                    let ea = effective_address_ds(ppu, op);
                    ppu.gpr[op.rd() as usize] = mem_read_u64_be(mem, ea)?;
                }
                2 => {
                    // lwa rd, ds(ra) — load word algebraic (sign-extend)
                    let ea = effective_address_ds(ppu, op);
                    let v = mem_read_u32_be(mem, ea)?;
                    ppu.gpr[op.rd() as usize] = v as i32 as i64 as u64;
                }
                _ => {
                    return Err(Error::Unimplemented {
                        inst,
                        cia,
                        reason: "DS-form LD XO not in iteration-2 subset",
                    });
                }
            }
        }
        primary::STD => {
            let xo = inst & 0x3;
            match xo {
                0 => {
                    // std rs, ds(ra)
                    let ea = effective_address_ds(ppu, op);
                    mem_write_u64_be(mem, ea, ppu.gpr[op.rs() as usize])?;
                }
                _ => {
                    return Err(Error::Unimplemented {
                        inst,
                        cia,
                        reason: "DS-form STD XO not in iteration-2 subset",
                    });
                }
            }
        }

        // ---- Branches (b/bl/ba/bla, primary 18) -----------------
        primary::B => {
            // Target = sign-extended `li` (24 bits, << 2) relative to CIA
            // unless AA=1, in which case absolute.
            let target = if op.aa() != 0 {
                op.bt24() as u32
            } else {
                cia.wrapping_add(op.bt24() as u32)
            };
            if op.lk() != 0 {
                ppu.lr = (cia.wrapping_add(4)) as u64;
            }
            ppu.cia = target;
            return Ok(StepOutcome::Continue);
        }

        // ---- Conditional branch (bc, primary 16) ----------------
        primary::BC => {
            let taken = branch_condition_taken(ppu, op.bo(), op.bi());
            if taken {
                let target = if op.aa() != 0 {
                    op.bt14() as u32
                } else {
                    cia.wrapping_add(op.bt14() as u32)
                };
                if op.lk() != 0 {
                    ppu.lr = (cia.wrapping_add(4)) as u64;
                }
                ppu.cia = target;
                return Ok(StepOutcome::Continue);
            }
            // Not taken: fall through (cia += 4 at end).
        }

        // ---- bclr / bcctr (primary 19 XO-form) ------------------
        primary::BCEXT => {
            let xo = (inst >> 1) & 0x3FF;
            match xo {
                16 => {
                    // bclr / blr
                    let taken = branch_condition_taken(ppu, op.bo(), op.bi());
                    if taken {
                        // Target = LR with low 2 bits masked to 0.
                        let target = (ppu.lr as u32) & !0x3;
                        if op.lk() != 0 {
                            ppu.lr = (cia.wrapping_add(4)) as u64;
                        }
                        ppu.cia = target;
                        return Ok(StepOutcome::Continue);
                    }
                }
                528 => {
                    // bcctr / bctr — branches to CTR (never decrements CTR).
                    // BO[2] must be 1 for bcctr (decrement-CTR forms are invalid).
                    let taken = branch_condition_taken_no_ctr(ppu, op.bo(), op.bi());
                    if taken {
                        let target = (ppu.ctr as u32) & !0x3;
                        if op.lk() != 0 {
                            ppu.lr = (cia.wrapping_add(4)) as u64;
                        }
                        ppu.cia = target;
                        return Ok(StepOutcome::Continue);
                    }
                }
                _ => {
                    return Err(Error::Unimplemented {
                        inst,
                        cia,
                        reason: "primary 19 XO opcode not in iteration-3 subset",
                    });
                }
            }
        }

        // ---- rlwinm (primary 21) — rotate-left word, AND with mask
        primary::RLWINM => {
            let sh = op.sh32();
            let mb = op.mb32();
            let me = op.me32();
            let rs32 = ppu.gpr[op.rs() as usize] as u32;
            let rotated = rs32.rotate_left(sh);
            let mask = rotate_mask_32(mb, me);
            let r = (rotated & mask) as u64;
            ppu.gpr[op.ra() as usize] = r;
            if op.rc() != 0 {
                update_cr0(ppu, r);
            }
        }

        // ---- rldicl/rldicr/rldic/rldimi (primary 30, XO in bits 27..=29)
        primary::RLDI => {
            let xo3 = (inst >> 2) & 0x7;
            let sh = op.sh64();
            let mbe = op.mbe64();
            let rs = ppu.gpr[op.rs() as usize];
            let rotated = rs.rotate_left(sh);
            let r = match xo3 {
                0 => {
                    // rldicl — mask with MASK(mb, 63)
                    rotated & ppu_rotate_mask(mbe, 63)
                }
                1 => {
                    // rldicr — mask with MASK(0, me)
                    rotated & ppu_rotate_mask(0, mbe)
                }
                2 => {
                    // rldic — mask with MASK(mb, 63-sh)
                    let me = 63u32.wrapping_sub(sh);
                    rotated & ppu_rotate_mask(mbe, me)
                }
                3 => {
                    // rldimi — insert: (rotated & mask) | (ra & !mask)
                    let mask = ppu_rotate_mask(mbe, 63u32.wrapping_sub(sh));
                    let prev = ppu.gpr[op.ra() as usize];
                    (rotated & mask) | (prev & !mask)
                }
                _ => {
                    return Err(Error::Unimplemented {
                        inst,
                        cia,
                        reason: "primary 30 XO not in iteration-5 subset",
                    });
                }
            };
            ppu.gpr[op.ra() as usize] = r;
            if op.rc() != 0 {
                update_cr0(ppu, r);
            }
        }

        // ---- Floating-point D-form loads/stores ------------------
        // lfs:  load float single, converts to f64 in FPR
        // lfd:  load float double
        // stfs: convert FPR (f64) → f32 → memory
        // stfd: store FPR (f64) to memory
        primary::LFS => {
            let ea = effective_address_d(ppu, op);
            let bits = mem_read_u32_be(mem, ea)?;
            ppu.fpr[op.frd() as usize] = f32::from_bits(bits) as f64;
        }
        primary::LFD => {
            let ea = effective_address_d(ppu, op);
            let bits = mem_read_u64_be(mem, ea)?;
            ppu.fpr[op.frd() as usize] = f64::from_bits(bits);
        }
        primary::STFS => {
            let ea = effective_address_d(ppu, op);
            let v = ppu.fpr[op.frs() as usize] as f32;
            mem_write_u32_be(mem, ea, v.to_bits())?;
        }
        primary::STFD => {
            let ea = effective_address_d(ppu, op);
            let v = ppu.fpr[op.frs() as usize];
            mem_write_u64_be(mem, ea, v.to_bits())?;
        }

        // ---- Floating-point single-precision (primary 59) --------
        // PPC single-precision ops compute in double then round the
        // result to f32 and back — FPR stays u64 (f64) but with a
        // "single" representation. We follow that contract via an
        // explicit `as f32 as f64` round-trip on the result.
        primary::FPS => {
            let xo5 = (inst >> 1) & 0x1F;
            match xo5 {
                21 => {
                    // fadds
                    let a = ppu.fpr[op.fra() as usize];
                    let b = ppu.fpr[op.frb() as usize];
                    let r = (a + b) as f32 as f64;
                    ppu.fpr[op.frd() as usize] = r;
                    fpscr_update_from_result(ppu, r);
                }
                20 => {
                    // fsubs
                    let a = ppu.fpr[op.fra() as usize];
                    let b = ppu.fpr[op.frb() as usize];
                    let r = (a - b) as f32 as f64;
                    ppu.fpr[op.frd() as usize] = r;
                    fpscr_update_from_result(ppu, r);
                }
                25 => {
                    // fmuls: A-form uses frC.
                    let a = ppu.fpr[op.fra() as usize];
                    let c = ppu.fpr[op.frc() as usize];
                    let r = (a * c) as f32 as f64;
                    ppu.fpr[op.frd() as usize] = r;
                    fpscr_update_from_result(ppu, r);
                }
                18 => {
                    // fdivs
                    let a = ppu.fpr[op.fra() as usize];
                    let b = ppu.fpr[op.frb() as usize];
                    let r = (a / b) as f32 as f64;
                    ppu.fpr[op.frd() as usize] = r;
                    fpscr_update_from_result(ppu, r);
                }
                _ => {
                    return Err(Error::Unimplemented {
                        inst,
                        cia,
                        reason: "primary 59 FP XO not in iteration-7 subset",
                    });
                }
            }
        }

        // ---- Floating-point double-precision (primary 63) --------
        // Primary 63 encodes both A-form (5-bit XO in bits 26..30,
        // with frC in bits 21..25) and X-form (10-bit XO in bits
        // 21..30). We dispatch on the 5-bit XO first to catch A-form
        // ops, then fall through to the 10-bit decode for X-form.
        primary::FPD => {
            let xo5 = (inst >> 1) & 0x1F;   // bits 26..30
            let xo10 = (inst >> 1) & 0x3FF; // bits 21..30
            match xo5 {
                21 => {
                    // fadd frD, frA, frB
                    let a = ppu.fpr[op.fra() as usize];
                    let b = ppu.fpr[op.frb() as usize];
                    ppu.fpr[op.frd() as usize] = a + b;
                    fpscr_update_from_result(ppu, ppu.fpr[op.frd() as usize]);
                }
                20 => {
                    // fsub frD, frA, frB
                    let a = ppu.fpr[op.fra() as usize];
                    let b = ppu.fpr[op.frb() as usize];
                    ppu.fpr[op.frd() as usize] = a - b;
                    fpscr_update_from_result(ppu, ppu.fpr[op.frd() as usize]);
                }
                25 => {
                    // fmul frD, frA, frC
                    let a = ppu.fpr[op.fra() as usize];
                    let c = ppu.fpr[op.frc() as usize];
                    ppu.fpr[op.frd() as usize] = a * c;
                    fpscr_update_from_result(ppu, ppu.fpr[op.frd() as usize]);
                }
                18 => {
                    // fdiv frD, frA, frB — IEEE754 semantics on
                    // zero/NaN are exactly what PPC wants; FPSCR.ZX
                    // enable is deferred.
                    let a = ppu.fpr[op.fra() as usize];
                    let b = ppu.fpr[op.frb() as usize];
                    ppu.fpr[op.frd() as usize] = a / b;
                    fpscr_update_from_result(ppu, ppu.fpr[op.frd() as usize]);
                }
                _ => match xo10 {
                    72 => {
                        // fmr frD, frB
                        ppu.fpr[op.frd() as usize] = ppu.fpr[op.frb() as usize];
                    }
                    40 => {
                        // fneg frD, frB
                        ppu.fpr[op.frd() as usize] = -ppu.fpr[op.frb() as usize];
                    }
                    264 => {
                        // fabs frD, frB
                        ppu.fpr[op.frd() as usize] = ppu.fpr[op.frb() as usize].abs();
                    }
                    136 => {
                        // fnabs frD, frB
                        ppu.fpr[op.frd() as usize] = -ppu.fpr[op.frb() as usize].abs();
                    }
                    _ => {
                        return Err(Error::Unimplemented {
                            inst,
                            cia,
                            reason: "primary 63 FP XO not in iteration-6 subset",
                        });
                    }
                },
            }
        }

        // ---- Altivec/VMX (primary 4) ------------------------------
        // Iter-1 subset: 4-lane single-precision float add/sub, and
        // the A-form fused multiply-add variant.
        primary::VX => {
            // VA-form (4 regs: vd, va, vb, vc) uses 6-bit XO at bits
            // 26..31. VX-form (3 regs) uses 11-bit XO at bits 21..31.
            // We try 6-bit first because vmaddfp (XO=46) would
            // otherwise collide with unused 11-bit values.
            let xo6 = inst & 0x3F;
            let xo11 = inst & 0x7FF;

            match xo6 {
                46 => {
                    // vmaddfp vd, va, vb, vc  — vd = (va * vc) + vb
                    let va = split_lanes(ppu.vr[op.va() as usize]);
                    let vb = split_lanes(ppu.vr[op.vb() as usize]);
                    let vc = split_lanes(ppu.vr[op.vc() as usize]);
                    let r = [
                        (f32::from_bits(va[0]) * f32::from_bits(vc[0])
                            + f32::from_bits(vb[0])).to_bits(),
                        (f32::from_bits(va[1]) * f32::from_bits(vc[1])
                            + f32::from_bits(vb[1])).to_bits(),
                        (f32::from_bits(va[2]) * f32::from_bits(vc[2])
                            + f32::from_bits(vb[2])).to_bits(),
                        (f32::from_bits(va[3]) * f32::from_bits(vc[3])
                            + f32::from_bits(vb[3])).to_bits(),
                    ];
                    ppu.vr[op.vd() as usize] = join_lanes(r);
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                43 => {
                    // vperm vd, va, vb, vc  — byte permutation.
                    // For each output byte out[i], selector sel = vc[i] & 0x1F;
                    // if sel < 16: out[i] = va[sel], else out[i] = vb[sel - 16].
                    let a = ppu.vr[op.va() as usize].to_be_bytes();
                    let b = ppu.vr[op.vb() as usize].to_be_bytes();
                    let c = ppu.vr[op.vc() as usize].to_be_bytes();
                    let mut out = [0u8; 16];
                    for i in 0..16 {
                        let sel = (c[i] & 0x1F) as usize;
                        out[i] = if sel < 16 { a[sel] } else { b[sel - 16] };
                    }
                    ppu.vr[op.vd() as usize] = u128::from_be_bytes(out);
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                _ => {}
            }

            match xo11 {
                10 => {
                    // vaddfp vd, va, vb  — 4-lane f32 add.
                    let va = split_lanes(ppu.vr[op.va() as usize]);
                    let vb = split_lanes(ppu.vr[op.vb() as usize]);
                    let r = [
                        (f32::from_bits(va[0]) + f32::from_bits(vb[0])).to_bits(),
                        (f32::from_bits(va[1]) + f32::from_bits(vb[1])).to_bits(),
                        (f32::from_bits(va[2]) + f32::from_bits(vb[2])).to_bits(),
                        (f32::from_bits(va[3]) + f32::from_bits(vb[3])).to_bits(),
                    ];
                    ppu.vr[op.vd() as usize] = join_lanes(r);
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                128 => {
                    // vadduwm vd, va, vb  — 4-lane u32 add (wrap).
                    let va = split_lanes(ppu.vr[op.va() as usize]);
                    let vb = split_lanes(ppu.vr[op.vb() as usize]);
                    let r = [
                        va[0].wrapping_add(vb[0]),
                        va[1].wrapping_add(vb[1]),
                        va[2].wrapping_add(vb[2]),
                        va[3].wrapping_add(vb[3]),
                    ];
                    ppu.vr[op.vd() as usize] = join_lanes(r);
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                1152 => {
                    // vsubuwm vd, va, vb  — 4-lane u32 sub (wrap).
                    let va = split_lanes(ppu.vr[op.va() as usize]);
                    let vb = split_lanes(ppu.vr[op.vb() as usize]);
                    let r = [
                        va[0].wrapping_sub(vb[0]),
                        va[1].wrapping_sub(vb[1]),
                        va[2].wrapping_sub(vb[2]),
                        va[3].wrapping_sub(vb[3]),
                    ];
                    ppu.vr[op.vd() as usize] = join_lanes(r);
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                1028 => {
                    // vand vd, va, vb — bitwise AND on full 128b.
                    ppu.vr[op.vd() as usize] =
                        ppu.vr[op.va() as usize] & ppu.vr[op.vb() as usize];
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                1156 => {
                    // vor vd, va, vb — bitwise OR.
                    ppu.vr[op.vd() as usize] =
                        ppu.vr[op.va() as usize] | ppu.vr[op.vb() as usize];
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                1220 => {
                    // vxor vd, va, vb — bitwise XOR.
                    ppu.vr[op.vd() as usize] =
                        ppu.vr[op.va() as usize] ^ ppu.vr[op.vb() as usize];
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                1284 => {
                    // vnor vd, va, vb — bitwise NOR.
                    ppu.vr[op.vd() as usize] =
                        !(ppu.vr[op.va() as usize] | ppu.vr[op.vb() as usize]);
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                74 => {
                    // vsubfp vd, va, vb  — 4-lane f32 sub.
                    let va = split_lanes(ppu.vr[op.va() as usize]);
                    let vb = split_lanes(ppu.vr[op.vb() as usize]);
                    let r = [
                        (f32::from_bits(va[0]) - f32::from_bits(vb[0])).to_bits(),
                        (f32::from_bits(va[1]) - f32::from_bits(vb[1])).to_bits(),
                        (f32::from_bits(va[2]) - f32::from_bits(vb[2])).to_bits(),
                        (f32::from_bits(va[3]) - f32::from_bits(vb[3])).to_bits(),
                    ];
                    ppu.vr[op.vd() as usize] = join_lanes(r);
                    ppu.cia = cia.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                _ => {
                    return Err(Error::Unimplemented {
                        inst,
                        cia,
                        reason: "primary 4 VX XO not in iteration-1 subset",
                    });
                }
            }
        }

        // ---- sc (syscall) -----------------------------------------
        primary::SC => {
            // `sc` advances CIA past itself and hands control to the
            // emu core for syscall dispatch. Caller reads r11 (syscall
            // number by LV2 convention) and r3..=r10 (arguments) from
            // `ppu`, then writes the return value into r3.
            ppu.cia = ppu.cia.wrapping_add(4);
            return Ok(StepOutcome::Syscall);
        }

        _ => {
            return Err(Error::Unimplemented {
                inst,
                cia,
                reason: "primary opcode not in iteration-2 subset",
            });
        }
    }

    ppu.cia = ppu.cia.wrapping_add(4);
    Ok(StepOutcome::Continue)
}

/// Convenience: execute up to `max_steps` instructions or until
/// [`step`] returns `Syscall` / an error. Returns the number of
/// instructions executed and the last outcome.
pub fn run_n(
    ppu: &mut PpuThread,
    mem: &mut SparseBackend,
    max_steps: usize,
) -> Result<(usize, StepOutcome), Error> {
    for i in 0..max_steps {
        let outcome = step(ppu, mem)?;
        if matches!(outcome, StepOutcome::Syscall) {
            return Ok((i + 1, outcome));
        }
    }
    Ok((max_steps, StepOutcome::Continue))
}

// =====================================================================
// Instruction encoder helpers (for tests)
// =====================================================================

/// Tiny hand-rolled PowerPC encoders, exposed for tests and tooling.
/// Real code should consume an ELF — these are just to build fixtures
/// without pulling in a full assembler.
pub mod encode {
    /// `addi rd, ra, simm16`
    #[must_use]
    pub const fn addi(rd: u32, ra: u32, simm16: i16) -> u32 {
        (14 << 26)
            | ((rd & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | (simm16 as u16 as u32)
    }

    /// `addis rd, ra, simm16`
    #[must_use]
    pub const fn addis(rd: u32, ra: u32, simm16: i16) -> u32 {
        (15 << 26)
            | ((rd & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | (simm16 as u16 as u32)
    }

    /// `ori ra, rs, uimm16`
    #[must_use]
    pub const fn ori(ra: u32, rs: u32, uimm16: u16) -> u32 {
        (24 << 26)
            | ((rs & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | uimm16 as u32
    }

    /// `xori ra, rs, uimm16`
    #[must_use]
    pub const fn xori(ra: u32, rs: u32, uimm16: u16) -> u32 {
        (26 << 26)
            | ((rs & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | uimm16 as u32
    }

    /// `andi. ra, rs, uimm16`
    #[must_use]
    pub const fn andi_dot(ra: u32, rs: u32, uimm16: u16) -> u32 {
        (28 << 26)
            | ((rs & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | uimm16 as u32
    }

    /// `mulli rd, ra, simm16`
    #[must_use]
    pub const fn mulli(rd: u32, ra: u32, simm16: i16) -> u32 {
        (7 << 26)
            | ((rd & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | (simm16 as u16 as u32)
    }

    /// `add rd, ra, rb`
    #[must_use]
    pub const fn add(rd: u32, ra: u32, rb: u32) -> u32 {
        xo_form(31, rd, ra, rb, 266, 0, 0)
    }

    /// `subf rd, ra, rb`  (rd = rb - ra)
    #[must_use]
    pub const fn subf(rd: u32, ra: u32, rb: u32) -> u32 {
        xo_form(31, rd, ra, rb, 40, 0, 0)
    }

    /// `mullw rd, ra, rb`
    #[must_use]
    pub const fn mullw(rd: u32, ra: u32, rb: u32) -> u32 {
        xo_form(31, rd, ra, rb, 235, 0, 0)
    }

    /// `and ra, rs, rb`
    #[must_use]
    pub const fn and(ra: u32, rs: u32, rb: u32) -> u32 {
        xo_form(31, rs, ra, rb, 28, 0, 0)
    }

    /// `or ra, rs, rb`
    #[must_use]
    pub const fn or(ra: u32, rs: u32, rb: u32) -> u32 {
        xo_form(31, rs, ra, rb, 444, 0, 0)
    }

    /// `xor ra, rs, rb`
    #[must_use]
    pub const fn xor(ra: u32, rs: u32, rb: u32) -> u32 {
        xo_form(31, rs, ra, rb, 316, 0, 0)
    }

    /// Canonical `nop` = `ori r0, r0, 0`.
    #[must_use]
    pub const fn nop() -> u32 {
        ori(0, 0, 0)
    }

    // -- Loads / stores (D-form, primary opcode 32..55) ------------

    /// `lbz rd, d(ra)` — load byte, zero-extend.
    #[must_use]
    pub const fn lbz(rd: u32, d: i16, ra: u32) -> u32 {
        d_form(34, rd, ra, d)
    }
    /// `lhz rd, d(ra)` — load halfword, zero-extend.
    #[must_use]
    pub const fn lhz(rd: u32, d: i16, ra: u32) -> u32 {
        d_form(40, rd, ra, d)
    }
    /// `lha rd, d(ra)` — load halfword, sign-extend.
    #[must_use]
    pub const fn lha(rd: u32, d: i16, ra: u32) -> u32 {
        d_form(42, rd, ra, d)
    }
    /// `lwz rd, d(ra)` — load word, zero-extend.
    #[must_use]
    pub const fn lwz(rd: u32, d: i16, ra: u32) -> u32 {
        d_form(32, rd, ra, d)
    }
    /// `stb rs, d(ra)`.
    #[must_use]
    pub const fn stb(rs: u32, d: i16, ra: u32) -> u32 {
        d_form(38, rs, ra, d)
    }
    /// `sth rs, d(ra)`.
    #[must_use]
    pub const fn sth(rs: u32, d: i16, ra: u32) -> u32 {
        d_form(44, rs, ra, d)
    }
    /// `stw rs, d(ra)`.
    #[must_use]
    pub const fn stw(rs: u32, d: i16, ra: u32) -> u32 {
        d_form(36, rs, ra, d)
    }

    // -- DS-form (primary 58 / 62) ---------------------------------

    /// `ld rd, ds*4(ra)` where `ds` is a signed 14-bit value.
    /// Encoded offset is `ds << 2`.
    #[must_use]
    pub const fn ld(rd: u32, ds_field: i16, ra: u32) -> u32 {
        ds_form(58, rd, ra, ds_field, 0)
    }
    /// `std rs, ds*4(ra)`.
    #[must_use]
    pub const fn std(rs: u32, ds_field: i16, ra: u32) -> u32 {
        ds_form(62, rs, ra, ds_field, 0)
    }

    // -- sc (system call) ------------------------------------------

    /// `sc 0` — canonical system call.
    #[must_use]
    pub const fn sc() -> u32 {
        // primary 17, LEV=0, final 2 bits = 10 (required by the encoding).
        (17 << 26) | 0b10
    }

    // -- Branches --------------------------------------------------

    /// `b displacement` — unconditional PC-relative branch
    /// (AA=0, LK=0). `displacement` must be a multiple of 4.
    #[must_use]
    pub const fn b(displacement: i32) -> u32 {
        // Primary 18, li = displacement >> 2, AA=0, LK=0
        let li = (displacement >> 2) as u32 & 0x00FF_FFFF;
        (18 << 26) | (li << 2)
    }

    /// `bl displacement` — branch-and-link (AA=0, LK=1).
    #[must_use]
    pub const fn bl(displacement: i32) -> u32 {
        b(displacement) | 1
    }

    /// `ba absolute_address` — absolute branch (AA=1, LK=0).
    /// Address must be a multiple of 4 and fit in signed 26 bits.
    #[must_use]
    pub const fn ba(abs: i32) -> u32 {
        b(abs) | 0b10
    }

    /// `bc BO, BI, displacement` — conditional branch (AA=0, LK=0).
    #[must_use]
    pub const fn bc(bo: u32, bi: u32, displacement: i16) -> u32 {
        let bd = ((displacement >> 2) as u16 as u32) & 0x3FFF;
        (16 << 26) | ((bo & 0x1F) << 21) | ((bi & 0x1F) << 16) | (bd << 2)
    }

    /// `blr` = `bclr 20, 0, 0` — return via LR.
    #[must_use]
    pub const fn blr() -> u32 {
        // primary 19, BO=20, BI=0, XO=16, LK=0
        (19 << 26) | (20 << 21) | (16 << 1)
    }

    /// `bctr` = `bcctr 20, 0, 0` — branch via CTR.
    #[must_use]
    pub const fn bctr() -> u32 {
        // primary 19, BO=20, BI=0, XO=528, LK=0
        (19 << 26) | (20 << 21) | (528 << 1)
    }

    /// `bctrl` = `bcctr 20, 0, 1` — branch via CTR with link.
    #[must_use]
    pub const fn bctrl() -> u32 {
        bctr() | 1
    }

    // -- Compare (D-form: cmpi/cmpli; X-form: cmp/cmpl) -----------

    /// `cmpi crfD, L, rA, simm16` — signed compare immediate.
    #[must_use]
    pub const fn cmpi(crfd: u32, l: u32, ra: u32, simm16: i16) -> u32 {
        (11 << 26)
            | ((crfd & 0x7) << 23)
            | ((l & 1) << 21)
            | ((ra & 0x1F) << 16)
            | (simm16 as u16 as u32)
    }

    /// `cmpli crfD, L, rA, uimm16` — unsigned compare immediate.
    #[must_use]
    pub const fn cmpli(crfd: u32, l: u32, ra: u32, uimm16: u16) -> u32 {
        (10 << 26)
            | ((crfd & 0x7) << 23)
            | ((l & 1) << 21)
            | ((ra & 0x1F) << 16)
            | uimm16 as u32
    }

    /// `cmp crfD, L, rA, rB` — signed register compare.
    #[must_use]
    pub const fn cmp(crfd: u32, l: u32, ra: u32, rb: u32) -> u32 {
        // XO = 0, primary 31.
        (31 << 26)
            | ((crfd & 0x7) << 23)
            | ((l & 1) << 21)
            | ((ra & 0x1F) << 16)
            | ((rb & 0x1F) << 11)
            | (0 << 1)
    }

    /// `cmpl crfD, L, rA, rB` — unsigned register compare.
    #[must_use]
    pub const fn cmpl(crfd: u32, l: u32, ra: u32, rb: u32) -> u32 {
        (31 << 26)
            | ((crfd & 0x7) << 23)
            | ((l & 1) << 21)
            | ((ra & 0x1F) << 16)
            | ((rb & 0x1F) << 11)
            | (32 << 1)
    }

    // -- Shifts ---------------------------------------------------

    /// `slw ra, rs, rb` — shift left word.
    #[must_use]
    pub const fn slw(ra: u32, rs: u32, rb: u32) -> u32 {
        shift_xo_form(rs, ra, rb, 24)
    }
    /// `srw ra, rs, rb` — shift right word logical.
    #[must_use]
    pub const fn srw(ra: u32, rs: u32, rb: u32) -> u32 {
        shift_xo_form(rs, ra, rb, 536)
    }
    /// `sraw ra, rs, rb` — shift right word algebraic, sets XER.CA.
    #[must_use]
    pub const fn sraw(ra: u32, rs: u32, rb: u32) -> u32 {
        shift_xo_form(rs, ra, rb, 792)
    }
    /// `srawi ra, rs, sh` — shift right word algebraic immediate.
    #[must_use]
    pub const fn srawi(ra: u32, rs: u32, sh: u32) -> u32 {
        (31 << 26)
            | ((rs & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | ((sh & 0x1F) << 11)
            | (824 << 1)
    }
    /// `sld ra, rs, rb` — shift left doubleword.
    #[must_use]
    pub const fn sld(ra: u32, rs: u32, rb: u32) -> u32 {
        shift_xo_form(rs, ra, rb, 27)
    }
    /// `srd ra, rs, rb` — shift right doubleword logical.
    #[must_use]
    pub const fn srd(ra: u32, rs: u32, rb: u32) -> u32 {
        shift_xo_form(rs, ra, rb, 539)
    }

    // -- mtspr / mfspr -------------------------------------------

    /// Encode the 10-bit `spr` field with the halves swapped
    /// (PPC encoding convention).
    const fn encode_spr(spr: u32) -> u32 {
        ((spr & 0x1F) << 5) | ((spr >> 5) & 0x1F)
    }

    /// `mtspr spr, rs`.
    #[must_use]
    pub const fn mtspr(spr: u32, rs: u32) -> u32 {
        (31 << 26)
            | ((rs & 0x1F) << 21)
            | (encode_spr(spr) << 11)
            | (467 << 1)
    }

    /// `mfspr rd, spr`.
    #[must_use]
    pub const fn mfspr(rd: u32, spr: u32) -> u32 {
        (31 << 26)
            | ((rd & 0x1F) << 21)
            | (encode_spr(spr) << 11)
            | (339 << 1)
    }

    /// `mtlr rs` — canonical alias for `mtspr LR, rs`.
    #[must_use]
    pub const fn mtlr(rs: u32) -> u32 {
        mtspr(8, rs)
    }
    /// `mflr rd`.
    #[must_use]
    pub const fn mflr(rd: u32) -> u32 {
        mfspr(rd, 8)
    }
    /// `mtctr rs`.
    #[must_use]
    pub const fn mtctr(rs: u32) -> u32 {
        mtspr(9, rs)
    }
    /// `mfctr rd`.
    #[must_use]
    pub const fn mfctr(rd: u32) -> u32 {
        mfspr(rd, 9)
    }

    const fn shift_xo_form(rs: u32, ra: u32, rb: u32, xo: u32) -> u32 {
        (31 << 26)
            | ((rs & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | ((rb & 0x1F) << 11)
            | ((xo & 0x3FF) << 1)
    }

    // -- Unary + extension + count-leading-zeros ------------------

    /// `neg rd, ra`.
    #[must_use]
    pub const fn neg(rd: u32, ra: u32) -> u32 {
        xo31_dar(rd, ra, 0, 104)
    }
    /// `extsb ra, rs`.
    #[must_use]
    pub const fn extsb(ra: u32, rs: u32) -> u32 {
        xo31_dar(rs, ra, 0, 954)
    }
    /// `extsh ra, rs`.
    #[must_use]
    pub const fn extsh(ra: u32, rs: u32) -> u32 {
        xo31_dar(rs, ra, 0, 922)
    }
    /// `extsw ra, rs`.
    #[must_use]
    pub const fn extsw(ra: u32, rs: u32) -> u32 {
        xo31_dar(rs, ra, 0, 986)
    }
    /// `cntlzw ra, rs`.
    #[must_use]
    pub const fn cntlzw(ra: u32, rs: u32) -> u32 {
        xo31_dar(rs, ra, 0, 26)
    }
    /// `cntlzd ra, rs`.
    #[must_use]
    pub const fn cntlzd(ra: u32, rs: u32) -> u32 {
        xo31_dar(rs, ra, 0, 58)
    }

    // -- Multiply-high + divide -----------------------------------

    #[must_use]
    pub const fn mulhwu(rd: u32, ra: u32, rb: u32) -> u32 {
        xo31_dar(rd, ra, rb, 11)
    }
    #[must_use]
    pub const fn mulhw(rd: u32, ra: u32, rb: u32) -> u32 {
        xo31_dar(rd, ra, rb, 75)
    }
    #[must_use]
    pub const fn mulhdu(rd: u32, ra: u32, rb: u32) -> u32 {
        xo31_dar(rd, ra, rb, 9)
    }
    #[must_use]
    pub const fn mulhd(rd: u32, ra: u32, rb: u32) -> u32 {
        xo31_dar(rd, ra, rb, 73)
    }
    #[must_use]
    pub const fn divw(rd: u32, ra: u32, rb: u32) -> u32 {
        xo31_dar(rd, ra, rb, 491)
    }
    #[must_use]
    pub const fn divwu(rd: u32, ra: u32, rb: u32) -> u32 {
        xo31_dar(rd, ra, rb, 459)
    }
    #[must_use]
    pub const fn divd(rd: u32, ra: u32, rb: u32) -> u32 {
        xo31_dar(rd, ra, rb, 489)
    }
    #[must_use]
    pub const fn divdu(rd: u32, ra: u32, rb: u32) -> u32 {
        xo31_dar(rd, ra, rb, 457)
    }

    const fn xo31_dar(rd: u32, ra: u32, rb: u32, xo: u32) -> u32 {
        (31 << 26)
            | ((rd & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | ((rb & 0x1F) << 11)
            | ((xo & 0x3FF) << 1)
    }

    // -- rlwinm (primary 21) --------------------------------------

    /// `rlwinm ra, rs, sh, mb, me` — rotate-left word, AND with mask.
    #[must_use]
    pub const fn rlwinm(ra: u32, rs: u32, sh: u32, mb: u32, me: u32) -> u32 {
        (21 << 26)
            | ((rs & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | ((sh & 0x1F) << 11)
            | ((mb & 0x1F) << 6)
            | ((me & 0x1F) << 1)
    }

    // -- rldicl/rldicr/rldic (primary 30, split sh+mb fields) -----

    /// `rldicl ra, rs, sh, mb` — doubleword rotate, clear left.
    #[must_use]
    pub const fn rldicl(ra: u32, rs: u32, sh: u32, mb: u32) -> u32 {
        md_form(ra, rs, sh, mb, 0)
    }
    /// `rldicr ra, rs, sh, me`.
    #[must_use]
    pub const fn rldicr(ra: u32, rs: u32, sh: u32, me: u32) -> u32 {
        md_form(ra, rs, sh, me, 1)
    }
    /// `rldic ra, rs, sh, mb`.
    #[must_use]
    pub const fn rldic(ra: u32, rs: u32, sh: u32, mb: u32) -> u32 {
        md_form(ra, rs, sh, mb, 2)
    }

    const fn md_form(ra: u32, rs: u32, sh: u32, mb_or_me: u32, xo3: u32) -> u32 {
        // Encoding:
        //   primary 30
        //   rs at 6..=10
        //   ra at 11..=15
        //   SH[0..=4] at 16..=20
        //   MB[0..=4] at 21..=25
        //   MB[5] at bit 26
        //   XO at 27..=29
        //   SH[5] at bit 30
        //   Rc at bit 31
        let sh_lo = sh & 0x1F;
        let sh_hi = (sh >> 5) & 0x1;
        let mbe_lo = mb_or_me & 0x1F;
        let mbe_hi = (mb_or_me >> 5) & 0x1;

        (30 << 26)
            | ((rs & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | (sh_lo << 11)
            | (mbe_lo << 6)
            | (mbe_hi << 5)
            | ((xo3 & 0x7) << 2)
            | (sh_hi << 1)
    }

    // -- Shared form helpers ---------------------------------------

    const fn d_form(primary: u32, dst: u32, ra: u32, d: i16) -> u32 {
        (primary << 26)
            | ((dst & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | (d as u16 as u32)
    }

    const fn ds_form(primary: u32, dst: u32, ra: u32, ds: i16, xo_lo2: u32) -> u32 {
        // DS field is 14 bits, sign-extended; occupies bits 16..=29,
        // with XO in bits 30..=31.
        let ds_mask = (ds as u16 as u32) & 0x3FFF;
        (primary << 26)
            | ((dst & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | (ds_mask << 2)
            | (xo_lo2 & 0x3)
    }

    const fn xo_form(primary: u32, rd: u32, ra: u32, rb: u32, xo: u32, oe: u32, rc: u32) -> u32 {
        (primary << 26)
            | ((rd & 0x1F) << 21)
            | ((ra & 0x1F) << 16)
            | ((rb & 0x1F) << 11)
            | ((oe & 1) << 10)
            | ((xo & 0x1FF) << 1)
            | (rc & 1)
    }

    // ---- Floating-point D-form loads/stores -------------------------

    /// `lfs frD, simm16(ra)`
    #[must_use]
    pub const fn lfs(frd: u32, ra: u32, simm16: i16) -> u32 {
        (48 << 26) | ((frd & 0x1F) << 21) | ((ra & 0x1F) << 16) | (simm16 as u16 as u32)
    }

    /// `lfd frD, simm16(ra)`
    #[must_use]
    pub const fn lfd(frd: u32, ra: u32, simm16: i16) -> u32 {
        (50 << 26) | ((frd & 0x1F) << 21) | ((ra & 0x1F) << 16) | (simm16 as u16 as u32)
    }

    /// `stfs frS, simm16(ra)`
    #[must_use]
    pub const fn stfs(frs: u32, ra: u32, simm16: i16) -> u32 {
        (52 << 26) | ((frs & 0x1F) << 21) | ((ra & 0x1F) << 16) | (simm16 as u16 as u32)
    }

    /// `stfd frS, simm16(ra)`
    #[must_use]
    pub const fn stfd(frs: u32, ra: u32, simm16: i16) -> u32 {
        (54 << 26) | ((frs & 0x1F) << 21) | ((ra & 0x1F) << 16) | (simm16 as u16 as u32)
    }

    // ---- Floating-point arithmetic (primary 63) ---------------------

    // A-form: primary (6) | frD (5) | frA (5) | frB (5) | frC (5) | XO (5) | Rc (1)
    const fn fp_a_form(frd: u32, fra: u32, frb: u32, frc: u32, xo: u32, rc: u32) -> u32 {
        (63 << 26)
            | ((frd & 0x1F) << 21)
            | ((fra & 0x1F) << 16)
            | ((frb & 0x1F) << 11)
            | ((frc & 0x1F) << 6)
            | ((xo & 0x1F) << 1)
            | (rc & 1)
    }

    // X-form unary: primary (6) | frD (5) | 0 (5) | frB (5) | XO (10) | Rc (1)
    const fn fp_x_form(frd: u32, frb: u32, xo: u32, rc: u32) -> u32 {
        (63 << 26)
            | ((frd & 0x1F) << 21)
            | ((frb & 0x1F) << 11)
            | ((xo & 0x3FF) << 1)
            | (rc & 1)
    }

    /// `fadd frD, frA, frB` — double add.
    #[must_use]
    pub const fn fadd(frd: u32, fra: u32, frb: u32) -> u32 {
        fp_a_form(frd, fra, frb, 0, 21, 0)
    }

    /// `fsub frD, frA, frB` — double sub.
    #[must_use]
    pub const fn fsub(frd: u32, fra: u32, frb: u32) -> u32 {
        fp_a_form(frd, fra, frb, 0, 20, 0)
    }

    /// `fmul frD, frA, frC` — double multiply (A-form uses `frC`).
    #[must_use]
    pub const fn fmul(frd: u32, fra: u32, frc: u32) -> u32 {
        fp_a_form(frd, fra, 0, frc, 25, 0)
    }

    /// `fdiv frD, frA, frB` — double divide.
    #[must_use]
    pub const fn fdiv(frd: u32, fra: u32, frb: u32) -> u32 {
        fp_a_form(frd, fra, frb, 0, 18, 0)
    }

    /// `fmr frD, frB` — move.
    #[must_use]
    pub const fn fmr(frd: u32, frb: u32) -> u32 {
        fp_x_form(frd, frb, 72, 0)
    }

    /// `fneg frD, frB` — negate.
    #[must_use]
    pub const fn fneg(frd: u32, frb: u32) -> u32 {
        fp_x_form(frd, frb, 40, 0)
    }

    /// `fabs frD, frB` — absolute value.
    #[must_use]
    pub const fn fabs(frd: u32, frb: u32) -> u32 {
        fp_x_form(frd, frb, 264, 0)
    }

    /// `fnabs frD, frB` — negative absolute value.
    #[must_use]
    pub const fn fnabs(frd: u32, frb: u32) -> u32 {
        fp_x_form(frd, frb, 136, 0)
    }

    // ---- Single-precision arithmetic (primary 59) -------------------

    const fn fp_sp_a_form(frd: u32, fra: u32, frb: u32, frc: u32, xo: u32) -> u32 {
        (59 << 26)
            | ((frd & 0x1F) << 21)
            | ((fra & 0x1F) << 16)
            | ((frb & 0x1F) << 11)
            | ((frc & 0x1F) << 6)
            | ((xo & 0x1F) << 1)
    }

    /// `fadds frD, frA, frB` — single-precision add.
    #[must_use]
    pub const fn fadds(frd: u32, fra: u32, frb: u32) -> u32 {
        fp_sp_a_form(frd, fra, frb, 0, 21)
    }

    /// `fsubs frD, frA, frB` — single-precision sub.
    #[must_use]
    pub const fn fsubs(frd: u32, fra: u32, frb: u32) -> u32 {
        fp_sp_a_form(frd, fra, frb, 0, 20)
    }

    /// `fmuls frD, frA, frC` — single-precision multiply.
    #[must_use]
    pub const fn fmuls(frd: u32, fra: u32, frc: u32) -> u32 {
        fp_sp_a_form(frd, fra, 0, frc, 25)
    }

    /// `fdivs frD, frA, frB` — single-precision divide.
    #[must_use]
    pub const fn fdivs(frd: u32, fra: u32, frb: u32) -> u32 {
        fp_sp_a_form(frd, fra, frb, 0, 18)
    }

    // ---- Altivec/VMX (primary 4) ---------------------------------

    /// `vaddfp vD, vA, vB` — 4-lane f32 add. VX-form, 11-bit XO = 10.
    #[must_use]
    pub const fn vaddfp(vd: u32, va: u32, vb: u32) -> u32 {
        (4 << 26) | ((vd & 0x1F) << 21) | ((va & 0x1F) << 16) | ((vb & 0x1F) << 11) | 10
    }

    /// `vsubfp vD, vA, vB` — 4-lane f32 sub. XO = 74.
    #[must_use]
    pub const fn vsubfp(vd: u32, va: u32, vb: u32) -> u32 {
        (4 << 26) | ((vd & 0x1F) << 21) | ((va & 0x1F) << 16) | ((vb & 0x1F) << 11) | 74
    }

    /// `vmaddfp vD, vA, vB, vC` — vd = va*vc + vb. VA-form, 6-bit XO = 46.
    /// Layout: primary(6) | vd(5) | va(5) | vb(5) | vc(5) | xo(6).
    #[must_use]
    pub const fn vmaddfp(vd: u32, va: u32, vb: u32, vc: u32) -> u32 {
        (4 << 26)
            | ((vd & 0x1F) << 21)
            | ((va & 0x1F) << 16)
            | ((vb & 0x1F) << 11)
            | ((vc & 0x1F) << 6)
            | 46
    }

    // ---- Integer VMX (primary 4, 11-bit XO) ---------------------

    const fn vx_form(xo: u32, vd: u32, va: u32, vb: u32) -> u32 {
        (4 << 26)
            | ((vd & 0x1F) << 21)
            | ((va & 0x1F) << 16)
            | ((vb & 0x1F) << 11)
            | (xo & 0x7FF)
    }

    /// `vadduwm vD, vA, vB` — 4-lane u32 add (modulo).
    #[must_use]
    pub const fn vadduwm(vd: u32, va: u32, vb: u32) -> u32 { vx_form(128, vd, va, vb) }
    /// `vsubuwm vD, vA, vB` — 4-lane u32 sub (modulo).
    #[must_use]
    pub const fn vsubuwm(vd: u32, va: u32, vb: u32) -> u32 { vx_form(1152, vd, va, vb) }
    /// `vand vD, vA, vB` — bitwise AND.
    #[must_use]
    pub const fn vand(vd: u32, va: u32, vb: u32) -> u32 { vx_form(1028, vd, va, vb) }
    /// `vor vD, vA, vB` — bitwise OR.
    #[must_use]
    pub const fn vor(vd: u32, va: u32, vb: u32) -> u32 { vx_form(1156, vd, va, vb) }
    /// `vxor vD, vA, vB` — bitwise XOR.
    #[must_use]
    pub const fn vxor(vd: u32, va: u32, vb: u32) -> u32 { vx_form(1220, vd, va, vb) }
    /// `vnor vD, vA, vB` — bitwise NOR.
    #[must_use]
    pub const fn vnor(vd: u32, va: u32, vb: u32) -> u32 { vx_form(1284, vd, va, vb) }

    /// `vperm vD, vA, vB, vC` — byte permutation (VA-form 6-bit XO = 43).
    #[must_use]
    pub const fn vperm(vd: u32, va: u32, vb: u32, vc: u32) -> u32 {
        (4 << 26)
            | ((vd & 0x1F) << 21)
            | ((va & 0x1F) << 16)
            | ((vb & 0x1F) << 11)
            | ((vc & 0x1F) << 6)
            | 43
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rpcs3_memory::PageFlags;
    use rpcs3_ppu_thread::PPU_ID_BASE;

    const PROG_BASE: u32 = 0x1000;

    fn make_env(program: &[u32]) -> (PpuThread, SparseBackend) {
        let mut mem = SparseBackend::new();
        // Allocate one page at 0x1000 for the program.
        mem.alloc_at(PROG_BASE, 0x1000, PageFlags::READABLE | PageFlags::EXECUTABLE | PageFlags::WRITABLE)
            .unwrap();
        // Write instructions (big-endian).
        let mut bytes = Vec::with_capacity(program.len() * 4);
        for inst in program {
            bytes.extend_from_slice(&inst.to_be_bytes());
        }
        mem.write(PROG_BASE, &bytes).unwrap();

        let mut ppu = PpuThread::new(PPU_ID_BASE);
        ppu.cia = PROG_BASE;
        (ppu, mem)
    }

    fn step_ok(ppu: &mut PpuThread, mem: &mut SparseBackend) {
        let outcome = step(ppu, mem).expect("step should succeed");
        assert_eq!(outcome, StepOutcome::Continue);
    }

    /// Execute a single step and return the outcome — for tests that
    /// explicitly care about the outcome variant (sc, branches).
    fn step_outcome(ppu: &mut PpuThread, mem: &mut SparseBackend) -> StepOutcome {
        step(ppu, mem).expect("step should succeed")
    }

    /// Add a data page at `addr..(addr+4KB)` readable+writable.
    fn alloc_data(mem: &mut SparseBackend, addr: u32) {
        mem.alloc_at(addr, 0x1000, PageFlags::READABLE | PageFlags::WRITABLE).unwrap();
    }

    // -- addi / addis ---------------------------------------------

    #[test]
    fn addi_with_ra_zero_loads_immediate() {
        let (mut ppu, mut mem) = make_env(&[encode::addi(3, 0, 42)]);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 42);
        assert_eq!(ppu.cia, PROG_BASE + 4);
    }

    #[test]
    fn addi_with_ra_zero_and_negative_imm() {
        let (mut ppu, mut mem) = make_env(&[encode::addi(3, 0, -1)]);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as i64, -1);
    }

    #[test]
    fn addi_reads_ra_when_nonzero() {
        let (mut ppu, mut mem) = make_env(&[encode::addi(3, 5, 7)]);
        ppu.gpr[5] = 100;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 107);
    }

    #[test]
    fn addis_shifts_immediate_by_16() {
        let (mut ppu, mut mem) = make_env(&[encode::addis(4, 0, 1)]);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[4], 0x0001_0000);
    }

    #[test]
    fn addi_chain_builds_large_value() {
        // Common PPC idiom to construct 0x12345678 in rN:
        //   addis rN, 0, 0x1234
        //   addi  rN, rN, 0x5678
        // The low immediate is signed, so 0x5678 fits without compensation.
        let prog = [
            encode::addis(3, 0, 0x1234),
            encode::addi(3, 3, 0x5678),
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0x1234_5678);
    }

    // -- ori / xori / andi. ---------------------------------------

    #[test]
    fn ori_sets_low_bits() {
        let (mut ppu, mut mem) = make_env(&[encode::ori(3, 3, 0xFF)]);
        ppu.gpr[3] = 0xABCD_0000_0000;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0xABCD_0000_00FF);
    }

    #[test]
    fn xori_toggles_bits() {
        let (mut ppu, mut mem) = make_env(&[encode::xori(4, 4, 0x00FF)]);
        ppu.gpr[4] = 0xAA55;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[4], 0xAAAA);
    }

    #[test]
    fn andi_dot_masks_and_sets_cr0() {
        let (mut ppu, mut mem) = make_env(&[encode::andi_dot(5, 5, 0x0F00)]);
        ppu.gpr[5] = 0xFFFF_FFFF;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0x0F00);
        // CR0: result > 0 → GT set, LT=0, EQ=0
        assert_eq!(ppu.cr.0[0], 0); // LT
        assert_eq!(ppu.cr.0[1], 1); // GT
        assert_eq!(ppu.cr.0[2], 0); // EQ
    }

    #[test]
    fn andi_dot_zero_result_sets_eq() {
        let (mut ppu, mut mem) = make_env(&[encode::andi_dot(5, 5, 0)]);
        ppu.gpr[5] = 0xFFFF;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0);
        assert_eq!(ppu.cr.0[2], 1); // EQ
    }

    // -- mulli / mullw --------------------------------------------

    #[test]
    fn mulli_signed_multiply_immediate() {
        let (mut ppu, mut mem) = make_env(&[encode::mulli(3, 4, -5)]);
        ppu.gpr[4] = 10;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as i64, -50);
    }

    #[test]
    fn mullw_low32_signed() {
        let (mut ppu, mut mem) = make_env(&[encode::mullw(3, 4, 5)]);
        ppu.gpr[4] = 7;
        ppu.gpr[5] = 6;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 42);
    }

    // -- add / subf / and / or / xor XO-form -----------------------

    #[test]
    fn add_xo_two_positive() {
        let (mut ppu, mut mem) = make_env(&[encode::add(3, 4, 5)]);
        ppu.gpr[4] = 1000;
        ppu.gpr[5] = 234;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 1234);
    }

    #[test]
    fn add_xo_wraps_on_overflow() {
        let (mut ppu, mut mem) = make_env(&[encode::add(3, 4, 5)]);
        ppu.gpr[4] = u64::MAX;
        ppu.gpr[5] = 1;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0);
    }

    #[test]
    fn subf_computes_rb_minus_ra() {
        let (mut ppu, mut mem) = make_env(&[encode::subf(3, 4, 5)]);
        ppu.gpr[4] = 30;
        ppu.gpr[5] = 100;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 70);
    }

    #[test]
    fn and_or_xor_register_form() {
        let prog = [
            encode::and(3, 4, 5),
            encode::or(6, 4, 5),
            encode::xor(7, 4, 5),
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xF0F0;
        ppu.gpr[5] = 0x0FF0;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0x00F0); // and
        assert_eq!(ppu.gpr[6], 0xFFF0); // or
        assert_eq!(ppu.gpr[7], 0xFF00); // xor
    }

    // -- Canonical `nop` is silent on registers -------------------

    #[test]
    fn nop_is_noop_but_advances_cia() {
        let (mut ppu, mut mem) = make_env(&[encode::nop()]);
        let regs_before = ppu.gpr;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr, regs_before);
        assert_eq!(ppu.cia, PROG_BASE + 4);
    }

    // -- Unimplemented + error surface -----------------------------

    #[test]
    fn unknown_primary_returns_unimplemented() {
        // primary 63 (FPD) not supported yet.
        let raw: u32 = 63 << 26;
        let (mut ppu, mut mem) = make_env(&[raw]);
        let err = step(&mut ppu, &mut mem).unwrap_err();
        match err {
            Error::Unimplemented { reason, .. } => {
                assert!(reason.contains("primary"));
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
        // CIA should NOT advance on error.
        assert_eq!(ppu.cia, PROG_BASE);
    }

    #[test]
    fn unknown_xo_returns_unimplemented() {
        // XO 999 → not in our table.
        let raw: u32 = (31 << 26) | (999 << 1);
        let (mut ppu, mut mem) = make_env(&[raw]);
        let err = step(&mut ppu, &mut mem).unwrap_err();
        match err {
            Error::Unimplemented { reason, .. } => {
                assert!(reason.contains("XO-form"));
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    // -- Multi-step program: compute 1 + 2 + 3 + 4 = 10 -----------

    #[test]
    fn small_program_sums_four_numbers() {
        let prog = [
            encode::addi(3, 0, 1),  // r3 = 1
            encode::addi(4, 0, 2),  // r4 = 2
            encode::add(3, 3, 4),   // r3 = 3
            encode::addi(5, 0, 3),  // r5 = 3
            encode::add(3, 3, 5),   // r3 = 6
            encode::addi(6, 0, 4),  // r6 = 4
            encode::add(3, 3, 6),   // r3 = 10
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        for _ in 0..prog.len() {
            step_ok(&mut ppu, &mut mem);
        }
        assert_eq!(ppu.gpr[3], 10);
        assert_eq!(ppu.cia, PROG_BASE + (prog.len() as u32) * 4);
    }

    // -- CR0 updates only when rc is set --------------------------

    #[test]
    fn plain_add_does_not_touch_cr0() {
        let (mut ppu, mut mem) = make_env(&[encode::add(3, 4, 5)]);
        ppu.gpr[4] = 1;
        ppu.gpr[5] = 2;
        ppu.cr.0[1] = 1; // pretend GT was set previously
        step_ok(&mut ppu, &mut mem);
        // CR0 untouched (no rc bit)
        assert_eq!(ppu.cr.0[1], 1);
    }

    // =============================================================
    // Iteration 2 — load/store + sc
    // =============================================================

    const DATA_BASE: u32 = 0x2000;

    // -- Byte loads/stores (lbz, stb) -----------------------------

    #[test]
    fn stb_then_lbz_roundtrip() {
        let prog = [
            encode::stb(3, 0x10, 4),
            encode::lbz(5, 0x10, 4),
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0xAB;
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0xAB);
    }

    #[test]
    fn stb_truncates_high_bits() {
        let prog = [encode::stb(3, 0, 4), encode::lbz(5, 0, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0xAABB_CCDD_EEFF_0042; // only 0x42 written
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0x42);
    }

    // -- Halfword loads (lhz / lha are BE) ------------------------

    #[test]
    fn sth_and_lhz_big_endian_roundtrip() {
        let prog = [encode::sth(3, 0, 4), encode::lhz(5, 0, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0x1234;
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0x1234);
        // On disk: 0x12 at +0, 0x34 at +1 (big-endian).
        let mut on_disk = [0u8; 2];
        mem.read(DATA_BASE, &mut on_disk).unwrap();
        assert_eq!(on_disk, [0x12, 0x34]);
    }

    #[test]
    fn lha_sign_extends() {
        let prog = [encode::sth(3, 0, 4), encode::lha(5, 0, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0xFFFF; // -1 as signed halfword
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        // Sign-extended to 64-bit.
        assert_eq!(ppu.gpr[5] as i64, -1);
    }

    // -- Word loads/stores ----------------------------------------

    #[test]
    fn stw_and_lwz_big_endian_roundtrip() {
        let prog = [encode::stw(3, 0, 4), encode::lwz(5, 0, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0xDEAD_BEEF;
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0xDEAD_BEEF);
    }

    #[test]
    fn lwz_zero_extends() {
        let prog = [encode::stw(3, 0, 4), encode::lwz(5, 0, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0xAABB_CCDD_8000_0000; // low 32 = 0x80000000
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        // Zero-extend, not sign-extend.
        assert_eq!(ppu.gpr[5], 0x8000_0000);
    }

    // -- ld / std (DS-form) ---------------------------------------

    #[test]
    fn std_and_ld_roundtrip() {
        let prog = [encode::std(3, 0, 4), encode::ld(5, 0, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0x0123_4567_89AB_CDEF;
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0x0123_4567_89AB_CDEF);
    }

    #[test]
    fn std_preserves_big_endian_on_disk() {
        let prog = [encode::std(3, 0, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0x1122_3344_5566_7788;
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        let mut on_disk = [0u8; 8];
        mem.read(DATA_BASE, &mut on_disk).unwrap();
        assert_eq!(on_disk, [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
    }

    #[test]
    fn ds_form_offset_is_multiplied_by_4() {
        // ds=4 in the encoded field → actual offset = 16 bytes
        let prog = [encode::std(3, 4, 4), encode::ld(5, 4, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.gpr[3] = 0xCAFEBABE_DEADBEEFu64;
        ppu.gpr[4] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0xCAFEBABE_DEADBEEFu64);
        // Data lives at offset +16, not +4.
        let mut at_plus_16 = [0u8; 8];
        mem.read(DATA_BASE + 16, &mut at_plus_16).unwrap();
        assert_eq!(at_plus_16[0], 0xCA);
    }

    // -- Load/store with ra=0 uses literal zero as base ----------

    #[test]
    fn lwz_with_ra_zero_uses_literal_zero_base() {
        let prog = [encode::lwz(5, 0, 0)]; // EA = 0 + 0 = 0
        let (mut ppu, mut mem) = make_env(&prog);
        mem.alloc_at(0, 0x1000, PageFlags::READABLE | PageFlags::WRITABLE).unwrap();
        mem.write(0, &0xDEAD_BEEFu32.to_be_bytes()).unwrap();
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0xDEAD_BEEF);
    }

    // -- Memory error propagates ---------------------------------

    #[test]
    fn load_from_unmapped_memory_returns_memory_error() {
        let prog = [encode::lwz(5, 0, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        // NO data page allocated.
        ppu.gpr[4] = 0xBEEF_0000; // unmapped
        let err = step(&mut ppu, &mut mem).unwrap_err();
        assert!(matches!(err, Error::Memory(_)));
        // CIA should NOT advance on error.
        assert_eq!(ppu.cia, PROG_BASE);
    }

    // -- sc (syscall) ---------------------------------------------

    #[test]
    fn sc_returns_syscall_outcome_and_advances_cia() {
        let (mut ppu, mut mem) = make_env(&[encode::sc()]);
        let out = step_outcome(&mut ppu, &mut mem);
        assert_eq!(out, StepOutcome::Syscall);
        // CIA advanced past the sc instruction.
        assert_eq!(ppu.cia, PROG_BASE + 4);
    }

    #[test]
    fn sc_preserves_registers() {
        let (mut ppu, mut mem) = make_env(&[encode::sc()]);
        ppu.gpr[3] = 42;
        ppu.gpr[11] = 0x1A; // syscall number (_sys_process_exit)
        let out = step_outcome(&mut ppu, &mut mem);
        assert_eq!(out, StepOutcome::Syscall);
        // All GPRs untouched — the emu core is responsible for syscall
        // argument/return handling.
        assert_eq!(ppu.gpr[3], 42);
        assert_eq!(ppu.gpr[11], 0x1A);
    }

    // -- run_n exits on Syscall -----------------------------------

    #[test]
    fn run_n_stops_at_first_sc() {
        let prog = [
            encode::addi(3, 0, 42),
            encode::sc(),
            encode::addi(3, 0, 99), // must not execute
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        let (n, out) = run_n(&mut ppu, &mut mem, 10).unwrap();
        assert_eq!(n, 2);
        assert_eq!(out, StepOutcome::Syscall);
        assert_eq!(ppu.gpr[3], 42);
        assert_eq!(ppu.cia, PROG_BASE + 8);
    }

    #[test]
    fn run_n_exhausts_budget_on_linear_program() {
        let prog = [encode::addi(3, 0, 1), encode::addi(4, 0, 2), encode::add(5, 3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        let (n, out) = run_n(&mut ppu, &mut mem, 3).unwrap();
        assert_eq!(n, 3);
        assert_eq!(out, StepOutcome::Continue);
        assert_eq!(ppu.gpr[5], 3);
    }

    // =============================================================
    // Iteration 3 — branches
    // =============================================================

    #[test]
    fn b_relative_positive_jump() {
        // At PROG_BASE: b +8  → should jump to PROG_BASE + 8
        // Fill with nops between to catch off-by-one.
        let prog = [encode::b(8), encode::nop(), encode::addi(3, 0, 99)];
        let (mut ppu, mut mem) = make_env(&prog);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.cia, PROG_BASE + 8);
        // Run the landing instruction.
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 99);
    }

    #[test]
    fn b_relative_negative_jump() {
        // Program: addi r3,0,1 ; addi r3,r3,1 ; b -4 (loops)
        // We stop after a few iterations manually.
        let prog = [encode::addi(3, 0, 1), encode::addi(3, 3, 1), encode::b(-4)];
        let (mut ppu, mut mem) = make_env(&prog);
        step_ok(&mut ppu, &mut mem); // r3 = 1, cia = +4
        step_ok(&mut ppu, &mut mem); // r3 = 2, cia = +8
        step_ok(&mut ppu, &mut mem); // b -4 → cia = +4
        assert_eq!(ppu.cia, PROG_BASE + 4);
        step_ok(&mut ppu, &mut mem); // r3 = 3, cia = +8
        assert_eq!(ppu.gpr[3], 3);
    }

    #[test]
    fn bl_sets_lr_and_branches() {
        let prog = [encode::bl(8), encode::nop(), encode::addi(3, 0, 7)];
        let (mut ppu, mut mem) = make_env(&prog);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.cia, PROG_BASE + 8);
        assert_eq!(ppu.lr, (PROG_BASE + 4) as u64);
    }

    #[test]
    fn blr_returns_to_lr() {
        // Program: addi r3,0,1; bl +8; nop; addi r3,r3,10; blr (at +16)
        // ... except bl returns to r3+=10 which also returns. We set LR
        // manually here to isolate blr behaviour.
        let prog = [encode::blr()];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.lr = 0x1C00; // arbitrary target (low 2 bits zero)
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.cia, 0x1C00);
    }

    #[test]
    fn blr_masks_low_2_bits_of_lr() {
        let prog = [encode::blr()];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.lr = 0x1C03; // low bits must be masked
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.cia, 0x1C00);
    }

    #[test]
    fn bctr_branches_to_ctr() {
        let prog = [encode::bctr()];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.ctr = 0x2000;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.cia, 0x2000);
    }

    #[test]
    fn bctrl_sets_lr_to_next_instruction() {
        let prog = [encode::bctrl()];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.ctr = 0x3000;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.cia, 0x3000);
        assert_eq!(ppu.lr, (PROG_BASE + 4) as u64);
    }

    #[test]
    fn bc_branch_if_true_taken() {
        // BO=12, BI=2 (EQ bit of CR0): branch if CR0.EQ == 1
        let prog = [encode::bc(12, 2, 8), encode::nop(), encode::addi(3, 0, 7)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.cr.0[2] = 1; // EQ set
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.cia, PROG_BASE + 8);
    }

    #[test]
    fn bc_branch_if_true_not_taken() {
        let prog = [encode::bc(12, 2, 8), encode::addi(3, 0, 1)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.cr.0[2] = 0; // EQ clear
        step_ok(&mut ppu, &mut mem);
        // Fall through.
        assert_eq!(ppu.cia, PROG_BASE + 4);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 1);
    }

    #[test]
    fn bc_branch_if_false_taken() {
        // BO=4, BI=2: branch if CR0.EQ == 0
        let prog = [encode::bc(4, 2, 8), encode::nop(), encode::addi(3, 0, 42)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.cr.0[2] = 0; // EQ clear → take branch
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.cia, PROG_BASE + 8);
    }

    #[test]
    fn bc_decrement_ctr_branch_if_nonzero() {
        // BO=16: decrement CTR, branch if CTR != 0
        let prog = [encode::bc(16, 0, 8), encode::nop(), encode::nop()];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.ctr = 3;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.ctr, 2);
        assert_eq!(ppu.cia, PROG_BASE + 8); // taken
    }

    #[test]
    fn bc_decrement_ctr_not_taken_when_ctr_hits_zero() {
        let prog = [encode::bc(16, 0, 8)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.ctr = 1;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.ctr, 0);
        assert_eq!(ppu.cia, PROG_BASE + 4); // fall through
    }

    #[test]
    fn function_call_return_via_bl_blr() {
        // Minimal call + return sequence without stack discipline.
        // (Real C ABI would save LR to the stack before calling; here
        // we just verify that bl sets LR and blr jumps to it.)
        //
        //   PROG_BASE+0:  addi r3, 0, 10  ; main: r3 = 10
        //   PROG_BASE+4:  bl +16          ; call callee at +20
        //   PROG_BASE+8:  addi r3, r3, 100; back from callee
        //   PROG_BASE+20: addi r3, r3, 5  ; callee: r3 += 5
        //   PROG_BASE+24: blr             ; return via LR=+8
        let prog = [
            encode::addi(3, 0, 10),
            encode::bl(16), // call to +20
            encode::addi(3, 3, 100),
            encode::nop(), // +12
            encode::nop(), // +16
            encode::addi(3, 3, 5), // +20
            encode::blr(), // +24
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        step_ok(&mut ppu, &mut mem); // r3 = 10
        step_ok(&mut ppu, &mut mem); // bl → PROG_BASE+20, LR = PROG_BASE+8
        assert_eq!(ppu.cia, PROG_BASE + 20);
        assert_eq!(ppu.lr, (PROG_BASE + 8) as u64);
        step_ok(&mut ppu, &mut mem); // r3 = 15
        step_ok(&mut ppu, &mut mem); // blr → PROG_BASE+8
        assert_eq!(ppu.cia, PROG_BASE + 8);
        assert_eq!(ppu.gpr[3], 15);
        step_ok(&mut ppu, &mut mem); // r3 = 115 at +8
        assert_eq!(ppu.gpr[3], 115);
    }

    // =============================================================
    // Iteration 4 — compares, shifts, mtspr/mfspr
    // =============================================================

    fn get_cr_field(ppu: &PpuThread, crfd: usize) -> (bool, bool, bool, bool) {
        let base = crfd * 4;
        (
            ppu.cr.0[base] != 0,
            ppu.cr.0[base + 1] != 0,
            ppu.cr.0[base + 2] != 0,
            ppu.cr.0[base + 3] != 0,
        )
    }

    // -- cmpi (signed) -------------------------------------------

    #[test]
    fn cmpi_signed_less_greater_equal() {
        let prog = [encode::cmpi(0, 0, 3, 10)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = 5;
        step_ok(&mut ppu, &mut mem);
        let (lt, gt, eq, _so) = get_cr_field(&ppu, 0);
        assert!(lt && !gt && !eq);
    }

    #[test]
    fn cmpi_signed_negative_less_than_zero() {
        let prog = [encode::cmpi(0, 0, 3, 0)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = (-1i32) as u32 as u64; // low 32 = -1
        step_ok(&mut ppu, &mut mem);
        let (lt, _, _, _) = get_cr_field(&ppu, 0);
        assert!(lt);
    }

    #[test]
    fn cmpi_64bit_uses_full_width() {
        let prog = [encode::cmpi(0, 1, 3, 0)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = 0x8000_0000; // positive in 64-bit
        step_ok(&mut ppu, &mut mem);
        let (lt, gt, _, _) = get_cr_field(&ppu, 0);
        assert!(!lt && gt);
    }

    // -- cmpli (unsigned) ----------------------------------------

    #[test]
    fn cmpli_unsigned_ordering() {
        let prog = [encode::cmpli(7, 0, 3, 100)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = 200;
        step_ok(&mut ppu, &mut mem);
        let (lt, gt, eq, _) = get_cr_field(&ppu, 7);
        assert!(!lt && gt && !eq);
    }

    // -- cmp / cmpl (register form) ------------------------------

    #[test]
    fn cmp_equal() {
        let prog = [encode::cmp(0, 0, 3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = 42;
        ppu.gpr[4] = 42;
        step_ok(&mut ppu, &mut mem);
        let (_, _, eq, _) = get_cr_field(&ppu, 0);
        assert!(eq);
    }

    #[test]
    fn cmpl_unsigned_large_values() {
        let prog = [encode::cmpl(0, 1, 3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = u64::MAX - 1;
        ppu.gpr[4] = u64::MAX;
        step_ok(&mut ppu, &mut mem);
        let (lt, _, _, _) = get_cr_field(&ppu, 0);
        assert!(lt);
    }

    // -- cmp + bc integration ------------------------------------

    #[test]
    fn cmp_and_conditional_branch() {
        // Program:
        //   cmpi 0, 0, 3, 5     ; compare r3 vs 5
        //   bc 12, 2, +8        ; branch if EQ
        //   addi 4, 0, 99       ; (skipped if EQ)
        //   addi 4, 0, 42       ; landing
        let prog = [
            encode::cmpi(0, 0, 3, 5),
            encode::bc(12, 2, 8),
            encode::addi(4, 0, 99),
            encode::addi(4, 0, 42),
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = 5; // EQ → branch taken
        step_ok(&mut ppu, &mut mem); // cmpi
        step_ok(&mut ppu, &mut mem); // bc → jump
        step_ok(&mut ppu, &mut mem); // addi 42
        assert_eq!(ppu.gpr[4], 42);
    }

    // -- Shifts --------------------------------------------------

    #[test]
    fn slw_shifts_word() {
        let prog = [encode::slw(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x0000_0001;
        ppu.gpr[5] = 4;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0x10);
    }

    #[test]
    fn slw_zero_when_shift_geq_32() {
        let prog = [encode::slw(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xFFFF_FFFF;
        ppu.gpr[5] = 32;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0);
    }

    #[test]
    fn srw_logical_right() {
        let prog = [encode::srw(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x8000_0000; // MSB set
        ppu.gpr[5] = 4;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0x0800_0000); // no sign-extend
    }

    #[test]
    fn sraw_sign_extends() {
        let prog = [encode::sraw(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF; // low 32 = -1
        ppu.gpr[5] = 4;
        step_ok(&mut ppu, &mut mem);
        // Sign-extended result stays -1 in 64-bit.
        assert_eq!(ppu.gpr[3] as i64, -1);
    }

    #[test]
    fn srawi_immediate_sign_extend() {
        let prog = [encode::srawi(3, 4, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xFFFF_FFF0; // low 32-bit = -16
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as i64, -1);
    }

    #[test]
    fn srawi_sets_xer_ca_when_negative_and_bits_shifted_out() {
        let prog = [encode::srawi(3, 4, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xFFFF_FFFF; // -1, all bits set
        step_ok(&mut ppu, &mut mem);
        assert!(ppu.xer.ca);
    }

    #[test]
    fn sld_doubleword_shift_left() {
        let prog = [encode::sld(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 1;
        ppu.gpr[5] = 32;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0x1_0000_0000);
    }

    #[test]
    fn srd_doubleword_shift_right_logical() {
        let prog = [encode::srd(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x8000_0000_0000_0000;
        ppu.gpr[5] = 4;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0x0800_0000_0000_0000);
    }

    #[test]
    fn sld_zero_when_shift_geq_64() {
        let prog = [encode::sld(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = u64::MAX;
        ppu.gpr[5] = 64;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0);
    }

    // -- mtspr / mfspr ------------------------------------------

    #[test]
    fn mtlr_mflr_roundtrip() {
        let prog = [encode::mtlr(3), encode::mflr(5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = 0xDEAD_BEEF_CAFE_0000;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.lr, 0xDEAD_BEEF_CAFE_0000);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0xDEAD_BEEF_CAFE_0000);
    }

    #[test]
    fn mtctr_mfctr_roundtrip() {
        let prog = [encode::mtctr(3), encode::mfctr(5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[3] = 0x1234_5678;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.ctr, 0x1234_5678);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], 0x1234_5678);
    }

    #[test]
    fn mtxer_mfxer_roundtrip() {
        let prog = [encode::mtspr(1, 3), encode::mfspr(5, 1)];
        let (mut ppu, mut mem) = make_env(&prog);
        // Encode SO=1, OV=0, CA=1, cnt=0x42
        let packed = (1u64 << 31) | (1u64 << 29) | 0x42;
        ppu.gpr[3] = packed;
        step_ok(&mut ppu, &mut mem);
        assert!(ppu.xer.so);
        assert!(!ppu.xer.ov);
        assert!(ppu.xer.ca);
        assert_eq!(ppu.xer.cnt, 0x42);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[5], packed);
    }

    #[test]
    fn unknown_spr_is_unimplemented() {
        // SPR 100 not handled in this iteration.
        let prog = [encode::mtspr(100, 3)];
        let (mut ppu, mut mem) = make_env(&prog);
        assert!(matches!(
            step(&mut ppu, &mut mem).unwrap_err(),
            Error::Unimplemented { reason, .. } if reason.contains("SPR")
        ));
    }

    // =============================================================
    // Iteration 5 — rotate+mask, unary, multiply-high, divide
    // =============================================================

    // -- neg / ext / cntlz ---------------------------------------

    #[test]
    fn neg_twos_complement() {
        let prog = [encode::neg(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 42;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as i64, -42);
    }

    #[test]
    fn neg_of_zero_is_zero() {
        let prog = [encode::neg(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0);
    }

    #[test]
    fn extsb_sign_extends_byte() {
        let prog = [encode::extsb(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xFFFF_FFFF_FFFF_FF80; // byte = 0x80 (negative)
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as i64, -128);
    }

    #[test]
    fn extsh_sign_extends_halfword() {
        let prog = [encode::extsh(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x0000_0000_0000_8000; // halfword = -32768
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as i64, -32768);
    }

    #[test]
    fn extsw_sign_extends_word() {
        let prog = [encode::extsw(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x0000_0000_8000_0000;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as i64, -(1 << 31));
    }

    #[test]
    fn cntlzw_counts_leading_zeros_in_low_32() {
        let prog = [encode::cntlzw(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x0000_0000_0000_0001;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 31);
    }

    #[test]
    fn cntlzw_zero_input_gives_32() {
        let prog = [encode::cntlzw(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 32);
    }

    #[test]
    fn cntlzd_counts_leading_zeros_in_64() {
        let prog = [encode::cntlzd(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x0000_0000_0000_0001;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 63);
    }

    #[test]
    fn cntlzd_msb_set_is_zero() {
        let prog = [encode::cntlzd(3, 4)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x8000_0000_0000_0000;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0);
    }

    // -- multiply-high -------------------------------------------

    #[test]
    fn mulhwu_unsigned_high_32() {
        let prog = [encode::mulhwu(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        // 0xFFFFFFFF * 0xFFFFFFFF = 0xFFFFFFFE_00000001
        // High 32 = 0xFFFFFFFE
        ppu.gpr[4] = 0xFFFF_FFFF;
        ppu.gpr[5] = 0xFFFF_FFFF;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0xFFFF_FFFE);
    }

    #[test]
    fn mulhw_signed_high_32() {
        let prog = [encode::mulhw(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        // (-2) * 3 = -6; low 32 = 0xFFFFFFFA, high 32 = 0xFFFFFFFF
        ppu.gpr[4] = 0xFFFF_FFFE; // -2 in 32-bit
        ppu.gpr[5] = 3;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as u32, 0xFFFF_FFFF);
    }

    #[test]
    fn mulhdu_unsigned_high_64() {
        let prog = [encode::mulhdu(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = u64::MAX;
        ppu.gpr[5] = u64::MAX;
        step_ok(&mut ppu, &mut mem);
        // (2^64-1)^2 high 64 bits = 2^64-2
        assert_eq!(ppu.gpr[3], u64::MAX - 1);
    }

    #[test]
    fn mulhd_signed_high_64() {
        let prog = [encode::mulhd(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        // -2 * 3 = -6; in 128-bit signed, high 64 = all ones
        ppu.gpr[4] = (-2i64) as u64;
        ppu.gpr[5] = 3;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], u64::MAX); // sign-extended to -1
    }

    // -- divide --------------------------------------------------

    #[test]
    fn divw_signed_32_division() {
        let prog = [encode::divw(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 100;
        ppu.gpr[5] = 7;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3] as u32, 14);
    }

    #[test]
    fn divw_by_zero_returns_zero() {
        let prog = [encode::divw(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 42;
        ppu.gpr[5] = 0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0);
    }

    #[test]
    fn divw_overflow_case_returns_zero() {
        // INT_MIN / -1 would overflow; PPC returns undefined, we return 0
        let prog = [encode::divw(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = i32::MIN as u32 as u64;
        ppu.gpr[5] = (-1i32) as u32 as u64;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0);
    }

    #[test]
    fn divwu_unsigned_division() {
        let prog = [encode::divwu(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xFFFF_FFFF;
        ppu.gpr[5] = 3;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0x5555_5555);
    }

    #[test]
    fn divd_signed_64_division() {
        let prog = [encode::divd(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 1_000_000_000_000;
        ppu.gpr[5] = 7;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 142_857_142_857);
    }

    #[test]
    fn divdu_by_zero_returns_zero() {
        let prog = [encode::divdu(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = u64::MAX;
        ppu.gpr[5] = 0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0);
    }

    // -- rlwinm --------------------------------------------------

    #[test]
    fn rlwinm_clears_high_bits() {
        // rlwinm r3, r4, 0, 28, 31 — extract low 4 bits of r4 low-32
        let prog = [encode::rlwinm(3, 4, 0, 28, 31)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xAABB_CCDE;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0xE); // low nibble
    }

    #[test]
    fn rlwinm_shift_and_mask() {
        // rlwinm r3, r4, 4, 0, 31 — rotate left 4 bits, full mask (= shift left 4 on u32, zero-extend)
        let prog = [encode::rlwinm(3, 4, 4, 0, 27)]; // me=27 clears low 4
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0x1234_5678;
        step_ok(&mut ppu, &mut mem);
        // rotate_left(0x12345678, 4) = 0x23456781; mask(0, 27) = top 28 bits = 0xFFFFFFF0
        assert_eq!(ppu.gpr[3], 0x23456780);
    }

    // -- rldicl / rldicr -----------------------------------------

    #[test]
    fn rldicl_clears_high_bits() {
        // rldicl r3, r4, 0, 60 — keep low 4 bits
        let prog = [encode::rldicl(3, 4, 0, 60)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xAABB_CCDD_EEFF_1234;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0x4); // low 4 bits
    }

    #[test]
    fn rldicl_shift_right_by_32() {
        // rldicl r3, r4, 32, 32 — rotate left 32 (= logical right shift 32 for high bits).
        // Actually: rotate_left(v, 32) puts high 32 into low 32 position. mask(32, 63) keeps low 32.
        // Effectively: extracts original high 32 into result.
        let prog = [encode::rldicl(3, 4, 32, 32)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xAABB_CCDD_EEFF_1234;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0xAABB_CCDD);
    }

    #[test]
    fn rldicr_clears_low_bits() {
        // rldicr r3, r4, 0, 3 — keep top 4 bits
        let prog = [encode::rldicr(3, 4, 0, 3)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xF000_0000_0000_0000;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0xF000_0000_0000_0000);
    }

    #[test]
    fn rldicr_mask_truncates_low() {
        // rldicr r3, r4, 0, 31 — keep top 32 bits
        let prog = [encode::rldicr(3, 4, 0, 31)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.gpr[4] = 0xAABB_CCDD_EEFF_1234;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.gpr[3], 0xAABB_CCDD_0000_0000);
    }

    // -- Floating-point: primary 63 + D-form loads/stores ---------

    #[test]
    fn lfd_reads_be_double_from_memory() {
        let prog = [encode::lfd(0, 10, 0)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        let v: f64 = 2.71828182845904;
        mem.write(DATA_BASE, &v.to_bits().to_be_bytes()).unwrap();
        ppu.gpr[10] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], v);
    }

    #[test]
    fn stfd_writes_be_double_to_memory() {
        let prog = [encode::stfd(1, 10, 0)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.fpr[1] = -12345.6789;
        ppu.gpr[10] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        let mut raw = [0u8; 8];
        mem.read(DATA_BASE, &mut raw).unwrap();
        assert_eq!(f64::from_bits(u64::from_be_bytes(raw)), -12345.6789);
    }

    #[test]
    fn lfs_widens_single_to_double() {
        let prog = [encode::lfs(2, 10, 0)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        let v: f32 = 0.5f32;
        mem.write(DATA_BASE, &v.to_bits().to_be_bytes()).unwrap();
        ppu.gpr[10] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        // 0.5 is exactly representable in both formats.
        assert_eq!(ppu.fpr[2], 0.5f64);
    }

    #[test]
    fn stfs_narrows_double_to_single() {
        let prog = [encode::stfs(3, 10, 0)];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        ppu.fpr[3] = 1.25f64; // exactly representable
        ppu.gpr[10] = DATA_BASE as u64;
        step_ok(&mut ppu, &mut mem);
        let mut raw = [0u8; 4];
        mem.read(DATA_BASE, &mut raw).unwrap();
        assert_eq!(f32::from_bits(u32::from_be_bytes(raw)), 1.25f32);
    }

    #[test]
    fn fadd_sums_two_doubles() {
        let prog = [encode::fadd(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 3.5;
        ppu.fpr[2] = 2.25;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 5.75);
    }

    #[test]
    fn fsub_subtracts_two_doubles() {
        let prog = [encode::fsub(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 10.0;
        ppu.fpr[2] = 3.5;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 6.5);
    }

    #[test]
    fn fmul_uses_frc_not_frb() {
        // A-form fmul reads frA + frC, not frB — verify by placing
        // the multiplier in frC and leaving frB at 0.
        let prog = [encode::fmul(0, 1, 3)]; // frD=0, frA=1, frC=3
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 4.0;
        ppu.fpr[3] = 2.5;
        ppu.fpr[2] = 999.0; // must NOT be read
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 10.0);
    }

    #[test]
    fn fdiv_divides_doubles() {
        let prog = [encode::fdiv(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 7.0;
        ppu.fpr[2] = 2.0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 3.5);
    }

    #[test]
    fn fdiv_by_zero_yields_infinity() {
        let prog = [encode::fdiv(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 1.0;
        ppu.fpr[2] = 0.0;
        step_ok(&mut ppu, &mut mem);
        assert!(ppu.fpr[0].is_infinite());
        assert!(ppu.fpr[0] > 0.0);
    }

    #[test]
    fn fdiv_zero_by_zero_yields_nan_and_sets_vx() {
        let prog = [encode::fdiv(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 0.0;
        ppu.fpr[2] = 0.0;
        ppu.fpscr = 0; // ensure VX starts clear
        step_ok(&mut ppu, &mut mem);
        assert!(ppu.fpr[0].is_nan());
        // VX (0x20000000) must be set, along with FX (sticky summary).
        assert_ne!(ppu.fpscr & 0x2000_0000, 0, "VX must be set");
        assert_ne!(ppu.fpscr & 0x8000_0000, 0, "FX sticky must be set");
    }

    #[test]
    fn fmr_moves_fpr() {
        let prog = [encode::fmr(5, 7)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[7] = 123.456;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[5], 123.456);
    }

    #[test]
    fn fneg_flips_sign_and_preserves_magnitude() {
        let prog = [encode::fneg(0, 1)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 2.5;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], -2.5);
    }

    #[test]
    fn fneg_of_neg_is_pos() {
        let prog = [encode::fneg(0, 1)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = -7.0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 7.0);
    }

    #[test]
    fn fabs_yields_magnitude() {
        let prog = [encode::fabs(0, 1)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = -3.14;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 3.14);
    }

    #[test]
    fn fnabs_yields_negative_magnitude() {
        let prog = [encode::fnabs(0, 1)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 4.0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], -4.0);
    }

    #[test]
    fn fadd_result_sets_fprf_neg_normal() {
        let prog = [encode::fadd(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = -1.0;
        ppu.fpr[2] = -0.5;
        ppu.fpscr = 0;
        step_ok(&mut ppu, &mut mem);
        // Negative normal: 0b01000 in FPRF, shifted to bits 12..16.
        let fprf = (ppu.fpscr >> 12) & 0x1F;
        assert_eq!(fprf, 0b01000);
    }

    #[test]
    fn fadd_result_sets_fprf_pos_zero() {
        let prog = [encode::fadd(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 2.0;
        ppu.fpr[2] = -2.0;
        ppu.fpscr = 0;
        step_ok(&mut ppu, &mut mem);
        let fprf = (ppu.fpscr >> 12) & 0x1F;
        assert_eq!(fprf, 0b00010, "+0 FPRF");
    }

    // -- Single-precision FP (primary 59) -------------------------

    #[test]
    fn fadds_rounds_to_f32_precision() {
        // 1.0 + 2^-30 in f64 is representable; in f32 it loses the
        // low bits because f32 only has 23 mantissa bits. Use a
        // value that exercises the f32 round-trip.
        let prog = [encode::fadds(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 1.0f64;
        ppu.fpr[2] = (1u64 << 30) as f64 * f64::from_bits(0x3C70_0000_0000_0000); // tiny
        step_ok(&mut ppu, &mut mem);
        let expected = ((ppu.fpr[1] + ppu.fpr[2]) as f32) as f64;
        assert_eq!(ppu.fpr[0], expected);
    }

    #[test]
    fn fadds_simple_sum_exact() {
        let prog = [encode::fadds(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 1.5;
        ppu.fpr[2] = 0.25;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 1.75f64);
    }

    #[test]
    fn fsubs_produces_f32_rounded_result() {
        let prog = [encode::fsubs(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 10.0;
        ppu.fpr[2] = 3.5;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 6.5f64);
    }

    #[test]
    fn fmuls_uses_frc_not_frb() {
        let prog = [encode::fmuls(0, 1, 3)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 2.0;
        ppu.fpr[2] = 999.0; // must NOT be read
        ppu.fpr[3] = 3.0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.fpr[0], 6.0f64);
    }

    #[test]
    fn fdivs_rounds_result_to_f32() {
        // 1.0 / 3.0 differs in f32 vs f64 precision.
        let prog = [encode::fdivs(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 1.0;
        ppu.fpr[2] = 3.0;
        step_ok(&mut ppu, &mut mem);
        let expected = ((1.0f64 / 3.0f64) as f32) as f64;
        assert_eq!(ppu.fpr[0], expected);
        assert_ne!(ppu.fpr[0], 1.0f64 / 3.0f64, "f32 round-trip must change the bits");
    }

    #[test]
    fn fdivs_by_zero_yields_infinity() {
        let prog = [encode::fdivs(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.fpr[1] = 4.0;
        ppu.fpr[2] = 0.0;
        step_ok(&mut ppu, &mut mem);
        assert!(ppu.fpr[0].is_infinite());
        assert!(ppu.fpr[0] > 0.0);
    }

    #[test]
    fn fp_chain_load_compute_store() {
        // Load two doubles, multiply, store result.
        let prog = [
            encode::lfd(1, 10, 0),       // frD=1 ← *(f64*)(r10+0)
            encode::lfd(2, 10, 8),       // frD=2 ← *(f64*)(r10+8)
            encode::fmul(0, 1, 2),       // frD=0 = frA=1 * frC=2
            encode::stfd(0, 10, 16),     // *(f64*)(r10+16) = frD=0
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        mem.write(DATA_BASE, &3.0f64.to_bits().to_be_bytes()).unwrap();
        mem.write(DATA_BASE + 8, &4.0f64.to_bits().to_be_bytes()).unwrap();
        ppu.gpr[10] = DATA_BASE as u64;
        for _ in 0..prog.len() {
            step_ok(&mut ppu, &mut mem);
        }
        let mut raw = [0u8; 8];
        mem.read(DATA_BASE + 16, &mut raw).unwrap();
        assert_eq!(f64::from_bits(u64::from_be_bytes(raw)), 12.0);
    }

    // -- Integrated: load + compute + store -----------------------

    #[test]
    fn load_compute_store_sequence() {
        // Load two u32 BE values, add them, store result.
        let prog = [
            encode::lwz(3, 0, 10),          // r3 = *(u32*)(r10 + 0)
            encode::lwz(4, 4, 10),          // r4 = *(u32*)(r10 + 4)
            encode::add(5, 3, 4),           // r5 = r3 + r4
            encode::stw(5, 8, 10),          // *(u32*)(r10 + 8) = r5
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        alloc_data(&mut mem, DATA_BASE);
        mem.write(DATA_BASE, &100u32.to_be_bytes()).unwrap();
        mem.write(DATA_BASE + 4, &200u32.to_be_bytes()).unwrap();
        ppu.gpr[10] = DATA_BASE as u64;
        for _ in 0..prog.len() {
            step_ok(&mut ppu, &mut mem);
        }
        assert_eq!(ppu.gpr[5], 300);
        let mut stored = [0u8; 4];
        mem.read(DATA_BASE + 8, &mut stored).unwrap();
        assert_eq!(u32::from_be_bytes(stored), 300);
    }

    // --- Altivec/VMX (iter-1) -----------------------------------

    fn set_vr_f32x4(ppu: &mut PpuThread, idx: usize, x0: f32, x1: f32, x2: f32, x3: f32) {
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&x0.to_bits().to_be_bytes());
        bytes[4..8].copy_from_slice(&x1.to_bits().to_be_bytes());
        bytes[8..12].copy_from_slice(&x2.to_bits().to_be_bytes());
        bytes[12..16].copy_from_slice(&x3.to_bits().to_be_bytes());
        ppu.vr[idx] = u128::from_be_bytes(bytes);
    }

    fn get_vr_f32x4(ppu: &PpuThread, idx: usize) -> [f32; 4] {
        let b = ppu.vr[idx].to_be_bytes();
        [
            f32::from_bits(u32::from_be_bytes([b[0], b[1], b[2], b[3]])),
            f32::from_bits(u32::from_be_bytes([b[4], b[5], b[6], b[7]])),
            f32::from_bits(u32::from_be_bytes([b[8], b[9], b[10], b[11]])),
            f32::from_bits(u32::from_be_bytes([b[12], b[13], b[14], b[15]])),
        ]
    }

    #[test]
    fn vaddfp_adds_four_floats_lane_wise() {
        let prog = [encode::vaddfp(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        set_vr_f32x4(&mut ppu, 4, 1.0, 2.0, 3.0, 4.0);
        set_vr_f32x4(&mut ppu, 5, 0.5, 0.25, 0.125, -1.0);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(get_vr_f32x4(&ppu, 3), [1.5, 2.25, 3.125, 3.0]);
    }

    #[test]
    fn vsubfp_subtracts_lane_wise() {
        let prog = [encode::vsubfp(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        set_vr_f32x4(&mut ppu, 4, 10.0, 0.0, 7.5, -1.0);
        set_vr_f32x4(&mut ppu, 5, 4.5, 0.5, 3.0, 2.0);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(get_vr_f32x4(&ppu, 3), [5.5, -0.5, 4.5, -3.0]);
    }

    #[test]
    fn vmaddfp_computes_va_vc_plus_vb() {
        // vd = va*vc + vb per lane.
        let prog = [encode::vmaddfp(0, 1, 2, 3)];
        let (mut ppu, mut mem) = make_env(&prog);
        set_vr_f32x4(&mut ppu, 1, 2.0, 3.0, 4.0, 5.0);   // va
        set_vr_f32x4(&mut ppu, 2, 0.5, -1.0, 10.0, 0.0);  // vb
        set_vr_f32x4(&mut ppu, 3, 4.0, 2.0, 0.5, 1.0);   // vc
        step_ok(&mut ppu, &mut mem);
        // lane 0: 2*4+0.5=8.5. lane 1: 3*2-1=5. lane 2: 4*0.5+10=12. lane 3: 5*1+0=5.
        assert_eq!(get_vr_f32x4(&ppu, 0), [8.5, 5.0, 12.0, 5.0]);
    }

    #[test]
    fn vector_ops_preserve_other_vr_registers() {
        let prog = [encode::vaddfp(0, 1, 2)];
        let (mut ppu, mut mem) = make_env(&prog);
        set_vr_f32x4(&mut ppu, 1, 1.0, 2.0, 3.0, 4.0);
        set_vr_f32x4(&mut ppu, 2, 1.0, 1.0, 1.0, 1.0);
        // Pre-seed vr[7] with a sentinel that must not change.
        ppu.vr[7] = 0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.vr[7], 0xDEAD_BEEF_CAFE_BABE_1234_5678_9ABC_DEF0);
    }

    #[test]
    fn vaddfp_handles_nan_and_inf() {
        let prog = [encode::vaddfp(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        set_vr_f32x4(&mut ppu, 4, f32::INFINITY, f32::INFINITY, f32::NAN, 2.0);
        set_vr_f32x4(&mut ppu, 5, 1.0, f32::NEG_INFINITY, 1.0, 3.0);
        step_ok(&mut ppu, &mut mem);
        let r = get_vr_f32x4(&ppu, 3);
        assert!(r[0].is_infinite() && r[0] > 0.0);
        assert!(r[1].is_nan(), "inf + -inf = nan");
        assert!(r[2].is_nan());
        assert_eq!(r[3], 5.0);
    }

    #[test]
    fn vmaddfp_chain_with_vaddfp() {
        // r6 = va + vb, then r3 = r6*vc + vd — quick smoke showing
        // multiple vector ops chain cleanly through the dispatcher.
        let prog = [
            encode::vaddfp(6, 4, 5),         // v6 = v4 + v5
            encode::vmaddfp(3, 6, 7, 8),     // v3 = v6 * v8 + v7
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        set_vr_f32x4(&mut ppu, 4, 1.0, 1.0, 1.0, 1.0);
        set_vr_f32x4(&mut ppu, 5, 1.0, 1.0, 1.0, 1.0);  // v6 = 2.0 per lane
        set_vr_f32x4(&mut ppu, 7, 0.5, 0.5, 0.5, 0.5);
        set_vr_f32x4(&mut ppu, 8, 3.0, 3.0, 3.0, 3.0);
        for _ in 0..prog.len() { step_ok(&mut ppu, &mut mem); }
        assert_eq!(get_vr_f32x4(&ppu, 3), [6.5; 4]);  // 2*3+0.5=6.5
    }

    #[test]
    fn primary_4_unknown_xo_returns_unimplemented() {
        // Primary 4 with XO that doesn't match any implemented op.
        // XO 99 = 0x63, not used (but note 0x63 is also a potential
        // 6-bit VA-form XO — so we pick something that fits neither).
        // 11-bit 0x100 = 256 isn't mapped; 6-bit low = 0x00 isn't
        // either. Use inst with 6-bit low == 0 and 11-bit == 256.
        let inst = (4u32 << 26) | (256 << 6);
        let prog = [inst];
        let (mut ppu, mut mem) = make_env(&prog);
        let err = step(&mut ppu, &mut mem).unwrap_err();
        assert!(matches!(err, Error::Unimplemented { .. }));
    }

    // --- Altivec iter-2: integer vectors + vperm ----------------

    fn set_vr_u32x4(ppu: &mut PpuThread, idx: usize, x0: u32, x1: u32, x2: u32, x3: u32) {
        let mut bytes = [0u8; 16];
        bytes[0..4].copy_from_slice(&x0.to_be_bytes());
        bytes[4..8].copy_from_slice(&x1.to_be_bytes());
        bytes[8..12].copy_from_slice(&x2.to_be_bytes());
        bytes[12..16].copy_from_slice(&x3.to_be_bytes());
        ppu.vr[idx] = u128::from_be_bytes(bytes);
    }

    fn get_vr_u32x4(ppu: &PpuThread, idx: usize) -> [u32; 4] {
        let b = ppu.vr[idx].to_be_bytes();
        [
            u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            u32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            u32::from_be_bytes([b[8], b[9], b[10], b[11]]),
            u32::from_be_bytes([b[12], b[13], b[14], b[15]]),
        ]
    }

    #[test]
    fn vadduwm_adds_words_modulo_2_32() {
        let prog = [encode::vadduwm(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        set_vr_u32x4(&mut ppu, 4, 1, 0xFFFF_FFFF, 100, 0x8000_0000);
        set_vr_u32x4(&mut ppu, 5, 2, 2, 50, 0x8000_0000);
        step_ok(&mut ppu, &mut mem);
        // lane 1 wraps, lane 3 overflows to 0.
        assert_eq!(get_vr_u32x4(&ppu, 3), [3, 1, 150, 0]);
    }

    #[test]
    fn vsubuwm_subtracts_words_with_wrap() {
        let prog = [encode::vsubuwm(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        set_vr_u32x4(&mut ppu, 4, 5, 0, 100, 1);
        set_vr_u32x4(&mut ppu, 5, 3, 1, 40, 2);
        step_ok(&mut ppu, &mut mem);
        // lane 1 wraps to 0xFFFF_FFFF, lane 3 wraps.
        assert_eq!(get_vr_u32x4(&ppu, 3), [2, 0xFFFF_FFFF, 60, 0xFFFF_FFFF]);
    }

    #[test]
    fn vand_masks_128_bits() {
        let prog = [encode::vand(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.vr[4] = 0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA;
        ppu.vr[5] = 0xFF00_FF00_FF00_FF00_FF00_FF00_FF00_FF00;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(
            ppu.vr[3],
            0xAA00_AA00_AA00_AA00_AA00_AA00_AA00_AA00,
        );
    }

    #[test]
    fn vor_sets_bits() {
        let prog = [encode::vor(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.vr[4] = 0x0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F;
        ppu.vr[5] = 0xF0F0_F0F0_F0F0_F0F0_F0F0_F0F0_F0F0_F0F0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.vr[3], u128::MAX);
    }

    #[test]
    fn vxor_flips_bits() {
        let prog = [encode::vxor(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.vr[4] = 0x0000_FFFF_0000_FFFF_0000_FFFF_0000_FFFF;
        ppu.vr[5] = u128::MAX;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(
            ppu.vr[3],
            0xFFFF_0000_FFFF_0000_FFFF_0000_FFFF_0000,
        );
    }

    #[test]
    fn vnor_is_negated_vor() {
        let prog = [encode::vnor(3, 4, 5)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.vr[4] = 0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA;
        ppu.vr[5] = 0;
        step_ok(&mut ppu, &mut mem);
        assert_eq!(
            ppu.vr[3],
            0x5555_5555_5555_5555_5555_5555_5555_5555,
        );
    }

    #[test]
    fn vperm_identity_permutation() {
        let prog = [encode::vperm(3, 4, 5, 6)];
        let (mut ppu, mut mem) = make_env(&prog);
        // va = distinct byte pattern.
        ppu.vr[4] = u128::from_be_bytes([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        ]);
        ppu.vr[5] = 0;
        // selectors 0..15 → identity.
        ppu.vr[6] = u128::from_be_bytes([
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        ]);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.vr[3], ppu.vr[4]);
    }

    #[test]
    fn vperm_reverses_bytes() {
        let prog = [encode::vperm(3, 4, 5, 6)];
        let (mut ppu, mut mem) = make_env(&prog);
        let pattern = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        ];
        ppu.vr[4] = u128::from_be_bytes(pattern);
        ppu.vr[5] = 0;
        // selectors 15..0 → reverse.
        let mut sel = [0u8; 16];
        for i in 0..16 { sel[i] = (15 - i) as u8; }
        ppu.vr[6] = u128::from_be_bytes(sel);
        step_ok(&mut ppu, &mut mem);
        let mut expected = pattern;
        expected.reverse();
        assert_eq!(ppu.vr[3].to_be_bytes(), expected);
    }

    #[test]
    fn vperm_mixes_va_and_vb() {
        let prog = [encode::vperm(3, 4, 5, 6)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.vr[4] = u128::from_be_bytes([0xAA; 16]);
        ppu.vr[5] = u128::from_be_bytes([0xBB; 16]);
        // first 8 from va (indices 0..7), next 8 from vb (indices 16..23).
        ppu.vr[6] = u128::from_be_bytes([
            0, 1, 2, 3, 4, 5, 6, 7, 16, 17, 18, 19, 20, 21, 22, 23,
        ]);
        step_ok(&mut ppu, &mut mem);
        let out = ppu.vr[3].to_be_bytes();
        assert_eq!(&out[..8], &[0xAA; 8]);
        assert_eq!(&out[8..], &[0xBB; 8]);
    }

    #[test]
    fn vperm_selector_masks_low_5_bits() {
        // vc bytes with high bits set must still resolve to sel & 0x1F.
        let prog = [encode::vperm(3, 4, 5, 6)];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.vr[4] = u128::from_be_bytes([
            0xAA, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77,
            0x88, 0x99, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00,
        ]);
        ppu.vr[5] = 0;
        // 0xE0 & 0x1F = 0, should pick va[0] = 0xAA.
        let mut sel = [0u8; 16];
        sel[0] = 0xE0;
        ppu.vr[6] = u128::from_be_bytes(sel);
        step_ok(&mut ppu, &mut mem);
        assert_eq!(ppu.vr[3].to_be_bytes()[0], 0xAA);
    }

    #[test]
    fn vand_chain_with_vor() {
        // Build a small program: vand r6, r4, r5 ; vor r3, r6, r4.
        let prog = [
            encode::vand(6, 4, 5),
            encode::vor(3, 6, 4),
        ];
        let (mut ppu, mut mem) = make_env(&prog);
        ppu.vr[4] = 0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA;
        ppu.vr[5] = 0xFF00_FF00_FF00_FF00_FF00_FF00_FF00_FF00;
        for _ in 0..prog.len() {
            step_ok(&mut ppu, &mut mem);
        }
        // vand = 0xAA00... ; then or with r4 = 0xAAAA... = 0xAAAA...
        assert_eq!(
            ppu.vr[3],
            0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA,
        );
    }
}
