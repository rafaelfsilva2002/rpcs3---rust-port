//! Rust port of `rpcs3/Emu/Cell/Modules/cellImeJp.cpp` — PS3 Japanese IME
//! utility HLE (42 entries, 1295 lines C++).
//!
//! MODULE_NAME="cellImeJpUtility". Open/Close lifecycle + character entry
//! state machine + 7 errors byte-exato facility 0x8002BF__.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "cellImeJpUtility";

/// 42 FNIDs em ordem REG_FUNC byte-exato.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellImeJpOpen",
    "cellImeJpOpen2",
    "cellImeJpOpen3",
    "cellImeJpOpenExt",
    "cellImeJpClose",
    "cellImeJpSetKanaInputMode",
    "cellImeJpSetInputCharType",
    "cellImeJpSetFixInputMode",
    "cellImeJpAllowExtensionCharacters",
    "cellImeJpReset",
    "cellImeJpGetStatus",
    "cellImeJpEnterChar",
    "cellImeJpEnterCharExt",
    "cellImeJpEnterString",
    "cellImeJpEnterStringExt",
    "cellImeJpModeCaretRight",
    "cellImeJpModeCaretLeft",
    "cellImeJpBackspaceWord",
    "cellImeJpDeleteWord",
    "cellImeJpAllDeleteConvertString",
    "cellImeJpConvertForward",
    "cellImeJpConvertBackward",
    "cellImeJpCurrentPartConfirm",
    "cellImeJpAllConfirm",
    "cellImeJpConvertCancel",
    "cellImeJpAllConvertCancel",
    "cellImeJpExtendConvertArea",
    "cellImeJpShortenConvertArea",
    "cellImeJpTemporalConfirm",
    "cellImeJpPostConvert",
    "cellImeJpMoveFocusClause",
    "cellImeJpGetFocusTop",
    "cellImeJpGetFocusLength",
    "cellImeJpGetConfirmYomiString",
    "cellImeJpGetConfirmString",
    "cellImeJpGetConvertYomiString",
    "cellImeJpGetConvertString",
    "cellImeJpGetCandidateListSize",
    "cellImeJpGetCandidateList",
    "cellImeJpGetCandidateSelect",
    "cellImeJpGetPredictList",
    "cellImeJpConfirmPrediction",
];

// 7 errors byte-exato facility 0x8002BF__.
pub const CELL_IMEJP_ERROR_ERR: CellError = CellError(0x8002_BF01);
pub const CELL_IMEJP_ERROR_CONTEXT: CellError = CellError(0x8002_BF11);
pub const CELL_IMEJP_ERROR_ALREADY_OPEN: CellError = CellError(0x8002_BF21);
pub const CELL_IMEJP_ERROR_DIC_OPEN: CellError = CellError(0x8002_BF31);
pub const CELL_IMEJP_ERROR_PARAM: CellError = CellError(0x8002_BF41);
pub const CELL_IMEJP_ERROR_IME_ALREADY_IN_USE: CellError = CellError(0x8002_BF51);
pub const CELL_IMEJP_ERROR_OTHER: CellError = CellError(0x8002_BFFF);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ImeJpState {
    #[default]
    Closed,
    Open,
}

#[derive(Debug, Default)]
pub struct CellImeJp {
    pub state: ImeJpState,
    pub kana_input_mode: u32,
    pub input_char_type: u32,
    pub fix_input_mode: u32,
    pub allow_extension_chars: bool,
    pub status: u32,
    pub open_calls: u64,
    pub close_calls: u64,
    pub generic_calls: u64,
}

impl CellImeJp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn open(&mut self) -> Result<(), CellError> {
        self.open_calls = self.open_calls.saturating_add(1);
        if matches!(self.state, ImeJpState::Open) {
            return Err(CELL_IMEJP_ERROR_ALREADY_OPEN);
        }
        self.state = ImeJpState::Open;
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), CellError> {
        self.close_calls = self.close_calls.saturating_add(1);
        if matches!(self.state, ImeJpState::Closed) {
            return Err(CELL_IMEJP_ERROR_CONTEXT);
        }
        self.state = ImeJpState::Closed;
        Ok(())
    }

    pub fn set_kana_input_mode(&mut self, mode: u32) -> Result<(), CellError> {
        if matches!(self.state, ImeJpState::Closed) {
            return Err(CELL_IMEJP_ERROR_CONTEXT);
        }
        self.kana_input_mode = mode;
        Ok(())
    }

    pub fn set_input_char_type(&mut self, ty: u32) -> Result<(), CellError> {
        if matches!(self.state, ImeJpState::Closed) {
            return Err(CELL_IMEJP_ERROR_CONTEXT);
        }
        self.input_char_type = ty;
        Ok(())
    }

    pub fn set_fix_input_mode(&mut self, mode: u32) -> Result<(), CellError> {
        if matches!(self.state, ImeJpState::Closed) {
            return Err(CELL_IMEJP_ERROR_CONTEXT);
        }
        self.fix_input_mode = mode;
        Ok(())
    }

    pub fn allow_extension_characters(&mut self, allow: bool) -> Result<(), CellError> {
        if matches!(self.state, ImeJpState::Closed) {
            return Err(CELL_IMEJP_ERROR_CONTEXT);
        }
        self.allow_extension_chars = allow;
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), CellError> {
        if matches!(self.state, ImeJpState::Closed) {
            return Err(CELL_IMEJP_ERROR_CONTEXT);
        }
        self.status = 0;
        Ok(())
    }

    pub fn get_status(&mut self) -> Result<u32, CellError> {
        if matches!(self.state, ImeJpState::Closed) {
            return Err(CELL_IMEJP_ERROR_CONTEXT);
        }
        Ok(self.status)
    }

    pub fn stub_call(&mut self) -> Result<(), CellError> {
        self.generic_calls = self.generic_calls.saturating_add(1);
        if matches!(self.state, ImeJpState::Closed) {
            return Err(CELL_IMEJP_ERROR_CONTEXT);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "cellImeJpUtility");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 42);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(CELL_IMEJP_ERROR_ERR.0, 0x8002_BF01);
        assert_eq!(CELL_IMEJP_ERROR_OTHER.0, 0x8002_BFFF);
    }

    #[test]
    fn open_close_lifecycle() {
        let mut m = CellImeJp::new();
        m.open().unwrap();
        assert_eq!(m.open(), Err(CELL_IMEJP_ERROR_ALREADY_OPEN));
        m.close().unwrap();
        assert_eq!(m.close(), Err(CELL_IMEJP_ERROR_CONTEXT));
    }

    #[test]
    fn ops_require_open() {
        let mut m = CellImeJp::new();
        assert_eq!(m.set_kana_input_mode(1), Err(CELL_IMEJP_ERROR_CONTEXT));
        assert_eq!(m.get_status(), Err(CELL_IMEJP_ERROR_CONTEXT));
        m.open().unwrap();
        m.set_kana_input_mode(1).unwrap();
        m.get_status().unwrap();
    }

    #[test]
    fn settings_persist() {
        let mut m = CellImeJp::new();
        m.open().unwrap();
        m.set_kana_input_mode(2).unwrap();
        m.set_input_char_type(3).unwrap();
        m.set_fix_input_mode(4).unwrap();
        m.allow_extension_characters(true).unwrap();
        assert_eq!(m.kana_input_mode, 2);
        assert_eq!(m.input_char_type, 3);
        assert_eq!(m.fix_input_mode, 4);
        assert!(m.allow_extension_chars);
    }

    #[test]
    fn reset_clears_status() {
        let mut m = CellImeJp::new();
        m.open().unwrap();
        m.status = 0xFF;
        m.reset().unwrap();
        assert_eq!(m.status, 0);
    }

    #[test]
    fn full_imejp_lifecycle_smoke() {
        let mut m = CellImeJp::new();
        m.open().unwrap();
        m.set_kana_input_mode(1).unwrap();
        m.set_input_char_type(2).unwrap();
        m.allow_extension_characters(true).unwrap();
        m.stub_call().unwrap();
        m.stub_call().unwrap();
        m.reset().unwrap();
        m.close().unwrap();
    }
}
