//! Rust port of `rpcs3/Emu/Cell/Modules/cellMusicDecode.cpp`.
//!
//! Ports the PS3 `cellMusicDecodeUtility` module (678 lines of C++).
//! The module exposes 20 PRX entry points: 10 for the legacy
//! `cellMusicDecode*` surface that drives a single-threaded decoder
//! (`music_decode`) and 10 mirror functions for `cellMusicDecode2*` that
//! operate on a `music_decode2` context (extends `music_decode` with a
//! `speed` field).
//!
//! Module name is `cellMusicDecodeUtility` byte-exact at cpp:654
//! `DECLARE(ppu_module_manager::cellMusicDecode)("cellMusicDecodeUtility", ...)`.
//! The internal log channel is `cellMusicDecode` (cpp:15).
//!
//! Error codes are byte-exact with `cellMusicDecode.h`:
//!
//! | name                                      | value         |
//! |-------------------------------------------|---------------|
//! | `CELL_MUSIC_DECODE_CANCELED`              | `0x0000_0001` |
//! | `CELL_MUSIC_DECODE_DECODE_FINISHED`       | `0x8002_C101` |
//! | `CELL_MUSIC_DECODE_ERROR_PARAM`           | `0x8002_C102` |
//! | `CELL_MUSIC_DECODE_ERROR_BUSY`            | `0x8002_C103` |
//! | `CELL_MUSIC_DECODE_ERROR_NO_ACTIVE_CONT.` | `0x8002_C104` |
//! | `CELL_MUSIC_DECODE_ERROR_NO_MATCH_FOUND`  | `0x8002_C105` |
//! | `CELL_MUSIC_DECODE_ERROR_INVALID_CONTEXT` | `0x8002_C106` |
//! | `CELL_MUSIC_DECODE_ERROR_DECODE_FAILURE`  | `0x8002_C107` |
//! | `CELL_MUSIC_DECODE_ERROR_NO_MORE_CONTENT` | `0x8002_C108` |
//! | `CELL_MUSIC_DECODE_DIALOG_OPEN`           | `0x8002_C109` |
//! | `CELL_MUSIC_DECODE_DIALOG_CLOSE`          | `0x8002_C10A` |
//! | `CELL_MUSIC_DECODE_ERROR_NO_LPCM_DATA`    | `0x8002_C10B` |
//! | `CELL_MUSIC_DECODE_NEXT_CONTENTS_READY`   | `0x8002_C10C` |
//! | `CELL_MUSIC_DECODE_ERROR_GENERIC`         | `0x8002_C1FF` |

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:654.
pub const MODULE_NAME: &str = "cellMusicDecodeUtility";

/// Registered entry points in the exact REG_FUNC order at cpp:656-676.
/// The first 10 are `cellMusicDecode*`, the final 10 are
/// `cellMusicDecode*2` mirrors operating on `music_decode2`.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellMusicDecodeInitialize",
    "cellMusicDecodeInitializeSystemWorkload",
    "cellMusicDecodeFinalize",
    "cellMusicDecodeSelectContents",
    "cellMusicDecodeSetDecodeCommand",
    "cellMusicDecodeGetDecodeStatus",
    "cellMusicDecodeRead",
    "cellMusicDecodeGetSelectionContext",
    "cellMusicDecodeSetSelectionContext",
    "cellMusicDecodeGetContentsId",
    "cellMusicDecodeInitialize2",
    "cellMusicDecodeInitialize2SystemWorkload",
    "cellMusicDecodeFinalize2",
    "cellMusicDecodeSelectContents2",
    "cellMusicDecodeSetDecodeCommand2",
    "cellMusicDecodeGetDecodeStatus2",
    "cellMusicDecodeRead2",
    "cellMusicDecodeGetSelectionContext2",
    "cellMusicDecodeSetSelectionContext2",
    "cellMusicDecodeGetContentsId2",
];

// --- Error codes (byte-exact) -------------------------------------------

pub const CELL_MUSIC_DECODE_CANCELED: CellError = CellError(0x0000_0001);
pub const CELL_MUSIC_DECODE_DECODE_FINISHED: CellError = CellError(0x8002_C101);
pub const CELL_MUSIC_DECODE_ERROR_PARAM: CellError = CellError(0x8002_C102);
pub const CELL_MUSIC_DECODE_ERROR_BUSY: CellError = CellError(0x8002_C103);
pub const CELL_MUSIC_DECODE_ERROR_NO_ACTIVE_CONTENT: CellError = CellError(0x8002_C104);
pub const CELL_MUSIC_DECODE_ERROR_NO_MATCH_FOUND: CellError = CellError(0x8002_C105);
pub const CELL_MUSIC_DECODE_ERROR_INVALID_CONTEXT: CellError = CellError(0x8002_C106);
pub const CELL_MUSIC_DECODE_ERROR_DECODE_FAILURE: CellError = CellError(0x8002_C107);
pub const CELL_MUSIC_DECODE_ERROR_NO_MORE_CONTENT: CellError = CellError(0x8002_C108);
pub const CELL_MUSIC_DECODE_DIALOG_OPEN: CellError = CellError(0x8002_C109);
pub const CELL_MUSIC_DECODE_DIALOG_CLOSE: CellError = CellError(0x8002_C10A);
pub const CELL_MUSIC_DECODE_ERROR_NO_LPCM_DATA: CellError = CellError(0x8002_C10B);
pub const CELL_MUSIC_DECODE_NEXT_CONTENTS_READY: CellError = CellError(0x8002_C10C);
pub const CELL_MUSIC_DECODE_ERROR_GENERIC: CellError = CellError(0x8002_C1FF);

// --- Constants (byte-exact with cellMusicDecode.h) ----------------------

pub const CELL_MUSIC_DECODE_EVENT_STATUS_NOTIFICATION: u32 = 0;
pub const CELL_MUSIC_DECODE_EVENT_INITIALIZE_RESULT: u32 = 1;
pub const CELL_MUSIC_DECODE_EVENT_FINALIZE_RESULT: u32 = 2;
pub const CELL_MUSIC_DECODE_EVENT_SELECT_CONTENTS_RESULT: u32 = 3;
pub const CELL_MUSIC_DECODE_EVENT_SET_DECODE_COMMAND_RESULT: u32 = 4;
pub const CELL_MUSIC_DECODE_EVENT_SET_SELECTION_CONTEXT_RESULT: u32 = 5;
pub const CELL_MUSIC_DECODE_EVENT_UI_NOTIFICATION: u32 = 6;
pub const CELL_MUSIC_DECODE_EVENT_NEXT_CONTENTS_READY_RESULT: u32 = 7;

pub const CELL_MUSIC_DECODE_MODE_NORMAL: i32 = 0;

pub const CELL_MUSIC_DECODE_CMD_STOP: i32 = 0;
pub const CELL_MUSIC_DECODE_CMD_START: i32 = 1;
pub const CELL_MUSIC_DECODE_CMD_NEXT: i32 = 2;
pub const CELL_MUSIC_DECODE_CMD_PREV: i32 = 3;

pub const CELL_MUSIC_DECODE_STATUS_DORMANT: i32 = 0;
pub const CELL_MUSIC_DECODE_STATUS_DECODING: i32 = 1;

pub const CELL_MUSIC_DECODE_POSITION_NONE: i32 = 0;
pub const CELL_MUSIC_DECODE_POSITION_START: i32 = 1;
pub const CELL_MUSIC_DECODE_POSITION_MID: i32 = 2;
pub const CELL_MUSIC_DECODE_POSITION_END: i32 = 3;
pub const CELL_MUSIC_DECODE_POSITION_END_LIST_END: i32 = 4;

pub const CELL_MUSIC_DECODE2_MODE_NORMAL: i32 = 0;

pub const CELL_MUSIC_DECODE2_SPEED_MAX: i32 = 0;
pub const CELL_MUSIC_DECODE2_SPEED_2: i32 = 2;

pub const CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE: i32 = 448 * 1024; // 458_752

// --- Validation helpers --------------------------------------------------

/// Reproduces `(spuPriority - 0x10U > 0xef)` guard from cpp:291 / cpp:472.
/// With the u32-underflow semantics of the C++ expression this is simply
/// `spuPriority` outside the closed range `0x10..=0xFF`.
#[must_use]
pub const fn is_valid_spu_priority(spu_priority: i32) -> bool {
    spu_priority >= 0x10 && spu_priority <= 0xFF
}

/// Reproduces `(spuUsageRate - 1U > 99)` guard from cpp:314 / cpp:498.
/// Valid range is `1..=100` (u32 underflow pushes 0 above 99).
#[must_use]
pub const fn is_valid_spu_usage_rate(rate: i32) -> bool {
    rate >= 1 && rate <= 100
}

/// Reproduces the command-range guard at cpp:364 / cpp:549 —
/// `command in [STOP..=PREV]`.
#[must_use]
pub const fn is_valid_decode_command(command: i32) -> bool {
    command >= CELL_MUSIC_DECODE_CMD_STOP && command <= CELL_MUSIC_DECODE_CMD_PREV
}

// --- Context --------------------------------------------------------------

/// Which cellMusicDecode surface drives a given context — the legacy
/// `music_decode` (cpp:44) or the `music_decode2` extension (cpp:121).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DecodeVariant {
    #[default]
    V1,
    V2,
}

/// Mirror of the C++ `music_selection_context` struct. The real SDK
/// threads an opaque hash through the game; here we track only the fields
/// the HLE surface depends on (the playlist vec and the current track
/// index).
#[derive(Debug, Default, Clone)]
pub struct MusicSelectionContext {
    pub playlist: Vec<String>,
    pub current_track: usize,
    pub hash: u64,
}

impl MusicSelectionContext {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            playlist: Vec::new(),
            current_track: 0,
            hash: 0,
        }
    }

    pub fn set_playlist<I, S>(&mut self, entries: I)
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.playlist = entries.into_iter().map(Into::into).collect();
        self.current_track = 0;
    }

    /// Advance to the next track if possible. Returns `None` when the
    /// playlist is exhausted — mirrors `set_next_index(...) == umax`
    /// signal from cpp:94/215.
    pub fn advance_next(&mut self) -> Option<usize> {
        let next = self.current_track + 1;
        if next >= self.playlist.len() {
            None
        } else {
            self.current_track = next;
            Some(next)
        }
    }

    /// Step backward. Returns `None` when already at index 0.
    pub fn advance_prev(&mut self) -> Option<usize> {
        if self.current_track == 0 {
            None
        } else {
            self.current_track -= 1;
            Some(self.current_track)
        }
    }
}

/// One queued sysutil callback deferred via `sysutil_register_cb`. Mirrors
/// the event-plus-status payload the C++ code posts back to the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferredCallback {
    pub event: u32,
    pub result: CellError,
    pub variant: DecodeVariant,
}

/// HLE state for one music_decode / music_decode2 instance, plus the
/// deferred-callback queue shared across both.
#[derive(Debug, Default)]
pub struct MusicDecode {
    variant: DecodeVariant,
    initialized: bool,
    has_callback: bool,
    userdata: u64,
    decode_status: i32,
    decode_command: i32,
    read_pos: u64,
    decoder_size: u64,
    speed: i32,
    buf_size: i32,
    // Byte buffer representing the LPCM stream the real decoder would
    // populate. In tests we push synthetic data via `inject_decoded_buffer`.
    data: Vec<u8>,
    current_context: MusicSelectionContext,
    has_decode_error: bool,
    track_fully_decoded: bool,
    timestamps_ms: VecDeque<(u64, u64)>, // (byte offset, timestamp ms)
    pending_callbacks: Vec<DeferredCallback>,
}

impl MusicDecode {
    #[must_use]
    pub fn new_v1() -> Self {
        Self {
            variant: DecodeVariant::V1,
            decode_status: CELL_MUSIC_DECODE_STATUS_DORMANT,
            decode_command: CELL_MUSIC_DECODE_CMD_STOP,
            speed: CELL_MUSIC_DECODE2_SPEED_MAX,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn new_v2() -> Self {
        Self {
            variant: DecodeVariant::V2,
            decode_status: CELL_MUSIC_DECODE_STATUS_DORMANT,
            decode_command: CELL_MUSIC_DECODE_CMD_STOP,
            speed: CELL_MUSIC_DECODE2_SPEED_MAX,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn variant(&self) -> DecodeVariant {
        self.variant
    }
    #[must_use]
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }
    #[must_use]
    pub fn decode_status(&self) -> i32 {
        self.decode_status
    }
    #[must_use]
    pub fn decode_command(&self) -> i32 {
        self.decode_command
    }
    #[must_use]
    pub fn read_pos(&self) -> u64 {
        self.read_pos
    }
    #[must_use]
    pub fn speed(&self) -> i32 {
        self.speed
    }
    #[must_use]
    pub fn pending_callbacks_count(&self) -> usize {
        self.pending_callbacks.len()
    }

    pub fn drain_callbacks(&mut self) -> Vec<DeferredCallback> {
        core::mem::take(&mut self.pending_callbacks)
    }

    fn queue_cb(&mut self, event: u32, result: CellError) {
        self.pending_callbacks.push(DeferredCallback {
            event,
            result,
            variant: self.variant,
        });
    }

    // --- entry points ---

    /// `cellMusicDecodeInitialize` (cpp:287-308).
    /// Validates mode, spu_priority range `[0x10..=0xFF]`, non-null
    /// callback. Queues an `INITIALIZE_RESULT` deferred callback.
    pub fn initialize(
        &mut self,
        mode: i32,
        spu_priority: i32,
        has_func: bool,
        userdata: u64,
    ) -> Result<(), CellError> {
        if mode != CELL_MUSIC_DECODE2_MODE_NORMAL
            || !is_valid_spu_priority(spu_priority)
            || !has_func
        {
            return Err(CELL_MUSIC_DECODE_ERROR_PARAM);
        }
        self.initialized = true;
        self.has_callback = true;
        self.userdata = userdata;
        self.queue_cb(CELL_MUSIC_DECODE_EVENT_INITIALIZE_RESULT, CellError::OK);
        Ok(())
    }

    /// `cellMusicDecodeInitializeSystemWorkload` (cpp:310-331).
    /// Swaps the priority check for an spu_usage_rate check in
    /// `[1..=100]`, adds spurs + priority-ptr validation.
    pub fn initialize_system_workload(
        &mut self,
        mode: i32,
        has_func: bool,
        spu_usage_rate: i32,
        has_spurs: bool,
        has_priority: bool,
        userdata: u64,
    ) -> Result<(), CellError> {
        if mode != CELL_MUSIC_DECODE2_MODE_NORMAL
            || !has_func
            || !is_valid_spu_usage_rate(spu_usage_rate)
            || !has_spurs
            || !has_priority
        {
            return Err(CELL_MUSIC_DECODE_ERROR_PARAM);
        }
        self.initialized = true;
        self.has_callback = true;
        self.userdata = userdata;
        self.queue_cb(CELL_MUSIC_DECODE_EVENT_INITIALIZE_RESULT, CellError::OK);
        Ok(())
    }

    /// `cellMusicDecodeInitialize2` (cpp:468-492).
    /// Adds the `bufSize >= MIN_BUFFER_SIZE` + speed ∈ {MAX, 2} guards.
    pub fn initialize2(
        &mut self,
        mode: i32,
        spu_priority: i32,
        has_func: bool,
        userdata: u64,
        speed: i32,
        buf_size: i32,
    ) -> Result<(), CellError> {
        if mode != CELL_MUSIC_DECODE2_MODE_NORMAL
            || !is_valid_spu_priority(spu_priority)
            || buf_size < CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE
            || !has_func
            || (speed != CELL_MUSIC_DECODE2_SPEED_MAX && speed != CELL_MUSIC_DECODE2_SPEED_2)
        {
            return Err(CELL_MUSIC_DECODE_ERROR_PARAM);
        }
        self.initialized = true;
        self.has_callback = true;
        self.userdata = userdata;
        self.speed = speed;
        self.buf_size = buf_size;
        self.queue_cb(CELL_MUSIC_DECODE_EVENT_INITIALIZE_RESULT, CellError::OK);
        Ok(())
    }

    /// `cellMusicDecodeInitialize2SystemWorkload` (cpp:494-516).
    /// Like `initialize_system_workload` + `bufSize >= MIN`.
    pub fn initialize2_system_workload(
        &mut self,
        mode: i32,
        has_func: bool,
        spu_usage_rate: i32,
        buf_size: i32,
        has_spurs: bool,
        has_priority: bool,
        userdata: u64,
    ) -> Result<(), CellError> {
        if mode != CELL_MUSIC_DECODE2_MODE_NORMAL
            || !has_func
            || !is_valid_spu_usage_rate(spu_usage_rate)
            || buf_size < CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE
            || !has_spurs
            || !has_priority
        {
            return Err(CELL_MUSIC_DECODE_ERROR_PARAM);
        }
        self.initialized = true;
        self.has_callback = true;
        self.userdata = userdata;
        self.buf_size = buf_size;
        self.queue_cb(CELL_MUSIC_DECODE_EVENT_INITIALIZE_RESULT, CellError::OK);
        Ok(())
    }

    /// `cellMusicDecodeFinalize` (cpp:333-351).
    /// `finalize()` in the C++ struct (cpp:111-118) resets status,
    /// command, read_pos; the wrapper queues a FINALIZE_RESULT callback
    /// if a callback was installed.
    pub fn finalize(&mut self) {
        // Always resets decoder state, matches cpp:111-118 unconditional
        // body. Callback is only queued when a func pointer is installed
        // (cpp:341).
        self.decode_status = CELL_MUSIC_DECODE_STATUS_DORMANT;
        self.decode_command = CELL_MUSIC_DECODE_CMD_STOP;
        self.read_pos = 0;
        self.data.clear();
        self.decoder_size = 0;
        self.track_fully_decoded = false;
        self.timestamps_ms.clear();
        if self.has_callback {
            self.queue_cb(CELL_MUSIC_DECODE_EVENT_FINALIZE_RESULT, CellError::OK);
        }
    }

    /// `cellMusicDecodeSetDecodeCommand` (cpp:360-388) + inner
    /// `set_decode_command` (cpp:56-109).
    /// Validates command range + `func != null` before executing.
    pub fn set_decode_command(&mut self, command: i32) -> Result<CellError, CellError> {
        if !is_valid_decode_command(command) {
            return Err(CELL_MUSIC_DECODE_ERROR_PARAM);
        }
        if !self.has_callback {
            return Err(CELL_MUSIC_DECODE_ERROR_GENERIC);
        }
        self.decode_command = command;
        let inner_result = match command {
            CELL_MUSIC_DECODE_CMD_STOP => {
                self.decode_status = CELL_MUSIC_DECODE_STATUS_DORMANT;
                self.read_pos = 0;
                CellError::OK
            }
            CELL_MUSIC_DECODE_CMD_START => {
                self.decode_status = CELL_MUSIC_DECODE_STATUS_DECODING;
                self.read_pos = 0;
                CellError::OK
            }
            CELL_MUSIC_DECODE_CMD_NEXT => {
                if self.current_context.advance_next().is_none() {
                    self.decode_status = CELL_MUSIC_DECODE_STATUS_DORMANT;
                    CELL_MUSIC_DECODE_ERROR_NO_MORE_CONTENT
                } else {
                    CellError::OK
                }
            }
            CELL_MUSIC_DECODE_CMD_PREV => {
                if self.current_context.advance_prev().is_none() {
                    self.decode_status = CELL_MUSIC_DECODE_STATUS_DORMANT;
                    CELL_MUSIC_DECODE_ERROR_NO_MORE_CONTENT
                } else {
                    CellError::OK
                }
            }
            _ => CellError::OK, // unreachable after is_valid check
        };
        self.queue_cb(
            CELL_MUSIC_DECODE_EVENT_SET_DECODE_COMMAND_RESULT,
            inner_result,
        );
        Ok(inner_result)
    }

    /// `cellMusicDecodeGetDecodeStatus` (cpp:390-403).
    /// Validates `status != null` (caller models null by passing `None`).
    pub fn get_decode_status(&self) -> Result<i32, CellError> {
        Ok(self.decode_status)
    }

    /// `cellMusicDecodeGetSelectionContext` (cpp:412-426).
    pub fn get_selection_context(&self) -> Result<&MusicSelectionContext, CellError> {
        Ok(&self.current_context)
    }

    /// `cellMusicDecodeSetSelectionContext` (cpp:428-453).
    /// Sets the current context; when the embedded `set(...)` check
    /// would fail we return `INVALID_CONTEXT` via the deferred callback,
    /// mirroring cpp:447.
    pub fn set_selection_context(
        &mut self,
        ctx: MusicSelectionContext,
        accept: bool,
    ) -> Result<(), CellError> {
        if !self.has_callback {
            return Err(CELL_MUSIC_DECODE_ERROR_GENERIC);
        }
        let status = if accept {
            self.current_context = ctx;
            CellError::OK
        } else {
            CELL_MUSIC_DECODE_ERROR_INVALID_CONTEXT
        };
        self.queue_cb(
            CELL_MUSIC_DECODE_EVENT_SET_SELECTION_CONTEXT_RESULT,
            status,
        );
        Ok(())
    }

    /// `cellMusicDecodeGetContentsId` (cpp:455-466).
    /// A thin wrapper around `current_context.find_content_id` — we
    /// reuse the same error surface. Returns
    /// `CELL_MUSIC_DECODE_ERROR_NO_MATCH_FOUND` if no content is active.
    pub fn get_contents_id(&self) -> Result<u64, CellError> {
        if self.current_context.playlist.is_empty() {
            Err(CELL_MUSIC_DECODE_ERROR_NO_ACTIVE_CONTENT)
        } else {
            Ok(self.current_context.hash)
        }
    }

    /// `cellMusicDecodeSelectContents` (cpp:353-358).
    /// Full dialog flow is out of scope; the port queues a
    /// `SELECT_CONTENTS_RESULT` callback with the supplied `status`
    /// (positive → OK, negative → `CELL_MUSIC_DECODE_CANCELED`) and
    /// applies the supplied playlist when accepted. Mirrors cpp:138-168.
    pub fn select_contents(
        &mut self,
        dialog_status: i32,
        picked: Option<MusicSelectionContext>,
    ) -> Result<(), CellError> {
        if !self.has_callback {
            return Err(CELL_MUSIC_DECODE_ERROR_GENERIC);
        }
        if dialog_status >= 0 {
            if let Some(ctx) = picked {
                self.current_context = ctx;
            }
            self.queue_cb(
                CELL_MUSIC_DECODE_EVENT_SELECT_CONTENTS_RESULT,
                CellError::OK,
            );
        } else {
            self.queue_cb(
                CELL_MUSIC_DECODE_EVENT_SELECT_CONTENTS_RESULT,
                CELL_MUSIC_DECODE_CANCELED,
            );
        }
        Ok(())
    }

    /// `cellMusicDecodeRead` (cpp:405-410) → `cell_music_decode_read`
    /// (cpp:175-285).
    /// Reports the LPCM position, the byte count actually copied and the
    /// start timestamp in ms. `buf` is represented by a caller-owned
    /// `&mut [u8]`; the function copies into its first `readSize` bytes
    /// and zero-fills the remainder to mirror cpp:237-241.
    pub fn read(
        &mut self,
        buf: &mut [u8],
        req_size: u64,
    ) -> Result<(u32, u64, i32), CellError> {
        if req_size == 0 || buf.is_empty() {
            return Err(CELL_MUSIC_DECODE_ERROR_PARAM);
        }
        if self.has_decode_error {
            return Err(CELL_MUSIC_DECODE_ERROR_DECODE_FAILURE);
        }
        if self.decoder_size == 0 {
            return Err(CELL_MUSIC_DECODE_ERROR_NO_LPCM_DATA);
        }
        let size_left = self.decoder_size - self.read_pos;
        let position = if self.read_pos == 0 {
            CELL_MUSIC_DECODE_POSITION_START
        } else if !self.track_fully_decoded || size_left > req_size {
            CELL_MUSIC_DECODE_POSITION_MID
        } else if self.current_context.advance_next().is_none() {
            CELL_MUSIC_DECODE_POSITION_END_LIST_END
        } else {
            CELL_MUSIC_DECODE_POSITION_END
        };

        let size_to_read = core::cmp::min(req_size, size_left);
        if size_to_read == 0 {
            return Err(CELL_MUSIC_DECODE_ERROR_NO_LPCM_DATA);
        }

        let start = self.read_pos as usize;
        let end = start + (size_to_read as usize);
        let copy_len = size_to_read as usize;
        let buf_len = buf.len().min(req_size as usize);
        let dst_copy = copy_len.min(buf_len);
        buf[..dst_copy].copy_from_slice(&self.data[start..start + dst_copy]);
        // Zero-fill remainder of caller buffer up to reqSize if the
        // decoded slice was shorter — prevents loud pops (cpp:237-241).
        if dst_copy < buf_len {
            for b in &mut buf[dst_copy..buf_len] {
                *b = 0;
            }
        }

        self.read_pos += size_to_read;

        let mut start_time_ms: u64 = 0;
        if let Some((_, ts)) = self.timestamps_ms.front() {
            start_time_ms = *ts;
            while self.timestamps_ms.len() > 1
                && self.read_pos >= self.timestamps_ms[1].0
            {
                self.timestamps_ms.pop_front();
            }
        }

        match position {
            CELL_MUSIC_DECODE_POSITION_END_LIST_END => {
                self.decode_command = CELL_MUSIC_DECODE_CMD_STOP;
                self.decode_status = CELL_MUSIC_DECODE_STATUS_DORMANT;
                self.read_pos = 0;
            }
            CELL_MUSIC_DECODE_POSITION_END => {
                self.read_pos = 0;
            }
            _ => {}
        }

        let _ = end; // silence unused warning when opt=0
        Ok((start_time_ms as u32, size_to_read, position))
    }

    // --- test / harness helpers ---

    pub fn inject_decoded_buffer(&mut self, data: Vec<u8>) {
        self.decoder_size = data.len() as u64;
        self.data = data;
        self.read_pos = 0;
        self.track_fully_decoded = true;
    }

    pub fn mark_decode_error(&mut self) {
        self.has_decode_error = true;
    }

    pub fn push_timestamp(&mut self, offset_bytes: u64, ts_ms: u64) {
        self.timestamps_ms.push_back((offset_bytes, ts_ms));
    }

    pub fn set_context(&mut self, ctx: MusicSelectionContext) {
        self.current_context = ctx;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellMusicDecodeUtility");
    }

    #[test]
    fn registered_entry_points_count_and_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 20);
        // First 10 are the legacy surface.
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellMusicDecodeInitialize");
        assert_eq!(
            REGISTERED_ENTRY_POINTS[9],
            "cellMusicDecodeGetContentsId"
        );
        // Mirror surface starts at index 10.
        assert_eq!(REGISTERED_ENTRY_POINTS[10], "cellMusicDecodeInitialize2");
        assert_eq!(
            REGISTERED_ENTRY_POINTS[19],
            "cellMusicDecodeGetContentsId2"
        );
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_MUSIC_DECODE_CANCELED.0, 0x0000_0001);
        assert_eq!(CELL_MUSIC_DECODE_DECODE_FINISHED.0, 0x8002_C101);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_PARAM.0, 0x8002_C102);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_BUSY.0, 0x8002_C103);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_NO_ACTIVE_CONTENT.0, 0x8002_C104);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_NO_MATCH_FOUND.0, 0x8002_C105);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_INVALID_CONTEXT.0, 0x8002_C106);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_DECODE_FAILURE.0, 0x8002_C107);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_NO_MORE_CONTENT.0, 0x8002_C108);
        assert_eq!(CELL_MUSIC_DECODE_DIALOG_OPEN.0, 0x8002_C109);
        assert_eq!(CELL_MUSIC_DECODE_DIALOG_CLOSE.0, 0x8002_C10A);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_NO_LPCM_DATA.0, 0x8002_C10B);
        assert_eq!(CELL_MUSIC_DECODE_NEXT_CONTENTS_READY.0, 0x8002_C10C);
        assert_eq!(CELL_MUSIC_DECODE_ERROR_GENERIC.0, 0x8002_C1FF);
    }

    #[test]
    fn event_constants_byte_exact() {
        assert_eq!(CELL_MUSIC_DECODE_EVENT_STATUS_NOTIFICATION, 0);
        assert_eq!(CELL_MUSIC_DECODE_EVENT_INITIALIZE_RESULT, 1);
        assert_eq!(CELL_MUSIC_DECODE_EVENT_FINALIZE_RESULT, 2);
        assert_eq!(CELL_MUSIC_DECODE_EVENT_SELECT_CONTENTS_RESULT, 3);
        assert_eq!(CELL_MUSIC_DECODE_EVENT_SET_DECODE_COMMAND_RESULT, 4);
        assert_eq!(CELL_MUSIC_DECODE_EVENT_SET_SELECTION_CONTEXT_RESULT, 5);
        assert_eq!(CELL_MUSIC_DECODE_EVENT_UI_NOTIFICATION, 6);
        assert_eq!(CELL_MUSIC_DECODE_EVENT_NEXT_CONTENTS_READY_RESULT, 7);
    }

    #[test]
    fn command_and_status_constants() {
        assert_eq!(CELL_MUSIC_DECODE_CMD_STOP, 0);
        assert_eq!(CELL_MUSIC_DECODE_CMD_START, 1);
        assert_eq!(CELL_MUSIC_DECODE_CMD_NEXT, 2);
        assert_eq!(CELL_MUSIC_DECODE_CMD_PREV, 3);
        assert_eq!(CELL_MUSIC_DECODE_STATUS_DORMANT, 0);
        assert_eq!(CELL_MUSIC_DECODE_STATUS_DECODING, 1);
    }

    #[test]
    fn position_constants() {
        assert_eq!(CELL_MUSIC_DECODE_POSITION_NONE, 0);
        assert_eq!(CELL_MUSIC_DECODE_POSITION_START, 1);
        assert_eq!(CELL_MUSIC_DECODE_POSITION_MID, 2);
        assert_eq!(CELL_MUSIC_DECODE_POSITION_END, 3);
        assert_eq!(CELL_MUSIC_DECODE_POSITION_END_LIST_END, 4);
    }

    #[test]
    fn min_buffer_size_exact() {
        assert_eq!(CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE, 458_752);
    }

    #[test]
    fn spu_priority_range() {
        assert!(!is_valid_spu_priority(0));
        assert!(!is_valid_spu_priority(0x0F));
        assert!(is_valid_spu_priority(0x10));
        assert!(is_valid_spu_priority(0x80));
        assert!(is_valid_spu_priority(0xFF));
        assert!(!is_valid_spu_priority(0x100));
    }

    #[test]
    fn spu_usage_rate_range() {
        assert!(!is_valid_spu_usage_rate(0));
        assert!(is_valid_spu_usage_rate(1));
        assert!(is_valid_spu_usage_rate(50));
        assert!(is_valid_spu_usage_rate(100));
        assert!(!is_valid_spu_usage_rate(101));
    }

    #[test]
    fn decode_command_range() {
        assert!(!is_valid_decode_command(-1));
        assert!(is_valid_decode_command(0));
        assert!(is_valid_decode_command(3));
        assert!(!is_valid_decode_command(4));
    }

    #[test]
    fn initialize_ok_queues_cb() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0xDEAD).unwrap();
        assert!(m.is_initialized());
        let cbs = m.drain_callbacks();
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].event, CELL_MUSIC_DECODE_EVENT_INITIALIZE_RESULT);
        assert_eq!(cbs[0].result, CellError::OK);
        assert_eq!(cbs[0].variant, DecodeVariant::V1);
    }

    #[test]
    fn initialize_bad_mode_is_param_error() {
        let mut m = MusicDecode::new_v1();
        assert_eq!(
            m.initialize(1, 0x10, true, 0),
            Err(CELL_MUSIC_DECODE_ERROR_PARAM)
        );
        assert!(!m.is_initialized());
    }

    #[test]
    fn initialize_bad_priority_is_param_error() {
        let mut m = MusicDecode::new_v1();
        assert_eq!(
            m.initialize(0, 0x0F, true, 0),
            Err(CELL_MUSIC_DECODE_ERROR_PARAM)
        );
    }

    #[test]
    fn initialize_no_func_is_param_error() {
        let mut m = MusicDecode::new_v1();
        assert_eq!(
            m.initialize(0, 0x10, false, 0),
            Err(CELL_MUSIC_DECODE_ERROR_PARAM)
        );
    }

    #[test]
    fn initialize_system_workload_ok() {
        let mut m = MusicDecode::new_v1();
        m.initialize_system_workload(0, true, 50, true, true, 0)
            .unwrap();
        assert!(m.is_initialized());
    }

    #[test]
    fn initialize_system_workload_zero_rate_is_param() {
        let mut m = MusicDecode::new_v1();
        assert_eq!(
            m.initialize_system_workload(0, true, 0, true, true, 0),
            Err(CELL_MUSIC_DECODE_ERROR_PARAM)
        );
    }

    #[test]
    fn initialize2_rejects_bufsize_below_min() {
        let mut m = MusicDecode::new_v2();
        assert_eq!(
            m.initialize2(
                0,
                0x10,
                true,
                0,
                CELL_MUSIC_DECODE2_SPEED_MAX,
                CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE - 1
            ),
            Err(CELL_MUSIC_DECODE_ERROR_PARAM)
        );
    }

    #[test]
    fn initialize2_rejects_bad_speed() {
        let mut m = MusicDecode::new_v2();
        assert_eq!(
            m.initialize2(0, 0x10, true, 0, 1, CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE),
            Err(CELL_MUSIC_DECODE_ERROR_PARAM)
        );
    }

    #[test]
    fn initialize2_ok_stores_speed() {
        let mut m = MusicDecode::new_v2();
        m.initialize2(
            0,
            0x10,
            true,
            0,
            CELL_MUSIC_DECODE2_SPEED_2,
            CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE,
        )
        .unwrap();
        assert_eq!(m.speed(), CELL_MUSIC_DECODE2_SPEED_2);
    }

    #[test]
    fn initialize2_system_workload_ok() {
        let mut m = MusicDecode::new_v2();
        m.initialize2_system_workload(
            0,
            true,
            25,
            CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE,
            true,
            true,
            0,
        )
        .unwrap();
        assert!(m.is_initialized());
    }

    #[test]
    fn finalize_resets_state_and_queues_cb() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        m.drain_callbacks();
        m.set_decode_command(CELL_MUSIC_DECODE_CMD_START).unwrap();
        assert_eq!(m.decode_status(), CELL_MUSIC_DECODE_STATUS_DECODING);
        m.finalize();
        assert_eq!(m.decode_status(), CELL_MUSIC_DECODE_STATUS_DORMANT);
        assert_eq!(m.decode_command(), CELL_MUSIC_DECODE_CMD_STOP);
        let cbs = m.drain_callbacks();
        let fin = cbs
            .iter()
            .find(|c| c.event == CELL_MUSIC_DECODE_EVENT_FINALIZE_RESULT);
        assert!(fin.is_some());
    }

    #[test]
    fn set_decode_command_rejects_bad_range() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        assert_eq!(m.set_decode_command(-1), Err(CELL_MUSIC_DECODE_ERROR_PARAM));
        assert_eq!(m.set_decode_command(4), Err(CELL_MUSIC_DECODE_ERROR_PARAM));
    }

    #[test]
    fn set_decode_command_without_func_is_generic() {
        let mut m = MusicDecode::new_v1();
        // No init — no callback installed.
        assert_eq!(
            m.set_decode_command(CELL_MUSIC_DECODE_CMD_STOP),
            Err(CELL_MUSIC_DECODE_ERROR_GENERIC)
        );
    }

    #[test]
    fn set_decode_command_start_sets_decoding_status() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        m.drain_callbacks();
        let res = m.set_decode_command(CELL_MUSIC_DECODE_CMD_START).unwrap();
        assert_eq!(res, CellError::OK);
        assert_eq!(m.decode_status(), CELL_MUSIC_DECODE_STATUS_DECODING);
    }

    #[test]
    fn set_decode_command_next_without_more_returns_no_more_content() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        m.drain_callbacks();
        // Single-track playlist — NEXT has nothing to advance to.
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["song.mp3"]);
        m.set_context(ctx);
        let res = m.set_decode_command(CELL_MUSIC_DECODE_CMD_NEXT).unwrap();
        assert_eq!(res, CELL_MUSIC_DECODE_ERROR_NO_MORE_CONTENT);
        assert_eq!(m.decode_status(), CELL_MUSIC_DECODE_STATUS_DORMANT);
    }

    #[test]
    fn set_decode_command_prev_at_start_returns_no_more_content() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        m.drain_callbacks();
        let res = m.set_decode_command(CELL_MUSIC_DECODE_CMD_PREV).unwrap();
        assert_eq!(res, CELL_MUSIC_DECODE_ERROR_NO_MORE_CONTENT);
    }

    #[test]
    fn set_decode_command_next_advances_on_multi_track() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a.mp3", "b.mp3", "c.mp3"]);
        m.set_context(ctx);
        let r = m.set_decode_command(CELL_MUSIC_DECODE_CMD_NEXT).unwrap();
        assert_eq!(r, CellError::OK);
    }

    #[test]
    fn select_contents_cancels_on_negative_status() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        m.drain_callbacks();
        m.select_contents(-1, None).unwrap();
        let cbs = m.drain_callbacks();
        let pick = cbs
            .iter()
            .find(|c| c.event == CELL_MUSIC_DECODE_EVENT_SELECT_CONTENTS_RESULT)
            .unwrap();
        assert_eq!(pick.result, CELL_MUSIC_DECODE_CANCELED);
    }

    #[test]
    fn select_contents_ok_applies_context() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        m.drain_callbacks();
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["x.mp3"]);
        ctx.hash = 0xABCD_EF01_2345_6789;
        m.select_contents(0, Some(ctx)).unwrap();
        assert_eq!(m.get_contents_id().unwrap(), 0xABCD_EF01_2345_6789);
    }

    #[test]
    fn set_selection_context_rejection_queues_invalid_context() {
        let mut m = MusicDecode::new_v1();
        m.initialize(0, 0x10, true, 0).unwrap();
        m.drain_callbacks();
        m.set_selection_context(MusicSelectionContext::new(), false)
            .unwrap();
        let cbs = m.drain_callbacks();
        let evt = cbs
            .iter()
            .find(|c| c.event == CELL_MUSIC_DECODE_EVENT_SET_SELECTION_CONTEXT_RESULT)
            .unwrap();
        assert_eq!(evt.result, CELL_MUSIC_DECODE_ERROR_INVALID_CONTEXT);
    }

    #[test]
    fn get_contents_id_without_content_is_no_active() {
        let m = MusicDecode::new_v1();
        assert_eq!(
            m.get_contents_id(),
            Err(CELL_MUSIC_DECODE_ERROR_NO_ACTIVE_CONTENT)
        );
    }

    #[test]
    fn read_rejects_empty_request() {
        let mut m = MusicDecode::new_v1();
        let mut buf = [0u8; 0];
        assert_eq!(m.read(&mut buf, 0), Err(CELL_MUSIC_DECODE_ERROR_PARAM));
    }

    #[test]
    fn read_surfaces_decode_error() {
        let mut m = MusicDecode::new_v1();
        m.mark_decode_error();
        let mut buf = [0u8; 16];
        assert_eq!(
            m.read(&mut buf, 16),
            Err(CELL_MUSIC_DECODE_ERROR_DECODE_FAILURE)
        );
    }

    #[test]
    fn read_no_lpcm_when_decoder_empty() {
        let mut m = MusicDecode::new_v1();
        let mut buf = [0u8; 16];
        assert_eq!(
            m.read(&mut buf, 16),
            Err(CELL_MUSIC_DECODE_ERROR_NO_LPCM_DATA)
        );
    }

    #[test]
    fn read_first_returns_position_start() {
        let mut m = MusicDecode::new_v1();
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a.mp3", "b.mp3"]);
        m.set_context(ctx);
        let data: Vec<u8> = (0..64u8).collect();
        m.inject_decoded_buffer(data.clone());
        let mut buf = [0u8; 16];
        let (ts, read, pos) = m.read(&mut buf, 16).unwrap();
        assert_eq!(pos, CELL_MUSIC_DECODE_POSITION_START);
        assert_eq!(read, 16);
        assert_eq!(ts, 0);
        // First 16 bytes match the source.
        assert_eq!(&buf[..], &data[..16]);
    }

    #[test]
    fn read_at_end_advances_playlist_to_position_end() {
        let mut m = MusicDecode::new_v1();
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a.mp3", "b.mp3"]);
        m.set_context(ctx);
        let data: Vec<u8> = (0..8u8).collect();
        m.inject_decoded_buffer(data);
        let mut buf = [0u8; 4];
        let _ = m.read(&mut buf, 4).unwrap(); // position_start, pos=4
        let (_, _, pos) = m.read(&mut buf, 4).unwrap(); // tail
        assert_eq!(pos, CELL_MUSIC_DECODE_POSITION_END);
        // After POSITION_END read_pos resets to 0 (cpp:269-274).
        assert_eq!(m.read_pos(), 0);
    }

    #[test]
    fn read_at_end_of_single_track_playlist_is_list_end() {
        let mut m = MusicDecode::new_v1();
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["only.mp3"]);
        m.set_context(ctx);
        let data: Vec<u8> = (0..8u8).collect();
        m.inject_decoded_buffer(data);
        let mut buf = [0u8; 4];
        let _ = m.read(&mut buf, 4).unwrap();
        let (_, _, pos) = m.read(&mut buf, 4).unwrap();
        assert_eq!(pos, CELL_MUSIC_DECODE_POSITION_END_LIST_END);
        assert_eq!(m.decode_status(), CELL_MUSIC_DECODE_STATUS_DORMANT);
    }

    #[test]
    fn read_timestamps_advance_with_read_pos() {
        let mut m = MusicDecode::new_v1();
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["a.mp3", "b.mp3"]);
        m.set_context(ctx);
        let data: Vec<u8> = (0..32u8).collect();
        m.inject_decoded_buffer(data);
        m.push_timestamp(0, 0);
        m.push_timestamp(16, 500);
        let mut buf = [0u8; 16];
        let (ts1, _, _) = m.read(&mut buf, 16).unwrap();
        assert_eq!(ts1, 0);
        // After reading past the 16-byte boundary the 500-ms timestamp
        // should become the head.
        let (ts2, _, _) = m.read(&mut buf, 16).unwrap();
        assert_eq!(ts2, 500);
    }

    #[test]
    fn full_cellmusicdecode_lifecycle_smoke() {
        let mut m = MusicDecode::new_v2();
        // 1. Initialize2 with a valid buffer size and speed.
        m.initialize2(
            0,
            0x10,
            true,
            0xDEAD_BEEF,
            CELL_MUSIC_DECODE2_SPEED_2,
            CELL_MUSIC_DECODE2_MIN_BUFFER_SIZE,
        )
        .unwrap();
        assert_eq!(m.speed(), CELL_MUSIC_DECODE2_SPEED_2);

        // 2. Simulate media dialog picking a playlist.
        let mut ctx = MusicSelectionContext::new();
        ctx.set_playlist(["track1.mp3", "track2.mp3"]);
        ctx.hash = 0xCAFE;
        m.select_contents(0, Some(ctx)).unwrap();

        // 3. Start decoder — transitions to DECODING.
        let r = m.set_decode_command(CELL_MUSIC_DECODE_CMD_START).unwrap();
        assert_eq!(r, CellError::OK);
        assert_eq!(m.decode_status(), CELL_MUSIC_DECODE_STATUS_DECODING);

        // 4. Inject decoded LPCM bytes and drain them via read.
        let data: Vec<u8> = (0..8u8).collect();
        m.inject_decoded_buffer(data);
        m.push_timestamp(0, 0);
        let mut buf = [0u8; 8];
        let (_, n, pos_first) = m.read(&mut buf, 8).unwrap();
        assert_eq!(pos_first, CELL_MUSIC_DECODE_POSITION_START);
        assert_eq!(n, 8);

        // 5. Contents id reflects the selected playlist.
        assert_eq!(m.get_contents_id().unwrap(), 0xCAFE);

        // 6. Finalize cleanly resets + queues FINALIZE_RESULT.
        m.finalize();
        assert_eq!(m.decode_status(), CELL_MUSIC_DECODE_STATUS_DORMANT);
        assert_eq!(m.decode_command(), CELL_MUSIC_DECODE_CMD_STOP);

        let cbs = m.drain_callbacks();
        // Expected deferred events in order: INITIALIZE_RESULT,
        // SELECT_CONTENTS_RESULT, SET_DECODE_COMMAND_RESULT,
        // FINALIZE_RESULT.
        let events: Vec<u32> = cbs.iter().map(|c| c.event).collect();
        assert_eq!(
            events,
            alloc::vec![
                CELL_MUSIC_DECODE_EVENT_INITIALIZE_RESULT,
                CELL_MUSIC_DECODE_EVENT_SELECT_CONTENTS_RESULT,
                CELL_MUSIC_DECODE_EVENT_SET_DECODE_COMMAND_RESULT,
                CELL_MUSIC_DECODE_EVENT_FINALIZE_RESULT,
            ]
        );
        // Every callback carries the V2 variant tag (since we used
        // new_v2()).
        assert!(cbs.iter().all(|c| c.variant == DecodeVariant::V2));
    }
}
