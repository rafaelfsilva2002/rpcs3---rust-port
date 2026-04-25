//! `rpcs3-io-recording-config` — Rust port of `rpcs3/Emu/Io/recording_config.cpp` + `.h`.
//!
//! Defaults + bounds for the in-emulator video recorder (ffmpeg-driven).
//! Every numeric field has a clamp range declared in the cpp header via
//! `cfg::uint<min, max>`; we freeze them here as const bounds + a
//! `clamp_*` helper per field so UI or CLI can enforce the same range.
//!
//! Frozen:
//!
//! - Config file name: `recording.yml` (cpp:10).
//! - Video defaults + bounds (cpp header:15..23):
//!   - framerate `[0, 60]` default 30
//!   - width `[640, 7680]` default 1280
//!   - height `[360, 4320]` default 720
//!   - pixel_format `[0, 192]` default 0 (YUV420P)
//!   - video_codec `[0, 0xFFFF]` default 12 (MPEG4)
//!   - video_bps `[1_000_000, 60_000_000]` default 4_000_000
//!   - max_b_frames `[0, 3]` default 2
//!   - gop_size `[1, 120]` default 30
//! - Audio defaults + bounds (cpp header:27..30):
//!   - audio_codec `[0x10000, 0x17000]` default 86018 (AAC)
//!   - audio_bps `[64_000, 320_000]` default 192_000

pub const CONFIG_FILE_NAME: &str = "recording.yml";

// Video bounds + defaults.
pub const FRAMERATE_MIN: u32 = 0;
pub const FRAMERATE_MAX: u32 = 60;
pub const FRAMERATE_DEFAULT: u32 = 30;

pub const WIDTH_MIN: u32 = 640;
pub const WIDTH_MAX: u32 = 7680;
pub const WIDTH_DEFAULT: u32 = 1280;

pub const HEIGHT_MIN: u32 = 360;
pub const HEIGHT_MAX: u32 = 4320;
pub const HEIGHT_DEFAULT: u32 = 720;

pub const PIXEL_FORMAT_MIN: u32 = 0;
pub const PIXEL_FORMAT_MAX: u32 = 192;
/// `AVPixelFormat::AV_PIX_FMT_YUV420P = 0`.
pub const PIXEL_FORMAT_DEFAULT: u32 = 0;

pub const VIDEO_CODEC_MIN: u32 = 0;
pub const VIDEO_CODEC_MAX: u32 = 0xFFFF;
/// `AVCodecID::AV_CODEC_ID_MPEG4 = 12`.
pub const VIDEO_CODEC_DEFAULT: u32 = 12;

pub const VIDEO_BPS_MIN: u32 = 1_000_000;
pub const VIDEO_BPS_MAX: u32 = 60_000_000;
pub const VIDEO_BPS_DEFAULT: u32 = 4_000_000;

pub const MAX_B_FRAMES_MIN: u32 = 0;
pub const MAX_B_FRAMES_MAX: u32 = 3;
pub const MAX_B_FRAMES_DEFAULT: u32 = 2;

pub const GOP_SIZE_MIN: u32 = 1;
pub const GOP_SIZE_MAX: u32 = 120;
pub const GOP_SIZE_DEFAULT: u32 = 30;

// Audio bounds + defaults.
pub const AUDIO_CODEC_MIN: u32 = 0x10000;
pub const AUDIO_CODEC_MAX: u32 = 0x17000;
/// `AVCodecID::AV_CODEC_ID_AAC = 86018 = 0x15002`.
pub const AUDIO_CODEC_DEFAULT: u32 = 86018;

pub const AUDIO_BPS_MIN: u32 = 64_000;
pub const AUDIO_BPS_MAX: u32 = 320_000;
pub const AUDIO_BPS_DEFAULT: u32 = 192_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoConfig {
    pub framerate: u32,
    pub width: u32,
    pub height: u32,
    pub pixel_format: u32,
    pub video_codec: u32,
    pub video_bps: u32,
    pub max_b_frames: u32,
    pub gop_size: u32,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            framerate: FRAMERATE_DEFAULT,
            width: WIDTH_DEFAULT,
            height: HEIGHT_DEFAULT,
            pixel_format: PIXEL_FORMAT_DEFAULT,
            video_codec: VIDEO_CODEC_DEFAULT,
            video_bps: VIDEO_BPS_DEFAULT,
            max_b_frames: MAX_B_FRAMES_DEFAULT,
            gop_size: GOP_SIZE_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioConfig {
    pub audio_codec: u32,
    pub audio_bps: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            audio_codec: AUDIO_CODEC_DEFAULT,
            audio_bps: AUDIO_BPS_DEFAULT,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RecordingConfig {
    pub video: VideoConfig,
    pub audio: AudioConfig,
}

/// Clamp a value into `[min, max]` inclusive.
#[must_use]
pub const fn clamp_u32(v: u32, min: u32, max: u32) -> u32 {
    if v < min { min } else if v > max { max } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_name() {
        assert_eq!(CONFIG_FILE_NAME, "recording.yml");
    }

    #[test]
    fn video_defaults_match_cpp_header() {
        let v = VideoConfig::default();
        assert_eq!(v.framerate, 30);
        assert_eq!(v.width, 1280);
        assert_eq!(v.height, 720);
        assert_eq!(v.pixel_format, 0);
        assert_eq!(v.video_codec, 12);
        assert_eq!(v.video_bps, 4_000_000);
        assert_eq!(v.max_b_frames, 2);
        assert_eq!(v.gop_size, 30);
    }

    #[test]
    fn audio_defaults_match_cpp_header() {
        let a = AudioConfig::default();
        assert_eq!(a.audio_codec, 86018);
        assert_eq!(a.audio_bps, 192_000);
    }

    #[test]
    fn video_bounds_match_cpp_header() {
        assert_eq!(FRAMERATE_MIN, 0);
        assert_eq!(FRAMERATE_MAX, 60);
        assert_eq!(WIDTH_MIN, 640);
        assert_eq!(WIDTH_MAX, 7680);
        assert_eq!(HEIGHT_MIN, 360);
        assert_eq!(HEIGHT_MAX, 4320);
        assert_eq!(PIXEL_FORMAT_MAX, 192);
        assert_eq!(VIDEO_CODEC_MAX, 0xFFFF);
        assert_eq!(VIDEO_BPS_MIN, 1_000_000);
        assert_eq!(VIDEO_BPS_MAX, 60_000_000);
        assert_eq!(MAX_B_FRAMES_MAX, 3);
        assert_eq!(GOP_SIZE_MAX, 120);
    }

    #[test]
    fn audio_bounds_match_cpp_header() {
        assert_eq!(AUDIO_CODEC_MIN, 0x10000);
        assert_eq!(AUDIO_CODEC_MAX, 0x17000);
        assert_eq!(AUDIO_BPS_MIN, 64_000);
        assert_eq!(AUDIO_BPS_MAX, 320_000);
    }

    #[test]
    fn defaults_within_bounds() {
        assert!(FRAMERATE_DEFAULT >= FRAMERATE_MIN && FRAMERATE_DEFAULT <= FRAMERATE_MAX);
        assert!(WIDTH_DEFAULT >= WIDTH_MIN && WIDTH_DEFAULT <= WIDTH_MAX);
        assert!(HEIGHT_DEFAULT >= HEIGHT_MIN && HEIGHT_DEFAULT <= HEIGHT_MAX);
        assert!(VIDEO_CODEC_DEFAULT >= VIDEO_CODEC_MIN && VIDEO_CODEC_DEFAULT <= VIDEO_CODEC_MAX);
        assert!(VIDEO_BPS_DEFAULT >= VIDEO_BPS_MIN && VIDEO_BPS_DEFAULT <= VIDEO_BPS_MAX);
        assert!(GOP_SIZE_DEFAULT >= GOP_SIZE_MIN && GOP_SIZE_DEFAULT <= GOP_SIZE_MAX);
        assert!(AUDIO_CODEC_DEFAULT >= AUDIO_CODEC_MIN && AUDIO_CODEC_DEFAULT <= AUDIO_CODEC_MAX);
        assert!(AUDIO_BPS_DEFAULT >= AUDIO_BPS_MIN && AUDIO_BPS_DEFAULT <= AUDIO_BPS_MAX);
    }

    #[test]
    fn clamp_helper_works() {
        assert_eq!(clamp_u32(500, 640, 7680), 640);
        assert_eq!(clamp_u32(1280, 640, 7680), 1280);
        assert_eq!(clamp_u32(8000, 640, 7680), 7680);
    }

    #[test]
    fn recording_config_default_is_combined() {
        let c = RecordingConfig::default();
        assert_eq!(c.video, VideoConfig::default());
        assert_eq!(c.audio, AudioConfig::default());
    }
}
