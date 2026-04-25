//! `rpcs3-hle-celltrophy` — NP trophy system HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/sceNpTrophy.cpp`. The real implementation
//! reads `TROPCONF.SFM` from VFS and hands trophy icons to an overlay; this
//! crate covers the **observable state machine** of the API surface so that
//! higher layers can validate argument handling and byte-exact error codes
//! without pulling in XML / VFS / SPU dependencies.
//!
//! ## Entry points covered
//!
//! | HLE function                          | Rust wrapper                  |
//! |---------------------------------------|-------------------------------|
//! | `sceNpTrophyInit`                     | [`Trophy::init`]              |
//! | `sceNpTrophyTerm`                     | [`Trophy::term`]              |
//! | `sceNpTrophyCreateHandle`             | [`Trophy::create_handle`]     |
//! | `sceNpTrophyDestroyHandle`            | [`Trophy::destroy_handle`]    |
//! | `sceNpTrophyAbortHandle`              | [`Trophy::abort_handle`]      |
//! | `sceNpTrophyCreateContext`            | [`Trophy::create_context`]    |
//! | `sceNpTrophyDestroyContext`           | [`Trophy::destroy_context`]   |
//! | `sceNpTrophyRegisterContext`          | [`Trophy::register_context`]  |
//! | `sceNpTrophyGetRequiredDiskSpace`     | [`Trophy::required_disk_space`] |
//! | `sceNpTrophySetSoundLevel`            | [`Trophy::set_sound_level`]   |
//! | `sceNpTrophyGetGameInfo`              | [`Trophy::get_game_info`]     |
//! | `sceNpTrophyGetGameProgress`          | [`Trophy::game_progress`]     |
//! | `sceNpTrophyUnlockTrophy`             | [`Trophy::unlock_trophy`]     |
//! | `sceNpTrophyGetTrophyUnlockState`     | [`Trophy::unlock_state`]      |
//! | `sceNpTrophyGetTrophyInfo`            | [`Trophy::get_trophy_info`]   |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with sceNpTrophy.h:10-56
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ALREADY_INITIALIZED:    CellError = CellError(0x8002_2901);
    pub const NOT_INITIALIZED:        CellError = CellError(0x8002_2902);
    pub const NOT_SUPPORTED:          CellError = CellError(0x8002_2903);
    pub const CONTEXT_NOT_REGISTERED: CellError = CellError(0x8002_2904);
    pub const OUT_OF_MEMORY:          CellError = CellError(0x8002_2905);
    pub const INVALID_ARGUMENT:       CellError = CellError(0x8002_2906);
    pub const EXCEEDS_MAX:            CellError = CellError(0x8002_2907);
    pub const INSUFFICIENT:           CellError = CellError(0x8002_2909);
    pub const UNKNOWN_CONTEXT:        CellError = CellError(0x8002_290A);
    pub const INVALID_FORMAT:         CellError = CellError(0x8002_290B);
    pub const INVALID_GRADE:          CellError = CellError(0x8002_290D);
    pub const INVALID_CONTEXT:        CellError = CellError(0x8002_290E);
    pub const ABORT:                  CellError = CellError(0x8002_2910);
    pub const UNKNOWN_HANDLE:         CellError = CellError(0x8002_2911);
    pub const LOCKED:                 CellError = CellError(0x8002_2912);
    pub const HIDDEN:                 CellError = CellError(0x8002_2913);
    pub const CANNOT_UNLOCK_PLATINUM: CellError = CellError(0x8002_2914);
    pub const ALREADY_UNLOCKED:       CellError = CellError(0x8002_2915);
    pub const INVALID_TYPE:           CellError = CellError(0x8002_2916);
    pub const INVALID_HANDLE:         CellError = CellError(0x8002_2917);
    pub const INVALID_NP_COMM_ID:     CellError = CellError(0x8002_2918);
    pub const UNKNOWN_NP_COMM_ID:     CellError = CellError(0x8002_2919);
    pub const INVALID_TROPHY_ID:      CellError = CellError(0x8002_2920);
    pub const UNKNOWN_TROPHY_ID:      CellError = CellError(0x8002_2921);
    pub const UNKNOWN:                CellError = CellError(0x8002_29FF);
}

// =====================================================================
// Constants — byte-exact with sceNpTrophy.h:58-156
// =====================================================================

pub const TITLE_MAX_SIZE:      usize = 128;
pub const GAME_DESCR_MAX_SIZE: usize = 1024;
pub const NAME_MAX_SIZE:       usize = 128;
pub const DESCR_MAX_SIZE:      usize = 1024;

pub const FLAG_SETSIZE:    usize = 128;
pub const FLAG_BITS_SHIFT: u32   = 5;
/// Number of `u32` words in a [`TrophyFlagArray`] — 128 bits / 32.
pub const FLAG_WORDS: usize = FLAG_SETSIZE >> FLAG_BITS_SHIFT; // = 4

pub const INVALID_CONTEXT: u32 = 0;
pub const INVALID_HANDLE:  u32 = 0;
pub const INVALID_TROPHY_ID: u32 = 0xFFFF_FFFF;

/// Commercial-signature magic word at `commSign.data[0..4]`.
pub const NP_TROPHY_COMM_SIGN_MAGIC: u32 = 0xB9DD_E13B;

/// BE-u16 at `commSign.data[4..6]` — only version `1.00` is accepted by
/// the firmware.
pub const NP_TROPHY_COMM_SIGN_VERSION: u16 = 0x0100;

/// Inclusive lower bound for handle / context ids (from C++ `id_base = 1`).
pub const ID_BASE: u32 = 1;
/// Number of concurrent handles / contexts (from C++ `id_count = 4`).
pub const ID_COUNT: u32 = 4;

/// Upper exclusive bound used by the range check.
pub const ID_LIMIT: u32 = ID_BASE + ID_COUNT;

// ---- SetSoundLevel bounds ------------------------------------------

/// Minimum accepted by `sceNpTrophySetSoundLevel`.
pub const SOUND_LEVEL_MIN: u32 = 20;
/// Maximum accepted by `sceNpTrophySetSoundLevel`.
pub const SOUND_LEVEL_MAX: u32 = 100;

// ---- CreateContext options -----------------------------------------

pub const OPTIONS_CREATE_CONTEXT_READ_ONLY:       u64 = 1;
pub const OPTIONS_REGISTER_CONTEXT_SHOW_ERROR_EXIT: u64 = 1;

// ---- Grades --------------------------------------------------------

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrophyGrade {
    Unknown  = 0,
    Platinum = 1,
    Gold     = 2,
    Silver   = 3,
    Bronze   = 4,
}

impl TrophyGrade {
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Unknown),
            1 => Some(Self::Platinum),
            2 => Some(Self::Gold),
            3 => Some(Self::Silver),
            4 => Some(Self::Bronze),
            _ => None,
        }
    }
}

// ---- Status (callback arg) -----------------------------------------

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrophyStatus {
    Unknown             = 0,
    NotInstalled        = 1,
    DataCorrupt         = 2,
    Installed           = 3,
    RequiresUpdate      = 4,
    ProcessingSetup     = 5,
    ProcessingProgress  = 6,
    ProcessingFinalize  = 7,
    ProcessingComplete  = 8,
    ChangesDetected     = 9,
}

// =====================================================================
// NP Communication ID & Signature
// =====================================================================

/// `SceNpCommunicationId` — 9-byte title identifier + 2-digit number + a
/// `\0` terminator (see `np/np_handler.cpp`).  We keep the exact layout
/// but expose the fields the trophy validator looks at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommunicationId {
    /// 9 bytes, NUL-padded title prefix (e.g. `"NPWR01234"` → `b"NPWR01234"`).
    pub data: [u8; 9],
    /// Padding byte — `term` in the C++ struct, unused by RPCS3.
    pub term: u8,
    /// Trophy set number (`_01`, `_02`, … up to `_99`).
    pub num: i32,
}

impl CommunicationId {
    #[must_use]
    pub const fn new(data: [u8; 9], num: i32) -> Self {
        Self { data, term: 0, num }
    }
}

/// `SceNpCommunicationSignature` — a 160-byte blob whose leading layout is
/// `[magic u32 BE | version u16 BE | 6 zero bytes | signature bytes]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommunicationSignature {
    pub data: [u8; 160],
}

impl CommunicationSignature {
    /// Build a signature that passes the firmware checks — magic
    /// `0xB9DDE13B`, version `0x0100`, 6 zero-padding bytes, body zero-
    /// padded up to 160 bytes.
    #[must_use]
    pub fn valid() -> Self {
        let mut data = [0u8; 160];
        data[..4].copy_from_slice(&NP_TROPHY_COMM_SIGN_MAGIC.to_be_bytes());
        data[4..6].copy_from_slice(&NP_TROPHY_COMM_SIGN_VERSION.to_be_bytes());
        // bytes [6..12] intentionally left zero — padding.
        Self { data }
    }
}

// =====================================================================
// Trophy manifest (subset of TROPCONF.SFM)
// =====================================================================

/// Minimal trophy descriptor — mirrors the fields the firmware exposes via
/// `SceNpTrophyDetails` + unlock state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrophyEntry {
    pub id: u32,
    pub grade: TrophyGrade,
    pub hidden: bool,
    pub unlocked: bool,
    pub timestamp: u64,
}

impl TrophyEntry {
    #[must_use]
    pub const fn new(id: u32, grade: TrophyGrade, hidden: bool) -> Self {
        Self { id, grade, hidden, unlocked: false, timestamp: 0 }
    }
}

// =====================================================================
// Handles & contexts
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Handle {
    id: u32,
    is_aborted: bool,
}

#[derive(Debug, Clone)]
struct Context {
    id: u32,
    name: [u8; 12], // "NPWR01234_xx" — 9 + 3
    read_only: bool,
    registered: bool,
    trophies: Vec<TrophyEntry>,
}

// =====================================================================
// Flag array (bitmap of unlocked trophies)
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrophyFlagArray {
    pub words: [u32; FLAG_WORDS],
}

impl Default for TrophyFlagArray {
    fn default() -> Self { Self { words: [0; FLAG_WORDS] } }
}

impl TrophyFlagArray {
    #[must_use]
    pub const fn new() -> Self { Self { words: [0; FLAG_WORDS] } }

    /// Set the bit for trophy `id`.  Returns `false` if `id >= FLAG_SETSIZE`.
    pub fn set(&mut self, id: u32) -> bool {
        if id as usize >= FLAG_SETSIZE { return false; }
        let word = (id >> FLAG_BITS_SHIFT) as usize;
        let bit  = id & 0x1F;
        self.words[word] |= 1 << bit;
        true
    }

    #[must_use]
    pub fn get(&self, id: u32) -> bool {
        if id as usize >= FLAG_SETSIZE { return false; }
        let word = (id >> FLAG_BITS_SHIFT) as usize;
        let bit  = id & 0x1F;
        (self.words[word] & (1 << bit)) != 0
    }

    #[must_use]
    pub fn popcount(&self) -> u32 {
        self.words.iter().map(|w| w.count_ones()).sum()
    }
}

// =====================================================================
// Trophy manager (mirror of `sce_np_trophy_manager`)
// =====================================================================

#[derive(Debug, Default, Clone)]
pub struct Trophy {
    is_initialized: bool,
    handles: Vec<Handle>,
    contexts: Vec<Context>,
}

/// Aggregate game data returned by [`Trophy::get_game_info`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GameInfo {
    pub num_trophies: u32,
    pub num_platinum: u32,
    pub num_gold:     u32,
    pub num_silver:   u32,
    pub num_bronze:   u32,
    pub unlocked_trophies: u32,
    pub unlocked_platinum: u32,
    pub unlocked_gold:     u32,
    pub unlocked_silver:   u32,
    pub unlocked_bronze:   u32,
}

impl Trophy {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    #[must_use]
    pub fn is_initialized(&self) -> bool { self.is_initialized }

    // ---- Init / Term ------------------------------------------------

    /// Port of `sceNpTrophyInit`.
    ///
    /// # Errors
    /// * `ALREADY_INITIALIZED` if already up.
    /// * `NOT_SUPPORTED`       if `options != 0`.
    pub fn init(&mut self, options: u64) -> Result<(), CellError> {
        if self.is_initialized {
            return Err(errors::ALREADY_INITIALIZED);
        }
        if options > 0 {
            return Err(errors::NOT_SUPPORTED);
        }
        self.is_initialized = true;
        Ok(())
    }

    /// Port of `sceNpTrophyTerm`.
    ///
    /// # Errors
    /// * `NOT_INITIALIZED` if the manager was never initialised.
    pub fn term(&mut self) -> Result<(), CellError> {
        if !self.is_initialized {
            return Err(errors::NOT_INITIALIZED);
        }
        self.handles.clear();
        self.contexts.clear();
        self.is_initialized = false;
        Ok(())
    }

    // ---- Handle lifecycle ------------------------------------------

    /// Port of `sceNpTrophyCreateHandle`.  `out_handle_ptr_valid = false`
    /// models the `!handle` null-pointer check in C++.
    ///
    /// # Errors
    /// * `NOT_INITIALIZED`   if `init` wasn't called.
    /// * `INVALID_ARGUMENT`  if the destination pointer is null.
    /// * `EXCEEDS_MAX`       if the handle table is full.
    pub fn create_handle(&mut self, out_handle_ptr_valid: bool) -> Result<u32, CellError> {
        if !self.is_initialized {
            return Err(errors::NOT_INITIALIZED);
        }
        if !out_handle_ptr_valid {
            return Err(errors::INVALID_ARGUMENT);
        }
        if self.handles.len() as u32 >= ID_COUNT {
            return Err(errors::EXCEEDS_MAX);
        }
        let id = ID_BASE + self.handles.len() as u32;
        self.handles.push(Handle { id, is_aborted: false });
        Ok(id)
    }

    /// Port of `sceNpTrophyDestroyHandle`.
    ///
    /// # Errors
    /// * `INVALID_ARGUMENT` if the handle is outside `[1, 5)`.
    /// * `UNKNOWN_HANDLE`   if it was never created / already destroyed.
    pub fn destroy_handle(&mut self, handle: u32) -> Result<(), CellError> {
        if !(ID_BASE..ID_LIMIT).contains(&handle) {
            return Err(errors::INVALID_ARGUMENT);
        }
        let pos = self.handles.iter().position(|h| h.id == handle)
            .ok_or(errors::UNKNOWN_HANDLE)?;
        self.handles.swap_remove(pos);
        Ok(())
    }

    /// Port of `sceNpTrophyAbortHandle`.
    ///
    /// # Errors
    /// * `INVALID_ARGUMENT` if the handle is outside `[1, 5)`.
    /// * `UNKNOWN_HANDLE`   if it was never created.
    pub fn abort_handle(&mut self, handle: u32) -> Result<(), CellError> {
        if !(ID_BASE..ID_LIMIT).contains(&handle) {
            return Err(errors::INVALID_ARGUMENT);
        }
        let h = self.handles.iter_mut().find(|h| h.id == handle)
            .ok_or(errors::UNKNOWN_HANDLE)?;
        h.is_aborted = true;
        Ok(())
    }

    #[must_use]
    pub fn handle_aborted(&self, handle: u32) -> Option<bool> {
        self.handles.iter().find(|h| h.id == handle).map(|h| h.is_aborted)
    }

    // ---- Context lifecycle -----------------------------------------

    /// Port of `sceNpTrophyCreateContext`.  `context_ptr_valid`,
    /// `comm_id_ptr_valid`, `comm_sign_ptr_valid` model the three null
    /// checks the firmware performs (in the order given in C++).
    ///
    /// # Errors
    /// * `INVALID_ARGUMENT`    if `commSign` is null.
    /// * `NOT_INITIALIZED`     if `init` wasn't called.
    /// * `INVALID_ARGUMENT`    if `context` or `commId` is null.
    /// * `NOT_SUPPORTED`       if `options > READ_ONLY` (i.e., `>1`).
    /// * `INVALID_NP_COMM_ID`  for `num > 99`, wrong magic, non-zero
    ///                         padding, or version != `0x0100`.
    /// * `EXCEEDS_MAX`         if the context table is full.
    pub fn create_context(
        &mut self,
        context_ptr_valid: bool,
        comm_id_ptr_valid: bool,
        comm_sign_ptr_valid: bool,
        comm_id: &CommunicationId,
        comm_sign: &CommunicationSignature,
        options: u64,
    ) -> Result<u32, CellError> {
        // Ordering of checks mirrors sceNpTrophy.cpp:430-462.
        if !comm_sign_ptr_valid {
            return Err(errors::INVALID_ARGUMENT);
        }
        if !self.is_initialized {
            return Err(errors::NOT_INITIALIZED);
        }
        if !context_ptr_valid || !comm_id_ptr_valid {
            return Err(errors::INVALID_ARGUMENT);
        }
        if options > OPTIONS_CREATE_CONTEXT_READ_ONLY {
            return Err(errors::NOT_SUPPORTED);
        }
        if comm_id.num > 99 {
            return Err(errors::INVALID_NP_COMM_ID);
        }
        let magic = u32::from_be_bytes([
            comm_sign.data[0], comm_sign.data[1],
            comm_sign.data[2], comm_sign.data[3],
        ]);
        if magic != NP_TROPHY_COMM_SIGN_MAGIC {
            return Err(errors::INVALID_NP_COMM_ID);
        }
        // Padding bytes [6..12] must all be zero.
        if comm_sign.data[6..12].iter().any(|&b| b != 0) {
            return Err(errors::INVALID_NP_COMM_ID);
        }
        let version = u16::from_be_bytes([comm_sign.data[4], comm_sign.data[5]]);
        if version != NP_TROPHY_COMM_SIGN_VERSION {
            return Err(errors::INVALID_NP_COMM_ID);
        }
        if self.contexts.len() as u32 >= ID_COUNT {
            return Err(errors::EXCEEDS_MAX);
        }

        let id = ID_BASE + self.contexts.len() as u32;
        // Name layout: first 9 bytes from commId.data (up to first '\0'),
        // followed by `_<num:02d>`.
        let mut name = [0u8; 12];
        let title_len = comm_id.data.iter().position(|&b| b == 0).unwrap_or(9).min(9);
        name[..title_len].copy_from_slice(&comm_id.data[..title_len]);
        // Fill `_xx` at the tail.  The real code uses snprintf; we match
        // the observable shape (two decimal digits, zero-padded).
        name[9] = b'_';
        let tens = (comm_id.num / 10) as u8;
        let ones = (comm_id.num % 10) as u8;
        name[10] = b'0' + tens;
        name[11] = b'0' + ones;

        self.contexts.push(Context {
            id,
            name,
            read_only: (options & OPTIONS_CREATE_CONTEXT_READ_ONLY) != 0,
            registered: false,
            trophies: Vec::new(),
        });
        Ok(id)
    }

    /// Port of `sceNpTrophyDestroyContext`.
    ///
    /// # Errors
    /// * `NOT_INITIALIZED`  if `init` wasn't called.
    /// * `INVALID_ARGUMENT` if `context` is outside `[1, 5)`.
    /// * `UNKNOWN_CONTEXT`  if it doesn't exist.
    pub fn destroy_context(&mut self, context: u32) -> Result<(), CellError> {
        if !self.is_initialized {
            return Err(errors::NOT_INITIALIZED);
        }
        if !(ID_BASE..ID_LIMIT).contains(&context) {
            return Err(errors::INVALID_ARGUMENT);
        }
        let pos = self.contexts.iter().position(|c| c.id == context)
            .ok_or(errors::UNKNOWN_CONTEXT)?;
        self.contexts.swap_remove(pos);
        Ok(())
    }

    /// Port of `sceNpTrophyRegisterContext`.  `trophies` is the
    /// materialised trophy list the firmware would normally parse from
    /// `TROPCONF.SFM`.
    ///
    /// # Errors
    /// * `NOT_INITIALIZED` / `INVALID_ARGUMENT` / `UNKNOWN_HANDLE` /
    ///   `UNKNOWN_CONTEXT` as in the C++ `get_context_ex` helper.
    pub fn register_context(
        &mut self,
        context: u32,
        handle: u32,
        trophies: &[TrophyEntry],
    ) -> Result<(), CellError> {
        self.check_context_handle(context, handle)?;
        let ctx = self.context_mut(context)?;
        if ctx.registered {
            return Err(errors::ALREADY_INITIALIZED);
        }
        ctx.trophies = trophies.to_vec();
        ctx.registered = true;
        Ok(())
    }

    /// Port of `sceNpTrophyGetRequiredDiskSpace`.  The real helper probes
    /// the VFS for a pre-existing trophy directory; the Rust port just
    /// validates the arguments and returns 0 for installed contexts, or
    /// `INSUFFICIENT` if the context isn't registered.
    ///
    /// # Errors
    /// Validation errors from [`check_context_handle`]; does **not** fail
    /// on unregistered contexts — the C++ side returns 0 in that case too.
    pub fn required_disk_space(
        &self,
        context: u32,
        handle: u32,
        out_reqspace_ptr_valid: bool,
        options: u64,
    ) -> Result<u64, CellError> {
        self.check_context_handle(context, handle)?;
        if !out_reqspace_ptr_valid {
            return Err(errors::INVALID_ARGUMENT);
        }
        if options > 0 {
            return Err(errors::NOT_SUPPORTED);
        }
        Ok(0)
    }

    /// Port of `sceNpTrophySetSoundLevel`.
    ///
    /// # Errors
    /// * `INVALID_ARGUMENT` for `level < 20` or `level > 100`.
    /// * `NOT_SUPPORTED`    if `options != 0`.
    /// * Validation errors from [`check_context_handle`].
    pub fn set_sound_level(
        &self,
        context: u32,
        handle: u32,
        level: u32,
        options: u64,
    ) -> Result<(), CellError> {
        // C++ checks level + options BEFORE touching the manager.
        if level > SOUND_LEVEL_MAX || level < SOUND_LEVEL_MIN {
            return Err(errors::INVALID_ARGUMENT);
        }
        if options > 0 {
            return Err(errors::NOT_SUPPORTED);
        }
        self.check_context_handle(context, handle)
    }

    /// Port of `sceNpTrophyGetGameInfo`.  Accepts `details_valid` / `data_valid`
    /// to mirror the twin-output-pointer API.
    ///
    /// # Errors
    /// * Validation errors from [`check_context_handle`].
    /// * `CONTEXT_NOT_REGISTERED` if the caller didn't register first.
    /// * `INVALID_ARGUMENT`       if both output pointers are null.
    pub fn get_game_info(
        &self,
        context: u32,
        handle: u32,
        details_valid: bool,
        data_valid: bool,
    ) -> Result<GameInfo, CellError> {
        self.check_context_handle(context, handle)?;
        let ctx = self.context(context)?;
        if !ctx.registered {
            return Err(errors::CONTEXT_NOT_REGISTERED);
        }
        if !details_valid && !data_valid {
            return Err(errors::INVALID_ARGUMENT);
        }
        let mut info = GameInfo::default();
        for t in &ctx.trophies {
            info.num_trophies += 1;
            match t.grade {
                TrophyGrade::Bronze   => info.num_bronze   += 1,
                TrophyGrade::Silver   => info.num_silver   += 1,
                TrophyGrade::Gold     => info.num_gold     += 1,
                TrophyGrade::Platinum => info.num_platinum += 1,
                TrophyGrade::Unknown  => {}
            }
            if t.unlocked {
                info.unlocked_trophies += 1;
                match t.grade {
                    TrophyGrade::Bronze   => info.unlocked_bronze   += 1,
                    TrophyGrade::Silver   => info.unlocked_silver   += 1,
                    TrophyGrade::Gold     => info.unlocked_gold     += 1,
                    TrophyGrade::Platinum => info.unlocked_platinum += 1,
                    TrophyGrade::Unknown  => {}
                }
            }
        }
        Ok(info)
    }

    /// Port of `sceNpTrophyGetGameProgress` — returns integer percentage of
    /// non-platinum unlocks (C++ ignores platinum per comment at line 1380).
    ///
    /// # Errors
    /// Validation errors from [`check_context_handle`] + `CONTEXT_NOT_REGISTERED`.
    pub fn game_progress(
        &self,
        context: u32,
        handle: u32,
        out_pct_valid: bool,
    ) -> Result<i32, CellError> {
        self.check_context_handle(context, handle)?;
        let ctx = self.context(context)?;
        if !ctx.registered {
            return Err(errors::CONTEXT_NOT_REGISTERED);
        }
        if !out_pct_valid {
            return Err(errors::INVALID_ARGUMENT);
        }
        let non_plat: Vec<&TrophyEntry> = ctx.trophies.iter()
            .filter(|t| t.grade != TrophyGrade::Platinum).collect();
        if non_plat.is_empty() { return Ok(0); }
        let unlocked = non_plat.iter().filter(|t| t.unlocked).count() as i32;
        Ok(unlocked * 100 / non_plat.len() as i32)
    }

    /// Port of `sceNpTrophyUnlockTrophy`.
    ///
    /// # Errors
    /// Usual handle/context validation, plus:
    /// * `CONTEXT_NOT_REGISTERED` if the context was never registered.
    /// * `INVALID_TROPHY_ID`      if `trophy_id` is out of range.
    /// * `CANNOT_UNLOCK_PLATINUM` if it's the platinum grade.
    /// * `ALREADY_UNLOCKED`       if it's already set.
    pub fn unlock_trophy(
        &mut self,
        context: u32,
        handle: u32,
        trophy_id: i32,
        timestamp: u64,
    ) -> Result<(), CellError> {
        self.check_context_handle(context, handle)?;
        let ctx = self.context_mut(context)?;
        if !ctx.registered {
            return Err(errors::CONTEXT_NOT_REGISTERED);
        }
        if trophy_id < 0 || (trophy_id as usize) >= ctx.trophies.len() {
            return Err(errors::INVALID_TROPHY_ID);
        }
        let t = &mut ctx.trophies[trophy_id as usize];
        if t.grade == TrophyGrade::Platinum {
            return Err(errors::CANNOT_UNLOCK_PLATINUM);
        }
        if t.unlocked {
            return Err(errors::ALREADY_UNLOCKED);
        }
        t.unlocked = true;
        t.timestamp = timestamp;
        Ok(())
    }

    /// Port of `sceNpTrophyGetTrophyUnlockState`.
    ///
    /// # Errors
    /// Validation errors from [`check_context_handle`].  Matching the C++
    /// side, `!flags || !count` → `INVALID_ARGUMENT`.
    pub fn unlock_state(
        &self,
        context: u32,
        handle: u32,
        flags_valid: bool,
        count_valid: bool,
    ) -> Result<(TrophyFlagArray, u32), CellError> {
        if !flags_valid || !count_valid {
            return Err(errors::INVALID_ARGUMENT);
        }
        self.check_context_handle(context, handle)?;
        let ctx = self.context(context)?;
        if !ctx.registered {
            return Err(errors::CONTEXT_NOT_REGISTERED);
        }
        let mut arr = TrophyFlagArray::new();
        let mut count = 0u32;
        for t in &ctx.trophies {
            if t.unlocked {
                arr.set(t.id);
            }
            count += 1;
        }
        Ok((arr, count))
    }

    /// Port of `sceNpTrophyGetTrophyInfo`.
    ///
    /// # Errors
    /// Validation + `INVALID_TROPHY_ID` if `trophy_id` is not in range.
    pub fn get_trophy_info(
        &self,
        context: u32,
        handle: u32,
        trophy_id: i32,
    ) -> Result<TrophyEntry, CellError> {
        self.check_context_handle(context, handle)?;
        let ctx = self.context(context)?;
        if !ctx.registered {
            return Err(errors::CONTEXT_NOT_REGISTERED);
        }
        if trophy_id < 0 || (trophy_id as usize) >= ctx.trophies.len() {
            return Err(errors::INVALID_TROPHY_ID);
        }
        Ok(ctx.trophies[trophy_id as usize])
    }

    // ---- helpers --------------------------------------------------

    fn check_context_handle(&self, context: u32, handle: u32) -> Result<(), CellError> {
        if !self.is_initialized {
            return Err(errors::NOT_INITIALIZED);
        }
        if !(ID_BASE..ID_LIMIT).contains(&context) {
            return Err(errors::INVALID_ARGUMENT);
        }
        if !(ID_BASE..ID_LIMIT).contains(&handle) {
            return Err(errors::INVALID_ARGUMENT);
        }
        if !self.contexts.iter().any(|c| c.id == context) {
            return Err(errors::UNKNOWN_CONTEXT);
        }
        if !self.handles.iter().any(|h| h.id == handle) {
            return Err(errors::UNKNOWN_HANDLE);
        }
        Ok(())
    }

    fn context(&self, id: u32) -> Result<&Context, CellError> {
        self.contexts.iter().find(|c| c.id == id).ok_or(errors::UNKNOWN_CONTEXT)
    }

    fn context_mut(&mut self, id: u32) -> Result<&mut Context, CellError> {
        self.contexts.iter_mut().find(|c| c.id == id).ok_or(errors::UNKNOWN_CONTEXT)
    }

    #[must_use]
    pub fn context_name(&self, id: u32) -> Option<[u8; 12]> {
        self.contexts.iter().find(|c| c.id == id).map(|c| c.name)
    }

    #[must_use]
    pub fn context_read_only(&self, id: u32) -> Option<bool> {
        self.contexts.iter().find(|c| c.id == id).map(|c| c.read_only)
    }

    #[must_use]
    pub fn context_registered(&self, id: u32) -> Option<bool> {
        self.contexts.iter().find(|c| c.id == id).map(|c| c.registered)
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn good_comm_id() -> CommunicationId {
        CommunicationId::new(*b"NPWR01234", 1)
    }
    fn good_sign() -> CommunicationSignature { CommunicationSignature::valid() }

    fn seeded_trophies() -> Vec<TrophyEntry> {
        vec![
            TrophyEntry::new(0, TrophyGrade::Bronze,   false),
            TrophyEntry::new(1, TrophyGrade::Silver,   false),
            TrophyEntry::new(2, TrophyGrade::Gold,     true),
            TrophyEntry::new(3, TrophyGrade::Platinum, false),
        ]
    }

    // ---- constants & error codes -------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::ALREADY_INITIALIZED.0,    0x8002_2901);
        assert_eq!(errors::NOT_INITIALIZED.0,        0x8002_2902);
        assert_eq!(errors::NOT_SUPPORTED.0,          0x8002_2903);
        assert_eq!(errors::CONTEXT_NOT_REGISTERED.0, 0x8002_2904);
        assert_eq!(errors::OUT_OF_MEMORY.0,          0x8002_2905);
        assert_eq!(errors::INVALID_ARGUMENT.0,       0x8002_2906);
        assert_eq!(errors::EXCEEDS_MAX.0,            0x8002_2907);
        assert_eq!(errors::UNKNOWN_CONTEXT.0,        0x8002_290A);
        assert_eq!(errors::UNKNOWN_HANDLE.0,         0x8002_2911);
        assert_eq!(errors::CANNOT_UNLOCK_PLATINUM.0, 0x8002_2914);
        assert_eq!(errors::ALREADY_UNLOCKED.0,       0x8002_2915);
        assert_eq!(errors::INVALID_NP_COMM_ID.0,     0x8002_2918);
        assert_eq!(errors::INVALID_TROPHY_ID.0,      0x8002_2920);
        assert_eq!(errors::UNKNOWN.0,                0x8002_29FF);
    }

    #[test]
    fn constants_byte_exact() {
        assert_eq!(TITLE_MAX_SIZE, 128);
        assert_eq!(GAME_DESCR_MAX_SIZE, 1024);
        assert_eq!(FLAG_SETSIZE, 128);
        assert_eq!(FLAG_BITS_SHIFT, 5);
        assert_eq!(FLAG_WORDS, 4);
        assert_eq!(NP_TROPHY_COMM_SIGN_MAGIC, 0xB9DD_E13B);
        assert_eq!(NP_TROPHY_COMM_SIGN_VERSION, 0x0100);
        assert_eq!(ID_BASE, 1);
        assert_eq!(ID_COUNT, 4);
        assert_eq!(ID_LIMIT, 5);
        assert_eq!(SOUND_LEVEL_MIN, 20);
        assert_eq!(SOUND_LEVEL_MAX, 100);
        assert_eq!(INVALID_TROPHY_ID, 0xFFFF_FFFF);
    }

    #[test]
    fn grade_ordinals_match_cpp() {
        assert_eq!(TrophyGrade::Unknown  as u32, 0);
        assert_eq!(TrophyGrade::Platinum as u32, 1);
        assert_eq!(TrophyGrade::Gold     as u32, 2);
        assert_eq!(TrophyGrade::Silver   as u32, 3);
        assert_eq!(TrophyGrade::Bronze   as u32, 4);
    }

    #[test]
    fn status_ordinals_match_cpp() {
        assert_eq!(TrophyStatus::Installed          as u32, 3);
        assert_eq!(TrophyStatus::ProcessingComplete as u32, 8);
        assert_eq!(TrophyStatus::ChangesDetected    as u32, 9);
    }

    // ---- init / term -------------------------------------------------

    #[test]
    fn init_happy_path() {
        let mut t = Trophy::new();
        assert!(!t.is_initialized());
        t.init(0).unwrap();
        assert!(t.is_initialized());
    }

    #[test]
    fn init_twice_returns_already_initialized() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(t.init(0).unwrap_err(), errors::ALREADY_INITIALIZED);
    }

    #[test]
    fn init_nonzero_options_is_not_supported() {
        let mut t = Trophy::new();
        assert_eq!(t.init(1).unwrap_err(), errors::NOT_SUPPORTED);
    }

    #[test]
    fn term_without_init_fails() {
        let mut t = Trophy::new();
        assert_eq!(t.term().unwrap_err(), errors::NOT_INITIALIZED);
    }

    #[test]
    fn term_clears_handles_and_contexts() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        t.create_handle(true).unwrap();
        t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.term().unwrap();
        assert!(!t.is_initialized());
        // Re-init + create — ids restart at 1.
        t.init(0).unwrap();
        assert_eq!(t.create_handle(true).unwrap(), 1);
    }

    // ---- handles -----------------------------------------------------

    #[test]
    fn create_handle_requires_init() {
        let mut t = Trophy::new();
        assert_eq!(t.create_handle(true).unwrap_err(), errors::NOT_INITIALIZED);
    }

    #[test]
    fn create_handle_rejects_null_pointer() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(t.create_handle(false).unwrap_err(), errors::INVALID_ARGUMENT);
    }

    #[test]
    fn create_handle_allocates_sequentially() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let a = t.create_handle(true).unwrap();
        let b = t.create_handle(true).unwrap();
        let c = t.create_handle(true).unwrap();
        let d = t.create_handle(true).unwrap();
        assert_eq!((a, b, c, d), (1, 2, 3, 4));
    }

    #[test]
    fn create_handle_exceeds_max_at_fifth() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        for _ in 0..4 { t.create_handle(true).unwrap(); }
        assert_eq!(t.create_handle(true).unwrap_err(), errors::EXCEEDS_MAX);
    }

    #[test]
    fn destroy_handle_out_of_range_is_invalid_argument() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(t.destroy_handle(0).unwrap_err(), errors::INVALID_ARGUMENT);
        assert_eq!(t.destroy_handle(5).unwrap_err(), errors::INVALID_ARGUMENT);
        assert_eq!(t.destroy_handle(1000).unwrap_err(), errors::INVALID_ARGUMENT);
    }

    #[test]
    fn destroy_handle_unknown_is_unknown_handle() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(t.destroy_handle(3).unwrap_err(), errors::UNKNOWN_HANDLE);
    }

    #[test]
    fn destroy_handle_removes() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let id = t.create_handle(true).unwrap();
        t.destroy_handle(id).unwrap();
        assert_eq!(t.destroy_handle(id).unwrap_err(), errors::UNKNOWN_HANDLE);
    }

    #[test]
    fn abort_handle_sets_flag() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let id = t.create_handle(true).unwrap();
        assert_eq!(t.handle_aborted(id), Some(false));
        t.abort_handle(id).unwrap();
        assert_eq!(t.handle_aborted(id), Some(true));
    }

    #[test]
    fn abort_handle_unknown() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(t.abort_handle(2).unwrap_err(), errors::UNKNOWN_HANDLE);
    }

    #[test]
    fn abort_handle_out_of_range() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(t.abort_handle(0).unwrap_err(), errors::INVALID_ARGUMENT);
        assert_eq!(t.abort_handle(5).unwrap_err(), errors::INVALID_ARGUMENT);
    }

    // ---- contexts ----------------------------------------------------

    #[test]
    fn create_context_null_sign_beats_all() {
        let mut t = Trophy::new();
        // even without init, null sign is checked first
        assert_eq!(
            t.create_context(true, true, false, &good_comm_id(), &good_sign(), 0).unwrap_err(),
            errors::INVALID_ARGUMENT,
        );
    }

    #[test]
    fn create_context_requires_init() {
        let mut t = Trophy::new();
        assert_eq!(
            t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap_err(),
            errors::NOT_INITIALIZED,
        );
    }

    #[test]
    fn create_context_null_ctx_or_comm_id() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(
            t.create_context(false, true, true, &good_comm_id(), &good_sign(), 0).unwrap_err(),
            errors::INVALID_ARGUMENT,
        );
        assert_eq!(
            t.create_context(true, false, true, &good_comm_id(), &good_sign(), 0).unwrap_err(),
            errors::INVALID_ARGUMENT,
        );
    }

    #[test]
    fn create_context_bad_options_is_not_supported() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(
            t.create_context(true, true, true, &good_comm_id(), &good_sign(), 2).unwrap_err(),
            errors::NOT_SUPPORTED,
        );
    }

    #[test]
    fn create_context_bad_num() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let cid = CommunicationId::new(*b"NPWR01234", 100);
        assert_eq!(
            t.create_context(true, true, true, &cid, &good_sign(), 0).unwrap_err(),
            errors::INVALID_NP_COMM_ID,
        );
    }

    #[test]
    fn create_context_bad_magic() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let mut sign = good_sign();
        sign.data[0] = 0xDE;
        assert_eq!(
            t.create_context(true, true, true, &good_comm_id(), &sign, 0).unwrap_err(),
            errors::INVALID_NP_COMM_ID,
        );
    }

    #[test]
    fn create_context_bad_padding() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let mut sign = good_sign();
        sign.data[7] = 0xFF; // pad byte must be 0
        assert_eq!(
            t.create_context(true, true, true, &good_comm_id(), &sign, 0).unwrap_err(),
            errors::INVALID_NP_COMM_ID,
        );
    }

    #[test]
    fn create_context_bad_version() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let mut sign = good_sign();
        sign.data[4] = 0x02;
        sign.data[5] = 0x00; // version becomes 0x0200 — only 0x0100 accepted
        assert_eq!(
            t.create_context(true, true, true, &good_comm_id(), &sign, 0).unwrap_err(),
            errors::INVALID_NP_COMM_ID,
        );
    }

    #[test]
    fn create_context_happy_path_and_name_format() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let id = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        assert_eq!(id, 1);
        let name = t.context_name(id).unwrap();
        assert_eq!(&name[..9], b"NPWR01234");
        assert_eq!(name[9], b'_');
        assert_eq!(&name[10..12], b"01");
    }

    #[test]
    fn create_context_read_only_option_propagates() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let id = t.create_context(true, true, true, &good_comm_id(), &good_sign(),
            OPTIONS_CREATE_CONTEXT_READ_ONLY).unwrap();
        assert_eq!(t.context_read_only(id), Some(true));
    }

    #[test]
    fn create_context_exceeds_max_at_fifth() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        for _ in 0..4 {
            t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        }
        assert_eq!(
            t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap_err(),
            errors::EXCEEDS_MAX,
        );
    }

    #[test]
    fn destroy_context_out_of_range() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(t.destroy_context(0).unwrap_err(), errors::INVALID_ARGUMENT);
        assert_eq!(t.destroy_context(5).unwrap_err(), errors::INVALID_ARGUMENT);
    }

    #[test]
    fn destroy_context_unknown() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(t.destroy_context(2).unwrap_err(), errors::UNKNOWN_CONTEXT);
    }

    #[test]
    fn destroy_context_removes() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let id = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.destroy_context(id).unwrap();
        assert_eq!(t.destroy_context(id).unwrap_err(), errors::UNKNOWN_CONTEXT);
    }

    // ---- register + game info ---------------------------------------

    #[test]
    fn register_context_stores_trophies() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        assert_eq!(t.context_registered(c), Some(true));
    }

    #[test]
    fn register_context_requires_known_handle_and_context() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        assert_eq!(
            t.register_context(1, 1, &[]).unwrap_err(),
            errors::UNKNOWN_CONTEXT,
        );
    }

    #[test]
    fn register_twice_is_already_initialized() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &[]).unwrap();
        assert_eq!(
            t.register_context(c, h, &[]).unwrap_err(),
            errors::ALREADY_INITIALIZED,
        );
    }

    #[test]
    fn get_game_info_counts_per_grade() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        let info = t.get_game_info(c, h, true, true).unwrap();
        assert_eq!(info.num_trophies, 4);
        assert_eq!(info.num_bronze,   1);
        assert_eq!(info.num_silver,   1);
        assert_eq!(info.num_gold,     1);
        assert_eq!(info.num_platinum, 1);
        assert_eq!(info.unlocked_trophies, 0);
    }

    #[test]
    fn get_game_info_requires_registration() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        assert_eq!(
            t.get_game_info(c, h, true, true).unwrap_err(),
            errors::CONTEXT_NOT_REGISTERED,
        );
    }

    #[test]
    fn get_game_info_both_pointers_null_is_invalid() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        assert_eq!(
            t.get_game_info(c, h, false, false).unwrap_err(),
            errors::INVALID_ARGUMENT,
        );
    }

    // ---- required disk space / set sound level ----------------------

    #[test]
    fn required_disk_space_zero_on_happy_path() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        assert_eq!(t.required_disk_space(c, h, true, 0).unwrap(), 0);
    }

    #[test]
    fn required_disk_space_null_output_is_invalid() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        assert_eq!(
            t.required_disk_space(c, h, false, 0).unwrap_err(),
            errors::INVALID_ARGUMENT,
        );
    }

    #[test]
    fn set_sound_level_range() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        // Lower bound
        assert_eq!(t.set_sound_level(c, h, 19, 0).unwrap_err(), errors::INVALID_ARGUMENT);
        assert!(t.set_sound_level(c, h, 20, 0).is_ok());
        // Upper bound
        assert!(t.set_sound_level(c, h, 100, 0).is_ok());
        assert_eq!(t.set_sound_level(c, h, 101, 0).unwrap_err(), errors::INVALID_ARGUMENT);
    }

    #[test]
    fn set_sound_level_nonzero_options_not_supported() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        assert_eq!(t.set_sound_level(c, h, 50, 1).unwrap_err(), errors::NOT_SUPPORTED);
    }

    // ---- unlock trophy ----------------------------------------------

    #[test]
    fn unlock_trophy_bronze_ok() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        t.unlock_trophy(c, h, 0, 0xABCD).unwrap();
        let info = t.get_game_info(c, h, true, true).unwrap();
        assert_eq!(info.unlocked_trophies, 1);
        assert_eq!(info.unlocked_bronze,   1);
    }

    #[test]
    fn unlock_trophy_twice_is_already_unlocked() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        t.unlock_trophy(c, h, 0, 0).unwrap();
        assert_eq!(t.unlock_trophy(c, h, 0, 0).unwrap_err(), errors::ALREADY_UNLOCKED);
    }

    #[test]
    fn unlock_trophy_platinum_rejected() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        assert_eq!(
            t.unlock_trophy(c, h, 3, 0).unwrap_err(),
            errors::CANNOT_UNLOCK_PLATINUM,
        );
    }

    #[test]
    fn unlock_trophy_invalid_id() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        assert_eq!(t.unlock_trophy(c, h, -1, 0).unwrap_err(), errors::INVALID_TROPHY_ID);
        assert_eq!(t.unlock_trophy(c, h, 99, 0).unwrap_err(), errors::INVALID_TROPHY_ID);
    }

    #[test]
    fn unlock_trophy_requires_registration() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        assert_eq!(
            t.unlock_trophy(c, h, 0, 0).unwrap_err(),
            errors::CONTEXT_NOT_REGISTERED,
        );
    }

    // ---- unlock state / flag array ----------------------------------

    #[test]
    fn unlock_state_null_output_is_invalid() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        assert_eq!(t.unlock_state(c, h, false, true).unwrap_err(), errors::INVALID_ARGUMENT);
        assert_eq!(t.unlock_state(c, h, true, false).unwrap_err(), errors::INVALID_ARGUMENT);
    }

    #[test]
    fn unlock_state_reflects_unlocks() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        t.unlock_trophy(c, h, 0, 0).unwrap();
        t.unlock_trophy(c, h, 1, 0).unwrap();
        let (arr, count) = t.unlock_state(c, h, true, true).unwrap();
        assert_eq!(count, 4);
        assert!(arr.get(0));
        assert!(arr.get(1));
        assert!(!arr.get(2));
        assert_eq!(arr.popcount(), 2);
    }

    #[test]
    fn flag_array_layout() {
        let mut arr = TrophyFlagArray::new();
        assert!(arr.set(0));
        assert!(arr.set(31));
        assert!(arr.set(32));
        assert!(arr.set(127));
        assert!(!arr.set(128));
        assert_eq!(arr.words[0], (1u32) | (1u32 << 31));
        assert_eq!(arr.words[1], 1u32);
        assert_eq!(arr.words[3], 1u32 << 31);
    }

    // ---- game progress ----------------------------------------------

    #[test]
    fn game_progress_excludes_platinum() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        assert_eq!(t.game_progress(c, h, true).unwrap(), 0);
        t.unlock_trophy(c, h, 0, 0).unwrap();
        // 1 of 3 non-platinum = 33%
        assert_eq!(t.game_progress(c, h, true).unwrap(), 33);
        t.unlock_trophy(c, h, 1, 0).unwrap();
        t.unlock_trophy(c, h, 2, 0).unwrap();
        assert_eq!(t.game_progress(c, h, true).unwrap(), 100);
    }

    // ---- get trophy info --------------------------------------------

    #[test]
    fn get_trophy_info_returns_entry() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        let info = t.get_trophy_info(c, h, 2).unwrap();
        assert_eq!(info.grade, TrophyGrade::Gold);
        assert!(info.hidden);
    }

    #[test]
    fn get_trophy_info_out_of_range() {
        let mut t = Trophy::new();
        t.init(0).unwrap();
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(), 0).unwrap();
        t.register_context(c, h, &seeded_trophies()).unwrap();
        assert_eq!(t.get_trophy_info(c, h, -1).unwrap_err(), errors::INVALID_TROPHY_ID);
        assert_eq!(t.get_trophy_info(c, h, 99).unwrap_err(), errors::INVALID_TROPHY_ID);
    }

    // ---- check_context_handle error precedence ----------------------

    #[test]
    fn set_sound_level_precedence_level_first_then_init_then_range() {
        let t = Trophy::new();
        // level/options are pre-manager checks — they fire before NOT_INITIALIZED.
        assert_eq!(t.set_sound_level(0, 0, 19, 0).unwrap_err(), errors::INVALID_ARGUMENT);
        assert_eq!(t.set_sound_level(0, 0, 50, 99).unwrap_err(), errors::NOT_SUPPORTED);
        // Valid level + options → NOT_INITIALIZED (mgr not up) even with bad ids.
        assert_eq!(t.set_sound_level(0, 0, 50, 0).unwrap_err(), errors::NOT_INITIALIZED);
    }

    // ---- full lifecycle smoke ---------------------------------------

    #[test]
    fn full_trophy_lifecycle_smoke() {
        let mut t = Trophy::new();
        t.init(0).unwrap();

        // Allocate a handle and a context using a valid commId/sign.
        let h = t.create_handle(true).unwrap();
        let c = t.create_context(true, true, true, &good_comm_id(), &good_sign(),
            OPTIONS_CREATE_CONTEXT_READ_ONLY).unwrap();
        assert_eq!(t.context_read_only(c), Some(true));

        // Register with a synthetic trophy set.
        t.register_context(c, h, &seeded_trophies()).unwrap();

        // Before unlocks: progress = 0.
        assert_eq!(t.game_progress(c, h, true).unwrap(), 0);

        // Unlock bronze + silver (ids 0 and 1) — not platinum (3).
        t.unlock_trophy(c, h, 0, 0x1111).unwrap();
        t.unlock_trophy(c, h, 1, 0x2222).unwrap();

        let info = t.get_game_info(c, h, true, true).unwrap();
        assert_eq!(info.num_trophies, 4);
        assert_eq!(info.unlocked_trophies, 2);
        assert_eq!(info.unlocked_bronze, 1);
        assert_eq!(info.unlocked_silver, 1);

        let (arr, count) = t.unlock_state(c, h, true, true).unwrap();
        assert_eq!(count, 4);
        assert_eq!(arr.popcount(), 2);

        // Platinum cannot be unlocked directly.
        assert_eq!(t.unlock_trophy(c, h, 3, 0).unwrap_err(),
                   errors::CANNOT_UNLOCK_PLATINUM);

        // Tear down in reverse.
        t.destroy_context(c).unwrap();
        t.destroy_handle(h).unwrap();
        t.term().unwrap();
    }
}
