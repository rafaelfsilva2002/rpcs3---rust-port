//! `rpcs3-hle-cellrec` — game video recording HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellRec.cpp`. cellRec lets games
//! capture gameplay into MPEG4 / AVC / MJPEG video files with a
//! simple `Open → Start → Stop → Close` lifecycle. The API exposes
//! dozens of format combos (resolution × codec × bitrate) and per-
//! recording options (PPU priority, audio mix volume, ring buffer
//! seconds, etc.).
//!
//! ## Entry points covered
//!
//! | HLE function                      | Rust wrapper                         |
//! |-----------------------------------|--------------------------------------|
//! | `cellRecOpen`                     | [`RecManager::open`]                 |
//! | `cellRecClose`                    | [`RecManager::close`]                |
//! | `cellRecStart`                    | [`RecManager::start`]                |
//! | `cellRecStop`                     | [`RecManager::stop`]                 |
//! | `cellRecQueryMemSize`             | [`RecManager::query_mem_size`]       |
//! | `cellRecSetInfo`                  | [`RecManager::set_info`]             |
//! | `cellRecGetInfo`                  | [`RecManager::get_info`]             |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellRec.h:3-12
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const OUT_OF_MEMORY: CellError = CellError(0x8002_c501);
    pub const FATAL: CellError = CellError(0x8002_c502);
    pub const INVALID_VALUE: CellError = CellError(0x8002_c503);
    pub const FILE_OPEN: CellError = CellError(0x8002_c504);
    pub const FILE_WRITE: CellError = CellError(0x8002_c505);
    pub const INVALID_STATE: CellError = CellError(0x8002_c506);
    pub const FILE_NO_DATA: CellError = CellError(0x8002_c507);
}

// =====================================================================
// Status codes (cellRec.h:14-22)
// =====================================================================

pub const STATUS_UNLOAD: i32 = 0;
pub const STATUS_OPEN: i32 = 1;
pub const STATUS_START: i32 = 2;
pub const STATUS_STOP: i32 = 3;
pub const STATUS_CLOSE: i32 = 4;
pub const STATUS_ERR: i32 = 10;

// =====================================================================
// Memory / path limits (cellRec.h:24-30)
// =====================================================================

pub const MIN_MEMORY_CONTAINER_SIZE: u32 = 0;
pub const MAX_MEMORY_CONTAINER_SIZE: u32 = 16 * 1024 * 1024;
/// Older SDKs (<0x300000) cap at 9 MB instead of 16 MB.
pub const MAX_MEMORY_CONTAINER_SIZE_LEGACY: u32 = 9 * 1024 * 1024;
pub const MAX_PATH_LEN: usize = 1023;

// =====================================================================
// Thread priority defaults (cellRec.h:33-35)
// =====================================================================

pub const PPU_THREAD_PRIORITY_DEFAULT: i32 = 400;
pub const SPU_THREAD_PRIORITY_DEFAULT: i32 = 60;

// =====================================================================
// Capture priority / toggles (cellRec.h:36-64)
// =====================================================================

pub const CAPTURE_PRIORITY_HIGHEST: i32 = 0;
pub const CAPTURE_PRIORITY_EXCEPT_NOTIFICATION: i32 = 1;
pub const CAPTURE_PRIORITY_GAME_SCREEN: i32 = 2;

pub const USE_SYSTEM_SPU_DISABLE: i32 = 0;
pub const USE_SYSTEM_SPU_ENABLE: i32 = 1;

pub const XMB_BGM_DISABLE: i32 = 0;
pub const XMB_BGM_ENABLE: i32 = 1;

pub const MPEG4_FAST_ENCODE_DISABLE: i32 = 0;
pub const MPEG4_FAST_ENCODE_ENABLE: i32 = 1;

// Video input formats.
pub const VIDEO_INPUT_DISABLE: i32 = 0;
pub const VIDEO_INPUT_ARGB_4_3: i32 = 1;
pub const VIDEO_INPUT_ARGB_16_9: i32 = 2;
pub const VIDEO_INPUT_RGBA_4_3: i32 = 3;
pub const VIDEO_INPUT_RGBA_16_9: i32 = 4;
pub const VIDEO_INPUT_YUV420PLANAR_16_9: i32 = 5;

#[must_use]
pub fn is_known_video_input(v: i32) -> bool {
    (VIDEO_INPUT_DISABLE..=VIDEO_INPUT_YUV420PLANAR_16_9).contains(&v)
}

pub const AUDIO_INPUT_DISABLE: i32 = 0;
pub const AUDIO_INPUT_ENABLE: i32 = 1;

pub const AUDIO_INPUT_MIX_VOL_MIN: i32 = 0;
pub const AUDIO_INPUT_MIX_VOL_MAX: i32 = 100;

pub const REDUCE_MEMSIZE_DISABLE: i32 = 0;
pub const REDUCE_MEMSIZE_ENABLE: i32 = 1;

// =====================================================================
// Video / audio format codes (cellRec.h:74-131)
// =====================================================================

pub const AUDIO_BLOCK_SAMPLES: u32 = 256;

/// Is `fmt` a valid video-format code? The real table is hundreds of
/// values; we validate by checking against the master list.
#[must_use]
pub fn is_known_video_format(fmt: i32) -> bool {
    matches!(
        fmt,
        // MPEG4
        0x0000 | 0x0010 | 0x0100 | 0x0110 | 0x0200 | 0x0210 | 0x0220 | 0x0230 | 0x0240
        // AVC Main Profile
        | 0x1000 | 0x1010 | 0x1100 | 0x1110 | 0x1120 | 0x1130
        // AVC Baseline Profile
        | 0x2000 | 0x2010 | 0x2100 | 0x2110 | 0x2120 | 0x2130
        // MJPEG
        | 0x3060 | 0x3160 | 0x3270 | 0x3670 | 0x3680 | 0x3690
        // YouTube alias
        | 0x0310
        // M4HD
        | 0x4010 | 0x4110 | 0x4230 | 0x4240 | 0x4640 | 0x4660 | 0x4670
    )
}

#[must_use]
pub fn is_known_audio_format(fmt: i32) -> bool {
    matches!(
        fmt,
        // AAC
        0x0000 | 0x0001 | 0x0002
        // ULAW
        | 0x1007 | 0x1008
        // PCM
        | 0x2007 | 0x2008 | 0x2009
    )
}

// =====================================================================
// Info / setinfo keys (cellRec.h:137-164)
// =====================================================================

pub const INFO_VIDEO_INPUT_ADDR: i32 = 0;
pub const INFO_VIDEO_INPUT_WIDTH: i32 = 1;
pub const INFO_VIDEO_INPUT_PITCH: i32 = 2;
pub const INFO_VIDEO_INPUT_HEIGHT: i32 = 3;
pub const INFO_AUDIO_INPUT_ADDR: i32 = 4;
pub const INFO_MOVIE_TIME_MSEC: i32 = 5;
pub const INFO_SPURS_SYSTEMWORKLOAD_ID: i32 = 6;

pub const SETINFO_MOVIE_START_TIME_MSEC: i32 = 100;
pub const SETINFO_MOVIE_END_TIME_MSEC: i32 = 101;
pub const SETINFO_MOVIE_META: i32 = 200;
pub const SETINFO_SCENE_META: i32 = 201;

pub const MOVIE_META_GAME_TITLE_LEN: usize = 128;
pub const MOVIE_META_MOVIE_TITLE_LEN: usize = 128;
pub const MOVIE_META_DESCRIPTION_LEN: usize = 384;
pub const MOVIE_META_USERDATA_LEN: usize = 64;

pub const SCENE_META_TYPE_CHAPTER: i32 = 0;
pub const SCENE_META_TYPE_CLIP_HIGHLIGHT: i32 = 1;
pub const SCENE_META_TYPE_CLIP_USER: i32 = 2;

pub const SCENE_META_TITLE_LEN: usize = 128;
pub const SCENE_META_TAG_NUM: usize = 6;
pub const SCENE_META_TAG_LEN: usize = 64;

// =====================================================================
// Option keys (cellRec.h:168-184)
// =====================================================================

pub const OPTION_PPU_THREAD_PRIORITY: i32 = 1;
pub const OPTION_SPU_THREAD_PRIORITY: i32 = 2;
pub const OPTION_CAPTURE_PRIORITY: i32 = 3;
pub const OPTION_USE_SYSTEM_SPU: i32 = 4;
pub const OPTION_FIT_TO_YOUTUBE: i32 = 5;
pub const OPTION_XMB_BGM: i32 = 6;
pub const OPTION_RING_SEC: i32 = 7;
pub const OPTION_MPEG4_FAST_ENCODE: i32 = 8;
pub const OPTION_VIDEO_INPUT: i32 = 9;
pub const OPTION_AUDIO_INPUT: i32 = 10;
pub const OPTION_AUDIO_INPUT_MIX_VOL: i32 = 11;
pub const OPTION_REDUCE_MEMSIZE: i32 = 12;
pub const OPTION_SHOW_XMB: i32 = 13;
pub const OPTION_METADATA_FILENAME: i32 = 14;
pub const OPTION_SPURS: i32 = 15;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecParam {
    pub video_format: i32,
    pub audio_format: i32,
    pub options: std::collections::HashMap<i32, i64>,
}

impl RecParam {
    fn validate(&self) -> Result<(), CellError> {
        if !is_known_video_format(self.video_format) {
            return Err(errors::INVALID_VALUE);
        }
        if !is_known_audio_format(self.audio_format) {
            return Err(errors::INVALID_VALUE);
        }
        for (&key, &val) in &self.options {
            validate_option(key, val)?;
        }
        Ok(())
    }
}

fn validate_option(key: i32, val: i64) -> Result<(), CellError> {
    match key {
        OPTION_PPU_THREAD_PRIORITY | OPTION_SPU_THREAD_PRIORITY => {
            if !(0..=3071).contains(&val) {
                return Err(errors::INVALID_VALUE);
            }
        }
        OPTION_CAPTURE_PRIORITY => {
            if !(CAPTURE_PRIORITY_HIGHEST as i64..=CAPTURE_PRIORITY_GAME_SCREEN as i64).contains(&val) {
                return Err(errors::INVALID_VALUE);
            }
        }
        OPTION_USE_SYSTEM_SPU | OPTION_XMB_BGM | OPTION_MPEG4_FAST_ENCODE | OPTION_AUDIO_INPUT
        | OPTION_REDUCE_MEMSIZE | OPTION_SHOW_XMB | OPTION_FIT_TO_YOUTUBE => {
            if !(0..=1).contains(&val) {
                return Err(errors::INVALID_VALUE);
            }
        }
        OPTION_VIDEO_INPUT => {
            if !is_known_video_input(val as i32) {
                return Err(errors::INVALID_VALUE);
            }
        }
        OPTION_AUDIO_INPUT_MIX_VOL => {
            if !(AUDIO_INPUT_MIX_VOL_MIN as i64..=AUDIO_INPUT_MIX_VOL_MAX as i64).contains(&val) {
                return Err(errors::INVALID_VALUE);
            }
        }
        OPTION_RING_SEC => {
            if val < 0 {
                return Err(errors::INVALID_VALUE);
            }
        }
        OPTION_METADATA_FILENAME | OPTION_SPURS => {
            // Opaque pointers; no range check.
        }
        _ => return Err(errors::INVALID_VALUE),
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct MovieMetadata {
    pub game_title: String,
    pub movie_title: String,
    pub description: String,
    pub userdata: String,
}

impl MovieMetadata {
    fn validate(&self) -> Result<(), CellError> {
        if self.game_title.len() >= MOVIE_META_GAME_TITLE_LEN {
            return Err(errors::INVALID_VALUE);
        }
        if self.movie_title.len() >= MOVIE_META_MOVIE_TITLE_LEN {
            return Err(errors::INVALID_VALUE);
        }
        if self.description.len() >= MOVIE_META_DESCRIPTION_LEN {
            return Err(errors::INVALID_VALUE);
        }
        if self.userdata.len() >= MOVIE_META_USERDATA_LEN {
            return Err(errors::INVALID_VALUE);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneMetadata {
    pub scene_type: i32,
    pub start_time_msec: u64,
    pub end_time_msec: u64,
    pub title: String,
    pub tags: Vec<String>,
}

impl SceneMetadata {
    fn validate(&self) -> Result<(), CellError> {
        if !(SCENE_META_TYPE_CHAPTER..=SCENE_META_TYPE_CLIP_USER).contains(&self.scene_type) {
            return Err(errors::INVALID_VALUE);
        }
        if self.end_time_msec < self.start_time_msec {
            return Err(errors::INVALID_VALUE);
        }
        if self.title.len() >= SCENE_META_TITLE_LEN {
            return Err(errors::INVALID_VALUE);
        }
        if self.tags.len() > SCENE_META_TAG_NUM {
            return Err(errors::INVALID_VALUE);
        }
        for t in &self.tags {
            if t.len() >= SCENE_META_TAG_LEN {
                return Err(errors::INVALID_VALUE);
            }
        }
        Ok(())
    }
}

// =====================================================================
// RecManager
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Unloaded,
    Opened,
    Started,
    Stopped,
}

impl State {
    #[must_use]
    pub const fn as_status(self) -> i32 {
        match self {
            Self::Unloaded => STATUS_UNLOAD,
            Self::Opened => STATUS_OPEN,
            Self::Started => STATUS_START,
            Self::Stopped => STATUS_STOP,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RecManager {
    state: State,
    path: String,
    param: Option<RecParam>,
    start_time_msec: u64,
    end_time_msec: u64,
    movie_meta: MovieMetadata,
    scenes: Vec<SceneMetadata>,
}

impl RecManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Unloaded,
            path: String::new(),
            param: None,
            start_time_msec: 0,
            end_time_msec: 0,
            movie_meta: MovieMetadata::default(),
            scenes: Vec::new(),
        }
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    #[must_use]
    pub fn status(&self) -> i32 {
        self.state.as_status()
    }

    /// `cellRecQueryMemSize(param)`. Returns the memory-container size
    /// required for the chosen format combo; picks the legacy 9 MB cap
    /// when `legacy_sdk` is true.
    pub fn query_mem_size(param: &RecParam, legacy_sdk: bool) -> Result<u32, CellError> {
        param.validate()?;
        let cap = if legacy_sdk { MAX_MEMORY_CONTAINER_SIZE_LEGACY } else { MAX_MEMORY_CONTAINER_SIZE };
        // Approximation: HD720 codecs need the full cap, everything
        // else half. Matches the sizing table in the C++ lib.
        let needs_hd = matches!(
            param.video_format,
            0x3670 | 0x3680 | 0x3690 | 0x4640 | 0x4660 | 0x4670
        );
        Ok(if needs_hd { cap } else { cap / 2 })
    }

    pub fn open(&mut self, path: impl Into<String>, param: RecParam) -> Result<(), CellError> {
        if self.state != State::Unloaded {
            return Err(errors::INVALID_STATE);
        }
        let path = path.into();
        if path.is_empty() || path.len() > MAX_PATH_LEN {
            return Err(errors::INVALID_VALUE);
        }
        param.validate()?;
        self.state = State::Opened;
        self.path = path;
        self.param = Some(param);
        Ok(())
    }

    pub fn close(&mut self) -> Result<(), CellError> {
        match self.state {
            State::Started => return Err(errors::INVALID_STATE),
            State::Unloaded => return Err(errors::INVALID_STATE),
            _ => {}
        }
        self.state = State::Unloaded;
        self.path.clear();
        self.param = None;
        self.start_time_msec = 0;
        self.end_time_msec = 0;
        self.movie_meta = MovieMetadata::default();
        self.scenes.clear();
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), CellError> {
        if self.state != State::Opened && self.state != State::Stopped {
            return Err(errors::INVALID_STATE);
        }
        self.state = State::Started;
        Ok(())
    }

    pub fn stop(&mut self) -> Result<(), CellError> {
        if self.state != State::Started {
            return Err(errors::INVALID_STATE);
        }
        self.state = State::Stopped;
        Ok(())
    }

    pub fn set_info(&mut self, key: i32, value: SetInfoValue) -> Result<(), CellError> {
        if self.state == State::Unloaded {
            return Err(errors::INVALID_STATE);
        }
        match (key, value) {
            (SETINFO_MOVIE_START_TIME_MSEC, SetInfoValue::Time(t)) => {
                self.start_time_msec = t;
                Ok(())
            }
            (SETINFO_MOVIE_END_TIME_MSEC, SetInfoValue::Time(t)) => {
                if t < self.start_time_msec {
                    return Err(errors::INVALID_VALUE);
                }
                self.end_time_msec = t;
                Ok(())
            }
            (SETINFO_MOVIE_META, SetInfoValue::Movie(meta)) => {
                meta.validate()?;
                self.movie_meta = meta;
                Ok(())
            }
            (SETINFO_SCENE_META, SetInfoValue::Scene(meta)) => {
                meta.validate()?;
                self.scenes.push(meta);
                Ok(())
            }
            _ => Err(errors::INVALID_VALUE),
        }
    }

    pub fn get_info(&self, key: i32) -> Result<GetInfoValue, CellError> {
        if self.state == State::Unloaded {
            return Err(errors::INVALID_STATE);
        }
        match key {
            INFO_VIDEO_INPUT_ADDR
            | INFO_VIDEO_INPUT_WIDTH
            | INFO_VIDEO_INPUT_PITCH
            | INFO_VIDEO_INPUT_HEIGHT
            | INFO_AUDIO_INPUT_ADDR
            | INFO_SPURS_SYSTEMWORKLOAD_ID => Ok(GetInfoValue::U64(0)),
            INFO_MOVIE_TIME_MSEC => Ok(GetInfoValue::U64(self.end_time_msec.saturating_sub(self.start_time_msec))),
            _ => Err(errors::INVALID_VALUE),
        }
    }

    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    #[must_use]
    pub fn scene_count(&self) -> usize {
        self.scenes.len()
    }

    #[must_use]
    pub fn movie_metadata(&self) -> &MovieMetadata {
        &self.movie_meta
    }
}

impl Default for RecManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetInfoValue {
    Time(u64),
    Movie(MovieMetadata),
    Scene(SceneMetadata),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GetInfoValue {
    U64(u64),
    Str(String),
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_param() -> RecParam {
        let mut opts = std::collections::HashMap::new();
        opts.insert(OPTION_PPU_THREAD_PRIORITY, PPU_THREAD_PRIORITY_DEFAULT as i64);
        opts.insert(OPTION_AUDIO_INPUT_MIX_VOL, 50);
        RecParam { video_format: 0x1110, audio_format: 0x0001, options: opts }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::OUT_OF_MEMORY.0, 0x8002_c501);
        assert_eq!(errors::FATAL.0, 0x8002_c502);
        assert_eq!(errors::INVALID_VALUE.0, 0x8002_c503);
        assert_eq!(errors::FILE_OPEN.0, 0x8002_c504);
        assert_eq!(errors::FILE_WRITE.0, 0x8002_c505);
        assert_eq!(errors::INVALID_STATE.0, 0x8002_c506);
        assert_eq!(errors::FILE_NO_DATA.0, 0x8002_c507);
    }

    #[test]
    fn status_constants_stable() {
        assert_eq!(STATUS_UNLOAD, 0);
        assert_eq!(STATUS_OPEN, 1);
        assert_eq!(STATUS_START, 2);
        assert_eq!(STATUS_STOP, 3);
        assert_eq!(STATUS_CLOSE, 4);
        assert_eq!(STATUS_ERR, 10);
    }

    #[test]
    fn memory_container_constants_stable() {
        assert_eq!(MAX_MEMORY_CONTAINER_SIZE, 16 * 1024 * 1024);
        assert_eq!(MAX_MEMORY_CONTAINER_SIZE_LEGACY, 9 * 1024 * 1024);
        assert_eq!(MAX_PATH_LEN, 1023);
    }

    #[test]
    fn thread_priority_defaults_stable() {
        assert_eq!(PPU_THREAD_PRIORITY_DEFAULT, 400);
        assert_eq!(SPU_THREAD_PRIORITY_DEFAULT, 60);
    }

    #[test]
    fn capture_priority_constants_stable() {
        assert_eq!(CAPTURE_PRIORITY_HIGHEST, 0);
        assert_eq!(CAPTURE_PRIORITY_EXCEPT_NOTIFICATION, 1);
        assert_eq!(CAPTURE_PRIORITY_GAME_SCREEN, 2);
    }

    #[test]
    fn video_input_constants_stable() {
        assert_eq!(VIDEO_INPUT_DISABLE, 0);
        assert_eq!(VIDEO_INPUT_ARGB_4_3, 1);
        assert_eq!(VIDEO_INPUT_YUV420PLANAR_16_9, 5);
    }

    #[test]
    fn audio_mix_vol_range_stable() {
        assert_eq!(AUDIO_INPUT_MIX_VOL_MIN, 0);
        assert_eq!(AUDIO_INPUT_MIX_VOL_MAX, 100);
    }

    #[test]
    fn audio_block_samples_stable() {
        assert_eq!(AUDIO_BLOCK_SAMPLES, 256);
    }

    #[test]
    fn option_keys_stable() {
        assert_eq!(OPTION_PPU_THREAD_PRIORITY, 1);
        assert_eq!(OPTION_SPU_THREAD_PRIORITY, 2);
        assert_eq!(OPTION_METADATA_FILENAME, 14);
        assert_eq!(OPTION_SPURS, 15);
    }

    #[test]
    fn info_keys_stable() {
        assert_eq!(INFO_VIDEO_INPUT_ADDR, 0);
        assert_eq!(INFO_MOVIE_TIME_MSEC, 5);
        assert_eq!(SETINFO_MOVIE_START_TIME_MSEC, 100);
        assert_eq!(SETINFO_SCENE_META, 201);
    }

    #[test]
    fn is_known_video_format_covers_core_codecs() {
        assert!(is_known_video_format(0x0000)); // MPEG4 small 512k
        assert!(is_known_video_format(0x1000)); // AVC MP small 512k
        assert!(is_known_video_format(0x2000)); // AVC BL small 512k
        assert!(is_known_video_format(0x3060)); // MJPEG small
        assert!(is_known_video_format(0x4670)); // M4HD HD720
        assert!(is_known_video_format(0x0310)); // YouTube alias
        assert!(!is_known_video_format(0xFFFF));
        assert!(!is_known_video_format(0x9999));
    }

    #[test]
    fn is_known_audio_format_covers_core_codecs() {
        assert!(is_known_audio_format(0x0000)); // AAC 96K
        assert!(is_known_audio_format(0x0001)); // AAC 128K
        assert!(is_known_audio_format(0x1007)); // ULAW 384K
        assert!(is_known_audio_format(0x2009)); // PCM 1536K
        assert!(!is_known_audio_format(0x9999));
    }

    #[test]
    fn rec_param_validate_bad_video_format_rejected() {
        let mut p = ok_param();
        p.video_format = 0x9999;
        assert_eq!(p.validate(), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn rec_param_validate_bad_audio_format_rejected() {
        let mut p = ok_param();
        p.audio_format = 0x9999;
        assert_eq!(p.validate(), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn rec_param_validate_bad_option_value_rejected() {
        let mut p = ok_param();
        p.options.insert(OPTION_AUDIO_INPUT_MIX_VOL, 150);
        assert_eq!(p.validate(), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn rec_param_validate_bad_capture_priority_rejected() {
        let mut p = ok_param();
        p.options.insert(OPTION_CAPTURE_PRIORITY, 99);
        assert_eq!(p.validate(), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn rec_param_validate_unknown_option_rejected() {
        let mut p = ok_param();
        p.options.insert(999, 0);
        assert_eq!(p.validate(), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn query_mem_size_hd_needs_full_cap() {
        let mut p = ok_param();
        p.video_format = 0x4670; // M4HD HD720
        assert_eq!(RecManager::query_mem_size(&p, false).unwrap(), MAX_MEMORY_CONTAINER_SIZE);
        assert_eq!(RecManager::query_mem_size(&p, true).unwrap(), MAX_MEMORY_CONTAINER_SIZE_LEGACY);
    }

    #[test]
    fn query_mem_size_non_hd_uses_half() {
        let p = ok_param();
        assert_eq!(RecManager::query_mem_size(&p, false).unwrap(), MAX_MEMORY_CONTAINER_SIZE / 2);
    }

    #[test]
    fn open_empty_path_rejected() {
        let mut m = RecManager::new();
        assert_eq!(m.open("", ok_param()), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn open_oversize_path_rejected() {
        let mut m = RecManager::new();
        let long = "a".repeat(MAX_PATH_LEN + 1);
        assert_eq!(m.open(long, ok_param()), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn open_already_opened_rejected() {
        let mut m = RecManager::new();
        m.open("/dev_hdd0/test.mp4", ok_param()).unwrap();
        assert_eq!(m.open("/dev_hdd0/test2.mp4", ok_param()), Err(errors::INVALID_STATE));
    }

    #[test]
    fn close_when_unloaded_rejected() {
        let mut m = RecManager::new();
        assert_eq!(m.close(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn close_when_started_rejected() {
        let mut m = RecManager::new();
        m.open("/dev_hdd0/test.mp4", ok_param()).unwrap();
        m.start().unwrap();
        assert_eq!(m.close(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn start_when_unloaded_rejected() {
        let mut m = RecManager::new();
        assert_eq!(m.start(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn start_stop_restart_round_trip() {
        let mut m = RecManager::new();
        m.open("/dev_hdd0/test.mp4", ok_param()).unwrap();
        m.start().unwrap();
        m.stop().unwrap();
        m.start().unwrap();
        m.stop().unwrap();
        m.close().unwrap();
        assert_eq!(m.state(), State::Unloaded);
    }

    #[test]
    fn stop_when_not_started_rejected() {
        let mut m = RecManager::new();
        m.open("/dev_hdd0/test.mp4", ok_param()).unwrap();
        assert_eq!(m.stop(), Err(errors::INVALID_STATE));
    }

    #[test]
    fn status_matches_state_enum() {
        let mut m = RecManager::new();
        assert_eq!(m.status(), STATUS_UNLOAD);
        m.open("/x.mp4", ok_param()).unwrap();
        assert_eq!(m.status(), STATUS_OPEN);
        m.start().unwrap();
        assert_eq!(m.status(), STATUS_START);
        m.stop().unwrap();
        assert_eq!(m.status(), STATUS_STOP);
    }

    #[test]
    fn set_info_start_and_end_time() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        m.set_info(SETINFO_MOVIE_START_TIME_MSEC, SetInfoValue::Time(1000)).unwrap();
        m.set_info(SETINFO_MOVIE_END_TIME_MSEC, SetInfoValue::Time(5000)).unwrap();
        let duration = m.get_info(INFO_MOVIE_TIME_MSEC).unwrap();
        assert_eq!(duration, GetInfoValue::U64(4000));
    }

    #[test]
    fn set_info_end_before_start_rejected() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        m.set_info(SETINFO_MOVIE_START_TIME_MSEC, SetInfoValue::Time(5000)).unwrap();
        assert_eq!(
            m.set_info(SETINFO_MOVIE_END_TIME_MSEC, SetInfoValue::Time(1000)),
            Err(errors::INVALID_VALUE)
        );
    }

    #[test]
    fn set_info_movie_metadata_happy_path() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        let meta = MovieMetadata {
            game_title: "My Game".into(),
            movie_title: "Epic Clip".into(),
            description: "Boss fight".into(),
            userdata: "tag42".into(),
        };
        m.set_info(SETINFO_MOVIE_META, SetInfoValue::Movie(meta.clone())).unwrap();
        assert_eq!(m.movie_metadata(), &meta);
    }

    #[test]
    fn set_info_movie_metadata_oversized_rejected() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        let mut meta = MovieMetadata::default();
        meta.game_title = "x".repeat(MOVIE_META_GAME_TITLE_LEN);
        assert_eq!(
            m.set_info(SETINFO_MOVIE_META, SetInfoValue::Movie(meta)),
            Err(errors::INVALID_VALUE)
        );
    }

    #[test]
    fn set_info_scene_metadata_happy_path() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        let scene = SceneMetadata {
            scene_type: SCENE_META_TYPE_CHAPTER,
            start_time_msec: 0,
            end_time_msec: 5000,
            title: "Chapter 1".into(),
            tags: vec!["tag1".into(), "tag2".into()],
        };
        m.set_info(SETINFO_SCENE_META, SetInfoValue::Scene(scene)).unwrap();
        assert_eq!(m.scene_count(), 1);
    }

    #[test]
    fn set_info_scene_metadata_bad_type_rejected() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        let mut scene = SceneMetadata {
            scene_type: 99,
            start_time_msec: 0,
            end_time_msec: 1000,
            title: String::new(),
            tags: vec![],
        };
        assert_eq!(
            m.set_info(SETINFO_SCENE_META, SetInfoValue::Scene(scene.clone())),
            Err(errors::INVALID_VALUE)
        );
        scene.scene_type = SCENE_META_TYPE_CHAPTER;
        scene.end_time_msec = 500; // before start
        scene.start_time_msec = 1000;
        assert_eq!(
            m.set_info(SETINFO_SCENE_META, SetInfoValue::Scene(scene)),
            Err(errors::INVALID_VALUE)
        );
    }

    #[test]
    fn set_info_scene_metadata_too_many_tags_rejected() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        let scene = SceneMetadata {
            scene_type: SCENE_META_TYPE_CHAPTER,
            start_time_msec: 0,
            end_time_msec: 1000,
            title: String::new(),
            tags: (0..=SCENE_META_TAG_NUM).map(|i| format!("t{i}")).collect(),
        };
        assert_eq!(
            m.set_info(SETINFO_SCENE_META, SetInfoValue::Scene(scene)),
            Err(errors::INVALID_VALUE)
        );
    }

    #[test]
    fn set_info_when_unloaded_rejected() {
        let mut m = RecManager::new();
        assert_eq!(
            m.set_info(SETINFO_MOVIE_START_TIME_MSEC, SetInfoValue::Time(0)),
            Err(errors::INVALID_STATE)
        );
    }

    #[test]
    fn set_info_unknown_key_rejected() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        assert_eq!(m.set_info(999, SetInfoValue::Time(0)), Err(errors::INVALID_VALUE));
    }

    #[test]
    fn get_info_unknown_key_rejected() {
        let mut m = RecManager::new();
        m.open("/x.mp4", ok_param()).unwrap();
        assert_eq!(m.get_info(999).err(), Some(errors::INVALID_VALUE));
    }

    #[test]
    fn get_info_when_unloaded_rejected() {
        let m = RecManager::new();
        assert_eq!(m.get_info(INFO_VIDEO_INPUT_ADDR).err(), Some(errors::INVALID_STATE));
    }

    #[test]
    fn full_rec_lifecycle_smoke() {
        let mut m = RecManager::new();
        let mem = RecManager::query_mem_size(&ok_param(), false).unwrap();
        assert!(mem > 0);
        m.open("/dev_hdd0/movies/clip.mp4", ok_param()).unwrap();
        m.set_info(
            SETINFO_MOVIE_META,
            SetInfoValue::Movie(MovieMetadata {
                game_title: "RPCS3".into(),
                movie_title: "Test".into(),
                description: "Smoke".into(),
                userdata: String::new(),
            }),
        )
        .unwrap();
        m.set_info(SETINFO_MOVIE_START_TIME_MSEC, SetInfoValue::Time(0)).unwrap();
        m.start().unwrap();
        m.set_info(SETINFO_MOVIE_END_TIME_MSEC, SetInfoValue::Time(30_000)).unwrap();
        m.set_info(
            SETINFO_SCENE_META,
            SetInfoValue::Scene(SceneMetadata {
                scene_type: SCENE_META_TYPE_CLIP_HIGHLIGHT,
                start_time_msec: 10_000,
                end_time_msec: 20_000,
                title: "Highlight".into(),
                tags: vec!["boss".into()],
            }),
        )
        .unwrap();
        m.stop().unwrap();
        assert_eq!(m.get_info(INFO_MOVIE_TIME_MSEC), Ok(GetInfoValue::U64(30_000)));
        m.close().unwrap();
    }
}
