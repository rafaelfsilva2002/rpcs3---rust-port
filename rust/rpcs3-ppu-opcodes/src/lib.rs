//! `rpcs3-ppu-opcodes` — PowerPC 64 instruction format decoder.
//!
//! Mirrors `rpcs3/Emu/Cell/PPUOpcodes.h`:
//!
//! * `PpuOpcode` — 32-bit instruction word with typed accessors for
//!   every documented bitfield (matches `union ppu_opcode_t`).
//! * `ppu_decode(inst)` — primary+extended opcode rotate used to
//!   produce the 17-bit lookup index (PPUOpcodes.h:70-73).
//! * `ppu_rotate_mask(mb, me)` — the canonical PPC rotate-mask
//!   generator (PPUOpcodes.h:64-68).
//!
//! ## What we intentionally do NOT port here
//!
//! * The 131072-slot lookup table (`m_table`) — it's filled
//!   programmatically by `ppu_decoder::register()` at startup,
//!   indexing instructions to handler fn pointers. That table
//!   belongs in `rpcs3-ppu-interpreter`.
//!
//! ## Bit numbering
//!
//! PowerPC counts bits **left-to-right**, from MSB = bit 0 to LSB = bit 31.
//! The C++ `bf_t<T, pos, width>` starts from LSB, so the wrapper
//! `ppu_bf_t<T, I, N> = bf_t<T, sizeof(T) * 8 - N - I, N>` translates
//! "bit I with width N" (PPC numbering) to "LSB position" (standard).
//! We replicate that discipline here: every accessor is documented
//! by PPC bit range.

// =====================================================================
// Instruction word wrapper
// =====================================================================

/// PowerPC instruction word (32 bits, big-endian on the wire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct PpuOpcode(pub u32);

impl PpuOpcode {
    #[must_use]
    pub const fn new(word: u32) -> Self {
        Self(word)
    }

    #[inline]
    const fn bits(self, start_bit: u32, width: u32) -> u32 {
        // PPC bit-numbering: bit 0 = MSB. Convert to LSB position.
        let lsb_pos = 32 - width - start_bit;
        (self.0 >> lsb_pos) & ((1u32 << width) - 1)
    }

    #[inline]
    #[allow(dead_code)]
    const fn sbits(self, start_bit: u32, width: u32) -> i32 {
        // Signed extract: left-shift so sign bit lands in MSB of i32,
        // then arithmetic right-shift.
        let _lsb_pos = 32 - width - start_bit;
        let _shift_left = 32 - width;
        ((self.0 as i32) << (start_bit + _lsb_pos - _lsb_pos) >> _lsb_pos) as i32
            >> 0 // unused branch trick below
                * 0
            + (((self.0 << start_bit) as i32) >> (32 - width))
    }

    // --- Primary + extension opcodes ------------------------------

    /// `main` — bits 0..=5 (primary opcode).
    #[must_use]
    pub const fn main(self) -> u32 { self.bits(0, 6) }

    // --- Branch fields --------------------------------------------

    /// `aa` — bit 30 (absolute-address branch).
    #[must_use]
    pub const fn aa(self) -> u32 { self.bits(30, 1) }
    /// `lk` — bit 31 (link bit for branch-and-link).
    #[must_use]
    pub const fn lk(self) -> u32 { self.bits(31, 1) }
    /// `bo` — bits 6..=10 (branch operation).
    #[must_use]
    pub const fn bo(self) -> u32 { self.bits(6, 5) }
    /// `bi` — bits 11..=15 (branch condition bit).
    #[must_use]
    pub const fn bi(self) -> u32 { self.bits(11, 5) }
    /// `bh` — bits 19..=20 (branch hint).
    #[must_use]
    pub const fn bh(self) -> u32 { self.bits(19, 2) }

    // --- Integer register fields ----------------------------------

    /// `rd` — bits 6..=10 (dest GPR, also `rs` / `rt` in docs).
    #[must_use]
    pub const fn rd(self) -> u32 { self.bits(6, 5) }
    /// `rs` — alias of `rd` for source register context.
    #[must_use]
    pub const fn rs(self) -> u32 { self.bits(6, 5) }
    /// `ra` — bits 11..=15.
    #[must_use]
    pub const fn ra(self) -> u32 { self.bits(11, 5) }
    /// `rb` — bits 16..=20.
    #[must_use]
    pub const fn rb(self) -> u32 { self.bits(16, 5) }

    // --- Immediate fields -----------------------------------------

    /// `uimm16` — bits 16..=31 (unsigned 16-bit immediate).
    #[must_use]
    pub const fn uimm16(self) -> u32 { self.bits(16, 16) }
    /// `simm16` — bits 16..=31 (signed 16-bit immediate).
    #[must_use]
    pub const fn simm16(self) -> i32 {
        ((self.0 & 0xFFFF) as i16) as i32
    }
    /// `ds` — bits 16..=29 (signed 14-bit immediate for ds-form loads/stores).
    #[must_use]
    pub const fn ds(self) -> i32 {
        // bits 16..29 → 14 bits, sign-extend
        let raw = (self.0 >> 2) & 0x3FFF;
        if raw & 0x2000 != 0 {
            (raw | 0xFFFF_C000) as i32
        } else {
            raw as i32
        }
    }
    /// `li` — bits 6..=29 (signed 24-bit displacement for b/bl).
    #[must_use]
    pub const fn li(self) -> i32 {
        let raw = (self.0 >> 2) & 0x00FF_FFFF;
        if raw & 0x0080_0000 != 0 {
            (raw | 0xFF00_0000) as i32
        } else {
            raw as i32
        }
    }
    /// `bt14` — branch target for conditional branches: `(ds-form displacement) << 2`.
    /// Matches the PPC BC form: branch target bits are `ds || 0b00`.
    #[must_use]
    pub const fn bt14(self) -> i32 {
        self.ds() << 2
    }
    /// `bt24` — branch target for unconditional branches: `(li displacement) << 2`.
    #[must_use]
    pub const fn bt24(self) -> i32 {
        self.li() << 2
    }

    // --- Condition register / SPR fields --------------------------

    /// `crfd` — bits 6..=8 (destination CR field, 3 bits).
    #[must_use]
    pub const fn crfd(self) -> u32 { self.bits(6, 3) }
    /// `crfs` — bits 11..=13 (source CR field, 3 bits).
    #[must_use]
    pub const fn crfs(self) -> u32 { self.bits(11, 3) }
    /// `crbd` — bits 6..=10 (destination CR bit).
    #[must_use]
    pub const fn crbd(self) -> u32 { self.bits(6, 5) }
    /// `crba` — bits 11..=15.
    #[must_use]
    pub const fn crba(self) -> u32 { self.bits(11, 5) }
    /// `crbb` — bits 16..=20.
    #[must_use]
    pub const fn crbb(self) -> u32 { self.bits(16, 5) }
    /// `crm` — bits 12..=19 (CR mask for mtcrf).
    #[must_use]
    pub const fn crm(self) -> u32 { self.bits(12, 8) }
    /// `rc` — bit 31 (record flag).
    #[must_use]
    pub const fn rc(self) -> u32 { self.bits(31, 1) }
    /// `oe` — bit 21 (overflow-enable).
    #[must_use]
    pub const fn oe(self) -> u32 { self.bits(21, 1) }
    /// `spr` — bits 11..=20 (SPR number).
    #[must_use]
    pub const fn spr(self) -> u32 { self.bits(11, 10) }
    /// `lev` — bits 20..=26 (system call level).
    #[must_use]
    pub const fn lev(self) -> u32 { self.bits(20, 7) }

    // --- Rotate / shift fields ------------------------------------

    /// `sh32` — bits 16..=20 (shift amount for 32-bit rotates).
    #[must_use]
    pub const fn sh32(self) -> u32 { self.bits(16, 5) }
    /// `mb32` — bits 21..=25 (rotate mask begin).
    #[must_use]
    pub const fn mb32(self) -> u32 { self.bits(21, 5) }
    /// `me32` — bits 26..=30 (rotate mask end).
    #[must_use]
    pub const fn me32(self) -> u32 { self.bits(26, 5) }

    // --- FPR fields -----------------------------------------------

    /// `frd` — bits 6..=10 (FPR dest).
    #[must_use]
    pub const fn frd(self) -> u32 { self.bits(6, 5) }
    /// `frs` — bits 6..=10 (FPR source).
    #[must_use]
    pub const fn frs(self) -> u32 { self.bits(6, 5) }
    /// `fra` — bits 11..=15.
    #[must_use]
    pub const fn fra(self) -> u32 { self.bits(11, 5) }
    /// `frb` — bits 16..=20.
    #[must_use]
    pub const fn frb(self) -> u32 { self.bits(16, 5) }
    /// `frc` — bits 21..=25.
    #[must_use]
    pub const fn frc(self) -> u32 { self.bits(21, 5) }
    /// `flm` — bits 7..=14 (FP mask for mtfsf).
    #[must_use]
    pub const fn flm(self) -> u32 { self.bits(7, 8) }

    // --- AltiVec / VMX fields -------------------------------------

    /// `vd` — bits 6..=10 (VR dest).
    #[must_use]
    pub const fn vd(self) -> u32 { self.bits(6, 5) }
    /// `vs` — bits 6..=10 (VR source).
    #[must_use]
    pub const fn vs(self) -> u32 { self.bits(6, 5) }
    /// `va` — bits 11..=15.
    #[must_use]
    pub const fn va(self) -> u32 { self.bits(11, 5) }
    /// `vb` — bits 16..=20.
    #[must_use]
    pub const fn vb(self) -> u32 { self.bits(16, 5) }
    /// `vc` — bits 21..=25.
    #[must_use]
    pub const fn vc(self) -> u32 { self.bits(21, 5) }
    /// `vuimm` — bits 11..=15 (AltiVec unsigned immediate).
    #[must_use]
    pub const fn vuimm(self) -> u32 { self.bits(11, 5) }
    /// `vsimm` — bits 11..=15 (AltiVec signed 5-bit immediate).
    #[must_use]
    pub const fn vsimm(self) -> i32 {
        let raw = (self.0 >> 16) & 0x1F;
        if raw & 0x10 != 0 {
            (raw | 0xFFFF_FFE0) as i32
        } else {
            raw as i32
        }
    }
    /// `vsh` — bits 22..=25.
    #[must_use]
    pub const fn vsh(self) -> u32 { self.bits(22, 4) }

    // --- Single-bit scalar fields ---------------------------------

    /// `l6` — bit 6.
    #[must_use]
    pub const fn l6(self) -> u32 { self.bits(6, 1) }
    /// `l10` — bit 10.
    #[must_use]
    pub const fn l10(self) -> u32 { self.bits(10, 1) }
    /// `l11` — bit 11.
    #[must_use]
    pub const fn l11(self) -> u32 { self.bits(11, 1) }
    /// `l15` — bit 15.
    #[must_use]
    pub const fn l15(self) -> u32 { self.bits(15, 1) }

    /// `i` — bits 16..=19.
    #[must_use]
    pub const fn i(self) -> u32 { self.bits(16, 4) }

    // --- 6-bit rotate/shift fields (primary 30) ------------------

    /// `sh64` — 6-bit shift amount, split across bit 30 (MSB) + bits 16..=20.
    /// Used by rldicl/rldicr/rldic/rldimi.
    #[must_use]
    pub const fn sh64(self) -> u32 {
        let low5 = self.bits(16, 5);
        let high1 = self.bits(30, 1);
        (high1 << 5) | low5
    }

    /// `mbe64` — 6-bit mb/me, split across bit 26 (MSB) + bits 21..=25.
    /// Used by rldicl (as mb) and rldicr (as me).
    #[must_use]
    pub const fn mbe64(self) -> u32 {
        let low5 = self.bits(21, 5);
        let high1 = self.bits(26, 1);
        (high1 << 5) | low5
    }
}

// =====================================================================
// Decoder helpers
// =====================================================================

/// Return the 17-bit lookup index used by `ppu_decoder::m_table`.
/// Mirrors `ppu_decode` at PPUOpcodes.h:70-73:
///
/// ```text
/// ((inst >> 26) | (inst << 6)) & 0x1ffff
/// ```
///
/// The rotation combines the 6-bit primary opcode with an 11-bit
/// extended opcode slice to produce a single flat index.
#[must_use]
pub const fn ppu_decode(inst: u32) -> u32 {
    ((inst >> 26) | (inst << 6)) & 0x1_FFFF
}

/// PowerPC rotate-mask helper. Generates a 64-bit mask from bit
/// positions `mb` (mask begin) and `me` (mask end). Used by
/// `rldicl`, `rldicr`, `rldic`, `rldimi`.
///
/// Mirrors `ppu_rotate_mask` at PPUOpcodes.h:64-68. The C++ relies on
/// unsigned wraparound in `(me - mb)` when `me < mb`; we use
/// `wrapping_sub` to preserve that semantics without panicking in
/// debug builds.
#[must_use]
pub const fn ppu_rotate_mask(mb: u32, me: u32) -> u64 {
    let shift_l: u32 = (!me.wrapping_sub(mb)) & 63;
    let mask: u64 = (!0u64).wrapping_shl(shift_l);
    let right = (mask).wrapping_shr(mb & 63);
    let left = (mask).wrapping_shl(64u32.wrapping_sub(mb) & 63);
    right | left
}

// =====================================================================
// Primary opcode (bits 0..5) constants — the most important slice
// =====================================================================

/// Primary opcode constants for the most commonly dispatched PPC ops.
/// These are the values of `opcode.main()` for each instruction class.
pub mod primary {
    /// `twi` — Trap word immediate.
    pub const TWI: u32 = 3;
    /// AltiVec opcode group.
    pub const VX: u32 = 4;
    /// `mulli` — Multiply low immediate.
    pub const MULLI: u32 = 7;
    /// `subfic`.
    pub const SUBFIC: u32 = 8;
    /// `cmpli` / `cmpldi` (D-form unsigned compare immediate).
    pub const CMPLI: u32 = 10;
    /// `cmpi` / `cmpdi` (D-form signed compare immediate).
    pub const CMPI: u32 = 11;
    /// `addic`.
    pub const ADDIC: u32 = 12;
    /// `addic.` (with record).
    pub const ADDIC_D: u32 = 13;
    /// `addi`.
    pub const ADDI: u32 = 14;
    /// `addis`.
    pub const ADDIS: u32 = 15;
    /// `bc` — branch conditional.
    pub const BC: u32 = 16;
    /// `sc` — system call.
    pub const SC: u32 = 17;
    /// `b` / `bl` — branch / branch-and-link.
    pub const B: u32 = 18;
    /// Branch-condition extended (bclr, bcctr).
    pub const BCEXT: u32 = 19;
    /// `rlwimi`.
    pub const RLWIMI: u32 = 20;
    /// `rlwinm`.
    pub const RLWINM: u32 = 21;
    /// `rlwnm`.
    pub const RLWNM: u32 = 23;
    /// `ori`.
    pub const ORI: u32 = 24;
    /// `oris`.
    pub const ORIS: u32 = 25;
    /// `xori`.
    pub const XORI: u32 = 26;
    /// `xoris`.
    pub const XORIS: u32 = 27;
    /// `andi.`.
    pub const ANDI_D: u32 = 28;
    /// `andis.`.
    pub const ANDIS_D: u32 = 29;
    /// Rotate doubleword (rldicl/rldicr/rldic/rldimi).
    pub const RLDI: u32 = 30;
    /// Extended arithmetic/logic (add, sub, and, or, xor, mul, div, …).
    pub const XO_ALU: u32 = 31;
    /// `lwz`.
    pub const LWZ: u32 = 32;
    /// `lwzu`.
    pub const LWZU: u32 = 33;
    /// `lbz`.
    pub const LBZ: u32 = 34;
    /// `lbzu`.
    pub const LBZU: u32 = 35;
    /// `stw`.
    pub const STW: u32 = 36;
    /// `stwu`.
    pub const STWU: u32 = 37;
    /// `stb`.
    pub const STB: u32 = 38;
    /// `stbu`.
    pub const STBU: u32 = 39;
    /// `lhz`.
    pub const LHZ: u32 = 40;
    /// `lhzu`.
    pub const LHZU: u32 = 41;
    /// `lha`.
    pub const LHA: u32 = 42;
    /// `lhau`.
    pub const LHAU: u32 = 43;
    /// `sth`.
    pub const STH: u32 = 44;
    /// `sthu`.
    pub const STHU: u32 = 45;
    /// `lmw`.
    pub const LMW: u32 = 46;
    /// `stmw`.
    pub const STMW: u32 = 47;
    /// `lfs`.
    pub const LFS: u32 = 48;
    /// `lfsu`.
    pub const LFSU: u32 = 49;
    /// `lfd`.
    pub const LFD: u32 = 50;
    /// `lfdu`.
    pub const LFDU: u32 = 51;
    /// `stfs`.
    pub const STFS: u32 = 52;
    /// `stfsu`.
    pub const STFSU: u32 = 53;
    /// `stfd`.
    pub const STFD: u32 = 54;
    /// `stfdu`.
    pub const STFDU: u32 = 55;
    /// DS-form loads (ld, lwa, ldu).
    pub const LD: u32 = 58;
    /// FP single-precision ops (fadds, fsubs, fmuls, fdivs).
    pub const FPS: u32 = 59;
    /// DS-form stores (std, stdu).
    pub const STD: u32 = 62;
    /// FP double-precision ops.
    pub const FPD: u32 = 63;
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Bitfield accessors using real PPC instructions -----------

    /// `addi r3, r0, 1` → 0x38600001
    /// main=14, rd=3, ra=0, simm16=1
    #[test]
    fn addi_r3_r0_1() {
        let op = PpuOpcode::new(0x38600001);
        assert_eq!(op.main(), primary::ADDI);
        assert_eq!(op.rd(), 3);
        assert_eq!(op.ra(), 0);
        assert_eq!(op.simm16(), 1);
    }

    /// `li r4, -1` = `addi r4, r0, -1` → 0x3880FFFF
    #[test]
    fn addi_negative_simm() {
        let op = PpuOpcode::new(0x3880_FFFF);
        assert_eq!(op.main(), primary::ADDI);
        assert_eq!(op.rd(), 4);
        assert_eq!(op.ra(), 0);
        assert_eq!(op.simm16(), -1);
    }

    /// `lwz r5, 0x100(r1)` → 0x80A10100
    #[test]
    fn lwz_fields() {
        let op = PpuOpcode::new(0x80A1_0100);
        assert_eq!(op.main(), primary::LWZ);
        assert_eq!(op.rd(), 5);
        assert_eq!(op.ra(), 1);
        assert_eq!(op.simm16(), 0x100);
    }

    /// `stw r0, 0x10(r1)` → 0x90010010
    #[test]
    fn stw_fields() {
        let op = PpuOpcode::new(0x9001_0010);
        assert_eq!(op.main(), primary::STW);
        assert_eq!(op.rs(), 0);
        assert_eq!(op.ra(), 1);
        assert_eq!(op.simm16(), 0x10);
    }

    /// `sc` system call → 0x44000002
    /// main=17, lev=0
    #[test]
    fn sc_system_call() {
        let op = PpuOpcode::new(0x4400_0002);
        assert_eq!(op.main(), primary::SC);
        assert_eq!(op.lev(), 0);
    }

    /// `blr` = `bclr 20, 0, 0` = 0x4E800020
    /// main=19 (BCEXT)
    #[test]
    fn blr_fields() {
        let op = PpuOpcode::new(0x4E80_0020);
        assert_eq!(op.main(), primary::BCEXT);
        assert_eq!(op.bo(), 20);
        assert_eq!(op.bi(), 0);
    }

    /// `b 0x1000` (relative) → 0x48001000
    /// main=18, AA=0, LK=0, li=0x400 (signed 24-bit)
    #[test]
    fn b_relative_fields() {
        let op = PpuOpcode::new(0x4800_1000);
        assert_eq!(op.main(), primary::B);
        assert_eq!(op.aa(), 0);
        assert_eq!(op.lk(), 0);
        assert_eq!(op.bt24(), 0x1000);
    }

    /// `bl 0x1000` → 0x48001001 (LK set)
    #[test]
    fn bl_has_lk_set() {
        let op = PpuOpcode::new(0x4800_1001);
        assert_eq!(op.main(), primary::B);
        assert_eq!(op.lk(), 1);
    }

    /// `b -0x1000` → 0x4BFFF000 (negative displacement)
    #[test]
    fn b_negative_displacement() {
        let op = PpuOpcode::new(0x4BFF_F000);
        assert_eq!(op.main(), primary::B);
        assert_eq!(op.bt24(), -0x1000);
    }

    /// `add r3, r4, r5` (xo-form) → 0x7C642A14
    /// main=31, rd=3, ra=4, rb=5, oe=0, xo=266, rc=0
    #[test]
    fn add_xo_form_fields() {
        let op = PpuOpcode::new(0x7C64_2A14);
        assert_eq!(op.main(), primary::XO_ALU);
        assert_eq!(op.rd(), 3);
        assert_eq!(op.ra(), 4);
        assert_eq!(op.rb(), 5);
        assert_eq!(op.oe(), 0);
        assert_eq!(op.rc(), 0);
    }

    /// `add.` = `add` with Rc=1 → 0x7C642A15
    #[test]
    fn add_record_bit_set() {
        let op = PpuOpcode::new(0x7C64_2A15);
        assert_eq!(op.rc(), 1);
    }

    // -- SPR decoding: `mtspr` / `mfspr` uses bits 11..20 ----------

    /// `mtspr LR, r3` = `mtlr r3` → 0x7C6803A6
    /// spr is encoded as `(high5 << 5) | low5` with SPR=8 for LR.
    #[test]
    fn mtspr_lr_fields() {
        let op = PpuOpcode::new(0x7C68_03A6);
        assert_eq!(op.main(), primary::XO_ALU);
        // spr encoding is swapped: low5 in bits 16..20, high5 in 11..15
        // We just check the raw 10-bit slice matches the PPC encoding.
        // mtlr LR=8 encodes as spr=0x100 (swap: 0x08 -> high5=0, low5=8 -> 0100 0b00000 1000 = 0x100? Actually
        // mtspr uses `(SPR[5..10] << 5) | SPR[0..5]`, so LR (=8) becomes:
        // low5 = 8 >> 5 = 0, high5 = 8 & 0x1F = 8 → encoded as 8 << 5 | 0 = 0x100
        assert_eq!(op.spr(), 0x100);
    }

    // -- DS-form load (ld) bits 16..=29 (14-bit signed, *4) -------

    /// `ld r3, 0x80(r1)` → 0xE8610080 (main=58, rd=3, ra=1, ds=0x20)
    /// ds-field raw = 0x20, actual offset = 0x20 * 4 = 0x80
    #[test]
    fn ld_ds_form_fields() {
        let op = PpuOpcode::new(0xE861_0080);
        assert_eq!(op.main(), primary::LD);
        assert_eq!(op.rd(), 3);
        assert_eq!(op.ra(), 1);
        assert_eq!(op.ds(), 0x20);
    }

    #[test]
    fn ld_ds_negative_offset() {
        // ds = -1 → raw 14-bit = 0x3FFF → bit pattern 0xFFFC in low 16
        let op = PpuOpcode::new(0xE861_FFFC);
        assert_eq!(op.ds(), -1);
    }

    // -- ppu_decode rotate-mask -----------------------------------

    #[test]
    fn ppu_decode_primary_is_low_6_bits() {
        // The C++ formula rotates primary opcode to the LOW 6 bits of
        // the 17-bit lookup index; the high 11 bits are PPC bits 21..31
        // (OE/XO/Rc) for XO-form ops. So extracting primary = `idx & 0x3F`.

        // addi: primary 14
        assert_eq!(ppu_decode(0x3860_0001) & 0x3F, 14);

        // add (xo-form): primary 31 (the canonical `add r3, r4, r5`)
        assert_eq!(ppu_decode(0x7C64_2A14) & 0x3F, 31);

        // b (primary 18)
        assert_eq!(ppu_decode(0x4800_1000) & 0x3F, 18);

        // sc (primary 17)
        assert_eq!(ppu_decode(0x4400_0002) & 0x3F, 17);
    }

    #[test]
    fn ppu_decode_mask_stays_17_bits() {
        for &word in &[0x0, 0xFFFF_FFFF, 0x4800_1000, 0x7C64_2214] {
            assert!(ppu_decode(word) < (1 << 17));
        }
    }

    // -- ppu_rotate_mask ------------------------------------------

    #[test]
    fn rotate_mask_full_word() {
        // mb=0, me=63 → all ones
        assert_eq!(ppu_rotate_mask(0, 63), u64::MAX);
    }

    #[test]
    fn rotate_mask_single_bit_at_msb() {
        // mb=0, me=0 → just the top bit
        assert_eq!(ppu_rotate_mask(0, 0), 0x8000_0000_0000_0000);
    }

    #[test]
    fn rotate_mask_single_bit_at_lsb() {
        // mb=63, me=63 → just the bottom bit
        assert_eq!(ppu_rotate_mask(63, 63), 1);
    }

    #[test]
    fn rotate_mask_contiguous_middle() {
        // mb=8, me=15 → 8 bits, shifted 8 from top
        // Bits 8..15 inclusive
        assert_eq!(ppu_rotate_mask(8, 15), 0x00FF_0000_0000_0000);
    }

    #[test]
    fn rotate_mask_wraps_past_end() {
        // me < mb → wraps around, giving two runs of 1s
        // mb=60, me=3 → bits 60..=63 + bits 0..=3
        let mask = ppu_rotate_mask(60, 3);
        assert_eq!(mask, 0xF000_0000_0000_000F);
    }

    // -- Primary opcode constants check ---------------------------

    #[test]
    fn primary_opcode_constants_match_spec() {
        assert_eq!(primary::ADDI, 14);
        assert_eq!(primary::ADDIS, 15);
        assert_eq!(primary::BC, 16);
        assert_eq!(primary::SC, 17);
        assert_eq!(primary::B, 18);
        assert_eq!(primary::BCEXT, 19);
        assert_eq!(primary::LWZ, 32);
        assert_eq!(primary::STW, 36);
        assert_eq!(primary::XO_ALU, 31);
        assert_eq!(primary::LD, 58);
        assert_eq!(primary::STD, 62);
    }

    // -- AltiVec field access --------------------------------------

    /// `vxor v0, v1, v2` → 0x100110C4
    /// main=4, vd=0, va=1, vb=2, extended opcode in bits 21..=30
    #[test]
    fn vxor_vmx_fields() {
        let op = PpuOpcode::new(0x1001_10C4);
        assert_eq!(op.main(), primary::VX);
        assert_eq!(op.vd(), 0);
        assert_eq!(op.va(), 1);
        assert_eq!(op.vb(), 2);
    }

    // -- Record bit / oe -------------------------------------------

    #[test]
    fn record_bit_detection() {
        let with_rc = PpuOpcode::new(0x0000_0001);
        let without_rc = PpuOpcode::new(0x0000_0000);
        assert_eq!(with_rc.rc(), 1);
        assert_eq!(without_rc.rc(), 0);
    }

    #[test]
    fn aa_and_lk_bits_independent() {
        // Bit 30 = AA, bit 31 = LK
        assert_eq!(PpuOpcode::new(0x0000_0002).aa(), 1);
        assert_eq!(PpuOpcode::new(0x0000_0001).lk(), 1);
        assert_eq!(PpuOpcode::new(0x0000_0003).aa(), 1);
        assert_eq!(PpuOpcode::new(0x0000_0003).lk(), 1);
    }
}
