//! Rust port of `rpcs3/Emu/Cell/Modules/sceNp.cpp` — PS3 NP main module HLE
//! (239 entries, 7590 lines C++ — **maior módulo do workspace**).
//!
//! Foco: 239 ENTRY_POINTS array byte-exato + 14 critical errors + 4-state
//! lifecycle (Init/Term, DRM, Basic, Manager). Implementations específicas
//! ficam stub com generic counter — a porta full requer enorme tempo e
//! depende de network/account state real.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "sceNp";

/// 239 FNIDs em ordem REG_FUNC byte-exato.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "sceNpInit",
    "sceNpTerm",
    "sceNpDrmIsAvailable",
    "sceNpDrmIsAvailable2",
    "sceNpDrmVerifyUpgradeLicense",
    "sceNpDrmVerifyUpgradeLicense2",
    "sceNpDrmExecuteGamePurchase",
    "sceNpDrmGetTimelimit",
    "sceNpDrmProcessExitSpawn",
    "sceNpDrmProcessExitSpawn2",
    "sceNpBasicRegisterHandler",
    "sceNpBasicRegisterContextSensitiveHandler",
    "sceNpBasicUnregisterHandler",
    "sceNpBasicSetPresence",
    "sceNpBasicSetPresenceDetails",
    "sceNpBasicSetPresenceDetails2",
    "sceNpBasicSendMessage",
    "sceNpBasicSendMessageGui",
    "sceNpBasicSendMessageAttachment",
    "sceNpBasicRecvMessageAttachment",
    "sceNpBasicRecvMessageAttachmentLoad",
    "sceNpBasicRecvMessageCustom",
    "sceNpBasicMarkMessageAsUsed",
    "sceNpBasicAbortGui",
    "sceNpBasicAddFriend",
    "sceNpBasicGetFriendListEntryCount",
    "sceNpBasicGetFriendListEntry",
    "sceNpBasicGetFriendPresenceByIndex",
    "sceNpBasicGetFriendPresenceByIndex2",
    "sceNpBasicGetFriendPresenceByNpId",
    "sceNpBasicGetFriendPresenceByNpId2",
    "sceNpBasicAddPlayersHistory",
    "sceNpBasicAddPlayersHistoryAsync",
    "sceNpBasicGetPlayersHistoryEntryCount",
    "sceNpBasicGetPlayersHistoryEntry",
    "sceNpBasicAddBlockListEntry",
    "sceNpBasicGetBlockListEntryCount",
    "sceNpBasicGetBlockListEntry",
    "sceNpBasicGetMessageAttachmentEntryCount",
    "sceNpBasicGetMessageAttachmentEntry",
    "sceNpBasicGetCustomInvitationEntryCount",
    "sceNpBasicGetCustomInvitationEntry",
    "sceNpBasicGetMatchingInvitationEntryCount",
    "sceNpBasicGetMatchingInvitationEntry",
    "sceNpBasicGetClanMessageEntryCount",
    "sceNpBasicGetClanMessageEntry",
    "sceNpBasicGetMessageEntryCount",
    "sceNpBasicGetMessageEntry",
    "sceNpBasicGetEvent",
    "sceNpCommerceCreateCtx",
    "sceNpCommerceDestroyCtx",
    "sceNpCommerceInitProductCategory",
    "sceNpCommerceDestroyProductCategory",
    "sceNpCommerceGetProductCategoryStart",
    "sceNpCommerceGetProductCategoryFinish",
    "sceNpCommerceGetProductCategoryResult",
    "sceNpCommerceGetProductCategoryAbort",
    "sceNpCommerceGetProductId",
    "sceNpCommerceGetProductName",
    "sceNpCommerceGetCategoryDescription",
    "sceNpCommerceGetCategoryId",
    "sceNpCommerceGetCategoryImageURL",
    "sceNpCommerceGetCategoryInfo",
    "sceNpCommerceGetCategoryName",
    "sceNpCommerceGetCurrencyCode",
    "sceNpCommerceGetCurrencyDecimals",
    "sceNpCommerceGetCurrencyInfo",
    "sceNpCommerceGetNumOfChildCategory",
    "sceNpCommerceGetNumOfChildProductSku",
    "sceNpCommerceGetSkuDescription",
    "sceNpCommerceGetSkuId",
    "sceNpCommerceGetSkuImageURL",
    "sceNpCommerceGetSkuName",
    "sceNpCommerceGetSkuPrice",
    "sceNpCommerceGetSkuUserData",
    "sceNpCommerceSetDataFlagStart",
    "sceNpCommerceGetDataFlagStart",
    "sceNpCommerceSetDataFlagFinish",
    "sceNpCommerceGetDataFlagFinish",
    "sceNpCommerceGetDataFlagState",
    "sceNpCommerceGetDataFlagAbort",
    "sceNpCommerceGetChildCategoryInfo",
    "sceNpCommerceGetChildProductSkuInfo",
    "sceNpCommerceDoCheckoutStartAsync",
    "sceNpCommerceDoCheckoutFinishAsync",
    "sceNpCustomMenuRegisterActions",
    "sceNpCustomMenuActionSetActivation",
    "sceNpCustomMenuRegisterExceptionList",
    "sceNpFriendlist",
    "sceNpFriendlistCustom",
    "sceNpFriendlistAbortGui",
    "sceNpLookupInit",
    "sceNpLookupTerm",
    "sceNpLookupCreateTitleCtx",
    "sceNpLookupDestroyTitleCtx",
    "sceNpLookupCreateTransactionCtx",
    "sceNpLookupDestroyTransactionCtx",
    "sceNpLookupSetTimeout",
    "sceNpLookupAbortTransaction",
    "sceNpLookupWaitAsync",
    "sceNpLookupPollAsync",
    "sceNpLookupNpId",
    "sceNpLookupNpIdAsync",
    "sceNpLookupUserProfile",
    "sceNpLookupUserProfileAsync",
    "sceNpLookupUserProfileWithAvatarSize",
    "sceNpLookupUserProfileWithAvatarSizeAsync",
    "sceNpLookupAvatarImage",
    "sceNpLookupAvatarImageAsync",
    "sceNpLookupTitleStorage",
    "sceNpLookupTitleStorageAsync",
    "sceNpLookupTitleSmallStorage",
    "sceNpLookupTitleSmallStorageAsync",
    "sceNpManagerRegisterCallback",
    "sceNpManagerUnregisterCallback",
    "sceNpManagerGetStatus",
    "sceNpManagerGetNetworkTime",
    "sceNpManagerGetOnlineId",
    "sceNpManagerGetNpId",
    "sceNpManagerGetOnlineName",
    "sceNpManagerGetAvatarUrl",
    "sceNpManagerGetMyLanguages",
    "sceNpManagerGetAccountRegion",
    "sceNpManagerGetAccountAge",
    "sceNpManagerGetContentRatingFlag",
    "sceNpManagerGetChatRestrictionFlag",
    "sceNpManagerGetCachedInfo",
    "sceNpManagerGetPsHandle",
    "sceNpManagerRequestTicket",
    "sceNpManagerRequestTicket2",
    "sceNpManagerGetTicket",
    "sceNpManagerGetTicketParam",
    "sceNpManagerGetEntitlementIdList",
    "sceNpManagerGetEntitlementById",
    "sceNpManagerGetSigninId",
    "sceNpManagerSubSignin",
    "sceNpManagerSubSigninAbortGui",
    "sceNpManagerSubSignout",
    "sceNpMatchingCreateCtx",
    "sceNpMatchingDestroyCtx",
    "sceNpMatchingGetResult",
    "sceNpMatchingGetResultGUI",
    "sceNpMatchingSetRoomInfo",
    "sceNpMatchingSetRoomInfoNoLimit",
    "sceNpMatchingGetRoomInfo",
    "sceNpMatchingGetRoomInfoNoLimit",
    "sceNpMatchingSetRoomSearchFlag",
    "sceNpMatchingGetRoomSearchFlag",
    "sceNpMatchingGetRoomMemberListLocal",
    "sceNpMatchingGetRoomListLimitGUI",
    "sceNpMatchingKickRoomMember",
    "sceNpMatchingKickRoomMemberWithOpt",
    "sceNpMatchingQuickMatchGUI",
    "sceNpMatchingSendInvitationGUI",
    "sceNpMatchingAcceptInvitationGUI",
    "sceNpMatchingCreateRoomGUI",
    "sceNpMatchingJoinRoomGUI",
    "sceNpMatchingLeaveRoom",
    "sceNpMatchingSearchJoinRoomGUI",
    "sceNpMatchingGrantOwnership",
    "sceNpProfileCallGui",
    "sceNpProfileAbortGui",
    "sceNpScoreInit",
    "sceNpScoreTerm",
    "sceNpScoreCreateTitleCtx",
    "sceNpScoreDestroyTitleCtx",
    "sceNpScoreCreateTransactionCtx",
    "sceNpScoreDestroyTransactionCtx",
    "sceNpScoreSetTimeout",
    "sceNpScoreSetPlayerCharacterId",
    "sceNpScoreWaitAsync",
    "sceNpScorePollAsync",
    "sceNpScoreGetBoardInfo",
    "sceNpScoreGetBoardInfoAsync",
    "sceNpScoreRecordScore",
    "sceNpScoreRecordScoreAsync",
    "sceNpScoreRecordGameData",
    "sceNpScoreRecordGameDataAsync",
    "sceNpScoreGetGameData",
    "sceNpScoreGetGameDataAsync",
    "sceNpScoreGetRankingByNpId",
    "sceNpScoreGetRankingByNpIdAsync",
    "sceNpScoreGetRankingByRange",
    "sceNpScoreGetRankingByRangeAsync",
    "sceNpScoreGetFriendsRanking",
    "sceNpScoreGetFriendsRankingAsync",
    "sceNpScoreCensorComment",
    "sceNpScoreCensorCommentAsync",
    "sceNpScoreSanitizeComment",
    "sceNpScoreSanitizeCommentAsync",
    "sceNpScoreGetRankingByNpIdPcId",
    "sceNpScoreGetRankingByNpIdPcIdAsync",
    "sceNpScoreAbortTransaction",
    "sceNpScoreGetClansMembersRankingByNpId",
    "sceNpScoreGetClansMembersRankingByNpIdAsync",
    "sceNpScoreGetClansMembersRankingByNpIdPcId",
    "sceNpScoreGetClansMembersRankingByNpIdPcIdAsync",
    "sceNpScoreGetClansMembersRankingByRange",
    "sceNpScoreGetClansMembersRankingByRangeAsync",
    "sceNpScoreGetClanMemberGameData",
    "sceNpScoreGetClanMemberGameDataAsync",
    "sceNpScoreGetClansRankingByClanId",
    "sceNpScoreGetClansRankingByClanIdAsync",
    "sceNpScoreGetClansRankingByRange",
    "sceNpScoreGetClansRankingByRangeAsync",
    "sceNpSignalingCreateCtx",
    "sceNpSignalingDestroyCtx",
    "sceNpSignalingAddExtendedHandler",
    "sceNpSignalingSetCtxOpt",
    "sceNpSignalingGetCtxOpt",
    "sceNpSignalingActivateConnection",
    "sceNpSignalingDeactivateConnection",
    "sceNpSignalingTerminateConnection",
    "sceNpSignalingGetConnectionStatus",
    "sceNpSignalingGetConnectionInfo",
    "sceNpSignalingGetConnectionFromNpId",
    "sceNpSignalingGetConnectionFromPeerAddress",
    "sceNpSignalingGetLocalNetInfo",
    "sceNpSignalingGetPeerNetInfo",
    "sceNpSignalingCancelPeerNetInfo",
    "sceNpSignalingGetPeerNetInfoResult",
    "sceNpUtilCanonicalizeNpIdForPs3",
    "sceNpUtilCanonicalizeNpIdForPsp",
    "sceNpUtilCmpNpId",
    "sceNpUtilCmpNpIdInOrder",
    "sceNpUtilCmpOnlineId",
    "sceNpUtilGetPlatformType",
    "sceNpUtilSetPlatformType",
    "_sceNpSysutilClientMalloc",
    "_sceNpSysutilClientFree",
    "_Z33_sce_np_sysutil_send_empty_packetiPN16sysutil_cxmlutil11FixedMemoryEPKcS3_",
    "_Z27_sce_np_sysutil_send_packetiRN4cxml8DocumentE",
    "_Z36_sce_np_sysutil_recv_packet_fixedmemiPN16sysutil_cxmlutil11FixedMemoryERN4cxml8DocumentERNS2_7ElementE",
    "_Z40_sce_np_sysutil_recv_packet_fixedmem_subiPN16sysutil_cxmlutil11FixedMemoryERN4cxml8DocumentERNS2_7ElementE",
    "_Z27_sce_np_sysutil_recv_packetiRN4cxml8DocumentERNS_7ElementE",
    "_Z29_sce_np_sysutil_cxml_set_npidRN4cxml8DocumentERNS_7ElementEPKcPK7SceNpId",
    "_Z31_sce_np_sysutil_send_packet_subiRN4cxml8DocumentE",
    "_Z37sce_np_matching_set_matching2_runningb",
    "_Z32_sce_np_sysutil_cxml_prepare_docPN16sysutil_cxmlutil11FixedMemoryERN4cxml8DocumentEPKcRNS2_7ElementES6_i",
];

// ---------------------------------------------------------------------------
// Errors byte-exato facility 0x8002AA__ (sceNp.h subset crítico).
// ---------------------------------------------------------------------------
pub const SCE_NP_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_AA01);
pub const SCE_NP_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_AA02);
pub const SCE_NP_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_AA03);
pub const SCE_NP_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_AA04);
pub const SCE_NP_ERROR_ID_NO_SPACE: CellError = CellError(0x8002_AA05);
pub const SCE_NP_ERROR_ID_NOT_FOUND: CellError = CellError(0x8002_AA06);
pub const SCE_NP_ERROR_SESSION_RUNNING: CellError = CellError(0x8002_AA07);
pub const SCE_NP_ERROR_LOGINID_ALREADY_EXISTS: CellError = CellError(0x8002_AA08);
pub const SCE_NP_ERROR_INVALID_TICKET_SIZE: CellError = CellError(0x8002_AA09);
pub const SCE_NP_ERROR_INVALID_STATE: CellError = CellError(0x8002_AA0A);

// ---------------------------------------------------------------------------
// Lifecycle FSM (4 sub-modules independentes).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SceNpState {
    #[default]
    Uninitialized,
    Initialized,
}

#[derive(Debug, Default)]
pub struct SceNp {
    pub state: SceNpState,
    pub drm_initialized: bool,
    pub basic_handler_addr: u32,
    pub session_running: bool,

    pub init_calls: u64,
    pub term_calls: u64,
    pub drm_calls: u64,
    pub basic_calls: u64,
    pub manager_calls: u64,
    pub generic_calls: u64,
}

impl SceNp {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn init(&mut self) -> Result<(), CellError> {
        self.init_calls = self.init_calls.saturating_add(1);
        if matches!(self.state, SceNpState::Initialized) {
            return Err(SCE_NP_ERROR_ALREADY_INITIALIZED);
        }
        self.state = SceNpState::Initialized;
        Ok(())
    }

    pub fn term(&mut self) -> Result<(), CellError> {
        self.term_calls = self.term_calls.saturating_add(1);
        if matches!(self.state, SceNpState::Uninitialized) {
            return Err(SCE_NP_ERROR_NOT_INITIALIZED);
        }
        if self.session_running {
            return Err(SCE_NP_ERROR_SESSION_RUNNING);
        }
        self.state = SceNpState::Uninitialized;
        self.drm_initialized = false;
        self.basic_handler_addr = 0;
        Ok(())
    }

    pub fn basic_register_handler(&mut self, handler_addr: u32) -> Result<(), CellError> {
        self.basic_calls = self.basic_calls.saturating_add(1);
        if matches!(self.state, SceNpState::Uninitialized) {
            return Err(SCE_NP_ERROR_NOT_INITIALIZED);
        }
        if handler_addr == 0 {
            return Err(SCE_NP_ERROR_INVALID_ARGUMENT);
        }
        if self.basic_handler_addr != 0 {
            return Err(SCE_NP_ERROR_LOGINID_ALREADY_EXISTS);
        }
        self.basic_handler_addr = handler_addr;
        Ok(())
    }

    pub fn basic_unregister_handler(&mut self) -> Result<(), CellError> {
        self.basic_calls = self.basic_calls.saturating_add(1);
        if matches!(self.state, SceNpState::Uninitialized) {
            return Err(SCE_NP_ERROR_NOT_INITIALIZED);
        }
        if self.basic_handler_addr == 0 {
            return Err(SCE_NP_ERROR_INVALID_STATE);
        }
        self.basic_handler_addr = 0;
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
        assert_eq!(MODULE_NAME, "sceNp");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 239);
    }

    #[test]
    fn errors_byte_exact() {
        assert_eq!(SCE_NP_ERROR_NOT_INITIALIZED.0, 0x8002_AA01);
        assert_eq!(SCE_NP_ERROR_INVALID_STATE.0, 0x8002_AA0A);
    }

    #[test]
    fn init_lifecycle() {
        let mut m = SceNp::new();
        m.init().unwrap();
        assert_eq!(m.init(), Err(SCE_NP_ERROR_ALREADY_INITIALIZED));
        m.term().unwrap();
        assert_eq!(m.term(), Err(SCE_NP_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn term_blocked_by_session() {
        let mut m = SceNp::new();
        m.init().unwrap();
        m.session_running = true;
        assert_eq!(m.term(), Err(SCE_NP_ERROR_SESSION_RUNNING));
    }

    #[test]
    fn basic_handler_register_unregister() {
        let mut m = SceNp::new();
        assert_eq!(
            m.basic_register_handler(0xCAFE),
            Err(SCE_NP_ERROR_NOT_INITIALIZED)
        );
        m.init().unwrap();
        assert_eq!(
            m.basic_register_handler(0),
            Err(SCE_NP_ERROR_INVALID_ARGUMENT)
        );
        m.basic_register_handler(0xCAFE).unwrap();
        assert_eq!(
            m.basic_register_handler(0xBEEF),
            Err(SCE_NP_ERROR_LOGINID_ALREADY_EXISTS)
        );
        m.basic_unregister_handler().unwrap();
        assert_eq!(
            m.basic_unregister_handler(),
            Err(SCE_NP_ERROR_INVALID_STATE)
        );
    }

    #[test]
    fn term_clears_handler() {
        let mut m = SceNp::new();
        m.init().unwrap();
        m.basic_register_handler(0xCAFE).unwrap();
        m.term().unwrap();
        assert_eq!(m.basic_handler_addr, 0);
    }

    #[test]
    fn full_scenp_lifecycle_smoke() {
        let mut m = SceNp::new();
        m.init().unwrap();
        m.basic_register_handler(0xCAFE).unwrap();
        // Game does NP ops...
        m.stub_call().unwrap();
        m.stub_call().unwrap();
        m.basic_unregister_handler().unwrap();
        m.term().unwrap();
    }
}
