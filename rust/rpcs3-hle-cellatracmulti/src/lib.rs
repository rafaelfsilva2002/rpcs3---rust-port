//! Rust port of `rpcs3/Emu/Cell/Modules/cellAtracMulti.cpp` — PS3 ATRAC
//! multi-track audio decoder HLE surface.
//!
//! Upstream ships 24 entries on the `cellAtracMulti` PRX. Most are stubs that
//! write constants into out-pointers:
//!
//! * `SetDataAndGetMemSize` → `*puiWorkMemByte = 0x1000`
//! * `CreateDecoder/Ext` → `memcpy(pHandle->ucWorkMem, pucWorkMem, 512)`
//! * `Decode` → `samples=0`, `finishFlag=1`, `remainFrame=ALLDATA_IS_ON_MEMORY`
//! * `GetStreamDataInfo` → `writePointer = handle.addr()`, writable=0x1000, readPos=0
//! * `GetRemainFrame` → `-1 (ALLDATA_IS_ON_MEMORY)`
//! * `GetVacantSize` → `0x1000`
//! * `IsSecondBufferNeeded` → `0 (false, via not_an_error)`
//! * `GetSecondBufferInfo` → `(0, 0)`
//! * `GetChannel` → `2`
//! * `GetMaxSample` → `512`
//! * `GetNextSample` → `0`
//! * `GetSoundInfo` → `(0, 0, 0)` (end/loopStart/loopEnd)
//! * `GetNextDecodePosition` → writes `0` **AND returns
//!   `CELL_ATRACMULTI_ERROR_ALLDATA_WAS_DECODED`** (the only entry that
//!   actively returns an error on every call — upstream peculiarity preserved).
//! * `GetBitrate` → `128`
//! * `GetBufferInfoForResetting` → writeAddr=handle, writable=0x1000, min=0, read=0
//! * `GetInternalErrorInfo` → `0`
//! * `GetSamplingRate` → `UNIMPLEMENTED_FUNC` (just a stub).
//!
//! The 22 error codes in `0x8061_0B__` are preserved byte-exact across 9
//! sub-ranges (`_B0_`, `_B1_` ... `_B9_`).
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::mem::size_of;

use rpcs3_emu_types::CellError;

/// Upstream PRX name registered by `DECLARE(ppu_module_manager::cellAtracMulti)`.
pub const MODULE_NAME: &str = "cellAtracMulti";

/// 25 FNIDs in exact `REG_FUNC` order (cpp:248-280).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellAtracMultiSetDataAndGetMemSize",
    "cellAtracMultiCreateDecoder",
    "cellAtracMultiCreateDecoderExt",
    "cellAtracMultiDeleteDecoder",
    "cellAtracMultiDecode",
    "cellAtracMultiGetStreamDataInfo",
    "cellAtracMultiAddStreamData",
    "cellAtracMultiGetRemainFrame",
    "cellAtracMultiGetVacantSize",
    "cellAtracMultiIsSecondBufferNeeded",
    "cellAtracMultiGetSecondBufferInfo",
    "cellAtracMultiSetSecondBuffer",
    "cellAtracMultiGetChannel",
    "cellAtracMultiGetMaxSample",
    "cellAtracMultiGetNextSample",
    "cellAtracMultiGetSoundInfo",
    "cellAtracMultiGetNextDecodePosition",
    "cellAtracMultiGetBitrate",
    "cellAtracMultiGetTrackArray",
    "cellAtracMultiGetLoopInfo",
    "cellAtracMultiSetLoopNum",
    "cellAtracMultiGetBufferInfoForResetting",
    "cellAtracMultiResetPlayPosition",
    "cellAtracMultiGetInternalErrorInfo",
    "cellAtracMultiGetSamplingRate",
];

// ---------------------------------------------------------------------------
// Error codes — byte-exact `CellAtracMultiError` (facility 0x8061_0B__, 9 sub-ranges).
// ---------------------------------------------------------------------------

pub const CELL_ATRACMULTI_ERROR_API_FAIL: CellError = CellError(0x8061_0B01);

pub const CELL_ATRACMULTI_ERROR_READSIZE_OVER_BUFFER: CellError = CellError(0x8061_0B11);
pub const CELL_ATRACMULTI_ERROR_UNKNOWN_FORMAT: CellError = CellError(0x8061_0B12);
pub const CELL_ATRACMULTI_ERROR_READSIZE_IS_TOO_SMALL: CellError = CellError(0x8061_0B13);
pub const CELL_ATRACMULTI_ERROR_ILLEGAL_SAMPLING_RATE: CellError = CellError(0x8061_0B14);
pub const CELL_ATRACMULTI_ERROR_ILLEGAL_DATA: CellError = CellError(0x8061_0B15);

pub const CELL_ATRACMULTI_ERROR_NO_DECODER: CellError = CellError(0x8061_0B21);
pub const CELL_ATRACMULTI_ERROR_UNSET_DATA: CellError = CellError(0x8061_0B22);
pub const CELL_ATRACMULTI_ERROR_DECODER_WAS_CREATED: CellError = CellError(0x8061_0B23);

pub const CELL_ATRACMULTI_ERROR_ALLDATA_WAS_DECODED: CellError = CellError(0x8061_0B31);
pub const CELL_ATRACMULTI_ERROR_NODATA_IN_BUFFER: CellError = CellError(0x8061_0B32);
pub const CELL_ATRACMULTI_ERROR_NOT_ALIGNED_OUT_BUFFER: CellError = CellError(0x8061_0B33);
pub const CELL_ATRACMULTI_ERROR_NEED_SECOND_BUFFER: CellError = CellError(0x8061_0B34);

pub const CELL_ATRACMULTI_ERROR_ALLDATA_IS_ONMEMORY: CellError = CellError(0x8061_0B41);
pub const CELL_ATRACMULTI_ERROR_ADD_DATA_IS_TOO_BIG: CellError = CellError(0x8061_0B42);

pub const CELL_ATRACMULTI_ERROR_NONEED_SECOND_BUFFER: CellError = CellError(0x8061_0B51);

pub const CELL_ATRACMULTI_ERROR_UNSET_LOOP_NUM: CellError = CellError(0x8061_0B61);

pub const CELL_ATRACMULTI_ERROR_ILLEGAL_SAMPLE: CellError = CellError(0x8061_0B71);
pub const CELL_ATRACMULTI_ERROR_ILLEGAL_RESET_BYTE: CellError = CellError(0x8061_0B72);

pub const CELL_ATRACMULTI_ERROR_ILLEGAL_PPU_THREAD_PRIORITY: CellError = CellError(0x8061_0B81);
pub const CELL_ATRACMULTI_ERROR_ILLEGAL_SPU_THREAD_PRIORITY: CellError = CellError(0x8061_0B82);

pub const CELL_ATRACMULTI_ERROR_API_PARAMETER: CellError = CellError(0x8061_0B91);

// ---------------------------------------------------------------------------
// Constants & sentinels.
// ---------------------------------------------------------------------------

/// Size of `CellAtracMultiHandle::ucWorkMem` in bytes.
pub const CELL_ATRACMULTI_HANDLE_SIZE: usize = 512;

/// RemainFrame sentinel: all data already sitting in memory (full clip loaded).
pub const CELL_ATRACMULTI_ALLDATA_IS_ON_MEMORY: i32 = -1;
/// RemainFrame sentinel: non-loop stream fully buffered.
pub const CELL_ATRACMULTI_NONLOOP_STREAM_DATA_IS_ON_MEMORY: i32 = -2;
/// RemainFrame sentinel: loop stream fully buffered.
pub const CELL_ATRACMULTI_LOOP_STREAM_DATA_IS_ON_MEMORY: i32 = -3;

/// Observable constants upstream writes on each corresponding entry.
pub const STUB_WORK_MEM_BYTES: u32 = 0x1000;
pub const STUB_WRITABLE_BYTES: u32 = 0x1000;
pub const STUB_VACANT_SIZE: u32 = 0x1000;
pub const STUB_CHANNEL: u32 = 2;
pub const STUB_MAX_SAMPLE: u32 = 512;
pub const STUB_BITRATE_KBPS: u32 = 128;

// ---------------------------------------------------------------------------
// Wire structs.
// ---------------------------------------------------------------------------

#[repr(C, align(8))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellAtracMultiHandle {
    pub uc_work_mem: [u8; CELL_ATRACMULTI_HANDLE_SIZE],
}

impl Default for CellAtracMultiHandle {
    fn default() -> Self {
        Self {
            uc_work_mem: [0; CELL_ATRACMULTI_HANDLE_SIZE],
        }
    }
}

const _: () = assert!(size_of::<CellAtracMultiHandle>() == 512);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellAtracMultiBufferInfo {
    pub puc_write_addr: u32,
    pub ui_writable_byte: u32,
    pub ui_min_write_byte: u32,
    pub ui_read_position: u32,
}
const _: () = assert!(size_of::<CellAtracMultiBufferInfo>() == 16);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellAtracMultiExtRes {
    pub p_spurs: u32,
    pub priority: [u8; 8],
}
const _: () = assert!(size_of::<CellAtracMultiExtRes>() == 12);

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

/// Per-instance state (one per live handle address).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InstanceState {
    /// Set by `SetDataAndGetMemSize` when a buffer is registered.
    pub data_set: bool,
    /// Set by `CreateDecoder*`.
    pub decoder_created: bool,
    /// Set by `SetLoopNum`.
    pub loop_num_set: bool,
    /// Last known loop num, when set.
    pub loop_num: i32,
}

#[derive(Debug, Default)]
pub struct AtracMulti {
    /// Keyed on the address the caller passed as `pHandle`. Mirrors upstream's
    /// implicit per-handle state captured in `ucWorkMem`.
    instances: Vec<(u32, InstanceState)>,

    // Per-entry counters — 24 entries.
    pub set_data_and_get_mem_size_calls: u64,
    pub create_decoder_calls: u64,
    pub create_decoder_ext_calls: u64,
    pub delete_decoder_calls: u64,
    pub decode_calls: u64,
    pub get_stream_data_info_calls: u64,
    pub add_stream_data_calls: u64,
    pub get_remain_frame_calls: u64,
    pub get_vacant_size_calls: u64,
    pub is_second_buffer_needed_calls: u64,
    pub get_second_buffer_info_calls: u64,
    pub set_second_buffer_calls: u64,
    pub get_channel_calls: u64,
    pub get_max_sample_calls: u64,
    pub get_next_sample_calls: u64,
    pub get_sound_info_calls: u64,
    pub get_next_decode_position_calls: u64,
    pub get_bitrate_calls: u64,
    pub get_track_array_calls: u64,
    pub get_loop_info_calls: u64,
    pub set_loop_num_calls: u64,
    pub get_buffer_info_for_resetting_calls: u64,
    pub reset_play_position_calls: u64,
    pub get_internal_error_info_calls: u64,
    pub get_sampling_rate_calls: u64,
}

impl AtracMulti {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn instance(&self, handle_addr: u32) -> Option<InstanceState> {
        self.instances
            .iter()
            .find(|(a, _)| *a == handle_addr)
            .map(|(_, s)| *s)
    }

    pub fn instances(&self) -> &[(u32, InstanceState)] {
        &self.instances
    }

    fn instance_mut(&mut self, handle_addr: u32) -> &mut InstanceState {
        if let Some(pos) = self.instances.iter().position(|(a, _)| *a == handle_addr) {
            &mut self.instances[pos].1
        } else {
            self.instances.push((handle_addr, InstanceState::default()));
            &mut self.instances.last_mut().unwrap().1
        }
    }

    /// `cellAtracMultiSetDataAndGetMemSize(...)` — writes `*puiWorkMemByte = 0x1000`.
    pub fn set_data_and_get_mem_size(
        &mut self,
        handle_addr: u32,
        _buffer_addr: u32,
        _read_byte: u32,
        _buffer_byte: u32,
        _output_ch_num: u32,
        _track_array_addr: u32,
        out_work_mem_byte: &mut u32,
    ) -> Result<(), CellError> {
        self.set_data_and_get_mem_size_calls =
            self.set_data_and_get_mem_size_calls.saturating_add(1);
        *out_work_mem_byte = STUB_WORK_MEM_BYTES;
        self.instance_mut(handle_addr).data_set = true;
        Ok(())
    }

    /// `cellAtracMultiCreateDecoder(...)` — `memcpy(handle.ucWorkMem, work_mem, 512)`.
    /// Returns the new handle state written into the supplied slice.
    pub fn create_decoder(
        &mut self,
        handle_addr: u32,
        out_handle: &mut CellAtracMultiHandle,
        work_mem: &[u8; CELL_ATRACMULTI_HANDLE_SIZE],
        _ppu_priority: u32,
        _spu_priority: u32,
    ) -> Result<(), CellError> {
        self.create_decoder_calls = self.create_decoder_calls.saturating_add(1);
        out_handle.uc_work_mem.copy_from_slice(work_mem);
        self.instance_mut(handle_addr).decoder_created = true;
        Ok(())
    }

    /// `cellAtracMultiCreateDecoderExt(...)` — same memcpy as `CreateDecoder`.
    pub fn create_decoder_ext(
        &mut self,
        handle_addr: u32,
        out_handle: &mut CellAtracMultiHandle,
        work_mem: &[u8; CELL_ATRACMULTI_HANDLE_SIZE],
        _ppu_priority: u32,
        _ext_res: &CellAtracMultiExtRes,
    ) -> Result<(), CellError> {
        self.create_decoder_ext_calls = self.create_decoder_ext_calls.saturating_add(1);
        out_handle.uc_work_mem.copy_from_slice(work_mem);
        self.instance_mut(handle_addr).decoder_created = true;
        Ok(())
    }

    pub fn delete_decoder(&mut self, handle_addr: u32) -> Result<(), CellError> {
        self.delete_decoder_calls = self.delete_decoder_calls.saturating_add(1);
        if let Some(pos) = self.instances.iter().position(|(a, _)| *a == handle_addr) {
            self.instances.remove(pos);
        }
        Ok(())
    }

    /// `cellAtracMultiDecode(...)` — writes samples=0, finish=1, remain=ALLDATA_IS_ON_MEMORY.
    pub fn decode(
        &mut self,
        _handle_addr: u32,
        _pf_out_addr: u32,
        out_samples: &mut u32,
        out_finish_flag: &mut u32,
        out_remain_frame: &mut i32,
    ) -> Result<(), CellError> {
        self.decode_calls = self.decode_calls.saturating_add(1);
        *out_samples = 0;
        *out_finish_flag = 1;
        *out_remain_frame = CELL_ATRACMULTI_ALLDATA_IS_ON_MEMORY;
        Ok(())
    }

    /// `cellAtracMultiGetStreamDataInfo(...)` — writePointer = handle.addr().
    pub fn get_stream_data_info(
        &mut self,
        handle_addr: u32,
        out_write_pointer: &mut u32,
        out_writable_byte: &mut u32,
        out_read_position: &mut u32,
    ) -> Result<(), CellError> {
        self.get_stream_data_info_calls = self.get_stream_data_info_calls.saturating_add(1);
        *out_write_pointer = handle_addr;
        *out_writable_byte = STUB_WRITABLE_BYTES;
        *out_read_position = 0;
        Ok(())
    }

    pub fn add_stream_data(
        &mut self,
        _handle_addr: u32,
        _add_byte: u32,
    ) -> Result<(), CellError> {
        self.add_stream_data_calls = self.add_stream_data_calls.saturating_add(1);
        Ok(())
    }

    /// `cellAtracMultiGetRemainFrame(...)` — writes ALLDATA_IS_ON_MEMORY.
    pub fn get_remain_frame(
        &mut self,
        _handle_addr: u32,
        out_remain_frame: &mut i32,
    ) -> Result<(), CellError> {
        self.get_remain_frame_calls = self.get_remain_frame_calls.saturating_add(1);
        *out_remain_frame = CELL_ATRACMULTI_ALLDATA_IS_ON_MEMORY;
        Ok(())
    }

    pub fn get_vacant_size(
        &mut self,
        _handle_addr: u32,
        out_vacant_size: &mut u32,
    ) -> Result<(), CellError> {
        self.get_vacant_size_calls = self.get_vacant_size_calls.saturating_add(1);
        *out_vacant_size = STUB_VACANT_SIZE;
        Ok(())
    }

    /// `cellAtracMultiIsSecondBufferNeeded(...)` — upstream `not_an_error(0)` →
    /// boolean false, returned via Ok with an i32. We wrap as i32 to preserve
    /// the encoded return type.
    pub fn is_second_buffer_needed(&mut self, _handle_addr: u32) -> Result<i32, CellError> {
        self.is_second_buffer_needed_calls =
            self.is_second_buffer_needed_calls.saturating_add(1);
        Ok(0)
    }

    pub fn get_second_buffer_info(
        &mut self,
        _handle_addr: u32,
        out_read_position: &mut u32,
        out_data_byte: &mut u32,
    ) -> Result<(), CellError> {
        self.get_second_buffer_info_calls = self.get_second_buffer_info_calls.saturating_add(1);
        *out_read_position = 0;
        *out_data_byte = 0;
        Ok(())
    }

    pub fn set_second_buffer(
        &mut self,
        _handle_addr: u32,
        _second_buffer_addr: u32,
        _second_buffer_byte: u32,
    ) -> Result<(), CellError> {
        self.set_second_buffer_calls = self.set_second_buffer_calls.saturating_add(1);
        Ok(())
    }

    pub fn get_channel(
        &mut self,
        _handle_addr: u32,
        out_channel: &mut u32,
    ) -> Result<(), CellError> {
        self.get_channel_calls = self.get_channel_calls.saturating_add(1);
        *out_channel = STUB_CHANNEL;
        Ok(())
    }

    pub fn get_max_sample(
        &mut self,
        _handle_addr: u32,
        out_max_sample: &mut u32,
    ) -> Result<(), CellError> {
        self.get_max_sample_calls = self.get_max_sample_calls.saturating_add(1);
        *out_max_sample = STUB_MAX_SAMPLE;
        Ok(())
    }

    pub fn get_next_sample(
        &mut self,
        _handle_addr: u32,
        out_next_sample: &mut u32,
    ) -> Result<(), CellError> {
        self.get_next_sample_calls = self.get_next_sample_calls.saturating_add(1);
        *out_next_sample = 0;
        Ok(())
    }

    pub fn get_sound_info(
        &mut self,
        _handle_addr: u32,
        out_end_sample: &mut i32,
        out_loop_start_sample: &mut i32,
        out_loop_end_sample: &mut i32,
    ) -> Result<(), CellError> {
        self.get_sound_info_calls = self.get_sound_info_calls.saturating_add(1);
        *out_end_sample = 0;
        *out_loop_start_sample = 0;
        *out_loop_end_sample = 0;
        Ok(())
    }

    /// `cellAtracMultiGetNextDecodePosition(...)` — writes 0 AND returns
    /// `ALLDATA_WAS_DECODED`. This is the one entry that always errors.
    pub fn get_next_decode_position(
        &mut self,
        _handle_addr: u32,
        out_sample_position: &mut u32,
    ) -> Result<(), CellError> {
        self.get_next_decode_position_calls =
            self.get_next_decode_position_calls.saturating_add(1);
        *out_sample_position = 0;
        Err(CELL_ATRACMULTI_ERROR_ALLDATA_WAS_DECODED)
    }

    pub fn get_bitrate(
        &mut self,
        _handle_addr: u32,
        out_bitrate: &mut u32,
    ) -> Result<(), CellError> {
        self.get_bitrate_calls = self.get_bitrate_calls.saturating_add(1);
        *out_bitrate = STUB_BITRATE_KBPS;
        Ok(())
    }

    pub fn get_track_array(
        &mut self,
        _handle_addr: u32,
        _track_array_addr: u32,
    ) -> Result<(), CellError> {
        self.get_track_array_calls = self.get_track_array_calls.saturating_add(1);
        Ok(())
    }

    pub fn get_loop_info(
        &mut self,
        _handle_addr: u32,
        out_loop_num: &mut i32,
        out_loop_status: &mut u32,
    ) -> Result<(), CellError> {
        self.get_loop_info_calls = self.get_loop_info_calls.saturating_add(1);
        *out_loop_num = 0;
        *out_loop_status = 0;
        Ok(())
    }

    pub fn set_loop_num(
        &mut self,
        handle_addr: u32,
        loop_num: i32,
    ) -> Result<(), CellError> {
        self.set_loop_num_calls = self.set_loop_num_calls.saturating_add(1);
        let inst = self.instance_mut(handle_addr);
        inst.loop_num_set = true;
        inst.loop_num = loop_num;
        Ok(())
    }

    /// `cellAtracMultiGetBufferInfoForResetting(...)` — fills BufferInfo with
    /// writeAddr=handle_addr, writableByte=0x1000, minWriteByte=0, readPos=0.
    pub fn get_buffer_info_for_resetting(
        &mut self,
        handle_addr: u32,
        _sample: u32,
        out: &mut CellAtracMultiBufferInfo,
    ) -> Result<(), CellError> {
        self.get_buffer_info_for_resetting_calls = self
            .get_buffer_info_for_resetting_calls
            .saturating_add(1);
        out.puc_write_addr = handle_addr;
        out.ui_writable_byte = STUB_WRITABLE_BYTES;
        out.ui_min_write_byte = 0;
        out.ui_read_position = 0;
        Ok(())
    }

    pub fn reset_play_position(
        &mut self,
        _handle_addr: u32,
        _sample: u32,
        _write_byte: u32,
        _track_array_addr: u32,
    ) -> Result<(), CellError> {
        self.reset_play_position_calls = self.reset_play_position_calls.saturating_add(1);
        Ok(())
    }

    pub fn get_internal_error_info(
        &mut self,
        _handle_addr: u32,
        out_result: &mut i32,
    ) -> Result<(), CellError> {
        self.get_internal_error_info_calls =
            self.get_internal_error_info_calls.saturating_add(1);
        *out_result = 0;
        Ok(())
    }

    pub fn get_sampling_rate(&mut self) -> Result<(), CellError> {
        self.get_sampling_rate_calls = self.get_sampling_rate_calls.saturating_add(1);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const H: u32 = 0x1000_0000;

    #[test]
    fn module_name_and_entry_count() {
        assert_eq!(MODULE_NAME, "cellAtracMulti");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 25);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellAtracMultiSetDataAndGetMemSize");
        assert_eq!(REGISTERED_ENTRY_POINTS[4], "cellAtracMultiDecode");
        assert_eq!(REGISTERED_ENTRY_POINTS[24], "cellAtracMultiGetSamplingRate");
    }

    #[test]
    fn error_codes_byte_exact_all_22() {
        assert_eq!(CELL_ATRACMULTI_ERROR_API_FAIL.0, 0x8061_0B01);
        // _B1_ sub-range
        assert_eq!(CELL_ATRACMULTI_ERROR_READSIZE_OVER_BUFFER.0, 0x8061_0B11);
        assert_eq!(CELL_ATRACMULTI_ERROR_UNKNOWN_FORMAT.0, 0x8061_0B12);
        assert_eq!(CELL_ATRACMULTI_ERROR_READSIZE_IS_TOO_SMALL.0, 0x8061_0B13);
        assert_eq!(CELL_ATRACMULTI_ERROR_ILLEGAL_SAMPLING_RATE.0, 0x8061_0B14);
        assert_eq!(CELL_ATRACMULTI_ERROR_ILLEGAL_DATA.0, 0x8061_0B15);
        // _B2_
        assert_eq!(CELL_ATRACMULTI_ERROR_NO_DECODER.0, 0x8061_0B21);
        assert_eq!(CELL_ATRACMULTI_ERROR_UNSET_DATA.0, 0x8061_0B22);
        assert_eq!(CELL_ATRACMULTI_ERROR_DECODER_WAS_CREATED.0, 0x8061_0B23);
        // _B3_
        assert_eq!(CELL_ATRACMULTI_ERROR_ALLDATA_WAS_DECODED.0, 0x8061_0B31);
        assert_eq!(CELL_ATRACMULTI_ERROR_NODATA_IN_BUFFER.0, 0x8061_0B32);
        assert_eq!(CELL_ATRACMULTI_ERROR_NOT_ALIGNED_OUT_BUFFER.0, 0x8061_0B33);
        assert_eq!(CELL_ATRACMULTI_ERROR_NEED_SECOND_BUFFER.0, 0x8061_0B34);
        // _B4_
        assert_eq!(CELL_ATRACMULTI_ERROR_ALLDATA_IS_ONMEMORY.0, 0x8061_0B41);
        assert_eq!(CELL_ATRACMULTI_ERROR_ADD_DATA_IS_TOO_BIG.0, 0x8061_0B42);
        // _B5_
        assert_eq!(CELL_ATRACMULTI_ERROR_NONEED_SECOND_BUFFER.0, 0x8061_0B51);
        // _B6_
        assert_eq!(CELL_ATRACMULTI_ERROR_UNSET_LOOP_NUM.0, 0x8061_0B61);
        // _B7_
        assert_eq!(CELL_ATRACMULTI_ERROR_ILLEGAL_SAMPLE.0, 0x8061_0B71);
        assert_eq!(CELL_ATRACMULTI_ERROR_ILLEGAL_RESET_BYTE.0, 0x8061_0B72);
        // _B8_
        assert_eq!(CELL_ATRACMULTI_ERROR_ILLEGAL_PPU_THREAD_PRIORITY.0, 0x8061_0B81);
        assert_eq!(CELL_ATRACMULTI_ERROR_ILLEGAL_SPU_THREAD_PRIORITY.0, 0x8061_0B82);
        // _B9_
        assert_eq!(CELL_ATRACMULTI_ERROR_API_PARAMETER.0, 0x8061_0B91);
    }

    #[test]
    fn remain_frame_sentinels_signed_negatives() {
        assert_eq!(CELL_ATRACMULTI_ALLDATA_IS_ON_MEMORY, -1);
        assert_eq!(CELL_ATRACMULTI_NONLOOP_STREAM_DATA_IS_ON_MEMORY, -2);
        assert_eq!(CELL_ATRACMULTI_LOOP_STREAM_DATA_IS_ON_MEMORY, -3);
    }

    #[test]
    fn handle_struct_is_exactly_512_bytes_and_8_byte_aligned() {
        assert_eq!(core::mem::size_of::<CellAtracMultiHandle>(), 512);
        assert_eq!(core::mem::align_of::<CellAtracMultiHandle>(), 8);
    }

    #[test]
    fn buffer_info_and_ext_res_sizes() {
        assert_eq!(core::mem::size_of::<CellAtracMultiBufferInfo>(), 16);
        assert_eq!(core::mem::size_of::<CellAtracMultiExtRes>(), 12);
    }

    #[test]
    fn set_data_returns_0x1000_work_mem() {
        let mut m = AtracMulti::new();
        let mut out = 0u32;
        m.set_data_and_get_mem_size(H, 0, 0, 0, 2, 0, &mut out).unwrap();
        assert_eq!(out, 0x1000);
        assert!(m.instance(H).unwrap().data_set);
    }

    #[test]
    fn create_decoder_memcpys_512_bytes() {
        let mut m = AtracMulti::new();
        let mut handle = CellAtracMultiHandle::default();
        let mut work_mem = [0u8; CELL_ATRACMULTI_HANDLE_SIZE];
        for (i, b) in work_mem.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        m.create_decoder(H, &mut handle, &work_mem, 1000, 100).unwrap();
        assert_eq!(handle.uc_work_mem, work_mem);
        assert!(m.instance(H).unwrap().decoder_created);
    }

    #[test]
    fn create_decoder_ext_also_memcpys() {
        let mut m = AtracMulti::new();
        let mut handle = CellAtracMultiHandle::default();
        let mut work_mem = [0u8; CELL_ATRACMULTI_HANDLE_SIZE];
        work_mem[0] = 0xAA;
        work_mem[511] = 0xFF;
        let ext = CellAtracMultiExtRes::default();
        m.create_decoder_ext(H, &mut handle, &work_mem, 500, &ext).unwrap();
        assert_eq!(handle.uc_work_mem[0], 0xAA);
        assert_eq!(handle.uc_work_mem[511], 0xFF);
    }

    #[test]
    fn delete_decoder_removes_instance() {
        let mut m = AtracMulti::new();
        let mut handle = CellAtracMultiHandle::default();
        let work = [0u8; CELL_ATRACMULTI_HANDLE_SIZE];
        m.create_decoder(H, &mut handle, &work, 0, 0).unwrap();
        assert!(m.instance(H).is_some());
        m.delete_decoder(H).unwrap();
        assert!(m.instance(H).is_none());
    }

    #[test]
    fn decode_writes_fixed_triplet() {
        let mut m = AtracMulti::new();
        let mut samples = 999u32;
        let mut finish = 0u32;
        let mut remain = 0i32;
        m.decode(H, 0x4000_0000, &mut samples, &mut finish, &mut remain).unwrap();
        assert_eq!(samples, 0);
        assert_eq!(finish, 1);
        assert_eq!(remain, CELL_ATRACMULTI_ALLDATA_IS_ON_MEMORY);
    }

    #[test]
    fn get_stream_data_info_writes_handle_addr_as_write_pointer() {
        let mut m = AtracMulti::new();
        let mut wp = 0u32;
        let mut wb = 0u32;
        let mut rp = 0u32;
        m.get_stream_data_info(H, &mut wp, &mut wb, &mut rp).unwrap();
        assert_eq!(wp, H);
        assert_eq!(wb, 0x1000);
        assert_eq!(rp, 0);
    }

    #[test]
    fn get_remain_frame_returns_alldata_sentinel() {
        let mut m = AtracMulti::new();
        let mut r = 0i32;
        m.get_remain_frame(H, &mut r).unwrap();
        assert_eq!(r, CELL_ATRACMULTI_ALLDATA_IS_ON_MEMORY);
    }

    #[test]
    fn get_vacant_size_is_0x1000() {
        let mut m = AtracMulti::new();
        let mut v = 0u32;
        m.get_vacant_size(H, &mut v).unwrap();
        assert_eq!(v, 0x1000);
    }

    #[test]
    fn is_second_buffer_needed_returns_zero_as_not_an_error() {
        let mut m = AtracMulti::new();
        assert_eq!(m.is_second_buffer_needed(H).unwrap(), 0);
    }

    #[test]
    fn get_second_buffer_info_both_zero() {
        let mut m = AtracMulti::new();
        let mut rp = 99u32;
        let mut db = 99u32;
        m.get_second_buffer_info(H, &mut rp, &mut db).unwrap();
        assert_eq!(rp, 0);
        assert_eq!(db, 0);
    }

    #[test]
    fn get_channel_is_always_two() {
        let mut m = AtracMulti::new();
        let mut c = 0u32;
        m.get_channel(H, &mut c).unwrap();
        assert_eq!(c, 2);
    }

    #[test]
    fn get_max_sample_is_512() {
        let mut m = AtracMulti::new();
        let mut s = 0u32;
        m.get_max_sample(H, &mut s).unwrap();
        assert_eq!(s, 512);
    }

    #[test]
    fn get_next_sample_is_zero() {
        let mut m = AtracMulti::new();
        let mut s = 99u32;
        m.get_next_sample(H, &mut s).unwrap();
        assert_eq!(s, 0);
    }

    #[test]
    fn get_sound_info_all_three_zero() {
        let mut m = AtracMulti::new();
        let mut end = 99i32;
        let mut ls = 99i32;
        let mut le = 99i32;
        m.get_sound_info(H, &mut end, &mut ls, &mut le).unwrap();
        assert_eq!(end, 0);
        assert_eq!(ls, 0);
        assert_eq!(le, 0);
    }

    #[test]
    fn get_next_decode_position_returns_alldata_was_decoded_error() {
        let mut m = AtracMulti::new();
        let mut pos = 999u32;
        assert_eq!(
            m.get_next_decode_position(H, &mut pos),
            Err(CELL_ATRACMULTI_ERROR_ALLDATA_WAS_DECODED)
        );
        // The out-param is still written to 0 even on error — matches cpp:179.
        assert_eq!(pos, 0);
    }

    #[test]
    fn get_bitrate_is_128() {
        let mut m = AtracMulti::new();
        let mut b = 0u32;
        m.get_bitrate(H, &mut b).unwrap();
        assert_eq!(b, 128);
    }

    #[test]
    fn get_loop_info_returns_zero_pair() {
        let mut m = AtracMulti::new();
        let mut num = 99i32;
        let mut status = 99u32;
        m.get_loop_info(H, &mut num, &mut status).unwrap();
        assert_eq!(num, 0);
        assert_eq!(status, 0);
    }

    #[test]
    fn set_loop_num_persists_state() {
        let mut m = AtracMulti::new();
        m.set_loop_num(H, 5).unwrap();
        let inst = m.instance(H).unwrap();
        assert!(inst.loop_num_set);
        assert_eq!(inst.loop_num, 5);
        m.set_loop_num(H, -1).unwrap();
        assert_eq!(m.instance(H).unwrap().loop_num, -1);
    }

    #[test]
    fn get_buffer_info_for_resetting_fills_struct() {
        let mut m = AtracMulti::new();
        let mut info = CellAtracMultiBufferInfo::default();
        m.get_buffer_info_for_resetting(H, 0x123, &mut info).unwrap();
        assert_eq!(info.puc_write_addr, H);
        assert_eq!(info.ui_writable_byte, 0x1000);
        assert_eq!(info.ui_min_write_byte, 0);
        assert_eq!(info.ui_read_position, 0);
    }

    #[test]
    fn get_internal_error_info_is_zero() {
        let mut m = AtracMulti::new();
        let mut r = 99i32;
        m.get_internal_error_info(H, &mut r).unwrap();
        assert_eq!(r, 0);
    }

    #[test]
    fn multiple_handles_tracked_independently() {
        let mut m = AtracMulti::new();
        let mut out = 0u32;
        m.set_data_and_get_mem_size(0x2000, 0, 0, 0, 2, 0, &mut out).unwrap();
        m.set_data_and_get_mem_size(0x3000, 0, 0, 0, 2, 0, &mut out).unwrap();
        m.set_loop_num(0x2000, 7).unwrap();
        m.set_loop_num(0x3000, 11).unwrap();
        assert_eq!(m.instance(0x2000).unwrap().loop_num, 7);
        assert_eq!(m.instance(0x3000).unwrap().loop_num, 11);
        assert_eq!(m.instances().len(), 2);
    }

    #[test]
    fn no_op_entries_return_ok_and_count() {
        let mut m = AtracMulti::new();
        m.add_stream_data(H, 0x200).unwrap();
        m.set_second_buffer(H, 0x4000, 0x100).unwrap();
        m.get_track_array(H, 0).unwrap();
        m.reset_play_position(H, 0, 0, 0).unwrap();
        m.get_sampling_rate().unwrap();
        assert_eq!(m.add_stream_data_calls, 1);
        assert_eq!(m.set_second_buffer_calls, 1);
        assert_eq!(m.get_track_array_calls, 1);
        assert_eq!(m.reset_play_position_calls, 1);
        assert_eq!(m.get_sampling_rate_calls, 1);
    }

    #[test]
    fn counters_track_every_entry() {
        let mut m = AtracMulti::new();
        let mut handle = CellAtracMultiHandle::default();
        let work = [0u8; CELL_ATRACMULTI_HANDLE_SIZE];
        let ext = CellAtracMultiExtRes::default();
        let mut scratch_u32 = 0u32;
        let mut scratch_i32 = 0i32;
        let mut info = CellAtracMultiBufferInfo::default();

        m.set_data_and_get_mem_size(H, 0, 0, 0, 2, 0, &mut scratch_u32).unwrap();
        m.create_decoder(H, &mut handle, &work, 0, 0).unwrap();
        m.create_decoder_ext(H, &mut handle, &work, 0, &ext).unwrap();
        m.delete_decoder(H).unwrap();
        let mut a = 0u32; let mut b = 0u32; let mut c = 0i32;
        m.decode(H, 0, &mut a, &mut b, &mut c).unwrap();
        m.get_stream_data_info(H, &mut a, &mut b, &mut scratch_u32).unwrap();
        m.add_stream_data(H, 0).unwrap();
        m.get_remain_frame(H, &mut scratch_i32).unwrap();
        m.get_vacant_size(H, &mut scratch_u32).unwrap();
        let _ = m.is_second_buffer_needed(H);
        {
            let mut r2 = 0u32;
            let mut d2 = 0u32;
            m.get_second_buffer_info(H, &mut r2, &mut d2).unwrap();
        }
        m.set_second_buffer(H, 0, 0).unwrap();
        m.get_channel(H, &mut scratch_u32).unwrap();
        m.get_max_sample(H, &mut scratch_u32).unwrap();
        m.get_next_sample(H, &mut scratch_u32).unwrap();
        {
            let mut e = 0i32;
            let mut ls = 0i32;
            let mut le = 0i32;
            m.get_sound_info(H, &mut e, &mut ls, &mut le).unwrap();
        }
        let _ = m.get_next_decode_position(H, &mut scratch_u32);
        m.get_bitrate(H, &mut scratch_u32).unwrap();
        m.get_track_array(H, 0).unwrap();
        m.get_loop_info(H, &mut scratch_i32, &mut scratch_u32).unwrap();
        m.set_loop_num(H, 0).unwrap();
        m.get_buffer_info_for_resetting(H, 0, &mut info).unwrap();
        m.reset_play_position(H, 0, 0, 0).unwrap();
        m.get_internal_error_info(H, &mut scratch_i32).unwrap();
        m.get_sampling_rate().unwrap();

        let sum = m.set_data_and_get_mem_size_calls
            + m.create_decoder_calls
            + m.create_decoder_ext_calls
            + m.delete_decoder_calls
            + m.decode_calls
            + m.get_stream_data_info_calls
            + m.add_stream_data_calls
            + m.get_remain_frame_calls
            + m.get_vacant_size_calls
            + m.is_second_buffer_needed_calls
            + m.get_second_buffer_info_calls
            + m.set_second_buffer_calls
            + m.get_channel_calls
            + m.get_max_sample_calls
            + m.get_next_sample_calls
            + m.get_sound_info_calls
            + m.get_next_decode_position_calls
            + m.get_bitrate_calls
            + m.get_track_array_calls
            + m.get_loop_info_calls
            + m.set_loop_num_calls
            + m.get_buffer_info_for_resetting_calls
            + m.reset_play_position_calls
            + m.get_internal_error_info_calls
            + m.get_sampling_rate_calls;
        // Every one of the 25 entries was called exactly once above.
        assert_eq!(sum, 25);
    }

    #[test]
    fn full_atracmulti_lifecycle_smoke() {
        let mut m = AtracMulti::new();
        let mut handle = CellAtracMultiHandle::default();
        let mut work = [0u8; CELL_ATRACMULTI_HANDLE_SIZE];
        for (i, b) in work.iter_mut().enumerate() {
            *b = ((i * 7 + 3) & 0xFF) as u8;
        }

        // 1. Probe required work mem size.
        let mut mem_size = 0u32;
        m.set_data_and_get_mem_size(H, 0x4000_0000, 0x100, 0x1000, 2, 0, &mut mem_size).unwrap();
        assert_eq!(mem_size, 0x1000);

        // 2. Create decoder, handle gets the work memory.
        m.create_decoder(H, &mut handle, &work, 1000, 50).unwrap();
        assert_eq!(handle.uc_work_mem, work);

        // 3. Query channel count, bitrate, max sample.
        let mut c = 0u32;
        m.get_channel(H, &mut c).unwrap();
        assert_eq!(c, 2);
        let mut br = 0u32;
        m.get_bitrate(H, &mut br).unwrap();
        assert_eq!(br, 128);
        let mut ms = 0u32;
        m.get_max_sample(H, &mut ms).unwrap();
        assert_eq!(ms, 512);

        // 4. Decode — always (0, finish, ALLDATA_IS_ON_MEMORY).
        let mut samples = 0u32;
        let mut finish = 0u32;
        let mut remain = 0i32;
        m.decode(H, 0x5000_0000, &mut samples, &mut finish, &mut remain).unwrap();
        assert_eq!((samples, finish, remain), (0, 1, CELL_ATRACMULTI_ALLDATA_IS_ON_MEMORY));

        // 5. GetNextDecodePosition — peculiarity: always errors.
        let mut pos = 0u32;
        assert_eq!(
            m.get_next_decode_position(H, &mut pos),
            Err(CELL_ATRACMULTI_ERROR_ALLDATA_WAS_DECODED)
        );

        // 6. SetLoopNum persists, verify state.
        m.set_loop_num(H, 3).unwrap();
        assert_eq!(m.instance(H).unwrap().loop_num, 3);

        // 7. Teardown.
        m.delete_decoder(H).unwrap();
        assert!(m.instance(H).is_none());
    }
}
