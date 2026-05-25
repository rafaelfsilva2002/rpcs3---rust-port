//! `rpcs3-loader-elf-self` — ELF + SELF header parsing (no decryption).
//!
//! Mirrors behavior from:
//!   * `rpcs3/Loader/ELF.h`
//!   * `rpcs3/Crypto/unself.h`
//!
//! ## Scope of this crate (Wave 2b)
//!
//! We parse plaintext ELF binaries and the **unencrypted** portion of
//! SELF (SCE) files — that is, the SCE header and the SELF header.
//! Actual decryption of SELF segments (AES-128 CBC + HMAC validation)
//! lives in the `rpcs3-loader-self-decrypt` crate (Wave 2c) because it
//! needs `rpcs3-crypto` and key material.
//!
//! ## Binary discrimination
//!
//! The first 4 bytes of the input determine what we're looking at:
//!
//! | Magic                 | Kind     |
//! |-----------------------|----------|
//! | `7F 45 4C 46`         | ELF plaintext |
//! | `53 43 45 00`         | SELF/SCE (encrypted body) |
//! | anything else         | Unknown/invalid |
//!
//! ## PPU memory range validation
//!
//! For PPU exec ELFs, all `PT_LOAD` segments must map into
//! `[0x00000000, 0x30000000)` (ppu_load_exec enforces this in
//! `rpcs3/Emu/Cell/PPUModule.cpp:2080-2120`).

use goblin::elf::Elf;

// ---------------------------------------------------------------------
// Constants (from rpcs3/Crypto/unself.h and standard ELF spec)
// ---------------------------------------------------------------------

pub const MAGIC_ELF: [u8; 4] = [0x7F, 0x45, 0x4C, 0x46]; // "\x7fELF"
pub const MAGIC_SCE: [u8; 4] = [0x53, 0x43, 0x45, 0x00]; // "SCE\0"

/// PPU RAM window for PT_LOAD validation (ppu_load_exec).
pub const PPU_MIN_ADDR: u64 = 0x0000_0000;
pub const PPU_MAX_ADDR: u64 = 0x3000_0000;

/// Overlay window (ppu_load_overlay).
pub const OVERLAY_MIN_ADDR: u64 = 0x3000_0000;
pub const OVERLAY_MAX_ADDR: u64 = 0x4000_0000;

// SCE / SELF e_type values (unself.h:16-30)
pub const ET_SCE_EXEC: u16 = 0xFE00;
pub const ET_SCE_RELEXEC: u16 = 0xFE04;
pub const ET_SCE_PPURELEXEC: u16 = 0xFFA4;

// ELF machines (ELF.h:30-36)
pub const EM_PPC64: u16 = 0x15;
pub const EM_SPU: u16 = 0x17;
pub const EM_ARM: u16 = 0x28;
pub const EM_MIPS: u16 = 0x08;

// ELF OS ABI for Cell (ELF.h:13 + unself.h:34)
pub const ELFOSABI_CELL_LV2: u8 = 102;

// ---------------------------------------------------------------------
// Public error type
// ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Input is empty or too short to identify.
    TooShort,
    /// Neither ELF nor SELF magic.
    UnknownMagic,
    /// Recognized magic but structure is malformed.
    Malformed(String),
    /// PT_LOAD segment maps outside the allowed window.
    AddressRangeViolation {
        p_vaddr: u64,
        p_memsz: u64,
        allowed_min: u64,
        allowed_max: u64,
    },
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::TooShort => f.write_str("input too short to identify"),
            Error::UnknownMagic => f.write_str("unknown magic (not ELF nor SELF)"),
            Error::Malformed(m) => write!(f, "malformed binary: {m}"),
            Error::AddressRangeViolation { p_vaddr, p_memsz, allowed_min, allowed_max } => {
                write!(
                    f,
                    "PT_LOAD [0x{p_vaddr:x}, 0x{:x}) outside allowed [0x{allowed_min:x}, 0x{allowed_max:x})",
                    p_vaddr + p_memsz
                )
            }
        }
    }
}

impl std::error::Error for Error {}

// ---------------------------------------------------------------------
// Binary kind discrimination
// ---------------------------------------------------------------------

/// What the first 4 bytes of the input say the binary is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryKind {
    Elf,
    Self_,
    Unknown,
}

/// Identify the binary type by magic only (O(1), no parsing).
#[must_use]
pub fn identify(bytes: &[u8]) -> BinaryKind {
    if bytes.len() < 4 {
        return BinaryKind::Unknown;
    }
    let magic: [u8; 4] = bytes[..4].try_into().unwrap();
    if magic == MAGIC_ELF {
        BinaryKind::Elf
    } else if magic == MAGIC_SCE {
        BinaryKind::Self_
    } else {
        BinaryKind::Unknown
    }
}

// ---------------------------------------------------------------------
// ELF parsing (thin wrapper over goblin)
// ---------------------------------------------------------------------

/// Flattened view of what we care about from an ELF header.
#[derive(Debug, Clone)]
pub struct ElfInfo {
    pub e_class: u8,        // 1 = ELF32, 2 = ELF64
    pub e_data: u8,         // 1 = LE, 2 = BE
    pub e_os_abi: u8,
    pub e_type: u16,
    pub e_machine: u16,
    pub e_entry: u64,
    pub program_headers: Vec<ProgramHeader>,
}

#[derive(Debug, Clone, Copy)]
pub struct ProgramHeader {
    pub p_type: u32,
    pub p_flags: u32,
    pub p_offset: u64,
    pub p_vaddr: u64,
    pub p_paddr: u64,
    pub p_filesz: u64,
    pub p_memsz: u64,
    pub p_align: u64,
}

impl ElfInfo {
    /// Returns true if this is a PPU64 (PowerPC 64-bit) LV2 binary.
    #[must_use]
    pub fn is_ppu64(&self) -> bool {
        self.e_machine == EM_PPC64 && self.e_class == 2
    }

    /// Returns true if this is an SPU ELF.
    #[must_use]
    pub fn is_spu(&self) -> bool {
        self.e_machine == EM_SPU
    }

    /// Iterate PT_LOAD segments (program header type == 1).
    pub fn pt_load_iter(&self) -> impl Iterator<Item = &ProgramHeader> {
        self.program_headers.iter().filter(|p| p.p_type == 1)
    }

    /// R9.1g.2 — return the PT_SCE_PPU_PROCESS_PARAM segment (custom
    /// PHDR type `0x60000001`) if present. PSL1GHT-built `.self`
    /// binaries emit this for every executable; it holds the
    /// `sys_process_param_t` block (priority + stack size + sdk_ver
    /// + magic).
    pub fn pt_proc_param(&self) -> Option<&ProgramHeader> {
        self.program_headers
            .iter()
            .find(|p| p.p_type == PT_SCE_PPU_PROCESS_PARAM)
    }

    /// R9.1g.3 — return the PT_SCE_PPU_PROC_PARAM segment (custom
    /// PHDR type `0x60000002`) if present. Holds pointers to
    /// malloc/free init hooks lv2 must call before _start.
    pub fn pt_proc_proc_param(&self) -> Option<&ProgramHeader> {
        self.program_headers
            .iter()
            .find(|p| p.p_type == PT_SCE_PPU_PROC_PARAM)
    }

    /// R9.1g.4 — return the PT_TLS segment if present. PSL1GHT
    /// binaries emit one PT_TLS that describes the thread-local
    /// storage template: `p_filesz` bytes of initialized data
    /// (sometimes 0 = pure tbss / zero-init), `p_memsz` total
    /// bytes per thread, and `p_align` byte alignment.
    pub fn pt_tls(&self) -> Option<&ProgramHeader> {
        self.program_headers
            .iter()
            .find(|p| p.p_type == PT_TLS)
    }
}

/// Standard ELF program-header type for thread-local storage.
pub const PT_TLS: u32 = 7;

/// R9.1g.2 — custom PHDR type for PSL1GHT `sys_process_param_t`
/// segment. The standard ELF spec reserves the `0x60000000..=0x6FFFFFFF`
/// range for OS-specific use; PS3 lv2 occupies a subset.
pub const PT_SCE_PPU_PROCESS_PARAM: u32 = 0x6000_0001;

/// R9.1g.3 — custom PHDR type for the proc_param struct (malloc /
/// free / fixed-alloc init hooks).
pub const PT_SCE_PPU_PROC_PARAM: u32 = 0x6000_0002;

/// R9.1g.2 — parsed contents of the `sys_process_param_t` segment
/// (PT_SCE_PPU_PROCESS_PARAM, 32 bytes). Field layout reverse-
/// engineered from PSL1GHT-built `.self` binaries; cross-verified
/// against the `SYS_PROCESS_PARAM(prio, stack)` macro values:
/// observed values for `single_spu_mailbox_v1.self` match the
/// macro inputs (prio=1001, stack=0x10000).
///
/// All fields are big-endian on the wire (PowerPC native).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysProcessParam {
    /// Self-reported size of this struct (always `0x20` for v1).
    pub size: u32,
    /// Magic value `0x13BCC5F6` identifying the PSL1GHT v1 layout.
    pub magic: u32,
    /// SDK version reported to the loader (e.g. `0x00009000`).
    pub sdk_version: u32,
    /// PS3 firmware-version code the binary was built against
    /// (e.g. `0x00192001` = 4.93). Not always meaningful for
    /// homebrew, but the loader records it for diagnostics.
    pub fw_version_code: u32,
    /// Primary thread priority (matches `SYS_PROCESS_PARAM` macro
    /// arg 1; e.g. 1001 for our oracle fixtures).
    pub primary_prio: u32,
    /// Primary thread stack size in bytes (matches macro arg 2;
    /// e.g. `0x10000` = 64 KB for our oracle fixtures).
    pub primary_stacksize: u32,
    /// Default page size for the lv2 user-mode `malloc` heap
    /// (typically `0x100000` = 1 MB).
    pub malloc_pagesize: u32,
    /// Reserved (zero in observed binaries).
    pub reserved: u32,
}

pub const SYS_PROCESS_PARAM_SIZE: usize = 32;
pub const SYS_PROCESS_PARAM_MAGIC: u32 = 0x13BC_C5F6;

/// R9.1g.3/.5 — parsed contents of the `ppu_proc_prx_param_t`
/// segment (PT_SCE_PPU_PROC_PARAM, 40 bytes). Holds the start /
/// end vaddrs of the `.libent` (library-export descriptor) and
/// `.libstub` (library-import descriptor) sections plus a few
/// PSL1GHT runtime fields. Field names + semantics aligned with
/// RPCS3 C++ `PPUModule.cpp:2479` (struct ppu_proc_prx_param_t).
///
/// In R9.1g.3 these fields were initially mis-named as
/// `prx_*_table` (based on incomplete reverse-engineering); the
/// correct names per RPCS3's reference loader are `libent_*` /
/// `libstub_*`. The corrected field set lands in R9.1g.5.
///
/// All fields are big-endian on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysProcPrxParam {
    /// Self-reported size of this struct (always 0x28).
    pub size: u32,
    /// Magic value `0x1B434CEC` identifying the v1 layout.
    pub magic: u32,
    /// Version field (observed: 2).
    pub version: u32,
    /// Reserved (observed: 0).
    pub unk0: u32,
    /// Start vaddr of the `.libent` section (library exports).
    /// For executables that don't export anything (most PSL1GHT
    /// homebrews including all 20 R8.x oracle fixtures),
    /// `libent_start == libent_end` (empty range).
    pub libent_start: u32,
    /// End vaddr of the `.libent` section.
    pub libent_end: u32,
    /// Start vaddr of the `.libstub` section (library imports).
    /// Each entry is a `PpuPrxModuleInfo` (44 bytes) describing
    /// one imported PRX module + its NID/address arrays.
    pub libstub_start: u32,
    /// End vaddr of the `.libstub` section.
    pub libstub_end: u32,
    /// SDK / runtime version (observed: 0x0101).
    pub ver: u16,
    /// Unknown / runtime flags (observed: 0x0000).
    pub unk1: u16,
    /// Reserved (observed: 0).
    pub unk2: u32,
}

pub const SYS_PROC_PRX_PARAM_SIZE: usize = 40;
pub const SYS_PROC_PRX_PARAM_MAGIC: u32 = 0x1B43_4CEC;

impl SysProcPrxParam {
    /// Parse the 40-byte BE struct from a raw byte slice. Errors
    /// if the slice is shorter than `SYS_PROC_PRX_PARAM_SIZE` or
    /// if the `size`/`magic` fields disagree.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < SYS_PROC_PRX_PARAM_SIZE {
            return Err(Error::TooShort);
        }
        let size = read_u32_be(bytes, 0)?;
        let magic = read_u32_be(bytes, 4)?;
        if size as usize != SYS_PROC_PRX_PARAM_SIZE {
            return Err(Error::Malformed(format!(
                "sys_proc_prx_param size 0x{size:x} != expected 0x{:x}",
                SYS_PROC_PRX_PARAM_SIZE
            )));
        }
        if magic != SYS_PROC_PRX_PARAM_MAGIC {
            return Err(Error::Malformed(format!(
                "sys_proc_prx_param magic 0x{magic:08x} != expected 0x{:08x}",
                SYS_PROC_PRX_PARAM_MAGIC
            )));
        }
        Ok(Self {
            size,
            magic,
            version: read_u32_be(bytes, 8)?,
            unk0: read_u32_be(bytes, 12)?,
            libent_start: read_u32_be(bytes, 16)?,
            libent_end: read_u32_be(bytes, 20)?,
            libstub_start: read_u32_be(bytes, 24)?,
            libstub_end: read_u32_be(bytes, 28)?,
            ver: read_u16_be(bytes, 32)?,
            unk1: read_u16_be(bytes, 34)?,
            unk2: read_u32_be(bytes, 36)?,
        })
    }
}

/// R9.1g.5 — single libstub entry describing one imported PRX
/// module. Each entry is **44 bytes** in the `.libstub` section.
/// Field layout mirrors RPCS3 C++ `struct ppu_prx_module_info`
/// (PPUModule.cpp:675). All multi-byte fields are big-endian.
///
/// The `addrs` field points to an array of `num_func` u32 slots
/// that the loader is expected to **populate at startup** with
/// the resolved function descriptor pointers for each imported
/// function (identified by the parallel NID array at `nids`).
/// This is the mechanism R9.1f's crash exposed as missing — the
/// addrs[] slots contain unresolved data until R9.1g.6 populates
/// them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuPrxModuleInfo {
    /// Self-reported size (always 0x2C = 44).
    pub size: u8,
    /// Reserved (typically 0).
    pub unk0: u8,
    /// Library version code.
    pub version: u16,
    /// Library attributes bitfield.
    pub attributes: u16,
    /// Number of imported functions (parallel to `nids` / `addrs`).
    pub num_func: u16,
    /// Number of imported variables.
    pub num_var: u16,
    /// Number of imported TLS variables.
    pub num_tlsvar: u16,
    pub info_hash: u8,
    pub info_tlshash: u8,
    /// Reserved (2 bytes).
    pub unk1: [u8; 2],
    /// vaddr of the null-terminated module name string.
    pub name: u32,
    /// vaddr of the NID array (`num_func` × u32 BE).
    pub nids: u32,
    /// vaddr of the address array (`num_func` × u32 BE) — the
    /// loader writes resolved FD pointers here at startup.
    pub addrs: u32,
    /// vaddr of the imported-variable NID array.
    pub vnids: u32,
    /// vaddr of the imported-variable stub array.
    pub vstubs: u32,
    pub unk4: u32,
    pub unk5: u32,
}

pub const PPU_PRX_MODULE_INFO_SIZE: usize = 44;

impl PpuPrxModuleInfo {
    /// Parse a single 44-byte BE-encoded libstub entry from the
    /// given slice. Errors if the slice is shorter than
    /// `PPU_PRX_MODULE_INFO_SIZE` or the self-reported `size`
    /// byte disagrees.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < PPU_PRX_MODULE_INFO_SIZE {
            return Err(Error::TooShort);
        }
        let size = bytes[0];
        if size as usize != PPU_PRX_MODULE_INFO_SIZE {
            return Err(Error::Malformed(format!(
                "ppu_prx_module_info size {size} != expected {}",
                PPU_PRX_MODULE_INFO_SIZE
            )));
        }
        Ok(Self {
            size,
            unk0: bytes[1],
            version: read_u16_be(bytes, 2)?,
            attributes: read_u16_be(bytes, 4)?,
            num_func: read_u16_be(bytes, 6)?,
            num_var: read_u16_be(bytes, 8)?,
            num_tlsvar: read_u16_be(bytes, 10)?,
            info_hash: bytes[12],
            info_tlshash: bytes[13],
            unk1: [bytes[14], bytes[15]],
            name: read_u32_be(bytes, 16)?,
            nids: read_u32_be(bytes, 20)?,
            addrs: read_u32_be(bytes, 24)?,
            vnids: read_u32_be(bytes, 28)?,
            vstubs: read_u32_be(bytes, 32)?,
            unk4: read_u32_be(bytes, 36)?,
            unk5: read_u32_be(bytes, 40)?,
        })
    }
}

impl SysProcessParam {
    /// Parse the 32-byte BE struct from a raw byte slice. Errors if
    /// the slice is shorter than `SYS_PROCESS_PARAM_SIZE` or if the
    /// `size` field disagrees with the slice length / `magic` is
    /// off.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < SYS_PROCESS_PARAM_SIZE {
            return Err(Error::TooShort);
        }
        let size = read_u32_be(bytes, 0)?;
        let magic = read_u32_be(bytes, 4)?;
        if size as usize != SYS_PROCESS_PARAM_SIZE {
            return Err(Error::Malformed(format!(
                "sys_process_param size 0x{size:x} != expected 0x{:x}",
                SYS_PROCESS_PARAM_SIZE
            )));
        }
        if magic != SYS_PROCESS_PARAM_MAGIC {
            return Err(Error::Malformed(format!(
                "sys_process_param magic 0x{magic:08x} != expected 0x{:08x}",
                SYS_PROCESS_PARAM_MAGIC
            )));
        }
        Ok(Self {
            size,
            magic,
            sdk_version: read_u32_be(bytes, 8)?,
            fw_version_code: read_u32_be(bytes, 12)?,
            primary_prio: read_u32_be(bytes, 16)?,
            primary_stacksize: read_u32_be(bytes, 20)?,
            malloc_pagesize: read_u32_be(bytes, 24)?,
            reserved: read_u32_be(bytes, 28)?,
        })
    }
}

/// Parse an ELF header + program headers. Delegates to `goblin`.
pub fn parse_elf(bytes: &[u8]) -> Result<ElfInfo, Error> {
    let elf = Elf::parse(bytes).map_err(|e| Error::Malformed(e.to_string()))?;
    let header = elf.header;

    let phs = elf
        .program_headers
        .iter()
        .map(|p| ProgramHeader {
            p_type: p.p_type,
            p_flags: p.p_flags,
            p_offset: p.p_offset,
            p_vaddr: p.p_vaddr,
            p_paddr: p.p_paddr,
            p_filesz: p.p_filesz,
            p_memsz: p.p_memsz,
            p_align: p.p_align,
        })
        .collect();

    Ok(ElfInfo {
        e_class: if elf.is_64 { 2 } else { 1 },
        e_data: if elf.little_endian { 1 } else { 2 },
        e_os_abi: header.e_ident[goblin::elf::header::EI_OSABI],
        e_type: header.e_type,
        e_machine: header.e_machine,
        e_entry: header.e_entry,
        program_headers: phs,
    })
}

/// Validate that every PT_LOAD segment maps into the allowed window.
/// Mirrors the check `ppu_load_exec` runs before mapping into VM.
pub fn validate_load_range(info: &ElfInfo, min: u64, max: u64) -> Result<(), Error> {
    for ph in info.pt_load_iter() {
        let end = ph.p_vaddr.saturating_add(ph.p_memsz);
        if ph.p_vaddr < min || end > max {
            return Err(Error::AddressRangeViolation {
                p_vaddr: ph.p_vaddr,
                p_memsz: ph.p_memsz,
                allowed_min: min,
                allowed_max: max,
            });
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------
// SELF / SCE header parsing (manual)
// ---------------------------------------------------------------------
//
// SCE header (32 bytes) — first 4 bytes LE, rest BE.
// Layout taken from rpcs3/Crypto/unself.h (SceHeader struct).

/// SCE header of a SELF file.
#[derive(Debug, Clone, Copy)]
pub struct SceHeader {
    pub magic: u32,             // LE; equals 0x00454353 when read as LE u32 of "SCE\0"
    pub header_version: u32,    // LE
    pub key_revision: u16,      // BE
    pub header_type: u16,       // BE; 1=SELF, 2=RVK, 3=PKG, 4=SPP
    pub metadata_offset: u32,   // BE
    pub header_length: u64,     // BE
    pub elf_size: u64,          // BE
}

pub const SCE_HEADER_SIZE: usize = 32;

/// SELF extended header (follows SCE header for header_type=1).
/// Layout matches the SELF header struct in rpcs3/Crypto/unself.h.
#[derive(Debug, Clone, Copy)]
pub struct SelfExtHeader {
    pub self_type: u64,              // BE; 1=LV0 2=LV1 3=LV2 4=APP 5=ISO 6=LDR 8=NPDRM
    pub app_info_offset: u64,
    pub elf_offset: u64,
    pub program_header_offset: u64,
    pub section_header_offset: u64,
    pub segment_info_offset: u64,
    pub sce_version_offset: u64,
    pub control_info_offset: u64,
    pub control_info_size: u64,
    pub padding: u64,
}

pub const SELF_EXT_HEADER_SIZE: usize = 80;

/// Known SELF `self_type` values, from unself.h.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfType {
    Lv0 = 1,
    Lv1 = 2,
    Lv2 = 3,
    App = 4,
    Iso = 5,
    Ldr = 6,
    Npdrm = 8,
    Unknown = 0xFFFF_FFFF,
}

impl From<u64> for SelfType {
    fn from(v: u64) -> Self {
        match v {
            1 => Self::Lv0,
            2 => Self::Lv1,
            3 => Self::Lv2,
            4 => Self::App,
            5 => Self::Iso,
            6 => Self::Ldr,
            8 => Self::Npdrm,
            _ => Self::Unknown,
        }
    }
}

fn read_u16_be(buf: &[u8], at: usize) -> Result<u16, Error> {
    buf.get(at..at + 2)
        .and_then(|s| s.try_into().ok())
        .map(u16::from_be_bytes)
        .ok_or_else(|| Error::Malformed(format!("unexpected EOF reading u16 at {at}")))
}

fn read_u32_be(buf: &[u8], at: usize) -> Result<u32, Error> {
    buf.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_be_bytes)
        .ok_or_else(|| Error::Malformed(format!("unexpected EOF reading u32 at {at}")))
}

fn read_u32_le(buf: &[u8], at: usize) -> Result<u32, Error> {
    buf.get(at..at + 4)
        .and_then(|s| s.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| Error::Malformed(format!("unexpected EOF reading u32 at {at}")))
}

fn read_u64_be(buf: &[u8], at: usize) -> Result<u64, Error> {
    buf.get(at..at + 8)
        .and_then(|s| s.try_into().ok())
        .map(u64::from_be_bytes)
        .ok_or_else(|| Error::Malformed(format!("unexpected EOF reading u64 at {at}")))
}

/// Parse the SCE header.
pub fn parse_sce_header(bytes: &[u8]) -> Result<SceHeader, Error> {
    if bytes.len() < SCE_HEADER_SIZE {
        return Err(Error::TooShort);
    }
    let magic = read_u32_le(bytes, 0)?;
    if magic.to_le_bytes() != MAGIC_SCE {
        return Err(Error::UnknownMagic);
    }

    Ok(SceHeader {
        magic,
        header_version: read_u32_le(bytes, 4)?,
        key_revision: read_u16_be(bytes, 8)?,
        header_type: read_u16_be(bytes, 10)?,
        metadata_offset: read_u32_be(bytes, 12)?,
        header_length: read_u64_be(bytes, 16)?,
        elf_size: read_u64_be(bytes, 24)?,
    })
}

/// Parse the SELF extended header, which follows the SCE header for
/// SCE files of header_type=1 (SELF).
pub fn parse_self_ext_header(bytes: &[u8]) -> Result<SelfExtHeader, Error> {
    let base = SCE_HEADER_SIZE;
    if bytes.len() < base + SELF_EXT_HEADER_SIZE {
        return Err(Error::TooShort);
    }

    Ok(SelfExtHeader {
        self_type: read_u64_be(bytes, base)?,
        app_info_offset: read_u64_be(bytes, base + 8)?,
        elf_offset: read_u64_be(bytes, base + 16)?,
        program_header_offset: read_u64_be(bytes, base + 24)?,
        section_header_offset: read_u64_be(bytes, base + 32)?,
        segment_info_offset: read_u64_be(bytes, base + 40)?,
        sce_version_offset: read_u64_be(bytes, base + 48)?,
        control_info_offset: read_u64_be(bytes, base + 56)?,
        control_info_size: read_u64_be(bytes, base + 64)?,
        padding: read_u64_be(bytes, base + 72)?,
    })
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- R9.1g.2 sys_process_param parser --------------------------

    /// Empirical bytes from `single_spu_mailbox_v1.self` PHDR[6]
    /// (PT_SCE_PPU_PROCESS_PARAM segment at vaddr 0x2BEC0).
    /// SYS_PROCESS_PARAM(1001, 0x10000) → prio=0x3E9, stack=0x10000.
    const MAILBOX_V1_PROC_PARAM_BYTES: [u8; 32] = [
        0x00, 0x00, 0x00, 0x20,   // size = 32
        0x13, 0xBC, 0xC5, 0xF6,   // magic
        0x00, 0x00, 0x90, 0x00,   // sdk_version
        0x00, 0x19, 0x20, 0x01,   // fw_version_code
        0x00, 0x00, 0x03, 0xE9,   // primary_prio = 1001
        0x00, 0x01, 0x00, 0x00,   // primary_stacksize = 64 KB
        0x00, 0x10, 0x00, 0x00,   // malloc_pagesize = 1 MB
        0x00, 0x00, 0x00, 0x00,   // reserved
    ];

    #[test]
    fn sys_process_param_parses_mailbox_v1_layout() {
        let p = SysProcessParam::parse(&MAILBOX_V1_PROC_PARAM_BYTES).unwrap();
        assert_eq!(p.size, 0x20);
        assert_eq!(p.magic, SYS_PROCESS_PARAM_MAGIC);
        assert_eq!(p.sdk_version, 0x9000);
        assert_eq!(p.fw_version_code, 0x0019_2001);
        assert_eq!(p.primary_prio, 1001);
        assert_eq!(p.primary_stacksize, 0x10000);
        assert_eq!(p.malloc_pagesize, 0x10_0000);
        assert_eq!(p.reserved, 0);
    }

    #[test]
    fn sys_process_param_rejects_short_input() {
        let bytes = [0u8; 16];
        assert!(matches!(
            SysProcessParam::parse(&bytes),
            Err(Error::TooShort)
        ));
    }

    #[test]
    fn sys_process_param_rejects_wrong_magic() {
        let mut bytes = MAILBOX_V1_PROC_PARAM_BYTES;
        bytes[4..8].copy_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
        assert!(matches!(
            SysProcessParam::parse(&bytes),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn sys_process_param_rejects_wrong_size_field() {
        let mut bytes = MAILBOX_V1_PROC_PARAM_BYTES;
        bytes[0..4].copy_from_slice(&0x40u32.to_be_bytes()); // size says 64, not 32
        assert!(matches!(
            SysProcessParam::parse(&bytes),
            Err(Error::Malformed(_))
        ));
    }

    // -- R9.1g.3 sys_proc_prx_param parser -------------------------

    /// Empirical bytes from `single_spu_mailbox_v1.self` PHDR[7]
    /// (PT_SCE_PPU_PROC_PARAM segment at vaddr 0x2BEE0). The three
    /// prx_*_table pointers all reference 0x0002BE94 in the
    /// observed mailbox_v1 layout; the sys_process_param_ptr
    /// back-references PHDR[6]'s vaddr 0x0002BEC0.
    const MAILBOX_V1_PRX_PARAM_BYTES: [u8; 40] = [
        0x00, 0x00, 0x00, 0x28,   // size = 40
        0x1B, 0x43, 0x4C, 0xEC,   // magic
        0x00, 0x00, 0x00, 0x02,   // version = 2
        0x00, 0x00, 0x00, 0x00,   // reserved0
        0x00, 0x02, 0xBE, 0x94,   // prx_load_table
        0x00, 0x02, 0xBE, 0x94,   // prx_unload_table
        0x00, 0x02, 0xBE, 0x94,   // prx_resident_table
        0x00, 0x02, 0xBE, 0xC0,   // sys_process_param_ptr (= PHDR[6] vaddr)
        0x01, 0x01, 0x00, 0x00,   // flags
        0x00, 0x00, 0x00, 0x00,   // reserved1
    ];

    #[test]
    fn sys_proc_prx_param_parses_mailbox_v1_layout() {
        let p = SysProcPrxParam::parse(&MAILBOX_V1_PRX_PARAM_BYTES).unwrap();
        assert_eq!(p.size, 0x28);
        assert_eq!(p.magic, SYS_PROC_PRX_PARAM_MAGIC);
        assert_eq!(p.version, 2);
        assert_eq!(p.unk0, 0);
        // R9.1g.5 — corrected field names (libent/libstub) per
        // RPCS3 C++ reference layout. mailbox_v1 has:
        //   libent: empty range (no exports)
        //   libstub: 0x2BE94..0x2BEC0 (single 44-byte entry = 1 imported module)
        assert_eq!(p.libent_start, 0x2BE94);
        assert_eq!(p.libent_end, 0x2BE94, "no exports — empty range");
        assert_eq!(p.libstub_start, 0x2BE94);
        assert_eq!(
            p.libstub_end - p.libstub_start,
            PPU_PRX_MODULE_INFO_SIZE as u32,
            "exactly one 44-byte libstub entry",
        );
        assert_eq!(p.libstub_end, 0x2BEC0);
        assert_eq!(p.ver, 0x0101);
        assert_eq!(p.unk1, 0x0000);
        assert_eq!(p.unk2, 0);
    }

    #[test]
    fn sys_proc_prx_param_rejects_short_input() {
        let bytes = [0u8; 20];
        assert!(matches!(
            SysProcPrxParam::parse(&bytes),
            Err(Error::TooShort)
        ));
    }

    #[test]
    fn sys_proc_prx_param_rejects_wrong_magic() {
        let mut bytes = MAILBOX_V1_PRX_PARAM_BYTES;
        bytes[4..8].copy_from_slice(&0xCAFE_BABEu32.to_be_bytes());
        assert!(matches!(
            SysProcPrxParam::parse(&bytes),
            Err(Error::Malformed(_))
        ));
    }

    #[test]
    fn sys_proc_prx_param_rejects_wrong_size_field() {
        let mut bytes = MAILBOX_V1_PRX_PARAM_BYTES;
        bytes[0..4].copy_from_slice(&0x100u32.to_be_bytes());
        assert!(matches!(
            SysProcPrxParam::parse(&bytes),
            Err(Error::Malformed(_))
        ));
    }

    // -- R9.1g.5 PpuPrxModuleInfo (libstub entry) parser -----------

    /// Empirical bytes from `single_spu_mailbox_v1.self` at
    /// vaddr 0x2BE94 (first libstub entry). 119 imported
    /// functions; module name + NID array + addrs array all
    /// in PHDR[0] (.text + .rodata) and PHDR[1] (.data).
    const MAILBOX_V1_LIBSTUB_ENTRY_BYTES: [u8; 44] = [
        0x2C, 0x00,             // size + unk0
        0x00, 0x01,             // version
        0x00, 0x09,             // attributes
        0x00, 0x77,             // num_func = 119
        0x00, 0x00,             // num_var
        0x00, 0x00,             // num_tlsvar
        0x00, 0x00,             // info_hash + info_tlshash
        0x00, 0x00,             // unk1
        0x00, 0x02, 0xBC, 0xA8, // name
        0x00, 0x02, 0xBC, 0xB8, // nids
        0x00, 0x03, 0x00, 0x40, // addrs ← loader populates this
        0x00, 0x00, 0x00, 0x00, // vnids
        0x00, 0x00, 0x00, 0x00, // vstubs
        0x00, 0x00, 0x00, 0x00, // unk4
        0x00, 0x00, 0x00, 0x20, // unk5
    ];

    #[test]
    fn ppu_prx_module_info_parses_mailbox_v1_entry() {
        let m = PpuPrxModuleInfo::parse(&MAILBOX_V1_LIBSTUB_ENTRY_BYTES).unwrap();
        assert_eq!(m.size, 0x2C);
        assert_eq!(m.unk0, 0);
        assert_eq!(m.version, 0x0001);
        assert_eq!(m.attributes, 0x0009);
        assert_eq!(m.num_func, 119);
        assert_eq!(m.num_var, 0);
        assert_eq!(m.num_tlsvar, 0);
        assert_eq!(m.name, 0x0002BCA8);
        assert_eq!(m.nids, 0x0002BCB8);
        assert_eq!(m.addrs, 0x00030040,
            "addrs[] table is where the loader writes resolved FDs");
        // `addrs` start is at 0x30040; ends at 0x30040 + 119*4 = 0x301BC.
        // The R9.1f crash site read mem[0x30108] which is
        // addrs[(0x30108 - 0x30040) / 4] = addrs[50].
        let crash_index = (0x30108u32 - m.addrs) / 4;
        assert_eq!(crash_index, 50);
        assert!(crash_index < m.num_func as u32);
        assert_eq!(m.vnids, 0);
        assert_eq!(m.vstubs, 0);
    }

    #[test]
    fn ppu_prx_module_info_rejects_short_input() {
        let bytes = [0u8; 20];
        assert!(matches!(
            PpuPrxModuleInfo::parse(&bytes),
            Err(Error::TooShort)
        ));
    }

    #[test]
    fn ppu_prx_module_info_rejects_wrong_size_byte() {
        let mut bytes = MAILBOX_V1_LIBSTUB_ENTRY_BYTES;
        bytes[0] = 0x40; // claim 64 bytes, not 44
        assert!(matches!(
            PpuPrxModuleInfo::parse(&bytes),
            Err(Error::Malformed(_))
        ));
    }

    // -- Magic identification --------------------------------------

    #[test]
    fn identify_elf_magic() {
        let mut bytes = vec![0x7F, 0x45, 0x4C, 0x46];
        bytes.extend_from_slice(&[0; 60]);
        assert_eq!(identify(&bytes), BinaryKind::Elf);
    }

    #[test]
    fn identify_sce_magic() {
        let bytes = [0x53, 0x43, 0x45, 0x00, 0, 0, 0, 2];
        assert_eq!(identify(&bytes), BinaryKind::Self_);
    }

    #[test]
    fn identify_unknown_magic() {
        assert_eq!(identify(b"MZ\0\0"), BinaryKind::Unknown);
    }

    #[test]
    fn identify_too_short() {
        assert_eq!(identify(&[0x7F, 0x45]), BinaryKind::Unknown);
        assert_eq!(identify(&[]), BinaryKind::Unknown);
    }

    // -- SCE header parsing ----------------------------------------

    #[test]
    fn sce_header_minimal() {
        // Build a 32-byte SCE header with known fields.
        let mut bytes = vec![0u8; SCE_HEADER_SIZE];
        bytes[0..4].copy_from_slice(&MAGIC_SCE);
        bytes[4..8].copy_from_slice(&2u32.to_le_bytes()); // version
        bytes[8..10].copy_from_slice(&0x000Au16.to_be_bytes()); // key_revision
        bytes[10..12].copy_from_slice(&1u16.to_be_bytes()); // header_type = SELF
        bytes[12..16].copy_from_slice(&0x0000_1000u32.to_be_bytes()); // metadata_offset
        bytes[16..24].copy_from_slice(&0x0000_0000_0000_1000u64.to_be_bytes()); // header_length
        bytes[24..32].copy_from_slice(&0x0000_0000_0020_0000u64.to_be_bytes()); // elf_size

        let h = parse_sce_header(&bytes).unwrap();
        assert_eq!(h.header_version, 2);
        assert_eq!(h.key_revision, 0x000A);
        assert_eq!(h.header_type, 1);
        assert_eq!(h.metadata_offset, 0x1000);
        assert_eq!(h.header_length, 0x1000);
        assert_eq!(h.elf_size, 0x0020_0000);
    }

    #[test]
    fn sce_header_rejects_short_input() {
        assert_eq!(parse_sce_header(&[0u8; 16]).unwrap_err(), Error::TooShort);
    }

    #[test]
    fn sce_header_rejects_non_sce_magic() {
        let mut bytes = vec![0u8; SCE_HEADER_SIZE];
        bytes[0..4].copy_from_slice(b"XXXX");
        assert_eq!(parse_sce_header(&bytes).unwrap_err(), Error::UnknownMagic);
    }

    // -- SELF ext header parsing -----------------------------------

    #[test]
    fn self_ext_header_parses_self_type() {
        let mut bytes = vec![0u8; SCE_HEADER_SIZE + SELF_EXT_HEADER_SIZE];
        bytes[0..4].copy_from_slice(&MAGIC_SCE);
        // self_type = 4 (APP)
        bytes[SCE_HEADER_SIZE..SCE_HEADER_SIZE + 8]
            .copy_from_slice(&4u64.to_be_bytes());
        // elf_offset = 0x1000
        bytes[SCE_HEADER_SIZE + 16..SCE_HEADER_SIZE + 24]
            .copy_from_slice(&0x1000u64.to_be_bytes());

        let ext = parse_self_ext_header(&bytes).unwrap();
        assert_eq!(SelfType::from(ext.self_type), SelfType::App);
        assert_eq!(ext.elf_offset, 0x1000);
    }

    #[test]
    fn self_type_conversion_covers_known_values() {
        assert_eq!(SelfType::from(1), SelfType::Lv0);
        assert_eq!(SelfType::from(2), SelfType::Lv1);
        assert_eq!(SelfType::from(3), SelfType::Lv2);
        assert_eq!(SelfType::from(4), SelfType::App);
        assert_eq!(SelfType::from(5), SelfType::Iso);
        assert_eq!(SelfType::from(6), SelfType::Ldr);
        assert_eq!(SelfType::from(8), SelfType::Npdrm);
        assert_eq!(SelfType::from(99), SelfType::Unknown);
    }

    // -- ELF parsing via goblin ------------------------------------

    /// Build a minimal but syntactically-valid ELF64-BE PPU file with
    /// one PT_LOAD segment. Enough for goblin to parse and for our
    /// downstream validation to make sense.
    fn build_minimal_ppu_elf(load_vaddr: u64, load_memsz: u64) -> Vec<u8> {
        const EHDR_SIZE: usize = 64;
        const PHDR_SIZE: usize = 56;
        let total = EHDR_SIZE + PHDR_SIZE;

        let mut bytes = vec![0u8; total];

        // e_ident
        bytes[0..4].copy_from_slice(&MAGIC_ELF);
        bytes[4] = 2; // EI_CLASS = ELFCLASS64
        bytes[5] = 2; // EI_DATA = ELFDATA2MSB (big endian)
        bytes[6] = 1; // EI_VERSION = EV_CURRENT
        bytes[7] = ELFOSABI_CELL_LV2;
        // e_ident[8..16] = 0

        // e_type = ET_EXEC (2), BE
        bytes[16..18].copy_from_slice(&2u16.to_be_bytes());
        // e_machine = EM_PPC64
        bytes[18..20].copy_from_slice(&EM_PPC64.to_be_bytes());
        // e_version = 1
        bytes[20..24].copy_from_slice(&1u32.to_be_bytes());
        // e_entry = load_vaddr
        bytes[24..32].copy_from_slice(&load_vaddr.to_be_bytes());
        // e_phoff = EHDR_SIZE
        bytes[32..40].copy_from_slice(&(EHDR_SIZE as u64).to_be_bytes());
        // e_shoff = 0
        // e_flags = 0
        // e_ehsize
        bytes[52..54].copy_from_slice(&(EHDR_SIZE as u16).to_be_bytes());
        // e_phentsize
        bytes[54..56].copy_from_slice(&(PHDR_SIZE as u16).to_be_bytes());
        // e_phnum = 1
        bytes[56..58].copy_from_slice(&1u16.to_be_bytes());
        // e_shentsize = 0, e_shnum = 0, e_shstrndx = 0

        // Program header #0: PT_LOAD
        let ph_base = EHDR_SIZE;
        bytes[ph_base..ph_base + 4].copy_from_slice(&1u32.to_be_bytes()); // p_type = PT_LOAD
        bytes[ph_base + 4..ph_base + 8].copy_from_slice(&5u32.to_be_bytes()); // p_flags = R+X
        // p_offset
        bytes[ph_base + 8..ph_base + 16].copy_from_slice(&0u64.to_be_bytes());
        // p_vaddr
        bytes[ph_base + 16..ph_base + 24].copy_from_slice(&load_vaddr.to_be_bytes());
        // p_paddr
        bytes[ph_base + 24..ph_base + 32].copy_from_slice(&load_vaddr.to_be_bytes());
        // p_filesz
        bytes[ph_base + 32..ph_base + 40].copy_from_slice(&load_memsz.to_be_bytes());
        // p_memsz
        bytes[ph_base + 40..ph_base + 48].copy_from_slice(&load_memsz.to_be_bytes());
        // p_align
        bytes[ph_base + 48..ph_base + 56].copy_from_slice(&0x1000u64.to_be_bytes());

        bytes
    }

    #[test]
    fn parse_minimal_ppu_elf() {
        let bytes = build_minimal_ppu_elf(0x1_0000, 0x2000);
        let info = parse_elf(&bytes).expect("parse elf");
        assert!(info.is_ppu64());
        assert!(!info.is_spu());
        assert_eq!(info.e_machine, EM_PPC64);
        assert_eq!(info.e_class, 2);
        assert_eq!(info.e_data, 2); // BE
        assert_eq!(info.e_os_abi, ELFOSABI_CELL_LV2);
        assert_eq!(info.e_type, 2);
        assert_eq!(info.e_entry, 0x1_0000);
        assert_eq!(info.program_headers.len(), 1);
        let ph = &info.program_headers[0];
        assert_eq!(ph.p_type, 1);
        assert_eq!(ph.p_vaddr, 0x1_0000);
        assert_eq!(ph.p_memsz, 0x2000);
    }

    #[test]
    fn validate_ppu_range_accepts_in_range() {
        let bytes = build_minimal_ppu_elf(0x10_0000, 0x2000);
        let info = parse_elf(&bytes).unwrap();
        validate_load_range(&info, PPU_MIN_ADDR, PPU_MAX_ADDR).unwrap();
    }

    #[test]
    fn validate_ppu_range_rejects_above_window() {
        // 0x4000_0000 is beyond PPU max
        let bytes = build_minimal_ppu_elf(0x4000_0000, 0x1000);
        let info = parse_elf(&bytes).unwrap();
        let err = validate_load_range(&info, PPU_MIN_ADDR, PPU_MAX_ADDR).unwrap_err();
        assert!(matches!(err, Error::AddressRangeViolation { .. }));
    }

    #[test]
    fn validate_ppu_range_rejects_segment_crossing_boundary() {
        // Starts in-range but extends past max
        let bytes = build_minimal_ppu_elf(0x2FFF_F000, 0x2000);
        let info = parse_elf(&bytes).unwrap();
        assert!(validate_load_range(&info, PPU_MIN_ADDR, PPU_MAX_ADDR).is_err());
    }

    #[test]
    fn parse_elf_rejects_garbage() {
        let garbage = vec![0xFFu8; 128];
        assert!(parse_elf(&garbage).is_err());
    }
}
