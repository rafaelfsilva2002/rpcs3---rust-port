//! Rust port of `rpcs3/Emu/Cell/Modules/cellCelpEnc.cpp`.
//!
//! 10 PRX entries under the module name `cellCelpEnc` — the CELP
//! (Code-Excited Linear Prediction) speech encoder used by games that
//! compress voice chat / speaker dialogue offline. All C++ bodies are
//! `todo()` stubs returning `CELL_OK`; the Rust port adds FSM
//! enforcement + handle registry + param validation so the failure
//! modes the real SDK surfaces are testable.
//!
//! REG_FUNC order at cpp:89-98:
//!
//!  1. `cellCelpEncQueryAttr`
//!  2. `cellCelpEncOpen`
//!  3. `cellCelpEncOpenEx`
//!  4. `cellCelpEncOpenExt`
//!  5. `cellCelpEncClose`
//!  6. `cellCelpEncStart`
//!  7. `cellCelpEncEnd`
//!  8. `cellCelpEncEncodeFrame`
//!  9. `cellCelpEncWaitForOutput`
//! 10. `cellCelpEncGetAu`
//!
//! Module name byte-exact at cpp:6 / cpp:87.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:6 / cpp:87.
pub const MODULE_NAME: &str = "cellCelpEnc";

/// REG_FUNC order at cpp:89-98.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellCelpEncQueryAttr",
    "cellCelpEncOpen",
    "cellCelpEncOpenEx",
    "cellCelpEncOpenExt",
    "cellCelpEncClose",
    "cellCelpEncStart",
    "cellCelpEncEnd",
    "cellCelpEncEncodeFrame",
    "cellCelpEncWaitForOutput",
    "cellCelpEncGetAu",
];

// --- Error codes (byte-exact cellCelpEnc.h:10-18) -----------------------

pub const CELL_CELPENC_ERROR_FAILED: CellError = CellError(0x8061_4001);
pub const CELL_CELPENC_ERROR_SEQ: CellError = CellError(0x8061_4002);
pub const CELL_CELPENC_ERROR_ARG: CellError = CellError(0x8061_4003);
pub const CELL_CELPENC_ERROR_CORE_FAILED: CellError = CellError(0x8061_4081);
pub const CELL_CELPENC_ERROR_CORE_SEQ: CellError = CellError(0x8061_4082);
pub const CELL_CELPENC_ERROR_CORE_ARG: CellError = CellError(0x8061_4083);

// --- Configuration enums (cellCelpEnc.h:20-43) --------------------------

pub const CELL_CELPENC_RPE_CONFIG_0: u32 = 0;
pub const CELL_CELPENC_RPE_CONFIG_1: u32 = 1;
pub const CELL_CELPENC_RPE_CONFIG_2: u32 = 2;
pub const CELL_CELPENC_RPE_CONFIG_3: u32 = 3;

pub const CELL_CELPENC_FS_16KHZ: u32 = 2;

pub const CELL_CELPENC_EXCITATION_MODE_RPE: u32 = 1;

pub const CELL_CELPENC_WORD_SZ_INT16_LE: u32 = 0;
pub const CELL_CELPENC_WORD_SZ_FLOAT: u32 = 1;

/// Upper/lower version words reported by `QueryAttr`. The real firmware
/// publishes the driver's SDK version; the port commits a stable pair
/// that tests can assert against.
pub const CELL_CELPENC_VERSION_UPPER: u32 = 0x0001_0000;
pub const CELL_CELPENC_VERSION_LOWER: u32 = 0x0000_0000;

/// Required working memory the CELP encoder asks for (2 MiB — a
/// conventional figure for SPU-side audio modules on PS3).
pub const CELL_CELPENC_WORK_MEM_SIZE: u32 = 2 * 1024 * 1024;

// --- Wire structs (byte-exact with cellCelpEnc.h) -----------------------

/// Mirror of `CellCelpEncAttr` (cellCelpEnc.h:45-50).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelpEncAttr {
    pub work_mem_size: u32,
    pub celp_enc_ver_upper: u32,
    pub celp_enc_ver_lower: u32,
}

/// Mirror of `CellCelpEncResource` (cellCelpEnc.h:52-59).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelpEncResource {
    pub total_mem_size: u32,
    pub start_addr: u32,
    pub ppu_thread_priority: u32,
    pub spu_thread_priority: u32,
    pub ppu_thread_stack_size: u32,
}

/// Mirror of `CellCelpEncParam` (cellCelpEnc.h:61-69).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelpEncParam {
    pub excitation_mode: u32,
    pub sample_rate: u32,
    pub configuration: u32,
    pub word_size: u32,
    pub out_buff: u32,
    pub out_size: u32,
}

/// Mirror of `CellCelpEncAuInfo` (cellCelpEnc.h:71-75).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelpEncAuInfo {
    pub start_addr: u32,
    pub size: u32,
}

/// Mirror of `CellCelpEncPcmInfo` (cellCelpEnc.h:77-81).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelpEncPcmInfo {
    pub start_addr: u32,
    pub size: u32,
}

/// Mirror of `CellCelpEncResourceEx` (cellCelpEnc.h:83-90).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelpEncResourceEx {
    pub total_mem_size: u32,
    pub start_addr: u32,
    pub spurs: u32,
    pub priority: [u8; 8],
    pub max_contention: u32,
}

// --- Validation helpers -------------------------------------------------

/// `configuration ∈ {0,1,2,3}` (cellCelpEnc.h:21-27).
#[must_use]
pub const fn is_valid_rpe_config(c: u32) -> bool {
    c <= CELL_CELPENC_RPE_CONFIG_3
}

/// `excitation_mode == RPE` (cellCelpEnc.h:34-37).
#[must_use]
pub const fn is_valid_excitation_mode(m: u32) -> bool {
    m == CELL_CELPENC_EXCITATION_MODE_RPE
}

/// `sample_rate == FS_16KHZ` (cellCelpEnc.h:29-32).
#[must_use]
pub const fn is_valid_sample_rate(r: u32) -> bool {
    r == CELL_CELPENC_FS_16KHZ
}

/// `word_size ∈ {INT16_LE, FLOAT}` (cellCelpEnc.h:39-43).
#[must_use]
pub const fn is_valid_word_size(w: u32) -> bool {
    w == CELL_CELPENC_WORD_SZ_INT16_LE || w == CELL_CELPENC_WORD_SZ_FLOAT
}

// --- FSM + handle model -------------------------------------------------

/// Per-handle lifecycle: `Opened` (after `Open`/`OpenEx`) → `Started`
/// (after `Start`) ↔ encode/read cycle → `Ended` (after `End`) →
/// dropped on `Close`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderState {
    Opened,
    Started,
    Ended,
}

/// Which Open variant produced a handle. Tests assert against this to
/// verify the right resource shape was supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenVariant {
    Open,
    OpenEx,
    OpenExt,
}

#[derive(Debug, Clone)]
pub struct EncoderHandle {
    pub id: u32,
    pub state: EncoderState,
    pub variant: OpenVariant,
    pub param: Option<CellCelpEncParam>,
    /// Simulated access-unit queue: tests push bytes via
    /// [`CelpEnc::inject_au`]; [`CelpEnc::get_au`] drains them.
    pub pending_aus: VecDeque<Vec<u8>>,
    pub encode_frame_count: u32,
    pub wait_calls: u32,
}

/// Singleton manager — the firmware allows many handles concurrently;
/// the port caps `MAX_HANDLES` to a small number so runaway tests
/// fail fast.
#[derive(Debug)]
pub struct CelpEnc {
    handles: Vec<EncoderHandle>,
    next_id: u32,
    query_attr_calls: u32,
    open_calls: u32,
    open_ex_calls: u32,
    open_ext_calls: u32,
    close_calls: u32,
    start_calls: u32,
    end_calls: u32,
    encode_frame_calls: u32,
    wait_for_output_calls: u32,
    get_au_calls: u32,
}

impl Default for CelpEnc {
    fn default() -> Self {
        Self::new()
    }
}

impl CelpEnc {
    pub const MAX_HANDLES: usize = 8;

    #[must_use]
    pub const fn new() -> Self {
        Self {
            handles: Vec::new(),
            next_id: 1,
            query_attr_calls: 0,
            open_calls: 0,
            open_ex_calls: 0,
            open_ext_calls: 0,
            close_calls: 0,
            start_calls: 0,
            end_calls: 0,
            encode_frame_calls: 0,
            wait_for_output_calls: 0,
            get_au_calls: 0,
        }
    }

    #[must_use]
    pub fn handle_count(&self) -> usize {
        self.handles.len()
    }

    fn handle_mut(&mut self, id: u32) -> Result<&mut EncoderHandle, CellError> {
        self.handles
            .iter_mut()
            .find(|h| h.id == id)
            .ok_or(CELL_CELPENC_ERROR_ARG)
    }

    #[allow(dead_code)]
    fn handle_ref(&self, id: u32) -> Result<&EncoderHandle, CellError> {
        self.handles
            .iter()
            .find(|h| h.id == id)
            .ok_or(CELL_CELPENC_ERROR_ARG)
    }

    // --- counters ---

    #[must_use]
    pub fn query_attr_calls(&self) -> u32 {
        self.query_attr_calls
    }
    #[must_use]
    pub fn open_calls(&self) -> u32 {
        self.open_calls
    }
    #[must_use]
    pub fn open_ex_calls(&self) -> u32 {
        self.open_ex_calls
    }
    #[must_use]
    pub fn open_ext_calls(&self) -> u32 {
        self.open_ext_calls
    }
    #[must_use]
    pub fn close_calls(&self) -> u32 {
        self.close_calls
    }
    #[must_use]
    pub fn start_calls(&self) -> u32 {
        self.start_calls
    }
    #[must_use]
    pub fn end_calls(&self) -> u32 {
        self.end_calls
    }
    #[must_use]
    pub fn encode_frame_calls(&self) -> u32 {
        self.encode_frame_calls
    }
    #[must_use]
    pub fn wait_for_output_calls(&self) -> u32 {
        self.wait_for_output_calls
    }
    #[must_use]
    pub fn get_au_calls(&self) -> u32 {
        self.get_au_calls
    }

    // --- test hooks ---

    /// Queue an encoded access unit so the next `get_au` returns it.
    /// The real firmware populates the queue from the SPU encoder.
    pub fn inject_au(&mut self, handle_id: u32, bytes: Vec<u8>) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        h.pending_aus.push_back(bytes);
        Ok(())
    }

    // --- entry points ---

    /// `cellCelpEncQueryAttr` (cpp:27-31). Populates the caller's
    /// `CellCelpEncAttr` with the work-mem requirement and SDK
    /// version.
    pub fn query_attr(&mut self) -> Result<CellCelpEncAttr, CellError> {
        self.query_attr_calls = self.query_attr_calls.saturating_add(1);
        Ok(CellCelpEncAttr {
            work_mem_size: CELL_CELPENC_WORK_MEM_SIZE,
            celp_enc_ver_upper: CELL_CELPENC_VERSION_UPPER,
            celp_enc_ver_lower: CELL_CELPENC_VERSION_LOWER,
        })
    }

    fn open_inner(
        &mut self,
        variant: OpenVariant,
        start_addr: u32,
        total_mem_size: u32,
    ) -> Result<u32, CellError> {
        if start_addr == 0 || total_mem_size < CELL_CELPENC_WORK_MEM_SIZE {
            return Err(CELL_CELPENC_ERROR_ARG);
        }
        if self.handles.len() >= Self::MAX_HANDLES {
            return Err(CELL_CELPENC_ERROR_FAILED);
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.handles.push(EncoderHandle {
            id,
            state: EncoderState::Opened,
            variant,
            param: None,
            pending_aus: VecDeque::new(),
            encode_frame_count: 0,
            wait_calls: 0,
        });
        Ok(id)
    }

    /// `cellCelpEncOpen` (cpp:33-37). Allocates a handle tied to
    /// a `CellCelpEncResource`.
    pub fn open(&mut self, res: &CellCelpEncResource) -> Result<u32, CellError> {
        self.open_calls = self.open_calls.saturating_add(1);
        self.open_inner(OpenVariant::Open, res.start_addr, res.total_mem_size)
    }

    /// `cellCelpEncOpenEx` (cpp:39-43).
    pub fn open_ex(&mut self, res: &CellCelpEncResourceEx) -> Result<u32, CellError> {
        self.open_ex_calls = self.open_ex_calls.saturating_add(1);
        self.open_inner(OpenVariant::OpenEx, res.start_addr, res.total_mem_size)
    }

    /// `cellCelpEncOpenExt` (cpp:45-49). The C++ stub takes no
    /// arguments; the port commits minimal defaults (work mem is
    /// allocated internally by the SDK in this variant).
    pub fn open_ext(&mut self) -> Result<u32, CellError> {
        self.open_ext_calls = self.open_ext_calls.saturating_add(1);
        self.open_inner(OpenVariant::OpenExt, 0x1_0000, CELL_CELPENC_WORK_MEM_SIZE)
    }

    /// `cellCelpEncClose` (cpp:51-55). Accepts any state (Opened,
    /// Started, Ended) — the firmware allows emergency close.
    pub fn close(&mut self, handle_id: u32) -> Result<(), CellError> {
        let pos = self
            .handles
            .iter()
            .position(|h| h.id == handle_id)
            .ok_or(CELL_CELPENC_ERROR_ARG)?;
        self.handles.swap_remove(pos);
        self.close_calls = self.close_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelpEncStart` (cpp:57-61). Validates every param field
    /// against the firmware enum values before transitioning
    /// `Opened → Started`. A second `Start` on an already-running
    /// handle returns `SEQ`.
    pub fn start(
        &mut self,
        handle_id: u32,
        param: &CellCelpEncParam,
    ) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Opened {
            return Err(CELL_CELPENC_ERROR_SEQ);
        }
        if !is_valid_excitation_mode(param.excitation_mode)
            || !is_valid_sample_rate(param.sample_rate)
            || !is_valid_rpe_config(param.configuration)
            || !is_valid_word_size(param.word_size)
        {
            return Err(CELL_CELPENC_ERROR_ARG);
        }
        if param.out_buff == 0 || param.out_size == 0 {
            return Err(CELL_CELPENC_ERROR_ARG);
        }
        h.state = EncoderState::Started;
        h.param = Some(*param);
        self.start_calls = self.start_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelpEncEnd` (cpp:63-67). `Started → Ended`. Accepts
    /// Started only.
    pub fn end(&mut self, handle_id: u32) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Started {
            return Err(CELL_CELPENC_ERROR_SEQ);
        }
        h.state = EncoderState::Ended;
        self.end_calls = self.end_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelpEncEncodeFrame` (cpp:69-73). Requires `Started` state
    /// and a non-empty PCM frame.
    pub fn encode_frame(
        &mut self,
        handle_id: u32,
        frame: &CellCelpEncPcmInfo,
    ) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Started {
            return Err(CELL_CELPENC_ERROR_SEQ);
        }
        if frame.start_addr == 0 || frame.size == 0 {
            return Err(CELL_CELPENC_ERROR_ARG);
        }
        h.encode_frame_count = h.encode_frame_count.saturating_add(1);
        self.encode_frame_calls = self.encode_frame_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelpEncWaitForOutput` (cpp:75-79). Requires at least
    /// one encoded frame queued — returns `SEQ` otherwise so tests
    /// that race ahead of the SPU encoder see a clear error.
    pub fn wait_for_output(&mut self, handle_id: u32) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Started {
            return Err(CELL_CELPENC_ERROR_SEQ);
        }
        if h.pending_aus.is_empty() {
            return Err(CELL_CELPENC_ERROR_SEQ);
        }
        h.wait_calls = h.wait_calls.saturating_add(1);
        self.wait_for_output_calls = self.wait_for_output_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelpEncGetAu` (cpp:81-85). Drains the next queued AU into
    /// `out_buffer` (up to its capacity) and populates `au_item`.
    pub fn get_au(
        &mut self,
        handle_id: u32,
        out_buffer: &mut [u8],
    ) -> Result<CellCelpEncAuInfo, CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Started {
            return Err(CELL_CELPENC_ERROR_SEQ);
        }
        let bytes = h
            .pending_aus
            .pop_front()
            .ok_or(CELL_CELPENC_ERROR_SEQ)?;
        if out_buffer.len() < bytes.len() {
            // Put it back so tests can resize + retry.
            h.pending_aus.push_front(bytes);
            return Err(CELL_CELPENC_ERROR_ARG);
        }
        out_buffer[..bytes.len()].copy_from_slice(&bytes);
        let info = CellCelpEncAuInfo {
            start_addr: 0,
            size: bytes.len() as u32,
        };
        self.get_au_calls = self.get_au_calls.saturating_add(1);
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_resource() -> CellCelpEncResource {
        CellCelpEncResource {
            total_mem_size: CELL_CELPENC_WORK_MEM_SIZE + 0x10000,
            start_addr: 0x4000_0000,
            ppu_thread_priority: 300,
            spu_thread_priority: 100,
            ppu_thread_stack_size: 0x10000,
        }
    }

    fn sample_param() -> CellCelpEncParam {
        CellCelpEncParam {
            excitation_mode: CELL_CELPENC_EXCITATION_MODE_RPE,
            sample_rate: CELL_CELPENC_FS_16KHZ,
            configuration: CELL_CELPENC_RPE_CONFIG_2,
            word_size: CELL_CELPENC_WORD_SZ_INT16_LE,
            out_buff: 0x5000_0000,
            out_size: 0x2000,
        }
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellCelpEnc");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 10);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellCelpEncQueryAttr");
        assert_eq!(REGISTERED_ENTRY_POINTS[9], "cellCelpEncGetAu");
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_CELPENC_ERROR_FAILED.0, 0x8061_4001);
        assert_eq!(CELL_CELPENC_ERROR_SEQ.0, 0x8061_4002);
        assert_eq!(CELL_CELPENC_ERROR_ARG.0, 0x8061_4003);
        assert_eq!(CELL_CELPENC_ERROR_CORE_FAILED.0, 0x8061_4081);
        assert_eq!(CELL_CELPENC_ERROR_CORE_SEQ.0, 0x8061_4082);
        assert_eq!(CELL_CELPENC_ERROR_CORE_ARG.0, 0x8061_4083);
    }

    #[test]
    fn config_enum_values_byte_exact() {
        assert_eq!(CELL_CELPENC_RPE_CONFIG_0, 0);
        assert_eq!(CELL_CELPENC_RPE_CONFIG_1, 1);
        assert_eq!(CELL_CELPENC_RPE_CONFIG_2, 2);
        assert_eq!(CELL_CELPENC_RPE_CONFIG_3, 3);
        assert_eq!(CELL_CELPENC_FS_16KHZ, 2);
        assert_eq!(CELL_CELPENC_EXCITATION_MODE_RPE, 1);
        assert_eq!(CELL_CELPENC_WORD_SZ_INT16_LE, 0);
        assert_eq!(CELL_CELPENC_WORD_SZ_FLOAT, 1);
    }

    #[test]
    fn validators_boundary_checks() {
        assert!(is_valid_rpe_config(0));
        assert!(is_valid_rpe_config(3));
        assert!(!is_valid_rpe_config(4));
        assert!(is_valid_excitation_mode(1));
        assert!(!is_valid_excitation_mode(0));
        assert!(is_valid_sample_rate(2));
        assert!(!is_valid_sample_rate(1));
        assert!(is_valid_word_size(0));
        assert!(is_valid_word_size(1));
        assert!(!is_valid_word_size(2));
    }

    #[test]
    fn query_attr_returns_version_and_mem() {
        let mut e = CelpEnc::new();
        let a = e.query_attr().unwrap();
        assert_eq!(a.work_mem_size, CELL_CELPENC_WORK_MEM_SIZE);
        assert_eq!(a.celp_enc_ver_upper, CELL_CELPENC_VERSION_UPPER);
        assert_eq!(a.celp_enc_ver_lower, CELL_CELPENC_VERSION_LOWER);
    }

    #[test]
    fn open_happy_path() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        assert_eq!(h, 1);
        assert_eq!(e.handle_count(), 1);
    }

    #[test]
    fn open_null_start_addr_is_arg() {
        let mut e = CelpEnc::new();
        let mut res = sample_resource();
        res.start_addr = 0;
        assert_eq!(e.open(&res), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn open_undersized_mem_is_arg() {
        let mut e = CelpEnc::new();
        let mut res = sample_resource();
        res.total_mem_size = CELL_CELPENC_WORK_MEM_SIZE - 1;
        assert_eq!(e.open(&res), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn open_max_handles_exhausted_is_failed() {
        let mut e = CelpEnc::new();
        for _ in 0..CelpEnc::MAX_HANDLES {
            e.open(&sample_resource()).unwrap();
        }
        assert_eq!(
            e.open(&sample_resource()),
            Err(CELL_CELPENC_ERROR_FAILED)
        );
    }

    #[test]
    fn open_ex_and_open_ext_allocate() {
        let mut e = CelpEnc::new();
        let rex = CellCelpEncResourceEx {
            total_mem_size: CELL_CELPENC_WORK_MEM_SIZE + 0x1000,
            start_addr: 0x1000_0000,
            spurs: 0,
            priority: [16; 8],
            max_contention: 1,
        };
        let h1 = e.open_ex(&rex).unwrap();
        let h2 = e.open_ext().unwrap();
        assert_ne!(h1, h2);
        assert_eq!(e.handle_count(), 2);
    }

    #[test]
    fn close_unknown_handle_is_arg() {
        let mut e = CelpEnc::new();
        assert_eq!(e.close(99), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn close_removes_handle() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.close(h).unwrap();
        assert_eq!(e.handle_count(), 0);
    }

    #[test]
    fn start_bad_excitation_is_arg() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        let mut p = sample_param();
        p.excitation_mode = 99;
        assert_eq!(e.start(h, &p), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn start_bad_sample_rate_is_arg() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        let mut p = sample_param();
        p.sample_rate = 1;
        assert_eq!(e.start(h, &p), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn start_bad_rpe_config_is_arg() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        let mut p = sample_param();
        p.configuration = 4;
        assert_eq!(e.start(h, &p), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn start_bad_word_size_is_arg() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        let mut p = sample_param();
        p.word_size = 2;
        assert_eq!(e.start(h, &p), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn start_null_out_buffer_is_arg() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        let mut p = sample_param();
        p.out_buff = 0;
        assert_eq!(e.start(h, &p), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn start_twice_is_seq() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        assert_eq!(e.start(h, &sample_param()), Err(CELL_CELPENC_ERROR_SEQ));
    }

    #[test]
    fn encode_frame_without_start_is_seq() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        let frame = CellCelpEncPcmInfo {
            start_addr: 0x2000_0000,
            size: 320,
        };
        assert_eq!(e.encode_frame(h, &frame), Err(CELL_CELPENC_ERROR_SEQ));
    }

    #[test]
    fn encode_frame_zero_size_is_arg() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        let frame = CellCelpEncPcmInfo {
            start_addr: 0x2000_0000,
            size: 0,
        };
        assert_eq!(e.encode_frame(h, &frame), Err(CELL_CELPENC_ERROR_ARG));
    }

    #[test]
    fn encode_frame_counter_increments() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        let frame = CellCelpEncPcmInfo {
            start_addr: 0x2000_0000,
            size: 320,
        };
        for _ in 0..5 {
            e.encode_frame(h, &frame).unwrap();
        }
        assert_eq!(e.encode_frame_calls(), 5);
    }

    #[test]
    fn wait_for_output_requires_started_and_queue() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        // Not started yet.
        assert_eq!(e.wait_for_output(h), Err(CELL_CELPENC_ERROR_SEQ));
        e.start(h, &sample_param()).unwrap();
        // Started but empty.
        assert_eq!(e.wait_for_output(h), Err(CELL_CELPENC_ERROR_SEQ));
        e.inject_au(h, alloc::vec![0xAA; 16]).unwrap();
        e.wait_for_output(h).unwrap();
    }

    #[test]
    fn get_au_drains_queue() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        e.inject_au(h, alloc::vec![0x11, 0x22, 0x33]).unwrap();
        let mut buf = [0u8; 8];
        let info = e.get_au(h, &mut buf).unwrap();
        assert_eq!(info.size, 3);
        assert_eq!(&buf[..3], &[0x11, 0x22, 0x33]);
    }

    #[test]
    fn get_au_small_buffer_returns_arg_and_keeps_queue() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        e.inject_au(h, alloc::vec![0u8; 16]).unwrap();
        let mut tiny = [0u8; 4];
        assert_eq!(e.get_au(h, &mut tiny), Err(CELL_CELPENC_ERROR_ARG));
        // Queue preserved — retry with bigger buffer succeeds.
        let mut buf = [0u8; 16];
        let info = e.get_au(h, &mut buf).unwrap();
        assert_eq!(info.size, 16);
    }

    #[test]
    fn end_requires_started() {
        let mut e = CelpEnc::new();
        let h = e.open(&sample_resource()).unwrap();
        assert_eq!(e.end(h), Err(CELL_CELPENC_ERROR_SEQ));
        e.start(h, &sample_param()).unwrap();
        e.end(h).unwrap();
        // Can't encode after end.
        let frame = CellCelpEncPcmInfo {
            start_addr: 1,
            size: 1,
        };
        assert_eq!(e.encode_frame(h, &frame), Err(CELL_CELPENC_ERROR_SEQ));
    }

    #[test]
    fn full_celpenc_lifecycle_smoke() {
        let mut e = CelpEnc::new();
        let attr = e.query_attr().unwrap();
        assert_eq!(attr.work_mem_size, CELL_CELPENC_WORK_MEM_SIZE);

        // Open + start with a valid param.
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();

        // Encode 3 frames + inject 3 encoded AUs.
        for i in 0..3 {
            let frame = CellCelpEncPcmInfo {
                start_addr: 0x2000_0000 + i * 0x100,
                size: 320,
            };
            e.encode_frame(h, &frame).unwrap();
            e.inject_au(h, alloc::vec![i as u8; 32]).unwrap();
        }

        // Drain 3 outputs.
        for _ in 0..3 {
            e.wait_for_output(h).unwrap();
            let mut out = [0u8; 64];
            let info = e.get_au(h, &mut out).unwrap();
            assert_eq!(info.size, 32);
        }

        // End + close.
        e.end(h).unwrap();
        e.close(h).unwrap();
        assert_eq!(e.handle_count(), 0);

        // Counter trace.
        assert_eq!(e.query_attr_calls(), 1);
        assert_eq!(e.open_calls(), 1);
        assert_eq!(e.start_calls(), 1);
        assert_eq!(e.encode_frame_calls(), 3);
        assert_eq!(e.wait_for_output_calls(), 3);
        assert_eq!(e.get_au_calls(), 3);
        assert_eq!(e.end_calls(), 1);
        assert_eq!(e.close_calls(), 1);
    }
}
