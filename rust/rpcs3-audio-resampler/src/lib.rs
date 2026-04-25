//! `rpcs3-audio-resampler` — Rust port of `rpcs3/Emu/Audio/audio_resampler.cpp`.
//!
//! The C++ class is a thin wrapper around SoundTouch. Porting SoundTouch
//! itself is a separate project (15k+ LOC of anti-aliased resampling in
//! C++). What we freeze here is the RPCS3-specific contract surface:
//!
//! - `AudioFreq` / `AudioChannelCnt` / `AudioSampleSize` / `AudioStateEvent`
//!   enums from `AudioBackend.h:18..47`, byte-exact discriminants.
//! - Defaults from `AudioBackend.h:10..16` (DEFAULT_AUDIO_SAMPLING_RATE
//!   et al).
//! - Tempo bounds `RESAMPLER_MIN_FREQ_VAL = 0.1` and `RESAMPLER_MAX_FREQ_VAL
//!   = 1.0` (resampler.h:16..17).
//! - The quality-knob settings the ctor writes (cpp:8..12) —
//!   SEQUENCE_MS=40, SEEKWINDOW_MS=15, OVERLAP_MS=8, USE_QUICKSEEK=0,
//!   USE_AA_FILTER=1. Keeping these in source lets future audits diff the
//!   Rust binding against the C++ tuning without chasing a separate file.
//! - An `AudioResamplerState` struct that tracks channels / freq / tempo /
//!   sample counts / flushed flag, wired so wrappers can implement
//!   `set_params` / `set_tempo` / `flush` without the SoundTouch engine
//!   present.
// AudioBackend.h:10..16 — integer defaults that hot-path code reads.
pub const DEFAULT_AUDIO_SAMPLING_RATE: u32 = 48_000;
pub const MAX_AUDIO_BUFFERS: u32 = 64;
pub const AUDIO_BUFFER_SAMPLES: u32 = 256;
pub const AUDIO_MAX_CHANNELS: u32 = 8;

/// Resampler tempo bounds (resampler.h:16..17). `set_tempo(x)` clamps into
/// `[MIN, MAX]` and returns the value actually applied.
pub const RESAMPLER_MIN_FREQ_VAL: f64 = 0.1;
pub const RESAMPLER_MAX_FREQ_VAL: f64 = 1.0;

// SoundTouch quality-knob settings baked by the RPCS3 constructor.
// Preserving them as named constants so changes to the tuning are visible
// in diffs (cpp:8..12).
pub const SOUNDTOUCH_SEQUENCE_MS: i32 = 40;
pub const SOUNDTOUCH_SEEKWINDOW_MS: i32 = 15;
pub const SOUNDTOUCH_OVERLAP_MS: i32 = 8;
pub const SOUNDTOUCH_USE_QUICKSEEK: i32 = 0;
pub const SOUNDTOUCH_USE_AA_FILTER: i32 = 1;

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFreq {
    Freq32K = 32_000,
    Freq44K = 44_100,
    Freq48K = 48_000,
    Freq88K = 88_200,
    Freq96K = 96_000,
    Freq176K = 176_400,
    Freq192K = 192_000,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSampleSize {
    Float = 4,
    S16 = 2,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioChannelCnt {
    Stereo = 2,
    Surround5_1 = 6,
    Surround7_1 = 8,
}

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioStateEvent {
    UnspecifiedError = 0,
    DefaultDeviceMaybeChanged = 1,
}

/// Backend-agnostic mirror of `audio_resampler` member state.
///
/// A real implementation would own a SoundTouch instance; this struct lets
/// non-SoundTouch drivers (e.g. a WGPU audio pipe, a Null backend, tests)
/// carry the same contract shape. `buffered_samples` is maintained by the
/// caller — `put_samples` adds, `get_samples` removes, `flush` zeroes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioResamplerState {
    pub channels: AudioChannelCnt,
    pub freq: AudioFreq,
    pub tempo: f64,
    pub buffered_samples: u32,
}

impl AudioResamplerState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            channels: AudioChannelCnt::Stereo,
            freq: AudioFreq::Freq48K,
            tempo: 1.0,
            buffered_samples: 0,
        }
    }

    /// `audio_resampler::set_params(ch_cnt, freq)` — flushes then swaps
    /// (cpp:19..24). Returns the prior `(channels, freq)`.
    pub fn set_params(
        &mut self,
        ch_cnt: AudioChannelCnt,
        freq: AudioFreq,
    ) -> (AudioChannelCnt, AudioFreq) {
        self.flush();
        let prev = (self.channels, self.freq);
        self.channels = ch_cnt;
        self.freq = freq;
        prev
    }

    /// `audio_resampler::set_tempo(new_tempo)` (cpp:26..31). Clamps into
    /// `[RESAMPLER_MIN_FREQ_VAL, RESAMPLER_MAX_FREQ_VAL]` and returns the
    /// value that actually got applied.
    pub fn set_tempo(&mut self, new_tempo: f64) -> f64 {
        let clamped = new_tempo.clamp(RESAMPLER_MIN_FREQ_VAL, RESAMPLER_MAX_FREQ_VAL);
        self.tempo = clamped;
        clamped
    }

    /// Getter helper mirroring `samples_available()` (cpp:46..49).
    #[must_use]
    pub fn samples_available(&self) -> u32 {
        self.buffered_samples
    }

    /// The core input/output ratio SoundTouch would compute — here we
    /// expose it from the state so callers can drive buffer math without
    /// a live SoundTouch instance. Equal to `tempo` (input sr / output sr
    /// after time-stretch = tempo at unit pitch).
    #[must_use]
    pub fn resample_ratio(&self) -> f64 {
        self.tempo
    }

    /// `audio_resampler::flush()` (cpp:56..59).
    pub fn flush(&mut self) {
        self.buffered_samples = 0;
    }

    /// Mirror `put_samples` bookkeeping: record that `sample_cnt` more
    /// samples are queued (engine itself is owned by the wrapper).
    pub fn put_samples(&mut self, sample_cnt: u32) {
        self.buffered_samples = self.buffered_samples.saturating_add(sample_cnt);
    }

    /// Mirror `get_samples` — consume up to `sample_cnt` samples, return
    /// how many were actually delivered.
    pub fn take_samples(&mut self, sample_cnt: u32) -> u32 {
        let taken = sample_cnt.min(self.buffered_samples);
        self.buffered_samples -= taken;
        taken
    }
}

impl Default for AudioResamplerState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_freq_discriminants() {
        assert_eq!(AudioFreq::Freq32K as u32, 32_000);
        assert_eq!(AudioFreq::Freq44K as u32, 44_100);
        assert_eq!(AudioFreq::Freq48K as u32, 48_000);
        assert_eq!(AudioFreq::Freq88K as u32, 88_200);
        assert_eq!(AudioFreq::Freq96K as u32, 96_000);
        assert_eq!(AudioFreq::Freq176K as u32, 176_400);
        assert_eq!(AudioFreq::Freq192K as u32, 192_000);
    }

    #[test]
    fn audio_channel_cnt_discriminants() {
        assert_eq!(AudioChannelCnt::Stereo as u32, 2);
        assert_eq!(AudioChannelCnt::Surround5_1 as u32, 6);
        assert_eq!(AudioChannelCnt::Surround7_1 as u32, 8);
    }

    #[test]
    fn sample_size_discriminants() {
        assert_eq!(AudioSampleSize::Float as u32, 4);
        assert_eq!(AudioSampleSize::S16 as u32, 2);
    }

    #[test]
    fn state_event_order_frozen() {
        assert_eq!(AudioStateEvent::UnspecifiedError as u32, 0);
        assert_eq!(AudioStateEvent::DefaultDeviceMaybeChanged as u32, 1);
    }

    #[test]
    fn default_audio_constants() {
        assert_eq!(DEFAULT_AUDIO_SAMPLING_RATE, 48_000);
        assert_eq!(MAX_AUDIO_BUFFERS, 64);
        assert_eq!(AUDIO_BUFFER_SAMPLES, 256);
        assert_eq!(AUDIO_MAX_CHANNELS, 8);
    }

    #[test]
    fn soundtouch_settings_frozen() {
        assert_eq!(SOUNDTOUCH_SEQUENCE_MS, 40);
        assert_eq!(SOUNDTOUCH_SEEKWINDOW_MS, 15);
        assert_eq!(SOUNDTOUCH_OVERLAP_MS, 8);
        assert_eq!(SOUNDTOUCH_USE_QUICKSEEK, 0);
        assert_eq!(SOUNDTOUCH_USE_AA_FILTER, 1);
    }

    #[test]
    fn tempo_bounds() {
        assert_eq!(RESAMPLER_MIN_FREQ_VAL, 0.1);
        assert_eq!(RESAMPLER_MAX_FREQ_VAL, 1.0);
    }

    #[test]
    fn set_tempo_clamps_and_returns_applied() {
        let mut s = AudioResamplerState::new();
        assert_eq!(s.set_tempo(0.5), 0.5);
        assert_eq!(s.tempo, 0.5);
        assert_eq!(s.set_tempo(2.0), 1.0);
        assert_eq!(s.tempo, 1.0);
        assert_eq!(s.set_tempo(-0.5), 0.1);
        assert_eq!(s.tempo, 0.1);
    }

    #[test]
    fn set_params_flushes_and_swaps() {
        let mut s = AudioResamplerState::new();
        s.put_samples(1024);
        assert_eq!(s.samples_available(), 1024);
        let prev = s.set_params(AudioChannelCnt::Surround7_1, AudioFreq::Freq96K);
        assert_eq!(prev, (AudioChannelCnt::Stereo, AudioFreq::Freq48K));
        assert_eq!(s.samples_available(), 0, "params change flushes buffer");
        assert_eq!(s.channels, AudioChannelCnt::Surround7_1);
        assert_eq!(s.freq, AudioFreq::Freq96K);
    }

    #[test]
    fn put_and_take_samples_bookkeeping() {
        let mut s = AudioResamplerState::new();
        s.put_samples(500);
        s.put_samples(500);
        assert_eq!(s.samples_available(), 1000);
        assert_eq!(s.take_samples(300), 300);
        assert_eq!(s.samples_available(), 700);
        // Asking for more than available returns what's there.
        assert_eq!(s.take_samples(2000), 700);
        assert_eq!(s.samples_available(), 0);
    }

    #[test]
    fn flush_zeros_buffer() {
        let mut s = AudioResamplerState::new();
        s.put_samples(123);
        s.flush();
        assert_eq!(s.samples_available(), 0);
    }

    #[test]
    fn resample_ratio_tracks_tempo() {
        let mut s = AudioResamplerState::new();
        s.set_tempo(0.75);
        assert_eq!(s.resample_ratio(), 0.75);
    }
}
