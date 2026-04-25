//! `rpcs3-hle-cellmusic` — background music playback HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellMusic.cpp`. cellMusic is the
//! API games use to play the user's XMB music library under their own UI
//! (without losing focus). The player is singleton: `Init → SelectContents →
//! SetPlaybackCommand(PLAY) → ... → SetPlaybackCommand(STOP) → Finalize`.
//!
//! ## Entry points covered
//!
//! | HLE function                          | Rust wrapper                           |
//! |---------------------------------------|----------------------------------------|
//! | `cellMusicInitialize`                 | [`MusicPlayer::initialize`]            |
//! | `cellMusicFinalize`                   | [`MusicPlayer::finalize`]              |
//! | `cellMusicSelectContents`             | [`MusicPlayer::select_contents`]       |
//! | `cellMusicSetSelectionContext`        | [`MusicPlayer::set_selection_context`] |
//! | `cellMusicGetSelectionContext`        | [`MusicPlayer::get_selection_context`] |
//! | `cellMusicSetPlaybackCommand`         | [`MusicPlayer::set_playback_command`]  |
//! | `cellMusicGetPlaybackStatus`          | [`MusicPlayer::playback_status`]       |
//! | `cellMusicGetVolume` / `SetVolume`    | [`MusicPlayer::volume`] / `set_volume` |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellMusic.h:8-22 (Music v1 + Music2 share
// the same numeric space)
// =====================================================================

pub const CANCELED: i32 = 1;

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const PLAYBACK_FINISHED: CellError = CellError(0x8002_c101);
    pub const PARAM: CellError = CellError(0x8002_c102);
    pub const BUSY: CellError = CellError(0x8002_c103);
    pub const NO_ACTIVE_CONTENT: CellError = CellError(0x8002_c104);
    pub const NO_MATCH_FOUND: CellError = CellError(0x8002_c105);
    pub const INVALID_CONTEXT: CellError = CellError(0x8002_c106);
    pub const PLAYBACK_FAILURE: CellError = CellError(0x8002_c107);
    pub const NO_MORE_CONTENT: CellError = CellError(0x8002_c108);
    pub const DIALOG_OPEN: CellError = CellError(0x8002_c109);
    pub const DIALOG_CLOSE: CellError = CellError(0x8002_c10a);
    pub const GENERIC: CellError = CellError(0x8002_c1ff);
}

// =====================================================================
// Sysutil lifecycle events (cellMusic.h:41-48)
// =====================================================================

pub const SYSUTIL_INITIALIZING_FINISHED: u32 = 1;
pub const SYSUTIL_SHUTDOWN_FINISHED: u32 = 4;
pub const SYSUTIL_LOADING_FINISHED: u32 = 5;
pub const SYSUTIL_UNLOADING_FINISHED: u32 = 7;
pub const SYSUTIL_RELEASED: u32 = 9;
pub const SYSUTIL_GRABBED: u32 = 11;

// =====================================================================
// Event types (cellMusic.h:61-70)
// =====================================================================

pub const EVENT_STATUS_NOTIFICATION: u32 = 0;
pub const EVENT_INITIALIZE_RESULT: u32 = 1;
pub const EVENT_FINALIZE_RESULT: u32 = 2;
pub const EVENT_SELECT_CONTENTS_RESULT: u32 = 3;
pub const EVENT_SET_PLAYBACK_COMMAND_RESULT: u32 = 4;
pub const EVENT_SET_VOLUME_RESULT: u32 = 5;
pub const EVENT_SET_SELECTION_CONTEXT_RESULT: u32 = 6;
pub const EVENT_UI_NOTIFICATION: u32 = 7;

// =====================================================================
// Playback commands (cellMusic.h:85-93)
// =====================================================================

pub const PB_CMD_STOP: i32 = 0;
pub const PB_CMD_PLAY: i32 = 1;
pub const PB_CMD_PAUSE: i32 = 2;
pub const PB_CMD_NEXT: i32 = 3;
pub const PB_CMD_PREV: i32 = 4;
pub const PB_CMD_FASTFORWARD: i32 = 5;
pub const PB_CMD_FASTREVERSE: i32 = 6;

#[must_use]
pub fn is_known_command(cmd: i32) -> bool {
    (PB_CMD_STOP..=PB_CMD_FASTREVERSE).contains(&cmd)
}

// =====================================================================
// Playback status (cellMusic.h:107-113)
// =====================================================================

pub const PB_STATUS_STOP: i32 = 0;
pub const PB_STATUS_PLAY: i32 = 1;
pub const PB_STATUS_PAUSE: i32 = 2;
pub const PB_STATUS_FASTFORWARD: i32 = 3;
pub const PB_STATUS_FASTREVERSE: i32 = 4;

// =====================================================================
// Constants (cellMusic.h:124-130)
// =====================================================================

pub const PLAYBACK_MEMORY_CONTAINER_SIZE: u32 = 11 * 1024 * 1024;
pub const PLAYER_MODE_NORMAL: i32 = 0;
pub const SELECTION_CONTEXT_SIZE: usize = 2048;

// Repeat modes mirror cellSearch (pulled in via cellSearch.h include).
pub const REPEATMODE_NONE: i32 = 0;
pub const REPEATMODE_REPEAT1: i32 = 1;
pub const REPEATMODE_ALL: i32 = 2;
pub const REPEATMODE_NOREPEAT1: i32 = 3;

pub const CONTEXTOPTION_NONE: i32 = 0;
pub const CONTEXTOPTION_SHUFFLE: i32 = 1;

// Selection context magic (header of the serialized blob returned to games).
pub const CONTEXT_MAGIC: &[u8; 4] = b"SUS\0";

// =====================================================================
// SelectionContext — shape of the serialized byte array exported to games
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionContext {
    pub hash: String,
    pub repeat_mode: i32,
    pub context_option: i32,
    pub first_track: u32,
    pub current_track: u32,
    pub playlist: Vec<String>,
}

impl SelectionContext {
    #[must_use]
    pub fn new(hash: impl Into<String>, playlist: Vec<String>) -> Self {
        Self {
            hash: hash.into(),
            repeat_mode: REPEATMODE_NONE,
            context_option: CONTEXTOPTION_NONE,
            first_track: 0,
            current_track: 0,
            playlist,
        }
    }

    /// Serialize to a `SELECTION_CONTEXT_SIZE`-byte blob (`music_selection_context::get`
    /// in C++): first 4 bytes magic, then hash as hex + delimiter + track index.
    #[must_use]
    pub fn serialize(&self) -> [u8; SELECTION_CONTEXT_SIZE] {
        let mut buf = [0u8; SELECTION_CONTEXT_SIZE];
        buf[..4].copy_from_slice(CONTEXT_MAGIC);
        let hash_bytes = self.hash.as_bytes();
        let hash_len = hash_bytes.len().min(SELECTION_CONTEXT_SIZE - 4 - 4);
        buf[4..4 + hash_len].copy_from_slice(&hash_bytes[..hash_len]);
        // Last 4 bytes: current_track BE.
        let pos = SELECTION_CONTEXT_SIZE - 4;
        buf[pos..].copy_from_slice(&self.current_track.to_be_bytes());
        buf
    }

    /// Deserialize a game-provided context; mirrors
    /// `music_selection_context::set`. Returns INVALID_CONTEXT on a bad
    /// magic header.
    pub fn deserialize(buf: &[u8; SELECTION_CONTEXT_SIZE]) -> Result<Self, CellError> {
        if &buf[..4] != CONTEXT_MAGIC {
            return Err(errors::INVALID_CONTEXT);
        }
        let hash_end = buf[4..].iter().position(|&b| b == 0).map_or(0, |i| 4 + i);
        let hash = core::str::from_utf8(&buf[4..hash_end]).unwrap_or("").to_string();
        let pos = SELECTION_CONTEXT_SIZE - 4;
        let current_track = u32::from_be_bytes(buf[pos..].try_into().unwrap_or([0; 4]));
        Ok(Self {
            hash,
            repeat_mode: REPEATMODE_NONE,
            context_option: CONTEXTOPTION_NONE,
            first_track: 0,
            current_track,
            playlist: Vec::new(),
        })
    }
}

// =====================================================================
// MusicPlayer — singleton state
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerState {
    Uninitialized,
    Initialized,
    ContentsSelected,
}

#[derive(Clone, Debug)]
pub struct MusicPlayer {
    state: PlayerState,
    context: Option<SelectionContext>,
    pb_status: i32,
    volume: f32, // 0.0 .. 1.0
    mode: i32,
    dialog_open: bool,
}

impl MusicPlayer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: PlayerState::Uninitialized,
            context: None,
            pb_status: PB_STATUS_STOP,
            volume: 1.0,
            mode: PLAYER_MODE_NORMAL,
            dialog_open: false,
        }
    }

    #[must_use]
    pub fn state(&self) -> PlayerState {
        self.state
    }

    #[must_use]
    pub fn playback_status(&self) -> i32 {
        self.pb_status
    }

    #[must_use]
    pub fn volume(&self) -> f32 {
        self.volume
    }

    // ----------------- Lifecycle -----------------

    pub fn initialize(&mut self, mode: i32) -> Result<(), CellError> {
        if mode != PLAYER_MODE_NORMAL {
            return Err(errors::PARAM);
        }
        if self.state != PlayerState::Uninitialized {
            return Err(errors::BUSY);
        }
        self.mode = mode;
        self.state = PlayerState::Initialized;
        self.pb_status = PB_STATUS_STOP;
        self.context = None;
        self.dialog_open = false;
        Ok(())
    }

    pub fn finalize(&mut self) -> Result<(), CellError> {
        if self.state == PlayerState::Uninitialized {
            return Err(errors::GENERIC);
        }
        self.state = PlayerState::Uninitialized;
        self.pb_status = PB_STATUS_STOP;
        self.context = None;
        self.dialog_open = false;
        Ok(())
    }

    // ----------------- Content selection -----------------

    /// `cellMusicSelectContents(callback, userData)` opens the XMB music
    /// picker. The real lib posts a SELECT_CONTENTS_RESULT event; the
    /// test backend pre-populates a playlist via `inject_context`.
    pub fn select_contents(&mut self) -> Result<(), CellError> {
        self.require_initialized()?;
        if self.dialog_open {
            return Err(errors::DIALOG_OPEN);
        }
        self.dialog_open = true;
        Ok(())
    }

    /// Fire the post-selection callback result. Test hook mirroring the
    /// async callback injection path.
    pub fn complete_selection(&mut self, context: SelectionContext) -> Result<(), CellError> {
        if !self.dialog_open {
            return Err(errors::DIALOG_CLOSE);
        }
        self.context = Some(context);
        self.state = PlayerState::ContentsSelected;
        self.dialog_open = false;
        Ok(())
    }

    pub fn cancel_selection(&mut self) -> Result<(), CellError> {
        if !self.dialog_open {
            return Err(errors::DIALOG_CLOSE);
        }
        self.dialog_open = false;
        Ok(())
    }

    pub fn set_selection_context(&mut self, blob: &[u8; SELECTION_CONTEXT_SIZE]) -> Result<(), CellError> {
        self.require_initialized()?;
        let ctx = SelectionContext::deserialize(blob)?;
        self.context = Some(ctx);
        self.state = PlayerState::ContentsSelected;
        Ok(())
    }

    pub fn get_selection_context(&self) -> Result<[u8; SELECTION_CONTEXT_SIZE], CellError> {
        self.require_initialized()?;
        let ctx = self.context.as_ref().ok_or(errors::NO_ACTIVE_CONTENT)?;
        Ok(ctx.serialize())
    }

    // ----------------- Playback control -----------------

    pub fn set_playback_command(&mut self, cmd: i32) -> Result<(), CellError> {
        self.require_initialized()?;
        if !is_known_command(cmd) {
            return Err(errors::PARAM);
        }
        if self.state != PlayerState::ContentsSelected && cmd != PB_CMD_STOP {
            return Err(errors::NO_ACTIVE_CONTENT);
        }
        let new_status = match cmd {
            PB_CMD_STOP => PB_STATUS_STOP,
            PB_CMD_PLAY => PB_STATUS_PLAY,
            PB_CMD_PAUSE => PB_STATUS_PAUSE,
            PB_CMD_FASTFORWARD => PB_STATUS_FASTFORWARD,
            PB_CMD_FASTREVERSE => PB_STATUS_FASTREVERSE,
            PB_CMD_NEXT | PB_CMD_PREV => {
                // Advance / retreat within the playlist; stay in PLAY.
                let ctx = self.context.as_mut().ok_or(errors::NO_ACTIVE_CONTENT)?;
                if ctx.playlist.is_empty() {
                    return Err(errors::NO_MORE_CONTENT);
                }
                if cmd == PB_CMD_NEXT {
                    let next = ctx.current_track + 1;
                    if (next as usize) >= ctx.playlist.len() {
                        return Err(errors::NO_MORE_CONTENT);
                    }
                    ctx.current_track = next;
                } else {
                    if ctx.current_track == 0 {
                        return Err(errors::NO_MORE_CONTENT);
                    }
                    ctx.current_track -= 1;
                }
                PB_STATUS_PLAY
            }
            _ => unreachable!(),
        };
        self.pb_status = new_status;
        Ok(())
    }

    // ----------------- Volume -----------------

    pub fn set_volume(&mut self, volume: f32) -> Result<(), CellError> {
        self.require_initialized()?;
        if !volume.is_finite() || !(0.0..=1.0).contains(&volume) {
            return Err(errors::PARAM);
        }
        self.volume = volume;
        Ok(())
    }

    // ----------------- Helpers -----------------

    fn require_initialized(&self) -> Result<(), CellError> {
        if self.state == PlayerState::Uninitialized {
            Err(errors::GENERIC)
        } else {
            Ok(())
        }
    }
}

impl Default for MusicPlayer {
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

    fn ready_player_with_playlist() -> MusicPlayer {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        p.select_contents().unwrap();
        p.complete_selection(SelectionContext::new(
            "hash1",
            vec!["song1.mp3".into(), "song2.mp3".into(), "song3.mp3".into()],
        ))
        .unwrap();
        p
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::PLAYBACK_FINISHED.0, 0x8002_c101);
        assert_eq!(errors::PARAM.0, 0x8002_c102);
        assert_eq!(errors::BUSY.0, 0x8002_c103);
        assert_eq!(errors::NO_ACTIVE_CONTENT.0, 0x8002_c104);
        assert_eq!(errors::INVALID_CONTEXT.0, 0x8002_c106);
        assert_eq!(errors::NO_MORE_CONTENT.0, 0x8002_c108);
        assert_eq!(errors::DIALOG_OPEN.0, 0x8002_c109);
        assert_eq!(errors::DIALOG_CLOSE.0, 0x8002_c10a);
        assert_eq!(errors::GENERIC.0, 0x8002_c1ff);
    }

    #[test]
    fn canceled_sentinel_is_1() {
        assert_eq!(CANCELED, 1);
    }

    #[test]
    fn sysutil_events_stable() {
        assert_eq!(SYSUTIL_INITIALIZING_FINISHED, 1);
        assert_eq!(SYSUTIL_SHUTDOWN_FINISHED, 4);
        assert_eq!(SYSUTIL_LOADING_FINISHED, 5);
        assert_eq!(SYSUTIL_UNLOADING_FINISHED, 7);
        assert_eq!(SYSUTIL_RELEASED, 9);
        assert_eq!(SYSUTIL_GRABBED, 11);
    }

    #[test]
    fn event_types_stable() {
        assert_eq!(EVENT_STATUS_NOTIFICATION, 0);
        assert_eq!(EVENT_INITIALIZE_RESULT, 1);
        assert_eq!(EVENT_FINALIZE_RESULT, 2);
        assert_eq!(EVENT_SET_PLAYBACK_COMMAND_RESULT, 4);
        assert_eq!(EVENT_SET_VOLUME_RESULT, 5);
        assert_eq!(EVENT_UI_NOTIFICATION, 7);
    }

    #[test]
    fn pb_command_constants_stable() {
        assert_eq!(PB_CMD_STOP, 0);
        assert_eq!(PB_CMD_PLAY, 1);
        assert_eq!(PB_CMD_PAUSE, 2);
        assert_eq!(PB_CMD_NEXT, 3);
        assert_eq!(PB_CMD_PREV, 4);
        assert_eq!(PB_CMD_FASTFORWARD, 5);
        assert_eq!(PB_CMD_FASTREVERSE, 6);
    }

    #[test]
    fn pb_status_constants_stable() {
        assert_eq!(PB_STATUS_STOP, 0);
        assert_eq!(PB_STATUS_PLAY, 1);
        assert_eq!(PB_STATUS_PAUSE, 2);
        assert_eq!(PB_STATUS_FASTFORWARD, 3);
        assert_eq!(PB_STATUS_FASTREVERSE, 4);
    }

    #[test]
    fn container_and_context_sizes_stable() {
        assert_eq!(PLAYBACK_MEMORY_CONTAINER_SIZE, 11 * 1024 * 1024);
        assert_eq!(SELECTION_CONTEXT_SIZE, 2048);
    }

    #[test]
    fn initialize_happy_path() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        assert_eq!(p.state(), PlayerState::Initialized);
    }

    #[test]
    fn initialize_bad_mode_rejected() {
        let mut p = MusicPlayer::new();
        assert_eq!(p.initialize(99), Err(errors::PARAM));
    }

    #[test]
    fn initialize_twice_is_busy() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        assert_eq!(p.initialize(PLAYER_MODE_NORMAL), Err(errors::BUSY));
    }

    #[test]
    fn finalize_without_init_is_generic() {
        let mut p = MusicPlayer::new();
        assert_eq!(p.finalize(), Err(errors::GENERIC));
    }

    #[test]
    fn finalize_after_init_ok() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        p.finalize().unwrap();
        assert_eq!(p.state(), PlayerState::Uninitialized);
    }

    #[test]
    fn select_contents_without_init_is_generic() {
        let mut p = MusicPlayer::new();
        assert_eq!(p.select_contents(), Err(errors::GENERIC));
    }

    #[test]
    fn select_contents_twice_is_dialog_open() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        p.select_contents().unwrap();
        assert_eq!(p.select_contents(), Err(errors::DIALOG_OPEN));
    }

    #[test]
    fn complete_selection_without_open_dialog_is_dialog_close() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        assert_eq!(
            p.complete_selection(SelectionContext::new("x", vec!["a".into()])),
            Err(errors::DIALOG_CLOSE)
        );
    }

    #[test]
    fn cancel_selection_cycle() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        p.select_contents().unwrap();
        p.cancel_selection().unwrap();
        // After cancel, can start a new selection.
        p.select_contents().unwrap();
    }

    #[test]
    fn set_playback_command_before_selection_no_active_content() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        assert_eq!(p.set_playback_command(PB_CMD_PLAY), Err(errors::NO_ACTIVE_CONTENT));
    }

    #[test]
    fn set_playback_command_stop_before_selection_ok() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        // STOP is legal even without content.
        p.set_playback_command(PB_CMD_STOP).unwrap();
    }

    #[test]
    fn set_playback_command_unknown_rejected() {
        let mut p = ready_player_with_playlist();
        assert_eq!(p.set_playback_command(99), Err(errors::PARAM));
    }

    #[test]
    fn play_pause_stop_cycle() {
        let mut p = ready_player_with_playlist();
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        assert_eq!(p.playback_status(), PB_STATUS_PLAY);
        p.set_playback_command(PB_CMD_PAUSE).unwrap();
        assert_eq!(p.playback_status(), PB_STATUS_PAUSE);
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        assert_eq!(p.playback_status(), PB_STATUS_PLAY);
        p.set_playback_command(PB_CMD_STOP).unwrap();
        assert_eq!(p.playback_status(), PB_STATUS_STOP);
    }

    #[test]
    fn fast_forward_and_reverse_transition_status() {
        let mut p = ready_player_with_playlist();
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        p.set_playback_command(PB_CMD_FASTFORWARD).unwrap();
        assert_eq!(p.playback_status(), PB_STATUS_FASTFORWARD);
        p.set_playback_command(PB_CMD_FASTREVERSE).unwrap();
        assert_eq!(p.playback_status(), PB_STATUS_FASTREVERSE);
    }

    #[test]
    fn next_advances_current_track() {
        let mut p = ready_player_with_playlist();
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        p.set_playback_command(PB_CMD_NEXT).unwrap();
        assert_eq!(p.context.as_ref().unwrap().current_track, 1);
        p.set_playback_command(PB_CMD_NEXT).unwrap();
        assert_eq!(p.context.as_ref().unwrap().current_track, 2);
    }

    #[test]
    fn next_at_end_is_no_more_content() {
        let mut p = ready_player_with_playlist();
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        p.set_playback_command(PB_CMD_NEXT).unwrap();
        p.set_playback_command(PB_CMD_NEXT).unwrap();
        // Only 3 tracks (0/1/2); a 4th next must fail.
        assert_eq!(p.set_playback_command(PB_CMD_NEXT), Err(errors::NO_MORE_CONTENT));
    }

    #[test]
    fn prev_at_start_is_no_more_content() {
        let mut p = ready_player_with_playlist();
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        assert_eq!(p.set_playback_command(PB_CMD_PREV), Err(errors::NO_MORE_CONTENT));
    }

    #[test]
    fn prev_walks_backwards() {
        let mut p = ready_player_with_playlist();
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        p.set_playback_command(PB_CMD_NEXT).unwrap();
        p.set_playback_command(PB_CMD_PREV).unwrap();
        assert_eq!(p.context.as_ref().unwrap().current_track, 0);
    }

    #[test]
    fn set_volume_range() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        p.set_volume(0.0).unwrap();
        p.set_volume(1.0).unwrap();
        assert_eq!(p.set_volume(-0.1), Err(errors::PARAM));
        assert_eq!(p.set_volume(1.5), Err(errors::PARAM));
        assert_eq!(p.set_volume(f32::NAN), Err(errors::PARAM));
    }

    #[test]
    fn set_volume_before_init_is_generic() {
        let mut p = MusicPlayer::new();
        assert_eq!(p.set_volume(0.5), Err(errors::GENERIC));
    }

    #[test]
    fn selection_context_serialize_roundtrip_magic_preserved() {
        let ctx = SelectionContext::new("abc", vec![]);
        let blob = ctx.serialize();
        assert_eq!(&blob[..4], CONTEXT_MAGIC);
    }

    #[test]
    fn selection_context_deserialize_bad_magic_invalid_context() {
        let mut blob = [0u8; SELECTION_CONTEXT_SIZE];
        blob[..4].copy_from_slice(b"BAD!");
        assert_eq!(SelectionContext::deserialize(&blob).err(), Some(errors::INVALID_CONTEXT));
    }

    #[test]
    fn selection_context_deserialize_reads_current_track() {
        let mut ctx = SelectionContext::new("xyz", vec![]);
        ctx.current_track = 42;
        let blob = ctx.serialize();
        let roundtrip = SelectionContext::deserialize(&blob).unwrap();
        assert_eq!(roundtrip.current_track, 42);
        assert_eq!(roundtrip.hash, "xyz");
    }

    #[test]
    fn set_selection_context_via_blob_transitions_to_selected() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        let ctx = SelectionContext::new("from-game", vec![]);
        let blob = ctx.serialize();
        p.set_selection_context(&blob).unwrap();
        assert_eq!(p.state(), PlayerState::ContentsSelected);
    }

    #[test]
    fn set_selection_context_bad_blob_is_invalid_context() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        let blob = [0u8; SELECTION_CONTEXT_SIZE];
        assert_eq!(p.set_selection_context(&blob), Err(errors::INVALID_CONTEXT));
    }

    #[test]
    fn get_selection_context_without_content_is_no_active_content() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        assert_eq!(p.get_selection_context().err(), Some(errors::NO_ACTIVE_CONTENT));
    }

    #[test]
    fn get_selection_context_happy_path_returns_blob() {
        let p = ready_player_with_playlist();
        let blob = p.get_selection_context().unwrap();
        assert_eq!(&blob[..4], CONTEXT_MAGIC);
    }

    #[test]
    fn repeat_and_shuffle_constants_stable() {
        assert_eq!(REPEATMODE_NONE, 0);
        assert_eq!(REPEATMODE_ALL, 2);
        assert_eq!(CONTEXTOPTION_NONE, 0);
        assert_eq!(CONTEXTOPTION_SHUFFLE, 1);
    }

    #[test]
    fn full_playback_lifecycle_smoke() {
        let mut p = MusicPlayer::new();
        p.initialize(PLAYER_MODE_NORMAL).unwrap();
        p.select_contents().unwrap();
        p.complete_selection(SelectionContext::new("playlist-1", vec!["a".into(), "b".into()]))
            .unwrap();
        p.set_volume(0.7).unwrap();
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        p.set_playback_command(PB_CMD_NEXT).unwrap();
        p.set_playback_command(PB_CMD_PAUSE).unwrap();
        p.set_playback_command(PB_CMD_PLAY).unwrap();
        p.set_playback_command(PB_CMD_STOP).unwrap();
        p.finalize().unwrap();
    }
}
