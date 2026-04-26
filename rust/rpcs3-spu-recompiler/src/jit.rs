//! Cranelift-backed JIT for SPU functions.
//!
//! ## Status (2026-04-25, R2.5 expansion)
//!
//! Multi-block compile: every SPU basic block becomes a Cranelift
//! block, with direct branches turning into Cranelift jumps and
//! conditional branches into `brif`. Functions containing any
//! indirect terminator (bi/bisl/iret/biz/binz/bihz/bihnz) or
//! unrecognised opcode are rejected up front.
//!
//! Supported instructions:
//! - `stop`, `nop`, `lnop`
//! - `il`, `ila`, `ilh`, `ilhu`, `iohl`
//! - `a`, `sf`, `and`, `or`, `xor`, `nor` (RR word ops)
//! - `ai`, `andi`, `ori`, `xori`, `ceqi`, `cgti` (RI10 word imm)
//! - `br`, `bra`, `brnz`, `brz`, `brsl` (direct + cond branches)
//!
//! Anything else: `JitError::Unsupported`. Caller (`RecompilerExecutor`)
//! falls back to the interpreter for that function.
//!
//! ## ABI
//!
//! ```text
//! extern "C" fn(state: *mut JitState) -> u32
//! ```
//!
//! Returns one of [`JIT_OUTCOME_*`] codes. `state.pc` is updated
//! before return; for `STOP`, `state.stop_code` carries the 14-bit code.

use std::collections::BTreeMap;

use cranelift::codegen::ir::condcodes::IntCC;
use cranelift::codegen::ir::types::{I32, I64};
use cranelift::codegen::ir::{AbiParam, Block, InstBuilder, MemFlags, Signature, Value};
use cranelift::frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift::prelude::settings::{self, Configurable};
use cranelift::prelude::EntityRef;
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Linkage, Module};

use rpcs3_spu_decoder::{
    decode_function, BlockTerminator, DecodeError, SpuFunction, SpuInstKind,
};

/// JIT outcome: clean stop. `state.stop_code` carries the 14-bit code,
/// `state.pc` is the address of the stop instruction itself.
pub const JIT_OUTCOME_STOP: u32 = 0;
/// JIT outcome: indirect branch taken — the dispatcher should resume
/// execution at `state.pc` (already updated to the indirect target).
/// Replaces the older "bailout to interpreter" semantics: with R4a's
/// dispatcher loop, the JIT now stays in JIT-land across indirect
/// branches by re-entering the dispatcher with a new compile-or-fetch.
pub const JIT_OUTCOME_CONTINUE_TO: u32 = 1;
/// Reserved for channel stalls (not yet emitted by the JIT).
pub const JIT_OUTCOME_STALL: u32 = 2;
/// Decoder hit something it doesn't recognise mid-execution. Caller
/// must fall back to interpreter for the remainder. `state.pc` is
/// where execution should resume.
pub const JIT_OUTCOME_UNKNOWN_OPCODE: u32 = 3;

/// 256 KB local store, masked to 4-byte alignment for PCs.
const SPU_LS_MASK_PC: u32 = 0x3FFFC;

const SPU_LS_MASK: u32 = 0x3FFFC;

/// Layout of the state the JIT operates on. Memory layout MUST match
/// what `RecompilerExecutor` allocates — the codegen computes field
/// offsets at compile time via `offset_of!`.
#[repr(C)]
pub struct JitState {
    /// 128 GPRs × 4 lanes × u32. Lane 0 = preferred slot (high u32).
    pub gpr_lanes: [[u32; 4]; 128],
    /// Final PC after execution.
    pub pc: u32,
    /// 14-bit stop code (valid only when outcome == JIT_OUTCOME_STOP).
    pub stop_code: u32,
    /// Pointer to the 256 KB Local Store. Read/written by lqd/stqd/
    /// lqx/stqx codegen. Tests that don't trigger LS access can
    /// leave this as a null pointer; the JIT only dereferences it
    /// when one of the load/store opcodes is actually compiled.
    pub ls_ptr: *mut u8,
}

impl JitState {
    pub fn new() -> Self {
        Self { gpr_lanes: [[0; 4]; 128], pc: 0, stop_code: 0, ls_ptr: std::ptr::null_mut() }
    }

    pub fn store_gpr(&mut self, idx: usize, value: u128) {
        self.gpr_lanes[idx] = [
            (value >> 96) as u32,
            (value >> 64) as u32,
            (value >> 32) as u32,
            value as u32,
        ];
    }

    pub fn load_gpr(&self, idx: usize) -> u128 {
        let l = &self.gpr_lanes[idx];
        ((l[0] as u128) << 96)
            | ((l[1] as u128) << 64)
            | ((l[2] as u128) << 32)
            | (l[3] as u128)
    }
}

impl Default for JitState {
    fn default() -> Self { Self::new() }
}

#[derive(Debug, Clone)]
pub enum JitError {
    Unsupported { pc: u32, raw: u32, reason: String },
    Decode(DecodeError),
    Backend(String),
}

impl std::fmt::Display for JitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { pc, raw, reason } => {
                write!(f, "JIT unsupported at pc=0x{pc:x} raw=0x{raw:08x}: {reason}")
            }
            Self::Decode(e) => write!(f, "JIT decode error: {e}"),
            Self::Backend(s) => write!(f, "JIT backend error: {s}"),
        }
    }
}

impl std::error::Error for JitError {}

impl From<DecodeError> for JitError {
    fn from(e: DecodeError) -> Self { Self::Decode(e) }
}

pub struct CompiledFunction {
    entry: extern "C" fn(*mut JitState) -> u32,
}

impl CompiledFunction {
    pub fn call(&self, state: &mut JitState) -> u32 {
        (self.entry)(state as *mut JitState)
    }

    /// Raw `extern "C"` function pointer to the JIT-compiled entry. Stable
    /// for the lifetime of the owning `JITModule` (Cranelift never relocates
    /// finalised code), so safe to copy into a chain table for direct calls.
    pub fn entry_fn(&self) -> extern "C" fn(*mut JitState) -> u32 {
        self.entry
    }
}

pub struct JitBackend {
    module: JITModule,
    counter: u64,
}

impl JitBackend {
    pub fn new() -> Self {
        let mut flag_builder = settings::builder();
        flag_builder.set("use_colocated_libcalls", "false").unwrap();
        flag_builder.set("is_pic", "false").unwrap();
        let isa_builder = cranelift_native::builder().expect("cranelift native ISA");
        let isa = isa_builder
            .finish(settings::Flags::new(flag_builder))
            .expect("Cranelift ISA finalisation");
        let builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
        let module = JITModule::new(builder);
        Self { module, counter: 0 }
    }

    pub fn compile_at(&mut self, ls: &[u8], entry_pc: u32) -> Result<CompiledFunction, JitError> {
        let func = decode_function(ls, entry_pc, 256)?;
        self.compile(&func)
    }

    pub fn compile(&mut self, function: &SpuFunction) -> Result<CompiledFunction, JitError> {
        // Pre-flight: every block + every instruction must be supported.
        for block in function.blocks.values() {
            check_block_supported(block.start_pc, &block.terminator)?;
            for inst in &block.instructions {
                supported_check(inst.kind, inst.pc, inst.raw)?;
            }
            // Also: every direct branch target must point to a block
            // we know about (decoder's two-pass guarantees this for
            // well-formed input, but defend against decoder edge cases).
            for succ in block.terminator.direct_successors() {
                if !function.blocks.contains_key(&succ) {
                    return Err(JitError::Unsupported {
                        pc: block.end_pc,
                        raw: 0,
                        reason: format!("branch target 0x{succ:x} not in function"),
                    });
                }
            }
        }

        let name = format!("spu_func_{}", self.counter);
        self.counter += 1;

        let mut sig = Signature::new(self.module.target_config().default_call_conv);
        sig.params.push(AbiParam::new(I64));   // *mut JitState
        sig.returns.push(AbiParam::new(I32));  // outcome

        let func_id = self.module
            .declare_function(&name, Linkage::Local, &sig)
            .map_err(|e| JitError::Backend(format!("declare: {e}")))?;

        let mut ctx = self.module.make_context();
        ctx.func.signature = sig;

        let mut fb_ctx = FunctionBuilderContext::new();
        let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);

        // Variable that carries the JitState pointer across blocks.
        let state_var = Variable::new(0);
        builder.declare_var(state_var, I64);

        // Create one Cranelift block per SPU basic block.
        let mut block_map: BTreeMap<u32, Block> = BTreeMap::new();
        for &pc in function.blocks.keys() {
            block_map.insert(pc, builder.create_block());
        }

        // The SPU entry block becomes the function entry. Append the
        // *mut JitState parameter and bind it to the Variable.
        let entry_block = *block_map.get(&function.entry).ok_or_else(|| {
            JitError::Backend("no entry block in decoded function".into())
        })?;
        builder.append_block_params_for_function_params(entry_block);
        builder.switch_to_block(entry_block);
        let state_param = builder.block_params(entry_block)[0];
        builder.def_var(state_var, state_param);

        // Now emit each block.
        for (pc, spu_block) in &function.blocks {
            let cl_block = block_map[pc];
            if *pc != function.entry {
                builder.switch_to_block(cl_block);
            }
            let state = builder.use_var(state_var);
            emit_block(&mut builder, state, state_var, spu_block, &block_map)?;
        }

        builder.seal_all_blocks();
        builder.finalize();

        self.module
            .define_function(func_id, &mut ctx)
            .map_err(|e| JitError::Backend(format!("define: {e}")))?;
        self.module.clear_context(&mut ctx);
        self.module
            .finalize_definitions()
            .map_err(|e| JitError::Backend(format!("finalize: {e}")))?;

        let raw_ptr = self.module.get_finalized_function(func_id);
        let entry_fn: extern "C" fn(*mut JitState) -> u32 =
            unsafe { std::mem::transmute(raw_ptr) };
        Ok(CompiledFunction { entry: entry_fn })
    }
}

impl Default for JitBackend {
    fn default() -> Self { Self::new() }
}

// =====================================================================
// Block-level codegen
// =====================================================================

fn emit_block(
    builder: &mut FunctionBuilder,
    state: Value,
    state_var: Variable,
    block: &rpcs3_spu_decoder::SpuBasicBlock,
    block_map: &BTreeMap<u32, Block>,
) -> Result<(), JitError> {
    // Emit every instruction except possibly the last (if the last is
    // a branch / stop, it's handled by the terminator emitter below).
    let last_idx = block.instructions.len();
    let last_is_terminator = block.instructions.last().map(|i| is_block_terminator(i.kind)).unwrap_or(false);
    let inst_emit_count = if last_is_terminator { last_idx.saturating_sub(1) } else { last_idx };

    for inst in &block.instructions[..inst_emit_count] {
        emit_inst(builder, state, inst.kind, inst.pc, inst.raw)?;
    }

    let last = block.instructions.last();
    match &block.terminator {
        BlockTerminator::Stop { code } => {
            let last = last.expect("Stop block must have an instruction");
            emit_stop(builder, state, last.pc, *code);
        }
        BlockTerminator::UncondDirect { target } => {
            // Last instruction was a direct branch (br/bra/brsl).
            // For brsl, write the link register before jumping.
            if let Some(li) = last {
                if let SpuInstKind::BranchDirectLink { rt, .. } = li.kind {
                    let link = li.pc.wrapping_add(4) & SPU_LS_MASK;
                    emit_broadcast_imm(builder, state, rt as usize, link);
                }
            }
            let target_block = *block_map.get(target).ok_or_else(|| JitError::Backend(
                format!("target 0x{target:x} not in block_map (pre-flight should have caught)")))?;
            builder.ins().jump(target_block, &[]);
        }
        BlockTerminator::CondDirect { taken, fall_through } => {
            // Last instruction was brnz / brz.
            let li = last.expect("CondDirect block must have an instruction");
            let (rt, is_brnz) = match li.kind {
                SpuInstKind::BranchCond { rt, .. } => {
                    let p9 = (li.raw >> 23) & 0x1FF;
                    (rt, p9 == 0x042)
                }
                _ => return Err(JitError::Backend("CondDirect not produced by BranchCond".into())),
            };
            let lane0 = builder.ins().load(
                I32, MemFlags::trusted(), state,
                gpr_lane_offset(rt as usize, 0) as i32,
            );
            let zero = builder.ins().iconst(I32, 0);
            let cond = builder.ins().icmp(
                if is_brnz { IntCC::NotEqual } else { IntCC::Equal },
                lane0, zero,
            );
            let taken_block = *block_map.get(taken).ok_or_else(|| JitError::Backend(
                format!("brnz/brz taken target 0x{taken:x} not in map")))?;
            let fall_block = *block_map.get(fall_through).ok_or_else(|| JitError::Backend(
                format!("brnz/brz fall_through 0x{fall_through:x} not in map")))?;
            builder.ins().brif(cond, taken_block, &[], fall_block, &[]);
        }
        BlockTerminator::UncondIndirect => {
            // bi / iret / bisl. For BISL, write the link register first.
            let li = last.expect("UncondIndirect block must have an instruction");
            let ra_idx = match li.kind {
                SpuInstKind::BranchIndirect { ra } => ra,
                SpuInstKind::BranchIndirectLink { rt, ra } => {
                    let link = li.pc.wrapping_add(4) & SPU_LS_MASK_PC;
                    emit_broadcast_imm(builder, state, rt as usize, link);
                    ra
                }
                _ => return Err(JitError::Backend(
                    "UncondIndirect terminator from unexpected instruction kind".into(),
                )),
            };
            emit_indirect_continue(builder, state, ra_idx);
        }
        BlockTerminator::CondIndirect { fall_through } => {
            // biz / binz / bihz / bihnz: branch indirect if rt's preferred
            // slot satisfies a condition (== 0 / != 0 / low half == 0 / != 0).
            let li = last.expect("CondIndirect block must have an instruction");
            let (rt_idx, ra_idx, p11) = match li.kind {
                SpuInstKind::BranchIndirectCond { rt, ra } => (rt, ra, li.raw >> 21),
                _ => return Err(JitError::Backend(
                    "CondIndirect terminator from unexpected instruction kind".into(),
                )),
            };
            let lane0 = builder.ins().load(
                I32, MemFlags::trusted(), state,
                gpr_lane_offset(rt_idx as usize, 0) as i32,
            );
            // For BIHZ/BIHNZ, mask to low halfword first.
            let cond_val = if matches!(p11, 0x12A | 0x12B) {
                let mask = builder.ins().iconst(I32, 0xFFFF);
                builder.ins().band(lane0, mask)
            } else {
                lane0
            };
            let zero = builder.ins().iconst(I32, 0);
            let cc = match p11 {
                0x128 | 0x12A => IntCC::Equal,    // biz / bihz
                0x129 | 0x12B => IntCC::NotEqual, // binz / bihnz
                _ => return Err(JitError::Backend(format!(
                    "CondIndirect with unexpected p11=0x{p11:x}"
                ))),
            };
            let cond = builder.ins().icmp(cc, cond_val, zero);

            // Two-way branch: take_block (indirect target) vs fall_block (direct).
            let take_block = builder.create_block();
            let fall_block_cl = *block_map.get(fall_through).ok_or_else(||
                JitError::Backend(format!(
                    "CondIndirect fall_through 0x{fall_through:x} not in block_map"
                ))
            )?;
            builder.ins().brif(cond, take_block, &[], fall_block_cl, &[]);

            // Emit the take_block: load ra preferred, write to pc, return CONTINUE_TO.
            builder.switch_to_block(take_block);
            let state_in_take = builder.use_var(state_var);
            emit_indirect_continue(builder, state_in_take, ra_idx);
        }
        BlockTerminator::UnknownOpcode { .. }
        | BlockTerminator::FellThroughLimit { .. } => {
            // These slipped past pre-flight (shouldn't happen in practice
            // because check_block_supported rejects them up front). Surface
            // as CONTINUE_TO so the dispatcher can fall back cleanly.
            emit_bailout(builder, state, block.end_pc);
        }
    }

    Ok(())
}

/// Emit "load ra preferred slot, mask, store to state.pc, return CONTINUE_TO".
fn emit_indirect_continue(
    builder: &mut FunctionBuilder,
    state: Value,
    ra_idx: u8,
) {
    let target = builder.ins().load(
        I32, MemFlags::trusted(), state,
        gpr_lane_offset(ra_idx as usize, 0) as i32,
    );
    let mask = builder.ins().iconst(I32, SPU_LS_MASK_PC as i64);
    let aligned = builder.ins().band(target, mask);
    let pc_off = std::mem::offset_of!(JitState, pc) as i32;
    builder.ins().store(MemFlags::trusted(), aligned, state, pc_off);
    let v_continue = builder.ins().iconst(I32, JIT_OUTCOME_CONTINUE_TO as i64);
    builder.ins().return_(&[v_continue]);
}

fn check_block_supported(pc: u32, term: &BlockTerminator) -> Result<(), JitError> {
    match term {
        // R4a: indirect branches now generate a CONTINUE_TO outcome
        // that the dispatcher resolves. They no longer reject the
        // function from compiling.
        BlockTerminator::UncondIndirect => Ok(()),
        BlockTerminator::CondIndirect { .. } => Ok(()),
        BlockTerminator::UnknownOpcode { .. } => Err(JitError::Unsupported {
            pc, raw: 0,
            reason: "block contains an unrecognised opcode".into(),
        }),
        BlockTerminator::FellThroughLimit { .. } => Err(JitError::Unsupported {
            pc, raw: 0,
            reason: "block ran off the end of LS without a terminator".into(),
        }),
        _ => Ok(()),
    }
}

fn is_block_terminator(kind: SpuInstKind) -> bool {
    matches!(
        kind,
        SpuInstKind::Stop { .. }
        | SpuInstKind::BranchDirect { .. }
        | SpuInstKind::BranchDirectLink { .. }
        | SpuInstKind::BranchCond { .. }
        | SpuInstKind::BranchIndirect { .. }
        | SpuInstKind::BranchIndirectLink { .. }
        | SpuInstKind::BranchIndirectCond { .. }
    )
}

// =====================================================================
// Per-instruction codegen + supported_check
// =====================================================================

fn supported_check(kind: SpuInstKind, pc: u32, raw: u32) -> Result<(), JitError> {
    match kind {
        SpuInstKind::Stop { .. } => Ok(()),
        SpuInstKind::Nop => Ok(()),
        SpuInstKind::LoadImm { .. } => {
            // il (0x081), ila (7-bit 0x21), ilh (0x083), ilhu (0x082), iohl (0x0C1)
            let p9 = (raw >> 23) & 0x1FF;
            let p7 = (raw >> 25) & 0x7F;
            if p9 == 0x081 || p9 == 0x082 || p9 == 0x083 || p9 == 0x0C1 || p7 == 0x21 {
                Ok(())
            } else {
                Err(JitError::Unsupported {
                    pc, raw,
                    reason: format!("LoadImm variant p9=0x{p9:x} p7=0x{p7:x} not codegen'd"),
                })
            }
        }
        SpuInstKind::AluRr { .. } => {
            // ALU word: a/sf/and/or/xor/nor + nand/eqv/andc/orc.
            // Halfword: ah (0xC8), sfh (0x48).
            // Word compares: ceq (0x3C0), cgt (0x240), clgt (0x2C0).
            // Word shifts: shl (0x05B), rot (0x058), rotm (0x059), rotma (0x05A).
            // Carry/borrow generate: cg (0x0C2), bg (0x042).
            // Float arith: fa (0x2C4), fs (0x2C5), fm (0x2C6 with denormal flush).
            // Halfword compares: ceqh (0x3C8), cgth (0x248), clgth (0x2C8).
            // Multiplies: mpy (0x3C4 signed 16×16), mpyu (0x3CC unsigned 16×16).
            // Float compares: fcgt (0x2C2), fcmgt (0x2CA), fceq (0x3C2), fcmeq (0x3CA).
            // Byte compares: ceqb (0x3D0), cgtb (0x250), clgtb (0x2D0).
            // Halfword RR shifts: shlh (0x05F), roth (0x05C), rothm (0x05D), rotmah (0x05E).
            let p11 = raw >> 21;
            if matches!(p11,
                0x0C0 | 0x040 | 0x0C1 | 0x041 | 0x241 | 0x049
                | 0x0C9 | 0x249 | 0x2C1 | 0x2C9
                | 0x0C8 | 0x048
                | 0x3C0 | 0x240 | 0x2C0
                | 0x05B | 0x058 | 0x059 | 0x05A
                | 0x0C2 | 0x042
                | 0x2C4 | 0x2C5 | 0x2C6
                | 0x3C8 | 0x248 | 0x2C8
                | 0x3C4 | 0x3CC
                | 0x2C2 | 0x2CA | 0x3C2 | 0x3CA
                | 0x3D0 | 0x250 | 0x2D0
                | 0x05F | 0x05C | 0x05D | 0x05E
                | 0x3C5 | 0x3C6 | 0x3C7
            ) {
                Ok(())
            } else {
                Err(JitError::Unsupported {
                    pc, raw,
                    reason: format!("AluRr p11=0x{p11:03x} not codegen'd"),
                })
            }
        }
        SpuInstKind::Unary { .. } => {
            // ORX (0x1F0), CLZ (0x2A5), XSWD (0x2A6), XSHW (0x2AE),
            // CNTB (0x2B4), XSBH (0x2B6), FSM (0x1B4),
            // FREST (0x1B8), FRSQEST (0x1B9).
            let p11 = raw >> 21;
            if matches!(p11,
                0x1F0 | 0x2A5 | 0x2A6 | 0x2AE | 0x2B4 | 0x2B6
                | 0x1B4 | 0x1B8 | 0x1B9
            ) {
                Ok(())
            } else {
                Err(JitError::Unsupported {
                    pc, raw,
                    reason: format!("Unary p11=0x{p11:03x} not codegen'd"),
                })
            }
        }
        SpuInstKind::AluImm { .. } => {
            // Word RI10: ai (0x1C), andi (0x14), ori (0x04), xori (0x44),
            //   ceqi (0x7C), cgti (0x4C), clgti (0x5C),
            //   mpyi (0x74), mpyui (0x75),
            //   sfi (0x0C).
            // Halfword RI10: ahi (0x1D), ceqhi (0x7D), cgthi (0x4D), clgthi (0x5D).
            // Byte RI8: orbi (0x06), andbi (0x16), xorbi (0x46),
            //   cgtbi (0x4E), clgtbi (0x5E), ceqbi (0x7E).
            let p8 = raw >> 24;
            if matches!(p8,
                0x1C | 0x14 | 0x04 | 0x44 | 0x7C | 0x4C | 0x5C
                | 0x74 | 0x75
                | 0x7D | 0x4D | 0x5D
                | 0x1D | 0x0C
                | 0x06 | 0x16 | 0x46 | 0x4E | 0x5E | 0x7E
            ) {
                Ok(())
            } else {
                Err(JitError::Unsupported {
                    pc, raw,
                    reason: format!("AluImm p8=0x{p8:02x} not codegen'd"),
                })
            }
        }
        SpuInstKind::AluImm7 { .. } => {
            // Word shift imm: shli (0x07B), roti (0x078), rotmi (0x079), rotmai (0x07A).
            // Halfword shift imm: shlhi (0x07F), rothi (0x07C), rothmi (0x07D), rotmahi (0x07E).
            // Quadword byte shift imm: rotqbyi (0x1FC), shlqbyi (0x1FF).
            let p11 = raw >> 21;
            if matches!(p11,
                0x07B | 0x078 | 0x079 | 0x07A
                | 0x07F | 0x07C | 0x07D | 0x07E
                | 0x1FC | 0x1FF
            ) {
                Ok(())
            } else {
                Err(JitError::Unsupported {
                    pc, raw,
                    reason: format!("AluImm7 p11=0x{p11:03x} not codegen'd"),
                })
            }
        }
        SpuInstKind::Convert { .. } => {
            // cflts (0x1D8), cfltu (0x1D9), csflt (0x1DA), cuflt (0x1DB).
            // Decoder gives this from the 10-bit primary.
            let p10 = raw >> 22;
            if matches!(p10, 0x1D8 | 0x1D9 | 0x1DA | 0x1DB) {
                Ok(())
            } else {
                Err(JitError::Unsupported {
                    pc, raw,
                    reason: format!("Convert p10=0x{p10:x} not codegen'd"),
                })
            }
        }
        SpuInstKind::LoadStoreDForm { .. } => Ok(()),
        SpuInstKind::LoadStoreIndexed { .. } => Ok(()),
        SpuInstKind::Rrr { .. } => {
            // RRR-form: 4-bit primary.
            //   selb (0x8): bit-select
            //   shufb (0xB): byte permutation
            //   fma (0xE), fnms (0xD), fms (0xF): float multiply-add family
            let p4 = raw >> 28;
            if matches!(p4, 0x8 | 0xB | 0xE | 0xD | 0xF) {
                Ok(())
            } else {
                Err(JitError::Unsupported {
                    pc, raw,
                    reason: format!("Rrr p4=0x{p4:x} not codegen'd"),
                })
            }
        }
        SpuInstKind::BranchDirect { .. }
        | SpuInstKind::BranchCond { .. }
        | SpuInstKind::BranchDirectLink { .. } => Ok(()),
        SpuInstKind::BranchHint => Ok(()),  // hbr/hbra/hbrr — NOP in interpreter
        // Indirect branches are handled by the block-terminator emitter;
        // they don't need an emit_inst arm because they're always the
        // last instruction in their block.
        SpuInstKind::BranchIndirect { .. }
        | SpuInstKind::BranchIndirectLink { .. }
        | SpuInstKind::BranchIndirectCond { .. } => Ok(()),
        _ => Err(JitError::Unsupported {
            pc, raw,
            reason: format!("kind {kind:?} not yet codegen'd"),
        }),
    }
}

fn emit_inst(
    builder: &mut FunctionBuilder,
    state: Value,
    kind: SpuInstKind,
    pc: u32,
    raw: u32,
) -> Result<(), JitError> {
    match kind {
        SpuInstKind::Nop => Ok(()),
        SpuInstKind::BranchHint => Ok(()), // NOP — recompiler-only prefetch hint
        SpuInstKind::Stop { .. }
        | SpuInstKind::BranchDirect { .. }
        | SpuInstKind::BranchCond { .. }
        | SpuInstKind::BranchDirectLink { .. } => {
            // These are handled by the block terminator emitter; the
            // inst loop should have skipped them.
            Err(JitError::Backend(
                "emit_inst called on a terminator instruction".into(),
            ))
        }
        SpuInstKind::LoadImm { rt } => {
            emit_load_imm(builder, state, rt as usize, raw);
            Ok(())
        }
        SpuInstKind::AluRr { rt, ra, rb } => {
            let p11 = raw >> 21;
            // Halfword variants are handled separately.
            if p11 == 0x0C8 {
                emit_halfword_arith(builder, state, rt, ra, rb, HalfArith::Add);
                return Ok(());
            }
            if p11 == 0x048 {
                emit_halfword_arith(builder, state, rt, ra, rb, HalfArith::SubFrom);
                return Ok(());
            }
            // Float arith.
            if matches!(p11, 0x2C4 | 0x2C5 | 0x2C6) {
                let op = match p11 {
                    0x2C4 => FloatOp::Add,
                    0x2C5 => FloatOp::Sub,
                    0x2C6 => FloatOp::MulFlushed,
                    _ => unreachable!(),
                };
                emit_float_op(builder, state, rt, ra, rb, op);
                return Ok(());
            }
            // Halfword compares.
            if matches!(p11, 0x3C8 | 0x248 | 0x2C8) {
                let op = match p11 {
                    0x3C8 => HalfCmp::Eq,
                    0x248 => HalfCmp::GtSigned,
                    0x2C8 => HalfCmp::GtUnsigned,
                    _ => unreachable!(),
                };
                emit_halfword_cmp(builder, state, rt, ra, rb, op);
                return Ok(());
            }
            // Multiplies (16×16 → 32 per word lane).
            if matches!(p11, 0x3C4 | 0x3CC) {
                let signed = p11 == 0x3C4;
                emit_word_mpy(builder, state, rt, ra, rb, signed);
                return Ok(());
            }
            // Extended multiplies: mpyh/mpyhh/mpys.
            if matches!(p11, 0x3C5 | 0x3C6 | 0x3C7) {
                emit_extended_mpy(builder, state, rt, ra, rb, p11);
                return Ok(());
            }
            // Float compares.
            if matches!(p11, 0x2C2 | 0x2CA | 0x3C2 | 0x3CA) {
                let op = match p11 {
                    0x2C2 => FloatCmp::Gt,
                    0x2CA => FloatCmp::MagGt,
                    0x3C2 => FloatCmp::Eq,
                    0x3CA => FloatCmp::MagEq,
                    _ => unreachable!(),
                };
                emit_float_cmp(builder, state, rt, ra, rb, op);
                return Ok(());
            }
            // Byte compares (16 lanes per word, 4 word lanes = 16 bytes).
            if matches!(p11, 0x3D0 | 0x250 | 0x2D0) {
                let op = match p11 {
                    0x3D0 => ByteCmp::Eq,
                    0x250 => ByteCmp::GtSigned,
                    0x2D0 => ByteCmp::GtUnsigned,
                    _ => unreachable!(),
                };
                emit_byte_cmp(builder, state, rt, ra, rb, op);
                return Ok(());
            }
            // Halfword RR shifts (count from rb's halfword lane).
            if matches!(p11, 0x05F | 0x05C | 0x05D | 0x05E) {
                let op = match p11 {
                    0x05F => HalfShift::Shl,
                    0x05C => HalfShift::Rot,
                    0x05D => HalfShift::Shr,
                    0x05E => HalfShift::Sar,
                    _ => unreachable!(),
                };
                emit_halfword_rr_shift(builder, state, rt, ra, rb, op);
                return Ok(());
            }
            let op = match p11 {
                0x0C0 => WordOp::Add,
                0x040 => WordOp::SubFrom,
                0x0C1 => WordOp::And,
                0x041 => WordOp::Or,
                0x241 => WordOp::Xor,
                0x049 => WordOp::Nor,
                0x0C9 => WordOp::Nand,
                0x249 => WordOp::Eqv,
                0x2C1 => WordOp::AndC,
                0x2C9 => WordOp::OrC,
                0x3C0 => WordOp::CmpEq,
                0x240 => WordOp::CmpGtSigned,
                0x2C0 => WordOp::CmpGtUnsigned,
                0x05B => WordOp::Shl,
                0x058 => WordOp::Rot,
                0x059 => WordOp::Shr,
                0x05A => WordOp::Sar,
                0x0C2 => WordOp::CarryGen,
                0x042 => WordOp::BorrowGen,
                _ => unreachable!("supported_check passed unhandled AluRr"),
            };
            emit_word_op(builder, state, rt, ra, rb, op);
            Ok(())
        }
        SpuInstKind::Unary { rt, ra } => {
            let p11 = raw >> 21;
            emit_unary(builder, state, rt, ra, p11);
            Ok(())
        }
        SpuInstKind::LoadStoreDForm { rt, ra, offset, is_store } => {
            // lqd/stqd: addr = (ra preferred + offset*16) & 0x3FFF0
            emit_qword_dform(builder, state, rt, ra, offset, is_store);
            Ok(())
        }
        SpuInstKind::LoadStoreIndexed { rt, ra, rb, is_store } => {
            // lqx/stqx: addr = (ra preferred + rb preferred) & 0x3FFF0
            emit_qword_indexed(builder, state, rt, ra, rb, is_store);
            Ok(())
        }
        SpuInstKind::Rrr { rt, ra, rb, rc } => {
            let p4 = raw >> 28;
            match p4 {
                0x8 => {
                    // selb: bit-wise (rc & rb) | (!rc & ra) per lane.
                    for lane in 0..4 {
                        let av = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, lane) as i32);
                        let bv = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(rb as usize, lane) as i32);
                        let cv = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(rc as usize, lane) as i32);
                        let nc = builder.ins().bnot(cv);
                        let a_part = builder.ins().band(nc, av);
                        let b_part = builder.ins().band(cv, bv);
                        let result = builder.ins().bor(a_part, b_part);
                        builder.ins().store(MemFlags::trusted(), result, state, gpr_lane_offset(rt as usize, lane) as i32);
                    }
                }
                0xB => {
                    emit_shufb(builder, state, rt, ra, rb, rc);
                }
                0xE | 0xD | 0xF => {
                    let op = match p4 {
                        0xE => FmaOp::Add,    // fma: ra*rb + rc
                        0xD => FmaOp::NegSub, // fnms: rc - ra*rb
                        0xF => FmaOp::Sub,    // fms: ra*rb - rc
                        _ => unreachable!(),
                    };
                    emit_float_fma(builder, state, rt, ra, rb, rc, op);
                }
                _ => unreachable!("supported_check passed unhandled Rrr"),
            }
            Ok(())
        }
        SpuInstKind::AluImm7 { rt, ra, imm7 } => {
            let p11 = raw >> 21;
            let count = imm7 as u32;
            match p11 {
                // Word shift immediates
                0x07B => emit_word_const_shift(builder, state, rt, ra, count & 0x3F, ConstShift::Shl),
                0x078 => emit_word_const_shift(builder, state, rt, ra, count & 0x1F, ConstShift::Rot),
                0x079 => {
                    let n = (0u32.wrapping_sub(count)) & 0x3F;
                    emit_word_const_shift(builder, state, rt, ra, n, ConstShift::Shr);
                }
                0x07A => {
                    let n = (0u32.wrapping_sub(count)) & 0x3F;
                    emit_word_const_shift(builder, state, rt, ra, n, ConstShift::Sar);
                }
                // Halfword shift immediates (counts mask different — see interpreter:
                // shlhi mask 0x1F, rothi mask 0xF, rothmi/rotmahi mask 0x1F).
                0x07F => emit_halfword_const_shift(builder, state, rt, ra, count & 0x1F, ConstShift::Shl),
                0x07C => emit_halfword_const_shift(builder, state, rt, ra, count & 0xF, ConstShift::Rot),
                0x07D => {
                    let n = (0u32.wrapping_sub(count)) & 0x1F;
                    emit_halfword_const_shift(builder, state, rt, ra, n, ConstShift::Shr);
                }
                0x07E => {
                    let n = (0u32.wrapping_sub(count)) & 0x1F;
                    emit_halfword_const_shift(builder, state, rt, ra, n, ConstShift::Sar);
                }
                // Quadword byte rotate immediate: rotqbyi shifts the entire
                // 128-bit register by N bytes (mod 16).
                0x1FC => emit_quadword_byte_rotate(builder, state, rt, ra, count, false),
                // Quadword byte shift left immediate (zero-fill).
                0x1FF => emit_quadword_byte_rotate(builder, state, rt, ra, count, true),
                _ => unreachable!("supported_check passed unhandled AluImm7"),
            };
            Ok(())
        }
        SpuInstKind::Convert { rt, ra, scale } => {
            // cflts/cfltu/csflt/cuflt — float↔int conversions with scale factor.
            let p10 = raw >> 22;
            emit_convert(builder, state, rt, ra, scale, p10);
            Ok(())
        }
        SpuInstKind::AluImm { rt, ra, imm10 } => {
            let p8 = raw >> 24;
            let imm32 = imm10 as i32;
            // Halfword immediate compares are handled separately (different output shape).
            if matches!(p8, 0x7D | 0x4D | 0x5D) {
                let op = match p8 {
                    0x7D => HalfImmCmp::Eq,
                    0x4D => HalfImmCmp::GtSigned,
                    0x5D => HalfImmCmp::GtUnsigned,
                    _ => unreachable!(),
                };
                emit_halfword_imm_cmp(builder, state, rt, ra, imm10 as i16, op);
                return Ok(());
            }
            // Multiplies immediate: 16×16 → 32 per word lane, with sext imm to 16 bits.
            if matches!(p8, 0x74 | 0x75) {
                let signed = p8 == 0x74;
                emit_word_mpyi(builder, state, rt, ra, imm32, signed);
                return Ok(());
            }
            // Halfword arith immediate: ahi (0x1D), sfi (0x0C). The interpreter
            // doesn't currently expose these in its match, but the decoder
            // returns AluImm — we map them via the standard pattern.
            // ahi: per-halfword add with imm broadcast.
            // sfi: per-word imm - ra.
            if p8 == 0x1D {
                emit_halfword_imm_add(builder, state, rt, ra, imm10 as i16);
                return Ok(());
            }
            if p8 == 0x0C {
                // sfi rt, ra, imm10: rt = sext(imm10) - ra per word lane.
                let v_imm = builder.ins().iconst(I32, imm32 as i64);
                for lane in 0..4 {
                    let av = builder.ins().load(I32, MemFlags::trusted(), state,
                        gpr_lane_offset(ra as usize, lane) as i32);
                    let result = builder.ins().isub(v_imm, av);
                    builder.ins().store(MemFlags::trusted(), result, state,
                        gpr_lane_offset(rt as usize, lane) as i32);
                }
                return Ok(());
            }
            // Byte-immediate ops: imm10 already holds the sign-extended
            // imm8. Broadcast it across 4 bytes within each word lane.
            if matches!(p8, 0x06 | 0x16 | 0x46 | 0x4E | 0x5E | 0x7E) {
                let imm8 = imm10 as i8 as u32 & 0xFF;
                let broadcast = imm8 | (imm8 << 8) | (imm8 << 16) | (imm8 << 24);
                let op = match p8 {
                    0x06 => ByteImmOp::Or,
                    0x16 => ByteImmOp::And,
                    0x46 => ByteImmOp::Xor,
                    0x7E => ByteImmOp::CmpEq,
                    0x4E => ByteImmOp::CmpGtSigned,
                    0x5E => ByteImmOp::CmpGtUnsigned,
                    _ => unreachable!(),
                };
                emit_byte_imm(builder, state, rt, ra, broadcast, imm8 as u8, op);
                return Ok(());
            }
            let op = match p8 {
                0x1C => ImmOp::Add,
                0x14 => ImmOp::And,
                0x04 => ImmOp::Or,
                0x44 => ImmOp::Xor,
                0x7C => ImmOp::CmpEq,
                0x4C => ImmOp::CmpGtSigned,
                0x5C => ImmOp::CmpGtUnsigned,
                _ => unreachable!("supported_check passed unhandled AluImm"),
            };
            emit_word_imm(builder, state, rt, ra, imm32, op);
            Ok(())
        }
        _ => Err(JitError::Unsupported {
            pc, raw,
            reason: "emit_inst hit kind not classified by supported_check".into(),
        }),
    }
}

fn emit_stop(builder: &mut FunctionBuilder, state: Value, pc: u32, code: u32) {
    let pc_off = std::mem::offset_of!(JitState, pc) as i32;
    let stop_off = std::mem::offset_of!(JitState, stop_code) as i32;
    let v_pc = builder.ins().iconst(I32, pc as i64);
    builder.ins().store(MemFlags::trusted(), v_pc, state, pc_off);
    let v_code = builder.ins().iconst(I32, code as i64);
    builder.ins().store(MemFlags::trusted(), v_code, state, stop_off);
    let v_stop = builder.ins().iconst(I32, JIT_OUTCOME_STOP as i64);
    builder.ins().return_(&[v_stop]);
}

fn emit_bailout(builder: &mut FunctionBuilder, state: Value, resume_pc: u32) {
    // Now an UNKNOWN_OPCODE outcome — the dispatcher will fall back to
    // the interpreter for the rest of execution. Used only for
    // unsupported terminators (UnknownOpcode / FellThroughLimit).
    let pc_off = std::mem::offset_of!(JitState, pc) as i32;
    let v_pc = builder.ins().iconst(I32, resume_pc as i64);
    builder.ins().store(MemFlags::trusted(), v_pc, state, pc_off);
    let v_bailout = builder.ins().iconst(I32, JIT_OUTCOME_UNKNOWN_OPCODE as i64);
    builder.ins().return_(&[v_bailout]);
}

fn emit_load_imm(builder: &mut FunctionBuilder, state: Value, rt: usize, raw: u32) {
    let p9 = (raw >> 23) & 0x1FF;
    let p7 = (raw >> 25) & 0x7F;
    if p7 == 0x21 {
        // ila rt, imm18 — 18-bit unsigned broadcast.
        let imm = (raw >> 7) & 0x3FFFF;
        emit_broadcast_imm(builder, state, rt, imm);
    } else if p9 == 0x081 {
        // il rt, imm16 — sign-extended 16-bit broadcast.
        let imm16 = (raw >> 7) & 0xFFFF;
        let imm = (imm16 as i16) as i32 as u32;
        emit_broadcast_imm(builder, state, rt, imm);
    } else if p9 == 0x083 {
        // ilh rt, imm16 — broadcast 16-bit pattern within each 32-bit lane
        // (each word lane = imm16 << 16 | imm16).
        let h = (raw >> 7) & 0xFFFF;
        let pattern = (h << 16) | h;
        emit_broadcast_imm(builder, state, rt, pattern);
    } else if p9 == 0x082 {
        // ilhu rt, imm16 — upper-half immediate (low 16 zero).
        let imm = ((raw >> 7) & 0xFFFF) << 16;
        emit_broadcast_imm(builder, state, rt, imm);
    } else if p9 == 0x0C1 {
        // iohl rt, imm16 — OR low half into existing rt; upper preserved.
        let imm = (raw >> 7) & 0xFFFF;
        let v_imm = builder.ins().iconst(I32, imm as i64);
        for lane in 0..4 {
            let off = gpr_lane_offset(rt, lane) as i32;
            let cur = builder.ins().load(I32, MemFlags::trusted(), state, off);
            let result = builder.ins().bor(cur, v_imm);
            builder.ins().store(MemFlags::trusted(), result, state, off);
        }
    }
}

fn emit_broadcast_imm(builder: &mut FunctionBuilder, state: Value, rt: usize, imm: u32) {
    let v = builder.ins().iconst(I32, imm as i64);
    for lane in 0..4 {
        builder.ins().store(
            MemFlags::trusted(), v, state,
            gpr_lane_offset(rt, lane) as i32,
        );
    }
}

#[derive(Clone, Copy)]
enum WordOp {
    Add, SubFrom, And, Or, Xor, Nor, Nand, Eqv, AndC, OrC,
    CmpEq, CmpGtSigned, CmpGtUnsigned,
    Shl, Shr, Sar, Rot,
    CarryGen, BorrowGen,
}

fn emit_word_op(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    op: WordOp,
) {
    for lane in 0..4 {
        let av = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32,
        );
        let bv = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32,
        );
        let result = match op {
            WordOp::Add => builder.ins().iadd(av, bv),
            WordOp::SubFrom => builder.ins().isub(bv, av),
            WordOp::And => builder.ins().band(av, bv),
            WordOp::Or  => builder.ins().bor(av, bv),
            WordOp::Xor => builder.ins().bxor(av, bv),
            WordOp::Nor => { let or = builder.ins().bor(av, bv); builder.ins().bnot(or) }
            WordOp::Nand => { let an = builder.ins().band(av, bv); builder.ins().bnot(an) }
            WordOp::Eqv => { let xor = builder.ins().bxor(av, bv); builder.ins().bnot(xor) }
            WordOp::AndC => { let nb = builder.ins().bnot(bv); builder.ins().band(av, nb) }
            WordOp::OrC => { let nb = builder.ins().bnot(bv); builder.ins().bor(av, nb) }
            WordOp::CmpEq => emit_cmp_to_mask(builder, IntCC::Equal, av, bv),
            WordOp::CmpGtSigned => emit_cmp_to_mask(builder, IntCC::SignedGreaterThan, av, bv),
            WordOp::CmpGtUnsigned => emit_cmp_to_mask(builder, IntCC::UnsignedGreaterThan, av, bv),
            // SPU shift count comes from rb's lane, masked to 6 bits;
            // shifts of 32+ produce 0 (logical) or sign-fill (arith).
            WordOp::Shl => emit_dyn_shift_word(builder, av, bv, ShiftKind::Shl),
            WordOp::Shr => emit_dyn_shift_word(builder, av, bv, ShiftKind::Shr),
            WordOp::Sar => emit_dyn_shift_word(builder, av, bv, ShiftKind::Sar),
            // ROT: 5-bit count, no truncation issue.
            WordOp::Rot => {
                let mask = builder.ins().iconst(I32, 0x1F);
                let n = builder.ins().band(bv, mask);
                builder.ins().rotl(av, n)
            }
            // CG: carry generate per word — 1 if (ra+rb) overflows u32, else 0.
            WordOp::CarryGen => {
                let av64 = builder.ins().uextend(I64, av);
                let bv64 = builder.ins().uextend(I64, bv);
                let sum = builder.ins().iadd(av64, bv64);
                let shr = builder.ins().ushr_imm(sum, 32);
                builder.ins().ireduce(I32, shr)
            }
            // BG: borrow generate — 1 if ra <= rb (no underflow on rb-ra).
            WordOp::BorrowGen => {
                let cmp = builder.ins().icmp(IntCC::UnsignedLessThanOrEqual, av, bv);
                let one = builder.ins().iconst(I32, 1);
                let zero = builder.ins().iconst(I32, 0);
                builder.ins().select(cmp, one, zero)
            }
        };
        builder.ins().store(
            MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32,
        );
    }
}

fn emit_cmp_to_mask(builder: &mut FunctionBuilder, cc: IntCC, av: Value, bv: Value) -> Value {
    let cmp = builder.ins().icmp(cc, av, bv);
    let all_ones = builder.ins().iconst(I32, -1i64);
    let zero = builder.ins().iconst(I32, 0);
    builder.ins().select(cmp, all_ones, zero)
}

#[derive(Clone, Copy)]
enum ShiftKind { Shl, Shr, Sar }

/// Dynamic-count word shift with SPU semantics: count masked to 6 bits;
/// shifts of 32+ produce 0 / sign-fill instead of x86's "mod 32" UB.
/// Implemented via select(count >= 32, saturate, shift_value).
fn emit_dyn_shift_word(
    builder: &mut FunctionBuilder,
    av: Value,
    bv: Value,
    kind: ShiftKind,
) -> Value {
    // Compute the actual shift amount based on the variant.
    // Shr/Sar use SPU's "negative count" trick: actual = (-count) & 0x3F.
    let count = match kind {
        ShiftKind::Shl => {
            let mask = builder.ins().iconst(I32, 0x3F);
            builder.ins().band(bv, mask)
        }
        ShiftKind::Shr | ShiftKind::Sar => {
            let zero = builder.ins().iconst(I32, 0);
            let neg = builder.ins().isub(zero, bv);
            let mask = builder.ins().iconst(I32, 0x3F);
            builder.ins().band(neg, mask)
        }
    };
    let thirty_two = builder.ins().iconst(I32, 32);
    let cap_pred = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, count, thirty_two);

    // "Normal" shift result (only valid when count < 32).
    let normal = match kind {
        ShiftKind::Shl => builder.ins().ishl(av, count),
        ShiftKind::Shr => builder.ins().ushr(av, count),
        ShiftKind::Sar => builder.ins().sshr(av, count),
    };

    // Saturated result for count >= 32.
    let saturated = match kind {
        ShiftKind::Shl | ShiftKind::Shr => builder.ins().iconst(I32, 0),
        ShiftKind::Sar => {
            // Sign-fill: ((av as i32) >> 31) gives 0 or 0xFFFFFFFF.
            builder.ins().sshr_imm(av, 31)
        }
    };

    builder.ins().select(cap_pred, saturated, normal)
}

#[derive(Clone, Copy)]
enum ConstShift { Shl, Shr, Sar, Rot }

/// Constant-count word shift. Caller has already canonicalised `n`
/// (e.g. shli passes n=imm7 & 0x3F; rotmi passes n=(-imm7) & 0x3F;
/// roti passes n=imm7 & 0x1F).
fn emit_word_const_shift(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    n: u32,
    kind: ConstShift,
) {
    for lane in 0..4 {
        let av = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32,
        );
        let result = match kind {
            ConstShift::Rot => builder.ins().rotl_imm(av, (n & 0x1F) as i64),
            ConstShift::Shl => {
                if n >= 32 { builder.ins().iconst(I32, 0) }
                else { builder.ins().ishl_imm(av, n as i64) }
            }
            ConstShift::Shr => {
                if n >= 32 { builder.ins().iconst(I32, 0) }
                else { builder.ins().ushr_imm(av, n as i64) }
            }
            ConstShift::Sar => {
                if n >= 32 { builder.ins().sshr_imm(av, 31) }
                else { builder.ins().sshr_imm(av, n as i64) }
            }
        };
        builder.ins().store(
            MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32,
        );
    }
}

#[derive(Clone, Copy)]
enum ImmOp { Add, And, Or, Xor, CmpEq, CmpGtSigned, CmpGtUnsigned }

#[derive(Clone, Copy)]
enum ByteImmOp { Or, And, Xor, CmpEq, CmpGtSigned, CmpGtUnsigned }

/// Byte-immediate ops: per-byte logical/compare with broadcast imm.
/// `broadcast` is the imm8 replicated across 4 bytes in a u32.
/// `imm_byte` is the raw byte value (used for per-byte compares).
fn emit_byte_imm(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    broadcast: u32,
    imm_byte: u8,
    op: ByteImmOp,
) {
    use cranelift::codegen::ir::types::I8;
    let v_bcast = builder.ins().iconst(I32, broadcast as i64);

    match op {
        ByteImmOp::And | ByteImmOp::Or | ByteImmOp::Xor => {
            // Logical: per-word lane, just apply with broadcast.
            for lane in 0..4 {
                let av = builder.ins().load(I32, MemFlags::trusted(), state,
                    gpr_lane_offset(ra as usize, lane) as i32);
                let r = match op {
                    ByteImmOp::And => builder.ins().band(av, v_bcast),
                    ByteImmOp::Or  => builder.ins().bor(av, v_bcast),
                    ByteImmOp::Xor => builder.ins().bxor(av, v_bcast),
                    _ => unreachable!(),
                };
                builder.ins().store(MemFlags::trusted(), r, state,
                    gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        ByteImmOp::CmpEq | ByteImmOp::CmpGtSigned | ByteImmOp::CmpGtUnsigned => {
            // Per-byte compare: extract each byte, compare against
            // the imm_byte (signed for CmpGtSigned, unsigned else),
            // pack 0xFF/0 results.
            let mask8 = builder.ins().iconst(I32, 0xFF);
            let true8 = builder.ins().iconst(I32, 0xFF);
            let zero = builder.ins().iconst(I32, 0);
            let imm_signed = (imm_byte as i8) as i32;
            let imm_signed_v = builder.ins().iconst(I32, imm_signed as i64);
            let imm_unsigned_v = builder.ins().iconst(I32, imm_byte as i64);

            for lane in 0..4 {
                let av = builder.ins().load(I32, MemFlags::trusted(), state,
                    gpr_lane_offset(ra as usize, lane) as i32);
                let mut packed = zero;
                for byte_idx in 0..4 {
                    let shift = (byte_idx * 8) as i64;
                    let av_b_u = if byte_idx == 0 { builder.ins().band(av, mask8) }
                                 else { let s = builder.ins().ushr_imm(av, shift); builder.ins().band(s, mask8) };
                    let (a_op, imm_op_val) = match op {
                        ByteImmOp::CmpGtSigned => {
                            let a8 = builder.ins().ireduce(I8, av_b_u);
                            let a_s = builder.ins().sextend(I32, a8);
                            (a_s, imm_signed_v)
                        }
                        _ => (av_b_u, imm_unsigned_v),
                    };
                    let cc = match op {
                        ByteImmOp::CmpEq => IntCC::Equal,
                        ByteImmOp::CmpGtSigned => IntCC::SignedGreaterThan,
                        ByteImmOp::CmpGtUnsigned => IntCC::UnsignedGreaterThan,
                        _ => unreachable!(),
                    };
                    let cmp = builder.ins().icmp(cc, a_op, imm_op_val);
                    let mask_byte = builder.ins().select(cmp, true8, zero);
                    let shifted = if byte_idx == 0 { mask_byte }
                                  else { builder.ins().ishl_imm(mask_byte, shift) };
                    packed = builder.ins().bor(packed, shifted);
                }
                builder.ins().store(MemFlags::trusted(), packed, state,
                    gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum HalfImmCmp { Eq, GtSigned, GtUnsigned }

/// `ceqhi` / `cgthi` / `clgthi`: per-halfword compare with broadcast
/// imm10 (sign-extended to halfword).
fn emit_halfword_imm_cmp(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    imm10: i16,
    op: HalfImmCmp,
) {
    use cranelift::codegen::ir::types::I16;
    let mask16 = builder.ins().iconst(I32, 0xFFFF);
    let all_ones16 = builder.ins().iconst(I32, 0xFFFF);
    let zero = builder.ins().iconst(I32, 0);

    // imm sign-extended to a halfword value, then to i32 for comparisons.
    let imm_signed = imm10 as i32;
    let imm_unsigned = (imm10 as i16 as u16) as i32;

    for lane in 0..4 {
        let av = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let av_lo_u = builder.ins().band(av, mask16);
        let av_hi_u = builder.ins().ushr_imm(av, 16);
        let (av_lo_op, av_hi_op, imm_val) = match op {
            HalfImmCmp::GtSigned => {
                let alo16 = builder.ins().ireduce(I16, av_lo_u);
                let alo = builder.ins().sextend(I32, alo16);
                let ahi16 = builder.ins().ireduce(I16, av_hi_u);
                let ahi = builder.ins().sextend(I32, ahi16);
                (alo, ahi, builder.ins().iconst(I32, imm_signed as i64))
            }
            _ => (av_lo_u, av_hi_u, builder.ins().iconst(I32, imm_unsigned as i64)),
        };
        let cc = match op {
            HalfImmCmp::Eq => IntCC::Equal,
            HalfImmCmp::GtSigned => IntCC::SignedGreaterThan,
            HalfImmCmp::GtUnsigned => IntCC::UnsignedGreaterThan,
        };
        let cmp_lo = builder.ins().icmp(cc, av_lo_op, imm_val);
        let cmp_hi = builder.ins().icmp(cc, av_hi_op, imm_val);
        let mask_lo = builder.ins().select(cmp_lo, all_ones16, zero);
        let mask_hi = builder.ins().select(cmp_hi, all_ones16, zero);
        let hi_shifted = builder.ins().ishl_imm(mask_hi, 16);
        let result = builder.ins().bor(hi_shifted, mask_lo);
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

/// `mpyi` / `mpyui`: 16×16 → 32 per word lane, where the second
/// operand is a sign-extended (or zero-extended) imm10 truncated to 16 bits.
fn emit_word_mpyi(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    imm10: i32,
    signed: bool,
) {
    use cranelift::codegen::ir::types::I16;
    let imm_val = if signed {
        // Truncate sign-extended imm10 to 16 bits, sign-extend back to i32.
        ((imm10 as i16) as i32) as i64
    } else {
        // Mask to 16 bits.
        ((imm10 as u32) & 0xFFFF) as i64
    };
    let v_imm = builder.ins().iconst(I32, imm_val);
    for lane in 0..4 {
        let av = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let av_op = if signed {
            let a16 = builder.ins().ireduce(I16, av);
            builder.ins().sextend(I32, a16)
        } else {
            let mask = builder.ins().iconst(I32, 0xFFFF);
            builder.ins().band(av, mask)
        };
        let result = builder.ins().imul(av_op, v_imm);
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

/// `ahi rt, ra, imm10`: per-halfword add with broadcast imm10.
fn emit_halfword_imm_add(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    imm10: i16,
) {
    let mask16 = builder.ins().iconst(I32, 0xFFFF);
    let v_imm = builder.ins().iconst(I32, ((imm10 as i32) & 0xFFFF) as i64);
    for lane in 0..4 {
        let av = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let av_lo = builder.ins().band(av, mask16);
        let av_hi = builder.ins().ushr_imm(av, 16);
        let sum_lo = builder.ins().iadd(av_lo, v_imm);
        let sum_hi = builder.ins().iadd(av_hi, v_imm);
        let lo_masked = builder.ins().band(sum_lo, mask16);
        let hi_masked = builder.ins().band(sum_hi, mask16);
        let hi_shifted = builder.ins().ishl_imm(hi_masked, 16);
        let result = builder.ins().bor(hi_shifted, lo_masked);
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

fn emit_word_imm(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    imm: i32,
    op: ImmOp,
) {
    let v_imm = builder.ins().iconst(I32, imm as i64);
    for lane in 0..4 {
        let av = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32,
        );
        let result = match op {
            ImmOp::Add => builder.ins().iadd(av, v_imm),
            ImmOp::And => builder.ins().band(av, v_imm),
            ImmOp::Or  => builder.ins().bor(av, v_imm),
            ImmOp::Xor => builder.ins().bxor(av, v_imm),
            ImmOp::CmpEq => {
                let cmp = builder.ins().icmp(IntCC::Equal, av, v_imm);
                let all_ones = builder.ins().iconst(I32, -1i64);
                let zero = builder.ins().iconst(I32, 0);
                builder.ins().select(cmp, all_ones, zero)
            }
            ImmOp::CmpGtSigned => {
                let cmp = builder.ins().icmp(IntCC::SignedGreaterThan, av, v_imm);
                let all_ones = builder.ins().iconst(I32, -1i64);
                let zero = builder.ins().iconst(I32, 0);
                builder.ins().select(cmp, all_ones, zero)
            }
            ImmOp::CmpGtUnsigned => {
                let cmp = builder.ins().icmp(IntCC::UnsignedGreaterThan, av, v_imm);
                let all_ones = builder.ins().iconst(I32, -1i64);
                let zero = builder.ins().iconst(I32, 0);
                builder.ins().select(cmp, all_ones, zero)
            }
        };
        builder.ins().store(
            MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32,
        );
    }
}

/// Per-halfword constant-count shift on 8 lanes of u16.
/// Caller has already canonicalised `n` to the SPU mask (5 bits for
/// shl/shr/sar, 4 bits for rot).
fn emit_halfword_const_shift(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    n: u32,
    kind: ConstShift,
) {
    let mask16 = builder.ins().iconst(I32, 0xFFFF);
    for lane in 0..4 {
        let av = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32,
        );
        let av_lo_u = builder.ins().band(av, mask16);
        let av_hi_u = builder.ins().ushr_imm(av, 16);
        // For arith shr, sign-extend the halfword value to i32 first.
        let (lo_op, hi_op) = match kind {
            ConstShift::Sar => {
                use cranelift::codegen::ir::types::I16;
                let lo_red = builder.ins().ireduce(I16, av_lo_u);
                let av_lo_s = builder.ins().sextend(I32, lo_red);
                let hi_red = builder.ins().ireduce(I16, av_hi_u);
                let av_hi_s = builder.ins().sextend(I32, hi_red);
                (av_lo_s, av_hi_s)
            }
            _ => (av_lo_u, av_hi_u),
        };
        let lo_result = halfword_const_shift_one(builder, lo_op, n, kind);
        let hi_result = halfword_const_shift_one(builder, hi_op, n, kind);
        let lo_masked = builder.ins().band(lo_result, mask16);
        let hi_masked = builder.ins().band(hi_result, mask16);
        let hi_shifted = builder.ins().ishl_imm(hi_masked, 16);
        let result = builder.ins().bor(hi_shifted, lo_masked);
        builder.ins().store(
            MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32,
        );
    }
}

fn halfword_const_shift_one(
    builder: &mut FunctionBuilder,
    av: Value,
    n: u32,
    kind: ConstShift,
) -> Value {
    match kind {
        ConstShift::Rot => {
            // Halfword rotate: simulate via shl + shr|, both within 16 bits.
            // We're operating in i32 space — rotate via (av << n) | (av >> (16-n))
            // both masked to 16 bits.
            let n = n & 0xF;
            if n == 0 { return av; }
            let lo = builder.ins().ishl_imm(av, n as i64);
            let hi = builder.ins().ushr_imm(av, (16 - n) as i64);
            let combined = builder.ins().bor(lo, hi);
            // Mask happens in the caller.
            combined
        }
        ConstShift::Shl => {
            if n >= 16 { builder.ins().iconst(I32, 0) }
            else { builder.ins().ishl_imm(av, n as i64) }
        }
        ConstShift::Shr => {
            if n >= 16 { builder.ins().iconst(I32, 0) }
            else { builder.ins().ushr_imm(av, n as i64) }
        }
        ConstShift::Sar => {
            // av is already sign-extended to i32 by the caller.
            if n >= 16 { builder.ins().sshr_imm(av, 15) }
            else { builder.ins().sshr_imm(av, n as i64) }
        }
    }
}

/// `rotqbyi rt, ra, imm7`: rotate the entire 128-bit register left by
/// `imm7 & 0xF` bytes. `is_shift=true` zero-fills (shlqbyi semantics)
/// instead of rotating.
///
/// Implementation: handle byte counts by interpreting the 16 bytes
/// in BE order (i.e. lane 0 bytes 0..3 are the highest-significance
/// bytes). For lane-aligned shifts (multiples of 4) we just permute
/// the 4 lanes; for non-aligned shifts we'd need bit-level work
/// across lanes. For now, only lane-aligned counts are emitted; other
/// counts bail to the "non-zero-shifted" path with combined shr/shl
/// across 32-bit lanes.
fn emit_quadword_byte_rotate(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    imm7: u32,
    is_shift: bool,
) {
    let n = (imm7 & 0xF) as usize; // byte count 0..15

    // Pre-load all 4 lanes of ra.
    let lane0 = builder.ins().load(I32, MemFlags::trusted(), state,
        gpr_lane_offset(ra as usize, 0) as i32);
    let lane1 = builder.ins().load(I32, MemFlags::trusted(), state,
        gpr_lane_offset(ra as usize, 1) as i32);
    let lane2 = builder.ins().load(I32, MemFlags::trusted(), state,
        gpr_lane_offset(ra as usize, 2) as i32);
    let lane3 = builder.ins().load(I32, MemFlags::trusted(), state,
        gpr_lane_offset(ra as usize, 3) as i32);
    let lanes = [lane0, lane1, lane2, lane3];
    // SPU rotate quadword left by N bytes:
    // BE byte 0 of result = BE byte (N) of source.
    // Output BE byte i = source BE byte (i + N) mod 16 (rotate)
    //                  = 0 if (i + N) >= 16 (shift).
    //
    // For each output 32-bit lane (4 bytes), compose by gathering
    // bytes from one or two source lanes.

    let zero = builder.ins().iconst(I32, 0);
    let mut out = [zero; 4];

    // For each output BE byte i (0..15), determine source BE byte (i + N).
    // Then output[i / 4] |= (source byte) << ((3 - i % 4) * 8).
    // BE byte i within lane L=i/4 is at LE bit position ((3 - i%4) * 8).
    for out_byte in 0..16 {
        let src_byte = out_byte + n;
        let src_lane = src_byte / 4;
        let src_pos_be = src_byte % 4;     // 0=high, 3=low in BE within lane
        let src_shift = (3 - src_pos_be) * 8; // LE bit position
        let dst_lane = out_byte / 4;
        let dst_pos_be = out_byte % 4;
        let dst_shift = (3 - dst_pos_be) * 8;

        if is_shift && src_byte >= 16 {
            // Zero fill — nothing to add.
            continue;
        }
        let src_lane_idx = src_lane % 4;
        // Extract that byte from source lane.
        let src = lanes[src_lane_idx];
        let mask = builder.ins().iconst(I32, 0xFF);
        let extracted = if src_shift == 0 {
            builder.ins().band(src, mask)
        } else {
            let s = builder.ins().ushr_imm(src, src_shift as i64);
            builder.ins().band(s, mask)
        };
        let placed = if dst_shift == 0 {
            extracted
        } else {
            builder.ins().ishl_imm(extracted, dst_shift as i64)
        };
        out[dst_lane] = builder.ins().bor(out[dst_lane], placed);
    }

    for lane in 0..4 {
        builder.ins().store(MemFlags::trusted(), out[lane], state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

/// `cflts/cfltu/csflt/cuflt rt, ra, scale`: per-lane float↔int
/// conversion with 2^exp_bias scaling. The interpreter uses naive
/// scaled conversion with saturation; we mirror that path in IR.
///
/// `p10` = 0x1D8 (cflts: f32→i32 signed),
///        0x1D9 (cfltu: f32→u32),
///        0x1DA (csflt: i32→f32 signed),
///        0x1DB (cuflt: u32→f32).
fn emit_convert(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    scale: u8,
    p10: u32,
) {
    use cranelift::codegen::ir::types::F32;
    // Compute scale factor at compile time (constant per call).
    let scale_i = scale as i32;

    for lane in 0..4 {
        let av_i = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let result = match p10 {
            0x1D8 => {
                // cflts: f32 → i32 (signed), scaled by 2^(173 - scale).
                // Implementation: f * 2^exp_bias, then saturating cast.
                // For simplicity, use Cranelift fcvt_to_sint_sat which
                // handles NaN→0 and inf→i32::MAX/MIN.
                let f = builder.ins().bitcast(F32, MemFlags::new(), av_i);
                let exp_bias = 173 - scale_i;
                let scaled = scale_float(builder, f, exp_bias);
                builder.ins().fcvt_to_sint_sat(I32, scaled)
            }
            0x1D9 => {
                // cfltu: f32 → u32 (unsigned), scaled by 2^(173 - scale).
                let f = builder.ins().bitcast(F32, MemFlags::new(), av_i);
                let exp_bias = 173 - scale_i;
                let scaled = scale_float(builder, f, exp_bias);
                builder.ins().fcvt_to_uint_sat(I32, scaled)
            }
            0x1DA => {
                // csflt: i32 → f32 (signed), scaled by 2^(scale - 155).
                let f = builder.ins().fcvt_from_sint(F32, av_i);
                let exp_bias = scale_i - 155;
                let scaled = scale_float(builder, f, exp_bias);
                builder.ins().bitcast(I32, MemFlags::new(), scaled)
            }
            0x1DB => {
                // cuflt: u32 → f32 (unsigned), scaled by 2^(scale - 155).
                let f = builder.ins().fcvt_from_uint(F32, av_i);
                let exp_bias = scale_i - 155;
                let scaled = scale_float(builder, f, exp_bias);
                builder.ins().bitcast(I32, MemFlags::new(), scaled)
            }
            _ => unreachable!("supported_check rejected this convert"),
        };
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

/// Multiply a float Value by 2^exp_bias. Cranelift doesn't have a
/// `ldexp` intrinsic, so we materialise 2.0^exp_bias as a constant
/// f32 and multiply. For exp_bias outside f32 normal range, we fall
/// through to standard saturation behaviour.
fn scale_float(
    builder: &mut FunctionBuilder,
    f: Value,
    exp_bias: i32,
) -> Value {
    if exp_bias == 0 {
        return f;
    }
    let factor: f32 = 2f32.powi(exp_bias);
    // f32::powi may produce 0.0 / inf for extreme exp_bias; that
    // matches the interpreter's `2f32.powi(...)` and downstream ops.
    let factor_const = builder.ins().f32const(factor);
    builder.ins().fmul(f, factor_const)
}

/// `shufb rt, ra, rb, rc`: per-byte permutation.
/// For each output byte `i ∈ 0..16`, examine selector byte `rc[i]`:
///   - `(sel & 0xC0) == 0x80` → output 0x00
///   - `(sel & 0xE0) == 0xC0` → output 0xFF
///   - `(sel & 0xE0) == 0xE0` → output 0x80
///   - else: idx = `sel & 0x1F`, output `(idx < 16) ? ra[idx] : rb[idx-16]`
///
/// SPU bytes are big-endian within the 128-bit register; our LE u32
/// lane storage maps BE byte `i` to LSB-0 byte offset `(i & ~3) | (3 - (i & 3))`,
/// which simplifies to `i ^ 3`.
fn emit_shufb(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8, rc: u8,
) {
    use cranelift::codegen::ir::types::{I8, I64};

    let gpr_base = std::mem::offset_of!(JitState, gpr_lanes) as i64;
    let ra_offset = gpr_base + (ra as i64) * 16;
    let rb_offset = gpr_base + (rb as i64) * 16;
    let rt_offset = gpr_base + (rt as i64) * 16;
    let rc_offset = gpr_base + (rc as i64) * 16;

    let zero8 = builder.ins().iconst(I8, 0);
    let ff8 = builder.ins().iconst(I8, 0xFF);
    let _80_8 = builder.ins().iconst(I8, 0x80u32 as i64 as u8 as i64);
    let mask_c0 = builder.ins().iconst(I8, 0xC0u32 as i64 as u8 as i64);
    let mask_e0 = builder.ins().iconst(I8, 0xE0u32 as i64 as u8 as i64);
    let val_80 = builder.ins().iconst(I8, 0x80u32 as i64 as u8 as i64);
    let val_c0 = builder.ins().iconst(I8, 0xC0u32 as i64 as u8 as i64);
    let val_e0 = builder.ins().iconst(I8, 0xE0u32 as i64 as u8 as i64);
    let _ = _80_8;

    // Cast state pointer to i64 for byte-address arithmetic.
    let state64 = state;  // already i64

    for out_byte in 0..16i32 {
        // BE byte i within a GPR's u32-lane storage is at LE offset (i ^ 3).
        let out_byte_offset = (out_byte ^ 3) as i64;
        let rc_byte_offset = rc_offset + out_byte_offset;

        // Load selector byte.
        let sel = builder.ins().load(I8, MemFlags::trusted(), state64, rc_byte_offset as i32);

        // Pattern checks.
        let masked_c0 = builder.ins().band(sel, mask_c0);
        let is_zero = builder.ins().icmp(IntCC::Equal, masked_c0, val_80);
        let masked_e0 = builder.ins().band(sel, mask_e0);
        let is_ff = builder.ins().icmp(IntCC::Equal, masked_e0, val_c0);
        let is_80 = builder.ins().icmp(IntCC::Equal, masked_e0, val_e0);

        // Compute source byte from ra/rb based on idx = sel & 0x1F.
        let mask_1f = builder.ins().iconst(I8, 0x1F);
        let idx = builder.ins().band(sel, mask_1f);
        let sixteen = builder.ins().iconst(I8, 16);
        let use_rb = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, idx, sixteen);
        // src_idx_in_reg = idx & 0xF (i.e., idx mod 16, since idx < 32)
        let mask_f = builder.ins().iconst(I8, 0xF);
        let src_idx_in_reg = builder.ins().band(idx, mask_f);
        // src_byte_offset = src_idx_in_reg ^ 3 (BE→LE mapping)
        let three = builder.ins().iconst(I8, 3);
        let src_byte_off_local = builder.ins().bxor(src_idx_in_reg, three);
        let src_byte_off64 = builder.ins().uextend(I64, src_byte_off_local);

        // Compute base = state + (use_rb ? rb_offset : ra_offset).
        let ra_base = builder.ins().iadd_imm(state64, ra_offset);
        let rb_base = builder.ins().iadd_imm(state64, rb_offset);
        let chosen_base = builder.ins().select(use_rb, rb_base, ra_base);
        let src_addr = builder.ins().iadd(chosen_base, src_byte_off64);
        let src_byte = builder.ins().load(I8, MemFlags::trusted(), src_addr, 0);

        // Apply pattern selection (precedence: zero > ff > 80 > src).
        let after_80 = builder.ins().select(is_80, val_80, src_byte);
        let after_ff = builder.ins().select(is_ff, ff8, after_80);
        let final_byte = builder.ins().select(is_zero, zero8, after_ff);

        // Store to rt's output byte position.
        let rt_byte_addr = (rt_offset + out_byte_offset) as i32;
        builder.ins().store(MemFlags::trusted(), final_byte, state64, rt_byte_addr);
    }
}

#[derive(Clone, Copy)]
enum FmaOp { Add, Sub, NegSub }

/// `fma`/`fnms`/`fms` per-lane f32 multiply-add. Interpreter does
/// non-fused mul-then-add (with intermediate rounding), so we mirror
/// that via Cranelift `fmul` + `fadd`/`fsub` instead of the native
/// `fma` intrinsic. No FTZ flush — interpreter doesn't apply it for
/// these ops.
///
/// - `Add` (fma):    rt = ra * rb + rc
/// - `Sub` (fms):    rt = ra * rb - rc
/// - `NegSub` (fnms): rt = rc - ra * rb
fn emit_float_fma(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8, rc: u8,
    op: FmaOp,
) {
    use cranelift::codegen::ir::types::F32;
    for lane in 0..4 {
        let av_i = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let bv_i = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32);
        let cv_i = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(rc as usize, lane) as i32);
        let af = builder.ins().bitcast(F32, MemFlags::new(), av_i);
        let bf = builder.ins().bitcast(F32, MemFlags::new(), bv_i);
        let cf = builder.ins().bitcast(F32, MemFlags::new(), cv_i);
        let prod = builder.ins().fmul(af, bf);
        let rf = match op {
            FmaOp::Add => builder.ins().fadd(prod, cf),
            FmaOp::Sub => builder.ins().fsub(prod, cf),
            FmaOp::NegSub => builder.ins().fsub(cf, prod),
        };
        let ri = builder.ins().bitcast(I32, MemFlags::new(), rf);
        builder.ins().store(MemFlags::trusted(), ri, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

/// SPU unary RR ops: rt = f(ra).
fn emit_unary(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8,
    p11: u32,
) {
    use cranelift::codegen::ir::types::{F32, I8, I16};
    let zero = builder.ins().iconst(I32, 0);

    match p11 {
        // orx: OR-across word lanes into preferred slot
        0x1F0 => {
            let l0 = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, 0) as i32);
            let l1 = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, 1) as i32);
            let l2 = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, 2) as i32);
            let l3 = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, 3) as i32);
            let or01 = builder.ins().bor(l0, l1);
            let or23 = builder.ins().bor(l2, l3);
            let result = builder.ins().bor(or01, or23);
            builder.ins().store(MemFlags::trusted(), result, state, gpr_lane_offset(rt as usize, 0) as i32);
            for lane in 1..4 {
                builder.ins().store(MemFlags::trusted(), zero, state, gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        // clz: count leading zeros per word
        0x2A5 => {
            for lane in 0..4 {
                let av = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, lane) as i32);
                let result = builder.ins().clz(av);
                builder.ins().store(MemFlags::trusted(), result, state, gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        // cntb: per-byte popcount, result stored as byte count per byte position
        0x2B4 => {
            let mask8 = builder.ins().iconst(I32, 0xFF);
            for lane in 0..4 {
                let av = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, lane) as i32);
                let mut packed = zero;
                for byte_idx in 0..4 {
                    let shift = (byte_idx * 8) as i64;
                    let byte_val = if byte_idx == 0 { builder.ins().band(av, mask8) }
                                   else { let s = builder.ins().ushr_imm(av, shift); builder.ins().band(s, mask8) };
                    let popcnt = builder.ins().popcnt(byte_val);
                    let shifted = if byte_idx == 0 { popcnt }
                                  else { builder.ins().ishl_imm(popcnt, shift) };
                    packed = builder.ins().bor(packed, shifted);
                }
                builder.ins().store(MemFlags::trusted(), packed, state, gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        // xsbh: sign-extend bytes to halfwords (8 lanes of u16, each from a byte)
        0x2B6 => {
            let mask16 = builder.ins().iconst(I32, 0xFFFF);
            for lane in 0..4 {
                let av = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, lane) as i32);
                // Each halfword in the SPU result is sextend(low_byte_of_halfword).
                // The halfword's low byte is bits 0..7 (low half) and 16..23 (high half).
                // Take low 8 bits of low half, sextend i8→i32, mask 16 bits.
                let lo_byte = builder.ins().ireduce(I8, av);
                let lo_sext = builder.ins().sextend(I32, lo_byte);
                let lo_masked = builder.ins().band(lo_sext, mask16);
                // High half: extract bits 16..23.
                let hi_shifted = builder.ins().ushr_imm(av, 16);
                let hi_byte = builder.ins().ireduce(I8, hi_shifted);
                let hi_sext = builder.ins().sextend(I32, hi_byte);
                let hi_masked = builder.ins().band(hi_sext, mask16);
                let hi_final = builder.ins().ishl_imm(hi_masked, 16);
                let result = builder.ins().bor(hi_final, lo_masked);
                builder.ins().store(MemFlags::trusted(), result, state, gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        // xshw: sign-extend halfwords to words (4 word lanes from low halfword of each)
        0x2AE => {
            for lane in 0..4 {
                let av = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, lane) as i32);
                let h16 = builder.ins().ireduce(I16, av);
                let result = builder.ins().sextend(I32, h16);
                builder.ins().store(MemFlags::trusted(), result, state, gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        // xswd: sign-extend words to doublewords (2 dword pairs, each from low word of pair)
        0x2A6 => {
            // Layout: lanes [hi0, lo0, hi1, lo1] form 2 dwords {dword0=hi0:lo0, dword1=hi1:lo1}.
            // Sign-extend lo0 → dword0; lo1 → dword1. So:
            // rt[0] = sext(rt[1]) >> 32 = (rt[1] as i32) < 0 ? 0xFFFFFFFF : 0
            // rt[1] = ra[1]
            // rt[2] = (rt[3] as i32) < 0 ? 0xFFFFFFFF : 0
            // rt[3] = ra[3]
            for pair in 0..2 {
                let lo_lane = pair * 2 + 1;
                let hi_lane = pair * 2;
                let lo_val = builder.ins().load(I32, MemFlags::trusted(), state,
                    gpr_lane_offset(ra as usize, lo_lane) as i32);
                let hi_val = builder.ins().sshr_imm(lo_val, 31);
                builder.ins().store(MemFlags::trusted(), hi_val, state,
                    gpr_lane_offset(rt as usize, hi_lane) as i32);
                builder.ins().store(MemFlags::trusted(), lo_val, state,
                    gpr_lane_offset(rt as usize, lo_lane) as i32);
            }
        }
        // fsm: form select mask word.
        // Take low 4 bits of ra preferred slot. Bit i (i=0..3) → word lane (3-i):
        // bit 3 → lane 0, bit 2 → lane 1, bit 1 → lane 2, bit 0 → lane 3.
        0x1B4 => {
            let pref = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, 0) as i32);
            let all_ones = builder.ins().iconst(I32, -1i64);
            for lane in 0..4 {
                let bit = 3 - lane;  // lane 0 = bit 3, lane 3 = bit 0
                let bit_mask = builder.ins().iconst(I32, 1i64 << bit);
                let masked = builder.ins().band(pref, bit_mask);
                let cond = builder.ins().icmp(IntCC::NotEqual, masked, zero);
                let result = builder.ins().select(cond, all_ones, zero);
                builder.ins().store(MemFlags::trusted(), result, state, gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        // frest: naive 1/x with FTZ flush
        0x1B8 => {
            for lane in 0..4 {
                let av_i = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, lane) as i32);
                let flushed = emit_flush_denorm(builder, av_i);
                let f = builder.ins().bitcast(F32, MemFlags::new(), flushed);
                let one = builder.ins().f32const(1.0f32);
                let recip = builder.ins().fdiv(one, f);
                let recip_i = builder.ins().bitcast(I32, MemFlags::new(), recip);
                let result = emit_flush_denorm(builder, recip_i);
                builder.ins().store(MemFlags::trusted(), result, state, gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        // frsqest: naive 1/sqrt(|x|) with FTZ flush
        0x1B9 => {
            let abs_mask = builder.ins().iconst(I32, 0x7FFFFFFF);
            for lane in 0..4 {
                let av_i = builder.ins().load(I32, MemFlags::trusted(), state, gpr_lane_offset(ra as usize, lane) as i32);
                let abs_i = builder.ins().band(av_i, abs_mask);
                let flushed = emit_flush_denorm(builder, abs_i);
                let f = builder.ins().bitcast(F32, MemFlags::new(), flushed);
                let sqrt_f = builder.ins().sqrt(f);
                let one = builder.ins().f32const(1.0f32);
                let recip = builder.ins().fdiv(one, sqrt_f);
                let recip_i = builder.ins().bitcast(I32, MemFlags::new(), recip);
                let result = emit_flush_denorm(builder, recip_i);
                builder.ins().store(MemFlags::trusted(), result, state, gpr_lane_offset(rt as usize, lane) as i32);
            }
        }
        _ => unreachable!("supported_check passed unhandled Unary"),
    }
}

#[derive(Clone, Copy)]
enum FloatCmp { Gt, MagGt, Eq, MagEq }

/// Per-lane float compare with SPU FTZ semantics: flush denormals to
/// +0 before comparing, then IEEE compare via Cranelift `fcmp`.
/// Result is 0xFFFFFFFF (true) or 0 (false) per lane.
/// Magnitude variants (MagGt/MagEq) clear the sign bit before flush+compare.
fn emit_float_cmp(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    op: FloatCmp,
) {
    use cranelift::codegen::ir::condcodes::FloatCC;
    use cranelift::codegen::ir::types::F32;
    let abs_mask = builder.ins().iconst(I32, 0x7FFFFFFF);
    let all_ones = builder.ins().iconst(I32, -1i64);
    let zero = builder.ins().iconst(I32, 0);

    for lane in 0..4 {
        let av_raw = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let bv_raw = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32);
        // For magnitude variants, clear sign bit first.
        let (av_pre, bv_pre) = match op {
            FloatCmp::MagGt | FloatCmp::MagEq => (
                builder.ins().band(av_raw, abs_mask),
                builder.ins().band(bv_raw, abs_mask),
            ),
            _ => (av_raw, bv_raw),
        };
        let av_i = emit_flush_denorm(builder, av_pre);
        let bv_i = emit_flush_denorm(builder, bv_pre);
        let af = builder.ins().bitcast(F32, MemFlags::new(), av_i);
        let bf = builder.ins().bitcast(F32, MemFlags::new(), bv_i);
        let cc = match op {
            FloatCmp::Gt | FloatCmp::MagGt => FloatCC::GreaterThan,
            FloatCmp::Eq | FloatCmp::MagEq => FloatCC::Equal,
        };
        let cmp = builder.ins().fcmp(cc, af, bf);
        let result = builder.ins().select(cmp, all_ones, zero);
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

#[derive(Clone, Copy)]
enum ByteCmp { Eq, GtSigned, GtUnsigned }

/// Per-byte compare on 16 lanes. Output 0xFF (true) or 0 (false) per byte.
/// Pattern: extract 4 bytes per word lane via shifts/masks, compare, pack.
fn emit_byte_cmp(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    op: ByteCmp,
) {
    use cranelift::codegen::ir::types::I8;
    let mask8 = builder.ins().iconst(I32, 0xFF);
    let true8 = builder.ins().iconst(I32, 0xFF);
    let zero = builder.ins().iconst(I32, 0);

    for lane in 0..4 {
        let av = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let bv = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32);

        // Extract bytes 0..3 from each (byte 0 = bits 0..7).
        let mut packed = zero;
        for byte_idx in 0..4 {
            let shift = (byte_idx * 8) as i64;
            let av_b_u = if byte_idx == 0 { builder.ins().band(av, mask8) }
                         else { let s = builder.ins().ushr_imm(av, shift); builder.ins().band(s, mask8) };
            let bv_b_u = if byte_idx == 0 { builder.ins().band(bv, mask8) }
                         else { let s = builder.ins().ushr_imm(bv, shift); builder.ins().band(s, mask8) };
            let (av_op, bv_op) = match op {
                ByteCmp::GtSigned => {
                    let a8 = builder.ins().ireduce(I8, av_b_u);
                    let av_s = builder.ins().sextend(I32, a8);
                    let b8 = builder.ins().ireduce(I8, bv_b_u);
                    let bv_s = builder.ins().sextend(I32, b8);
                    (av_s, bv_s)
                }
                _ => (av_b_u, bv_b_u),
            };
            let cc = match op {
                ByteCmp::Eq => IntCC::Equal,
                ByteCmp::GtSigned => IntCC::SignedGreaterThan,
                ByteCmp::GtUnsigned => IntCC::UnsignedGreaterThan,
            };
            let cmp = builder.ins().icmp(cc, av_op, bv_op);
            let mask_byte = builder.ins().select(cmp, true8, zero);
            let shifted = if byte_idx == 0 { mask_byte }
                          else { builder.ins().ishl_imm(mask_byte, shift) };
            packed = builder.ins().bor(packed, shifted);
        }
        builder.ins().store(MemFlags::trusted(), packed, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

#[derive(Clone, Copy)]
enum HalfShift { Shl, Shr, Sar, Rot }

/// Per-halfword RR-form shift: count comes from each halfword lane of rb.
/// Each pair of (av_h, bv_h) is processed independently within the
/// 32-bit word lane.
fn emit_halfword_rr_shift(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    op: HalfShift,
) {
    use cranelift::codegen::ir::types::I16;
    let mask16 = builder.ins().iconst(I32, 0xFFFF);
    let zero = builder.ins().iconst(I32, 0);

    for lane in 0..4 {
        let av = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let bv = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32);

        let av_lo_u = builder.ins().band(av, mask16);
        let av_hi_u = builder.ins().ushr_imm(av, 16);
        let bv_lo = builder.ins().band(bv, mask16);
        let bv_hi = builder.ins().ushr_imm(bv, 16);

        // Compute count for each kind.
        let (count_lo, count_hi, av_lo_op, av_hi_op) = match op {
            HalfShift::Shl => {
                let m = builder.ins().iconst(I32, 0x1F);
                (builder.ins().band(bv_lo, m), builder.ins().band(bv_hi, m), av_lo_u, av_hi_u)
            }
            HalfShift::Shr => {
                let zr = builder.ins().iconst(I32, 0);
                let m = builder.ins().iconst(I32, 0x1F);
                let nlo = builder.ins().isub(zr, bv_lo);
                let nhi = builder.ins().isub(zr, bv_hi);
                (builder.ins().band(nlo, m), builder.ins().band(nhi, m), av_lo_u, av_hi_u)
            }
            HalfShift::Sar => {
                let zr = builder.ins().iconst(I32, 0);
                let m = builder.ins().iconst(I32, 0x1F);
                let nlo = builder.ins().isub(zr, bv_lo);
                let nhi = builder.ins().isub(zr, bv_hi);
                let alo16 = builder.ins().ireduce(I16, av_lo_u);
                let alo_s = builder.ins().sextend(I32, alo16);
                let ahi16 = builder.ins().ireduce(I16, av_hi_u);
                let ahi_s = builder.ins().sextend(I32, ahi16);
                (builder.ins().band(nlo, m), builder.ins().band(nhi, m), alo_s, ahi_s)
            }
            HalfShift::Rot => {
                let m = builder.ins().iconst(I32, 0xF);
                (builder.ins().band(bv_lo, m), builder.ins().band(bv_hi, m), av_lo_u, av_hi_u)
            }
        };

        // Apply shift per kind. For shl/shr/sar with count >= 16 → 0 (or sign-fill).
        let lo_result = halfword_dyn_shift_one(builder, av_lo_op, count_lo, op);
        let hi_result = halfword_dyn_shift_one(builder, av_hi_op, count_hi, op);
        // Mask to 16 bits.
        let lo_masked = builder.ins().band(lo_result, mask16);
        let hi_masked = builder.ins().band(hi_result, mask16);
        let hi_shifted = builder.ins().ishl_imm(hi_masked, 16);
        let result = builder.ins().bor(hi_shifted, lo_masked);
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
        let _ = zero;
    }
}

fn halfword_dyn_shift_one(
    builder: &mut FunctionBuilder,
    av: Value,
    count: Value,
    kind: HalfShift,
) -> Value {
    match kind {
        HalfShift::Rot => {
            // Halfword rotate: (av << count) | (av >> (16 - count)), only
            // works for count in 1..=15; for count == 0 we'd shift by 16
            // which is fine for our purposes. We'll AND mask later.
            let lo = builder.ins().ishl(av, count);
            let sixteen = builder.ins().iconst(I32, 16);
            let inv_count = builder.ins().isub(sixteen, count);
            let hi = builder.ins().ushr(av, inv_count);
            builder.ins().bor(lo, hi)
        }
        HalfShift::Shl => {
            let cap = builder.ins().iconst(I32, 16);
            let zero = builder.ins().iconst(I32, 0);
            let normal = builder.ins().ishl(av, count);
            let too_big = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, count, cap);
            builder.ins().select(too_big, zero, normal)
        }
        HalfShift::Shr => {
            let cap = builder.ins().iconst(I32, 16);
            let zero = builder.ins().iconst(I32, 0);
            let normal = builder.ins().ushr(av, count);
            let too_big = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, count, cap);
            builder.ins().select(too_big, zero, normal)
        }
        HalfShift::Sar => {
            let cap = builder.ins().iconst(I32, 16);
            // Saturate to sign bit when count >= 16: shift right by 15
            // (all sign bits).
            let sat = builder.ins().sshr_imm(av, 15);
            let normal = builder.ins().sshr(av, count);
            let too_big = builder.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, count, cap);
            builder.ins().select(too_big, sat, normal)
        }
    }
}

/// `mpyh rt, ra, rb`: rt[i] = (high_half_signed(ra) * low_half_signed(rb)) << 16.
/// `mpyhh rt, ra, rb`: rt[i] = high_half_signed(ra) * high_half_signed(rb).
/// `mpys rt, ra, rb`: rt[i] = sign_extend(low_half_signed(ra) * low_half_signed(rb), truncated to 16 bits).
fn emit_extended_mpy(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    p11: u32,
) {
    use cranelift::codegen::ir::types::I16;
    for lane in 0..4 {
        let av = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let bv = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32);

        // Extract signed high/low halves of each.
        let a_lo16 = builder.ins().ireduce(I16, av);
        let a_lo_s = builder.ins().sextend(I32, a_lo16);
        let a_hi_pre = builder.ins().ushr_imm(av, 16);
        let a_hi16 = builder.ins().ireduce(I16, a_hi_pre);
        let a_hi_s = builder.ins().sextend(I32, a_hi16);

        let b_lo16 = builder.ins().ireduce(I16, bv);
        let b_lo_s = builder.ins().sextend(I32, b_lo16);
        let b_hi_pre = builder.ins().ushr_imm(bv, 16);
        let b_hi16 = builder.ins().ireduce(I16, b_hi_pre);
        let b_hi_s = builder.ins().sextend(I32, b_hi16);

        let result = match p11 {
            0x3C5 => {
                // mpyh: a_hi * b_lo, then << 16.
                let p = builder.ins().imul(a_hi_s, b_lo_s);
                builder.ins().ishl_imm(p, 16)
            }
            0x3C6 => {
                // mpyhh: a_hi * b_hi.
                builder.ins().imul(a_hi_s, b_hi_s)
            }
            0x3C7 => {
                // mpys: (a_lo * b_lo) truncated to 16 bits, sign-extended.
                let p = builder.ins().imul(a_lo_s, b_lo_s);
                let p16 = builder.ins().ireduce(I16, p);
                builder.ins().sextend(I32, p16)
            }
            _ => unreachable!(),
        };
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

/// `mpy rt, ra, rb` (signed=true) or `mpyu rt, ra, rb` (signed=false):
/// per word lane, multiply the low 16 bits of `ra` with the low 16
/// bits of `rb`, store the 32-bit result. Sign of the multiply
/// depends on `signed`.
fn emit_word_mpy(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    signed: bool,
) {
    use cranelift::codegen::ir::types::I16;
    for lane in 0..4 {
        let av = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let bv = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32);
        let result = if signed {
            // Truncate to i16, sign-extend to i32, multiply.
            let a16 = builder.ins().ireduce(I16, av);
            let a32 = builder.ins().sextend(I32, a16);
            let b16 = builder.ins().ireduce(I16, bv);
            let b32 = builder.ins().sextend(I32, b16);
            builder.ins().imul(a32, b32)
        } else {
            // Mask to 16 bits (zero-extend), multiply.
            let mask = builder.ins().iconst(I32, 0xFFFF);
            let a16 = builder.ins().band(av, mask);
            let b16 = builder.ins().band(bv, mask);
            builder.ins().imul(a16, b16)
        };
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

#[derive(Clone, Copy)]
enum HalfCmp { Eq, GtSigned, GtUnsigned }

/// Per-halfword compare on 8 lanes of u16. Output is 0xFFFF (true) or
/// 0 (false) per halfword. Same split-mask-pack pattern as
/// `emit_halfword_arith`.
fn emit_halfword_cmp(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    op: HalfCmp,
) {
    use cranelift::codegen::ir::types::I16;
    let mask16 = builder.ins().iconst(I32, 0xFFFF);
    let all_ones16 = builder.ins().iconst(I32, 0xFFFF);
    let zero = builder.ins().iconst(I32, 0);

    for lane in 0..4 {
        let av = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32);
        let bv = builder.ins().load(I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32);

        let av_lo_u = builder.ins().band(av, mask16);
        let av_hi_u = builder.ins().ushr_imm(av, 16);
        let bv_lo_u = builder.ins().band(bv, mask16);
        let bv_hi_u = builder.ins().ushr_imm(bv, 16);

        // For signed compares, sign-extend 16→32; for unsigned/eq, use
        // the zero-extended values directly.
        let (av_lo_op, av_hi_op, bv_lo_op, bv_hi_op) = match op {
            HalfCmp::GtSigned => {
                let alo16 = builder.ins().ireduce(I16, av_lo_u);
                let alo = builder.ins().sextend(I32, alo16);
                let ahi16 = builder.ins().ireduce(I16, av_hi_u);
                let ahi = builder.ins().sextend(I32, ahi16);
                let blo16 = builder.ins().ireduce(I16, bv_lo_u);
                let blo = builder.ins().sextend(I32, blo16);
                let bhi16 = builder.ins().ireduce(I16, bv_hi_u);
                let bhi = builder.ins().sextend(I32, bhi16);
                (alo, ahi, blo, bhi)
            }
            _ => (av_lo_u, av_hi_u, bv_lo_u, bv_hi_u),
        };
        let cc = match op {
            HalfCmp::Eq => IntCC::Equal,
            HalfCmp::GtSigned => IntCC::SignedGreaterThan,
            HalfCmp::GtUnsigned => IntCC::UnsignedGreaterThan,
        };
        let cmp_lo = builder.ins().icmp(cc, av_lo_op, bv_lo_op);
        let cmp_hi = builder.ins().icmp(cc, av_hi_op, bv_hi_op);
        let mask_lo = builder.ins().select(cmp_lo, all_ones16, zero);
        let mask_hi = builder.ins().select(cmp_hi, all_ones16, zero);
        let hi_shifted = builder.ins().ishl_imm(mask_hi, 16);
        let result = builder.ins().bor(hi_shifted, mask_lo);
        builder.ins().store(MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32);
    }
}

#[derive(Clone, Copy)]
enum FloatOp { Add, Sub, MulFlushed }

/// Per-lane f32 arithmetic. `MulFlushed` matches the SPU `fm` semantic:
/// flush denormal inputs to +0, multiply, flush denormal result.
/// `Add`/`Sub` use raw IEEE add/sub (matching the interpreter `fa`/`fs`).
fn emit_float_op(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    op: FloatOp,
) {
    use cranelift::codegen::ir::types::F32;
    for lane in 0..4 {
        let av_i = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32,
        );
        let bv_i = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32,
        );
        let (af_i, bf_i) = match op {
            FloatOp::MulFlushed => (
                emit_flush_denorm(builder, av_i),
                emit_flush_denorm(builder, bv_i),
            ),
            _ => (av_i, bv_i),
        };
        let af = builder.ins().bitcast(F32, MemFlags::new(), af_i);
        let bf = builder.ins().bitcast(F32, MemFlags::new(), bf_i);
        let rf = match op {
            FloatOp::Add => builder.ins().fadd(af, bf),
            FloatOp::Sub => builder.ins().fsub(af, bf),
            FloatOp::MulFlushed => builder.ins().fmul(af, bf),
        };
        let mut ri = builder.ins().bitcast(I32, MemFlags::new(), rf);
        if matches!(op, FloatOp::MulFlushed) {
            ri = emit_flush_denorm(builder, ri);
        }
        builder.ins().store(
            MemFlags::trusted(), ri, state,
            gpr_lane_offset(rt as usize, lane) as i32,
        );
    }
}

/// SPU FTZ-style denormal flush: if `bits & 0x7F800000 == 0` (exponent
/// is zero, so the value is denormal or +/- 0), return 0; otherwise
/// pass through unchanged. Matches `flush_denorm_f32` in the interpreter.
fn emit_flush_denorm(builder: &mut FunctionBuilder, bits: Value) -> Value {
    let exp_mask = builder.ins().iconst(I32, 0x7F800000);
    let exp = builder.ins().band(bits, exp_mask);
    let zero = builder.ins().iconst(I32, 0);
    let is_denorm = builder.ins().icmp(IntCC::Equal, exp, zero);
    builder.ins().select(is_denorm, zero, bits)
}

#[derive(Clone, Copy)]
enum HalfArith { Add, SubFrom }

/// Per-halfword arithmetic on 8 lanes of u16. Each 32-bit GPR lane
/// contains 2 halfwords (high 16 + low 16). We process them in
/// parallel using bit masks: split → add/sub → re-pack.
///
/// `SubFrom` matches SPU `sfh rt, ra, rb` semantics: `rt = rb - ra`.
fn emit_halfword_arith(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8, ra: u8, rb: u8,
    op: HalfArith,
) {
    let mask = builder.ins().iconst(I32, 0xFFFF);
    for lane in 0..4 {
        let av = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(ra as usize, lane) as i32,
        );
        let bv = builder.ins().load(
            I32, MemFlags::trusted(), state,
            gpr_lane_offset(rb as usize, lane) as i32,
        );
        let av_lo = builder.ins().band(av, mask);
        let av_hi = builder.ins().ushr_imm(av, 16);
        let bv_lo = builder.ins().band(bv, mask);
        let bv_hi = builder.ins().ushr_imm(bv, 16);
        let (sum_lo, sum_hi) = match op {
            HalfArith::Add => (
                builder.ins().iadd(av_lo, bv_lo),
                builder.ins().iadd(av_hi, bv_hi),
            ),
            HalfArith::SubFrom => (
                builder.ins().isub(bv_lo, av_lo),
                builder.ins().isub(bv_hi, av_hi),
            ),
        };
        let lo_mask = builder.ins().band(sum_lo, mask);
        let hi_mask = builder.ins().band(sum_hi, mask);
        let hi_shifted = builder.ins().ishl_imm(hi_mask, 16);
        let result = builder.ins().bor(hi_shifted, lo_mask);
        builder.ins().store(
            MemFlags::trusted(), result, state,
            gpr_lane_offset(rt as usize, lane) as i32,
        );
    }
}

fn gpr_lane_offset(reg_idx: usize, lane: usize) -> usize {
    let base = std::mem::offset_of!(JitState, gpr_lanes);
    base + reg_idx * 16 + lane * 4
}

fn ls_ptr_offset() -> i32 {
    std::mem::offset_of!(JitState, ls_ptr) as i32
}

/// Compute the host address inside the LS for the SPU address `aligned_lsa`.
/// Returns a 64-bit pointer Value usable for load/store.
fn emit_ls_target(
    builder: &mut FunctionBuilder,
    state: Value,
    aligned_lsa: Value,
) -> Value {
    let ls_ptr = builder.ins().load(I64, MemFlags::trusted(), state, ls_ptr_offset());
    let lsa64 = builder.ins().uextend(I64, aligned_lsa);
    builder.ins().iadd(ls_ptr, lsa64)
}

/// `lqd rt, imm10*16(ra)` or `stqd rt, imm10*16(ra)`.
/// Address = (ra preferred-slot + imm10*16) & 0x3FFF0.
fn emit_qword_dform(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8,
    ra: u8,
    imm10: i16,
    is_store: bool,
) {
    let base = builder.ins().load(
        I32, MemFlags::trusted(), state,
        gpr_lane_offset(ra as usize, 0) as i32,
    );
    let offset_bytes = (imm10 as i32).wrapping_mul(16);
    let v_off = builder.ins().iconst(I32, offset_bytes as i64);
    let raw_addr = builder.ins().iadd(base, v_off);
    let mask = builder.ins().iconst(I32, 0x3FFF0);
    let aligned = builder.ins().band(raw_addr, mask);
    let target = emit_ls_target(builder, state, aligned);
    emit_qword_xfer(builder, state, target, rt, is_store);
}

/// `lqx rt, ra, rb` or `stqx rt, ra, rb`.
/// Address = (ra preferred + rb preferred) & 0x3FFF0.
fn emit_qword_indexed(
    builder: &mut FunctionBuilder,
    state: Value,
    rt: u8,
    ra: u8,
    rb: u8,
    is_store: bool,
) {
    let av = builder.ins().load(
        I32, MemFlags::trusted(), state,
        gpr_lane_offset(ra as usize, 0) as i32,
    );
    let bv = builder.ins().load(
        I32, MemFlags::trusted(), state,
        gpr_lane_offset(rb as usize, 0) as i32,
    );
    let raw_addr = builder.ins().iadd(av, bv);
    let mask = builder.ins().iconst(I32, 0x3FFF0);
    let aligned = builder.ins().band(raw_addr, mask);
    let target = emit_ls_target(builder, state, aligned);
    emit_qword_xfer(builder, state, target, rt, is_store);
}

/// Move 16 bytes between LS and rt's 4 lanes. SPU LS stores big-endian
/// dwords; the GPR lane representation in `JitState` is a u32 (host
/// little-endian), so each lane needs a `bswap` to/from the LS form.
fn emit_qword_xfer(
    builder: &mut FunctionBuilder,
    state: Value,
    target: Value,
    rt: u8,
    is_store: bool,
) {
    for lane in 0..4 {
        let lane_addr = builder.ins().iadd_imm(target, (lane * 4) as i64);
        if is_store {
            // Read lane from rt (host LE u32), bswap to BE, store at LS.
            let lane_val = builder.ins().load(
                I32, MemFlags::trusted(), state,
                gpr_lane_offset(rt as usize, lane) as i32,
            );
            let be = builder.ins().bswap(lane_val);
            builder.ins().store(MemFlags::trusted(), be, lane_addr, 0);
        } else {
            // Load BE u32 from LS, bswap to host LE, store into rt lane.
            let raw = builder.ins().load(I32, MemFlags::trusted(), lane_addr, 0);
            let lane_val = builder.ins().bswap(raw);
            builder.ins().store(
                MemFlags::trusted(), lane_val, state,
                gpr_lane_offset(rt as usize, lane) as i32,
            );
        }
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ls(entry: u32, insts: &[u32]) -> Vec<u8> {
        let mut ls = vec![0u8; 0x40000];
        for (i, w) in insts.iter().enumerate() {
            let off = entry as usize + i * 4;
            ls[off..off + 4].copy_from_slice(&w.to_be_bytes());
        }
        ls
    }

    fn il(rt: u32, imm: i16) -> u32 {
        ((0x081u32 & 0x1FF) << 23) | ((imm as u16 as u32 & 0xFFFF) << 7) | (rt & 0x7F)
    }
    fn ila(rt: u32, imm18: u32) -> u32 {
        (0x21u32 << 25) | ((imm18 & 0x3FFFF) << 7) | (rt & 0x7F)
    }
    fn ai(rt: u32, ra: u32, imm10: i16) -> u32 {
        (0x1Cu32 << 24) | ((imm10 as u32 & 0x3FF) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }
    fn ceqi(rt: u32, ra: u32, imm10: i16) -> u32 {
        (0x7Cu32 << 24) | ((imm10 as u32 & 0x3FF) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }
    fn a(rt: u32, ra: u32, rb: u32) -> u32 {
        (0x0C0u32 << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F)
    }
    fn br(imm16: i16) -> u32 {
        (0x064u32 << 23) | ((imm16 as u16 as u32 & 0xFFFF) << 7)
    }
    fn brnz(rt: u32, imm16: i16) -> u32 {
        (0x042u32 << 23) | ((imm16 as u16 as u32 & 0xFFFF) << 7) | (rt & 0x7F)
    }
    fn stop_op(code: u32) -> u32 { code & 0x3FFF }

    #[test]
    fn jit_state_round_trip_u128() {
        let mut s = JitState::new();
        s.store_gpr(7, 0xDEAD_BEEF_CAFE_F00D_1234_5678_9ABC_DEF0u128);
        assert_eq!(s.load_gpr(7), 0xDEAD_BEEF_CAFE_F00D_1234_5678_9ABC_DEF0u128);
    }

    #[test]
    fn jit_compiles_il_stop_program() {
        let ls = make_ls(0x100, &[il(3, 0x1234), stop_op(0x55)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.stop_code, 0x55);
        assert_eq!(state.load_gpr(3), 0x00001234_00001234_00001234_00001234);
    }

    #[test]
    fn jit_compiles_unconditional_branch_chain() {
        // 0x100: il r3, 1; br +2 → 0x10C
        // 0x108: stop 0xAA  (skipped)
        // 0x10C: il r3, 2; stop 0xBB
        let ls = make_ls(0x100, &[
            il(3, 1),
            br(2),
            stop_op(0xAA),
            0x4020_0000, // padding nop at 0x10C? No wait — br +2 = pc + 8 bytes = 0x10C
            il(3, 2),
            stop_op(0xBB),
        ]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.stop_code, 0xBB);
        // r3 = 2 (from the second il), broadcast.
        assert_eq!(state.load_gpr(3), 0x00000002_00000002_00000002_00000002);
    }

    #[test]
    fn jit_compiles_loop_summing_one_through_ten() {
        // Mirror of `synthetic_loop.elf` (sums 1+2+...+10 = 55 into r3).
        // Layout (PC offsets from entry 0x100):
        //   0x100 ila r3, 0
        //   0x104 ila r4, 1
        //   0x108 a r3, r3, r4    (loop top)
        //   0x10C ai r4, r4, 1
        //   0x110 ceqi r5, r4, 11
        //   0x114 brnz r5, +2     (→ 0x11C exit)
        //   0x118 br -4           (→ 0x108 back)
        //   0x11C stop 0x55
        let ls = make_ls(0x100, &[
            ila(3, 0),
            ila(4, 1),
            a(3, 3, 4),
            ai(4, 4, 1),
            ceqi(5, 4, 11),
            brnz(5, 2),
            br(-4),
            stop_op(0x55),
        ]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.stop_code, 0x55);
        // r3 = 1+2+...+10 = 55 = 0x37 broadcast.
        assert_eq!(state.load_gpr(3), 0x00000037_00000037_00000037_00000037);
    }

    #[test]
    fn jit_compiles_indirect_branch_and_returns_continue_to() {
        // R4a: indirect branches no longer reject the function.
        // The JIT compiles bi, runs, and returns CONTINUE_TO with
        // state.pc set to the indirect target (= ra preferred slot).
        let bi = (0x1A8u32 << 21) | (4u32 << 7);
        let ls = make_ls(0x100, &[bi, stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("R4a should compile bi");
        let mut state = JitState::new();
        // r4 preferred slot = 0x200 (will be the indirect target).
        state.gpr_lanes[4] = [0x200, 0, 0, 0];
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_CONTINUE_TO);
        assert_eq!(state.pc, 0x200);
    }

    #[test]
    fn jit_bails_on_unsupported_alu_opcode() {
        // DFA (double-precision add) = primary 0x2CC. Not codegen'd
        // by the JIT (double-precision family is rare in real code).
        let dfa = (0x2CCu32 << 21) | (0u32 << 14) | (0u32 << 7);
        let ls = make_ls(0x100, &[dfa, stop_op(0)]);
        let mut backend = JitBackend::new();
        let r = backend.compile_at(&ls, 0x100);
        assert!(matches!(r, Err(JitError::Unsupported { .. })));
    }

    #[test]
    fn jit_handles_nop_pass_through() {
        let nop = 0x4020_0000u32;
        let ls = make_ls(0x100, &[nop, nop, stop_op(0x99)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.stop_code, 0x99);
    }

    #[test]
    fn jit_compiles_brz_taken_path() {
        // Set r5 = 0, brz r5, +2 → branch taken, skip nop, hit stop 0x42.
        // Layout:
        //   0x100 il r5, 0
        //   0x104 brz r5, +2    → 0x10C
        //   0x108 stop 0xFF     (NOT executed)
        //   0x10C stop 0x42
        let brz = |rt: u32, imm: i16| (0x040u32 << 23) | ((imm as u16 as u32 & 0xFFFF) << 7) | rt;
        let ls = make_ls(0x100, &[
            il(5, 0),
            brz(5, 2),
            stop_op(0xFF),
            stop_op(0x42),
        ]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.stop_code, 0x42);
    }

    #[test]
    fn jit_compiles_brz_fallthrough() {
        // Set r5 = 1, brz r5, +2 → fallthrough, hit stop 0xFF first.
        let brz = |rt: u32, imm: i16| (0x040u32 << 23) | ((imm as u16 as u32 & 0xFFFF) << 7) | rt;
        let ls = make_ls(0x100, &[
            il(5, 1),
            brz(5, 2),
            stop_op(0xFF),
            stop_op(0x42),
        ]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.stop_code, 0xFF);
    }

    #[test]
    fn jit_compiles_shli_immediate_shift() {
        // il r3, 1; shli r4, r3, 5; stop. r4 should be 32 broadcast.
        let shli = |rt: u32, ra: u32, imm7: u32| (0x07Bu32 << 21) | ((imm7 & 0x7F) << 14) | ((ra & 0x7F) << 7) | rt;
        let ls = make_ls(0x100, &[il(3, 1), shli(4, 3, 5), stop_op(0x33)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.load_gpr(4), 0x00000020_00000020_00000020_00000020);
    }

    #[test]
    fn jit_compiles_rotmi_logical_shr() {
        // il r3, -16 (0xFFF0); rotmi r4, r3, -4 (shr 4); stop.
        // r3 sign-ext to 0xFFFFFFF0; r4 = 0x0FFFFFFF.
        let rotmi = |rt: u32, ra: u32, imm7: i8| (0x079u32 << 21) | (((imm7 as u32) & 0x7F) << 14) | ((ra & 0x7F) << 7) | rt;
        let ls = make_ls(0x100, &[il(3, -16), rotmi(4, 3, -4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        func.call(&mut state);
        assert_eq!(state.load_gpr(4), 0x0FFFFFFF_0FFFFFFF_0FFFFFFF_0FFFFFFF);
    }

    #[test]
    fn jit_compiles_ceq_word_compare() {
        // il r3, 5; il r4, 5; ceq r5, r3, r4; stop.
        // r5 should be all 0xFFFFFFFF (5 == 5 per lane).
        let ceq = |rt: u32, ra: u32, rb: u32| (0x3C0u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[il(3, 5), il(4, 5), ceq(5, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        func.call(&mut state);
        assert_eq!(state.load_gpr(5), 0xFFFFFFFF_FFFFFFFF_FFFFFFFF_FFFFFFFF);
    }

    #[test]
    fn jit_compiles_shl_dynamic_shift() {
        // il r3, 1; il r4, 4; shl r5, r3, r4; stop. r5 = 16 broadcast.
        let shl = |rt: u32, ra: u32, rb: u32| (0x05Bu32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[il(3, 1), il(4, 4), shl(5, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        func.call(&mut state);
        assert_eq!(state.load_gpr(5), 0x00000010_00000010_00000010_00000010);
    }

    #[test]
    fn jit_shl_handles_count_ge_32_as_zero() {
        // il r3, 1; il r4, 32; shl r5, r3, r4; stop. r5 = 0 broadcast.
        let shl = |rt: u32, ra: u32, rb: u32| (0x05Bu32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[il(3, 1), il(4, 32), shl(5, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        func.call(&mut state);
        assert_eq!(state.load_gpr(5), 0);
    }

    #[test]
    fn jit_compiles_stqd_then_lqd_round_trip() {
        // il r3, 0x5A5A          ; pattern broadcast
        // ila r4, 0x40           ; base address
        // stqd r3, 1(r4)         ; store at LSA 0x40 + 16 = 0x50
        // lqd r5, 1(r4)          ; load same address back
        // stop 0xAB
        let ila = |rt: u32, imm: u32| (0x21u32 << 25) | ((imm & 0x3FFFF) << 7) | rt;
        let stqd = |rt: u32, ra: u32, imm: i16|
            (0x24u32 << 24) | (((imm as u32) & 0x3FF) << 14) | (ra << 7) | rt;
        let lqd = |rt: u32, ra: u32, imm: i16|
            (0x34u32 << 24) | (((imm as u32) & 0x3FF) << 14) | (ra << 7) | rt;
        let ls_program = make_ls(0x100, &[
            il(3, 0x5A5A),
            ila(4, 0x40),
            stqd(3, 4, 1),
            lqd(5, 4, 1),
            stop_op(0xAB),
        ]);

        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls_program, 0x100).expect("compile");

        // Allocate a real LS buffer for the JIT to access.
        let mut ls_buf = vec![0u8; 0x40000];
        // Need to ALSO stage the program code in this buffer (the JIT
        // reads from `ls_program` for instruction bytes through
        // compile_at, but stqd/lqd dereference the LIVE ls_buf).
        // For this test we don't fetch instructions at runtime, so
        // ls_buf can be empty for code; just need it for data ops.
        let mut state = JitState::new();
        state.ls_ptr = ls_buf.as_mut_ptr();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.stop_code, 0xAB);

        // r5 should hold the same value r3 held when stored.
        assert_eq!(state.load_gpr(5), 0x00005A5A_00005A5A_00005A5A_00005A5A);
        // The LS at 0x50 should have been written with the BE pattern.
        let written = u128::from_be_bytes(ls_buf[0x50..0x60].try_into().unwrap());
        assert_eq!(written, 0x00005A5A_00005A5A_00005A5A_00005A5A);
    }

    #[test]
    fn jit_compiles_ah_per_halfword() {
        // il r3, 0x1234; il r4, 0x5678; ah r5, r3, r4; stop.
        // il sign-extends imm16 into each 32-bit lane → high half = 0,
        // low half = imm. ah sums each halfword pair:
        //   hw_hi: 0 + 0 = 0
        //   hw_lo: 0x1234 + 0x5678 = 0x68AC
        // → each lane of r5 = 0x000068AC.
        let ah = |rt: u32, ra: u32, rb: u32| (0x0C8u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[il(3, 0x1234), il(4, 0x5678), ah(5, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        func.call(&mut state);
        assert_eq!(state.load_gpr(5), 0x000068AC_000068AC_000068AC_000068ACu128);
    }

    #[test]
    fn jit_ah_handles_halfword_wrap() {
        // Pre-set r3 = 0xFFFF_0001 in each lane (high half = max, low = 1).
        // Pre-set r4 = 0x0001_FFFF (high half = 1, low = max).
        // ah:
        //   hw_hi: 0xFFFF + 0x0001 = 0x10000 → masked to 0x0000.
        //   hw_lo: 0x0001 + 0xFFFF = 0x10000 → masked to 0x0000.
        // → r5 each lane = 0x00000000.
        let ah = |rt: u32, ra: u32, rb: u32| (0x0C8u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[ah(5, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0xFFFF_0001; }
        for lane in 0..4 { state.gpr_lanes[4][lane] = 0x0001_FFFF; }
        func.call(&mut state);
        assert_eq!(state.load_gpr(5), 0);
    }

    #[test]
    fn jit_compiles_orx_collapse_into_preferred_slot() {
        // Manually set r4 lanes to [1, 2, 4, 8] then orx r6, r4 → r6 = [0xF, 0, 0, 0].
        let orx = |rt: u32, ra: u32| (0x1F0u32 << 21) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[orx(6, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        state.gpr_lanes[4] = [1, 2, 4, 8];
        func.call(&mut state);
        assert_eq!(state.load_gpr(6), 0x0000000F_00000000_00000000_00000000);
    }

    #[test]
    fn jit_compiles_fcgt_and_fceq() {
        // r3 = 2.0, r4 = 1.0. fcgt r5, r3, r4 → all lanes 0xFFFFFFFF (2 > 1).
        // fceq r6, r3, r4 → all lanes 0 (2 != 1).
        let fcgt = |rt: u32, ra: u32, rb: u32| (0x2C2u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let fceq = |rt: u32, ra: u32, rb: u32| (0x3C2u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[fcgt(5, 3, 4), fceq(6, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0x40000000; }  // 2.0
        for lane in 0..4 { state.gpr_lanes[4][lane] = 0x3F800000; }  // 1.0
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0xFFFFFFFF, "fcgt lane {lane}");
            assert_eq!(state.gpr_lanes[6][lane], 0, "fceq lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_ceqb_byte_compare() {
        // ceqb: per-byte equality. ra = 0xAA_BB_CC_DD × 4, rb = 0xAA_00_CC_00 × 4.
        // Result: 0xFF_00_FF_00 × 4.
        let ceqb = |rt: u32, ra: u32, rb: u32| (0x3D0u32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[ceqb(5, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0xAABBCCDD; }
        for lane in 0..4 { state.gpr_lanes[4][lane] = 0xAA00CC00; }
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0xFF00FF00, "ceqb lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_shlh_halfword_rr_shift() {
        // shlh: per-halfword shl, count from rb's halfword.
        // ra = 0x0001_0001 × 4, rb = 0x0008_0004 × 4 → r5 = 0x0100_0010 × 4.
        let shlh = |rt: u32, ra: u32, rb: u32| (0x05Fu32 << 21) | (rb << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[shlh(5, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0x00010001; }
        for lane in 0..4 { state.gpr_lanes[4][lane] = 0x00080004; }
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0x01000010, "shlh lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_mpyi_immediate_multiply() {
        // mpyi r5, r3, 7. r3 each lane = 6 → r5 each lane = 42 = 0x2A.
        let mpyi = |rt: u32, ra: u32, imm10: u32|
            (0x74u32 << 24) | ((imm10 & 0x3FF) << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[mpyi(5, 3, 7), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[3][lane] = 6; }
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 42, "mpyi lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_ceqhi_halfword_imm_compare() {
        // ceqhi r5, r3, 42. r3 lanes have halfwords with various values.
        let ceqhi = |rt: u32, ra: u32, imm10: u32|
            (0x7Du32 << 24) | ((imm10 & 0x3FF) << 14) | (ra << 7) | rt;
        let ls = make_ls(0x100, &[ceqhi(5, 3, 42), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        // each lane has high half = 42, low half = 1. So r5 should be 0xFFFF_0000 × 4.
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0x002A_0001; }
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0xFFFF_0000, "ceqhi lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_clz_count_leading_zeros() {
        let clz = |rt: u32, ra: u32| (0x2A5u32 << 21) | ((ra & 0x7F) << 7) | rt;
        let ls = make_ls(0x100, &[clz(5, 3), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        state.gpr_lanes[3] = [0x80000000, 0x00000001, 0x00000000, 0x00FFFFFF];
        func.call(&mut state);
        assert_eq!(state.gpr_lanes[5], [0, 31, 32, 8]);
    }

    #[test]
    fn jit_compiles_xshw_sign_extend_halfword() {
        let xshw = |rt: u32, ra: u32| (0x2AEu32 << 21) | ((ra & 0x7F) << 7) | rt;
        let ls = make_ls(0x100, &[xshw(5, 3), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        // Each lane has low half = -1 (0xFFFF). After xshw, each lane = 0xFFFFFFFF.
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0xAAAA_FFFF; }
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0xFFFF_FFFF, "lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_fsm_form_select_mask() {
        let fsm = |rt: u32, ra: u32| (0x1B4u32 << 21) | ((ra & 0x7F) << 7) | rt;
        let ls = make_ls(0x100, &[fsm(5, 3), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        // Preferred slot (lane 0) = 0b1010 = bits 3 and 1 set.
        // → lane 0 (bit 3) = 0xFFFFFFFF, lane 1 (bit 2) = 0, lane 2 (bit 1) = 0xFFFFFFFF, lane 3 (bit 0) = 0.
        state.gpr_lanes[3][0] = 0b1010;
        func.call(&mut state);
        assert_eq!(state.gpr_lanes[5], [0xFFFFFFFF, 0, 0xFFFFFFFF, 0]);
    }

    #[test]
    fn jit_compiles_frest_naive_reciprocal() {
        let frest = |rt: u32, ra: u32| (0x1B8u32 << 21) | ((ra & 0x7F) << 7) | rt;
        let ls = make_ls(0x100, &[frest(5, 3), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        // r3 = 2.0 in each lane. frest → 0.5 (= 0x3F000000).
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0x40000000; }
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0x3F000000, "lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_andbi_byte_immediate_and() {
        // andbi rt, ra, imm8: rt = ra & broadcast(imm8) per byte.
        // ra = 0xFFFFFFFF × 4, imm8 = 0x55 → rt = 0x55555555 × 4.
        let andbi = |rt: u32, ra: u32, imm8: u32|
            (0x16u32 << 24) | ((imm8 & 0xFF) << 16) | ((ra & 0x7F) << 7) | (rt & 0x7F);
        let ls = make_ls(0x100, &[andbi(5, 3, 0x55), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0xFFFFFFFF; }
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0x55555555, "lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_ceqbi_per_byte_compare_with_imm() {
        // ceqbi rt, ra, imm8: 0xFF where byte == imm8, 0 else.
        let ceqbi = |rt: u32, ra: u32, imm8: u32|
            (0x7Eu32 << 24) | ((imm8 & 0xFF) << 16) | ((ra & 0x7F) << 7) | (rt & 0x7F);
        let ls = make_ls(0x100, &[ceqbi(5, 3, 0xAB), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        // Each lane has bytes [0xAB, 0x00, 0xAB, 0xFF]: bytes 0,2 match imm8.
        // In LE u32: byte 0 = LSB. Lane = 0xFF_AB_00_AB.
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0xFFAB00AB; }
        func.call(&mut state);
        // Expected per byte: [0xFF, 0x00, 0xFF, 0x00] in LE → 0x00FF00FF.
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0x00FF00FF, "lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_shufb_byte_permutation_identity() {
        // shufb rt, ra, rb, rc with rc selecting bytes 0..15 from ra
        // (identity permutation) → rt should equal ra.
        let shufb = |rt: u32, ra: u32, rb: u32, rc: u32|
            (0xBu32 << 28) | ((rc & 0x7F) << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F);
        let ls = make_ls(0x100, &[shufb(5, 2, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        // ra (r2) = 0x00112233 _ 0x44556677 _ 0x8899AABB _ 0xCCDDEEFF
        state.gpr_lanes[2] = [0x00112233, 0x44556677, 0x8899AABB, 0xCCDDEEFFu32];
        // rc (r4) = identity selectors 0..15. In SPU BE byte layout,
        // selector byte 0 = high byte of lane 0 = byte at LE offset 3.
        // So we need to lay out [0,1,...,15] in BE order.
        // Lane 0 BE bytes 0..3 = 0,1,2,3 → in LE u32 = (3<<24)|(2<<16)|(1<<8)|0 = 0x00010203
        state.gpr_lanes[4] = [0x00010203, 0x04050607, 0x08090A0B, 0x0C0D0E0F];
        func.call(&mut state);
        // rt should equal ra.
        assert_eq!(state.gpr_lanes[5], state.gpr_lanes[2],
                   "shufb identity should produce ra unchanged");
    }

    #[test]
    fn jit_compiles_shufb_constant_patterns() {
        // shufb with selector bytes triggering constant patterns.
        // Selector 0x80 → byte = 0x00.
        // Selector 0xC0 → byte = 0xFF.
        // Selector 0xE0 → byte = 0x80.
        let shufb = |rt: u32, ra: u32, rb: u32, rc: u32|
            (0xBu32 << 28) | ((rc & 0x7F) << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F);
        let ls = make_ls(0x100, &[shufb(5, 2, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        state.gpr_lanes[2] = [0xAAAAAAAA, 0xBBBBBBBB, 0xCCCCCCCC, 0xDDDDDDDDu32];
        // 16 selector bytes (BE):
        //   bytes 0..3:   0x80,0x80,0x80,0x80   → output 0x00,0x00,0x00,0x00
        //   bytes 4..7:   0xC0,0xC0,0xC0,0xC0   → output 0xFF,0xFF,0xFF,0xFF
        //   bytes 8..11:  0xE0,0xE0,0xE0,0xE0   → output 0x80,0x80,0x80,0x80
        //   bytes 12..15: 0x00,0x01,0x02,0x03   → output ra bytes 0..3
        // Lane 0 BE = 0x80,0x80,0x80,0x80 → LE u32 = 0x80808080
        // Lane 1 BE = 0xC0,0xC0,0xC0,0xC0 → LE = 0xC0C0C0C0
        // Lane 2 BE = 0xE0,0xE0,0xE0,0xE0 → LE = 0xE0E0E0E0
        // Lane 3 BE = 0x00,0x01,0x02,0x03 → LE = 0x00010203
        state.gpr_lanes[4] = [0x80808080, 0xC0C0C0C0, 0xE0E0E0E0, 0x00010203];
        func.call(&mut state);
        // Output BE bytes:
        //   lane 0: 0x00,0x00,0x00,0x00 → LE = 0x00000000
        //   lane 1: 0xFF,0xFF,0xFF,0xFF → LE = 0xFFFFFFFF
        //   lane 2: 0x80,0x80,0x80,0x80 → LE = 0x80808080
        //   lane 3: ra bytes 0..3 = 0xAA,0xAA,0xAA,0xAA → LE = 0xAAAAAAAA
        // (because ra lane 0 = 0xAAAAAAAA, all bytes 0xAA)
        assert_eq!(state.gpr_lanes[5], [0x00000000, 0xFFFFFFFF, 0x80808080, 0xAAAAAAAA]);
    }

    #[test]
    fn jit_compiles_fma_multiply_add() {
        // fma rt, ra, rb, rc → rt = ra*rb + rc per lane.
        // ra=2.0, rb=3.0, rc=1.0 → rt = 7.0 (= 0x40E00000).
        let fma = |rt: u32, ra: u32, rb: u32, rc: u32|
            (0xEu32 << 28) | ((rc & 0x7F) << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F);
        let ls = make_ls(0x100, &[fma(5, 2, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[2][lane] = 0x40000000; } // 2.0
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0x40400000; } // 3.0
        for lane in 0..4 { state.gpr_lanes[4][lane] = 0x3F800000; } // 1.0
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0x40E00000, "lane {lane} (= 7.0)");
        }
    }

    #[test]
    fn jit_compiles_fnms_negative_subtract() {
        // fnms rt, ra, rb, rc → rt = rc - ra*rb per lane.
        // ra=2.0, rb=3.0, rc=10.0 → rt = 10 - 6 = 4.0 (= 0x40800000).
        let fnms = |rt: u32, ra: u32, rb: u32, rc: u32|
            (0xDu32 << 28) | ((rc & 0x7F) << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F);
        let ls = make_ls(0x100, &[fnms(5, 2, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[2][lane] = 0x40000000; } // 2.0
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0x40400000; } // 3.0
        for lane in 0..4 { state.gpr_lanes[4][lane] = 0x41200000; } // 10.0
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0x40800000, "lane {lane} (= 4.0)");
        }
    }

    #[test]
    fn jit_compiles_selb_bit_select() {
        // selb r5, r2, r3, r4 → r5 = (r4 & r3) | (!r4 & r2) per lane.
        // Pre-set r2 = 0xAAAAAAAA × 4, r3 = 0x55555555 × 4, r4 = 0xFF00FF00 × 4.
        // Where r4 bit is 1: pick r3 bit. Where 0: pick r2 bit.
        // r5 lane = (0xFF00FF00 & 0x55555555) | (0x00FF00FF & 0xAAAAAAAA)
        //        = 0x55005500 | 0x00AA00AA = 0x55AA55AA.
        let selb = |rt: u32, ra: u32, rb: u32, rc: u32|
            (0x8u32 << 28) | ((rc & 0x7F) << 21) | ((rb & 0x7F) << 14) | ((ra & 0x7F) << 7) | (rt & 0x7F);
        let ls = make_ls(0x100, &[selb(5, 2, 3, 4), stop_op(0)]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        for lane in 0..4 { state.gpr_lanes[2][lane] = 0xAAAAAAAA; }
        for lane in 0..4 { state.gpr_lanes[3][lane] = 0x55555555; }
        for lane in 0..4 { state.gpr_lanes[4][lane] = 0xFF00FF00; }
        func.call(&mut state);
        for lane in 0..4 {
            assert_eq!(state.gpr_lanes[5][lane], 0x55AA55AA, "lane {lane}");
        }
    }

    #[test]
    fn jit_compiles_brsl_with_link_register() {
        // Subroutine writes r3, returns via... wait, brsl needs to be
        // followed by code that uses the link. Without indirect branch
        // support we can't call+return. So just verify link is written
        // before the jump.
        // 0x100 brsl r2, +2  → jumps to 0x108, link r2 = 0x104
        // 0x104 stop 0xCC    (NOT executed)
        // 0x108 stop 0xDD
        let brsl = |rt: u32, imm: i16| (0x066u32 << 23) | ((imm as u16 as u32 & 0xFFFF) << 7) | rt;
        let ls = make_ls(0x100, &[
            brsl(2, 2),
            stop_op(0xCC),
            stop_op(0xDD),
        ]);
        let mut backend = JitBackend::new();
        let func = backend.compile_at(&ls, 0x100).expect("compile");
        let mut state = JitState::new();
        let outcome = func.call(&mut state);
        assert_eq!(outcome, JIT_OUTCOME_STOP);
        assert_eq!(state.stop_code, 0xDD);
        // link register: 0x104 broadcast.
        assert_eq!(state.load_gpr(2), 0x00000104_00000104_00000104_00000104);
    }
}
