//! `rpcs3-hle-cellvideoexport` — XMB video library export HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellVideoExport.cpp`. Games call into
//! cellVideoExport to publish a recorded video file (e.g. gameplay clip)
//! to the user's XMB video library. The API is:
//!
//! 1. `Initialize(version, container, cb, ud)` — register the async
//!    finish callback.
//! 2. `FromFile(src_path, setParam, cb, ud)` — stage a file for export.
//!    The lib copies it to `/dev_hdd0/video/` and registers metadata.
//! 3. `Progress(progress_out)` — poll 0..65535 progress counter.
//! 4. `Finalize(cb, ud)` — tear down.
//!
//! ## Entry points covered
//!
//! | HLE function                         | Rust wrapper                           |
//! |--------------------------------------|----------------------------------------|
//! | `cellVideoExportInitialize`          | [`VideoExportManager::initialize`]     |
//! | `cellVideoExportInitialize2`         | [`VideoExportManager::initialize2`]    |
//! | `cellVideoExportFromFile`            | [`VideoExportManager::from_file`]      |
//! | `cellVideoExportProgress`            | [`VideoExportManager::progress`]       |
//! | `cellVideoExportFinalize`            | [`VideoExportManager::finalize`]       |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellVideoExport.cpp:9-21
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const BUSY: CellError = CellError(0x8002_ca01);
    pub const INTERNAL: CellError = CellError(0x8002_ca02);
    pub const PARAM: CellError = CellError(0x8002_ca03);
    pub const ACCESS_ERROR: CellError = CellError(0x8002_ca04);
    pub const DB_INTERNAL: CellError = CellError(0x8002_ca05);
    pub const DB_REGIST: CellError = CellError(0x8002_ca06);
    pub const SET_META: CellError = CellError(0x8002_ca07);
    pub const FLUSH_META: CellError = CellError(0x8002_ca08);
    pub const MOVE: CellError = CellError(0x8002_ca09);
    pub const INITIALIZE: CellError = CellError(0x8002_ca0a);
}

// =====================================================================
// Return / version / size constants
// =====================================================================

pub const RET_OK: u32 = 0;
pub const RET_CANCEL: u32 = 1;

pub const VERSION_CURRENT: u32 = 0;
pub const HDD_PATH_MAX: usize = 1055;
pub const VIDEO_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_COMMENT_MAX_SIZE: usize = 1024;

/// Progress counter maximum — real lib reports 0..=PROGRESS_MAX.
pub const PROGRESS_MAX: u32 = 0xFFFF;

/// Sentinel passed for `container` in `cellVideoExportInitialize` to
/// indicate "no container" / use the default allocation path.
pub const CONTAINER_NONE: u32 = 0xFFFF_FFFE;

pub const MIN_CONTAINER_SIZE: u32 = 0x500000; // 5 MiB per C++ comment

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetParam {
    pub title: String,
    pub game_title: String,
    pub game_comment: String,
    pub editable: i32,
}

impl SetParam {
    fn validate(&self) -> Result<(), CellError> {
        if self.title.len() >= VIDEO_TITLE_MAX_LENGTH * 3 {
            return Err(errors::PARAM);
        }
        if self.game_title.len() >= GAME_TITLE_MAX_LENGTH * 3 {
            return Err(errors::PARAM);
        }
        if self.game_comment.len() >= GAME_COMMENT_MAX_SIZE {
            return Err(errors::PARAM);
        }
        Ok(())
    }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Uninitialized,
    Ready,
    Exporting,
}

#[derive(Clone, Debug)]
pub struct VideoExportManager {
    state: State,
    progress: u32,
    last_dst: Option<String>,
}

impl VideoExportManager {
    #[must_use]
    pub fn new() -> Self {
        Self { state: State::Uninitialized, progress: 0, last_dst: None }
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    #[must_use]
    pub fn last_destination(&self) -> Option<&str> {
        self.last_dst.as_deref()
    }

    /// Validates that a path is acceptable as the source file for
    /// `FromFile` (mirrors `check_movie_path` in C++).
    #[must_use]
    pub fn check_path(path: &str) -> bool {
        if path.len() >= HDD_PATH_MAX {
            return false;
        }
        for c in path.chars() {
            let ok = c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.';
            if !ok {
                return false;
            }
        }
        if !(path.starts_with("/dev_hdd0") || path.starts_with("/dev_bdvd") || path.starts_with("/dev_hdd1")) {
            return false;
        }
        if path.contains("..") {
            return false;
        }
        true
    }

    /// Build a non-colliding destination path under `/dev_hdd0/video/`.
    /// Mirrors the `get_available_movie_path` loop: if the filename
    /// already exists (simulated via a `known` callable), add `_N`
    /// suffix before the extension until it doesn't.
    #[must_use]
    pub fn make_destination<F: Fn(&str) -> bool>(filename: &str, exists: F) -> String {
        let root = "/dev_hdd0/video/";
        let mut candidate = format!("{root}{filename}");
        let mut i = 0u32;
        while exists(&candidate) {
            let suffix = format!("_{i}");
            let new_name = if let Some(pos) = filename.rfind('.') {
                let mut s = filename.to_string();
                s.insert_str(pos, &suffix);
                s
            } else {
                format!("{filename}{suffix}")
            };
            candidate = format!("{root}{new_name}");
            i += 1;
        }
        candidate
    }

    // ----------------- Lifecycle -----------------

    /// `cellVideoExportInitialize(version, container, cb, ud)`.
    pub fn initialize(&mut self, version: u32, container: u32) -> Result<(), CellError> {
        if version != VERSION_CURRENT {
            return Err(errors::PARAM);
        }
        // The C++ code conceptually validates container size when not
        // the CONTAINER_NONE sentinel. We mirror that gate.
        if container != CONTAINER_NONE {
            // Any finite container size <MIN_CONTAINER_SIZE is rejected.
            // Tests pass a sentinel above MIN for positive cases.
            if container < MIN_CONTAINER_SIZE {
                return Err(errors::PARAM);
            }
        }
        if self.state != State::Uninitialized {
            return Err(errors::BUSY);
        }
        self.state = State::Ready;
        self.progress = 0;
        self.last_dst = None;
        Ok(())
    }

    /// `cellVideoExportInitialize2(version, cb, ud)` — no-container
    /// variant. Container is implicitly CONTAINER_NONE.
    pub fn initialize2(&mut self, version: u32) -> Result<(), CellError> {
        self.initialize(version, CONTAINER_NONE)
    }

    /// `cellVideoExportFinalize(cb, ud)`.
    pub fn finalize(&mut self) -> Result<(), CellError> {
        if self.state == State::Uninitialized {
            return Err(errors::INITIALIZE);
        }
        if self.state == State::Exporting {
            return Err(errors::BUSY);
        }
        self.state = State::Uninitialized;
        self.progress = 0;
        self.last_dst = None;
        Ok(())
    }

    /// `cellVideoExportFromFile(srcPath, setParam, cb, ud)`. Begins the
    /// copy-to-XMB flow. Tests finalise via `complete_export`.
    pub fn from_file(&mut self, src_path: &str, param: SetParam) -> Result<(), CellError> {
        if self.state != State::Ready {
            return Err(if self.state == State::Uninitialized { errors::INITIALIZE } else { errors::BUSY });
        }
        if !Self::check_path(src_path) {
            return Err(errors::PARAM);
        }
        param.validate()?;
        self.state = State::Exporting;
        self.progress = 0;
        Ok(())
    }

    /// Test hook: advance the progress counter (0..=PROGRESS_MAX).
    pub fn tick_progress(&mut self, value: u32) -> Result<(), CellError> {
        if self.state != State::Exporting {
            return Err(errors::INTERNAL);
        }
        if value > PROGRESS_MAX {
            return Err(errors::PARAM);
        }
        self.progress = value;
        Ok(())
    }

    /// `cellVideoExportProgress(progress_out)`.
    pub fn progress(&self) -> Result<u32, CellError> {
        if self.state == State::Uninitialized {
            return Err(errors::INITIALIZE);
        }
        Ok(self.progress)
    }

    /// Completes the active export by recording the resolved destination
    /// path and returning to Ready. `exists` simulates the existing-file
    /// check used to pick a non-colliding name.
    pub fn complete_export<F: Fn(&str) -> bool>(
        &mut self,
        filename: &str,
        exists: F,
    ) -> Result<String, CellError> {
        if self.state != State::Exporting {
            return Err(errors::INTERNAL);
        }
        if filename.is_empty() {
            return Err(errors::PARAM);
        }
        let dst = Self::make_destination(filename, exists);
        self.last_dst = Some(dst.clone());
        self.progress = PROGRESS_MAX;
        self.state = State::Ready;
        Ok(dst)
    }

    pub fn cancel_export(&mut self) -> Result<u32, CellError> {
        if self.state != State::Exporting {
            return Err(errors::INTERNAL);
        }
        self.state = State::Ready;
        self.progress = 0;
        Ok(RET_CANCEL)
    }
}

impl Default for VideoExportManager {
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

    fn ok_param() -> SetParam {
        SetParam {
            title: "Clip".into(),
            game_title: "Game".into(),
            game_comment: "Nice moment".into(),
            editable: 1,
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::BUSY.0, 0x8002_ca01);
        assert_eq!(errors::INTERNAL.0, 0x8002_ca02);
        assert_eq!(errors::PARAM.0, 0x8002_ca03);
        assert_eq!(errors::ACCESS_ERROR.0, 0x8002_ca04);
        assert_eq!(errors::DB_INTERNAL.0, 0x8002_ca05);
        assert_eq!(errors::DB_REGIST.0, 0x8002_ca06);
        assert_eq!(errors::SET_META.0, 0x8002_ca07);
        assert_eq!(errors::FLUSH_META.0, 0x8002_ca08);
        assert_eq!(errors::MOVE.0, 0x8002_ca09);
        assert_eq!(errors::INITIALIZE.0, 0x8002_ca0a);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(RET_OK, 0);
        assert_eq!(RET_CANCEL, 1);
        assert_eq!(VERSION_CURRENT, 0);
        assert_eq!(HDD_PATH_MAX, 1055);
        assert_eq!(VIDEO_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_COMMENT_MAX_SIZE, 1024);
        assert_eq!(PROGRESS_MAX, 0xFFFF);
        assert_eq!(CONTAINER_NONE, 0xFFFF_FFFE);
        assert_eq!(MIN_CONTAINER_SIZE, 0x500000);
    }

    #[test]
    fn check_path_accepts_whitelist() {
        assert!(VideoExportManager::check_path("/dev_hdd0/clips/my-video_1.mp4"));
        assert!(VideoExportManager::check_path("/dev_bdvd/trailer.mp4"));
        assert!(VideoExportManager::check_path("/dev_hdd1/cache/clip.avi"));
    }

    #[test]
    fn check_path_rejects_traversal_and_non_ascii() {
        assert!(!VideoExportManager::check_path("/dev_hdd0/../secret"));
        assert!(!VideoExportManager::check_path("/dev_hdd0/clip with space.mp4"));
        assert!(!VideoExportManager::check_path("/dev_hdd0/音声.mp4"));
    }

    #[test]
    fn check_path_rejects_foreign_roots() {
        assert!(!VideoExportManager::check_path("/dev_usb000/evil.mp4"));
        assert!(!VideoExportManager::check_path("/dev_flash/a.mp4"));
        assert!(!VideoExportManager::check_path("foo/bar.mp4"));
    }

    #[test]
    fn check_path_rejects_too_long() {
        let path = "/dev_hdd0/".to_string() + &"a".repeat(HDD_PATH_MAX);
        assert!(!VideoExportManager::check_path(&path));
    }

    #[test]
    fn make_destination_no_collision() {
        let dst = VideoExportManager::make_destination("clip.mp4", |_| false);
        assert_eq!(dst, "/dev_hdd0/video/clip.mp4");
    }

    #[test]
    fn make_destination_with_collision_adds_suffix_before_ext() {
        let dst = VideoExportManager::make_destination("clip.mp4", |p| p == "/dev_hdd0/video/clip.mp4");
        assert_eq!(dst, "/dev_hdd0/video/clip_0.mp4");
    }

    #[test]
    fn make_destination_collides_multiple_times() {
        let taken = ["/dev_hdd0/video/clip.mp4", "/dev_hdd0/video/clip_0.mp4", "/dev_hdd0/video/clip_1.mp4"];
        let dst = VideoExportManager::make_destination("clip.mp4", |p| taken.contains(&p));
        assert_eq!(dst, "/dev_hdd0/video/clip_2.mp4");
    }

    #[test]
    fn make_destination_no_extension_appends_suffix() {
        let dst = VideoExportManager::make_destination("clip", |p| p == "/dev_hdd0/video/clip");
        assert_eq!(dst, "/dev_hdd0/video/clip_0");
    }

    #[test]
    fn initialize_happy_path() {
        let mut m = VideoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.state(), State::Ready);
    }

    #[test]
    fn initialize_bad_version_rejected() {
        let mut m = VideoExportManager::new();
        assert_eq!(m.initialize(99, CONTAINER_NONE), Err(errors::PARAM));
    }

    #[test]
    fn initialize_small_container_rejected() {
        let mut m = VideoExportManager::new();
        assert_eq!(m.initialize(VERSION_CURRENT, 1024), Err(errors::PARAM));
    }

    #[test]
    fn initialize_with_container_at_min_ok() {
        let mut m = VideoExportManager::new();
        m.initialize(VERSION_CURRENT, MIN_CONTAINER_SIZE).unwrap();
    }

    #[test]
    fn initialize_twice_is_busy() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        assert_eq!(m.initialize2(VERSION_CURRENT), Err(errors::BUSY));
    }

    #[test]
    fn initialize2_delegates_to_no_container() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        assert_eq!(m.state(), State::Ready);
    }

    #[test]
    fn finalize_without_init_is_initialize() {
        let mut m = VideoExportManager::new();
        assert_eq!(m.finalize(), Err(errors::INITIALIZE));
    }

    #[test]
    fn finalize_while_exporting_is_busy() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/clip.mp4", ok_param()).unwrap();
        assert_eq!(m.finalize(), Err(errors::BUSY));
    }

    #[test]
    fn finalize_round_trip() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.finalize().unwrap();
        assert_eq!(m.state(), State::Uninitialized);
    }

    #[test]
    fn from_file_without_init_is_initialize() {
        let mut m = VideoExportManager::new();
        assert_eq!(m.from_file("/dev_hdd0/x.mp4", ok_param()), Err(errors::INITIALIZE));
    }

    #[test]
    fn from_file_happy_path() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/clip.mp4", ok_param()).unwrap();
        assert_eq!(m.state(), State::Exporting);
        assert_eq!(m.progress(), Ok(0));
    }

    #[test]
    fn from_file_bad_path_rejected() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        assert_eq!(m.from_file("/tmp/../secret", ok_param()), Err(errors::PARAM));
    }

    #[test]
    fn from_file_duplicate_is_busy() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/a.mp4", ok_param()).unwrap();
        assert_eq!(m.from_file("/dev_hdd0/b.mp4", ok_param()), Err(errors::BUSY));
    }

    #[test]
    fn from_file_long_title_rejected() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        let mut p = ok_param();
        p.title = "t".repeat(VIDEO_TITLE_MAX_LENGTH * 3);
        assert_eq!(m.from_file("/dev_hdd0/x.mp4", p), Err(errors::PARAM));
    }

    #[test]
    fn from_file_long_comment_rejected() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        let mut p = ok_param();
        p.game_comment = "c".repeat(GAME_COMMENT_MAX_SIZE);
        assert_eq!(m.from_file("/dev_hdd0/x.mp4", p), Err(errors::PARAM));
    }

    #[test]
    fn tick_progress_tracks_value() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/x.mp4", ok_param()).unwrap();
        m.tick_progress(30_000).unwrap();
        assert_eq!(m.progress(), Ok(30_000));
    }

    #[test]
    fn tick_progress_out_of_range_rejected() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/x.mp4", ok_param()).unwrap();
        assert_eq!(m.tick_progress(PROGRESS_MAX + 1), Err(errors::PARAM));
    }

    #[test]
    fn tick_progress_when_not_exporting_is_internal() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        assert_eq!(m.tick_progress(1), Err(errors::INTERNAL));
    }

    #[test]
    fn progress_without_init_is_initialize() {
        let m = VideoExportManager::new();
        assert_eq!(m.progress(), Err(errors::INITIALIZE));
    }

    #[test]
    fn complete_export_returns_to_ready_and_records_dst() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/clip.mp4", ok_param()).unwrap();
        let dst = m.complete_export("clip.mp4", |_| false).unwrap();
        assert_eq!(dst, "/dev_hdd0/video/clip.mp4");
        assert_eq!(m.state(), State::Ready);
        assert_eq!(m.last_destination(), Some("/dev_hdd0/video/clip.mp4"));
        assert_eq!(m.progress(), Ok(PROGRESS_MAX));
    }

    #[test]
    fn complete_export_handles_collision() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/clip.mp4", ok_param()).unwrap();
        let dst = m.complete_export("clip.mp4", |p| p == "/dev_hdd0/video/clip.mp4").unwrap();
        assert_eq!(dst, "/dev_hdd0/video/clip_0.mp4");
    }

    #[test]
    fn complete_export_empty_filename_rejected() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/clip.mp4", ok_param()).unwrap();
        assert_eq!(m.complete_export("", |_| false), Err(errors::PARAM));
    }

    #[test]
    fn complete_without_active_export_is_internal() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        assert_eq!(m.complete_export("x.mp4", |_| false), Err(errors::INTERNAL));
    }

    #[test]
    fn cancel_export_happy_path() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/clip.mp4", ok_param()).unwrap();
        assert_eq!(m.cancel_export(), Ok(RET_CANCEL));
        assert_eq!(m.state(), State::Ready);
        assert_eq!(m.progress(), Ok(0));
    }

    #[test]
    fn cancel_export_without_active_is_internal() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        assert_eq!(m.cancel_export(), Err(errors::INTERNAL));
    }

    #[test]
    fn full_export_lifecycle_smoke() {
        let mut m = VideoExportManager::new();
        m.initialize2(VERSION_CURRENT).unwrap();
        m.from_file("/dev_hdd0/recordings/clip1.mp4", ok_param()).unwrap();
        m.tick_progress(0x1000).unwrap();
        m.tick_progress(0x8000).unwrap();
        let dst = m.complete_export("clip1.mp4", |_| false).unwrap();
        assert!(dst.starts_with("/dev_hdd0/video/"));
        // A second export re-uses the same manager.
        m.from_file("/dev_hdd0/recordings/clip2.mp4", ok_param()).unwrap();
        m.cancel_export().unwrap();
        m.finalize().unwrap();
    }
}
