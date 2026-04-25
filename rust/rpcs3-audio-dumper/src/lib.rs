//! `rpcs3-audio-dumper` — Rust port of `rpcs3/Emu/Audio/AudioDumper.cpp`.
//!
//! The C++ class streams a live WAV file to disk containing every sample
//! the audio backend produced. Porting the filesystem half is not useful
//! (every frontend already has an `std::fs::File` wrapper), but the WAV
//! layout and the per-frame bookkeeping are worth freezing so dumper
//! output stays bit-identical across frontends.
//!
//! This crate freezes:
//!
//! - The three WAV sub-headers (`RIFF`, `fmt `, `fact`) and the wrapper
//!   `WAVHeader`, including the exact field order, magic-byte ASCII, and
//!   little-endian layout (`AudioDumper.h:7..73`). Size of the whole
//!   header is 56 bytes — asserted at compile time.
//! - `AudioFormat` discriminants: 1 for PCM S16, 3 for IEEE Float
//!   (cpp:37 — `sample_size == FLOAT ? 3 : 1`).
//! - The `WriteData` bookkeeping contract: block size = ch * sample_size,
//!   reject unaligned writes, bump `Size`, `RIFF.Size`, `FACT.SampleLength`
//!   per frame (cpp:51..89).
//! - The `Close` padding quirk: if `Size` is odd, emit a single zero byte
//!   and grow `RIFF.Size` by 1 (cpp:35..48) — WAV chunks must be word-aligned.
//!
//! Actual file I/O and path construction (`audio_{title_id}_{date}.wav`)
//! are host-side concerns and stay out.
use core::mem::size_of;

use rpcs3_audio_resampler::{AudioChannelCnt, AudioFreq, AudioSampleSize};

// AudioFormat discriminants stored in `FMT.AudioFormat` (cpp:37).
pub const WAVE_FORMAT_PCM: u16 = 1;
pub const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;

/// `RIFF` chunk header (`AudioDumper.h:9..21`). Raw bytes on the wire.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RiffHeader {
    pub id: [u8; 4],
    pub size: u32,
    pub wave: [u8; 4],
}

impl RiffHeader {
    #[must_use]
    pub const fn new(size: u32) -> Self {
        Self { id: *b"RIFF", size, wave: *b"WAVE" }
    }
}

impl Default for RiffHeader {
    fn default() -> Self {
        Self::new(0)
    }
}

/// `fmt ` subchunk (`AudioDumper.h:23..45`).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FmtHeader {
    pub id: [u8; 4],
    pub size: u32,
    pub audio_format: u16,
    pub num_channels: u16,
    pub sample_rate: u32,
    pub byte_rate: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
}

impl FmtHeader {
    #[must_use]
    pub fn new(ch: AudioChannelCnt, sample_rate: AudioFreq, sample_size: AudioSampleSize) -> Self {
        let num_channels = ch as u16;
        let sample_rate_u32 = sample_rate as u32;
        let sample_size_u32 = sample_size as u32;
        let bits_per_sample = (sample_size_u32 * 8) as u16;
        let byte_rate = sample_rate_u32 * u32::from(num_channels) * sample_size_u32;
        let block_align = num_channels * sample_size_u32 as u16;
        let audio_format = match sample_size {
            AudioSampleSize::Float => WAVE_FORMAT_IEEE_FLOAT,
            AudioSampleSize::S16 => WAVE_FORMAT_PCM,
        };
        Self {
            id: *b"fmt ",
            size: 16,
            audio_format,
            num_channels,
            sample_rate: sample_rate_u32,
            byte_rate,
            block_align,
            bits_per_sample,
        }
    }
}

impl Default for FmtHeader {
    fn default() -> Self {
        Self {
            id: *b"fmt ",
            size: 16,
            audio_format: 0,
            num_channels: 0,
            sample_rate: 0,
            byte_rate: 0,
            block_align: 0,
            bits_per_sample: 0,
        }
    }
}

/// `fact` subchunk (`AudioDumper.h:47..59`) — total samples per channel.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FactChunk {
    pub id: [u8; 4],
    pub chunk_length: u32,
    pub sample_length: u32,
}

impl FactChunk {
    #[must_use]
    pub const fn new(sample_len: u32) -> Self {
        Self { id: *b"fact", chunk_length: 4, sample_length: sample_len }
    }
}

impl Default for FactChunk {
    fn default() -> Self {
        Self::new(0)
    }
}

/// Full header laid out as it appears on disk (`AudioDumper.h:7..73`).
/// Layout: `RIFF | fmt  | fact | "data" | size`. The leading "data" magic
/// and size live on the wrapper itself (cpp:61..62).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WavHeader {
    pub riff: RiffHeader,
    pub fmt: FmtHeader,
    pub fact: FactChunk,
    pub data_id: [u8; 4],
    pub size: u32,
}

const _: () = assert!(size_of::<RiffHeader>() == 12);
const _: () = assert!(size_of::<FmtHeader>() == 24);
const _: () = assert!(size_of::<FactChunk>() == 12);
const _: () = assert!(size_of::<WavHeader>() == 56);

impl WavHeader {
    #[must_use]
    pub fn new(ch: AudioChannelCnt, sample_rate: AudioFreq, sample_size: AudioSampleSize) -> Self {
        let riff_size = (size_of::<RiffHeader>() + size_of::<FmtHeader>()) as u32;
        Self {
            riff: RiffHeader::new(riff_size),
            fmt: FmtHeader::new(ch, sample_rate, sample_size),
            fact: FactChunk::new(0),
            data_id: *b"data",
            size: 0,
        }
    }

    #[must_use]
    pub fn num_channels(&self) -> u16 {
        self.fmt.num_channels
    }

    /// Bytes per sample (`AudioDumper.h:89` — `BitsPerSample / 8`).
    #[must_use]
    pub fn sample_size_bytes(&self) -> u16 {
        self.fmt.bits_per_sample / 8
    }

    /// Bytes per frame (ch * sample_size). Used to validate `WriteData`
    /// alignment (cpp:55..58).
    #[must_use]
    pub fn block_size(&self) -> u32 {
        u32::from(self.fmt.num_channels) * u32::from(self.sample_size_bytes())
    }
}

impl Default for WavHeader {
    fn default() -> Self {
        Self {
            riff: RiffHeader::default(),
            fmt: FmtHeader::default(),
            fact: FactChunk::default(),
            data_id: *b"data",
            size: 0,
        }
    }
}

/// Outcome of a `write_data` call. `Misaligned` mirrors the cpp:58 `ensure`
/// that a whole number of frames was passed; instead of aborting we surface
/// it to the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Wrote `bytes` bytes / `frames` frames to the data chunk.
    Wrote { bytes: u32, frames: u32 },
    /// The buffer size was not a multiple of `block_size()`.
    Misaligned { size: u32, block_size: u32 },
    /// No-op: dumper was closed (num_channels == 0).
    Closed,
    /// No-op: size == 0.
    Empty,
}

/// Bookkeeping-only dumper. Wraps a `WavHeader` and updates the size
/// counters as the caller reports writes; the actual bytes-to-disk step is
/// the caller's responsibility because `fs::file` is platform-specific.
pub struct AudioDumper {
    pub header: WavHeader,
    pub padded: bool,
}

impl AudioDumper {
    #[must_use]
    pub const fn new() -> Self {
        Self { header: WavHeader { riff: RiffHeader { id: *b"RIFF", size: 0, wave: *b"WAVE" },
                                   fmt: FmtHeader { id: *b"fmt ", size: 16, audio_format: 0,
                                                    num_channels: 0, sample_rate: 0, byte_rate: 0,
                                                    block_align: 0, bits_per_sample: 0 },
                                   fact: FactChunk { id: *b"fact", chunk_length: 4, sample_length: 0 },
                                   data_id: *b"data", size: 0 },
               padded: false }
    }

    /// `Open(ch, sample_rate, sample_size)` (cpp:18..31) — installs a fresh
    /// WAV header. `Close()` must be called before reopening.
    pub fn open(&mut self, ch: AudioChannelCnt, sample_rate: AudioFreq, sample_size: AudioSampleSize) {
        self.close();
        self.header = WavHeader::new(ch, sample_rate, sample_size);
        self.padded = false;
    }

    /// `Close()` (cpp:33..49) — emits WAV's word-alignment padding when
    /// data `size` is odd (RIFF.Size gains 1), then clears `num_channels`
    /// so further writes no-op. Returns whether a pad byte is needed.
    pub fn close(&mut self) -> bool {
        let need_pad = self.header.num_channels() != 0 && (self.header.size & 1) == 1;
        if need_pad {
            self.header.riff.size = self.header.riff.size.saturating_add(1);
            self.padded = true;
        }
        if self.header.num_channels() != 0 {
            self.header.fmt.num_channels = 0;
        }
        need_pad
    }

    /// `WriteData(buffer, size)` bookkeeping half (cpp:51..89). Bails out
    /// on closed/empty/misaligned before updating any counter, then bumps
    /// `Size`, `RIFF.Size`, `FACT.SampleLength` in lockstep.
    pub fn write_data(&mut self, size: u32) -> WriteOutcome {
        if self.header.num_channels() == 0 {
            return WriteOutcome::Closed;
        }
        if size == 0 {
            return WriteOutcome::Empty;
        }
        let block = self.header.block_size();
        if block == 0 || size % block != 0 {
            return WriteOutcome::Misaligned { size, block_size: block };
        }
        let frames = size / block;
        self.header.size = self.header.size.saturating_add(size);
        self.header.riff.size = self.header.riff.size.saturating_add(size);
        self.header.fact.sample_length = self.header.fact.sample_length.saturating_add(frames);
        WriteOutcome::Wrote { bytes: size, frames }
    }
}

impl Default for AudioDumper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_sizes_frozen() {
        assert_eq!(size_of::<RiffHeader>(), 12);
        assert_eq!(size_of::<FmtHeader>(), 24);
        assert_eq!(size_of::<FactChunk>(), 12);
        assert_eq!(size_of::<WavHeader>(), 56);
    }

    #[test]
    fn riff_and_data_magic_bytes() {
        let h = WavHeader::new(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::Float);
        assert_eq!(&h.riff.id, b"RIFF");
        assert_eq!(&h.riff.wave, b"WAVE");
        assert_eq!(&h.fmt.id, b"fmt ");
        assert_eq!(&h.fact.id, b"fact");
        assert_eq!(&h.data_id, b"data");
    }

    #[test]
    fn fmt_header_for_float_stereo_48k() {
        let h = FmtHeader::new(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::Float);
        assert_eq!(h.audio_format, WAVE_FORMAT_IEEE_FLOAT);
        assert_eq!(h.num_channels, 2);
        assert_eq!(h.sample_rate, 48_000);
        assert_eq!(h.bits_per_sample, 32);
        assert_eq!(h.block_align, 2 * 4);
        assert_eq!(h.byte_rate, 48_000 * 2 * 4);
        assert_eq!(h.size, 16);
    }

    #[test]
    fn fmt_header_for_s16_surround_7_1() {
        let h = FmtHeader::new(AudioChannelCnt::Surround7_1, AudioFreq::Freq96K, AudioSampleSize::S16);
        assert_eq!(h.audio_format, WAVE_FORMAT_PCM);
        assert_eq!(h.num_channels, 8);
        assert_eq!(h.sample_rate, 96_000);
        assert_eq!(h.bits_per_sample, 16);
        assert_eq!(h.block_align, 8 * 2);
        assert_eq!(h.byte_rate, 96_000 * 8 * 2);
    }

    #[test]
    fn wav_header_initial_riff_size_is_riff_plus_fmt() {
        let h = WavHeader::new(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::Float);
        // 12 + 24 = 36 per cpp:67 (sizeof(RIFFHeader) + sizeof(FMTHeader)).
        assert_eq!(h.riff.size, 36);
        assert_eq!(h.size, 0);
        assert_eq!(h.fact.sample_length, 0);
    }

    #[test]
    fn open_resets_header() {
        let mut d = AudioDumper::new();
        d.open(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::Float);
        assert_eq!(d.header.num_channels(), 2);
        assert_eq!(d.header.sample_size_bytes(), 4);
        assert_eq!(d.header.block_size(), 8);
    }

    #[test]
    fn write_data_aligned_bumps_counters() {
        let mut d = AudioDumper::new();
        d.open(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::Float);
        // 10 frames of stereo float = 10 * 8 bytes = 80.
        match d.write_data(80) {
            WriteOutcome::Wrote { bytes, frames } => {
                assert_eq!(bytes, 80);
                assert_eq!(frames, 10);
            }
            other => panic!("unexpected {other:?}"),
        }
        assert_eq!(d.header.size, 80);
        assert_eq!(d.header.riff.size, 36 + 80);
        assert_eq!(d.header.fact.sample_length, 10);
    }

    #[test]
    fn write_data_misaligned_rejected_before_update() {
        let mut d = AudioDumper::new();
        d.open(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::Float);
        match d.write_data(7) {
            WriteOutcome::Misaligned { size, block_size } => {
                assert_eq!(size, 7);
                assert_eq!(block_size, 8);
            }
            other => panic!("unexpected {other:?}"),
        }
        // Nothing changed.
        assert_eq!(d.header.size, 0);
        assert_eq!(d.header.fact.sample_length, 0);
    }

    #[test]
    fn write_data_before_open_is_closed_noop() {
        let mut d = AudioDumper::new();
        assert_eq!(d.write_data(8), WriteOutcome::Closed);
    }

    #[test]
    fn write_data_empty_is_noop() {
        let mut d = AudioDumper::new();
        d.open(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::S16);
        assert_eq!(d.write_data(0), WriteOutcome::Empty);
    }

    #[test]
    fn close_pads_when_size_is_odd_and_clears_channels() {
        let mut d = AudioDumper::new();
        d.open(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::S16);
        // Force an odd size by smuggling one frame of a hypothetical mono
        // recording (block_size == 4) then override.
        d.header.size = 5;
        d.header.riff.size = 36 + 5;
        assert!(d.close(), "odd size → pad byte emitted");
        assert_eq!(d.header.riff.size, 36 + 5 + 1);
        assert_eq!(d.header.num_channels(), 0, "close clears channel count");
        assert!(d.padded);
    }

    #[test]
    fn close_no_pad_when_size_is_even() {
        let mut d = AudioDumper::new();
        d.open(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::Float);
        d.write_data(80).unwrap_wrote();
        assert!(!d.close());
        assert_eq!(d.header.num_channels(), 0);
        assert!(!d.padded);
    }

    #[test]
    fn close_noop_when_already_closed() {
        let mut d = AudioDumper::new();
        // num_channels == 0 at construction → close is a no-op.
        assert!(!d.close());
        assert_eq!(d.header.num_channels(), 0);
    }

    #[test]
    fn open_after_partial_write_resets_cleanly() {
        let mut d = AudioDumper::new();
        d.open(AudioChannelCnt::Stereo, AudioFreq::Freq48K, AudioSampleSize::Float);
        d.write_data(80).unwrap_wrote();
        d.open(AudioChannelCnt::Surround5_1, AudioFreq::Freq96K, AudioSampleSize::S16);
        assert_eq!(d.header.num_channels(), 6);
        assert_eq!(d.header.sample_rate(), 96_000);
        assert_eq!(d.header.size, 0);
        assert_eq!(d.header.fact.sample_length, 0);
    }

    // Tiny ergonomic helpers for tests.
    impl WriteOutcome {
        fn unwrap_wrote(self) {
            match self {
                WriteOutcome::Wrote { .. } => {}
                other => panic!("expected Wrote, got {other:?}"),
            }
        }
    }

    impl WavHeader {
        fn sample_rate(&self) -> u32 {
            self.fmt.sample_rate
        }
    }
}
