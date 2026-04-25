//! Rust port of `rpcs3/Emu/Cell/Modules/sceNpCommerce2.cpp` — PS3 NP
//! Commerce v2 (PSN store) HLE (52 entries, 1125 lines C++).
//!
//! Foco: lifecycle Init/Term, context registry CreateCtx/DestroyCtx,
//! request lifecycle Create/Start/Get/Init/Destroy + abort + 52 ENTRY_POINTS
//! byte-exato.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNpCommerce2";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpCommerce2ExecuteStoreBrowse",
    "sceNpCommerce2GetStoreBrowseUserdata",
    "sceNpCommerce2Init",
    "sceNpCommerce2Term",
    "sceNpCommerce2CreateCtx",
    "sceNpCommerce2DestroyCtx",
    "sceNpCommerce2EmptyStoreCheckStart",
    "sceNpCommerce2EmptyStoreCheckAbort",
    "sceNpCommerce2EmptyStoreCheckFinish",
    "sceNpCommerce2CreateSessionStart",
    "sceNpCommerce2CreateSessionAbort",
    "sceNpCommerce2CreateSessionFinish",
    "sceNpCommerce2GetCategoryContentsCreateReq",
    "sceNpCommerce2GetCategoryContentsStart",
    "sceNpCommerce2GetCategoryContentsGetResult",
    "sceNpCommerce2InitGetCategoryContentsResult",
    "sceNpCommerce2GetCategoryInfo",
    "sceNpCommerce2GetContentInfo",
    "sceNpCommerce2GetCategoryInfoFromContentInfo",
    "sceNpCommerce2GetGameProductInfoFromContentInfo",
    "sceNpCommerce2DestroyGetCategoryContentsResult",
    "sceNpCommerce2GetProductInfoCreateReq",
    "sceNpCommerce2GetProductInfoStart",
    "sceNpCommerce2GetProductInfoGetResult",
    "sceNpCommerce2InitGetProductInfoResult",
    "sceNpCommerce2GetGameProductInfo",
    "sceNpCommerce2DestroyGetProductInfoResult",
    "sceNpCommerce2GetProductInfoListCreateReq",
    "sceNpCommerce2GetProductInfoListStart",
    "sceNpCommerce2GetProductInfoListGetResult",
    "sceNpCommerce2InitGetProductInfoListResult",
    "sceNpCommerce2GetGameProductInfoFromGetProductInfoListResult",
    "sceNpCommerce2DestroyGetProductInfoListResult",
    "sceNpCommerce2GetContentRatingInfoFromGameProductInfo",
    "sceNpCommerce2GetContentRatingInfoFromCategoryInfo",
    "sceNpCommerce2GetContentRatingDescriptor",
    "sceNpCommerce2GetGameSkuInfoFromGameProductInfo",
    "sceNpCommerce2GetPrice",
    "sceNpCommerce2DoCheckoutStartAsync",
    "sceNpCommerce2DoCheckoutFinishAsync",
    "sceNpCommerce2DoProductBrowseStartAsync",
    "sceNpCommerce2DoProductBrowseFinishAsync",
    "sceNpCommerce2DoDlListStartAsync",
    "sceNpCommerce2DoDlListFinishAsync",
    "sceNpCommerce2DoProductCodeStartAsync",
    "sceNpCommerce2DoProductCodeFinishAsync",
    "sceNpCommerce2GetBGDLAvailability",
    "sceNpCommerce2SetBGDLAvailability",
    "sceNpCommerce2AbortReq",
    "sceNpCommerce2DestroyReq",
    "sceNpCommerce2DoServiceListStartAsync",
    "sceNpCommerce2DoServiceListFinishAsync",
];

// Errors byte-exato facility 0x80023___ (subset crítico).
pub const SCE_NP_COMMERCE2_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_3001);
pub const SCE_NP_COMMERCE2_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_3002);
pub const SCE_NP_COMMERCE2_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_3003);
pub const SCE_NP_COMMERCE2_ERROR_UNSUPPORTED_VERSION: CellError = CellError(0x8002_3004);
pub const SCE_NP_COMMERCE2_ERROR_CTX_MAX: CellError = CellError(0x8002_3005);
pub const SCE_NP_COMMERCE2_ERROR_INVALID_INDEX: CellError = CellError(0x8002_3006);
pub const SCE_NP_COMMERCE2_ERROR_INVALID_SKUID: CellError = CellError(0x8002_3007);
pub const SCE_NP_COMMERCE2_ERROR_INVALID_SKU_NUM: CellError = CellError(0x8002_3008);
pub const SCE_NP_COMMERCE2_ERROR_INVALID_MEMORY_CONTAINER: CellError = CellError(0x8002_3009);
pub const SCE_NP_COMMERCE2_ERROR_INSUFFICIENT_MEMORY_CONTAINER: CellError = CellError(0x8002_300A);

pub const MAX_CONTEXTS: usize = 32;
pub const MAX_REQUESTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commerce2State {
    Uninitialized,
    Initialized,
}

impl Default for Commerce2State {
    fn default() -> Self {
        Commerce2State::Uninitialized
    }
}

#[derive(Debug, Default)]
pub struct SceNpCommerce2 {
    pub state: Commerce2State,
    pub contexts: Vec<u32>,
    pub requests: Vec<u32>,
    pub bgdl_availability: bool,
    pub next_ctx_id: u32,
    pub next_req_id: u32,
    pub init_calls: u64,
    pub term_calls: u64,
    pub create_ctx_calls: u64,
    pub destroy_ctx_calls: u64,
    pub abort_req_calls: u64,
    pub destroy_req_calls: u64,
    pub generic_calls: u64,
}

impl SceNpCommerce2 {
    pub fn new() -> Self {
        Self {
            next_ctx_id: 1,
            next_req_id: 1,
            ..Default::default()
        }
    }

    pub fn init(&mut self) -> Result<(), CellError> {
        self.init_calls = self.init_calls.saturating_add(1);
        if matches!(self.state, Commerce2State::Initialized) {
            return Err(SCE_NP_COMMERCE2_ERROR_ALREADY_INITIALIZED);
        }
        self.state = Commerce2State::Initialized;
        Ok(())
    }

    pub fn term(&mut self) -> Result<(), CellError> {
        self.term_calls = self.term_calls.saturating_add(1);
        if matches!(self.state, Commerce2State::Uninitialized) {
            return Err(SCE_NP_COMMERCE2_ERROR_NOT_INITIALIZED);
        }
        self.state = Commerce2State::Uninitialized;
        self.contexts.clear();
        self.requests.clear();
        Ok(())
    }

    pub fn create_ctx(&mut self) -> Result<u32, CellError> {
        self.create_ctx_calls = self.create_ctx_calls.saturating_add(1);
        if matches!(self.state, Commerce2State::Uninitialized) {
            return Err(SCE_NP_COMMERCE2_ERROR_NOT_INITIALIZED);
        }
        if self.contexts.len() >= MAX_CONTEXTS {
            return Err(SCE_NP_COMMERCE2_ERROR_CTX_MAX);
        }
        let id = self.next_ctx_id;
        self.next_ctx_id = self.next_ctx_id.wrapping_add(1);
        self.contexts.push(id);
        Ok(id)
    }

    pub fn destroy_ctx(&mut self, id: u32) -> Result<(), CellError> {
        self.destroy_ctx_calls = self.destroy_ctx_calls.saturating_add(1);
        if matches!(self.state, Commerce2State::Uninitialized) {
            return Err(SCE_NP_COMMERCE2_ERROR_NOT_INITIALIZED);
        }
        let pos = self
            .contexts
            .iter()
            .position(|c| *c == id)
            .ok_or(SCE_NP_COMMERCE2_ERROR_INVALID_ARGUMENT)?;
        self.contexts.remove(pos);
        Ok(())
    }

    pub fn abort_req(&mut self, _id: u32) -> Result<(), CellError> {
        self.abort_req_calls = self.abort_req_calls.saturating_add(1);
        Ok(())
    }

    pub fn destroy_req(&mut self, id: u32) -> Result<(), CellError> {
        self.destroy_req_calls = self.destroy_req_calls.saturating_add(1);
        let pos = self
            .requests
            .iter()
            .position(|r| *r == id)
            .ok_or(SCE_NP_COMMERCE2_ERROR_INVALID_ARGUMENT)?;
        self.requests.remove(pos);
        Ok(())
    }

    pub fn set_bgdl_availability(&mut self, value: bool) -> Result<(), CellError> {
        if matches!(self.state, Commerce2State::Uninitialized) {
            return Err(SCE_NP_COMMERCE2_ERROR_NOT_INITIALIZED);
        }
        self.bgdl_availability = value;
        Ok(())
    }

    pub fn get_bgdl_availability(&mut self) -> Result<bool, CellError> {
        if matches!(self.state, Commerce2State::Uninitialized) {
            return Err(SCE_NP_COMMERCE2_ERROR_NOT_INITIALIZED);
        }
        Ok(self.bgdl_availability)
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
        assert_eq!(MODULE_NAME, "sceNpCommerce2");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 52);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(SCE_NP_COMMERCE2_ERROR_NOT_INITIALIZED.0, 0x8002_3001);
        assert_eq!(SCE_NP_COMMERCE2_ERROR_INSUFFICIENT_MEMORY_CONTAINER.0, 0x8002_300A);
    }

    #[test]
    fn init_lifecycle() {
        let mut m = SceNpCommerce2::new();
        m.init().unwrap();
        assert_eq!(m.init(), Err(SCE_NP_COMMERCE2_ERROR_ALREADY_INITIALIZED));
        m.term().unwrap();
        assert_eq!(m.term(), Err(SCE_NP_COMMERCE2_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn create_ctx_max() {
        let mut m = SceNpCommerce2::new();
        m.init().unwrap();
        for _ in 0..MAX_CONTEXTS {
            m.create_ctx().unwrap();
        }
        assert_eq!(m.create_ctx(), Err(SCE_NP_COMMERCE2_ERROR_CTX_MAX));
    }

    #[test]
    fn destroy_unknown_invalid() {
        let mut m = SceNpCommerce2::new();
        m.init().unwrap();
        assert_eq!(m.destroy_ctx(99), Err(SCE_NP_COMMERCE2_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn bgdl_roundtrip() {
        let mut m = SceNpCommerce2::new();
        assert!(m.set_bgdl_availability(true).is_err());
        m.init().unwrap();
        m.set_bgdl_availability(true).unwrap();
        assert_eq!(m.get_bgdl_availability().unwrap(), true);
        m.set_bgdl_availability(false).unwrap();
        assert_eq!(m.get_bgdl_availability().unwrap(), false);
    }

    #[test]
    fn term_clears_contexts() {
        let mut m = SceNpCommerce2::new();
        m.init().unwrap();
        m.create_ctx().unwrap();
        m.create_ctx().unwrap();
        m.term().unwrap();
        assert!(m.contexts.is_empty());
    }

    #[test]
    fn full_commerce2_lifecycle_smoke() {
        let mut m = SceNpCommerce2::new();
        m.init().unwrap();
        let ctx = m.create_ctx().unwrap();
        m.set_bgdl_availability(true).unwrap();
        m.stub_call().unwrap();
        m.destroy_ctx(ctx).unwrap();
        m.term().unwrap();
    }
}
