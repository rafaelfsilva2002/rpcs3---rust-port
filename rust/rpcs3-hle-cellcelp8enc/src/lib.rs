//! Rust port of `rpcs3/Emu/Cell/Modules/cellCelp8Enc.cpp`.
//!
//! 9 PRX entries under the module name `cellCelp8Enc` — the 8 kHz MPE
//! companion to the 16 kHz RPE `cellCelpEnc`. Both share the same
//! facility prefix `0x8061_40__` but use disjoint sub-ranges:
//!
//!  - `cellCelpEnc` → `0x8061_4001..0083`.
//!  - `cellCelp8Enc` → `0x8061_40A1..40B3`.
//!
//! Distinct from `cellCelpEnc`:
//!
//!  - 9 entries (no `OpenExt`).
//!  - Valid `configuration` set is a non-contiguous list of 10 MPE
//!    configurations (`0`, `2`, `6`, `9`, `12`, `15`, `18`, `21`, `24`,
//!    `26`).
//!  - Single sample rate `FS_8kHz = 1`.
//!  - Single excitation mode `MPE = 0`.
//!  - Single word-size option `FLOAT = 0` (no `INT16_LE`).
//!
//! REG_FUNC order at cpp:83-91.
//!
//! Module name byte-exact at cpp:6 / cpp:81.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:6 / cpp:81.
pub const MODULE_NAME: &str = "cellCelp8Enc";

/// REG_FUNC order at cpp:83-91.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellCelp8EncQueryAttr",
    "cellCelp8EncOpen",
    "cellCelp8EncOpenEx",
    "cellCelp8EncClose",
    "cellCelp8EncStart",
    "cellCelp8EncEnd",
    "cellCelp8EncEncodeFrame",
    "cellCelp8EncWaitForOutput",
    "cellCelp8EncGetAu",
];

// --- Error codes (byte-exact cellCelp8Enc.h:10-18) ----------------------

pub const CELL_CELP8ENC_ERROR_FAILED: CellError = CellError(0x8061_40A1);
pub const CELL_CELP8ENC_ERROR_SEQ: CellError = CellError(0x8061_40A2);
pub const CELL_CELP8ENC_ERROR_ARG: CellError = CellError(0x8061_40A3);
pub const CELL_CELP8ENC_ERROR_CORE_FAILED: CellError = CellError(0x8061_40B1);
pub const CELL_CELP8ENC_ERROR_CORE_SEQ: CellError = CellError(0x8061_40B2);
pub const CELL_CELP8ENC_ERROR_CORE_ARG: CellError = CellError(0x8061_40B3);

// --- Configuration enums (cellCelp8Enc.h:20-48) -------------------------

/// Valid MPE configuration values, in the order declared at
/// cellCelp8Enc.h:21-33. The values are **non-contiguous** — tests
/// should treat this array as the canonical whitelist.
pub const CELL_CELP8ENC_MPE_CONFIGS: &[u32] = &[0, 2, 6, 9, 12, 15, 18, 21, 24, 26];

pub const CELL_CELP8ENC_FS_8KHZ: u32 = 1;
pub const CELL_CELP8ENC_EXCITATION_MODE_MPE: u32 = 0;
pub const CELL_CELP8ENC_WORD_SZ_FLOAT: u32 = 0;

pub const CELL_CELP8ENC_VERSION_UPPER: u32 = 0x0001_0000;
pub const CELL_CELP8ENC_VERSION_LOWER: u32 = 0x0000_0000;

/// Required work memory — 1 MiB, smaller than `cellCelpEnc` (2 MiB)
/// because the 8 kHz pipeline has a narrower frame size.
pub const CELL_CELP8ENC_WORK_MEM_SIZE: u32 = 1024 * 1024;

// --- Wire structs (byte-exact cellCelp8Enc.h:50-95) ---------------------

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelp8EncAttr {
    pub work_mem_size: u32,
    pub celp_enc_ver_upper: u32,
    pub celp_enc_ver_lower: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelp8EncResource {
    pub total_mem_size: u32,
    pub start_addr: u32,
    pub ppu_thread_priority: u32,
    pub spu_thread_priority: u32,
    pub ppu_thread_stack_size: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelp8EncParam {
    pub excitation_mode: u32,
    pub sample_rate: u32,
    pub configuration: u32,
    pub word_size: u32,
    pub out_buff: u32,
    pub out_size: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelp8EncAuInfo {
    pub start_addr: u32,
    pub size: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelp8EncPcmInfo {
    pub start_addr: u32,
    pub size: u32,
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellCelp8EncResourceEx {
    pub total_mem_size: u32,
    pub start_addr: u32,
    pub spurs: u32,
    pub priority: [u8; 8],
    pub max_contention: u32,
}

// --- Validators ---------------------------------------------------------

/// `configuration ∈ {0, 2, 6, 9, 12, 15, 18, 21, 24, 26}`.
#[must_use]
pub fn is_valid_mpe_config(c: u32) -> bool {
    CELL_CELP8ENC_MPE_CONFIGS.iter().any(|&v| v == c)
}

/// Only `MPE = 0` is accepted.
#[must_use]
pub const fn is_valid_excitation_mode(m: u32) -> bool {
    m == CELL_CELP8ENC_EXCITATION_MODE_MPE
}

/// Only `FS_8KHZ = 1` is accepted.
#[must_use]
pub const fn is_valid_sample_rate(r: u32) -> bool {
    r == CELL_CELP8ENC_FS_8KHZ
}

/// Only `FLOAT = 0` is accepted.
#[must_use]
pub const fn is_valid_word_size(w: u32) -> bool {
    w == CELL_CELP8ENC_WORD_SZ_FLOAT
}

// --- FSM + handle model -------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderState {
    Opened,
    Started,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenVariant {
    Open,
    OpenEx,
}

#[derive(Debug, Clone)]
pub struct EncoderHandle {
    pub id: u32,
    pub state: EncoderState,
    pub variant: OpenVariant,
    pub param: Option<CellCelp8EncParam>,
    pub pending_aus: VecDeque<Vec<u8>>,
    pub encode_frame_count: u32,
}

#[derive(Debug)]
pub struct Celp8Enc {
    handles: Vec<EncoderHandle>,
    next_id: u32,
    query_attr_calls: u32,
    open_calls: u32,
    open_ex_calls: u32,
    close_calls: u32,
    start_calls: u32,
    end_calls: u32,
    encode_frame_calls: u32,
    wait_for_output_calls: u32,
    get_au_calls: u32,
}

impl Default for Celp8Enc {
    fn default() -> Self {
        Self::new()
    }
}

impl Celp8Enc {
    pub const MAX_HANDLES: usize = 8;

    #[must_use]
    pub const fn new() -> Self {
        Self {
            handles: Vec::new(),
            next_id: 1,
            query_attr_calls: 0,
            open_calls: 0,
            open_ex_calls: 0,
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

    fn handle_mut(&mut self, id: u32) -> Result<&mut EncoderHandle, CellError> {
        self.handles
            .iter_mut()
            .find(|h| h.id == id)
            .ok_or(CELL_CELP8ENC_ERROR_ARG)
    }

    /// Test hook — inject an encoded AU onto the handle's output queue.
    pub fn inject_au(&mut self, handle_id: u32, bytes: Vec<u8>) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        h.pending_aus.push_back(bytes);
        Ok(())
    }

    /// `cellCelp8EncQueryAttr` (cpp:27-31).
    pub fn query_attr(&mut self) -> Result<CellCelp8EncAttr, CellError> {
        self.query_attr_calls = self.query_attr_calls.saturating_add(1);
        Ok(CellCelp8EncAttr {
            work_mem_size: CELL_CELP8ENC_WORK_MEM_SIZE,
            celp_enc_ver_upper: CELL_CELP8ENC_VERSION_UPPER,
            celp_enc_ver_lower: CELL_CELP8ENC_VERSION_LOWER,
        })
    }

    fn open_inner(
        &mut self,
        variant: OpenVariant,
        start_addr: u32,
        total_mem_size: u32,
    ) -> Result<u32, CellError> {
        if start_addr == 0 || total_mem_size < CELL_CELP8ENC_WORK_MEM_SIZE {
            return Err(CELL_CELP8ENC_ERROR_ARG);
        }
        if self.handles.len() >= Self::MAX_HANDLES {
            return Err(CELL_CELP8ENC_ERROR_FAILED);
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
        });
        Ok(id)
    }

    /// `cellCelp8EncOpen` (cpp:33-37).
    pub fn open(&mut self, res: &CellCelp8EncResource) -> Result<u32, CellError> {
        self.open_calls = self.open_calls.saturating_add(1);
        self.open_inner(OpenVariant::Open, res.start_addr, res.total_mem_size)
    }

    /// `cellCelp8EncOpenEx` (cpp:39-43).
    pub fn open_ex(&mut self, res: &CellCelp8EncResource) -> Result<u32, CellError> {
        self.open_ex_calls = self.open_ex_calls.saturating_add(1);
        self.open_inner(OpenVariant::OpenEx, res.start_addr, res.total_mem_size)
    }

    /// `cellCelp8EncClose` (cpp:45-49).
    pub fn close(&mut self, handle_id: u32) -> Result<(), CellError> {
        let pos = self
            .handles
            .iter()
            .position(|h| h.id == handle_id)
            .ok_or(CELL_CELP8ENC_ERROR_ARG)?;
        self.handles.swap_remove(pos);
        self.close_calls = self.close_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelp8EncStart` (cpp:51-55).
    pub fn start(
        &mut self,
        handle_id: u32,
        param: &CellCelp8EncParam,
    ) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Opened {
            return Err(CELL_CELP8ENC_ERROR_SEQ);
        }
        if !is_valid_excitation_mode(param.excitation_mode)
            || !is_valid_sample_rate(param.sample_rate)
            || !is_valid_mpe_config(param.configuration)
            || !is_valid_word_size(param.word_size)
        {
            return Err(CELL_CELP8ENC_ERROR_ARG);
        }
        if param.out_buff == 0 || param.out_size == 0 {
            return Err(CELL_CELP8ENC_ERROR_ARG);
        }
        h.state = EncoderState::Started;
        h.param = Some(*param);
        self.start_calls = self.start_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelp8EncEnd` (cpp:57-61).
    pub fn end(&mut self, handle_id: u32) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Started {
            return Err(CELL_CELP8ENC_ERROR_SEQ);
        }
        h.state = EncoderState::Ended;
        self.end_calls = self.end_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelp8EncEncodeFrame` (cpp:63-67).
    pub fn encode_frame(
        &mut self,
        handle_id: u32,
        frame: &CellCelp8EncPcmInfo,
    ) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Started {
            return Err(CELL_CELP8ENC_ERROR_SEQ);
        }
        if frame.start_addr == 0 || frame.size == 0 {
            return Err(CELL_CELP8ENC_ERROR_ARG);
        }
        h.encode_frame_count = h.encode_frame_count.saturating_add(1);
        self.encode_frame_calls = self.encode_frame_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelp8EncWaitForOutput` (cpp:69-73).
    pub fn wait_for_output(&mut self, handle_id: u32) -> Result<(), CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Started {
            return Err(CELL_CELP8ENC_ERROR_SEQ);
        }
        if h.pending_aus.is_empty() {
            return Err(CELL_CELP8ENC_ERROR_SEQ);
        }
        self.wait_for_output_calls = self.wait_for_output_calls.saturating_add(1);
        Ok(())
    }

    /// `cellCelp8EncGetAu` (cpp:75-79).
    pub fn get_au(
        &mut self,
        handle_id: u32,
        out_buffer: &mut [u8],
    ) -> Result<CellCelp8EncAuInfo, CellError> {
        let h = self.handle_mut(handle_id)?;
        if h.state != EncoderState::Started {
            return Err(CELL_CELP8ENC_ERROR_SEQ);
        }
        let bytes = h
            .pending_aus
            .pop_front()
            .ok_or(CELL_CELP8ENC_ERROR_SEQ)?;
        if out_buffer.len() < bytes.len() {
            h.pending_aus.push_front(bytes);
            return Err(CELL_CELP8ENC_ERROR_ARG);
        }
        out_buffer[..bytes.len()].copy_from_slice(&bytes);
        let info = CellCelp8EncAuInfo {
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

    fn sample_resource() -> CellCelp8EncResource {
        CellCelp8EncResource {
            total_mem_size: CELL_CELP8ENC_WORK_MEM_SIZE + 0x10000,
            start_addr: 0x4000_0000,
            ppu_thread_priority: 300,
            spu_thread_priority: 100,
            ppu_thread_stack_size: 0x10000,
        }
    }

    fn sample_param() -> CellCelp8EncParam {
        CellCelp8EncParam {
            excitation_mode: CELL_CELP8ENC_EXCITATION_MODE_MPE,
            sample_rate: CELL_CELP8ENC_FS_8KHZ,
            configuration: 9,
            word_size: CELL_CELP8ENC_WORD_SZ_FLOAT,
            out_buff: 0x5000_0000,
            out_size: 0x1000,
        }
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellCelp8Enc");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 9);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellCelp8EncQueryAttr");
        assert_eq!(REGISTERED_ENTRY_POINTS[8], "cellCelp8EncGetAu");
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_CELP8ENC_ERROR_FAILED.0, 0x8061_40A1);
        assert_eq!(CELL_CELP8ENC_ERROR_SEQ.0, 0x8061_40A2);
        assert_eq!(CELL_CELP8ENC_ERROR_ARG.0, 0x8061_40A3);
        assert_eq!(CELL_CELP8ENC_ERROR_CORE_FAILED.0, 0x8061_40B1);
        assert_eq!(CELL_CELP8ENC_ERROR_CORE_SEQ.0, 0x8061_40B2);
        assert_eq!(CELL_CELP8ENC_ERROR_CORE_ARG.0, 0x8061_40B3);
    }

    #[test]
    fn mpe_configs_non_contiguous_whitelist() {
        assert_eq!(CELL_CELP8ENC_MPE_CONFIGS.len(), 10);
        // Exact values from cellCelp8Enc.h:23-32.
        assert_eq!(
            CELL_CELP8ENC_MPE_CONFIGS,
            &[0, 2, 6, 9, 12, 15, 18, 21, 24, 26]
        );
        // Verify non-contiguity at the obvious gap points.
        assert!(!is_valid_mpe_config(1));
        assert!(!is_valid_mpe_config(3));
        assert!(!is_valid_mpe_config(4));
        assert!(!is_valid_mpe_config(5));
        assert!(!is_valid_mpe_config(7));
        assert!(!is_valid_mpe_config(25));
        assert!(!is_valid_mpe_config(27));
    }

    #[test]
    fn mpe_configs_accept_whitelist() {
        for &c in CELL_CELP8ENC_MPE_CONFIGS {
            assert!(is_valid_mpe_config(c));
        }
    }

    #[test]
    fn distinct_facility_range_from_celpenc() {
        // cellCelpEnc occupies 0x8061_4001..4003 / 4081..4083.
        // cellCelp8Enc occupies 0x8061_40A1..40A3 / 40B1..40B3.
        // No overlap — verify the boundary.
        assert!(CELL_CELP8ENC_ERROR_FAILED.0 > 0x8061_4083);
        assert!(CELL_CELP8ENC_ERROR_FAILED.0 < 0x8061_40B1);
    }

    #[test]
    fn word_size_accepts_only_float() {
        assert!(is_valid_word_size(0));
        assert!(!is_valid_word_size(1));
        assert!(!is_valid_word_size(2));
    }

    #[test]
    fn sample_rate_accepts_only_8khz() {
        assert!(is_valid_sample_rate(1));
        assert!(!is_valid_sample_rate(0));
        assert!(!is_valid_sample_rate(2));
    }

    #[test]
    fn excitation_accepts_only_mpe() {
        assert!(is_valid_excitation_mode(0));
        assert!(!is_valid_excitation_mode(1));
    }

    #[test]
    fn query_attr_returns_expected() {
        let mut e = Celp8Enc::new();
        let a = e.query_attr().unwrap();
        assert_eq!(a.work_mem_size, CELL_CELP8ENC_WORK_MEM_SIZE);
        assert_eq!(a.celp_enc_ver_upper, CELL_CELP8ENC_VERSION_UPPER);
    }

    #[test]
    fn open_happy_path() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        assert_eq!(h, 1);
    }

    #[test]
    fn open_null_start_addr_is_arg() {
        let mut e = Celp8Enc::new();
        let mut r = sample_resource();
        r.start_addr = 0;
        assert_eq!(e.open(&r), Err(CELL_CELP8ENC_ERROR_ARG));
    }

    #[test]
    fn open_undersized_mem_is_arg() {
        let mut e = Celp8Enc::new();
        let mut r = sample_resource();
        r.total_mem_size = CELL_CELP8ENC_WORK_MEM_SIZE - 1;
        assert_eq!(e.open(&r), Err(CELL_CELP8ENC_ERROR_ARG));
    }

    #[test]
    fn open_max_handles_exhausted_is_failed() {
        let mut e = Celp8Enc::new();
        for _ in 0..Celp8Enc::MAX_HANDLES {
            e.open(&sample_resource()).unwrap();
        }
        assert_eq!(
            e.open(&sample_resource()),
            Err(CELL_CELP8ENC_ERROR_FAILED)
        );
    }

    #[test]
    fn open_ex_tracks_variant() {
        let mut e = Celp8Enc::new();
        let _ = e.open_ex(&sample_resource()).unwrap();
        assert_eq!(e.open_ex_calls(), 1);
    }

    #[test]
    fn close_unknown_handle_is_arg() {
        let mut e = Celp8Enc::new();
        assert_eq!(e.close(99), Err(CELL_CELP8ENC_ERROR_ARG));
    }

    #[test]
    fn start_bad_excitation_is_arg() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        let mut p = sample_param();
        p.excitation_mode = 1; // RPE — valid for cellCelpEnc but not cellCelp8Enc.
        assert_eq!(e.start(h, &p), Err(CELL_CELP8ENC_ERROR_ARG));
    }

    #[test]
    fn start_bad_sample_rate_is_arg() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        let mut p = sample_param();
        p.sample_rate = 2; // 16 kHz — rejected by cellCelp8Enc.
        assert_eq!(e.start(h, &p), Err(CELL_CELP8ENC_ERROR_ARG));
    }

    #[test]
    fn start_rejects_non_whitelisted_config() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        for bad in [1, 3, 5, 7, 10, 11, 25, 27, 100] {
            let mut p = sample_param();
            p.configuration = bad;
            assert_eq!(
                e.start(h, &p),
                Err(CELL_CELP8ENC_ERROR_ARG),
                "bad config {} should be rejected",
                bad
            );
        }
    }

    #[test]
    fn start_accepts_every_whitelisted_config() {
        for &c in CELL_CELP8ENC_MPE_CONFIGS {
            let mut e = Celp8Enc::new();
            let h = e.open(&sample_resource()).unwrap();
            let mut p = sample_param();
            p.configuration = c;
            e.start(h, &p).unwrap();
        }
    }

    #[test]
    fn start_twice_is_seq() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        assert_eq!(
            e.start(h, &sample_param()),
            Err(CELL_CELP8ENC_ERROR_SEQ)
        );
    }

    #[test]
    fn end_requires_started() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        assert_eq!(e.end(h), Err(CELL_CELP8ENC_ERROR_SEQ));
    }

    #[test]
    fn encode_frame_flow() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        let frame = CellCelp8EncPcmInfo {
            start_addr: 0x2000_0000,
            size: 160,
        };
        e.encode_frame(h, &frame).unwrap();
        assert_eq!(e.encode_frame_calls(), 1);
    }

    #[test]
    fn get_au_drains_queue() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        e.inject_au(h, alloc::vec![0xCA, 0xFE, 0xBA]).unwrap();
        let mut buf = [0u8; 8];
        let info = e.get_au(h, &mut buf).unwrap();
        assert_eq!(info.size, 3);
        assert_eq!(&buf[..3], &[0xCA, 0xFE, 0xBA]);
    }

    #[test]
    fn get_au_small_buffer_preserves_queue() {
        let mut e = Celp8Enc::new();
        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();
        e.inject_au(h, alloc::vec![0u8; 32]).unwrap();
        let mut tiny = [0u8; 4];
        assert_eq!(e.get_au(h, &mut tiny), Err(CELL_CELP8ENC_ERROR_ARG));
        let mut big = [0u8; 64];
        let info = e.get_au(h, &mut big).unwrap();
        assert_eq!(info.size, 32);
    }

    #[test]
    fn full_celp8enc_lifecycle_smoke() {
        let mut e = Celp8Enc::new();
        let attr = e.query_attr().unwrap();
        assert_eq!(attr.work_mem_size, CELL_CELP8ENC_WORK_MEM_SIZE);

        let h = e.open(&sample_resource()).unwrap();
        e.start(h, &sample_param()).unwrap();

        // Encode 2 frames; inject 2 AUs.
        for i in 0..2 {
            let frame = CellCelp8EncPcmInfo {
                start_addr: 0x2000_0000 + i * 0x100,
                size: 160,
            };
            e.encode_frame(h, &frame).unwrap();
            e.inject_au(h, alloc::vec![i as u8; 16]).unwrap();
        }

        for _ in 0..2 {
            e.wait_for_output(h).unwrap();
            let mut out = [0u8; 32];
            let info = e.get_au(h, &mut out).unwrap();
            assert_eq!(info.size, 16);
        }

        e.end(h).unwrap();
        e.close(h).unwrap();
        assert_eq!(e.handle_count(), 0);
        assert_eq!(e.encode_frame_calls(), 2);
    }
}
