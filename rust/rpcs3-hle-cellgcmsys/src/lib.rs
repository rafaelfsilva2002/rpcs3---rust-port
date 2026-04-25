//! `rpcs3-hle-cellgcmsys` — Rust port of `rpcs3/Emu/Cell/Modules/cellGcmSys.cpp`.
//!
//! PS3 GCM (Graphics Command Manager) system PRX HLE — the main RSX command-
//! buffer / display-buffer / tile / zcull binding surface that games talk to
//! before any draw happens. The C++ implementation drives the RSX command
//! ring, FIFO, vsync callbacks, and IO mapping. Porting the RSX runtime into
//! Rust is out of scope for now; this crate freezes the ABI contract:
//!
//! - `CellGcmError` codes (header:6..14, facility 0x80210___).
//! - 100 ENTRY_POINTS preserved in REG_FUNC order from cpp:1515..1617.
//! - Per-entry dispatch counters for telemetry.
#![no_std]
extern crate alloc;

use rpcs3_emu::CellError;
use rpcs3_emu_types as rpcs3_emu;

pub const CELL_OK: u32 = 0;

// CellGcmError (cpp header:6..14). Note: FAILURE (0xff) is out of sequence.
pub const CELL_GCM_ERROR_FAILURE: u32 = 0x8021_00ff;
pub const CELL_GCM_ERROR_NO_IO_PAGE_TABLE: u32 = 0x8021_0001;
pub const CELL_GCM_ERROR_INVALID_ENUM: u32 = 0x8021_0002;
pub const CELL_GCM_ERROR_INVALID_VALUE: u32 = 0x8021_0003;
pub const CELL_GCM_ERROR_INVALID_ALIGNMENT: u32 = 0x8021_0004;
pub const CELL_GCM_ERROR_ADDRESS_OVERWRAP: u32 = 0x8021_0005;

/// REG_FUNC order from cpp:1515..1617, byte-exact. 100 entries including
/// `_cellGcm*` private helpers and `cellGcmGpad*` capture APIs.
pub const ENTRY_POINTS: &[&str] = &[
    "cellGcmGetCurrentField",
    "cellGcmGetLabelAddress",
    "cellGcmGetNotifyDataAddress",
    "_cellGcmFunc12",
    "cellGcmGetReport",
    "cellGcmGetReportDataAddress",
    "cellGcmGetReportDataAddressLocation",
    "cellGcmGetReportDataLocation",
    "cellGcmGetTimeStamp",
    "cellGcmGetTimeStampLocation",
    "cellGcmGetControlRegister",
    "cellGcmGetDefaultCommandWordSize",
    "cellGcmGetDefaultSegmentWordSize",
    "cellGcmInitDefaultFifoMode",
    "cellGcmSetDefaultFifoSize",
    "cellGcmBindTile",
    "cellGcmBindZcull",
    "cellGcmDumpGraphicsError",
    "cellGcmGetConfiguration",
    "cellGcmGetDisplayBufferByFlipIndex",
    "cellGcmGetFlipStatus",
    "cellGcmGetFlipStatus2",
    "cellGcmGetLastFlipTime",
    "cellGcmGetLastFlipTime2",
    "cellGcmGetLastSecondVTime",
    "cellGcmGetTiledPitchSize",
    "cellGcmGetVBlankCount",
    "cellGcmGetVBlankCount2",
    "cellGcmSysGetLastVBlankTime",
    "_cellGcmFunc1",
    "_cellGcmFunc15",
    "_cellGcmInitBody",
    "cellGcmInitSystemMode",
    "cellGcmResetFlipStatus",
    "cellGcmResetFlipStatus2",
    "cellGcmSetDebugOutputLevel",
    "cellGcmSetDisplayBuffer",
    "cellGcmSetFlip",
    "cellGcmSetFlipHandler",
    "cellGcmSetFlipHandler2",
    "cellGcmSetFlipImmediate",
    "cellGcmSetFlipImmediate2",
    "cellGcmSetFlipMode",
    "cellGcmSetFlipMode2",
    "cellGcmSetFlipStatus",
    "cellGcmSetFlipStatus2",
    "cellGcmSetGraphicsHandler",
    "cellGcmSetPrepareFlip",
    "cellGcmSetQueueHandler",
    "cellGcmSetSecondVFrequency",
    "cellGcmSetSecondVHandler",
    "cellGcmSetTileInfo",
    "cellGcmSetUserHandler",
    "cellGcmSetUserCommand",
    "cellGcmSetVBlankFrequency",
    "cellGcmSetVBlankHandler",
    "cellGcmSetWaitFlip",
    "cellGcmSetWaitFlipUnsafe",
    "cellGcmSetZcull",
    "cellGcmSortRemapEaIoAddress",
    "cellGcmUnbindTile",
    "cellGcmUnbindZcull",
    "cellGcmGetTileInfo",
    "cellGcmGetZcullInfo",
    "cellGcmGetDisplayInfo",
    "cellGcmGetCurrentDisplayBufferId",
    "cellGcmSetInvalidateTile",
    "cellGcmTerminate",
    "cellGcmAddressToOffset",
    "cellGcmGetMaxIoMapSize",
    "cellGcmGetOffsetTable",
    "cellGcmIoOffsetToAddress",
    "cellGcmMapEaIoAddress",
    "cellGcmMapEaIoAddressWithFlags",
    "cellGcmMapLocalMemory",
    "cellGcmMapMainMemory",
    "cellGcmReserveIoMapSize",
    "cellGcmUnmapEaIoAddress",
    "cellGcmUnmapIoAddress",
    "cellGcmUnreserveIoMapSize",
    "cellGcmInitCursor",
    "cellGcmSetCursorEnable",
    "cellGcmSetCursorDisable",
    "cellGcmSetCursorImageOffset",
    "cellGcmSetCursorPosition",
    "cellGcmUpdateCursor",
    "cellGcmSetDefaultCommandBuffer",
    "cellGcmSetDefaultCommandBufferAndSegmentWordSize",
    "_cellGcmSetFlipCommand",
    "_cellGcmSetFlipCommand2",
    "_cellGcmSetFlipCommandWithWaitLabel",
    "cellGcmSetTile",
    "_cellGcmFunc2",
    "_cellGcmFunc3",
    "_cellGcmFunc4",
    "_cellGcmFunc13",
    "_cellGcmFunc38",
    "cellGcmGpadGetStatus",
    "cellGcmGpadNotifyCaptureSurface",
    "cellGcmGpadCaptureSnapshot",
];

/// Contract-only dispatcher. The real RSX flip/queue/tile surfaces route
/// into the GPU emulator layer; here we only enforce the symbol contract.
pub struct GcmSysHle {
    pub call_counts: [u64; 100],
}

impl GcmSysHle {
    pub const fn new() -> Self {
        Self { call_counts: [0; 100] }
    }

    pub fn dispatch(&mut self, index: usize) -> Result<u32, CellError> {
        if index >= ENTRY_POINTS.len() {
            return Err(CellError(CELL_GCM_ERROR_INVALID_VALUE));
        }
        self.call_counts[index] = self.call_counts[index].saturating_add(1);
        Ok(CELL_OK)
    }

    pub fn dispatch_by_name(&mut self, name: &str) -> Result<u32, CellError> {
        match ENTRY_POINTS.iter().position(|&e| e == name) {
            Some(i) => self.dispatch(i),
            None => Err(CellError(CELL_GCM_ERROR_INVALID_ENUM)),
        }
    }
}

impl Default for GcmSysHle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_GCM_ERROR_FAILURE, 0x8021_00ff);
        assert_eq!(CELL_GCM_ERROR_NO_IO_PAGE_TABLE, 0x8021_0001);
        assert_eq!(CELL_GCM_ERROR_INVALID_ENUM, 0x8021_0002);
        assert_eq!(CELL_GCM_ERROR_INVALID_VALUE, 0x8021_0003);
        assert_eq!(CELL_GCM_ERROR_INVALID_ALIGNMENT, 0x8021_0004);
        assert_eq!(CELL_GCM_ERROR_ADDRESS_OVERWRAP, 0x8021_0005);
    }

    #[test]
    fn entry_count_matches_cpp() {
        assert_eq!(ENTRY_POINTS.len(), 100, "REG_FUNC count cpp:1515..1617");
    }

    #[test]
    fn first_and_last_entry() {
        assert_eq!(ENTRY_POINTS[0], "cellGcmGetCurrentField");
        assert_eq!(ENTRY_POINTS[99], "cellGcmGpadCaptureSnapshot");
    }

    #[test]
    fn private_helpers_preserved() {
        // Spot-check non-public helpers kept verbatim — games call these
        // through PRX FNID lookups.
        assert!(ENTRY_POINTS.contains(&"_cellGcmFunc12"));
        assert!(ENTRY_POINTS.contains(&"_cellGcmInitBody"));
        assert!(ENTRY_POINTS.contains(&"_cellGcmSetFlipCommand"));
        assert!(ENTRY_POINTS.contains(&"_cellGcmFunc38"));
    }

    #[test]
    fn dispatch_returns_ok_and_counts() {
        let mut hle = GcmSysHle::new();
        assert_eq!(hle.dispatch(0).unwrap(), CELL_OK);
        assert_eq!(hle.call_counts[0], 1);
        hle.dispatch_by_name("cellGcmSetFlip").unwrap();
        let idx = ENTRY_POINTS.iter().position(|&e| e == "cellGcmSetFlip").unwrap();
        assert_eq!(hle.call_counts[idx], 1);
    }

    #[test]
    fn dispatch_oob_uses_invalid_value() {
        let mut hle = GcmSysHle::new();
        assert_eq!(hle.dispatch(100).unwrap_err().0, CELL_GCM_ERROR_INVALID_VALUE);
    }

    #[test]
    fn dispatch_unknown_name_uses_invalid_enum() {
        let mut hle = GcmSysHle::new();
        assert_eq!(hle.dispatch_by_name("nope").unwrap_err().0, CELL_GCM_ERROR_INVALID_ENUM);
    }
}
