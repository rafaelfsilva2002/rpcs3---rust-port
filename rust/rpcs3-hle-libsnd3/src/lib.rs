//! `rpcs3-hle-libsnd3` — PS3 Sound Player 3 + MIDI/SMF HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/libsnd3.cpp` (390 linhas).  The
//! firmware ships ~47 entry points for a MIDI-style voice player and
//! Standard MIDI File (SMF) playback; the C++ stubs all return
//! `CELL_OK`.  The Rust port adds the observable lifecycle (Init →
//! BindSoundData → NoteOn/Off / VoiceOps → UnbindSoundData → Exit)
//! plus SMF transport (Play / Pause / Resume / Stop).

extern crate alloc;

use alloc::vec::Vec;
use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with libsnd3.h:8-23
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const PARAM:          CellError = CellError(0x8031_0301);
    pub const CREATE_MUTEX:   CellError = CellError(0x8031_0302);
    pub const SYNTH:          CellError = CellError(0x8031_0303);
    pub const ALREADY:        CellError = CellError(0x8031_0304);
    pub const NOTINIT:        CellError = CellError(0x8031_0305);
    pub const SMFFULL:        CellError = CellError(0x8031_0306);
    pub const HD3ID:          CellError = CellError(0x8031_0307);
    pub const SMF:            CellError = CellError(0x8031_0308);
    pub const SMFCTX:         CellError = CellError(0x8031_0309);
    pub const FORMAT:         CellError = CellError(0x8031_030A);
    pub const SMFID:          CellError = CellError(0x8031_030B);
    pub const SOUNDDATAFULL:  CellError = CellError(0x8031_030C);
    pub const VOICENUM:       CellError = CellError(0x8031_030D);
    pub const RESERVEDVOICE:  CellError = CellError(0x8031_030E);
    pub const REQUESTQUEFULL: CellError = CellError(0x8031_030F);
    pub const OUTPUTMODE:     CellError = CellError(0x8031_0310);
}

// =====================================================================
// Core FSM
// =====================================================================

/// Observable state of libsnd3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Snd3State {
    Uninitialized,
    Initialized,
}

impl Default for Snd3State {
    fn default() -> Self { Self::Uninitialized }
}

/// Voice states — port of `cellSnd3VoiceGetStatus` semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoiceState {
    #[default]
    Idle,
    Playing,
    SustainHold,
    /// After `VoiceKeyOff`; the voice releases via its envelope and
    /// eventually returns to `Idle`.
    Releasing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Voice {
    pub num: u32,
    pub state: VoiceState,
    pub reserve_mode: u32,
    pub sustain_hold: u32,
    pub pitch: i32,
    pub velocity: u32,
    pub panpot: u32,
    pub panpot_ex: u32,
    pub pitch_bend: u32,
    pub envelope: u32,
    pub key_on_id: u32,
    pub midi_channel: u32,
}

/// Sound data block (HD3 file — header-data-3 for SCE audio banks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoundData {
    pub hd3_id: u32,
    pub synth_mem_offset: u32,
}

/// Standard MIDI File playback context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Smf {
    pub smf_id: u32,
    pub hd3_id: u32,
    pub play_status: SmfStatus,
    pub tempo: i32,
    pub play_velocity: u32,
    pub play_panpot: u32,
    pub play_panpot_ex: u32,
    pub play_channel_bit: u32,
    pub key_on_ids: [u32; 16], // per MIDI channel
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SmfStatus {
    #[default]
    Stopped,
    Playing,
    Paused,
}

// =====================================================================
// Constants
// =====================================================================

/// Max voices the firmware allows (practical upper bound from
/// `cellSnd3Init(maxVoice, …)` semantics).
pub const MAX_VOICES: u32 = 128;

/// Max concurrent HD3 sound banks.
pub const MAX_HD3: u32 = 64;

/// Max concurrent SMF players.
pub const MAX_SMF: u32 = 16;

/// MIDI channel count.
pub const MIDI_CHANNELS: usize = 16;

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Clone, Default)]
pub struct Snd3 {
    pub state: Snd3State,
    pub max_voice: u32,
    pub samples: u32,
    pub output_mode: u32,
    pub voices: Vec<Voice>,
    pub sound_data: Vec<SoundData>,
    pub smfs: Vec<Smf>,
    pub effect_type: u16,
    pub effect_return_vol: i16,
    pub effect_delay: u16,
    pub effect_feedback: u16,
    pub next_key_on_id: u32,
    pub next_hd3_id: u32,
    pub next_smf_id: u32,
}

impl Snd3 {
    #[must_use]
    pub fn new() -> Self { Self::default() }

    fn require_init(&self) -> Result<(), CellError> {
        if self.state != Snd3State::Initialized { return Err(errors::NOTINIT); }
        Ok(())
    }

    fn get_voice_mut(&mut self, voice_num: u32) -> Result<&mut Voice, CellError> {
        self.require_init()?;
        if voice_num >= self.max_voice { return Err(errors::VOICENUM); }
        self.voices.iter_mut().find(|v| v.num == voice_num)
            .ok_or(errors::VOICENUM)
    }

    // ---- Init / Exit ------------------------------------------------

    /// Port of `cellSnd3Init` (cpp:37-41).  `max_voice` is the upper
    /// bound on concurrent voices; each voice is pre-allocated here so
    /// later ops don't have to grow the table.
    ///
    /// # Errors
    /// * [`errors::ALREADY`] if already initialised.
    /// * [`errors::PARAM`] if `max_voice == 0` or `> MAX_VOICES`.
    pub fn init(&mut self, max_voice: u32, samples: u32) -> Result<(), CellError> {
        if self.state == Snd3State::Initialized { return Err(errors::ALREADY); }
        if max_voice == 0 || max_voice > MAX_VOICES { return Err(errors::PARAM); }
        self.max_voice = max_voice;
        self.samples = samples;
        self.voices = (0..max_voice).map(|num| Voice { num, ..Default::default() }).collect();
        self.state = Snd3State::Initialized;
        Ok(())
    }

    /// Port of `cellSnd3Exit`.
    pub fn exit(&mut self) -> Result<(), CellError> {
        self.require_init()?;
        self.state = Snd3State::Uninitialized;
        self.voices.clear();
        self.sound_data.clear();
        self.smfs.clear();
        Ok(())
    }

    /// Port of `cellSnd3SetOutputMode`.
    ///
    /// # Errors
    /// * [`errors::OUTPUTMODE`] if `mode > 1` (firmware accepts 0 or 1).
    pub fn set_output_mode(&mut self, mode: u32) -> Result<(), CellError> {
        self.require_init()?;
        if mode > 1 { return Err(errors::OUTPUTMODE); }
        self.output_mode = mode;
        Ok(())
    }

    // ---- HD3 sound-data -------------------------------------------

    /// Port of `cellSnd3BindSoundData`.  Returns the new HD3 id.
    ///
    /// # Errors
    /// * [`errors::SOUNDDATAFULL`] if the table is full.
    pub fn bind_sound_data(&mut self, synth_mem_offset: u32) -> Result<u32, CellError> {
        self.require_init()?;
        if self.sound_data.len() as u32 >= MAX_HD3 {
            return Err(errors::SOUNDDATAFULL);
        }
        let hd3_id = self.next_hd3_id + 1;
        self.next_hd3_id = hd3_id;
        self.sound_data.push(SoundData { hd3_id, synth_mem_offset });
        Ok(hd3_id)
    }

    /// Port of `cellSnd3UnbindSoundData`.
    ///
    /// # Errors
    /// * [`errors::HD3ID`] if the id is unknown.
    pub fn unbind_sound_data(&mut self, hd3_id: u32) -> Result<(), CellError> {
        self.require_init()?;
        let pos = self.sound_data.iter().position(|s| s.hd3_id == hd3_id)
            .ok_or(errors::HD3ID)?;
        self.sound_data.swap_remove(pos);
        Ok(())
    }

    // ---- NoteOn / NoteOff ----------------------------------------

    /// Port of `cellSnd3NoteOnByTone`.  Allocates a key-on id and
    /// returns it.  The firmware stub just returns CELL_OK without
    /// assigning anything observable; the port gives callers a fresh
    /// id so tests can correlate Off → On.
    pub fn note_on_by_tone(&mut self, hd3_id: u32, _note: u32) -> Result<u32, CellError> {
        self.require_init()?;
        if !self.sound_data.iter().any(|s| s.hd3_id == hd3_id) {
            return Err(errors::HD3ID);
        }
        self.next_key_on_id = self.next_key_on_id.wrapping_add(1);
        Ok(self.next_key_on_id)
    }

    /// Port of `cellSnd3NoteOff`.  Also clears any voice whose
    /// `key_on_id` matches.
    pub fn note_off(&mut self, midi_channel: u32, _note: u32, key_on_id: u32) -> Result<(), CellError> {
        self.require_init()?;
        for v in &mut self.voices {
            if v.key_on_id == key_on_id && v.midi_channel == midi_channel {
                v.state = VoiceState::Releasing;
            }
        }
        Ok(())
    }

    /// Port of `cellSnd3VoiceKeyOff`.  Transitions the named voice to
    /// `Releasing`.
    pub fn voice_key_off(&mut self, voice_num: u32) -> Result<(), CellError> {
        let v = self.get_voice_mut(voice_num)?;
        v.state = VoiceState::Releasing;
        Ok(())
    }

    /// Port of `cellSnd3VoiceAllKeyOff`.
    pub fn voice_all_key_off(&mut self) -> Result<(), CellError> {
        self.require_init()?;
        for v in &mut self.voices {
            if v.state != VoiceState::Idle {
                v.state = VoiceState::Releasing;
            }
        }
        Ok(())
    }

    /// Port of `cellSnd3VoiceNoteOnByTone`.  Marks the specific voice
    /// as `Playing` with the caller-supplied state.
    pub fn voice_note_on_by_tone(
        &mut self,
        hd3_id: u32,
        voice_num: u32,
        _tone_index: u32,
        _note: u32,
        key_on_id: u32,
    ) -> Result<(), CellError> {
        if !self.sound_data.iter().any(|s| s.hd3_id == hd3_id) {
            return Err(errors::HD3ID);
        }
        let v = self.get_voice_mut(voice_num)?;
        v.state = VoiceState::Playing;
        v.key_on_id = key_on_id;
        Ok(())
    }

    // ---- Voice setters ----------------------------------------------

    pub fn voice_set_reserve_mode(&mut self, voice_num: u32, mode: u32) -> Result<(), CellError> {
        self.get_voice_mut(voice_num)?.reserve_mode = mode;
        Ok(())
    }

    pub fn voice_set_sustain_hold(&mut self, voice_num: u32, hold: u32) -> Result<(), CellError> {
        let v = self.get_voice_mut(voice_num)?;
        v.sustain_hold = hold;
        if hold != 0 { v.state = VoiceState::SustainHold; }
        Ok(())
    }

    pub fn voice_set_pitch(&mut self, voice_num: u32, add_pitch: i32) -> Result<(), CellError> {
        self.get_voice_mut(voice_num)?.pitch = add_pitch;
        Ok(())
    }

    pub fn voice_set_velocity(&mut self, voice_num: u32, velocity: u32) -> Result<(), CellError> {
        self.get_voice_mut(voice_num)?.velocity = velocity;
        Ok(())
    }

    pub fn voice_set_panpot(&mut self, voice_num: u32, panpot: u32) -> Result<(), CellError> {
        self.get_voice_mut(voice_num)?.panpot = panpot;
        Ok(())
    }

    pub fn voice_set_panpot_ex(&mut self, voice_num: u32, panpot_ex: u32) -> Result<(), CellError> {
        self.get_voice_mut(voice_num)?.panpot_ex = panpot_ex;
        Ok(())
    }

    pub fn voice_set_pitch_bend(&mut self, voice_num: u32, bend: u32) -> Result<(), CellError> {
        self.get_voice_mut(voice_num)?.pitch_bend = bend;
        Ok(())
    }

    #[must_use]
    pub fn voice_status(&self, voice_num: u32) -> Option<VoiceState> {
        if voice_num >= self.max_voice { return None; }
        self.voices.iter().find(|v| v.num == voice_num).map(|v| v.state)
    }

    // ---- Effect ----------------------------------------------------

    pub fn set_effect_type(&mut self, effect_type: u16, return_vol: i16, delay: u16, feedback: u16) -> Result<(), CellError> {
        self.require_init()?;
        self.effect_type = effect_type;
        self.effect_return_vol = return_vol;
        self.effect_delay = delay;
        self.effect_feedback = feedback;
        Ok(())
    }

    // ---- SMF / MIDI file -------------------------------------------

    /// Port of `cellSnd3SMFBind`.  Returns a fresh SMF id.
    pub fn smf_bind(&mut self, hd3_id: u32) -> Result<u32, CellError> {
        self.require_init()?;
        if !self.sound_data.iter().any(|s| s.hd3_id == hd3_id) {
            return Err(errors::HD3ID);
        }
        if self.smfs.len() as u32 >= MAX_SMF {
            return Err(errors::SMFFULL);
        }
        let smf_id = self.next_smf_id + 1;
        self.next_smf_id = smf_id;
        self.smfs.push(Smf {
            smf_id,
            hd3_id,
            play_status: SmfStatus::Stopped,
            tempo: 0,
            play_velocity: 0,
            play_panpot: 0,
            play_panpot_ex: 0,
            play_channel_bit: 0xFFFF,
            key_on_ids: [0; MIDI_CHANNELS],
        });
        Ok(smf_id)
    }

    pub fn smf_unbind(&mut self, smf_id: u32) -> Result<(), CellError> {
        self.require_init()?;
        let pos = self.smfs.iter().position(|s| s.smf_id == smf_id)
            .ok_or(errors::SMFID)?;
        self.smfs.swap_remove(pos);
        Ok(())
    }

    fn smf_mut(&mut self, smf_id: u32) -> Result<&mut Smf, CellError> {
        self.require_init()?;
        self.smfs.iter_mut().find(|s| s.smf_id == smf_id).ok_or(errors::SMFID)
    }

    pub fn smf_play(&mut self, smf_id: u32, velocity: u32, panpot: u32, _count: u32) -> Result<(), CellError> {
        let s = self.smf_mut(smf_id)?;
        s.play_status = SmfStatus::Playing;
        s.play_velocity = velocity;
        s.play_panpot = panpot;
        Ok(())
    }

    pub fn smf_pause(&mut self, smf_id: u32) -> Result<(), CellError> {
        let s = self.smf_mut(smf_id)?;
        if s.play_status != SmfStatus::Playing { return Err(errors::SMFCTX); }
        s.play_status = SmfStatus::Paused;
        Ok(())
    }

    pub fn smf_resume(&mut self, smf_id: u32) -> Result<(), CellError> {
        let s = self.smf_mut(smf_id)?;
        if s.play_status != SmfStatus::Paused { return Err(errors::SMFCTX); }
        s.play_status = SmfStatus::Playing;
        Ok(())
    }

    pub fn smf_stop(&mut self, smf_id: u32) -> Result<(), CellError> {
        let s = self.smf_mut(smf_id)?;
        s.play_status = SmfStatus::Stopped;
        Ok(())
    }

    pub fn smf_add_tempo(&mut self, smf_id: u32, add_tempo: i32) -> Result<(), CellError> {
        let s = self.smf_mut(smf_id)?;
        s.tempo = s.tempo.wrapping_add(add_tempo);
        Ok(())
    }

    #[must_use]
    pub fn smf_get_tempo(&self, smf_id: u32) -> Option<i32> {
        self.smfs.iter().find(|s| s.smf_id == smf_id).map(|s| s.tempo)
    }

    #[must_use]
    pub fn smf_get_play_status(&self, smf_id: u32) -> Option<SmfStatus> {
        self.smfs.iter().find(|s| s.smf_id == smf_id).map(|s| s.play_status)
    }
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
        assert_eq!(errors::PARAM.0,          0x8031_0301);
        assert_eq!(errors::CREATE_MUTEX.0,   0x8031_0302);
        assert_eq!(errors::ALREADY.0,        0x8031_0304);
        assert_eq!(errors::NOTINIT.0,        0x8031_0305);
        assert_eq!(errors::SMFFULL.0,        0x8031_0306);
        assert_eq!(errors::HD3ID.0,          0x8031_0307);
        assert_eq!(errors::SMFID.0,          0x8031_030B);
        assert_eq!(errors::SOUNDDATAFULL.0,  0x8031_030C);
        assert_eq!(errors::VOICENUM.0,       0x8031_030D);
        assert_eq!(errors::OUTPUTMODE.0,     0x8031_0310);
    }

    #[test]
    fn error_codes_contiguous_range() {
        assert_eq!(errors::OUTPUTMODE.0 - errors::PARAM.0 + 1, 16);
    }

    // ---- init / exit -------------------------------------------------

    #[test]
    fn init_accepts_32_voices() {
        let mut s = Snd3::new();
        s.init(32, 256).unwrap();
        assert_eq!(s.state, Snd3State::Initialized);
        assert_eq!(s.voices.len(), 32);
    }

    #[test]
    fn init_rejects_zero_voices() {
        let mut s = Snd3::new();
        assert_eq!(s.init(0, 256).unwrap_err(), errors::PARAM);
    }

    #[test]
    fn init_rejects_too_many_voices() {
        let mut s = Snd3::new();
        assert_eq!(s.init(MAX_VOICES + 1, 256).unwrap_err(), errors::PARAM);
    }

    #[test]
    fn init_rejects_double() {
        let mut s = Snd3::new();
        s.init(32, 256).unwrap();
        assert_eq!(s.init(32, 256).unwrap_err(), errors::ALREADY);
    }

    #[test]
    fn exit_requires_init() {
        let mut s = Snd3::new();
        assert_eq!(s.exit().unwrap_err(), errors::NOTINIT);
    }

    #[test]
    fn exit_clears_state_and_tables() {
        let mut s = Snd3::new();
        s.init(32, 256).unwrap();
        s.exit().unwrap();
        assert_eq!(s.state, Snd3State::Uninitialized);
        assert!(s.voices.is_empty());
    }

    // ---- output mode -------------------------------------------------

    #[test]
    fn output_mode_0_and_1_ok() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        s.set_output_mode(0).unwrap();
        s.set_output_mode(1).unwrap();
    }

    #[test]
    fn output_mode_2_is_einval() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        assert_eq!(s.set_output_mode(2).unwrap_err(), errors::OUTPUTMODE);
    }

    // ---- HD3 sound data ---------------------------------------------

    #[test]
    fn bind_sound_data_returns_fresh_id() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        let id1 = s.bind_sound_data(0x1000).unwrap();
        let id2 = s.bind_sound_data(0x2000).unwrap();
        assert_ne!(id1, id2);
        assert!(id1 > 0 && id2 > 0);
    }

    #[test]
    fn bind_sound_data_full_is_sounddatafull() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        for _ in 0..MAX_HD3 {
            s.bind_sound_data(0).unwrap();
        }
        assert_eq!(s.bind_sound_data(0).unwrap_err(), errors::SOUNDDATAFULL);
    }

    #[test]
    fn unbind_unknown_hd3_is_hd3id() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        assert_eq!(s.unbind_sound_data(99).unwrap_err(), errors::HD3ID);
    }

    #[test]
    fn unbind_valid_hd3() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        let id = s.bind_sound_data(0x1000).unwrap();
        s.unbind_sound_data(id).unwrap();
        assert!(s.sound_data.is_empty());
    }

    // ---- NoteOn / NoteOff -------------------------------------------

    #[test]
    fn note_on_allocates_key_on_id() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        let id1 = s.note_on_by_tone(hd3, 60).unwrap();
        let id2 = s.note_on_by_tone(hd3, 61).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn note_on_unknown_hd3_is_hd3id() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        assert_eq!(s.note_on_by_tone(99, 60).unwrap_err(), errors::HD3ID);
    }

    // ---- Voice ops --------------------------------------------------

    #[test]
    fn voice_oob_is_voicenum() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        assert_eq!(s.voice_set_pitch(99, 0).unwrap_err(), errors::VOICENUM);
    }

    #[test]
    fn voice_set_pitch_stores_value() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        s.voice_set_pitch(2, -500).unwrap();
        assert_eq!(s.voices[2].pitch, -500);
    }

    #[test]
    fn voice_set_velocity_stores_value() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        s.voice_set_velocity(3, 127).unwrap();
        assert_eq!(s.voices[3].velocity, 127);
    }

    #[test]
    fn voice_set_panpot_and_ex() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        s.voice_set_panpot(1, 64).unwrap();
        s.voice_set_panpot_ex(1, 200).unwrap();
        assert_eq!(s.voices[1].panpot, 64);
        assert_eq!(s.voices[1].panpot_ex, 200);
    }

    #[test]
    fn voice_set_pitch_bend_stores_value() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        s.voice_set_pitch_bend(0, 8192).unwrap();
        assert_eq!(s.voices[0].pitch_bend, 8192);
    }

    #[test]
    fn voice_set_sustain_hold_transitions_state() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        s.voice_set_sustain_hold(1, 1).unwrap();
        assert_eq!(s.voices[1].state, VoiceState::SustainHold);
    }

    #[test]
    fn voice_set_reserve_mode_stores() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        s.voice_set_reserve_mode(2, 1).unwrap();
        assert_eq!(s.voices[2].reserve_mode, 1);
    }

    #[test]
    fn voice_note_on_by_tone_transitions_playing() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        s.voice_note_on_by_tone(hd3, 0, 0, 60, 42).unwrap();
        assert_eq!(s.voices[0].state, VoiceState::Playing);
        assert_eq!(s.voices[0].key_on_id, 42);
    }

    #[test]
    fn voice_key_off_releasing() {
        let mut s = Snd3::new();
        s.init(8, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        s.voice_note_on_by_tone(hd3, 1, 0, 60, 99).unwrap();
        s.voice_key_off(1).unwrap();
        assert_eq!(s.voices[1].state, VoiceState::Releasing);
    }

    #[test]
    fn voice_all_key_off_releases_active_voices() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        s.voice_note_on_by_tone(hd3, 0, 0, 60, 1).unwrap();
        s.voice_note_on_by_tone(hd3, 2, 0, 62, 2).unwrap();
        s.voice_all_key_off().unwrap();
        assert_eq!(s.voices[0].state, VoiceState::Releasing);
        assert_eq!(s.voices[2].state, VoiceState::Releasing);
        // Voices that were idle stay idle.
        assert_eq!(s.voices[1].state, VoiceState::Idle);
    }

    #[test]
    fn voice_status_returns_state() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        s.voice_note_on_by_tone(hd3, 0, 0, 60, 1).unwrap();
        assert_eq!(s.voice_status(0), Some(VoiceState::Playing));
        assert_eq!(s.voice_status(1), Some(VoiceState::Idle));
        assert_eq!(s.voice_status(99), None);
    }

    // ---- Effect ------------------------------------------------------

    #[test]
    fn set_effect_type_stores_params() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        s.set_effect_type(3, -10, 100, 50).unwrap();
        assert_eq!(s.effect_type, 3);
        assert_eq!(s.effect_return_vol, -10);
        assert_eq!(s.effect_delay, 100);
        assert_eq!(s.effect_feedback, 50);
    }

    // ---- SMF playback ------------------------------------------------

    #[test]
    fn smf_bind_requires_hd3() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        assert_eq!(s.smf_bind(99).unwrap_err(), errors::HD3ID);
    }

    #[test]
    fn smf_bind_returns_fresh_id() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        let s1 = s.smf_bind(hd3).unwrap();
        let s2 = s.smf_bind(hd3).unwrap();
        assert_ne!(s1, s2);
    }

    #[test]
    fn smf_bind_full_is_smffull() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        for _ in 0..MAX_SMF {
            s.smf_bind(hd3).unwrap();
        }
        assert_eq!(s.smf_bind(hd3).unwrap_err(), errors::SMFFULL);
    }

    #[test]
    fn smf_unbind_unknown_is_smfid() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        assert_eq!(s.smf_unbind(99).unwrap_err(), errors::SMFID);
    }

    #[test]
    fn smf_play_transitions_playing() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        let smf = s.smf_bind(hd3).unwrap();
        s.smf_play(smf, 100, 64, 1).unwrap();
        assert_eq!(s.smf_get_play_status(smf), Some(SmfStatus::Playing));
    }

    #[test]
    fn smf_pause_requires_playing() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        let smf = s.smf_bind(hd3).unwrap();
        assert_eq!(s.smf_pause(smf).unwrap_err(), errors::SMFCTX);
    }

    #[test]
    fn smf_pause_resume_roundtrip() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        let smf = s.smf_bind(hd3).unwrap();
        s.smf_play(smf, 100, 64, 1).unwrap();
        s.smf_pause(smf).unwrap();
        assert_eq!(s.smf_get_play_status(smf), Some(SmfStatus::Paused));
        s.smf_resume(smf).unwrap();
        assert_eq!(s.smf_get_play_status(smf), Some(SmfStatus::Playing));
    }

    #[test]
    fn smf_resume_from_stopped_is_smfctx() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        let smf = s.smf_bind(hd3).unwrap();
        assert_eq!(s.smf_resume(smf).unwrap_err(), errors::SMFCTX);
    }

    #[test]
    fn smf_stop_from_any_state() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        let smf = s.smf_bind(hd3).unwrap();
        s.smf_play(smf, 100, 64, 1).unwrap();
        s.smf_stop(smf).unwrap();
        assert_eq!(s.smf_get_play_status(smf), Some(SmfStatus::Stopped));
    }

    #[test]
    fn smf_add_tempo_accumulates() {
        let mut s = Snd3::new();
        s.init(4, 256).unwrap();
        let hd3 = s.bind_sound_data(0).unwrap();
        let smf = s.smf_bind(hd3).unwrap();
        s.smf_add_tempo(smf, 120).unwrap();
        s.smf_add_tempo(smf, -50).unwrap();
        assert_eq!(s.smf_get_tempo(smf), Some(70));
    }

    // ---- require_init guard -----------------------------------------

    #[test]
    fn voice_ops_without_init_are_notinit() {
        let mut s = Snd3::new();
        assert_eq!(s.voice_set_pitch(0, 0).unwrap_err(), errors::NOTINIT);
        assert_eq!(s.bind_sound_data(0).unwrap_err(), errors::NOTINIT);
    }

    // ---- full smoke --------------------------------------------------

    #[test]
    fn full_libsnd3_lifecycle_smoke() {
        let mut s = Snd3::new();

        // 1. Init for 16 voices.
        s.init(16, 256).unwrap();
        s.set_output_mode(1).unwrap();
        s.set_effect_type(2, -20, 50, 30).unwrap();

        // 2. Bind a sound bank + SMF.
        let hd3 = s.bind_sound_data(0x8000).unwrap();
        let smf = s.smf_bind(hd3).unwrap();

        // 3. Play SMF with pause/resume.
        s.smf_play(smf, 127, 64, 1).unwrap();
        assert_eq!(s.smf_get_play_status(smf), Some(SmfStatus::Playing));
        s.smf_pause(smf).unwrap();
        s.smf_resume(smf).unwrap();
        s.smf_add_tempo(smf, 120).unwrap();
        assert_eq!(s.smf_get_tempo(smf), Some(120));

        // 4. Start a few voices.
        s.voice_note_on_by_tone(hd3, 0, 0, 60, 1).unwrap();
        s.voice_note_on_by_tone(hd3, 5, 0, 64, 2).unwrap();
        s.voice_set_velocity(0, 100).unwrap();
        s.voice_set_panpot(0, 64).unwrap();
        s.voice_set_pitch(5, 200).unwrap();

        // 5. Release voice 0, all-key-off rest.
        s.voice_key_off(0).unwrap();
        assert_eq!(s.voice_status(0), Some(VoiceState::Releasing));
        s.voice_all_key_off().unwrap();
        assert_eq!(s.voice_status(5), Some(VoiceState::Releasing));

        // 6. Stop SMF + unbind.
        s.smf_stop(smf).unwrap();
        s.smf_unbind(smf).unwrap();

        // 7. Unbind HD3 + exit.
        s.unbind_sound_data(hd3).unwrap();
        s.exit().unwrap();
        assert_eq!(s.state, Snd3State::Uninitialized);
    }
}
