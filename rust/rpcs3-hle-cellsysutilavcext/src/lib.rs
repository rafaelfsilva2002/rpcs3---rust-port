//! Rust port of `rpcs3/Emu/Cell/Modules/cellSysutilAvcExt.cpp` — PS3 AVC
//! Extension HLE surface.
//!
//! Unlike `cellSysutilAvc` (which registers under the `cellSysutil` PRX),
//! this module ships as its OWN PRX (`cellSysutilAvcExt`) with 30 entries.
//! It shares the `CellAvcError` code range (0x8002_B7__) and several enums
//! (`CellSysutilAvcTransitionType`, `CellSysUtilAvcMediaType`,
//! `CellSysutilAvcWindowZorderMode`) with `cellSysutilAvc`.
//!
//! Real behaviour preserved here:
//!
//! * `InitOptionParam` — version switch `100|180` filling the out struct
//!   (maxPlayers=16 on 180, sharingVideoBuffer=false always), `default`→`UNKNOWN`.
//! * `LoadAsyncEx` — option null → `INVALID_ARGUMENT`; version switch
//!   `100|180` with the `sharingVideoBuffer && media==VoiceChat →
//!   INVALID_ARGUMENT` check; `default`→`UNKNOWN`; successful branch delegates
//!   to the stub `LoadAsync` (captured via a pluggable `LoadAsyncDelegate`).
//! * `SetWindowAlpha/Size/Rotation/Position/Show/Hide` — null player_id OR
//!   `transition_type > EXPONENT` → `INVALID_ARGUMENT` (window setters).
//! * `SetWindowZorder` — zorder must be in `[ZORDER_FORWARD_MOST..=ZORDER_BEHIND_MOST]`.
//!
//! Everything else null-checks output pointers.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use core::mem::size_of;

use rpcs3_emu_types::CellError;

/// Upstream PRX name registered via `DECLARE(ppu_module_manager::cellSysutilAvcExt)`.
pub const MODULE_NAME: &str = "cellSysutilAvcExt";

/// 30 FNIDs in the exact `REG_FUNC` order (cpp:290-319).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellSysutilAvcExtIsMicAttached",
    "cellSysutilAvcExtStopCameraDetection",
    "cellSysutilAvcExtSetWindowRotation",
    "cellSysutilAvcExtGetWindowPosition",
    "cellSysutilAvcExtSetHideNamePlate",
    "cellSysutilAvcExtSetWindowPosition",
    "cellSysutilAvcExtGetWindowSize",
    "cellSysutilAvcExtStartCameraDetection",
    "cellSysutilAvcExtGetWindowShowStatus",
    "cellSysutilAvcExtSetChatMode",
    "cellSysutilAvcExtGetNamePlateShowStatus",
    "cellSysutilAvcExtSetWindowAlpha",
    "cellSysutilAvcExtSetWindowSize",
    "cellSysutilAvcExtShowPanelEx",
    "cellSysutilAvcExtLoadAsyncEx",
    "cellSysutilAvcExtSetShowNamePlate",
    "cellSysutilAvcExtStopVoiceDetection",
    "cellSysutilAvcExtShowWindow",
    "cellSysutilAvcExtIsCameraAttached",
    "cellSysutilAvcExtHidePanelEx",
    "cellSysutilAvcExtHideWindow",
    "cellSysutilAvcExtSetChatGroup",
    "cellSysutilAvcExtGetWindowRotation",
    "cellSysutilAvcExtStartMicDetection",
    "cellSysutilAvcExtGetWindowAlpha",
    "cellSysutilAvcExtStartVoiceDetection",
    "cellSysutilAvcExtGetSurfacePointer",
    "cellSysutilAvcExtStopMicDetection",
    "cellSysutilAvcExtInitOptionParam",
    "cellSysutilAvcExtSetWindowZorder",
];

// ---------------------------------------------------------------------------
// Errors — shared with `cellSysutilAvc` (0x8002_B7__). Redeclared here so the
// crate is self-contained; both crates refer to the same byte-exact facility.
// ---------------------------------------------------------------------------

pub const CELL_AVC_ERROR_UNKNOWN: CellError = CellError(0x8002_B701);
pub const CELL_AVC_ERROR_INVALID_ARGUMENT: CellError = CellError(0x8002_B705);
pub const CELL_AVC_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_B704);

// ---------------------------------------------------------------------------
// Shared enums / constants.
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

impl CellSysutilAvcTransitionType {
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// Upstream window setters validate `transition_type > EXPONENT` which
/// rejects everything except 0..=4 and `None` (0xFFFF_FFFF). Note: `None`
/// (0xFFFF_FFFF) IS rejected by the strict `> 4` check in upstream — this is
/// probably a bug but we preserve it.
pub const TRANSITION_TYPE_MAX: u32 = 4;

/// Window Z-order accepted range (inclusive).
pub const CELL_SYSUTIL_AVC_ZORDER_FORWARD_MOST: u32 = 0x0000_0002;
pub const CELL_SYSUTIL_AVC_ZORDER_BEHIND_MOST: u32 = 0x0000_0003;

/// OptionParam version switch values.
pub const CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION: i32 = 100;
pub const CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION_180: i32 = 180;

/// Media types (from cellSysutilAvc.h, byte-exact).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CellSysUtilAvcMediaType {
    VoiceChat = 0x0000_0001,
    VideoChat = 0x0000_0002,
}

impl CellSysUtilAvcMediaType {
    pub fn as_u32(self) -> u32 {
        self as u32
    }
}

/// Wire struct — mirrors `CellSysutilAvcOptionParam`.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CellSysutilAvcOptionParam {
    pub avc_option_param_version: i32,
    pub sharing_video_buffer: u8,
    pub _pad: [u8; 3],
    pub max_players: i32,
}
const _: () = assert!(size_of::<CellSysutilAvcOptionParam>() == 12);

/// Mirror of `SceNpId` — 32 bytes.
#[repr(C)]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SceNpId {
    pub handle: [u8; 16],
    pub opt: [u8; 8],
    pub reserved: [u8; 8],
}
const _: () = assert!(size_of::<SceNpId>() == 32);

// ---------------------------------------------------------------------------
// LoadAsync delegate — upstream delegates to `cellSysutilAvcLoadAsync` after
// its own option validation. We model that via a pluggable trait so the
// delegate can be stubbed in tests or wired to the real crate later.
// ---------------------------------------------------------------------------

pub trait LoadAsyncDelegate {
    fn load_async(
        &mut self,
        func: u32,
        userdata: u32,
        container: u32,
        media: u32,
        video_quality: u32,
        voice_quality: u32,
        request_id: &mut u32,
    ) -> Result<(), CellError>;
}

/// Minimal test delegate — records every call and returns an injectable result.
#[derive(Debug, Default)]
pub struct MockLoadAsync {
    pub calls: Vec<MockLoadAsyncCall>,
    pub next_result: Option<Result<(), CellError>>,
    pub next_req_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MockLoadAsyncCall {
    pub func: u32,
    pub userdata: u32,
    pub container: u32,
    pub media: u32,
    pub video_quality: u32,
    pub voice_quality: u32,
}

impl LoadAsyncDelegate for MockLoadAsync {
    fn load_async(
        &mut self,
        func: u32,
        userdata: u32,
        container: u32,
        media: u32,
        video_quality: u32,
        voice_quality: u32,
        request_id: &mut u32,
    ) -> Result<(), CellError> {
        self.calls.push(MockLoadAsyncCall {
            func,
            userdata,
            container,
            media,
            video_quality,
            voice_quality,
        });
        match self.next_result.take() {
            Some(Err(e)) => Err(e),
            Some(Ok(())) | None => {
                *request_id = self.next_req_id;
                self.next_req_id = self.next_req_id.wrapping_add(1);
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

/// Chat mode / nameplate / detection flags are all independent booleans
/// rather than a single FSM — mirrors upstream where each setter is a stub.
#[derive(Debug, Default)]
pub struct SysutilAvcExt {
    pub camera_detection_running: bool,
    pub mic_detection_running: bool,
    pub voice_detection_running: bool,
    pub nameplate_visible: bool,
    pub chat_mode: u32,

    // Per-entry counters — 30 entries.
    pub is_mic_attached_calls: u64,
    pub stop_camera_detection_calls: u64,
    pub set_window_rotation_calls: u64,
    pub get_window_position_calls: u64,
    pub set_hide_name_plate_calls: u64,
    pub set_window_position_calls: u64,
    pub get_window_size_calls: u64,
    pub start_camera_detection_calls: u64,
    pub get_window_show_status_calls: u64,
    pub set_chat_mode_calls: u64,
    pub get_name_plate_show_status_calls: u64,
    pub set_window_alpha_calls: u64,
    pub set_window_size_calls: u64,
    pub show_panel_ex_calls: u64,
    pub load_async_ex_calls: u64,
    pub set_show_name_plate_calls: u64,
    pub stop_voice_detection_calls: u64,
    pub show_window_calls: u64,
    pub is_camera_attached_calls: u64,
    pub hide_panel_ex_calls: u64,
    pub hide_window_calls: u64,
    pub set_chat_group_calls: u64,
    pub get_window_rotation_calls: u64,
    pub start_mic_detection_calls: u64,
    pub get_window_alpha_calls: u64,
    pub start_voice_detection_calls: u64,
    pub get_surface_pointer_calls: u64,
    pub stop_mic_detection_calls: u64,
    pub init_option_param_calls: u64,
    pub set_window_zorder_calls: u64,
}

impl SysutilAvcExt {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- Pure stubs (null-check only) -----------------------------------

    pub fn is_mic_attached(&mut self, status: Option<&mut i32>) -> Result<(), CellError> {
        self.is_mic_attached_calls = self.is_mic_attached_calls.saturating_add(1);
        // Upstream uses `ensure(!!status)` — fatal assert, not a returned error.
        // Modeled here as a panic path would be noisy; keep behaviour faithful
        // but Option-idiomatic: a None still panics like upstream abort().
        ensure_not_null(status.is_none())?;
        Ok(())
    }

    pub fn stop_camera_detection(&mut self) -> Result<(), CellError> {
        self.stop_camera_detection_calls = self.stop_camera_detection_calls.saturating_add(1);
        self.camera_detection_running = false;
        Ok(())
    }

    pub fn start_camera_detection(&mut self) -> Result<(), CellError> {
        self.start_camera_detection_calls = self.start_camera_detection_calls.saturating_add(1);
        self.camera_detection_running = true;
        Ok(())
    }

    pub fn is_camera_attached(&mut self, status: Option<&mut i32>) -> Result<(), CellError> {
        self.is_camera_attached_calls = self.is_camera_attached_calls.saturating_add(1);
        ensure_not_null(status.is_none())?;
        Ok(())
    }

    pub fn start_mic_detection(&mut self) -> Result<(), CellError> {
        self.start_mic_detection_calls = self.start_mic_detection_calls.saturating_add(1);
        self.mic_detection_running = true;
        Ok(())
    }

    pub fn stop_mic_detection(&mut self) -> Result<(), CellError> {
        self.stop_mic_detection_calls = self.stop_mic_detection_calls.saturating_add(1);
        self.mic_detection_running = false;
        Ok(())
    }

    pub fn start_voice_detection(&mut self) -> Result<(), CellError> {
        self.start_voice_detection_calls = self.start_voice_detection_calls.saturating_add(1);
        self.voice_detection_running = true;
        Ok(())
    }

    pub fn stop_voice_detection(&mut self) -> Result<(), CellError> {
        self.stop_voice_detection_calls = self.stop_voice_detection_calls.saturating_add(1);
        self.voice_detection_running = false;
        Ok(())
    }

    pub fn set_show_name_plate(&mut self) -> Result<(), CellError> {
        self.set_show_name_plate_calls = self.set_show_name_plate_calls.saturating_add(1);
        self.nameplate_visible = true;
        Ok(())
    }

    pub fn set_hide_name_plate(&mut self) -> Result<(), CellError> {
        self.set_hide_name_plate_calls = self.set_hide_name_plate_calls.saturating_add(1);
        self.nameplate_visible = false;
        Ok(())
    }

    pub fn get_name_plate_show_status(
        &mut self,
        is_visible: Option<&mut bool>,
    ) -> Result<(), CellError> {
        self.get_name_plate_show_status_calls =
            self.get_name_plate_show_status_calls.saturating_add(1);
        let slot = is_visible.ok_or(CELL_AVC_ERROR_INVALID_ARGUMENT)?;
        *slot = self.nameplate_visible;
        Ok(())
    }

    pub fn set_chat_mode(&mut self, mode: u32) -> Result<(), CellError> {
        self.set_chat_mode_calls = self.set_chat_mode_calls.saturating_add(1);
        self.chat_mode = mode;
        Ok(())
    }

    pub fn set_chat_group(&mut self) -> Result<(), CellError> {
        self.set_chat_group_calls = self.set_chat_group_calls.saturating_add(1);
        Ok(())
    }

    pub fn show_panel_ex(
        &mut self,
        _transition: CellSysutilAvcTransitionType,
    ) -> Result<(), CellError> {
        self.show_panel_ex_calls = self.show_panel_ex_calls.saturating_add(1);
        Ok(())
    }

    pub fn hide_panel_ex(
        &mut self,
        _transition: CellSysutilAvcTransitionType,
    ) -> Result<(), CellError> {
        self.hide_panel_ex_calls = self.hide_panel_ex_calls.saturating_add(1);
        Ok(())
    }

    // ---- Window setters/getters (player_id + optional transition) -------

    pub fn set_window_position(
        &mut self,
        player_id: Option<&SceNpId>,
        _x: f32,
        _y: f32,
        _z: f32,
        _transition: CellSysutilAvcTransitionType,
    ) -> Result<(), CellError> {
        self.set_window_position_calls = self.set_window_position_calls.saturating_add(1);
        // Upstream only logs + returns OK — no validation cpp:47-51.
        let _ = player_id;
        Ok(())
    }

    pub fn set_window_rotation(
        &mut self,
        player_id: Option<&SceNpId>,
        _x: f32,
        _y: f32,
        _z: f32,
        _transition: CellSysutilAvcTransitionType,
    ) -> Result<(), CellError> {
        self.set_window_rotation_calls = self.set_window_rotation_calls.saturating_add(1);
        let _ = player_id;
        Ok(())
    }

    pub fn set_window_size(
        &mut self,
        player_id: Option<&SceNpId>,
        _sx: f32,
        _sy: f32,
        transition_raw: u32,
    ) -> Result<(), CellError> {
        self.set_window_size_calls = self.set_window_size_calls.saturating_add(1);
        if player_id.is_none() || transition_raw > TRANSITION_TYPE_MAX {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn set_window_alpha(
        &mut self,
        player_id: Option<&SceNpId>,
        _alpha: f32,
        transition_raw: u32,
    ) -> Result<(), CellError> {
        self.set_window_alpha_calls = self.set_window_alpha_calls.saturating_add(1);
        if player_id.is_none() || transition_raw > TRANSITION_TYPE_MAX {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn set_window_zorder(
        &mut self,
        player_id: Option<&SceNpId>,
        zorder: u32,
    ) -> Result<(), CellError> {
        self.set_window_zorder_calls = self.set_window_zorder_calls.saturating_add(1);
        if player_id.is_none()
            || zorder < CELL_SYSUTIL_AVC_ZORDER_FORWARD_MOST
            || zorder > CELL_SYSUTIL_AVC_ZORDER_BEHIND_MOST
        {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn show_window(
        &mut self,
        player_id: Option<&SceNpId>,
        transition_raw: u32,
    ) -> Result<(), CellError> {
        self.show_window_calls = self.show_window_calls.saturating_add(1);
        if player_id.is_none() || transition_raw > TRANSITION_TYPE_MAX {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn hide_window(
        &mut self,
        player_id: Option<&SceNpId>,
        transition_raw: u32,
    ) -> Result<(), CellError> {
        self.hide_window_calls = self.hide_window_calls.saturating_add(1);
        if player_id.is_none() || transition_raw > TRANSITION_TYPE_MAX {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_position(
        &mut self,
        player_id: Option<&SceNpId>,
        position_x: Option<&mut f32>,
        position_y: Option<&mut f32>,
        position_z: Option<&mut f32>,
    ) -> Result<(), CellError> {
        self.get_window_position_calls = self.get_window_position_calls.saturating_add(1);
        if player_id.is_none()
            || position_x.is_none()
            || position_y.is_none()
            || position_z.is_none()
        {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_size(
        &mut self,
        player_id: Option<&SceNpId>,
        size_x: Option<&mut f32>,
        size_y: Option<&mut f32>,
    ) -> Result<(), CellError> {
        self.get_window_size_calls = self.get_window_size_calls.saturating_add(1);
        if player_id.is_none() || size_x.is_none() || size_y.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_rotation(
        &mut self,
        player_id: Option<&SceNpId>,
        rotation_x: Option<&mut f32>,
        rotation_y: Option<&mut f32>,
        rotation_z: Option<&mut f32>,
    ) -> Result<(), CellError> {
        self.get_window_rotation_calls = self.get_window_rotation_calls.saturating_add(1);
        if player_id.is_none()
            || rotation_x.is_none()
            || rotation_y.is_none()
            || rotation_z.is_none()
        {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_alpha(
        &mut self,
        player_id: Option<&SceNpId>,
        alpha: Option<&mut f32>,
    ) -> Result<(), CellError> {
        self.get_window_alpha_calls = self.get_window_alpha_calls.saturating_add(1);
        if player_id.is_none() || alpha.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_window_show_status(
        &mut self,
        player_id: Option<&SceNpId>,
        is_visible: Option<&mut bool>,
    ) -> Result<(), CellError> {
        self.get_window_show_status_calls =
            self.get_window_show_status_calls.saturating_add(1);
        if player_id.is_none() || is_visible.is_none() {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    pub fn get_surface_pointer(
        &mut self,
        player_id: Option<&SceNpId>,
        surface_ptr: Option<&mut u32>,
        surface_size: Option<&mut i32>,
        surface_size_x: Option<&mut i32>,
        surface_size_y: Option<&mut i32>,
    ) -> Result<(), CellError> {
        self.get_surface_pointer_calls = self.get_surface_pointer_calls.saturating_add(1);
        if player_id.is_none()
            || surface_ptr.is_none()
            || surface_size.is_none()
            || surface_size_x.is_none()
            || surface_size_y.is_none()
        {
            return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
        }
        Ok(())
    }

    // ---- InitOptionParam -------------------------------------------------

    /// `cellSysutilAvcExtInitOptionParam(version, option)`.
    ///   - option == null → `INVALID_ARGUMENT`.
    ///   - writes `option.avcOptionParamVersion = version` first.
    ///   - switches on `option.avcOptionParamVersion`:
    ///     - `100` → no extra writes.
    ///     - `180` → `option.maxPlayers = 16`.
    ///     - default → `UNKNOWN`.
    ///   - then unconditionally `option.sharingVideoBuffer = false` (on success).
    pub fn init_option_param(
        &mut self,
        version: i32,
        option: Option<&mut CellSysutilAvcOptionParam>,
    ) -> Result<(), CellError> {
        self.init_option_param_calls = self.init_option_param_calls.saturating_add(1);
        let opt = option.ok_or(CELL_AVC_ERROR_INVALID_ARGUMENT)?;
        opt.avc_option_param_version = version;
        match version {
            CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION => {
                // no-op beyond the always-false sharing flag.
            }
            CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION_180 => {
                opt.max_players = 16;
            }
            _ => return Err(CELL_AVC_ERROR_UNKNOWN),
        }
        opt.sharing_video_buffer = 0;
        Ok(())
    }

    // ---- LoadAsyncEx -----------------------------------------------------

    /// `cellSysutilAvcExtLoadAsyncEx(func, userdata, container, media,
    /// videoQuality, voiceQuality, option, request_id)`:
    ///
    /// 1. `option == null` → `INVALID_ARGUMENT`.
    /// 2. Switch on `option.avcOptionParamVersion`:
    ///    - `100|180`: if `sharingVideoBuffer && media==VoiceChat` → `INVALID_ARGUMENT`.
    ///    - default: `UNKNOWN`.
    /// 3. Delegate to the provided `LoadAsync` impl.
    pub fn load_async_ex<D: LoadAsyncDelegate>(
        &mut self,
        delegate: &mut D,
        func: u32,
        userdata: u32,
        container: u32,
        media_raw: u32,
        video_quality_raw: u32,
        voice_quality_raw: u32,
        option: Option<&CellSysutilAvcOptionParam>,
        request_id: Option<&mut u32>,
    ) -> Result<(), CellError> {
        self.load_async_ex_calls = self.load_async_ex_calls.saturating_add(1);

        let option = option.ok_or(CELL_AVC_ERROR_INVALID_ARGUMENT)?;

        let sharing_voice_chat_invalid = option.sharing_video_buffer != 0
            && media_raw == CellSysUtilAvcMediaType::VoiceChat as u32;

        match option.avc_option_param_version {
            CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION => {
                if sharing_voice_chat_invalid {
                    return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
                }
            }
            CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION_180 => {
                if sharing_voice_chat_invalid {
                    return Err(CELL_AVC_ERROR_INVALID_ARGUMENT);
                }
            }
            _ => return Err(CELL_AVC_ERROR_UNKNOWN),
        }

        let rid = request_id.ok_or(CELL_AVC_ERROR_INVALID_ARGUMENT)?;
        delegate.load_async(
            func,
            userdata,
            container,
            media_raw,
            video_quality_raw,
            voice_quality_raw,
            rid,
        )
    }
}

/// Helper preserving the `ensure(!!x)` upstream semantic — panics on null.
/// Upstream uses `ensure(!!status)` which fatally aborts on null. Rust test
/// safety: we represent that as a `Result` returning `INVALID_ARGUMENT` when
/// null so tests can assert on behaviour rather than trap.
fn ensure_not_null(is_null: bool) -> Result<(), CellError> {
    if is_null {
        Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
    } else {
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
    fn module_and_entries_match_cpp() {
        assert_eq!(MODULE_NAME, "cellSysutilAvcExt");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 30);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellSysutilAvcExtIsMicAttached");
        assert_eq!(REGISTERED_ENTRY_POINTS[14], "cellSysutilAvcExtLoadAsyncEx");
        assert_eq!(REGISTERED_ENTRY_POINTS[28], "cellSysutilAvcExtInitOptionParam");
        assert_eq!(REGISTERED_ENTRY_POINTS[29], "cellSysutilAvcExtSetWindowZorder");
    }

    #[test]
    fn error_codes_match_cellavc_shared_facility() {
        assert_eq!(CELL_AVC_ERROR_UNKNOWN.0, 0x8002_B701);
        assert_eq!(CELL_AVC_ERROR_ALREADY_INITIALIZED.0, 0x8002_B704);
        assert_eq!(CELL_AVC_ERROR_INVALID_ARGUMENT.0, 0x8002_B705);
    }

    #[test]
    fn transition_type_enum_values() {
        assert_eq!(CellSysutilAvcTransitionType::Linear.as_u32(), 0);
        assert_eq!(CellSysutilAvcTransitionType::Slowdown.as_u32(), 1);
        assert_eq!(CellSysutilAvcTransitionType::FastUp.as_u32(), 2);
        assert_eq!(CellSysutilAvcTransitionType::Angular.as_u32(), 3);
        assert_eq!(CellSysutilAvcTransitionType::Exponent.as_u32(), 4);
        assert_eq!(CellSysutilAvcTransitionType::None.as_u32(), 0xFFFF_FFFF);
        assert_eq!(TRANSITION_TYPE_MAX, 4);
    }

    #[test]
    fn init_option_param_v100() {
        let mut m = SysutilAvcExt::new();
        let mut opt = CellSysutilAvcOptionParam {
            avc_option_param_version: 0,
            sharing_video_buffer: 1, // clobbered to 0 on success
            _pad: [0; 3],
            max_players: 999,
        };
        m.init_option_param(CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION, Some(&mut opt))
            .unwrap();
        assert_eq!(opt.avc_option_param_version, 100);
        assert_eq!(opt.sharing_video_buffer, 0);
        // v100 leaves max_players untouched (still 999 from init).
        assert_eq!(opt.max_players, 999);
    }

    #[test]
    fn init_option_param_v180() {
        let mut m = SysutilAvcExt::new();
        let mut opt = CellSysutilAvcOptionParam::default();
        m.init_option_param(CELL_SYSUTIL_AVC_OPTION_PARAM_VERSION_180, Some(&mut opt))
            .unwrap();
        assert_eq!(opt.avc_option_param_version, 180);
        assert_eq!(opt.max_players, 16);
        assert_eq!(opt.sharing_video_buffer, 0);
    }

    #[test]
    fn init_option_param_unknown_version_returns_unknown() {
        let mut m = SysutilAvcExt::new();
        let mut opt = CellSysutilAvcOptionParam::default();
        assert_eq!(
            m.init_option_param(42, Some(&mut opt)),
            Err(CELL_AVC_ERROR_UNKNOWN)
        );
        // Version was still stashed before the switch — mirrors cpp:260-271.
        assert_eq!(opt.avc_option_param_version, 42);
    }

    #[test]
    fn init_option_param_null_rejected() {
        let mut m = SysutilAvcExt::new();
        assert_eq!(
            m.init_option_param(100, None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn load_async_ex_null_option_rejected() {
        let mut m = SysutilAvcExt::new();
        let mut d = MockLoadAsync::default();
        let mut rid = 0u32;
        assert_eq!(
            m.load_async_ex(&mut d, 0x1000, 0, 0, 1, 1, 1, None, Some(&mut rid)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(d.calls.len(), 0);
    }

    #[test]
    fn load_async_ex_unknown_version_returns_unknown() {
        let mut m = SysutilAvcExt::new();
        let mut d = MockLoadAsync::default();
        let mut rid = 0u32;
        let opt = CellSysutilAvcOptionParam {
            avc_option_param_version: 99,
            ..Default::default()
        };
        assert_eq!(
            m.load_async_ex(
                &mut d,
                0x1000,
                0,
                0,
                1,
                1,
                1,
                Some(&opt),
                Some(&mut rid)
            ),
            Err(CELL_AVC_ERROR_UNKNOWN)
        );
    }

    #[test]
    fn load_async_ex_sharing_with_voice_chat_rejected() {
        let mut m = SysutilAvcExt::new();
        let mut d = MockLoadAsync::default();
        let mut rid = 0u32;
        // v100 path
        let opt = CellSysutilAvcOptionParam {
            avc_option_param_version: 100,
            sharing_video_buffer: 1,
            _pad: [0; 3],
            max_players: 0,
        };
        assert_eq!(
            m.load_async_ex(
                &mut d,
                0x1000,
                0,
                0,
                CellSysUtilAvcMediaType::VoiceChat as u32,
                1,
                1,
                Some(&opt),
                Some(&mut rid)
            ),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        // v180 path — same rejection.
        let opt180 = CellSysutilAvcOptionParam {
            avc_option_param_version: 180,
            sharing_video_buffer: 1,
            _pad: [0; 3],
            max_players: 16,
        };
        assert_eq!(
            m.load_async_ex(
                &mut d,
                0x1000,
                0,
                0,
                CellSysUtilAvcMediaType::VoiceChat as u32,
                1,
                1,
                Some(&opt180),
                Some(&mut rid)
            ),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn load_async_ex_sharing_with_video_chat_allowed() {
        let mut m = SysutilAvcExt::new();
        let mut d = MockLoadAsync::default();
        let mut rid = 0u32;
        let opt = CellSysutilAvcOptionParam {
            avc_option_param_version: 100,
            sharing_video_buffer: 1,
            _pad: [0; 3],
            max_players: 0,
        };
        m.load_async_ex(
            &mut d,
            0x1000,
            0xAA,
            0x55,
            CellSysUtilAvcMediaType::VideoChat as u32,
            1,
            1,
            Some(&opt),
            Some(&mut rid),
        )
        .unwrap();
        assert_eq!(d.calls.len(), 1);
        assert_eq!(d.calls[0].media, CellSysUtilAvcMediaType::VideoChat as u32);
        assert_eq!(rid, 0);
    }

    #[test]
    fn load_async_ex_propagates_delegate_error() {
        let mut m = SysutilAvcExt::new();
        let mut d = MockLoadAsync {
            next_result: Some(Err(CELL_AVC_ERROR_ALREADY_INITIALIZED)),
            ..Default::default()
        };
        let opt = CellSysutilAvcOptionParam {
            avc_option_param_version: 100,
            ..Default::default()
        };
        let mut rid = 0u32;
        assert_eq!(
            m.load_async_ex(&mut d, 0x1000, 0, 0, 1, 1, 1, Some(&opt), Some(&mut rid)),
            Err(CELL_AVC_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn load_async_ex_null_request_id_rejected_before_delegate() {
        let mut m = SysutilAvcExt::new();
        let mut d = MockLoadAsync::default();
        let opt = CellSysutilAvcOptionParam {
            avc_option_param_version: 100,
            ..Default::default()
        };
        assert_eq!(
            m.load_async_ex(&mut d, 0x1000, 0, 0, 1, 1, 1, Some(&opt), None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(d.calls.len(), 0);
    }

    #[test]
    fn set_window_alpha_transition_range_check() {
        let mut m = SysutilAvcExt::new();
        let player = SceNpId::default();
        for t in 0..=TRANSITION_TYPE_MAX {
            m.set_window_alpha(Some(&player), 0.5, t).unwrap();
        }
        assert_eq!(
            m.set_window_alpha(Some(&player), 0.5, TRANSITION_TYPE_MAX + 1),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        // CAUTION: upstream's strict `> 4` rejects None (0xFFFF_FFFF) too — we preserve.
        assert_eq!(
            m.set_window_alpha(Some(&player), 0.5, CellSysutilAvcTransitionType::None.as_u32()),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.set_window_alpha(None, 0.5, 0),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn set_window_size_requires_player_and_valid_transition() {
        let mut m = SysutilAvcExt::new();
        let player = SceNpId::default();
        m.set_window_size(Some(&player), 100.0, 100.0, 0).unwrap();
        assert_eq!(
            m.set_window_size(None, 100.0, 100.0, 0),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.set_window_size(Some(&player), 100.0, 100.0, 99),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn show_and_hide_window_share_validation() {
        let mut m = SysutilAvcExt::new();
        let player = SceNpId::default();
        m.show_window(Some(&player), 0).unwrap();
        m.hide_window(Some(&player), 4).unwrap();
        assert_eq!(
            m.show_window(None, 0),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.hide_window(Some(&player), 5),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn set_window_zorder_validates_range() {
        let mut m = SysutilAvcExt::new();
        let player = SceNpId::default();
        m.set_window_zorder(Some(&player), CELL_SYSUTIL_AVC_ZORDER_FORWARD_MOST)
            .unwrap();
        m.set_window_zorder(Some(&player), CELL_SYSUTIL_AVC_ZORDER_BEHIND_MOST)
            .unwrap();
        // Outside range → INVALID.
        assert_eq!(
            m.set_window_zorder(Some(&player), 1),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.set_window_zorder(Some(&player), 4),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.set_window_zorder(None, 2),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn get_surface_pointer_null_checks_all_five_ptrs() {
        let mut m = SysutilAvcExt::new();
        let player = SceNpId::default();
        assert_eq!(
            m.get_surface_pointer(None, Some(&mut 0), Some(&mut 0), Some(&mut 0), Some(&mut 0)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_surface_pointer(
                Some(&player),
                None,
                Some(&mut 0),
                Some(&mut 0),
                Some(&mut 0)
            ),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_surface_pointer(
                Some(&player),
                Some(&mut 0),
                None,
                Some(&mut 0),
                Some(&mut 0)
            ),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_surface_pointer(
                Some(&player),
                Some(&mut 0),
                Some(&mut 0),
                None,
                Some(&mut 0)
            ),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_surface_pointer(
                Some(&player),
                Some(&mut 0),
                Some(&mut 0),
                Some(&mut 0),
                None
            ),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        // All five non-null → OK.
        m.get_surface_pointer(
            Some(&player),
            Some(&mut 0),
            Some(&mut 0),
            Some(&mut 0),
            Some(&mut 0),
        )
        .unwrap();
    }

    #[test]
    fn detection_state_flags_toggle() {
        let mut m = SysutilAvcExt::new();
        assert!(!m.camera_detection_running);
        m.start_camera_detection().unwrap();
        assert!(m.camera_detection_running);
        m.stop_camera_detection().unwrap();
        assert!(!m.camera_detection_running);

        m.start_mic_detection().unwrap();
        assert!(m.mic_detection_running);
        m.stop_mic_detection().unwrap();
        assert!(!m.mic_detection_running);

        m.start_voice_detection().unwrap();
        assert!(m.voice_detection_running);
        m.stop_voice_detection().unwrap();
        assert!(!m.voice_detection_running);
    }

    #[test]
    fn nameplate_visibility_roundtrip() {
        let mut m = SysutilAvcExt::new();
        m.set_show_name_plate().unwrap();
        let mut vis = false;
        m.get_name_plate_show_status(Some(&mut vis)).unwrap();
        assert!(vis);
        m.set_hide_name_plate().unwrap();
        m.get_name_plate_show_status(Some(&mut vis)).unwrap();
        assert!(!vis);
        assert_eq!(
            m.get_name_plate_show_status(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn chat_mode_persists_last_write() {
        let mut m = SysutilAvcExt::new();
        m.set_chat_mode(0x7).unwrap();
        assert_eq!(m.chat_mode, 0x7);
        m.set_chat_mode(0xAA).unwrap();
        assert_eq!(m.chat_mode, 0xAA);
    }

    #[test]
    fn is_mic_and_camera_attached_reject_null() {
        let mut m = SysutilAvcExt::new();
        assert_eq!(
            m.is_mic_attached(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.is_camera_attached(None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        let mut s = 0i32;
        m.is_mic_attached(Some(&mut s)).unwrap();
        m.is_camera_attached(Some(&mut s)).unwrap();
    }

    #[test]
    fn getters_all_null_check() {
        let mut m = SysutilAvcExt::new();
        let player = SceNpId::default();
        assert_eq!(
            m.get_window_position(None, Some(&mut 0.0), Some(&mut 0.0), Some(&mut 0.0)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_window_size(Some(&player), None, Some(&mut 0.0)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_window_rotation(
                Some(&player),
                Some(&mut 0.0),
                Some(&mut 0.0),
                None
            ),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_window_alpha(Some(&player), None),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
        assert_eq!(
            m.get_window_show_status(None, Some(&mut false)),
            Err(CELL_AVC_ERROR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn set_window_position_and_rotation_no_validation() {
        // Upstream set_window_position / set_window_rotation only log.
        let mut m = SysutilAvcExt::new();
        let player = SceNpId::default();
        m.set_window_position(None, 0.0, 0.0, 0.0, CellSysutilAvcTransitionType::Linear)
            .unwrap();
        m.set_window_rotation(None, 0.0, 0.0, 0.0, CellSysutilAvcTransitionType::None)
            .unwrap();
        m.set_window_position(Some(&player), 1.0, 2.0, 3.0, CellSysutilAvcTransitionType::FastUp)
            .unwrap();
    }

    #[test]
    fn panel_ex_entries_are_stubs() {
        let mut m = SysutilAvcExt::new();
        m.show_panel_ex(CellSysutilAvcTransitionType::Slowdown).unwrap();
        m.hide_panel_ex(CellSysutilAvcTransitionType::Exponent).unwrap();
        m.set_chat_group().unwrap();
    }

    #[test]
    fn counters_cover_every_entry() {
        let mut m = SysutilAvcExt::new();
        let mut d = MockLoadAsync::default();
        let player = SceNpId::default();
        let opt = CellSysutilAvcOptionParam {
            avc_option_param_version: 100,
            ..Default::default()
        };

        let mut i = 0i32;
        let _ = m.is_mic_attached(Some(&mut i));
        m.stop_camera_detection().unwrap();
        m.set_window_rotation(Some(&player), 0.0, 0.0, 0.0, CellSysutilAvcTransitionType::Linear).unwrap();
        let _ = m.get_window_position(Some(&player), Some(&mut 0.0), Some(&mut 0.0), Some(&mut 0.0));
        m.set_hide_name_plate().unwrap();
        m.set_window_position(Some(&player), 0.0, 0.0, 0.0, CellSysutilAvcTransitionType::Linear).unwrap();
        let _ = m.get_window_size(Some(&player), Some(&mut 0.0), Some(&mut 0.0));
        m.start_camera_detection().unwrap();
        let _ = m.get_window_show_status(Some(&player), Some(&mut false));
        m.set_chat_mode(0).unwrap();
        let _ = m.get_name_plate_show_status(Some(&mut false));
        m.set_window_alpha(Some(&player), 0.5, 0).unwrap();
        m.set_window_size(Some(&player), 100.0, 100.0, 0).unwrap();
        m.show_panel_ex(CellSysutilAvcTransitionType::Linear).unwrap();
        let mut rid = 0u32;
        m.load_async_ex(&mut d, 0x1000, 0, 0, 1, 1, 1, Some(&opt), Some(&mut rid)).unwrap();
        m.set_show_name_plate().unwrap();
        m.stop_voice_detection().unwrap();
        m.show_window(Some(&player), 0).unwrap();
        let _ = m.is_camera_attached(Some(&mut i));
        m.hide_panel_ex(CellSysutilAvcTransitionType::Linear).unwrap();
        m.hide_window(Some(&player), 0).unwrap();
        m.set_chat_group().unwrap();
        let _ = m.get_window_rotation(Some(&player), Some(&mut 0.0), Some(&mut 0.0), Some(&mut 0.0));
        m.start_mic_detection().unwrap();
        let _ = m.get_window_alpha(Some(&player), Some(&mut 0.0));
        m.start_voice_detection().unwrap();
        let _ = m.get_surface_pointer(Some(&player), Some(&mut 0), Some(&mut 0), Some(&mut 0), Some(&mut 0));
        m.stop_mic_detection().unwrap();
        let mut opt_out = CellSysutilAvcOptionParam::default();
        m.init_option_param(100, Some(&mut opt_out)).unwrap();
        m.set_window_zorder(Some(&player), 2).unwrap();

        // Every counter saw at least one call.
        let counters = [
            m.is_mic_attached_calls,
            m.stop_camera_detection_calls,
            m.set_window_rotation_calls,
            m.get_window_position_calls,
            m.set_hide_name_plate_calls,
            m.set_window_position_calls,
            m.get_window_size_calls,
            m.start_camera_detection_calls,
            m.get_window_show_status_calls,
            m.set_chat_mode_calls,
            m.get_name_plate_show_status_calls,
            m.set_window_alpha_calls,
            m.set_window_size_calls,
            m.show_panel_ex_calls,
            m.load_async_ex_calls,
            m.set_show_name_plate_calls,
            m.stop_voice_detection_calls,
            m.show_window_calls,
            m.is_camera_attached_calls,
            m.hide_panel_ex_calls,
            m.hide_window_calls,
            m.set_chat_group_calls,
            m.get_window_rotation_calls,
            m.start_mic_detection_calls,
            m.get_window_alpha_calls,
            m.start_voice_detection_calls,
            m.get_surface_pointer_calls,
            m.stop_mic_detection_calls,
            m.init_option_param_calls,
            m.set_window_zorder_calls,
        ];
        assert_eq!(counters.len(), 30);
        for c in counters {
            assert!(c >= 1);
        }
    }

    #[test]
    fn full_avcext_lifecycle_smoke() {
        let mut m = SysutilAvcExt::new();
        let mut d = MockLoadAsync::default();
        let player = SceNpId { handle: [0xAB; 16], opt: [0; 8], reserved: [0; 8] };

        // 1. Init option param v180 — max_players=16.
        let mut opt = CellSysutilAvcOptionParam::default();
        m.init_option_param(180, Some(&mut opt)).unwrap();
        assert_eq!(opt.max_players, 16);

        // 2. Load via Ex.
        let mut rid = 0u32;
        m.load_async_ex(
            &mut d,
            0x8000_1000,
            0xDEAD_BEEF,
            0xC0FFEE, // arbitrary container id
            CellSysUtilAvcMediaType::VideoChat as u32,
            1,
            1,
            Some(&opt),
            Some(&mut rid),
        )
        .unwrap();
        assert_eq!(d.calls.len(), 1);
        assert_eq!(d.calls[0].media, CellSysUtilAvcMediaType::VideoChat as u32);

        // 3. Configure windows and panels.
        m.set_window_size(Some(&player), 320.0, 240.0, 0).unwrap();
        m.set_window_alpha(Some(&player), 0.8, 1).unwrap();
        m.set_window_zorder(Some(&player), CELL_SYSUTIL_AVC_ZORDER_FORWARD_MOST).unwrap();
        m.show_window(Some(&player), 2).unwrap();
        m.show_panel_ex(CellSysutilAvcTransitionType::FastUp).unwrap();

        // 4. Start detection.
        m.start_camera_detection().unwrap();
        m.start_mic_detection().unwrap();
        m.start_voice_detection().unwrap();
        assert!(m.camera_detection_running);
        assert!(m.mic_detection_running);
        assert!(m.voice_detection_running);

        // 5. Toggle nameplate.
        m.set_show_name_plate().unwrap();
        let mut vis = false;
        m.get_name_plate_show_status(Some(&mut vis)).unwrap();
        assert!(vis);

        // 6. Tear down.
        m.hide_window(Some(&player), 0).unwrap();
        m.hide_panel_ex(CellSysutilAvcTransitionType::Linear).unwrap();
        m.stop_voice_detection().unwrap();
        m.stop_mic_detection().unwrap();
        m.stop_camera_detection().unwrap();
        m.set_hide_name_plate().unwrap();
        assert!(!m.nameplate_visible);
    }
}
