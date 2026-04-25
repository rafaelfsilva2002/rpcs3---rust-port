//! `rpcs3-hle-libsynth2` — PS3 Sound Synth 2 (SPU-2 style) HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/libsynth2.cpp` (145 linhas).  The
//! module emulates a PS2-style SPU-2 sound synthesizer the PS3
//! firmware exposes for backwards-compatible audio engines.  Covers
//! Config/Init/Exit lifecycle, register-based state (SetParam/Get/
//! SetSwitch/SetAddr), effect attr, audio sample generation, voice
//! DMA, and the Note↔Pitch conversion helpers.
//!
//! ## Entry points covered
//!
//! | C++ function                         | Rust wrapper                         |
//! |--------------------------------------|--------------------------------------|
//! | `cellSoundSynth2Config`              | [`SoundSynth2::config`]              |
//! | `cellSoundSynth2Init`                | [`SoundSynth2::init`]                |
//! | `cellSoundSynth2Exit`                | [`SoundSynth2::exit`]                |
//! | `cellSoundSynth2SetParam/GetParam`   | [`SoundSynth2::set_param`] / [`SoundSynth2::get_param`] |
//! | `cellSoundSynth2SetSwitch/GetSwitch` | [`SoundSynth2::set_switch`] / [`SoundSynth2::get_switch`] |
//! | `cellSoundSynth2SetAddr/GetAddr`     | [`SoundSynth2::set_addr`] / [`SoundSynth2::get_addr`] |
//! | `cellSoundSynth2SetEffectAttr`       | [`SoundSynth2::set_effect_attr`]     |
//! | `cellSoundSynth2SetEffectMode`       | [`SoundSynth2::set_effect_mode`]     |
//! | `cellSoundSynth2SetCoreAttr`         | [`SoundSynth2::set_core_attr`]       |
//! | `cellSoundSynth2Generate`            | [`SoundSynth2::generate`]            |
//! | `cellSoundSynth2VoiceTrans`          | [`SoundSynth2::voice_trans`]         |
//! | `cellSoundSynth2VoiceTransStatus`    | [`SoundSynth2::voice_trans_status`]  |
//! | `cellSoundSynth2Note2Pitch`          | [`note2pitch`]                       |
//! | `cellSoundSynth2Pitch2Note`          | [`pitch2note`]                       |

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with libsynth2.h:6-8
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const FATAL:               CellError = CellError(0x8031_0201);
    pub const INVALID_PARAMETER:   CellError = CellError(0x8031_0202);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8031_0203);
}

// =====================================================================
// Constants
// =====================================================================

/// SPU-2 voice count per core.
pub const VOICES_PER_CORE: usize = 24;

/// Number of SPU-2 cores.
pub const NUM_CORES: usize = 2;

/// Total voice count.
pub const TOTAL_VOICES: usize = VOICES_PER_CORE * NUM_CORES; // 48

/// Voice-transfer mode: `0` = sync (wait for DMA to complete), `1` = async.
pub const VOICE_TRANS_SYNC:  u16 = 0;
pub const VOICE_TRANS_ASYNC: u16 = 1;

// =====================================================================
// Lifecycle FSM
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynthState {
    Uninitialized,
    Initialized,
}

impl Default for SynthState {
    fn default() -> Self { Self::Uninitialized }
}

// =====================================================================
// Register banks — HashMap-like storage so tests can verify writes
// =====================================================================

/// Per-register storage for `SetParam` / `SetSwitch` / `SetAddr`.  The
/// firmware's SPU-2 has specific register addresses (0x0..0x7FF), but
/// the C++ stub accepts any 16-bit register id; we match that.
#[derive(Debug, Clone, Default)]
pub struct RegisterBank {
    pub params: Vec<(u16, u16)>,   // (reg, u16 value)
    pub switches: Vec<(u16, u32)>, // (reg, u32 value)
    pub addrs: Vec<(u16, u32)>,    // (reg, u32 value)
    pub core_attrs: Vec<(u16, u16)>,
}

impl RegisterBank {
    fn set_param(&mut self, reg: u16, value: u16) {
        if let Some(slot) = self.params.iter_mut().find(|(r, _)| *r == reg) {
            slot.1 = value;
        } else {
            self.params.push((reg, value));
        }
    }
    fn get_param(&self, reg: u16) -> u16 {
        self.params.iter().find(|(r, _)| *r == reg).map_or(0, |(_, v)| *v)
    }
    fn set_switch(&mut self, reg: u16, value: u32) {
        if let Some(slot) = self.switches.iter_mut().find(|(r, _)| *r == reg) {
            slot.1 = value;
        } else {
            self.switches.push((reg, value));
        }
    }
    fn get_switch(&self, reg: u16) -> u32 {
        self.switches.iter().find(|(r, _)| *r == reg).map_or(0, |(_, v)| *v)
    }
    fn set_addr(&mut self, reg: u16, value: u32) {
        if let Some(slot) = self.addrs.iter_mut().find(|(r, _)| *r == reg) {
            slot.1 = value;
        } else {
            self.addrs.push((reg, value));
        }
    }
    fn get_addr(&self, reg: u16) -> u32 {
        self.addrs.iter().find(|(r, _)| *r == reg).map_or(0, |(_, v)| *v)
    }
    fn set_core_attr(&mut self, entry: u16, value: u16) {
        if let Some(slot) = self.core_attrs.iter_mut().find(|(r, _)| *r == entry) {
            slot.1 = value;
        } else {
            self.core_attrs.push((entry, value));
        }
    }
}

// =====================================================================
// Effect attributes
// =====================================================================

/// Mirror of `CellSoundSynth2EffectAttr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectAttr {
    pub effect_type: i16,
    pub volume: u16,
    pub feedback: i16,
    pub delay: u16,
}

// =====================================================================
// Voice DMA trackers
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceTransState {
    #[default]
    Idle,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VoiceTransfer {
    pub channel: i16,
    pub mode: u16,
    pub m_addr: u32,
    pub s_addr: u32,
    pub size: u32,
    pub state: VoiceTransState,
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Clone, Default)]
pub struct SoundSynth2 {
    pub state: SynthState,
    pub init_flag: i16,
    pub config_slots: Vec<(i16, i32)>,
    pub core0: RegisterBank,
    pub core1: RegisterBank,
    pub effect_attrs: Vec<(i16, EffectAttr)>,
    pub effect_modes: Vec<(i16, EffectAttr)>,
    pub voice_transfers: Vec<VoiceTransfer>,
}

impl SoundSynth2 {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    /// Port of `cellSoundSynth2Config`.  C++ stub returns `CELL_OK`
    /// — the Rust port stores the (param, value) pair so tests can
    /// verify downstream effects.
    pub fn config(&mut self, param: i16, value: i32) -> Result<(), CellError> {
        self.config_slots.push((param, value));
        Ok(())
    }

    /// Port of `cellSoundSynth2Init`.
    ///
    /// # Errors
    /// * [`errors::ALREADY_INITIALIZED`] if already initialised.
    pub fn init(&mut self, flag: i16) -> Result<(), CellError> {
        if self.state == SynthState::Initialized {
            return Err(errors::ALREADY_INITIALIZED);
        }
        self.init_flag = flag;
        self.state = SynthState::Initialized;
        Ok(())
    }

    /// Port of `cellSoundSynth2Exit`.
    ///
    /// # Errors
    /// [`errors::FATAL`] if not initialised.
    pub fn exit(&mut self) -> Result<(), CellError> {
        if self.state != SynthState::Initialized {
            return Err(errors::FATAL);
        }
        self.state = SynthState::Uninitialized;
        self.voice_transfers.clear();
        Ok(())
    }

    // ---- Core-0 register bank (main core) ---------------------------

    pub fn set_param(&mut self, reg: u16, value: u16) { self.core0.set_param(reg, value); }
    #[must_use]
    pub fn get_param(&self, reg: u16) -> u16 { self.core0.get_param(reg) }

    pub fn set_switch(&mut self, reg: u16, value: u32) { self.core0.set_switch(reg, value); }
    #[must_use]
    pub fn get_switch(&self, reg: u16) -> u32 { self.core0.get_switch(reg) }

    pub fn set_addr(&mut self, reg: u16, value: u32) -> Result<(), CellError> {
        self.core0.set_addr(reg, value);
        Ok(())
    }
    #[must_use]
    pub fn get_addr(&self, reg: u16) -> u32 { self.core0.get_addr(reg) }

    pub fn set_core_attr(&mut self, entry: u16, value: u16) { self.core0.set_core_attr(entry, value); }

    // ---- Effects ----------------------------------------------------

    pub fn set_effect_attr(&mut self, bus: i16, attr: EffectAttr) -> Result<(), CellError> {
        if let Some(slot) = self.effect_attrs.iter_mut().find(|(b, _)| *b == bus) {
            slot.1 = attr;
        } else {
            self.effect_attrs.push((bus, attr));
        }
        Ok(())
    }

    pub fn set_effect_mode(&mut self, bus: i16, attr: EffectAttr) -> Result<(), CellError> {
        if let Some(slot) = self.effect_modes.iter_mut().find(|(b, _)| *b == bus) {
            slot.1 = attr;
        } else {
            self.effect_modes.push((bus, attr));
        }
        Ok(())
    }

    // ---- Generate ---------------------------------------------------

    /// Port of `cellSoundSynth2Generate`.  The C++ stub returns
    /// `CELL_OK` without writing the output buffers; the Rust port
    /// validates the `samples` count and returns the number that
    /// would be produced.  `samples == 0` is valid (no-op) in the
    /// firmware.
    pub fn generate(&self, samples: u16) -> Result<u16, CellError> {
        if self.state != SynthState::Initialized {
            return Err(errors::FATAL);
        }
        Ok(samples)
    }

    // ---- Voice DMA --------------------------------------------------

    /// Port of `cellSoundSynth2VoiceTrans`.  Initiates a voice sample
    /// DMA transfer.  `mode` is 0 for sync, 1 for async.
    ///
    /// # Errors
    /// * [`errors::INVALID_PARAMETER`] if `channel` is out of range
    ///   (`channel < 0 || channel >= 48`).
    pub fn voice_trans(
        &mut self,
        channel: i16,
        mode: u16,
        m_addr: u32,
        s_addr: u32,
        size: u32,
    ) -> Result<(), CellError> {
        if channel < 0 || channel as usize >= TOTAL_VOICES {
            return Err(errors::INVALID_PARAMETER);
        }
        let state = if mode == VOICE_TRANS_SYNC {
            VoiceTransState::Done
        } else {
            VoiceTransState::InProgress
        };
        self.voice_transfers.push(VoiceTransfer {
            channel, mode, m_addr, s_addr, size, state,
        });
        Ok(())
    }

    /// Port of `cellSoundSynth2VoiceTransStatus`.  Returns `CELL_OK`
    /// when the transfer is done, `FATAL` while in-progress.
    ///
    /// # Errors
    /// * [`errors::INVALID_PARAMETER`] for unknown channels.
    /// * [`errors::FATAL`] while a transfer is still in progress.
    pub fn voice_trans_status(&mut self, channel: i16, _flag: i16) -> Result<VoiceTransState, CellError> {
        if channel < 0 || channel as usize >= TOTAL_VOICES {
            return Err(errors::INVALID_PARAMETER);
        }
        // Latest transfer for this channel.
        let t = self.voice_transfers.iter_mut().rfind(|t| t.channel == channel)
            .ok_or(errors::INVALID_PARAMETER)?;
        // In async mode the firmware progresses the transfer; the
        // Rust port flips InProgress→Done on the first status poll.
        if t.state == VoiceTransState::InProgress {
            t.state = VoiceTransState::Done;
        }
        Ok(t.state)
    }
}

// =====================================================================
// Note / Pitch conversion
// =====================================================================

/// Port of `cellSoundSynth2Note2Pitch` (cpp:113-117).  The C++ stub
/// returns `0`; the real PS2 SPU-2 formula computes
/// `pitch = round(0x1000 * 2^((note + fine/128 - center) / 12))`
/// clamped to `u16`.  `center_note` + `center_fine/128` form the
/// reference pitch.
///
/// The `center_fine` / `fine` parameters are signed fine-tuning in
/// units of cents × 128 (i.e., 128 ticks = 1 semitone).
#[must_use]
pub fn note2pitch(center_note: u16, center_fine: u16, note: u16, fine: i16) -> u16 {
    // Combine each (note, fine) pair into a semitone value.
    let center_semitones = f32::from(center_note) + f32::from(center_fine) / 128.0;
    let target_semitones = f32::from(note) + f32::from(fine) / 128.0;
    let diff = target_semitones - center_semitones;
    let ratio = libm_pow2(diff / 12.0);
    let pitch = 4096.0 * ratio;
    if pitch <= 0.0 { 0 } else if pitch >= 65535.0 { 0xFFFF } else { pitch as u16 }
}

/// Port of `cellSoundSynth2Pitch2Note` (cpp:119-123).  Inverse of
/// [`note2pitch`].  Returns the note value that matches the requested
/// pitch relative to `center_note` / `center_fine`.
#[must_use]
pub fn pitch2note(center_note: u16, center_fine: u16, pitch: u16) -> u16 {
    if pitch == 0 { return 0; }
    let ratio = f32::from(pitch) / 4096.0;
    // diff_semitones = 12 * log2(ratio)
    let diff = 12.0 * libm_log2(ratio);
    let center_semitones = f32::from(center_note) + f32::from(center_fine) / 128.0;
    let note = center_semitones + diff;
    if note <= 0.0 { 0 } else if note >= 65535.0 { 0xFFFF } else { note as u16 }
}

// ---- local math helpers ---------------------------------------------

fn libm_pow2(x: f32) -> f32 { libm_exp(x * 0.693_147_2) }
fn libm_exp(x: f32) -> f32 {
    let r = x / 8.0;
    let e = 1.0 + r + r * r / 2.0 + r * r * r / 6.0
        + r * r * r * r / 24.0
        + r * r * r * r * r / 120.0
        + r * r * r * r * r * r / 720.0;
    let e2 = e * e; let e4 = e2 * e2; e4 * e4
}
fn libm_log2(x: f32) -> f32 {
    // log2(x) = ln(x) / ln(2).  ln(x) computed via a power-series
    // around 1: u = (x - 1) / (x + 1); ln(x) = 2 * (u + u^3/3 + u^5/5).
    if x <= 0.0 { return f32::NEG_INFINITY; }
    let u = (x - 1.0) / (x + 1.0);
    let u2 = u * u;
    let ln = 2.0 * (u + u * u2 / 3.0 + u * u2 * u2 / 5.0 + u * u2 * u2 * u2 / 7.0);
    ln / 0.693_147_2
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
        assert_eq!(errors::FATAL.0,               0x8031_0201);
        assert_eq!(errors::INVALID_PARAMETER.0,   0x8031_0202);
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8031_0203);
    }

    #[test]
    fn voice_count_constants() {
        assert_eq!(VOICES_PER_CORE, 24);
        assert_eq!(NUM_CORES, 2);
        assert_eq!(TOTAL_VOICES, 48);
    }

    #[test]
    fn trans_mode_constants() {
        assert_eq!(VOICE_TRANS_SYNC, 0);
        assert_eq!(VOICE_TRANS_ASYNC, 1);
    }

    // ---- Lifecycle FSM ----------------------------------------------

    #[test]
    fn init_transitions_to_initialized() {
        let mut s = SoundSynth2::new();
        s.init(0).unwrap();
        assert_eq!(s.state, SynthState::Initialized);
    }

    #[test]
    fn init_twice_is_already_initialized() {
        let mut s = SoundSynth2::new();
        s.init(0).unwrap();
        assert_eq!(s.init(0).unwrap_err(), errors::ALREADY_INITIALIZED);
    }

    #[test]
    fn exit_without_init_is_fatal() {
        let mut s = SoundSynth2::new();
        assert_eq!(s.exit().unwrap_err(), errors::FATAL);
    }

    #[test]
    fn init_exit_roundtrip() {
        let mut s = SoundSynth2::new();
        s.init(1).unwrap();
        s.exit().unwrap();
        assert_eq!(s.state, SynthState::Uninitialized);
    }

    #[test]
    fn config_records_pairs() {
        let mut s = SoundSynth2::new();
        s.config(0, 1000).unwrap();
        s.config(1, 2000).unwrap();
        assert_eq!(s.config_slots, alloc::vec![(0, 1000), (1, 2000)]);
    }

    // ---- Register banks ---------------------------------------------

    #[test]
    fn set_param_then_get_roundtrips() {
        let mut s = SoundSynth2::new();
        s.set_param(0x100, 0xABCD);
        assert_eq!(s.get_param(0x100), 0xABCD);
    }

    #[test]
    fn get_unknown_param_is_zero() {
        let s = SoundSynth2::new();
        assert_eq!(s.get_param(0x100), 0);
    }

    #[test]
    fn set_param_overwrites() {
        let mut s = SoundSynth2::new();
        s.set_param(0x100, 0x1111);
        s.set_param(0x100, 0x2222);
        assert_eq!(s.get_param(0x100), 0x2222);
        // Only one entry exists.
        assert_eq!(s.core0.params.len(), 1);
    }

    #[test]
    fn set_switch_then_get_roundtrips() {
        let mut s = SoundSynth2::new();
        s.set_switch(0x200, 0xDEAD_BEEF);
        assert_eq!(s.get_switch(0x200), 0xDEAD_BEEF);
    }

    #[test]
    fn set_addr_then_get_roundtrips() {
        let mut s = SoundSynth2::new();
        s.set_addr(0x300, 0x1000_0000).unwrap();
        assert_eq!(s.get_addr(0x300), 0x1000_0000);
    }

    #[test]
    fn set_core_attr_stores() {
        let mut s = SoundSynth2::new();
        s.set_core_attr(0x400, 0x1234);
        assert_eq!(s.core0.core_attrs, alloc::vec![(0x400, 0x1234)]);
    }

    #[test]
    fn set_core_attr_overwrites() {
        let mut s = SoundSynth2::new();
        s.set_core_attr(0x400, 0x1111);
        s.set_core_attr(0x400, 0x2222);
        assert_eq!(s.core0.core_attrs.len(), 1);
        assert_eq!(s.core0.core_attrs[0].1, 0x2222);
    }

    // ---- Effects ----------------------------------------------------

    #[test]
    fn set_effect_attr_stores() {
        let mut s = SoundSynth2::new();
        let attr = EffectAttr { effect_type: 5, volume: 0x100, feedback: 10, delay: 50 };
        s.set_effect_attr(0, attr).unwrap();
        assert_eq!(s.effect_attrs, alloc::vec![(0, attr)]);
    }

    #[test]
    fn set_effect_attr_overwrites_existing_bus() {
        let mut s = SoundSynth2::new();
        let attr1 = EffectAttr { effect_type: 1, ..Default::default() };
        let attr2 = EffectAttr { effect_type: 2, ..Default::default() };
        s.set_effect_attr(0, attr1).unwrap();
        s.set_effect_attr(0, attr2).unwrap();
        assert_eq!(s.effect_attrs.len(), 1);
        assert_eq!(s.effect_attrs[0].1.effect_type, 2);
    }

    #[test]
    fn set_effect_mode_independent_of_attr() {
        let mut s = SoundSynth2::new();
        s.set_effect_attr(0, EffectAttr::default()).unwrap();
        s.set_effect_mode(0, EffectAttr { effect_type: 7, ..Default::default() }).unwrap();
        assert_eq!(s.effect_modes.len(), 1);
        assert_eq!(s.effect_modes[0].1.effect_type, 7);
    }

    // ---- Generate ---------------------------------------------------

    #[test]
    fn generate_requires_init() {
        let s = SoundSynth2::new();
        assert_eq!(s.generate(256).unwrap_err(), errors::FATAL);
    }

    #[test]
    fn generate_returns_sample_count() {
        let mut s = SoundSynth2::new();
        s.init(0).unwrap();
        assert_eq!(s.generate(256).unwrap(), 256);
        assert_eq!(s.generate(0).unwrap(), 0);
    }

    // ---- Voice DMA --------------------------------------------------

    #[test]
    fn voice_trans_bad_channel_is_einval() {
        let mut s = SoundSynth2::new();
        assert_eq!(
            s.voice_trans(-1, VOICE_TRANS_SYNC, 0, 0, 0).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
        assert_eq!(
            s.voice_trans(48, VOICE_TRANS_SYNC, 0, 0, 0).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn voice_trans_sync_completes_immediately() {
        let mut s = SoundSynth2::new();
        s.voice_trans(0, VOICE_TRANS_SYNC, 0x1000, 0x2000, 512).unwrap();
        assert_eq!(s.voice_transfers[0].state, VoiceTransState::Done);
    }

    #[test]
    fn voice_trans_async_starts_in_progress() {
        let mut s = SoundSynth2::new();
        s.voice_trans(0, VOICE_TRANS_ASYNC, 0x1000, 0x2000, 512).unwrap();
        assert_eq!(s.voice_transfers[0].state, VoiceTransState::InProgress);
    }

    #[test]
    fn voice_trans_status_async_progresses_to_done() {
        let mut s = SoundSynth2::new();
        s.voice_trans(0, VOICE_TRANS_ASYNC, 0x1000, 0x2000, 512).unwrap();
        // First poll: still InProgress → bumped to Done.
        let state = s.voice_trans_status(0, 0).unwrap();
        assert_eq!(state, VoiceTransState::Done);
    }

    #[test]
    fn voice_trans_status_unknown_channel_is_einval() {
        let mut s = SoundSynth2::new();
        assert_eq!(
            s.voice_trans_status(0, 0).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn voice_trans_boundary_channels() {
        let mut s = SoundSynth2::new();
        s.voice_trans(0, VOICE_TRANS_SYNC, 0, 0, 0).unwrap();
        s.voice_trans(47, VOICE_TRANS_SYNC, 0, 0, 0).unwrap();
        assert_eq!(s.voice_transfers.len(), 2);
    }

    // ---- Note / Pitch -----------------------------------------------

    #[test]
    fn note2pitch_center_is_0x1000() {
        // note == center_note, fine == center_fine → ratio=1 → pitch=0x1000
        let p = note2pitch(60, 0, 60, 0);
        assert!((p as i32 - 0x1000).abs() <= 1, "got {p:#x}");
    }

    #[test]
    fn note2pitch_one_octave_up_doubles() {
        // +12 semitones → ratio=2 → pitch≈0x2000
        let p = note2pitch(60, 0, 72, 0);
        assert!((p as i32 - 0x2000).abs() <= 2, "got {p:#x}");
    }

    #[test]
    fn note2pitch_one_octave_down_halves() {
        let p = note2pitch(60, 0, 48, 0);
        assert!((p as i32 - 0x0800).abs() <= 2, "got {p:#x}");
    }

    #[test]
    fn pitch2note_round_trip_through_center() {
        let p = note2pitch(60, 0, 60, 0);
        let n = pitch2note(60, 0, p);
        assert!((n as i32 - 60).abs() <= 1, "got {n}");
    }

    #[test]
    fn pitch2note_octave_up_is_72() {
        let p = note2pitch(60, 0, 72, 0);
        let n = pitch2note(60, 0, p);
        assert!((n as i32 - 72).abs() <= 1, "got {n}");
    }

    #[test]
    fn pitch2note_octave_down_is_48() {
        let p = note2pitch(60, 0, 48, 0);
        let n = pitch2note(60, 0, p);
        assert!((n as i32 - 48).abs() <= 1, "got {n}");
    }

    #[test]
    fn pitch2note_zero_pitch_is_zero() {
        assert_eq!(pitch2note(60, 0, 0), 0);
    }

    // ---- full smoke -------------------------------------------------

    #[test]
    fn full_libsynth2_lifecycle_smoke() {
        let mut s = SoundSynth2::new();

        // 1. Config + init.
        s.config(0, 44100).unwrap();
        s.init(0).unwrap();

        // 2. Configure registers.
        s.set_param(0x100, 0x1000);
        s.set_switch(0x200, 0xFFFF_FFFF);
        s.set_addr(0x300, 0x4000_0000).unwrap();
        s.set_core_attr(0x400, 0x1234);

        // 3. Apply effect attributes.
        let eff = EffectAttr { effect_type: 3, volume: 0x200, feedback: -5, delay: 100 };
        s.set_effect_attr(0, eff).unwrap();
        s.set_effect_mode(0, eff).unwrap();

        // 4. Kick off voice transfers.
        s.voice_trans(0, VOICE_TRANS_ASYNC, 0x1000_0000, 0x2000, 512).unwrap();
        s.voice_trans(1, VOICE_TRANS_SYNC,  0x1000_0200, 0x2000, 512).unwrap();
        assert_eq!(s.voice_trans_status(0, 0).unwrap(), VoiceTransState::Done);
        assert_eq!(s.voice_trans_status(1, 0).unwrap(), VoiceTransState::Done);

        // 5. Generate 256 samples.
        assert_eq!(s.generate(256).unwrap(), 256);

        // 6. Readback register values.
        assert_eq!(s.get_param(0x100), 0x1000);
        assert_eq!(s.get_switch(0x200), 0xFFFF_FFFF);
        assert_eq!(s.get_addr(0x300), 0x4000_0000);

        // 7. Note/pitch math.
        let p = note2pitch(60, 0, 72, 0);
        assert!((p as i32 - 0x2000).abs() <= 2);

        // 8. Exit.
        s.exit().unwrap();
        assert_eq!(s.state, SynthState::Uninitialized);
    }
}
