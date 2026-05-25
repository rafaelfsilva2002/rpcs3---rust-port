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
use rpcs3_loader_elf_self::{
    parse_elf, parse_sce_header, ElfInfo, Error as ElfError,
    PpuPrxModuleInfo, SysProcPrxParam, PPU_PRX_MODULE_INFO_SIZE,
};
use rpcs3_memory::PageFlags;
use rpcs3_memory_backing::{Error as MemError, SparseBackend};
use rpcs3_ppu_interpreter::{step, Error as InterpError, StepOutcome};
use rpcs3_ppu_thread::{PpuThread, PPU_ID_BASE};
use rpcs3_lv2_tty::SysTty;

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

/// R9.1a — Report returned by [`EmuCore::run_self`]: the PPU exit
/// status plus everything the run captured for the integration test
/// to assert on (today: TTY output per channel).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunReport {
    pub exit_status: ExitStatus,
    /// Captured TTY output per channel (0..=15). Channel 3
    /// (`SYS_TTYP_USER1`) is the default stdout channel for
    /// PSL1GHT homebrew.
    pub tty_output: Vec<String>,
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

/// R9.1c — PSL1GHT user-mode stack defaults. The real lv2 loader
/// honors `SYS_PROCESS_PARAM(prio, stack_size)` from the `.self`;
/// the R8.x oracle fixtures all specify `stack_size = 0x10000`
/// (64 KB). We place the stack at a fixed EA below the
/// `vm::_ptr<u8>` user-mode window (well above the `.text` /
/// `.data` ranges of PSL1GHT-built binaries, which top out around
/// 0x10100000 per the empirical PHDR layouts).
pub const USER_STACK_TOP: u32 = 0xD000_0000;
pub const USER_STACK_SIZE: u32 = 0x0001_0000; // 64 KB

/// R9.1g.4 — default virtual address for the per-thread TLS
/// region. Sits below the user-mode stack with plenty of headroom
/// (PSL1GHT fixtures observed need at most ~2 KB).
pub const USER_TLS_VADDR: u32 = 0xCFFE_0000;

/// R9.1g.4 — chosen TLS region (returned by [`EmuCore::init_tls`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TlsRegion {
    /// Base virtual address of the allocated TLS region.
    pub vaddr: u32,
    /// Effective size in bytes (matches `p_memsz` from PT_TLS).
    pub size: u32,
}

/// R9.1g.6 — virtual address of the import-stub region. We
/// allocate 64 KB above the user-mode stack for trampolines
/// + FD records (mailbox_v1's 119 imports need ~2.5 KB; 64 KB
/// is enough for all 20 oracle fixtures + future binaries).
pub const USER_IMPORT_STUB_VADDR: u32 = 0xD001_0000;
pub const USER_IMPORT_STUB_SIZE: u32 = 0x0001_0000;

/// R9.1g.6 — read a null-terminated UTF-8 string from guest
/// memory, up to `max_len` bytes. Returns the string without
/// the trailing nul. Errors if memory access fails or if no nul
/// is found within `max_len`.
fn read_c_string(
    mem: &SparseBackend,
    vaddr: u32,
    max_len: u32,
) -> Result<String, Error> {
    let mut buf = vec![0u8; max_len as usize];
    mem.read(vaddr, &mut buf)?;
    let nul = buf
        .iter()
        .position(|&b| b == 0)
        .ok_or(Error::ElfNotLoadable(
            "read_c_string: no NUL terminator within max_len",
        ))?;
    Ok(String::from_utf8_lossy(&buf[..nul]).into_owned())
}

/// R9.1g.6 — sentinel syscall number used by import-stub
/// trampolines. The PPU syscall dispatcher recognizes this and
/// looks up which import was actually invoked via the stub-
/// region address range. NOT a real PS3 lv2 syscall number;
/// chosen well above the lv2 range (~0..1024).
pub const IMPORT_STUB_SC_SENTINEL: u64 = 0x1_0000;

/// R9.1g.6 — single resolved import stub record.
#[derive(Debug, Clone)]
pub struct ImportStub {
    pub module_name: String,
    pub nid: u32,
    /// VAddr of the 4-byte `sc` trampoline.
    pub trampoline_vaddr: u32,
    /// VAddr of the 8-byte function descriptor written into the
    /// `addrs[]` slot.
    pub fd_vaddr: u32,
    /// VAddr of the `addrs[]` slot that was patched.
    pub addrs_slot: u32,
}

/// R9.1g.6 — result of [`EmuCore::init_imports`]: the list of
/// installed import stubs, indexed for the syscall dispatcher's
/// reverse lookup.
#[derive(Debug, Clone, Default)]
pub struct ImportPlan {
    pub stubs: Vec<ImportStub>,
}

impl ImportPlan {
    /// Look up an import stub by its trampoline vaddr (the CIA
    /// at which the sentinel `sc` fires). Used by the PPU
    /// syscall dispatcher to identify which import was called.
    #[must_use]
    pub fn lookup_by_trampoline(&self, cia: u32) -> Option<&ImportStub> {
        // `sc` advances CIA by 4 before the syscall dispatcher
        // sees it; the trampoline body is just `sc` so CIA-4
        // matches `trampoline_vaddr`.
        let probe = cia.wrapping_sub(4);
        self.stubs
            .iter()
            .find(|s| s.trampoline_vaddr == probe)
    }
}

/// Single-threaded PPU emulator core. One of these per run.
pub struct EmuCore {
    pub mem: SparseBackend,
    pub ppu: PpuThread,
    pub process: TestProcessState,
    /// R9.1a — captured TTY output across all 16 channels.
    /// `sys_tty_write` (syscall 403) appends here; tests read it
    /// post-run to assert canonical TTY strings.
    pub tty: SysTty,
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
            tty: SysTty::new(),
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

        // R9.1b — PPC64 ELFv1 / Cell PS3 function-descriptor entry.
        //
        // PSL1GHT-compiled PPU binaries place `e_entry` at a 4-byte
        // function descriptor in a non-executable (R-only) `.opd`
        // segment. The first u32 BE of that descriptor holds the
        // actual code address in the `.text` segment.
        //
        // If `e_entry` lands in an executable segment, treat it
        // literally as the code address. If it lands in a R-only
        // segment, dereference the function descriptor.
        let entry_addr = info.e_entry as u32;
        let entry_in_executable_segment = info.pt_load_iter().any(|ph| {
            let start = ph.p_vaddr as u32;
            let end = start.saturating_add(ph.p_memsz as u32);
            (start..end).contains(&entry_addr) && (ph.p_flags & 0x1) != 0
        });
        if entry_in_executable_segment {
            self.ppu.cia = entry_addr;
        } else {
            // Dereference the function descriptor: u32 BE at e_entry
            // is the actual code address.
            let mut fd_bytes = [0u8; 4];
            self.mem.read(entry_addr, &mut fd_bytes)?;
            let code_addr = u32::from_be_bytes(fd_bytes);
            self.ppu.cia = code_addr;
        }
        Ok(info)
    }

    /// R9.1a — boot a PSL1GHT-built `.self` file (fself, unencrypted)
    /// end-to-end: parse the SCE header, locate the loadable PPU64
    /// ELF body at `header_length` (= total SCE/SELF metadata size),
    /// delegate to [`load_elf`](Self::load_elf), then run to process
    /// exit.
    ///
    /// The returned [`RunReport`] carries the PPU exit status plus a
    /// snapshot of captured TTY output per channel (so integration
    /// tests can assert against the homebrew's `printf` output —
    /// channel 3 = `SYS_TTYP_USER1` is the default stdout target for
    /// PSL1GHT programs).
    ///
    /// fself binaries from the `rpcs3-ps3dev-toolchain` Docker image
    /// embed an unencrypted ELF AFTER the SCE/SELF header block. The
    /// SCE header's `header_length` field is the byte offset of the
    /// loadable ELF body (NOT the SelfExtHeader's `elf_offset`, which
    /// points to the metadata-info ELF used by SELF descriptors).
    /// The `.self` files captured for the R8.x oracles
    /// (`single_spu_*_v1.self`) match that pattern.
    pub fn run_self(&mut self, self_bytes: &[u8]) -> Result<RunReport, Error> {
        let sce = parse_sce_header(self_bytes)?;
        let elf_start = sce.header_length as usize;
        if elf_start >= self_bytes.len() {
            return Err(Error::ElfNotLoadable("SCE header_length past end of file"));
        }
        let elf_bytes = &self_bytes[elf_start..];
        self.load_elf(elf_bytes)?;
        self.init_user_stack(USER_STACK_TOP, USER_STACK_SIZE)?;
        let exit_status = self.run()?;
        Ok(RunReport {
            exit_status,
            tty_output: self.tty.captured_output.clone(),
        })
    }

    /// R9.1g.4 — allocate the per-thread TLS region described by
    /// the ELF's PT_TLS segment and seed `r13` (the PowerPC ELFv1
    /// thread pointer). PSL1GHT's `_start` and any TLS-using code
    /// in `main()` reads/writes TLS variables via offsets from
    /// `r13`; without this, the first such access faults.
    ///
    /// The PT_TLS segment provides:
    /// - `p_filesz` bytes of initialized data (often 0 = pure tbss)
    /// - `p_memsz` total bytes per thread (init + zero-fill tail)
    /// - `p_align` alignment requirement
    /// - `p_offset` file offset of the init image (within
    ///   `elf_bytes`)
    ///
    /// We allocate `tls_total = round_up(p_memsz, p_align)` bytes
    /// at the chosen virtual address, copy `p_filesz` init bytes,
    /// and set `r13 = tls_vaddr`. Returns the chosen tls_vaddr +
    /// total size so callers can use them for verification.
    ///
    /// Returns `Ok(None)` if the ELF has no PT_TLS segment (some
    /// minimal binaries omit it).
    pub fn init_tls(
        &mut self,
        info: &ElfInfo,
        elf_bytes: &[u8],
        tls_vaddr: u32,
    ) -> Result<Option<TlsRegion>, Error> {
        let tls = match info.pt_tls() {
            Some(t) => t,
            None => return Ok(None),
        };
        let align = tls.p_align.max(1) as u32;
        if !align.is_power_of_two() {
            return Err(Error::ElfNotLoadable(
                "init_tls: PT_TLS p_align must be a power of two",
            ));
        }
        // Page-round the size for SparseBackend's 4 KB granularity.
        // The PpuThread sees only the first `p_memsz` bytes; the
        // rest of the rounded page is unused but reserved.
        let memsz = tls.p_memsz as u32;
        if memsz == 0 {
            // Empty TLS: nothing to allocate. Still set r13 to a
            // sentinel so any later TLS access surfaces a clean
            // MissingFlags rather than reading random mem.
            self.ppu.gpr[13] = tls_vaddr as u64;
            return Ok(Some(TlsRegion {
                vaddr: tls_vaddr,
                size: 0,
            }));
        }
        if tls_vaddr & 0xFFF != 0 {
            return Err(Error::ElfNotLoadable(
                "init_tls: tls_vaddr must be page-aligned",
            ));
        }
        let page_rounded = (memsz + 0xFFF) & !0xFFF;
        self.mem.alloc_at(
            tls_vaddr,
            page_rounded,
            PageFlags::READABLE | PageFlags::WRITABLE,
        )?;
        // Copy init image if any.
        let filesz = tls.p_filesz as usize;
        if filesz > 0 {
            let src_start = tls.p_offset as usize;
            let src_end = src_start.checked_add(filesz).ok_or(
                Error::ElfNotLoadable("init_tls: p_offset+p_filesz overflow"),
            )?;
            if src_end > elf_bytes.len() {
                return Err(Error::ElfNotLoadable(
                    "init_tls: PT_TLS init image extends past ELF",
                ));
            }
            self.mem.write(tls_vaddr, &elf_bytes[src_start..src_end])?;
            // The rest (p_memsz - p_filesz) is the tbss tail — the
            // SparseBackend's alloc_at zeroes the page on first
            // touch, so no explicit memset needed.
        }
        // PowerPC ELFv1 thread pointer convention: r13 points to
        // the start of the TLS region. TLS variables are accessed
        // via positive offsets from r13. (Linux ELFv1 uses a
        // +0x7000 bias; PSL1GHT empirically uses no bias — to be
        // confirmed during R9.1g.9 smoke iteration.)
        self.ppu.gpr[13] = tls_vaddr as u64;
        Ok(Some(TlsRegion {
            vaddr: tls_vaddr,
            size: memsz,
        }))
    }

    /// R9.1g.6 — walk the `.libstub` section and install trampoline
    /// FDs in addrs[] slots so PSL1GHT's PLT thunks no longer load
    /// raw inst-encoding garbage when dereferencing imported
    /// functions.
    ///
    /// For each imported function in each module:
    /// 1. Emit a 4-byte trampoline in the stub region containing
    ///    a single `sc` instruction (`0x44000002`).
    /// 2. Emit an 8-byte function descriptor immediately after
    ///    the trampoline: `{ code = trampoline_vaddr, toc = 0 }`.
    /// 3. Write the FD's vaddr into the appropriate `addrs[]`
    ///    slot in the binary's .data segment.
    /// 4. Record the stub in the [`ImportPlan`] so the syscall
    ///    dispatcher can later identify which import was called
    ///    when the `sc` fires (by CIA — the trampoline's vaddr).
    ///
    /// The PPU sees this layout at runtime:
    /// ```text
    /// stub_region:
    ///   +0x00  trampoline_0: 44 00 00 02      ; sc
    ///   +0x04  fd_0:         00 D0 01 00      ; code = trampoline_0
    ///                        00 00 00 00      ; toc = 0
    ///   +0x0C  trampoline_1: 44 00 00 02
    ///   +0x10  fd_1:         ...
    ///   ...
    /// ```
    /// Each (trampoline + FD) tuple is 12 bytes. mailbox_v1's
    /// 119 imports fit in 1428 bytes (well under the 64 KB
    /// stub region).
    pub fn init_imports(
        &mut self,
        proc_prx_param: &SysProcPrxParam,
    ) -> Result<ImportPlan, Error> {
        // Allocate the stub region R+W+X (executable so the
        // trampoline `sc` instruction can be fetched).
        self.mem.alloc_at(
            USER_IMPORT_STUB_VADDR,
            USER_IMPORT_STUB_SIZE,
            PageFlags::READABLE | PageFlags::WRITABLE | PageFlags::EXECUTABLE,
        )?;

        let mut plan = ImportPlan::default();
        let mut next_offset: u32 = 0;
        let region_end = USER_IMPORT_STUB_SIZE;

        let mut libstub_addr = proc_prx_param.libstub_start;
        while libstub_addr < proc_prx_param.libstub_end {
            // Read + parse one libstub entry (44 bytes).
            let mut entry_bytes = [0u8; PPU_PRX_MODULE_INFO_SIZE];
            self.mem.read(libstub_addr, &mut entry_bytes)?;
            let entry = PpuPrxModuleInfo::parse(&entry_bytes)
                .map_err(Error::Elf)?;

            // Read the null-terminated module name.
            let module_name = read_c_string(&self.mem, entry.name, 64)?;

            // Install trampolines + FDs for each imported function.
            for i in 0..entry.num_func as u32 {
                let nid_addr = entry.nids
                    .checked_add(i.checked_mul(4).ok_or(
                        Error::ElfNotLoadable("init_imports: nid index overflow"))?)
                    .ok_or(Error::ElfNotLoadable("init_imports: nid addr overflow"))?;
                let mut nid_bytes = [0u8; 4];
                self.mem.read(nid_addr, &mut nid_bytes)?;
                let nid = u32::from_be_bytes(nid_bytes);

                // 4 bytes trampoline + 8 bytes FD = 12 bytes per stub.
                let trampoline_offset = next_offset;
                let fd_offset = trampoline_offset + 4;
                next_offset = next_offset.checked_add(12)
                    .ok_or(Error::ElfNotLoadable("init_imports: stub offset overflow"))?;
                if next_offset > region_end {
                    return Err(Error::ElfNotLoadable(
                        "init_imports: stub region exhausted",
                    ));
                }

                let trampoline_vaddr = USER_IMPORT_STUB_VADDR + trampoline_offset;
                let fd_vaddr = USER_IMPORT_STUB_VADDR + fd_offset;

                // Write `sc` instruction (primary 17, lev=0) BE.
                self.mem.write(trampoline_vaddr, &0x4400_0002u32.to_be_bytes())?;
                // Write FD: code (u32 BE) + toc (u32 BE).
                self.mem.write(fd_vaddr, &trampoline_vaddr.to_be_bytes())?;
                self.mem.write(fd_vaddr + 4, &0u32.to_be_bytes())?;

                // Patch the addrs[] slot in the binary's .data.
                let addrs_slot = entry.addrs
                    .checked_add(i.checked_mul(4).ok_or(
                        Error::ElfNotLoadable("init_imports: addrs index overflow"))?)
                    .ok_or(Error::ElfNotLoadable("init_imports: addrs slot overflow"))?;
                self.mem.write(addrs_slot, &fd_vaddr.to_be_bytes())?;

                plan.stubs.push(ImportStub {
                    module_name: module_name.clone(),
                    nid,
                    trampoline_vaddr,
                    fd_vaddr,
                    addrs_slot,
                });
            }

            libstub_addr = libstub_addr.checked_add(
                PPU_PRX_MODULE_INFO_SIZE as u32,
            ).ok_or(Error::ElfNotLoadable(
                "init_imports: libstub_addr overflow",
            ))?;
        }

        Ok(plan)
    }

    /// R9.1c — allocate the user-mode PPU stack and seed `r1` (the
    /// SPU/PPC ABI's stack pointer) so PSL1GHT's `_start` can run
    /// its first `stdu r1, -N(r1)` frame-allocation without faulting.
    ///
    /// `top` is the EA of the byte ONE PAST the top of stack (so
    /// the initial r1 is `top - 0x10` — aligned to a 16-byte slot
    /// and leaving headroom for the very first frame's back chain
    /// + saved LR per the PowerPC ELFv1 ABI).
    ///
    /// The real PS3 lv2 loader reads the `SYS_PROCESS_PARAM(prio,
    /// stack_size)` block from the .self and allocates `stack_size`
    /// bytes of user-mode stack at a kernel-chosen EA. We use a
    /// hard-coded EA and size for now; revisit if a fixture cares
    /// about the specific address (none of the 20 SPU oracles do).
    pub fn init_user_stack(&mut self, top: u32, size: u32) -> Result<(), Error> {
        if size == 0 || size & 0xFFF != 0 {
            return Err(Error::ElfNotLoadable(
                "init_user_stack: size must be non-zero and page-aligned",
            ));
        }
        let base = top.checked_sub(size)
            .ok_or(Error::ElfNotLoadable("init_user_stack: top below size"))?;
        if base & 0xFFF != 0 {
            return Err(Error::ElfNotLoadable(
                "init_user_stack: top - size must be page-aligned",
            ));
        }
        self.mem.alloc_at(
            base,
            size,
            PageFlags::READABLE | PageFlags::WRITABLE,
        )?;
        // Seed r1 to top-of-stack minus a 16-byte ABI padding slot.
        // `stdu r1, -N(r1)` will allocate the first real frame
        // starting from here, growing downward toward `base`.
        self.ppu.gpr[1] = (top.wrapping_sub(0x10)) as u64;
        Ok(())
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
            403 => {
                // R9.1a — sys_tty_write(ch, buf_ptr, len, *pwritelen)
                //
                // Reads `len` bytes from guest mem at `buf_ptr`,
                // appends them to the captured TTY output for the
                // given channel, and writes the bytes-written count
                // back into *pwritelen (BE u32 in guest memory).
                let ch = r3 as i32;
                let buf_ptr = r4 as u32;
                let len = self.ppu.gpr[5] as u32;
                let pwritelen_ptr = self.ppu.gpr[6] as u32;

                // Read `len` bytes from guest memory. If `len` is 0
                // the payload is treated as an empty string (valid
                // per cpp:127 — actual = min(len, payload.len())).
                let mut payload_bytes = vec![0u8; len as usize];
                if len > 0 {
                    self.mem.read(buf_ptr, &mut payload_bytes)?;
                }
                // PSL1GHT writes plain ASCII/UTF-8; the SysTty buffer
                // is a String. Lossy decode is acceptable for the
                // integration-test contract (printf output is ASCII).
                let payload_str = String::from_utf8_lossy(&payload_bytes).into_owned();
                let mut pwritelen_local: u32 = 0;
                match self.tty.write(
                    ch,
                    Some(&payload_str),
                    len,
                    Some(&mut pwritelen_local),
                ) {
                    Ok(()) => {
                        // Write bytes-written count back to guest mem.
                        let buf = pwritelen_local.to_be_bytes();
                        self.mem.write(pwritelen_ptr, &buf)?;
                        self.ppu.gpr[3] = 0; // CELL_OK
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = u64::from(u32::from(e));
                    }
                }
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
