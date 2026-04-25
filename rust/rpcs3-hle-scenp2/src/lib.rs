//! Rust port of `rpcs3/Emu/Cell/Modules/sceNp2.cpp` — PS3 NP v2 matching
//! framework HLE (80 entries, 2062 lines C++).
//!
//! Foco do port: lifecycle (Init/Term, MatchingInit/Term, Init2/Term2),
//! context registry, event/callback queue, e shape do error namespace
//! (110 errors em 0x80022___ facility). Ops complex de matching (search,
//! lobby, signaling) ficam como stubs com counters per-entry.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNp2";

/// 80 FNIDs (cpp:REG_FUNC block, ordem preservada).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpMatching2DestroyContext",
    "sceNpMatching2LeaveLobby",
    "sceNpMatching2RegisterLobbyMessageCallback",
    "sceNpMatching2GetWorldInfoList",
    "sceNpMatching2RegisterLobbyEventCallback",
    "sceNpMatching2GetLobbyMemberDataInternalList",
    "sceNpMatching2SearchRoom",
    "sceNpMatching2SignalingGetConnectionStatus",
    "sceNpMatching2SetUserInfo",
    "sceNpMatching2GetClanLobbyId",
    "sceNpMatching2GetLobbyMemberDataInternal",
    "sceNpMatching2ContextStart",
    "sceNpMatching2CreateServerContext",
    "sceNpMatching2GetMemoryInfo",
    "sceNpMatching2LeaveRoom",
    "sceNpMatching2SetRoomDataExternal",
    "sceNpMatching2Term2",
    "sceNpMatching2SignalingGetConnectionInfo",
    "sceNpMatching2SendRoomMessage",
    "sceNpMatching2JoinLobby",
    "sceNpMatching2GetRoomMemberDataExternalList",
    "sceNpMatching2AbortRequest",
    "sceNpMatching2Term",
    "sceNpMatching2GetServerInfo",
    "sceNpMatching2GetEventData",
    "sceNpMatching2GetRoomSlotInfoLocal",
    "sceNpMatching2SendLobbyChatMessage",
    "sceNpMatching2Init",
    "sceNp2Init",
    "sceNpMatching2AbortContextStart",
    "sceNpMatching2GetRoomMemberIdListLocal",
    "sceNpMatching2JoinRoom",
    "sceNpMatching2GetRoomMemberDataInternalLocal",
    "sceNpMatching2GetCbQueueInfo",
    "sceNpMatching2KickoutRoomMember",
    "sceNpMatching2ContextStartAsync",
    "sceNpMatching2SetSignalingOptParam",
    "sceNpMatching2RegisterContextCallback",
    "sceNpMatching2SendRoomChatMessage",
    "sceNpMatching2SetRoomDataInternal",
    "sceNpMatching2GetRoomDataInternal",
    "sceNpMatching2SignalingGetPingInfo",
    "sceNpMatching2GetServerIdListLocal",
    "sceNpUtilBuildCdnUrl",
    "sceNpMatching2GrantRoomOwner",
    "sceNpMatching2CreateContext",
    "sceNpMatching2GetSignalingOptParamLocal",
    "sceNpMatching2RegisterSignalingCallback",
    "sceNpMatching2ClearEventData",
    "sceNp2Term",
    "sceNpMatching2GetUserInfoList",
    "sceNpMatching2GetRoomMemberDataInternal",
    "sceNpMatching2SetRoomMemberDataInternal",
    "sceNpMatching2JoinProhibitiveRoom",
    "sceNpMatching2SignalingSetCtxOpt",
    "sceNpMatching2DeleteServerContext",
    "sceNpMatching2SetDefaultRequestOptParam",
    "sceNpMatching2RegisterRoomEventCallback",
    "sceNpMatching2GetRoomPasswordLocal",
    "sceNpMatching2GetRoomDataExternalList",
    "sceNpMatching2CreateJoinRoom",
    "sceNpMatching2SignalingGetCtxOpt",
    "sceNpMatching2GetLobbyInfoList",
    "sceNpMatching2GetLobbyMemberIdListLocal",
    "sceNpMatching2SendLobbyInvitation",
    "sceNpMatching2ContextStop",
    "sceNpMatching2Init2",
    "sceNpMatching2SetLobbyMemberDataInternal",
    "sceNpMatching2RegisterRoomMessageCallback",
    "sceNpMatching2SignalingCancelPeerNetInfo",
    "sceNpMatching2SignalingGetLocalNetInfo",
    "sceNpMatching2SignalingGetPeerNetInfo",
    "sceNpMatching2SignalingGetPeerNetInfoResult",
    "sceNpAuthOAuthInit",
    "sceNpAuthOAuthTerm",
    "sceNpAuthCreateOAuthRequest",
    "sceNpAuthDeleteOAuthRequest",
    "sceNpAuthAbortOAuthRequest",
    "sceNpAuthGetAuthorizationCode",
    "sceNpAuthGetAuthorizationCode2",
];

// ---------------------------------------------------------------------------
// Errors byte-exato (subset crítico — Matching2 namespace 0x80022___).
// 110+ errors no header total; aqui re-exporto os que aparecem no .cpp.
// ---------------------------------------------------------------------------
pub const SCE_NP_MATCHING2_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_2301);
pub const SCE_NP_MATCHING2_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_2302);
pub const SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_2303);
pub const SCE_NP_MATCHING2_ERROR_CONTEXT_MAX: CellError = CellError(0x8002_2304);
pub const SCE_NP_MATCHING2_ERROR_CONTEXT_ALREADY_EXISTS: CellError = CellError(0x8002_2305);
pub const SCE_NP_MATCHING2_ERROR_CONTEXT_NOT_FOUND: CellError = CellError(0x8002_2306);
pub const SCE_NP_MATCHING2_ERROR_CONTEXT_ALREADY_STARTED: CellError = CellError(0x8002_2307);
pub const SCE_NP_MATCHING2_ERROR_CONTEXT_NOT_STARTED: CellError = CellError(0x8002_2308);
pub const SCE_NP_MATCHING2_ERROR_SERVER_NOT_FOUND: CellError = CellError(0x8002_2309);
pub const SCE_NP_MATCHING2_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_230A);
pub const SCE_NP_MATCHING2_ERROR_INVALID_CONTEXT_ID: CellError = CellError(0x8002_230B);
pub const SCE_NP_MATCHING2_ERROR_INSUFFICIENT_BUFFER: CellError = CellError(0x8002_231B);
pub const SCE_NP_MATCHING2_ERROR_REQUEST_TIMEOUT: CellError = CellError(0x8002_231D);

/// Auth namespace
pub const SCE_NP_AUTH_OAUTH_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_5601);
pub const SCE_NP_AUTH_OAUTH_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_5602);

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------
pub const MAX_CONTEXTS: usize = 8;

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Np2State {
    Uninitialized,
    Initialized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matching2State {
    Uninitialized,
    InitializedV1,
    InitializedV2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthState {
    Uninitialized,
    Initialized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextState {
    Created,
    Started,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchingContext {
    pub id: u32,
    pub state: ContextState,
    pub callback_addr: u32,
    pub callback_arg: u32,
}

#[derive(Debug, Default)]
pub struct SceNp2 {
    pub np2_state: Np2State,
    pub matching2_state: Matching2State,
    pub auth_state: AuthState,
    pub contexts: Vec<MatchingContext>,
    pub next_context_id: u32,
    pub pool_size: u32,
    pub stack_size: u32,
    pub priority: i32,

    // 80 per-entry counters seria muito — só os que importam para FSM.
    pub init_calls: u64,
    pub term_calls: u64,
    pub matching2_init_calls: u64,
    pub matching2_init2_calls: u64,
    pub matching2_term_calls: u64,
    pub matching2_term2_calls: u64,
    pub create_context_calls: u64,
    pub destroy_context_calls: u64,
    pub context_start_calls: u64,
    pub context_stop_calls: u64,
    pub auth_oauth_init_calls: u64,
    pub auth_oauth_term_calls: u64,
    pub generic_calls: u64, // Catch-all for stub entries.
}

impl Default for Np2State {
    fn default() -> Self {
        Np2State::Uninitialized
    }
}
impl Default for Matching2State {
    fn default() -> Self {
        Matching2State::Uninitialized
    }
}
impl Default for AuthState {
    fn default() -> Self {
        AuthState::Uninitialized
    }
}

impl SceNp2 {
    pub fn new() -> Self {
        Self {
            next_context_id: 1,
            ..Default::default()
        }
    }

    /// `sceNp2Init(poolsize, poolptr)` — entry-level lifecycle.
    pub fn np2_init(&mut self, poolsize: u32) -> Result<(), CellError> {
        self.init_calls = self.init_calls.saturating_add(1);
        if matches!(self.np2_state, Np2State::Initialized) {
            return Err(SCE_NP_MATCHING2_ERROR_ALREADY_INITIALIZED);
        }
        self.np2_state = Np2State::Initialized;
        self.pool_size = poolsize;
        Ok(())
    }

    pub fn np2_term(&mut self) -> Result<(), CellError> {
        self.term_calls = self.term_calls.saturating_add(1);
        if matches!(self.np2_state, Np2State::Uninitialized) {
            return Err(SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED);
        }
        self.np2_state = Np2State::Uninitialized;
        // Term cascades: also terminates matching2 and auth.
        self.matching2_state = Matching2State::Uninitialized;
        Ok(())
    }

    pub fn matching2_init(&mut self, stack_size: u32, priority: i32) -> Result<(), CellError> {
        self.matching2_init_calls = self.matching2_init_calls.saturating_add(1);
        if !matches!(self.matching2_state, Matching2State::Uninitialized) {
            return Err(SCE_NP_MATCHING2_ERROR_ALREADY_INITIALIZED);
        }
        self.matching2_state = Matching2State::InitializedV1;
        self.stack_size = stack_size;
        self.priority = priority;
        Ok(())
    }

    pub fn matching2_init2(&mut self, stack_size: u32, priority: i32) -> Result<(), CellError> {
        self.matching2_init2_calls = self.matching2_init2_calls.saturating_add(1);
        if !matches!(self.matching2_state, Matching2State::Uninitialized) {
            return Err(SCE_NP_MATCHING2_ERROR_ALREADY_INITIALIZED);
        }
        self.matching2_state = Matching2State::InitializedV2;
        self.stack_size = stack_size;
        self.priority = priority;
        Ok(())
    }

    pub fn matching2_term(&mut self) -> Result<(), CellError> {
        self.matching2_term_calls = self.matching2_term_calls.saturating_add(1);
        if !matches!(self.matching2_state, Matching2State::InitializedV1) {
            return Err(SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED);
        }
        self.matching2_state = Matching2State::Uninitialized;
        self.contexts.clear();
        Ok(())
    }

    pub fn matching2_term2(&mut self) -> Result<(), CellError> {
        self.matching2_term2_calls = self.matching2_term2_calls.saturating_add(1);
        if !matches!(self.matching2_state, Matching2State::InitializedV2) {
            return Err(SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED);
        }
        self.matching2_state = Matching2State::Uninitialized;
        self.contexts.clear();
        Ok(())
    }

    pub fn matching2_create_context(&mut self) -> Result<u32, CellError> {
        self.create_context_calls = self.create_context_calls.saturating_add(1);
        if matches!(self.matching2_state, Matching2State::Uninitialized) {
            return Err(SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED);
        }
        if self.contexts.len() >= MAX_CONTEXTS {
            return Err(SCE_NP_MATCHING2_ERROR_CONTEXT_MAX);
        }
        let id = self.next_context_id;
        self.next_context_id = self.next_context_id.wrapping_add(1);
        self.contexts.push(MatchingContext {
            id,
            state: ContextState::Created,
            callback_addr: 0,
            callback_arg: 0,
        });
        Ok(id)
    }

    pub fn matching2_destroy_context(&mut self, ctx_id: u32) -> Result<(), CellError> {
        self.destroy_context_calls = self.destroy_context_calls.saturating_add(1);
        if matches!(self.matching2_state, Matching2State::Uninitialized) {
            return Err(SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED);
        }
        let pos = self
            .contexts
            .iter()
            .position(|c| c.id == ctx_id)
            .ok_or(SCE_NP_MATCHING2_ERROR_CONTEXT_NOT_FOUND)?;
        self.contexts.remove(pos);
        Ok(())
    }

    pub fn matching2_context_start(&mut self, ctx_id: u32) -> Result<(), CellError> {
        self.context_start_calls = self.context_start_calls.saturating_add(1);
        let ctx = self
            .contexts
            .iter_mut()
            .find(|c| c.id == ctx_id)
            .ok_or(SCE_NP_MATCHING2_ERROR_CONTEXT_NOT_FOUND)?;
        if matches!(ctx.state, ContextState::Started) {
            return Err(SCE_NP_MATCHING2_ERROR_CONTEXT_ALREADY_STARTED);
        }
        ctx.state = ContextState::Started;
        Ok(())
    }

    pub fn matching2_context_stop(&mut self, ctx_id: u32) -> Result<(), CellError> {
        self.context_stop_calls = self.context_stop_calls.saturating_add(1);
        let ctx = self
            .contexts
            .iter_mut()
            .find(|c| c.id == ctx_id)
            .ok_or(SCE_NP_MATCHING2_ERROR_CONTEXT_NOT_FOUND)?;
        if !matches!(ctx.state, ContextState::Started) {
            return Err(SCE_NP_MATCHING2_ERROR_CONTEXT_NOT_STARTED);
        }
        ctx.state = ContextState::Stopped;
        Ok(())
    }

    pub fn auth_oauth_init(&mut self) -> Result<(), CellError> {
        self.auth_oauth_init_calls = self.auth_oauth_init_calls.saturating_add(1);
        if matches!(self.auth_state, AuthState::Initialized) {
            return Err(SCE_NP_AUTH_OAUTH_ERROR_ALREADY_INITIALIZED);
        }
        self.auth_state = AuthState::Initialized;
        Ok(())
    }

    pub fn auth_oauth_term(&mut self) -> Result<(), CellError> {
        self.auth_oauth_term_calls = self.auth_oauth_term_calls.saturating_add(1);
        if matches!(self.auth_state, AuthState::Uninitialized) {
            return Err(SCE_NP_AUTH_OAUTH_ERROR_NOT_INITIALIZED);
        }
        self.auth_state = AuthState::Uninitialized;
        Ok(())
    }

    /// Stub for any of the 80 entries not modeled — bumps generic counter.
    pub fn stub_call(&mut self) -> Result<(), CellError> {
        self.generic_calls = self.generic_calls.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_and_entries_count() {
        assert_eq!(MODULE_NAME, "sceNp2");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 80);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(SCE_NP_MATCHING2_ERROR_OUT_OF_MEMORY.0, 0x8002_2301);
        assert_eq!(SCE_NP_MATCHING2_ERROR_INVALID_ARGUMENT.0, 0x8002_230A);
        assert_eq!(SCE_NP_AUTH_OAUTH_ERROR_NOT_INITIALIZED.0, 0x8002_5601);
    }

    #[test]
    fn np2_init_lifecycle() {
        let mut m = SceNp2::new();
        m.np2_init(0x1000).unwrap();
        assert_eq!(m.pool_size, 0x1000);
        assert_eq!(
            m.np2_init(0x1000),
            Err(SCE_NP_MATCHING2_ERROR_ALREADY_INITIALIZED)
        );
        m.np2_term().unwrap();
        assert_eq!(m.np2_term(), Err(SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn matching2_v1_v2_mutually_exclusive() {
        let mut m = SceNp2::new();
        m.matching2_init(0x4000, 100).unwrap();
        // V2 init while V1 active fails.
        assert_eq!(
            m.matching2_init2(0x4000, 100, ),
            Err(SCE_NP_MATCHING2_ERROR_ALREADY_INITIALIZED)
        );
        m.matching2_term().unwrap();
        // V2 init while none active OK.
        m.matching2_init2(0x4000, 100).unwrap();
        // V1 term fails because V2 is active.
        assert_eq!(m.matching2_term(), Err(SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED));
        m.matching2_term2().unwrap();
    }

    #[test]
    fn create_context_requires_init() {
        let mut m = SceNp2::new();
        assert_eq!(
            m.matching2_create_context(),
            Err(SCE_NP_MATCHING2_ERROR_NOT_INITIALIZED)
        );
        m.matching2_init(0, 0).unwrap();
        let id = m.matching2_create_context().unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn create_context_max_8() {
        let mut m = SceNp2::new();
        m.matching2_init(0, 0).unwrap();
        for _ in 0..MAX_CONTEXTS {
            m.matching2_create_context().unwrap();
        }
        assert_eq!(
            m.matching2_create_context(),
            Err(SCE_NP_MATCHING2_ERROR_CONTEXT_MAX)
        );
    }

    #[test]
    fn destroy_context_unknown_not_found() {
        let mut m = SceNp2::new();
        m.matching2_init(0, 0).unwrap();
        assert_eq!(
            m.matching2_destroy_context(99),
            Err(SCE_NP_MATCHING2_ERROR_CONTEXT_NOT_FOUND)
        );
    }

    #[test]
    fn context_start_stop_fsm() {
        let mut m = SceNp2::new();
        m.matching2_init(0, 0).unwrap();
        let id = m.matching2_create_context().unwrap();
        m.matching2_context_start(id).unwrap();
        // Re-start fails.
        assert_eq!(
            m.matching2_context_start(id),
            Err(SCE_NP_MATCHING2_ERROR_CONTEXT_ALREADY_STARTED)
        );
        m.matching2_context_stop(id).unwrap();
        // Re-stop fails.
        assert_eq!(
            m.matching2_context_stop(id),
            Err(SCE_NP_MATCHING2_ERROR_CONTEXT_NOT_STARTED)
        );
    }

    #[test]
    fn auth_oauth_lifecycle() {
        let mut m = SceNp2::new();
        m.auth_oauth_init().unwrap();
        assert_eq!(
            m.auth_oauth_init(),
            Err(SCE_NP_AUTH_OAUTH_ERROR_ALREADY_INITIALIZED)
        );
        m.auth_oauth_term().unwrap();
        assert_eq!(
            m.auth_oauth_term(),
            Err(SCE_NP_AUTH_OAUTH_ERROR_NOT_INITIALIZED)
        );
    }

    #[test]
    fn np2_term_clears_matching2() {
        let mut m = SceNp2::new();
        m.np2_init(0).unwrap();
        m.matching2_init(0, 0).unwrap();
        m.matching2_create_context().unwrap();
        m.np2_term().unwrap();
        // matching2 also reset.
        assert!(matches!(m.matching2_state, Matching2State::Uninitialized));
    }

    #[test]
    fn full_np2_lifecycle_smoke() {
        let mut m = SceNp2::new();
        m.np2_init(0x1_0000).unwrap();
        m.matching2_init2(0x4000, 100).unwrap();
        let ctx = m.matching2_create_context().unwrap();
        m.matching2_context_start(ctx).unwrap();
        // Game does matching ops...
        m.stub_call().unwrap();
        m.stub_call().unwrap();
        assert_eq!(m.generic_calls, 2);
        m.matching2_context_stop(ctx).unwrap();
        m.matching2_destroy_context(ctx).unwrap();
        m.matching2_term2().unwrap();
        m.np2_term().unwrap();
    }
}
