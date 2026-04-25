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
use rpcs3_spu_thread::{ChannelStatus, SpuThread, SPU_LS_SIZE};

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
    let mut arr = [0u8; 16];
    arr.copy_from_slice(bytes);
    // Big-endian 128-bit load — lane 0 is the high bytes.
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

#[inline]
fn broadcast_u32(v: u32) -> u128 {
    join_lanes([v, v, v, v])
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

    // ---- 11-bit register-form ALU -----------------------------
    match bits(inst, 0, 11) {
        // a rt, ra, rb  — word add
        0x180 => {
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
        // sf rt, ra, rb  — word sub-from (rb - ra)
        0x080 => {
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
        // and rt, ra, rb
        0x181 => {
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
        // shlqbyi rt, ra, imm7 — shift left quadword by bytes immediate
        0x1FB => {
            let sh = (i7(inst) & 0x1F) as u32;
            let bytes = spu.gpr[ra(inst)].to_be_bytes();
            let mut out = [0u8; 16];
            if sh < 16 {
                // Shift left: byte 0 of out = byte sh of bytes, etc.
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
        // fm rt, ra, rb — float multiply
        0x2C6 => {
            let a = split_lanes(spu.gpr[ra(inst)]);
            let b = split_lanes(spu.gpr[rb(inst)]);
            let r = [
                (f32::from_bits(a[0]) * f32::from_bits(b[0])).to_bits(),
                (f32::from_bits(a[1]) * f32::from_bits(b[1])).to_bits(),
                (f32::from_bits(a[2]) * f32::from_bits(b[2])).to_bits(),
                (f32::from_bits(a[3]) * f32::from_bits(b[3])).to_bits(),
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
                    return Ok(StepOutcome::ChannelStall { channel, is_write: false });
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
            match spu.channels.write(channel, value) {
                Ok(()) => {
                    spu.pc = pc.wrapping_add(4);
                    return Ok(StepOutcome::Continue);
                }
                Err(ChannelStatus::WouldStall) => {
                    return Ok(StepOutcome::ChannelStall { channel, is_write: true });
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
    // Layout: primary (4) | rc (7) | rb (7) | ra (7) | rt (7)
    // `rc` sits at bits 25..31 (same position as `rt` in RR form);
    // `rt` migrates to bits 4..=10 in RRR — the SPU ISA peculiarity.
    match bits(inst, 0, 4) {
        0x8 => {
            // selb rt, ra, rb, rc  — (rc & rb) | (!rc & ra) bit-wise.
            let rt_idx = bits(inst, 25, 7) as usize;  // RRR: rt in low 7 bits
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 4, 7) as usize;
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
            let rt_idx = bits(inst, 25, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 4, 7) as usize;
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
            let rt_idx = bits(inst, 25, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 4, 7) as usize;
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
            let rt_idx = bits(inst, 25, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 4, 7) as usize;
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
            let rt_idx = bits(inst, 25, 7) as usize;
            let rb_idx = bits(inst, 11, 7) as usize;
            let ra_idx = bits(inst, 18, 7) as usize;
            let rc_idx = bits(inst, 4, 7) as usize;
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
        // stqd rt, imm10*16(ra)  — store qword
        0x24 => {
            let off = i10(inst).wrapping_mul(16);
            let base = split_lanes(spu.gpr[ra(inst)])[0];
            let lsa = base.wrapping_add_signed(off) & (SPU_LS_SIZE as u32 - 1);
            write_qword_be(spu, lsa, spu.gpr[rt(inst)])?;
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
    pub const fn a(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x180, rt, ra, rb) }
    /// `sf rt, ra, rb` — rt = rb - ra
    #[must_use]
    pub const fn sf(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x080, rt, ra, rb) }
    /// `and rt, ra, rb`
    #[must_use]
    pub const fn and(rt: u32, ra: u32, rb: u32) -> u32 { pack_rr(0x181, rt, ra, rb) }
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
    /// `shlqbyi rt, ra, imm7` — shift left quadword by imm7 bytes.
    #[must_use]
    pub const fn shlqbyi(rt: u32, ra: u32, imm7: i8) -> u32 { pack_ri7(0x1FB, rt, ra, imm7) }

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

    // ---- RRR-form (iter-5): primary 4 | rc 7 | rb 7 | ra 7 | rt 7 ----
    const fn pack_rrr(primary_4: u32, rt: u32, ra: u32, rb: u32, rc: u32) -> u32 {
        ((primary_4 & 0xF) << 28)
            | ((rc & 0x7F) << 21)
            | ((rb & 0x7F) << 14)
            | ((ra & 0x7F) << 7)
            | (rt & 0x7F)
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
        let mut spu = make_env(&[
            encode::rdch(3, ch::SPU_RDSIGNOTIFY1),
            encode::rdch(4, ch::SPU_RDSIGNOTIFY1),
            encode::stop(0),
        ]);
        spu.channels.signal(0, 0xA5A5);
        run_n(&mut spu, 10).unwrap();
        assert_eq!(split_lanes(spu.gpr[3])[0], 0xA5A5);
        assert_eq!(split_lanes(spu.gpr[4])[0], 0, "signal cleared after first read");
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
}
