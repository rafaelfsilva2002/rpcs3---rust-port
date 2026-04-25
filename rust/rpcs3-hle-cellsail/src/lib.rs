//! `rpcs3-hle-cellsail` — Streaming AV Interface Library (SAIL) HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSail.cpp`. SAIL is PS3's high-level
//! media-playback framework: it wires together a demuxer (`cellDmux`), audio
//! decoder (`cellAdec`), video decoder (`cellVdec`), post-processor
//! (`cellVpost`) and output adapters. The Player object enforces a
//! well-defined FSM and routes `cellSail*Player*` calls through it.
//!
//! ## Entry points covered
//!
//! | HLE function                           | Rust wrapper                         |
//! |----------------------------------------|--------------------------------------|
//! | `cellSailPlayerInitialize`             | [`Player::new`]                      |
//! | `cellSailPlayerFinalize`               | [`Player::finalize`]                 |
//! | `cellSailPlayerBoot`                   | [`Player::boot`]                     |
//! | `cellSailPlayerCreateDescriptor`       | [`Player::create_descriptor`]        |
//! | `cellSailPlayerAddDescriptor`          | [`Player::add_descriptor`]           |
//! | `cellSailPlayerRemoveDescriptor`       | [`Player::remove_descriptor`]        |
//! | `cellSailPlayerGetDescriptorCount`     | [`Player::descriptor_count`]         |
//! | `cellSailPlayerOpenStream`             | [`Player::open_stream`]              |
//! | `cellSailPlayerCloseStream`            | [`Player::close_stream`]             |
//! | `cellSailPlayerStart` / `Stop`         | [`Player::start`] / [`Player::stop`] |
//! | `cellSailPlayerSetPaused`              | [`Player::set_paused`]               |
//! | `cellSailPlayerSetParameter` / `Get`   | [`Player::set_parameter`] / `get`    |
//! | `cellSailPlayerSetPreset`              | [`Player::set_preset`]               |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellSail.h:7-21
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const INVALID_ARG: CellError = CellError(0x8061_0701);
    pub const INVALID_STATE: CellError = CellError(0x8061_0702);
    pub const UNSUPPORTED_STREAM: CellError = CellError(0x8061_0703);
    pub const INDEX_OUT_OF_RANGE: CellError = CellError(0x8061_0704);
    pub const EMPTY: CellError = CellError(0x8061_0705);
    pub const FULLED: CellError = CellError(0x8061_0706);
    pub const USING: CellError = CellError(0x8061_0707);
    pub const NOT_AVAILABLE: CellError = CellError(0x8061_0708);
    pub const CANCEL: CellError = CellError(0x8061_0709);
    pub const MEMORY: CellError = CellError(0x8061_07F0);
    pub const INVALID_FD: CellError = CellError(0x8061_07F1);
    pub const FATAL: CellError = CellError(0x8061_07FF);
}

// =====================================================================
// Player FSM (cellSail.h:47-60)
// =====================================================================

#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerState {
    Initialized = 0,
    BootTransition = 1,
    Closed = 2,
    OpenTransition = 3,
    Opened = 4,
    StartTransition = 5,
    Running = 6,
    StopTransition = 7,
    CloseTransition = 8,
    Lost = 9,
}

// =====================================================================
// Stream types (cellSail.h:173-180)
// =====================================================================

pub const STREAM_PAMF: i32 = 0;
pub const STREAM_MP4: i32 = 1;
pub const STREAM_AVI: i32 = 2;
pub const STREAM_UNSPECIFIED: i32 = -1;

#[must_use]
pub fn is_known_stream_type(stream: i32) -> bool {
    matches!(stream, STREAM_PAMF | STREAM_MP4 | STREAM_AVI | STREAM_UNSPECIFIED)
}

// =====================================================================
// Preset types (cellSail.h:63-72)
// =====================================================================

pub const PRESET_AV_SYNC: i32 = 0; // deprecated alias for 59_94HZ
pub const PRESET_AS_IS: i32 = 1;
pub const PRESET_AV_SYNC_59_94HZ: i32 = 2;
pub const PRESET_AV_SYNC_29_97HZ: i32 = 3;
pub const PRESET_AV_SYNC_50HZ: i32 = 4;
pub const PRESET_AV_SYNC_25HZ: i32 = 5;
pub const PRESET_AV_SYNC_AUTO_DETECT: i32 = 6;

#[must_use]
pub fn is_known_preset(preset: i32) -> bool {
    (PRESET_AV_SYNC..=PRESET_AV_SYNC_AUTO_DETECT).contains(&preset)
}

// =====================================================================
// Event types (cellSail.h:75-91)
// =====================================================================

pub const EVENT_EMPTY: i32 = 0;
pub const EVENT_ERROR_OCCURRED: i32 = 1;
pub const EVENT_PLAYER_CALL_COMPLETED: i32 = 2;
pub const EVENT_PLAYER_STATE_CHANGED: i32 = 3;
pub const EVENT_STREAM_OPENED: i32 = 4;
pub const EVENT_STREAM_CLOSED: i32 = 5;
pub const EVENT_SESSION_STARTED: i32 = 6;
pub const EVENT_PAUSE_STATE_CHANGED: i32 = 7;
pub const EVENT_SOURCE_EOS: i32 = 8;
pub const EVENT_ES_OPENED: i32 = 9;
pub const EVENT_ES_CLOSED: i32 = 10;
pub const EVENT_MEDIA_STATE_CHANGED: i32 = 11;

// =====================================================================
// Player call types (cellSail.h:24-44)
// =====================================================================

pub const CALL_NONE: i32 = 0;
pub const CALL_BOOT: i32 = 1;
pub const CALL_OPEN_STREAM: i32 = 2;
pub const CALL_CLOSE_STREAM: i32 = 3;
pub const CALL_OPEN_ES_AUDIO: i32 = 4;
pub const CALL_OPEN_ES_VIDEO: i32 = 5;
pub const CALL_OPEN_ES_USER: i32 = 6;
pub const CALL_CLOSE_ES_AUDIO: i32 = 7;
pub const CALL_CLOSE_ES_VIDEO: i32 = 8;
pub const CALL_CLOSE_ES_USER: i32 = 9;
pub const CALL_START: i32 = 10;
pub const CALL_STOP: i32 = 11;
pub const CALL_NEXT: i32 = 12;
pub const CALL_REOPEN_ES_AUDIO: i32 = 13;
pub const CALL_REOPEN_ES_VIDEO: i32 = 14;
pub const CALL_REOPEN_ES_USER: i32 = 15;

// =====================================================================
// Media states (cellSail.h:165-170)
// =====================================================================

pub const MEDIA_STATE_FINE: i32 = 0;
pub const MEDIA_STATE_BAD: i32 = 1;
pub const MEDIA_STATE_LOST: i32 = 2;

// =====================================================================
// Sync modes (cellSail.h:183-187)
// =====================================================================

pub const SYNC_MODE_REPEAT: u32 = 1 << 0;
pub const SYNC_MODE_SKIP: u32 = 1 << 1;

// =====================================================================
// Parameter types (cellSail.h:94-162) — covered subset
// =====================================================================

pub const PARAM_ENABLE_VPOST: i32 = 0;
pub const PARAM_CONTROL_QUEUE_DEPTH: i32 = 1;
pub const PARAM_CONTROL_PPU_THREAD_PRIORITY: i32 = 2;
pub const PARAM_SPURS_NUM_OF_SPUS: i32 = 3;
pub const PARAM_SPURS_SPU_THREAD_PRIORITY: i32 = 4;
pub const PARAM_SPURS_PPU_THREAD_PRIORITY: i32 = 5;
pub const PARAM_SPURS_EXIT_IF_NO_WORK: i32 = 6;
pub const PARAM_IO_PPU_THREAD_PRIORITY: i32 = 7;
pub const PARAM_DMUX_PPU_THREAD_PRIORITY: i32 = 8;
pub const PARAM_ADEC_PPU_THREAD_PRIORITY: i32 = 12;
pub const PARAM_VDEC_PPU_THREAD_PRIORITY: i32 = 16;
pub const PARAM_VPOST_NUM_OF_SPUS: i32 = 23;
pub const PARAM_CONTROL_PPU_THREAD_STACK_SIZE: i32 = 29;
pub const PARAM_VIDEO_PERFORMANCE_POLICY: i32 = 35;
pub const PARAM_COUNT: i32 = 36;

// =====================================================================
// Descriptor — opaque container for a media URI + stream metadata
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub stream_type: i32,
    pub uri: String,
    pub auto_selection: bool,
    pub open: bool,
    pub media_info: Vec<u8>,
}

impl Descriptor {
    #[must_use]
    pub fn new(stream_type: i32, uri: impl Into<String>) -> Self {
        Self { stream_type, uri: uri.into(), auto_selection: true, open: false, media_info: Vec::new() }
    }
}

// =====================================================================
// ElementaryStream tracking — audio/video/user
// =====================================================================

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EsKind {
    Audio,
    Video,
    User,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementaryStream {
    pub kind: EsKind,
    pub index: u32,
    pub muted: bool,
    pub paused: bool,
}

// =====================================================================
// Player
// =====================================================================

#[derive(Clone, Debug)]
pub struct Player {
    state: PlayerState,
    user_param: u64,
    preset: i32,
    paused: bool,
    media_state: i32,
    descriptors: Vec<Descriptor>,
    elementary_streams: Vec<ElementaryStream>,
    parameters: [u64; PARAM_COUNT as usize],
    subscribed_events: Vec<i32>,
    current_descriptor: Option<usize>,
}

impl Player {
    /// Initial state after `cellSailPlayerInitialize`. The C++ impl fills a
    /// CellSailPlayer struct on first call but keeps the state as
    /// `INITIALIZED` until `boot` transitions it.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: PlayerState::Initialized,
            user_param: 0,
            preset: PRESET_AS_IS,
            paused: false,
            media_state: MEDIA_STATE_FINE,
            descriptors: Vec::new(),
            elementary_streams: Vec::new(),
            parameters: [0; PARAM_COUNT as usize],
            subscribed_events: Vec::new(),
            current_descriptor: None,
        }
    }

    #[must_use]
    pub fn state(&self) -> PlayerState {
        self.state
    }

    #[must_use]
    pub fn preset(&self) -> i32 {
        self.preset
    }

    #[must_use]
    pub fn paused(&self) -> bool {
        self.paused
    }

    #[must_use]
    pub fn media_state(&self) -> i32 {
        self.media_state
    }

    #[must_use]
    pub fn user_param(&self) -> u64 {
        self.user_param
    }

    // ----------------- Lifecycle -----------------

    /// `cellSailPlayerFinalize`: must be in a non-running state. Here we
    /// mirror the real lib and reject from `Running`/`Opened` — the game
    /// must `stop`+`close_stream` first.
    pub fn finalize(&mut self) -> Result<(), CellError> {
        match self.state {
            PlayerState::Running
            | PlayerState::Opened
            | PlayerState::StartTransition
            | PlayerState::StopTransition
            | PlayerState::OpenTransition
            | PlayerState::CloseTransition
            | PlayerState::BootTransition => Err(errors::INVALID_STATE),
            _ => {
                self.state = PlayerState::Closed;
                self.descriptors.clear();
                self.elementary_streams.clear();
                self.subscribed_events.clear();
                self.current_descriptor = None;
                Ok(())
            }
        }
    }

    /// `cellSailPlayerBoot`: must be fresh (INITIALIZED) — transitions to
    /// CLOSED (ready for a stream to be opened).
    pub fn boot(&mut self, user_param: u64) -> Result<(), CellError> {
        if self.state != PlayerState::Initialized {
            return Err(errors::INVALID_STATE);
        }
        self.user_param = user_param;
        self.state = PlayerState::Closed;
        Ok(())
    }

    // ----------------- Descriptors -----------------

    pub fn create_descriptor(
        &mut self,
        stream_type: i32,
        uri: impl Into<String>,
    ) -> Result<usize, CellError> {
        if !is_known_stream_type(stream_type) {
            return Err(errors::UNSUPPORTED_STREAM);
        }
        self.descriptors.push(Descriptor::new(stream_type, uri));
        Ok(self.descriptors.len() - 1)
    }

    pub fn add_descriptor(&mut self, desc: Descriptor) -> Result<usize, CellError> {
        if !is_known_stream_type(desc.stream_type) {
            return Err(errors::UNSUPPORTED_STREAM);
        }
        self.descriptors.push(desc);
        Ok(self.descriptors.len() - 1)
    }

    pub fn remove_descriptor(&mut self, index: usize) -> Result<Descriptor, CellError> {
        if index >= self.descriptors.len() {
            return Err(errors::INDEX_OUT_OF_RANGE);
        }
        if self.current_descriptor == Some(index) {
            return Err(errors::USING);
        }
        // When removing, indices shift — update the current pointer
        // accordingly.
        let removed = self.descriptors.remove(index);
        if let Some(cur) = self.current_descriptor {
            if cur > index {
                self.current_descriptor = Some(cur - 1);
            }
        }
        Ok(removed)
    }

    #[must_use]
    pub fn descriptor_count(&self) -> usize {
        self.descriptors.len()
    }

    pub fn get_descriptor(&self, index: usize) -> Result<&Descriptor, CellError> {
        self.descriptors.get(index).ok_or(errors::INDEX_OUT_OF_RANGE)
    }

    // ----------------- Stream open/close -----------------

    /// `cellSailPlayerOpenStream`: goes from CLOSED → OPEN_TRANSITION →
    /// OPENED after validating that a descriptor is selected.
    pub fn open_stream(&mut self, descriptor_index: usize) -> Result<(), CellError> {
        if self.state != PlayerState::Closed {
            return Err(errors::INVALID_STATE);
        }
        if descriptor_index >= self.descriptors.len() {
            return Err(errors::INDEX_OUT_OF_RANGE);
        }
        self.state = PlayerState::OpenTransition;
        self.descriptors[descriptor_index].open = true;
        self.current_descriptor = Some(descriptor_index);
        self.state = PlayerState::Opened;
        Ok(())
    }

    /// `cellSailPlayerCloseStream`: OPENED/STOPPED → CLOSE_TRANSITION → CLOSED.
    pub fn close_stream(&mut self) -> Result<(), CellError> {
        match self.state {
            PlayerState::Opened => {}
            PlayerState::Running => return Err(errors::INVALID_STATE),
            _ => return Err(errors::INVALID_STATE),
        }
        self.state = PlayerState::CloseTransition;
        if let Some(idx) = self.current_descriptor.take() {
            if let Some(d) = self.descriptors.get_mut(idx) {
                d.open = false;
            }
        }
        self.elementary_streams.clear();
        self.state = PlayerState::Closed;
        Ok(())
    }

    // ----------------- Run/pause control -----------------

    /// `cellSailPlayerStart`: OPENED → START_TRANSITION → RUNNING.
    pub fn start(&mut self) -> Result<(), CellError> {
        if self.state != PlayerState::Opened {
            return Err(errors::INVALID_STATE);
        }
        self.state = PlayerState::StartTransition;
        self.state = PlayerState::Running;
        Ok(())
    }

    /// `cellSailPlayerStop`: RUNNING → STOP_TRANSITION → OPENED.
    pub fn stop(&mut self) -> Result<(), CellError> {
        if self.state != PlayerState::Running {
            return Err(errors::INVALID_STATE);
        }
        self.state = PlayerState::StopTransition;
        self.state = PlayerState::Opened;
        Ok(())
    }

    /// `cellSailPlayerNext`: RUNNING, moves to the next descriptor if there
    /// is one. The C++ lib moves to a CLOSED state between descriptors; we
    /// model it as Running → Opened.
    pub fn next(&mut self) -> Result<Option<usize>, CellError> {
        if self.state != PlayerState::Running {
            return Err(errors::INVALID_STATE);
        }
        let Some(cur) = self.current_descriptor else {
            return Err(errors::INVALID_STATE);
        };
        let next = cur + 1;
        if next >= self.descriptors.len() {
            // EOS: stays in current descriptor, returns None.
            return Ok(None);
        }
        self.current_descriptor = Some(next);
        self.elementary_streams.clear();
        Ok(Some(next))
    }

    pub fn set_paused(&mut self, paused: bool) -> Result<(), CellError> {
        // C++ allows set_paused only in RUNNING.
        if self.state != PlayerState::Running {
            return Err(errors::INVALID_STATE);
        }
        self.paused = paused;
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), CellError> {
        // Cancel from any transitional state; mirrors `cellSailPlayerCancel`.
        if matches!(self.state, PlayerState::Initialized | PlayerState::Lost) {
            return Err(errors::INVALID_STATE);
        }
        Ok(())
    }

    // ----------------- Parameters / presets -----------------

    pub fn set_parameter(&mut self, param: i32, value0: u64, _value1: u64) -> Result<(), CellError> {
        if !(0..PARAM_COUNT).contains(&param) {
            return Err(errors::INVALID_ARG);
        }
        self.parameters[param as usize] = value0;
        Ok(())
    }

    pub fn get_parameter(&self, param: i32) -> Result<(u64, u64), CellError> {
        if !(0..PARAM_COUNT).contains(&param) {
            return Err(errors::INVALID_ARG);
        }
        Ok((self.parameters[param as usize], 0))
    }

    pub fn set_preset(&mut self, preset: i32) -> Result<(), CellError> {
        if !is_known_preset(preset) {
            return Err(errors::INVALID_ARG);
        }
        self.preset = preset;
        Ok(())
    }

    // ----------------- Elementary streams -----------------

    pub fn open_es(&mut self, kind: EsKind, index: u32) -> Result<(), CellError> {
        if self.state != PlayerState::Opened && self.state != PlayerState::Running {
            return Err(errors::INVALID_STATE);
        }
        if self.elementary_streams.iter().any(|es| es.kind == kind && es.index == index) {
            return Err(errors::USING);
        }
        self.elementary_streams.push(ElementaryStream { kind, index, muted: false, paused: false });
        Ok(())
    }

    pub fn close_es(&mut self, kind: EsKind, index: u32) -> Result<(), CellError> {
        let pos =
            self.elementary_streams.iter().position(|es| es.kind == kind && es.index == index).ok_or(errors::NOT_AVAILABLE)?;
        self.elementary_streams.remove(pos);
        Ok(())
    }

    #[must_use]
    pub fn es_count(&self, kind: EsKind) -> usize {
        self.elementary_streams.iter().filter(|es| es.kind == kind).count()
    }

    pub fn set_es_muted(&mut self, kind: EsKind, index: u32, muted: bool) -> Result<(), CellError> {
        let es = self
            .elementary_streams
            .iter_mut()
            .find(|es| es.kind == kind && es.index == index)
            .ok_or(errors::NOT_AVAILABLE)?;
        es.muted = muted;
        Ok(())
    }

    // ----------------- Events -----------------

    pub fn subscribe_event(&mut self, event: i32) -> Result<(), CellError> {
        if !(0..=EVENT_MEDIA_STATE_CHANGED).contains(&event) {
            return Err(errors::INVALID_ARG);
        }
        if self.subscribed_events.contains(&event) {
            return Err(errors::USING);
        }
        self.subscribed_events.push(event);
        Ok(())
    }

    pub fn unsubscribe_event(&mut self, event: i32) -> Result<(), CellError> {
        let pos = self.subscribed_events.iter().position(|&e| e == event).ok_or(errors::NOT_AVAILABLE)?;
        self.subscribed_events.remove(pos);
        Ok(())
    }

    #[must_use]
    pub fn is_event_subscribed(&self, event: i32) -> bool {
        self.subscribed_events.contains(&event)
    }

    pub fn mark_media_state(&mut self, state: i32) -> Result<(), CellError> {
        if !(MEDIA_STATE_FINE..=MEDIA_STATE_LOST).contains(&state) {
            return Err(errors::INVALID_ARG);
        }
        self.media_state = state;
        if state == MEDIA_STATE_LOST {
            self.state = PlayerState::Lost;
        }
        Ok(())
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn booted_player() -> Player {
        let mut p = Player::new();
        p.boot(0xABCD).unwrap();
        let idx = p.create_descriptor(STREAM_PAMF, "dev_bdvd:/movie.pamf").unwrap();
        p.open_stream(idx).unwrap();
        p
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::INVALID_ARG.0, 0x8061_0701);
        assert_eq!(errors::INVALID_STATE.0, 0x8061_0702);
        assert_eq!(errors::CANCEL.0, 0x8061_0709);
        assert_eq!(errors::MEMORY.0, 0x8061_07F0);
        assert_eq!(errors::INVALID_FD.0, 0x8061_07F1);
        assert_eq!(errors::FATAL.0, 0x8061_07FF);
    }

    #[test]
    fn player_state_enum_ordinals_match_c() {
        assert_eq!(PlayerState::Initialized as i32, 0);
        assert_eq!(PlayerState::Closed as i32, 2);
        assert_eq!(PlayerState::Opened as i32, 4);
        assert_eq!(PlayerState::Running as i32, 6);
        assert_eq!(PlayerState::Lost as i32, 9);
    }

    #[test]
    fn stream_type_constants_stable() {
        assert_eq!(STREAM_PAMF, 0);
        assert_eq!(STREAM_MP4, 1);
        assert_eq!(STREAM_AVI, 2);
        assert_eq!(STREAM_UNSPECIFIED, -1);
    }

    #[test]
    fn is_known_stream_type_accepts_valid_rejects_others() {
        assert!(is_known_stream_type(STREAM_PAMF));
        assert!(is_known_stream_type(STREAM_MP4));
        assert!(is_known_stream_type(STREAM_AVI));
        assert!(is_known_stream_type(STREAM_UNSPECIFIED));
        assert!(!is_known_stream_type(42));
        assert!(!is_known_stream_type(-2));
    }

    #[test]
    fn preset_constants_stable() {
        assert_eq!(PRESET_AV_SYNC, 0);
        assert_eq!(PRESET_AS_IS, 1);
        assert_eq!(PRESET_AV_SYNC_59_94HZ, 2);
        assert_eq!(PRESET_AV_SYNC_AUTO_DETECT, 6);
    }

    #[test]
    fn event_constants_stable() {
        assert_eq!(EVENT_ERROR_OCCURRED, 1);
        assert_eq!(EVENT_STREAM_OPENED, 4);
        assert_eq!(EVENT_SESSION_STARTED, 6);
        assert_eq!(EVENT_ES_OPENED, 9);
        assert_eq!(EVENT_MEDIA_STATE_CHANGED, 11);
    }

    #[test]
    fn fresh_player_starts_initialized() {
        let p = Player::new();
        assert_eq!(p.state(), PlayerState::Initialized);
        assert_eq!(p.descriptor_count(), 0);
        assert_eq!(p.media_state(), MEDIA_STATE_FINE);
    }

    #[test]
    fn boot_transitions_to_closed() {
        let mut p = Player::new();
        p.boot(0xDEADBEEF).unwrap();
        assert_eq!(p.state(), PlayerState::Closed);
        assert_eq!(p.user_param(), 0xDEADBEEF);
    }

    #[test]
    fn boot_twice_is_invalid_state() {
        let mut p = Player::new();
        p.boot(0).unwrap();
        assert_eq!(p.boot(0), Err(errors::INVALID_STATE));
    }

    #[test]
    fn create_descriptor_unsupported_stream_rejected() {
        let mut p = Player::new();
        p.boot(0).unwrap();
        assert_eq!(p.create_descriptor(42, "x"), Err(errors::UNSUPPORTED_STREAM));
    }

    #[test]
    fn remove_descriptor_bad_index_rejected() {
        let mut p = Player::new();
        p.boot(0).unwrap();
        assert_eq!(p.remove_descriptor(0), Err(errors::INDEX_OUT_OF_RANGE));
        p.create_descriptor(STREAM_PAMF, "a").unwrap();
        assert_eq!(p.remove_descriptor(5), Err(errors::INDEX_OUT_OF_RANGE));
    }

    #[test]
    fn remove_current_descriptor_is_using() {
        let mut p = booted_player();
        assert_eq!(p.remove_descriptor(0), Err(errors::USING));
    }

    #[test]
    fn open_stream_requires_closed_state() {
        let mut p = Player::new();
        // Fresh player is INITIALIZED, not CLOSED.
        assert_eq!(p.open_stream(0), Err(errors::INVALID_STATE));
    }

    #[test]
    fn open_stream_bad_index_rejected() {
        let mut p = Player::new();
        p.boot(0).unwrap();
        assert_eq!(p.open_stream(99), Err(errors::INDEX_OUT_OF_RANGE));
    }

    #[test]
    fn open_stream_happy_path() {
        let p = booted_player();
        assert_eq!(p.state(), PlayerState::Opened);
    }

    #[test]
    fn close_stream_round_trip() {
        let mut p = booted_player();
        p.close_stream().unwrap();
        assert_eq!(p.state(), PlayerState::Closed);
        assert_eq!(p.descriptor_count(), 1);
        // Descriptor still present but no longer "open".
        assert!(!p.get_descriptor(0).unwrap().open);
    }

    #[test]
    fn close_stream_while_running_is_invalid_state() {
        let mut p = booted_player();
        p.start().unwrap();
        assert_eq!(p.close_stream(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn start_stop_round_trip() {
        let mut p = booted_player();
        p.start().unwrap();
        assert_eq!(p.state(), PlayerState::Running);
        p.stop().unwrap();
        assert_eq!(p.state(), PlayerState::Opened);
    }

    #[test]
    fn start_from_non_opened_is_invalid_state() {
        let mut p = Player::new();
        assert_eq!(p.start(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn stop_from_non_running_is_invalid_state() {
        let mut p = Player::new();
        assert_eq!(p.stop(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn set_paused_only_runs_when_running() {
        let mut p = Player::new();
        assert_eq!(p.set_paused(true), Err(errors::INVALID_STATE));
        let mut p = booted_player();
        p.start().unwrap();
        p.set_paused(true).unwrap();
        assert!(p.paused());
        p.set_paused(false).unwrap();
        assert!(!p.paused());
    }

    #[test]
    fn next_advances_to_second_descriptor() {
        let mut p = Player::new();
        p.boot(0).unwrap();
        p.create_descriptor(STREAM_PAMF, "a.pamf").unwrap();
        let _ = p.create_descriptor(STREAM_MP4, "b.mp4").unwrap();
        p.open_stream(0).unwrap();
        p.start().unwrap();
        assert_eq!(p.next(), Ok(Some(1)));
    }

    #[test]
    fn next_at_eos_returns_none() {
        let mut p = booted_player();
        p.start().unwrap();
        assert_eq!(p.next(), Ok(None));
    }

    #[test]
    fn next_when_not_running_is_invalid_state() {
        let mut p = booted_player();
        // OPENED, not RUNNING
        assert_eq!(p.next(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn set_and_get_parameter_round_trip() {
        let mut p = Player::new();
        p.set_parameter(PARAM_SPURS_NUM_OF_SPUS, 3, 0).unwrap();
        assert_eq!(p.get_parameter(PARAM_SPURS_NUM_OF_SPUS), Ok((3, 0)));
    }

    #[test]
    fn set_parameter_bad_index_rejected() {
        let mut p = Player::new();
        assert_eq!(p.set_parameter(999, 0, 0), Err(errors::INVALID_ARG));
        assert_eq!(p.set_parameter(-1, 0, 0), Err(errors::INVALID_ARG));
    }

    #[test]
    fn set_preset_accepts_known_rejects_others() {
        let mut p = Player::new();
        for preset in 0..=PRESET_AV_SYNC_AUTO_DETECT {
            p.set_preset(preset).unwrap();
            assert_eq!(p.preset(), preset);
        }
        assert_eq!(p.set_preset(99), Err(errors::INVALID_ARG));
    }

    #[test]
    fn subscribe_and_unsubscribe_events() {
        let mut p = Player::new();
        p.subscribe_event(EVENT_STREAM_OPENED).unwrap();
        assert!(p.is_event_subscribed(EVENT_STREAM_OPENED));
        assert_eq!(p.subscribe_event(EVENT_STREAM_OPENED), Err(errors::USING));
        p.unsubscribe_event(EVENT_STREAM_OPENED).unwrap();
        assert!(!p.is_event_subscribed(EVENT_STREAM_OPENED));
        assert_eq!(p.unsubscribe_event(EVENT_STREAM_OPENED), Err(errors::NOT_AVAILABLE));
    }

    #[test]
    fn subscribe_bad_event_rejected() {
        let mut p = Player::new();
        assert_eq!(p.subscribe_event(999), Err(errors::INVALID_ARG));
        assert_eq!(p.subscribe_event(-1), Err(errors::INVALID_ARG));
    }

    #[test]
    fn open_and_close_elementary_stream() {
        let mut p = booted_player();
        p.open_es(EsKind::Audio, 0).unwrap();
        p.open_es(EsKind::Video, 0).unwrap();
        assert_eq!(p.es_count(EsKind::Audio), 1);
        assert_eq!(p.es_count(EsKind::Video), 1);
        p.close_es(EsKind::Audio, 0).unwrap();
        assert_eq!(p.es_count(EsKind::Audio), 0);
        assert_eq!(p.close_es(EsKind::Audio, 0), Err(errors::NOT_AVAILABLE));
    }

    #[test]
    fn open_duplicate_es_is_using() {
        let mut p = booted_player();
        p.open_es(EsKind::Audio, 0).unwrap();
        assert_eq!(p.open_es(EsKind::Audio, 0), Err(errors::USING));
    }

    #[test]
    fn open_es_requires_stream_open() {
        let mut p = Player::new();
        p.boot(0).unwrap();
        assert_eq!(p.open_es(EsKind::Audio, 0), Err(errors::INVALID_STATE));
    }

    #[test]
    fn set_es_muted_happy_path() {
        let mut p = booted_player();
        p.open_es(EsKind::Audio, 0).unwrap();
        p.set_es_muted(EsKind::Audio, 0, true).unwrap();
        assert_eq!(p.elementary_streams[0].muted, true);
    }

    #[test]
    fn mark_media_state_lost_transitions_to_lost() {
        let mut p = booted_player();
        p.mark_media_state(MEDIA_STATE_LOST).unwrap();
        assert_eq!(p.state(), PlayerState::Lost);
        assert_eq!(p.media_state(), MEDIA_STATE_LOST);
    }

    #[test]
    fn mark_media_state_bad_is_bad() {
        let mut p = booted_player();
        p.mark_media_state(MEDIA_STATE_BAD).unwrap();
        assert_eq!(p.media_state(), MEDIA_STATE_BAD);
        // Not LOST, so state stays OPENED.
        assert_eq!(p.state(), PlayerState::Opened);
    }

    #[test]
    fn finalize_from_running_is_invalid_state() {
        let mut p = booted_player();
        p.start().unwrap();
        assert_eq!(p.finalize(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn finalize_from_initialized_ok() {
        let mut p = Player::new();
        p.finalize().unwrap();
        assert_eq!(p.state(), PlayerState::Closed);
    }

    #[test]
    fn full_playback_pipeline_smoke() {
        let mut p = Player::new();
        p.boot(0).unwrap();
        let d1 = p.create_descriptor(STREAM_PAMF, "track1").unwrap();
        let _d2 = p.create_descriptor(STREAM_PAMF, "track2").unwrap();
        p.open_stream(d1).unwrap();
        p.open_es(EsKind::Audio, 0).unwrap();
        p.open_es(EsKind::Video, 0).unwrap();
        p.start().unwrap();
        p.set_paused(true).unwrap();
        p.set_paused(false).unwrap();
        assert_eq!(p.next(), Ok(Some(1)));
        // After next(), ES list is cleared for the new descriptor.
        assert_eq!(p.es_count(EsKind::Audio), 0);
        p.stop().unwrap();
        p.close_stream().unwrap();
        assert_eq!(p.state(), PlayerState::Closed);
    }

    #[test]
    fn cancel_in_lost_is_invalid_state() {
        let mut p = booted_player();
        p.mark_media_state(MEDIA_STATE_LOST).unwrap();
        assert_eq!(p.cancel(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn cancel_in_open_state_is_ok() {
        let mut p = booted_player();
        assert!(p.cancel().is_ok());
    }

    #[test]
    fn descriptor_remove_shifts_current_correctly() {
        let mut p = Player::new();
        p.boot(0).unwrap();
        let _a = p.create_descriptor(STREAM_PAMF, "a").unwrap();
        let b = p.create_descriptor(STREAM_MP4, "b").unwrap();
        p.open_stream(b).unwrap();
        // Removing descriptor 0 (not current): current was 1, should shift to 0.
        p.remove_descriptor(0).unwrap();
        assert_eq!(p.current_descriptor, Some(0));
        // The surviving descriptor is the MP4 one.
        assert_eq!(p.get_descriptor(0).unwrap().stream_type, STREAM_MP4);
    }

    #[test]
    fn call_types_stable() {
        assert_eq!(CALL_BOOT, 1);
        assert_eq!(CALL_OPEN_STREAM, 2);
        assert_eq!(CALL_START, 10);
        assert_eq!(CALL_NEXT, 12);
        assert_eq!(CALL_REOPEN_ES_USER, 15);
    }
}
