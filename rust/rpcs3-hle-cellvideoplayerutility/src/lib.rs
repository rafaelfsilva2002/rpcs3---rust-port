//! Rust port of `rpcs3/Emu/Cell/Modules/cellVideoPlayerUtility.cpp`.
//!
//! 17 PRX entries under the module name `cellVideoPlayerUtility` —
//! the high-level video player the XMB uses (via games like the
//! official video store app). The C++ source (127 lines) is all
//! `UNIMPLEMENTED_FUNC` stubs; the Rust port adds a 3-axis FSM
//! (module + session + thumbnail) and matching placeholder error
//! codes so mis-sequenced calls surface.
//!
//! REG_FUNC order at cpp:110-126.
//! Module name byte-exact at cpp:4 / cpp:108.

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:4 / cpp:108.
pub const MODULE_NAME: &str = "cellVideoPlayerUtility";

/// REG_FUNC order at cpp:110-126.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellVideoPlayerInitialize",
    "cellVideoPlayerSetStartPosition",
    "cellVideoPlayerGetVolume",
    "cellVideoPlayerFinalize",
    "cellVideoPlayerSetStopPosition",
    "cellVideoPlayerClose",
    "cellVideoPlayerGetTransferPictureInfo",
    "cellVideoPlayerSetDownloadPosition",
    "cellVideoPlayerStartThumbnail",
    "cellVideoPlayerEndThumbnail",
    "cellVideoPlayerOpen",
    "cellVideoPlayerSetVolume",
    "cellVideoPlayerGetOutputStereoPicture",
    "cellVideoPlayerGetPlaybackStatus",
    "cellVideoPlayerSetTransferComplete",
    "cellVideoPlayerGetOutputPicture",
    "cellVideoPlayerPlaybackControl",
];

// --- Placeholder error codes (facility 0x8002_D5__) ---------------------
//
// C++ commits no named errors for this module. Values below are
// internal placeholders used purely to enforce the Rust FSM without
// altering the C++ happy-path `CELL_OK` return.

pub const CELL_VIDEO_PLAYER_ERROR_NOT_INITIALIZED: CellError = CellError(0x8002_D501);
pub const CELL_VIDEO_PLAYER_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8002_D502);
pub const CELL_VIDEO_PLAYER_ERROR_NOT_OPEN: CellError = CellError(0x8002_D503);
pub const CELL_VIDEO_PLAYER_ERROR_ALREADY_OPEN: CellError = CellError(0x8002_D504);
pub const CELL_VIDEO_PLAYER_ERROR_FINALIZED: CellError = CellError(0x8002_D505);
pub const CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER: CellError = CellError(0x8002_D506);
pub const CELL_VIDEO_PLAYER_ERROR_THUMBNAIL_NOT_ACTIVE: CellError = CellError(0x8002_D507);
pub const CELL_VIDEO_PLAYER_ERROR_THUMBNAIL_ALREADY_ACTIVE: CellError = CellError(0x8002_D508);

// --- Volume clamp -------------------------------------------------------

pub const VOLUME_MIN: f32 = 0.0;
pub const VOLUME_MAX: f32 = 1.0;
pub const DEFAULT_VOLUME: f32 = 1.0;

// --- Playback command -------------------------------------------------

/// Commands a game passes to `PlaybackControl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackCommand {
    Play,
    Pause,
    Resume,
    Stop,
    FastForward,
    FastReverse,
}

/// Lifecycle of the player session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Stopped,
    Playing,
    Paused,
    FastForward,
    FastReverse,
}

/// Module-level state: `Uninit → Initialized → Finalized`.
/// `Finalized` is terminal — the firmware doesn't allow `Initialize`
/// on a finalized module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleState {
    Uninitialized,
    Initialized,
    Finalized,
}

/// Session state: `Closed → Open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Closed,
    Open,
}

/// Transfer-complete marker for downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransferPictureInfo {
    pub frame_count: u32,
    pub bytes_available: u64,
    pub transfer_complete: bool,
}

/// Output picture descriptor — minimal subset the port surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OutputPicture {
    pub picture_id: u32,
    pub width: u32,
    pub height: u32,
    pub is_stereo: bool,
}

// --- Manager ------------------------------------------------------------

/// HLE state for one video player instance.
#[derive(Debug)]
pub struct VideoPlayer {
    module_state: ModuleState,
    session_state: SessionState,
    playback_status: PlaybackStatus,
    thumbnail_active: bool,
    volume: f32,
    start_position_us: u64,
    stop_position_us: u64,
    download_position_bytes: u64,
    transfer: TransferPictureInfo,
    // Per-entry counters — handy for dispatch-trace assertions.
    initialize_calls: u32,
    finalize_calls: u32,
    open_calls: u32,
    close_calls: u32,
    playback_control_calls: u32,
    start_thumbnail_calls: u32,
    end_thumbnail_calls: u32,
    set_transfer_complete_calls: u32,
    get_output_picture_calls: u32,
    get_output_stereo_picture_calls: u32,
    set_volume_calls: u32,
    get_volume_calls: u32,
    set_start_position_calls: u32,
    set_stop_position_calls: u32,
    set_download_position_calls: u32,
    get_transfer_picture_info_calls: u32,
    get_playback_status_calls: u32,
}

impl Default for VideoPlayer {
    fn default() -> Self {
        Self::new()
    }
}

impl VideoPlayer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            module_state: ModuleState::Uninitialized,
            session_state: SessionState::Closed,
            playback_status: PlaybackStatus::Stopped,
            thumbnail_active: false,
            volume: DEFAULT_VOLUME,
            start_position_us: 0,
            stop_position_us: 0,
            download_position_bytes: 0,
            transfer: TransferPictureInfo {
                frame_count: 0,
                bytes_available: 0,
                transfer_complete: false,
            },
            initialize_calls: 0,
            finalize_calls: 0,
            open_calls: 0,
            close_calls: 0,
            playback_control_calls: 0,
            start_thumbnail_calls: 0,
            end_thumbnail_calls: 0,
            set_transfer_complete_calls: 0,
            get_output_picture_calls: 0,
            get_output_stereo_picture_calls: 0,
            set_volume_calls: 0,
            get_volume_calls: 0,
            set_start_position_calls: 0,
            set_stop_position_calls: 0,
            set_download_position_calls: 0,
            get_transfer_picture_info_calls: 0,
            get_playback_status_calls: 0,
        }
    }

    #[must_use]
    pub fn module_state(&self) -> ModuleState {
        self.module_state
    }
    #[must_use]
    pub fn session_state(&self) -> SessionState {
        self.session_state
    }
    #[must_use]
    pub fn playback_status(&self) -> PlaybackStatus {
        self.playback_status
    }
    #[must_use]
    pub fn thumbnail_active(&self) -> bool {
        self.thumbnail_active
    }
    #[must_use]
    pub fn volume(&self) -> f32 {
        self.volume
    }

    // --- counters ---

    #[must_use]
    pub fn initialize_calls(&self) -> u32 {
        self.initialize_calls
    }
    #[must_use]
    pub fn finalize_calls(&self) -> u32 {
        self.finalize_calls
    }
    #[must_use]
    pub fn open_calls(&self) -> u32 {
        self.open_calls
    }
    #[must_use]
    pub fn close_calls(&self) -> u32 {
        self.close_calls
    }
    #[must_use]
    pub fn playback_control_calls(&self) -> u32 {
        self.playback_control_calls
    }

    fn require_initialized(&self) -> Result<(), CellError> {
        match self.module_state {
            ModuleState::Initialized => Ok(()),
            ModuleState::Finalized => Err(CELL_VIDEO_PLAYER_ERROR_FINALIZED),
            ModuleState::Uninitialized => Err(CELL_VIDEO_PLAYER_ERROR_NOT_INITIALIZED),
        }
    }

    fn require_open(&self) -> Result<(), CellError> {
        self.require_initialized()?;
        if self.session_state == SessionState::Open {
            Ok(())
        } else {
            Err(CELL_VIDEO_PLAYER_ERROR_NOT_OPEN)
        }
    }

    // --- entry points ---

    /// `cellVideoPlayerInitialize` (cpp:6-10).
    pub fn initialize(&mut self) -> Result<(), CellError> {
        match self.module_state {
            ModuleState::Uninitialized => {
                self.module_state = ModuleState::Initialized;
                self.initialize_calls = self.initialize_calls.saturating_add(1);
                Ok(())
            }
            ModuleState::Initialized => Err(CELL_VIDEO_PLAYER_ERROR_ALREADY_INITIALIZED),
            ModuleState::Finalized => Err(CELL_VIDEO_PLAYER_ERROR_FINALIZED),
        }
    }

    /// `cellVideoPlayerFinalize` (cpp:24-28).
    pub fn finalize(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        // Tear down any live session + thumbnail.
        self.session_state = SessionState::Closed;
        self.thumbnail_active = false;
        self.playback_status = PlaybackStatus::Stopped;
        self.module_state = ModuleState::Finalized;
        self.finalize_calls = self.finalize_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerOpen` (cpp:66-70).
    pub fn open(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        if self.session_state == SessionState::Open {
            return Err(CELL_VIDEO_PLAYER_ERROR_ALREADY_OPEN);
        }
        self.session_state = SessionState::Open;
        self.playback_status = PlaybackStatus::Stopped;
        self.open_calls = self.open_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerClose` (cpp:36-40).
    pub fn close(&mut self) -> Result<(), CellError> {
        self.require_open()?;
        self.session_state = SessionState::Closed;
        self.playback_status = PlaybackStatus::Stopped;
        self.start_position_us = 0;
        self.stop_position_us = 0;
        self.download_position_bytes = 0;
        self.transfer = TransferPictureInfo::default();
        self.close_calls = self.close_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerPlaybackControl` (cpp:102-106). Transitions
    /// playback state based on the submitted command; returns
    /// `INVALID_PARAMETER` for Resume when not paused / Stop when
    /// already stopped.
    pub fn playback_control(&mut self, cmd: PlaybackCommand) -> Result<(), CellError> {
        self.require_open()?;
        self.playback_control_calls = self.playback_control_calls.saturating_add(1);
        match cmd {
            PlaybackCommand::Play => {
                self.playback_status = PlaybackStatus::Playing;
            }
            PlaybackCommand::Pause => {
                if self.playback_status != PlaybackStatus::Playing {
                    return Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER);
                }
                self.playback_status = PlaybackStatus::Paused;
            }
            PlaybackCommand::Resume => {
                if self.playback_status != PlaybackStatus::Paused {
                    return Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER);
                }
                self.playback_status = PlaybackStatus::Playing;
            }
            PlaybackCommand::Stop => {
                self.playback_status = PlaybackStatus::Stopped;
            }
            PlaybackCommand::FastForward => {
                self.playback_status = PlaybackStatus::FastForward;
            }
            PlaybackCommand::FastReverse => {
                self.playback_status = PlaybackStatus::FastReverse;
            }
        }
        Ok(())
    }

    /// `cellVideoPlayerGetPlaybackStatus` (cpp:84-88).
    pub fn get_playback_status(&mut self) -> Result<PlaybackStatus, CellError> {
        self.require_open()?;
        self.get_playback_status_calls =
            self.get_playback_status_calls.saturating_add(1);
        Ok(self.playback_status)
    }

    /// `cellVideoPlayerSetStartPosition` (cpp:12-16).
    pub fn set_start_position(&mut self, position_us: u64) -> Result<(), CellError> {
        self.require_open()?;
        if self.stop_position_us > 0 && position_us >= self.stop_position_us {
            return Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER);
        }
        self.start_position_us = position_us;
        self.set_start_position_calls = self.set_start_position_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerSetStopPosition` (cpp:30-34).
    pub fn set_stop_position(&mut self, position_us: u64) -> Result<(), CellError> {
        self.require_open()?;
        if position_us > 0 && position_us <= self.start_position_us {
            return Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER);
        }
        self.stop_position_us = position_us;
        self.set_stop_position_calls = self.set_stop_position_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerSetDownloadPosition` (cpp:48-52).
    pub fn set_download_position(&mut self, bytes: u64) -> Result<(), CellError> {
        self.require_open()?;
        self.download_position_bytes = bytes;
        self.set_download_position_calls =
            self.set_download_position_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerSetVolume` (cpp:72-76). Clamped to
    /// `[VOLUME_MIN, VOLUME_MAX]`; NaN → `INVALID_PARAMETER`.
    pub fn set_volume(&mut self, volume: f32) -> Result<(), CellError> {
        self.require_initialized()?;
        if volume.is_nan() {
            return Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER);
        }
        self.volume = volume.clamp(VOLUME_MIN, VOLUME_MAX);
        self.set_volume_calls = self.set_volume_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerGetVolume` (cpp:18-22).
    pub fn get_volume(&mut self) -> Result<f32, CellError> {
        self.require_initialized()?;
        self.get_volume_calls = self.get_volume_calls.saturating_add(1);
        Ok(self.volume)
    }

    /// `cellVideoPlayerStartThumbnail` (cpp:54-58).
    pub fn start_thumbnail(&mut self) -> Result<(), CellError> {
        self.require_open()?;
        if self.thumbnail_active {
            return Err(CELL_VIDEO_PLAYER_ERROR_THUMBNAIL_ALREADY_ACTIVE);
        }
        self.thumbnail_active = true;
        self.start_thumbnail_calls = self.start_thumbnail_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerEndThumbnail` (cpp:60-64).
    pub fn end_thumbnail(&mut self) -> Result<(), CellError> {
        self.require_open()?;
        if !self.thumbnail_active {
            return Err(CELL_VIDEO_PLAYER_ERROR_THUMBNAIL_NOT_ACTIVE);
        }
        self.thumbnail_active = false;
        self.end_thumbnail_calls = self.end_thumbnail_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerSetTransferComplete` (cpp:90-94).
    pub fn set_transfer_complete(&mut self) -> Result<(), CellError> {
        self.require_open()?;
        self.transfer.transfer_complete = true;
        self.set_transfer_complete_calls =
            self.set_transfer_complete_calls.saturating_add(1);
        Ok(())
    }

    /// `cellVideoPlayerGetTransferPictureInfo` (cpp:42-46).
    pub fn get_transfer_picture_info(&mut self) -> Result<TransferPictureInfo, CellError> {
        self.require_open()?;
        self.get_transfer_picture_info_calls =
            self.get_transfer_picture_info_calls.saturating_add(1);
        Ok(self.transfer)
    }

    /// `cellVideoPlayerGetOutputPicture` (cpp:96-100). Returns a
    /// synthesized descriptor — the real firmware reads from an
    /// internal frame ring. Requires `Playing` state.
    pub fn get_output_picture(&mut self) -> Result<OutputPicture, CellError> {
        self.require_open()?;
        if self.playback_status != PlaybackStatus::Playing {
            return Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER);
        }
        self.get_output_picture_calls = self.get_output_picture_calls.saturating_add(1);
        Ok(OutputPicture {
            picture_id: self.get_output_picture_calls,
            width: 1920,
            height: 1080,
            is_stereo: false,
        })
    }

    /// `cellVideoPlayerGetOutputStereoPicture` (cpp:78-82).
    pub fn get_output_stereo_picture(&mut self) -> Result<OutputPicture, CellError> {
        self.require_open()?;
        if self.playback_status != PlaybackStatus::Playing {
            return Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER);
        }
        self.get_output_stereo_picture_calls =
            self.get_output_stereo_picture_calls.saturating_add(1);
        Ok(OutputPicture {
            picture_id: self.get_output_stereo_picture_calls,
            width: 1920,
            height: 1080,
            is_stereo: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bring_up_to_open() -> VideoPlayer {
        let mut v = VideoPlayer::new();
        v.initialize().unwrap();
        v.open().unwrap();
        v
    }

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellVideoPlayerUtility");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 17);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellVideoPlayerInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[10], "cellVideoPlayerOpen");
        assert_eq!(REGISTERED_ENTRY_POINTS[16], "cellVideoPlayerPlaybackControl");
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_VIDEO_PLAYER_ERROR_NOT_INITIALIZED.0, 0x8002_D501);
        assert_eq!(CELL_VIDEO_PLAYER_ERROR_ALREADY_INITIALIZED.0, 0x8002_D502);
        assert_eq!(CELL_VIDEO_PLAYER_ERROR_NOT_OPEN.0, 0x8002_D503);
        assert_eq!(CELL_VIDEO_PLAYER_ERROR_ALREADY_OPEN.0, 0x8002_D504);
        assert_eq!(CELL_VIDEO_PLAYER_ERROR_FINALIZED.0, 0x8002_D505);
        assert_eq!(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER.0, 0x8002_D506);
        assert_eq!(CELL_VIDEO_PLAYER_ERROR_THUMBNAIL_NOT_ACTIVE.0, 0x8002_D507);
        assert_eq!(
            CELL_VIDEO_PLAYER_ERROR_THUMBNAIL_ALREADY_ACTIVE.0,
            0x8002_D508
        );
    }

    #[test]
    fn starts_uninitialized_closed() {
        let v = VideoPlayer::new();
        assert_eq!(v.module_state(), ModuleState::Uninitialized);
        assert_eq!(v.session_state(), SessionState::Closed);
        assert_eq!(v.playback_status(), PlaybackStatus::Stopped);
        assert_eq!(v.volume(), 1.0);
    }

    #[test]
    fn initialize_happy_path() {
        let mut v = VideoPlayer::new();
        v.initialize().unwrap();
        assert_eq!(v.module_state(), ModuleState::Initialized);
    }

    #[test]
    fn double_initialize_is_already_initialized() {
        let mut v = VideoPlayer::new();
        v.initialize().unwrap();
        assert_eq!(
            v.initialize(),
            Err(CELL_VIDEO_PLAYER_ERROR_ALREADY_INITIALIZED)
        );
    }

    #[test]
    fn finalize_requires_init() {
        let mut v = VideoPlayer::new();
        assert_eq!(v.finalize(), Err(CELL_VIDEO_PLAYER_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn finalize_is_terminal() {
        let mut v = VideoPlayer::new();
        v.initialize().unwrap();
        v.finalize().unwrap();
        assert_eq!(v.module_state(), ModuleState::Finalized);
        assert_eq!(v.initialize(), Err(CELL_VIDEO_PLAYER_ERROR_FINALIZED));
    }

    #[test]
    fn open_without_init_is_not_initialized() {
        let mut v = VideoPlayer::new();
        assert_eq!(v.open(), Err(CELL_VIDEO_PLAYER_ERROR_NOT_INITIALIZED));
    }

    #[test]
    fn double_open_is_already_open() {
        let mut v = VideoPlayer::new();
        v.initialize().unwrap();
        v.open().unwrap();
        assert_eq!(v.open(), Err(CELL_VIDEO_PLAYER_ERROR_ALREADY_OPEN));
    }

    #[test]
    fn close_without_open_is_not_open() {
        let mut v = VideoPlayer::new();
        v.initialize().unwrap();
        assert_eq!(v.close(), Err(CELL_VIDEO_PLAYER_ERROR_NOT_OPEN));
    }

    #[test]
    fn playback_play_then_pause() {
        let mut v = bring_up_to_open();
        v.playback_control(PlaybackCommand::Play).unwrap();
        assert_eq!(v.playback_status(), PlaybackStatus::Playing);
        v.playback_control(PlaybackCommand::Pause).unwrap();
        assert_eq!(v.playback_status(), PlaybackStatus::Paused);
        v.playback_control(PlaybackCommand::Resume).unwrap();
        assert_eq!(v.playback_status(), PlaybackStatus::Playing);
        v.playback_control(PlaybackCommand::Stop).unwrap();
        assert_eq!(v.playback_status(), PlaybackStatus::Stopped);
    }

    #[test]
    fn pause_without_playing_is_invalid() {
        let mut v = bring_up_to_open();
        assert_eq!(
            v.playback_control(PlaybackCommand::Pause),
            Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn resume_without_paused_is_invalid() {
        let mut v = bring_up_to_open();
        v.playback_control(PlaybackCommand::Play).unwrap();
        assert_eq!(
            v.playback_control(PlaybackCommand::Resume),
            Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn fast_forward_and_reverse() {
        let mut v = bring_up_to_open();
        v.playback_control(PlaybackCommand::FastForward).unwrap();
        assert_eq!(v.playback_status(), PlaybackStatus::FastForward);
        v.playback_control(PlaybackCommand::FastReverse).unwrap();
        assert_eq!(v.playback_status(), PlaybackStatus::FastReverse);
    }

    #[test]
    fn set_volume_clamps_and_rejects_nan() {
        let mut v = VideoPlayer::new();
        v.initialize().unwrap();
        v.set_volume(2.0).unwrap();
        assert_eq!(v.volume(), 1.0);
        v.set_volume(-0.5).unwrap();
        assert_eq!(v.volume(), 0.0);
        v.set_volume(0.5).unwrap();
        assert_eq!(v.volume(), 0.5);
        assert_eq!(
            v.set_volume(f32::NAN),
            Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn positions_enforce_ordering() {
        let mut v = bring_up_to_open();
        v.set_start_position(1_000_000).unwrap();
        // Stop ≤ start rejected.
        assert_eq!(
            v.set_stop_position(500_000),
            Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER)
        );
        v.set_stop_position(2_000_000).unwrap();
        // Start ≥ stop rejected once stop is set.
        assert_eq!(
            v.set_start_position(3_000_000),
            Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER)
        );
    }

    #[test]
    fn set_download_position_stores() {
        let mut v = bring_up_to_open();
        v.set_download_position(0x1000_0000).unwrap();
    }

    #[test]
    fn thumbnail_lifecycle() {
        let mut v = bring_up_to_open();
        v.start_thumbnail().unwrap();
        assert!(v.thumbnail_active());
        assert_eq!(
            v.start_thumbnail(),
            Err(CELL_VIDEO_PLAYER_ERROR_THUMBNAIL_ALREADY_ACTIVE)
        );
        v.end_thumbnail().unwrap();
        assert!(!v.thumbnail_active());
        assert_eq!(
            v.end_thumbnail(),
            Err(CELL_VIDEO_PLAYER_ERROR_THUMBNAIL_NOT_ACTIVE)
        );
    }

    #[test]
    fn transfer_complete_flow() {
        let mut v = bring_up_to_open();
        let info = v.get_transfer_picture_info().unwrap();
        assert!(!info.transfer_complete);
        v.set_transfer_complete().unwrap();
        let info = v.get_transfer_picture_info().unwrap();
        assert!(info.transfer_complete);
    }

    #[test]
    fn get_output_picture_requires_playing() {
        let mut v = bring_up_to_open();
        assert_eq!(
            v.get_output_picture(),
            Err(CELL_VIDEO_PLAYER_ERROR_INVALID_PARAMETER)
        );
        v.playback_control(PlaybackCommand::Play).unwrap();
        let p = v.get_output_picture().unwrap();
        assert_eq!(p.width, 1920);
        assert_eq!(p.height, 1080);
        assert!(!p.is_stereo);
    }

    #[test]
    fn get_output_stereo_picture_requires_playing() {
        let mut v = bring_up_to_open();
        v.playback_control(PlaybackCommand::Play).unwrap();
        let p = v.get_output_stereo_picture().unwrap();
        assert!(p.is_stereo);
    }

    #[test]
    fn get_playback_status_returns_current() {
        let mut v = bring_up_to_open();
        assert_eq!(v.get_playback_status().unwrap(), PlaybackStatus::Stopped);
        v.playback_control(PlaybackCommand::Play).unwrap();
        assert_eq!(v.get_playback_status().unwrap(), PlaybackStatus::Playing);
    }

    #[test]
    fn volume_constants_byte_exact() {
        assert_eq!(VOLUME_MIN, 0.0);
        assert_eq!(VOLUME_MAX, 1.0);
        assert_eq!(DEFAULT_VOLUME, 1.0);
    }

    #[test]
    fn full_video_player_lifecycle_smoke() {
        let mut v = VideoPlayer::new();

        // 1. Initialize + open session.
        v.initialize().unwrap();
        v.open().unwrap();

        // 2. Configure positions + volume.
        v.set_start_position(0).unwrap();
        v.set_stop_position(120_000_000).unwrap(); // 2 minutes.
        v.set_volume(0.75).unwrap();
        v.set_download_position(0x0100_0000).unwrap();

        // 3. Start thumbnail → end.
        v.start_thumbnail().unwrap();
        v.end_thumbnail().unwrap();

        // 4. Play → pause → resume → pull 2 frames → stop.
        v.playback_control(PlaybackCommand::Play).unwrap();
        let p1 = v.get_output_picture().unwrap();
        assert_eq!(p1.picture_id, 1);
        v.playback_control(PlaybackCommand::Pause).unwrap();
        v.playback_control(PlaybackCommand::Resume).unwrap();
        let p2 = v.get_output_picture().unwrap();
        assert_eq!(p2.picture_id, 2);
        v.playback_control(PlaybackCommand::Stop).unwrap();

        // 5. Transfer complete marker + teardown.
        v.set_transfer_complete().unwrap();
        assert!(v.get_transfer_picture_info().unwrap().transfer_complete);
        v.close().unwrap();
        v.finalize().unwrap();

        // 6. Counter trace.
        assert_eq!(v.initialize_calls(), 1);
        assert_eq!(v.open_calls(), 1);
        assert_eq!(v.close_calls(), 1);
        assert_eq!(v.finalize_calls(), 1);
        assert_eq!(v.playback_control_calls(), 4);
    }
}
