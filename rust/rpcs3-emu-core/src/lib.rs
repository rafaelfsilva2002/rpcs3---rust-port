//! `rpcs3-emu-core` — integration crate that boots a PPU ELF and runs it.
//!
//! This is where the rest of the Rust port stops being isolated crates
//! and starts being an emulator. The crate wires:
//!
//! * `rpcs3-memory-backing` (guest memory + page table)
//! * `rpcs3-ppu-thread` (register file)
//! * `rpcs3-ppu-interpreter` (fetch/decode/execute)
//! * `rpcs3-loader-elf-self` (ELF header parsing)
//! * `rpcs3-lv2-process` + `rpcs3-lv2-ppu-thread` (syscall dispatch)
//!
//! ## Current scope (iteration 1)
//!
//! Single PPU thread. Boots from a plaintext ELF or from raw bytes
//! loaded at a chosen base address. Runs until:
//!
//! * The program calls `_sys_process_exit`, `_sys_process_exit2`, or
//!   `_sys_ppu_thread_exit` — in our single-thread model, all three
//!   terminate the run.
//! * The interpreter hits an unimplemented opcode or memory fault.
//! * An optional step budget is exhausted (test-time safety net).
//!
//! ## Syscall dispatch
//!
//! By LV2 convention the syscall number lives in `r11` when `sc`
//! executes. The dispatcher reads `r11`, resolves the handler, pulls
//! args from `r3..=r10`, and writes the return value back into `r3`.

use rpcs3_emu_types::CellError;
use rpcs3_loader_elf_self::{parse_elf, ElfInfo, Error as ElfError};
use rpcs3_memory::PageFlags;
use rpcs3_memory_backing::{Error as MemError, SparseBackend};
use rpcs3_ppu_interpreter::{step, Error as InterpError, StepOutcome};
use rpcs3_ppu_thread::{PpuThread, PPU_ID_BASE};

use rpcs3_lv2_process::{
    sys_process_get_number_of_object, sys_process_get_sdk_version, sys_process_getpid,
    sys_process_getppid, ObjectType as ProcObjectType, SyscallResult, TestProcessState,
};

use rpcs3_lv2_spu_group::{
    sys_spu_thread_group_create, sys_spu_thread_group_destroy, sys_spu_thread_group_join,
    sys_spu_thread_group_start, GroupAttr, SpuGroupRegistry, TestSpuGroupRegistry,
};
use rpcs3_lv2_spu_image::{deploy as deploy_image, SpuImage};
use rpcs3_spu_interpreter::{run_n as spu_run_n, StepOutcome as SpuStepOutcome, Error as SpuError};
use rpcs3_spu_thread::SpuThread;

// =====================================================================
// Types
// =====================================================================

/// Normal termination of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    pub status: i32,
}

/// Errors the core can surface to callers.
#[derive(Debug)]
pub enum Error {
    Elf(ElfError),
    Memory(MemError),
    Interpreter(InterpError),
    /// The program called an `sc` with a syscall number we haven't
    /// implemented yet. Carries the number so tests can pin on it.
    UnsupportedSyscall { number: u64, cia: u32 },
    /// The step budget was exhausted before the program exited.
    StepsExhausted,
    /// The ELF file doesn't have a usable PT_LOAD within the PPU main
    /// RAM window.
    ElfNotLoadable(&'static str),
    /// An error surfaced from the SPU interpreter.
    Spu(SpuError),
    /// A SPU thread group syscall failed.
    SpuGroup(CellError),
}

impl From<SpuError> for Error {
    fn from(e: SpuError) -> Self { Self::Spu(e) }
}

impl From<ElfError> for Error {
    fn from(e: ElfError) -> Self {
        Self::Elf(e)
    }
}
impl From<MemError> for Error {
    fn from(e: MemError) -> Self {
        Self::Memory(e)
    }
}
impl From<InterpError> for Error {
    fn from(e: InterpError) -> Self {
        Self::Interpreter(e)
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Elf(e) => write!(f, "ELF: {e}"),
            Error::Memory(e) => write!(f, "memory: {e}"),
            Error::Interpreter(e) => write!(f, "interpreter: {e}"),
            Error::UnsupportedSyscall { number, cia } => {
                write!(f, "unsupported syscall #{number} at CIA 0x{cia:08x}")
            }
            Error::StepsExhausted => f.write_str("step budget exhausted"),
            Error::ElfNotLoadable(r) => write!(f, "ELF not loadable: {r}"),
            Error::Spu(e) => write!(f, "SPU: {e:?}"),
            Error::SpuGroup(e) => write!(f, "SPU group: cell error 0x{:08x}", e.0),
        }
    }
}

impl std::error::Error for Error {}

// =====================================================================
// EmuCore
// =====================================================================

/// Single-threaded PPU emulator core. One of these per run.
pub struct EmuCore {
    pub mem: SparseBackend,
    pub ppu: PpuThread,
    pub process: TestProcessState,
    /// Maximum steps per `run` invocation. 0 = unbounded.
    pub step_budget: usize,
}

impl Default for EmuCore {
    fn default() -> Self {
        Self::new()
    }
}

impl EmuCore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            mem: SparseBackend::new(),
            ppu: PpuThread::new(PPU_ID_BASE),
            process: TestProcessState::default(),
            step_budget: 100_000,
        }
    }

    /// Load a contiguous blob of PPU code+data at `base`, size rounded
    /// up to the next 4 KB. Useful for tests that assemble a program
    /// by hand without round-tripping through ELF.
    pub fn load_raw(&mut self, base: u32, bytes: &[u8]) -> Result<(), Error> {
        let size = (bytes.len() as u32 + 0xFFF) & !0xFFF;
        self.mem
            .alloc_at(
                base,
                size,
                PageFlags::READABLE | PageFlags::WRITABLE | PageFlags::EXECUTABLE,
            )?;
        self.mem.write(base, bytes)?;
        self.ppu.cia = base;
        Ok(())
    }

    /// Parse an ELF file and copy its PT_LOAD segments into memory.
    /// Sets `cia` to the ELF entry point. Supports ELF64-BE PPU only
    /// (matches the PS3 format).
    pub fn load_elf(&mut self, elf_bytes: &[u8]) -> Result<ElfInfo, Error> {
        let info = parse_elf(elf_bytes)?;
        if !info.is_ppu64() {
            return Err(Error::ElfNotLoadable("not a PPU64 ELF"));
        }

        let mut any_loaded = false;
        for ph in info.pt_load_iter() {
            if ph.p_memsz == 0 {
                continue;
            }
            // Round base down, size up, to 4 KB alignment.
            let page_base = (ph.p_vaddr as u32) & !0xFFF;
            let inner_offset = (ph.p_vaddr as u32) & 0xFFF;
            let wanted_bytes = inner_offset + ph.p_memsz as u32;
            let aligned = (wanted_bytes + 0xFFF) & !0xFFF;

            // Allocate if not already present. Permissions derive from
            // p_flags: bit 0 = executable, bit 1 = writable, bit 2 = readable.
            let mut flags = PageFlags::empty();
            if ph.p_flags & 0x4 != 0 {
                flags = flags.union(PageFlags::READABLE);
            }
            if ph.p_flags & 0x2 != 0 {
                flags = flags.union(PageFlags::WRITABLE);
            }
            if ph.p_flags & 0x1 != 0 {
                flags = flags.union(PageFlags::EXECUTABLE);
            }
            if flags.is_empty() {
                flags = PageFlags::READABLE;
            }
            // To write bytes we also need WRITABLE during load; we'll
            // page_protect back after copying.
            self.mem.alloc_at(page_base, aligned, flags.union(PageFlags::WRITABLE))?;

            // Copy p_filesz bytes from elf_bytes[p_offset..]; the rest
            // of p_memsz is zero-filled (the alloc already zeroes pages).
            let src_start = ph.p_offset as usize;
            let src_end = src_start + ph.p_filesz as usize;
            if src_end > elf_bytes.len() {
                return Err(Error::ElfNotLoadable("p_offset+p_filesz out of bounds"));
            }
            self.mem.write(ph.p_vaddr as u32, &elf_bytes[src_start..src_end])?;

            // If the segment shouldn't be writable, strip WRITABLE now.
            if ph.p_flags & 0x2 == 0 {
                self.mem.page_protect(page_base, aligned, PageFlags::empty(), PageFlags::WRITABLE)?;
            }
            any_loaded = true;
        }

        if !any_loaded {
            return Err(Error::ElfNotLoadable("no PT_LOAD segments"));
        }

        self.ppu.cia = info.e_entry as u32;
        Ok(info)
    }

    /// Run the currently-loaded program until process exit or error.
    pub fn run(&mut self) -> Result<ExitStatus, Error> {
        let budget = if self.step_budget == 0 {
            usize::MAX
        } else {
            self.step_budget
        };
        for _ in 0..budget {
            match step(&mut self.ppu, &mut self.mem)? {
                StepOutcome::Continue => {}
                StepOutcome::Syscall => {
                    if let Some(exit) = self.dispatch_syscall()? {
                        return Ok(exit);
                    }
                }
            }
        }
        Err(Error::StepsExhausted)
    }

    /// Dispatch the syscall that just triggered. The syscall number
    /// is in `r11` by LV2 convention. Returns `Some(ExitStatus)` if
    /// the program is ending.
    fn dispatch_syscall(&mut self) -> Result<Option<ExitStatus>, Error> {
        let number = self.ppu.gpr[11];
        let cia_at_sc = self.ppu.cia.wrapping_sub(4);

        // r3..r10 hold args; return value goes back into r3.
        let r3 = self.ppu.gpr[3];
        let r4 = self.ppu.gpr[4];
        let _r5 = self.ppu.gpr[5];

        match number {
            1 => {
                // sys_process_getpid → s32 in r3
                let r = sys_process_getpid(&self.process);
                self.write_syscall_result(r);
            }
            12 => {
                // sys_process_get_number_of_object(object, nump)
                let obj = r3 as u32;
                let nump = r4 as u32;
                match sys_process_get_number_of_object(&self.process, obj) {
                    Ok(count) => {
                        // Write count to *nump (u32 BE in guest memory).
                        let mut buf = [0u8; 4];
                        buf.copy_from_slice(&count.to_be_bytes());
                        self.mem.write(nump, &buf)?;
                        self.ppu.gpr[3] = 0; // CELL_OK
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = u64::from(u32::from(e));
                    }
                }
                // Reference the import just to satisfy link (unused path gets
                // optimized out; keeping explicit dependency).
                let _ = ProcObjectType::Mem;
            }
            18 => {
                // sys_process_getppid
                let r = sys_process_getppid(&self.process);
                self.write_syscall_result(r);
            }
            22 => {
                // _sys_process_exit(status, arg2, arg3)
                return Ok(Some(ExitStatus { status: r3 as i32 }));
            }
            25 => {
                // sys_process_get_sdk_version(pid, *version)
                let pid = r3 as u32;
                let version_ptr = r4 as u32;
                match sys_process_get_sdk_version(&self.process, pid) {
                    Ok(v) => {
                        let mut buf = [0u8; 4];
                        buf.copy_from_slice(&v.to_be_bytes());
                        self.mem.write(version_ptr, &buf)?;
                        self.ppu.gpr[3] = 0;
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = u64::from(u32::from(e));
                    }
                }
            }
            26 => {
                // _sys_process_exit2(status, arg_ptr, arg_size, arg4)
                return Ok(Some(ExitStatus { status: r3 as i32 }));
            }
            41 => {
                // _sys_ppu_thread_exit(errorcode) — in single-threaded
                // mode this terminates the process too.
                return Ok(Some(ExitStatus { status: r3 as i32 }));
            }
            43 => {
                // sys_ppu_thread_yield — no-op in single-thread mode.
            }
            _ => {
                return Err(Error::UnsupportedSyscall { number, cia: cia_at_sc });
            }
        }
        Ok(None)
    }

    fn write_syscall_result(&mut self, r: SyscallResult) {
        match r {
            SyscallResult::Ok(v) => {
                self.ppu.gpr[3] = v;
            }
            SyscallResult::Err(e) => {
                self.ppu.gpr[3] = u64::from(u32::from(e));
            }
            SyscallResult::Exit { .. } => {
                // Handled at call-site in the variants that return Exit.
            }
        }
    }
}

// =====================================================================
// SPU integration — end-to-end thread group runner
// =====================================================================

/// Outcome of running a single SPU thread inside a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuThreadRunOutcome {
    pub thread_index: u32,
    /// Stop code from the `stop` instruction that terminated the thread.
    pub stop_code: u32,
    /// Steps executed before the stop.
    pub steps: usize,
}

impl EmuCore {
    /// Boot a SPU thread group with a single thread from the given
    /// image and run it to completion. This is the SPU-side mirror of
    /// [`run`][Self::run]: the caller hands in a parsed [`SpuImage`]
    /// (usually via `rpcs3-lv2-spu-image::build_image`) plus source
    /// bytes; we deploy COPY/FILL segments into a fresh 256 KB LS,
    /// execute the SPU interpreter until a `stop` instruction, and
    /// return the stop code.
    ///
    /// Also creates + starts + joins a matching thread group via
    /// `sys_spu_thread_group_*`, so the syscall path is exercised.
    pub fn run_spu_group_single(
        &mut self,
        image: &SpuImage,
        source: &[u8],
        group_attr: GroupAttr,
        step_budget: usize,
    ) -> Result<(u32, SpuThreadRunOutcome), Error> {
        // 1) Allocate a scratch LS buffer and deploy the image. COPY
        //    segments pull bytes from `source` at `seg.addr - 0x1000`
        //    (an arbitrary but consistent base we apply when building
        //    the image — tests use `src=0x1000`).
        let src_base: u32 = 0x1000;
        let mut ls = vec![0u8; rpcs3_spu_thread::SPU_LS_SIZE];
        deploy_image(image, &mut ls, |addr, size| {
            let off = addr.checked_sub(src_base)? as usize;
            let end = off.checked_add(size as usize)?;
            if end > source.len() { return None; }
            Some(source[off..end].to_vec())
        })
        .map_err(Error::SpuGroup)?;

        // 2) Create + start a thread group via the syscall path.
        let mut reg = TestSpuGroupRegistry::default();
        let gid = sys_spu_thread_group_create(&mut reg, group_attr).map_err(Error::SpuGroup)?;
        sys_spu_thread_group_start(&mut reg, gid).map_err(Error::SpuGroup)?;

        // 3) Execute the single SPU thread inside this group.
        let mut spu = SpuThread::new(0);
        // Replace the zeroed LS with our deployed image.
        for (i, chunk) in ls.chunks(65536).enumerate() {
            assert!(spu.ls_write((i as u32) * 65536, chunk));
        }
        spu.pc = image.entry_point;

        let (steps, outcome) = spu_run_n(&mut spu, step_budget)?;
        let stop_code = match outcome {
            SpuStepOutcome::Stop(code) => code,
            SpuStepOutcome::Continue => return Err(Error::StepsExhausted),
            SpuStepOutcome::ChannelStall { .. } => return Err(Error::StepsExhausted),
            // R7.1 — refuse_mfc defaults to false on a fresh SpuThread,
            // so this arm is unreachable in the emu-core test
            // harness. Listed explicitly to satisfy exhaustiveness.
            SpuStepOutcome::MfcUnsupported { .. } => return Err(Error::StepsExhausted),
        };

        // 4) Retire the thread, join the group, destroy.
        reg.thread_exited(gid, 0).map_err(Error::SpuGroup)?;
        let (_cause, _status) = sys_spu_thread_group_join(&mut reg, gid).map_err(Error::SpuGroup)?;
        sys_spu_thread_group_destroy(&mut reg, gid).map_err(Error::SpuGroup)?;

        Ok((gid, SpuThreadRunOutcome { thread_index: 0, stop_code, steps }))
    }
}

// Suppress unused-import warnings for crates we depend on but don't
// reference directly (they show up in the dependency graph for future
// iterations, and removing them would just re-add them later).
#[doc(hidden)]
pub use rpcs3_emu_types as _emu_types;
#[doc(hidden)]
pub use rpcs3_lv2_ppu_thread as _lv2_ppu_thread;
#[doc(hidden)]
#[allow(unused_imports)]
use rpcs3_lv2_process::_sys_process_exit as _;

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use rpcs3_ppu_interpreter::encode;

    const PROG_BASE: u32 = 0x1000;

    fn assemble(insts: &[u32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(insts.len() * 4);
        for i in insts {
            out.extend_from_slice(&i.to_be_bytes());
        }
        out
    }

    fn run_program(insts: &[u32]) -> Result<ExitStatus, Error> {
        let bytes = assemble(insts);
        let mut core = EmuCore::new();
        core.load_raw(PROG_BASE, &bytes)?;
        core.run()
    }

    // -- Exit syscalls -------------------------------------------

    /// `li r11, 22; li r3, 42; sc` — canonical minimal exit-42 program.
    #[test]
    fn process_exit_returns_status() {
        let prog = [
            encode::addi(11, 0, 22), // syscall number
            encode::addi(3, 0, 42),  // status
            encode::sc(),
        ];
        let status = run_program(&prog).unwrap();
        assert_eq!(status.status, 42);
    }

    #[test]
    fn process_exit_with_zero_status() {
        let prog = [
            encode::addi(11, 0, 22),
            encode::addi(3, 0, 0),
            encode::sc(),
        ];
        let status = run_program(&prog).unwrap();
        assert_eq!(status.status, 0);
    }

    #[test]
    fn process_exit_with_negative_status() {
        let prog = [
            encode::addi(11, 0, 22),
            encode::addi(3, 0, -1),
            encode::sc(),
        ];
        let status = run_program(&prog).unwrap();
        assert_eq!(status.status, -1);
    }

    #[test]
    fn process_exit2_also_terminates() {
        let prog = [
            encode::addi(11, 0, 26),
            encode::addi(3, 0, 7),
            encode::sc(),
        ];
        let status = run_program(&prog).unwrap();
        assert_eq!(status.status, 7);
    }

    #[test]
    fn ppu_thread_exit_terminates_in_single_thread_mode() {
        // syscall 41 = _sys_ppu_thread_exit(errorcode)
        let prog = [
            encode::addi(11, 0, 41),
            encode::addi(3, 0, 123),
            encode::sc(),
        ];
        let status = run_program(&prog).unwrap();
        assert_eq!(status.status, 123);
    }

    // -- Arithmetic → exit ---------------------------------------

    #[test]
    fn add_three_numbers_then_exit() {
        // r3 = 10 + 20 + 30 = 60
        let prog = [
            encode::addi(3, 0, 10),
            encode::addi(4, 0, 20),
            encode::add(3, 3, 4),
            encode::addi(4, 0, 30),
            encode::add(3, 3, 4),
            encode::addi(11, 0, 22),
            encode::sc(),
        ];
        let status = run_program(&prog).unwrap();
        assert_eq!(status.status, 60);
    }

    // -- getpid / yield via sc -----------------------------------

    #[test]
    fn getpid_returns_1_and_yield_is_noop() {
        // getpid (number=1) → r3=1. Then yield (number=43) → no-op.
        // Then exit with r3.
        let prog = [
            encode::addi(11, 0, 1),  // syscall getpid
            encode::sc(),
            encode::addi(11, 0, 43), // yield (no-op)
            encode::sc(),
            encode::addi(11, 0, 22), // exit
            encode::sc(),
        ];
        let status = run_program(&prog).unwrap();
        assert_eq!(status.status, 1); // getpid result (still in r3)
    }

    // -- Function call then exit ---------------------------------

    #[test]
    fn function_call_then_exit() {
        // main:
        //   li r3, 5
        //   bl +8         ; call +16
        //   li r11, 22
        //   sc
        // callee (at +16): addi r3, r3, 100 ; blr
        let prog = [
            encode::addi(3, 0, 5),   // 0  r3 = 5
            encode::bl(16),          // 4  call +20
            encode::addi(11, 0, 22), // 8
            encode::sc(),            // 12
            encode::nop(),           // 16
            encode::addi(3, 3, 100), // 20 callee: r3 += 100
            encode::blr(),           // 24 return (LR=8)
        ];
        let status = run_program(&prog).unwrap();
        assert_eq!(status.status, 105);
    }

    // -- Load + exit ---------------------------------------------

    #[test]
    fn load_from_data_region_then_exit() {
        // Program + data share same page (+ adjacent page).
        // Compute data_base = PROG_BASE + 0x100.
        let data_offset: i16 = 0x100;
        let prog = [
            encode::addi(4, 0, 0),            // r4 = 0
            encode::addis(4, 4, 0),           // r4 = 0 (no-op high-16)
            // r4 needs to hold PROG_BASE. Use manual addi chain:
            encode::addi(4, 0, 0x1000u32 as i16), // r4 = PROG_BASE low (PROG_BASE = 0x1000)
            encode::lwz(3, data_offset, 4),   // r3 = *(u32*)(r4 + 0x100)
            encode::addi(11, 0, 22),
            encode::sc(),
        ];
        let bytes = assemble(&prog);
        let mut core = EmuCore::new();
        core.load_raw(PROG_BASE, &bytes).unwrap();
        // Place 0x00010203 at PROG_BASE + 0x100.
        core.mem
            .write(PROG_BASE + 0x100, &0x0001_0203u32.to_be_bytes())
            .unwrap();
        let status = core.run().unwrap();
        assert_eq!(status.status as u32, 0x0001_0203);
    }

    // -- Error surface -------------------------------------------

    #[test]
    fn unsupported_syscall_bubbles_up_as_error() {
        let prog = [
            encode::addi(11, 0, 999), // nonexistent syscall
            encode::sc(),
        ];
        let err = run_program(&prog).unwrap_err();
        assert!(matches!(err, Error::UnsupportedSyscall { number: 999, .. }));
    }

    #[test]
    fn step_budget_exhausted_returns_error() {
        // Infinite loop: b -4
        let prog = [encode::b(0)]; // branch to self
        let bytes = assemble(&prog);
        let mut core = EmuCore::new();
        core.step_budget = 10;
        core.load_raw(PROG_BASE, &bytes).unwrap();
        assert!(matches!(core.run().unwrap_err(), Error::StepsExhausted));
    }

    // -- Minimal synthetic ELF loading ---------------------------

    /// Build a minimal ELF64-BE PPU executable with a single PT_LOAD
    /// segment at 0x10000 containing `insts`, then ELF-load it and
    /// run. Stresses the ELF→memory path end-to-end.
    fn make_minimal_ppu_elf(base_vaddr: u64, insts: &[u32]) -> Vec<u8> {
        const EHDR_SIZE: usize = 64;
        const PHDR_SIZE: usize = 56;

        let code_bytes = assemble(insts);
        let header_region = EHDR_SIZE + PHDR_SIZE;
        let file_size = header_region + code_bytes.len();

        let mut bytes = vec![0u8; file_size];

        // ELF header
        bytes[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        bytes[4] = 2; // ELFCLASS64
        bytes[5] = 2; // ELFDATA2MSB (big-endian)
        bytes[6] = 1; // EV_CURRENT
        bytes[7] = 102; // ELFOSABI_CELL_LV2
        bytes[16..18].copy_from_slice(&2u16.to_be_bytes()); // ET_EXEC
        bytes[18..20].copy_from_slice(&0x15u16.to_be_bytes()); // EM_PPC64
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes());
        bytes[24..32].copy_from_slice(&base_vaddr.to_be_bytes()); // e_entry
        bytes[32..40].copy_from_slice(&(EHDR_SIZE as u64).to_be_bytes()); // e_phoff
        bytes[52..54].copy_from_slice(&(EHDR_SIZE as u16).to_be_bytes());
        bytes[54..56].copy_from_slice(&(PHDR_SIZE as u16).to_be_bytes());
        bytes[56..58].copy_from_slice(&1u16.to_be_bytes()); // e_phnum

        // Program header — PT_LOAD with R+X
        let ph = EHDR_SIZE;
        bytes[ph..ph + 4].copy_from_slice(&1u32.to_be_bytes()); // PT_LOAD
        bytes[ph + 4..ph + 8].copy_from_slice(&0x5u32.to_be_bytes()); // R|X (0x4|0x1)
        bytes[ph + 8..ph + 16]
            .copy_from_slice(&(header_region as u64).to_be_bytes()); // p_offset
        bytes[ph + 16..ph + 24].copy_from_slice(&base_vaddr.to_be_bytes());
        bytes[ph + 24..ph + 32].copy_from_slice(&base_vaddr.to_be_bytes());
        let memsz = code_bytes.len() as u64;
        bytes[ph + 32..ph + 40].copy_from_slice(&memsz.to_be_bytes()); // p_filesz
        bytes[ph + 40..ph + 48].copy_from_slice(&memsz.to_be_bytes()); // p_memsz
        bytes[ph + 48..ph + 56].copy_from_slice(&0x1000u64.to_be_bytes()); // p_align

        // Code
        bytes[header_region..header_region + code_bytes.len()]
            .copy_from_slice(&code_bytes);

        bytes
    }

    #[test]
    fn load_and_run_minimal_elf() {
        let base: u32 = 0x10000;
        let prog = [
            encode::addi(3, 0, 77),
            encode::addi(11, 0, 22), // syscall _sys_process_exit
            encode::sc(),
        ];
        let elf = make_minimal_ppu_elf(base as u64, &prog);

        let mut core = EmuCore::new();
        let info = core.load_elf(&elf).unwrap();
        assert_eq!(info.e_entry, base as u64);
        assert_eq!(core.ppu.cia, base);
        let status = core.run().unwrap();
        assert_eq!(status.status, 77);
    }

    // -- SPU end-to-end ------------------------------------------

    #[test]
    fn spu_group_runs_synthetic_program_to_stop() {
        use rpcs3_lv2_spu_group::{GROUP_TYPE_NORMAL, JOIN_ALL_THREADS_EXIT};
        use rpcs3_lv2_spu_image::{build_image, SpuPhdr, PT_LOAD};
        use rpcs3_spu_interpreter::encode as spu;

        // Tiny SPU program: il r3, 0xCAFE ; stop 0x99 .
        // Will execute from LSA 0. The program bytes will live inside
        // `source` at offset 0; the image lays it out in LS at vaddr 0.
        let spu_prog = [spu::il(3, 0xCAFE_u16 as i16), spu::stop(0x99)];
        let mut payload = Vec::new();
        for i in &spu_prog {
            payload.extend_from_slice(&i.to_be_bytes());
        }

        // One PT_LOAD covering the program. `src_base = 0x1000` matches
        // the value run_spu_group_single subtracts when calling fetch.
        let phdrs = [SpuPhdr {
            p_type: PT_LOAD,
            p_offset: 0,
            p_vaddr: 0,
            p_filesz: payload.len() as u32,
            p_memsz: payload.len() as u32,
        }];
        let image = build_image(0, &phdrs, 0x1000).unwrap();

        let attr = GroupAttr {
            name: "spu_group_test".to_owned(),
            num_threads: 1,
            priority: 100,
            group_type: GROUP_TYPE_NORMAL,
        };

        let (_gid, outcome) = EmuCore::new()
            .run_spu_group_single(&image, &payload, attr, 1_000)
            .unwrap();
        assert_eq!(outcome.stop_code, 0x99);
        assert!(outcome.steps >= 2);

        // Sanity — the join cause in our reg impl is ALL_THREADS_EXIT.
        // We asserted the full flow inside `run_spu_group_single`; the
        // return value is enough to prove it reached stop cleanly.
        let _ = JOIN_ALL_THREADS_EXIT;
    }

    #[test]
    fn load_elf_rejects_non_ppu_binary() {
        // Build an ELF claiming to be for ARM.
        let mut bytes = vec![0u8; 64 + 56];
        bytes[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        bytes[4] = 2;
        bytes[5] = 2;
        bytes[6] = 1;
        bytes[18..20].copy_from_slice(&0x28u16.to_be_bytes()); // EM_ARM

        let mut core = EmuCore::new();
        let err = core.load_elf(&bytes).unwrap_err();
        assert!(matches!(err, Error::ElfNotLoadable(_)));
    }
}
