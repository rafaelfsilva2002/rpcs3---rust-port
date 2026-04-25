//! Rust port of `rpcs3/Emu/Cell/Modules/cellSysutilAvc.cpp` — PS3 Audio/Video
//! Chat (AVC) sysutil sub-module.
//!
//! Upstream registers 20 entries on the `cellSysutil` PRX and keeps a
//! singleton `avc_settings { avc_cb, avc_cb_arg, req_id_cnt }`. Only five
//! entries actually *do* something meaningful:
//!
//! * `cellSysutilAvcLoadAsync` — validates media/quality args, rejects if a
//!   callback is already registered (`ALREADY_INITIALIZED`), stashes the
//!   callback and queues a `LOAD_SUCCEEDED` deferred event.
//! * `cellSysutilAvcUnloadAsync` — queues `UNLOAD_SUCCEEDED`; when the
//!   scheduler actually delivers that event the callback is cleared.
//! * `cellSysutilAvcJoinRequest` — validates room/req pointers, queues
//!   `JOIN_SUCCEEDED`.
//! * `cellSysutilAvcByeRequest` — queues `BYE_SUCCEEDED`.
//! * `cellSysutilAvcSetSpeakerVolumeLevel` / `SetLayoutMode` — range checks.
//!
//! Everything else only null-checks its pointers. Request IDs come from an
//! atomic counter starting at 0.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::mem::{size_of, take};

use rpcs3_emu_types::CellError;

/// Upstream registers entries into the `cellSysutil` PRX.
pub const HOST_MODULE_NAME: &str = "cellSysutil";
pub const SUBMODULE_NAME: &str = "cellSysutilAvc";

/// 20 FNIDs in the exact `REG_FUNC` order (cpp:353-372).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellSysutilAvcByeRequest",
    "cellSysutilAvcCancelByeRequest",
    "cellSysutilAvcCancelJoinRequest",
    "cellSysutilAvcEnumPlayers",
    "cellSysutilAvcGetAttribute",
    "cellSysutilAvcGetLayoutMode",
    "cellSysutilAvcGetShowStatus",
    "cellSysutilAvcGetSpeakerVolumeLevel",
    "cellSysutilAvcGetVideoMuting",
    "cellSysutilAvcGetVoiceMuting",
    "cellSysutilAvcHidePanel",
    "cellSysutilAvcJoinRequest",
    "cellSysutilAvcLoadAsync",
    "cellSysutilAvcSetAttribute",
    "cellSysutilAvcSetLayoutMode",
    "cellSysutilAvcSetSpeakerVolumeLevel",
    "cellSysutilAvcSetVideoMuting",
    "cellSysutilAvcSetVoiceMuting",
    "cellSysutilAvcShowPanel",
    "cellSysutilAvcUnloadAsync",
];

// ---------------------------------------------------------------------------
// Errors — byte-exact `CellAvcError` (0x8002_B701..B710, with 708/709/70C/70F
// explicitly left as gaps in the C++ enum).
// ---------------------------------------------------------------------------

pub const CELL_AVC_ERROR_UNKNOWN: CellError = CellError(0x8002_B701);
pub const CELL_AVC_ERROR_NOT_SUPPORTED: CellError = CellError(0x8002_B702);
pub const CELL_AVC_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_B703);
pub const CELL_AVC_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_B704);
pub const CELL_AVC_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_B705);
pub const CELL_AVC_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_B706);
pub const CELL_AVC_ERROR_BAD_ID: CellError = CellError(0x8002_B707);
pub const CELL_AVC_ERROR_INVALID_STATUS: CellError = CellError(0x8002_B70A);
pub const CELL_AVC_ERROR_TIMEOUT: CellError = CellError(0x8002_B70B);
pub const CELL_AVC_ERROR_NO_SESSION: CellError = CellError(0x8002_B70D);
pub const CELL_AVC_ERROR_INCOMPATIBLE_PROTOCOL: CellError = CellError(0x8002_B70E);
pub const CELL_AVC_ERROR_PEER_UNREACHABLE: CellError = CellError(0x8002_B710);

// ---------------------------------------------------------------------------
// Enums from the header.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysutilAvcTransitionType {
    Linear = 0x0000_0000,
    Slowdown = 0x0000_0001,
    FastUp = 0x0000_0002,
    Angular = 0x0000_0003,
    Exponent = 0x0000_0004,
    None = 0xFFFF_FFFF,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysutilAvcWindowZorderMode {
    ForwardMost = 0x0000_0002,
    BehindMost = 0x0000_0003,
}

pub const CELL_AVC_REQUEST_ID_SYSTEM_EVENT: u32 = 0x0000_0000;
pub const CELL_SYSUTIL_AVC_VIDEO_MEMORY_SIZE: u32 = 26 * 1024 * 1024;
pub const CELL_SYSUTIL_AVC_VOICE_MEMORY_SIZE: u32 = 8 * 1024 * 1024;
pub const CELL_SYSUTIL_AVC_EXTRA_MEMORY_SIZE_FOR_SHARING_VIDEO_BUFFER: u32 = 2 * 1024 * 1024;
pub const CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION: u32 = 100;

/// Event IDs delivered through the AVC callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysutilAvcEvent {
    LoadSucceeded = 0x0000_0001,
    LoadFailed = 0x0000_0002,
    UnloadSucceeded = 0x0000_0003,
    UnloadFailed = 0x0000_0004,
    JoinSucceeded = 0x0000_0005,
    JoinFailed = 0x0000_0006,
    ByeSucceeded = 0x0000_0007,
    ByeFailed = 0x0000_0008,
    SystemNewMemberJoined = 0x1000_0001,
    SystemMemberLeft = 0x1000_0002,
    SystemSessionEstablished = 0x1000_0003,
    SystemSessionCannotEstablished = 0x1000_0004,
    SystemSessionDisconnected = 0x1000_0005,
    SystemVoiceDetected = 0x1000_0006,
    SystemMicDetected = 0x1000_0007,
    SystemCameraDetected = 0x1000_0008,
}

impl CellSysutilAvcEvent {
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// `CellSysUtilAvcAttribute`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysUtilAvcAttribute {
    DefaultTransitionType = 0x0000_0001,
    DefaultTransitionDuration = 0x0000_0002,
    DefaultInitialShowStatus = 0x0000_0003,
    VoiceDetectEventType = 0x0000_0004,
    VoiceDetectIntervalTime = 0x0000_0005,
    VoiceDetectSignalLevel = 0x0000_0006,
    RoomPrivilegeType = 0x0000_0007,
    VideoMaxBitrate = 0x0000_0008,
}

/// `CellSysutilAvcLayoutMode` — accepted range is [LEFT..=BOTTOM].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysutilAvcLayoutMode {
    Left = 0x0000_0000,
    Right = 0x0000_0001,
    Top = 0x0000_0002,
    Bottom = 0x0000_0003,
}

/// `CellSysUtilAvcMediaType` — only VoiceChat/VideoChat accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysUtilAvcMediaType {
    VoiceChat = 0x0000_0001,
    VideoChat = 0x0000_0002,
}

/// `CellSysUtilAvcVideoQuality` — only DEFAULT(1) is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysUtilAvcVideoQuality {
    Default = 0x0000_0001,
}

/// `CellSysUtilAvcVoiceQuality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysUtilAvcVoiceQuality {
    Default = 0x0000_0001,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysutilAvcRoomPrivilegeType {
    NoAutoGrant = 0x0000_0000,
    AutoGrant = 0x0000_0001,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysutilAvcVoiceDetectEventType {
    Signal = 0x0000_0001,
    Speak = 0x0000_0002,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysutilAvcVoiceDetectSpeakData {
    Stop = 0x0000_0000,
    Start = 0x0000_0001,
}

// ---------------------------------------------------------------------------
// Wire structs.
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilAvcOptionParam {
    pub avc_option_param_version: i32,
    pub sharing_video_buffer: u8,
    pub _pad: [u8; 3],
    pub max_players: i32,
}
const _: () = assert!(size_of::<CellSysutilAvcOptionParam>() == 12);

/// Mirror of `SceNpId` used by `CellSysutilAvcVoiceDetectData` — PS3 uses a
/// 20-byte fixed buffer (16-byte handle + term + 3-byte reserved).
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SceNpId {
    pub handle: [u8; 16],
    pub opt: [u8; 8],
    pub reserved: [u8; 8],
}
const _: () = assert!(size_of::<SceNpId>() == 32);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilAvcVoiceDetectData {
    pub npid: SceNpId,
    pub data: i32,
}
const _: () = assert!(size_of::<CellSysutilAvcVoiceDetectData>() == 36);

/// Mirror of `SceNpRoomId` — 16-byte opaque room identifier.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SceNpRoomId {
    pub data: [u8; 16],
}
const _: () = assert!(size_of::<SceNpRoomId>() == 16);

pub type CellSysutilAvcRequestId = u32;
pub type CellSysUtilAvcEventParam = u64;

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

/// Volume range upstream enforces — `[0, 10]` inclusive.
pub const MIN_VOLUME: i32 = 0;
pub const MAX_VOLUME: i32 = 10;

/// Layout mode max value (`CELL_SYSUTIL_AVC_LAYOUT_BOTTOM`).
pub const MAX_LAYOUT_MODE: u32 = 0x0000_0003;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingCallback {
    pub request_id: CellSysutilAvcRequestId,
    pub event_id: CellSysutilAvcEvent,
    pub param: CellSysUtilAvcEventParam,
}

#[derive(Debug, Default)]
pub struct SysutilAvc {
    avc_cb: u32,
    avc_cb_arg: u32,
    req_id_cnt: u32,
    pending: Vec<PendingCallback>,
    /// Whether the most-recent UnloadAsync's deferred callback has already
    /// been *consumed* by `deliver_pending`. Mirrors the cpp behaviour where
    /// delivering `UNLOAD_SUCCEEDED` clears the stashed callback.
    unload_consumed: bool,

    // Per-entry counters — 20 entries.
    pub bye_request_calls: u64,
    pub cancel_bye_request_calls: u64,
    pub cancel_join_request_calls: u64,
    pub enum_players_calls: u64,
    pub get_attribute_calls: u64,
    pub get_layout_mode_calls: u64,
    pub get_show_status_calls: u64,
    pub get_speaker_volume_level_calls: u64,
    pub get_video_muting_calls: u64,
    pub get_voice_muting_calls: u64,
    pub hide_panel_calls: u64,
    pub join_request_calls: u64,
    pub load_async_calls: u64,
    pub set_attribute_calls: u64,
    pub set_layout_mode_calls: u64,
    pub set_speaker_volume_level_calls: u64,
    pub set_video_muting_calls: u64,
    pub set_voice_muting_calls: u64,
    pub show_panel_calls: u64,
    pub unload_async_calls: u64,
}

impl SysutilAvc {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn avc_cb(&self) -> u32 {
        self.avc_cb
    }

    pub fn avc_cb_arg(&self) -> u32 {
        self.avc_cb_arg
    }

    pub fn next_req_id(&self) -> u32 {
        self.req_id_cnt
    }

    pub fn pending(&self) -> &[PendingCallback] {
        &self.pending
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_loaded(&self) -> bool {
        self.avc_cb != 0
    }

    fn alloc_request_id(&mut self) -> CellSysutilAvcRequestId {
        let id = self.req_id_cnt;
        self.req_id_cnt = self.req_id_cnt.wrapping_add(1);
        id
    }

    fn enqueue(
        &mut self,
        request_id: CellSysutilAvcRequestId,
        event_id: CellSysutilAvcEvent,
        param: CellSysUtilAvcEventParam,
    ) {
        self.pending.push(PendingCallback {
            request_id,
            event_id,
            param,
        });
    }

    /// Drains the deferred callback queue. Mirrors upstream semantics: when an
    /// `UNLOAD_SUCCEEDED` event is *delivered* (not just queued), the stashed
    /// callback/arg are cleared.
    pub fn deliver_pending(&mut self) -> Vec<PendingCallback> {
        let drained = take(&mut self.pending);
        for ev in &drained {
            if ev.event_id == CellSysutilAvcEvent::UnloadSucceeded {
                self.avc_cb = 0;
                self.avc_cb_arg = 0;
                self.unload_consumed = true;
            }
        }
        drained
    }

    // -- entries ---------------------------------------------------------

    /// `cellSysutilAvcByeRequest(request_id)`. Queues `BYE_SUCCEEDED`.
    pub fn bye_request(
        &mut self,
        request_id: Option<&mut CellSysutilAvcRequestId>,
    ) -> Result<(), CellError> {
        self.bye_request_calls = self.bye_request_calls.saturating_add(1);
        let rid_slot = request_id.ok_or(CELL_AVC_ERROR_INVALID_ARGUMENT)?;
        let rid = self.alloc_request_id();
        *rid_slot = rid;
        self.enqueue(rid, CellSysutilAvcEvent::ByeSucceeded, 0);
        Ok(())
    }

    pub fn cancel_bye_request(
        &mut self,
        request_id: Option<&CellSysutilAvcRequestId>,
    ) -> Result<(), CellError> {
        self.cancel_bye_request_calls = self.cancel_bye_request_calls.saturating_add(1);
        if request_id.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn cancel_join_request(
        &mut self,
        request_id: Option<&CellSysutilAvcRequestId>,
    ) -> Result<(), CellError> {
        self.cancel_join_request_calls = self.cancel_join_request_calls.saturating_add(1);
        if request_id.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    /// `cellSysutilAvcEnumPlayers(players_id, players_num)`. If `players_id`
    /// is null, writes participant count (always 0 in upstream stub). If
    /// `players_id` is non-null, it's a fill request — we do nothing since no
    /// real participants exist.
    pub fn enum_players(
        &mut self,
        players_id: Option<&mut [SceNpId]>,
        players_num: Option<&mut i32>,
    ) -> Result<(), CellError> {
        self.enum_players_calls = self.enum_players_calls.saturating_add(1);
        let num_slot = players_num.ok_or(CELL_AVC_ERROR_INVALID_ARGUMENT)?;
        if players_id.is_none() {
            *num_slot = 0;
        }
        Ok(())
    }

    pub fn get_attribute(
        &mut self,
        _attr_id: CellSysUtilAvcAttribute,
        param: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.get_attribute_calls = self.get_attribute_calls.saturating_add(1);
        if param.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_layout_mode(
        &mut self,
        layout: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.get_layout_mode_calls = self.get_layout_mode_calls.saturating_add(1);
        if layout.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_show_status(
        &mut self,
        is_visible: Option<&mut bool>,
    ) -> Result<(), CellError> {
        self.get_show_status_calls = self.get_show_status_calls.saturating_add(1);
        if is_visible.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_speaker_volume_level(
        &mut self,
        volume_level: Option<&mut i32>,
    ) -> Result<(), CellError> {
        self.get_speaker_volume_level_calls =
            self.get_speaker_volume_level_calls.saturating_add(1);
        if volume_level.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_video_muting(
        &mut self,
        is_muting: Option<&mut bool>,
    ) -> Result<(), CellError> {
        self.get_video_muting_calls = self.get_video_muting_calls.saturating_add(1);
        if is_muting.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_voice_muting(
        &mut self,
        is_muting: Option<&mut bool>,
    ) -> Result<(), CellError> {
        self.get_voice_muting_calls = self.get_voice_muting_calls.saturating_add(1);
        if is_muting.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn hide_panel(&mut self) -> Result<(), CellError> {
        self.hide_panel_calls = self.hide_panel_calls.saturating_add(1);
        Ok(())
    }

    /// `cellSysutilAvcJoinRequest(ctx_id, room_id, request_id)`. Queues
    /// `JOIN_SUCCEEDED` with the allocated request ID.
    pub fn join_request(
        &mut self,
        _ctx_id: u32,
        room_id: Option<&SceNpRoomId>,
        request_id: Option<&mut CellSysutilAvcRequestId>,
    ) -> Result<(), CellError> {
        self.join_request_calls = self.join_request_calls.saturating_add(1);
        if room_id.is_none() || request_id.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        let rid_slot = request_id.unwrap();
        let rid = self.alloc_request_id();
        *rid_slot = rid;
        self.enqueue(rid, CellSysutilAvcEvent::JoinSucceeded, 0);
        Ok(())
    }

    /// `cellSysutilAvcLoadAsync(func, userdata, container, media, videoQuality,
    /// voiceQuality, request_id)`. Validation cascade preserves cpp:252-279:
    ///   1. `media ∈ {VoiceChat, VideoChat}` else INVALID_ARGUMENT
    ///   2. `func != 0 && request_id != null` else INVALID_ARGUMENT
    ///   3. `videoQuality == DEFAULT && voiceQuality == DEFAULT` else INVALID_ARGUMENT
    ///   4. if already loaded → ALREADY_INITIALIZED
    ///   5. stash callback, allocate request ID, queue LOAD_SUCCEEDED.
    pub fn load_async(
        &mut self,
        func: u32,
        userdata: u32,
        _container: u32,
        media_raw: u32,
        video_quality_raw: u32,
        voice_quality_raw: u32,
        request_id: Option<&mut CellSysutilAvcRequestId>,
    ) -> Result<(), CellError> {
        self.load_async_calls = self.load_async_calls.saturating_add(1);

        // Step 1: media type.
        if media_raw != CellSysUtilAvcMediaType::VoiceChat as u32
            && media_raw != CellSysUtilAvcMediaType::VideoChat as u32
        {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }

        // Step 2: func + request_id non-null.
        if func == 0 || request_id.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }

        // Step 3: quality must be DEFAULT.
        if video_quality_raw != CellSysUtilAvcVideoQuality::Default as u32
            || voice_quality_raw != CellSysUtilAvcVoiceQuality::Default as u32
        {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }

        // Step 4: already loaded.
        if self.avc_cb != 0 {
            return Err(CELL_AVC_ERROR_ALREADY_INITIALIZED);
        }

        self.avc_cb = func;
        self.avc_cb_arg = userdata;
        self.unload_consumed = false;

        let rid_slot = request_id.unwrap();
        let rid = self.alloc_request_id();
        *rid_slot = rid;
        self.enqueue(rid, CellSysutilAvcEvent::LoadSucceeded, 0);
        Ok(())
    }

    pub fn set_attribute(
        &mut self,
        _attr_id: CellSysUtilAvcAttribute,
        param: Option<&u32>,
    ) -> Result<(), CellError> {
        self.set_attribute_calls = self.set_attribute_calls.saturating_add(1);
        if param.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    /// `cellSysutilAvcSetLayoutMode(layout)` — `layout > BOTTOM → INVALID_ARGUMENT`.
    pub fn set_layout_mode(&mut self, layout_raw: u32) -> Result<(), CellError> {
        self.set_layout_mode_calls = self.set_layout_mode_calls.saturating_add(1);
        if layout_raw > MAX_LAYOUT_MODE {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    /// `cellSysutilAvcSetSpeakerVolumeLevel(volumeLevel)` — accepted `[0, 10]`.
    pub fn set_speaker_volume_level(&mut self, volume: i32) -> Result<(), CellError> {
        self.set_speaker_volume_level_calls =
            self.set_speaker_volume_level_calls.saturating_add(1);
        if !(MIN_VOLUME..=MAX_VOLUME).contains(&volume) {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn set_video_muting(&mut self, _is_muting: bool) -> Result<(), CellError> {
        self.set_video_muting_calls = self.set_video_muting_calls.saturating_add(1);
        Ok(())
    }

    pub fn set_voice_muting(&mut self, _is_muting: bool) -> Result<(), CellError> {
        self.set_voice_muting_calls = self.set_voice_muting_calls.saturating_add(1);
        Ok(())
    }

    pub fn show_panel(&mut self) -> Result<(), CellError> {
        self.show_panel_calls = self.show_panel_calls.saturating_add(1);
        Ok(())
    }

    /// `cellSysutilAvcUnloadAsync(request_id)`. Queues `UNLOAD_SUCCEEDED`.
    /// The stashed callback is cleared *when the event is delivered*, not
    /// immediately — matches cpp:92-97.
    pub fn unload_async(
        &mut self,
        request_id: Option<&mut CellSysutilAvcRequestId>,
    ) -> Result<(), CellError> {
        self.unload_async_calls = self.unload_async_calls.saturating_add(1);
        let rid_slot = request_id.ok_or(CELL_AVC_ERROR_INVALID_ARGUMENT)?;
        let rid = self.alloc_request_id();
        *rid_slot = rid;
        self.enqueue(rid, CellSysutilAvcEvent::UnloadSucceeded, 0);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_module_and_entries_match_cpp() {
        assert_eq!(HOST_MODULE_NAME, "cellSysutil");
        assert_eq!(SUBMODULE_NAME, "cellSysutilAvc");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 20);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellSysutilAvcByeRequest");
        assert_eq!(REGISTERED_ENTRY_POINTS[12], "cellSysutilAvcLoadAsync");
        assert_eq!(REGISTERED_ENTRY_POINTS[19], "cellSysutilAvcUnloadAsync");
    }

    #[test]
    fn error_codes_byte_exact_with_gaps() {
        assert_eq!(CELL_AVC_ERROR_UNKNOWN.0, 0x8002_B701);
        assert_eq!(CELL_AVC_ERROR_NOT_SUPPORTED.0, 0x8002_B702);
        assert_eq!(CELL_AVC_ERROR_NOT_INITIALIZED.0, 0x8002_B703);
        assert_eq!(CELL_AVC_ERROR_ALREADY_INITIALIZED.0, 0x8002_B704);
        assert_eq!(CELL_AVC_ERROR_INVALID_ARGUMENT.0, 0x8002_B705);
        assert_eq!(CELL_AVC_ERROR_OUT_OF_MEMORY.0, 0x8002_B706);
        assert_eq!(CELL_AVC_ERROR_BAD_ID.0, 0x8002_B707);
        // gap: 708, 709
        assert_eq!(CELL_AVC_ERROR_INVALID_STATUS.0, 0x8002_B70A);
        assert_eq!(CELL_AVC_ERROR_TIMEOUT.0, 0x8002_B70B);
        // gap: 70C
        assert_eq!(CELL_AVC_ERROR_NO_SESSION.0, 0x8002_B70D);
        assert_eq!(CELL_AVC_ERROR_INCOMPATIBLE_PROTOCOL.0, 0x8002_B70E);
        // gap: 70F
        assert_eq!(CELL_AVC_ERROR_PEER_UNREACHABLE.0, 0x8002_B710);
    }

    #[test]
    fn event_codes_byte_exact() {
        assert_eq!(CellSysutilAvcEvent::LoadSucceeded.as_u32(), 0x0000_0001);
        assert_eq!(CellSysutilAvcEvent::LoadFailed.as_u32(), 0x0000_0002);
        assert_eq!(CellSysutilAvcEvent::UnloadSucceeded.as_u32(), 0x0000_0003);
        assert_eq!(CellSysutilAvcEvent::UnloadFailed.as_u32(), 0x0000_0004);
        assert_eq!(CellSysutilAvcEvent::JoinSucceeded.as_u32(), 0x0000_0005);
        assert_eq!(CellSysutilAvcEvent::JoinFailed.as_u32(), 0x0000_0006);
        assert_eq!(CellSysutilAvcEvent::ByeSucceeded.as_u32(), 0x0000_0007);
        assert_eq!(CellSysutilAvcEvent::ByeFailed.as_u32(), 0x0000_0008);
        assert_eq!(CellSysutilAvcEvent::SystemNewMemberJoined.as_u32(), 0x1000_0001);
        assert_eq!(CellSysutilAvcEvent::SystemMemberLeft.as_u32(), 0x1000_0002);
        assert_eq!(
            CellSysutilAvcEvent::SystemSessionEstablished.as_u32(),
            0x1000_0003
        );
        assert_eq!(CellSysutilAvcEvent::SystemCameraDetected.as_u32(), 0x1000_0008);
    }

    #[test]
    fn memory_size_constants_match_header() {
        assert_eq!(CELL_SYSUTIL_AVC_VIDEO_MEMORY_SIZE, 26 * 1024 * 1024);
        assert_eq!(CELL_SYSUTIL_AVC_VOICE_MEMORY_SIZE, 8 * 1024 * 1024);
        assert_eq!(
            CELL_SYSUTIL_AVC_EXTRA_MEMORY_SIZE_FOR_SHARING_VIDEO_BUFFER,
            2 * 1024 * 1024
        );
        assert_eq!(CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION, 100);
    }

    #[test]
    fn wire_struct_sizes() {
        assert_eq!(core::mem::size_of::<CellSysutilAvcOptionParam>(), 12);
        assert_eq!(core::mem::size_of::<SceNpId>(), 32);
        assert_eq!(core::mem::size_of::<CellSysutilAvcVoiceDetectData>(), 36);
        assert_eq!(core::mem::size_of::<SceNpRoomId>(), 16);
    }

    #[test]
    fn transition_type_values_byte_exact() {
        assert_eq!(CellSysutilAvcTransitionType::Linear as u32, 0x0000_0000);
        assert_eq!(CellSysutilAvcTransitionType::Slowdown as u32, 0x0000_0001);
        assert_eq!(CellSysutilAvcTransitionType::FastUp as u32, 0x0000_0002);
        assert_eq!(CellSysutilAvcTransitionType::Angular as u32, 0x0000_0003);
        assert_eq!(CellSysutilAvcTransitionType::Exponent as u32, 0x0000_0004);
        assert_eq!(CellSysutilAvcTransitionType::None as u32, 0xFFFF_FFFF);
    }

    #[test]
    fn load_async_validation_cascade() {
        let mut m = SysutilAvc::new();
        // bad media
        let mut rid = 0u32;
        assert_eq!(
            m.load_async(0x1000, 0, 0, 0xFF, 1, 1, Some(&mut rid)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.load_async(0x1000, 0, 0, 0, 1, 1, Some(&mut rid)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        // null func
        assert_eq!(
            m.load_async(0, 0, 0, 1, 1, 1, Some(&mut rid)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        // null request_id
        assert_eq!(
            m.load_async(0x1000, 0, 0, 1, 1, 1, None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        // wrong video quality
        assert_eq!(
            m.load_async(0x1000, 0, 0, 1, 2, 1, Some(&mut rid)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        // wrong voice quality
        assert_eq!(
            m.load_async(0x1000, 0, 0, 1, 1, 2, Some(&mut rid)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        // happy path
        assert!(m.load_async(0x1000, 0xAA, 0, 1, 1, 1, Some(&mut rid)).is_ok());
        assert_eq!(rid, 0);
        assert_eq!(m.avc_cb(), 0x1000);
        assert_eq!(m.avc_cb_arg(), 0xAA);
        // double load → ALREADY_INITIALIZED
        assert_eq!(
            m.load_async(0x2000, 0, 0, 1, 1, 1, Some(&mut rid)),
            Err(CELL_AVC_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn load_async_accepts_both_media_types() {
        let mut m1 = SysutilAvc::new();
        let mut rid = 0u32;
        m1.load_async(
            0x1000,
            0,
            0,
            CellSysUtilAvcMediaType::VoiceChat as u32,
            1,
            1,
            Some(&mut rid),
        )
        .unwrap();

        let mut m2 = SysutilAvc::new();
        m2.load_async(
            0x1000,
            0,
            0,
            CellSysUtilAvcMediaType::VideoChat as u32,
            1,
            1,
            Some(&mut rid),
        )
        .unwrap();
    }

    #[test]
    fn unload_async_queues_event_and_clears_on_deliver() {
        let mut m = SysutilAvc::new();
        let mut rid_load = 0u32;
        m.load_async(0x1000, 0xAA, 0, 1, 1, 1, Some(&mut rid_load)).unwrap();
        assert!(m.is_loaded());

        let mut rid_unload = 0u32;
        m.unload_async(Some(&mut rid_unload)).unwrap();
        assert_eq!(rid_unload, 1);
        // Still loaded — callback is cleared only on delivery.
        assert!(m.is_loaded());

        let delivered = m.deliver_pending();
        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[0].event_id, CellSysutilAvcEvent::LoadSucceeded);
        assert_eq!(delivered[1].event_id, CellSysutilAvcEvent::UnloadSucceeded);
        assert!(!m.is_loaded()); // cleared now
    }

    #[test]
    fn unload_async_rejects_null() {
        let mut m = SysutilAvc::new();
        assert_eq!(
            m.unload_async(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn join_request_queues_and_validates() {
        let mut m = SysutilAvc::new();
        let room = SceNpRoomId::default();
        let mut rid = 0u32;

        assert_eq!(m.join_request(0, None, Some(&mut rid)), Err(CELL_AVC_ERROR_INVALID_ARGUMENT));
        assert_eq!(m.join_request(0, Some(&room), None), Err(CELL_AVC_ERROR_INVALID_ARGUMENT));

        m.join_request(0x12, Some(&room), Some(&mut rid)).unwrap();
        assert_eq!(rid, 0);
        let ev = m.pending()[0];
        assert_eq!(ev.event_id, CellSysutilAvcEvent::JoinSucceeded);
        assert_eq!(ev.request_id, 0);
        assert_eq!(ev.param, 0);
    }

    #[test]
    fn bye_request_queues_event() {
        let mut m = SysutilAvc::new();
        let mut rid = 0u32;
        assert_eq!(m.bye_request(None), Err(CELL_AVC_ERROR_INVALID_ARGUMENT));
        m.bye_request(Some(&mut rid)).unwrap();
        assert_eq!(rid, 0);
        assert_eq!(m.pending()[0].event_id, CellSysutilAvcEvent::ByeSucceeded);
    }

    #[test]
    fn cancel_requests_null_check() {
        let mut m = SysutilAvc::new();
        assert_eq!(
            m.cancel_bye_request(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.cancel_join_request(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        let rid = 0u32;
        m.cancel_bye_request(Some(&rid)).unwrap();
        m.cancel_join_request(Some(&rid)).unwrap();
    }

    #[test]
    fn enum_players_null_num_rejected_and_defaults_to_zero() {
        let mut m = SysutilAvc::new();
        assert_eq!(
            m.enum_players(None, None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        let mut n: i32 = 99;
        m.enum_players(None, Some(&mut n)).unwrap();
        assert_eq!(n, 0);

        // With non-null players_id, upstream doesn't touch players_num.
        let mut n2: i32 = 99;
        let mut ids: [SceNpId; 1] = [SceNpId::default()];
        m.enum_players(Some(&mut ids), Some(&mut n2)).unwrap();
        assert_eq!(n2, 99);
    }

    #[test]
    fn getters_reject_null_output() {
        let mut m = SysutilAvc::new();
        assert_eq!(
            m.get_attribute(CellSysUtilAvcAttribute::DefaultTransitionType, None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_layout_mode(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_show_status(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_speaker_volume_level(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_video_muting(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_voice_muting(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn set_layout_mode_range_check() {
        let mut m = SysutilAvc::new();
        for v in 0..=MAX_LAYOUT_MODE {
            m.set_layout_mode(v).unwrap();
        }
        assert_eq!(
            m.set_layout_mode(MAX_LAYOUT_MODE + 1),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.set_layout_mode(0xFFFF_FFFF),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn set_speaker_volume_level_clamps_to_0_to_10() {
        let mut m = SysutilAvc::new();
        for v in 0..=10 {
            m.set_speaker_volume_level(v).unwrap();
        }
        assert_eq!(
            m.set_speaker_volume_level(-1),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.set_speaker_volume_level(11),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.set_speaker_volume_level(i32::MAX),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn set_attribute_requires_param() {
        let mut m = SysutilAvc::new();
        assert_eq!(
            m.set_attribute(CellSysUtilAvcAttribute::DefaultTransitionType, None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        let v = 42u32;
        m.set_attribute(CellSysUtilAvcAttribute::DefaultTransitionType, Some(&v))
            .unwrap();
    }

    #[test]
    fn request_id_counter_monotonic_across_all_requests() {
        let mut m = SysutilAvc::new();
        let mut id_load = 0u32;
        m.load_async(0x1000, 0, 0, 1, 1, 1, Some(&mut id_load)).unwrap();
        let mut id_join = 0u32;
        let room = SceNpRoomId::default();
        m.join_request(0, Some(&room), Some(&mut id_join)).unwrap();
        let mut id_bye = 0u32;
        m.bye_request(Some(&mut id_bye)).unwrap();
        let mut id_unload = 0u32;
        m.unload_async(Some(&mut id_unload)).unwrap();
        assert_eq!(id_load, 0);
        assert_eq!(id_join, 1);
        assert_eq!(id_bye, 2);
        assert_eq!(id_unload, 3);
    }

    #[test]
    fn hide_show_panel_no_validation() {
        let mut m = SysutilAvc::new();
        m.hide_panel().unwrap();
        m.show_panel().unwrap();
        assert_eq!(m.hide_panel_calls, 1);
        assert_eq!(m.show_panel_calls, 1);
    }

    #[test]
    fn set_video_voice_muting_no_validation() {
        let mut m = SysutilAvc::new();
        m.set_video_muting(true).unwrap();
        m.set_video_muting(false).unwrap();
        m.set_voice_muting(true).unwrap();
        m.set_voice_muting(false).unwrap();
        assert_eq!(m.set_video_muting_calls, 2);
        assert_eq!(m.set_voice_muting_calls, 2);
    }

    #[test]
    fn deliver_pending_only_clears_on_unload_event() {
        let mut m = SysutilAvc::new();
        let mut rid = 0u32;
        m.load_async(0x1000, 0, 0, 1, 1, 1, Some(&mut rid)).unwrap();
        m.bye_request(Some(&mut rid)).unwrap();
        m.deliver_pending(); // load+bye — callback NOT cleared
        assert!(m.is_loaded());

        m.unload_async(Some(&mut rid)).unwrap();
        assert!(m.is_loaded()); // still there until delivery
        m.deliver_pending();
        assert!(!m.is_loaded());
    }

    #[test]
    fn counters_cover_all_entries() {
        let mut m = SysutilAvc::new();
        let mut rid = 0u32;
        let room = SceNpRoomId::default();

        let _ = m.bye_request(Some(&mut rid));
        let _ = m.cancel_bye_request(Some(&rid));
        let _ = m.cancel_join_request(Some(&rid));
        let mut n = 0i32;
        let _ = m.enum_players(None, Some(&mut n));
        let mut param = 0u32;
        let _ = m.get_attribute(CellSysUtilAvcAttribute::DefaultTransitionType, Some(&mut param));
        let mut layout = 0u32;
        let _ = m.get_layout_mode(Some(&mut layout));
        let mut vis = false;
        let _ = m.get_show_status(Some(&mut vis));
        let mut vol = 0i32;
        let _ = m.get_speaker_volume_level(Some(&mut vol));
        let mut mute = false;
        let _ = m.get_video_muting(Some(&mut mute));
        let _ = m.get_voice_muting(Some(&mut mute));
        let _ = m.hide_panel();
        let _ = m.join_request(0, Some(&room), Some(&mut rid));
        let _ = m.load_async(0x1000, 0, 0, 1, 1, 1, Some(&mut rid));
        let p = 0u32;
        let _ = m.set_attribute(CellSysUtilAvcAttribute::DefaultTransitionType, Some(&p));
        let _ = m.set_layout_mode(0);
        let _ = m.set_speaker_volume_level(5);
        let _ = m.set_video_muting(true);
        let _ = m.set_voice_muting(false);
        let _ = m.show_panel();
        let _ = m.unload_async(Some(&mut rid));

        assert_eq!(m.bye_request_calls, 1);
        assert_eq!(m.cancel_bye_request_calls, 1);
        assert_eq!(m.cancel_join_request_calls, 1);
        assert_eq!(m.enum_players_calls, 1);
        assert_eq!(m.get_attribute_calls, 1);
        assert_eq!(m.get_layout_mode_calls, 1);
        assert_eq!(m.get_show_status_calls, 1);
        assert_eq!(m.get_speaker_volume_level_calls, 1);
        assert_eq!(m.get_video_muting_calls, 1);
        assert_eq!(m.get_voice_muting_calls, 1);
        assert_eq!(m.hide_panel_calls, 1);
        assert_eq!(m.join_request_calls, 1);
        assert_eq!(m.load_async_calls, 1);
        assert_eq!(m.set_attribute_calls, 1);
        assert_eq!(m.set_layout_mode_calls, 1);
        assert_eq!(m.set_speaker_volume_level_calls, 1);
        assert_eq!(m.set_video_muting_calls, 1);
        assert_eq!(m.set_voice_muting_calls, 1);
        assert_eq!(m.show_panel_calls, 1);
        assert_eq!(m.unload_async_calls, 1);
    }

    #[test]
    fn full_sysutil_avc_lifecycle_smoke() {
        let mut m = SysutilAvc::new();

        // 1. Load with voice chat.
        let mut rid_load = 0u32;
        m.load_async(
            0x8000_1000,
            0xDEAD_BEEF,
            0,
            CellSysUtilAvcMediaType::VoiceChat as u32,
            CellSysUtilAvcVideoQuality::Default as u32,
            CellSysUtilAvcVoiceQuality::Default as u32,
            Some(&mut rid_load),
        )
        .unwrap();
        assert_eq!(rid_load, 0);

        // 2. Set panel attributes.
        m.set_layout_mode(CellSysutilAvcLayoutMode::Top as u32).unwrap();
        m.set_speaker_volume_level(7).unwrap();
        m.show_panel().unwrap();

        // 3. Join a room.
        let room = SceNpRoomId { data: [0xAA; 16] };
        let mut rid_join = 0u32;
        m.join_request(0x1234, Some(&room), Some(&mut rid_join)).unwrap();
        assert_eq!(rid_join, 1);

        // 4. Bye.
        let mut rid_bye = 0u32;
        m.bye_request(Some(&mut rid_bye)).unwrap();
        assert_eq!(rid_bye, 2);

        // 5. Deliver load/join/bye — callback still armed.
        let d1 = m.deliver_pending();
        assert_eq!(d1.len(), 3);
        assert_eq!(d1[0].event_id, CellSysutilAvcEvent::LoadSucceeded);
        assert_eq!(d1[1].event_id, CellSysutilAvcEvent::JoinSucceeded);
        assert_eq!(d1[2].event_id, CellSysutilAvcEvent::ByeSucceeded);
        assert!(m.is_loaded());

        // 6. Unload and deliver — callback clears.
        let mut rid_unload = 0u32;
        m.unload_async(Some(&mut rid_unload)).unwrap();
        let d2 = m.deliver_pending();
        assert_eq!(d2.len(), 1);
        assert_eq!(d2[0].event_id, CellSysutilAvcEvent::UnloadSucceeded);
        assert!(!m.is_loaded());

        // 7. New load after unload succeeds (fresh session).
        let mut rid_reload = 0u32;
        m.load_async(
            0x9000_0000,
            0,
            0,
            CellSysUtilAvcMediaType::VideoChat as u32,
            1,
            1,
            Some(&mut rid_reload),
        )
        .unwrap();
        assert_eq!(rid_reload, 4);
    }
}
