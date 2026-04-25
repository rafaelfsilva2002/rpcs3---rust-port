//! Rust port of `rpcs3/Emu/Cell/Modules/cellSysutilAvc2.cpp` — PS3 AVC v2
//! (voice + video chat) sysutil module.
//!
//! Upstream registers 54 entries on its own `cellSysutilAvc2` PRX and keeps a
//! singleton `avc2_settings` holding the callback, streaming mode, muting
//! booleans, speaker volume, voice-muting player set and video bitrate state.
//!
//! This is a deeper variant of `cellSysutilAvc`, sharing the `0x8002_B7__`
//! error facility but adding *new* codes at `B70F..B712` (WINDOW_*). Unlike
//! AVC v1, AVC2 exposes full persistent chat state: voice/video/speaker
//! muting, per-member voice muting, speaker volume level (default `40.0`),
//! streaming mode selector, and a richer load validation cascade across 5
//! init-param versions (100/110/120/130/140) for two media types
//! (VoiceChat/VideoChat).
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::mem::{size_of, take};

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "cellSysutilAvc2";

/// 54 FNIDs in exact `REG_FUNC` order (cpp:1141-1194).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellSysutilAvc2GetPlayerInfo",
    "cellSysutilAvc2JoinChat",
    "cellSysutilAvc2StopStreaming",
    "cellSysutilAvc2ChangeVideoResolution",
    "cellSysutilAvc2ShowScreen",
    "cellSysutilAvc2GetVideoMuting",
    "cellSysutilAvc2GetWindowAttribute",
    "cellSysutilAvc2StopStreaming2",
    "cellSysutilAvc2SetVoiceMuting",
    "cellSysutilAvc2StartVoiceDetection",
    "cellSysutilAvc2UnloadAsync",
    "cellSysutilAvc2StopVoiceDetection",
    "cellSysutilAvc2GetAttribute",
    "cellSysutilAvc2LoadAsync",
    "cellSysutilAvc2SetSpeakerVolumeLevel",
    "cellSysutilAvc2SetWindowString",
    "cellSysutilAvc2EstimateMemoryContainerSize",
    "cellSysutilAvc2SetVideoMuting",
    "cellSysutilAvc2SetPlayerVoiceMuting",
    "cellSysutilAvc2SetStreamingTarget",
    "cellSysutilAvc2Unload",
    "cellSysutilAvc2DestroyWindow",
    "cellSysutilAvc2SetWindowPosition",
    "cellSysutilAvc2GetSpeakerVolumeLevel",
    "cellSysutilAvc2IsCameraAttached",
    "cellSysutilAvc2MicRead",
    "cellSysutilAvc2GetPlayerVoiceMuting",
    "cellSysutilAvc2JoinChatRequest",
    "cellSysutilAvc2StartStreaming",
    "cellSysutilAvc2SetWindowAttribute",
    "cellSysutilAvc2GetWindowShowStatus",
    "cellSysutilAvc2InitParam",
    "cellSysutilAvc2GetWindowSize",
    "cellSysutilAvc2SetStreamPriority",
    "cellSysutilAvc2LeaveChatRequest",
    "cellSysutilAvc2IsMicAttached",
    "cellSysutilAvc2CreateWindow",
    "cellSysutilAvc2GetSpeakerMuting",
    "cellSysutilAvc2ShowWindow",
    "cellSysutilAvc2SetWindowSize",
    "cellSysutilAvc2EnumPlayers",
    "cellSysutilAvc2GetWindowString",
    "cellSysutilAvc2LeaveChat",
    "cellSysutilAvc2SetSpeakerMuting",
    "cellSysutilAvc2Load",
    "cellSysutilAvc2SetAttribute",
    "cellSysutilAvc2UnloadAsync2",
    "cellSysutilAvc2StartStreaming2",
    "cellSysutilAvc2HideScreen",
    "cellSysutilAvc2HideWindow",
    "cellSysutilAvc2GetVoiceMuting",
    "cellSysutilAvc2GetScreenShowStatus",
    "cellSysutilAvc2Unload2",
    "cellSysutilAvc2GetWindowPosition",
];

// ---------------------------------------------------------------------------
// Errors — byte-exact `CellSysutilAvc2Error` (0x8002_B7__, extends cellSysutilAvc).
// ---------------------------------------------------------------------------

pub const CELL_AVC2_ERROR_UNKNOWN: CellError = CellError(0x8002_B701);
pub const CELL_AVC2_ERROR_NOT_SUPPORTED: CellError = CellError(0x8002_B702);
pub const CELL_AVC2_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_B703);
pub const CELL_AVC2_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_B704);
pub const CELL_AVC2_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_B705);
pub const CELL_AVC2_ERROR_OUT_OF_MEMORY: CellError = CellError(0x8002_B706);
pub const CELL_AVC2_ERROR_ERROR_BAD_ID: CellError = CellError(0x8002_B707);
pub const CELL_AVC2_ERROR_INVALID_STATUS: CellError = CellError(0x8002_B70A);
pub const CELL_AVC2_ERROR_TIMEOUT: CellError = CellError(0x8002_B70B);
pub const CELL_AVC2_ERROR_NO_SESSION: CellError = CellError(0x8002_B70D);
pub const CELL_AVC2_ERROR_WINDOW_ALREADY_EXISTS: CellError = CellError(0x8002_B70F);
pub const CELL_AVC2_ERROR_TOO_MANY_WINDOWS: CellError = CellError(0x8002_B710);
pub const CELL_AVC2_ERROR_TOO_MANY_PEER_WINDOWS: CellError = CellError(0x8002_B711);
pub const CELL_AVC2_ERROR_WINDOW_NOT_FOUND: CellError = CellError(0x8002_B712);

// ---------------------------------------------------------------------------
// Enums.
// ---------------------------------------------------------------------------

pub const CELL_SYSUTIL_AVC2_VOICE_CHAT: u32 = 0x0000_0001;
pub const CELL_SYSUTIL_AVC2_VIDEO_CHAT: u32 = 0x0000_0010;

pub const CELL_SYSUTIL_AVC2_VOICE_QUALITY_NORMAL: u32 = 0x0000_0001;
pub const CELL_SYSUTIL_AVC2_VIDEO_QUALITY_NORMAL: u32 = 0x0000_0001;

pub const CELL_SYSUTIL_AVC2_FRAME_MODE_NORMAL: u32 = 0x0000_0001;
pub const CELL_SYSUTIL_AVC2_FRAME_MODE_INTRA_ONLY: u32 = 0x0000_0002;

pub const CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QQVGA: u32 = 0x0000_0001;
pub const CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QVGA: u32 = 0x0000_0002;

pub const CELL_SYSUTIL_AVC2_CHAT_TARGET_MODE_ROOM: u32 = 0x0000_0100;
pub const CELL_SYSUTIL_AVC2_CHAT_TARGET_MODE_TEAM: u32 = 0x0000_0200;
pub const CELL_SYSUTIL_AVC2_CHAT_TARGET_MODE_PRIVATE: u32 = 0x0000_0300;
pub const CELL_SYSUTIL_AVC2_CHAT_TARGET_MODE_DIRECT: u32 = 0x0000_1000;

pub const CELL_SYSUTIL_AVC2_TRANSITION_NONE: u32 = 0xFFFF_FFFF;
pub const CELL_SYSUTIL_AVC2_TRANSITION_LINEAR: u32 = 0x0000_0000;
pub const CELL_SYSUTIL_AVC2_TRANSITION_SLOWDOWN: u32 = 0x0000_0001;
pub const CELL_SYSUTIL_AVC2_TRANSITION_FASTUP: u32 = 0x0000_0002;
pub const CELL_SYSUTIL_AVC2_TRANSITION_ANGULAR: u32 = 0x0000_0003;
pub const CELL_SYSUTIL_AVC2_TRANSITION_EXPONENT: u32 = 0x0000_0004;

pub const CELL_SYSUTIL_AVC2_ZORDER_FORWARD_MOST: u32 = 0x0000_0001;
pub const CELL_SYSUTIL_AVC2_ZORDER_BEHIND_MOST: u32 = 0x0000_0002;

pub const CELL_AVC2_CAMERA_STATUS_DETACHED: u8 = 0;
pub const CELL_AVC2_CAMERA_STATUS_ATTACHED_OFF: u8 = 1;
pub const CELL_AVC2_CAMERA_STATUS_ATTACHED_ON: u8 = 2;
pub const CELL_AVC2_CAMERA_STATUS_UNKNOWN: u8 = 3;

pub const CELL_AVC2_MIC_STATUS_DETACHED: u8 = 0;
pub const CELL_AVC2_MIC_STATUS_ATTACHED_OFF: u8 = 1;
pub const CELL_AVC2_MIC_STATUS_ATTACHED_ON: u8 = 2;
pub const CELL_AVC2_MIC_STATUS_UNKNOWN: u8 = 3;

pub const CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL: u32 = 0;
pub const CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_WAN: u32 = 1;
pub const CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_LAN: u32 = 2;

pub const CELL_SYSUTIL_AVC2_VIDEO_SHARING_MODE_DISABLE: u8 = 0;
pub const CELL_SYSUTIL_AVC2_VIDEO_SHARING_MODE_1: u8 = 1;
pub const CELL_SYSUTIL_AVC2_VIDEO_SHARING_MODE_2: u8 = 2;
pub const CELL_SYSUTIL_AVC2_VIDEO_SHARING_MODE_3: u8 = 3;

// Events (standard + system).
pub const CELL_AVC2_EVENT_LOAD_SUCCEEDED: u32 = 0x0000_0001;
pub const CELL_AVC2_EVENT_LOAD_FAILED: u32 = 0x0000_0002;
pub const CELL_AVC2_EVENT_UNLOAD_SUCCEEDED: u32 = 0x0000_0003;
pub const CELL_AVC2_EVENT_UNLOAD_FAILED: u32 = 0x0000_0004;
pub const CELL_AVC2_EVENT_JOIN_SUCCEEDED: u32 = 0x0000_0005;
pub const CELL_AVC2_EVENT_JOIN_FAILED: u32 = 0x0000_0006;
pub const CELL_AVC2_EVENT_LEAVE_SUCCEEDED: u32 = 0x0000_0007;
pub const CELL_AVC2_EVENT_LEAVE_FAILED: u32 = 0x0000_0008;

pub const CELL_AVC2_EVENT_SYSTEM_NEW_MEMBER_JOINED: u32 = 0x1000_0001;
pub const CELL_AVC2_EVENT_SYSTEM_MEMBER_LEFT: u32 = 0x1000_0002;
pub const CELL_AVC2_EVENT_SYSTEM_SESSION_ESTABLISHED: u32 = 0x1000_0003;
pub const CELL_AVC2_EVENT_SYSTEM_SESSION_CANNOT_ESTABLISHED: u32 = 0x1000_0004;
pub const CELL_AVC2_EVENT_SYSTEM_SESSION_DISCONNECTED: u32 = 0x1000_0005;
pub const CELL_AVC2_EVENT_SYSTEM_VOICE_DETECTED: u32 = 0x1000_0006;
pub const CELL_AVC2_EVENT_SYSTEM_MIC_DETECTED: u32 = 0x1000_0007;
pub const CELL_AVC2_EVENT_SYSTEM_CAMERA_DETECTED: u32 = 0x1000_0008;

pub const CELL_AVC2_REQUEST_ID_SYSTEM_EVENT: u32 = 0x0000_0000;

pub const CELL_SYSUTIL_AVC2_INIT_PARAM_VERSION: u16 = 140;

pub const AVC2_SPECIAL_ROOM_MEMBER_ID_CUSTOM_VIDEO_WINDOW: u32 = 0xFFF0;

/// Accepted init-param versions (100, 110, 120, 130, 140).
pub const ACCEPTED_INIT_VERSIONS: &[u16] = &[100, 110, 120, 130, 140];

/// Default speaker volume set by fresh `avc2_settings` (cpp:88).
pub const DEFAULT_SPEAKER_VOLUME_LEVEL: f32 = 40.0;

// ---------------------------------------------------------------------------
// Wire structs (simplified reprs — full BE layout is header-defined; these
// preserve size_of and field offsets for #[repr(C)] test assertions).
// ---------------------------------------------------------------------------

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilAvc2VoiceInitParam {
    pub voice_quality: u32,
    pub max_speakers: u16,
    pub mic_out_stream_sharing: u8,
    pub reserved: [u8; 25],
}
const _: () = assert!(size_of::<CellSysutilAvc2VoiceInitParam>() == 32);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilAvc2VideoInitParam {
    pub video_quality: u32,
    pub frame_mode: u32,
    pub max_video_resolution: u32,
    pub max_video_windows: u16,
    pub max_video_framerate: u16,
    pub max_video_bitrate: u32,
    pub coordinates_form: u32,
    pub video_stream_sharing: u8,
    pub no_use_camera_device: u8,
    pub reserved: [u8; 6],
}
const _: () = assert!(size_of::<CellSysutilAvc2VideoInitParam>() == 32);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilAvc2StreamingModeParam {
    pub mode: u16,
    pub port: u16,
    pub reserved: [u8; 10],
}
const _: () = assert!(size_of::<CellSysutilAvc2StreamingModeParam>() == 14);

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilAvc2InitParam {
    pub avc_init_param_version: u16,
    pub max_players: u16,
    pub spu_load_average: u16,
    /// Union `{u16 direct_streaming_mode; StreamingModeParam streaming_mode}`.
    /// We model the first u16 explicitly; the extra mode param bytes live in
    /// `streaming_mode_tail`.
    pub direct_streaming_mode: u16,
    pub streaming_mode_tail: [u8; 12], // rest of StreamingModeParam (14 - 2 = 12)
    pub reserved: [u8; 18],
    pub media_type: u32,
    pub voice_param: CellSysutilAvc2VoiceInitParam,
    pub video_param: CellSysutilAvc2VideoInitParam,
    pub reserved2: [u8; 22],
}

#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilAvc2PlayerInfo {
    pub member_id: u16,
    pub joined: u8,
    pub connected: u8,
    pub mic_attached: u8,
    pub reserved: [u8; 11],
}
const _: () = assert!(size_of::<CellSysutilAvc2PlayerInfo>() == 16);

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

pub type SceNpMatching2RoomMemberId = u16;
pub type CellSysutilAvc2EventId = u32;
pub type CellSysutilAvc2EventParam = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingCallback {
    pub event_id: u32,
    pub error_code: u32,
}

#[derive(Debug)]
pub struct SysutilAvc2 {
    avc2_cb: u32,
    avc2_cb_arg: u32,

    pub streaming_mode: u32,
    pub mic_out_stream_sharing: u8,
    pub video_stream_sharing: u8,
    pub total_video_bitrate: u32,
    pub voice_muting_players: Vec<u16>,
    pub voice_muting: bool,
    pub video_muting: bool,
    pub speaker_muting: bool,
    pub speaker_volume_level: f32,

    pending: Vec<PendingCallback>,

    // Per-entry counters (54 entries).
    pub get_player_info_calls: u64,
    pub join_chat_calls: u64,
    pub stop_streaming_calls: u64,
    pub change_video_resolution_calls: u64,
    pub show_screen_calls: u64,
    pub get_video_muting_calls: u64,
    pub get_window_attribute_calls: u64,
    pub stop_streaming2_calls: u64,
    pub set_voice_muting_calls: u64,
    pub start_voice_detection_calls: u64,
    pub unload_async_calls: u64,
    pub stop_voice_detection_calls: u64,
    pub get_attribute_calls: u64,
    pub load_async_calls: u64,
    pub set_speaker_volume_level_calls: u64,
    pub set_window_string_calls: u64,
    pub estimate_memory_container_size_calls: u64,
    pub set_video_muting_calls: u64,
    pub set_player_voice_muting_calls: u64,
    pub set_streaming_target_calls: u64,
    pub unload_calls: u64,
    pub destroy_window_calls: u64,
    pub set_window_position_calls: u64,
    pub get_speaker_volume_level_calls: u64,
    pub is_camera_attached_calls: u64,
    pub mic_read_calls: u64,
    pub get_player_voice_muting_calls: u64,
    pub join_chat_request_calls: u64,
    pub start_streaming_calls: u64,
    pub set_window_attribute_calls: u64,
    pub get_window_show_status_calls: u64,
    pub init_param_calls: u64,
    pub get_window_size_calls: u64,
    pub set_stream_priority_calls: u64,
    pub leave_chat_request_calls: u64,
    pub is_mic_attached_calls: u64,
    pub create_window_calls: u64,
    pub get_speaker_muting_calls: u64,
    pub show_window_calls: u64,
    pub set_window_size_calls: u64,
    pub enum_players_calls: u64,
    pub get_window_string_calls: u64,
    pub leave_chat_calls: u64,
    pub set_speaker_muting_calls: u64,
    pub load_calls: u64,
    pub set_attribute_calls: u64,
    pub unload_async2_calls: u64,
    pub start_streaming2_calls: u64,
    pub hide_screen_calls: u64,
    pub hide_window_calls: u64,
    pub get_voice_muting_calls: u64,
    pub get_screen_show_status_calls: u64,
    pub unload2_calls: u64,
    pub get_window_position_calls: u64,
}

impl Default for SysutilAvc2 {
    fn default() -> Self {
        // Match cpp:85-88 initial state.
        Self {
            avc2_cb: 0,
            avc2_cb_arg: 0,
            streaming_mode: CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL,
            mic_out_stream_sharing: 0,
            video_stream_sharing: 0,
            total_video_bitrate: 0,
            voice_muting_players: Vec::new(),
            voice_muting: true,
            video_muting: true,
            speaker_muting: true,
            speaker_volume_level: DEFAULT_SPEAKER_VOLUME_LEVEL,
            pending: Vec::new(),
            get_player_info_calls: 0,
            join_chat_calls: 0,
            stop_streaming_calls: 0,
            change_video_resolution_calls: 0,
            show_screen_calls: 0,
            get_video_muting_calls: 0,
            get_window_attribute_calls: 0,
            stop_streaming2_calls: 0,
            set_voice_muting_calls: 0,
            start_voice_detection_calls: 0,
            unload_async_calls: 0,
            stop_voice_detection_calls: 0,
            get_attribute_calls: 0,
            load_async_calls: 0,
            set_speaker_volume_level_calls: 0,
            set_window_string_calls: 0,
            estimate_memory_container_size_calls: 0,
            set_video_muting_calls: 0,
            set_player_voice_muting_calls: 0,
            set_streaming_target_calls: 0,
            unload_calls: 0,
            destroy_window_calls: 0,
            set_window_position_calls: 0,
            get_speaker_volume_level_calls: 0,
            is_camera_attached_calls: 0,
            mic_read_calls: 0,
            get_player_voice_muting_calls: 0,
            join_chat_request_calls: 0,
            start_streaming_calls: 0,
            set_window_attribute_calls: 0,
            get_window_show_status_calls: 0,
            init_param_calls: 0,
            get_window_size_calls: 0,
            set_stream_priority_calls: 0,
            leave_chat_request_calls: 0,
            is_mic_attached_calls: 0,
            create_window_calls: 0,
            get_speaker_muting_calls: 0,
            show_window_calls: 0,
            set_window_size_calls: 0,
            enum_players_calls: 0,
            get_window_string_calls: 0,
            leave_chat_calls: 0,
            set_speaker_muting_calls: 0,
            load_calls: 0,
            set_attribute_calls: 0,
            unload_async2_calls: 0,
            start_streaming2_calls: 0,
            hide_screen_calls: 0,
            hide_window_calls: 0,
            get_voice_muting_calls: 0,
            get_screen_show_status_calls: 0,
            unload2_calls: 0,
            get_window_position_calls: 0,
        }
    }
}

impl SysutilAvc2 {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_loaded(&self) -> bool {
        self.avc2_cb != 0
    }

    pub fn avc2_cb(&self) -> u32 {
        self.avc2_cb
    }

    pub fn pending(&self) -> &[PendingCallback] {
        &self.pending
    }

    fn enqueue(&mut self, event_id: u32, error_code: u32) {
        self.pending.push(PendingCallback {
            event_id,
            error_code,
        });
    }

    /// Mirrors the upstream scheduler: auto-clears the callback when a
    /// LOAD_FAILED/UNLOAD_SUCCEEDED/UNLOAD_FAILED event with `error_code < 2`
    /// is delivered (cpp:137-147).
    pub fn deliver_pending(&mut self) -> Vec<PendingCallback> {
        let drained = take(&mut self.pending);
        for ev in &drained {
            let auto_clear = matches!(
                ev.event_id,
                CELL_AVC2_EVENT_LOAD_FAILED
                    | CELL_AVC2_EVENT_UNLOAD_SUCCEEDED
                    | CELL_AVC2_EVENT_UNLOAD_FAILED
            ) && ev.error_code < 2;
            if auto_clear {
                self.avc2_cb = 0;
                self.avc2_cb_arg = 0;
            }
        }
        drained
    }

    // ---- Entries without own state logic (just counters + null-check) ---

    pub fn stop_streaming(&mut self) -> Result<(), CellError> {
        self.stop_streaming_calls = self.stop_streaming_calls.saturating_add(1);
        Ok(())
    }

    pub fn change_video_resolution(&mut self, _resolution: u32) -> Result<(), CellError> {
        self.change_video_resolution_calls = self.change_video_resolution_calls.saturating_add(1);
        Ok(())
    }

    pub fn show_screen(&mut self) -> Result<(), CellError> {
        self.show_screen_calls = self.show_screen_calls.saturating_add(1);
        Ok(())
    }

    pub fn hide_screen(&mut self) -> Result<(), CellError> {
        self.hide_screen_calls = self.hide_screen_calls.saturating_add(1);
        Ok(())
    }

    pub fn start_voice_detection(&mut self) -> Result<(), CellError> {
        self.start_voice_detection_calls = self.start_voice_detection_calls.saturating_add(1);
        Ok(())
    }

    pub fn stop_voice_detection(&mut self) -> Result<(), CellError> {
        self.stop_voice_detection_calls = self.stop_voice_detection_calls.saturating_add(1);
        Ok(())
    }

    pub fn start_streaming(&mut self) -> Result<(), CellError> {
        self.start_streaming_calls = self.start_streaming_calls.saturating_add(1);
        Ok(())
    }

    pub fn set_stream_priority(&mut self, _priority: u8) -> Result<(), CellError> {
        self.set_stream_priority_calls = self.set_stream_priority_calls.saturating_add(1);
        Ok(())
    }

    pub fn create_window(&mut self, _member_id: u16) -> Result<(), CellError> {
        self.create_window_calls = self.create_window_calls.saturating_add(1);
        Ok(())
    }

    pub fn destroy_window(&mut self, _member_id: u16) -> Result<(), CellError> {
        self.destroy_window_calls = self.destroy_window_calls.saturating_add(1);
        Ok(())
    }

    pub fn show_window(&mut self, _member_id: u16) -> Result<(), CellError> {
        self.show_window_calls = self.show_window_calls.saturating_add(1);
        Ok(())
    }

    pub fn hide_window(&mut self, _member_id: u16) -> Result<(), CellError> {
        self.hide_window_calls = self.hide_window_calls.saturating_add(1);
        Ok(())
    }

    pub fn set_window_position(
        &mut self,
        _member_id: u16,
        _x: f32,
        _y: f32,
        _z: f32,
    ) -> Result<(), CellError> {
        self.set_window_position_calls = self.set_window_position_calls.saturating_add(1);
        Ok(())
    }

    pub fn set_window_size(
        &mut self,
        _member_id: u16,
        _w: f32,
        _h: f32,
    ) -> Result<(), CellError> {
        self.set_window_size_calls = self.set_window_size_calls.saturating_add(1);
        Ok(())
    }

    pub fn set_streaming_target(&mut self) -> Result<(), CellError> {
        self.set_streaming_target_calls = self.set_streaming_target_calls.saturating_add(1);
        Ok(())
    }

    pub fn leave_chat(&mut self) -> Result<(), CellError> {
        self.leave_chat_calls = self.leave_chat_calls.saturating_add(1);
        Ok(())
    }

    pub fn leave_chat_request(&mut self) -> Result<(), CellError> {
        self.leave_chat_request_calls = self.leave_chat_request_calls.saturating_add(1);
        self.enqueue(CELL_AVC2_EVENT_LEAVE_SUCCEEDED, 0);
        Ok(())
    }

    // ---- Muting state (voice, video, speaker) ---------------------------

    pub fn set_voice_muting(&mut self, muting: u8) -> Result<(), CellError> {
        self.set_voice_muting_calls = self.set_voice_muting_calls.saturating_add(1);
        self.voice_muting = muting != 0;
        Ok(())
    }

    pub fn get_voice_muting(&mut self, out: Option<&mut u8>) -> Result<(), CellError> {
        self.get_voice_muting_calls = self.get_voice_muting_calls.saturating_add(1);
        let slot = out.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *slot = self.voice_muting as u8;
        Ok(())
    }

    pub fn set_video_muting(&mut self, muting: u8) -> Result<(), CellError> {
        self.set_video_muting_calls = self.set_video_muting_calls.saturating_add(1);
        // Preserves the upstream cpp:432 check `muting > 1` → INVALID_ARGUMENT.
        if muting > 1 {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        self.video_muting = muting != 0;
        Ok(())
    }

    pub fn get_video_muting(&mut self, out: Option<&mut u8>) -> Result<(), CellError> {
        self.get_video_muting_calls = self.get_video_muting_calls.saturating_add(1);
        let slot = out.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *slot = self.video_muting as u8;
        Ok(())
    }

    pub fn set_speaker_muting(&mut self, muting: u8) -> Result<(), CellError> {
        self.set_speaker_muting_calls = self.set_speaker_muting_calls.saturating_add(1);
        self.speaker_muting = muting != 0;
        Ok(())
    }

    pub fn get_speaker_muting(&mut self, out: Option<&mut u8>) -> Result<(), CellError> {
        self.get_speaker_muting_calls = self.get_speaker_muting_calls.saturating_add(1);
        let slot = out.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *slot = self.speaker_muting as u8;
        Ok(())
    }

    pub fn set_speaker_volume_level(&mut self, level: f32) -> Result<(), CellError> {
        self.set_speaker_volume_level_calls =
            self.set_speaker_volume_level_calls.saturating_add(1);
        self.speaker_volume_level = level;
        Ok(())
    }

    pub fn get_speaker_volume_level(
        &mut self,
        out: Option<&mut f32>,
    ) -> Result<(), CellError> {
        self.get_speaker_volume_level_calls =
            self.get_speaker_volume_level_calls.saturating_add(1);
        let slot = out.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *slot = self.speaker_volume_level;
        Ok(())
    }

    pub fn set_player_voice_muting(
        &mut self,
        member_id: u16,
        muting: u8,
    ) -> Result<(), CellError> {
        self.set_player_voice_muting_calls =
            self.set_player_voice_muting_calls.saturating_add(1);
        if muting != 0 {
            if !self.voice_muting_players.contains(&member_id) {
                self.voice_muting_players.push(member_id);
            }
        } else {
            self.voice_muting_players.retain(|m| *m != member_id);
        }
        Ok(())
    }

    pub fn get_player_voice_muting(
        &mut self,
        member_id: u16,
        out: Option<&mut u8>,
    ) -> Result<(), CellError> {
        self.get_player_voice_muting_calls =
            self.get_player_voice_muting_calls.saturating_add(1);
        let slot = out.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *slot = if self.voice_muting_players.contains(&member_id) { 1 } else { 0 };
        Ok(())
    }

    // ---- Player info / enum ---------------------------------------------

    pub fn get_player_info(
        &mut self,
        player_id: Option<u16>,
        out: Option<&mut CellSysutilAvc2PlayerInfo>,
    ) -> Result<(), CellError> {
        self.get_player_info_calls = self.get_player_info_calls.saturating_add(1);
        let id = player_id.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        let info = out.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        info.connected = 1;
        info.joined = 1;
        info.mic_attached = CELL_AVC2_MIC_STATUS_DETACHED;
        info.member_id = id;
        Ok(())
    }

    pub fn enum_players(
        &mut self,
        players_num: Option<&mut i32>,
        players_id: Option<&mut [u16]>,
    ) -> Result<(), CellError> {
        self.enum_players_calls = self.enum_players_calls.saturating_add(1);
        let num = players_num.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        if let Some(ids) = players_id {
            let n = (*num) as usize;
            for (i, slot) in ids.iter_mut().enumerate().take(n) {
                *slot = (i + 1) as u16;
            }
        } else {
            *num = 1;
        }
        Ok(())
    }

    pub fn is_camera_attached(&mut self, out: Option<&mut u8>) -> Result<(), CellError> {
        self.is_camera_attached_calls = self.is_camera_attached_calls.saturating_add(1);
        let slot = out.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *slot = CELL_AVC2_CAMERA_STATUS_DETACHED;
        Ok(())
    }

    pub fn is_mic_attached(&mut self, out: Option<&mut u8>) -> Result<(), CellError> {
        self.is_mic_attached_calls = self.is_mic_attached_calls.saturating_add(1);
        // Upstream uses `ensure(!!status)` — fatal; we map to INVALID_ARGUMENT.
        let slot = out.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *slot = CELL_AVC2_MIC_STATUS_DETACHED;
        Ok(())
    }

    pub fn mic_read(
        &mut self,
        ptr: Option<&mut [u8]>,
        size: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.mic_read_calls = self.mic_read_calls.saturating_add(1);
        // If mic_out_stream_sharing is 0, upstream returns CELL_OK immediately
        // without touching either pointer.
        if self.mic_out_stream_sharing == 0 {
            return Ok(());
        }
        match size {
            Some(s) => {
                // When ptr is null but size isn't, upstream writes *size = 0.
                if ptr.is_none() {
                    *s = 0;
                    return Ok(());
                }
                // No ringbuffer in the stub — always zero read.
                *s = 0;
                Ok(())
            }
            None => Ok(()),
        }
    }

    pub fn set_window_string(
        &mut self,
        _member_id: u16,
        string: Option<&str>,
    ) -> Result<(), CellError> {
        self.set_window_string_calls = self.set_window_string_calls.saturating_add(1);
        let s = string.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        if s.len() >= 64 {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_string(
        &mut self,
        _member_id: u16,
        string: Option<&mut [u8]>,
        len: Option<&mut u8>,
    ) -> Result<(), CellError> {
        self.get_window_string_calls = self.get_window_string_calls.saturating_add(1);
        if string.is_none() || len.is_none() {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_show_status(
        &mut self,
        _member_id: u16,
        visible: Option<&mut u8>,
    ) -> Result<(), CellError> {
        self.get_window_show_status_calls =
            self.get_window_show_status_calls.saturating_add(1);
        let v = visible.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *v = 0;
        Ok(())
    }

    pub fn get_screen_show_status(
        &mut self,
        visible: Option<&mut u8>,
    ) -> Result<(), CellError> {
        self.get_screen_show_status_calls =
            self.get_screen_show_status_calls.saturating_add(1);
        let v = visible.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *v = 0;
        Ok(())
    }

    pub fn get_window_size(
        &mut self,
        _member_id: u16,
        w: Option<&mut f32>,
        h: Option<&mut f32>,
    ) -> Result<(), CellError> {
        self.get_window_size_calls = self.get_window_size_calls.saturating_add(1);
        if w.is_none() || h.is_none() {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_position(
        &mut self,
        _member_id: u16,
        x: Option<&mut f32>,
        y: Option<&mut f32>,
        z: Option<&mut f32>,
    ) -> Result<(), CellError> {
        self.get_window_position_calls = self.get_window_position_calls.saturating_add(1);
        if x.is_none() || y.is_none() || z.is_none() {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_attribute(
        &mut self,
        _member_id: u16,
        attr: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.get_window_attribute_calls = self.get_window_attribute_calls.saturating_add(1);
        if attr.is_none() {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn set_window_attribute(
        &mut self,
        _member_id: u16,
        attr: Option<&u32>,
    ) -> Result<(), CellError> {
        self.set_window_attribute_calls = self.set_window_attribute_calls.saturating_add(1);
        if attr.is_none() {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_attribute(&mut self, attr: Option<&u32>) -> Result<(), CellError> {
        self.get_attribute_calls = self.get_attribute_calls.saturating_add(1);
        if attr.is_none() {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn set_attribute(&mut self, attr: Option<&u32>) -> Result<(), CellError> {
        self.set_attribute_calls = self.set_attribute_calls.saturating_add(1);
        if attr.is_none() {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    // ---- Chat (join/leave/request) --------------------------------------

    pub fn join_chat(
        &mut self,
        room_id: Option<u64>,
        _event_id: Option<&mut u32>,
        _event_param: Option<&mut u64>,
    ) -> Result<(), CellError> {
        self.join_chat_calls = self.join_chat_calls.saturating_add(1);
        if room_id.is_none() && self.streaming_mode != CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn join_chat_request(&mut self, room_id: Option<u64>) -> Result<(), CellError> {
        self.join_chat_request_calls = self.join_chat_request_calls.saturating_add(1);
        if room_id.is_none() && self.streaming_mode != CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        self.enqueue(CELL_AVC2_EVENT_JOIN_SUCCEEDED, 0);
        Ok(())
    }

    pub fn stop_streaming2(&mut self, media_type: u32) -> Result<(), CellError> {
        self.stop_streaming2_calls = self.stop_streaming2_calls.saturating_add(1);
        if media_type != CELL_SYSUTIL_AVC2_VOICE_CHAT && media_type != CELL_SYSUTIL_AVC2_VIDEO_CHAT
        {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn start_streaming2(&mut self, media_type: u32) -> Result<(), CellError> {
        self.start_streaming2_calls = self.start_streaming2_calls.saturating_add(1);
        if media_type != CELL_SYSUTIL_AVC2_VOICE_CHAT && media_type != CELL_SYSUTIL_AVC2_VIDEO_CHAT
        {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    // ---- InitParam + EstimateMemoryContainerSize + Load/Unload --------

    /// `cellSysutilAvc2InitParam(version, option)` — zeroes `option`, stores
    /// version, validates version ∈ {100, 110, 120, 130, 140}.
    pub fn init_param(
        &mut self,
        version: u16,
        option: Option<&mut CellSysutilAvc2InitParam>,
    ) -> Result<(), CellError> {
        self.init_param_calls = self.init_param_calls.saturating_add(1);
        let opt = option.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        *opt = CellSysutilAvc2InitParam::default();
        opt.avc_init_param_version = version;
        if !ACCEPTED_INIT_VERSIONS.contains(&version) {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    /// `cellSysutilAvc2EstimateMemoryContainerSize(initparam, size)`:
    ///   * v100: size=0x400000 always
    ///   * v110/120/130/140:
    ///     - VOICE_CHAT: size=0x300000
    ///     - VIDEO_CHAT: complex formula based on resolution/frame_mode/windows/sharing
    ///     - other media: INVALID_ARGUMENT (size writes 0)
    ///   * other versions: INVALID_ARGUMENT (no size write)
    pub fn estimate_memory_container_size(
        &mut self,
        initparam: Option<&CellSysutilAvc2InitParam>,
        size: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.estimate_memory_container_size_calls = self
            .estimate_memory_container_size_calls
            .saturating_add(1);
        let ip = initparam.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        let sz = size.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;

        match ip.avc_init_param_version {
            100 => {
                *sz = 0x400000;
                Ok(())
            }
            110 | 120 | 130 | 140 => match ip.media_type {
                CELL_SYSUTIL_AVC2_VOICE_CHAT => {
                    *sz = 0x300000;
                    Ok(())
                }
                CELL_SYSUTIL_AVC2_VIDEO_CHAT => {
                    *sz = estimate_video_chat_memory(ip);
                    Ok(())
                }
                _ => {
                    *sz = 0;
                    Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
                }
            },
            _ => Err(CELL_AVC2_ERROR_INVALID_ARGUMENT),
        }
    }

    /// `cellSysutilAvc2Load / LoadAsync` shared validation (cpp:800-961):
    /// 1. init_param non-null + version ∈ accepted set
    /// 2. media_type switch (VOICE_CHAT / VIDEO_CHAT / other → NOT_SUPPORTED)
    /// 3. VOICE_CHAT: max_players ∈ [2, 64], spu_load_average ≤ 100,
    ///    voice_quality == NORMAL, max_speakers ∈ [1, 16], streaming_mode
    ///    version-gated, callback non-null, not-already-registered.
    /// 4. VIDEO_CHAT: max_video_windows > 0 and ≤ (frame_mode==NORMAL ? 6 : 16),
    ///    bitrate ∈ [1000, 512000], framerate ∈ [1, 30].
    pub fn load(
        &mut self,
        callback_func: u32,
        user_data: u32,
        init_param: Option<&CellSysutilAvc2InitParam>,
    ) -> Result<(), CellError> {
        self.load_calls = self.load_calls.saturating_add(1);
        self.load_shared(callback_func, user_data, init_param)
    }

    pub fn load_async(
        &mut self,
        callback_func: u32,
        user_data: u32,
        init_param: Option<&CellSysutilAvc2InitParam>,
    ) -> Result<(), CellError> {
        self.load_async_calls = self.load_async_calls.saturating_add(1);
        self.load_shared(callback_func, user_data, init_param)?;
        self.enqueue(CELL_AVC2_EVENT_LOAD_SUCCEEDED, 0);
        Ok(())
    }

    fn load_shared(
        &mut self,
        callback_func: u32,
        user_data: u32,
        init_param: Option<&CellSysutilAvc2InitParam>,
    ) -> Result<(), CellError> {
        let ip = init_param.ok_or(CELL_AVC2_ERROR_INVALID_ARGUMENT)?;
        if !ACCEPTED_INIT_VERSIONS.contains(&ip.avc_init_param_version) {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }

        match ip.media_type {
            CELL_SYSUTIL_AVC2_VOICE_CHAT => self.load_voice_chat(callback_func, user_data, ip),
            CELL_SYSUTIL_AVC2_VIDEO_CHAT => self.load_video_chat(callback_func, ip),
            _ => Err(CELL_AVC2_ERROR_NOT_SUPPORTED),
        }
    }

    fn load_voice_chat(
        &mut self,
        callback_func: u32,
        user_data: u32,
        ip: &CellSysutilAvc2InitParam,
    ) -> Result<(), CellError> {
        if !(2..=64).contains(&ip.max_players)
            || ip.spu_load_average > 100
            || ip.voice_param.voice_quality != CELL_SYSUTIL_AVC2_VOICE_QUALITY_NORMAL
            || ip.voice_param.max_speakers == 0
            || ip.voice_param.max_speakers > 16
        {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }

        // Streaming mode validation is version-gated (cpp:832-867).
        let ds_mode = ip.direct_streaming_mode as u32;
        let chosen_mode = if ip.avc_init_param_version >= 120 {
            match ds_mode {
                CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL
                | CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_WAN
                | CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_LAN => ds_mode,
                _ => return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT),
            }
        } else if ip.avc_init_param_version >= 110 {
            match ds_mode {
                CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL
                | CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_WAN => ds_mode,
                _ => return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT),
            }
        } else {
            self.streaming_mode
        };

        if callback_func == 0 {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }

        if self.avc2_cb != 0 {
            return Err(CELL_AVC2_ERROR_ALREADY_INITIALIZED);
        }

        self.streaming_mode = chosen_mode;
        self.mic_out_stream_sharing = ip.voice_param.mic_out_stream_sharing;
        self.avc2_cb = callback_func;
        self.avc2_cb_arg = user_data;
        Ok(())
    }

    fn load_video_chat(
        &mut self,
        callback_func: u32,
        ip: &CellSysutilAvc2InitParam,
    ) -> Result<(), CellError> {
        // VIDEO_CHAT must have a null callback (cpp:903) — passing one is an error.
        if callback_func != 0 {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }

        let max_windows_cap = if ip.video_param.frame_mode == CELL_SYSUTIL_AVC2_FRAME_MODE_NORMAL {
            6
        } else {
            16
        };
        if ip.video_param.max_video_windows == 0
            || ip.video_param.max_video_windows > max_windows_cap
        {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }

        if ip.video_param.max_video_bitrate < 1000 || ip.video_param.max_video_bitrate > 512_000 {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }

        if ip.video_param.max_video_framerate == 0 || ip.video_param.max_video_framerate > 30 {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }

        self.video_stream_sharing = ip.video_param.video_stream_sharing;
        self.total_video_bitrate = video_chat_total_bitrate(ip);
        Ok(())
    }

    /// `cellSysutilAvc2Unload()` — rejects when no callback registered.
    pub fn unload(&mut self) -> Result<(), CellError> {
        self.unload_calls = self.unload_calls.saturating_add(1);
        if self.avc2_cb == 0 {
            return Err(CELL_AVC2_ERROR_NOT_INITIALIZED);
        }
        self.avc2_cb = 0;
        self.avc2_cb_arg = 0;
        Ok(())
    }

    /// `cellSysutilAvc2UnloadAsync()` — always enqueues UNLOAD_SUCCEEDED(0);
    /// the delivery auto-clears the callback.
    pub fn unload_async(&mut self) -> Result<(), CellError> {
        self.unload_async_calls = self.unload_async_calls.saturating_add(1);
        self.enqueue(CELL_AVC2_EVENT_UNLOAD_SUCCEEDED, 0);
        Ok(())
    }

    /// `cellSysutilAvc2Unload2(mediaType)`.
    /// VOICE_CHAT: requires callback (NOT_INITIALIZED otherwise), clears it.
    /// VIDEO_CHAT: no-op OK.
    /// Other: INVALID_ARGUMENT.
    pub fn unload2(&mut self, media_type: u32) -> Result<(), CellError> {
        self.unload2_calls = self.unload2_calls.saturating_add(1);
        match media_type {
            CELL_SYSUTIL_AVC2_VOICE_CHAT => {
                if self.avc2_cb == 0 {
                    return Err(CELL_AVC2_ERROR_NOT_INITIALIZED);
                }
                self.avc2_cb = 0;
                self.avc2_cb_arg = 0;
                Ok(())
            }
            CELL_SYSUTIL_AVC2_VIDEO_CHAT => Ok(()),
            _ => Err(CELL_AVC2_ERROR_INVALID_ARGUMENT),
        }
    }

    /// `cellSysutilAvc2UnloadAsync2(mediaType)`.
    /// VOICE_CHAT: enqueue UNLOAD_SUCCEEDED(0).
    /// VIDEO_CHAT: enqueue UNLOAD_SUCCEEDED(2) — error_code 2 does NOT auto-clear.
    pub fn unload_async2(&mut self, media_type: u32) -> Result<(), CellError> {
        self.unload_async2_calls = self.unload_async2_calls.saturating_add(1);
        if media_type != CELL_SYSUTIL_AVC2_VOICE_CHAT && media_type != CELL_SYSUTIL_AVC2_VIDEO_CHAT
        {
            return Err(CELL_AVC2_ERROR_INVALID_ARGUMENT);
        }
        let err = if media_type == CELL_SYSUTIL_AVC2_VOICE_CHAT { 0 } else { 2 };
        self.enqueue(CELL_AVC2_EVENT_UNLOAD_SUCCEEDED, err);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Derivations.
// ---------------------------------------------------------------------------

/// `EstimateMemoryContainerSize` for VIDEO_CHAT — encapsulates the upstream
/// formula (cpp:370-410). Keeping it free-standing lets tests exercise it
/// directly against a struct-literal input.
pub fn estimate_video_chat_memory(ip: &CellSysutilAvc2InitParam) -> u32 {
    let mut estimated_size: u32 = 0x40e666;
    let max_windows = ip.video_param.max_video_windows as u32;
    let mut window_count = max_windows as i32;

    if ip.video_param.video_stream_sharing == CELL_SYSUTIL_AVC2_VIDEO_SHARING_MODE_2 {
        window_count += 1;
    }

    if ip.video_param.max_video_resolution == CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QQVGA {
        estimated_size =
            (((window_count as u32).wrapping_mul(0x12c00)) & 0xfff00000).wrapping_add(0x50e666);
    } else if ip.video_param.max_video_resolution == CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QVGA {
        estimated_size = estimated_size
            .wrapping_add((((window_count as u32).wrapping_mul(0x4b000)) & 0xfff00000).wrapping_add(0x100000));
    }

    window_count = if ip.video_param.frame_mode == CELL_SYSUTIL_AVC2_FRAME_MODE_NORMAL {
        (max_windows as i32).saturating_sub(1)
    } else {
        1
    };

    let mut val: u32 = max_windows.wrapping_mul(10000);

    if ip.video_param.max_video_resolution == CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QQVGA {
        val = val.wrapping_add((window_count as u32).wrapping_mul(0x96000).wrapping_add(0x10c9e0));
    } else {
        val = val.wrapping_add(((window_count as f64 * 1_258_291.2) as u32).wrapping_add(0x1ed846));
    }

    let val_i = val as i32;
    let shifted = if val_i < 0 && (val & 0x7f) != 0 {
        (val_i >> 7).wrapping_add(1) as u32
    } else {
        (val_i >> 7) as u32
    };
    estimated_size = ((estimated_size.wrapping_add(shifted.wrapping_mul(0x80)).wrapping_add(0x80080))
        & 0xfff00000)
        .wrapping_add(0x100000);

    estimated_size
}

/// `Load_shared` VIDEO_CHAT bitrate formula (cpp:940-949).
pub fn video_chat_total_bitrate(ip: &CellSysutilAvc2InitParam) -> u32 {
    let bitrate = match ip.video_param.max_video_resolution {
        CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QQVGA => 76_800u32,
        CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QVGA => 307_200u32,
        _ => 0,
    };
    if bitrate == 0 {
        return 0;
    }
    let mut window_count = ip.video_param.max_video_windows as u32;
    if ip.video_param.video_stream_sharing == CELL_SYSUTIL_AVC2_VIDEO_SHARING_MODE_2 {
        window_count += 1;
    }
    let raw = window_count.wrapping_mul(bitrate);
    let aligned = (raw + 0x100000 - 1) & !(0x100000u32 - 1);
    aligned + 0x100000
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_voice_initparam(version: u16, streaming: u16, mic_share: u8) -> CellSysutilAvc2InitParam {
        CellSysutilAvc2InitParam {
            avc_init_param_version: version,
            max_players: 8,
            spu_load_average: 50,
            direct_streaming_mode: streaming,
            streaming_mode_tail: [0; 12],
            reserved: [0; 18],
            media_type: CELL_SYSUTIL_AVC2_VOICE_CHAT,
            voice_param: CellSysutilAvc2VoiceInitParam {
                voice_quality: CELL_SYSUTIL_AVC2_VOICE_QUALITY_NORMAL,
                max_speakers: 4,
                mic_out_stream_sharing: mic_share,
                reserved: [0; 25],
            },
            video_param: CellSysutilAvc2VideoInitParam::default(),
            reserved2: [0; 22],
        }
    }

    fn mk_video_initparam(windows: u16, res: u32, bitrate: u32, framerate: u16) -> CellSysutilAvc2InitParam {
        CellSysutilAvc2InitParam {
            avc_init_param_version: 140,
            max_players: 8,
            spu_load_average: 50,
            direct_streaming_mode: 0,
            streaming_mode_tail: [0; 12],
            reserved: [0; 18],
            media_type: CELL_SYSUTIL_AVC2_VIDEO_CHAT,
            voice_param: CellSysutilAvc2VoiceInitParam::default(),
            video_param: CellSysutilAvc2VideoInitParam {
                video_quality: CELL_SYSUTIL_AVC2_VIDEO_QUALITY_NORMAL,
                frame_mode: CELL_SYSUTIL_AVC2_FRAME_MODE_NORMAL,
                max_video_resolution: res,
                max_video_windows: windows,
                max_video_framerate: framerate,
                max_video_bitrate: bitrate,
                coordinates_form: 0,
                video_stream_sharing: 0,
                no_use_camera_device: 0,
                reserved: [0; 6],
            },
            reserved2: [0; 22],
        }
    }

    #[test]
    fn module_and_entry_count() {
        assert_eq!(MODULE_NAME, "cellSysutilAvc2");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 54);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellSysutilAvc2GetPlayerInfo");
        assert_eq!(REGISTERED_ENTRY_POINTS[31], "cellSysutilAvc2InitParam");
        assert_eq!(REGISTERED_ENTRY_POINTS[44], "cellSysutilAvc2Load");
        assert_eq!(REGISTERED_ENTRY_POINTS[53], "cellSysutilAvc2GetWindowPosition");
    }

    #[test]
    fn error_codes_byte_exact_with_window_extensions() {
        assert_eq!(CELL_AVC2_ERROR_UNKNOWN.0, 0x8002_B701);
        assert_eq!(CELL_AVC2_ERROR_NOT_SUPPORTED.0, 0x8002_B702);
        assert_eq!(CELL_AVC2_ERROR_NOT_INITIALIZED.0, 0x8002_B703);
        assert_eq!(CELL_AVC2_ERROR_ALREADY_INITIALIZED.0, 0x8002_B704);
        assert_eq!(CELL_AVC2_ERROR_INVALID_ARGUMENT.0, 0x8002_B705);
        assert_eq!(CELL_AVC2_ERROR_OUT_OF_MEMORY.0, 0x8002_B706);
        assert_eq!(CELL_AVC2_ERROR_ERROR_BAD_ID.0, 0x8002_B707);
        assert_eq!(CELL_AVC2_ERROR_INVALID_STATUS.0, 0x8002_B70A);
        assert_eq!(CELL_AVC2_ERROR_TIMEOUT.0, 0x8002_B70B);
        assert_eq!(CELL_AVC2_ERROR_NO_SESSION.0, 0x8002_B70D);
        // AVC2-specific window-related codes.
        assert_eq!(CELL_AVC2_ERROR_WINDOW_ALREADY_EXISTS.0, 0x8002_B70F);
        assert_eq!(CELL_AVC2_ERROR_TOO_MANY_WINDOWS.0, 0x8002_B710);
        assert_eq!(CELL_AVC2_ERROR_TOO_MANY_PEER_WINDOWS.0, 0x8002_B711);
        assert_eq!(CELL_AVC2_ERROR_WINDOW_NOT_FOUND.0, 0x8002_B712);
    }

    #[test]
    fn events_and_constants_byte_exact() {
        assert_eq!(CELL_AVC2_EVENT_LOAD_SUCCEEDED, 1);
        assert_eq!(CELL_AVC2_EVENT_UNLOAD_SUCCEEDED, 3);
        assert_eq!(CELL_AVC2_EVENT_LEAVE_SUCCEEDED, 7);
        assert_eq!(CELL_AVC2_EVENT_SYSTEM_NEW_MEMBER_JOINED, 0x1000_0001);
        assert_eq!(CELL_AVC2_EVENT_SYSTEM_CAMERA_DETECTED, 0x1000_0008);
        assert_eq!(CELL_SYSUTIL_AVC2_INIT_PARAM_VERSION, 140);
        assert_eq!(AVC2_SPECIAL_ROOM_MEMBER_ID_CUSTOM_VIDEO_WINDOW, 0xFFF0);
        assert_eq!(DEFAULT_SPEAKER_VOLUME_LEVEL, 40.0);
    }

    #[test]
    fn default_state_mirrors_cpp_init() {
        let m = SysutilAvc2::new();
        assert_eq!(m.streaming_mode, CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL);
        assert!(m.voice_muting);
        assert!(m.video_muting);
        assert!(m.speaker_muting);
        assert_eq!(m.speaker_volume_level, 40.0);
        assert_eq!(m.mic_out_stream_sharing, 0);
        assert_eq!(m.video_stream_sharing, 0);
        assert_eq!(m.total_video_bitrate, 0);
        assert!(m.voice_muting_players.is_empty());
        assert!(!m.is_loaded());
    }

    #[test]
    fn set_video_muting_rejects_values_above_1() {
        let mut m = SysutilAvc2::new();
        m.set_video_muting(0).unwrap();
        assert!(!m.video_muting);
        m.set_video_muting(1).unwrap();
        assert!(m.video_muting);
        assert_eq!(
            m.set_video_muting(2),
            Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn player_voice_muting_roundtrip_and_dedup() {
        let mut m = SysutilAvc2::new();
        m.set_player_voice_muting(42, 1).unwrap();
        m.set_player_voice_muting(42, 1).unwrap(); // dup — only one entry
        m.set_player_voice_muting(43, 1).unwrap();
        assert_eq!(m.voice_muting_players.len(), 2);
        let mut mute = 0u8;
        m.get_player_voice_muting(42, Some(&mut mute)).unwrap();
        assert_eq!(mute, 1);
        m.set_player_voice_muting(42, 0).unwrap();
        m.get_player_voice_muting(42, Some(&mut mute)).unwrap();
        assert_eq!(mute, 0);
        assert_eq!(m.voice_muting_players.len(), 1);
    }

    #[test]
    fn init_param_writes_version_even_on_invalid() {
        let mut m = SysutilAvc2::new();
        let mut opt = CellSysutilAvc2InitParam::default();
        // Valid
        m.init_param(140, Some(&mut opt)).unwrap();
        assert_eq!(opt.avc_init_param_version, 140);
        // Invalid — preserves upstream `*option = {}; option->version = version;`
        // BEFORE the switch fails (cpp:664-677).
        opt.max_players = 99;
        assert_eq!(
            m.init_param(42, Some(&mut opt)),
            Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(opt.avc_init_param_version, 42);
        assert_eq!(opt.max_players, 0); // *option = {} zeroed it
    }

    #[test]
    fn estimate_memory_v100_is_0x400000() {
        let mut m = SysutilAvc2::new();
        let ip = mk_voice_initparam(100, 0, 0);
        let mut sz = 0u32;
        m.estimate_memory_container_size(Some(&ip), Some(&mut sz)).unwrap();
        assert_eq!(sz, 0x400000);
    }

    #[test]
    fn estimate_memory_voice_chat_110_to_140_is_0x300000() {
        for v in [110u16, 120, 130, 140] {
            let mut m = SysutilAvc2::new();
            let ip = mk_voice_initparam(v, 0, 0);
            let mut sz = 0u32;
            m.estimate_memory_container_size(Some(&ip), Some(&mut sz)).unwrap();
            assert_eq!(sz, 0x300000);
        }
    }

    #[test]
    fn estimate_memory_unknown_version_rejected() {
        let mut m = SysutilAvc2::new();
        let mut ip = mk_voice_initparam(100, 0, 0);
        ip.avc_init_param_version = 99;
        let mut sz = 0u32;
        assert_eq!(
            m.estimate_memory_container_size(Some(&ip), Some(&mut sz)),
            Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn estimate_memory_invalid_media_zeroes_size() {
        let mut m = SysutilAvc2::new();
        let mut ip = mk_voice_initparam(140, 0, 0);
        ip.media_type = 0x99; // invalid
        let mut sz = 0xDEADu32;
        assert_eq!(
            m.estimate_memory_container_size(Some(&ip), Some(&mut sz)),
            Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(sz, 0); // cpp:413 writes 0 before returning error
    }

    #[test]
    fn load_voice_chat_happy_path_and_dedup() {
        let mut m = SysutilAvc2::new();
        let ip = mk_voice_initparam(140, CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL as u16, 1);
        m.load(0x1000, 0xAA, Some(&ip)).unwrap();
        assert!(m.is_loaded());
        assert_eq!(m.avc2_cb(), 0x1000);
        assert_eq!(m.mic_out_stream_sharing, 1);
        // Second load → ALREADY_INITIALIZED.
        assert_eq!(
            m.load(0x2000, 0xBB, Some(&ip)),
            Err(CELL_AVC2_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn load_voice_chat_rejects_players_out_of_range() {
        let mut m = SysutilAvc2::new();
        let mut ip = mk_voice_initparam(140, 0, 0);
        ip.max_players = 1;
        assert_eq!(m.load(0x1000, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        ip.max_players = 65;
        assert_eq!(m.load(0x1000, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn load_voice_chat_rejects_wrong_quality_and_speakers() {
        let mut m = SysutilAvc2::new();
        let mut ip = mk_voice_initparam(140, 0, 0);
        ip.voice_param.voice_quality = 99;
        assert_eq!(m.load(0x1000, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        ip.voice_param.voice_quality = CELL_SYSUTIL_AVC2_VOICE_QUALITY_NORMAL;
        ip.voice_param.max_speakers = 0;
        assert_eq!(m.load(0x1000, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        ip.voice_param.max_speakers = 17;
        assert_eq!(m.load(0x1000, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn load_voice_chat_null_callback_rejected() {
        let mut m = SysutilAvc2::new();
        let ip = mk_voice_initparam(140, CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL as u16, 0);
        assert_eq!(m.load(0, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn load_voice_chat_version_110_rejects_direct_lan() {
        let mut m = SysutilAvc2::new();
        let ip = mk_voice_initparam(110, CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_LAN as u16, 0);
        assert_eq!(m.load(0x1000, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        // v120+ accepts DIRECT_LAN.
        let ip120 = mk_voice_initparam(120, CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_LAN as u16, 0);
        let mut m2 = SysutilAvc2::new();
        m2.load(0x1000, 0, Some(&ip120)).unwrap();
        assert_eq!(m2.streaming_mode, CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_LAN);
    }

    #[test]
    fn load_video_chat_rejects_non_null_callback() {
        let mut m = SysutilAvc2::new();
        let ip = mk_video_initparam(4, CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QVGA, 128_000, 30);
        // VIDEO_CHAT must have null callback.
        assert_eq!(m.load(0x1000, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        // With null callback, succeeds.
        m.load(0, 0, Some(&ip)).unwrap();
        // Bitrate computed.
        assert!(m.total_video_bitrate > 0);
    }

    #[test]
    fn load_video_chat_windows_range_normal_vs_intra() {
        let mut m = SysutilAvc2::new();
        let mut ip = mk_video_initparam(7, CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QVGA, 128_000, 30);
        // NORMAL mode caps at 6.
        assert_eq!(m.load(0, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        // INTRA_ONLY caps at 16.
        ip.video_param.frame_mode = CELL_SYSUTIL_AVC2_FRAME_MODE_INTRA_ONLY;
        m.load(0, 0, Some(&ip)).unwrap();
    }

    #[test]
    fn load_video_chat_bitrate_and_framerate_checks() {
        let mut m = SysutilAvc2::new();
        let mut ip = mk_video_initparam(4, CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QVGA, 999, 30);
        assert_eq!(m.load(0, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        ip.video_param.max_video_bitrate = 600_000;
        assert_eq!(m.load(0, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        ip.video_param.max_video_bitrate = 128_000;
        ip.video_param.max_video_framerate = 0;
        assert_eq!(m.load(0, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        ip.video_param.max_video_framerate = 31;
        assert_eq!(m.load(0, 0, Some(&ip)), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        ip.video_param.max_video_framerate = 30;
        m.load(0, 0, Some(&ip)).unwrap();
    }

    #[test]
    fn unsupported_media_type_returns_not_supported() {
        let mut m = SysutilAvc2::new();
        let mut ip = mk_voice_initparam(140, 0, 0);
        ip.media_type = 0x99;
        assert_eq!(m.load(0x1000, 0, Some(&ip)), Err(CELL_AVC2_ERROR_NOT_SUPPORTED));
    }

    #[test]
    fn unload_requires_prior_load() {
        let mut m = SysutilAvc2::new();
        assert_eq!(m.unload(), Err(CELL_AVC2_ERROR_NOT_INITIALIZED));
        let ip = mk_voice_initparam(140, 0, 0);
        m.load(0x1000, 0, Some(&ip)).unwrap();
        m.unload().unwrap();
        assert!(!m.is_loaded());
    }

    #[test]
    fn unload_async_auto_clears_on_delivery() {
        let mut m = SysutilAvc2::new();
        let ip = mk_voice_initparam(140, 0, 0);
        m.load(0x1000, 0, Some(&ip)).unwrap();
        assert!(m.is_loaded());
        m.unload_async().unwrap();
        // Not cleared yet.
        assert!(m.is_loaded());
        let drained = m.deliver_pending();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].event_id, CELL_AVC2_EVENT_UNLOAD_SUCCEEDED);
        assert_eq!(drained[0].error_code, 0);
        assert!(!m.is_loaded());
    }

    #[test]
    fn unload_async2_video_uses_error_code_2_which_does_not_auto_clear() {
        let mut m = SysutilAvc2::new();
        let ip = mk_voice_initparam(140, 0, 0);
        m.load(0x1000, 0, Some(&ip)).unwrap();
        m.unload_async2(CELL_SYSUTIL_AVC2_VIDEO_CHAT).unwrap();
        let drained = m.deliver_pending();
        assert_eq!(drained[0].error_code, 2);
        // error_code >= 2 → no auto-clear.
        assert!(m.is_loaded());
    }

    #[test]
    fn unload_async2_voice_uses_error_code_0_and_clears() {
        let mut m = SysutilAvc2::new();
        let ip = mk_voice_initparam(140, 0, 0);
        m.load(0x1000, 0, Some(&ip)).unwrap();
        m.unload_async2(CELL_SYSUTIL_AVC2_VOICE_CHAT).unwrap();
        let drained = m.deliver_pending();
        assert_eq!(drained[0].error_code, 0);
        assert!(!m.is_loaded());
    }

    #[test]
    fn unload2_video_no_op_ok_voice_requires_load() {
        let mut m = SysutilAvc2::new();
        assert_eq!(
            m.unload2(CELL_SYSUTIL_AVC2_VOICE_CHAT),
            Err(CELL_AVC2_ERROR_NOT_INITIALIZED)
        );
        m.unload2(CELL_SYSUTIL_AVC2_VIDEO_CHAT).unwrap();
        assert_eq!(m.unload2(0x99), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn start_stop_streaming2_validate_media() {
        let mut m = SysutilAvc2::new();
        m.start_streaming2(CELL_SYSUTIL_AVC2_VOICE_CHAT).unwrap();
        m.stop_streaming2(CELL_SYSUTIL_AVC2_VIDEO_CHAT).unwrap();
        assert_eq!(m.start_streaming2(0x99), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        assert_eq!(m.stop_streaming2(0), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
    }

    #[test]
    fn get_player_info_writes_fields() {
        let mut m = SysutilAvc2::new();
        let mut info = CellSysutilAvc2PlayerInfo::default();
        m.get_player_info(Some(12), Some(&mut info)).unwrap();
        assert_eq!(info.member_id, 12);
        assert_eq!(info.joined, 1);
        assert_eq!(info.connected, 1);
        assert_eq!(info.mic_attached, CELL_AVC2_MIC_STATUS_DETACHED);
    }

    #[test]
    fn is_camera_attached_returns_detached() {
        let mut m = SysutilAvc2::new();
        let mut s = 99u8;
        m.is_camera_attached(Some(&mut s)).unwrap();
        assert_eq!(s, CELL_AVC2_CAMERA_STATUS_DETACHED);
    }

    #[test]
    fn is_mic_attached_returns_detached() {
        let mut m = SysutilAvc2::new();
        let mut s = 99u8;
        m.is_mic_attached(Some(&mut s)).unwrap();
        assert_eq!(s, CELL_AVC2_MIC_STATUS_DETACHED);
    }

    #[test]
    fn mic_read_without_sharing_is_noop_ok() {
        let mut m = SysutilAvc2::new();
        assert_eq!(m.mic_out_stream_sharing, 0);
        let mut sz = 0xDEADu32;
        m.mic_read(None, Some(&mut sz)).unwrap();
        // Untouched because sharing disabled.
        assert_eq!(sz, 0xDEAD);
    }

    #[test]
    fn mic_read_with_sharing_and_null_ptr_writes_zero_size() {
        let mut m = SysutilAvc2::new();
        m.mic_out_stream_sharing = 1;
        let mut sz = 99u32;
        m.mic_read(None, Some(&mut sz)).unwrap();
        assert_eq!(sz, 0);
    }

    #[test]
    fn enum_players_null_id_writes_count_one() {
        let mut m = SysutilAvc2::new();
        let mut n: i32 = 99;
        m.enum_players(Some(&mut n), None).unwrap();
        assert_eq!(n, 1);
    }

    #[test]
    fn enum_players_fill_assigns_i_plus_1() {
        let mut m = SysutilAvc2::new();
        let mut n: i32 = 5;
        let mut ids = [0u16; 5];
        m.enum_players(Some(&mut n), Some(&mut ids)).unwrap();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn set_window_string_rejects_long() {
        let mut m = SysutilAvc2::new();
        m.set_window_string(0, Some("hello")).unwrap();
        let long = core::str::from_utf8(&[b'a'; 64]).unwrap();
        assert_eq!(
            m.set_window_string(0, Some(long)),
            Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
        );
        let ok = core::str::from_utf8(&[b'a'; 63]).unwrap();
        m.set_window_string(0, Some(ok)).unwrap();
    }

    #[test]
    fn join_chat_null_room_with_direct_mode_rejected() {
        let mut m = SysutilAvc2::new();
        m.streaming_mode = CELL_SYSUTIL_AVC2_STREAMING_MODE_DIRECT_WAN;
        assert_eq!(
            m.join_chat(None, None, None),
            Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
        );
        m.join_chat(Some(0x123), None, None).unwrap();
    }

    #[test]
    fn join_chat_request_queues_event() {
        let mut m = SysutilAvc2::new();
        m.join_chat_request(Some(0xAB)).unwrap();
        assert_eq!(m.pending()[0].event_id, CELL_AVC2_EVENT_JOIN_SUCCEEDED);
    }

    #[test]
    fn leave_chat_request_queues_event() {
        let mut m = SysutilAvc2::new();
        m.leave_chat_request().unwrap();
        assert_eq!(m.pending()[0].event_id, CELL_AVC2_EVENT_LEAVE_SUCCEEDED);
    }

    #[test]
    fn getters_null_check() {
        let mut m = SysutilAvc2::new();
        assert_eq!(m.get_voice_muting(None), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        assert_eq!(m.get_video_muting(None), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        assert_eq!(m.get_speaker_muting(None), Err(CELL_AVC2_ERROR_INVALID_ARGUMENT));
        assert_eq!(
            m.get_speaker_volume_level(None),
            Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_screen_show_status(None),
            Err(CELL_AVC2_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn speaker_volume_roundtrip() {
        let mut m = SysutilAvc2::new();
        m.set_speaker_volume_level(77.0).unwrap();
        let mut v = 0.0f32;
        m.get_speaker_volume_level(Some(&mut v)).unwrap();
        assert_eq!(v, 77.0);
    }

    #[test]
    fn video_chat_total_bitrate_aligns_to_1mb() {
        // QQVGA (per-window 76800): 4 windows × 76800 = 307200 = 0x4B000.
        // Align up to 0x100000 = 0x100000; + 0x100000 buffer = 0x200000.
        let ip_qqvga = mk_video_initparam(4, CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QQVGA, 128_000, 30);
        assert_eq!(video_chat_total_bitrate(&ip_qqvga), 0x200000);
        // QVGA (per-window 307200): 4 × 307200 = 1228800; align up to 0x200000; + 0x100000 = 0x300000.
        let ip_qvga = mk_video_initparam(4, CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QVGA, 128_000, 30);
        assert_eq!(video_chat_total_bitrate(&ip_qvga), 0x300000);
        // Sharing mode 2 adds a window (5 × 307200 = 1536000); still aligns to 0x200000 + 0x100000.
        let mut ip_share = ip_qvga;
        ip_share.video_param.video_stream_sharing = CELL_SYSUTIL_AVC2_VIDEO_SHARING_MODE_2;
        assert_eq!(video_chat_total_bitrate(&ip_share), 0x300000);
    }

    #[test]
    fn estimate_video_chat_memory_is_deterministic_for_qqvga() {
        let ip = mk_video_initparam(4, CELL_SYSUTIL_AVC2_VIDEO_RESOLUTION_QQVGA, 128_000, 30);
        // Just verify the function returns something nonzero and consistent.
        let a = estimate_video_chat_memory(&ip);
        let b = estimate_video_chat_memory(&ip);
        assert_eq!(a, b);
        assert!(a > 0);
    }

    #[test]
    fn full_sysutil_avc2_lifecycle_smoke() {
        let mut m = SysutilAvc2::new();

        // Init via init_param fills version properly.
        let mut opt = CellSysutilAvc2InitParam::default();
        m.init_param(140, Some(&mut opt)).unwrap();
        assert_eq!(opt.avc_init_param_version, 140);

        // Load VOICE_CHAT.
        let ip = mk_voice_initparam(140, CELL_SYSUTIL_AVC2_STREAMING_MODE_NORMAL as u16, 1);
        m.load_async(0x1000_0000, 0xDEAD_BEEF, Some(&ip)).unwrap();
        assert!(m.is_loaded());

        // Configure state.
        m.set_voice_muting(0).unwrap();
        m.set_video_muting(0).unwrap();
        m.set_speaker_muting(0).unwrap();
        m.set_speaker_volume_level(80.0).unwrap();
        m.set_player_voice_muting(5, 1).unwrap();
        m.set_player_voice_muting(9, 1).unwrap();

        // Join + streaming.
        m.join_chat_request(Some(0x1234_5678_9ABC)).unwrap();
        m.start_streaming2(CELL_SYSUTIL_AVC2_VOICE_CHAT).unwrap();

        // Deliver pending: load + join events.
        let drained = m.deliver_pending();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].event_id, CELL_AVC2_EVENT_LOAD_SUCCEEDED);
        assert_eq!(drained[1].event_id, CELL_AVC2_EVENT_JOIN_SUCCEEDED);
        assert!(m.is_loaded()); // LOAD event does NOT auto-clear.

        // Tear down.
        m.stop_streaming2(CELL_SYSUTIL_AVC2_VOICE_CHAT).unwrap();
        m.leave_chat_request().unwrap();
        m.unload_async().unwrap();
        let drained2 = m.deliver_pending();
        assert_eq!(drained2.len(), 2);
        assert_eq!(drained2[1].event_id, CELL_AVC2_EVENT_UNLOAD_SUCCEEDED);
        assert!(!m.is_loaded());
    }
}
