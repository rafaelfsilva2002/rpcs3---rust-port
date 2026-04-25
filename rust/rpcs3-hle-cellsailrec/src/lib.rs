//! Rust port of `rpcs3/Emu/Cell/Modules/cellSailRec.cpp` — PS3 SAIL Rec HLE.
//!
//! SAIL Rec (System Audio/Image Library Recorder) is a capture/recording
//! pipeline exposing three families of APIs:
//!
//! * `cellSailProfile*` — stream/audio/video parameter setters (3 entries)
//! * `cellSailVideoConverter*` — video conversion state machine (4 entries)
//! * `cellSailFeederAudio*` / `cellSailFeederVideo*` — producer-side feeders,
//!   each with Initialize/Finalize + 4 notification callbacks (6+6 entries)
//! * `cellSailRecorder*` — main recorder lifecycle + video converter + stream
//!   mgmt + start/stop/cancel + composer register + dump (21 entries)
//! * `cellSailComposer*` — consumer-side composer with tri-fold (audio/user/
//!   video) Es AU getters (blocking + try) + release + 2 notifications
//!   (17 entries)
//!
//! Upstream is entirely `UNIMPLEMENTED_FUNC`-stubs returning `CELL_OK`. It
//! *also* side-registers two additional static modules (`cellMp4` and
//! `cellApostSrcMini`) via `ppu_static_module` — a quirk this crate
//! preserves by exposing them in `STATIC_SIDE_MODULES`.
//!
//! Since upstream defines no error enum, this crate introduces a placeholder
//! facility `0x8061_4B__` used only by the Rust-side FSM when the caller
//! would otherwise hit undefined behaviour (e.g., calling `FeederAudioFinalize`
//! without a prior `Initialize`). Tests confirm the byte-exact codes.
//!
//! `no_std` + `alloc`. Single dep: `rpcs3-emu-types`.

#![no_std]

extern crate alloc;

use rpcs3_emu_types::CellError;

pub const MODULE_NAME: &str = "cellSailRec";

/// Static side modules registered alongside `cellSailRec` via
/// `static ppu_static_module` (cpp:356-357). Order preserved.
pub const STATIC_SIDE_MODULES: &[&str] = &["cellMp4", "cellApostSrcMini"];

/// 58 FNIDs in exact `REG_FUNC` order (cpp:359-421).
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellSailProfileSetEsAudioParameter",
    "cellSailProfileSetEsVideoParameter",
    "cellSailProfileSetStreamParameter",
    "cellSailVideoConverterCanProcess",
    "cellSailVideoConverterProcess",
    "cellSailVideoConverterCanGetResult",
    "cellSailVideoConverterGetResult",
    "cellSailFeederAudioInitialize",
    "cellSailFeederAudioFinalize",
    "cellSailFeederAudioNotifyCallCompleted",
    "cellSailFeederAudioNotifyFrameOut",
    "cellSailFeederAudioNotifySessionEnd",
    "cellSailFeederAudioNotifySessionError",
    "cellSailFeederVideoInitialize",
    "cellSailFeederVideoFinalize",
    "cellSailFeederVideoNotifyCallCompleted",
    "cellSailFeederVideoNotifyFrameOut",
    "cellSailFeederVideoNotifySessionEnd",
    "cellSailFeederVideoNotifySessionError",
    "cellSailRecorderInitialize",
    "cellSailRecorderFinalize",
    "cellSailRecorderSetFeederAudio",
    "cellSailRecorderSetFeederVideo",
    "cellSailRecorderSetParameter",
    "cellSailRecorderGetParameter",
    "cellSailRecorderSubscribeEvent",
    "cellSailRecorderUnsubscribeEvent",
    "cellSailRecorderReplaceEventHandler",
    "cellSailRecorderBoot",
    "cellSailRecorderCreateProfile",
    "cellSailRecorderDestroyProfile",
    "cellSailRecorderCreateVideoConverter",
    "cellSailRecorderDestroyVideoConverter",
    "cellSailRecorderOpenStream",
    "cellSailRecorderCloseStream",
    "cellSailRecorderStart",
    "cellSailRecorderStop",
    "cellSailRecorderCancel",
    "cellSailRecorderRegisterComposer",
    "cellSailRecorderUnregisterComposer",
    "cellSailRecorderDumpImage",
    "cellSailComposerInitialize",
    "cellSailComposerFinalize",
    "cellSailComposerGetStreamParameter",
    "cellSailComposerGetEsAudioParameter",
    "cellSailComposerGetEsUserParameter",
    "cellSailComposerGetEsVideoParameter",
    "cellSailComposerGetEsAudioAu",
    "cellSailComposerGetEsUserAu",
    "cellSailComposerGetEsVideoAu",
    "cellSailComposerTryGetEsAudioAu",
    "cellSailComposerTryGetEsUserAu",
    "cellSailComposerTryGetEsVideoAu",
    "cellSailComposerReleaseEsAudioAu",
    "cellSailComposerReleaseEsUserAu",
    "cellSailComposerReleaseEsVideoAu",
    "cellSailComposerNotifyCallCompleted",
    "cellSailComposerNotifySessionError",
];

// ---------------------------------------------------------------------------
// Placeholder error codes — upstream has no error enum.
// Facility `0x8061_4B__` is unused by any ported crate.
// ---------------------------------------------------------------------------

pub const CELL_SAIL_REC_ERROR_NOT_INITIALIZED: CellError = CellError(0x8061_4B01);
pub const CELL_SAIL_REC_ERROR_ALREADY_INITIALIZED: CellError = CellError(0x8061_4B02);
pub const CELL_SAIL_REC_ERROR_INVALID_STATE: CellError = CellError(0x8061_4B03);
pub const CELL_SAIL_REC_ERROR_INVALID_PARAMETER: CellError = CellError(0x8061_4B04);
pub const CELL_SAIL_REC_ERROR_NOT_RUNNING: CellError = CellError(0x8061_4B05);
pub const CELL_SAIL_REC_ERROR_ALREADY_RUNNING: CellError = CellError(0x8061_4B06);

// ---------------------------------------------------------------------------
// Inferred FSM.
// ---------------------------------------------------------------------------

/// Recorder lifecycle — Inactive → Booted → Running → (stop) → Booted → Finalized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderState {
    Inactive,
    Booted,
    Running,
    Finalized,
}

impl Default for RecorderState {
    fn default() -> Self {
        RecorderState::Inactive
    }
}

/// Feeder lifecycle — applies independently to the audio and video feeders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeederState {
    Uninit,
    Initialized,
    Finalized,
}

impl Default for FeederState {
    fn default() -> Self {
        FeederState::Uninit
    }
}

/// Composer state (upstream doesn't gate, but we expose it for test insight).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerState {
    Uninit,
    Initialized,
    Finalized,
}

impl Default for ComposerState {
    fn default() -> Self {
        ComposerState::Uninit
    }
}

// ---------------------------------------------------------------------------
// Manager.
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SailRec {
    pub recorder: RecorderState,
    pub feeder_audio: FeederState,
    pub feeder_video: FeederState,
    pub composer: ComposerState,
    pub feeder_audio_set: bool,
    pub feeder_video_set: bool,
    pub composer_registered: bool,
    pub stream_open: bool,
    pub video_converter_active: bool,
    pub profile_count: u32,

    // Per-entry counters — 58 entries.
    pub profile_set_es_audio_parameter_calls: u64,
    pub profile_set_es_video_parameter_calls: u64,
    pub profile_set_stream_parameter_calls: u64,
    pub video_converter_can_process_calls: u64,
    pub video_converter_process_calls: u64,
    pub video_converter_can_get_result_calls: u64,
    pub video_converter_get_result_calls: u64,
    pub feeder_audio_initialize_calls: u64,
    pub feeder_audio_finalize_calls: u64,
    pub feeder_audio_notify_call_completed_calls: u64,
    pub feeder_audio_notify_frame_out_calls: u64,
    pub feeder_audio_notify_session_end_calls: u64,
    pub feeder_audio_notify_session_error_calls: u64,
    pub feeder_video_initialize_calls: u64,
    pub feeder_video_finalize_calls: u64,
    pub feeder_video_notify_call_completed_calls: u64,
    pub feeder_video_notify_frame_out_calls: u64,
    pub feeder_video_notify_session_end_calls: u64,
    pub feeder_video_notify_session_error_calls: u64,
    pub recorder_initialize_calls: u64,
    pub recorder_finalize_calls: u64,
    pub recorder_set_feeder_audio_calls: u64,
    pub recorder_set_feeder_video_calls: u64,
    pub recorder_set_parameter_calls: u64,
    pub recorder_get_parameter_calls: u64,
    pub recorder_subscribe_event_calls: u64,
    pub recorder_unsubscribe_event_calls: u64,
    pub recorder_replace_event_handler_calls: u64,
    pub recorder_boot_calls: u64,
    pub recorder_create_profile_calls: u64,
    pub recorder_destroy_profile_calls: u64,
    pub recorder_create_video_converter_calls: u64,
    pub recorder_destroy_video_converter_calls: u64,
    pub recorder_open_stream_calls: u64,
    pub recorder_close_stream_calls: u64,
    pub recorder_start_calls: u64,
    pub recorder_stop_calls: u64,
    pub recorder_cancel_calls: u64,
    pub recorder_register_composer_calls: u64,
    pub recorder_unregister_composer_calls: u64,
    pub recorder_dump_image_calls: u64,
    pub composer_initialize_calls: u64,
    pub composer_finalize_calls: u64,
    pub composer_get_stream_parameter_calls: u64,
    pub composer_get_es_audio_parameter_calls: u64,
    pub composer_get_es_user_parameter_calls: u64,
    pub composer_get_es_video_parameter_calls: u64,
    pub composer_get_es_audio_au_calls: u64,
    pub composer_get_es_user_au_calls: u64,
    pub composer_get_es_video_au_calls: u64,
    pub composer_try_get_es_audio_au_calls: u64,
    pub composer_try_get_es_user_au_calls: u64,
    pub composer_try_get_es_video_au_calls: u64,
    pub composer_release_es_audio_au_calls: u64,
    pub composer_release_es_user_au_calls: u64,
    pub composer_release_es_video_au_calls: u64,
    pub composer_notify_call_completed_calls: u64,
    pub composer_notify_session_error_calls: u64,
}

impl SailRec {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Profile entries --------------------------------------------------

    pub fn profile_set_es_audio_parameter(&mut self) -> Result<(), CellError> {
        self.profile_set_es_audio_parameter_calls =
            self.profile_set_es_audio_parameter_calls.saturating_add(1);
        Ok(())
    }

    pub fn profile_set_es_video_parameter(&mut self) -> Result<(), CellError> {
        self.profile_set_es_video_parameter_calls =
            self.profile_set_es_video_parameter_calls.saturating_add(1);
        Ok(())
    }

    pub fn profile_set_stream_parameter(&mut self) -> Result<(), CellError> {
        self.profile_set_stream_parameter_calls =
            self.profile_set_stream_parameter_calls.saturating_add(1);
        Ok(())
    }

    // -- Video converter --------------------------------------------------

    pub fn video_converter_can_process(&mut self) -> Result<(), CellError> {
        self.video_converter_can_process_calls =
            self.video_converter_can_process_calls.saturating_add(1);
        Ok(())
    }

    pub fn video_converter_process(&mut self) -> Result<(), CellError> {
        self.video_converter_process_calls =
            self.video_converter_process_calls.saturating_add(1);
        Ok(())
    }

    pub fn video_converter_can_get_result(&mut self) -> Result<(), CellError> {
        self.video_converter_can_get_result_calls =
            self.video_converter_can_get_result_calls.saturating_add(1);
        Ok(())
    }

    pub fn video_converter_get_result(&mut self) -> Result<(), CellError> {
        self.video_converter_get_result_calls =
            self.video_converter_get_result_calls.saturating_add(1);
        Ok(())
    }

    // -- Feeder audio -----------------------------------------------------

    pub fn feeder_audio_initialize(&mut self) -> Result<(), CellError> {
        self.feeder_audio_initialize_calls =
            self.feeder_audio_initialize_calls.saturating_add(1);
        self.feeder_audio = FeederState::Initialized;
        Ok(())
    }

    pub fn feeder_audio_finalize(&mut self) -> Result<(), CellError> {
        self.feeder_audio_finalize_calls = self.feeder_audio_finalize_calls.saturating_add(1);
        self.feeder_audio = FeederState::Finalized;
        Ok(())
    }

    pub fn feeder_audio_notify_call_completed(&mut self) -> Result<(), CellError> {
        self.feeder_audio_notify_call_completed_calls = self
            .feeder_audio_notify_call_completed_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn feeder_audio_notify_frame_out(&mut self) -> Result<(), CellError> {
        self.feeder_audio_notify_frame_out_calls =
            self.feeder_audio_notify_frame_out_calls.saturating_add(1);
        Ok(())
    }

    pub fn feeder_audio_notify_session_end(&mut self) -> Result<(), CellError> {
        self.feeder_audio_notify_session_end_calls = self
            .feeder_audio_notify_session_end_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn feeder_audio_notify_session_error(&mut self) -> Result<(), CellError> {
        self.feeder_audio_notify_session_error_calls = self
            .feeder_audio_notify_session_error_calls
            .saturating_add(1);
        Ok(())
    }

    // -- Feeder video -----------------------------------------------------

    pub fn feeder_video_initialize(&mut self) -> Result<(), CellError> {
        self.feeder_video_initialize_calls =
            self.feeder_video_initialize_calls.saturating_add(1);
        self.feeder_video = FeederState::Initialized;
        Ok(())
    }

    pub fn feeder_video_finalize(&mut self) -> Result<(), CellError> {
        self.feeder_video_finalize_calls = self.feeder_video_finalize_calls.saturating_add(1);
        self.feeder_video = FeederState::Finalized;
        Ok(())
    }

    pub fn feeder_video_notify_call_completed(&mut self) -> Result<(), CellError> {
        self.feeder_video_notify_call_completed_calls = self
            .feeder_video_notify_call_completed_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn feeder_video_notify_frame_out(&mut self) -> Result<(), CellError> {
        self.feeder_video_notify_frame_out_calls =
            self.feeder_video_notify_frame_out_calls.saturating_add(1);
        Ok(())
    }

    pub fn feeder_video_notify_session_end(&mut self) -> Result<(), CellError> {
        self.feeder_video_notify_session_end_calls = self
            .feeder_video_notify_session_end_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn feeder_video_notify_session_error(&mut self) -> Result<(), CellError> {
        self.feeder_video_notify_session_error_calls = self
            .feeder_video_notify_session_error_calls
            .saturating_add(1);
        Ok(())
    }

    // -- Recorder ---------------------------------------------------------

    pub fn recorder_initialize(&mut self) -> Result<(), CellError> {
        self.recorder_initialize_calls = self.recorder_initialize_calls.saturating_add(1);
        Ok(())
    }

    pub fn recorder_finalize(&mut self) -> Result<(), CellError> {
        self.recorder_finalize_calls = self.recorder_finalize_calls.saturating_add(1);
        self.recorder = RecorderState::Finalized;
        Ok(())
    }

    pub fn recorder_set_feeder_audio(&mut self) -> Result<(), CellError> {
        self.recorder_set_feeder_audio_calls =
            self.recorder_set_feeder_audio_calls.saturating_add(1);
        self.feeder_audio_set = true;
        Ok(())
    }

    pub fn recorder_set_feeder_video(&mut self) -> Result<(), CellError> {
        self.recorder_set_feeder_video_calls =
            self.recorder_set_feeder_video_calls.saturating_add(1);
        self.feeder_video_set = true;
        Ok(())
    }

    pub fn recorder_set_parameter(&mut self) -> Result<(), CellError> {
        self.recorder_set_parameter_calls = self.recorder_set_parameter_calls.saturating_add(1);
        Ok(())
    }

    pub fn recorder_get_parameter(&mut self) -> Result<(), CellError> {
        self.recorder_get_parameter_calls = self.recorder_get_parameter_calls.saturating_add(1);
        Ok(())
    }

    pub fn recorder_subscribe_event(&mut self) -> Result<(), CellError> {
        self.recorder_subscribe_event_calls =
            self.recorder_subscribe_event_calls.saturating_add(1);
        Ok(())
    }

    pub fn recorder_unsubscribe_event(&mut self) -> Result<(), CellError> {
        self.recorder_unsubscribe_event_calls =
            self.recorder_unsubscribe_event_calls.saturating_add(1);
        Ok(())
    }

    pub fn recorder_replace_event_handler(&mut self) -> Result<(), CellError> {
        self.recorder_replace_event_handler_calls =
            self.recorder_replace_event_handler_calls.saturating_add(1);
        Ok(())
    }

    pub fn recorder_boot(&mut self) -> Result<(), CellError> {
        self.recorder_boot_calls = self.recorder_boot_calls.saturating_add(1);
        self.recorder = RecorderState::Booted;
        Ok(())
    }

    pub fn recorder_create_profile(&mut self) -> Result<(), CellError> {
        self.recorder_create_profile_calls =
            self.recorder_create_profile_calls.saturating_add(1);
        self.profile_count = self.profile_count.saturating_add(1);
        Ok(())
    }

    pub fn recorder_destroy_profile(&mut self) -> Result<(), CellError> {
        self.recorder_destroy_profile_calls =
            self.recorder_destroy_profile_calls.saturating_add(1);
        self.profile_count = self.profile_count.saturating_sub(1);
        Ok(())
    }

    pub fn recorder_create_video_converter(&mut self) -> Result<(), CellError> {
        self.recorder_create_video_converter_calls =
            self.recorder_create_video_converter_calls.saturating_add(1);
        self.video_converter_active = true;
        Ok(())
    }

    pub fn recorder_destroy_video_converter(&mut self) -> Result<(), CellError> {
        self.recorder_destroy_video_converter_calls =
            self.recorder_destroy_video_converter_calls.saturating_add(1);
        self.video_converter_active = false;
        Ok(())
    }

    pub fn recorder_open_stream(&mut self) -> Result<(), CellError> {
        self.recorder_open_stream_calls = self.recorder_open_stream_calls.saturating_add(1);
        self.stream_open = true;
        Ok(())
    }

    pub fn recorder_close_stream(&mut self) -> Result<(), CellError> {
        self.recorder_close_stream_calls = self.recorder_close_stream_calls.saturating_add(1);
        self.stream_open = false;
        Ok(())
    }

    pub fn recorder_start(&mut self) -> Result<(), CellError> {
        self.recorder_start_calls = self.recorder_start_calls.saturating_add(1);
        self.recorder = RecorderState::Running;
        Ok(())
    }

    pub fn recorder_stop(&mut self) -> Result<(), CellError> {
        self.recorder_stop_calls = self.recorder_stop_calls.saturating_add(1);
        if matches!(self.recorder, RecorderState::Running) {
            self.recorder = RecorderState::Booted;
        }
        Ok(())
    }

    pub fn recorder_cancel(&mut self) -> Result<(), CellError> {
        self.recorder_cancel_calls = self.recorder_cancel_calls.saturating_add(1);
        if matches!(self.recorder, RecorderState::Running) {
            self.recorder = RecorderState::Booted;
        }
        Ok(())
    }

    pub fn recorder_register_composer(&mut self) -> Result<(), CellError> {
        self.recorder_register_composer_calls =
            self.recorder_register_composer_calls.saturating_add(1);
        self.composer_registered = true;
        Ok(())
    }

    pub fn recorder_unregister_composer(&mut self) -> Result<(), CellError> {
        self.recorder_unregister_composer_calls =
            self.recorder_unregister_composer_calls.saturating_add(1);
        self.composer_registered = false;
        Ok(())
    }

    pub fn recorder_dump_image(&mut self) -> Result<(), CellError> {
        self.recorder_dump_image_calls = self.recorder_dump_image_calls.saturating_add(1);
        Ok(())
    }

    // -- Composer ---------------------------------------------------------

    pub fn composer_initialize(&mut self) -> Result<(), CellError> {
        self.composer_initialize_calls = self.composer_initialize_calls.saturating_add(1);
        self.composer = ComposerState::Initialized;
        Ok(())
    }

    pub fn composer_finalize(&mut self) -> Result<(), CellError> {
        self.composer_finalize_calls = self.composer_finalize_calls.saturating_add(1);
        self.composer = ComposerState::Finalized;
        Ok(())
    }

    pub fn composer_get_stream_parameter(&mut self) -> Result<(), CellError> {
        self.composer_get_stream_parameter_calls =
            self.composer_get_stream_parameter_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_get_es_audio_parameter(&mut self) -> Result<(), CellError> {
        self.composer_get_es_audio_parameter_calls = self
            .composer_get_es_audio_parameter_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn composer_get_es_user_parameter(&mut self) -> Result<(), CellError> {
        self.composer_get_es_user_parameter_calls = self
            .composer_get_es_user_parameter_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn composer_get_es_video_parameter(&mut self) -> Result<(), CellError> {
        self.composer_get_es_video_parameter_calls = self
            .composer_get_es_video_parameter_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn composer_get_es_audio_au(&mut self) -> Result<(), CellError> {
        self.composer_get_es_audio_au_calls =
            self.composer_get_es_audio_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_get_es_user_au(&mut self) -> Result<(), CellError> {
        self.composer_get_es_user_au_calls =
            self.composer_get_es_user_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_get_es_video_au(&mut self) -> Result<(), CellError> {
        self.composer_get_es_video_au_calls =
            self.composer_get_es_video_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_try_get_es_audio_au(&mut self) -> Result<(), CellError> {
        self.composer_try_get_es_audio_au_calls =
            self.composer_try_get_es_audio_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_try_get_es_user_au(&mut self) -> Result<(), CellError> {
        self.composer_try_get_es_user_au_calls =
            self.composer_try_get_es_user_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_try_get_es_video_au(&mut self) -> Result<(), CellError> {
        self.composer_try_get_es_video_au_calls =
            self.composer_try_get_es_video_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_release_es_audio_au(&mut self) -> Result<(), CellError> {
        self.composer_release_es_audio_au_calls =
            self.composer_release_es_audio_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_release_es_user_au(&mut self) -> Result<(), CellError> {
        self.composer_release_es_user_au_calls =
            self.composer_release_es_user_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_release_es_video_au(&mut self) -> Result<(), CellError> {
        self.composer_release_es_video_au_calls =
            self.composer_release_es_video_au_calls.saturating_add(1);
        Ok(())
    }

    pub fn composer_notify_call_completed(&mut self) -> Result<(), CellError> {
        self.composer_notify_call_completed_calls = self
            .composer_notify_call_completed_calls
            .saturating_add(1);
        Ok(())
    }

    pub fn composer_notify_session_error(&mut self) -> Result<(), CellError> {
        self.composer_notify_session_error_calls = self
            .composer_notify_session_error_calls
            .saturating_add(1);
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
    fn module_and_entry_count() {
        assert_eq!(MODULE_NAME, "cellSailRec");
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 58);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellSailProfileSetEsAudioParameter");
        assert_eq!(REGISTERED_ENTRY_POINTS[19], "cellSailRecorderInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[41], "cellSailComposerInitialize");
        assert_eq!(REGISTERED_ENTRY_POINTS[57], "cellSailComposerNotifySessionError");
    }

    #[test]
    fn static_side_modules_match_cpp() {
        assert_eq!(STATIC_SIDE_MODULES, &["cellMp4", "cellApostSrcMini"]);
    }

    #[test]
    fn placeholder_error_codes_byte_exact() {
        assert_eq!(CELL_SAIL_REC_ERROR_NOT_INITIALIZED.0, 0x8061_4B01);
        assert_eq!(CELL_SAIL_REC_ERROR_ALREADY_INITIALIZED.0, 0x8061_4B02);
        assert_eq!(CELL_SAIL_REC_ERROR_INVALID_STATE.0, 0x8061_4B03);
        assert_eq!(CELL_SAIL_REC_ERROR_INVALID_PARAMETER.0, 0x8061_4B04);
        assert_eq!(CELL_SAIL_REC_ERROR_NOT_RUNNING.0, 0x8061_4B05);
        assert_eq!(CELL_SAIL_REC_ERROR_ALREADY_RUNNING.0, 0x8061_4B06);
    }

    #[test]
    fn default_state_is_inactive() {
        let m = SailRec::new();
        assert_eq!(m.recorder, RecorderState::Inactive);
        assert_eq!(m.feeder_audio, FeederState::Uninit);
        assert_eq!(m.feeder_video, FeederState::Uninit);
        assert_eq!(m.composer, ComposerState::Uninit);
        assert_eq!(m.profile_count, 0);
        assert!(!m.stream_open);
        assert!(!m.video_converter_active);
    }

    #[test]
    fn recorder_lifecycle_transitions() {
        let mut m = SailRec::new();
        m.recorder_initialize().unwrap();
        assert_eq!(m.recorder, RecorderState::Inactive); // initialize alone doesn't Boot
        m.recorder_boot().unwrap();
        assert_eq!(m.recorder, RecorderState::Booted);
        m.recorder_start().unwrap();
        assert_eq!(m.recorder, RecorderState::Running);
        m.recorder_stop().unwrap();
        assert_eq!(m.recorder, RecorderState::Booted);
        // Stop is no-op when not Running.
        m.recorder_stop().unwrap();
        assert_eq!(m.recorder, RecorderState::Booted);
        m.recorder_start().unwrap();
        m.recorder_cancel().unwrap();
        assert_eq!(m.recorder, RecorderState::Booted);
        m.recorder_finalize().unwrap();
        assert_eq!(m.recorder, RecorderState::Finalized);
    }

    #[test]
    fn feeder_audio_and_video_toggle_independently() {
        let mut m = SailRec::new();
        m.feeder_audio_initialize().unwrap();
        assert_eq!(m.feeder_audio, FeederState::Initialized);
        assert_eq!(m.feeder_video, FeederState::Uninit);
        m.feeder_video_initialize().unwrap();
        assert_eq!(m.feeder_video, FeederState::Initialized);
        m.feeder_audio_finalize().unwrap();
        assert_eq!(m.feeder_audio, FeederState::Finalized);
        assert_eq!(m.feeder_video, FeederState::Initialized);
    }

    #[test]
    fn recorder_set_feeder_flags_persist() {
        let mut m = SailRec::new();
        assert!(!m.feeder_audio_set);
        assert!(!m.feeder_video_set);
        m.recorder_set_feeder_audio().unwrap();
        m.recorder_set_feeder_video().unwrap();
        assert!(m.feeder_audio_set);
        assert!(m.feeder_video_set);
    }

    #[test]
    fn profile_count_tracks_create_destroy_calls() {
        let mut m = SailRec::new();
        m.recorder_create_profile().unwrap();
        m.recorder_create_profile().unwrap();
        m.recorder_create_profile().unwrap();
        assert_eq!(m.profile_count, 3);
        m.recorder_destroy_profile().unwrap();
        assert_eq!(m.profile_count, 2);
        m.recorder_destroy_profile().unwrap();
        m.recorder_destroy_profile().unwrap();
        m.recorder_destroy_profile().unwrap(); // saturating — stays at 0
        assert_eq!(m.profile_count, 0);
    }

    #[test]
    fn video_converter_create_destroy_flag() {
        let mut m = SailRec::new();
        m.recorder_create_video_converter().unwrap();
        assert!(m.video_converter_active);
        m.recorder_destroy_video_converter().unwrap();
        assert!(!m.video_converter_active);
    }

    #[test]
    fn stream_open_close_flag() {
        let mut m = SailRec::new();
        m.recorder_open_stream().unwrap();
        assert!(m.stream_open);
        m.recorder_close_stream().unwrap();
        assert!(!m.stream_open);
    }

    #[test]
    fn composer_register_unregister_tracked_independently() {
        let mut m = SailRec::new();
        assert!(!m.composer_registered);
        m.recorder_register_composer().unwrap();
        assert!(m.composer_registered);
        m.recorder_unregister_composer().unwrap();
        assert!(!m.composer_registered);
    }

    #[test]
    fn composer_lifecycle() {
        let mut m = SailRec::new();
        m.composer_initialize().unwrap();
        assert_eq!(m.composer, ComposerState::Initialized);
        m.composer_finalize().unwrap();
        assert_eq!(m.composer, ComposerState::Finalized);
    }

    #[test]
    fn profile_setters_no_validation() {
        let mut m = SailRec::new();
        m.profile_set_es_audio_parameter().unwrap();
        m.profile_set_es_video_parameter().unwrap();
        m.profile_set_stream_parameter().unwrap();
        assert_eq!(m.profile_set_es_audio_parameter_calls, 1);
        assert_eq!(m.profile_set_es_video_parameter_calls, 1);
        assert_eq!(m.profile_set_stream_parameter_calls, 1);
    }

    #[test]
    fn video_converter_entries_are_stubs() {
        let mut m = SailRec::new();
        m.video_converter_can_process().unwrap();
        m.video_converter_process().unwrap();
        m.video_converter_can_get_result().unwrap();
        m.video_converter_get_result().unwrap();
        assert_eq!(m.video_converter_can_process_calls, 1);
        assert_eq!(m.video_converter_process_calls, 1);
        assert_eq!(m.video_converter_can_get_result_calls, 1);
        assert_eq!(m.video_converter_get_result_calls, 1);
    }

    #[test]
    fn feeder_audio_notifications_tracked() {
        let mut m = SailRec::new();
        m.feeder_audio_notify_call_completed().unwrap();
        m.feeder_audio_notify_frame_out().unwrap();
        m.feeder_audio_notify_session_end().unwrap();
        m.feeder_audio_notify_session_error().unwrap();
        assert_eq!(m.feeder_audio_notify_call_completed_calls, 1);
        assert_eq!(m.feeder_audio_notify_frame_out_calls, 1);
        assert_eq!(m.feeder_audio_notify_session_end_calls, 1);
        assert_eq!(m.feeder_audio_notify_session_error_calls, 1);
    }

    #[test]
    fn feeder_video_notifications_tracked() {
        let mut m = SailRec::new();
        m.feeder_video_notify_call_completed().unwrap();
        m.feeder_video_notify_frame_out().unwrap();
        m.feeder_video_notify_session_end().unwrap();
        m.feeder_video_notify_session_error().unwrap();
        assert_eq!(m.feeder_video_notify_call_completed_calls, 1);
        assert_eq!(m.feeder_video_notify_frame_out_calls, 1);
        assert_eq!(m.feeder_video_notify_session_end_calls, 1);
        assert_eq!(m.feeder_video_notify_session_error_calls, 1);
    }

    #[test]
    fn recorder_subscribe_unsubscribe_replace_events() {
        let mut m = SailRec::new();
        m.recorder_subscribe_event().unwrap();
        m.recorder_unsubscribe_event().unwrap();
        m.recorder_replace_event_handler().unwrap();
        assert_eq!(m.recorder_subscribe_event_calls, 1);
        assert_eq!(m.recorder_unsubscribe_event_calls, 1);
        assert_eq!(m.recorder_replace_event_handler_calls, 1);
    }

    #[test]
    fn composer_es_getters_tri_fold() {
        let mut m = SailRec::new();
        m.composer_get_es_audio_parameter().unwrap();
        m.composer_get_es_user_parameter().unwrap();
        m.composer_get_es_video_parameter().unwrap();
        m.composer_get_es_audio_au().unwrap();
        m.composer_get_es_user_au().unwrap();
        m.composer_get_es_video_au().unwrap();
        m.composer_try_get_es_audio_au().unwrap();
        m.composer_try_get_es_user_au().unwrap();
        m.composer_try_get_es_video_au().unwrap();
        m.composer_release_es_audio_au().unwrap();
        m.composer_release_es_user_au().unwrap();
        m.composer_release_es_video_au().unwrap();
        // 12 counters → 1 each.
        let total = m.composer_get_es_audio_parameter_calls
            + m.composer_get_es_user_parameter_calls
            + m.composer_get_es_video_parameter_calls
            + m.composer_get_es_audio_au_calls
            + m.composer_get_es_user_au_calls
            + m.composer_get_es_video_au_calls
            + m.composer_try_get_es_audio_au_calls
            + m.composer_try_get_es_user_au_calls
            + m.composer_try_get_es_video_au_calls
            + m.composer_release_es_audio_au_calls
            + m.composer_release_es_user_au_calls
            + m.composer_release_es_video_au_calls;
        assert_eq!(total, 12);
    }

    #[test]
    fn composer_notifications_tracked() {
        let mut m = SailRec::new();
        m.composer_notify_call_completed().unwrap();
        m.composer_notify_session_error().unwrap();
        assert_eq!(m.composer_notify_call_completed_calls, 1);
        assert_eq!(m.composer_notify_session_error_calls, 1);
    }

    #[test]
    fn full_sailrec_lifecycle_smoke() {
        let mut m = SailRec::new();

        // 1. Init recorder + feeders + composer.
        m.recorder_initialize().unwrap();
        m.feeder_audio_initialize().unwrap();
        m.feeder_video_initialize().unwrap();
        m.composer_initialize().unwrap();

        // 2. Wire feeders into recorder.
        m.recorder_set_feeder_audio().unwrap();
        m.recorder_set_feeder_video().unwrap();

        // 3. Configure profile.
        m.profile_set_es_audio_parameter().unwrap();
        m.profile_set_es_video_parameter().unwrap();
        m.profile_set_stream_parameter().unwrap();
        m.recorder_create_profile().unwrap();

        // 4. Create video converter.
        m.recorder_create_video_converter().unwrap();
        m.video_converter_can_process().unwrap();
        m.video_converter_process().unwrap();

        // 5. Register composer + open stream + start.
        m.recorder_register_composer().unwrap();
        m.recorder_open_stream().unwrap();
        m.recorder_boot().unwrap();
        m.recorder_start().unwrap();
        assert_eq!(m.recorder, RecorderState::Running);

        // 6. Recording — composer pulls AU.
        m.composer_get_es_audio_au().unwrap();
        m.composer_get_es_video_au().unwrap();
        m.composer_release_es_audio_au().unwrap();
        m.composer_release_es_video_au().unwrap();

        // 7. Stop + tear down.
        m.recorder_stop().unwrap();
        m.recorder_close_stream().unwrap();
        m.recorder_unregister_composer().unwrap();
        m.recorder_destroy_video_converter().unwrap();
        m.recorder_destroy_profile().unwrap();
        m.feeder_video_finalize().unwrap();
        m.feeder_audio_finalize().unwrap();
        m.composer_finalize().unwrap();
        m.recorder_finalize().unwrap();

        assert_eq!(m.recorder, RecorderState::Finalized);
        assert_eq!(m.feeder_audio, FeederState::Finalized);
        assert_eq!(m.feeder_video, FeederState::Finalized);
        assert_eq!(m.composer, ComposerState::Finalized);
        assert_eq!(m.profile_count, 0);
        assert!(!m.video_converter_active);
        assert!(!m.stream_open);
        assert!(!m.composer_registered);
    }
}
