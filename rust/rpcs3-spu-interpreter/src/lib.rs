//! `rpcs3-spu-interpreter` — SPU ISA interpreter, iteration 1.
//!
//! Iteration 1 covers the smallest subset that lets a synthetic SPU
//! program run end-to-end and terminate deterministically:
//!
//! * Immediate-form ALU: `il`, `ilh`, `ilhu`, `ila`, `iohl`.
//! * Register-form ALU: `a` (add word), `ah` (add half), `sf` (sub-from),
//!   `or`, `and`, `xor`, `nor`.
//! * Load/store quadword from/to the local store: `lqd`, `stqd`
//!   (d-form with register base — absolute `lqa`/`stqa` deferred to iter-2
//!   because their 9-bit primary needs the full dispatcher refactor).
//! * Control flow: `br` (relative), `bra` (absolute), `stop`, `nop`.
//!
//! The instruction set is **big-endian**, fixed-width 4 bytes. All
//! ALU ops work on 128-bit registers as 4×u32 lanes unless noted.
//!
//! Later iterations will add: halfword/byte granularities, shifts,
//! comparisons, branch-if, floating-point, SPU→PPU channels/events,
//! and full MFC DMA semantics.

use rpcs3_emu_types::CellError;
use rpcs3_spu_thread::{ChannelStatus, SpuParkReason, SpuThread, SPU_LS_SIZE};

/// What [`step`] returns after executing a single instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// Fall-through; `pc` already points at the next instruction.
    Continue,
    /// `stop` / `stopd` executed — the guest program has asked to
    /// halt. The `u32` is the stop-signal code from the instruction's
    /// immediate field.
    Stop(u32),
    /// A channel read/write would block (channel empty/full). The
    /// emu core parks the SPU thread until the counterpart services
    /// the channel. `pc` has NOT been advanced — a later `step` call
    /// will retry the same instruction.
    ChannelStall { channel: u32, is_write: bool },
    /// R7.1 — an MFC/DMA channel op (`wrch ch16-23` / `rdch ch24-25`
    /// / `rchcnt` on any of those) ran into the honest-fallback
    /// gate (`SpuChannels::refuse_mfc == true`). `pc` is the address
    /// of the refusing instruction (NOT advanced); the SPU's MFC
    /// state and the parked-thread state are both untouched. The
    /// caller (typically the C++ runtime bridge via
    /// `rust_spu_run_until_event`) is expected to drop the Rust
    /// session and fall back to the C++ SPU executor without
    /// committing any Rust state.
    MfcUnsupported { channel: u32, is_write: bool, is_count: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Reached an opcode we haven't implemented yet.
    Unimplemented { inst: u32, pc: u32, reason: &'static str },
    /// Local-store read/write out of bounds.
    BadAccess { pc: u32, lsa: u32 },
    /// Cell-level error raised by a dispatched helper.
    Cell(CellError),
}

impl From<CellError> for Error {
    fn from(value: CellError) -> Self { Self::Cell(value) }
}

// =====================================================================
// Helpers: read/write BE quadwords and u32s in the LS.
// =====================================================================

fn read_inst_be(spu: &SpuThread, pc: u32) -> Result<u32, Error> {
    let bytes = spu
        .ls_read(pc, 4)
        .ok_or(Error::BadAccess { pc, lsa: pc })?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_qword_be(spu: &SpuThread, lsa: u32) -> Result<u128, Error> {
    let aligned = lsa & !0xF;
    let bytes = spu
        .ls_read(aligned, 16)
        .ok_or(Error::BadAccess { pc: 0, lsa })?;
    // Big-endian 128-bit load — lane 0 is the high bytes.
    // try_into avoids an extra `[0u8;16] + copy_from_slice` round-trip.
    let arr: [u8; 16] = bytes.try_into().map_err(|_| Error::BadAccess { pc: 0, lsa })?;
    Ok(u128::from_be_bytes(arr))
}

fn write_qword_be(spu: &mut SpuThread, lsa: u32, v: u128) -> Result<(), Error> {
    let aligned = lsa & !0xF;
    let bytes = v.to_be_bytes();
    if !spu.ls_write(aligned, &bytes) {
        return Err(Error::BadAccess { pc: 0, lsa });
    }
    Ok(())
}

// =====================================================================
// Register helpers (lane 0 = high u32 of u128)
// =====================================================================

// Lane-0 is the high u32 of u128 (big-endian layout matches RSX/SPU
// register numbering: lane 0 = preferred slot, occupies bits 96..127).
// Bit-shift form is byte-exact with the previous to_be_bytes path on
// every platform — u128 is always little-endian-ordered limbs in
// memory but to_be_bytes/shifts both produce the same logical lanes.

#[inline]
const fn split_lanes(v: u128) -> [u32; 4] {
    [
        (v >> 96) as u32,
        (v >> 64) as u32,
        (v >> 32) as u32,
        v as u32,
    ]
}

#[inline]
const fn join_lanes(lanes: [u32; 4]) -> u128 {
    ((lanes[0] as u128) << 96)
        | ((lanes[1] as u128) << 64)
        | ((lanes[2] as u128) << 32)
        | (lanes[3] as u128)
}

#[inline]
const fn broadcast_u32(v: u32) -> u128 {
    // Multiplying by 0x0000_0001_0000_0001_0000_0001_0000_0001 splats the
    // value into all four 32-bit lanes in one mul instruction, but the
    // codegen is identical to the OR-shift form below on x86_64/ARM64
    // and the OR form is clearer.
    let w = v as u128;
    (w << 96) | (w << 64) | (w << 32) | w
}

// =====================================================================
// Convert helpers (shared by cflts/cfltu/csflt/cuflt)
// =====================================================================

/// `f32 → i32` with 2^exp_bias scaling. Saturates on overflow.
fn float_to_signed_int(bits: u32, exp_bias: i32) -> u32 {
    let f = f32::from_bits(bits);
    if !f.is_finite() {
        return if f.is_nan() { 0 } else if f > 0.0 { i32::MAX as u32 } else { i32::MIN as u32 };
    }
    let scaled = f * 2f32.powi(exp_bias);
    if scaled >= i32::MAX as f32 {
        return i32::MAX as u32;
    }
    if scaled <= i32::MIN as f32 {
        return i32::MIN as u32;
    }
    (scaled as i32) as u32
}

/// `f32 → u32` with 2^exp_bias scaling. Saturates on overflow.
fn float_to_unsigned_int(bits: u32, exp_bias: i32) -> u32 {
    let f = f32::from_bits(bits);
    if !f.is_finite() {
        return if f.is_nan() { 0 } else if f > 0.0 { u32::MAX } else { 0 };
    }
    if f < 0.0 {
        return 0;
    }
    let scaled = f * 2f32.powi(exp_bias);
    if scaled >= u32::MAX as f32 {
        return u32::MAX;
    }
    scaled as u32
}

/// `i32 → f32` scaled by 2^exp_bias.
fn signed_int_to_float(bits: u32, exp_bias: i32) -> u32 {
    let i = bits as i32;
    let f = (i as f32) * 2f32.powi(exp_bias);
    f.to_bits()
}

/// `u32 → f32` scaled by 2^exp_bias.
fn unsigned_int_to_float(bits: u32, exp_bias: i32) -> u32 {
    let f = (bits as f32) * 2f32.powi(exp_bias);
    f.to_bits()
}

// ---------------------------------------------------------------------
// SPU float helpers — denormal-flush is FTZ semantics applied per lane
// before/after IEEE op. The cpp "fast" path uses SSE primitives; we
// emulate the same observable behavior in scalar Rust.
// ---------------------------------------------------------------------

/// Treat denormals (exp == 0, mantissa != 0) as +0.0 with sign
/// preserved. Matches the SSE `_mm_andnot_ps(denorm_check, x)` idiom
/// used throughout `SPUInterpreter.cpp` (cpp:1147..1163).
#[inline]
fn flush_denorm_f32(bits: u32) -> u32 {
    if (bits & 0x7F80_0000) == 0 { 0 } else { bits }
}

/// `fcgt` per-lane (cpp:1131..1167). Flush denormals on both inputs,
/// then compare strictly-greater-than as IEEE floats.
#[inline]
fn fcmp_gt(a: u32, b: u32) -> u32 {
    let af = f32::from_bits(flush_denorm_f32(a));
    let bf = f32::from_bits(flush_denorm_f32(b));
    if af > bf { 0xFFFF_FFFF } else { 0 }
}

/// `fcmgt` per-lane (cpp:1237..1262). Compare on absolute magnitudes,
/// with denormal flush applied first.
#[inline]
fn fcmp_mgt(a: u32, b: u32) -> u32 {
    let aa = flush_denorm_f32(a) & 0x7FFF_FFFF;
    let bb = flush_denorm_f32(b) & 0x7FFF_FFFF;
    let af = f32::from_bits(aa);
    let bf = f32::from_bits(bb);
    if af > bf { 0xFFFF_FFFF } else { 0 }
}

/// `fceq` per-lane. After flush, equal-comparison via IEEE (NaN never
/// compares equal — matches `_mm_cmpeq_ps`).
#[inline]
fn fcmp_eq(a: u32, b: u32) -> u32 {
    let af = f32::from_bits(flush_denorm_f32(a));
    let bf = f32::from_bits(flush_denorm_f32(b));
    if af == bf { 0xFFFF_FFFF } else { 0 }
}

/// `fcmeq` per-lane. Magnitude equality with flush.
#[inline]
fn fcmp_meq(a: u32, b: u32) -> u32 {
    let aa = flush_denorm_f32(a) & 0x7FFF_FFFF;
    let bb = flush_denorm_f32(b) & 0x7FFF_FFFF;
    let af = f32::from_bits(aa);
    let bf = f32::from_bits(bb);
    if af == bf { 0xFFFF_FFFF } else { 0 }
}

/// `fm` per-lane (cpp:1192..1219). Flush denormals on inputs, then
/// `a*b`, then flush the result if it landed in denormal range.
#[inline]
fn fmul_flushed(a: u32, b: u32) -> u32 {
    let af = f32::from_bits(flush_denorm_f32(a));
    let bf = f32::from_bits(flush_denorm_f32(b));
    let r = af * bf;
    flush_denorm_f32(r.to_bits())
}

/// `frest` naïve approximation (cpp:690..712 uses 5-bit fraction +
/// 8-bit exponent LUT). We compute `1/x` directly with denormal flush
/// — accurate for exact powers of two, off by ≤1ulp elsewhere.
/// TODO(spu-frest-lut): port `spu_frest_fraction_lut` /
/// `spu_frest_exponent_lut` for byte-exact behavior.
#[inline]
fn frest_naive(bits: u32) -> u32 {
    let f = f32::from_bits(flush_denorm_f32(bits));
    if f == 0.0 {
        // Match the SSE behavior of dividing by zero: signed infinity.
        return if bits & 0x8000_0000 != 0 { 0xFF80_0000 } else { 0x7F80_0000 };
    }
    flush_denorm_f32((1.0_f32 / f).to_bits())
}

/// `frsqest` naïve approximation (cpp:715..735). Computes
/// `1/sqrt(|x|)` — the SPU op ignores the sign of the operand.
/// TODO(spu-frest-lut): replace with LUT path.
#[inline]
fn frsqest_naive(bits: u32) -> u32 {
    let f = f32::from_bits(flush_denorm_f32(bits & 0x7FFF_FFFF));
    if f == 0.0 {
        return 0x7F80_0000;
    }
    flush_denorm_f32((1.0_f32 / f.sqrt()).to_bits())
}

// ---------------------------------------------------------------------
// Per-word shift helpers (SPU semantics: count masked to 6 bits, but
// shifts of 32+ produce 0 for unsigned and sign-bit-fill for signed —
// matches the cpp's `(u64)val << (count & 0x3F)` then truncate idiom).
// ---------------------------------------------------------------------

#[inline]
fn shl_word(value: u32, count: u32) -> u32 {
    let n = count & 0x3F;
    if n >= 32 { 0 } else { value << n }
}

#[inline]
fn shr_word(value: u32, count: u32) -> u32 {
    // cpp `value >> ((0 - count) & 0x3f)` — count is interpreted as
    // "negative shift" form. We compute the actual right-shift count
    // explicitly.
    let n = (0u32.wrapping_sub(count)) & 0x3F;
    if n >= 32 { 0 } else { value >> n }
}

#[inline]
fn sar_word(value: u32, count: u32) -> u32 {
    let n = (0u32.wrapping_sub(count)) & 0x3F;
    let v = value as i32;
    if n >= 32 {
        // Saturate at sign bit
        if v < 0 { 0xFFFF_FFFF } else { 0 }
    } else {
        (v >> n) as u32
    }
}

/// Const-count variant for the immediate-form opcodes (ROTMI/ROTMAI).
/// `count` is already canonicalised (the dispatcher does the mask).
#[inline]
fn shr_const(value: u32, n: u32) -> u32 {
    if n >= 32 { 0 } else { value >> n }
}

#[inline]
fn sar_const(value: u32, n: u32) -> u32 {
    let v = value as i32;
    if n >= 32 {
        if v < 0 { 0xFFFF_FFFF } else { 0 }
    } else {
        (v >> n) as u32
    }
}

/// Per-halfword addition: 8 lanes of u16, modular wrap.
#[inline]
fn halfword_add(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut out = [0u8; 16];
    for i in 0..8 {
        let av = u16::from_be_bytes([ab[2*i], ab[2*i+1]]);
        let bv = u16::from_be_bytes([bb[2*i], bb[2*i+1]]);
        out[2*i..2*i+2].copy_from_slice(&av.wrapping_add(bv).to_be_bytes());
    }
    u128::from_be_bytes(out)
}

/// Per-halfword subtraction: 8 lanes of u16, computes `a - b` modular.
#[inline]
fn halfword_sub(a: u128, b: u128) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut out = [0u8; 16];
    for i in 0..8 {
        let av = u16::from_be_bytes([ab[2*i], ab[2*i+1]]);
        let bv = u16::from_be_bytes([bb[2*i], bb[2*i+1]]);
        out[2*i..2*i+2].copy_from_slice(&av.wrapping_sub(bv).to_be_bytes());
    }
    u128::from_be_bytes(out)
}

/// Apply a per-halfword binary op with the count coming from b's lane.
#[inline]
fn halfword_op<F: Fn(u16, u16) -> u16>(a: u128, b: u128, op: F) -> u128 {
    let ab = a.to_be_bytes();
    let bb = b.to_be_bytes();
    let mut out = [0u8; 16];
    for i in 0..8 {
        let av = u16::from_be_bytes([ab[2*i], ab[2*i+1]]);
        let bv = u16::from_be_bytes([bb[2*i], bb[2*i+1]]);
        out[2*i..2*i+2].copy_from_slice(&op(av, bv).to_be_bytes());
    }
    u128::from_be_bytes(out)
}

/// Apply a per-halfword unary op with a constant count.
#[inline]
fn halfword_const_op<F: Fn(u16) -> u16>(a: u128, op: F) -> u128 {
    let ab = a.to_be_bytes();
    let mut out = [0u8; 16];
    for i in 0..8 {
        let av = u16::from_be_bytes([ab[2*i], ab[2*i+1]]);
        out[2*i..2*i+2].copy_from_slice(&op(av).to_be_bytes());
    }
    u128::from_be_bytes(out)
}

#[inline]
fn halfword_shl(a: u128, b: u128) -> u128 {
    halfword_op(a, b, |av, bv| {
        let n = (bv as u32) & 0x1F;
        if n >= 16 { 0 } else { av << n }
    })
}

#[inline]
fn halfword_shr(a: u128, b: u128) -> u128 {
    halfword_op(a, b, |av, bv| {
        let n = (0u32.wrapping_sub(bv as u32)) & 0x1F;
        if n >= 16 { 0 } else { av >> n }
    })
}

#[inline]
fn halfword_sar(a: u128, b: u128) -> u128 {
    halfword_op(a, b, |av, bv| {
        let n = (0u32.wrapping_sub(bv as u32)) & 0x1F;
        let v = av as i16;
        if n >= 16 {
            if v < 0 { 0xFFFF } else { 0 }
        } else {
            (v >> n) as u16
        }
    })
}

#[inline]
fn halfword_rot(a: u128, b: u128) -> u128 {
    halfword_op(a, b, |av, bv| av.rotate_left(bv as u32 & 0xF))
}

#[inline]
fn halfword_shl_const(a: u128, n: u32) -> u128 {
    halfword_const_op(a, |av| if n >= 16 { 0 } else { av << n })
}

#[inline]
fn halfword_shr_const(a: u128, n: u32) -> u128 {
    halfword_const_op(a, |av| if n >= 16 { 0 } else { av >> n })
}

#[inline]
fn halfword_sar_const(a: u128, n: u32) -> u128 {
    halfword_const_op(a, |av| {
        let v = av as i16;
        if n >= 16 {
            if v < 0 { 0xFFFF } else { 0 }
        } else {
            (v >> n) as u16
        }
    })
}

#[inline]
fn halfword_rot_const(a: u128, n: u32) -> u128 {
    halfword_const_op(a, |av| av.rotate_left(n))
}

// =====================================================================
// Decoder — SPU uses variable-length primary opcodes. We match the
// C++ table layout (see `SPUInterpreter.cpp`).
// =====================================================================

/// Extract a `nb`-bit field starting at bit `pos` (MSB=0 numbering).
#[inline]
const fn bits(inst: u32, pos: u32, nb: u32) -> u32 {
    (inst >> (32 - pos - nb)) & ((1 << nb) - 1)
}

// Instruction field helpers (SPU ABI, MSB=0):
// * rt: bits 25..31
// * ra: bits 18..24
// * rb: bits 11..17 (register-form)
// * rc: bits  4..10
// * i7:  bits 18..24 (after 11-bit primary opcode)
// * i10: bits 15..24 (after 8-bit primary opcode)
// * i16: bits  9..24 (after 9-bit primary opcode, `br`-family)
// * i18: bits  7..24 (after 7-bit primary opcode, `ila`)

#[inline] fn rt(inst: u32) -> usize { bits(inst, 25, 7) as usize }
#[inline] fn ra(inst: u32) -> usize { bits(inst, 18, 7) as usize }
#[inline] fn rb(inst: u32) -> usize { bits(inst, 11, 7) as usize }
#[inline] fn i7(inst: u32) -> i32 {
    // RI7 format: imm7 is at MSB bits 11..17 (right after the 11-bit
    // primary opcode, where RB sits in RR form). Sign-extended.
    let v = bits(inst, 11, 7) as i32;
    if v & 0x40 != 0 { v | !0x7F } else { v }
}
#[inline] fn i10(inst: u32) -> i32 {
    // RI10 format: imm10 sits at MSB bits 8..=17 (the 10 bits right
    // after the 8-bit primary opcode). That's `inst >> 14 & 0x3FF`
    // in LE terms.
    let v = bits(inst, 8, 10) as i32;
    if v & 0x200 != 0 { v | !0x3FF } else { v }
}
#[inline] fn u16imm(inst: u32) -> u32 { bits(inst, 9, 16) }
#[inline] fn i16_rel(inst: u32) -> i32 {
    // 16-bit signed, for `br` / `brsl`.
    let v = u16imm(inst) as i32;
    if v & 0x8000 != 0 { v | !0xFFFF } else { v }
}
#[inline] fn i18(inst: u32) -> u32 { bits(inst, 7, 18) }

// =====================================================================
// Single-step executor
// =====================================================================

/// Execute one SPU instruction at `spu.pc`. Advances `pc` by 4 on
/// fall-through.
pub fn step(spu: &mut SpuThread) -> Result<StepOutcome, Error> {
    let pc = spu.pc;
    let inst = read_inst_be(spu, pc)?;

    // Match against the longest primary opcode first. We only decode
    // the subset we implement in iteration 1; everything else goes to
    // `Unimplemented`.

    // ---- stop (11-bit primary 0x000, bits 0..10) --------------
    if bits(inst, 0, 11) == 0x000 {
        // `stop` — low 14 bits carry the stop-signal code.
        let code = bits(inst, 18, 14);
        return Ok(StepOutcome::Stop(code));
    }

    // ---- nop / lnop (primary 0x001 / 0x201, same treatment) ---
    if bits(inst, 0, 11) == 0x001 || bits(inst, 0, 11) == 0x201 {
        spu.pc = pc.wrapping_add(4);
        return Ok(StepOutcome::Continue);
    }

    // ---- R5.10d — C-family insert-control opcodes -----------
    // CBX/CHX/CWX/CDX (RR-form, primaries 0x1D4..0x1D7) and
    // CBD/CHD/CWD/CDD (RI7-form, primaries 0x1F4..0x1F7) all build a
    // 16-byte `shufb` mask.
    //
    // Default mask in SPU big-endian byte order is `0x10..0x1F` linear:
    //   bytes 0..7  = 0x10 0x11 0x12 0x13 0x14 0x15 0x16 0x17
    //   bytes 8..15 = 0x18 0x19 0x1A 0x1B 0x1C 0x1D 0x1E 0x1F
    // (= "select source B" identity in SPU-BE byte order). Matches
    // the RPCS3 C++ `from64(0x18191A1B1C1D1E1F, 0x1011121314151617)`
    // when read in SPU-BE view: `_u32[3]` (preferred slot) = SPU
    // bytes 0..3 = 0x10111213; `_u32[0]` (low word) = SPU bytes
    // 12..15 = 0x1C1D1E1F. R5.11b — corrected from the prior
    // half-swapped form which made the `cwd_generates_word_insert_mask`
    // unit test self-consistent but diverge from real captured
    // RPCS3 behaviour (caught by the single_spu_loadstore_v1
    // fixture's r7 = 0x10111213 cwd output assertion).
    //
    // Then `granularity` consecutive bytes at SPU-BE byte offset
    // `addr & granularity_mask` are overwritten with `0x00 0x01 ...
    // (granularity-1)` (= "select bytes 0..(granularity-1) of source
    // A's preferred slot"). This builds the shuffle mask that
    // `shufb rt2, rA, rB, mask` consumes to insert A's preferred slot
    // into B at the chosen byte/halfword/word/doubleword position.
    //
    // RA convention: `gpr[ra]_lane0` (= preferred slot, `split_lanes(v)[0]`).
    // RB convention (RR-form): `gpr[rb]_lane0` similarly.
    // imm7 (RI7-form): sign-extended 7-bit, added as u32 (sign-extends
    // via `as u32` from i32, which gives the same wrapping_add result
    // as the C++ `op.i7 + spu.gpr[op.ra]._u32[3]` 32-bit arithmetic).
    match bits(inst, 0, 11) {
        // CBX/CHX/CWX/CDX (RR-form) — addr = ra_lane0 + rb_lane0.
        // CBD/CHD/CWD/CDD (RI7-form) — addr = ra_lane0 + sext(imm7).
        p @ (0x1D4 | 0x1D5 | 0x1D6 | 0x1D7 | 0x1F4 | 0x1F5 | 0x1F6 | 0x1F7) => {
            let granularity = match p & 0x3 {
                0x0 => 1usize, // Byte
                0x1 => 2usize, // Halfword
                0x2 => 4usize, // Word
                0x3 => 8usize, // Doubleword
                _ => unreachable!(),
            };
            let alignment_mask: u32 = match granularity {
                1 => 0xF,
                2 => 0xE,
                4 => 0xC,
                8 => 0x8,
                _ => unreachable!(),
            };
            let ra_lane0 = split_lanes(spu.gpr[ra(inst)])[0];
            let addr = if (p & 0x020) != 0 {
                // RI7-form: addr = ra_lane0 + sext(imm7) (32-bit wrapping).
                ra_lane0.wrapping_add(i7(inst) as u32)
            } else {
                // RR-form: addr = ra_lane0 + rb_lane0.
                ra_lane0.wrapping_add(split_lanes(spu.gpr[rb(inst)])[0])
            };
            let p_byte = (addr & alignment_mask) as usize;

            // Insert pattern: granularity bytes whose VALUES point into
            // source A's preferred slot. The preferred byte is A[3],
            // preferred halfword is A[2..=3], preferred word is A[0..=3],
            // preferred doubleword extends past the 4-byte preferred-
            // slot to A[0..=7]. So the start offset is `4 - g` for g ≤ 4
            // and `0` for g = 8 (matches the C++ C-family bodies bit-
            // for-bit: CBD writes 0x03, CHD writes 0x0203, CWD writes
            // 0x00010203, CDD writes 0x0001020304050607).
            let a_start: u8 = match granularity {
                1 => 3,
                2 => 2,
                4 => 0,
                8 => 0,
                _ => unreachable!(),
            };

            let mut bytes: [u8; 16] = [
                0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
                0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
            ];
            for i in 0..granularity {
                bytes[p_byte + i] = a_start + i as u8;
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(bytes);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        _ => {}
    }

    // ---- 11-bit register-form ALU -----------------------------
    match bits(inst, 0, 11) {
        // a rt, ra, rb  — word add (canonical SPU primary 0xC0)
        0x0C0 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                a[0].wrapping_add(b[0]),
                a[1].wrapping_add(b[1]),
                a[2].wrapping_add(b[2]),
                a[3].wrapping_add(b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // sf rt, ra, rb  — word sub-from (rb - ra) (canonical 0x40)
        0x040 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                b[0].wrapping_sub(a[0]),
                b[1].wrapping_sub(a[1]),
                b[2].wrapping_sub(a[2]),
                b[3].wrapping_sub(a[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // and rt, ra, rb (canonical 0xC1)
        0x0C1 => {
            spu.gpr[rt(inst)] = spu.gpr[ra(inst)] & spu.gpr[rb(inst)];
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // or rt, ra, rb
        0x041 => {
            spu.gpr[rt(inst)] = spu.gpr[ra(inst)] | spu.gpr[rb(inst)];
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // xor rt, ra, rb
        0x241 => {
            spu.gpr[rt(inst)] = spu.gpr[ra(inst)] ^ spu.gpr[rb(inst)];
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // nor rt, ra, rb
        0x049 => {
            spu.gpr[rt(inst)] = !(spu.gpr[ra(inst)] | spu.gpr[rb(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ceq rt, ra, rb  — compare equal word, lane-wise all-1s / 0
        0x3C0 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                if a[0] == b[0] { 0xFFFF_FFFF } else { 0 },
                if a[1] == b[1] { 0xFFFF_FFFF } else { 0 },
                if a[2] == b[2] { 0xFFFF_FFFF } else { 0 },
                if a[3] == b[3] { 0xFFFF_FFFF } else { 0 },
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // cgt rt, ra, rb  — signed greater-than, lane-wise
        0x240 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                if (a[0] as i32) > (b[0] as i32) { 0xFFFF_FFFF } else { 0 },
                if (a[1] as i32) > (b[1] as i32) { 0xFFFF_FFFF } else { 0 },
                if (a[2] as i32) > (b[2] as i32) { 0xFFFF_FFFF } else { 0 },
                if (a[3] as i32) > (b[3] as i32) { 0xFFFF_FFFF } else { 0 },
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // shli rt, ra, imm7 — shift left word immediate (per lane)
        0x07B => {
            let sh = (i7(inst) & 0x3F) as u32; // low 6 bits per spec
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = if sh >= 32 {
                [0, 0, 0, 0]
            } else {
                [a[0] << sh, a[1] << sh, a[2] << sh, a[3] << sh]
            };
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rotqbyi rt, ra, imm7 — rotate quadword left by imm7 & 0x0F bytes
        0x1FC => {
            let sh = (i7(inst) & 0x0F) as u32;
            let bytes = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..16u32 {
                out[i as usize] = bytes[((i + sh) & 0x0F) as usize];
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.11b — rotqby rt, ra, rb (RR-form sibling of rotqbyi 0x1FC).
        // Shift count is the low 4 bits of rb's preferred slot value.
        // Surfaced by GCC -O2 codegen for runtime-indexed extraction
        // of a 4-byte slot from a 16-byte aligned LS load (see
        // single_spu_loadstore_v1 fixture). Same byte-rotation
        // semantics as rotqbyi; only the shift-count source differs.
        0x1DC => {
            let sh = ((spu.gpr[rb(inst)] >> 96) as u32) & 0x0F;
            let bytes = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..16u32 {
                out[i as usize] = bytes[((i + sh) & 0x0F) as usize];
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10m — rotqmbyi rt, ra, imm7 (cpp:981) — quadword right-shift
        // by `(0 - imm7) & 0x1F` bytes with zero-fill (the "M" stands for
        // "mask" / zero-fill, NOT "rotate"; the name is a misnomer).
        // For our v4 site `imm7=0x7C` (=-4 signed), shift count is 4.
        // If shift count >= 16, output is all zeros.
        0x1FD => {
            let n = ((0i32.wrapping_sub(i7(inst))) & 0x1F) as usize;
            let bytes = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            if n < 16 {
                for i in n..16 {
                    out[i] = bytes[i - n];
                }
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10m — shlqbii rt, ra, imm7 (cpp:963) — quadword LEFT-shift
        // by `imm7 & 0x7` BITS (zero-fill). Pre-R5.10m this primary was
        // mistakenly assigned byte-shift semantics labeled "shlqbyi";
        // the labeling bug is now fixed: 0x1FB is SHLQBII (bit-shift),
        // and the byte-shift moves to 0x1FF (= SHLQBYI per RPCS3).
        // Shift in big-endian SPU byte order corresponds to a u128
        // left-shift in our `from_be_bytes` representation (SPU byte 0
        // = u128 MSB). For `n=0` the shift is identity; the mask `& 0x7`
        // bounds n to 0..=7 which is a safe u128 shift amount.
        0x1FB => {
            let n = (i7(inst) & 0x7) as u32;
            spu.gpr[rt(inst)] = spu.gpr[ra(inst)] << n;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10m — shlqbyi rt, ra, imm7 (cpp:990) — quadword LEFT-shift
        // by `imm7 & 0x1F` BYTES (zero-fill). Moved here from 0x1FB
        // (where it was mislabeled pre-R5.10m). For shift count >= 16,
        // output is all zeros. Identical byte-stride logic as before
        // — only the dispatch primary changed to match RPCS3.
        0x1FF => {
            let sh = (i7(inst) & 0x1F) as u32;
            let bytes = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            if sh < 16 {
                for i in 0..(16 - sh) {
                    out[i as usize] = bytes[(i + sh) as usize];
                }
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // fa rt, ra, rb — float add (4 × single-precision)
        0x2C4 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                (f32::from_bits(a[0]) + f32::from_bits(b[0])).to_bits(),
                (f32::from_bits(a[1]) + f32::from_bits(b[1])).to_bits(),
                (f32::from_bits(a[2]) + f32::from_bits(b[2])).to_bits(),
                (f32::from_bits(a[3]) + f32::from_bits(b[3])).to_bits(),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // fs rt, ra, rb — float sub
        0x2C5 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                (f32::from_bits(a[0]) - f32::from_bits(b[0])).to_bits(),
                (f32::from_bits(a[1]) - f32::from_bits(b[1])).to_bits(),
                (f32::from_bits(a[2]) - f32::from_bits(b[2])).to_bits(),
                (f32::from_bits(a[3]) - f32::from_bits(b[3])).to_bits(),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // fm rt, ra, rb (cpp:1192)  — float multiply, lane-wise.
        // Matches the "fast" SSE path: denormal-flush the inputs, do
        // the multiply, then flush the result if it landed in
        // sub-normal territory.
        0x2C6 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                fmul_flushed(a[0], b[0]),
                fmul_flushed(a[1], b[1]),
                fmul_flushed(a[2], b[2]),
                fmul_flushed(a[3], b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // mpy rt, ra, rb — half × half → word, lane-wise low 16 × low 16 (signed)
        0x3C4 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                ((a[0] & 0xFFFF) as i16 as i32 * (b[0] & 0xFFFF) as i16 as i32) as u32,
                ((a[1] & 0xFFFF) as i16 as i32 * (b[1] & 0xFFFF) as i16 as i32) as u32,
                ((a[2] & 0xFFFF) as i16 as i32 * (b[2] & 0xFFFF) as i16 as i32) as u32,
                ((a[3] & 0xFFFF) as i16 as i32 * (b[3] & 0xFFFF) as i16 as i32) as u32,
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rdch rt, ch — read channel into rt.
        // Channel number is in the `ra` field (bits 18..=24, low 7 bits).
        0x00D => {
            let channel = ra(inst) as u32 & 0x7F;
            match spu.channels.read(channel) {
                Ok(value) => {
                    // SPU channel reads return 32b; broadcast to lane 0,
                    // zero the rest.
                    spu.gpr[rt(inst)] = join_lanes([value, 0, 0, 0]);
                    spu.pc = pc.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                Err(ChannelStatus::WouldStall) => {
                    // R5.4a: park the SPU thread on this rdch. The
                    // park PC is the rdch instruction itself (not
                    // pc+4) — re-running from this PC after the
                    // mailbox refills will retry the same op.
                    spu.park_on_channel(pc, SpuParkReason::ChannelRead { channel });
                    return Ok(StepOutcome::ChannelStall { channel, is_write: false });
                }
                Err(ChannelStatus::MfcRefused) => {
                    // R7.1 honest-fallback: do NOT advance PC, do NOT
                    // park, do NOT mutate any MFC state — `SpuChannels`
                    // already short-circuited before touching anything.
                    return Ok(StepOutcome::MfcUnsupported {
                        channel,
                        is_write: false,
                        is_count: false,
                    });
                }
                Err(_) => {
                    return Err(Error::Unimplemented {
                        inst, pc,
                        reason: "rdch: unknown channel",
                    });
                }
            }
        }
        // wrch ch, rt — write rt (preferred slot) into channel.
        0x10D => {
            let channel = ra(inst) as u32 & 0x7F;
            let value = split_lanes(spu.gpr[rt(inst)])[0];
            // R7.2 / R8.1 — runtime DMA callback path for ch21
            // (MFC_Cmd). When the C++ bridge has installed a GET or
            // PUT callback, we intercept the wrch BEFORE delegating
            // to `SpuChannels::write` so the appropriate callback
            // can execute the real DMA via RPCS3 vm:: memory.
            // Pre-conditions: ch16-20 wrch already populated mfc_lsa
            // / eah / eal / size / tag_id; the captured MFC param
            // state is read straight from `spu.channels`. Validation
            // mirrors the R6.7 design § 3 supported subset (cmd ∈
            // {0x40, 0x20}, eah=0, tag<32, size in {1,2,4,8} ∪
            // {16k | k>0, 16k<=16384}, lsa+size<=256 KiB). Any
            // validation failure surfaces as `MfcUnsupported` so the
            // bridge falls back honestly.
            if channel == 21 /* MFC_CMD */
                && (spu.channels.dma_get_callback.is_some()
                    || spu.channels.dma_put_callback.is_some())
            {
                let cmd = value & 0xff;
                let lsa = spu.channels.mfc_lsa as usize;
                let eah = spu.channels.mfc_eah;
                let eal = spu.channels.mfc_eal;
                let size = spu.channels.mfc_size as usize;
                let tag = spu.channels.mfc_tag_id;
                let size_ok = size > 0
                    && size <= 0x4000
                    && (matches!(size, 1 | 2 | 4 | 8) || size.is_multiple_of(16));
                let lsa_ok = lsa.checked_add(size).map_or(false, |end| end <= SPU_LS_SIZE);
                let validated = eah == 0 && tag < 32 && size_ok && lsa_ok;
                if validated {
                    if cmd == 0x40 {
                        if let Some(cb) = spu.channels.dma_get_callback {
                            // R7.2 GET: callback writes into LS.
                            let dst_ptr = unsafe { spu.ls_mut_ptr_unchecked(lsa as u32) };
                            let rc = unsafe {
                                (cb.func)(cb.user_data, eal, dst_ptr, size as u32, tag)
                            };
                            if rc == 0 {
                                spu.channels.mfc_tag_stat_queue.push_back(1u32 << tag);
                                spu.pc = pc.wrapping_add(4);
                                return Ok(StepOutcome::Continue);
                            }
                        }
                    } else if cmd == 0x20 {
                        if let Some(cb) = spu.channels.dma_put_callback {
                            // R8.1 PUT: callback reads from LS as
                            // read-only source. We take a `*const u8`
                            // by reborrowing the LS slice.
                            let src_ptr = unsafe {
                                spu.ls_mut_ptr_unchecked(lsa as u32) as *const u8
                            };
                            let rc = unsafe {
                                (cb.func)(cb.user_data, eal, src_ptr, size as u32, tag)
                            };
                            if rc == 0 {
                                spu.channels.mfc_tag_stat_queue.push_back(1u32 << tag);
                                spu.pc = pc.wrapping_add(4);
                                return Ok(StepOutcome::Continue);
                            }
                        }
                    }
                    // cmd not in {0x40, 0x20} OR callback missing for
                    // this cmd direction OR callback returned non-zero
                    // — fall through to MfcUnsupported.
                }
                return Ok(StepOutcome::MfcUnsupported {
                    channel,
                    is_write: true,
                    is_count: false,
                });
            }
            if channel == 21 {
                // No callback installed: fall through to
                // SpuChannels::write (which applies the R7.1 refuse
                // gate or, in replay mode with refuse_mfc=false, the
                // Phase C no-op).
            }
            match spu.channels.write(channel, value) {
                Ok(()) => {
                    spu.pc = pc.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                Err(ChannelStatus::WouldStall) => {
                    // R5.4a: park on the wrch. PC is the wrch itself
                    // so a future scheduler can resume from here once
                    // the mailbox drains.
                    spu.park_on_channel(pc, SpuParkReason::ChannelWrite { channel });
                    return Ok(StepOutcome::ChannelStall { channel, is_write: true });
                }
                Err(ChannelStatus::MfcRefused) => {
                    // R7.1 honest-fallback — see comment in the rdch
                    // arm above. The wrch case is the load-bearing
                    // path for `single_spu_dma_get_v1` because the
                    // FIRST MFC channel touch is `wrch ch16 (MFC_LSA)`
                    // at the SPU's entry block, so the bridge sees
                    // this outcome before any LS / GPR / channel
                    // mutation has happened.
                    return Ok(StepOutcome::MfcUnsupported {
                        channel,
                        is_write: true,
                        is_count: false,
                    });
                }
                Err(_) => {
                    return Err(Error::Unimplemented {
                        inst, pc,
                        reason: "wrch: unknown channel",
                    });
                }
            }
        }
        // rchcnt rt, ch — read channel count into rt.
        0x00F => {
            let channel = ra(inst) as u32 & 0x7F;
            match spu.channels.count(channel) {
                Ok(count) => {
                    spu.gpr[rt(inst)] = join_lanes([count, 0, 0, 0]);
                    spu.pc = pc.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                Err(ChannelStatus::MfcRefused) => {
                    // R7.1 honest-fallback for `rchcnt` on an MFC
                    // channel. Same semantics as the rdch/wrch arms
                    // above: PC unchanged, no GPR write, no park.
                    return Ok(StepOutcome::MfcUnsupported {
                        channel,
                        is_write: false,
                        is_count: true,
                    });
                }
                Err(_) => {
                    return Err(Error::Unimplemented {
                        inst, pc,
                        reason: "rchcnt: unknown channel",
                    });
                }
            }
        }
        // clz rt, ra — count leading zeros per 4 lanes
        0x2A5 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                a[0].leading_zeros(),
                a[1].leading_zeros(),
                a[2].leading_zeros(),
                a[3].leading_zeros(),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // xsbh rt, ra — sign-extend byte to halfword (8 half-words per reg)
        0x2B6 => {
            let bytes = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            // Each odd byte keeps its byte, preceding even byte becomes
            // 0xFF or 0x00 based on sign.
            for i in (0..16).step_by(2) {
                let signed = bytes[i + 1] as i8;
                out[i] = if signed < 0 { 0xFF } else { 0x00 };
                out[i + 1] = bytes[i + 1];
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // xshw rt, ra — sign-extend halfword to word (4 words)
        0x2AE => {
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in (0..16).step_by(4) {
                let low_half = i16::from_be_bytes([a[i + 2], a[i + 3]]);
                let extended = low_half as i32 as u32;
                out[i..i + 4].copy_from_slice(&extended.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // xswd rt, ra — sign-extend word to doubleword (2 dwords)
        0x2A6 => {
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in (0..16).step_by(8) {
                let low_word = i32::from_be_bytes([a[i + 4], a[i + 5], a[i + 6], a[i + 7]]);
                let extended = low_word as i64 as u64;
                out[i..i + 8].copy_from_slice(&extended.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // cntb rt, ra — per-byte popcount (lane-wise popcount of each byte)
        0x2B4 => {
            let bytes = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..16 {
                out[i] = bytes[i].count_ones() as u8;
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // mpyu rt, ra, rb — unsigned half × half → word
        0x3CC => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                ((a[0] & 0xFFFF) * (b[0] & 0xFFFF)),
                ((a[1] & 0xFFFF) * (b[1] & 0xFFFF)),
                ((a[2] & 0xFFFF) * (b[2] & 0xFFFF)),
                ((a[3] & 0xFFFF) * (b[3] & 0xFFFF)),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // clgt rt, ra, rb  — logical (unsigned) greater-than
        0x2C0 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                if a[0] > b[0] { 0xFFFF_FFFF } else { 0 },
                if a[1] > b[1] { 0xFFFF_FFFF } else { 0 },
                if a[2] > b[2] { 0xFFFF_FFFF } else { 0 },
                if a[3] > b[3] { 0xFFFF_FFFF } else { 0 },
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Single-precision float compares (FCGT/FCMGT/FCEQ/FCMEQ)
        // Semantics from `SPUInterpreter.cpp` "fast" handlers (anonymous
        // namespace, lines 1131..1265). The fast path on x86 uses SSE
        // primitives with denormal flush before the comparison; we
        // mirror that lane-by-lane in scalar Rust via the helpers
        // declared in the convert section above.

        // fcgt rt, ra, rb (cpp:1131)  — float-compare-greater-than with
        // denormal flush on both operands.
        0x2C2 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                fcmp_gt(a[0], b[0]),
                fcmp_gt(a[1], b[1]),
                fcmp_gt(a[2], b[2]),
                fcmp_gt(a[3], b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // fcmgt rt, ra, rb (cpp:1237)  — magnitude compare-gt: |a| > |b|.
        0x2CA => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                fcmp_mgt(a[0], b[0]),
                fcmp_mgt(a[1], b[1]),
                fcmp_mgt(a[2], b[2]),
                fcmp_mgt(a[3], b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // fceq rt, ra, rb  — float compare-equal (bit-pattern after
        // denormal flush; NaN never compares equal).
        0x3C2 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                fcmp_eq(a[0], b[0]),
                fcmp_eq(a[1], b[1]),
                fcmp_eq(a[2], b[2]),
                fcmp_eq(a[3], b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // fcmeq rt, ra, rb  — magnitude compare-equal: |a| == |b|.
        0x3CA => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                fcmp_meq(a[0], b[0]),
                fcmp_meq(a[1], b[1]),
                fcmp_meq(a[2], b[2]),
                fcmp_meq(a[3], b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Float reciprocal estimates (FREST/FRSQEST) ==========
        // The C++ uses 5-bit fraction-LUT + 8-bit exponent-LUT
        // (cpp:690..735). For the iter-2 wave we use the IEEE direct
        // form `1/x` and `1/sqrt(x)` as a *starting* approximation
        // (denormal flush on input) — this is byte-exact only for
        // exact-power-of-two inputs, but enough for getting code that
        // doesn't rely on the exact bit-pattern of the estimate.
        // TODO(spu-frest-lut): replace with the LUT path when we port
        // `spu_frest_*_lut` (binary tables ~256 + 32 entries).
        0x1B8 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                frest_naive(a[0]),
                frest_naive(a[1]),
                frest_naive(a[2]),
                frest_naive(a[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0x1B9 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                frsqest_naive(a[0]),
                frsqest_naive(a[1]),
                frsqest_naive(a[2]),
                frsqest_naive(a[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== FSM rt, ra  (cpp:661)  — form select mask, word ====
        // Picks the low 4 bits of ra's preferred slot. Bit i (i=0..3)
        // expands into element (3-i)'s 32-bit lane (because the cpp
        // uses _mm_set_epi32(8,4,2,1): element 0 tests bit 3, etc.).
        0x1B4 => {
            let m = split_lanes(spu.gpr[ra(inst)])[0] & 0xF;
            let r = [
                if m & 0x8 != 0 { 0xFFFF_FFFF } else { 0 },
                if m & 0x4 != 0 { 0xFFFF_FFFF } else { 0 },
                if m & 0x2 != 0 { 0xFFFF_FFFF } else { 0 },
                if m & 0x1 != 0 { 0xFFFF_FFFF } else { 0 },
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== FSMH rt, ra (cpp:670) — form select mask, halfword =====
        // R5.10f. Take the low 8 bits of ra's preferred slot. Bit i
        // (i=0..7) expands into halfword (7-i): bit 7 → halfword 0,
        // bit 6 → halfword 1, ..., bit 0 → halfword 7. Each halfword is
        // 0xFFFF if its bit is set, else 0x0000. SPU big-endian byte
        // order: halfword 0 occupies SPU bytes 0..1.
        0x1B5 => {
            let m = (split_lanes(spu.gpr[ra(inst)])[0] & 0xFF) as u32;
            let mut bytes = [0u8; 16];
            for h in 0..8 {
                let bit = 7 - h;
                if m & (1 << bit) != 0 {
                    bytes[h * 2]     = 0xFF;
                    bytes[h * 2 + 1] = 0xFF;
                }
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(bytes);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== FSMB rt, ra (cpp:680) — form select mask, byte =========
        // R5.10f. Take the low 16 bits of ra's preferred slot. Bit i
        // (i=0..15) expands into byte (15-i): bit 15 → SPU byte 0, ...,
        // bit 0 → SPU byte 15. Each byte is 0xFF if its bit is set,
        // else 0x00. (Same shape as FSMBI but the source is `ra` bits
        // instead of an immediate.)
        0x1B6 => {
            let m = (split_lanes(spu.gpr[ra(inst)])[0] & 0xFFFF) as u32;
            let mut bytes = [0u8; 16];
            for k in 0..16 {
                let bit = 15 - k;
                if m & (1 << bit) != 0 {
                    bytes[k] = 0xFF;
                }
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(bytes);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Indexed load/store (LQX/STQX) =====================
        // cpp:738..742. addr = (ra[3] + rb[3]) & 0x3fff0 in C++ LE
        // terms — that's our lane 0 + lane 0, masked to 16-byte align.
        0x1C4 => {
            // lqx rt, ra, rb
            let base = split_lanes(spu.gpr[ra(inst)])[0];
            let off = split_lanes(spu.gpr[rb(inst)])[0];
            let lsa = base.wrapping_add(off) & 0x3FFF0;
            let v = read_qword_be(spu, lsa)?;
            spu.gpr[rt(inst)] = v;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0x144 => {
            // stqx rt, ra, rb
            let base = split_lanes(spu.gpr[ra(inst)])[0];
            let off = split_lanes(spu.gpr[rb(inst)])[0];
            let lsa = base.wrapping_add(off) & 0x3FFF0;
            write_qword_be(spu, lsa, spu.gpr[rt(inst)])?;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Branches indirect (BI / BISL / IRET / BIZ family) ===
        // All take ra's preferred slot as the target address, masked
        // to 4-byte alignment within the 256 KB local store
        // (& 0x3fffc per `spu_branch_target` in SPUOpcodes.h:53).

        // bi ra (cpp opcode 0x1a8)  — unconditional indirect branch
        0x1A8 => {
            let target = split_lanes(spu.gpr[ra(inst)])[0] & 0x3FFFC;
            spu.pc = target;
            return Ok(StepOutcome::Continue);
        }
        // bisl rt, ra (0x1a9)  — branch indirect with link.
        // rt gets the next-pc (broadcast across all lanes per ABI).
        0x1A9 => {
            let target = split_lanes(spu.gpr[ra(inst)])[0] & 0x3FFFC;
            let link = pc.wrapping_add(4) & 0x3FFFC;
            spu.gpr[rt(inst)] = broadcast_u32(link);
            spu.pc = target;
            return Ok(StepOutcome::Continue);
        }
        // iret ra (0x1aa)  — interrupt return. Without modeled
        // interrupts it degenerates to BI (target = ra preferred).
        0x1AA => {
            let target = split_lanes(spu.gpr[ra(inst)])[0] & 0x3FFFC;
            spu.pc = target;
            return Ok(StepOutcome::Continue);
        }
        // hbr ra, ro (0x1ac)  — branch hint. NOP for the interpreter,
        // pure recompiler hint to prefetch the indirect target.
        0x1AC => {
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // biz rt, ra (0x128)  — indirect branch if rt preferred == 0
        0x128 => {
            let cond = split_lanes(spu.gpr[rt(inst)])[0] == 0;
            spu.pc = if cond {
                split_lanes(spu.gpr[ra(inst)])[0] & 0x3FFFC
            } else {
                pc.wrapping_add(4)
            };
            return Ok(StepOutcome::Continue);
        }
        // binz rt, ra (0x129)  — opposite of biz
        0x129 => {
            let cond = split_lanes(spu.gpr[rt(inst)])[0] != 0;
            spu.pc = if cond {
                split_lanes(spu.gpr[ra(inst)])[0] & 0x3FFFC
            } else {
                pc.wrapping_add(4)
            };
            return Ok(StepOutcome::Continue);
        }
        // bihz rt, ra (0x12a)  — preferred-slot low-half == 0
        0x12A => {
            let cond = (split_lanes(spu.gpr[rt(inst)])[0] & 0xFFFF) == 0;
            spu.pc = if cond {
                split_lanes(spu.gpr[ra(inst)])[0] & 0x3FFFC
            } else {
                pc.wrapping_add(4)
            };
            return Ok(StepOutcome::Continue);
        }
        // bihnz rt, ra (0x12b)  — preferred-slot low-half != 0
        0x12B => {
            let cond = (split_lanes(spu.gpr[rt(inst)])[0] & 0xFFFF) != 0;
            spu.pc = if cond {
                split_lanes(spu.gpr[ra(inst)])[0] & 0x3FFFC
            } else {
                pc.wrapping_add(4)
            };
            return Ok(StepOutcome::Continue);
        }

        // ===== Iter-9: shifts vetoriais por palavra (cpp:287..339) ===
        // SPU shift count comes from the **same lane** of rb (per-lane
        // shift, not broadcast). Counts are masked to 6 bits (0..63);
        // shifts of 32+ produce 0 / sign-bit because the cpp does the
        // arithmetic in u64/s64 then truncates back to u32/s32.

        // shl rt, ra, rb  — logical shift left per word
        0x5B => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                shl_word(a[0], b[0]),
                shl_word(a[1], b[1]),
                shl_word(a[2], b[2]),
                shl_word(a[3], b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rot rt, ra, rb  — rotate left per word, count = rb mod 32
        0x58 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                a[0].rotate_left(b[0] & 0x1F),
                a[1].rotate_left(b[1] & 0x1F),
                a[2].rotate_left(b[2] & 0x1F),
                a[3].rotate_left(b[3] & 0x1F),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rotm rt, ra, rb  — logical shift right per word, count = -rb & 0x3F
        0x59 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                shr_word(a[0], b[0]),
                shr_word(a[1], b[1]),
                shr_word(a[2], b[2]),
                shr_word(a[3], b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rotma rt, ra, rb  — arithmetic shift right per word, count = -rb & 0x3F
        0x5A => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                sar_word(a[0], b[0]),
                sar_word(a[1], b[1]),
                sar_word(a[2], b[2]),
                sar_word(a[3], b[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Iter-9: shift word imediatos (RR-form, count em rb's
        // bits 25..31 = same encoding slot as rt — the cpp uses op.i7
        // which maps to the i7 slot at 11..17. But the actual opcode
        // uses the rb field as the shift count. We follow the cpp
        // semantics: count comes from the i7 slot.

        // roti rt, ra, imm7  — rotate left per word, count = i7 & 0x1F
        0x78 => {
            let n = bits(inst, 11, 7) & 0x1F;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                a[0].rotate_left(n),
                a[1].rotate_left(n),
                a[2].rotate_left(n),
                a[3].rotate_left(n),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rotmi rt, ra, imm7  — logical shr per word, count = (-i7) & 0x3F
        0x79 => {
            let n = (0u32.wrapping_sub(bits(inst, 11, 7))) & 0x3F;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                shr_const(a[0], n),
                shr_const(a[1], n),
                shr_const(a[2], n),
                shr_const(a[3], n),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rotmai rt, ra, imm7  — arith shr per word, count = (-i7) & 0x3F
        0x7A => {
            let n = (0u32.wrapping_sub(bits(inst, 11, 7))) & 0x3F;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                sar_const(a[0], n),
                sar_const(a[1], n),
                sar_const(a[2], n),
                sar_const(a[3], n),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Iter-9: bitwise complementares =======================

        // nand rt, ra, rb  (cpp:487)  — ~(a & b)
        0xC9 => {
            spu.gpr[rt(inst)] = !(spu.gpr[ra(inst)] & spu.gpr[rb(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // eqv rt, ra, rb  (cpp:1026)  — ~(a ^ b) = a XNOR b
        0x249 => {
            spu.gpr[rt(inst)] = !(spu.gpr[ra(inst)] ^ spu.gpr[rb(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // andc rt, ra, rb  (cpp:1124)  — a & ~b
        0x2C1 => {
            spu.gpr[rt(inst)] = spu.gpr[ra(inst)] & !spu.gpr[rb(inst)];
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // orc rt, ra, rb  (cpp:1230)  — a | ~b
        0x2C9 => {
            spu.gpr[rt(inst)] = spu.gpr[ra(inst)] | !spu.gpr[rb(inst)];
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Iter-9: barriers + alias stops =======================

        // sync (0x002), dsync (0x003)  — memory/instruction barriers.
        // For the interpreter both are NOPs (no out-of-order execution
        // to fence). Encoded with all RT/RA/RB = 0 in the canonical
        // form, but we accept any low-bits since the SPU ignores them.
        0x002 | 0x003 => {
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // stopd ra, rb, rc (0x140)  — stop and signal in privileged
        // mode. cpp:579 just sets PC and stops. For our purposes it
        // behaves identically to `stop` with code 0.
        0x140 => {
            return Ok(StepOutcome::Stop(0));
        }

        // ===== Iter-9: compares estendidos (byte/halfword) ==========

        // ceqh rt, ra, rb (cpp:1485)  — eq-compare per halfword (8 lanes)
        0x3C8 => {
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let b = spu.gpr[rb(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..8 {
                let av = u16::from_be_bytes([a[2*i], a[2*i+1]]);
                let bv = u16::from_be_bytes([b[2*i], b[2*i+1]]);
                let mask: u16 = if av == bv { 0xFFFF } else { 0 };
                out[2*i..2*i+2].copy_from_slice(&mask.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ceqb rt, ra, rb (cpp:1516)  — eq-compare per byte (16 lanes)
        0x3D0 => {
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let b = spu.gpr[rb(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..16 {
                out[i] = if a[i] == b[i] { 0xFF } else { 0 };
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // cgth rt, ra, rb (cpp:1019)  — signed gt per halfword
        0x248 => {
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let b = spu.gpr[rb(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..8 {
                let av = i16::from_be_bytes([a[2*i], a[2*i+1]]);
                let bv = i16::from_be_bytes([b[2*i], b[2*i+1]]);
                let mask: u16 = if av > bv { 0xFFFF } else { 0 };
                out[2*i..2*i+2].copy_from_slice(&mask.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // cgtb rt, ra, rb  — signed gt per byte
        0x250 => {
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let b = spu.gpr[rb(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..16 {
                out[i] = if (a[i] as i8) > (b[i] as i8) { 0xFF } else { 0 };
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // clgth rt, ra, rb  — unsigned gt per halfword
        0x2C8 => {
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let b = spu.gpr[rb(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..8 {
                let av = u16::from_be_bytes([a[2*i], a[2*i+1]]);
                let bv = u16::from_be_bytes([b[2*i], b[2*i+1]]);
                let mask: u16 = if av > bv { 0xFFFF } else { 0 };
                out[2*i..2*i+2].copy_from_slice(&mask.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // clgtb rt, ra, rb  — unsigned gt per byte
        0x2D0 => {
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let b = spu.gpr[rb(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..16 {
                out[i] = if a[i] > b[i] { 0xFF } else { 0 };
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Iter-10: halfword arithmetic + carry/borrow ========

        // ah rt, ra, rb (cpp:480)  — add per halfword (8 lanes)
        0x0C8 => {
            spu.gpr[rt(inst)] = halfword_add(spu.gpr[ra(inst)], spu.gpr[rb(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // sfh rt, ra, rb (cpp:264)  — sub from halfword: rt = rb - ra
        0x048 => {
            spu.gpr[rt(inst)] = halfword_sub(spu.gpr[rb(inst)], spu.gpr[ra(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // cg rt, ra, rb (cpp:471)  — carry generate per word: 1 if
        // ra+rb overflows u32, else 0.
        0x0C2 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                ((a[0] as u64 + b[0] as u64) >> 32) as u32,
                ((a[1] as u64 + b[1] as u64) >> 32) as u32,
                ((a[2] as u64 + b[2] as u64) >> 32) as u32,
                ((a[3] as u64 + b[3] as u64) >> 32) as u32,
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // bg rt, ra, rb (cpp:257)  — borrow generate: 1 if rb-ra does
        // NOT underflow (i.e. ra ≤ rb), else 0. Matches SPU convention
        // that `sf rt,ra,rb` computes rb-ra.
        0x042 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                if a[0] <= b[0] { 1 } else { 0 },
                if a[1] <= b[1] { 1 } else { 0 },
                if a[2] <= b[2] { 1 } else { 0 },
                if a[3] <= b[3] { 1 } else { 0 },
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // orx rt, ra (cpp:882)  — OR-across all word lanes of ra,
        // result lands in preferred slot of rt; other lanes zero.
        0x1F0 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let or_all = a[0] | a[1] | a[2] | a[3];
            spu.gpr[rt(inst)] = join_lanes([or_all, 0, 0, 0]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Iter-11: halfword shifts (cpp:341..394) ==============
        // Per-halfword shift: 8 lanes of u16, count = b lane masked to
        // 5 bits; shifts of 16+ produce 0 / sign-fill.

        // roth rt, ra, rb (cpp:342)  — rotate left per halfword
        0x05C => {
            spu.gpr[rt(inst)] = halfword_rot(spu.gpr[ra(inst)], spu.gpr[rb(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rothm rt, ra, rb (cpp:355)  — logical shr per halfword
        0x05D => {
            spu.gpr[rt(inst)] = halfword_shr(spu.gpr[ra(inst)], spu.gpr[rb(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rotmah rt, ra, rb (cpp:369)  — arith shr per halfword
        0x05E => {
            spu.gpr[rt(inst)] = halfword_sar(spu.gpr[ra(inst)], spu.gpr[rb(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // shlh rt, ra, rb (cpp:382)  — logical shl per halfword
        0x05F => {
            spu.gpr[rt(inst)] = halfword_shl(spu.gpr[ra(inst)], spu.gpr[rb(inst)]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== Iter-11: halfword shift immediates (cpp:427..453) ====
        // Count comes from i7 slot, masked appropriately. Same as
        // word-shift immediates (rotmi/rotmai/roti/shli) but at
        // halfword granularity.

        // rothi rt, ra, imm7 (cpp:427)  — rotate left per halfword
        0x07C => {
            let n = bits(inst, 11, 7) & 0xF;
            spu.gpr[rt(inst)] = halfword_rot_const(spu.gpr[ra(inst)], n);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rothmi rt, ra, imm7 (cpp:436)  — logical shr per halfword
        0x07D => {
            let n = (0u32.wrapping_sub(bits(inst, 11, 7))) & 0x1F;
            spu.gpr[rt(inst)] = halfword_shr_const(spu.gpr[ra(inst)], n);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // rotmahi rt, ra, imm7 (cpp:443)  — arith shr per halfword
        0x07E => {
            let n = (0u32.wrapping_sub(bits(inst, 11, 7))) & 0x1F;
            spu.gpr[rt(inst)] = halfword_sar_const(spu.gpr[ra(inst)], n);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // shlhi rt, ra, imm7 (cpp:450)  — logical shl per halfword
        0x07F => {
            let n = bits(inst, 11, 7) & 0x1F;
            spu.gpr[rt(inst)] = halfword_shl_const(spu.gpr[ra(inst)], n);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        _ => {}
    }

    // ---- 9-bit primary (branches) -----------------------------
    match bits(inst, 0, 9) {
        // br i16 — relative, word offset << 2
        0x064 => {
            let offset = i16_rel(inst) * 4;
            spu.pc = ((pc as i64).wrapping_add(offset as i64)) as u32 & (SPU_LS_SIZE as u32 - 1);
            return Ok(StepOutcome::Continue);
        }
        // bra i16 — absolute, word offset << 2
        0x060 => {
            let target = (i16_rel(inst) as u32).wrapping_mul(4) & (SPU_LS_SIZE as u32 - 1);
            spu.pc = target;
            return Ok(StepOutcome::Continue);
        }
        // brnz rt, i16 — branch if preferred-slot (lane 0) != 0
        0x042 => {
            let cond = split_lanes(spu.gpr[rt(inst)])[0] != 0;
            if cond {
                let offset = i16_rel(inst) * 4;
                spu.pc = ((pc as i64).wrapping_add(offset as i64)) as u32 & (SPU_LS_SIZE as u32 - 1);
                return Ok(StepOutcome::Continue);
            }
            // Fall-through to normal advance.
        }
        // brz rt, i16 — branch if preferred-slot == 0
        0x040 => {
            let cond = split_lanes(spu.gpr[rt(inst)])[0] == 0;
            if cond {
                let offset = i16_rel(inst) * 4;
                spu.pc = ((pc as i64).wrapping_add(offset as i64)) as u32 & (SPU_LS_SIZE as u32 - 1);
                return Ok(StepOutcome::Continue);
            }
        }
        // brsl rt, i16 (cpp:1681)  — branch relative with link.
        // rt = next-pc broadcast; pc = pc + (i16 << 2), masked to LS.
        0x066 => {
            let target = ((pc as i64).wrapping_add((i16_rel(inst) * 4) as i64)) as u32
                & (SPU_LS_SIZE as u32 - 1);
            let link = pc.wrapping_add(4) & 0x3FFFC;
            spu.gpr[rt(inst)] = broadcast_u32(link);
            spu.pc = target;
            return Ok(StepOutcome::Continue);
        }
        // R5.10b — lqr rt, imm16 (cpp:1690) — load qword PC-relative.
        // Target = (pc + (imm16 << 2)) & 0x3FFF0 — same as RPCS3 C++
        // `spu_ls_target(pc, imm16)`: LS-mask AND 16-byte align (the
        // `& 0x3FFF0` does both — bottom 4 bits forced to 0 for the
        // qword alignment, top bits masked to the 256 KiB LS).
        // No channels, no FP, no DMA, no branches — pure LS→GPR load.
        0x067 => {
            let target = ((pc as i64).wrapping_add((i16_rel(inst) * 4) as i64)) as u32
                & 0x3FFF0;
            let v = read_qword_be(spu, target)?;
            spu.gpr[rt(inst)] = v;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10g — stqr rt, imm16 (cpp:1634) — store qword PC-relative.
        // Direct mirror of LQR (0x067): same `spu_ls_target(pc, imm16)`
        // address — `(pc + (imm16<<2)) & 0x3FFF0`, LS-mask AND 16-byte
        // align — just `LS[target..+16] = gpr[rt]` instead of the load.
        // No channels, no DMA, no FP, no atomics, no branches.
        0x047 => {
            let target = ((pc as i64).wrapping_add((i16_rel(inst) * 4) as i64)) as u32
                & 0x3FFF0;
            write_qword_be(spu, target, spu.gpr[rt(inst)])?;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10o — lqa rt, imm16 (cpp:1648) — load qword absolute.
        // Mirror of LQR (0x067) but with PC=0:
        //   target = (imm16 << 2) & 0x3FFF0
        // Pure LS read; no PC contribution to the address. Negative
        // imm16 wraps via the `& 0x3FFF0` mask onto the top of LS.
        0x061 => {
            let target = ((i16_rel(inst) * 4) as u32) & 0x3FFF0;
            spu.gpr[rt(inst)] = read_qword_be(spu, target)?;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10o — stqa rt, imm16 (cpp:1594) — store qword absolute.
        // Mirror of LQA above (0x061) with reversed direction; mirror
        // of STQR (0x047) with PC=0 in the address calc.
        0x041 => {
            let target = ((i16_rel(inst) * 4) as u32) & 0x3FFF0;
            write_qword_be(spu, target, spu.gpr[rt(inst)])?;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10f — fsmbi rt, imm16 (cpp:1671) — form select mask from
        // 16-bit immediate. Byte k of rt = 0xFF iff bit (15-k) of imm16
        // is set, else 0x00. SPU big-endian byte 0 → high u64 byte. No
        // ra/rb access, no LS read/write, no channels, no FP, no
        // branches — pure i16 → 16-byte mask compute.
        0x065 => {
            let m = u16imm(inst);
            let mut bytes = [0u8; 16];
            for k in 0..16 {
                let bit = 15 - k;
                if m & (1 << bit) != 0 {
                    bytes[k] = 0xFF;
                }
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(bytes);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        _ => {}
    }

    // `brnz`/`brz` reach here only on fall-through (cond was false).
    // Dispatch on the 9-bit primary one more time so we can `continue`
    // the outer dispatcher cleanly.
    if matches!(bits(inst, 0, 9), 0x042 | 0x040) {
        spu.pc = pc.wrapping_add(4);
        return Ok(StepOutcome::Continue);
    }

    // ---- 10-bit primary RI8 dispatch (convert ops with scale imm) --
    // cflts=0x1D8 / cfltu=0x1D9 / csflt=0x1DA / cuflt=0x1DB.
    // In RI8 format the 8-bit immediate sits at bits 10..17 (MSB=0).
    // It's a scale factor: the result is multiplied by 2^(173 - scale)
    // for float→int, or 2^(scale - 155) for int→float. We implement
    // scale = the conventional "no scaling" value (173 for float→int,
    // 155 for int→float), producing identity conversions when the
    // guest passes those sentinels. Other scales are approximated
    // with a power-of-two adjustment.
    match bits(inst, 0, 10) {
        0x1D8 => {
            // cflts rt, ra, scale  — f32 → signed i32, lane-wise
            let scale = bits(inst, 10, 8) as i32;
            let exp_bias = 173 - scale;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                float_to_signed_int(a[0], exp_bias),
                float_to_signed_int(a[1], exp_bias),
                float_to_signed_int(a[2], exp_bias),
                float_to_signed_int(a[3], exp_bias),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0x1D9 => {
            // cfltu rt, ra, scale  — f32 → unsigned u32
            let scale = bits(inst, 10, 8) as i32;
            let exp_bias = 173 - scale;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                float_to_unsigned_int(a[0], exp_bias),
                float_to_unsigned_int(a[1], exp_bias),
                float_to_unsigned_int(a[2], exp_bias),
                float_to_unsigned_int(a[3], exp_bias),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0x1DA => {
            // csflt rt, ra, scale  — signed i32 → f32
            let scale = bits(inst, 10, 8) as i32;
            let exp_bias = scale - 155;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                signed_int_to_float(a[0], exp_bias),
                signed_int_to_float(a[1], exp_bias),
                signed_int_to_float(a[2], exp_bias),
                signed_int_to_float(a[3], exp_bias),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0x1DB => {
            // cuflt rt, ra, scale  — unsigned u32 → f32
            let scale = bits(inst, 10, 8) as i32;
            let exp_bias = scale - 155;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                unsigned_int_to_float(a[0], exp_bias),
                unsigned_int_to_float(a[1], exp_bias),
                unsigned_int_to_float(a[2], exp_bias),
                unsigned_int_to_float(a[3], exp_bias),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        _ => {}
    }

    // ---- 4-bit RRR-form dispatch (selb / shufb / fma / fnms / fms)
    // Layout: primary (4) | rt (7) | rb (7) | ra (7) | rc (7)
    // R5.11b — `rt` (destination) is at bits 4..=10 (which is what the
    // assembler/disassembler outputs); `rc` (4th source) is at bits
    // 25..=31. Pre-R5.11b the dispatch had rt and rc field positions
    // swapped (matching pack_rrr's mirror error), making the executor
    // self-consistent against synthetic-encoded fixtures but
    // divergent from real captured binaries. Corrected together with
    // pack_rrr; both encoder and decoder now match the real SPU
    // encoding.
    match bits(inst, 0, 4) {
        0x8 => {
            // selb rt, ra, rb, rc  — (rc & rb) | (!rc & ra) bit-wise.
            let rt_idx = bits(inst, 4, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 25, 7) as usize;
            let a = spu.gpr[ra_idx];
            let b = spu.gpr[rb_idx];
            let c = spu.gpr[rc_idx];
            spu.gpr[rt_idx] = (c & b) | (!c & a);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0xB => {
            // shufb rt, ra, rb, rc  — per-byte permutation.
            // For each output byte (16 total), the selector byte in rc
            // picks one of 32 input bytes (ra bytes 0..15, rb bytes
            // 0..15) or produces a constant based on the high bits.
            let rt_idx = bits(inst, 4, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 25, 7) as usize;
            let a = spu.gpr[ra_idx].to_be_bytes();
            let b = spu.gpr[rb_idx].to_be_bytes();
            let c = spu.gpr[rc_idx].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..16 {
                let sel = c[i];
                // High 3 bits decide constant patterns:
                //   0b10xxxxxx → 0x00
                //   0b110xxxxx → 0xFF
                //   0b111xxxxx → 0x80
                // Else pick byte: sel & 0x1F, first 16 → ra, next 16 → rb.
                out[i] = if sel & 0xC0 == 0x80 {
                    0x00
                } else if sel & 0xE0 == 0xC0 {
                    0xFF
                } else if sel & 0xE0 == 0xE0 {
                    0x80
                } else {
                    let idx = (sel & 0x1F) as usize;
                    if idx < 16 { a[idx] } else { b[idx - 16] }
                };
            }
            spu.gpr[rt_idx] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0xE => {
            // fma rt, ra, rb, rc — rt = ra*rb + rc (lane-wise f32).
            // R5.11b — corrected RRR-form rt/rc field positions.
            let rt_idx = bits(inst, 4, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 25, 7) as usize;
            let a = split_lanes(spu.gpr[ra_idx]);
            let b = split_lanes(spu.gpr[rb_idx]);
            let c = split_lanes(spu.gpr[rc_idx]);
            let r = [
                (f32::from_bits(a[0]) * f32::from_bits(b[0]) + f32::from_bits(c[0])).to_bits(),
                (f32::from_bits(a[1]) * f32::from_bits(b[1]) + f32::from_bits(c[1])).to_bits(),
                (f32::from_bits(a[2]) * f32::from_bits(b[2]) + f32::from_bits(c[2])).to_bits(),
                (f32::from_bits(a[3]) * f32::from_bits(b[3]) + f32::from_bits(c[3])).to_bits(),
            ];
            spu.gpr[rt_idx] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0xD => {
            // fnms rt, ra, rb, rc — rt = rc - ra*rb (lane-wise f32).
            // R5.11b — corrected RRR-form rt/rc field positions.
            let rt_idx = bits(inst, 4, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 25, 7) as usize;
            let a = split_lanes(spu.gpr[ra_idx]);
            let b = split_lanes(spu.gpr[rb_idx]);
            let c = split_lanes(spu.gpr[rc_idx]);
            let r = [
                (f32::from_bits(c[0]) - f32::from_bits(a[0]) * f32::from_bits(b[0])).to_bits(),
                (f32::from_bits(c[1]) - f32::from_bits(a[1]) * f32::from_bits(b[1])).to_bits(),
                (f32::from_bits(c[2]) - f32::from_bits(a[2]) * f32::from_bits(b[2])).to_bits(),
                (f32::from_bits(c[3]) - f32::from_bits(a[3]) * f32::from_bits(b[3])).to_bits(),
            ];
            spu.gpr[rt_idx] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        0xF => {
            // fms rt, ra, rb, rc — rt = ra*rb - rc (lane-wise f32).
            // R5.11b — corrected RRR-form rt/rc field positions.
            let rt_idx = bits(inst, 4, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 25, 7) as usize;
            let a = split_lanes(spu.gpr[ra_idx]);
            let b = split_lanes(spu.gpr[rb_idx]);
            let c = split_lanes(spu.gpr[rc_idx]);
            let r = [
                (f32::from_bits(a[0]) * f32::from_bits(b[0]) - f32::from_bits(c[0])).to_bits(),
                (f32::from_bits(a[1]) * f32::from_bits(b[1]) - f32::from_bits(c[1])).to_bits(),
                (f32::from_bits(a[2]) * f32::from_bits(b[2]) - f32::from_bits(c[2])).to_bits(),
                (f32::from_bits(a[3]) * f32::from_bits(b[3]) - f32::from_bits(c[3])).to_bits(),
            ];
            spu.gpr[rt_idx] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        _ => {}
    }

    // ---- 8-bit primary (lqd/stqd, load/store quadword with d-form)
    match bits(inst, 0, 8) {
        // lqd rt, imm7*16(ra)  — load qword, offset = sext(i10) * 16
        0x34 => {
            let off = i10(inst).wrapping_mul(16);
            let base = split_lanes(spu.gpr[ra(inst)])[0];
            let lsa = base.wrapping_add_signed(off) & (SPU_LS_SIZE as u32 - 1);
            let v = read_qword_be(spu, lsa)?;
            spu.gpr[rt(inst)] = v;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // andi rt, ra, imm10 — AND immediate word (sext-imm10)
        0x14 => {
            let imm = i10(inst) as u32;
            let a = split_lanes(spu.gpr[ra(inst)]);
            spu.gpr[rt(inst)] = join_lanes([a[0] & imm, a[1] & imm, a[2] & imm, a[3] & imm]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ori rt, ra, imm10 — OR immediate word
        0x04 => {
            let imm = i10(inst) as u32;
            let a = split_lanes(spu.gpr[ra(inst)]);
            spu.gpr[rt(inst)] = join_lanes([a[0] | imm, a[1] | imm, a[2] | imm, a[3] | imm]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // xori rt, ra, imm10 — XOR immediate word
        0x44 => {
            let imm = i10(inst) as u32;
            let a = split_lanes(spu.gpr[ra(inst)]);
            spu.gpr[rt(inst)] = join_lanes([a[0] ^ imm, a[1] ^ imm, a[2] ^ imm, a[3] ^ imm]);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ai rt, ra, imm10  — add immediate word (signed 10-bit, broadcast)
        0x1C => {
            let imm = i10(inst);
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                a[0].wrapping_add_signed(imm),
                a[1].wrapping_add_signed(imm),
                a[2].wrapping_add_signed(imm),
                a[3].wrapping_add_signed(imm),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ceqi rt, ra, imm10  — compare-equal immediate word
        0x7C => {
            let imm = i10(inst) as u32;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                if a[0] == imm { 0xFFFF_FFFF } else { 0 },
                if a[1] == imm { 0xFFFF_FFFF } else { 0 },
                if a[2] == imm { 0xFFFF_FFFF } else { 0 },
                if a[3] == imm { 0xFFFF_FFFF } else { 0 },
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // cgti rt, ra, imm10  — signed greater-than immediate
        0x4C => {
            let imm = i10(inst);
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                if (a[0] as i32) > imm { 0xFFFF_FFFF } else { 0 },
                if (a[1] as i32) > imm { 0xFFFF_FFFF } else { 0 },
                if (a[2] as i32) > imm { 0xFFFF_FFFF } else { 0 },
                if (a[3] as i32) > imm { 0xFFFF_FFFF } else { 0 },
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10k — clgti rt, ra, imm10 (cpp:1862) — UNSIGNED word gt
        // imm. si10 sign-extended to i32, then compared as u32 against
        // each word lane of ra. Result lane = 0xFFFFFFFF/0.
        0x5C => {
            let imm_u = i10(inst) as u32;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                if a[0] > imm_u { 0xFFFF_FFFF } else { 0 },
                if a[1] > imm_u { 0xFFFF_FFFF } else { 0 },
                if a[2] > imm_u { 0xFFFF_FFFF } else { 0 },
                if a[3] > imm_u { 0xFFFF_FFFF } else { 0 },
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10k — sfi rt, ra, imm10 (cpp:1747) — per-word
        // `sext(si10) - gpr[ra]` (note operand order: imm minus ra).
        0x0C => {
            let imm = i10(inst);
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                (imm as u32).wrapping_sub(a[0]),
                (imm as u32).wrapping_sub(a[1]),
                (imm as u32).wrapping_sub(a[2]),
                (imm as u32).wrapping_sub(a[3]),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10k — ahi rt, ra, imm10 (cpp:1789) — per-HALFWORD add.
        // si10 broadcast as i16 across 8 halfword lanes; halfword-wise
        // add with wraparound.
        0x1D => {
            let imm_h = i10(inst) as i16 as u16;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..8 {
                let av = u16::from_be_bytes([a[2*i], a[2*i+1]]);
                let r  = av.wrapping_add(imm_h);
                out[2*i..2*i+2].copy_from_slice(&r.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10k — mpyi rt, ra, imm10 (cpp:1893) — per-WORD signed
        // 16x16->32 multiply: take low 16 bits of each ra word as an
        // i16, multiply by sext(si10) interpreted as i16; sign-extend
        // the 32-bit product to fill the word lane. The C++ uses
        // `_mm_madd_epi16(a, set1_epi32(si10 & 0xFFFF))` which puts 0
        // in the high i16 of each broadcast lane, so the high half of
        // `a` is multiplied by 0 and only the low half contributes.
        0x74 => {
            let imm_i16 = i10(inst) as i16;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                ((a[0] as i16) as i32).wrapping_mul(imm_i16 as i32) as u32,
                ((a[1] as i16) as i32).wrapping_mul(imm_i16 as i32) as u32,
                ((a[2] as i16) as i32).wrapping_mul(imm_i16 as i32) as u32,
                ((a[3] as i16) as i32).wrapping_mul(imm_i16 as i32) as u32,
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // R5.10k — mpyui rt, ra, imm10 (cpp:1900) — per-WORD UNSIGNED
        // 16x16->32 multiply. Same shape as MPYI but unsigned: take
        // low 16 bits of each ra word as u16, multiply by `(si10 &
        // 0xFFFF) as u16`; full 32-bit unsigned product fills the
        // word lane.
        0x75 => {
            let imm_u16 = (i10(inst) & 0xFFFF) as u32;
            let a = split_lanes(spu.gpr[ra(inst)]);
            let r = [
                (a[0] & 0xFFFF).wrapping_mul(imm_u16),
                (a[1] & 0xFFFF).wrapping_mul(imm_u16),
                (a[2] & 0xFFFF).wrapping_mul(imm_u16),
                (a[3] & 0xFFFF).wrapping_mul(imm_u16),
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // stqd rt, imm10*16(ra)  — store qword
        0x24 => {
            let off = i10(inst).wrapping_mul(16);
            let base = split_lanes(spu.gpr[ra(inst)])[0];
            let lsa = base.wrapping_add_signed(off) & (SPU_LS_SIZE as u32 - 1);
            write_qword_be(spu, lsa, spu.gpr[rt(inst)])?;
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ===== Iter-10: halfword-immediate compares (RI10 form) =====
        // imm10 is signed 10-bit, broadcast across all 8 halfword lanes.

        // ceqhi rt, ra, imm10 (cpp:1916)
        0x7D => {
            let imm = i10(inst) as i16 as u16;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..8 {
                let av = u16::from_be_bytes([a[2*i], a[2*i+1]]);
                let mask: u16 = if av == imm { 0xFFFF } else { 0 };
                out[2*i..2*i+2].copy_from_slice(&mask.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // cgthi rt, ra, imm10 (cpp:1838)  — signed gt halfword imm
        0x4D => {
            let imm = i10(inst) as i16;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..8 {
                let av = i16::from_be_bytes([a[2*i], a[2*i+1]]);
                let mask: u16 = if av > imm { 0xFFFF } else { 0 };
                out[2*i..2*i+2].copy_from_slice(&mask.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // clgthi rt, ra, imm10 (cpp:1869)  — unsigned gt halfword imm
        0x5D => {
            let imm = i10(inst) as i16 as u16;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for i in 0..8 {
                let av = u16::from_be_bytes([a[2*i], a[2*i+1]]);
                let mask: u16 = if av > imm { 0xFFFF } else { 0 };
                out[2*i..2*i+2].copy_from_slice(&mask.to_be_bytes());
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        // ===== R5.10i: byte-immediate RI10 family ===================
        // All 6 byte-imm opcodes take the same `i8` field (bits 14..21,
        // LSB-0) and broadcast it to all 16 bytes of the 128-bit
        // operand, then apply the per-byte op against `gpr[ra]`.
        // C++ refs: SPUInterpreter.cpp:1740 (ORBI), 1775 (ANDBI),
        // 1824 (XORBI), 1845 (CGTBI), 1876 (CLGTBI), 1923 (CEQBI).
        // The decoder (R5.10i) extracts `i8` correctly; we read it
        // back via `(inst >> 14) & 0xFF` here because the interpreter
        // dispatches on raw bits (not on decoder variants).

        // orbi rt, ra, i8 (cpp:1740)
        0x06 => {
            let i8b = ((inst >> 14) & 0xFF) as u8;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for k in 0..16 { out[k] = a[k] | i8b; }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // andbi rt, ra, i8 (cpp:1775)
        0x16 => {
            let i8b = ((inst >> 14) & 0xFF) as u8;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for k in 0..16 { out[k] = a[k] & i8b; }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // xorbi rt, ra, i8 (cpp:1824)
        0x46 => {
            let i8b = ((inst >> 14) & 0xFF) as u8;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for k in 0..16 { out[k] = a[k] ^ i8b; }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // cgtbi rt, ra, i8 (cpp:1845) — SIGNED byte greater-than
        0x4E => {
            let i8s = ((inst >> 14) & 0xFF) as u8 as i8;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for k in 0..16 {
                out[k] = if (a[k] as i8) > i8s { 0xFF } else { 0x00 };
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // clgtbi rt, ra, i8 (cpp:1876) — UNSIGNED byte greater-than.
        // C++ uses _mm_xor_si128(gpr[ra], 0x80808080) and a signed
        // _mm_cmpgt_epi8 against (i8 ^ 0x80) — the standard XOR-trick
        // for unsigned compare. The plain `as u8` compare is the
        // direct equivalent.
        0x5E => {
            let i8u = ((inst >> 14) & 0xFF) as u8;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for k in 0..16 {
                out[k] = if a[k] > i8u { 0xFF } else { 0x00 };
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ceqbi rt, ra, i8 (cpp:1923) — byte equality
        0x7E => {
            let i8b = ((inst >> 14) & 0xFF) as u8;
            let a = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            for k in 0..16 {
                out[k] = if a[k] == i8b { 0xFF } else { 0x00 };
            }
            spu.gpr[rt(inst)] = u128::from_be_bytes(out);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }

        _ => {}
    }

    // ---- 9-bit primary (immediate-form ALU) -------------------
    match bits(inst, 0, 9) {
        // il rt, i16  — load immediate (sign-extended)
        0x081 => {
            let sim = (u16imm(inst) as i16) as i32 as u32;
            spu.gpr[rt(inst)] = broadcast_u32(sim);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ilh rt, i16  — load halfword immediate (broadcast 8 halves)
        0x083 => {
            let h = (u16imm(inst) & 0xFFFF) as u16;
            let packed = ((h as u32) << 16) | h as u32;
            spu.gpr[rt(inst)] = broadcast_u32(packed);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // ilhu rt, i16  — load halfword immediate, upper half
        0x082 => {
            let packed = (u16imm(inst) & 0xFFFF) << 16;
            spu.gpr[rt(inst)] = broadcast_u32(packed);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        // iohl rt, i16  — OR immediate, low half (preserves high)
        0x0C1 => {
            let cur = split_lanes(spu.gpr[rt(inst)]);
            let masked = u16imm(inst) & 0xFFFF;
            let r = [
                cur[0] | masked,
                cur[1] | masked,
                cur[2] | masked,
                cur[3] | masked,
            ];
            spu.gpr[rt(inst)] = join_lanes(r);
            spu.pc = pc.wrapping_add(4);
            return Ok(StepOutcome::Continue);
        }
        _ => {}
    }

    // ---- 7-bit primary: branch hints HBRA / HBRR (cpp:1941..1947)
    // Both are interpreter NOPs (recompiler-only prefetch hints).
    if matches!(bits(inst, 0, 7), 0x08 | 0x09) {
        spu.pc = pc.wrapping_add(4);
        return Ok(StepOutcome::Continue);
    }

    // ---- 7-bit primary (ila = 18-bit immediate load) ----------
    if bits(inst, 0, 7) == 0x21 {
        spu.gpr[rt(inst)] = broadcast_u32(i18(inst));
        spu.pc = pc.wrapping_add(4);
        return Ok(StepOutcome::Continue);
    }

    Err(Error::Unimplemented {
        inst,
        pc,
        reason: "opcode not in iteration-1 subset",
    })
}

/// Run up to `max_steps` instructions, stopping early on `Stop` or error.
pub fn run_n(spu: &mut SpuThread, max_steps: usize) -> Result<(usize, StepOutcome), Error> {
    for i in 0..max_steps {
        match step(spu)? {
            StepOutcome::Stop(code) => return Ok((i + 1, StepOutcome::Stop(code))),
            StepOutcome::ChannelStall { channel, is_write } => {
                return Ok((i + 1, StepOutcome::ChannelStall { channel, is_write }));
            }
            // R7.1 — early exit on MFC refusal so the caller can hand
            // off to the C++ executor before any further SPU state
            // is mutated. PC has not been advanced past the refusing
            // instruction.
            StepOutcome::MfcUnsupported { channel, is_write, is_count } => {
                return Ok((i + 1, StepOutcome::MfcUnsupported { channel, is_write, is_count }));
            }
            StepOutcome::Continue => {}
        }
    }
    Ok((max_steps, StepOutcome::Continue))
}

// =====================================================================
// Encoders — for fixture tests
// =====================================================================

pub mod encode {
    /// `stop` with 14-bit code.
    #[must_use]
    pub const fn stop(code: u32) -> u32 {
        // primary 0x000 in bits 0..10. Low bits hold the code at bits 18..31.
        (code & 0x3FFF) << 0
    }
    /// `nop` — primary 0x001.
    #[must_use]
    pub const fn nop() -> u32 {
        0x001u32 << 21
    }

    const fn pack_rr(primary_11: u32, rt: u32, ra: u32, rb: u32) -> u32 {
        ((primary_11 & 0x7FF) << 21)
            | ((rb & 0x7F) << 14)
            | ((ra & 0x7F) << 7)
            | (rt & 0x7F)
    }

    /// `a rt, ra, rb`
    #[must_use]
    pub const fn a(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x0C0, rt, ra, rb) }
    /// `sf rt, ra, rb` — rt = rb - ra
    #[must_use]
    pub const fn sf(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x040, rt, ra, rb) }
    /// `and rt, ra, rb`
    #[must_use]
    pub const fn and(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x0C1, rt, ra, rb) }
    /// `or rt, ra, rb`
    #[must_use]
    pub const fn or(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x041, rt, ra, rb) }
    /// `xor rt, ra, rb`
    #[must_use]
    pub const fn xor(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x241, rt, ra, rb) }
    /// `nor rt, ra, rb`
    #[must_use]
    pub const fn nor(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x049, rt, ra, rb) }

    const fn pack_ri16(primary_9: u32, rt: u32, imm16: u16) -> u32 {
        ((primary_9 & 0x1FF) << 23) | ((imm16 as u32) << 7) | (rt & 0x7F)
    }
    /// `il rt, imm16` — signed, broadcast.
    #[must_use]
    pub const fn il(rt: u32, imm16: i16) -> u32 {
        pack_ri16(0x081, rt, imm16 as u16)
    }
    /// `ilh rt, imm16` — halfword broadcast.
    #[must_use]
    pub const fn ilh(rt: u32, imm16: u16) -> u32 { pack_ri16(0x083, rt, imm16) }
    /// `ilhu rt, imm16` — upper-half immediate.
    #[must_use]
    pub const fn ilhu(rt: u32, imm16: u16) -> u32 { pack_ri16(0x082, rt, imm16) }
    /// `iohl rt, imm16` — OR into low half.
    #[must_use]
    pub const fn iohl(rt: u32, imm16: u16) -> u32 { pack_ri16(0x0C1, rt, imm16) }
    /// `br imm16` — relative branch (signed halfword offset).
    #[must_use]
    pub const fn br(imm16: i16) -> u32 {
        ((0x064u32) << 23) | ((imm16 as u16 as u32) << 7)
    }

    /// `lqd rt, imm10(ra)` — load qword, imm10 is in qword units.
    #[must_use]
    pub const fn lqd(rt: u32, ra: u32, imm10: i16) -> u32 {
        let imm = (imm10 as u32) & 0x3FF;
        ((0x34u32) << 24) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }
    /// R5.10b — `lqr rt, imm16` — load qword PC-relative.
    /// imm16 is a signed halfword offset; target =
    /// `(pc + (imm16 << 2)) & 0x3FFF0` (LS-mask + 16-byte align).
    /// Encoding: 9-bit primary 0x067 at bits 0..8, signed imm16 at
    /// bits 9..24, rt at bits 25..31.
    #[must_use]
    pub const fn lqr(rt: u32, imm16: i16) -> u32 {
        pack_ri16(0x067, rt, imm16 as u16)
    }
    /// R5.10g — `stqr rt, imm16` — store qword PC-relative. Mirror of
    /// `lqr`; same encoding shape, primary `0x047` instead of `0x067`.
    #[must_use]
    pub const fn stqr(rt: u32, imm16: i16) -> u32 {
        pack_ri16(0x047, rt, imm16 as u16)
    }
    /// R5.10o — `lqa rt, imm16` — load qword ABSOLUTE. Same RI16
    /// encoding shape as LQR; primary `0x061`. Address calc is
    /// PC-independent: `(imm16 << 2) & 0x3FFF0`.
    #[must_use]
    pub const fn lqa(rt: u32, imm16: i16) -> u32 {
        pack_ri16(0x061, rt, imm16 as u16)
    }
    /// R5.10o — `stqa rt, imm16` — store qword ABSOLUTE. Mirror of
    /// `lqa`; primary `0x041`.
    #[must_use]
    pub const fn stqa(rt: u32, imm16: i16) -> u32 {
        pack_ri16(0x041, rt, imm16 as u16)
    }

    // R5.10d — C-family insert-control encoders. RR-form packs rb at
    // bits 11..17; RI7-form packs imm7 at the same field. Helpers are
    // used by the interpreter unit tests; they encode the canonical
    // 11-bit primary opcode followed by the appropriate body.
    const fn pack_rr_11(p11: u32, rt: u32, ra: u32, rb: u32) -> u32 {
        ((p11 & 0x7FF) << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }
    const fn pack_ri7_11(p11: u32, rt: u32, ra: u32, imm7: i8) -> u32 {
        ((p11 & 0x7FF) << 21) | (((imm7 as u32) & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }

    /// R5.10d — `cbx rt, ra, rb`  — generate byte-insert controls (RR).
    #[must_use]
    pub const fn cbx(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr_11(0x1D4, rt, ra, rb) }
    /// R5.10d — `chx rt, ra, rb`  — generate halfword-insert controls (RR).
    #[must_use]
    pub const fn chx(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr_11(0x1D5, rt, ra, rb) }
    /// R5.10d — `cwx rt, ra, rb`  — generate word-insert controls (RR).
    #[must_use]
    pub const fn cwx(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr_11(0x1D6, rt, ra, rb) }
    /// R5.10d — `cdx rt, ra, rb`  — generate doubleword-insert controls (RR).
    #[must_use]
    pub const fn cdx(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr_11(0x1D7, rt, ra, rb) }
    /// R5.10d — `cbd rt, imm7(ra)` — generate byte-insert controls (RI7).
    #[must_use]
    pub const fn cbd(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7_11(0x1F4, rt, ra, imm7) }
    /// R5.10d — `chd rt, imm7(ra)` — generate halfword-insert controls (RI7).
    #[must_use]
    pub const fn chd(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7_11(0x1F5, rt, ra, imm7) }
    /// R5.10d — `cwd rt, imm7(ra)` — generate word-insert controls (RI7).
    #[must_use]
    pub const fn cwd(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7_11(0x1F6, rt, ra, imm7) }
    /// R5.10d — `cdd rt, imm7(ra)` — generate doubleword-insert controls (RI7).
    #[must_use]
    pub const fn cdd(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7_11(0x1F7, rt, ra, imm7) }
    /// `stqd rt, imm10(ra)`
    #[must_use]
    pub const fn stqd(rt: u32, ra: u32, imm10: i16) -> u32 {
        let imm = (imm10 as u32) & 0x3FF;
        ((0x24u32) << 24) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }
    // ---- Iter-2 opcodes ----------------------------------------------

    /// `ceq rt, ra, rb` — lane-wise word equality test.
    #[must_use]
    pub const fn ceq(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x3C0, rt, ra, rb) }
    /// `cgt rt, ra, rb` — lane-wise signed greater-than.
    #[must_use]
    pub const fn cgt(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x240, rt, ra, rb) }
    /// `clgt rt, ra, rb` — lane-wise unsigned greater-than.
    #[must_use]
    pub const fn clgt(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2C0, rt, ra, rb) }

    const fn pack_8_i10(primary_8: u32, rt: u32, ra: u32, imm10: i16) -> u32 {
        let imm = (imm10 as u32) & 0x3FF;
        ((primary_8 & 0xFF) << 24) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }
    /// `ai rt, ra, imm10` — add immediate word.
    #[must_use]
    pub const fn ai(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x1C, rt, ra, imm10) }
    /// `ceqi rt, ra, imm10`
    #[must_use]
    pub const fn ceqi(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x7C, rt, ra, imm10) }
    /// `cgti rt, ra, imm10`
    #[must_use]
    pub const fn cgti(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x4C, rt, ra, imm10) }
    /// R5.10k — `clgti rt, ra, imm10` — unsigned word gt-than imm.
    #[must_use]
    pub const fn clgti(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x5C, rt, ra, imm10) }
    /// R5.10k — `sfi rt, ra, imm10` — per-word `sext(si10) - gpr[ra]`.
    #[must_use]
    pub const fn sfi(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x0C, rt, ra, imm10) }
    /// R5.10k — `ahi rt, ra, imm10` — per-halfword add immediate.
    #[must_use]
    pub const fn ahi(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x1D, rt, ra, imm10) }
    /// R5.10k — `mpyi rt, ra, imm10` — per-word signed 16x16->32 multiply.
    #[must_use]
    pub const fn mpyi(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x74, rt, ra, imm10) }
    /// R5.10k — `mpyui rt, ra, imm10` — per-word unsigned 16x16->32 multiply.
    #[must_use]
    pub const fn mpyui(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x75, rt, ra, imm10) }

    /// `brnz rt, imm16` — branch if preferred-slot != 0.
    #[must_use]
    pub const fn brnz(rt: u32, imm16: i16) -> u32 {
        ((0x042u32) << 23) | ((imm16 as u16 as u32) << 7) | (rt & 0x7F)
    }
    /// `brz rt, imm16` — branch if preferred-slot == 0.
    #[must_use]
    pub const fn brz(rt: u32, imm16: i16) -> u32 {
        ((0x040u32) << 23) | ((imm16 as u16 as u32) << 7) | (rt & 0x7F)
    }

    // ---- Iter-3 opcodes ----------------------------------------------

    /// `mpy rt, ra, rb` — signed half × half → word.
    #[must_use]
    pub const fn mpy(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x3C4, rt, ra, rb) }
    /// `mpyu rt, ra, rb` — unsigned half × half → word.
    #[must_use]
    pub const fn mpyu(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x3CC, rt, ra, rb) }

    /// `andi rt, ra, imm10`
    #[must_use]
    pub const fn andi(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x14, rt, ra, imm10) }
    /// `ori rt, ra, imm10`
    #[must_use]
    pub const fn ori(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x04, rt, ra, imm10) }
    /// `xori rt, ra, imm10`
    #[must_use]
    pub const fn xori(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x44, rt, ra, imm10) }

    /// R5.10i — pack a byte-immediate RI10 instruction. The 8-bit imm
    /// field occupies LSB-0 bits 14..21 (matching RPCS3 `bf_t<u32, 14,
    /// 8> i8`); the upper 2 bits of the 10-bit immediate slot (bits
    /// 22..23) are forced to 0 so the encoded inst matches what real
    /// compilers emit. The decoder reads back via `(raw >> 14) & 0xFF`.
    const fn pack_8_i8(primary_8: u32, rt: u32, ra: u32, i8v: i8) -> u32 {
        let imm = (i8v as u8) as u32;
        ((primary_8 & 0xFF) << 24) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }
    /// R5.10i — `orbi rt, ra, i8`
    #[must_use]
    pub const fn orbi(rt: u32, ra: u32, i8v: i8) -> u32 { pack_8_i8(0x06, rt, ra, i8v) }
    /// R5.10i — `andbi rt, ra, i8`
    #[must_use]
    pub const fn andbi(rt: u32, ra: u32, i8v: i8) -> u32 { pack_8_i8(0x16, rt, ra, i8v) }
    /// R5.10i — `xorbi rt, ra, i8`
    #[must_use]
    pub const fn xorbi(rt: u32, ra: u32, i8v: i8) -> u32 { pack_8_i8(0x46, rt, ra, i8v) }
    /// R5.10i — `cgtbi rt, ra, i8` (signed byte greater-than)
    #[must_use]
    pub const fn cgtbi(rt: u32, ra: u32, i8v: i8) -> u32 { pack_8_i8(0x4E, rt, ra, i8v) }
    /// R5.10i — `clgtbi rt, ra, i8` (unsigned byte greater-than)
    #[must_use]
    pub const fn clgtbi(rt: u32, ra: u32, i8v: i8) -> u32 { pack_8_i8(0x5E, rt, ra, i8v) }
    /// R5.10i — `ceqbi rt, ra, i8` (byte equality)
    #[must_use]
    pub const fn ceqbi(rt: u32, ra: u32, i8v: i8) -> u32 { pack_8_i8(0x7E, rt, ra, i8v) }

    /// Pack an RI7 instruction: primary (11) | imm7 (7, MSB 11..17) | ra (7) | rt (7).
    const fn pack_ri7(primary_11: u32, rt: u32, ra: u32, imm7: i8) -> u32 {
        let imm = (imm7 as u32) & 0x7F;
        ((primary_11 & 0x7FF) << 21) | (imm << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }

    /// `shli rt, ra, imm7` — shift left word immediate.
    #[must_use]
    pub const fn shli(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x07B, rt, ra, imm7) }
    /// `rotqbyi rt, ra, imm7` — rotate quadword left by imm7 bytes.
    #[must_use]
    pub const fn rotqbyi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x1FC, rt, ra, imm7) }
    /// R5.11b — `rotqby rt, ra, rb` (RR-form sibling of rotqbyi 0x1FC).
    /// Rotate quadword left by (rb_preferred_slot & 0x0F) bytes.
    #[must_use]
    pub const fn rotqby(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x1DC, rt, ra, rb) }
    /// R5.10m — `shlqbyi rt, ra, imm7` — quadword LEFT-shift by `imm7
    /// & 0x1F` BYTES (zero-fill). Pre-R5.10m this packed `0x1FB`,
    /// which was actually SHLQBII per RPCS3; the primary is now
    /// corrected to `0x1FF` (the real SHLQBYI). See R5.10l diagnose
    /// for the full labeling-bug story.
    #[must_use]
    pub const fn shlqbyi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x1FF, rt, ra, imm7) }
    /// R5.10m — `shlqbii rt, ra, imm7` — quadword LEFT-shift by `imm7
    /// & 0x7` BITS (zero-fill). Distinct from SHLQBYI (byte-shift).
    /// New helper at the correct RPCS3 primary `0x1FB`.
    #[must_use]
    pub const fn shlqbii(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x1FB, rt, ra, imm7) }
    /// R5.10m — `rotqmbyi rt, ra, imm7` — quadword RIGHT-shift by
    /// `(0 - imm7) & 0x1F` BYTES (zero-fill from high end). The "M"
    /// stands for "mask" (zero-fill), NOT "rotate"; the mnemonic is
    /// inherited from the SPU ISA spec. Primary `0x1FD` per RPCS3.
    #[must_use]
    pub const fn rotqmbyi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x1FD, rt, ra, imm7) }

    // ---- Iter-4: float-point single-precision (4-lane) --------------

    /// `fa rt, ra, rb` — lane-wise f32 add.
    #[must_use]
    pub const fn fa(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2C4, rt, ra, rb) }
    /// `fs rt, ra, rb` — lane-wise f32 sub.
    #[must_use]
    pub const fn fs(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2C5, rt, ra, rb) }
    /// `fm rt, ra, rb` — lane-wise f32 multiply.
    #[must_use]
    pub const fn fm(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2C6, rt, ra, rb) }

    // ---- RRR-form (iter-5): primary 4 | rt 7 | rb 7 | ra 7 | rc 7 ----
    // R5.11b — corrected from prior (rt and rc swapped) form which made
    // encoder + decoder self-consistent but diverged from real SPU
    // encoding. The actual SPU RRR-form has `rt` (target) at bits 4..10
    // and `rc` (4th source / mask) at bits 25..31. Caught by the
    // single_spu_loadstore_v1 fixture whose captured `shufb $12, $6,
    // $11, $7` (= 0xB182C307) puts $12 in bits 4..10 and $7 in bits
    // 25..31; the prior decoder swapped them, making shufb write to
    // r7 (wiping the cwd output) instead of r12.
    const fn pack_rrr(primary_4: u32, rt: u32, ra: u32, rb: u32, rc: u32) -> u32 {
        ((primary_4 & 0xF) << 28)
            | ((rt & 0x7F) << 21)
            | ((rb & 0x7F) << 14)
            | ((ra & 0x7F) << 7)
            | (rc & 0x7F)
    }

    /// `selb rt, ra, rb, rc` — bit-wise select (rc & rb) | (!rc & ra).
    #[must_use]
    pub const fn selb(rt: u32, ra: u32, rb: u32, rc: u32) -> u32 { pack_rrr(0x8, rt, ra, rb, rc) }

    /// `shufb rt, ra, rb, rc` — byte-wise permutation.
    #[must_use]
    pub const fn shufb(rt: u32, ra: u32, rb: u32, rc: u32) -> u32 { pack_rrr(0xB, rt, ra, rb, rc) }

    /// `fma rt, ra, rb, rc` — fused f32 multiply-add: rt = ra*rb + rc.
    #[must_use]
    pub const fn fma(rt: u32, ra: u32, rb: u32, rc: u32) -> u32 { pack_rrr(0xE, rt, ra, rb, rc) }

    /// `fnms rt, ra, rb, rc` — rt = rc - ra*rb.
    #[must_use]
    pub const fn fnms(rt: u32, ra: u32, rb: u32, rc: u32) -> u32 { pack_rrr(0xD, rt, ra, rb, rc) }

    /// `fms rt, ra, rb, rc` — rt = ra*rb - rc.
    #[must_use]
    pub const fn fms(rt: u32, ra: u32, rb: u32, rc: u32) -> u32 { pack_rrr(0xF, rt, ra, rb, rc) }

    // ---- Iter-6: RR ops (11-bit primary, ra + rt, no rb) ------------

    const fn pack_rr_unary(primary_11: u32, rt: u32, ra: u32) -> u32 {
        ((primary_11 & 0x7FF) << 21) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }

    /// `clz rt, ra` — count leading zeros per word lane.
    #[must_use]
    pub const fn clz(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x2A5, rt, ra) }

    /// `xsbh rt, ra` — sign-extend bytes to halfwords (8 halves per reg).
    #[must_use]
    pub const fn xsbh(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x2B6, rt, ra) }

    /// `xshw rt, ra` — sign-extend halfwords to words.
    #[must_use]
    pub const fn xshw(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x2AE, rt, ra) }

    /// `xswd rt, ra` — sign-extend words to doublewords.
    #[must_use]
    pub const fn xswd(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x2A6, rt, ra) }

    /// `cntb rt, ra` — per-byte popcount.
    #[must_use]
    pub const fn cntb(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x2B4, rt, ra) }

    // ---- Iter-6: RI8 convert ops (10-bit primary + 8-bit scale) ----

    const fn pack_ri8(primary_10: u32, rt: u32, ra: u32, scale: u8) -> u32 {
        ((primary_10 & 0x3FF) << 22)
            | ((scale as u32) << 14)
            | ((ra & 0x7F) << 7)
            | (rt & 0x7F)
    }

    /// `cflts rt, ra, scale` — f32 → signed i32.
    #[must_use]
    pub const fn cflts(rt: u32, ra: u32, scale: u8) -> u32 {
        pack_ri8(0x1D8, rt, ra, scale)
    }
    /// `cfltu rt, ra, scale` — f32 → unsigned u32.
    #[must_use]
    pub const fn cfltu(rt: u32, ra: u32, scale: u8) -> u32 {
        pack_ri8(0x1D9, rt, ra, scale)
    }
    /// `csflt rt, ra, scale` — signed i32 → f32.
    #[must_use]
    pub const fn csflt(rt: u32, ra: u32, scale: u8) -> u32 {
        pack_ri8(0x1DA, rt, ra, scale)
    }
    /// `cuflt rt, ra, scale` — unsigned u32 → f32.
    #[must_use]
    pub const fn cuflt(rt: u32, ra: u32, scale: u8) -> u32 {
        pack_ri8(0x1DB, rt, ra, scale)
    }

    // ---- Iter-7: channel ops (rdch/wrch/rchcnt, 11-bit primary) ----

    const fn pack_channel(primary_11: u32, rt: u32, channel: u32) -> u32 {
        // rt at bits 25..=31, channel at bits 18..=24 (in `ra` slot).
        ((primary_11 & 0x7FF) << 21) | ((channel & 0x7F) << 7) | (rt & 0x7F)
    }
    /// `rdch rt, ch` — read channel into rt.
    #[must_use]
    pub const fn rdch(rt: u32, channel: u32) -> u32 {
        pack_channel(0x00D, rt, channel)
    }
    /// `wrch ch, rt` — write rt's preferred-slot into channel.
    #[must_use]
    pub const fn wrch(rt: u32, channel: u32) -> u32 {
        pack_channel(0x10D, rt, channel)
    }
    /// `rchcnt rt, ch` — read channel count into rt.
    #[must_use]
    pub const fn rchcnt(rt: u32, channel: u32) -> u32 {
        pack_channel(0x00F, rt, channel)
    }

    /// `ila rt, imm18` — load 18-bit unsigned immediate.
    #[must_use]
    pub const fn ila(rt: u32, imm18: u32) -> u32 {
        ((0x21u32) << 25) | ((imm18 & 0x3FFFF) << 7) | (rt & 0x7F)
    }

    // ---- Iter-8: float compares + frest/frsqest + form-mask ---------

    /// `fcgt rt, ra, rb` — float compare-greater-than (denormal flush).
    #[must_use]
    pub const fn fcgt(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2C2, rt, ra, rb) }
    /// `fcmgt rt, ra, rb` — magnitude compare-greater-than.
    #[must_use]
    pub const fn fcmgt(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2CA, rt, ra, rb) }
    /// `fceq rt, ra, rb` — float compare-equal.
    #[must_use]
    pub const fn fceq(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x3C2, rt, ra, rb) }
    /// `fcmeq rt, ra, rb` — magnitude compare-equal.
    #[must_use]
    pub const fn fcmeq(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x3CA, rt, ra, rb) }
    /// `frest rt, ra` — reciprocal estimate.
    #[must_use]
    pub const fn frest(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x1B8, rt, ra) }
    /// `frsqest rt, ra` — reciprocal-sqrt estimate.
    #[must_use]
    pub const fn frsqest(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x1B9, rt, ra) }
    /// `fsm rt, ra` — form select mask (word).
    #[must_use]
    pub const fn fsm(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x1B4, rt, ra) }
    /// R5.10f — `fsmh rt, ra` — form select mask (halfword).
    #[must_use]
    pub const fn fsmh(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x1B5, rt, ra) }
    /// R5.10f — `fsmb rt, ra` — form select mask (byte).
    #[must_use]
    pub const fn fsmb(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x1B6, rt, ra) }
    /// R5.10f — `fsmbi rt, imm16` — form select mask (byte) from
    /// 16-bit immediate. RI16-form, p9=0x065. Encoding: bits 0..8 =
    /// primary 0x065, bits 9..24 = imm16, bits 25..31 = rt.
    #[must_use]
    pub const fn fsmbi(rt: u32, imm16: u16) -> u32 {
        ((0x065u32) << 23) | ((imm16 as u32) << 7) | (rt & 0x7F)
    }

    // ---- Iter-8: indexed load/store ---------------------------------

    /// `lqx rt, ra, rb` — load qword indexed.
    #[must_use]
    pub const fn lqx(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x1C4, rt, ra, rb) }
    /// `stqx rt, ra, rb` — store qword indexed.
    #[must_use]
    pub const fn stqx(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x144, rt, ra, rb) }

    // ---- Iter-8: indirect branches ----------------------------------

    /// `bi ra` — branch indirect.
    #[must_use]
    pub const fn bi(ra: u32) -> u32 { pack_rr_unary(0x1A8, 0, ra) }
    /// `bisl rt, ra` — branch indirect with link (rt = next-pc broadcast).
    #[must_use]
    pub const fn bisl(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x1A9, rt, ra) }
    /// `iret ra` — interrupt return (treated as bi without modeled IRQs).
    #[must_use]
    pub const fn iret(ra: u32) -> u32 { pack_rr_unary(0x1AA, 0, ra) }
    /// `hbr ra, ro` — branch hint (NOP for interpreter).
    #[must_use]
    pub const fn hbr(ra: u32) -> u32 { pack_rr_unary(0x1AC, 0, ra) }
    /// `biz rt, ra` — indirect branch if rt preferred == 0.
    #[must_use]
    pub const fn biz(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x128, rt, ra) }
    /// `binz rt, ra` — indirect branch if rt preferred != 0.
    #[must_use]
    pub const fn binz(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x129, rt, ra) }
    /// `bihz rt, ra` — indirect branch if rt preferred low-half == 0.
    #[must_use]
    pub const fn bihz(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x12A, rt, ra) }
    /// `bihnz rt, ra` — indirect branch if rt preferred low-half != 0.
    #[must_use]
    pub const fn bihnz(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x12B, rt, ra) }

    // ---- Iter-9: vector word shifts (RR-form, count from rb lane) ----

    /// `shl rt, ra, rb` — logical shift left per word.
    #[must_use]
    pub const fn shl(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x05B, rt, ra, rb) }
    /// `rot rt, ra, rb` — rotate left per word.
    #[must_use]
    pub const fn rot(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x058, rt, ra, rb) }
    /// `rotm rt, ra, rb` — logical shift right per word.
    #[must_use]
    pub const fn rotm(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x059, rt, ra, rb) }
    /// `rotma rt, ra, rb` — arithmetic shift right per word.
    #[must_use]
    pub const fn rotma(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x05A, rt, ra, rb) }

    // ---- Iter-9: word shift immediates (RI7) ----

    /// `roti rt, ra, imm7` — rotate left per word, count = i7 & 0x1F.
    #[must_use]
    pub const fn roti(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x078, rt, ra, imm7) }
    /// `rotmi rt, ra, imm7` — logical shr per word.
    #[must_use]
    pub const fn rotmi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x079, rt, ra, imm7) }
    /// `rotmai rt, ra, imm7` — arith shr per word.
    #[must_use]
    pub const fn rotmai(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x07A, rt, ra, imm7) }

    // ---- Iter-9: bitwise complementaries ----

    /// `nand rt, ra, rb` — `~(a & b)`.
    #[must_use]
    pub const fn nand(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x0C9, rt, ra, rb) }
    /// `eqv rt, ra, rb` — `~(a ^ b)`.
    #[must_use]
    pub const fn eqv(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x249, rt, ra, rb) }
    /// `andc rt, ra, rb` — `a & ~b`.
    #[must_use]
    pub const fn andc(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2C1, rt, ra, rb) }
    /// `orc rt, ra, rb` — `a | ~b`.
    #[must_use]
    pub const fn orc(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2C9, rt, ra, rb) }

    // ---- Iter-9: extended compares ----

    /// `ceqh rt, ra, rb` — eq-compare halfword.
    #[must_use]
    pub const fn ceqh(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x3C8, rt, ra, rb) }
    /// `ceqb rt, ra, rb` — eq-compare byte.
    #[must_use]
    pub const fn ceqb(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x3D0, rt, ra, rb) }
    /// `cgth rt, ra, rb` — signed gt halfword.
    #[must_use]
    pub const fn cgth(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x248, rt, ra, rb) }
    /// `cgtb rt, ra, rb` — signed gt byte.
    #[must_use]
    pub const fn cgtb(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x250, rt, ra, rb) }
    /// `clgth rt, ra, rb` — unsigned gt halfword.
    #[must_use]
    pub const fn clgth(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2C8, rt, ra, rb) }
    /// `clgtb rt, ra, rb` — unsigned gt byte.
    #[must_use]
    pub const fn clgtb(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x2D0, rt, ra, rb) }

    // ---- Iter-9: barriers + STOPD ----

    /// `sync` — instruction-stream barrier (NOP for the interpreter).
    #[must_use]
    pub const fn sync() -> u32 { 0x002 << 21 }
    /// `dsync` — data-stream barrier.
    #[must_use]
    pub const fn dsync() -> u32 { 0x003 << 21 }
    /// `stopd` — privileged stop. Behaves like `stop 0`.
    #[must_use]
    pub const fn stopd() -> u32 { 0x140 << 21 }

    // ---- Iter-10: halfword arith + carry/borrow + or-across ----

    /// `ah rt, ra, rb` — per-halfword add (8 lanes).
    #[must_use]
    pub const fn ah(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x0C8, rt, ra, rb) }
    /// `sfh rt, ra, rb` — per-halfword sub-from: `rt = rb - ra`.
    #[must_use]
    pub const fn sfh(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x048, rt, ra, rb) }
    /// `cg rt, ra, rb` — carry generate per word.
    #[must_use]
    pub const fn cg(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x0C2, rt, ra, rb) }
    /// `bg rt, ra, rb` — borrow generate per word (for `rb-ra`).
    #[must_use]
    pub const fn bg(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x042, rt, ra, rb) }
    /// `orx rt, ra` — OR-across word lanes; result in preferred slot.
    #[must_use]
    pub const fn orx(rt: u32, ra: u32) -> u32 { pack_rr_unary(0x1F0, rt, ra) }

    // ---- Iter-10: branch relative w/ link + branch hints ----

    /// `brsl rt, imm16` — branch relative w/ link.
    #[must_use]
    pub const fn brsl(rt: u32, imm16: i16) -> u32 {
        ((0x066 & 0x1FF) << 23) | ((imm16 as u16 as u32 & 0xFFFF) << 7) | (rt & 0x7F)
    }

    /// `hbra ro, ra` — absolute branch hint (NOP). 7-bit primary 0x08.
    #[must_use]
    pub const fn hbra(ra: u32) -> u32 {
        ((0x08u32) << 25) | ((ra & 0x7F) << 7)
    }
    /// `hbrr ro, imm16` — relative branch hint (NOP). 7-bit primary 0x09.
    #[must_use]
    pub const fn hbrr(imm16: i16) -> u32 {
        ((0x09u32) << 25) | ((imm16 as u16 as u32 & 0xFFFF) << 7)
    }

    // ---- Iter-10: halfword immediate compares (RI10 form) ----

    /// `ceqhi rt, ra, imm10` — per-halfword equality vs broadcast imm.
    #[must_use]
    pub const fn ceqhi(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x7D, rt, ra, imm10) }
    /// `cgthi rt, ra, imm10` — signed gt halfword imm.
    #[must_use]
    pub const fn cgthi(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x4D, rt, ra, imm10) }
    /// `clgthi rt, ra, imm10` — unsigned gt halfword imm.
    #[must_use]
    pub const fn clgthi(rt: u32, ra: u32, imm10: i16) -> u32 { pack_8_i10(0x5D, rt, ra, imm10) }

    // ---- Iter-11: halfword shifts (RR + RI7) ------------------------

    /// `roth rt, ra, rb` — rotate left per halfword.
    #[must_use]
    pub const fn roth(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x05C, rt, ra, rb) }
    /// `rothm rt, ra, rb` — logical shr per halfword.
    #[must_use]
    pub const fn rothm(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x05D, rt, ra, rb) }
    /// `rotmah rt, ra, rb` — arith shr per halfword.
    #[must_use]
    pub const fn rotmah(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x05E, rt, ra, rb) }
    /// `shlh rt, ra, rb` — logical shl per halfword.
    #[must_use]
    pub const fn shlh(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x05F, rt, ra, rb) }
    /// `rothi rt, ra, imm7` — rotate left per halfword.
    #[must_use]
    pub const fn rothi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x07C, rt, ra, imm7) }
    /// `rothmi rt, ra, imm7` — logical shr per halfword.
    #[must_use]
    pub const fn rothmi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x07D, rt, ra, imm7) }
    /// `rotmahi rt, ra, imm7` — arith shr per halfword.
    #[must_use]
    pub const fn rotmahi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x07E, rt, ra, imm7) }
    /// `shlhi rt, ra, imm7` — logical shl per halfword.
    #[must_use]
    pub const fn shlhi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x07F, rt, ra, imm7) }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_env(program: &[u32]) -> SpuThread {
        let mut spu = SpuThread::new(0);
        // Write program instructions at LSA 0 (BE).
        for (i, inst) in program.iter().enumerate() {
            let bytes = inst.to_be_bytes();
            assert!(spu.ls_write((i * 4) as u32, &bytes));
        }
        spu.pc = 0;
        spu
    }

    fn step_ok(spu: &mut SpuThread) -> StepOutcome {
        step(spu).expect("step failed")
    }

    // --- stop / nop -----------------------------------------------

    #[test]
    fn stop_halts_with_code() {
        let mut spu = make_env(&[encode::stop(0x1234)]);
        match step_ok(&mut spu) {
            StepOutcome::Stop(code) => assert_eq!(code, 0x1234),
            other => panic!("expected Stop, got {other:?}"),
        }
    }

    #[test]
    fn nop_just_advances_pc() {
        let mut spu = make_env(&[encode::nop(), encode::stop(7)]);
        assert_eq!(step_ok(&mut spu), StepOutcome::Continue);
        assert_eq!(spu.pc, 4);
        if let StepOutcome::Stop(c) = step_ok(&mut spu) {
            assert_eq!(c, 7);
        } else {
            panic!();
        }
    }

    // --- immediates -----------------------------------------------

    #[test]
    fn il_broadcasts_sign_extended_imm16() {
        let mut spu = make_env(&[encode::il(3, -1)]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], u128::MAX);
    }

    #[test]
    fn il_positive_imm_broadcasts_into_all_lanes() {
        let mut spu = make_env(&[encode::il(5, 0x1234)]);
        step_ok(&mut spu);
        let lanes = split_lanes(spu.gpr[5]);
        assert_eq!(lanes, [0x0000_1234, 0x0000_1234, 0x0000_1234, 0x0000_1234]);
    }

    #[test]
    fn ilh_packs_halfword_into_each_lane() {
        let mut spu = make_env(&[encode::ilh(4, 0xBEEF)]);
        step_ok(&mut spu);
        let lanes = split_lanes(spu.gpr[4]);
        assert_eq!(lanes, [0xBEEF_BEEF; 4]);
    }

    #[test]
    fn ilhu_shifts_imm_into_high_half() {
        let mut spu = make_env(&[encode::ilhu(6, 0xCAFE)]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[6]), [0xCAFE_0000; 4]);
    }

    #[test]
    fn iohl_ors_into_low_half_only() {
        let mut spu = make_env(&[
            encode::ilhu(3, 0x1000),
            encode::iohl(3, 0x00FF),
        ]);
        step_ok(&mut spu);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0x1000_00FF; 4]);
    }

    #[test]
    fn ila_loads_18_bit_unsigned_immediate() {
        let mut spu = make_env(&[encode::ila(7, 0x3_FFFF)]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[7]), [0x0003_FFFF; 4]);
    }

    // --- register ALU ---------------------------------------------

    #[test]
    fn a_adds_four_words_independently() {
        let mut spu = make_env(&[encode::a(3, 4, 5)]);
        spu.gpr[4] = join_lanes([1, 2, 3, 4]);
        spu.gpr[5] = join_lanes([10, 20, 30, 40]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [11, 22, 33, 44]);
    }

    #[test]
    fn sf_is_rb_minus_ra() {
        let mut spu = make_env(&[encode::sf(3, 4, 5)]);
        spu.gpr[4] = join_lanes([1, 2, 3, 4]);
        spu.gpr[5] = join_lanes([100, 100, 100, 100]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [99, 98, 97, 96]);
    }

    #[test]
    fn and_or_xor_nor_match_ref() {
        let mut spu = make_env(&[
            encode::and(0, 1, 2),
            encode::or(3, 1, 2),
            encode::xor(4, 1, 2),
            encode::nor(5, 1, 2),
        ]);
        spu.gpr[1] = 0xF0F0_F0F0_F0F0_F0F0_F0F0_F0F0_F0F0_F0F0;
        spu.gpr[2] = 0xFF00_FF00_FF00_FF00_FF00_FF00_FF00_FF00;
        for _ in 0..4 { step_ok(&mut spu); }
        assert_eq!(spu.gpr[0], 0xF000_F000_F000_F000_F000_F000_F000_F000);
        assert_eq!(spu.gpr[3], 0xFFF0_FFF0_FFF0_FFF0_FFF0_FFF0_FFF0_FFF0);
        assert_eq!(spu.gpr[4], 0x0FF0_0FF0_0FF0_0FF0_0FF0_0FF0_0FF0_0FF0);
        assert_eq!(spu.gpr[5], !spu.gpr[3]);
    }

    // --- branches --------------------------------------------------

    #[test]
    fn br_jumps_relative() {
        // Layout: il r3, 0xAAAA ; br +1 ; il r3, 0xBBBB ; stop
        // Expected: r3 = 0xAAAA (first il), then br skips the middle il
        // and lands on stop — r3 should still be 0xAAAA.
        //
        // br imm16 uses a signed halfword (<<2) relative offset. +1 means
        // skip 4 bytes from current pc → lands on the instruction AFTER
        // the skipped one.
        let mut spu = make_env(&[
            encode::il(3, 0xAAAA_u16 as i16),
            encode::br(2),
            encode::il(3, 0x1111),
            encode::stop(0),
        ]);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0xAAAAu16 as i16 as i32 as u32);
    }

    // --- loads/stores ----------------------------------------------

    #[test]
    fn stqd_lqd_base_plus_offset() {
        // base = r4 = 0x200; store r1 at 0x200; load it back from 0x210 with
        // offset -1 (quadword units = -16 bytes).
        let mut spu = make_env(&[
            encode::stqd(1, 4, 0),
            encode::lqd(2, 4, 0),
            encode::stop(0),
        ]);
        spu.gpr[1] = 0xDEAD_BEEF_1111_2222_3333_4444_5555_6666;
        spu.gpr[4] = join_lanes([0x200, 0, 0, 0]);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(spu.gpr[2], spu.gpr[1]);
    }

    // --- iter-2: compares ----------------------------------------

    #[test]
    fn ceq_returns_all_ones_per_matching_lane() {
        let mut spu = make_env(&[encode::ceq(3, 4, 5)]);
        spu.gpr[4] = join_lanes([1, 2, 3, 4]);
        spu.gpr[5] = join_lanes([1, 0, 3, 0]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0xFFFF_FFFF, 0, 0xFFFF_FFFF, 0]);
    }

    #[test]
    fn cgt_is_signed() {
        let mut spu = make_env(&[encode::cgt(3, 4, 5)]);
        spu.gpr[4] = join_lanes([1, u32::MAX, 100, 0]);
        spu.gpr[5] = join_lanes([0, 0, 200, 0]);
        step_ok(&mut spu);
        // Lane 1 signed = -1, which is < 0. So only lane 0 wins.
        assert_eq!(split_lanes(spu.gpr[3]), [0xFFFF_FFFF, 0, 0, 0]);
    }

    #[test]
    fn clgt_is_unsigned() {
        let mut spu = make_env(&[encode::clgt(3, 4, 5)]);
        spu.gpr[4] = join_lanes([1, u32::MAX, 100, 0]);
        spu.gpr[5] = join_lanes([0, 0, 200, 0]);
        step_ok(&mut spu);
        // Unsigned: 0xFFFF_FFFF > 0 is true.
        assert_eq!(split_lanes(spu.gpr[3]), [0xFFFF_FFFF, 0xFFFF_FFFF, 0, 0]);
    }

    #[test]
    fn ceqi_matches_sign_extended_immediate() {
        let mut spu = make_env(&[encode::ceqi(3, 4, -1)]);
        spu.gpr[4] = join_lanes([u32::MAX, 0, u32::MAX, 1]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0xFFFF_FFFF, 0, 0xFFFF_FFFF, 0]);
    }

    #[test]
    fn cgti_compares_signed() {
        let mut spu = make_env(&[encode::cgti(3, 4, 5)]);
        spu.gpr[4] = join_lanes([10, 5, 6, 0]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0xFFFF_FFFF, 0, 0xFFFF_FFFF, 0]);
    }

    // --- R5.10k: Class-A wider RI10 subfamily --------------------

    /// Regression-lock against the EXACT v4 instruction the R5.10i
    /// diagnostic surfaced at pc=0x6F0: `clgti r32, r3, 31` =
    /// `0x5C07C1A0`. Verifies BOTH the encoder bit pattern AND the
    /// runtime semantics: per-word UNSIGNED compare against 31.
    #[test]
    fn clgti_regression_v4_0x5C07C1A0() {
        assert_eq!(encode::clgti(32, 3, 31), 0x5C07C1A0);

        // Mix signed-negative-as-unsigned-large (0xFFFFFFFE = ~4 billion
        // > 31 unsigned), small (5 < 31), boundary (31 not >, 32 >).
        let mut spu = make_env(&[encode::clgti(32, 3, 31)]);
        spu.gpr[3] = join_lanes([0xFFFF_FFFE, 5, 31, 32]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[32]),
            [0xFFFF_FFFF, 0, 0, 0xFFFF_FFFF],
            "clgti must compare UNSIGNED — negative ints (large unsigned) > 31",
        );
    }

    /// Distinguishes CLGTI (unsigned) from CGTI (signed): a negative
    /// value is < 0 signed but > 31 unsigned. Anti-regression for any
    /// future copy-paste between the two arms.
    #[test]
    fn clgti_distinct_from_cgti_for_negative_values() {
        // Same input as cgti_compares_signed but result is different
        // because of unsigned semantics:
        //   ra lanes = [10, 5, 6, 0xFFFF_FFFA] (= [10, 5, 6, -6 signed])
        //   imm = 5
        //   CGTI signed: [10>5, 5>5, 6>5, -6>5]   = [T, F, T, F]
        //   CLGTI unsigned: [10>5, 5>5, 6>5, big>5] = [T, F, T, T]
        let mut spu = make_env(&[encode::clgti(3, 4, 5)]);
        spu.gpr[4] = join_lanes([10, 5, 6, 0xFFFF_FFFA]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [0xFFFF_FFFF, 0, 0xFFFF_FFFF, 0xFFFF_FFFF],
        );
    }

    /// SFI: imm minus ra (NOT ra minus imm). Boundary cases include
    /// signed wrap (imm=10, ra=0xFFFFFFFE → 10 - (-2) = 12 if
    /// interpreted signed; 10u32.wrapping_sub(0xFFFFFFFE) = 12 unsigned
    /// wrap matches), and `imm=0, ra=1 → -1 = 0xFFFFFFFF`.
    #[test]
    fn sfi_subtracts_ra_from_imm_with_wrapping() {
        let mut spu = make_env(&[encode::sfi(3, 4, 10)]);
        spu.gpr[4] = join_lanes([3, 0xFFFF_FFFE, 1, 0]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            // 10-3=7; 10u32.wrapping_sub(0xFFFFFFFE) = 12; 10-1=9; 10-0=10
            [7, 12, 9, 10],
        );
    }

    /// SFI with negative immediate (sign-extended): `sfi rt, ra, -1`
    /// means rt = -1 - ra. For ra=0 → -1 = 0xFFFFFFFF; for ra=5 → -6.
    #[test]
    fn sfi_with_negative_immediate() {
        let mut spu = make_env(&[encode::sfi(3, 4, -1)]);
        spu.gpr[4] = join_lanes([0, 5, 0xFFFF_FFFF, 100]);
        step_ok(&mut spu);
        // -1 - 0 = -1 = 0xFFFFFFFF
        // -1 - 5 = -6 = 0xFFFFFFFA
        // -1 - 0xFFFFFFFF = -1u32.wrapping_sub(0xFFFFFFFF) = 0
        // -1 - 100 = -101 = 0xFFFFFF9B
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [0xFFFF_FFFFu32, 0xFFFF_FFFA, 0, 0xFFFF_FF9B],
        );
    }

    /// AHI: per-halfword add. Tests boundary halfword wrap (0xFFFF +
    /// 1 = 0x0000) and negative immediate sign-extension to i16.
    #[test]
    fn ahi_per_halfword_add_with_wrap() {
        // imm = 1 → every halfword += 1
        let mut spu = make_env(&[encode::ahi(3, 4, 1)]);
        spu.gpr[4] = pack_halves([0, 1, 0xFFFE, 0xFFFF, 100, 200, 0x7FFF, 0x8000]);
        step_ok(&mut spu);
        assert_eq!(
            unpack_halves(spu.gpr[3]),
            [1, 2, 0xFFFF, 0x0000, 101, 201, 0x8000, 0x8001],
            "halfword add must wrap at 0xFFFF→0x0000",
        );
    }

    #[test]
    fn ahi_with_negative_immediate_subtracts() {
        // imm = -3 (sign-extended to i16 = 0xFFFD) — adds 0xFFFD per
        // halfword, which is equivalent to subtract 3 with wrap.
        let mut spu = make_env(&[encode::ahi(3, 4, -3)]);
        spu.gpr[4] = pack_halves([10, 3, 2, 0, 100, 0xFFFF, 0x8000, 0x7FFF]);
        step_ok(&mut spu);
        assert_eq!(
            unpack_halves(spu.gpr[3]),
            [
                7,                                    // 10 - 3
                0,                                    // 3 - 3
                0xFFFFu16,                            // 2 - 3 wraps
                0xFFFDu16,                            // 0 - 3 wraps
                97,                                   // 100 - 3
                0xFFFCu16,                            // 0xFFFF - 3
                0x7FFDu16,                            // 0x8000 - 3
                0x7FFCu16,                            // 0x7FFF - 3
            ],
        );
    }

    /// MPYI: per-word signed 16x16->32 multiply. The HIGH 16 bits of
    /// each ra word must NOT contribute (the C++ uses `_mm_madd_epi16`
    /// against `set1_epi32(si10 & 0xFFFF)` which puts 0 in the high
    /// 16 of each broadcast lane). Sign-extension matters for both
    /// the source low-half and the immediate.
    #[test]
    fn mpyi_signed_low_halfword_multiply() {
        // imm = -3 → multiply each ra-low-i16 by -3.
        // Lanes of ra: 0xDEAD_0005 (low=5),  0xCAFE_FFFE (low=-2 signed),
        //              0x0000_7FFF (low=32767), 0x1234_8000 (low=-32768).
        let mut spu = make_env(&[encode::mpyi(3, 4, -3)]);
        spu.gpr[4] = join_lanes([0xDEAD_0005, 0xCAFE_FFFE, 0x0000_7FFF, 0x1234_8000]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [
                (5_i32 * -3) as u32,           // -15 = 0xFFFF_FFF1
                ((-2_i32) * -3) as u32,        // 6
                (32767_i32 * -3) as u32,       // -98301 = 0xFFFE_8003
                ((-32768_i32) * -3) as u32,    // 98304 = 0x0001_8000
            ],
        );
    }

    /// MPYUI: per-word UNSIGNED 16x16->32 multiply. Negative-as-signed
    /// inputs must be treated as large unsigned u16 values; the
    /// product fills the full 32-bit lane.
    #[test]
    fn mpyui_unsigned_low_halfword_multiply() {
        // imm = 3 → multiply each ra-low-u16 by 3.
        let mut spu = make_env(&[encode::mpyui(3, 4, 3)]);
        spu.gpr[4] = join_lanes([0xDEAD_FFFF, 0xCAFE_8000, 0x0000_0001, 0x1234_5555]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [
                0xFFFF_u32 * 3,        // 0x0002_FFFD
                0x8000_u32 * 3,        // 0x0001_8000
                0x0001_u32 * 3,        // 3
                0x5555_u32 * 3,        // 0x0000_FFFF
            ],
        );
    }

    /// Cross-check: MPYUI vs MPYI on the SAME input must differ when
    /// the low halfword has high bit set (interpreted as negative
    /// signed vs large unsigned). Anti-regression.
    #[test]
    fn mpyui_distinct_from_mpyi_for_high_bit_set() {
        // Low halfword = 0xFFFF: signed -1, unsigned 65535. Multiply
        // by 3:  signed → -3 = 0xFFFFFFFD; unsigned → 0x0002FFFD.
        let mut spu = make_env(&[encode::mpyi(5, 4, 3), encode::mpyui(6, 4, 3)]);
        spu.gpr[4] = join_lanes([0xAAAA_FFFF, 0xAAAA_FFFF, 0xAAAA_FFFF, 0xAAAA_FFFF]);
        step_ok(&mut spu);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[5], join_lanes([0xFFFF_FFFD; 4]),
                   "MPYI signed: -1 * 3 = -3");
        assert_eq!(spu.gpr[6], join_lanes([0x0002_FFFD; 4]),
                   "MPYUI unsigned: 65535 * 3 = 196605");
    }

    // --- iter-2: add-immediate ------------------------------------

    #[test]
    fn ai_adds_signed_imm10_per_lane() {
        let mut spu = make_env(&[encode::ai(3, 4, -5)]);
        spu.gpr[4] = join_lanes([100, 10, 3, 0xFFFF_FFFA]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [95, 5, 0xFFFF_FFFEu32, 0xFFFF_FFF5],
        );
    }

    // --- iter-2: register-test branches ---------------------------

    #[test]
    fn brnz_jumps_when_preferred_slot_nonzero() {
        // Program:
        //   il r5, 0xAAAA       ; non-zero
        //   brnz r5, +2         ; skip the next instruction (+2 halfwords = 8 bytes)
        //   il r3, 0xDEAD       ; should NOT run
        //   stop 0
        let prog = [
            encode::il(5, 0xAAAAu16 as i16),
            encode::brnz(5, 2),
            encode::il(3, 0xDEADu16 as i16),
            encode::stop(0),
        ];
        let mut spu = make_env(&prog);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0, "r3 must stay zero");
    }

    #[test]
    fn brnz_falls_through_when_zero() {
        let prog = [
            encode::il(5, 0),
            encode::brnz(5, 2),
            encode::il(3, 0x1234),
            encode::stop(0),
        ];
        let mut spu = make_env(&prog);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0x1234);
    }

    #[test]
    fn brz_jumps_when_preferred_slot_zero() {
        let prog = [
            encode::il(5, 0),
            encode::brz(5, 2),
            encode::il(3, 0x1111),
            encode::stop(0),
        ];
        let mut spu = make_env(&prog);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0, "branch taken, r3 untouched");
    }

    #[test]
    fn brz_falls_through_when_nonzero() {
        let prog = [
            encode::il(5, 1),
            encode::brz(5, 2),
            encode::il(3, 0x4242),
            encode::stop(0),
        ];
        let mut spu = make_env(&prog);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0x4242);
    }

    // --- iter-3: immediate logic ---------------------------------

    #[test]
    fn andi_masks_with_sext_imm() {
        let mut spu = make_env(&[encode::andi(3, 4, 0x0FF)]);
        spu.gpr[4] = join_lanes([0xFFFF_FFFF, 0xDEAD_BEEF, 0, 0x1234_5678]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [0xFF, 0xEF, 0, 0x78],
        );
    }

    #[test]
    fn ori_sets_bits_matching_imm() {
        let mut spu = make_env(&[encode::ori(3, 4, 0xF)]);
        spu.gpr[4] = join_lanes([0xF0, 0, 0xFFFF_FF00, 0]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [0xFF, 0xF, 0xFFFF_FF0F, 0xF],
        );
    }

    #[test]
    fn xori_flips_bits() {
        let mut spu = make_env(&[encode::xori(3, 4, 0x3FF)]);
        spu.gpr[4] = join_lanes([0, 0xFFFF_FFFF, 0x155, 0x2AA]);
        step_ok(&mut spu);
        // imm10 = 0x3FF sign-extended → 0xFFFF_FFFF (since bit 9 is set).
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [0xFFFF_FFFF, 0, !0x155u32, !0x2AAu32],
        );
    }

    // --- iter-3: shifts ------------------------------------------

    #[test]
    fn shli_shifts_each_word_independently() {
        let mut spu = make_env(&[encode::shli(3, 4, 4)]);
        spu.gpr[4] = join_lanes([1, 0x10, 0xFF, 0x8000_0000]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [0x10, 0x100, 0xFF0, 0],
        );
    }

    #[test]
    fn shli_with_sh_ge_32_yields_zero() {
        let mut spu = make_env(&[encode::shli(3, 4, 32)]);
        spu.gpr[4] = join_lanes([1, 2, 3, 4]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], 0);
    }

    #[test]
    fn rotqbyi_rotates_quadword_left_by_bytes() {
        let mut spu = make_env(&[encode::rotqbyi(3, 4, 4)]);
        // Pack bytes 0x00..0x0F into register.
        spu.gpr[4] = u128::from_be_bytes([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        ]);
        step_ok(&mut spu);
        assert_eq!(
            spu.gpr[3].to_be_bytes(),
            [
                0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
                0x0C, 0x0D, 0x0E, 0x0F, 0x00, 0x01, 0x02, 0x03,
            ],
        );
    }

    #[test]
    fn rotqbyi_modulo_16_bytes() {
        let mut spu = make_env(&[encode::rotqbyi(3, 4, 17)]);
        spu.gpr[4] = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
            0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF,
        ]);
        step_ok(&mut spu);
        // rotqbyi takes low 4 bits → 17 & 0x0F = 1, same as rot by 1.
        assert_eq!(spu.gpr[3].to_be_bytes()[0], 0xA1);
    }

    /// R5.11b — RR-form rotqby. Same byte-rotate semantics as
    /// rotqbyi but the shift count comes from the low 4 bits of rb's
    /// preferred slot (top 32 bits of the u128). Surfaced by GCC -O2
    /// codegen for runtime-indexed extraction of a 4-byte slot from
    /// a 16-byte aligned LS load (single_spu_loadstore_v1 fixture).
    #[test]
    fn rotqby_rotates_quadword_left_by_rb_low_nibble() {
        let mut spu = make_env(&[encode::rotqby(3, 4, 5)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        ]);
        // rb's preferred slot = 4 → rotate by 4 bytes (same outcome
        // as rotqbyi rt, ra, 4).
        spu.gpr[5] = (4u128) << 96;
        step_ok(&mut spu);
        assert_eq!(
            spu.gpr[3].to_be_bytes(),
            [
                0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B,
                0x0C, 0x0D, 0x0E, 0x0F, 0x00, 0x01, 0x02, 0x03,
            ],
        );
    }

    /// rotqby ignores high bits of rb's preferred slot — only the
    /// low 4 bits matter. This mirrors rotqbyi's 0x0F mask.
    #[test]
    fn rotqby_modulo_16_bytes_in_rb_preferred_slot() {
        let mut spu = make_env(&[encode::rotqby(3, 4, 5)]);
        spu.gpr[4] = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
            0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF,
        ]);
        // rb preferred slot = 0xFF11 → low 4 bits = 1 → rotate by 1.
        spu.gpr[5] = (0xFF11u128) << 96;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3].to_be_bytes()[0], 0xA1);
    }

    #[test]
    fn shlqbyi_zero_fills_right_tail() {
        let mut spu = make_env(&[encode::shlqbyi(3, 4, 3)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
            0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x00,
        ]);
        step_ok(&mut spu);
        let out = spu.gpr[3].to_be_bytes();
        assert_eq!(&out[..13], &[
            0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB,
            0xCC, 0xDD, 0xEE, 0xFF, 0x00,
        ]);
        assert_eq!(&out[13..], &[0, 0, 0]);
    }

    // --- R5.10m: ROTQMBYI + SHLQBYI/SHLQBII labeling fix ----------

    /// R5.10m regression — exact v4 instance at pc=0x72C.
    /// `rotqmbyi r22, r29, 0x7C` (= -4 signed) produces a right-shift
    /// of `(0 - (-4)) & 0x1F = 4` bytes (zero-fill from the high end).
    #[test]
    fn rotqmbyi_shift_right_by_4_bytes_v4_regression() {
        assert_eq!(encode::rotqmbyi(22, 29, -4), 0x3FBF0E96);

        let mut spu = make_env(&[encode::rotqmbyi(22, 29, -4)]);
        spu.gpr[29] = u128::from_be_bytes([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        ]);
        step_ok(&mut spu);
        // After right-shift by 4 bytes: high 4 bytes = 0; rest = a's
        // bytes 0..11.
        assert_eq!(spu.gpr[22].to_be_bytes(), [
            0x00, 0x00, 0x00, 0x00,
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0A, 0x0B,
        ]);
    }

    /// `imm7 = 0` → effective shift count `(0 - 0) & 0x1F = 0` →
    /// identity (rt = ra unchanged).
    #[test]
    fn rotqmbyi_zero_immediate_is_identity() {
        let mut spu = make_env(&[encode::rotqmbyi(3, 4, 0)]);
        let payload: u128 = 0xCAFE_F00D_DEAD_BEEF_1234_5678_9ABC_DEF0;
        spu.gpr[4] = payload;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], payload);
    }

    /// Positive `imm7 = 4` → shift count `(0 - 4) & 0x1F = 28` → ≥ 16 →
    /// output is all zeros (the entire quadword falls off the high end).
    #[test]
    fn rotqmbyi_positive_immediate_zeroes_when_shift_ge_16() {
        let mut spu = make_env(&[encode::rotqmbyi(3, 4, 4)]);
        spu.gpr[4] = u128::MAX;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], 0);
    }

    /// Boundary: `imm7 = -1` → shift count = 1 → bytes shift right
    /// by exactly 1, with byte 0 = 0 and byte 15 = a's byte 14.
    #[test]
    fn rotqmbyi_minus_one_shifts_right_one_byte() {
        let mut spu = make_env(&[encode::rotqmbyi(3, 4, -1)]);
        spu.gpr[4] = u128::from_be_bytes([
            0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7,
            0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE, 0xAF,
        ]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3].to_be_bytes(), [
            0x00, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6,
            0xA7, 0xA8, 0xA9, 0xAA, 0xAB, 0xAC, 0xAD, 0xAE,
        ]);
    }

    /// R5.10m anti-regression — encode::shlqbyi MUST pack primary
    /// `0x1FF`, NOT `0x1FB`. Pre-R5.10m the helper packed `0x1FB`
    /// (which is SHLQBII per RPCS3) producing a wire-format mismatch.
    #[test]
    fn shlqbyi_uses_primary_0x1ff_not_0x1fb() {
        let inst = encode::shlqbyi(3, 4, 5);
        let p11 = inst >> 21;
        assert_eq!(
            p11, 0x1FF,
            "shlqbyi must encode at primary 0x1FF (SHLQBYI per RPCS3); \
             pre-R5.10m it was incorrectly 0x1FB (which is SHLQBII)",
        );
        assert_ne!(p11, 0x1FB, "0x1FB is SHLQBII, NOT SHLQBYI");
    }

    /// R5.10m — SHLQBII at the corrected primary 0x1FB performs a
    /// 128-bit LEFT-SHIFT BY BITS (not bytes). For `imm7 = 4`, mask
    /// `& 0x7 = 4` → shift by 4 bits. Pre-R5.10m the 0x1FB arm
    /// performed BYTE-shift (was mislabeled "shlqbyi") which is wrong
    /// by 8x for any non-zero count.
    #[test]
    fn shlqbii_bit_shift_left_distinct_from_byte_shift() {
        let mut spu = make_env(&[encode::shlqbii(3, 4, 4)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
        ]);
        step_ok(&mut spu);
        // 4-bit left-shift: 0x80...01 << 4 = 0x80<<4 | low_carry...
        // Actually: u128 left-shift by 4. The MSB byte 0x80 becomes
        // 0x00 (the 1-bit carries out of the u128 entirely), and the
        // rest shifts up. Trace by hand:
        //   input  hex = 0x80000000_00000000_00000000_00000001
        //   << 4       = 0x00000000_00000000_00000000_00000010
        // (the 0x80 bit shifts out, only the low 0x01 bit shifts up
        //  to 0x10 in the same byte).
        assert_eq!(spu.gpr[3].to_be_bytes(), [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10,
        ]);
        // Anti-regression: this MUST NOT be byte-shift-by-4 result
        // (which would have produced 0x00<<4 = 0x00000000_00000001_...
        // with byte 4 nonzero).
        assert_ne!(
            spu.gpr[3],
            // What pre-R5.10m would have produced (byte-shift by
            // `imm7 & 0x1F = 4`):
            u128::from_be_bytes([
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
            ]),
        );
    }

    /// SHLQBII bit-shift `imm7 = 0` → identity.
    #[test]
    fn shlqbii_zero_immediate_is_identity() {
        let mut spu = make_env(&[encode::shlqbii(3, 4, 0)]);
        let payload: u128 = 0xCAFE_F00D_DEAD_BEEF_1234_5678_9ABC_DEF0;
        spu.gpr[4] = payload;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], payload);
    }

    /// Anti-regression: SHLQBYI (byte-shift) at 0x1FF and SHLQBII
    /// (bit-shift) at 0x1FB MUST produce DIFFERENT results for the
    /// same `imm7=2` against the same input. Pre-R5.10m these were
    /// conflated (both byte-shift) at primary 0x1FB.
    #[test]
    fn shlqbyi_distinct_from_shlqbii_for_same_input() {
        let payload = u128::from_be_bytes([
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ]);
        // SHLQBYI imm7=2: byte-shift left by 2 bytes — 14 leading 0xFF
        // bytes + 2 trailing zeros.
        let mut spu_a = make_env(&[encode::shlqbyi(3, 4, 2)]);
        spu_a.gpr[4] = payload;
        step_ok(&mut spu_a);
        let byte_shift_result = spu_a.gpr[3];

        // SHLQBII imm7=2: bit-shift left by 2 bits — every byte
        // becomes 0xFC (= 0xFF & 0xFC after bit-shift) except the LSB
        // which loses its bottom 2 bits.
        let mut spu_b = make_env(&[encode::shlqbii(3, 4, 2)]);
        spu_b.gpr[4] = payload;
        step_ok(&mut spu_b);
        let bit_shift_result = spu_b.gpr[3];

        assert_ne!(byte_shift_result, bit_shift_result);
        assert_eq!(byte_shift_result.to_be_bytes(), [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00,
        ]);
        // bit-shift left by 2: each pair of consecutive bits carries.
        // u128 0xFF...FF << 2 = 0xFF...FC (top 2 bits shift out).
        assert_eq!(bit_shift_result.to_be_bytes(), [
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFC,
        ]);
    }

    // --- iter-3: multiply ----------------------------------------

    #[test]
    fn mpy_low_16_signed_multiply_per_lane() {
        let mut spu = make_env(&[encode::mpy(3, 4, 5)]);
        // Low 16 bits only: a = -3, b = 4 → -12 per lane.
        let a = 0xFFFD_u16 as u32;
        let b = 0x0004u32;
        spu.gpr[4] = join_lanes([a, a, a, a]);
        spu.gpr[5] = join_lanes([b, b, b, b]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [(-12i32) as u32; 4]);
    }

    #[test]
    fn mpyu_low_16_unsigned_multiply() {
        let mut spu = make_env(&[encode::mpyu(3, 4, 5)]);
        spu.gpr[4] = join_lanes([0xFFFF, 0x1234, 0, 0x0010]);
        spu.gpr[5] = join_lanes([0xFFFF, 0x5678, 999, 0x0100]);
        step_ok(&mut spu);
        assert_eq!(
            split_lanes(spu.gpr[3]),
            [
                0xFFFF * 0xFFFF,
                0x1234 * 0x5678,
                0,
                0x0010 * 0x0100,
            ],
        );
    }

    #[test]
    fn mpy_ignores_high_halves() {
        // Upper 16 bits of either operand must be masked out.
        let mut spu = make_env(&[encode::mpy(3, 4, 5)]);
        spu.gpr[4] = join_lanes([0xAAAA_0003, 0, 0, 0]); // low = 3
        spu.gpr[5] = join_lanes([0xBBBB_0004, 0, 0, 0]); // low = 4
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3])[0], 12);
    }

    // --- iter-4: single-precision FP -----------------------------

    fn f32x4(x0: f32, x1: f32, x2: f32, x3: f32) -> u128 {
        join_lanes([x0.to_bits(), x1.to_bits(), x2.to_bits(), x3.to_bits()])
    }

    fn as_f32_lanes(v: u128) -> [f32; 4] {
        let l = split_lanes(v);
        [
            f32::from_bits(l[0]),
            f32::from_bits(l[1]),
            f32::from_bits(l[2]),
            f32::from_bits(l[3]),
        ]
    }

    #[test]
    fn fa_adds_four_floats_lane_wise() {
        let mut spu = make_env(&[encode::fa(3, 4, 5)]);
        spu.gpr[4] = f32x4(1.0, 2.0, 3.0, 4.0);
        spu.gpr[5] = f32x4(0.5, 0.25, 0.125, -1.0);
        step_ok(&mut spu);
        assert_eq!(as_f32_lanes(spu.gpr[3]), [1.5, 2.25, 3.125, 3.0]);
    }

    #[test]
    fn fs_subtracts_lane_wise() {
        let mut spu = make_env(&[encode::fs(3, 4, 5)]);
        spu.gpr[4] = f32x4(10.0, 0.0, 7.5, -1.0);
        spu.gpr[5] = f32x4(4.5, 0.5, 3.0, 2.0);
        step_ok(&mut spu);
        assert_eq!(as_f32_lanes(spu.gpr[3]), [5.5, -0.5, 4.5, -3.0]);
    }

    #[test]
    fn fm_multiplies_lane_wise() {
        let mut spu = make_env(&[encode::fm(3, 4, 5)]);
        spu.gpr[4] = f32x4(2.0, -3.0, 0.5, 1.0);
        spu.gpr[5] = f32x4(4.0, 2.0, 8.0, 0.0);
        step_ok(&mut spu);
        assert_eq!(as_f32_lanes(spu.gpr[3]), [8.0, -6.0, 4.0, 0.0]);
    }

    #[test]
    fn fa_handles_infinity_and_nan() {
        let mut spu = make_env(&[encode::fa(3, 4, 5)]);
        // Lane 0: +inf + 1.0 = +inf. Lane 1: +inf + -inf = NaN.
        // Lane 2: NaN + 1.0 = NaN. Lane 3: 1.0 + 2.0 = 3.0.
        spu.gpr[4] = f32x4(f32::INFINITY, f32::INFINITY, f32::NAN, 1.0);
        spu.gpr[5] = f32x4(1.0, f32::NEG_INFINITY, 1.0, 2.0);
        step_ok(&mut spu);
        let r = as_f32_lanes(spu.gpr[3]);
        assert!(r[0].is_infinite() && r[0] > 0.0);
        assert!(r[1].is_nan(), "inf + -inf = nan");
        assert!(r[2].is_nan());
        assert_eq!(r[3], 3.0);
    }

    #[test]
    fn fp_chain_fma_approximation() {
        // With fa + fm we can emulate FMA for a quick smoke.
        // result = 2.5 * 4.0 + 1.0 = 11.0 per lane.
        let prog = [
            encode::fm(6, 4, 5),    // r6 = r4 * r5
            encode::fa(3, 6, 1),    // r3 = r6 + r1
            encode::stop(0),
        ];
        let mut spu = make_env(&prog);
        spu.gpr[4] = f32x4(2.5, 2.5, 2.5, 2.5);
        spu.gpr[5] = f32x4(4.0, 4.0, 4.0, 4.0);
        spu.gpr[1] = f32x4(1.0, 1.0, 1.0, 1.0);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(as_f32_lanes(spu.gpr[3]), [11.0; 4]);
    }

    // --- iter-5: RRR-form ----------------------------------------

    #[test]
    fn selb_picks_bits_per_mask() {
        let mut spu = make_env(&[encode::selb(3, 4, 5, 6)]);
        spu.gpr[4] = 0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA;
        spu.gpr[5] = 0x5555_5555_5555_5555_5555_5555_5555_5555;
        spu.gpr[6] = 0xFF00_FF00_FF00_FF00_FF00_FF00_FF00_FF00;
        step_ok(&mut spu);
        // Where mask=1 take b (0x55), where mask=0 take a (0xAA).
        assert_eq!(
            spu.gpr[3],
            0x55AA_55AA_55AA_55AA_55AA_55AA_55AA_55AA,
        );
    }

    #[test]
    fn shufb_identity_select_0_1_2_3() {
        // Selector bytes 0..15 map to ra bytes 0..15 — identity permutation.
        let mut spu = make_env(&[encode::shufb(3, 4, 5, 6)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        ]);
        spu.gpr[5] = 0;
        spu.gpr[6] = u128::from_be_bytes([
            0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
        ]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], spu.gpr[4]);
    }

    #[test]
    fn shufb_swaps_halves_between_ra_and_rb() {
        // First 8 bytes from ra, next 8 from rb — selectors 0..7, 16..23.
        let mut spu = make_env(&[encode::shufb(3, 4, 5, 6)]);
        spu.gpr[4] = u128::from_be_bytes([0xAA; 16]);
        spu.gpr[5] = u128::from_be_bytes([0xBB; 16]);
        spu.gpr[6] = u128::from_be_bytes([
            0, 1, 2, 3, 4, 5, 6, 7, 16, 17, 18, 19, 20, 21, 22, 23,
        ]);
        step_ok(&mut spu);
        let out = spu.gpr[3].to_be_bytes();
        assert_eq!(&out[..8], &[0xAA; 8]);
        assert_eq!(&out[8..], &[0xBB; 8]);
    }

    #[test]
    fn shufb_constant_patterns_from_high_bits() {
        // Selector 0x80 → 0x00, 0xC0 → 0xFF, 0xE0 → 0x80.
        let mut spu = make_env(&[encode::shufb(3, 4, 5, 6)]);
        spu.gpr[4] = u128::from_be_bytes([0x99; 16]);
        spu.gpr[5] = u128::from_be_bytes([0x88; 16]);
        let mut sel = [0u8; 16];
        sel[0] = 0x80; sel[1] = 0xC0; sel[2] = 0xE0; sel[3] = 0;
        for i in 4..16 { sel[i] = 0x80; }
        spu.gpr[6] = u128::from_be_bytes(sel);
        step_ok(&mut spu);
        let out = spu.gpr[3].to_be_bytes();
        assert_eq!(out[0], 0x00);
        assert_eq!(out[1], 0xFF);
        assert_eq!(out[2], 0x80);
        assert_eq!(out[3], 0x99); // idx 0 of ra
    }

    #[test]
    fn fma_computes_ra_times_rb_plus_rc() {
        let mut spu = make_env(&[encode::fma(3, 4, 5, 6)]);
        spu.gpr[4] = f32x4(2.5, 1.0, 0.0, -3.0);
        spu.gpr[5] = f32x4(4.0, 2.0, 100.0, 2.0);
        spu.gpr[6] = f32x4(1.0, -0.5, 7.5, 0.0);
        step_ok(&mut spu);
        // 2.5*4+1=11, 1*2-0.5=1.5, 0*100+7.5=7.5, -3*2+0=-6.
        assert_eq!(as_f32_lanes(spu.gpr[3]), [11.0, 1.5, 7.5, -6.0]);
    }

    #[test]
    fn fnms_computes_rc_minus_ra_times_rb() {
        let mut spu = make_env(&[encode::fnms(3, 4, 5, 6)]);
        spu.gpr[4] = f32x4(3.0, 0.0, 2.0, 1.0);
        spu.gpr[5] = f32x4(4.0, 5.0, 5.0, 1.0);
        spu.gpr[6] = f32x4(20.0, 10.0, 15.0, 10.0);
        step_ok(&mut spu);
        // 20-3*4=8, 10-0=10, 15-10=5, 10-1=9.
        assert_eq!(as_f32_lanes(spu.gpr[3]), [8.0, 10.0, 5.0, 9.0]);
    }

    #[test]
    fn fms_computes_ra_times_rb_minus_rc() {
        let mut spu = make_env(&[encode::fms(3, 4, 5, 6)]);
        spu.gpr[4] = f32x4(5.0, 2.0, 0.0, 1.0);
        spu.gpr[5] = f32x4(2.0, 3.0, 100.0, 4.0);
        spu.gpr[6] = f32x4(3.0, 1.0, 0.5, 0.0);
        step_ok(&mut spu);
        // 10-3=7, 6-1=5, 0-0.5=-0.5, 4-0=4.
        assert_eq!(as_f32_lanes(spu.gpr[3]), [7.0, 5.0, -0.5, 4.0]);
    }

    // --- iter-6: clz / sign-ext / cntb ---------------------------

    #[test]
    fn clz_counts_leading_zeros_per_word() {
        let mut spu = make_env(&[encode::clz(3, 4)]);
        spu.gpr[4] = join_lanes([0, 1, 0x8000_0000, 0x0000_FFFF]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [32, 31, 0, 16]);
    }

    #[test]
    fn xsbh_sign_extends_low_bytes_to_halfwords() {
        let mut spu = make_env(&[encode::xsbh(3, 4)]);
        spu.gpr[4] = u128::from_be_bytes([
            0xFF, 0x80, 0xFF, 0x7F, 0xFF, 0x00, 0xFF, 0xFF,
            0xFF, 0x40, 0xFF, 0x01, 0xFF, 0xFE, 0xFF, 0xAB,
        ]);
        step_ok(&mut spu);
        let out = spu.gpr[3].to_be_bytes();
        // 0x80 → halfword 0xFF80 (sign-extended from byte -128).
        assert_eq!(&out[..2], &[0xFF, 0x80]);
        // 0x7F → halfword 0x007F (positive).
        assert_eq!(&out[2..4], &[0x00, 0x7F]);
        // 0x00 → halfword 0x0000.
        assert_eq!(&out[4..6], &[0x00, 0x00]);
        // 0xFF → halfword 0xFFFF (sign-extended).
        assert_eq!(&out[6..8], &[0xFF, 0xFF]);
        // 0xAB → halfword 0xFFAB.
        assert_eq!(&out[14..16], &[0xFF, 0xAB]);
    }

    #[test]
    fn xshw_sign_extends_halfwords_to_words() {
        let mut spu = make_env(&[encode::xshw(3, 4)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x11, 0x22, 0x80, 0x00,
            0x33, 0x44, 0x7F, 0xFF,
            0x55, 0x66, 0x00, 0x01,
            0x77, 0x88, 0xFF, 0x00,
        ]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0xFFFF_8000);  // -0x8000 sign-ext
        assert_eq!(r[1], 0x0000_7FFF);  // positive
        assert_eq!(r[2], 0x0000_0001);
        assert_eq!(r[3], 0xFFFF_FF00);  // -0x0100 sign-ext
    }

    #[test]
    fn xswd_sign_extends_words_to_doublewords() {
        let mut spu = make_env(&[encode::xswd(3, 4)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x11, 0x22, 0x33, 0x44,  0xFF, 0xFF, 0xFF, 0xFF,
            0x55, 0x66, 0x77, 0x88,  0x00, 0x00, 0x00, 0x01,
        ]);
        step_ok(&mut spu);
        let out = spu.gpr[3].to_be_bytes();
        // First doubleword: sign-extend 0xFFFFFFFF → 0xFFFFFFFF_FFFFFFFF
        assert_eq!(&out[..8], &[0xFF; 8]);
        // Second doubleword: sign-extend 0x00000001 → 0x00000000_00000001
        assert_eq!(&out[8..], &[0, 0, 0, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn cntb_counts_bits_per_byte() {
        let mut spu = make_env(&[encode::cntb(3, 4)]);
        // 0xFF = 8 bits, 0x00 = 0, 0x0F = 4, 0x01 = 1.
        let mut input = [0u8; 16];
        for i in 0..16 {
            input[i] = match i % 4 {
                0 => 0xFF, 1 => 0x00, 2 => 0x0F, _ => 0x01,
            };
        }
        spu.gpr[4] = u128::from_be_bytes(input);
        step_ok(&mut spu);
        let out = spu.gpr[3].to_be_bytes();
        for i in 0..16 {
            let expected = match i % 4 {
                0 => 8, 1 => 0, 2 => 4, _ => 1,
            };
            assert_eq!(out[i], expected, "byte {}", i);
        }
    }

    // --- iter-6: convert ops -------------------------------------

    #[test]
    fn csflt_converts_signed_int_to_float_scale_155() {
        let mut spu = make_env(&[encode::csflt(3, 4, 155)]);
        spu.gpr[4] = join_lanes([0, 1, (-1i32) as u32, 1000]);
        step_ok(&mut spu);
        assert_eq!(as_f32_lanes(spu.gpr[3]), [0.0, 1.0, -1.0, 1000.0]);
    }

    #[test]
    fn cuflt_converts_unsigned_int_to_float_scale_155() {
        let mut spu = make_env(&[encode::cuflt(3, 4, 155)]);
        spu.gpr[4] = join_lanes([0, 1, 0x8000_0000, 100]);
        step_ok(&mut spu);
        let r = as_f32_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0.0);
        assert_eq!(r[1], 1.0);
        assert_eq!(r[2], 0x8000_0000u32 as f32);  // unsigned, not -2^31
        assert_eq!(r[3], 100.0);
    }

    #[test]
    fn cflts_converts_float_to_signed_int_scale_173() {
        let mut spu = make_env(&[encode::cflts(3, 4, 173)]);
        spu.gpr[4] = f32x4(0.0, 1.5, -2.5, 100.9);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0, 1, (-2i32) as u32, 100]);
    }

    #[test]
    fn cflts_saturates_on_overflow() {
        let mut spu = make_env(&[encode::cflts(3, 4, 173)]);
        spu.gpr[4] = f32x4(f32::INFINITY, f32::NEG_INFINITY, 1e30, f32::NAN);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], i32::MAX as u32);
        assert_eq!(r[1], i32::MIN as u32);
        assert_eq!(r[2], i32::MAX as u32);
        assert_eq!(r[3], 0); // NaN
    }

    #[test]
    fn cfltu_clamps_negative_to_zero() {
        let mut spu = make_env(&[encode::cfltu(3, 4, 173)]);
        spu.gpr[4] = f32x4(-5.0, 1.5, 1e30, 100.0);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0);
        assert_eq!(r[1], 1);
        assert_eq!(r[2], u32::MAX);
        assert_eq!(r[3], 100);
    }

    #[test]
    fn csflt_cflts_round_trip_for_small_ints() {
        let prog = [
            encode::csflt(5, 4, 155),  // i32 → f32
            encode::cflts(3, 5, 173),  // f32 → i32
            encode::stop(0),
        ];
        let mut spu = make_env(&prog);
        spu.gpr[4] = join_lanes([42, 100, (-7i32) as u32, 12345]);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3]), [42, 100, (-7i32) as u32, 12345]);
    }

    // --- iter-7: channel ops (rdch/wrch/rchcnt) ------------------

    use rpcs3_spu_thread::ch;

    #[test]
    fn wrch_outmbox_then_ppu_pops_it() {
        // Program: il r4, 0x1234 ; wrch SPU_WrOutMbox, r4 ; stop 0.
        let prog = [
            encode::il(4, 0x1234),
            encode::wrch(4, ch::SPU_WROUTMBOX),
            encode::stop(0),
        ];
        let mut spu = make_env(&prog);
        run_n(&mut spu, 10).unwrap();
        // PPU side pops the value.
        assert_eq!(spu.channels.ppu_pop_outmbox(), Some(0x1234));
    }

    #[test]
    fn rdch_inmbox_reads_ppu_pushed_value() {
        let mut spu = make_env(&[encode::rdch(3, ch::SPU_RDINMBOX), encode::stop(0)]);
        spu.channels.ppu_push_inmbox(0xDEAD_BEEF);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0xDEAD_BEEF);
    }

    #[test]
    fn rdch_empty_inmbox_stalls() {
        let mut spu = make_env(&[encode::rdch(3, ch::SPU_RDINMBOX)]);
        let outcome = step(&mut spu).unwrap();
        assert_eq!(outcome, StepOutcome::ChannelStall { channel: ch::SPU_RDINMBOX, is_write: false });
        // PC must NOT advance — the instruction will retry.
        assert_eq!(spu.pc, 0);
    }

    #[test]
    fn wrch_full_outmbox_stalls() {
        let mut spu = make_env(&[encode::wrch(4, ch::SPU_WROUTMBOX)]);
        // Pre-fill the mailbox.
        spu.channels.out_mbox = Some(0x99);
        spu.gpr[4] = join_lanes([0x12, 0, 0, 0]);
        let outcome = step(&mut spu).unwrap();
        assert_eq!(outcome, StepOutcome::ChannelStall { channel: ch::SPU_WROUTMBOX, is_write: true });
        assert_eq!(spu.pc, 0);
    }

    #[test]
    fn rchcnt_outmbox_is_1_when_empty_0_when_full() {
        let prog = [encode::rchcnt(3, ch::SPU_WROUTMBOX)];
        let mut spu = make_env(&prog);
        // Empty: 1 slot free.
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3])[0], 1);
        // Reset PC and refill to test the full case.
        spu.pc = 0;
        spu.channels.out_mbox = Some(0);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3])[0], 0);
    }

    #[test]
    fn rchcnt_inmbox_reflects_presence() {
        let mut spu = make_env(&[encode::rchcnt(3, ch::SPU_RDINMBOX)]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3])[0], 0, "empty → count 0");

        spu.pc = 0;
        spu.channels.in_mbox = Some(0x42);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3])[0], 1);
    }

    #[test]
    fn rdch_signal1_clears_after_read() {
        // R5.11: SNR reads now match Cell BE semantics (block when
        // count == 0), so a second consecutive rdch on a drained SNR
        // would park instead of returning 0. Test now: signal once,
        // read once, assert value + that the channel was cleared
        // (both the read result and the channels.snr[0] state).
        let mut spu = make_env(&[
            encode::rdch(3, ch::SPU_RDSIGNOTIFY1),
            encode::stop(0),
        ]);
        spu.channels.signal(0, 0xA5A5);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0xA5A5);
        assert_eq!(spu.channels.snr[0], 0, "snr[0] cleared after read");
    }

    #[test]
    fn event_mask_write_read_round_trip() {
        let mut spu = make_env(&[
            encode::il(5, 0x0003_u16 as i16),
            encode::wrch(5, ch::SPU_WREVENTMASK),
            encode::rdch(3, ch::SPU_RDEVENTMASK),
            encode::stop(0),
        ]);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0x0003);
    }

    #[test]
    fn event_ack_clears_stat_bits() {
        let mut spu = make_env(&[
            encode::il(5, 0x0001_u16 as i16),
            encode::wrch(5, ch::SPU_WREVENTACK),
            encode::stop(0),
        ]);
        spu.channels.event_stat = 0x0003;
        run_n(&mut spu, 10).unwrap();
        assert_eq!(spu.channels.event_stat, 0x0002);
    }

    #[test]
    fn decrementer_round_trip() {
        let mut spu = make_env(&[
            encode::il(5, 0x1234_u16 as i16),
            encode::wrch(5, ch::SPU_WRDEC),
            encode::rdch(3, ch::SPU_RDDEC),
            encode::stop(0),
        ]);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0x1234);
    }

    // --- unimplemented --------------------------------------------

    #[test]
    fn unknown_opcode_returns_unimplemented() {
        // 11-bit primary 0x008 is unused and 4-bit primary 0x0 isn't
        // RRR-form — encoded as 0x01 << 24 = 0x01000000.
        let mut spu = make_env(&[0x0100_0000]);
        match step(&mut spu).unwrap_err() {
            Error::Unimplemented { .. } => {}
            other => panic!("expected Unimplemented, got {other:?}"),
        }
    }

    // --- Iter-8: float compares ---------------------------------

    fn set_lane(spu: &mut SpuThread, gpr: usize, lanes: [u32; 4]) {
        spu.gpr[gpr] = join_lanes(lanes);
    }

    #[test]
    fn fcgt_strict_greater_than_per_lane() {
        // a = [2.0, 1.0, 0.0, -1.0]
        // b = [1.0, 1.0, 0.0, -2.0]
        // expect: [F, 0, 0, F] (lane 0: 2>1 yes; lane 1: 1>1 no; lane 2: 0>0 no; lane 3: -1>-2 yes)
        let mut spu = make_env(&[encode::fcgt(3, 4, 5)]);
        set_lane(&mut spu, 4, [2.0_f32.to_bits(), 1.0_f32.to_bits(), 0.0_f32.to_bits(), (-1.0_f32).to_bits()]);
        set_lane(&mut spu, 5, [1.0_f32.to_bits(), 1.0_f32.to_bits(), 0.0_f32.to_bits(), (-2.0_f32).to_bits()]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0xFFFF_FFFF, 0, 0, 0xFFFF_FFFF]);
    }

    #[test]
    fn fcgt_flushes_denormals() {
        // a tiny denormal vs +0 — both flush to +0, compare is false.
        let mut spu = make_env(&[encode::fcgt(3, 4, 5)]);
        set_lane(&mut spu, 4, [1, 0, 0, 0]); // denormal in lane 0
        set_lane(&mut spu, 5, [0, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3])[0], 0);
    }

    #[test]
    fn fcmgt_compares_magnitudes() {
        // |-3.0| > |2.0| → true; |1.0| > |-1.0| → false (equal mag)
        let mut spu = make_env(&[encode::fcmgt(3, 4, 5)]);
        set_lane(&mut spu, 4, [(-3.0_f32).to_bits(), 1.0_f32.to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()]);
        set_lane(&mut spu, 5, [2.0_f32.to_bits(), (-1.0_f32).to_bits(), 0.0_f32.to_bits(), 0.0_f32.to_bits()]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0xFFFF_FFFF);
        assert_eq!(r[1], 0);
    }

    #[test]
    fn fceq_equal_per_lane() {
        let mut spu = make_env(&[encode::fceq(3, 4, 5)]);
        set_lane(&mut spu, 4, [1.5_f32.to_bits(), 2.0_f32.to_bits(), 0.0_f32.to_bits(), f32::NAN.to_bits()]);
        set_lane(&mut spu, 5, [1.5_f32.to_bits(), 2.0_f32.to_bits(), 0.0_f32.to_bits(), f32::NAN.to_bits()]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0xFFFF_FFFF);
        assert_eq!(r[1], 0xFFFF_FFFF);
        assert_eq!(r[2], 0xFFFF_FFFF);
        // NaN never compares equal even to itself.
        assert_eq!(r[3], 0);
    }

    #[test]
    fn fcmeq_magnitude_equality() {
        let mut spu = make_env(&[encode::fcmeq(3, 4, 5)]);
        set_lane(&mut spu, 4, [(-1.5_f32).to_bits(), 1.5_f32.to_bits(), 2.0_f32.to_bits(), 0.0_f32.to_bits()]);
        set_lane(&mut spu, 5, [1.5_f32.to_bits(), (-1.5_f32).to_bits(), 3.0_f32.to_bits(), 0.0_f32.to_bits()]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0xFFFF_FFFF);
        assert_eq!(r[1], 0xFFFF_FFFF);
        assert_eq!(r[2], 0);
        assert_eq!(r[3], 0xFFFF_FFFF);
    }

    #[test]
    fn fm_flushes_denormal_inputs_and_results() {
        let mut spu = make_env(&[encode::fm(3, 4, 5)]);
        // 2.0 * 3.0 = 6.0 (normal path)
        set_lane(&mut spu, 4, [2.0_f32.to_bits(), 1.0_f32.to_bits(), 0.0_f32.to_bits(), 1, /* denorm */]);
        set_lane(&mut spu, 5, [3.0_f32.to_bits(), 0.0_f32.to_bits(), 5.0_f32.to_bits(), 1.0_f32.to_bits()]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 6.0_f32.to_bits());
        assert_eq!(r[1], 0.0_f32.to_bits());
        assert_eq!(r[2], 0.0_f32.to_bits());
        // denormal input flushed to +0 → 0 * 1 = 0.
        assert_eq!(r[3], 0);
    }

    #[test]
    fn frest_naive_one_over_two_is_half() {
        let mut spu = make_env(&[encode::frest(3, 4)]);
        set_lane(&mut spu, 4, [2.0_f32.to_bits(), 4.0_f32.to_bits(), 1.0_f32.to_bits(), 0.0_f32.to_bits()]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0.5_f32.to_bits());
        assert_eq!(r[1], 0.25_f32.to_bits());
        assert_eq!(r[2], 1.0_f32.to_bits());
        assert_eq!(r[3], 0x7F80_0000); // +inf for 1/0
    }

    #[test]
    fn frsqest_naive_one_over_sqrt_four_is_half() {
        let mut spu = make_env(&[encode::frsqest(3, 4)]);
        set_lane(&mut spu, 4, [4.0_f32.to_bits(), 16.0_f32.to_bits(), 1.0_f32.to_bits(), 0.0_f32.to_bits()]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0.5_f32.to_bits());
        assert_eq!(r[1], 0.25_f32.to_bits());
        assert_eq!(r[2], 1.0_f32.to_bits());
        assert_eq!(r[3], 0x7F80_0000); // 1/sqrt(0) = +inf
    }

    // --- Iter-8: form select mask --------------------------------

    #[test]
    fn fsm_bit_pattern_expands_per_lane() {
        // ra preferred = 0b1010 → lane 0 (bit 3 = 1) on, lane 1 (bit 2 = 0) off,
        // lane 2 (bit 1 = 1) on, lane 3 (bit 0 = 0) off.
        let mut spu = make_env(&[encode::fsm(3, 4)]);
        set_lane(&mut spu, 4, [0b1010, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0xFFFF_FFFF, 0, 0xFFFF_FFFF, 0]);
    }

    // --- R5.10f: rest of FSM family (FSMH / FSMB / FSMBI) -------------

    #[test]
    fn fsmh_expands_8_bits_to_8_halfwords() {
        // ra preferred low 8 = 0b10100110:
        //   bit 7 (=1) → halfword 0 = 0xFFFF
        //   bit 6 (=0) → halfword 1 = 0x0000
        //   bit 5 (=1) → halfword 2 = 0xFFFF
        //   bit 4 (=0) → halfword 3 = 0x0000
        //   bit 3 (=0) → halfword 4 = 0x0000
        //   bit 2 (=1) → halfword 5 = 0xFFFF
        //   bit 1 (=1) → halfword 6 = 0xFFFF
        //   bit 0 (=0) → halfword 7 = 0x0000
        let mut spu = make_env(&[encode::fsmh(3, 4)]);
        set_lane(&mut spu, 4, [0b1010_0110, 0, 0, 0]);
        step_ok(&mut spu);
        let expected: [u8; 16] = [
            0xFF, 0xFF, 0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00,
            0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00,
        ];
        assert_eq!(spu.gpr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn fsmh_all_zero_yields_all_zero_mask() {
        let mut spu = make_env(&[encode::fsmh(3, 4)]);
        set_lane(&mut spu, 4, [0, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], 0);
    }

    #[test]
    fn fsmh_all_ones_yields_all_ones_mask() {
        // Low 8 bits = 0xFF → all 8 halfwords = 0xFFFF.
        let mut spu = make_env(&[encode::fsmh(3, 4)]);
        set_lane(&mut spu, 4, [0xFF, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], u128::MAX);
    }

    #[test]
    fn fsmb_expands_16_bits_to_16_bytes() {
        // ra preferred low 16 = 0x8001 = 0b1000_0000_0000_0001:
        //   bit 15 → SPU byte 0 = 0xFF
        //   bit 0  → SPU byte 15 = 0xFF
        //   all others = 0x00
        let mut spu = make_env(&[encode::fsmb(3, 4)]);
        set_lane(&mut spu, 4, [0x8001, 0, 0, 0]);
        step_ok(&mut spu);
        let expected: [u8; 16] = [
            0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF,
        ];
        assert_eq!(spu.gpr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn fsmb_ignores_high_bits_of_ra() {
        // Bits 16+ of ra preferred slot must NOT affect the mask;
        // FSMB only reads the low 16. We set high bits to non-zero
        // and expect the same all-zero result.
        let mut spu = make_env(&[encode::fsmb(3, 4)]);
        set_lane(&mut spu, 4, [0xFFFF_0000, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], 0);
    }

    #[test]
    fn fsmbi_regression_v4_0x32880003_at_pc_864() {
        // R5.10f regression — exact value the v4 diagnostic at pc=0x864
        // hit before this iteration. Encoding: fsmbi r3, 0x1000 →
        //   inst hex = 0x32880003 (decimal 847_773_699).
        // Expected mask: only bit 12 of imm16 set → only SPU byte 3
        // becomes 0xFF; the other 15 bytes stay 0x00.
        assert_eq!(encode::fsmbi(3, 0x1000), 0x32880003);
        let mut spu = make_env(&[encode::fsmbi(3, 0x1000)]);
        step_ok(&mut spu);
        let expected: [u8; 16] = [
            0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(spu.gpr[3], u128::from_be_bytes(expected));
    }

    #[test]
    fn fsmbi_all_zero_yields_all_zero_mask() {
        let mut spu = make_env(&[encode::fsmbi(7, 0x0000)]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[7], 0);
    }

    #[test]
    fn fsmbi_all_ones_yields_all_ones_mask() {
        let mut spu = make_env(&[encode::fsmbi(9, 0xFFFF)]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[9], u128::MAX);
    }

    #[test]
    fn fsmbi_v4_imm_0x0202_pattern() {
        // Another i16 pattern observed in v4 (.spuimg first FSMBI
        // sites). 0x0202 = bits 9 and 1 set →
        //   bit 9 → SPU byte 6 = 0xFF
        //   bit 1 → SPU byte 14 = 0xFF
        let mut spu = make_env(&[encode::fsmbi(5, 0x0202)]);
        step_ok(&mut spu);
        let mut expected = [0u8; 16];
        expected[6]  = 0xFF;
        expected[14] = 0xFF;
        assert_eq!(spu.gpr[5], u128::from_be_bytes(expected));
    }

    // --- Iter-8: indexed load/store ------------------------------

    #[test]
    fn lqx_loads_quadword_at_ra_plus_rb() {
        // Pre-store something at LSA 0x40, then load via lqx.
        let payload = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210u128;
        let mut spu = make_env(&[encode::lqx(3, 4, 5), encode::stop(0)]);
        write_qword_be(&mut spu, 0x40, payload).unwrap();
        set_lane(&mut spu, 4, [0x30, 0, 0, 0]);
        set_lane(&mut spu, 5, [0x10, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], payload);
    }

    #[test]
    fn stqx_stores_quadword_at_ra_plus_rb() {
        let mut spu = make_env(&[encode::stqx(3, 4, 5), encode::stop(0)]);
        spu.gpr[3] = 0xDEAD_BEEF_CAFE_F00D_1234_5678_9ABC_DEF0u128;
        set_lane(&mut spu, 4, [0x80, 0, 0, 0]);
        set_lane(&mut spu, 5, [0x20, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(read_qword_be(&spu, 0xA0).unwrap(), spu.gpr[3]);
    }

    // --- R5.10b: lqr (load qword PC-relative) ----------------------

    /// Place an `lqr r7, +0x10` at pc=0x100. The PC-relative target
    /// is `(0x100 + (0x10 << 2)) & 0x3FFF0 = 0x140`. Pre-load a known
    /// payload at 0x140; step; assert gpr[7] receives the payload AND
    /// pc advances by 4 (lqr is not a branch).
    #[test]
    fn lqr_loads_quadword_from_pc_relative_target() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::lqr(7, 0x10);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));

        // Payload at the resolved target.
        let payload: u128 = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210;
        assert!(spu.ls_write(0x140, &payload.to_be_bytes()));

        spu.pc = pc;
        let outcome = step(&mut spu).expect("lqr must succeed");
        assert!(matches!(outcome, StepOutcome::Continue));
        assert_eq!(spu.gpr[7], payload, "gpr[rt] receives the loaded qword");
        assert_eq!(spu.pc, pc + 4, "pc advances by 4 (lqr is not a branch)");
    }

    /// Wrap-around: `lqr r3, -16` from pc=0x10 produces target
    /// `(0x10 + (-16 << 2)) & 0x3FFF0 = (0x10 - 0x40) & 0x3FFF0`
    /// `= 0xFFFFFFD0 & 0x3FFF0 = 0x3FFD0` (top of LS, 16-byte aligned).
    /// Verifies the LS-mask + 16-byte align contract matches RPCS3
    /// C++ `spu_ls_target`. The same instruction MUST also work when
    /// the target falls cleanly in the LS without wrapping.
    #[test]
    fn lqr_wraps_to_ls_bounds() {
        let mut spu = SpuThread::new(0);
        let pc = 0x10u32;
        let imm16: i16 = -16; // -16 halfwords = -64 bytes
        let target = 0x3FFD0u32; // (0x10 - 0x40) wrapped = 0x3FFD0

        let inst = encode::lqr(3, imm16);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));

        // Place a distinct payload at the wrapped target.
        let payload: u128 = 0xCAFE_F00D_DEAD_BEEF_1234_5678_9ABC_DEF0;
        assert!(spu.ls_write(target, &payload.to_be_bytes()));

        spu.pc = pc;
        step(&mut spu).expect("wrapped lqr must succeed");
        assert_eq!(
            spu.gpr[3], payload,
            "lqr should load from wrapped target 0x{target:X}",
        );
        assert_eq!(spu.pc, pc + 4);
    }

    /// 16-byte alignment: an `lqr` whose pure arithmetic target has
    /// bottom-4 bits set MUST land on the 16-byte-floored address.
    /// Constructed by encoding `lqr r5, +0x11` at pc=0x100:
    /// `(0x100 + (0x11 << 2)) & 0x3FFF0 = (0x100 + 0x44) & 0x3FFF0`
    /// `= 0x144 & 0x3FFF0 = 0x140` — bottom 4 bits cleared.
    #[test]
    fn lqr_aligns_target_to_16_bytes() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::lqr(5, 0x11);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));

        // Two distinct payloads: one at the aligned address (which the
        // load MUST hit), one at the would-be-unaligned address (which
        // the load MUST NOT hit). If the alignment is wrong, the test
        // catches it because the loaded qword would be the unaligned
        // payload.
        let aligned_payload: u128 = 0x1111_2222_3333_4444_5555_6666_7777_8888;
        let stray_payload: u128 = 0xAAAA_BBBB_CCCC_DDDD_EEEE_FFFF_0000_1111;
        assert!(spu.ls_write(0x140, &aligned_payload.to_be_bytes()));
        // "Unaligned" payload at 0x144 — overlaps the aligned region's
        // last 12 bytes; we just need the high bytes to differ so the
        // mismatch is detectable. A read from 0x144 (unaligned) would
        // produce the bytes starting at 0x144 (= last 12 of aligned +
        // first 4 of stray); a read from 0x140 (aligned) produces
        // exactly aligned_payload. We compare against aligned_payload.
        assert!(spu.ls_write(0x150, &stray_payload.to_be_bytes()));

        spu.pc = pc;
        step(&mut spu).expect("aligned lqr must succeed");
        assert_eq!(
            spu.gpr[5], aligned_payload,
            "lqr must load from 0x140 (aligned) not 0x144 (unaligned)",
        );
        assert_eq!(spu.pc, pc + 4);
    }

    // --- R5.10g: STQR (Store Quadword PC-Relative) -----------------
    //
    // Direct mirror of LQR (R5.10b): same `spu_ls_target(pc, imm16) =
    // (pc + (imm16<<2)) & 0x3FFF0` address contract; just the
    // direction reverses (LS receives gpr[rt]).

    /// Round-trip: prepare gpr[rt] with a known qword, encode an STQR,
    /// step, and assert LS at the resolved target receives the exact
    /// 16 bytes (and that pc advances by 4 — STQR is not a branch).
    #[test]
    fn stqr_stores_quadword_to_pc_relative_target() {
        let mut spu = make_env(&[encode::stqr(7, 0x10)]);

        // make_env starts pc at 0 → target = (0 + (0x10<<2)) & 0x3FFF0
        // = 0x40 (already 16-byte aligned).
        let payload: u128 = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210;
        spu.gpr[7] = payload;

        let pc_before = spu.pc;
        step_ok(&mut spu);

        assert_eq!(
            read_qword_be(&spu, 0x40).unwrap(), payload,
            "LS at 0x40 must equal the gpr[7] payload",
        );
        assert_eq!(spu.pc, pc_before + 4, "pc advances by 4 (stqr is not a branch)");
    }

    /// Wrap-around: same shape as `lqr_wraps_to_ls_bounds` but for the
    /// store side. From pc=0x10 with imm16=-16, target wraps to the
    /// top of LS at 0x3FFD0. Verifies the LS-mask + 16-byte align
    /// contract matches RPCS3 C++ `spu_ls_target` for negative offsets.
    #[test]
    fn stqr_wraps_to_ls_bounds() {
        let mut spu = SpuThread::new(0);
        let pc = 0x10u32;
        let imm16: i16 = -16;
        let target = 0x3FFD0u32;

        // Hand-place the inst at pc=0x10 (make_env always uses 0x100).
        let inst = encode::stqr(3, imm16);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));

        let payload: u128 = 0xCAFE_F00D_DEAD_BEEF_1234_5678_9ABC_DEF0;
        spu.gpr[3] = payload;

        spu.pc = pc;
        step(&mut spu).expect("wrapped stqr must succeed");

        assert_eq!(
            read_qword_be(&spu, target).unwrap(), payload,
            "stqr should write to wrapped target 0x{target:X}",
        );
        assert_eq!(spu.pc, pc + 4);
    }

    /// 16-byte alignment: `stqr r5, +0x11` at pc=0 (make_env default)
    /// → arithmetic target 0x44 (bottom 4 bits set), aligned target
    /// 0x40. The store MUST land at 0x40 AND must not stray into the
    /// adjacent qword at 0x50.
    #[test]
    fn stqr_aligns_target_to_16_bytes() {
        let mut spu = make_env(&[encode::stqr(5, 0x11)]);

        // Pre-fill 0x50 with a sentinel to detect off-by-16 stores.
        // (0x40 will be overwritten by the store under test.)
        let sentinel: u128 = 0xDEAD_FACE_CAFE_BABE_0000_1111_2222_3333;
        write_qword_be(&mut spu, 0x50, sentinel).unwrap();

        let payload: u128 = 0x1111_2222_3333_4444_5555_6666_7777_8888;
        spu.gpr[5] = payload;

        let pc_before = spu.pc;
        step_ok(&mut spu);

        assert_eq!(
            read_qword_be(&spu, 0x40).unwrap(), payload,
            "stqr must write at 0x40 (aligned), not 0x44 (unaligned)",
        );
        assert_eq!(
            read_qword_be(&spu, 0x50).unwrap(), sentinel,
            "stqr must NOT touch the next qword at 0x50",
        );
        assert_eq!(spu.pc, pc_before + 4);
    }

    /// Regression-lock against the EXACT v4 instruction the R5.10f
    /// diagnostic surfaced at pc=0x868 (decimal 2152): `stqr r2, -426`
    /// = inst hex `0x23FF2B02` (decimal 603,925,250). Resolved target
    /// is 0x1C0 (= (0x868 + (-426 * 4)) & 0x3FFF0). Verifies the
    /// encode helper produces the exact bit pattern AND that the
    /// interpreter actually performs the store there.
    #[test]
    fn stqr_real_v4_inst_at_pc_868() {
        assert_eq!(
            encode::stqr(2, -426), 0x23FF2B02,
            "encoder must produce the exact v4 bit pattern",
        );

        let mut spu = SpuThread::new(0);
        let pc = 0x868u32;
        assert!(spu.ls_write(pc, &(0x23FF2B02u32).to_be_bytes()));

        let payload: u128 = 0xA0A1_A2A3_A4A5_A6A7_B0B1_B2B3_B4B5_B6B7;
        spu.gpr[2] = payload;

        spu.pc = pc;
        step(&mut spu).expect("v4 stqr must succeed");

        assert_eq!(
            read_qword_be(&spu, 0x1C0).unwrap(), payload,
            "real v4 stqr must store at 0x1C0",
        );
        assert_eq!(spu.pc, 0x86C);
    }

    // --- R5.10o: LQA / STQA (Absolute qword load/store) ------------
    //
    // Same RI16 encoding form as LQR/STQR but address is PC-independent:
    //   target = (imm16 << 2) & 0x3FFF0
    // Negative imm16 wraps via the `& 0x3FFF0` mask onto the top of LS
    // (the standard prologue/epilogue save-restore pattern uses negative
    // imm16 values to address the top scratch area).

    /// Round-trip: prepare gpr[rt] with a known qword, encode an STQA,
    /// step, and assert LS at the absolute resolved target receives
    /// the exact 16 bytes (and pc advances by 4).
    #[test]
    fn stqa_stores_quadword_to_absolute_target() {
        let mut spu = make_env(&[encode::stqa(7, 0x10)]);
        // pc=0 doesn't matter — STQA target ignores pc.
        // target = (0x10 << 2) & 0x3FFF0 = 0x40.
        let payload: u128 = 0x0123_4567_89AB_CDEF_FEDC_BA98_7654_3210;
        spu.gpr[7] = payload;
        let pc_before = spu.pc;
        step_ok(&mut spu);
        assert_eq!(
            read_qword_be(&spu, 0x40).unwrap(), payload,
            "stqa must write at absolute address 0x40 (PC-independent)",
        );
        assert_eq!(spu.pc, pc_before + 4, "stqa advances pc by 4");
    }

    /// Negative imm16 wrap to top of LS — the standard pattern for v4
    /// prologue saves. From any pc, `imm16=-12` → target = `(-12<<2) &
    /// 0x3FFF0` = `0xFFFFFFD0 & 0x3FFF0` = `0x3FFD0` (matches the v4
    /// site at pc=0x734).
    #[test]
    fn stqa_wraps_negative_absolute_address_to_top_of_ls() {
        let mut spu = make_env(&[encode::stqa(9, -12)]);
        let payload: u128 = 0xCAFE_F00D_DEAD_BEEF_1234_5678_9ABC_DEF0;
        spu.gpr[9] = payload;
        step_ok(&mut spu);
        assert_eq!(
            read_qword_be(&spu, 0x3FFD0).unwrap(), payload,
            "stqa imm16=-12 must wrap to top of LS at 0x3FFD0",
        );
    }

    /// LQA happy-path: write a qword to LS at an absolute target, then
    /// LQA from the same target → gpr[rt] receives the exact bytes.
    #[test]
    fn lqa_loads_quadword_from_absolute_target() {
        let mut spu = make_env(&[encode::lqa(5, 0x20)]);
        // target = (0x20 << 2) & 0x3FFF0 = 0x80.
        let payload: u128 = 0x1111_2222_3333_4444_5555_6666_7777_8888;
        write_qword_be(&mut spu, 0x80, payload).unwrap();
        let pc_before = spu.pc;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[5], payload, "lqa must load from 0x80");
        assert_eq!(spu.pc, pc_before + 4);
    }

    /// LQA with negative imm16 wrap — same pattern as v4 epilogue
    /// restore. `imm16=-8` → target = `0x3FFE0`.
    #[test]
    fn lqa_wraps_negative_absolute_address_to_top_of_ls() {
        let mut spu = make_env(&[encode::lqa(4, -8)]);
        let payload: u128 = 0xDEAD_FACE_CAFE_BABE_0000_1111_2222_3333;
        write_qword_be(&mut spu, 0x3FFE0, payload).unwrap();
        step_ok(&mut spu);
        assert_eq!(
            spu.gpr[4], payload,
            "lqa imm16=-8 must read from top-of-LS at 0x3FFE0",
        );
    }

    /// LQA + STQA round-trip: store a payload to top-of-LS via STQA,
    /// then load it back via LQA from the same address. Byte-exact.
    /// Mirrors the v4 prologue/epilogue save-restore pattern.
    #[test]
    fn lqa_stqa_roundtrip_absolute_top_of_ls() {
        let mut spu = make_env(&[
            encode::stqa(3, -12), // store r3 → 0x3FFD0
            encode::lqa(8, -12),  // load 0x3FFD0 → r8
        ]);
        let payload: u128 = 0xFEEDFACECAFEBABEDEADBEEF12345678;
        spu.gpr[3] = payload;
        step_ok(&mut spu);
        step_ok(&mut spu);
        assert_eq!(
            spu.gpr[8], payload,
            "lqa→stqa round-trip at top-of-LS must be byte-identical",
        );
        assert_eq!(
            read_qword_be(&spu, 0x3FFD0).unwrap(), payload,
            "LS at 0x3FFD0 must hold the stored payload",
        );
    }

    /// Anti-regression: LQR (pc-relative) MUST still use pc; running
    /// the same imm16 from two different pcs produces two different
    /// targets. Anti-regression locks the LQR semantics distinct from
    /// LQA (PC=0).
    #[test]
    fn lqr_remains_pc_relative_after_lqa_landing() {
        let payload_a: u128 = 0xAAAAAAAAAAAAAAAA0000000000000000;
        let payload_b: u128 = 0xBBBBBBBBBBBBBBBB0000000000000000;

        // LQR with imm16=0x10 from pc=0: target = (0 + 0x40) & 0x3FFF0 = 0x40.
        let mut spu_a = SpuThread::new(0);
        let inst = encode::lqr(7, 0x10);
        assert!(spu_a.ls_write(0, &inst.to_be_bytes()));
        write_qword_be(&mut spu_a, 0x40, payload_a).unwrap();
        spu_a.pc = 0;
        step(&mut spu_a).expect("lqr at pc=0");
        assert_eq!(spu_a.gpr[7], payload_a, "lqr from pc=0, imm=0x10 → target 0x40");

        // LQR with imm16=0x10 from pc=0x100: target = (0x100 + 0x40) & 0x3FFF0 = 0x140.
        let mut spu_b = SpuThread::new(0);
        assert!(spu_b.ls_write(0x100, &inst.to_be_bytes()));
        write_qword_be(&mut spu_b, 0x140, payload_b).unwrap();
        spu_b.pc = 0x100;
        step(&mut spu_b).expect("lqr at pc=0x100");
        assert_eq!(spu_b.gpr[7], payload_b, "lqr from pc=0x100, imm=0x10 → target 0x140");

        // Critically: LQR's behaviour is pc-dependent.
        assert_ne!(payload_a, payload_b, "test sanity");
    }

    /// Anti-regression: STQR (pc-relative) MUST still use pc. Same
    /// shape as the LQR test above.
    #[test]
    fn stqr_remains_pc_relative_after_stqa_landing() {
        let payload: u128 = 0xCAFEBABEDEADBEEFFEEDFACECAFEFACE;

        // STQR imm16=0x10 from pc=0: target = 0x40.
        let mut spu_a = SpuThread::new(0);
        assert!(spu_a.ls_write(0, &encode::stqr(7, 0x10).to_be_bytes()));
        spu_a.gpr[7] = payload;
        spu_a.pc = 0;
        step(&mut spu_a).expect("stqr at pc=0");
        assert_eq!(read_qword_be(&spu_a, 0x40).unwrap(), payload);

        // STQR imm16=0x10 from pc=0x100: target = 0x140.
        let mut spu_b = SpuThread::new(0);
        assert!(spu_b.ls_write(0x100, &encode::stqr(7, 0x10).to_be_bytes()));
        spu_b.gpr[7] = payload;
        spu_b.pc = 0x100;
        step(&mut spu_b).expect("stqr at pc=0x100");
        assert_eq!(read_qword_be(&spu_b, 0x140).unwrap(), payload);
    }

    // --- R5.10d: C-family insert-control opcodes -------------------
    //
    // Default mask (SPU big-endian byte order):
    //   bytes 0..7  = 0x18 0x19 0x1A 0x1B 0x1C 0x1D 0x1E 0x1F
    //   bytes 8..15 = 0x10 0x11 0x12 0x13 0x14 0x15 0x16 0x17
    // Granularity-aligned offset within the qword is `addr & mask`,
    // where `mask` is 0xF (Byte) / 0xE (Halfword) / 0xC (Word) /
    // 0x8 (Doubleword). The corresponding `g` consecutive bytes of
    // the default mask are overwritten with `0x00 0x01 ... (g-1)`.
    //
    // The tests below assert the EXACT 16-byte mask values produced.

    /// CDD with `(imm7 + ra_lane0) & 0x8 == 0` (the "low" doubleword
    /// case): the granularity bytes are written at offset 0 → mask
    /// covers SPU bytes 0..7 with `0x00..0x07`.
    #[test]
    fn cdd_generates_low_doubleword_insert_mask() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::cdd(/*rt=*/3, /*ra=*/2, /*imm7=*/0);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));
        // ra_lane0 = 0 → addr = 0 → addr & 0x8 = 0 → offset 0.
        spu.gpr[2] = 0;
        spu.pc = pc;
        step(&mut spu).expect("cdd must succeed");

        // R5.11b — default mask in SPU-BE is 0x10..0x1F linear; offset 0
        // overrides bytes 0..7 with 0x00..0x07.
        let expected: [u8; 16] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
        ];
        assert_eq!(spu.gpr[3].to_be_bytes(), expected);
        assert_eq!(spu.pc, pc + 4);
    }

    /// CDD with `(imm7 + ra_lane0) & 0x8 != 0` (the "high" doubleword
    /// case): mask covers SPU bytes 8..15 with `0x00..0x07`.
    /// We pre-load lane 0 of ra with bit 3 set (= 0x08).
    #[test]
    fn cdd_generates_high_doubleword_insert_mask() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::cdd(/*rt=*/3, /*ra=*/2, /*imm7=*/0);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));
        // ra_lane0 = 0x08 → addr = 0x08 → addr & 0x8 = 0x8 → offset 8.
        // Lane 0 = high u32 of u128, so v = 0x08 << 96.
        spu.gpr[2] = 0x08u128 << 96;
        spu.pc = pc;
        step(&mut spu).expect("cdd must succeed");

        // R5.11b — default mask in SPU-BE is 0x10..0x1F linear; offset 8
        // overrides bytes 8..15 with 0x00..0x07.
        let expected: [u8; 16] = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
        ];
        assert_eq!(spu.gpr[3].to_be_bytes(), expected);
        assert_eq!(spu.pc, pc + 4);
    }

    /// CWD: addr=4 → offset 4 (word-aligned via `& 0xC`). Mask covers
    /// SPU bytes 4..7 with `0x00 0x01 0x02 0x03` (insert preferred
    /// word of A); rest unchanged.
    #[test]
    fn cwd_generates_word_insert_mask() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::cwd(/*rt=*/4, /*ra=*/2, /*imm7=*/4);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));
        spu.gpr[2] = 0; // ra_lane0=0 → addr = imm7 = 4 → 4 & 0xC = 4.
        spu.pc = pc;
        step(&mut spu).expect("cwd must succeed");

        // R5.11b — default mask in SPU-BE is 0x10..0x1F linear; offset 4
        // overrides bytes 4..7 with 0x00..0x03.
        let expected: [u8; 16] = [
            0x10, 0x11, 0x12, 0x13, // bytes 0..3: unchanged
            0x00, 0x01, 0x02, 0x03, // bytes 4..7: inserted word
            0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
        ];
        assert_eq!(spu.gpr[4].to_be_bytes(), expected);
        assert_eq!(spu.pc, pc + 4);
    }

    /// CHD: addr=10 (& 0xE = 10). Mask covers SPU bytes 10..11 with
    /// `0x02 0x03` (the preferred halfword of A is at A's bytes 2..3).
    #[test]
    fn chd_generates_halfword_insert_mask() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::chd(/*rt=*/5, /*ra=*/2, /*imm7=*/10);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));
        spu.gpr[2] = 0;
        spu.pc = pc;
        step(&mut spu).expect("chd must succeed");

        // R5.11b — default mask in SPU-BE is 0x10..0x1F linear; offset 10
        // overrides bytes 10..11 with 0x02..0x03.
        let expected: [u8; 16] = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, // bytes 8..9: unchanged
            0x02, 0x03, // bytes 10..11: inserted halfword
            0x1C, 0x1D, 0x1E, 0x1F,
        ];
        assert_eq!(spu.gpr[5].to_be_bytes(), expected);
        assert_eq!(spu.pc, pc + 4);
    }

    /// CBD: single-byte insert. addr=3 → offset 3. Mask byte at SPU
    /// byte 3 = 0x03 (insert byte 3 of A's preferred word, i.e., the
    /// low byte of the preferred slot).
    #[test]
    fn cbd_generates_byte_insert_mask() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::cbd(/*rt=*/6, /*ra=*/2, /*imm7=*/3);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));
        spu.gpr[2] = 0;
        spu.pc = pc;
        step(&mut spu).expect("cbd must succeed");

        // R5.11b — default mask in SPU-BE is 0x10..0x1F linear; offset 3
        // overrides byte 3 with A's preferred byte (= 0x03).
        let expected: [u8; 16] = [
            0x10, 0x11, 0x12, 0x03, // byte 3 overridden
            0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F,
        ];
        assert_eq!(spu.gpr[6].to_be_bytes(), expected);
        assert_eq!(spu.pc, pc + 4);
    }

    /// CBX (RR-form) — addr = ra_lane0 + rb_lane0. Verifies the RR
    /// dispatch path uses `rb` instead of imm7. Pre-load ra=2, rb=5
    /// with values that combine to a known offset.
    #[test]
    fn cbx_uses_rb_plus_ra_source() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::cbx(/*rt=*/7, /*ra=*/2, /*rb=*/5);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));
        // ra_lane0 = 0x4, rb_lane0 = 0x9 → addr = 0xD → 0xD & 0xF = 0xD.
        spu.gpr[2] = 0x04u128 << 96;
        spu.gpr[5] = 0x09u128 << 96;
        spu.pc = pc;
        step(&mut spu).expect("cbx must succeed");

        // R5.11b — default mask in SPU-BE is 0x10..0x1F linear; offset 13
        // overrides byte 13 with 0x03.
        let expected: [u8; 16] = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x18, 0x19, 0x1A, 0x1B, 0x1C,
            0x03, // byte 13 overridden
            0x1E, 0x1F,
        ];
        assert_eq!(spu.gpr[7].to_be_bytes(), expected);
        assert_eq!(spu.pc, pc + 4);
    }

    /// CDD with the EXACT v4 instruction word from the R5.10b
    /// diagnostic: 0x3EE00085 at pc=0x854, with ra=1 (stack pointer)
    /// arriving at this site with a 16-byte-aligned value. We use a
    /// 16-byte-aligned SP that has bit 3 set (→ "high" doubleword
    /// case). This is the v4 path the SPU thread executes immediately
    /// after the LQR at pc=0x850; if it produces the right mask, the
    /// pipeline can advance to pc=0x858.
    #[test]
    /// R5.11b — cwd with the EXACT addr from `single_spu_loadstore_v1`'s
    /// final iteration (i=7): r10=0x4000C, imm7=0. Expected lane 0 =
    /// 0x10111213 (matches captured trace's r7 final value).
    #[test]
    fn cwd_loadstore_v1_final_iter_matches_captured_trace() {
        let mut spu = SpuThread::new(0);
        let pc = 0x100u32;
        let inst = encode::cwd(/*rt=*/7, /*ra=*/10, /*imm7=*/0);
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));
        spu.gpr[10] = 0x4000Cu128 << 96; // r10 lane0 = 0x4000C
        spu.pc = pc;
        step(&mut spu).expect("cwd must succeed");

        // addr=0x4000C, & 0xC = 0xC → p_byte=12. Bytes 12..15 → 0x00..0x03.
        // Default mask in SPU-BE: 0x10..0x1F linear.
        // Final mask bytes 0..3 (lane 0) = 0x10, 0x11, 0x12, 0x13.
        let lane0 = (spu.gpr[7] >> 96) as u32;
        assert_eq!(lane0, 0x10111213,
            "cwd lane 0 must match captured RPCS3 r7 = 0x10111213; got 0x{lane0:08x}");
    }

    fn cdd_real_v4_inst_at_pc_854() {
        let mut spu = SpuThread::new(0);
        // Place the v4 inst at pc=0x100 (any aligned slot — pc=0x854
        // would also work but is not material to the test). The
        // important bit is that the inst word equals 0x3EE00085 and
        // the SPU dispatches it correctly.
        let pc = 0x100u32;
        let inst: u32 = 0x3EE00085;
        assert!(spu.ls_write(pc, &inst.to_be_bytes()));
        // SP = 0x3FFE8 (16-byte aligned, bit 3 set).
        spu.gpr[1] = 0x3FFE8u128 << 96;
        spu.pc = pc;
        step(&mut spu).expect("CDD on real v4 inst must succeed");

        // R5.11b — (imm7=0 + sp_lane0=0x3FFE8) & 0x8 = 0x8 → high-doubleword
        // case; default mask is 0x10..0x1F linear.
        let expected: [u8; 16] = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
        ];
        assert_eq!(spu.gpr[5].to_be_bytes(), expected);
        assert_eq!(spu.pc, pc + 4);
    }

    // --- Iter-8: indirect branches -------------------------------

    #[test]
    fn bi_jumps_to_ra_preferred_aligned() {
        let mut spu = make_env(&[encode::bi(4)]);
        set_lane(&mut spu, 4, [0x100, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x100);
    }

    #[test]
    fn bi_aligns_target_to_4_bytes_within_local_store() {
        let mut spu = make_env(&[encode::bi(4)]);
        // 0xFFFF_FFFF & 0x3FFFC = 0x3FFFC (last instruction slot).
        set_lane(&mut spu, 4, [0xFFFF_FFFF, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x3FFFC);
    }

    #[test]
    fn bisl_writes_link_register_and_branches() {
        let mut spu = make_env(&[
            0, // pc=0: padding
            encode::bisl(2, 4),  // pc=4: bisl rt=2, ra=4 → next-pc=8 broadcast to gpr[2]
        ]);
        spu.pc = 4;
        set_lane(&mut spu, 4, [0x200, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x200);
        // Link register holds next-pc (8) broadcast across all 4 lanes.
        assert_eq!(split_lanes(spu.gpr[2]), [8, 8, 8, 8]);
    }

    #[test]
    fn iret_behaves_as_bi_without_irq_model() {
        let mut spu = make_env(&[encode::iret(4)]);
        set_lane(&mut spu, 4, [0x80, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x80);
    }

    #[test]
    fn hbr_is_nop_for_interpreter() {
        let mut spu = make_env(&[encode::hbr(4)]);
        set_lane(&mut spu, 4, [0xDEAD, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 4);
    }

    #[test]
    fn biz_branches_when_rt_preferred_is_zero() {
        let mut spu = make_env(&[encode::biz(2, 4)]);
        set_lane(&mut spu, 2, [0, 0, 0, 0]);
        set_lane(&mut spu, 4, [0x300, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x300);
    }

    #[test]
    fn biz_falls_through_when_rt_preferred_nonzero() {
        let mut spu = make_env(&[encode::biz(2, 4)]);
        set_lane(&mut spu, 2, [42, 0, 0, 0]);
        set_lane(&mut spu, 4, [0x300, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 4);
    }

    #[test]
    fn binz_is_inverse_of_biz() {
        let mut spu = make_env(&[encode::binz(2, 4)]);
        set_lane(&mut spu, 2, [42, 0, 0, 0]);
        set_lane(&mut spu, 4, [0x400, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x400);
    }

    #[test]
    fn bihz_tests_low_halfword_only() {
        // High half nonzero, low half zero → branches taken.
        let mut spu = make_env(&[encode::bihz(2, 4)]);
        set_lane(&mut spu, 2, [0xABCD_0000, 0, 0, 0]);
        set_lane(&mut spu, 4, [0x500, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x500);
    }

    #[test]
    fn bihnz_inverse_of_bihz() {
        let mut spu = make_env(&[encode::bihnz(2, 4)]);
        set_lane(&mut spu, 2, [0x0000_BEEF, 0, 0, 0]);
        set_lane(&mut spu, 4, [0x600, 0, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x600);
    }

    // --- Iter-9: vector word shifts ----------------------------

    #[test]
    fn shl_per_lane_count_from_rb() {
        let mut spu = make_env(&[encode::shl(3, 4, 5)]);
        set_lane(&mut spu, 4, [1, 1, 1, 1]);
        set_lane(&mut spu, 5, [0, 1, 4, 31]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [1, 2, 16, 0x8000_0000]);
    }

    #[test]
    fn shl_count_32_or_more_yields_zero() {
        let mut spu = make_env(&[encode::shl(3, 4, 5)]);
        set_lane(&mut spu, 4, [0xDEAD_BEEF; 4]);
        set_lane(&mut spu, 5, [32, 33, 63, 0x40 /* 64 → masked to 0 */]);
        step_ok(&mut spu);
        // Lanes 0..2: ≥32 → zero. Lane 3: 0x40 & 0x3F = 0 → no shift.
        assert_eq!(split_lanes(spu.gpr[3]), [0, 0, 0, 0xDEAD_BEEF]);
    }

    #[test]
    fn rot_per_lane_modulo_32() {
        let mut spu = make_env(&[encode::rot(3, 4, 5)]);
        set_lane(&mut spu, 4, [0x12345678, 0x12345678, 0x12345678, 0x12345678]);
        set_lane(&mut spu, 5, [0, 4, 16, 32 /* 32 & 0x1F = 0 → no rot */]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]),
                   [0x12345678, 0x23456781, 0x56781234, 0x12345678]);
    }

    #[test]
    fn rotm_logical_shr_with_negative_count_form() {
        let mut spu = make_env(&[encode::rotm(3, 4, 5)]);
        set_lane(&mut spu, 4, [0x8000_0000; 4]);
        // count for shr = (-rb) & 0x3F. So rb=1 → shr 63 (zero), rb=-1 (0xFFFF_FFFF & 0x3F=0x3F → shr 63 → zero), rb=-32 (0xFFFF_FFE0 & 0x3F=0x20 → shr 32 → zero), rb=-31 (0xFFFF_FFE1 & 0x3F=0x21 → shr 33 → zero — actually 33≥32 so zero too).
        set_lane(&mut spu, 5, [0, 1u32.wrapping_neg(), 0u32.wrapping_sub(31), 0u32.wrapping_sub(1)]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        // rb=0 → -0=0 & 0x3F = 0 → no shift.
        assert_eq!(r[0], 0x8000_0000);
        // rb=-1 → 1 & 0x3F = 1 → shr 1.
        assert_eq!(r[1], 0x4000_0000);
        // rb=-31 → 31 → shr 31 = 1.
        assert_eq!(r[2], 1);
        // rb=-1 lane3 → also shr 1.
        assert_eq!(r[3], 0x4000_0000);
    }

    #[test]
    fn rotma_arith_shr_preserves_sign() {
        let mut spu = make_env(&[encode::rotma(3, 4, 5)]);
        set_lane(&mut spu, 4, [0x8000_0000; 4]);
        // Same encoding as rotm (count = -rb & 0x3F), but sign-fill.
        set_lane(&mut spu, 5, [0u32.wrapping_sub(1), 0u32.wrapping_sub(31), 0u32.wrapping_sub(32), 0]);
        step_ok(&mut spu);
        let r = split_lanes(spu.gpr[3]);
        assert_eq!(r[0], 0xC000_0000); // shr 1 with sign-fill
        assert_eq!(r[1], 0xFFFF_FFFF); // shr 31 of 0x80000000 = all ones
        assert_eq!(r[2], 0xFFFF_FFFF); // shr 32 → all sign bits
        assert_eq!(r[3], 0x8000_0000); // no shift
    }

    #[test]
    fn roti_immediate_rotate() {
        let mut spu = make_env(&[encode::roti(3, 4, 4)]);
        set_lane(&mut spu, 4, [0x12345678; 4]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0x23456781; 4]);
    }

    #[test]
    fn rotmi_immediate_logical_shr() {
        // i7 = -4 → count = 4 → shr 4.
        let mut spu = make_env(&[encode::rotmi(3, 4, -4)]);
        set_lane(&mut spu, 4, [0xFF00_0000; 4]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0x0FF0_0000; 4]);
    }

    #[test]
    fn rotmai_immediate_arith_shr() {
        let mut spu = make_env(&[encode::rotmai(3, 4, -4)]);
        set_lane(&mut spu, 4, [0xFF00_0000; 4]);
        step_ok(&mut spu);
        // -16777216 >> 4 sign-extended = 0xFFF0_0000.
        assert_eq!(split_lanes(spu.gpr[3]), [0xFFF0_0000; 4]);
    }

    // --- Iter-9: bitwise complementaries -----------------------

    #[test]
    fn nand_is_not_and() {
        let mut spu = make_env(&[encode::nand(3, 4, 5)]);
        spu.gpr[4] = 0xFF00_FF00_FF00_FF00_FF00_FF00_FF00_FF00;
        spu.gpr[5] = 0x0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], !(spu.gpr[4] & spu.gpr[5]));
    }

    #[test]
    fn eqv_is_xnor() {
        let mut spu = make_env(&[encode::eqv(3, 4, 5)]);
        spu.gpr[4] = 0xAAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA_AAAA;
        spu.gpr[5] = 0xCCCC_CCCC_CCCC_CCCC_CCCC_CCCC_CCCC_CCCC;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], !(spu.gpr[4] ^ spu.gpr[5]));
    }

    #[test]
    fn andc_is_a_and_not_b() {
        let mut spu = make_env(&[encode::andc(3, 4, 5)]);
        spu.gpr[4] = 0xFFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF_FFFF;
        spu.gpr[5] = 0x0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], 0xF0F0_F0F0_F0F0_F0F0_F0F0_F0F0_F0F0_F0F0);
    }

    #[test]
    fn orc_is_a_or_not_b() {
        let mut spu = make_env(&[encode::orc(3, 4, 5)]);
        spu.gpr[4] = 0;
        spu.gpr[5] = 0x0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F_0F0F;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], !spu.gpr[5]);
    }

    // --- Iter-9: barriers + stopd ------------------------------

    #[test]
    fn sync_dsync_are_nops() {
        let mut spu = make_env(&[encode::sync(), encode::dsync(), encode::stop(0)]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 4);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 8);
    }

    #[test]
    fn stopd_halts_with_code_zero() {
        let mut spu = make_env(&[encode::stopd()]);
        match step_ok(&mut spu) {
            StepOutcome::Stop(0) => {}
            other => panic!("expected Stop(0), got {other:?}"),
        }
    }

    // --- Iter-9: extended compares -----------------------------

    #[test]
    fn ceqh_per_halfword_lane() {
        let mut spu = make_env(&[encode::ceqh(3, 4, 5)]);
        // 8 halves: a = [1,2,3,4,5,6,7,8], b = [1,2,9,4,9,6,9,8]
        // → matches at idx 0,1,3,5,7
        spu.gpr[4] = 0x0001_0002_0003_0004_0005_0006_0007_0008;
        spu.gpr[5] = 0x0001_0002_0009_0004_0009_0006_0009_0008;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], 0xFFFF_FFFF_0000_FFFF_0000_FFFF_0000_FFFF);
    }

    #[test]
    fn ceqb_per_byte_lane() {
        let mut spu = make_env(&[encode::ceqb(3, 4, 5)]);
        spu.gpr[4] = 0xAA_BB_CC_DD_AA_BB_CC_DD_AA_BB_CC_DD_AA_BB_CC_DDu128;
        spu.gpr[5] = 0xAA_00_CC_00_AA_00_CC_00_AA_00_CC_00_AA_00_CC_00u128;
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], 0xFF_00_FF_00_FF_00_FF_00_FF_00_FF_00_FF_00_FF_00u128);
    }

    #[test]
    fn cgth_signed_halfword_gt() {
        let mut spu = make_env(&[encode::cgth(3, 4, 5)]);
        // [1, -1, 100, -100, 0x7FFF, -0x8000, 5, -5] vs [0, 0, 0, 0, 0, 0, 5, 5]
        let a_halves: [i16; 8] = [1, -1, 100, -100, 0x7FFF, -0x8000, 5, -5];
        let b_halves: [i16; 8] = [0, 0, 0, 0, 0, 0, 5, 5];
        let pack = |hs: [i16; 8]| -> u128 {
            let mut bytes = [0u8; 16];
            for (i, h) in hs.iter().enumerate() {
                bytes[2*i..2*i+2].copy_from_slice(&h.to_be_bytes());
            }
            u128::from_be_bytes(bytes)
        };
        spu.gpr[4] = pack(a_halves);
        spu.gpr[5] = pack(b_halves);
        step_ok(&mut spu);
        // expected: 1>0=T, -1>0=F, 100>0=T, -100>0=F, MAX>0=T, MIN>0=F, 5>5=F, -5>5=F
        assert_eq!(spu.gpr[3], 0xFFFF_0000_FFFF_0000_FFFF_0000_0000_0000);
    }

    #[test]
    fn clgtb_unsigned_byte_gt() {
        let mut spu = make_env(&[encode::clgtb(3, 4, 5)]);
        // 16 bytes: a = 0xFF (255), b = 0x80 (128) → all gt.
        spu.gpr[4] = u128::from_be_bytes([0xFF; 16]);
        spu.gpr[5] = u128::from_be_bytes([0x80; 16]);
        step_ok(&mut spu);
        assert_eq!(spu.gpr[3], u128::from_be_bytes([0xFF; 16]));
    }

    // --- Iter-10: halfword arith + carry/borrow + or-across ----

    fn pack_halves(hs: [u16; 8]) -> u128 {
        let mut bytes = [0u8; 16];
        for (i, h) in hs.iter().enumerate() {
            bytes[2*i..2*i+2].copy_from_slice(&h.to_be_bytes());
        }
        u128::from_be_bytes(bytes)
    }

    fn unpack_halves(v: u128) -> [u16; 8] {
        let bytes = v.to_be_bytes();
        let mut out = [0u16; 8];
        for i in 0..8 {
            out[i] = u16::from_be_bytes([bytes[2*i], bytes[2*i+1]]);
        }
        out
    }

    #[test]
    fn ah_per_halfword_add() {
        let mut spu = make_env(&[encode::ah(3, 4, 5)]);
        spu.gpr[4] = pack_halves([1, 2, 3, 4, 100, 200, 300, 400]);
        spu.gpr[5] = pack_halves([10, 20, 30, 40, 0xFFFF, 1, 0, 0]);
        step_ok(&mut spu);
        assert_eq!(unpack_halves(spu.gpr[3]),
                   [11, 22, 33, 44, 99 /* wrap */, 201, 300, 400]);
    }

    #[test]
    fn sfh_per_halfword_sub_from() {
        // sfh rt, ra, rb → rt = rb - ra
        let mut spu = make_env(&[encode::sfh(3, 4, 5)]);
        spu.gpr[4] = pack_halves([1, 2, 3, 4, 5, 6, 7, 8]);
        spu.gpr[5] = pack_halves([10, 20, 30, 40, 50, 60, 70, 80]);
        step_ok(&mut spu);
        assert_eq!(unpack_halves(spu.gpr[3]), [9, 18, 27, 36, 45, 54, 63, 72]);
    }

    #[test]
    fn cg_carry_generate_per_word() {
        let mut spu = make_env(&[encode::cg(3, 4, 5)]);
        // [no-carry, carry, no-carry, carry-on-edge]
        set_lane(&mut spu, 4, [1, 0xFFFF_FFFF, 0x7FFF_FFFF, 0xFFFF_FFFE]);
        set_lane(&mut spu, 5, [2, 1, 1, 1]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0, 1, 0, 0]);
    }

    #[test]
    fn bg_borrow_generate_per_word() {
        // bg = 1 if a ≤ b (no borrow when computing rb-ra), else 0.
        let mut spu = make_env(&[encode::bg(3, 4, 5)]);
        set_lane(&mut spu, 4, [5, 5, 0, 0xFFFF_FFFF]);
        set_lane(&mut spu, 5, [10, 5, 0xFFFF_FFFF, 0]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [1 /* 5≤10 */, 1 /* equal */, 1 /* 0≤max */, 0 /* max>0 */]);
    }

    #[test]
    fn orx_collects_or_into_preferred_slot() {
        let mut spu = make_env(&[encode::orx(3, 4)]);
        set_lane(&mut spu, 4, [0x0000_0001, 0x0000_0002, 0x0000_0004, 0x0000_0008]);
        step_ok(&mut spu);
        assert_eq!(split_lanes(spu.gpr[3]), [0xF, 0, 0, 0]);
    }

    // --- Iter-10: brsl + hbra/hbrr -----------------------------

    #[test]
    fn brsl_writes_link_and_branches_relative() {
        let mut spu = make_env(&[
            0x4020_0000, // 0x000: nop (padding)
            encode::brsl(2, 4),  // 0x004: brsl rt=2, +4*4=+16 → target 0x14
        ]);
        spu.pc = 4;
        step_ok(&mut spu);
        assert_eq!(spu.pc, 0x14);
        // Link = next-pc broadcast = 8.
        assert_eq!(split_lanes(spu.gpr[2]), [8, 8, 8, 8]);
    }

    #[test]
    fn hbra_is_nop_for_interpreter() {
        let mut spu = make_env(&[encode::hbra(4)]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 4);
    }

    #[test]
    fn hbrr_is_nop_for_interpreter() {
        let mut spu = make_env(&[encode::hbrr(0x100)]);
        step_ok(&mut spu);
        assert_eq!(spu.pc, 4);
    }

    // --- Iter-10: halfword immediate compares ------------------

    #[test]
    fn ceqhi_per_halfword_eq_with_imm() {
        let mut spu = make_env(&[encode::ceqhi(3, 4, 42)]);
        spu.gpr[4] = pack_halves([42, 41, 42, 43, 42, 0, 42, 100]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        assert_eq!(r, [0xFFFF, 0, 0xFFFF, 0, 0xFFFF, 0, 0xFFFF, 0]);
    }

    #[test]
    fn cgthi_signed_halfword_gt_imm() {
        let mut spu = make_env(&[encode::cgthi(3, 4, -5)]);
        spu.gpr[4] = pack_halves([0, 0u16.wrapping_sub(10), 0u16.wrapping_sub(5),
                                   0u16.wrapping_sub(4), 1, 100, 0, 0u16.wrapping_sub(1)]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        // 0>-5 T, -10>-5 F, -5>-5 F, -4>-5 T, 1>-5 T, 100>-5 T, 0>-5 T, -1>-5 T
        assert_eq!(r, [0xFFFF, 0, 0, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF, 0xFFFF]);
    }

    #[test]
    fn clgthi_unsigned_halfword_gt_imm() {
        let mut spu = make_env(&[encode::clgthi(3, 4, 100)]);
        spu.gpr[4] = pack_halves([99, 100, 101, 200, 0, 0xFFFF, 50, 1000]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        assert_eq!(r, [0, 0, 0xFFFF, 0xFFFF, 0, 0xFFFF, 0, 0xFFFF]);
    }

    // --- R5.10i: byte-immediate RI10 family --------------------------

    /// Regression-lock against the EXACT v4 instruction the R5.10g
    /// diagnostic surfaced at pc=0x86C: `andbi r3, r3, 0x20` =
    /// `0x16080183` (decimal 369,623,427). Two assertions:
    /// 1. `encode::andbi(3, 3, 0x20)` produces the exact bit pattern
    ///    (proves the encoder + decoder bit layout match RPCS3).
    /// 2. Stepping the inst against a known gpr[3] payload masks each
    ///    byte with 0x20 — i.e. the i8 value flowing through is 0x20,
    ///    not the buggy 0x08 the pre-R5.10i decoder produced.
    #[test]
    fn andbi_regression_v4_0x16080183() {
        assert_eq!(encode::andbi(3, 3, 0x20), 0x16080183);

        let mut spu = make_env(&[encode::andbi(3, 3, 0x20)]);
        // Byte pattern: 0xFF in every byte → result must be 0x20 in
        // every byte after AND with broadcast(0x20).
        spu.gpr[3] = u128::MAX;
        step_ok(&mut spu);
        let expected = u128::from_be_bytes([0x20; 16]);
        assert_eq!(spu.gpr[3], expected, "andbi must AND each byte with 0x20");
    }

    /// Regression-lock against the off-by-2-bits decoder bug. Build a
    /// byte-imm inst whose i8 differs depending on the extraction
    /// shift: i8=0xA5 with the upper 2 bits of the 10-bit imm slot
    /// forced to 0 (which `pack_8_i8` does). Wrong extraction would
    /// give 0x29 (= 0xA5 >> 2), correct extraction gives 0xA5.
    /// Verifies the decoder fix at the interpreter dispatch layer too
    /// (since the interpreter reads `(inst >> 14) & 0xFF` directly).
    #[test]
    fn byte_imm_uses_bits_14_21_not_16_23() {
        let mut spu = make_env(&[encode::andbi(7, 4, 0xA5u8 as i8)]);
        spu.gpr[4] = u128::from_be_bytes([0xFF; 16]);
        step_ok(&mut spu);
        let result_bytes = spu.gpr[7].to_be_bytes();
        for (k, &b) in result_bytes.iter().enumerate() {
            assert_eq!(
                b, 0xA5,
                "byte {k}: expected 0xA5 (i8 from bits 14..21), got 0x{b:02X}",
            );
            assert_ne!(
                b, 0x29,
                "byte {k}: bug regression — i8 was extracted from bits 16..23",
            );
        }
    }

    #[test]
    fn orbi_broadcasts_i8_to_all_bytes() {
        let mut spu = make_env(&[encode::orbi(3, 4, 0x0F)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x00, 0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70,
            0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0,
        ]);
        step_ok(&mut spu);
        // Each byte | 0x0F sets the low 4 bits.
        let expected = u128::from_be_bytes([
            0x0F, 0x1F, 0x2F, 0x3F, 0x4F, 0x5F, 0x6F, 0x7F,
            0x8F, 0x9F, 0xAF, 0xBF, 0xCF, 0xDF, 0xEF, 0xFF,
        ]);
        assert_eq!(spu.gpr[3], expected);
    }

    #[test]
    fn xorbi_broadcasts_i8_to_all_bytes() {
        let mut spu = make_env(&[encode::xorbi(3, 4, 0xAA_u8 as i8)]);
        spu.gpr[4] = u128::from_be_bytes([0xFF; 16]);
        step_ok(&mut spu);
        // 0xFF ^ 0xAA = 0x55 in every byte.
        assert_eq!(spu.gpr[3], u128::from_be_bytes([0x55; 16]));
    }

    #[test]
    fn ceqbi_sets_ff_for_equal_bytes() {
        let mut spu = make_env(&[encode::ceqbi(3, 4, 0x42)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x42, 0x41, 0x42, 0x43, 0x00, 0x42, 0xFF, 0x42,
            0x42, 0x42, 0x00, 0x42, 0x42, 0x43, 0x42, 0x40,
        ]);
        step_ok(&mut spu);
        let expected = u128::from_be_bytes([
            0xFF, 0x00, 0xFF, 0x00, 0x00, 0xFF, 0x00, 0xFF,
            0xFF, 0xFF, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00,
        ]);
        assert_eq!(spu.gpr[3], expected);
    }

    #[test]
    fn clgtbi_unsigned_compare() {
        // i8 = 0x80 (= 128 unsigned). Bytes > 128 unsigned: 0x81..0xFF
        // → 0xFF; bytes ≤ 128: 0x00.
        let mut spu = make_env(&[encode::clgtbi(3, 4, 0x80_u8 as i8)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x00, 0x7F, 0x80, 0x81, 0xFE, 0xFF, 0x40, 0xC0,
            0x01, 0x02, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
        ]);
        step_ok(&mut spu);
        let expected = u128::from_be_bytes([
            0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0x00, 0xFF,
            0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
        ]);
        assert_eq!(spu.gpr[3], expected);
    }

    #[test]
    fn cgtbi_signed_compare() {
        // i8 = -5 (signed). Bytes signed > -5: -4..127 → 0xFF;
        // bytes ≤ -5 (= 0x80..0xFB unsigned): 0x00.
        let mut spu = make_env(&[encode::cgtbi(3, 4, -5)]);
        spu.gpr[4] = u128::from_be_bytes([
            0x00, // 0   > -5 → T
            0xFB, // -5  > -5 → F
            0xFC, // -4  > -5 → T
            0x80, // -128 > -5 → F
            0x7F, // 127 > -5 → T
            0xFF, // -1  > -5 → T
            0xF0, // -16 > -5 → F
            0x05, // 5   > -5 → T
            0xFA, // -6  > -5 → F
            0xFB, // -5 again
            0x01, // 1   > -5 → T
            0xFE, // -2  > -5 → T
            0xC0, // -64 > -5 → F
            0x40, // 64  > -5 → T
            0x80, // -128
            0x7F, // 127
        ]);
        step_ok(&mut spu);
        let expected = u128::from_be_bytes([
            0xFF, 0x00, 0xFF, 0x00, 0xFF, 0xFF, 0x00, 0xFF,
            0x00, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00, 0xFF,
        ]);
        assert_eq!(spu.gpr[3], expected);
    }

    // --- Iter-11: halfword shifts ------------------------------

    #[test]
    fn shlh_per_halfword_count_from_rb() {
        let mut spu = make_env(&[encode::shlh(3, 4, 5)]);
        spu.gpr[4] = pack_halves([1, 1, 1, 1, 1, 1, 1, 1]);
        spu.gpr[5] = pack_halves([0, 1, 4, 8, 15, 16 /* ≥16 → 0 */, 17, 31 /* ≥16 → 0 */]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        assert_eq!(r, [1, 2, 16, 256, 0x8000, 0, 0, 0]);
    }

    #[test]
    fn rothm_logical_shr_per_halfword() {
        let mut spu = make_env(&[encode::rothm(3, 4, 5)]);
        spu.gpr[4] = pack_halves([0xFF00; 8]);
        // count = (-bv) & 0x1F. bv=0 → 0 (no shift); bv=-4 (0xFFFC) → 4
        // (because (-(-4)) & 0x1F = 4); bv=1 → -1 & 0x1F = 31 (≥16 → 0).
        spu.gpr[5] = pack_halves([
            0,
            0u16.wrapping_sub(4),
            0u16.wrapping_sub(8),
            0u16.wrapping_sub(15),
            0u16.wrapping_sub(16),  // → shift 16 ≥16 → 0
            1,                      // → shift 31 ≥16 → 0
            0u16.wrapping_sub(2),
            0,
        ]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        assert_eq!(r[0], 0xFF00); // no shift
        assert_eq!(r[1], 0x0FF0); // shr 4
        assert_eq!(r[2], 0x00FF); // shr 8
        assert_eq!(r[3], 0x0001); // shr 15
        assert_eq!(r[4], 0);
        assert_eq!(r[5], 0);
        assert_eq!(r[6], 0x3FC0); // shr 2
        assert_eq!(r[7], 0xFF00); // no shift
    }

    #[test]
    fn rotmah_arith_shr_preserves_sign_per_halfword() {
        let mut spu = make_env(&[encode::rotmah(3, 4, 5)]);
        spu.gpr[4] = pack_halves([0x8000; 8]); // -32768 each lane
        spu.gpr[5] = pack_halves([
            0u16.wrapping_sub(1),
            0u16.wrapping_sub(15),
            0u16.wrapping_sub(16),
            0,
            0u16.wrapping_sub(4),
            0u16.wrapping_sub(8),
            0u16.wrapping_sub(2),
            0u16.wrapping_sub(7),
        ]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        // shr 1 of 0x8000 with sign-fill = 0xC000
        assert_eq!(r[0], 0xC000);
        // shr 15 of 0x8000 sign-extend = 0xFFFF
        assert_eq!(r[1], 0xFFFF);
        // shr 16 saturates to all-ones (sign bit set)
        assert_eq!(r[2], 0xFFFF);
        // no shift
        assert_eq!(r[3], 0x8000);
    }

    #[test]
    fn roth_rotate_per_halfword() {
        let mut spu = make_env(&[encode::roth(3, 4, 5)]);
        spu.gpr[4] = pack_halves([0x1234; 8]);
        spu.gpr[5] = pack_halves([0, 4, 8, 12, 16, 20, 1, 15]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        // rotate counts mod 16
        assert_eq!(r[0], 0x1234);  // 0
        assert_eq!(r[1], 0x2341);  // 4
        assert_eq!(r[2], 0x3412);  // 8
        assert_eq!(r[3], 0x4123);  // 12
        assert_eq!(r[4], 0x1234);  // 16 mod 16 = 0
        assert_eq!(r[5], 0x2341);  // 20 mod 16 = 4
        assert_eq!(r[6], 0x2468);  // 1
    }

    #[test]
    fn shlhi_const_shl_per_halfword() {
        let mut spu = make_env(&[encode::shlhi(3, 4, 4)]);
        spu.gpr[4] = pack_halves([1, 2, 4, 8, 0x1000, 0x4000, 0x8000, 0xFFFF]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        assert_eq!(r, [0x10, 0x20, 0x40, 0x80, 0x0000, 0x0000, 0x0000, 0xFFF0]);
    }

    #[test]
    fn rothmi_const_shr_per_halfword() {
        // i7 = -4 → count = 4 → shr 4
        let mut spu = make_env(&[encode::rothmi(3, 4, -4)]);
        spu.gpr[4] = pack_halves([0xFF00; 8]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        assert_eq!(r, [0x0FF0; 8]);
    }

    #[test]
    fn rotmahi_const_arith_shr_per_halfword() {
        let mut spu = make_env(&[encode::rotmahi(3, 4, -4)]);
        spu.gpr[4] = pack_halves([0xF000; 8]);
        step_ok(&mut spu);
        let r = unpack_halves(spu.gpr[3]);
        // (-4096) >> 4 sign-extended = (-256) = 0xFF00
        assert_eq!(r, [0xFF00; 8]);
    }

    #[test]
    fn rothi_const_rotate_per_halfword() {
        let mut spu = make_env(&[encode::rothi(3, 4, 4)]);
        spu.gpr[4] = pack_halves([0x1234; 8]);
        step_ok(&mut spu);
        assert_eq!(unpack_halves(spu.gpr[3]), [0x2341; 8]);
    }

    // =================================================================
    // R5.4a — Channel parking model in interpreter step()
    // =================================================================

    /// `rdch ch=29` (SPU_RDINMBOX) on empty mailbox returns
    /// ChannelStall outcome AND sets the SPU's `park_state` to
    /// `ChannelRead { channel: 29 }` at the rdch's pc.
    #[test]
    fn step_rdch_empty_inmbox_parks_thread() {
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let mut spu = make_env(&[rdch(3, 29)]);
        let pc_before = spu.pc;

        match step(&mut spu) {
            Ok(StepOutcome::ChannelStall { channel, is_write: false }) => {
                assert_eq!(channel, 29);
            }
            other => panic!("expected ChannelStall, got {other:?}"),
        }

        assert!(spu.is_parked(), "rdch on empty inmbox must park the thread");
        assert_eq!(spu.parked_pc(), Some(pc_before),
                   "park PC must equal rdch's pc, NOT pc+4");
        assert_eq!(spu.parked_reason(),
                   Some(SpuParkReason::ChannelRead { channel: 29 }));
        // PC was NOT advanced — rdch must be re-runnable from the
        // same pc once the parking condition resolves.
        assert_eq!(spu.pc, pc_before, "stall must NOT advance pc");
    }

    /// `wrch ch=28` (SPU_WROUTMBOX) on full mailbox parks with
    /// `ChannelWrite { channel: 28 }`.
    #[test]
    fn step_wrch_full_outmbox_parks_thread() {
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let mut spu = make_env(&[wrch(3, 28)]);
        spu.gpr[3] = 0x99u128;  // value to write
        spu.channels.out_mbox = Some(0xAAA);  // pre-fill so wrch stalls
        let pc_before = spu.pc;

        match step(&mut spu) {
            Ok(StepOutcome::ChannelStall { channel, is_write: true }) => {
                assert_eq!(channel, 28);
            }
            other => panic!("expected ChannelStall, got {other:?}"),
        }

        assert!(spu.is_parked());
        assert_eq!(spu.parked_pc(), Some(pc_before));
        assert_eq!(spu.parked_reason(),
                   Some(SpuParkReason::ChannelWrite { channel: 28 }));
        // out_mbox unchanged — wrch on stall does not mutate.
        assert_eq!(spu.channels.out_mbox, Some(0xAAA));
    }

    /// `rdch` against a bad channel (e.g. 100) must NOT park —
    /// it returns `Err(Unimplemented)`. `BadChannel` is distinguished
    /// from `WouldStall` semantically; only the latter parks.
    #[test]
    fn step_rdch_bad_channel_does_not_park() {
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let mut spu = make_env(&[rdch(3, 100)]);

        match step(&mut spu) {
            Err(Error::Unimplemented { reason, .. }) => {
                assert!(reason.contains("rdch"));
            }
            other => panic!("expected Unimplemented, got {other:?}"),
        }

        assert!(!spu.is_parked(),
                "BadChannel must NOT park (only WouldStall does)");
        assert!(spu.park_state.is_none());
    }

    /// Successful `rdch` on a channel with a value available does
    /// NOT park (and advances pc, returns Continue).
    #[test]
    fn step_rdch_success_does_not_park() {
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        // SPU_RDDEC always returns Ok (count = 1 always-readable).
        let mut spu = make_env(&[rdch(3, 8 /* SPU_RDDEC */)]);
        spu.channels.decrementer = 0xDEADBEEF;
        let pc_before = spu.pc;
        step_ok(&mut spu);
        assert!(!spu.is_parked());
        assert_eq!(spu.pc, pc_before.wrapping_add(4));
        // rdch writes `join_lanes([value, 0, 0, 0])` — value lands
        // in lane 0 (high u32) of the u128 register.
        assert_eq!(spu.gpr[3] >> 96, 0xDEADBEEF_u128);
    }

    /// Resume flow: park on rdch → external code injects a value
    /// into the mailbox → clear_park → re-run → rdch consumes the
    /// value and continues normally.
    #[test]
    fn manual_resume_flow_after_park_and_inject() {
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = 0xAAu32 & 0x3FFF;
        let mut spu = make_env(&[rdch(3, 29), stop]);

        // First run: stall, park, no progress on pc.
        let r1 = run_n(&mut spu, 100).unwrap();
        match r1 {
            (n, StepOutcome::ChannelStall { channel: 29, is_write: false }) => {
                assert!(n <= 1, "interpreter should stop on the rdch step");
            }
            other => panic!("expected ChannelStall, got {other:?}"),
        }
        assert!(spu.is_parked());
        let parked_pc = spu.parked_pc().unwrap();

        // Inject a value externally (PPU side).
        assert!(spu.channels.ppu_push_inmbox(0x12345));

        // Clear the park state.
        spu.clear_park();
        assert!(!spu.is_parked());

        // Second run: same pc, but now mailbox has a value. rdch
        // consumes it, advances pc, hits stop.
        let (_n, outcome) = run_n(&mut spu, 100).unwrap();
        match outcome {
            StepOutcome::Stop(0xAA) => {}
            other => panic!("expected Stop, got {other:?}"),
        }
        assert_eq!(spu.gpr[3] >> 96, 0x12345u128,
                   "rdch must have consumed the injected value");
        // rdch advances pc to parked_pc+4; stop leaves pc at its own
        // address (= parked_pc+4). So final pc is parked_pc+4.
        assert_eq!(spu.pc, parked_pc + 4,
                   "rdch must have advanced past itself; stop holds pc");
        // Mailbox drained.
        assert_eq!(spu.channels.in_mbox, None);
    }

    // =====================================================================
    // R5.4b — wake API integration tests (interpreter side)
    //
    // Verifies the explicit wake handshake is byte-exact against the
    // existing manual flow (clear_park + run_n) for the same program.
    // =====================================================================

    /// Resume after wake on rdch SPU_RDINMBOX:
    /// 1. First run parks (channel empty).
    /// 2. `ppu_push_inmbox_and_try_wake` returns `Ready { pc }` at the
    ///    rdch's pc.
    /// 3. `run_n` from that pc consumes the value, advances, and stops.
    /// 4. Final SpuThread state matches the manual `clear_park + run_n`
    ///    flow byte-for-byte.
    #[test]
    fn wake_api_resume_rdch_inmbox_matches_manual_flow() {
        use rpcs3_spu_thread::SpuWakeResult;
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = encode::stop(0xAB);
        let program = [rdch(3, 29), stop];

        // ---- A) wake-API flow ------------------------------------
        let mut spu_a = make_env(&program);
        let r1 = run_n(&mut spu_a, 100).unwrap();
        match r1 {
            (_, StepOutcome::ChannelStall { channel: 29, is_write: false }) => {}
            other => panic!("expected ChannelStall, got {other:?}"),
        }
        assert!(spu_a.is_parked());
        let parked_pc = spu_a.parked_pc().unwrap();

        let wake = spu_a.ppu_push_inmbox_and_try_wake(0xCAFEBABE);
        match wake {
            SpuWakeResult::Ready { pc } => assert_eq!(pc, parked_pc),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(!spu_a.is_parked(), "Ready must clear park_state");
        // Resume from the returned pc by running directly — pc was
        // never altered by the stall, so run_n picks it up.
        assert_eq!(spu_a.pc, parked_pc);
        let (_n, outcome) = run_n(&mut spu_a, 100).unwrap();
        match outcome {
            StepOutcome::Stop(0xAB) => {}
            other => panic!("expected Stop(0xAB), got {other:?}"),
        }
        assert_eq!(spu_a.gpr[3] >> 96, 0xCAFEBABE_u128);
        assert_eq!(spu_a.pc, parked_pc + 4);
        assert_eq!(spu_a.channels.in_mbox, None);

        // ---- B) manual flow (existing R5.4a baseline) -------------
        let mut spu_b = make_env(&program);
        let _ = run_n(&mut spu_b, 100).unwrap();
        assert!(spu_b.channels.ppu_push_inmbox(0xCAFEBABE));
        spu_b.clear_park();
        let _ = run_n(&mut spu_b, 100).unwrap();

        // ---- C) byte-exact equivalence ---------------------------
        assert_eq!(spu_a.pc, spu_b.pc, "pc must match");
        assert_eq!(spu_a.gpr, spu_b.gpr, "gpr must match");
        assert_eq!(spu_a.channels, spu_b.channels, "channels must match");
        assert_eq!(spu_a.ls, spu_b.ls, "local store must match");
    }

    /// Resume after wake on wrch SPU_WROUTMBOX:
    /// 1. Pre-fill `out_mbox` so wrch parks.
    /// 2. PPU drains via `ppu_pop_outmbox_and_try_wake` → wake returns
    ///    Ready, drained value is the pre-fill value.
    /// 3. Resume executes wrch; new value lands in `out_mbox`.
    #[test]
    fn wake_api_resume_wrch_outmbox_matches_manual_flow() {
        use rpcs3_spu_thread::SpuWakeResult;
        let wrch = |rt: u32, ch: u32| (0x10Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let stop = encode::stop(0xCD);
        // r3 = 0x9999; wrch r3 -> SPU_WROUTMBOX(28); stop 0xCD
        let program = [encode::il(3, 0x1234), wrch(3, 28), stop];

        // ---- A) wake-API flow ------------------------------------
        let mut spu_a = make_env(&program);
        spu_a.channels.out_mbox = Some(0xAAAA_BBBB); // pre-fill — wrch will park
        let r1 = run_n(&mut spu_a, 100).unwrap();
        match r1 {
            (_, StepOutcome::ChannelStall { channel: 28, is_write: true }) => {}
            other => panic!("expected ChannelStall, got {other:?}"),
        }
        assert!(spu_a.is_parked());
        let parked_pc = spu_a.parked_pc().unwrap();

        let (drained, wake) = spu_a.ppu_pop_outmbox_and_try_wake();
        assert_eq!(drained, Some(0xAAAA_BBBB));
        match wake {
            SpuWakeResult::Ready { pc } => assert_eq!(pc, parked_pc),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(!spu_a.is_parked());
        assert_eq!(spu_a.pc, parked_pc);
        let (_n, outcome) = run_n(&mut spu_a, 100).unwrap();
        match outcome {
            StepOutcome::Stop(0xCD) => {}
            other => panic!("expected Stop, got {other:?}"),
        }
        // wrch wrote r3's preferred-slot (lane 0 of 0x1234 sign-extended
        // to u128) into out_mbox.
        assert_eq!(spu_a.channels.out_mbox, Some(0x1234));

        // ---- B) manual flow --------------------------------------
        let mut spu_b = make_env(&program);
        spu_b.channels.out_mbox = Some(0xAAAA_BBBB);
        let _ = run_n(&mut spu_b, 100).unwrap();
        let _ = spu_b.channels.ppu_pop_outmbox();
        spu_b.clear_park();
        let _ = run_n(&mut spu_b, 100).unwrap();

        // ---- C) byte-exact equivalence ---------------------------
        assert_eq!(spu_a.pc, spu_b.pc);
        assert_eq!(spu_a.gpr, spu_b.gpr);
        assert_eq!(spu_a.channels, spu_b.channels);
        assert_eq!(spu_a.ls, spu_b.ls);
    }

    /// Wake without satisfied condition does NOT alter GPRs, LS, or
    /// channel state (other than the helper's own primary side effect).
    /// Specifically: calling `signal_and_try_wake` while parked on
    /// rdch INMBOX (different channel) returns `StillBlocked` and
    /// leaves everything except `snr[slot]` untouched.
    #[test]
    fn wake_api_still_blocked_does_not_advance_state() {
        use rpcs3_spu_thread::SpuWakeResult;
        let rdch = |rt: u32, ch: u32| (0x00Du32 << 21) | ((ch & 0x7F) << 7) | rt;
        let mut spu = make_env(&[rdch(3, 29)]);
        let _ = run_n(&mut spu, 100).unwrap();
        assert!(spu.is_parked());

        let pc_before = spu.pc;
        let gpr_before = spu.gpr;
        let park_before = spu.park_state;
        let inmbox_before = spu.channels.in_mbox;

        // Wrong wake path: signal slot 0 — park is on RDINMBOX, not
        // RDSIGNOTIFY. Wake must return StillBlocked.
        let wake = spu.signal_and_try_wake(0, 0xFF);
        assert_eq!(wake, SpuWakeResult::StillBlocked);

        assert!(spu.is_parked());
        assert_eq!(spu.park_state, park_before);
        assert_eq!(spu.pc, pc_before);
        assert_eq!(spu.gpr, gpr_before);
        assert_eq!(spu.channels.in_mbox, inmbox_before);
        // The signal landed (helper's primary side effect) — that is
        // expected and not part of the "no advance" guarantee.
        assert_eq!(spu.channels.snr[0], 0xFF);
    }

    /// Pre-existing fixtures (no parking) must continue to terminate
    /// cleanly without ever touching `park_state`.
    #[test]
    fn fixtures_without_channel_ops_never_park() {
        // il r3, 0x42 ; stop 0
        let mut spu = make_env(&[encode::il(3, 0x42), encode::stop(0)]);
        let (_n, outcome) = run_n(&mut spu, 100).unwrap();
        match outcome {
            StepOutcome::Stop(0) => {}
            other => panic!("expected Stop, got {other:?}"),
        }
        assert!(!spu.is_parked());
        assert!(spu.park_state.is_none());
    }
}
