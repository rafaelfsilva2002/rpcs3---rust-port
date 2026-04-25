//! Rust port of `rpcs3/Emu/Cell/Modules/sceNpTus.cpp` — PS3 NP Title User
//! Storage HLE (62 entries, 1478 lines C++).
//!
//! TUS = Title User Storage. Per-game cloud storage para variables (counters),
//! data blobs, slots multi-user/multi-slot. Constants: MAX_CTX=32,
//! MAX_SLOT_PER_TRANS=64, MAX_USER_PER_TRANS=101, MAX_SELECTED_FRIENDS=100,
//! DATA_INFO_MAX=384.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNpTus";

/// 62 FNIDs — REG_FUNC ordem.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpTusInit",
    "sceNpTusTerm",
    "sceNpTusCreateTitleCtx",
    "sceNpTusDestroyTitleCtx",
    "sceNpTusCreateTransactionCtx",
    "sceNpTusDestroyTransactionCtx",
    "sceNpTusSetTimeout",
    "sceNpTusAbortTransaction",
    "sceNpTusWaitAsync",
    "sceNpTusPollAsync",
    "sceNpTusSetMultiSlotVariable",
    "sceNpTusSetMultiSlotVariableVUser",
    "sceNpTusSetMultiSlotVariableAsync",
    "sceNpTusSetMultiSlotVariableVUserAsync",
    "sceNpTusGetMultiSlotVariable",
    "sceNpTusGetMultiSlotVariableVUser",
    "sceNpTusGetMultiSlotVariableAsync",
    "sceNpTusGetMultiSlotVariableVUserAsync",
    "sceNpTusGetMultiUserVariable",
    "sceNpTusGetMultiUserVariableVUser",
    "sceNpTusGetMultiUserVariableAsync",
    "sceNpTusGetMultiUserVariableVUserAsync",
    "sceNpTusGetFriendsVariable",
    "sceNpTusGetFriendsVariableAsync",
    "sceNpTusAddAndGetVariable",
    "sceNpTusAddAndGetVariableVUser",
    "sceNpTusAddAndGetVariableAsync",
    "sceNpTusAddAndGetVariableVUserAsync",
    "sceNpTusTryAndSetVariable",
    "sceNpTusTryAndSetVariableVUser",
    "sceNpTusTryAndSetVariableAsync",
    "sceNpTusTryAndSetVariableVUserAsync",
    "sceNpTusDeleteMultiSlotVariable",
    "sceNpTusDeleteMultiSlotVariableVUser",
    "sceNpTusDeleteMultiSlotVariableAsync",
    "sceNpTusDeleteMultiSlotVariableVUserAsync",
    "sceNpTusSetData",
    "sceNpTusSetDataVUser",
    "sceNpTusSetDataAsync",
    "sceNpTusSetDataVUserAsync",
    "sceNpTusGetData",
    "sceNpTusGetDataVUser",
    "sceNpTusGetDataAsync",
    "sceNpTusGetDataVUserAsync",
    "sceNpTusGetMultiSlotDataStatus",
    "sceNpTusGetMultiSlotDataStatusVUser",
    "sceNpTusGetMultiSlotDataStatusAsync",
    "sceNpTusGetMultiSlotDataStatusVUserAsync",
    "sceNpTusGetMultiUserDataStatus",
    "sceNpTusGetMultiUserDataStatusVUser",
    "sceNpTusGetMultiUserDataStatusAsync",
    "sceNpTusGetMultiUserDataStatusVUserAsync",
    "sceNpTusGetFriendsDataStatus",
    "sceNpTusGetFriendsDataStatusAsync",
    "sceNpTusDeleteMultiSlotData",
    "sceNpTusDeleteMultiSlotDataVUser",
    "sceNpTusDeleteMultiSlotDataAsync",
    "sceNpTusDeleteMultiSlotDataVUserAsync",
    "sceNpTssGetData",
    "sceNpTssGetDataAsync",
    "sceNpTssGetDataNoLimit",
    "sceNpTssGetDataNoLimitAsync",
];

// Constants byte-exato sceNpTus.h.
pub const SCE_NP_TUS_DATA_INFO_MAX_SIZE: u32 = 384;
pub const SCE_NP_TUS_MAX_CTX_NUM: usize = 32;
pub const SCE_NP_TUS_MAX_SLOT_NUM_PER_TRANS: usize = 64;
pub const SCE_NP_TUS_MAX_USER_NUM_PER_TRANS: usize = 101;
pub const SCE_NP_TUS_MAX_SELECTED_FRIENDS_NUM: usize = 100;

// 6 OPE_TYPE values (TryAndSet comparison ops).
pub const SCE_NP_TUS_OPETYPE_EQUAL: u32 = 1;
pub const SCE_NP_TUS_OPETYPE_NOT_EQUAL: u32 = 2;
pub const SCE_NP_TUS_OPETYPE_GREATER_THAN: u32 = 3;
pub const SCE_NP_TUS_OPETYPE_GREATER_OR_EQUAL: u32 = 4;
pub const SCE_NP_TUS_OPETYPE_LESS_THAN: u32 = 5;
pub const SCE_NP_TUS_OPETYPE_LESS_OR_EQUAL: u32 = 6;

// 4 SORT_TYPE for variables.
pub const SCE_NP_TUS_VARIABLE_SORTTYPE_DESCENDING_DATE: u32 = 1;
pub const SCE_NP_TUS_VARIABLE_SORTTYPE_ASCENDING_DATE: u32 = 2;
pub const SCE_NP_TUS_VARIABLE_SORTTYPE_DESCENDING_VALUE: u32 = 3;
pub const SCE_NP_TUS_VARIABLE_SORTTYPE_ASCENDING_VALUE: u32 = 4;

// Placeholder errors (sceNpTus.h não declara errors próprios, usa genéricos).
pub const SCE_NP_TUS_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_2F01);
pub const SCE_NP_TUS_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_2F02);
pub const SCE_NP_TUS_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_2F03);
pub const SCE_NP_TUS_ERROR_CTX_MAX: CellError = CellError(0x8002_2F04);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TusState {
    Uninitialized,
    Initialized,
}
impl Default for TusState {
    fn default() -> Self {
        TusState::Uninitialized
    }
}

#[derive(Debug, Default)]
pub struct SceNpTus {
    pub state: TusState,
    pub title_contexts: Vec<u32>,
    pub transaction_contexts: Vec<u32>,
    pub timeout_us: u64,
    pub next_ctx_id: u32,
    pub init_calls: u64,
    pub term_calls: u64,
    pub create_title_ctx_calls: u64,
    pub destroy_title_ctx_calls: u64,
    pub create_trans_ctx_calls: u64,
    pub destroy_trans_ctx_calls: u64,
    pub abort_calls: u64,
    pub generic_calls: u64,
}

impl SceNpTus {
    pub fn new() -> Self {
        Self {
            next_ctx_id: 1,
            ..Default::default()
        }
    }

    pub fn init(&mut self) -> Result<(), CellError> {
        self.init_calls = self.init_calls.saturating_add(1);
        if matches!(self.state, TusState::Initialized) {
            return Err(SCE_NP_TUS_ERROR_ALREADY_INITIALIZED);
        }
        self.state = TusState::Initialized;
        Ok(())
    }

    pub fn term(&mut self) -> Result<(), CellError> {
        self.term_calls = self.term_calls.saturating_add(1);
        if matches!(self.state, TusState::Uninitialized) {
            return Err(SCE_NP_TUS_ERROR_NOT_INITIALIZED);
        }
        self.state = TusState::Uninitialized;
        self.title_contexts.clear();
        self.transaction_contexts.clear();
        Ok(())
    }

    pub fn create_title_ctx(&mut self) -> Result<u32, CellError> {
        self.create_title_ctx_calls = self.create_title_ctx_calls.saturating_add(1);
        if matches!(self.state, TusState::Uninitialized) {
            return Err(SCE_NP_TUS_ERROR_NOT_INITIALIZED);
        }
        if self.title_contexts.len() >= SCE_NP_TUS_MAX_CTX_NUM {
            return Err(SCE_NP_TUS_ERROR_CTX_MAX);
        }
        let id = self.next_ctx_id;
        self.next_ctx_id = self.next_ctx_id.wrapping_add(1);
        self.title_contexts.push(id);
        Ok(id)
    }

    pub fn destroy_title_ctx(&mut self, id: u32) -> Result<(), CellError> {
        self.destroy_title_ctx_calls = self.destroy_title_ctx_calls.saturating_add(1);
        let pos = self
            .title_contexts
            .iter()
            .position(|c| *c == id)
            .ok_or(SCE_NP_TUS_ERROR_INVALID_ARGUMENT)?;
        self.title_contexts.remove(pos);
        Ok(())
    }

    pub fn create_transaction_ctx(&mut self) -> Result<u32, CellError> {
        self.create_trans_ctx_calls = self.create_trans_ctx_calls.saturating_add(1);
        if matches!(self.state, TusState::Uninitialized) {
            return Err(SCE_NP_TUS_ERROR_NOT_INITIALIZED);
        }
        let id = self.next_ctx_id;
        self.next_ctx_id = self.next_ctx_id.wrapping_add(1);
        self.transaction_contexts.push(id);
        Ok(id)
    }

    pub fn destroy_transaction_ctx(&mut self, id: u32) -> Result<(), CellError> {
        self.destroy_trans_ctx_calls = self.destroy_trans_ctx_calls.saturating_add(1);
        let pos = self
            .transaction_contexts
            .iter()
            .position(|c| *c == id)
            .ok_or(SCE_NP_TUS_ERROR_INVALID_ARGUMENT)?;
        self.transaction_contexts.remove(pos);
        Ok(())
    }

    pub fn set_timeout(&mut self, us: u64) -> Result<(), CellError> {
        if matches!(self.state, TusState::Uninitialized) {
            return Err(SCE_NP_TUS_ERROR_NOT_INITIALIZED);
        }
        self.timeout_us = us;
        Ok(())
    }

    pub fn abort_transaction(&mut self, _id: u32) -> Result<(), CellError> {
        self.abort_calls = self.abort_calls.saturating_add(1);
        Ok(())
    }

    pub fn stub_call(&mut self) -> Result<(), CellError> {
        self.generic_calls = self.generic_calls.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sceNpTus");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 62);
    }

    #[test]
    fn constants_byte_exact() {
        assert_eq!(SCE_NP_TUS_DATA_INFO_MAX_SIZE, 384);
        assert_eq!(SCE_NP_TUS_MAX_CTX_NUM, 32);
        assert_eq!(SCE_NP_TUS_MAX_SLOT_NUM_PER_TRANS, 64);
        assert_eq!(SCE_NP_TUS_MAX_USER_NUM_PER_TRANS, 101);
        assert_eq!(SCE_NP_TUS_MAX_SELECTED_FRIENDS_NUM, 100);
    }

    #[test]
    fn ope_types_byte_exact() {
        assert_eq!(SCE_NP_TUS_OPETYPE_EQUAL, 1);
        assert_eq!(SCE_NP_TUS_OPETYPE_LESS_OR_EQUAL, 6);
    }

    #[test]
    fn init_lifecycle() {
        let mut m = SceNpTus::new();
        m.init().unwrap();
        assert_eq!(m.init(), Err(SCE_NP_TUS_ERROR_ALREADY_INITIALIZED));
        m.term().unwrap();
        assert_eq!(m.term(), Err(SCE_NP_TUS_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn title_ctx_max() {
        let mut m = SceNpTus::new();
        m.init().unwrap();
        for _ in 0..SCE_NP_TUS_MAX_CTX_NUM {
            m.create_title_ctx().unwrap();
        }
        assert_eq!(m.create_title_ctx(), Err(SCE_NP_TUS_ERROR_CTX_MAX));
    }

    #[test]
    fn destroy_unknown_invalid() {
        let mut m = SceNpTus::new();
        m.init().unwrap();
        assert_eq!(m.destroy_title_ctx(99), Err(SCE_NP_TUS_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn timeout_set_persists() {
        let mut m = SceNpTus::new();
        assert!(m.set_timeout(1000).is_err());
        m.init().unwrap();
        m.set_timeout(5000).unwrap();
        assert_eq!(m.timeout_us, 5000);
    }

    #[test]
    fn full_tus_lifecycle_smoke() {
        let mut m = SceNpTus::new();
        m.init().unwrap();
        let title = m.create_title_ctx().unwrap();
        let trans = m.create_transaction_ctx().unwrap();
        m.set_timeout(30_000_000).unwrap();
        m.stub_call().unwrap();
        m.abort_transaction(trans).unwrap();
        m.destroy_transaction_ctx(trans).unwrap();
        m.destroy_title_ctx(title).unwrap();
        m.term().unwrap();
    }
}
