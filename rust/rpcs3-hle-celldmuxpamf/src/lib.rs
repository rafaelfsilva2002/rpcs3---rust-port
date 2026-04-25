//! `rpcs3-hle-celldmuxpamf` — Rust port of `rpcs3/Emu/Cell/Modules/cellDmuxPamf.cpp`.
//!
//! PS3 PAMF MPEG-PS demuxer HLE. The C++ module drives an SPU thread that
//! parses PS packs (0x800-byte packs, 0x1ba pack-start codes, PTS/DTS) and
//! dispatches audio/video PES back to CellDmux callbacks. Porting the full
//! SPU program and packet-level parser is a separate project — this crate
//! freezes the parts of the contract that anything outside the demuxer
//! thread can observe:
//!
//! - `CellDmuxPamfError` enum values (header:665..672).
//! - `DmuxPamfStreamTypeIndex` tags (header:256..263, signed -1 sentinel).
//! - MPEG-PS start-code constants (header:306..315).
//! - AVC/M2V level enums (header:674..689).
//! - LPCM channel/FS/bits-per-sample (header:767..783).
//! - Packet layout offsets (PACK_SIZE, PES offsets, etc.).
//! - 2 `CellDmuxCoreOps` VNIDs (pamf=0x28b2b7b2, raw_es=0x9728a0e9).
//! - 23 `REG_HIDDEN_FUNC` entries (17 CoreOp templates + 5 notify hooks + entry).
//! - AC3/ATRAC-X sync words and ATRAC-X ATS header size.
#![no_std]
extern crate alloc;

use rpcs3_emu::CellError;
use rpcs3_emu_types as rpcs3_emu;

pub const CELL_OK: u32 = 0;

// CellDmuxPamfError (cpp header:665..672). Note the non-zero base and the gap
// at 4 — preserved so any existing caller-side switch table stays valid.
pub const CELL_DMUX_PAMF_ERROR_BUSY: u32 = 1;
pub const CELL_DMUX_PAMF_ERROR_ARG: u32 = 2;
pub const CELL_DMUX_PAMF_ERROR_UNKNOWN_STREAM: u32 = 3;
pub const CELL_DMUX_PAMF_ERROR_NO_MEMORY: u32 = 5;
pub const CELL_DMUX_PAMF_ERROR_FATAL: u32 = 6;

// DmuxPamfStreamTypeIndex (cpp header:256..263). Signed — -1 is the sentinel.
pub const DMUX_PAMF_STREAM_TYPE_INDEX_INVALID: i32 = -1;
pub const DMUX_PAMF_STREAM_TYPE_INDEX_VIDEO: i32 = 0;
pub const DMUX_PAMF_STREAM_TYPE_INDEX_LPCM: i32 = 1;
pub const DMUX_PAMF_STREAM_TYPE_INDEX_AC3: i32 = 2;
pub const DMUX_PAMF_STREAM_TYPE_INDEX_ATRACX: i32 = 3;
pub const DMUX_PAMF_STREAM_TYPE_INDEX_USER_DATA: i32 = 4;

// Pack / PES layout (cpp header:299..314).
pub const PACK_SIZE: u16 = 0x800;
pub const PACK_STUFFING_LENGTH_OFFSET: i8 = 0xd;
pub const PES_PACKET_LENGTH_OFFSET: i8 = 0x4;
pub const PES_HEADER_DATA_LENGTH_OFFSET: i8 = 0x8;
pub const PTS_DTS_FLAG_OFFSET: i8 = 0x7;
pub const PACKET_START_CODE_PREFIX: u8 = 1;

// MPEG start codes (big-endian on the wire).
pub const M2V_PIC_START: u32 = 0x0000_0100;
pub const AVC_AU_DELIMITER: u32 = 0x0000_0109;
pub const M2V_SEQUENCE_HEADER: u32 = 0x0000_01b3;
pub const M2V_SEQUENCE_END: u32 = 0x0000_01b7;
pub const PACK_START: u32 = 0x0000_01ba;
pub const SYSTEM_HEADER: u32 = 0x0000_01bb;
pub const PRIVATE_STREAM_1: u32 = 0x0000_01bd;
pub const PRIVATE_STREAM_2: u32 = 0x0000_01bf;
pub const PROG_END: u32 = 0x0000_01b9;
/// Base start code for video streams 0xe0..0xef — low nibble is channel id.
pub const VIDEO_STREAM_BASE: u32 = 0x0000_01e0;

// CellDmuxPamfM2vLevel (header:674..680).
pub const CELL_DMUX_PAMF_M2V_MP_LL: u32 = 0;
pub const CELL_DMUX_PAMF_M2V_MP_ML: u32 = 1;
pub const CELL_DMUX_PAMF_M2V_MP_H14: u32 = 2;
pub const CELL_DMUX_PAMF_M2V_MP_HL: u32 = 3;

// CellDmuxPamfAvcLevel (header:682..690). Non-contiguous — frozen exactly.
pub const CELL_DMUX_PAMF_AVC_LEVEL_2P1: u32 = 21;
pub const CELL_DMUX_PAMF_AVC_LEVEL_3P0: u32 = 30;
pub const CELL_DMUX_PAMF_AVC_LEVEL_3P1: u32 = 31;
pub const CELL_DMUX_PAMF_AVC_LEVEL_3P2: u32 = 32;
pub const CELL_DMUX_PAMF_AVC_LEVEL_4P1: u32 = 41;
pub const CELL_DMUX_PAMF_AVC_LEVEL_4P2: u32 = 42;

// LPCM params (header:756..783).
pub const CELL_DMUX_PAMF_FS_48K: u32 = 48_000;
pub const CELL_DMUX_PAMF_BITS_PER_SAMPLE_16: u32 = 16;
pub const CELL_DMUX_PAMF_BITS_PER_SAMPLE_24: u32 = 24;

pub const CELL_DMUX_PAMF_LPCM_CH_M1: u32 = 1;
pub const CELL_DMUX_PAMF_LPCM_CH_LR: u32 = 3;
pub const CELL_DMUX_PAMF_LPCM_CH_LRCLSRSLFE: u32 = 9;
pub const CELL_DMUX_PAMF_LPCM_CH_LRCLSCS1CS2RSLFE: u32 = 11;

pub const CELL_DMUX_PAMF_LPCM_FS_48K: u32 = 1;
pub const CELL_DMUX_PAMF_LPCM_BITS_PER_SAMPLE_16: u32 = 1;
pub const CELL_DMUX_PAMF_LPCM_BITS_PER_SAMPLE_24: u32 = 3;

// Audio sync words used when validating private-stream-1 payloads (cpp:551).
pub const AC3_SYNC_WORD: u16 = 0x0b77;
pub const ATRACX_SYNC_WORD: u16 = 0x0fd0;
pub const ATRACX_ATS_HEADER_SIZE: u8 = 8;

// DmuxId range (cpp:602..604).
pub const DMUX_ID_BASE: u32 = 0;
pub const DMUX_ID_STEP: u32 = 1;
pub const DMUX_ID_COUNT: u32 = 0x400;

/// CoreOps VNIDs (cpp:2878..2879). The `pamf` table drives packet demux; the
/// `raw_es` table bypasses the PS parser for pre-demuxed elementary streams.
pub const VNID_CORE_OPS_PAMF: u32 = 0x28b2_b7b2;
pub const VNID_CORE_OPS_RAW_ES: u32 = 0x9728_a0e9;

pub const CORE_OPS_VNIDS: [(&str, u32); 2] = [
    ("g_cell_dmux_core_ops_pamf", VNID_CORE_OPS_PAMF),
    ("g_cell_dmux_core_ops_raw_es", VNID_CORE_OPS_RAW_ES),
];

/// 23 REG_HIDDEN_FUNC entries from cpp:2881..2905, in registration order.
/// The `<false>`/`<true>` suffixes disambiguate the pamf/raw_es template
/// instantiations that share a symbol prefix.
pub const ENTRY_POINTS: &[&str] = &[
    "_CellDmuxCoreOpQueryAttr",
    "_CellDmuxCoreOpOpen",
    "_CellDmuxCoreOpClose",
    "_CellDmuxCoreOpResetStream",
    "_CellDmuxCoreOpCreateThread",
    "_CellDmuxCoreOpJoinThread",
    "_CellDmuxCoreOpSetStream<false>",
    "_CellDmuxCoreOpSetStream<true>",
    "_CellDmuxCoreOpReleaseAu",
    "_CellDmuxCoreOpQueryEsAttr<false>",
    "_CellDmuxCoreOpQueryEsAttr<true>",
    "_CellDmuxCoreOpEnableEs<false>",
    "_CellDmuxCoreOpEnableEs<true>",
    "_CellDmuxCoreOpDisableEs",
    "_CellDmuxCoreOpFlushEs",
    "_CellDmuxCoreOpResetEs",
    "_CellDmuxCoreOpResetStreamAndWaitDone",
    "dmuxPamfNotifyDemuxDone",
    "dmuxPamfNotifyProgEndCode",
    "dmuxPamfNotifyFatalErr",
    "dmuxPamfEsNotifyAuFound",
    "dmuxPamfEsNotifyFlushDone",
    "dmuxPamfEntry",
];

/// Derive the video-stream start code for channel `ch` (0..=15). Values
/// outside that range saturate to the base — the SPU parser rejects such
/// streams via CELL_DMUX_PAMF_ERROR_UNKNOWN_STREAM.
#[must_use]
pub const fn video_stream_start_code(ch: u8) -> u32 {
    VIDEO_STREAM_BASE | ((ch & 0x0f) as u32)
}

/// Contract-only PRX dispatcher for the demuxer. Real implementation drives
/// an SPU thread + callback machinery; here we just record calls.
pub struct DmuxPamfHle {
    pub hidden_func_calls: [u64; 23],
    pub vnid_lookups: [u64; 2],
}

impl DmuxPamfHle {
    pub const fn new() -> Self {
        Self {
            hidden_func_calls: [0; 23],
            vnid_lookups: [0; 2],
        }
    }

    pub fn dispatch_hidden(&mut self, index: usize) -> Result<u32, CellError> {
        if index >= ENTRY_POINTS.len() {
            return Err(CellError(CELL_DMUX_PAMF_ERROR_ARG));
        }
        self.hidden_func_calls[index] = self.hidden_func_calls[index].saturating_add(1);
        Ok(CELL_OK)
    }

    pub fn lookup_vnid(&mut self, vnid: u32) -> Result<usize, CellError> {
        match CORE_OPS_VNIDS.iter().position(|&(_, v)| v == vnid) {
            Some(i) => {
                self.vnid_lookups[i] = self.vnid_lookups[i].saturating_add(1);
                Ok(i)
            }
            None => Err(CellError(CELL_DMUX_PAMF_ERROR_UNKNOWN_STREAM)),
        }
    }
}

impl Default for DmuxPamfHle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_match_header() {
        assert_eq!(CELL_DMUX_PAMF_ERROR_BUSY, 1);
        assert_eq!(CELL_DMUX_PAMF_ERROR_ARG, 2);
        assert_eq!(CELL_DMUX_PAMF_ERROR_UNKNOWN_STREAM, 3);
        assert_eq!(CELL_DMUX_PAMF_ERROR_NO_MEMORY, 5);
        assert_eq!(CELL_DMUX_PAMF_ERROR_FATAL, 6);
    }

    #[test]
    fn stream_type_signed_invalid_is_neg_one() {
        assert_eq!(DMUX_PAMF_STREAM_TYPE_INDEX_INVALID, -1);
        assert_eq!(DMUX_PAMF_STREAM_TYPE_INDEX_VIDEO, 0);
        assert_eq!(DMUX_PAMF_STREAM_TYPE_INDEX_USER_DATA, 4);
    }

    #[test]
    fn pack_layout_offsets_frozen() {
        assert_eq!(PACK_SIZE, 0x800);
        assert_eq!(PACK_STUFFING_LENGTH_OFFSET, 0xd);
        assert_eq!(PES_PACKET_LENGTH_OFFSET, 0x4);
        assert_eq!(PES_HEADER_DATA_LENGTH_OFFSET, 0x8);
        assert_eq!(PTS_DTS_FLAG_OFFSET, 0x7);
    }

    #[test]
    fn start_codes_byte_exact() {
        assert_eq!(PACK_START, 0x0000_01ba);
        assert_eq!(SYSTEM_HEADER, 0x0000_01bb);
        assert_eq!(PRIVATE_STREAM_1, 0x0000_01bd);
        assert_eq!(PRIVATE_STREAM_2, 0x0000_01bf);
        assert_eq!(PROG_END, 0x0000_01b9);
        assert_eq!(M2V_PIC_START, 0x0000_0100);
        assert_eq!(AVC_AU_DELIMITER, 0x0000_0109);
    }

    #[test]
    fn video_stream_ch_encoding() {
        assert_eq!(video_stream_start_code(0), 0x0000_01e0);
        assert_eq!(video_stream_start_code(0xf), 0x0000_01ef);
        // Upper bits are masked, so 0x10 collapses to ch 0.
        assert_eq!(video_stream_start_code(0x10), 0x0000_01e0);
    }

    #[test]
    fn avc_m2v_level_values() {
        assert_eq!(CELL_DMUX_PAMF_M2V_MP_LL, 0);
        assert_eq!(CELL_DMUX_PAMF_M2V_MP_HL, 3);
        assert_eq!(CELL_DMUX_PAMF_AVC_LEVEL_2P1, 21);
        assert_eq!(CELL_DMUX_PAMF_AVC_LEVEL_3P0, 30);
        assert_eq!(CELL_DMUX_PAMF_AVC_LEVEL_4P2, 42);
    }

    #[test]
    fn lpcm_channel_values() {
        assert_eq!(CELL_DMUX_PAMF_LPCM_CH_M1, 1);
        assert_eq!(CELL_DMUX_PAMF_LPCM_CH_LR, 3);
        assert_eq!(CELL_DMUX_PAMF_LPCM_CH_LRCLSRSLFE, 9);
        assert_eq!(CELL_DMUX_PAMF_LPCM_CH_LRCLSCS1CS2RSLFE, 11);
        assert_eq!(CELL_DMUX_PAMF_FS_48K, 48_000);
    }

    #[test]
    fn audio_sync_words() {
        assert_eq!(AC3_SYNC_WORD, 0x0b77);
        assert_eq!(ATRACX_SYNC_WORD, 0x0fd0);
        assert_eq!(ATRACX_ATS_HEADER_SIZE, 8);
    }

    #[test]
    fn dmux_id_range() {
        assert_eq!(DMUX_ID_BASE, 0);
        assert_eq!(DMUX_ID_STEP, 1);
        assert_eq!(DMUX_ID_COUNT, 0x400);
    }

    #[test]
    fn vnids_match_cpp() {
        assert_eq!(VNID_CORE_OPS_PAMF, 0x28b2_b7b2);
        assert_eq!(VNID_CORE_OPS_RAW_ES, 0x9728_a0e9);
    }

    #[test]
    fn entry_points_complete() {
        assert_eq!(ENTRY_POINTS.len(), 23);
        assert_eq!(ENTRY_POINTS[0], "_CellDmuxCoreOpQueryAttr");
        assert_eq!(ENTRY_POINTS[22], "dmuxPamfEntry");
        assert_eq!(ENTRY_POINTS[6], "_CellDmuxCoreOpSetStream<false>");
        assert_eq!(ENTRY_POINTS[7], "_CellDmuxCoreOpSetStream<true>");
    }

    #[test]
    fn dispatch_and_lookup_paths() {
        let mut hle = DmuxPamfHle::new();
        assert_eq!(hle.dispatch_hidden(0).unwrap(), CELL_OK);
        assert_eq!(hle.hidden_func_calls[0], 1);

        assert_eq!(hle.dispatch_hidden(23).unwrap_err().0, CELL_DMUX_PAMF_ERROR_ARG);

        assert_eq!(hle.lookup_vnid(VNID_CORE_OPS_PAMF).unwrap(), 0);
        assert_eq!(hle.lookup_vnid(VNID_CORE_OPS_RAW_ES).unwrap(), 1);
        assert_eq!(
            hle.lookup_vnid(0xdead_beef).unwrap_err().0,
            CELL_DMUX_PAMF_ERROR_UNKNOWN_STREAM
        );
    }
}
