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
    PpuPrxModuleInfo, SysProcessParam, SysProcPrxParam,
    PPU_PRX_MODULE_INFO_SIZE,
};
use rpcs3_memory::PageFlags;
use rpcs3_memory_backing::{Error as MemError, SparseBackend};
use rpcs3_ppu_interpreter::{step, Error as InterpError, StepOutcome};
use rpcs3_ppu_thread::{CrBits, PpuThread, Xer, PPU_ID_BASE};
use rpcs3_lv2_tty::SysTty;
use rpcs3_lv2_sync::{BlockOutcome, Lv2SyncState, MutexAttr, SemaAttr, SyncTable};
use rpcs3_lv2_event::{EventRegistry, QueueAttr, ReceiveOutcome};
use rpcs3_lv2_cond::{CondAttr, CondRegistry, WaitOutcome as CondWaitOutcome};
use rpcs3_lv2_lwmutex::{
    sys_lwmutex_create as lv2_lwmutex_create,
    sys_lwmutex_lock as lv2_lwmutex_lock,
    sys_lwmutex_unlock as lv2_lwmutex_unlock,
    LockOutcome, LwMutexAttribute, LwMutexControl, LWMUTEX_RECURSIVE,
};

use rpcs3_lv2_process::{
    sys_process_get_number_of_object, sys_process_get_sdk_version, sys_process_getpid,
    sys_process_getppid, ObjectType as ProcObjectType, SyscallResult, TestProcessState,
};

use rpcs3_lv2_spu_group::{
    sys_spu_thread_group_create, sys_spu_thread_group_destroy, sys_spu_thread_group_join,
    sys_spu_thread_group_start, GroupAttr, SpuGroupRegistry, TestSpuGroupRegistry,
};
use rpcs3_lv2_spu_image::{deploy as deploy_image, build_image, SpuImage, SpuPhdr};
use rpcs3_spu_interpreter::{run_n as spu_run_n, StepOutcome as SpuStepOutcome, Error as SpuError};
use rpcs3_spu_thread::SpuThread;
use rpcs3_hle_cellsysutil::{
    cell_sysutil_check_callback, cell_sysutil_get_system_param_int,
    cell_sysutil_get_system_param_string, cell_sysutil_register_callback,
    cell_sysutil_unregister_callback, CallbackQueue, CallbackTable,
};
use rpcs3_hle_cellsysmodule::{
    cell_sysmodule_is_loaded, cell_sysmodule_load_module, SysmoduleManager,
};
use rpcs3_hle_cellvideoout::{
    cell_video_out_get_configuration, cell_video_out_get_number_of_device,
    cell_video_out_get_resolution, cell_video_out_get_resolution_availability,
    cell_video_out_get_state, VideoOutManager, VideoOutState,
};
use rpcs3_hle_sys_net_user::inet_addr_stub;
use rpcs3_hle_cellnetctl::{
    cell_net_ctl_get_info, cell_net_ctl_get_state, cell_net_ctl_init, NetCtlManager,
    NetInfo, StubConnectedBackend,
};
use rpcs3_hle_cellgame::cell_game_get_param_int;
use rpcs3_hle_cellmsgdialog::{
    cell_msg_dialog_close, cell_msg_dialog_open, last_button as msg_last_button,
    DialogManager, TypeFlags as MsgTypeFlags,
};
use rpcs3_lv2_fs::{
    sys_fs_close, sys_fs_lseek, sys_fs_open, sys_fs_read, sys_fs_stat, CellFsStat, FdTable,
};

mod vfs;
use vfs::MemVfs;

#[cfg(feature = "spu-recompiler")]
use rpcs3_spu_differential::{ExecutionStopReason, SpuExecutor, SpuProgram};
#[cfg(feature = "spu-recompiler")]
use rpcs3_spu_recompiler::RecompilerExecutor;

/// Which backend executes SPU thread code. Default `Interpreter`. The
/// `Recompiler` (Cranelift JIT) variant is honored only when built with the
/// `spu-recompiler` feature; otherwise SPU code always runs on the
/// interpreter. Per docs/PORT_STATUS_AND_ROADMAP.md §4.1–4.2 the JIT only
/// wins on hot loops, so it is opt-in, not the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpuBackend {
    #[default]
    Interpreter,
    Recompiler,
}

/// Fixed system-config provider backing HLE `cellSysutil` param lookups
/// (R13.6, first HLE-crate integration). Values mirror RPCS3's default system
/// parameters (English-US language, cross-confirm, etc.). Stateless: the params
/// are fixed config, so a unit struct suffices (no EmuCore field needed yet).
struct EmuSysutilConfig;

impl rpcs3_hle_cellsysutil::SysutilState for EmuSysutilConfig {
    fn get_param_int(&self, id: rpcs3_hle_cellsysutil::SysParamId) -> Option<i32> {
        use rpcs3_hle_cellsysutil::SysParamId as P;
        match id {
            P::Lang => Some(1), // CELL_SYSUTIL_LANG_ENGLISH_US
            P::EnterButtonAssign => Some(1),
            P::DateFormat => Some(1),
            P::TimeFormat => Some(1),
            P::Timezone => Some(0),
            P::Summertime => Some(0),
            P::GameParentalLevel => Some(1),
            P::CurrentUserHasNpAccount => Some(0),
            P::CameraPlfreq => Some(0),
            P::PadAutoOff => Some(0),
            _ => None,
        }
    }
    fn get_param_string(&self, id: rpcs3_hle_cellsysutil::SysParamId) -> Option<&str> {
        use rpcs3_hle_cellsysutil::SysParamId as P;
        match id {
            // Default account nickname / username. Fixed config, RPCS3-style.
            P::Nickname | P::CurrentUsername => Some("RPCS3"),
            _ => None,
        }
    }
    fn media_ver(&self) -> &str {
        "04.5500"
    }
}

/// HLE wave — fixed cellGame content/PSF provider. A homebrew run via `run_self`
/// has no real game content, so these are RPCS3-style deterministic defaults.
/// Only the PSF parental level is exercised by a fixture today; the rest are
/// safe placeholders for the (not-yet-wired) BootCheck/ContentPermit/DataCheck.
struct EmuGameConfig;

impl rpcs3_hle_cellgame::GameState for EmuGameConfig {
    fn game_type(&self) -> rpcs3_hle_cellgame::GameType {
        rpcs3_hle_cellgame::GameType::Hdd
    }
    fn attributes(&self) -> u32 {
        0
    }
    fn size_kb(&self) -> u64 {
        0
    }
    fn dir_name(&self) -> &str {
        ""
    }
    fn content_info_path(&self) -> &str {
        ""
    }
    fn usrdir_path(&self) -> &str {
        ""
    }
    fn psf_string(&self, _id: rpcs3_hle_cellgame::GameParamId) -> Option<&str> {
        None
    }
    fn psf_int(&self, id: rpcs3_hle_cellgame::GameParamId) -> Option<i32> {
        use rpcs3_hle_cellgame::GameParamId as P;
        match id {
            // PSF PARENTAL_LEVEL — least-restrictive non-zero default.
            P::ParentalLevel => Some(1),
            _ => None,
        }
    }
    fn game_data_exists(&self, _dir_name: &str) -> bool {
        false
    }
}

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
    /// A guest-callback invocation (`call_guest_function`) ran past the
    /// step budget without returning to the sentinel — the callback
    /// likely faulted or looped. The register frame is restored before
    /// this is surfaced.
    CallbackStepsExhausted,
    /// The ELF file doesn't have a usable PT_LOAD within the PPU main
    /// RAM window.
    ElfNotLoadable(&'static str),
    /// An error surfaced from the SPU interpreter.
    Spu(SpuError),
    /// A SPU thread group syscall failed.
    SpuGroup(CellError),
    /// R10.1.c — a syscall received an argument the wrapper rejected
    /// with `CellError::EINVAL` (e.g. a malformed attribute struct).
    /// Carries the original error so the NID handler can route it back
    /// to the guest's `r3` register.
    SyscallEinval(CellError),
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
            Error::CallbackStepsExhausted => {
                f.write_str("guest callback exceeded the step budget")
            }
            Error::ElfNotLoadable(r) => write!(f, "ELF not loadable: {r}"),
            Error::Spu(e) => write!(f, "SPU: {e:?}"),
            Error::SpuGroup(e) => write!(f, "SPU group: cell error 0x{:08x}", e.0),
            Error::SyscallEinval(e) => write!(f, "syscall EINVAL: cell error 0x{:08x}", e.0),
        }
    }
}

impl std::error::Error for Error {}

/// Return-sentinel `lr` value installed before invoking a guest callback
/// via [`EmuCore::call_guest_function`]. 4-byte aligned, OUTSIDE the
/// import-stub window (`0xD0010000..0xD0020000`), and never a mapped code
/// page — the callback's terminal `blr` lands `cia` here, which the
/// nested run loop detects to stop (the check runs BEFORE `step`, so the
/// sentinel is never executed).
pub const GUEST_CALLBACK_SENTINEL: u32 = 0xD0FF_0000;

/// Snapshot of the full architectural PPU register frame, taken before a
/// re-entrant guest-callback invocation and restored after it. `PpuThread`
/// is not `Clone` (it carries a `CpuState`), so the arch registers a
/// callback could clobber are copied out individually. Memory is
/// deliberately NOT snapshotted — guest writes during the callback are the
/// observable behavior we keep.
struct PpuRegSnapshot {
    gpr: [u64; 32],
    fpr: [f64; 32],
    vr: [u128; 32],
    cr: CrBits,
    fpscr: u32,
    lr: u64,
    ctr: u64,
    xer: Xer,
    cia: u32,
    vrsave: u32,
    nj: bool,
}

/// Serialize a `CellFsStat` to its 52-byte big-endian on-the-wire layout
/// (`sysFSStat`, `__attribute__((packed))`): mode@0(u32) uid@4(s32) gid@8(s32)
/// atime@12(s64) mtime@20 ctime@28 size@36(u64) blksize@44. Hand-rolled — the
/// host `CellFsStat` is natural-rep with no serialization helper.
fn cellfsstat_to_be(st: &CellFsStat) -> [u8; 52] {
    let mut b = [0u8; 52];
    b[0..4].copy_from_slice(&st.mode.to_be_bytes());
    b[4..8].copy_from_slice(&st.uid.to_be_bytes());
    b[8..12].copy_from_slice(&st.gid.to_be_bytes());
    b[12..20].copy_from_slice(&st.atime.to_be_bytes());
    b[20..28].copy_from_slice(&st.mtime.to_be_bytes());
    b[28..36].copy_from_slice(&st.ctime.to_be_bytes());
    b[36..44].copy_from_slice(&st.size.to_be_bytes());
    b[44..52].copy_from_slice(&st.blksize.to_be_bytes());
    b
}

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
pub const USER_STACK_SIZE: u32 = 0x0010_0000; // R9.1g.9 — 1 MB (PSL1GHT crt0 needs more than 64 KB)

/// R10.1.d — synthetic thread id used for every LV2 sync syscall in
/// the single-PPU model. PSL1GHT crt0 only exercises one PPU thread;
/// this constant is what gets passed as `tid` to `Lv2SyncState`
/// `mutex_lock`/`mutex_unlock`/etc so the registry's ownership /
/// reentrancy checks pass. When R11+ adds multi-PPU support, this
/// becomes per-PpuThread.
pub const PPU_THREAD_TID: u32 = 1;

/// R9.1g.4 — default virtual address for the per-thread TLS
/// region. Sits below the user-mode stack with plenty of headroom
/// (PSL1GHT fixtures observed need at most ~2 KB). R9.1g.9
/// repositioned this below the 1 MB stack region to avoid overlap.
pub const USER_TLS_VADDR: u32 = 0xCFE0_0000;

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
    /// R9.1g.6/.7 — resolved import stubs (set by
    /// [`EmuCore::init_imports`]). When the PPU `sc` instruction
    /// fires from a stub trampoline, the dispatcher looks up the
    /// stub here to either dispatch the import or return-to-caller.
    pub import_plan: Option<ImportPlan>,
    /// R9.1h — bump-allocator cursor for sys_mmapper_allocate_address.
    /// Each call advances by `max(size, alignment)`, returning the
    /// previous (aligned) cursor as the allocated base.
    pub mmapper_alloc_cursor: u32,
    /// R9.1g.9 — when true, unknown syscall numbers log + return
    /// CELL_OK instead of bubbling as Error::UnsupportedSyscall.
    /// `run_self` sets this so PSL1GHT binaries can advance
    /// through unimplemented syscalls during the iterative
    /// R9.1g.9 loop. Unit tests that intentionally probe the
    /// strict-fail path (e.g. `unsupported_syscall_bubbles_up_as_error`)
    /// keep the default `false`.
    pub permissive_unknown_syscalls: bool,
    /// R9.1g.10 — captured SPU image from
    /// `_sys_spu_image_import` (157). Stored across the
    /// `sys_spu_thread_initialize` / `_group_start` lifecycle
    /// so `_group_start` can deploy + run the SPU.
    pub spu_image: Option<SpuImage>,
    /// R9.1g.10 — PPU vaddr where the SPU ELF source bytes
    /// live. The `deploy()` closure reads `p_offset`-relative
    /// chunks from this base.
    pub spu_image_src_vaddr: u32,
    /// R9.1g.10 — captured SPU thread args from
    /// `sys_spu_thread_initialize` (172). PSL1GHT convention:
    /// arg0→r3, arg1→r4, arg2→r5, arg3→r6 (each 64-bit).
    pub spu_thread_args: [u64; 4],
    /// R9.1g.10 — OUT_MBOX value the SPU thread group emitted
    /// before halting. Read by `sys_spu_thread_group_join` (178)
    /// to populate the caller's `*status` pointer with the
    /// canonical TTY status the binary expects.
    pub spu_exit_status: Option<u32>,
    /// Which backend runs SPU thread code (default `Interpreter`).
    /// `Recompiler` is honored only under the `spu-recompiler` feature.
    pub spu_backend: SpuBackend,
    /// Maximum steps per `run` invocation. 0 = unbounded.
    pub step_budget: usize,
    /// R10.1.b — per-run LV2 sync primitive registry. Owns lwmutex
    /// (+ future mutex/sema/cond/event/rwlock) handle pool. State is
    /// per-`EmuCore`, never shared; reset on `EmuCore::new`.
    pub lv2_sync_state: Lv2SyncState,
    /// R13.1 — cellGcm HLE state. `_cellGcmInitBody` allocates the
    /// `CellGcmContextData` + `CellGcmControl` + records the io
    /// region; `cellGcmGetControlRegister` / `GetConfiguration`
    /// return these. 0 until init. Layout mirrors RPCS3
    /// `cellGcmSys.cpp` / `Emu/RSX/GCM.h`.
    pub gcm_context_addr: u32,
    pub gcm_control_addr: u32,
    pub gcm_io_address: u32,
    pub gcm_io_size: u32,
    /// HLE wave — cellSysModule dynamic-loader state. Refcount registry
    /// (load +1, unload -1, is_loaded != 0 iff outstanding refs). Stateful
    /// across guest calls; mirrors `lv2_sync_state`. Reset on `EmuCore::new`.
    pub sysmodule: SysmoduleManager,
    /// HLE wave — cellNetCtl manager (gates `initialized`). Paired with a fixed
    /// connected-network provider (`StubConnectedBackend`) so the emulated
    /// console reports an established network.
    pub netctl: NetCtlManager,
    /// HLE wave — cellVideoOut manager (supported-resolution table + per-port
    /// config/state). Backs GetResolutionAvailability (and future GetState /
    /// GetConfiguration / GetDeviceInfo arms).
    pub videoout: VideoOutManager,
    /// HLE wave — cellSysutil callback registry (slot -> guest fn ptr +
    /// userdata) + a pending-event queue. `cellSysutilCheckCallback` drains the
    /// queue and invokes each registered guest callback via
    /// [`EmuCore::call_guest_function`] (the first guest-PPU-callback consumer).
    pub sysutil_callbacks: CallbackTable,
    pub sysutil_queue: CallbackQueue,
    /// Callback wave — cellMsgDialog state. `cellMsgDialogOpen2` opens it and
    /// (headless) auto-confirms, invoking the guest dialog callback via
    /// [`EmuCore::call_guest_function`] with the default button.
    pub msgdialog: DialogManager,
    /// VFS wave — in-memory filesystem backing the lv2 `sys_fs_*` syscalls.
    /// Tests pre-seed files via [`EmuCore::vfs_add_file`] before `run_self`.
    pub vfs: MemVfs,
    /// VFS wave — lv2 file-descriptor table (fds start at 4; 0..3 = stdio).
    /// Persists across open/read/close within one run.
    pub fd_table: FdTable,
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
            import_plan: None,
            mmapper_alloc_cursor: 0x4000_0000,
            permissive_unknown_syscalls: false,
            spu_image: None,
            spu_image_src_vaddr: 0,
            spu_thread_args: [0; 4],
            spu_exit_status: None,
            spu_backend: SpuBackend::Interpreter,
            step_budget: 100_000,
            lv2_sync_state: Lv2SyncState::new(),
            gcm_context_addr: 0,
            gcm_control_addr: 0,
            gcm_io_address: 0,
            gcm_io_size: 0,
            sysmodule: SysmoduleManager::default(),
            netctl: NetCtlManager::default(),
            videoout: VideoOutManager::default(),
            sysutil_callbacks: CallbackTable::default(),
            sysutil_queue: CallbackQueue::default(),
            msgdialog: DialogManager::default(),
            vfs: MemVfs::new(),
            fd_table: FdTable::new(),
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
        // R9.1g.9 — enable permissive-syscall mode so unknown
        // lv2 syscalls log + return 0 instead of crashing the
        // PPU. PSL1GHT binaries hit many syscalls beyond the
        // ones we have explicit arms for.
        self.permissive_unknown_syscalls = true;

        let sce = parse_sce_header(self_bytes)?;
        let elf_start = sce.header_length as usize;
        if elf_start >= self_bytes.len() {
            return Err(Error::ElfNotLoadable("SCE header_length past end of file"));
        }
        let elf_bytes = &self_bytes[elf_start..];
        let info = self.load_elf(elf_bytes)?;

        // R9.1g.8 — PSL1GHT runtime init pipeline. Parses the
        // PSL1GHT-specific PT_SCE segments and wires up the
        // process state lv2 normally provides before _start.

        // Step 1: parse sys_process_param (PT_SCE_PPU_PROCESS_PARAM)
        // to extract the configured stack size. **R9.1g.9 caveat:**
        // PSL1GHT binaries declare `primary_stacksize = 0x10000`
        // (64 KB, per SYS_PROCESS_PARAM macro), but the actual
        // crt0 + libc + main paths reliably exceed that. Honor a
        // floor of USER_STACK_SIZE (1 MB) regardless of binary
        // declaration to avoid frame-overflow MissingFlags faults
        // that look like memory bugs but are really just under-
        // sized stack.
        let stack_size = if let Some(ph) = info.pt_proc_param() {
            let off = ph.p_offset as usize;
            let end = off
                .checked_add(ph.p_filesz as usize)
                .ok_or(Error::ElfNotLoadable(
                    "run_self: sys_process_param offset overflow",
                ))?;
            if end > elf_bytes.len() {
                return Err(Error::ElfNotLoadable(
                    "run_self: sys_process_param past ELF end",
                ));
            }
            let pp = SysProcessParam::parse(&elf_bytes[off..end])?;
            pp.primary_stacksize.max(USER_STACK_SIZE)
        } else {
            USER_STACK_SIZE
        };

        // Step 2: allocate the user-mode stack + seed r1.
        self.init_user_stack(USER_STACK_TOP, stack_size)?;

        // Step 2b: pre-allocate the heap region for mmapper
        // syscalls (R9.1g.9).
        self.init_user_heap()?;

        // Step 3: TLS init if the binary has a PT_TLS segment.
        // R9.1g.4: ignores empty TLS, sets r13 to a sentinel.
        self.init_tls(&info, elf_bytes, USER_TLS_VADDR)?;

        // Step 4: parse proc_prx_param + install import stubs
        // (R9.1g.5/.6). The PSL1GHT crt0 dereferences the
        // resolved addrs[] table immediately on entry to its
        // first imported library call, so this must happen
        // BEFORE we begin PPU execution.
        if let Some(ph) = info.pt_proc_proc_param() {
            let off = ph.p_offset as usize;
            let end = off
                .checked_add(ph.p_filesz as usize)
                .ok_or(Error::ElfNotLoadable(
                    "run_self: proc_prx_param offset overflow",
                ))?;
            if end > elf_bytes.len() {
                return Err(Error::ElfNotLoadable(
                    "run_self: proc_prx_param past ELF end",
                ));
            }
            let prx = SysProcPrxParam::parse(&elf_bytes[off..end])?;
            let plan = self.init_imports(&prx)?;
            eprintln!(
                "[R9.1g.8] init_imports installed {} stubs across \
                 [0x{:08x}..0x{:08x})",
                plan.stubs.len(),
                USER_IMPORT_STUB_VADDR,
                USER_IMPORT_STUB_VADDR.wrapping_add(USER_IMPORT_STUB_SIZE),
            );
            self.import_plan = Some(plan);
        }

        // Step 5: run the PPU from the entry point (set by
        // load_elf, with R9.1b FD-deref already applied).
        let exit_status = self.run()?;
        Ok(RunReport {
            exit_status,
            tty_output: self.tty.captured_output.clone(),
        })
    }

    /// R9.1g.9 — pre-allocate the PSL1GHT user-mode heap region
    /// at `[0xB0000000, 0xB2000000)` (32 MB) with R+W flags.
    /// PSL1GHT's crt0 calls multiple mmapper syscalls
    /// (sys_mmapper_allocate_address / _shared_memory /
    /// _search_and_map / _map_shared_memory) whose stubs return
    /// sentinel addresses in this region. Allocating upfront
    /// avoids per-call alloc bookkeeping; PSL1GHT then touches
    /// pages here as a normal heap.
    pub fn init_user_heap(&mut self) -> Result<(), Error> {
        const HEAP_BASE: u32 = 0xB000_0000;
        const HEAP_SIZE: u32 = 0x0200_0000; // 32 MB
        self.mem.alloc_at(
            HEAP_BASE,
            HEAP_SIZE,
            PageFlags::READABLE | PageFlags::WRITABLE,
        )?;
        Ok(())
    }

    /// R9.1g.4/.9 — allocate the per-thread TLS region described
    /// by the ELF's PT_TLS segment and seed `r13` (the PowerPC
    /// ELFv1 thread pointer). PSL1GHT's `_start` and any TLS-using
    /// code in `main()` reads/writes TLS variables via offsets
    /// from `r13`; without this, the first such access faults.
    ///
    /// PSL1GHT empirically uses the Linux ELFv1 TLS convention:
    /// `r13` is biased `+0x7000` above the actual TLS storage,
    /// and variables are accessed via negative offsets in the
    /// `[-0x8000, -0x4000)` range. R9.1g.9 widens the
    /// allocation to a generous `[r13 - 0x8000, r13 + 0x1000)`
    /// window (9 pages, 36 KB) — covers both negative-biased
    /// access AND positive offsets the linker may emit for
    /// large TLS regions.
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
        // R9.1g.9 — allocate a 36 KB window around `tls_vaddr`
        // (9 pages: 8 below for the negative-biased -0x8000 access
        // pattern, 1 above for any positive-offset use).
        const PPC_TP_NEGATIVE_BIAS: u32 = 0x8000;
        const PPC_TP_POSITIVE_HEADROOM: u32 = 0x1000;
        let alloc_base = tls_vaddr.checked_sub(PPC_TP_NEGATIVE_BIAS).ok_or(
            Error::ElfNotLoadable("init_tls: tls_vaddr too low for TP bias"),
        )?;
        let alloc_size = PPC_TP_NEGATIVE_BIAS + PPC_TP_POSITIVE_HEADROOM;
        self.mem.alloc_at(
            alloc_base,
            alloc_size,
            PageFlags::READABLE | PageFlags::WRITABLE,
        )?;
        // Copy init image if any — the TLS storage proper sits
        // at `tls_vaddr - 0x7000` per the Linux ELFv1 PPC TP
        // bias convention (= where -0x7000(r13) lands).
        const PPC_TP_TLS_BIAS: u32 = 0x7000;
        let tls_storage_vaddr = tls_vaddr - PPC_TP_TLS_BIAS;
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
            self.mem.write(tls_storage_vaddr, &elf_bytes[src_start..src_end])?;
            // The rest (p_memsz - p_filesz) is the tbss tail — the
            // SparseBackend's alloc_at zeroes the page on first
            // touch, so no explicit memset needed.
        }
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

    /// Invoke a guest PPU function (e.g. a registered HLE callback) and
    /// run it to completion, returning its `r3`. The full architectural
    /// register frame is snapshotted and restored, so the calling HLE arm
    /// can resume the original guest caller; memory side-effects persist.
    ///
    /// `fd_ptr` is a PSL1GHT compact 4-byte function descriptor (the
    /// function pointer the guest handed us, e.g. via RegisterCallback);
    /// its first BE u32 is the real `.text` code address. `args` are
    /// marshalled into `r3..=r10` (max 8, PPC64 GPR arg registers).
    ///
    /// The nested run loop mirrors [`EmuCore::run`] but stops when the
    /// callback returns to [`GUEST_CALLBACK_SENTINEL`]. This makes
    /// `dispatch_syscall` RE-ENTRANT: a callback that itself calls an HLE
    /// import resolves through the same dispatcher. The first re-entrant
    /// control flow in the port — see `.planning/GUEST_CALLBACK_DESIGN.md`.
    pub fn call_guest_function(&mut self, fd_ptr: u32, args: &[u64]) -> Result<u64, Error> {
        // Resolve the compact 4-byte function descriptor -> code address.
        // Guest memory is BIG-endian: raw read + from_be_bytes (never _le).
        // PSL1GHT function-pointer descriptors are full PPC64 OPD entries:
        // {code @0, toc @4, env @8} (4-byte BE fields on PS3's 32-bit address
        // space). Calling through a descriptor sets r2 = descriptor.toc so the
        // callee can reach its TOC-relative globals/literals. The import-stub
        // trampoline path leaves r2 = 0, so we MUST install the callback's own
        // toc here — without it the callback faults on its first TOC load.
        // (The compact 4-byte e_entry FD at boot is a separate special case.)
        let mut fd_bytes = [0u8; 4];
        self.mem.read(fd_ptr, &mut fd_bytes)?;
        let code_addr = u32::from_be_bytes(fd_bytes);
        let mut toc_bytes = [0u8; 4];
        self.mem.read(fd_ptr + 4, &mut toc_bytes)?;
        let toc = u32::from_be_bytes(toc_bytes);

        // Snapshot the full arch register frame (PpuThread is not Clone).
        // Memory is intentionally NOT saved — callback writes must persist.
        let snap = PpuRegSnapshot {
            gpr: self.ppu.gpr,
            fpr: self.ppu.fpr,
            vr: self.ppu.vr,
            cr: self.ppu.cr,
            fpscr: self.ppu.fpscr,
            lr: self.ppu.lr,
            ctr: self.ppu.ctr,
            xer: self.ppu.xer,
            cia: self.ppu.cia,
            vrsave: self.ppu.vrsave,
            nj: self.ppu.nj,
        };

        // Seed the call frame: r2 = callback's TOC, args -> r3..=r10,
        // lr -> sentinel, cia -> code.
        self.ppu.gpr[2] = u64::from(toc);
        for (i, a) in args.iter().take(8).enumerate() {
            self.ppu.gpr[3 + i] = *a;
        }
        self.ppu.lr = u64::from(GUEST_CALLBACK_SENTINEL);
        self.ppu.cia = code_addr;

        // Nested run loop — like `run`, but stops on the sentinel. The
        // guest callback's terminal `blr` sets cia = (lr & !0x3) = sentinel.
        let budget = if self.step_budget == 0 {
            usize::MAX
        } else {
            self.step_budget
        };
        // Capture any step/dispatch error so the frame is ALWAYS restored
        // before it propagates (a faulting callback must not leave the outer
        // caller's registers corrupted).
        let mut hit = false;
        let mut loop_err: Option<Error> = None;
        for _ in 0..budget {
            if self.ppu.cia == (GUEST_CALLBACK_SENTINEL & !0x3) {
                hit = true;
                break;
            }
            match step(&mut self.ppu, &mut self.mem) {
                Ok(StepOutcome::Continue) => {}
                // Re-entrant: a callback that calls an HLE import dispatches
                // here. A callback that exits the process ends the nested loop
                // (pathological; not expected).
                Ok(StepOutcome::Syscall) => match self.dispatch_syscall() {
                    Ok(Some(_)) => break,
                    Ok(None) => {}
                    Err(e) => {
                        loop_err = Some(e);
                        break;
                    }
                },
                Err(e) => {
                    loop_err = Some(Error::from(e));
                    break;
                }
            }
        }

        let ret = self.ppu.gpr[3];

        // Restore the saved frame (even on the budget-exhausted path) so
        // the outer HLE arm's `cia = lr & !3; return Ok(None)` resumes the
        // original caller correctly.
        self.ppu.gpr = snap.gpr;
        self.ppu.fpr = snap.fpr;
        self.ppu.vr = snap.vr;
        self.ppu.cr = snap.cr;
        self.ppu.fpscr = snap.fpscr;
        self.ppu.lr = snap.lr;
        self.ppu.ctr = snap.ctr;
        self.ppu.xer = snap.xer;
        self.ppu.cia = snap.cia;
        self.ppu.vrsave = snap.vrsave;
        self.ppu.nj = snap.nj;

        if let Some(e) = loop_err {
            return Err(e);
        }
        if !hit {
            return Err(Error::CallbackStepsExhausted);
        }
        Ok(ret)
    }

    /// Read a NUL-terminated guest C string (BE-agnostic raw bytes), bounded to
    /// `max` bytes. Reads one byte at a time and stops at the first NUL or read
    /// error, so it never over-reads past a mapped page. Lossy-UTF-8 decoded.
    fn read_guest_cstr(&self, addr: u32, max: usize) -> String {
        let mut bytes = Vec::new();
        for i in 0..max {
            let mut b = [0u8; 1];
            if self.mem.read(addr + i as u32, &mut b).is_err() || b[0] == 0 {
                break;
            }
            bytes.push(b[0]);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Seed an in-memory VFS file BEFORE `run_self` (deterministic stand-in for
    /// on-disk content). The path key must be byte-identical to the path the
    /// guest passes to `sysFsOpen`.
    pub fn vfs_add_file(&mut self, path: &str, data: Vec<u8>) {
        self.vfs.add_file(path, data);
    }

    /// Dispatch the syscall that just triggered. The syscall number
    /// is in `r11` by LV2 convention. Returns `Some(ExitStatus)` if
    /// the program is ending.
    fn dispatch_syscall(&mut self) -> Result<Option<ExitStatus>, Error> {
        let number = self.ppu.gpr[11];
        let cia_at_sc = self.ppu.cia.wrapping_sub(4);

        // R9.1g.7/.11 — if the `sc` fired from inside the import-
        // stub region, this is NOT a real lv2 syscall. It's a PLT
        // thunk hitting an installed trampoline. Look up the
        // import and either:
        //   (a) terminate the process for known-noreturn imports
        //       like sys_process_exit (NID 0xe6f2c1e7); or
        //   (b) return-to-caller with r3 = 0 for everything else.
        // The default "return 0" was the R9.1g.7 MVP; the exit
        // handling is R9.1g.11 — without it, PSL1GHT main's
        // exit() call falls through into trailing padding and
        // the PPU faults on inst=0.
        if self.is_in_import_stub_region(cia_at_sc) {
            if let Some(plan) = self.import_plan.as_ref() {
                if let Some(stub_meta) = plan.lookup_by_trampoline(self.ppu.cia) {
                    let nid = stub_meta.nid;
                    let module = stub_meta.module_name.clone();
                    // Copy the diagnostic fields too, so `stub_meta` (and the
                    // `plan` borrow it derives from) is NOT used past this point.
                    // This frees the immutable `self.import_plan` borrow before
                    // the match, letting arms call `&mut self` methods like
                    // `call_guest_function` (re-entrant guest callbacks).
                    let trampoline_vaddr = stub_meta.trampoline_vaddr;
                    let addrs_slot = stub_meta.addrs_slot;
                    let r3_in = self.ppu.gpr[3];
                    let r4_in = self.ppu.gpr[4];

                    // R9.1g.11 — known-noreturn import terminates.
                    if nid == 0xe6f2c1e7 {
                        let exit_r3 = r3_in as i32;
                        eprintln!(
                            "[R9.1g.11] sys_process_exit (NID 0x{nid:08x}) — \
                             terminating run, exit_status={exit_r3}",
                        );
                        return Ok(Some(ExitStatus { status: exit_r3 }));
                    }

                    // R9.1h — NID-specific minimum-viable
                    // implementations so PSL1GHT crt0 sees the
                    // right post-call memory state and doesn't
                    // bail to its cleanup-and-exit path.
                    match nid {
                        // sys_spinlock_initialize(*lock):
                        // zero the 4-byte spinlock at r3 so
                        // subsequent locks see a valid initialized
                        // state.
                        0x8c2bb498 => {
                            eprintln!(
                                "[R9.1h] sys_spinlock_initialize(*0x{r3_in:x}) \
                                 — zeroing 4 bytes",
                            );
                            self.mem
                                .write(r3_in as u32, &[0u8; 4])?;
                            self.ppu.gpr[3] = 0;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // sys_spinlock_lock(*lock):
                        // single-threaded → no-op.
                        0xa285139d => {
                            eprintln!(
                                "[R9.1h] sys_spinlock_lock(*0x{r3_in:x}) — no-op",
                            );
                            self.ppu.gpr[3] = 0;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // sys_spinlock_unlock(*lock):
                        // single-threaded → no-op.
                        0x5267cb35 => {
                            eprintln!(
                                "[R9.1h] sys_spinlock_unlock(*0x{r3_in:x}) \
                                 — no-op",
                            );
                            self.ppu.gpr[3] = 0;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // sys_mmapper_unmap_memory(addr, *alloc_addr):
                        // pretend success; write 0 to *alloc_addr.
                        0x4643ba6e => {
                            eprintln!(
                                "[R9.1h] sys_mmapper_unmap_memory(0x{r3_in:x}, \
                                 *0x{r4_in:x}) — success",
                            );
                            if r4_in != 0 {
                                self.mem
                                    .write(r4_in as u32, &0u32.to_be_bytes())
                                    .ok();
                            }
                            self.ppu.gpr[3] = 0;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // sys_mmapper_free_memory(mem_id): success.
                        0x409ad939 => {
                            eprintln!(
                                "[R9.1h] sys_mmapper_free_memory(\
                                 mem_id=0x{r3_in:x}) — success",
                            );
                            self.ppu.gpr[3] = 0;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R9.1h slice 2 — newlib `write(fd, buf,
                        // count)` (NID 0x526a496a). Routes any
                        // write to the per-channel TTY buffer
                        // (mapping fd→tty_channel as
                        // 1=stdout→ch3, 2=stderr→ch3 fallback).
                        // Returns count on success.
                        0x526a496a => {
                            let fd = r3_in as i32;
                            let buf_ptr = r4_in as u32;
                            let count = self.ppu.gpr[5] as u32;
                            let ch: i32 = if fd == 2 { 3 } else { 3 };
                            let mut bytes = vec![0u8; count as usize];
                            if count > 0 && self
                                .mem
                                .read(buf_ptr, &mut bytes)
                                .is_ok()
                            {
                                let s =
                                    String::from_utf8_lossy(&bytes)
                                        .into_owned();
                                let mut written: u32 = 0;
                                let _ = self.tty.write(
                                    ch, Some(&s), count, Some(&mut written),
                                );
                                eprintln!(
                                    "[R9.1h] write(fd={fd}, *0x{buf_ptr:x}, \
                                     {count}) → {written} (ch{ch})",
                                );
                            }
                            self.ppu.gpr[3] = count as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // newlib `puts(s)` (NID 0xe3cc73f3): write
                        // NUL-terminated string at r3 to TTY +
                        // append a newline.
                        0xe3cc73f3 => {
                            let str_ptr = r3_in as u32;
                            let s = read_c_string(&self.mem, str_ptr, 4096).unwrap_or_default();
                            let payload = format!("{s}\n");
                            let mut written: u32 = 0;
                            let _ = self.tty.write(
                                3,
                                Some(&payload),
                                payload.len() as u32,
                                Some(&mut written),
                            );
                            eprintln!(
                                "[R9.1h] puts(*0x{str_ptr:x}) → {written}",
                            );
                            self.ppu.gpr[3] = payload.len() as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R10.1.b — PSL1GHT lwmutex family wired to the
                        // real lv2-sync handle pool. PSL1GHT calling
                        // convention (matches PPC ABI):
                        //   r3 = sys_lwmutex_t* (32-byte control)
                        //   r4 = sys_lwmutex_attribute_t* (create) /
                        //        u64 timeout (lock)
                        // The wrappers operate on a host-side
                        // [`LwMutexControl`] mirror; helpers below
                        // round-trip it through guest memory.
                        //
                        // 0x2f85c0ef _sys_lwmutex_create(*lock, *attr).
                        0x2f85c0ef => {
                            let lock_ptr = r3_in as u32;
                            let attr_ptr = r4_in as u32;
                            // R10.1.c — typed attr parser via
                            // `LwMutexAttribute::parse` (handles both
                            // PSL1GHT user form 0x10/0x20/0x30/0x40 and
                            // kernel form 0x01/0x02/0x03/0x04, plus
                            // recursive folding).
                            let attr = match self.read_lwmutex_attr(attr_ptr) {
                                Ok(a) => a,
                                Err(Error::SyscallEinval(e)) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                    self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                                    return Ok(None);
                                }
                                Err(_) => {
                                    self.ppu.gpr[3] =
                                        u64::from(u32::from(CellError::EFAULT));
                                    self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                                    return Ok(None);
                                }
                            };
                            let mut ctrl =
                                LwMutexControl::new(attr.protocol, attr.recursive);
                            match lv2_lwmutex_create(
                                &mut self.lv2_sync_state,
                                attr.protocol,
                                &mut ctrl,
                                attr.recursive,
                            ) {
                                Ok(_id) => {
                                    self.write_lwmutex_control(lock_ptr, &ctrl).ok();
                                    self.ppu.gpr[3] = 0;
                                }
                                Err(e) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                }
                            }
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // 0x1573dc3f _sys_lwmutex_lock(*lock, timeout).
                        0x1573dc3f => {
                            let lock_ptr = r3_in as u32;
                            let timeout = r4_in;
                            let tid = self.ppu.id as u32;
                            let Ok(mut ctrl) = self.read_lwmutex_control(lock_ptr)
                            else {
                                self.ppu.gpr[3] = u64::from(u32::from(CellError::EFAULT));
                                self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                                return Ok(None);
                            };
                            let id = ctrl.sleep_queue();
                            match lv2_lwmutex_lock(
                                &mut self.lv2_sync_state,
                                &mut ctrl,
                                id,
                                tid,
                                timeout,
                            ) {
                                Ok(LockOutcome::Acquired) => {
                                    self.write_lwmutex_control(lock_ptr, &ctrl).ok();
                                    self.ppu.gpr[3] = 0;
                                }
                                Ok(LockOutcome::MustBlock) => {
                                    // Single-PPU model: contention
                                    // requires a parking scheduler we
                                    // don't have. The PSL1GHT crt0
                                    // path is single-threaded, so this
                                    // arm should be unreachable for the
                                    // current oracle set. Surface as
                                    // CELL_EBUSY so the (rare) caller
                                    // can decide; we also persist the
                                    // partial state (waiter+1) which is
                                    // what the wrapper already wrote.
                                    self.write_lwmutex_control(lock_ptr, &ctrl).ok();
                                    self.ppu.gpr[3] = u64::from(u32::from(CellError::EBUSY));
                                }
                                Ok(LockOutcome::Busy) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(CellError::EBUSY));
                                }
                                Err(e) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                }
                            }
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // 0x1bc200f4 _sys_lwmutex_unlock(*lock).
                        0x1bc200f4 => {
                            let lock_ptr = r3_in as u32;
                            let tid = self.ppu.id as u32;
                            let Ok(mut ctrl) = self.read_lwmutex_control(lock_ptr)
                            else {
                                self.ppu.gpr[3] = u64::from(u32::from(CellError::EFAULT));
                                self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                                return Ok(None);
                            };
                            let id = ctrl.sleep_queue();
                            match lv2_lwmutex_unlock(
                                &mut self.lv2_sync_state,
                                &mut ctrl,
                                id,
                                tid,
                            ) {
                                Ok(_handoff) => {
                                    self.write_lwmutex_control(lock_ptr, &ctrl).ok();
                                    self.ppu.gpr[3] = 0;
                                }
                                Err(e) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                }
                            }
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // 0xb257540b sys_mmapper_allocate_memory(
                        //   size, flags, *mem_id_out): write a
                        // unique mem_id and pretend the heap was
                        // allocated.
                        0xb257540b => {
                            self.mmapper_alloc_cursor =
                                self.mmapper_alloc_cursor.wrapping_add(1);
                            let mem_id = self.mmapper_alloc_cursor;
                            let out_ptr = self.ppu.gpr[5] as u32;
                            if out_ptr != 0 {
                                self.mem.write(out_ptr, &mem_id.to_be_bytes())
                                    .ok();
                            }
                            self.ppu.gpr[3] = 0;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // 0xebe5f72f sys_spu_image_import(*image, src, type):
                        // load SPU ELF from src into our captured
                        // SpuImage so subsequent sysSpuThreadGroup_*
                        // calls can run it.
                        0xebe5f72f => {
                            let image_ptr = r3_in as u32;
                            let src_ptr = r4_in as u32;
                            eprintln!(
                                "[R9.1h] sys_spu_image_import(*0x{image_ptr:x}, \
                                 src=0x{src_ptr:x})",
                            );
                            // Read the SPU ELF from PPU memory. The blob can
                            // sit near the end of a mapped segment, so a fixed
                            // 256 KB read may over-run into unmapped pages and
                            // fail — try progressively smaller spans and use
                            // the largest that maps. Every failure mode is now
                            // logged (the previous `if let Ok` swallowed them,
                            // which is why no homebrew SPU ever actually ran
                            // through run_self — see spu_selfcompute_jit test).
                            let mut blob: Vec<u8> = Vec::new();
                            for sz in [0x4_0000usize, 0x1_0000, 0x4000, 0x1000, 0x400] {
                                let mut b = vec![0u8; sz];
                                if self.mem.read(src_ptr, &mut b).is_ok() {
                                    blob = b;
                                    break;
                                }
                            }
                            if blob.is_empty() {
                                eprintln!(
                                    "[R9.1h] sys_spu_image_import: mem.read(0x{src_ptr:x}) \
                                     failed at all sizes — src not mapped?",
                                );
                            } else {
                                match rpcs3_loader_elf_self::parse_elf(&blob) {
                                    Err(e) => eprintln!(
                                        "[R9.1h] parse_elf(0x{src_ptr:x}) failed: {e:?} \
                                         (first bytes: {:02x?})",
                                        &blob[..blob.len().min(8)],
                                    ),
                                    Ok(info) if !info.is_spu() => eprintln!(
                                        "[R9.1h] not an SPU ELF at 0x{src_ptr:x} \
                                         (e_machine mismatch)",
                                    ),
                                    Ok(info) => {
                                        let phdrs: Vec<_> = info
                                            .program_headers
                                            .iter()
                                            .map(|p| rpcs3_lv2_spu_image::SpuPhdr {
                                                p_type: p.p_type,
                                                p_offset: p.p_offset as u32,
                                                p_vaddr: p.p_vaddr as u32,
                                                p_filesz: p.p_filesz as u32,
                                                p_memsz: p.p_memsz as u32,
                                            })
                                            .collect();
                                        match rpcs3_lv2_spu_image::build_image(
                                            info.e_entry as u32,
                                            &phdrs,
                                            src_ptr,
                                        ) {
                                            Err(e) => eprintln!(
                                                "[R9.1h] build_image failed: {e:?} \
                                                 (entry=0x{:x} phdrs={})",
                                                info.e_entry,
                                                phdrs.len(),
                                            ),
                                            Ok(image) => {
                                                eprintln!(
                                                    "[R9.1h] SPU image captured: \
                                                     entry=0x{:x} segs={}",
                                                    image.entry_point,
                                                    image.segments.len(),
                                                );
                                                self.spu_image = Some(image.clone());
                                                self.spu_image_src_vaddr = src_ptr;
                                                // Stub sysSpuImage struct:
                                                // type=0, entry, segs_ptr=0,
                                                // nsegs=count (16 BE bytes).
                                                let mut buf = [0u8; 16];
                                                buf[4..8].copy_from_slice(
                                                    &image.entry_point.to_be_bytes(),
                                                );
                                                buf[12..16].copy_from_slice(
                                                    &(image.segments.len() as u32)
                                                        .to_be_bytes(),
                                                );
                                                self.mem.write(image_ptr, &buf).ok();
                                            }
                                        }
                                    }
                                }
                            }
                            self.ppu.gpr[3] = 0;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R9.1i — console_putc(ch) (NID 0xe66bac36):
                        // emit single char to TTY ch3.
                        0xe66bac36 => {
                            let c = (r3_in as u8) as char;
                            let s = c.to_string();
                            let mut written: u32 = 0;
                            let _ = self.tty.write(
                                3,
                                Some(&s),
                                1,
                                Some(&mut written),
                            );
                            self.ppu.gpr[3] = 1;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R9.1i — console_write(buf, len) (NID
                        // 0xf57e1d6f): emit `len` bytes from r3 to
                        // TTY ch3. This is PSL1GHT's primary console
                        // write entry point — printf/puts/etc may
                        // route through here.
                        0xf57e1d6f => {
                            let buf_ptr = r3_in as u32;
                            let len = r4_in as u32;
                            let mut bytes = vec![0u8; len as usize];
                            if len > 0 && self
                                .mem
                                .read(buf_ptr, &mut bytes)
                                .is_ok()
                            {
                                let s = String::from_utf8_lossy(&bytes)
                                    .into_owned();
                                let mut written: u32 = 0;
                                let _ = self.tty.write(
                                    3,
                                    Some(&s),
                                    len,
                                    Some(&mut written),
                                );
                                eprintln!(
                                    "[R9.1i] console_write(*0x{buf_ptr:x}, \
                                     {len}) → {written}: {s:?}",
                                );
                            }
                            self.ppu.gpr[3] = len as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R9.1i — _sys_vprintf(fmt, va_list)
                        // (NID 0xfa7f693d): va_list pointer at r4.
                        // For the MVP, deref the va_list as a contiguous
                        // u64 array (PSL1GHT layout) and reuse
                        // mini_printf.
                        0xfa7f693d => {
                            let fmt_ptr = r3_in as u32;
                            let va_ptr = r4_in as u32;
                            let fmt = read_c_string(&self.mem, fmt_ptr, 4096)
                                .unwrap_or_default();
                            let mut args = [0u64; 7];
                            for (i, slot) in args.iter_mut().enumerate() {
                                let mut buf = [0u8; 8];
                                if self.mem.read(
                                    va_ptr.wrapping_add((i * 8) as u32),
                                    &mut buf,
                                ).is_ok() {
                                    *slot = u64::from_be_bytes(buf);
                                }
                            }
                            let formatted = mini_printf(&fmt, &args, &self.mem);
                            let mut written: u32 = 0;
                            let _ = self.tty.write(
                                3,
                                Some(&formatted),
                                formatted.len() as u32,
                                Some(&mut written),
                            );
                            eprintln!(
                                "[R9.1i] _sys_vprintf(*0x{fmt_ptr:x}) → {written}: {formatted:?}",
                            );
                            self.ppu.gpr[3] = formatted.len() as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R9.1i — _sys_snprintf(buf, n, fmt, ...)
                        // (NID 0x06574237) + _sys_sprintf(buf, fmt, ...)
                        // (NID 0xa1f9eafe): format into buf at r3.
                        0x06574237 | 0xa1f9eafe => {
                            let buf_ptr = r3_in as u32;
                            let (n_cap, fmt_ptr, fmt_args_start) = if nid == 0x06574237 {
                                (
                                    r4_in as usize,
                                    self.ppu.gpr[5] as u32,
                                    6usize,
                                )
                            } else {
                                (usize::MAX, r4_in as u32, 5usize)
                            };
                            let fmt = read_c_string(&self.mem, fmt_ptr, 4096)
                                .unwrap_or_default();
                            let args = [
                                self.ppu.gpr[fmt_args_start],
                                self.ppu.gpr[fmt_args_start + 1],
                                self.ppu.gpr[fmt_args_start + 2],
                                self.ppu.gpr[fmt_args_start + 3],
                                self.ppu.gpr[fmt_args_start + 4],
                                0,
                                0,
                            ];
                            let formatted = mini_printf(&fmt, &args, &self.mem);
                            let limit = formatted.len().min(n_cap.saturating_sub(1));
                            let mut bytes = formatted.into_bytes();
                            bytes.truncate(limit);
                            bytes.push(0);
                            self.mem.write(buf_ptr, &bytes).ok();
                            self.ppu.gpr[3] = (bytes.len() - 1) as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R9.1h slice 4 — `_sys_printf(fmt, ...)`
                        // (NID 0x9f04f7af): PSL1GHT's actual printf
                        // entry point as used by `printf()` in
                        // mailbox_v1's main(). Routes through
                        // mini_printf + tty.write to TTY ch3.
                        0x9f04f7af => {
                            let fmt_ptr = r3_in as u32;
                            let fmt = read_c_string(&self.mem, fmt_ptr, 4096)
                                .unwrap_or_default();
                            let args = [
                                self.ppu.gpr[4],
                                self.ppu.gpr[5],
                                self.ppu.gpr[6],
                                self.ppu.gpr[7],
                                self.ppu.gpr[8],
                                self.ppu.gpr[9],
                                self.ppu.gpr[10],
                            ];
                            let formatted = mini_printf(&fmt, &args, &self.mem);
                            let mut written: u32 = 0;
                            let _ = self.tty.write(
                                3,
                                Some(&formatted),
                                formatted.len() as u32,
                                Some(&mut written),
                            );
                            eprintln!(
                                "[R9.1h] _sys_printf(*0x{fmt_ptr:x}) → {written}: {formatted:?}",
                            );
                            self.ppu.gpr[3] = formatted.len() as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // `_sys_vsprintf(buf, fmt, va_list)` (NID
                        // 0x791b9219): write formatted output into
                        // `buf` (r3). The PSL1GHT runtime threads
                        // its va_list through r5. For the MVP, treat
                        // it like printf and dump the formatted
                        // result into the supplied buffer.
                        0x791b9219 => {
                            let buf_ptr = r3_in as u32;
                            let fmt_ptr = r4_in as u32;
                            let fmt = read_c_string(&self.mem, fmt_ptr, 4096)
                                .unwrap_or_default();
                            let args = [
                                self.ppu.gpr[5],
                                self.ppu.gpr[6],
                                self.ppu.gpr[7],
                                self.ppu.gpr[8],
                                self.ppu.gpr[9],
                                self.ppu.gpr[10],
                            ];
                            let formatted = mini_printf(&fmt, &args, &self.mem);
                            let mut bytes = formatted.into_bytes();
                            bytes.push(0);
                            self.mem.write(buf_ptr, &bytes).ok();
                            self.ppu.gpr[3] = (bytes.len() - 1) as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // newlib `printf(fmt, ...)` (NID 0xc01d9f97):
                        // for the R9.1h MVP we resolve %d / %x /
                        // %s / %c from r4-r10 and emit to TTY ch3.
                        // Full printf is libc territory; this
                        // covers the formats the oracle fixtures
                        // use ("OK status=0x%x\n", etc.).
                        0xc01d9f97 => {
                            let fmt_ptr = r3_in as u32;
                            let fmt = read_c_string(&self.mem, fmt_ptr, 4096).unwrap_or_default();
                            let args = [
                                self.ppu.gpr[4],
                                self.ppu.gpr[5],
                                self.ppu.gpr[6],
                                self.ppu.gpr[7],
                                self.ppu.gpr[8],
                                self.ppu.gpr[9],
                                self.ppu.gpr[10],
                            ];
                            let formatted = mini_printf(&fmt, &args, &self.mem);
                            let mut written: u32 = 0;
                            let _ = self.tty.write(
                                3,
                                Some(&formatted),
                                formatted.len() as u32,
                                Some(&mut written),
                            );
                            eprintln!(
                                "[R9.1h] printf(*0x{fmt_ptr:x}={fmt:?}) \
                                 → {written}: {formatted:?}",
                            );
                            self.ppu.gpr[3] = formatted.len() as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R13.1 — cellGcmSys: _cellGcmInitBody. Ports
                        // the guest-visible state setup of RPCS3
                        // cellGcmSys.cpp:390 (no renderer / GPU):
                        // build the CellGcmContextData + CellGcmControl
                        // in a reserved RSX region, write *context, and
                        // record the io region. Struct layouts from
                        // Emu/RSX/GCM.h (Context = 4×u32 be; Control =
                        // 3×u32 be).
                        0x15bae46b => {
                            // _cellGcmInitBody(context**, cmdSize,
                            //                  ioSize, ioAddress)
                            let context_pp = r3_in as u32;
                            let _cmd_size = r4_in as u32;
                            let io_size = self.ppu.gpr[5] as u32;
                            let io_address = self.ppu.gpr[6] as u32;

                            // Reserved RSX region for the kernel-side
                            // gcm structs (RPCS3 uses render->device_addr
                            // + dma_address; we carve a fixed unused page).
                            const GCM_CTX_ADDR: u32 = 0x3000_0000;
                            const GCM_CONTROL_ADDR: u32 = 0x3000_0040;
                            // alloc_at errors if already mapped (e.g. a
                            // second init); ignore — the writes below are
                            // what matter.
                            let _ = self.mem.alloc_at(
                                GCM_CTX_ADDR,
                                0x1000,
                                PageFlags::READABLE | PageFlags::WRITABLE,
                            );

                            // Local (video) memory region — RPCS3
                            // cellGcmSys.cpp:404-406 falloc(local_mem_base,
                            // local_size, vm::video). PSL1GHT's rsxInit
                            // builds a local-memory pool allocator over this
                            // span: it writes a free-block header at the base
                            // AND a boundary tag near the end (disasm: the
                            // pool init at 0x126c0 does std at [base] and
                            // stdx at [base + size - 16]), so the WHOLE
                            // region must be backed, not just the base page.
                            const GCM_LOCAL_ADDR: u32 = 0xC000_0000;
                            const GCM_LOCAL_SIZE: u32 = 0x0f90_0000;
                            let _ = self.mem.alloc_at(
                                GCM_LOCAL_ADDR,
                                GCM_LOCAL_SIZE,
                                PageFlags::READABLE | PageFlags::WRITABLE,
                            );

                            // CellGcmContextData: begin/end/current/
                            // callback (cellGcmSys.cpp:451-454).
                            let begin = io_address.wrapping_add(4096);
                            let end =
                                io_address.wrapping_add(32 * 1024 - 4);
                            let mut ctx = [0u8; 16];
                            ctx[0..4].copy_from_slice(&begin.to_be_bytes());
                            ctx[4..8].copy_from_slice(&end.to_be_bytes());
                            ctx[8..12].copy_from_slice(&begin.to_be_bytes());
                            // callback = 0: small frames never reach the
                            // buffer-full flush path that would call it.
                            ctx[12..16].copy_from_slice(&0u32.to_be_bytes());
                            self.mem.write(GCM_CTX_ADDR, &ctx).ok();

                            // *context = GCM_CTX_ADDR (cellGcmSys.cpp:462).
                            self.mem
                                .write(context_pp, &GCM_CTX_ADDR.to_be_bytes())
                                .ok();

                            // CellGcmControl { put, get, ref }: ref starts
                            // 0xffffffff per hardware; put/get 0.
                            let mut ctrl = [0u8; 12];
                            ctrl[8..12]
                                .copy_from_slice(&0xFFFF_FFFFu32.to_be_bytes());
                            self.mem.write(GCM_CONTROL_ADDR, &ctrl).ok();

                            self.gcm_context_addr = GCM_CTX_ADDR;
                            self.gcm_control_addr = GCM_CONTROL_ADDR;
                            self.gcm_io_address = io_address;
                            self.gcm_io_size = io_size;
                            eprintln!(
                                "[R13.1] _cellGcmInitBody(ctx**=0x{context_pp:x}, \
                                 io=0x{io_address:x}, ioSize=0x{io_size:x}) → \
                                 ctx@0x{GCM_CTX_ADDR:x} begin=0x{begin:x}",
                            );
                            self.ppu.gpr[3] = 0; // CELL_OK
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R13.1 — cellGcmSys: cellGcmGetConfiguration
                        // (NID confirmed via RPCS3's NID hash:
                        // SHA1("cellGcmGetConfiguration"+suffix)[..4] LE
                        // == 0xe315a0b2). Copies the CellGcmConfig that
                        // _cellGcmInitBody populated to *config
                        // (cellGcmSys.cpp:342-346). Layout = 6 BE u32
                        // (GCM.h:12): localAddress, ioAddress, localSize,
                        // ioSize, memoryFrequency, coreFrequency. PSL1GHT's
                        // rsxInit reads localAddress (off 0) + localSize
                        // (off 8) from here to seed its local-memory pool
                        // allocator (disasm 0x10844-0x10858) — a zero
                        // config makes that allocator store to null.
                        0xe315a0b2 => {
                            let config_ptr = r3_in as u32;
                            // local mem constants mirror _cellGcmInitBody
                            // (cellGcmSys.cpp:404-405; local_mem_base
                            // 0xC0000000, size 0xf900000).
                            const GCM_LOCAL_ADDR: u32 = 0xC000_0000;
                            const GCM_LOCAL_SIZE: u32 = 0x0f90_0000;
                            let mut cfg = [0u8; 24];
                            cfg[0..4]
                                .copy_from_slice(&GCM_LOCAL_ADDR.to_be_bytes());
                            cfg[4..8].copy_from_slice(
                                &self.gcm_io_address.to_be_bytes(),
                            );
                            cfg[8..12]
                                .copy_from_slice(&GCM_LOCAL_SIZE.to_be_bytes());
                            cfg[12..16]
                                .copy_from_slice(&self.gcm_io_size.to_be_bytes());
                            // memoryFrequency 650 MHz, coreFrequency 500 MHz
                            // (cellGcmSys.cpp:440-441).
                            cfg[16..20].copy_from_slice(
                                &650_000_000u32.to_be_bytes(),
                            );
                            cfg[20..24].copy_from_slice(
                                &500_000_000u32.to_be_bytes(),
                            );
                            self.mem.write(config_ptr, &cfg).ok();
                            eprintln!(
                                "[R13.1] cellGcmGetConfiguration(config=\
                                 0x{config_ptr:x}) → local=0x{GCM_LOCAL_ADDR:x} \
                                 io=0x{:x} localSize=0x{GCM_LOCAL_SIZE:x}",
                                self.gcm_io_address,
                            );
                            self.ppu.gpr[3] = 0; // void; CELL_OK is harmless
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R13.4 — cellGcmGetControlRegister() ->
                        // vm::ptr<CellGcmControl>. cellGcmSys.cpp:260
                        // returns gcm_info.control_addr. Our R13.1
                        // _cellGcmInitBody placed the control block at
                        // GCM_CONTROL_ADDR (0x30000040); return that as
                        // a 32-bit guest pointer in r3. Callers (libgcm
                        // flip path) deref this for put/get/ref — a
                        // silent return of 0 was the real R13.4 wall
                        // (the addressToOffset gap was a side branch).
                        0xa547adde => {
                            let ctrl = self.gcm_control_addr;
                            eprintln!(
                                "[R13.4] cellGcmGetControlRegister() → \
                                 0x{ctrl:08x}"
                            );
                            self.ppu.gpr[3] = ctrl as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R13.4 — cellGcmAddressToOffset(address, *offset).
                        // Translates a PPU effective address to an RSX IO
                        // offset (cellGcmSys.cpp). The result is returned
                        // via the *r4 OUT pointer; our prior fast-path
                        // returning 0 without writing left libgcm wrappers
                        // (gcmSetDisplayBuffer / gcmSetFlip / …) reading
                        // uninitialised storage → downstream null deref.
                        //
                        // Mapping per R13.1 _cellGcmInitBody (default 1:1
                        // io binding): an address in [io_address,
                        // io_address+io_size) maps to (address-io_address);
                        // an address in the local-mem region
                        // [0xC0000000, +0xf900000) maps to
                        // (address-0xC0000000). Otherwise return
                        // CELL_GCM_ERROR_FAILURE (0x802100ff).
                        0x21ac3697 => {
                            let address = r3_in as u32;
                            let offset_ptr = self.ppu.gpr[4] as u32;
                            const LOCAL_BASE: u32 = 0xC000_0000;
                            const LOCAL_SIZE: u32 = 0x0f90_0000;
                            let io_base = self.gcm_io_address;
                            let io_size = self.gcm_io_size;
                            let (io_offset, status) = if io_size != 0
                                && address >= io_base
                                && address < io_base.saturating_add(io_size)
                            {
                                (address - io_base, 0u32)
                            } else if address >= LOCAL_BASE
                                && address < LOCAL_BASE + LOCAL_SIZE
                            {
                                (address - LOCAL_BASE, 0u32)
                            } else {
                                (0u32, 0x802100ffu32)
                            };
                            self.mem
                                .write(offset_ptr, &io_offset.to_be_bytes())
                                .ok();
                            eprintln!(
                                "[R13.4] cellGcmAddressToOffset(addr=\
                                 0x{address:08x}, *offset=0x{offset_ptr:08x}) \
                                 → offset=0x{io_offset:08x} status=0x{status:x}"
                            );
                            self.ppu.gpr[3] = status as u64;
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // R13.6 — cellSysutil::cellSysutilGetSystemParamInt
                        // (NID 0x40e895d3). First HLE-crate integration: route
                        // to rpcs3-hle-cellsysutil backed by EmuSysutilConfig.
                        // r3 = param id, r4 = OUT *value.
                        0x40e895d3 => {
                            let id = self.ppu.gpr[3] as i32;
                            let value_ptr = self.ppu.gpr[4] as u32;
                            match cell_sysutil_get_system_param_int(&EmuSysutilConfig, id) {
                                Ok(v) => {
                                    self.mem.write(value_ptr, &v.to_be_bytes())?;
                                    self.ppu.gpr[3] = 0; // CELL_OK
                                }
                                Err(e) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                }
                            }
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellSysutil::cellSysutilGetSystemParamString
                        // (NID 0x938013a0). r3 = param id, r4 = OUT *buf,
                        // r5 = bufsize. Copies the string into the guest buffer,
                        // truncated to bufsize-1 and NUL-terminated.
                        0x938013a0 => {
                            let id = self.ppu.gpr[3] as i32;
                            let buf_ptr = self.ppu.gpr[4] as u32;
                            let bufsize = self.ppu.gpr[5] as u32;
                            match cell_sysutil_get_system_param_string(
                                &EmuSysutilConfig,
                                id,
                                bufsize,
                            ) {
                                Ok(s) => {
                                    let max = (bufsize as usize).saturating_sub(1);
                                    let n = s.len().min(max);
                                    self.mem.write(buf_ptr, &s.as_bytes()[..n])?;
                                    self.mem.write(buf_ptr + n as u32, &[0u8])?;
                                    self.ppu.gpr[3] = 0; // CELL_OK
                                }
                                Err(e) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                }
                            }
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellVideoOut::cellVideoOutGetResolution
                        // (NID 0xe558748d). STATELESS table lookup. r3 =
                        // resolution id, r4 = OUT *videoResolution{u16 w;u16 h}.
                        0xe558748d => {
                            let id = self.ppu.gpr[3] as u8;
                            let out_ptr = self.ppu.gpr[4] as u32;
                            match cell_video_out_get_resolution(id) {
                                Ok(r) => {
                                    self.mem
                                        .write(out_ptr, &(r.width as u16).to_be_bytes())?;
                                    self.mem.write(
                                        out_ptr + 2,
                                        &(r.height as u16).to_be_bytes(),
                                    )?;
                                    self.ppu.gpr[3] = 0; // CELL_OK
                                }
                                Err(e) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                }
                            }
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellVideoOut::cellVideoOutGetState
                        // (NID 0x887572d5). Stateful (VideoOutManager). r3 = port,
                        // r4 = deviceIndex, r5 = OUT *videoState {state@0,
                        // colorSpace@1, padding[6], displayMode@8{resolution@8,
                        // scanMode@9, conversion@10, aspect@11,...}}.
                        0x887572d5 => {
                            let port = self.ppu.gpr[3] as u32;
                            let device_index = self.ppu.gpr[4] as u32;
                            let state_ptr = self.ppu.gpr[5] as u32;
                            self.ppu.gpr[3] = match cell_video_out_get_state(
                                &self.videoout,
                                port,
                                device_index,
                            ) {
                                Ok(info) => {
                                    let state_byte: u8 = match info.state {
                                        VideoOutState::Enabled => 0,
                                        VideoOutState::Disabled => 1,
                                        VideoOutState::DeepSleep => 2,
                                    };
                                    self.mem
                                        .write(state_ptr, &[state_byte, info.color_space])?;
                                    self.mem.write(
                                        state_ptr + 8,
                                        &[info.resolution_id, info.scan_mode],
                                    )?;
                                    self.mem.write(state_ptr + 11, &[info.aspect])?;
                                    0 // CELL_OK
                                }
                                Err(e) => u64::from(u32::from(e)),
                            };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellVideoOut::cellVideoOutGetConfiguration
                        // (NID 0x15b0b0cd). Stateful (VideoOutManager). r3 = port,
                        // r4 = OUT *videoConfiguration {resolution@0,format@1,
                        // aspect@2, padding[9], pitch@12 (u32)}. r5 = option.
                        0x15b0b0cd => {
                            let port = self.ppu.gpr[3] as u32;
                            let cfg_ptr = self.ppu.gpr[4] as u32;
                            self.ppu.gpr[3] = match cell_video_out_get_configuration(
                                &self.videoout,
                                port,
                            ) {
                                Ok(c) => {
                                    self.mem.write(
                                        cfg_ptr,
                                        &[c.resolution_id, c.format, c.aspect],
                                    )?;
                                    self.mem
                                        .write(cfg_ptr + 12, &c.pitch.to_be_bytes())?;
                                    0 // CELL_OK
                                }
                                Err(e) => u64::from(u32::from(e)),
                            };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellVideoOut::cellVideoOutGetResolutionAvailability
                        // (NID 0xa322db75). Stateful (VideoOutManager). r3 = port,
                        // r4 = resolution id, r5 = aspect, r6 = option. Returns 1
                        // if the resolution is in the supported set, else 0.
                        0xa322db75 => {
                            let port = self.ppu.gpr[3] as u32;
                            let res_id = self.ppu.gpr[4] as u8;
                            let aspect = self.ppu.gpr[5] as u8;
                            self.ppu.gpr[3] = match cell_video_out_get_resolution_availability(
                                &self.videoout,
                                port,
                                res_id,
                                aspect,
                            ) {
                                Ok(n) => u64::from(n as u32),
                                Err(e) => u64::from(u32::from(e)),
                            };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellVideoOut::cellVideoOutGetNumberOfDevice
                        // (NID 0x75bbb672). STATELESS; r3 = videoOut port. The
                        // count is returned directly in r3 (no OUT pointer).
                        0x75bbb672 => {
                            let port = self.ppu.gpr[3] as u32;
                            self.ppu.gpr[3] =
                                match cell_video_out_get_number_of_device(port) {
                                    Ok(n) => u64::from(n as u32),
                                    Err(e) => u64::from(u32::from(e)),
                                };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellNetCtl::cellNetCtlInit (NID 0xbd5a59fc).
                        // Stateful: flips the NetCtlManager's `initialized` gate.
                        // No args.
                        0xbd5a59fc => {
                            self.ppu.gpr[3] = match cell_net_ctl_init(&mut self.netctl) {
                                Ok(()) => 0, // CELL_OK
                                Err(e) => u64::from(u32::from(e)),
                            };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellNetCtl::cellNetCtlGetState (NID 0x8b3eba69).
                        // r3 = OUT *s32 state. With a connected backend staged,
                        // reports CELL_NET_CTL_STATE_IPOBTAINED (3).
                        0x8b3eba69 => {
                            let state_ptr = self.ppu.gpr[3] as u32;
                            // Fixed connected-network provider (emulated console
                            // reports an established link). ip/mac are arbitrary
                            // but stable; only is_connected affects the state.
                            let backend = StubConnectedBackend {
                                ip: 0xC0A8_012A, // 192.168.1.42
                                mac: [0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34],
                            };
                            self.ppu.gpr[3] =
                                match cell_net_ctl_get_state(&self.netctl, &backend) {
                                    Ok(state) => {
                                        self.mem.write(state_ptr, &state.to_be_bytes())?;
                                        0 // CELL_OK
                                    }
                                    Err(e) => u64::from(u32::from(e)),
                                };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellNetCtl::cellNetCtlGetInfo (NID 0x1e585b5d).
                        // r3 = info code, r4 = OUT *union net_ctl_info. Reuses the
                        // shared netctl field + connected backend. Only the MTU
                        // path is fixture-validated; the u32 codes (MTU/Link) map
                        // cleanly to the union @offset 0; the IPv4/ether codes are
                        // wired best-effort (raw bytes) pending dedicated fixtures.
                        0x1e585b5d => {
                            let code = self.ppu.gpr[3] as u32;
                            let info_ptr = self.ppu.gpr[4] as u32;
                            let backend = StubConnectedBackend {
                                ip: 0xC0A8_012A,
                                mac: [0x00, 0xAB, 0xCD, 0xEF, 0x12, 0x34],
                            };
                            self.ppu.gpr[3] =
                                match cell_net_ctl_get_info(&self.netctl, &backend, code) {
                                    Ok(info) => {
                                        match info {
                                            NetInfo::Mtu(v) => self
                                                .mem
                                                .write(info_ptr, &v.to_be_bytes())?,
                                            NetInfo::LinkUp => self
                                                .mem
                                                .write(info_ptr, &1u32.to_be_bytes())?,
                                            NetInfo::LinkDown => self
                                                .mem
                                                .write(info_ptr, &0u32.to_be_bytes())?,
                                            NetInfo::IpAddress(b)
                                            | NetInfo::Netmask(b)
                                            | NetInfo::DefaultRoute(b)
                                            | NetInfo::PrimaryDns(b)
                                            | NetInfo::SecondaryDns(b) => {
                                                self.mem.write(info_ptr, &b)?;
                                            }
                                            NetInfo::EtherAddr(b) => {
                                                self.mem.write(info_ptr, &b)?;
                                            }
                                        }
                                        0 // CELL_OK
                                    }
                                    Err(e) => u64::from(u32::from(e)),
                                };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellGame::cellGameGetParamInt (NID 0xb7a45caf).
                        // Provider (EmuGameConfig). r3 = PSF param id (real PS3
                        // numbering; PARENTAL_LEVEL=103), r4 = OUT *value.
                        0xb7a45caf => {
                            let id = self.ppu.gpr[3] as i32;
                            let value_ptr = self.ppu.gpr[4] as u32;
                            match cell_game_get_param_int(&EmuGameConfig, id) {
                                Ok(v) => {
                                    self.mem.write(value_ptr, &v.to_be_bytes())?;
                                    self.ppu.gpr[3] = 0; // CELL_OK
                                }
                                Err(e) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                }
                            }
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — sys_net::inet_addr (NID 0xdabbc2c0).
                        // Firmware stub: unconditionally returns INET_ADDR_NONE
                        // (0xFFFFFFFF), byte-exact with RPCS3 sys_net_.cpp. r3 =
                        // const char* ip (ignored by the stub).
                        0xdabbc2c0 => {
                            self.ppu.gpr[3] = u64::from(inet_addr_stub(true));
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellSysModule::cellSysmoduleLoadModule
                        // (NID 0x32267a31). Stateful HLE crate: route to
                        // rpcs3-hle-cellsysmodule against EmuCore's persistent
                        // SysmoduleManager. r3 = module id.
                        0x32267a31 => {
                            let id = self.ppu.gpr[3] as u16;
                            self.ppu.gpr[3] =
                                match cell_sysmodule_load_module(&mut self.sysmodule, id) {
                                    Ok(()) => 0, // CELL_OK
                                    Err(e) => u64::from(u32::from(e)),
                                };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellSysModule::cellSysmoduleIsLoaded
                        // (NID 0x5a59e258). Returns CELL_SYSMODULE_LOADED (0) if
                        // refs are outstanding, else CELL_SYSMODULE_ERROR_UNLOADED.
                        // r3 = module id.
                        0x5a59e258 => {
                            let id = self.ppu.gpr[3] as u16;
                            self.ppu.gpr[3] =
                                match cell_sysmodule_is_loaded(&self.sysmodule, id) {
                                    Ok(v) => u64::from(v as u32),
                                    Err(e) => u64::from(u32::from(e)),
                                };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellSysutil::cellSysutilRegisterCallback
                        // (NID 0x9d98afa0). Stores a guest callback fn-ptr +
                        // userdata in a slot. NO guest call here. r3 = slot,
                        // r4 = callback fn descriptor ptr, r5 = userdata.
                        0x9d98afa0 => {
                            let slot = self.ppu.gpr[3] as u32;
                            let func = self.ppu.gpr[4] as u32;
                            let userdata = self.ppu.gpr[5] as u32;
                            self.ppu.gpr[3] = match cell_sysutil_register_callback(
                                &mut self.sysutil_callbacks,
                                slot,
                                func,
                                userdata,
                            ) {
                                Ok(()) => 0, // CELL_OK
                                Err(e) => u64::from(u32::from(e)),
                            };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellSysutil::cellSysutilUnregisterCallback
                        // (NID 0x02ff3c1b). Clears a slot. r3 = slot.
                        0x02ff3c1b => {
                            let slot = self.ppu.gpr[3] as u32;
                            self.ppu.gpr[3] = match cell_sysutil_unregister_callback(
                                &mut self.sysutil_callbacks,
                                slot,
                            ) {
                                Ok(()) => 0, // CELL_OK
                                Err(e) => u64::from(u32::from(e)),
                            };
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // HLE wave — cellSysutil::cellSysutilCheckCallback
                        // (NID 0x189a74da). Drains the pending-event queue and
                        // INVOKES each registered guest callback via
                        // call_guest_function — the FIRST re-entrant guest call
                        // driven by an HLE arm. cellSysutil callback ABI:
                        // cb(u64 status, u64 param, void* userdata).
                        0x189a74da => {
                            // Drain into an OWNED Vec (the stub_meta/plan borrow
                            // already ended above, so &mut self is free here).
                            let dispatches = cell_sysutil_check_callback(
                                &self.sysutil_callbacks,
                                &mut self.sysutil_queue,
                            );
                            for d in dispatches {
                                // void callback -> ignore the returned r3.
                                let _ = self.call_guest_function(
                                    d.cb.fn_addr,
                                    &[
                                        u64::from(d.event),
                                        d.param,
                                        u64::from(d.cb.user_data),
                                    ],
                                )?;
                            }
                            self.ppu.gpr[3] = 0; // CELL_OK
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        // Callback wave — cellMsgDialog::cellMsgDialogOpen2
                        // (NID 0x7603d3db). r3 = type flags, r4 = message ptr,
                        // r5 = callback FD, r6 = userdata, r7 = unused. With no
                        // user, emu-core headless-auto-confirms: open -> close
                        // (default button per type) -> invoke the guest callback
                        // cb(button, userdata) via call_guest_function.
                        0x7603d3db => {
                            let type_flags = self.ppu.gpr[3] as u32;
                            let msg_ptr = self.ppu.gpr[4] as u32;
                            let cb_fd = self.ppu.gpr[5] as u32;
                            let usrdata = self.ppu.gpr[6] as u32;
                            let message = self.read_guest_cstr(msg_ptr, 256);
                            match cell_msg_dialog_open(
                                &mut self.msgdialog,
                                MsgTypeFlags(type_flags),
                                &message,
                            ) {
                                Ok(()) => {
                                    let _ =
                                        cell_msg_dialog_close(&mut self.msgdialog, 0);
                                    let button = msg_last_button(&self.msgdialog);
                                    if cb_fd != 0 {
                                        let _ = self.call_guest_function(
                                            cb_fd,
                                            &[button as i64 as u64, u64::from(usrdata)],
                                        )?;
                                    }
                                    self.ppu.gpr[3] = 0; // CELL_OK
                                }
                                Err(e) => {
                                    self.ppu.gpr[3] = u64::from(u32::from(e));
                                }
                            }
                            self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                            return Ok(None);
                        }
                        _ => {}
                    }

                    eprintln!(
                        "[R9.1g.7] unimplemented import: {module}::0x{nid:08x} \
                         (trampoline=0x{trampoline_vaddr:08x} \
                         addrs_slot=0x{addrs_slot:08x}) r3=0x{r3_in:x} \
                         r4=0x{r4_in:x} — returning 0",
                    );
                    self.ppu.gpr[3] = 0;
                    self.ppu.cia = (self.ppu.lr as u32) & !0x3;
                    return Ok(None);
                }
            }
        }

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
            // R10.1.g — sys_cond family (#105-#109). PSL1GHT exposes
            // these via <sys/cond.h>. Routes into the R10.3
            // CondRegistry impl on Lv2SyncState. cond_wait (#107) is
            // wired but unexercisable single-PPU (MustBlock →
            // ETIMEDOUT).
            105 => {
                // sys_cond_create(*cond_out, mutex_id, *attr)
                let cond_out = r3 as u32;
                let mutex_id = r4 as u32;
                let attr_ptr = self.ppu.gpr[5] as u32;
                let attr = match Self::read_sys_cond_attr(&self.mem, attr_ptr) {
                    Ok(a) => a,
                    Err(e) => {
                        self.ppu.gpr[3] = e.0 as u64;
                        return Ok(None);
                    }
                };
                match self.lv2_sync_state.cond_create(attr, mutex_id) {
                    Ok(id) => {
                        self.mem.write(cond_out, &id.to_be_bytes())?;
                        self.ppu.gpr[3] = 0;
                    }
                    Err(e) => self.ppu.gpr[3] = e.0 as u64,
                }
            }
            106 => {
                // sys_cond_destroy(cond_id)
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.cond_destroy(id) {
                    Ok(()) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            107 => {
                // sys_cond_wait(cond_id, timeout). Single-PPU can't
                // park; nobody can signal → ETIMEDOUT honestly.
                let id = r3 as u32;
                let _timeout = r4;
                self.ppu.gpr[3] =
                    match self.lv2_sync_state.cond_wait(id, PPU_THREAD_TID, 0) {
                        Ok(CondWaitOutcome::Woken) => 0,
                        Ok(CondWaitOutcome::MustBlock)
                        | Ok(CondWaitOutcome::Timeout) => {
                            CellError::ETIMEDOUT.0 as u64
                        }
                        Err(e) => e.0 as u64,
                    };
            }
            108 => {
                // sys_cond_signal(cond_id). None on empty queue → OK.
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.cond_signal(id) {
                    Ok(_) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            109 => {
                // sys_cond_signal_all / broadcast(cond_id).
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.cond_signal_all(id) {
                    Ok(_) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            // R10.1.f — sys_event_queue + sys_event_port family
            // (#128-#130, #134-#138). PSL1GHT exposes these via
            // <sys/event_queue.h>. Routes into the R10.6
            // EventRegistry impl on Lv2SyncState.
            128 => {
                // sys_event_queue_create(*q_out, *attr, key, size)
                let q_out = r3 as u32;
                let attr_ptr = r4 as u32;
                let _key = self.ppu.gpr[5];
                let size = self.ppu.gpr[6] as u32;
                let attr = match Self::read_sys_event_queue_attr(&self.mem, attr_ptr)
                {
                    Ok(a) => a,
                    Err(e) => {
                        self.ppu.gpr[3] = e.0 as u64;
                        return Ok(None);
                    }
                };
                match self.lv2_sync_state.queue_create(attr, size) {
                    Ok(id) => {
                        self.mem.write(q_out, &id.to_be_bytes())?;
                        self.ppu.gpr[3] = 0;
                    }
                    Err(e) => self.ppu.gpr[3] = e.0 as u64,
                }
            }
            129 => {
                // sys_event_queue_destroy(q_id, mode)
                let id = r3 as u32;
                let mode = r4 as u32;
                self.ppu.gpr[3] =
                    match self.lv2_sync_state.queue_destroy(id, mode) {
                        Ok(()) => 0,
                        Err(e) => e.0 as u64,
                    };
            }
            130 => {
                // sys_event_queue_receive(q_id, *event_out, timeout).
                // The kernel returns the event in REGISTERS r4-r7
                // (not via the event pointer) — PSL1GHT's
                // REG_PASS_SYS_EVENT_QUEUE_RECEIVE macro copies
                // r4→source, r5→data_1, r6→data_2, r7→data_3 into
                // the caller's struct. So we must NOT touch the
                // event_out pointer; we set the registers.
                let id = r3 as u32;
                let _event_out = r4; // ignored by the real ABI
                let _timeout = self.ppu.gpr[5];
                match self.lv2_sync_state.queue_receive(id) {
                    Ok(ReceiveOutcome::Received(ev)) => {
                        self.ppu.gpr[3] = 0;
                        self.ppu.gpr[4] = ev.source;
                        self.ppu.gpr[5] = ev.data1;
                        self.ppu.gpr[6] = ev.data2;
                        self.ppu.gpr[7] = ev.data3;
                    }
                    Ok(ReceiveOutcome::MustBlock) => {
                        // Single-PPU: empty queue + nobody to send →
                        // would block forever. ETIMEDOUT honestly.
                        self.ppu.gpr[3] = CellError::ETIMEDOUT.0 as u64;
                    }
                    Err(e) => self.ppu.gpr[3] = e.0 as u64,
                }
            }
            133 => {
                // sys_event_queue_drain(q_id)
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.queue_drain(id) {
                    Ok(()) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            134 => {
                // sys_event_port_create(*port_out, port_type, name)
                let port_out = r3 as u32;
                let port_type = r4 as u32;
                let name = self.ppu.gpr[5];
                match self.lv2_sync_state.port_create(port_type, name) {
                    Ok(id) => {
                        self.mem.write(port_out, &id.to_be_bytes())?;
                        self.ppu.gpr[3] = 0;
                    }
                    Err(e) => self.ppu.gpr[3] = e.0 as u64,
                }
            }
            135 => {
                // sys_event_port_destroy(port_id)
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.port_destroy(id) {
                    Ok(()) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            136 => {
                // sys_event_port_connect_local(port_id, q_id)
                let port = r3 as u32;
                let queue = r4 as u32;
                self.ppu.gpr[3] =
                    match self.lv2_sync_state.port_connect_local(port, queue) {
                        Ok(()) => 0,
                        Err(e) => e.0 as u64,
                    };
            }
            137 => {
                // sys_event_port_disconnect(port_id)
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.port_disconnect(id) {
                    Ok(()) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            138 => {
                // sys_event_port_send(port_id, data0, data1, data2)
                let port = r3 as u32;
                let data0 = r4;
                let data1 = self.ppu.gpr[5];
                let data2 = self.ppu.gpr[6];
                self.ppu.gpr[3] =
                    match self.lv2_sync_state.port_send(port, data0, data1, data2) {
                        Ok(()) => 0,
                        Err(e) => e.0 as u64,
                    };
            }
            // R10.1.e — kernel sys_semaphore family (#90-#94 + #114).
            // PSL1GHT exposes these via <sys/sem.h>. Same single-PPU
            // model as sys_mutex: MustBlock from sema_wait surfaces
            // as ETIMEDOUT (would-block-forever on a single thread).
            90 => {
                // sys_semaphore_create(*sem_out, *attr, initial, max)
                let sem_out = r3 as u32;
                let attr_ptr = r4 as u32;
                let initial = self.ppu.gpr[5] as i32;
                let max = self.ppu.gpr[6] as i32;
                let attr = match Self::read_sys_sem_attr(&self.mem, attr_ptr) {
                    Ok(a) => a,
                    Err(e) => {
                        self.ppu.gpr[3] = e.0 as u64;
                        return Ok(None);
                    }
                };
                match self.lv2_sync_state.sema_create(attr, initial, max) {
                    Ok(id) => {
                        self.mem.write(sem_out, &id.to_be_bytes())?;
                        self.ppu.gpr[3] = 0;
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = e.0 as u64;
                    }
                }
            }
            91 => {
                // sys_semaphore_destroy(sem_id)
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.sema_destroy(id) {
                    Ok(()) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            92 => {
                // sys_semaphore_wait(sem_id, timeout_us)
                let id = r3 as u32;
                let _timeout = r4;
                self.ppu.gpr[3] = match self.lv2_sync_state.sema_wait(id) {
                    Ok(BlockOutcome::Acquired) => 0,
                    Ok(BlockOutcome::MustBlock) => {
                        // Single-PPU: value==0 and no other thread to
                        // post → would block forever. Surface as
                        // ETIMEDOUT honestly.
                        CellError::ETIMEDOUT.0 as u64
                    }
                    Ok(BlockOutcome::Timeout) => CellError::ETIMEDOUT.0 as u64,
                    Err(e) => e.0 as u64,
                };
            }
            93 => {
                // sys_semaphore_trywait(sem_id)
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.sema_trywait(id) {
                    Ok(()) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            94 => {
                // sys_semaphore_post(sem_id, count)
                let id = r3 as u32;
                let count = r4 as i32;
                self.ppu.gpr[3] = match self.lv2_sync_state.sema_post(id, count) {
                    Ok(()) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            114 => {
                // sys_semaphore_get_value(sem_id, *count_out).
                // Lives outside the 90-95 band — PSL1GHT puts it after
                // the rwlock family.
                let id = r3 as u32;
                let out_ptr = r4 as u32;
                match self.lv2_sync_state.sema_get_value(id) {
                    Ok(value) => {
                        self.mem.write(out_ptr, &value.to_be_bytes())?;
                        self.ppu.gpr[3] = 0;
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = e.0 as u64;
                    }
                }
            }
            // R10.1.d — kernel sys_mutex family (#100-#104). PSL1GHT
            // exposes these via <sys/mutex.h>. The arms route into the
            // Lv2SyncState SyncTable impl (R10.2). Single-PPU model:
            // we hardcode tid = PPU_THREAD_TID. MustBlock on a
            // single-thread fixture means the fixture deadlocked
            // itself — we surface it as EDEADLK.
            100 => {
                // sys_mutex_create(*mutex_out, *attr).
                let mutex_out = r3 as u32;
                let attr_ptr = r4 as u32;
                let attr = match Self::read_sys_mutex_attr(&self.mem, attr_ptr) {
                    Ok(a) => a,
                    Err(e) => {
                        self.ppu.gpr[3] = e.0 as u64;
                        return Ok(None);
                    }
                };
                match self.lv2_sync_state.mutex_create(attr) {
                    Ok(id) => {
                        self.mem.write(mutex_out, &id.to_be_bytes())?;
                        self.ppu.gpr[3] = 0;
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = e.0 as u64;
                    }
                }
            }
            101 => {
                // sys_mutex_destroy(mutex_id).
                let id = r3 as u32;
                self.ppu.gpr[3] = match self.lv2_sync_state.mutex_destroy(id) {
                    Ok(()) => 0,
                    Err(e) => e.0 as u64,
                };
            }
            102 => {
                // sys_mutex_lock(mutex_id, timeout_us).
                let id = r3 as u32;
                let _timeout = r4;
                let outcome = self.lv2_sync_state.mutex_lock(id, PPU_THREAD_TID);
                self.ppu.gpr[3] = match outcome {
                    Ok(BlockOutcome::Acquired) => 0,
                    Ok(BlockOutcome::MustBlock) => {
                        // Single-PPU: this is a deadlock on this fixture
                        // (would-be parker is the only thread).
                        CellError::EDEADLK.0 as u64
                    }
                    Ok(BlockOutcome::Timeout) => CellError::ETIMEDOUT.0 as u64,
                    Err(e) => e.0 as u64,
                };
            }
            103 => {
                // sys_mutex_trylock(mutex_id).
                let id = r3 as u32;
                self.ppu.gpr[3] =
                    match self.lv2_sync_state.mutex_trylock(id, PPU_THREAD_TID) {
                        Ok(()) => 0,
                        Err(e) => e.0 as u64,
                    };
            }
            104 => {
                // sys_mutex_unlock(mutex_id).
                let id = r3 as u32;
                self.ppu.gpr[3] =
                    match self.lv2_sync_state.mutex_unlock(id, PPU_THREAD_TID) {
                        Ok(()) => 0,
                        Err(e) => e.0 as u64,
                    };
            }
            330 => {
                // R9.1g.9 — sys_mmapper_allocate_address(size, flags,
                // alignment, *out_addr). PSL1GHT crt0 reserves a
                // virtual-address range for its heap. Stub returns
                // a fixed sentinel; SparseBackend's lazy mapping
                // handles per-page allocation on first touch.
                let _size = r3 as u32;
                let _flags = r4 as u32;
                let _alignment = self.ppu.gpr[5] as u32;
                let out_addr_ptr = self.ppu.gpr[6] as u32;
                const MMAPPER_FIXED_BASE: u32 = 0xB000_0000;
                self.mem.write(out_addr_ptr, &MMAPPER_FIXED_BASE.to_be_bytes())?;
                self.ppu.gpr[3] = 0;
            }
            // R9.1g.9 — SPU lifecycle syscalls (the integration
            // target). Minimal stubs that satisfy PSL1GHT's
            // expectations. Full end-to-end SPU execution wiring
            // is the next slice; for now we return CELL_OK with
            // sentinel IDs so the PPU's main() advances through
            // the full lifecycle.
            169 => {
                // sys_spu_initialize(max_usable_spu, max_raw_spu)
                // — PSL1GHT calls this once per process.
                self.ppu.gpr[3] = 0;
            }
            170 => {
                // sys_spu_thread_group_create(*group_id, num, prio, *attr)
                let group_id_ptr = r3 as u32;
                const STUB_GROUP_ID: u32 = 0x1000_0001;
                self.mem.write(group_id_ptr, &STUB_GROUP_ID.to_be_bytes())?;
                self.ppu.gpr[3] = 0;
            }
            171 => {
                // sys_spu_thread_group_destroy(group_id)
                self.ppu.gpr[3] = 0;
            }
            172 => {
                // R9.1g.10 — sys_spu_thread_initialize(*thread_id,
                //   group_id, thread_index, *image, *attr, *args)
                //
                // Reads the 32-byte `sysSpuThreadArgument` struct
                // from r6 (4× BE u64) and stashes them for the
                // subsequent group_start to seed SPU r3-r6.
                let thread_id_ptr = r3 as u32;
                let args_ptr = self.ppu.gpr[8] as u32;
                if args_ptr != 0 {
                    for i in 0..4u32 {
                        let mut buf = [0u8; 8];
                        if self.mem.read(args_ptr + i * 8, &mut buf).is_ok() {
                            self.spu_thread_args[i as usize] =
                                u64::from_be_bytes(buf);
                        }
                    }
                    eprintln!(
                        "[R9.1g.10] sys_spu_thread_initialize: args = \
                         arg0=0x{:016x} arg1=0x{:016x} arg2=0x{:016x} arg3=0x{:016x}",
                        self.spu_thread_args[0],
                        self.spu_thread_args[1],
                        self.spu_thread_args[2],
                        self.spu_thread_args[3],
                    );
                }
                const STUB_THREAD_ID: u32 = 0x1000_0002;
                self.mem.write(thread_id_ptr, &STUB_THREAD_ID.to_be_bytes())?;
                self.ppu.gpr[3] = 0;
            }
            173 => {
                // R9.1g.10 — sys_spu_thread_group_start(group_id)
                //
                // Actually run the SPU. Takes the stored SpuImage
                // (R9.1g.10 sys_spu_image_import) + thread args
                // (R9.1g.10 sys_spu_thread_initialize), allocates a
                // fresh SpuThread + LS, deploys the SPU image into
                // LS, seeds initial PC + GPRs, runs the SPU
                // interpreter to a stop instruction, and stashes
                // the OUT_MBOX value for the matching join (178)
                // to return as the group exit status.
                if let Some(image) = self.spu_image.clone() {
                    let mut ls = vec![0u8; rpcs3_spu_thread::SPU_LS_SIZE];
                    // Deploy: copy each SpuSegment from the SPU
                    // ELF blob in PPU memory into LS.
                    let mem_ref = &self.mem;
                    let deploy_result = deploy_image(
                        &image,
                        &mut ls,
                        |addr, size| {
                            let mut buf = vec![0u8; size as usize];
                            mem_ref.read(addr, &mut buf).ok().map(|()| buf)
                        },
                    );
                    if let Err(e) = deploy_result {
                        return Err(Error::SpuGroup(e));
                    }
                    // R9.1g.10 — execute the SPU. Default: the interpreter.
                    // Under the `spu-recompiler` feature with
                    // `spu_backend == Recompiler`, route through the Cranelift
                    // JIT instead (docs/PORT_STATUS_AND_ROADMAP.md §4) —
                    // byte-identical by construction (the JIT falls back to
                    // this same interpreter for unsupported ops). The default
                    // path below is left exactly as-is so the behavior-freeze
                    // gate is untouched.
                    #[cfg(feature = "spu-recompiler")]
                    let jit_selected = matches!(self.spu_backend, SpuBackend::Recompiler);
                    #[cfg(not(feature = "spu-recompiler"))]
                    let jit_selected = false;

                    if jit_selected {
                        #[cfg(feature = "spu-recompiler")]
                        {
                            let mut program = SpuProgram::new(image.entry_point, 10_000_000)
                                .with_segment(0, ls.clone());
                            for (slot, &arg) in self.spu_thread_args.iter().enumerate() {
                                program = program
                                    .with_initial_gpr((3 + slot) as u8, u128::from(arg) << 64);
                            }
                            eprintln!(
                                "[R9.1g.10] sys_spu_thread_group_start: launching SPU (JIT) \
                                 entry=0x{:x} arg0=0x{:x} arg1=0x{:x}",
                                image.entry_point,
                                self.spu_thread_args[0],
                                self.spu_thread_args[1],
                            );
                            let result = RecompilerExecutor::new().execute(&program);
                            match result.stop_reason {
                                ExecutionStopReason::Stop(code) => {
                                    let out_mbox =
                                        result.final_state.channels.out_mbox.unwrap_or(0);
                                    eprintln!(
                                        "[R9.1g.10] SPU halted (JIT): stop_code=0x{:x} \
                                         out_mbox=0x{:08x} steps={}",
                                        code, out_mbox, result.steps_executed,
                                    );
                                    self.spu_exit_status = Some(out_mbox);
                                }
                                ExecutionStopReason::MfcUnsupported { .. } => {
                                    self.spu_exit_status = Some(
                                        result.final_state.channels.out_mbox.unwrap_or(0),
                                    );
                                }
                                _ => return Err(Error::StepsExhausted),
                            }
                        }
                    } else {
                        // Build the SpuThread.
                        let mut spu = SpuThread::new(0);
                        for (i, chunk) in ls.chunks(65536).enumerate() {
                            spu.ls_write((i as u32) * 65536, chunk);
                        }
                        spu.pc = image.entry_point;
                        // Marshal arg0..arg3 into r3..r6 per PSL1GHT
                        // SPU calling convention. The args are u64 in
                        // the SPU's preferred slot (top 64 bits of
                        // 128-bit GPR).
                        for (slot, &arg) in self.spu_thread_args.iter().enumerate() {
                            spu.gpr[3 + slot] = (arg as u128) << 64;
                        }
                        eprintln!(
                            "[R9.1g.10] sys_spu_thread_group_start: launching SPU \
                             entry=0x{:x} arg0=0x{:x} arg1=0x{:x}",
                            spu.pc, self.spu_thread_args[0], self.spu_thread_args[1],
                        );
                        // Run the SPU interpreter until a stop
                        // instruction halts it (or the step budget is
                        // exhausted).
                        let (steps, outcome) = spu_run_n(&mut spu, 10_000_000)?;
                        match outcome {
                            SpuStepOutcome::Stop(code) => {
                                let out_mbox = spu.channels.out_mbox.unwrap_or(0);
                                eprintln!(
                                    "[R9.1g.10] SPU halted: stop_code=0x{:x} \
                                     out_mbox=0x{:08x} steps={}",
                                    code, out_mbox, steps,
                                );
                                self.spu_exit_status = Some(out_mbox);
                            }
                            SpuStepOutcome::Continue => {
                                return Err(Error::StepsExhausted);
                            }
                            SpuStepOutcome::ChannelStall { .. } => {
                                // Mailbox SPUs may stall on IN_MBOX
                                // waiting for the PPU; for now treat
                                // as an error (R9.1g.11+ would wire
                                // bidirectional mailbox plumbing).
                                return Err(Error::StepsExhausted);
                            }
                            SpuStepOutcome::MfcUnsupported { .. } => {
                                // The SPU hit an MFC variant not in
                                // our interpreter — bridge-side path.
                                // Treat as stop for R9.1g.10's MVP.
                                self.spu_exit_status = Some(
                                    spu.channels.out_mbox.unwrap_or(0),
                                );
                            }
                        }
                    }
                } else {
                    eprintln!(
                        "[R9.1g.10] sys_spu_thread_group_start: no SPU image \
                         was imported — skipping (stub returns 0)",
                    );
                }
                self.ppu.gpr[3] = 0;
            }
            178 => {
                // R9.1g.10 — sys_spu_thread_group_join(group_id,
                //   *cause, *status)
                //
                // Writes cause = 1 (JOIN_GROUP_EXIT) and status =
                // SPU's captured OUT_MBOX value. If no SPU ran,
                // status defaults to 0.
                let cause_ptr = r4 as u32;
                let status_ptr = self.ppu.gpr[5] as u32;
                let status = self.spu_exit_status.unwrap_or(0);
                self.mem.write(cause_ptr, &1u32.to_be_bytes())?;
                self.mem.write(status_ptr, &status.to_be_bytes())?;
                eprintln!(
                    "[R9.1g.10] sys_spu_thread_group_join: cause=1 status=0x{:08x}",
                    status,
                );
                self.ppu.gpr[3] = 0;
            }
            157 => {
                // R9.1g.10 — _sys_spu_image_import(*image, src, type)
                // PSL1GHT main passes the SPU ELF blob's PPU vaddr
                // in r4. Parse it as an SPU ELF, build an SpuImage,
                // and stash on EmuCore for the later
                // `sys_spu_thread_group_start` to actually run.
                let image_out_ptr = r3 as u32;
                let src_vaddr = r4 as u32;
                // Read enough bytes of the SPU ELF to parse. SPU
                // binaries are typically a few KB; cap at 256 KB
                // (LS size). We read it lazily here.
                let mut spu_elf_bytes = vec![0u8; 0x4_0000];
                self.mem.read(src_vaddr, &mut spu_elf_bytes)?;
                let info = parse_elf(&spu_elf_bytes)?;
                if !info.is_spu() {
                    return Err(Error::ElfNotLoadable(
                        "_sys_spu_image_import: not an SPU ELF",
                    ));
                }
                let phdrs: Vec<SpuPhdr> = info
                    .program_headers
                    .iter()
                    .map(|p| SpuPhdr {
                        p_type: p.p_type,
                        p_offset: p.p_offset as u32,
                        p_vaddr: p.p_vaddr as u32,
                        p_filesz: p.p_filesz as u32,
                        p_memsz: p.p_memsz as u32,
                    })
                    .collect();
                let image = build_image(info.e_entry as u32, &phdrs, src_vaddr)
                    .map_err(Error::SpuGroup)?;
                eprintln!(
                    "[R9.1g.10] _sys_spu_image_import: entry=0x{:x} \
                     segments={} src_vaddr=0x{:08x}",
                    image.entry_point,
                    image.segments.len(),
                    src_vaddr,
                );
                // Write a basic sysSpuImage struct (16 bytes BE):
                // u32 type + u32 entry_point + u32 segs + u32 nsegs.
                self.mem.write(image_out_ptr, &0u32.to_be_bytes())?;          // type
                self.mem.write(image_out_ptr + 4, &(image.entry_point).to_be_bytes())?;
                self.mem.write(image_out_ptr + 8, &0u32.to_be_bytes())?;      // segs ptr (unused)
                self.mem.write(image_out_ptr + 12, &(image.segments.len() as u32).to_be_bytes())?;
                self.spu_image = Some(image);
                self.spu_image_src_vaddr = src_vaddr;
                self.ppu.gpr[3] = 0;
            }
            155 | 156 | 158 | 159 | 160 | 161 |
            165 | 166 | 167 |
            174 | 175 | 176 | 177 |
            179 | 180 | 181 | 182 | 184 | 185 | 186 |
            187 | 188 | 190 | 191 | 192 | 193 | 194 => {
                // R9.1g.9 — SPU support syscalls (image open/close,
                // raw_spu, group state queries + write_ls / read_ls
                // / write_snr / event_connect). Stub all with
                // CELL_OK + sentinel into r3-target ID slot.
                eprintln!(
                    "[R9.1g.9] sys_spu_* syscall #{number} stubbed \
                     (r3=0x{:x} r4=0x{:x} r5=0x{:x} r6=0x{:x})",
                    r3, r4, self.ppu.gpr[5], self.ppu.gpr[6],
                );
                if matches!(number, 156) {
                    const STUB_IMAGE_ID: u32 = 0x2000_0001;
                    let out_ptr = r3 as u32;
                    self.mem.write(out_ptr, &STUB_IMAGE_ID.to_be_bytes())?;
                }
                self.ppu.gpr[3] = 0;
            }
            // VFS wave — sys_fs_open (#801): r3=path ptr, r4=flags, r5=*fd_out,
            // r6=mode, r7=*arg, r8=size. fd is written to *fd_out (BE u32); r3
            // carries only the error code. Numbered syscall: set r3, do NOT
            // touch cia (the `sc` handler already advanced it).
            801 => {
                let path_ptr = r3 as u32;
                let flags = r4 as u32;
                let fd_out = self.ppu.gpr[5] as u32;
                let path = self.read_guest_cstr(path_ptr, 1024);
                match sys_fs_open(&mut self.vfs, &mut self.fd_table, &path, flags) {
                    Ok(fd) => {
                        self.mem.write(fd_out, &fd.to_be_bytes())?;
                        self.ppu.gpr[3] = 0; // CELL_OK
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = u64::from(u32::from(e));
                    }
                }
            }
            // VFS wave — sys_fs_read (#802): r3=fd, r4=*buf, r5=nbytes,
            // r6=*nread. lv2 fills a host buffer; copy the bytes read into the
            // guest buffer + write the count (BE u64) to *nread.
            802 => {
                let fd = r3 as u32;
                let buf_ptr = r4 as u32;
                let nbytes = self.ppu.gpr[5] as usize;
                let nread_ptr = self.ppu.gpr[6] as u32;
                let mut tmp = vec![0u8; nbytes];
                match sys_fs_read(&mut self.vfs, &mut self.fd_table, fd, &mut tmp) {
                    Ok(n) => {
                        let n_usize = n as usize;
                        if n_usize > 0 {
                            self.mem.write(buf_ptr, &tmp[..n_usize])?;
                        }
                        self.mem.write(nread_ptr, &n.to_be_bytes())?;
                        self.ppu.gpr[3] = 0; // CELL_OK
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = u64::from(u32::from(e));
                    }
                }
            }
            // VFS wave — sys_fs_close (#804): r3=fd.
            804 => {
                match sys_fs_close(&mut self.vfs, &mut self.fd_table, r3 as u32) {
                    Ok(()) => self.ppu.gpr[3] = 0, // CELL_OK
                    Err(e) => self.ppu.gpr[3] = u64::from(u32::from(e)),
                }
            }
            // VFS wave — sys_fs_stat (#808): r3=path ptr, r4=*stat (52-byte BE
            // CellFsStat). Path-based.
            808 => {
                let path_ptr = r3 as u32;
                let stat_ptr = r4 as u32;
                let path = self.read_guest_cstr(path_ptr, 1024);
                match sys_fs_stat(&self.vfs, &path) {
                    Ok(st) => {
                        self.mem.write(stat_ptr, &cellfsstat_to_be(&st))?;
                        self.ppu.gpr[3] = 0; // CELL_OK
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = u64::from(u32::from(e));
                    }
                }
            }
            // VFS wave — sys_fs_lseek (#818): r3=fd, r4=offset (u64 bits;
            // PSL1GHT sysLv2FsLSeek64 passes u64 — reinterpret as i64 for
            // signed seeks), r5=whence (s32), r6=*pos. New position -> *pos (BE).
            818 => {
                let fd = r3 as u32;
                let offset = r4 as i64;
                let whence = self.ppu.gpr[5] as i32;
                let pos_ptr = self.ppu.gpr[6] as u32;
                match sys_fs_lseek(&mut self.vfs, &mut self.fd_table, fd, offset, whence) {
                    Ok(pos) => {
                        self.mem.write(pos_ptr, &pos.to_be_bytes())?;
                        self.ppu.gpr[3] = 0; // CELL_OK
                    }
                    Err(e) => {
                        self.ppu.gpr[3] = u64::from(u32::from(e));
                    }
                }
            }
            803 => {
                // R9.1i — sys_fs_write(fd, *buf, size, *pwritten).
                // PSL1GHT's stdio may route TTY writes via the
                // fs_write path (vs sys_tty_write #403). Routes
                // fd=1/2 (stdout/stderr) to TTY ch3 + writes
                // bytes count back to *pwritten.
                let fd = r3 as i32;
                let buf_ptr = r4 as u32;
                let size = self.ppu.gpr[5] as u32;
                let pwritten_ptr = self.ppu.gpr[6] as u32;
                let mut payload = vec![0u8; size as usize];
                if size > 0 {
                    self.mem.read(buf_ptr, &mut payload).ok();
                }
                if fd == 1 || fd == 2 {
                    let s = String::from_utf8_lossy(&payload).into_owned();
                    let mut written: u32 = 0;
                    let _ = self.tty.write(
                        3, Some(&s), size, Some(&mut written),
                    );
                    eprintln!(
                        "[R9.1i] sys_fs_write(fd={fd}, *0x{buf_ptr:x}, {size}) \
                         → {written}: {s:?}",
                    );
                }
                if pwritten_ptr != 0 {
                    self.mem.write(pwritten_ptr, &(size as u64).to_be_bytes())?;
                }
                self.ppu.gpr[3] = 0;
            }
            809 => {
                // sys_fs_fstat(fd, *stat_out). VFS wave: a real file fd (>=4,
                // resolvable in fd_table) gets a proper CellFsStat via
                // fd -> handle -> path -> stat. stdio fds (0..3) and unknown
                // fds keep the S_IFCHR char-device stat so PSL1GHT stdio still
                // detects a TTY. CellFsStat is 52 bytes BE (see cellfsstat_to_be).
                let fd = r3 as u32;
                let stat_ptr = r4 as u32;
                if let Some(handle) = self.fd_table.file_handle(fd) {
                    match self.vfs.stat_handle(handle) {
                        Ok(st) => {
                            self.mem.write(stat_ptr, &cellfsstat_to_be(&st))?;
                            self.ppu.gpr[3] = 0; // CELL_OK
                        }
                        Err(e) => {
                            self.ppu.gpr[3] = u64::from(u32::from(e));
                        }
                    }
                } else {
                    // stdio / unknown fd → char device (S_IFCHR | 0o666).
                    let mode: u32 = 0x2000 | 0o666;
                    let mut buf = [0u8; 52];
                    buf[0..4].copy_from_slice(&mode.to_be_bytes());
                    self.mem.write(stat_ptr, &buf)?;
                    self.ppu.gpr[3] = 0;
                }
            }
            324 | 325 | 326 | 327 | 328 | 329 | 331 | 332 | 333 |
            334 | 335 | 336 | 337 | 338 | 339 => {
                // R9.1g.9 — mmapper / memory-container family.
                // PSL1GHT crt0 calls these to set up heap + shared
                // memory regions; for our MVP we accept all with
                // CELL_OK and (for those with *out_addr / *out_id
                // arguments at r4 or r6) write a unique sentinel
                // value so downstream code sees consistent IDs.
                //
                // 324 sys_memory_container_create(*id_out, size)
                // 325 sys_memory_container_destroy(id)
                // 326 sys_mmapper_allocate_fixed_address
                // 327 sys_mmapper_enable_page_fault_notification
                // 328 sys_mmapper_allocate_shared_memory_from_container_ext
                // 329 sys_mmapper_free_shared_memory
                // 331 sys_mmapper_free_address
                // 332 sys_mmapper_allocate_shared_memory(size, page_size, flags, *out_id)
                // 333 sys_mmapper_set_shared_memory_flag
                // 334 sys_mmapper_map_shared_memory(addr, mem_id, flags)
                // 335 sys_mmapper_unmap_shared_memory(addr, *out_id)
                // 336 sys_mmapper_change_address_access_right
                // 337 sys_mmapper_search_and_map(start, mem_id, flags, *out_addr)
                // 338 sys_mmapper_get_shared_memory_attribute
                // 339 sys_mmapper_allocate_shared_memory_ext
                eprintln!(
                    "[R9.1g.9] mmapper-family syscall #{number} stubbed \
                     (r3=0x{:x} r4=0x{:x} r5=0x{:x} r6=0x{:x})",
                    r3, r4, self.ppu.gpr[5], self.ppu.gpr[6],
                );
                // For syscalls with a *out_addr at r6, write a
                // unique sentinel so the caller sees a valid-looking
                // address. Mmapper IDs use a counter pattern.
                if matches!(number, 337) {
                    let out_addr_ptr = self.ppu.gpr[6] as u32;
                    // Use a separate sentinel base per call to
                    // ensure caller doesn't collide addresses.
                    const MAPPED_BASE: u32 = 0xB100_0000;
                    self.mem.write(out_addr_ptr, &MAPPED_BASE.to_be_bytes())?;
                }
                if matches!(number, 324 | 328 | 332 | 339) {
                    // R9.1h — id_out at r3 (memory_container_create)
                    // or r6 (allocate_shared_memory variants).
                    // Use cursor-based unique IDs so PSL1GHT's
                    // tracking tables don't collide.
                    let id_ptr = if number == 324 {
                        r3 as u32
                    } else {
                        self.ppu.gpr[6] as u32
                    };
                    self.mmapper_alloc_cursor =
                        self.mmapper_alloc_cursor.wrapping_add(1);
                    let unique_id = self.mmapper_alloc_cursor;
                    self.mem.write(id_ptr, &unique_id.to_be_bytes())?;
                }
                if matches!(number, 338) {
                    // R9.1h — sys_memory_get_user_memory_size(*info_out)
                    // writes a memory_info_t struct at r3. Use safe
                    // defaults so caller's heap-size logic sees a
                    // plausible value (256 MB total, ~256 MB free).
                    let info_ptr = r3 as u32;
                    let mut buf = [0u8; 16];
                    buf[0..4].copy_from_slice(&0x1000_0000u32.to_be_bytes());
                    buf[4..8].copy_from_slice(&0x1000_0000u32.to_be_bytes());
                    buf[8..12].copy_from_slice(&0u32.to_be_bytes());
                    buf[12..16].copy_from_slice(&0u32.to_be_bytes());
                    self.mem.write(info_ptr, &buf)?;
                }
                self.ppu.gpr[3] = 0; // CELL_OK
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
                if self.permissive_unknown_syscalls {
                    // R9.1g.9 permissive mode: log + return CELL_OK
                    // so the PPU's main() can proceed past
                    // unrecognized syscalls. `run_self` enables
                    // this; strict-mode tests leave the default
                    // off and hit the Error::UnsupportedSyscall
                    // bubble-up below.
                    eprintln!(
                        "[R9.1g.9] catch-all syscall #{number} at CIA \
                         0x{cia_at_sc:08x} stubbed (r3=0x{:x} r4=0x{:x} \
                         r5=0x{:x} r6=0x{:x}) — returning 0",
                        r3, r4, self.ppu.gpr[5], self.ppu.gpr[6],
                    );
                    self.ppu.gpr[3] = 0;
                } else {
                    return Err(Error::UnsupportedSyscall {
                        number,
                        cia: cia_at_sc,
                    });
                }
            }
        }
        Ok(None)
    }

    /// R9.1g.7 — true if `cia` is inside the import-stub region.
    /// The check is range-based so the dispatcher recognizes any
    /// stub trampoline regardless of which import it belongs to.
    #[inline]
    fn is_in_import_stub_region(&self, cia: u32) -> bool {
        cia >= USER_IMPORT_STUB_VADDR
            && cia < USER_IMPORT_STUB_VADDR.wrapping_add(USER_IMPORT_STUB_SIZE)
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

    // -----------------------------------------------------------------
    // R10.1.b — lwmutex memory layout helpers
    // -----------------------------------------------------------------

    /// Decode a PSL1GHT `sys_lwmutex_attribute_t` at `attr_ptr` using
    /// [`LwMutexAttribute::parse`].
    ///
    /// `attr_ptr == 0` returns the default attr (FIFO, non-recursive).
    /// Read errors from guest memory propagate as `Error::Memory`.
    /// Unknown protocol bytes propagate as `CellError::EINVAL` packaged
    /// as `Error::SyscallEinval` so the calling NID handler can route
    /// it to `r3` directly.
    fn read_lwmutex_attr(&self, attr_ptr: u32) -> Result<LwMutexAttribute, Error> {
        if attr_ptr == 0 {
            return Ok(LwMutexAttribute::fifo_non_recursive());
        }
        let mut buf = [0u8; LwMutexAttribute::SIZE];
        self.mem.read(attr_ptr, &mut buf)?;
        LwMutexAttribute::parse(&buf).map_err(Error::SyscallEinval)
    }

    /// R10.1.d — Decode the 40-byte BE `sys_mutex_attr_t` at `attr_ptr`
    /// into a host-side [`MutexAttr`]. Only the protocol + recursive
    /// fields are semantically modeled; the others (pshared, adaptive,
    /// key, flags, name) are read for validation but not stored.
    ///
    /// Layout (PSL1GHT `<sys/mutex.h>`):
    /// ```text
    /// 0x00  attr_protocol  u32 BE  (1=FIFO, 2=PRIO, 3=PRIO_INHERIT)
    /// 0x04  attr_recursive u32 BE  (0x10=recursive, 0x20=not_recursive)
    /// 0x08  attr_pshared   u32 BE  (0x200=NOT_PSHARED default)
    /// 0x0C  attr_adaptive  u32 BE  (0x1000 / 0x2000)
    /// 0x10  key            u64 BE
    /// 0x18  flags          s32 BE
    /// 0x1C  _pad           u32 BE
    /// 0x20  name[8]        char
    /// ```
    fn read_sys_mutex_attr(
        mem: &SparseBackend,
        attr_ptr: u32,
    ) -> Result<MutexAttr, CellError> {
        if attr_ptr == 0 {
            return Ok(MutexAttr::default());
        }
        let mut buf = [0u8; 40];
        mem.read(attr_ptr, &mut buf).map_err(|_| CellError::EFAULT)?;
        let protocol = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let recursive_raw = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        let recursive = match recursive_raw {
            0x10 => true,
            0x20 | 0 => false,
            _ => return Err(CellError::EINVAL),
        };
        match protocol {
            1 | 2 | 3 => {}
            _ => return Err(CellError::EINVAL),
        }
        Ok(MutexAttr { protocol, recursive })
    }

    /// R10.1.e — Decode the 32-byte BE `sys_sem_attr_t` at `attr_ptr`
    /// into a host-side [`SemaAttr`]. Only the protocol is
    /// semantically modeled; the others (pshared, key, flags, name)
    /// are read for validation but not stored.
    ///
    /// Layout (PSL1GHT `<sys/sem.h>`):
    /// ```text
    /// 0x00  attr_protocol  u32 BE  (1=FIFO, 2=PRIO, 3=PRIO_INHERIT)
    /// 0x04  attr_pshared   u32 BE  (0x200=PSHARED default)
    /// 0x08  key            u64 BE
    /// 0x10  flags          s32 BE
    /// 0x14  pad            u32 BE
    /// 0x18  name[8]        char
    /// ```
    fn read_sys_sem_attr(
        mem: &SparseBackend,
        attr_ptr: u32,
    ) -> Result<SemaAttr, CellError> {
        if attr_ptr == 0 {
            return Ok(SemaAttr::default());
        }
        let mut buf = [0u8; 32];
        mem.read(attr_ptr, &mut buf).map_err(|_| CellError::EFAULT)?;
        let protocol = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        match protocol {
            1 | 2 | 3 => {}
            _ => return Err(CellError::EINVAL),
        }
        Ok(SemaAttr { protocol })
    }

    /// R10.1.f — Decode the 16-byte BE `sys_event_queue_attr_t` at
    /// `attr_ptr` into a host-side [`QueueAttr`].
    ///
    /// Layout (PSL1GHT `<sys/event_queue.h>`):
    /// ```text
    /// 0x00  attr_protocol  u32 BE  (1=FIFO, 2=PRIO, 3=PRIO_INHERIT)
    /// 0x04  type           s32 BE  (1=PPU, 2=SPU)
    /// 0x08  name[8]        char
    /// ```
    fn read_sys_event_queue_attr(
        mem: &SparseBackend,
        attr_ptr: u32,
    ) -> Result<QueueAttr, CellError> {
        if attr_ptr == 0 {
            return Ok(QueueAttr::default());
        }
        let mut buf = [0u8; 16];
        mem.read(attr_ptr, &mut buf).map_err(|_| CellError::EFAULT)?;
        let protocol = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let queue_type = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
        match protocol {
            1 | 2 | 3 => {}
            _ => return Err(CellError::EINVAL),
        }
        match queue_type {
            1 | 2 => {}
            _ => return Err(CellError::EINVAL),
        }
        Ok(QueueAttr { protocol, queue_type })
    }

    /// R10.1.g — Decode the 24-byte BE `sys_cond_attr_t` at
    /// `attr_ptr` into a host-side [`CondAttr`]. Only `pshared` is
    /// validated (must be 0x100 or 0x200); the cond's protocol comes
    /// from its bound mutex, not the cond attr.
    ///
    /// Layout (PSL1GHT `<sys/cond.h>`):
    /// ```text
    /// 0x00  attr_pshared  u32 BE  (0x200=NOT_PROCESS_SHARED default)
    /// 0x04  flags         s32 BE
    /// 0x08  key           u64 BE
    /// 0x10  name[8]       char
    /// ```
    fn read_sys_cond_attr(
        mem: &SparseBackend,
        attr_ptr: u32,
    ) -> Result<CondAttr, CellError> {
        if attr_ptr == 0 {
            return Ok(CondAttr::default());
        }
        let mut buf = [0u8; 24];
        mem.read(attr_ptr, &mut buf).map_err(|_| CellError::EFAULT)?;
        let pshared = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
        match pshared {
            0x100 | 0x200 => {}
            _ => return Err(CellError::EINVAL),
        }
        Ok(CondAttr { pshared, ..CondAttr::default() })
    }

    /// Read 32 bytes from guest memory at `ctrl_ptr` and decode into a
    /// host-side [`LwMutexControl`]. The fields are big-endian on the
    /// guest; we round-trip them through the public setters so the
    /// in-host representation matches what `LwMutexControl::new`
    /// produces and the existing wrapper code can mutate it in place.
    fn read_lwmutex_control(&self, ctrl_ptr: u32) -> Result<LwMutexControl, Error> {
        let mut buf = [0u8; 32];
        self.mem.read(ctrl_ptr, &mut buf)?;
        let read_u32 = |off: usize| {
            u32::from_be_bytes([
                buf[off], buf[off + 1], buf[off + 2], buf[off + 3],
            ])
        };
        let attribute = read_u32(0x08);
        let protocol = attribute & 0xFF;
        let recursive = attribute & LWMUTEX_RECURSIVE != 0;
        let mut ctrl = LwMutexControl::new(protocol, recursive);
        ctrl.set_owner(read_u32(0x00));
        ctrl.set_waiter(read_u32(0x04));
        ctrl.set_rcount(read_u32(0x0C));
        ctrl.set_sleep_queue(read_u32(0x10));
        Ok(ctrl)
    }

    /// Write a host-side [`LwMutexControl`] back to guest memory at
    /// `ctrl_ptr`. Encodes each field as 4-byte BE; the `reserved`
    /// trailing 8 bytes are written as zero so the guest sees a
    /// canonical struct.
    fn write_lwmutex_control(
        &mut self,
        ctrl_ptr: u32,
        ctrl: &LwMutexControl,
    ) -> Result<(), Error> {
        let mut buf = [0u8; 32];
        buf[0x00..0x04].copy_from_slice(&ctrl.owner().to_be_bytes());
        buf[0x04..0x08].copy_from_slice(&ctrl.waiter().to_be_bytes());
        buf[0x08..0x0C].copy_from_slice(&ctrl.attribute().to_be_bytes());
        buf[0x0C..0x10].copy_from_slice(&ctrl.rcount().to_be_bytes());
        buf[0x10..0x14].copy_from_slice(&ctrl.sleep_queue().to_be_bytes());
        // 0x14..0x18 pad0 + 0x18..0x20 reserved stay zero.
        self.mem.write(ctrl_ptr, &buf)?;
        Ok(())
    }
}

// =====================================================================
// R9.1h helpers — minimal libc fragments for printf-family imports
// =====================================================================

/// Minimal printf-family format-string resolver. Supports `%d`,
/// `%u`, `%x`, `%X`, `%s`, `%c`, `%p`, `%%`, plus the common
/// width-prefix forms (`%08x`, `%2d`). Args pulled from the
/// supplied `args` slice (mapped from PPU r4..r10 by the caller).
fn mini_printf(fmt: &str, args: &[u64], mem: &SparseBackend) -> String {
    let mut out = String::with_capacity(fmt.len());
    let mut chars = fmt.chars().peekable();
    let mut arg_idx = 0usize;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        // Width specifier (digit-only; flags like '0' just collapse
        // into width here for the MVP).
        let mut width_buf = String::new();
        while let Some(&p) = chars.peek() {
            if p.is_ascii_digit() {
                width_buf.push(p);
                chars.next();
            } else {
                break;
            }
        }
        let width: usize = width_buf.parse().unwrap_or(0);
        let pad_zero = width_buf.starts_with('0');
        let spec = match chars.next() {
            Some(s) => s,
            None => break,
        };
        let next_arg = || {
            args.get(arg_idx).copied().unwrap_or(0)
        };
        let formatted = match spec {
            '%' => "%".to_string(),
            'd' | 'i' => {
                let v = next_arg() as i64;
                arg_idx += 1;
                v.to_string()
            }
            'u' => {
                let v = next_arg() as u32;
                arg_idx += 1;
                v.to_string()
            }
            'x' => {
                let v = next_arg() as u32;
                arg_idx += 1;
                format!("{v:x}")
            }
            'X' => {
                let v = next_arg() as u32;
                arg_idx += 1;
                format!("{v:X}")
            }
            'p' => {
                let v = next_arg() as u32;
                arg_idx += 1;
                format!("0x{v:08x}")
            }
            'c' => {
                let v = next_arg() as u8 as char;
                arg_idx += 1;
                v.to_string()
            }
            's' => {
                let v = next_arg() as u32;
                arg_idx += 1;
                read_c_string(mem, v, 4096).unwrap_or_default()
            }
            other => format!("%{other}"),
        };
        if width > formatted.len() {
            let pad = if pad_zero { '0' } else { ' ' };
            for _ in 0..(width - formatted.len()) {
                out.push(pad);
            }
        }
        out.push_str(&formatted);
    }
    out
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

    // --- Guest-PPU callback primitive (Slice 1) -----------------------
    // First re-entrant guest-call mechanism: invoke a hand-assembled guest
    // function via a compact FD, run it to the sentinel, capture r3, and
    // prove the caller's register frame is fully restored. No HLE wiring.
    const RWX: PageFlags = PageFlags::READABLE
        .union(PageFlags::WRITABLE)
        .union(PageFlags::EXECUTABLE);

    #[test]
    fn call_guest_function_runs_then_restores_frame() {
        let mut core = EmuCore::new();
        let code_addr: u32 = 0x0001_0000;
        let fd_addr: u32 = 0x0002_0000;
        core.mem.alloc_at(code_addr, 0x1000, RWX).unwrap();
        core.mem.alloc_at(fd_addr, 0x1000, RWX).unwrap();
        // fn(x) -> x + 1:  addi r3, r3, 1 ; blr
        core.mem
            .write(code_addr, &assemble(&[0x3863_0001, 0x4E80_0020]))
            .unwrap();
        core.mem.write(fd_addr, &code_addr.to_be_bytes()).unwrap();

        // Caller-frame sentinels — must survive the nested call.
        core.ppu.gpr[3] = 0x0000_DEAD;
        core.ppu.gpr[5] = 0x5555_5555;
        core.ppu.lr = 0x0000_CAFE;
        core.ppu.cia = 0x0000_1234;

        let ret = core
            .call_guest_function(fd_addr, &[41])
            .expect("call_guest_function");
        assert_eq!(ret, 42, "addi r3,r3,1 on 41 -> 42");
        assert_eq!(core.ppu.gpr[3], 0x0000_DEAD, "r3 restored");
        assert_eq!(core.ppu.gpr[5], 0x5555_5555, "r5 restored");
        assert_eq!(core.ppu.lr, 0x0000_CAFE, "lr restored");
        assert_eq!(core.ppu.cia, 0x0000_1234, "cia restored");
    }

    #[test]
    fn call_guest_function_passes_multiple_args() {
        let mut core = EmuCore::new();
        let code_addr: u32 = 0x0003_0000;
        let fd_addr: u32 = 0x0004_0000;
        core.mem.alloc_at(code_addr, 0x1000, RWX).unwrap();
        core.mem.alloc_at(fd_addr, 0x1000, RWX).unwrap();
        // fn(a, b) -> a + b:  add r3, r3, r4 ; blr
        core.mem
            .write(code_addr, &assemble(&[0x7C63_2214, 0x4E80_0020]))
            .unwrap();
        core.mem.write(fd_addr, &code_addr.to_be_bytes()).unwrap();

        let ret = core
            .call_guest_function(fd_addr, &[1000, 337])
            .expect("call_guest_function");
        assert_eq!(ret, 1337, "add r3,r3,r4 on (1000,337) -> 1337");
    }

    // --- cellSysutil callback chain (Slice 3, synthetic) --------------
    // register -> enqueue -> drain -> call_guest_function -> guest cb writes a
    // sentinel to memory. Exercises the exact logic the CheckCallback NID arm
    // runs, end-to-end and deterministically, with a hand-assembled guest cb
    // (no PSL1GHT fixture). The NID-dispatch wrapper is validated separately by
    // the real CC0 fixture.
    #[test]
    fn cellsysutil_check_callback_invokes_guest_callback() {
        let mut core = EmuCore::new();
        let cb_code: u32 = 0x0005_0000;
        let cb_fd: u32 = 0x0006_0000;
        let sentinel_cell: u32 = 0x0007_0000;
        core.mem.alloc_at(cb_code, 0x1000, RWX).unwrap();
        core.mem.alloc_at(cb_fd, 0x1000, RWX).unwrap();
        core.mem.alloc_at(sentinel_cell, 0x1000, RWX).unwrap();
        // cb(status=r3, param=r4, userdata=r5): stw r3, 0(r5) ; blr
        // (writes the low 32 bits of `status` to *userdata).
        core.mem
            .write(cb_code, &assemble(&[0x9065_0000, 0x4E80_0020]))
            .unwrap();
        core.mem.write(cb_fd, &cb_code.to_be_bytes()).unwrap();

        // Register slot 0 with userdata = the sentinel cell's address.
        cell_sysutil_register_callback(&mut core.sysutil_callbacks, 0, cb_fd, sentinel_cell)
            .unwrap();
        // Host enqueues one event (status = 0x0101, param = 0).
        core.sysutil_queue.push(0x0101, 0);

        // Drive exactly what the CheckCallback arm does.
        let dispatches =
            cell_sysutil_check_callback(&core.sysutil_callbacks, &mut core.sysutil_queue);
        assert_eq!(dispatches.len(), 1, "one slot * one event");
        for d in dispatches {
            core.call_guest_function(
                d.cb.fn_addr,
                &[u64::from(d.event), d.param, u64::from(d.cb.user_data)],
            )
            .unwrap();
        }

        // The guest callback wrote `status` (0x0101) to *userdata (BE u32),
        // and the side-effect persisted in self.mem.
        let mut buf = [0u8; 4];
        core.mem.read(sentinel_cell, &mut buf).unwrap();
        assert_eq!(
            u32::from_be_bytes(buf),
            0x0101,
            "guest callback wrote status to *userdata"
        );
        assert!(core.sysutil_queue.is_empty(), "queue drained");
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

    // -- R10.1.b — lwmutex helper round-trip ----------------------

    /// Allocate a 4 KB scratch page in `core` at `base`, suitable for
    /// hosting a 32-byte lwmutex control struct or 8-byte attr.
    fn alloc_lwmutex_scratch(core: &mut EmuCore, base: u32, size: u32) {
        core.mem
            .alloc_at(
                base,
                size,
                PageFlags::READABLE | PageFlags::WRITABLE,
            )
            .unwrap();
    }

    #[test]
    fn r10_lwmutex_attr_decodes_psl1ght_upper_nibble_encoding() {
        let mut core = EmuCore::new();
        let attr_ptr: u32 = 0x4000_0000;
        alloc_lwmutex_scratch(&mut core, attr_ptr, 0x1000);

        // PSL1GHT: protocol=0x10 (FIFO), recursive=0x20 (recursive).
        let mut buf = [0u8; 16];
        buf[0..4].copy_from_slice(&0x10u32.to_be_bytes());
        buf[4..8].copy_from_slice(&0x20u32.to_be_bytes());
        core.mem.write(attr_ptr, &buf).unwrap();

        let attr = core.read_lwmutex_attr(attr_ptr).unwrap();
        assert_eq!(attr.protocol, 0x01); // folded to kernel form
        assert!(attr.recursive);
    }

    #[test]
    fn r10_lwmutex_attr_accepts_kernel_form_unchanged() {
        let mut core = EmuCore::new();
        let attr_ptr: u32 = 0x4000_0000;
        alloc_lwmutex_scratch(&mut core, attr_ptr, 0x1000);

        // Kernel form: protocol=0x02 (priority), recursive=0x02
        // (LWMUTEX_RECURSIVE).
        let mut buf = [0u8; 16];
        buf[0..4].copy_from_slice(&0x02u32.to_be_bytes());
        buf[4..8].copy_from_slice(&LWMUTEX_RECURSIVE.to_be_bytes());
        core.mem.write(attr_ptr, &buf).unwrap();

        let attr = core.read_lwmutex_attr(attr_ptr).unwrap();
        assert_eq!(attr.protocol, 0x02);
        assert!(attr.recursive);
    }

    #[test]
    fn r10_lwmutex_attr_null_ptr_returns_default() {
        let core = EmuCore::new();
        let attr = core.read_lwmutex_attr(0).unwrap();
        assert_eq!(attr.protocol, 0x01);
        assert!(!attr.recursive);
    }

    #[test]
    fn r10_lwmutex_attr_rejects_unknown_protocol() {
        let mut core = EmuCore::new();
        let attr_ptr: u32 = 0x4000_0000;
        alloc_lwmutex_scratch(&mut core, attr_ptr, 0x1000);

        let mut buf = [0u8; 16];
        buf[0..4].copy_from_slice(&0xDEADu32.to_be_bytes());
        core.mem.write(attr_ptr, &buf).unwrap();

        assert!(matches!(
            core.read_lwmutex_attr(attr_ptr),
            Err(Error::SyscallEinval(_))
        ));
    }

    #[test]
    fn r10_lwmutex_create_lock_unlock_round_trip() {
        let mut core = EmuCore::new();
        let lock_ptr: u32 = 0x4000_1000;
        let attr_ptr: u32 = 0x4000_2000;
        alloc_lwmutex_scratch(&mut core, lock_ptr, 0x1000);
        alloc_lwmutex_scratch(&mut core, attr_ptr, 0x1000);

        // FIFO non-recursive attr.
        let mut attr_buf = [0u8; 16];
        attr_buf[0..4].copy_from_slice(&0x10u32.to_be_bytes()); // FIFO
        attr_buf[4..8].copy_from_slice(&0x10u32.to_be_bytes()); // not_recursive
        core.mem.write(attr_ptr, &attr_buf).unwrap();

        // Simulate the create syscall by hand: build the control via the
        // crate's `new` + allocate kernel queue, then write back. This
        // exercises the same read/write helpers the dispatcher uses.
        let attr = core.read_lwmutex_attr(attr_ptr).unwrap();
        let mut ctrl = LwMutexControl::new(attr.protocol, attr.recursive);
        let id = lv2_lwmutex_create(
            &mut core.lv2_sync_state,
            attr.protocol,
            &mut ctrl,
            attr.recursive,
        )
        .unwrap();
        core.write_lwmutex_control(lock_ptr, &ctrl).unwrap();

        // Verify the on-guest bytes look right.
        let mut on_guest = [0u8; 32];
        core.mem.read(lock_ptr, &mut on_guest).unwrap();
        assert_eq!(
            u32::from_be_bytes([on_guest[0], on_guest[1], on_guest[2], on_guest[3]]),
            rpcs3_lv2_lwmutex::LWMUTEX_FREE,
        );
        assert_eq!(
            u32::from_be_bytes([on_guest[16], on_guest[17], on_guest[18], on_guest[19]]),
            id,
        );

        // Lock as PPU tid.
        let mut ctrl = core.read_lwmutex_control(lock_ptr).unwrap();
        let tid = core.ppu.id as u32;
        let outcome = lv2_lwmutex_lock(
            &mut core.lv2_sync_state,
            &mut ctrl,
            id,
            tid,
            0,
        )
        .unwrap();
        assert_eq!(outcome, LockOutcome::Acquired);
        assert_eq!(ctrl.owner(), tid);
        core.write_lwmutex_control(lock_ptr, &ctrl).unwrap();

        // Unlock — no waiters, mutex becomes FREE again.
        let mut ctrl = core.read_lwmutex_control(lock_ptr).unwrap();
        let handoff =
            lv2_lwmutex_unlock(&mut core.lv2_sync_state, &mut ctrl, id, tid).unwrap();
        assert!(handoff.is_none());
        assert_eq!(ctrl.owner(), rpcs3_lv2_lwmutex::LWMUTEX_FREE);
        core.write_lwmutex_control(lock_ptr, &ctrl).unwrap();

        // Final guest-memory check.
        let mut on_guest = [0u8; 4];
        core.mem.read(lock_ptr, &mut on_guest).unwrap();
        assert_eq!(
            u32::from_be_bytes(on_guest),
            rpcs3_lv2_lwmutex::LWMUTEX_FREE,
        );
    }

    #[test]
    fn r10_lwmutex_self_recursive_increments_rcount() {
        let mut core = EmuCore::new();
        let lock_ptr: u32 = 0x4000_3000;
        alloc_lwmutex_scratch(&mut core, lock_ptr, 0x1000);

        let mut ctrl = LwMutexControl::new(0x01, true);
        let id =
            lv2_lwmutex_create(&mut core.lv2_sync_state, 0x01, &mut ctrl, true).unwrap();
        core.write_lwmutex_control(lock_ptr, &ctrl).unwrap();
        let tid = core.ppu.id as u32;

        // First lock — Acquired, rcount=1.
        let mut c = core.read_lwmutex_control(lock_ptr).unwrap();
        lv2_lwmutex_lock(&mut core.lv2_sync_state, &mut c, id, tid, 0).unwrap();
        assert_eq!(c.rcount(), 1);
        core.write_lwmutex_control(lock_ptr, &c).unwrap();

        // Re-lock by same tid — recursive, rcount=2.
        let mut c = core.read_lwmutex_control(lock_ptr).unwrap();
        lv2_lwmutex_lock(&mut core.lv2_sync_state, &mut c, id, tid, 0).unwrap();
        assert_eq!(c.rcount(), 2);
        core.write_lwmutex_control(lock_ptr, &c).unwrap();

        // Unlock once — still owned, rcount=1.
        let mut c = core.read_lwmutex_control(lock_ptr).unwrap();
        lv2_lwmutex_unlock(&mut core.lv2_sync_state, &mut c, id, tid).unwrap();
        assert_eq!(c.rcount(), 1);
        assert_eq!(c.owner(), tid);
    }
}
