//! `rpcs3-hle-cellphotoexport` — XMB photo library export HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellPhotoExport.cpp`. Games call in to
//! publish an image file they produced (screenshot, photo-booth capture,
//! user-generated art) into the user's XMB photo library. Shape is
//! identical to `cellMusicExport` / `cellVideoExport`: Initialize →
//! FromFile → Progress → Finalize. Photo destination root is
//! `/dev_hdd0/photo/`.
//!
//! ## Entry points covered
//!
//! | HLE function                         | Rust wrapper                            |
//! |--------------------------------------|-----------------------------------------|
//! | `cellPhotoExportInitialize`          | [`PhotoExportManager::initialize`]      |
//! | `cellPhotoExportFromFile`            | [`PhotoExportManager::from_file`]       |
//! | `cellPhotoExportProgress`            | [`PhotoExportManager::progress`]        |
//! | `cellPhotoExportFinalize`            | [`PhotoExportManager::finalize`]        |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellPhotoExport.cpp:11-23
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const BUSY: CellError = CellError(0x8002_c201);
    pub const INTERNAL: CellError = CellError(0x8002_c202);
    pub const PARAM: CellError = CellError(0x8002_c203);
    pub const ACCESS_ERROR: CellError = CellError(0x8002_c204);
    pub const DB_INTERNAL: CellError = CellError(0x8002_c205);
    pub const DB_REGIST: CellError = CellError(0x8002_c206);
    pub const SET_META: CellError = CellError(0x8002_c207);
    pub const FLUSH_META: CellError = CellError(0x8002_c208);
    pub const MOVE: CellError = CellError(0x8002_c209);
    pub const INITIALIZE: CellError = CellError(0x8002_c20a);
}

// =====================================================================
// Constants (cellPhotoExport.cpp:48-59)
// =====================================================================

pub const VERSION_CURRENT: u32 = 0;
pub const HDD_PATH_MAX: usize = 1055;
pub const PHOTO_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_COMMENT_MAX_SIZE: usize = 1024;

pub const PROGRESS_MAX: u32 = 0xFFFF;
pub const CONTAINER_NONE: u32 = 0xFFFF_FFFE;
/// Photo exports are smaller than video exports; C++ has a TODO for
/// checking "container size >= 0x200000" (2 MiB).
pub const MIN_CONTAINER_SIZE: u32 = 0x200000;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetParam {
    pub photo_title: String,
    pub game_title: String,
    pub game_comment: String,
}

impl SetParam {
    fn validate(&self) -> Result<(), CellError> {
        if self.photo_title.len() >= PHOTO_TITLE_MAX_LENGTH * 3 {
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
pub struct PhotoExportManager {
    state: State,
    progress: u32,
    last_dst: Option<String>,
}

impl PhotoExportManager {
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

    /// Mirrors `check_photo_path` in C++: ASCII + /-_./ only, ≤1055 bytes,
    /// under /dev_hdd0/bdvd/hdd1, no `..` traversal.
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

    /// Destination under `/dev_hdd0/photo/` with `_N` suffix for
    /// collision resolution.
    #[must_use]
    pub fn make_destination<F: Fn(&str) -> bool>(filename: &str, exists: F) -> String {
        let root = "/dev_hdd0/photo/";
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

    pub fn initialize(&mut self, version: u32, container: u32) -> Result<(), CellError> {
        if version != VERSION_CURRENT {
            return Err(errors::PARAM);
        }
        if container != CONTAINER_NONE && container < MIN_CONTAINER_SIZE {
            return Err(errors::PARAM);
        }
        if self.state != State::Uninitialized {
            return Err(errors::BUSY);
        }
        self.state = State::Ready;
        self.progress = 0;
        self.last_dst = None;
        Ok(())
    }

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

    // ----------------- Export -----------------

    pub fn from_file(&mut self, src_path: &str, param: SetParam) -> Result<(), CellError> {
        match self.state {
            State::Uninitialized => return Err(errors::INITIALIZE),
            State::Exporting => return Err(errors::BUSY),
            State::Ready => {}
        }
        if !Self::check_path(src_path) {
            return Err(errors::PARAM);
        }
        param.validate()?;
        self.state = State::Exporting;
        self.progress = 0;
        Ok(())
    }

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

    pub fn progress(&self) -> Result<u32, CellError> {
        if self.state == State::Uninitialized {
            return Err(errors::INITIALIZE);
        }
        Ok(self.progress)
    }

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

    pub fn cancel_export(&mut self) -> Result<(), CellError> {
        if self.state != State::Exporting {
            return Err(errors::INTERNAL);
        }
        self.state = State::Ready;
        self.progress = 0;
        Ok(())
    }
}

impl Default for PhotoExportManager {
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
            photo_title: "Screenshot".into(),
            game_title: "Game".into(),
            game_comment: "Nice moment".into(),
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::BUSY.0, 0x8002_c201);
        assert_eq!(errors::INTERNAL.0, 0x8002_c202);
        assert_eq!(errors::PARAM.0, 0x8002_c203);
        assert_eq!(errors::ACCESS_ERROR.0, 0x8002_c204);
        assert_eq!(errors::DB_INTERNAL.0, 0x8002_c205);
        assert_eq!(errors::DB_REGIST.0, 0x8002_c206);
        assert_eq!(errors::SET_META.0, 0x8002_c207);
        assert_eq!(errors::FLUSH_META.0, 0x8002_c208);
        assert_eq!(errors::MOVE.0, 0x8002_c209);
        assert_eq!(errors::INITIALIZE.0, 0x8002_c20a);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(VERSION_CURRENT, 0);
        assert_eq!(HDD_PATH_MAX, 1055);
        assert_eq!(PHOTO_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_COMMENT_MAX_SIZE, 1024);
        assert_eq!(PROGRESS_MAX, 0xFFFF);
        assert_eq!(CONTAINER_NONE, 0xFFFF_FFFE);
        assert_eq!(MIN_CONTAINER_SIZE, 0x200000);
    }

    #[test]
    fn check_path_whitelist() {
        assert!(PhotoExportManager::check_path("/dev_hdd0/cap/pic.png"));
        assert!(PhotoExportManager::check_path("/dev_bdvd/bundled.jpg"));
        assert!(PhotoExportManager::check_path("/dev_hdd1/cached.jpg"));
    }

    #[test]
    fn check_path_traversal_rejected() {
        assert!(!PhotoExportManager::check_path("/dev_hdd0/../secret.png"));
    }

    #[test]
    fn check_path_non_ascii_rejected() {
        assert!(!PhotoExportManager::check_path("/dev_hdd0/写真.png"));
        assert!(!PhotoExportManager::check_path("/dev_hdd0/my pic.png"));
    }

    #[test]
    fn check_path_foreign_root_rejected() {
        assert!(!PhotoExportManager::check_path("/dev_usb000/x.png"));
        assert!(!PhotoExportManager::check_path("/tmp/x.png"));
    }

    #[test]
    fn check_path_too_long_rejected() {
        let path = "/dev_hdd0/".to_string() + &"a".repeat(HDD_PATH_MAX);
        assert!(!PhotoExportManager::check_path(&path));
    }

    #[test]
    fn make_destination_no_collision() {
        let dst = PhotoExportManager::make_destination("shot.png", |_| false);
        assert_eq!(dst, "/dev_hdd0/photo/shot.png");
    }

    #[test]
    fn make_destination_collision_suffix() {
        let dst = PhotoExportManager::make_destination("shot.png", |p| p == "/dev_hdd0/photo/shot.png");
        assert_eq!(dst, "/dev_hdd0/photo/shot_0.png");
    }

    #[test]
    fn make_destination_multi_collision() {
        let taken = ["/dev_hdd0/photo/x.jpg", "/dev_hdd0/photo/x_0.jpg", "/dev_hdd0/photo/x_1.jpg"];
        let dst = PhotoExportManager::make_destination("x.jpg", |p| taken.contains(&p));
        assert_eq!(dst, "/dev_hdd0/photo/x_2.jpg");
    }

    #[test]
    fn make_destination_no_extension() {
        let dst = PhotoExportManager::make_destination("pic", |p| p == "/dev_hdd0/photo/pic");
        assert_eq!(dst, "/dev_hdd0/photo/pic_0");
    }

    #[test]
    fn initialize_happy_path() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.state(), State::Ready);
    }

    #[test]
    fn initialize_bad_version_rejected() {
        let mut m = PhotoExportManager::new();
        assert_eq!(m.initialize(99, CONTAINER_NONE), Err(errors::PARAM));
    }

    #[test]
    fn initialize_small_container_rejected() {
        let mut m = PhotoExportManager::new();
        assert_eq!(m.initialize(VERSION_CURRENT, 1024), Err(errors::PARAM));
    }

    #[test]
    fn initialize_at_min_container_ok() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, MIN_CONTAINER_SIZE).unwrap();
    }

    #[test]
    fn initialize_twice_is_busy() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.initialize(VERSION_CURRENT, CONTAINER_NONE), Err(errors::BUSY));
    }

    #[test]
    fn finalize_without_init_is_initialize() {
        let mut m = PhotoExportManager::new();
        assert_eq!(m.finalize(), Err(errors::INITIALIZE));
    }

    #[test]
    fn finalize_while_exporting_is_busy() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/x.png", ok_param()).unwrap();
        assert_eq!(m.finalize(), Err(errors::BUSY));
    }

    #[test]
    fn from_file_without_init_is_initialize() {
        let mut m = PhotoExportManager::new();
        assert_eq!(m.from_file("/dev_hdd0/x.png", ok_param()), Err(errors::INITIALIZE));
    }

    #[test]
    fn from_file_bad_path_rejected() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.from_file("/tmp/../evil", ok_param()), Err(errors::PARAM));
    }

    #[test]
    fn from_file_duplicate_is_busy() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.png", ok_param()).unwrap();
        assert_eq!(m.from_file("/dev_hdd0/b.png", ok_param()), Err(errors::BUSY));
    }

    #[test]
    fn from_file_long_title_rejected() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        let mut p = ok_param();
        p.photo_title = "t".repeat(PHOTO_TITLE_MAX_LENGTH * 3);
        assert_eq!(m.from_file("/dev_hdd0/a.png", p), Err(errors::PARAM));
    }

    #[test]
    fn from_file_long_game_title_rejected() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        let mut p = ok_param();
        p.game_title = "t".repeat(GAME_TITLE_MAX_LENGTH * 3);
        assert_eq!(m.from_file("/dev_hdd0/a.png", p), Err(errors::PARAM));
    }

    #[test]
    fn from_file_long_comment_rejected() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        let mut p = ok_param();
        p.game_comment = "c".repeat(GAME_COMMENT_MAX_SIZE);
        assert_eq!(m.from_file("/dev_hdd0/a.png", p), Err(errors::PARAM));
    }

    #[test]
    fn tick_progress_tracks() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.png", ok_param()).unwrap();
        m.tick_progress(0x7FFF).unwrap();
        assert_eq!(m.progress(), Ok(0x7FFF));
    }

    #[test]
    fn tick_progress_out_of_range_rejected() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.png", ok_param()).unwrap();
        assert_eq!(m.tick_progress(PROGRESS_MAX + 1), Err(errors::PARAM));
    }

    #[test]
    fn tick_progress_when_not_exporting_is_internal() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.tick_progress(1), Err(errors::INTERNAL));
    }

    #[test]
    fn progress_without_init_is_initialize() {
        let m = PhotoExportManager::new();
        assert_eq!(m.progress(), Err(errors::INITIALIZE));
    }

    #[test]
    fn complete_export_happy_path() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.png", ok_param()).unwrap();
        let dst = m.complete_export("a.png", |_| false).unwrap();
        assert_eq!(dst, "/dev_hdd0/photo/a.png");
        assert_eq!(m.state(), State::Ready);
        assert_eq!(m.progress(), Ok(PROGRESS_MAX));
    }

    #[test]
    fn complete_export_empty_filename_rejected() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.png", ok_param()).unwrap();
        assert_eq!(m.complete_export("", |_| false), Err(errors::PARAM));
    }

    #[test]
    fn complete_without_export_is_internal() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.complete_export("x.png", |_| false), Err(errors::INTERNAL));
    }

    #[test]
    fn cancel_export_happy_path() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.png", ok_param()).unwrap();
        m.cancel_export().unwrap();
        assert_eq!(m.state(), State::Ready);
    }

    #[test]
    fn cancel_without_export_is_internal() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.cancel_export(), Err(errors::INTERNAL));
    }

    #[test]
    fn full_export_lifecycle_smoke() {
        let mut m = PhotoExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/cap/shot1.png", ok_param()).unwrap();
        m.tick_progress(0x2000).unwrap();
        m.tick_progress(0xC000).unwrap();
        let dst1 = m.complete_export("shot1.png", |_| false).unwrap();
        assert!(dst1.starts_with("/dev_hdd0/photo/"));
        m.from_file("/dev_hdd0/cap/shot1.png", ok_param()).unwrap();
        let dst2 = m.complete_export("shot1.png", |p| p == "/dev_hdd0/photo/shot1.png").unwrap();
        assert_eq!(dst2, "/dev_hdd0/photo/shot1_0.png");
        m.finalize().unwrap();
    }
}
