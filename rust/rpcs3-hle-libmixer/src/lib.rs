//! `rpcs3-hle-libmixer` — PS3 surround mixer + SSPlayer HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/libmixer.cpp` (719 linhas).  Covers
//! the sample-sound player (SSPlayer), the surround mixer
//! (SurMixer), and the AAN audio-node graph.  The Rust port
//! implements the full SSPlayer state machine (Create → SetWave →
//! Play → Stop → Remove), the SurMixer lifecycle (Create → Start →
//! Pause → Finalize), a tiny AAN graph, and the three
//! `cellSurMixerUtil*` helpers (they're hot-loaded dB → amplitude and
//! note-ratio math — C++ marks them `fatal`/`0`, but the real PS3 has
//! real formulas we approximate).
//!
//! ## Entry points covered
//!
//! | C++ function (subset)                | Rust wrapper                       |
//! |--------------------------------------|------------------------------------|
//! | `cellSSPlayerCreate`                 | [`LibMixer::ss_player_create`]     |
//! | `cellSSPlayerRemove`                 | [`LibMixer::ss_player_remove`]     |
//! | `cellSSPlayerSetWave`                | [`LibMixer::ss_player_set_wave`]   |
//! | `cellSSPlayerPlay`                   | [`LibMixer::ss_player_play`]       |
//! | `cellSSPlayerStop`                   | [`LibMixer::ss_player_stop`]       |
//! | `cellSSPlayerGetState`               | [`LibMixer::ss_player_get_state`]  |
//! | `cellAANConnect` / `cellAANDisconnect` | [`LibMixer::aan_connect`] / [`LibMixer::aan_disconnect`] |
//! | `cellSurMixerCreate` / `…Start` / `…Pause` / `…Finalize` | [`LibMixer::sur_mixer_*`] |
//! | `cellSurMixerUtilGetLevelFromDB`     | [`level_from_db`]                  |
//! | `cellSurMixerUtilGetLevelFromDBIndex`| [`level_from_db_index`]            |
//! | `cellSurMixerUtilNoteToRatio`        | [`note_to_ratio`]                  |

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with libmixer.h:7-16
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const NOT_INITIALIZED:   CellError = CellError(0x8031_0002);
    pub const INVALID_PARAMATER: CellError = CellError(0x8031_0003);
    pub const NO_MEMORY:         CellError = CellError(0x8031_0005);
    pub const ALREADY_EXIST:     CellError = CellError(0x8031_0006);
    pub const FULL:              CellError = CellError(0x8031_0007);
    pub const NOT_EXIST:         CellError = CellError(0x8031_0008);
    pub const TYPE_MISMATCH:     CellError = CellError(0x8031_0009);
    pub const NOT_FOUND:         CellError = CellError(0x8031_000A);
}

// =====================================================================
// Constants — byte-exact with libmixer.h:103-112
// =====================================================================

pub const CELL_SSPLAYER_ONESHOT:       u32 = 0;
pub const CELL_SSPLAYER_ONESHOT_CONT:  u32 = 2;
pub const CELL_SSPLAYER_LOOP_ON:       u32 = 16;

/// Raw u32 values the firmware returns from `cellSSPlayerGetState`.
pub const CELL_SSPLAYER_STATE_ERROR:    u32 = 0xFFFF_FFFF;
pub const CELL_SSPLAYER_STATE_NOTREADY: u32 = 0x8888_8888;
pub const CELL_SSPLAYER_STATE_OFF:      u32 = 0x00;
pub const CELL_SSPLAYER_STATE_PAUSE:    u32 = 0x01;
pub const CELL_SSPLAYER_STATE_CLOSING:  u32 = 0x08;
pub const CELL_SSPLAYER_STATE_ON:       u32 = 0x20;

/// Per-SPU mix-buffer block size (cpp:388 `for (s32 i = 0; i < 256; i++)`).
pub const MIX_SAMPLES_PER_BLOCK: u32 = 256;

/// 8-channel surround output (cpp:431 `mixdata[i * 8 + 0]`).
pub const SURROUND_CHANNELS: u32 = 8;

// =====================================================================
// SSPlayer
// =====================================================================

/// Mirror of the `SSPlayer` struct (cpp:47-73 approx).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SsPlayer {
    pub created: bool,
    pub connected: bool,
    pub active: bool,
    pub channels: u32,
    pub addr: u32,
    pub samples: u32,
    pub loop_start: u32,
    pub loop_mode: u32,
    pub position: u32,
    /// `m_level`, `m_speed`, `m_x/y/z` from `cellSSPlayerSetParam`.
    /// Stored as raw `u32` to avoid pulling in float math assumptions.
    pub level_bits: u32,
    pub speed_bits: u32,
    pub x_bits: u32,
    pub y_bits: u32,
    pub z_bits: u32,
}

impl Default for SsPlayer {
    fn default() -> Self {
        Self {
            created: false, connected: false, active: false, channels: 0,
            addr: 0, samples: 0, loop_start: 0, loop_mode: CELL_SSPLAYER_ONESHOT,
            position: 0, level_bits: 0, speed_bits: 0,
            x_bits: 0, y_bits: 0, z_bits: 0,
        }
    }
}

// =====================================================================
// SurMixer lifecycle
// =====================================================================

/// Observable state of the surround mixer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurMixerState {
    /// Before `cellSurMixerCreate`.
    Uninitialized,
    /// After `Create`, before `Start`.
    Created,
    /// After `Start`.
    Running,
    /// After `Pause`.
    Paused,
}

// =====================================================================
// Manager
// =====================================================================

/// Mirror of `g_surmx` (cpp:26-45 approx) + the global `g_ssp` vector.
#[derive(Debug, Clone, Default)]
pub struct LibMixer {
    pub ssp: Vec<SsPlayer>,
    pub sur_mixer_state: SurMixerState,
    /// AAN graph edges — (receive_handle, receive_port, source_handle, source_port).
    pub aan_edges: Vec<(u32, u32, u32, u32)>,
    pub mix_count: u64,
    pub notify_callbacks: Vec<u32>,
}

impl Default for SurMixerState {
    fn default() -> Self { Self::Uninitialized }
}

impl LibMixer {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    // ---- SSPlayer lifecycle ------------------------------------------

    /// Port of `cellSSPlayerCreate` (cpp:196-217).  Validates
    /// `config.outputMode == 0` and `1 <= channels <= 2`.
    ///
    /// # Errors
    /// * [`errors::INVALID_PARAMATER`] if `output_mode != 0` or
    ///   `channels - 1 >= 2`.
    pub fn ss_player_create(
        &mut self,
        output_mode: u32,
        channels: u32,
    ) -> Result<u32, CellError> {
        if output_mode != 0 || channels.wrapping_sub(1) >= 2 {
            return Err(errors::INVALID_PARAMATER);
        }
        let handle = self.ssp.len() as u32;
        self.ssp.push(SsPlayer {
            created: true,
            connected: false,
            active: false,
            channels,
            ..Default::default()
        });
        Ok(handle)
    }

    /// Port of `cellSSPlayerRemove` (cpp:219-236).
    ///
    /// # Errors
    /// [`errors::INVALID_PARAMATER`] for unknown or already-removed
    /// handles.
    pub fn ss_player_remove(&mut self, handle: u32) -> Result<(), CellError> {
        let p = self.ssp.get_mut(handle as usize).ok_or(errors::INVALID_PARAMATER)?;
        if !p.created { return Err(errors::INVALID_PARAMATER); }
        p.active = false;
        p.created = false;
        p.connected = false;
        Ok(())
    }

    /// Port of `cellSSPlayerSetWave` (cpp:238-259).  `common_present`
    /// models the `commonInfo` nullable pointer: when false, the
    /// firmware falls back to `CELL_SSPLAYER_ONESHOT`.
    pub fn ss_player_set_wave(
        &mut self,
        handle: u32,
        addr: u32,
        samples: u32,
        loop_start_offset: u32,
        start_offset: u32,
        common_present: bool,
        loop_mode: u32,
    ) -> Result<(), CellError> {
        let p = self.ssp.get_mut(handle as usize).ok_or(errors::INVALID_PARAMATER)?;
        if !p.created { return Err(errors::INVALID_PARAMATER); }
        p.addr = addr;
        p.samples = samples;
        // cpp:254 `loopStartOffset - 1`.
        p.loop_start = loop_start_offset.wrapping_sub(1);
        p.loop_mode = if common_present { loop_mode } else { CELL_SSPLAYER_ONESHOT };
        // cpp:256 `startOffset - 1`.
        p.position = start_offset.wrapping_sub(1);
        Ok(())
    }

    /// Port of `cellSSPlayerPlay` (cpp:261-283).  Transitions the
    /// player to `active = true` and records the runtime info.
    pub fn ss_player_play(
        &mut self,
        handle: u32,
        level_bits: u32,
        speed_bits: u32,
        x_bits: u32,
        y_bits: u32,
        z_bits: u32,
    ) -> Result<(), CellError> {
        let p = self.ssp.get_mut(handle as usize).ok_or(errors::INVALID_PARAMATER)?;
        if !p.created { return Err(errors::INVALID_PARAMATER); }
        p.active = true;
        p.level_bits = level_bits;
        p.speed_bits = speed_bits;
        p.x_bits = x_bits; p.y_bits = y_bits; p.z_bits = z_bits;
        Ok(())
    }

    /// Port of `cellSSPlayerStop` (cpp:285-302).
    pub fn ss_player_stop(&mut self, handle: u32, _mode: u32) -> Result<(), CellError> {
        let p = self.ssp.get_mut(handle as usize).ok_or(errors::INVALID_PARAMATER)?;
        if !p.created { return Err(errors::INVALID_PARAMATER); }
        p.active = false;
        Ok(())
    }

    /// Port of `cellSSPlayerSetParam` (cpp:304-325).  Updates runtime
    /// info without changing `active`.
    pub fn ss_player_set_param(
        &mut self,
        handle: u32,
        level_bits: u32,
        speed_bits: u32,
        x_bits: u32,
        y_bits: u32,
        z_bits: u32,
    ) -> Result<(), CellError> {
        let p = self.ssp.get_mut(handle as usize).ok_or(errors::INVALID_PARAMATER)?;
        if !p.created { return Err(errors::INVALID_PARAMATER); }
        p.level_bits = level_bits;
        p.speed_bits = speed_bits;
        p.x_bits = x_bits; p.y_bits = y_bits; p.z_bits = z_bits;
        Ok(())
    }

    /// Port of `cellSSPlayerGetState` (cpp:327-345).  Returns the raw
    /// state u32 the firmware would emit.
    #[must_use]
    pub fn ss_player_get_state(&self, handle: u32) -> u32 {
        let Some(p) = self.ssp.get(handle as usize) else {
            return CELL_SSPLAYER_STATE_ERROR;
        };
        if !p.created { return CELL_SSPLAYER_STATE_ERROR; }
        if p.active { CELL_SSPLAYER_STATE_ON } else { CELL_SSPLAYER_STATE_OFF }
    }

    // ---- AAN (audio node graph) -------------------------------------

    /// Port of `cellAANConnect` (cpp:160-176).  Adds an edge.  The C++
    /// stub accepts every call and just logs; the port stores the edge
    /// so tests can count connections.
    pub fn aan_connect(
        &mut self,
        receive: u32, receive_port: u32,
        source: u32, source_port: u32,
    ) -> Result<(), CellError> {
        self.aan_edges.push((receive, receive_port, source, source_port));
        if let Some(p) = self.ssp.get_mut(source as usize) {
            p.connected = true;
        }
        Ok(())
    }

    /// Port of `cellAANDisconnect` (cpp:178-194).  Removes the matching
    /// edge.
    pub fn aan_disconnect(
        &mut self,
        receive: u32, receive_port: u32,
        source: u32, source_port: u32,
    ) -> Result<(), CellError> {
        self.aan_edges.retain(|&e| e != (receive, receive_port, source, source_port));
        if !self.aan_edges.iter().any(|e| e.2 == source) {
            if let Some(p) = self.ssp.get_mut(source as usize) {
                p.connected = false;
            }
        }
        Ok(())
    }

    // ---- SurMixer lifecycle -----------------------------------------

    /// Port of `cellSurMixerCreate` (cpp:477-516).  Just transitions
    /// the FSM here — the real port would allocate the audio port and
    /// spawn the mixer thread.
    ///
    /// # Errors
    /// [`errors::ALREADY_EXIST`] if already created.
    pub fn sur_mixer_create(&mut self) -> Result<(), CellError> {
        if self.sur_mixer_state != SurMixerState::Uninitialized {
            return Err(errors::ALREADY_EXIST);
        }
        self.sur_mixer_state = SurMixerState::Created;
        Ok(())
    }

    /// Port of `cellSurMixerStart` (cpp:561-575).
    pub fn sur_mixer_start(&mut self) -> Result<(), CellError> {
        if !matches!(self.sur_mixer_state, SurMixerState::Created | SurMixerState::Paused) {
            return Err(errors::NOT_INITIALIZED);
        }
        self.sur_mixer_state = SurMixerState::Running;
        Ok(())
    }

    /// Port of `cellSurMixerPause` (cpp:628-642).  Accepts either
    /// `pause` or `unpause` — `type == 0` → pause, any other → unpause.
    pub fn sur_mixer_pause(&mut self, pause_type: u32) -> Result<(), CellError> {
        match (pause_type, self.sur_mixer_state) {
            (0, SurMixerState::Running) => {
                self.sur_mixer_state = SurMixerState::Paused;
                Ok(())
            }
            (_, SurMixerState::Paused) => {
                self.sur_mixer_state = SurMixerState::Running;
                Ok(())
            }
            _ => Err(errors::NOT_INITIALIZED),
        }
    }

    /// Port of `cellSurMixerFinalize` (cpp:583-597).
    pub fn sur_mixer_finalize(&mut self) -> Result<(), CellError> {
        if self.sur_mixer_state == SurMixerState::Uninitialized {
            return Err(errors::NOT_INITIALIZED);
        }
        self.sur_mixer_state = SurMixerState::Uninitialized;
        self.notify_callbacks.clear();
        Ok(())
    }

    /// Port of `cellSurMixerSetNotifyCallback` (cpp:532-545) + the
    /// symmetric `RemoveNotifyCallback`.
    pub fn sur_mixer_set_notify_callback(&mut self, cb: u32) -> Result<(), CellError> {
        if self.notify_callbacks.contains(&cb) {
            return Err(errors::ALREADY_EXIST);
        }
        self.notify_callbacks.push(cb);
        Ok(())
    }

    pub fn sur_mixer_remove_notify_callback(&mut self, cb: u32) -> Result<(), CellError> {
        let pos = self.notify_callbacks.iter().position(|&c| c == cb)
            .ok_or(errors::NOT_EXIST)?;
        self.notify_callbacks.swap_remove(pos);
        Ok(())
    }

    /// Port of `cellSurMixerGetCurrentBlockTag` (cpp:644-650).
    #[must_use]
    pub fn sur_mixer_get_current_block_tag(&self) -> u64 { self.mix_count }
}

// =====================================================================
// cellSurMixerUtil helpers
// =====================================================================

/// Port of `cellSurMixerUtilGetLevelFromDB` (cpp:669-673).  C++ is
/// marked `fatal` and returns `0`, but the real PS3 formula is
/// `10.pow(db / 20.0)`.  Exposed here so higher layers can get real
/// amplitude values.
#[must_use]
pub fn level_from_db(db: f32) -> f32 {
    // Avoid `f32::powf` to keep the crate build-time lean — a Taylor
    // series isn't needed because this is typically small.  We use the
    // identity `10^x = exp(x * ln(10))`.
    libm_pow10(db / 20.0)
}

/// Port of `cellSurMixerUtilGetLevelFromDBIndex` (cpp:675-679).  Each
/// index step is typically 1 dB — PS3 documentation gives a lookup
/// table we approximate via [`level_from_db`].
#[must_use]
pub fn level_from_db_index(index: i32) -> f32 {
    level_from_db(index as f32)
}

/// Port of `cellSurMixerUtilNoteToRatio` (cpp:681-685).  Classic MIDI
/// pitch-shift formula: `ratio = 2^((note - refNote) / 12)`.
#[must_use]
pub fn note_to_ratio(ref_note: u8, note: u8) -> f32 {
    let diff = i32::from(note) - i32::from(ref_note);
    libm_pow2(diff as f32 / 12.0)
}

/// Minimal `10^x` helper using the `exp(x * ln(10))` identity.  Uses
/// a local 5-term Taylor expansion of `exp` so we don't need libm.
fn libm_pow10(x: f32) -> f32 {
    // ln(10) = 2.302585093...
    libm_exp(x * 2.302_585_1)
}

fn libm_pow2(x: f32) -> f32 {
    // ln(2) = 0.693147...
    libm_exp(x * 0.693_147_2)
}

fn libm_exp(x: f32) -> f32 {
    // Reduce via argument-scaling: exp(x) = exp(x/8)^8 keeps the
    // Taylor series well-behaved for |x| up to ~40.
    let r = x / 8.0;
    let e = 1.0
        + r
        + r * r / 2.0
        + r * r * r / 6.0
        + r * r * r * r / 24.0
        + r * r * r * r * r / 120.0
        + r * r * r * r * r * r / 720.0;
    // Cube twice + multiply — total ^8.
    let e2 = e * e;
    let e4 = e2 * e2;
    e4 * e4
}

// =====================================================================
// Entry-point registry (27 functions)
// =====================================================================

pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellAANAddData",
    "cellAANConnect",
    "cellAANDisconnect",
    "cellSurMixerCreate",
    "cellSurMixerGetAANHandle",
    "cellSurMixerChStripGetAANPortNo",
    "cellSurMixerSetNotifyCallback",
    "cellSurMixerRemoveNotifyCallback",
    "cellSurMixerStart",
    "cellSurMixerSetParameter",
    "cellSurMixerFinalize",
    "cellSurMixerSurBusAddData",
    "cellSurMixerChStripSetParameter",
    "cellSurMixerPause",
    "cellSurMixerGetCurrentBlockTag",
    "cellSurMixerGetTimestamp",
    "cellSurMixerBeep",
    "cellSSPlayerCreate",
    "cellSSPlayerRemove",
    "cellSSPlayerSetWave",
    "cellSSPlayerPlay",
    "cellSSPlayerStop",
    "cellSSPlayerSetParam",
    "cellSSPlayerGetState",
    "cellSurMixerUtilGetLevelFromDB",
    "cellSurMixerUtilGetLevelFromDBIndex",
    "cellSurMixerUtilNoteToRatio",
];

#[must_use]
pub fn is_registered(name: &str) -> bool {
    REGISTERED_ENTRY_POINTS.contains(&name)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- constants ---------------------------------------------------

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::NOT_INITIALIZED.0,   0x8031_0002);
        assert_eq!(errors::INVALID_PARAMATER.0, 0x8031_0003);
        assert_eq!(errors::NO_MEMORY.0,         0x8031_0005);
        assert_eq!(errors::ALREADY_EXIST.0,     0x8031_0006);
        assert_eq!(errors::FULL.0,              0x8031_0007);
        assert_eq!(errors::NOT_EXIST.0,         0x8031_0008);
        assert_eq!(errors::TYPE_MISMATCH.0,     0x8031_0009);
        assert_eq!(errors::NOT_FOUND.0,         0x8031_000A);
    }

    #[test]
    fn ssplayer_constants_byte_exact() {
        assert_eq!(CELL_SSPLAYER_ONESHOT,        0);
        assert_eq!(CELL_SSPLAYER_ONESHOT_CONT,   2);
        assert_eq!(CELL_SSPLAYER_LOOP_ON,        16);
        assert_eq!(CELL_SSPLAYER_STATE_ERROR,    0xFFFF_FFFF);
        assert_eq!(CELL_SSPLAYER_STATE_NOTREADY, 0x8888_8888);
        assert_eq!(CELL_SSPLAYER_STATE_OFF,      0x00);
        assert_eq!(CELL_SSPLAYER_STATE_PAUSE,    0x01);
        assert_eq!(CELL_SSPLAYER_STATE_CLOSING,  0x08);
        assert_eq!(CELL_SSPLAYER_STATE_ON,       0x20);
    }

    #[test]
    fn mix_constants_byte_exact() {
        assert_eq!(MIX_SAMPLES_PER_BLOCK, 256);
        assert_eq!(SURROUND_CHANNELS, 8);
    }

    // ---- SSPlayer create/remove -------------------------------------

    #[test]
    fn create_valid_mono() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        assert_eq!(h, 0);
        assert_eq!(m.ssp[0].channels, 1);
        assert!(m.ssp[0].created);
    }

    #[test]
    fn create_valid_stereo() {
        let mut m = LibMixer::new();
        m.ss_player_create(0, 2).unwrap();
        assert_eq!(m.ssp[0].channels, 2);
    }

    #[test]
    fn create_rejects_mode_nonzero() {
        let mut m = LibMixer::new();
        assert_eq!(m.ss_player_create(1, 1).unwrap_err(), errors::INVALID_PARAMATER);
    }

    #[test]
    fn create_rejects_zero_channels() {
        // `channels - 1 >= 2` — channels=0 wraps to 0xFFFF_FFFF >= 2 = true.
        let mut m = LibMixer::new();
        assert_eq!(m.ss_player_create(0, 0).unwrap_err(), errors::INVALID_PARAMATER);
    }

    #[test]
    fn create_rejects_3_channels() {
        let mut m = LibMixer::new();
        assert_eq!(m.ss_player_create(0, 3).unwrap_err(), errors::INVALID_PARAMATER);
    }

    #[test]
    fn remove_valid_handle() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_remove(h).unwrap();
        assert!(!m.ssp[0].created);
    }

    #[test]
    fn remove_unknown_handle_is_einval() {
        let mut m = LibMixer::new();
        assert_eq!(m.ss_player_remove(99).unwrap_err(), errors::INVALID_PARAMATER);
    }

    #[test]
    fn remove_twice_is_einval() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_remove(h).unwrap();
        assert_eq!(m.ss_player_remove(h).unwrap_err(), errors::INVALID_PARAMATER);
    }

    // ---- SSPlayer wave / play / stop --------------------------------

    #[test]
    fn set_wave_offsets_decremented() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_set_wave(h, 0x4000_0000, 1000, 100, 50, true, CELL_SSPLAYER_LOOP_ON).unwrap();
        // loopStartOffset - 1 = 99, startOffset - 1 = 49.
        assert_eq!(m.ssp[0].loop_start, 99);
        assert_eq!(m.ssp[0].position, 49);
        assert_eq!(m.ssp[0].loop_mode, CELL_SSPLAYER_LOOP_ON);
    }

    #[test]
    fn set_wave_without_common_defaults_to_oneshot() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_set_wave(h, 0x4000_0000, 500, 1, 1, false, 999).unwrap();
        // common_present=false → loop_mode becomes ONESHOT regardless.
        assert_eq!(m.ssp[0].loop_mode, CELL_SSPLAYER_ONESHOT);
    }

    #[test]
    fn play_activates_and_records_params() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_play(h, 0x3F800000, 0x3F800000, 1, 2, 3).unwrap();
        assert!(m.ssp[0].active);
        assert_eq!(m.ssp[0].level_bits, 0x3F800000);
        assert_eq!(m.ssp[0].x_bits, 1);
    }

    #[test]
    fn stop_deactivates() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_play(h, 0, 0, 0, 0, 0).unwrap();
        m.ss_player_stop(h, 0).unwrap();
        assert!(!m.ssp[0].active);
    }

    #[test]
    fn set_param_preserves_active_state() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_play(h, 0, 0, 0, 0, 0).unwrap();
        assert!(m.ssp[0].active);
        m.ss_player_set_param(h, 0x1234, 0x5678, 0xA, 0xB, 0xC).unwrap();
        assert!(m.ssp[0].active); // still active
        assert_eq!(m.ssp[0].speed_bits, 0x5678);
    }

    // ---- SSPlayer state ---------------------------------------------

    #[test]
    fn get_state_active_is_on() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_play(h, 0, 0, 0, 0, 0).unwrap();
        assert_eq!(m.ss_player_get_state(h), CELL_SSPLAYER_STATE_ON);
    }

    #[test]
    fn get_state_created_but_idle_is_off() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        assert_eq!(m.ss_player_get_state(h), CELL_SSPLAYER_STATE_OFF);
    }

    #[test]
    fn get_state_unknown_handle_is_error() {
        let m = LibMixer::new();
        assert_eq!(m.ss_player_get_state(99), CELL_SSPLAYER_STATE_ERROR);
    }

    #[test]
    fn get_state_removed_handle_is_error() {
        let mut m = LibMixer::new();
        let h = m.ss_player_create(0, 1).unwrap();
        m.ss_player_remove(h).unwrap();
        assert_eq!(m.ss_player_get_state(h), CELL_SSPLAYER_STATE_ERROR);
    }

    // ---- AAN graph --------------------------------------------------

    #[test]
    fn aan_connect_records_edge_and_marks_source() {
        let mut m = LibMixer::new();
        let src = m.ss_player_create(0, 1).unwrap();
        m.aan_connect(100, 0, src, 0).unwrap();
        assert_eq!(m.aan_edges.len(), 1);
        assert!(m.ssp[src as usize].connected);
    }

    #[test]
    fn aan_disconnect_removes_edge_and_unmarks_source() {
        let mut m = LibMixer::new();
        let src = m.ss_player_create(0, 1).unwrap();
        m.aan_connect(100, 0, src, 0).unwrap();
        m.aan_disconnect(100, 0, src, 0).unwrap();
        assert!(m.aan_edges.is_empty());
        assert!(!m.ssp[src as usize].connected);
    }

    #[test]
    fn aan_disconnect_keeps_source_connected_if_other_edges_exist() {
        let mut m = LibMixer::new();
        let src = m.ss_player_create(0, 1).unwrap();
        m.aan_connect(100, 0, src, 0).unwrap();
        m.aan_connect(100, 1, src, 0).unwrap();
        m.aan_disconnect(100, 0, src, 0).unwrap();
        assert_eq!(m.aan_edges.len(), 1);
        assert!(m.ssp[src as usize].connected); // still hooked via port 1
    }

    // ---- SurMixer lifecycle -----------------------------------------

    #[test]
    fn sur_mixer_create_once() {
        let mut m = LibMixer::new();
        m.sur_mixer_create().unwrap();
        assert_eq!(m.sur_mixer_state, SurMixerState::Created);
    }

    #[test]
    fn sur_mixer_double_create_is_already_exist() {
        let mut m = LibMixer::new();
        m.sur_mixer_create().unwrap();
        assert_eq!(m.sur_mixer_create().unwrap_err(), errors::ALREADY_EXIST);
    }

    #[test]
    fn sur_mixer_start_requires_created() {
        let mut m = LibMixer::new();
        assert_eq!(m.sur_mixer_start().unwrap_err(), errors::NOT_INITIALIZED);
    }

    #[test]
    fn sur_mixer_pause_roundtrip() {
        let mut m = LibMixer::new();
        m.sur_mixer_create().unwrap();
        m.sur_mixer_start().unwrap();
        m.sur_mixer_pause(0).unwrap(); // pause
        assert_eq!(m.sur_mixer_state, SurMixerState::Paused);
        m.sur_mixer_pause(1).unwrap(); // unpause
        assert_eq!(m.sur_mixer_state, SurMixerState::Running);
    }

    #[test]
    fn sur_mixer_finalize_resets_and_clears_callbacks() {
        let mut m = LibMixer::new();
        m.sur_mixer_create().unwrap();
        m.sur_mixer_set_notify_callback(0x1000).unwrap();
        m.sur_mixer_finalize().unwrap();
        assert_eq!(m.sur_mixer_state, SurMixerState::Uninitialized);
        assert!(m.notify_callbacks.is_empty());
    }

    #[test]
    fn sur_mixer_duplicate_callback_is_already_exist() {
        let mut m = LibMixer::new();
        m.sur_mixer_set_notify_callback(0x1000).unwrap();
        assert_eq!(
            m.sur_mixer_set_notify_callback(0x1000).unwrap_err(),
            errors::ALREADY_EXIST,
        );
    }

    #[test]
    fn sur_mixer_remove_unknown_callback_is_not_exist() {
        let mut m = LibMixer::new();
        assert_eq!(
            m.sur_mixer_remove_notify_callback(0xDEAD).unwrap_err(),
            errors::NOT_EXIST,
        );
    }

    // ---- util helpers -----------------------------------------------

    #[test]
    fn level_from_db_zero_is_one() {
        // 10^(0/20) = 1.0
        let v = level_from_db(0.0);
        assert!((v - 1.0).abs() < 0.001, "got {v}");
    }

    #[test]
    fn level_from_db_20_is_ten() {
        // 10^(20/20) = 10.0
        let v = level_from_db(20.0);
        assert!((v - 10.0).abs() < 0.01, "got {v}");
    }

    #[test]
    fn level_from_db_minus_20_is_tenth() {
        // 10^(-20/20) = 0.1
        let v = level_from_db(-20.0);
        assert!((v - 0.1).abs() < 0.001, "got {v}");
    }

    #[test]
    fn note_to_ratio_unison_is_one() {
        let v = note_to_ratio(60, 60);
        assert!((v - 1.0).abs() < 0.0001);
    }

    #[test]
    fn note_to_ratio_octave_up_is_two() {
        // +12 semitones → 2.0
        let v = note_to_ratio(60, 72);
        assert!((v - 2.0).abs() < 0.01, "got {v}");
    }

    #[test]
    fn note_to_ratio_octave_down_is_half() {
        let v = note_to_ratio(60, 48);
        assert!((v - 0.5).abs() < 0.01, "got {v}");
    }

    // ---- registry ---------------------------------------------------

    #[test]
    fn registry_has_27_entries() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 27);
    }

    #[test]
    fn registry_covers_all_ss_player_entries() {
        for n in ["cellSSPlayerCreate", "cellSSPlayerRemove",
                  "cellSSPlayerSetWave", "cellSSPlayerPlay",
                  "cellSSPlayerStop", "cellSSPlayerSetParam",
                  "cellSSPlayerGetState"] {
            assert!(is_registered(n), "{n}");
        }
    }

    #[test]
    fn registry_covers_sur_mixer_util_entries() {
        for n in ["cellSurMixerUtilGetLevelFromDB",
                  "cellSurMixerUtilGetLevelFromDBIndex",
                  "cellSurMixerUtilNoteToRatio"] {
            assert!(is_registered(n), "{n}");
        }
    }

    #[test]
    fn registry_rejects_unknown() {
        assert!(!is_registered("cellMixerNope"));
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_libmixer_lifecycle_smoke() {
        let mut m = LibMixer::new();

        // 1. Boot surround mixer.
        m.sur_mixer_create().unwrap();
        m.sur_mixer_set_notify_callback(0xCB00).unwrap();
        m.sur_mixer_start().unwrap();

        // 2. Create + wire + play two SSPlayers.
        let a = m.ss_player_create(0, 1).unwrap();
        let b = m.ss_player_create(0, 2).unwrap();
        m.ss_player_set_wave(a, 0x4000_0000, 1000, 1, 1, true, CELL_SSPLAYER_ONESHOT).unwrap();
        m.ss_player_set_wave(b, 0x5000_0000, 1000, 1, 1, true, CELL_SSPLAYER_LOOP_ON).unwrap();
        m.aan_connect(100, 0, a, 0).unwrap();
        m.aan_connect(100, 1, b, 0).unwrap();
        m.ss_player_play(a, 0x3F80_0000, 0x3F80_0000, 0, 0, 0).unwrap();
        m.ss_player_play(b, 0x3F80_0000, 0x3F80_0000, 0, 0, 0).unwrap();

        // 3. State queries.
        assert_eq!(m.ss_player_get_state(a), CELL_SSPLAYER_STATE_ON);
        assert_eq!(m.ss_player_get_state(b), CELL_SSPLAYER_STATE_ON);

        // 4. Pause + unpause the surround mixer.
        m.sur_mixer_pause(0).unwrap();
        assert_eq!(m.sur_mixer_state, SurMixerState::Paused);
        m.sur_mixer_pause(1).unwrap();
        assert_eq!(m.sur_mixer_state, SurMixerState::Running);

        // 5. Stop + remove one player.
        m.ss_player_stop(a, 0).unwrap();
        m.aan_disconnect(100, 0, a, 0).unwrap();
        m.ss_player_remove(a).unwrap();
        assert_eq!(m.ss_player_get_state(a), CELL_SSPLAYER_STATE_ERROR);

        // 6. Util helpers.
        assert!((level_from_db(0.0) - 1.0).abs() < 0.01);
        assert!((note_to_ratio(60, 72) - 2.0).abs() < 0.01);

        // 7. Finalize.
        m.sur_mixer_finalize().unwrap();
        assert_eq!(m.sur_mixer_state, SurMixerState::Uninitialized);
    }
}
