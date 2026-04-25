//! `rpcs3-ppu-thread` — PPU (PowerPC 64-bit) register file and state.
//!
//! Mirrors `rpcs3/Emu/Cell/PPUThread.h:137+` (`class ppu_thread`).
//! This crate is the **state container only** — no decoder, no
//! interpreter, no JIT. Those live in `rpcs3-ppu-opcodes` and
//! `rpcs3-ppu-interpreter` (future waves).
//!
//! ## What is ABI-frozen here
//!
//! * `id_base = 0x0100_0000` — thread-class discriminant
//!   (`PPUThread.h:140`).
//! * Register file layout:
//!   * 32 × `u64` GPRs, 32 × `f64` FPRs, 32 × `v128` (u128) VRs
//!   * `cr` packed-to-32-bits view (8 condition register fields)
//!   * `lr`, `ctr`, `vrsave`, `cia`
//!   * `xer` (SO/OV/CA/cnt)
//!   * `fpscr` packed u32
//!   * `nj` non-Java mode (default `true`, per PPUThread.h:255)
//! * Default values: `vrsave = 0xFFFF_FFFF`, `nj = true`, everything
//!   else `0` (C++ `= {}` default).

use rpcs3_cpu_thread::{CpuState, ThreadClass};

// =====================================================================
// Constants
// =====================================================================

/// PPU thread-class discriminant — the top 8 bits of every PPU `ppu_thread.id`.
pub const PPU_ID_BASE: u32 = 0x0100_0000;

/// Number of PowerPC general-purpose registers.
pub const GPR_COUNT: usize = 32;
/// Number of PowerPC floating-point registers.
pub const FPR_COUNT: usize = 32;
/// Number of AltiVec (VMX) vector registers.
pub const VR_COUNT: usize = 32;

/// Default `vrsave` value after construction.
pub const VRSAVE_DEFAULT: u32 = 0xFFFF_FFFF;

// =====================================================================
// XER — Fixed-Point Exception Register (abstract representation)
// =====================================================================

/// Unpacked XER. Matches the anonymous struct at PPUThread.h:234-243.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Xer {
    /// Summary Overflow (sticky).
    pub so: bool,
    /// Overflow.
    pub ov: bool,
    /// Carry.
    pub ca: bool,
    /// Byte count for lswx/stswx, 0..=6 legally.
    pub cnt: u8,
}

impl Xer {
    /// Pack into the real PowerPC XER register layout, zero-extended to
    /// 64 bits (since SPRs are 64-bit in PPC64).
    #[must_use]
    pub const fn pack_word(self) -> u64 {
        self.pack() as u64
    }

    /// Pack into the real PowerPC XER register layout used by `mtxer`
    /// / `mfxer` — so (bit 0), ov (bit 1), ca (bit 2), reserved, cnt (bits 25..=31).
    #[must_use]
    pub const fn pack(self) -> u32 {
        let mut v: u32 = 0;
        if self.so {
            v |= 1 << 31;
        }
        if self.ov {
            v |= 1 << 30;
        }
        if self.ca {
            v |= 1 << 29;
        }
        v | (self.cnt as u32 & 0x7F)
    }

    /// Unpack from the 32-bit XER layout.
    #[must_use]
    pub const fn unpack(value: u32) -> Self {
        Self {
            so: (value & (1 << 31)) != 0,
            ov: (value & (1 << 30)) != 0,
            ca: (value & (1 << 29)) != 0,
            cnt: (value & 0x7F) as u8,
        }
    }
}

// =====================================================================
// Condition Register (CR)
// =====================================================================

/// Unpacked Condition Register — 32 bits laid out as 8 × 4-bit fields.
///
/// PowerPC CR has 8 fields (CR0..=CR7), each 4 bits wide.
/// `bits[i*4..i*4+4]` is CRi.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CrBits(pub [u8; 32]);

impl CrBits {
    /// Pack into a single u32 (bit 0 = CR0.lt, bit 31 = CR7.su).
    /// Matches PPUThread.h:183-194.
    #[must_use]
    pub fn pack(self) -> u32 {
        let mut result: u32 = 0;
        for b in self.0 {
            result <<= 1;
            result |= u32::from(b & 1);
        }
        result
    }

    /// Unpack a u32 into 32 single-bit entries. Matches PPUThread.h:197-204.
    #[must_use]
    pub fn unpack(mut value: u32) -> Self {
        let mut bits = [0u8; 32];
        for b in &mut bits {
            *b = u8::from((value & (1 << 31)) != 0);
            value <<= 1;
        }
        Self(bits)
    }

    /// Read one field (4 bits, 0..=15). `idx` must be in 0..8.
    #[must_use]
    pub fn field(&self, idx: usize) -> u8 {
        assert!(idx < 8, "CR field index must be 0..8");
        let base = idx * 4;
        let mut v = 0u8;
        for i in 0..4 {
            v = (v << 1) | (self.0[base + i] & 1);
        }
        v
    }

    /// Write one field (lower 4 bits). `idx` must be in 0..8.
    pub fn set_field(&mut self, idx: usize, value: u8) {
        assert!(idx < 8, "CR field index must be 0..8");
        let base = idx * 4;
        for i in 0..4 {
            self.0[base + i] = (value >> (3 - i)) & 1;
        }
    }
}

// =====================================================================
// PpuThread — state container
// =====================================================================

/// PPU thread state. Mirrors `ppu_thread` at PPUThread.h:137+.
///
/// Construction via `PpuThread::new(id)` applies the same defaults
/// as the C++ constructor (vrsave = 0xFFFF_FFFF, nj = true).
pub struct PpuThread {
    /// Unique id (`id_base | instance`), matching `cpu_thread::id`.
    pub id: u32,

    /// `cpu_thread::state` — atomic `cpu_flag` bitset.
    pub state: CpuState,

    // -- Architectural registers --

    /// 32 × 64-bit General-Purpose Registers.
    pub gpr: [u64; GPR_COUNT],
    /// 32 × 64-bit Floating-Point Registers.
    pub fpr: [f64; FPR_COUNT],
    /// 32 × 128-bit AltiVec Vector Registers.
    pub vr: [u128; VR_COUNT],

    /// Condition Register (unpacked into 32 bits).
    pub cr: CrBits,

    /// Floating-Point Status & Control Register (packed u32).
    pub fpscr: u32,

    /// Link Register.
    pub lr: u64,
    /// Count Register.
    pub ctr: u64,
    /// VR Save Register (default `0xFFFF_FFFF` per PPUThread.h:230).
    pub vrsave: u32,
    /// Current Instruction Address.
    pub cia: u32,

    /// Fixed-Point Exception Register (abstract form).
    pub xer: Xer,

    /// Non-Java mode (PPUThread.h:255). Default `true` = non-IEEE
    /// behaviour (denormals flushed to zero in vector FP ops).
    pub nj: bool,
}

impl core::fmt::Debug for PpuThread {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PpuThread")
            .field("id", &format_args!("0x{:08x}", self.id))
            .field("cia", &format_args!("0x{:08x}", self.cia))
            .field("lr", &format_args!("0x{:016x}", self.lr))
            .field("ctr", &self.ctr)
            .field("xer", &self.xer)
            .field("nj", &self.nj)
            .finish_non_exhaustive()
    }
}

impl PpuThread {
    /// Fresh PPU thread with `id` and all defaults per the C++ ctor.
    #[must_use]
    pub fn new(id: u32) -> Self {
        Self {
            id,
            state: CpuState::initial(),
            gpr: [0; GPR_COUNT],
            fpr: [0.0; FPR_COUNT],
            vr: [0; VR_COUNT],
            cr: CrBits::default(),
            fpscr: 0,
            lr: 0,
            ctr: 0,
            vrsave: VRSAVE_DEFAULT,
            cia: 0,
            xer: Xer::default(),
            nj: true,
        }
    }

    /// Compile-time: returns `ThreadClass::Ppu` for any id with the
    /// PPU discriminant. Useful when dispatching from a `cpu_thread`.
    #[must_use]
    pub fn thread_class(id: u32) -> ThreadClass {
        ThreadClass::from_id(id)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Defaults --------------------------------------------------

    #[test]
    fn new_thread_has_default_register_state() {
        let t = PpuThread::new(PPU_ID_BASE | 0);
        assert_eq!(t.gpr, [0u64; 32]);
        assert_eq!(t.fpr.iter().copied().all(|v| v == 0.0), true);
        assert_eq!(t.vr, [0u128; 32]);
        assert_eq!(t.lr, 0);
        assert_eq!(t.ctr, 0);
        assert_eq!(t.cia, 0);
        assert_eq!(t.fpscr, 0);
    }

    #[test]
    fn vrsave_defaults_to_all_ones() {
        let t = PpuThread::new(PPU_ID_BASE);
        assert_eq!(t.vrsave, 0xFFFF_FFFF);
    }

    #[test]
    fn nj_defaults_to_true() {
        let t = PpuThread::new(PPU_ID_BASE);
        assert!(t.nj);
    }

    #[test]
    fn xer_defaults_are_clear() {
        let t = PpuThread::new(PPU_ID_BASE);
        assert_eq!(t.xer, Xer { so: false, ov: false, ca: false, cnt: 0 });
    }

    #[test]
    fn initial_state_is_stop_plus_wait() {
        let t = PpuThread::new(PPU_ID_BASE);
        assert!(t.state.is_stopped()); // initial is stop+wait
    }

    // -- ID / class discrimination --------------------------------

    #[test]
    fn id_base_is_0x01000000() {
        assert_eq!(PPU_ID_BASE, 0x0100_0000);
    }

    #[test]
    fn thread_class_for_ppu_id_is_ppu() {
        assert_eq!(PpuThread::thread_class(PPU_ID_BASE), ThreadClass::Ppu);
        assert_eq!(PpuThread::thread_class(PPU_ID_BASE | 0xFFFF), ThreadClass::Ppu);
    }

    // -- XER pack/unpack -------------------------------------------

    #[test]
    fn xer_pack_unpack_roundtrip_all_flags() {
        let x = Xer { so: true, ov: true, ca: true, cnt: 0x42 };
        let packed = x.pack();
        assert_eq!(packed & (1 << 31), 1 << 31, "SO bit");
        assert_eq!(packed & (1 << 30), 1 << 30, "OV bit");
        assert_eq!(packed & (1 << 29), 1 << 29, "CA bit");
        assert_eq!(packed & 0x7F, 0x42, "cnt field");
        assert_eq!(Xer::unpack(packed), x);
    }

    #[test]
    fn xer_pack_clears_unused_bits() {
        let x = Xer { so: false, ov: false, ca: false, cnt: 0 };
        assert_eq!(x.pack(), 0);
    }

    #[test]
    fn xer_unpack_ignores_reserved_bits() {
        // Random reserved bits set should not leak into the struct.
        let x = Xer::unpack(0xE000_0042); // SO+OV+CA + reserved stuff + cnt=0x42
        assert!(x.so && x.ov && x.ca && x.cnt == 0x42);
    }

    // -- CR pack/unpack --------------------------------------------

    #[test]
    fn cr_pack_unpack_roundtrip() {
        let mut cr = CrBits::default();
        cr.set_field(0, 0b1010);
        cr.set_field(3, 0b0110);
        cr.set_field(7, 0b1111);
        let packed = cr.pack();
        let back = CrBits::unpack(packed);
        assert_eq!(back, cr);
    }

    #[test]
    fn cr_field_read_write_is_consistent() {
        let mut cr = CrBits::default();
        for i in 0..8 {
            cr.set_field(i, (i as u8) & 0xF);
        }
        for i in 0..8 {
            assert_eq!(cr.field(i), i as u8 & 0xF);
        }
    }

    #[test]
    fn cr_pack_all_ones_is_u32_max() {
        let mut cr = CrBits::default();
        for i in 0..8 {
            cr.set_field(i, 0xF);
        }
        assert_eq!(cr.pack(), u32::MAX);
    }

    #[test]
    fn cr_pack_all_zeros_is_zero() {
        let cr = CrBits::default();
        assert_eq!(cr.pack(), 0);
    }

    // -- Register size + alignment --------------------------------

    #[test]
    fn gpr_fpr_vr_are_correctly_sized() {
        // Compile-time invariants masquerading as runtime checks.
        assert_eq!(std::mem::size_of::<[u64; GPR_COUNT]>(), 32 * 8);
        assert_eq!(std::mem::size_of::<[f64; FPR_COUNT]>(), 32 * 8);
        assert_eq!(std::mem::size_of::<[u128; VR_COUNT]>(), 32 * 16);
    }

    #[test]
    fn gpr_fpr_counts_are_32() {
        assert_eq!(GPR_COUNT, 32);
        assert_eq!(FPR_COUNT, 32);
        assert_eq!(VR_COUNT, 32);
    }

    // -- Register write access --------------------------------------

    #[test]
    fn gpr_write_and_read_back() {
        let mut t = PpuThread::new(PPU_ID_BASE);
        t.gpr[3] = 0xDEAD_BEEF_CAFE_BABE;
        t.gpr[31] = 0x1000;
        assert_eq!(t.gpr[3], 0xDEAD_BEEF_CAFE_BABE);
        assert_eq!(t.gpr[31], 0x1000);
    }

    #[test]
    fn fpr_write_and_read_back() {
        let mut t = PpuThread::new(PPU_ID_BASE);
        t.fpr[0] = 3.14159265358979;
        assert!((t.fpr[0] - 3.14159265358979).abs() < 1e-15);
    }

    #[test]
    fn vr_write_and_read_back() {
        let mut t = PpuThread::new(PPU_ID_BASE);
        t.vr[5] = 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899;
        assert_eq!(t.vr[5], 0xAABB_CCDD_EEFF_0011_2233_4455_6677_8899);
    }
}
