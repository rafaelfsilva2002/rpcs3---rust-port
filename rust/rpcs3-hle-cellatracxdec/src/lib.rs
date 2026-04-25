//! `rpcs3-hle-cellatracxdec` — Rust port of `rpcs3/Emu/Cell/Modules/cellAtracXdec.cpp`.
//!
//! PS3 ATRAC3plus SPU-decoder PRX HLE. This is a contract-only port: the
//! real decoding path in C++ RPCS3 wraps FFmpeg (`avcodec_find_decoder`,
//! `AV_CODEC_ID_ATRAC3P`, `av_packet_alloc`, `av_frame_alloc`) plus an SPU
//! decode thread driven by CellSpurs tasks, and wiring those into no_std
//! Rust pulls in the entire SPU runtime + ffmpeg bindings. Instead we freeze
//! the bits that any caller can observe:
//!
//! - 54 error codes byte-exato (`cellAtracXdec.h:26..89`, facility 0x80612___).
//! - 4 `CellAdecCoreOps` VNID registrations (2ch/6ch/8ch/default).
//! - 15 REG_HIDDEN_FUNC entries (12 CoreOp vtable hooks + 3 GetMemSize template
//!   instantiations `<2>`/`<6>`/`<8>` + `atracXdecEntry`).
//! - `atracXdecGetSpursMemSize(nch_in)` — constant-table lookup, returns `u32::MAX`
//!   for invalid nch (cpp:90..104).
//! - `atracxdec_state` FSM (7 states, cpp:218..227).
//! - Word-size enum values (cpp header `CELL_ADEC_ATRACX_WORD_SZ_*`).
//! - CHECK_SIZE constants (`AtracXdecDecoder=0xa8`, `AtracXdecContext=0x268`).
#![no_std]
extern crate alloc;

use rpcs3_emu::CellError;
use rpcs3_emu_types as rpcs3_emu;

pub const CELL_OK: u32 = 0;

/// Facility prefix for ATRAC3plus SPU-decoder errors.
pub const CELL_ADEC_ERROR_ATX_FACILITY: u32 = 0x8061_2200;

// cpp header 26..89 — byte-exact.
pub const CELL_ADEC_ERROR_ATX_OFFSET: u32 = 0x8061_2200;
pub const CELL_ADEC_ERROR_ATX_NONE: u32 = 0x8061_2200;
pub const CELL_ADEC_ERROR_ATX_OK: u32 = 0x8061_2200;
pub const CELL_ADEC_ERROR_ATX_BUSY: u32 = 0x8061_2264;
pub const CELL_ADEC_ERROR_ATX_EMPTY: u32 = 0x8061_2265;
pub const CELL_ADEC_ERROR_ATX_ATSHDR: u32 = 0x8061_2266;
pub const CELL_ADEC_ERROR_ATX_NON_FATAL: u32 = 0x8061_2281;
pub const CELL_ADEC_ERROR_ATX_NOT_IMPLE: u32 = 0x8061_2282;
pub const CELL_ADEC_ERROR_ATX_PACK_CE_OVERFLOW: u32 = 0x8061_2283;
pub const CELL_ADEC_ERROR_ATX_ILLEGAL_NPROCQUS: u32 = 0x8061_2284;
pub const CELL_ADEC_ERROR_ATX_FATAL: u32 = 0x8061_228c;
pub const CELL_ADEC_ERROR_ATX_ENC_OVERFLOW: u32 = 0x8061_228d;
pub const CELL_ADEC_ERROR_ATX_PACK_CE_UNDERFLOW: u32 = 0x8061_228e;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDCT: u32 = 0x8061_228f;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_GAINADJ: u32 = 0x8061_2290;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDSF: u32 = 0x8061_2291;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_SPECTRA: u32 = 0x8061_2292;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDWL: u32 = 0x8061_2293;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_GHWAVE: u32 = 0x8061_2294;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_SHEADER: u32 = 0x8061_2295;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDWL_A: u32 = 0x8061_2296;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDWL_B: u32 = 0x8061_2297;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDWL_C: u32 = 0x8061_2298;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDWL_D: u32 = 0x8061_2299;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDWL_E: u32 = 0x8061_229a;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDSF_A: u32 = 0x8061_229b;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDSF_B: u32 = 0x8061_229c;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDSF_C: u32 = 0x8061_229d;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDSF_D: u32 = 0x8061_229e;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_IDCT_A: u32 = 0x8061_229f;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_GC_NGC: u32 = 0x8061_22a0;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_GC_IDLEV_A: u32 = 0x8061_22a1;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_GC_IDLOC_A: u32 = 0x8061_22a2;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_GC_IDLEV_B: u32 = 0x8061_22a3;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_GC_IDLOC_B: u32 = 0x8061_22a4;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_SN_NWVS: u32 = 0x8061_22a5;
pub const CELL_ADEC_ERROR_ATX_FATAL_HANDLE: u32 = 0x8061_22aa;
pub const CELL_ADEC_ERROR_ATX_ASSERT_SAMPLING_FREQ: u32 = 0x8061_22ab;
pub const CELL_ADEC_ERROR_ATX_ASSERT_CH_CONFIG_INDEX: u32 = 0x8061_22ac;
pub const CELL_ADEC_ERROR_ATX_ASSERT_NBYTES: u32 = 0x8061_22ad;
pub const CELL_ADEC_ERROR_ATX_ASSERT_BLOCK_NUM: u32 = 0x8061_22ae;
pub const CELL_ADEC_ERROR_ATX_ASSERT_BLOCK_ID: u32 = 0x8061_22af;
pub const CELL_ADEC_ERROR_ATX_ASSERT_CHANNELS: u32 = 0x8061_22b0;
pub const CELL_ADEC_ERROR_ATX_UNINIT_BLOCK_SPECIFIED: u32 = 0x8061_22b1;
pub const CELL_ADEC_ERROR_ATX_POSCFG_PRESENT: u32 = 0x8061_22b2;
pub const CELL_ADEC_ERROR_ATX_BUFFER_OVERFLOW: u32 = 0x8061_22b3;
pub const CELL_ADEC_ERROR_ATX_ILL_BLK_TYPE_ID: u32 = 0x8061_22b4;
pub const CELL_ADEC_ERROR_ATX_UNPACK_CHANNEL_BLK_FAILED: u32 = 0x8061_22b5;
pub const CELL_ADEC_ERROR_ATX_ILL_BLK_ID_USED_1: u32 = 0x8061_22b6;
pub const CELL_ADEC_ERROR_ATX_ILL_BLK_ID_USED_2: u32 = 0x8061_22b7;
pub const CELL_ADEC_ERROR_ATX_ILLEGAL_ENC_SETTING: u32 = 0x8061_22b8;
pub const CELL_ADEC_ERROR_ATX_ILLEGAL_DEC_SETTING: u32 = 0x8061_22b9;
pub const CELL_ADEC_ERROR_ATX_ASSERT_NSAMPLES: u32 = 0x8061_22ba;
pub const CELL_ADEC_ERROR_ATX_ILL_SYNCWORD: u32 = 0x8061_22bb;
pub const CELL_ADEC_ERROR_ATX_ILL_SAMPLING_FREQ: u32 = 0x8061_22bc;
pub const CELL_ADEC_ERROR_ATX_ILL_CH_CONFIG_INDEX: u32 = 0x8061_22bd;
pub const CELL_ADEC_ERROR_ATX_RAW_DATA_FRAME_SIZE_OVER: u32 = 0x8061_22be;
pub const CELL_ADEC_ERROR_ATX_SYNTAX_ENHANCE_LENGTH_OVER: u32 = 0x8061_22bf;
pub const CELL_ADEC_ERROR_ATX_SPU_INTERNAL_FAIL: u32 = 0x8061_22c8;

/// PCM output word-size tags (cpp header CELL_ADEC_ATRACX_WORD_SZ_*).
pub const CELL_ADEC_ATRACX_WORD_SZ_16BIT: u8 = 0x02;
pub const CELL_ADEC_ATRACX_WORD_SZ_24BIT: u8 = 0x03;
pub const CELL_ADEC_ATRACX_WORD_SZ_32BIT: u8 = 0x04;
pub const CELL_ADEC_ATRACX_WORD_SZ_FLOAT: u8 = 0x84;

/// CellSpurs + CellSpursTaskset + AtracXdecContext + scratch, wired into the
/// work-memory region passed by the caller (cpp:287).
pub const ATXDEC_SPURS_STRUCTS_SIZE: u32 = 0x1cf00;
pub const ATXDEC_SAMPLES_PER_FRAME: u16 = 0x800;
pub const ATXDEC_MAX_FRAME_LENGTH: u16 = 0x2000;

/// Maps ch_config_idx (0..7) to the number of decoder "blocks" spawned
/// (cpp:290). Index 0/1 collapse to a single block; 8ch still uses 5 because
/// one block carries a stereo pair.
pub const ATXDEC_NCH_BLOCKS_MAP: [u8; 8] = [0, 1, 1, 2, 3, 4, 5, 5];

/// Size requirements (cpp:215, 285) — frozen so future SPU-side layout
/// changes surface as loud test breaks rather than silent wire-format drift.
pub const ATRACXDEC_DECODER_STRUCT_SIZE: usize = 0xa8;
pub const ATRACXDEC_CONTEXT_STRUCT_SIZE: usize = 0x268;

/// SPURS scratch size per input-channel count (cpp:90..104).
/// Returns `u32::MAX` for invalid channel counts, matching the C++ `-1` sentinel.
#[must_use]
pub const fn atracx_dec_get_spurs_mem_size(nch_in: u32) -> u32 {
    match nch_in {
        1 => 0x6000,
        2 => 0x6000,
        3 => 0x1_2880,
        4 => 0x1_9c80,
        5 => u32::MAX,
        6 => 0x2_3080,
        7 => 0x2_a480,
        8 => 0x2_c480,
        _ => u32::MAX,
    }
}

/// HLE-only decode-loop state machine (cpp:218..227). Preserved as an enum so
/// tests can ensure the savestate writer/reader agrees on the tag bytes.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtracxdecState {
    Initial = 0,
    WaitingForCmd = 1,
    CheckingRunThread1 = 2,
    ExecutingCmd = 3,
    WaitingForOutput = 4,
    CheckingRunThread2 = 5,
    Decoding = 6,
}

/// `CellAdecCoreOps` VNID registrations (cpp:978..996). Four vtables, one per
/// channel-group label. `atracx` (default) reuses the 8-channel `GetMemSize`.
pub const VNID_CORE_OPS_ATRACX2CH: u32 = 0x076b_33ab;
pub const VNID_CORE_OPS_ATRACX6CH: u32 = 0x1d21_0eaa;
pub const VNID_CORE_OPS_ATRACX8CH: u32 = 0xe9a8_6e54;
pub const VNID_CORE_OPS_ATRACX: u32 = 0x4944_af9a;

pub const CORE_OPS_VNIDS: [(&str, u32); 4] = [
    ("g_cell_adec_core_ops_atracx2ch", VNID_CORE_OPS_ATRACX2CH),
    ("g_cell_adec_core_ops_atracx6ch", VNID_CORE_OPS_ATRACX6CH),
    ("g_cell_adec_core_ops_atracx8ch", VNID_CORE_OPS_ATRACX8CH),
    ("g_cell_adec_core_ops_atracx", VNID_CORE_OPS_ATRACX),
];

/// `REG_HIDDEN_FUNC` entries from cpp:999..1014, in registration order.
/// These are the SPU-facing vtable hooks; they never appear in the public
/// PRX symbol table but are addressable via `ppu_function_manager::func_addr`.
pub const ENTRY_POINTS: &[&str] = &[
    "_CellAdecCoreOpGetMemSize_atracx<2>",
    "_CellAdecCoreOpGetMemSize_atracx<6>",
    "_CellAdecCoreOpGetMemSize_atracx<8>",
    "_CellAdecCoreOpOpen_atracx",
    "_CellAdecCoreOpClose_atracx",
    "_CellAdecCoreOpStartSeq_atracx",
    "_CellAdecCoreOpEndSeq_atracx",
    "_CellAdecCoreOpDecodeAu_atracx",
    "_CellAdecCoreOpGetVersion_atracx",
    "_CellAdecCoreOpRealign_atracx",
    "_CellAdecCoreOpReleasePcm_atracx",
    "_CellAdecCoreOpGetPcmHandleNum_atracx",
    "_CellAdecCoreOpGetBsiInfoSize_atracx",
    "_CellAdecCoreOpOpenExt_atracx",
    "atracXdecEntry",
];

/// Contract-only HLE dispatcher — every call to one of the 15 hidden funcs or
/// to a VNID-resolved core op bumps a counter and returns `CELL_OK`. The real
/// C++ path delegates to FFmpeg + SPURS; in contract mode we just need to
/// ensure the PRX registration surface is stable.
pub struct AtracXdecHle {
    pub hidden_func_calls: [u64; 15],
    pub vnid_lookups: [u64; 4],
    pub state: AtracxdecState,
}

impl AtracXdecHle {
    pub const fn new() -> Self {
        Self {
            hidden_func_calls: [0; 15],
            vnid_lookups: [0; 4],
            state: AtracxdecState::Initial,
        }
    }

    /// Mark a hidden-func dispatch (by ENTRY_POINTS index) and return CELL_OK.
    pub fn dispatch_hidden(&mut self, index: usize) -> Result<u32, CellError> {
        if index >= ENTRY_POINTS.len() {
            return Err(CellError(0x8001_0000));
        }
        self.hidden_func_calls[index] = self.hidden_func_calls[index].saturating_add(1);
        Ok(CELL_OK)
    }

    /// Look up a CoreOps VNID. Returns the matching table's local index (0..3)
    /// or a not-found error.
    pub fn lookup_vnid(&mut self, vnid: u32) -> Result<usize, CellError> {
        match CORE_OPS_VNIDS.iter().position(|&(_, v)| v == vnid) {
            Some(i) => {
                self.vnid_lookups[i] = self.vnid_lookups[i].saturating_add(1);
                Ok(i)
            }
            None => Err(CellError(CELL_ADEC_ERROR_ATX_FATAL_HANDLE)),
        }
    }

    /// Drives the savestate-visible FSM forward. Intended purely for tests
    /// — the real transitions happen inside the decode thread.
    pub fn set_state(&mut self, s: AtracxdecState) {
        self.state = s;
    }
}

impl Default for AtracXdecHle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_ADEC_ERROR_ATX_OK, 0x8061_2200);
        assert_eq!(CELL_ADEC_ERROR_ATX_BUSY, 0x8061_2264);
        assert_eq!(CELL_ADEC_ERROR_ATX_ATSHDR, 0x8061_2266);
        assert_eq!(CELL_ADEC_ERROR_ATX_FATAL, 0x8061_228c);
        assert_eq!(CELL_ADEC_ERROR_ATX_FATAL_HANDLE, 0x8061_22aa);
        assert_eq!(CELL_ADEC_ERROR_ATX_SPU_INTERNAL_FAIL, 0x8061_22c8);
    }

    #[test]
    fn spurs_mem_size_matches_cpp_table() {
        // cpp:90..104 — mirror each case exactly, including the invalid-5 sentinel.
        assert_eq!(atracx_dec_get_spurs_mem_size(1), 0x6000);
        assert_eq!(atracx_dec_get_spurs_mem_size(2), 0x6000);
        assert_eq!(atracx_dec_get_spurs_mem_size(3), 0x1_2880);
        assert_eq!(atracx_dec_get_spurs_mem_size(4), 0x1_9c80);
        assert_eq!(atracx_dec_get_spurs_mem_size(5), u32::MAX);
        assert_eq!(atracx_dec_get_spurs_mem_size(6), 0x2_3080);
        assert_eq!(atracx_dec_get_spurs_mem_size(7), 0x2_a480);
        assert_eq!(atracx_dec_get_spurs_mem_size(8), 0x2_c480);
        assert_eq!(atracx_dec_get_spurs_mem_size(0), u32::MAX);
        assert_eq!(atracx_dec_get_spurs_mem_size(9), u32::MAX);
    }

    #[test]
    fn nch_blocks_map_matches_cpp() {
        assert_eq!(ATXDEC_NCH_BLOCKS_MAP, [0, 1, 1, 2, 3, 4, 5, 5]);
    }

    #[test]
    fn constants_frozen() {
        assert_eq!(ATXDEC_SPURS_STRUCTS_SIZE, 0x1cf00);
        assert_eq!(ATXDEC_SAMPLES_PER_FRAME, 0x800);
        assert_eq!(ATXDEC_MAX_FRAME_LENGTH, 0x2000);
        assert_eq!(ATRACXDEC_DECODER_STRUCT_SIZE, 0xa8);
        assert_eq!(ATRACXDEC_CONTEXT_STRUCT_SIZE, 0x268);
    }

    #[test]
    fn word_sizes_match_header() {
        assert_eq!(CELL_ADEC_ATRACX_WORD_SZ_16BIT, 0x02);
        assert_eq!(CELL_ADEC_ATRACX_WORD_SZ_24BIT, 0x03);
        assert_eq!(CELL_ADEC_ATRACX_WORD_SZ_32BIT, 0x04);
        assert_eq!(CELL_ADEC_ATRACX_WORD_SZ_FLOAT, 0x84);
    }

    #[test]
    fn vnids_match_cpp() {
        assert_eq!(VNID_CORE_OPS_ATRACX2CH, 0x076b_33ab);
        assert_eq!(VNID_CORE_OPS_ATRACX6CH, 0x1d21_0eaa);
        assert_eq!(VNID_CORE_OPS_ATRACX8CH, 0xe9a8_6e54);
        assert_eq!(VNID_CORE_OPS_ATRACX, 0x4944_af9a);
    }

    #[test]
    fn entry_points_count_and_order() {
        assert_eq!(ENTRY_POINTS.len(), 15, "REG_HIDDEN_FUNC count cpp:999..1014");
        assert_eq!(ENTRY_POINTS[0], "_CellAdecCoreOpGetMemSize_atracx<2>");
        assert_eq!(ENTRY_POINTS[14], "atracXdecEntry");
    }

    #[test]
    fn dispatch_hidden_ok_and_oob() {
        let mut hle = AtracXdecHle::new();
        assert_eq!(hle.dispatch_hidden(0).unwrap(), CELL_OK);
        assert_eq!(hle.hidden_func_calls[0], 1);
        let err = hle.dispatch_hidden(15).unwrap_err();
        assert_eq!(err.0, 0x8001_0000);
    }

    #[test]
    fn vnid_lookup_tracks_hits() {
        let mut hle = AtracXdecHle::new();
        let idx = hle.lookup_vnid(VNID_CORE_OPS_ATRACX8CH).unwrap();
        assert_eq!(idx, 2);
        assert_eq!(hle.vnid_lookups[2], 1);

        let err = hle.lookup_vnid(0xdead_beef).unwrap_err();
        assert_eq!(err.0, CELL_ADEC_ERROR_ATX_FATAL_HANDLE);
    }

    #[test]
    fn fsm_advances() {
        let mut hle = AtracXdecHle::new();
        assert_eq!(hle.state, AtracxdecState::Initial);
        hle.set_state(AtracxdecState::WaitingForCmd);
        assert_eq!(hle.state as u8, 1);
        hle.set_state(AtracxdecState::Decoding);
        assert_eq!(hle.state as u8, 6);
    }
}
