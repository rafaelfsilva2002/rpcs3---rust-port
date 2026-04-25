//! `rpcs3-hle-cellmusicexport` — XMB music library export HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellMusicExport.cpp`. Games call in to
//! publish an audio file they produced (gameplay recording, custom
//! soundtrack generator, etc.) into the user's XMB music library.
//! Mirrors the same `Initialize → FromFile → Progress → Finalize` shape
//! as `cellVideoExport`, but with music metadata (artist/genre/title) and
//! a smaller 3 MiB minimum container.
//!
//! ## Entry points covered
//!
//! | HLE function                         | Rust wrapper                            |
//! |--------------------------------------|-----------------------------------------|
//! | `cellMusicExportInitialize`          | [`MusicExportManager::initialize`]      |
//! | `cellMusicExportFromFile`            | [`MusicExportManager::from_file`]       |
//! | `cellMusicExportProgress`            | [`MusicExportManager::progress`]        |
//! | `cellMusicExportFinalize`            | [`MusicExportManager::finalize`]        |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellMusicExport.cpp:10-22
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const BUSY: CellError = CellError(0x8002_c601);
    pub const INTERNAL: CellError = CellError(0x8002_c602);
    pub const PARAM: CellError = CellError(0x8002_c603);
    pub const ACCESS_ERROR: CellError = CellError(0x8002_c604);
    pub const DB_INTERNAL: CellError = CellError(0x8002_c605);
    pub const DB_REGIST: CellError = CellError(0x8002_c606);
    pub const SET_META: CellError = CellError(0x8002_c607);
    pub const FLUSH_META: CellError = CellError(0x8002_c608);
    pub const MOVE: CellError = CellError(0x8002_c609);
    pub const INITIALIZE: CellError = CellError(0x8002_c60a);
}

// =====================================================================
// Constants (cellMusicExport.cpp:47-54)
// =====================================================================

pub const VERSION_CURRENT: u32 = 0;
pub const HDD_PATH_MAX: usize = 1055;
pub const MUSIC_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_COMMENT_MAX_SIZE: usize = 1024;

pub const PROGRESS_MAX: u32 = 0xFFFF;
pub const CONTAINER_NONE: u32 = 0xFFFF_FFFE;
pub const MIN_CONTAINER_SIZE: u32 = 0x300000; // 3 MiB per C++ TODO comment

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetParam {
    pub title: String,
    pub game_title: String,
    pub artist: String,
    pub genre: String,
    pub game_comment: String,
}

impl SetParam {
    fn validate(&self) -> Result<(), CellError> {
        // 64 chars × 3 bytes for UTF-8 safety on artist / title / genre.
        if self.title.len() >= MUSIC_TITLE_MAX_LENGTH * 3 {
            return Err(errors::PARAM);
        }
        if self.game_title.len() >= GAME_TITLE_MAX_LENGTH * 3 {
            return Err(errors::PARAM);
        }
        if self.artist.len() >= MUSIC_TITLE_MAX_LENGTH * 3 {
            return Err(errors::PARAM);
        }
        if self.genre.len() >= MUSIC_TITLE_MAX_LENGTH * 3 {
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
pub struct MusicExportManager {
    state: State,
    progress: u32,
    last_dst: Option<String>,
}

impl MusicExportManager {
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

    /// Mirrors `check_music_path` in C++: ≤HDD_PATH_MAX, ASCII + /-_./ only,
    /// must be under /dev_hdd0/bdvd/hdd1, no `..` traversal.
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

    /// Mirrors `get_available_music_path`: emit destination under
    /// `/dev_hdd0/music/` and add `_N` suffix until it no longer
    /// collides with anything `exists` reports.
    #[must_use]
    pub fn make_destination<F: Fn(&str) -> bool>(filename: &str, exists: F) -> String {
        let root = "/dev_hdd0/music/";
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

    /// `cellMusicExportInitialize(version, container, cb, ud)`.
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

impl Default for MusicExportManager {
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
            title: "Boss Theme".into(),
            game_title: "Game".into(),
            artist: "Composer".into(),
            genre: "Electronic".into(),
            game_comment: "From final battle".into(),
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::BUSY.0, 0x8002_c601);
        assert_eq!(errors::INTERNAL.0, 0x8002_c602);
        assert_eq!(errors::PARAM.0, 0x8002_c603);
        assert_eq!(errors::ACCESS_ERROR.0, 0x8002_c604);
        assert_eq!(errors::DB_INTERNAL.0, 0x8002_c605);
        assert_eq!(errors::DB_REGIST.0, 0x8002_c606);
        assert_eq!(errors::SET_META.0, 0x8002_c607);
        assert_eq!(errors::FLUSH_META.0, 0x8002_c608);
        assert_eq!(errors::MOVE.0, 0x8002_c609);
        assert_eq!(errors::INITIALIZE.0, 0x8002_c60a);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(VERSION_CURRENT, 0);
        assert_eq!(HDD_PATH_MAX, 1055);
        assert_eq!(MUSIC_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_COMMENT_MAX_SIZE, 1024);
        assert_eq!(PROGRESS_MAX, 0xFFFF);
        assert_eq!(CONTAINER_NONE, 0xFFFF_FFFE);
        assert_eq!(MIN_CONTAINER_SIZE, 0x300000);
    }

    #[test]
    fn check_path_whitelist_accepts() {
        assert!(MusicExportManager::check_path("/dev_hdd0/clips/song-1.mp3"));
        assert!(MusicExportManager::check_path("/dev_bdvd/soundtrack.aac"));
        assert!(MusicExportManager::check_path("/dev_hdd1/cache/track.wav"));
    }

    #[test]
    fn check_path_rejects_traversal() {
        assert!(!MusicExportManager::check_path("/dev_hdd0/../secret.mp3"));
    }

    #[test]
    fn check_path_rejects_foreign_roots() {
        assert!(!MusicExportManager::check_path("/dev_usb000/a.mp3"));
        assert!(!MusicExportManager::check_path("/tmp/b.mp3"));
    }

    #[test]
    fn check_path_rejects_bad_chars() {
        assert!(!MusicExportManager::check_path("/dev_hdd0/song with space.mp3"));
        assert!(!MusicExportManager::check_path("/dev_hdd0/음악.mp3"));
    }

    #[test]
    fn check_path_rejects_too_long() {
        let path = "/dev_hdd0/".to_string() + &"a".repeat(HDD_PATH_MAX);
        assert!(!MusicExportManager::check_path(&path));
    }

    #[test]
    fn make_destination_no_collision() {
        let dst = MusicExportManager::make_destination("song.mp3", |_| false);
        assert_eq!(dst, "/dev_hdd0/music/song.mp3");
    }

    #[test]
    fn make_destination_with_collision_adds_suffix() {
        let dst = MusicExportManager::make_destination("song.mp3", |p| p == "/dev_hdd0/music/song.mp3");
        assert_eq!(dst, "/dev_hdd0/music/song_0.mp3");
    }

    #[test]
    fn make_destination_multiple_collisions_pick_next_index() {
        let taken = ["/dev_hdd0/music/s.mp3", "/dev_hdd0/music/s_0.mp3", "/dev_hdd0/music/s_1.mp3"];
        let dst = MusicExportManager::make_destination("s.mp3", |p| taken.contains(&p));
        assert_eq!(dst, "/dev_hdd0/music/s_2.mp3");
    }

    #[test]
    fn make_destination_no_extension_appends_suffix() {
        let dst = MusicExportManager::make_destination("trackA", |p| p == "/dev_hdd0/music/trackA");
        assert_eq!(dst, "/dev_hdd0/music/trackA_0");
    }

    #[test]
    fn initialize_happy_path() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.state(), State::Ready);
    }

    #[test]
    fn initialize_bad_version_rejected() {
        let mut m = MusicExportManager::new();
        assert_eq!(m.initialize(99, CONTAINER_NONE), Err(errors::PARAM));
    }

    #[test]
    fn initialize_small_container_rejected() {
        let mut m = MusicExportManager::new();
        assert_eq!(m.initialize(VERSION_CURRENT, 1024), Err(errors::PARAM));
    }

    #[test]
    fn initialize_with_container_at_min_ok() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, MIN_CONTAINER_SIZE).unwrap();
    }

    #[test]
    fn initialize_twice_is_busy() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.initialize(VERSION_CURRENT, CONTAINER_NONE), Err(errors::BUSY));
    }

    #[test]
    fn finalize_without_init_is_initialize() {
        let mut m = MusicExportManager::new();
        assert_eq!(m.finalize(), Err(errors::INITIALIZE));
    }

    #[test]
    fn finalize_while_exporting_is_busy() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.mp3", ok_param()).unwrap();
        assert_eq!(m.finalize(), Err(errors::BUSY));
    }

    #[test]
    fn from_file_without_init_is_initialize() {
        let mut m = MusicExportManager::new();
        assert_eq!(m.from_file("/dev_hdd0/a.mp3", ok_param()), Err(errors::INITIALIZE));
    }

    #[test]
    fn from_file_happy_path() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.mp3", ok_param()).unwrap();
        assert_eq!(m.state(), State::Exporting);
    }

    #[test]
    fn from_file_bad_path_rejected() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.from_file("/tmp/../evil", ok_param()), Err(errors::PARAM));
    }

    #[test]
    fn from_file_duplicate_is_busy() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.mp3", ok_param()).unwrap();
        assert_eq!(m.from_file("/dev_hdd0/b.mp3", ok_param()), Err(errors::BUSY));
    }

    #[test]
    fn from_file_long_artist_rejected() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        let mut p = ok_param();
        p.artist = "a".repeat(MUSIC_TITLE_MAX_LENGTH * 3);
        assert_eq!(m.from_file("/dev_hdd0/a.mp3", p), Err(errors::PARAM));
    }

    #[test]
    fn from_file_long_title_rejected() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        let mut p = ok_param();
        p.title = "t".repeat(MUSIC_TITLE_MAX_LENGTH * 3);
        assert_eq!(m.from_file("/dev_hdd0/a.mp3", p), Err(errors::PARAM));
    }

    #[test]
    fn from_file_long_genre_rejected() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        let mut p = ok_param();
        p.genre = "g".repeat(MUSIC_TITLE_MAX_LENGTH * 3);
        assert_eq!(m.from_file("/dev_hdd0/a.mp3", p), Err(errors::PARAM));
    }

    #[test]
    fn from_file_long_comment_rejected() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        let mut p = ok_param();
        p.game_comment = "c".repeat(GAME_COMMENT_MAX_SIZE);
        assert_eq!(m.from_file("/dev_hdd0/a.mp3", p), Err(errors::PARAM));
    }

    #[test]
    fn tick_progress_tracks_value() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.mp3", ok_param()).unwrap();
        m.tick_progress(0x8000).unwrap();
        assert_eq!(m.progress(), Ok(0x8000));
    }

    #[test]
    fn tick_progress_out_of_range_rejected() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.mp3", ok_param()).unwrap();
        assert_eq!(m.tick_progress(PROGRESS_MAX + 1), Err(errors::PARAM));
    }

    #[test]
    fn tick_progress_when_not_exporting_is_internal() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.tick_progress(1), Err(errors::INTERNAL));
    }

    #[test]
    fn progress_without_init_is_initialize() {
        let m = MusicExportManager::new();
        assert_eq!(m.progress(), Err(errors::INITIALIZE));
    }

    #[test]
    fn complete_export_returns_to_ready() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.mp3", ok_param()).unwrap();
        let dst = m.complete_export("a.mp3", |_| false).unwrap();
        assert_eq!(dst, "/dev_hdd0/music/a.mp3");
        assert_eq!(m.state(), State::Ready);
        assert_eq!(m.last_destination(), Some("/dev_hdd0/music/a.mp3"));
        assert_eq!(m.progress(), Ok(PROGRESS_MAX));
    }

    #[test]
    fn complete_export_empty_filename_rejected() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.mp3", ok_param()).unwrap();
        assert_eq!(m.complete_export("", |_| false), Err(errors::PARAM));
    }

    #[test]
    fn complete_without_active_export_is_internal() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.complete_export("x.mp3", |_| false), Err(errors::INTERNAL));
    }

    #[test]
    fn cancel_export_happy_path() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/a.mp3", ok_param()).unwrap();
        m.cancel_export().unwrap();
        assert_eq!(m.state(), State::Ready);
    }

    #[test]
    fn cancel_export_without_active_is_internal() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        assert_eq!(m.cancel_export(), Err(errors::INTERNAL));
    }

    #[test]
    fn full_export_lifecycle_smoke() {
        let mut m = MusicExportManager::new();
        m.initialize(VERSION_CURRENT, CONTAINER_NONE).unwrap();
        m.from_file("/dev_hdd0/recordings/theme.mp3", ok_param()).unwrap();
        m.tick_progress(0x4000).unwrap();
        m.tick_progress(0xC000).unwrap();
        let dst = m.complete_export("theme.mp3", |_| false).unwrap();
        assert!(dst.starts_with("/dev_hdd0/music/"));
        // Re-export a second file with collision suffix.
        m.from_file("/dev_hdd0/recordings/theme.mp3", ok_param()).unwrap();
        let dst2 = m.complete_export("theme.mp3", |p| p == "/dev_hdd0/music/theme.mp3").unwrap();
        assert_eq!(dst2, "/dev_hdd0/music/theme_0.mp3");
        m.finalize().unwrap();
    }
}
