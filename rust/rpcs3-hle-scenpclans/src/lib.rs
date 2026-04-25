//! Rust port of `rpcs3/Emu/Cell/Modules/sceNpClans.cpp` — PS3 NP clans
//! (clan management) HLE (39 entries, 1282 lines C++).
//!
//! Foco: lifecycle Init/Term, request registry com Create/Destroy/Abort,
//! 13 critical errors byte-exato, 39 ENTRY_POINTS array.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNpClans";

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpClansInit",
    "sceNpClansTerm",
    "sceNpClansCreateRequest",
    "sceNpClansDestroyRequest",
    "sceNpClansAbortRequest",
    "sceNpClansCreateClan",
    "sceNpClansDisbandClan",
    "sceNpClansGetClanList",
    "sceNpClansGetClanListByNpId",
    "sceNpClansSearchByProfile",
    "sceNpClansSearchByName",
    "sceNpClansGetClanInfo",
    "sceNpClansUpdateClanInfo",
    "sceNpClansGetMemberList",
    "sceNpClansGetMemberInfo",
    "sceNpClansUpdateMemberInfo",
    "sceNpClansChangeMemberRole",
    "sceNpClansGetAutoAcceptStatus",
    "sceNpClansUpdateAutoAcceptStatus",
    "sceNpClansJoinClan",
    "sceNpClansLeaveClan",
    "sceNpClansKickMember",
    "sceNpClansSendInvitation",
    "sceNpClansCancelInvitation",
    "sceNpClansSendInvitationResponse",
    "sceNpClansSendMembershipRequest",
    "sceNpClansCancelMembershipRequest",
    "sceNpClansSendMembershipResponse",
    "sceNpClansGetBlacklist",
    "sceNpClansAddBlacklistEntry",
    "sceNpClansRemoveBlacklistEntry",
    "sceNpClansRetrieveAnnouncements",
    "sceNpClansPostAnnouncement",
    "sceNpClansRemoveAnnouncement",
    "sceNpClansPostChallenge",
    "sceNpClansRetrievePostedChallenges",
    "sceNpClansRemovePostedChallenge",
    "sceNpClansRetrieveChallenges",
    "sceNpClansRemoveChallenge",
];

// Errors byte-exato (10 critical from header 0x80022700+)
pub const SCE_NP_CLANS_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_2701);
pub const SCE_NP_CLANS_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_2702);
pub const SCE_NP_CLANS_ERROR_NOT_SUPPORTED: CellError = CellError(0x8002_2703);
pub const SCE_NP_CLANS_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_2704);
pub const SCE_NP_CLANS_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_2705);
pub const SCE_NP_CLANS_ERROR_EXCEEDS_MAX: CellError = CellError(0x8002_2706);
pub const SCE_NP_CLANS_ERROR_BAD_RESPONSE: CellError = CellError(0x8002_2707);
pub const SCE_NP_CLANS_ERROR_BAD_DATA: CellError = CellError(0x8002_2708);
pub const SCE_NP_CLANS_ERROR_BAD_REQUEST: CellError = CellError(0x8002_2709);
pub const SCE_NP_CLANS_ERROR_INVALID_SIGNATURE: CellError = CellError(0x8002_270A);

pub const MAX_REQUESTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClansState {
    Uninitialized,
    Initialized,
}

impl Default for ClansState {
    fn default() -> Self {
        ClansState::Uninitialized
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    Pending,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClanRequest {
    pub id: u32,
    pub state: RequestState,
}

#[derive(Debug, Default)]
pub struct SceNpClans {
    pub state: ClansState,
    pub requests: Vec<ClanRequest>,
    pub next_request_id: u32,
    pub init_calls: u64,
    pub term_calls: u64,
    pub create_request_calls: u64,
    pub destroy_request_calls: u64,
    pub abort_request_calls: u64,
    pub generic_calls: u64,
}

impl SceNpClans {
    pub fn new() -> Self {
        Self {
            next_request_id: 1,
            ..Default::default()
        }
    }

    pub fn init(&mut self) -> Result<(), CellError> {
        self.init_calls = self.init_calls.saturating_add(1);
        if matches!(self.state, ClansState::Initialized) {
            return Err(SCE_NP_CLANS_ERROR_ALREADY_INITIALIZED);
        }
        self.state = ClansState::Initialized;
        Ok(())
    }

    pub fn term(&mut self) -> Result<(), CellError> {
        self.term_calls = self.term_calls.saturating_add(1);
        if matches!(self.state, ClansState::Uninitialized) {
            return Err(SCE_NP_CLANS_ERROR_NOT_INITIALIZED);
        }
        self.state = ClansState::Uninitialized;
        self.requests.clear();
        Ok(())
    }

    pub fn create_request(&mut self) -> Result<u32, CellError> {
        self.create_request_calls = self.create_request_calls.saturating_add(1);
        if matches!(self.state, ClansState::Uninitialized) {
            return Err(SCE_NP_CLANS_ERROR_NOT_INITIALIZED);
        }
        if self.requests.len() >= MAX_REQUESTS {
            return Err(SCE_NP_CLANS_ERROR_EXCEEDS_MAX);
        }
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.requests.push(ClanRequest {
            id,
            state: RequestState::Pending,
        });
        Ok(id)
    }

    pub fn destroy_request(&mut self, id: u32) -> Result<(), CellError> {
        self.destroy_request_calls = self.destroy_request_calls.saturating_add(1);
        if matches!(self.state, ClansState::Uninitialized) {
            return Err(SCE_NP_CLANS_ERROR_NOT_INITIALIZED);
        }
        let pos = self
            .requests
            .iter()
            .position(|r| r.id == id)
            .ok_or(SCE_NP_CLANS_ERROR_INVALID_ARGUMENT)?;
        self.requests.remove(pos);
        Ok(())
    }

    pub fn abort_request(&mut self, id: u32) -> Result<(), CellError> {
        self.abort_request_calls = self.abort_request_calls.saturating_add(1);
        let req = self
            .requests
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or(SCE_NP_CLANS_ERROR_INVALID_ARGUMENT)?;
        req.state = RequestState::Aborted;
        Ok(())
    }

    pub fn stub_call(&mut self) -> Result<(), CellError> {
        self.generic_calls = self.generic_calls.saturating_add(1);
        if matches!(self.state, ClansState::Uninitialized) {
            return Err(SCE_NP_CLANS_ERROR_NOT_INITIALIZED);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries() {
        assert_eq!(MODULE_NAME, "sceNpClans");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 39);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(SCE_NP_CLANS_ERROR_ALREADY_INITIALIZED.0, 0x8002_2701);
        assert_eq!(SCE_NP_CLANS_ERROR_INVALID_SIGNATURE.0, 0x8002_270A);
    }

    #[test]
    fn init_lifecycle() {
        let mut m = SceNpClans::new();
        m.init().unwrap();
        assert_eq!(m.init(), Err(SCE_NP_CLANS_ERROR_ALREADY_INITIALIZED));
        m.term().unwrap();
        assert_eq!(m.term(), Err(SCE_NP_CLANS_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn create_request_requires_init() {
        let mut m = SceNpClans::new();
        assert_eq!(m.create_request(), Err(SCE_NP_CLANS_ERROR_NOT_INITIALIZED));
        m.init().unwrap();
        let id = m.create_request().unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn create_request_max() {
        let mut m = SceNpClans::new();
        m.init().unwrap();
        for _ in 0..MAX_REQUESTS {
            m.create_request().unwrap();
        }
        assert_eq!(m.create_request(), Err(SCE_NP_CLANS_ERROR_EXCEEDS_MAX));
    }

    #[test]
    fn destroy_unknown_invalid() {
        let mut m = SceNpClans::new();
        m.init().unwrap();
        assert_eq!(m.destroy_request(99), Err(SCE_NP_CLANS_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn abort_marks_state() {
        let mut m = SceNpClans::new();
        m.init().unwrap();
        let id = m.create_request().unwrap();
        m.abort_request(id).unwrap();
        let req = m.requests.iter().find(|r| r.id == id).unwrap();
        assert_eq!(req.state, RequestState::Aborted);
    }

    #[test]
    fn term_clears_requests() {
        let mut m = SceNpClans::new();
        m.init().unwrap();
        m.create_request().unwrap();
        m.create_request().unwrap();
        m.term().unwrap();
        assert!(m.requests.is_empty());
    }

    #[test]
    fn full_clans_lifecycle_smoke() {
        let mut m = SceNpClans::new();
        m.init().unwrap();
        let id = m.create_request().unwrap();
        // Game does clan ops...
        m.stub_call().unwrap();
        m.stub_call().unwrap();
        m.destroy_request(id).unwrap();
        m.term().unwrap();
    }
}
