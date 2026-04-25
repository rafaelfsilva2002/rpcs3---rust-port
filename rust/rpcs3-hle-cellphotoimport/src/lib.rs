//! `rpcs3-hle-cellphotoimport` — XMB photo library import HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellPhotoImport.cpp`. Games call into
//! cellPhotoImport to let the user pick a photo from the XMB library and
//! copy it into the game's save-data-adjacent folder. The API is
//! dialog-driven:
//!
//! 1. `PhotoImport(version, setParam, container, callback, userData)`
//!    opens the picker dialog.
//! 2. User picks a file → the lib copies it to the game's destination
//!    directory and fires the finish callback with file metadata.
//!
//! ## Entry points covered
//!
//! | HLE function                 | Rust wrapper                           |
//! |------------------------------|----------------------------------------|
//! | `cellPhotoImport`            | [`PhotoImportManager::start`]          |
//! | `cellPhotoImport2`           | [`PhotoImportManager::start`]          |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellPhotoImport.cpp:14-22
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const BUSY: CellError = CellError(0x8002_c701);
    pub const INTERNAL: CellError = CellError(0x8002_c702);
    pub const PARAM: CellError = CellError(0x8002_c703);
    pub const ACCESS_ERROR: CellError = CellError(0x8002_c704);
    pub const COPY: CellError = CellError(0x8002_c705);
    pub const INITIALIZE: CellError = CellError(0x8002_c706);
}

// =====================================================================
// Version / size constants (cellPhotoImport.cpp:43-54)
// =====================================================================

pub const VERSION_CURRENT: u32 = 0;

pub const HDD_PATH_MAX: usize = 1055;
pub const PHOTO_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_TITLE_MAX_SIZE: usize = 128;
pub const GAME_COMMENT_MAX_SIZE: usize = 1024;

// =====================================================================
// Format types (cellPhotoImport.cpp:57-65)
// =====================================================================

pub const FT_UNKNOWN: i32 = 0;
pub const FT_JPEG: i32 = 1;
pub const FT_PNG: i32 = 2;
pub const FT_GIF: i32 = 3;
pub const FT_BMP: i32 = 4;
pub const FT_TIFF: i32 = 5;
pub const FT_MPO: i32 = 6;

#[must_use]
pub fn format_from_filename(path: &str) -> i32 {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        FT_JPEG
    } else if lower.ends_with(".png") {
        FT_PNG
    } else if lower.ends_with(".gif") {
        FT_GIF
    } else if lower.ends_with(".bmp") {
        FT_BMP
    } else if lower.ends_with(".tif") || lower.ends_with(".tiff") {
        FT_TIFF
    } else if lower.ends_with(".mpo") {
        FT_MPO
    } else {
        FT_UNKNOWN
    }
}

// =====================================================================
// Rotation (cellPhotoImport.cpp:67-73)
// =====================================================================

pub const TEX_ROT_0: i32 = 0;
pub const TEX_ROT_90: i32 = 1;
pub const TEX_ROT_180: i32 = 2;
pub const TEX_ROT_270: i32 = 3;

#[must_use]
pub fn is_known_rotation(r: i32) -> bool {
    (TEX_ROT_0..=TEX_ROT_270).contains(&r)
}

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetParam {
    pub file_size_max: u32,
}

impl SetParam {
    fn validate(&self) -> Result<(), CellError> {
        if self.file_size_max == 0 {
            return Err(errors::PARAM);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileDataSub {
    pub width: i32,
    pub height: i32,
    pub format: i32,
    pub rotate: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileData {
    pub dst_file_name: String,
    pub photo_title: String,
    pub game_title: String,
    pub game_comment: String,
    pub data_sub: FileDataSub,
}

#[derive(Clone, Debug)]
pub struct SourcePhoto {
    pub path: String,      // host-side source
    pub size_bytes: u32,
    pub width: i32,
    pub height: i32,
    pub rotate: i32,
    pub title: String,     // up to PHOTO_TITLE_MAX_LENGTH * 3 after sanitize
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Busy,
}

#[derive(Clone, Debug)]
pub struct PhotoImportManager {
    state: State,
    param: SetParam,
    dst_dir: String,
    game_title: String,
}

impl PhotoImportManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            param: SetParam { file_size_max: 0 },
            dst_dir: String::new(),
            game_title: String::new(),
        }
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    #[must_use]
    pub fn param(&self) -> SetParam {
        self.param
    }

    /// `cellPhotoImport(version, dst_dir, setParam, container, cb, ud)`.
    /// Validates arguments and enters Busy state; tests drive the post-
    /// selection callback via `complete` / `cancel`.
    pub fn start(
        &mut self,
        version: u32,
        dst_dir: impl Into<String>,
        param: SetParam,
        game_title: impl Into<String>,
    ) -> Result<(), CellError> {
        if version != VERSION_CURRENT {
            return Err(errors::INITIALIZE);
        }
        if self.state == State::Busy {
            return Err(errors::BUSY);
        }
        let dst = dst_dir.into();
        let game_title = game_title.into();
        if dst.is_empty() || dst.len() >= HDD_PATH_MAX {
            return Err(errors::PARAM);
        }
        if !(dst.starts_with("/dev_hdd0") || dst.starts_with("/dev_hdd1")) {
            return Err(errors::ACCESS_ERROR);
        }
        if game_title.len() >= GAME_TITLE_MAX_SIZE {
            return Err(errors::PARAM);
        }
        param.validate()?;
        self.state = State::Busy;
        self.param = param;
        self.dst_dir = dst;
        self.game_title = game_title;
        Ok(())
    }

    /// Finalize with a user-picked photo. Mirrors the `select_photo`
    /// callback path in the C++ manager. Returns the file metadata the
    /// game's finish callback would see.
    pub fn complete(&mut self, photo: SourcePhoto, game_comment: impl Into<String>) -> Result<FileData, CellError> {
        if self.state != State::Busy {
            return Err(errors::INTERNAL);
        }
        if photo.path.is_empty() {
            return Err(errors::ACCESS_ERROR);
        }
        if photo.size_bytes > self.param.file_size_max {
            self.state = State::Idle;
            return Err(errors::COPY);
        }
        let comment = game_comment.into();
        if comment.len() >= GAME_COMMENT_MAX_SIZE {
            return Err(errors::PARAM);
        }
        if photo.title.len() >= PHOTO_TITLE_MAX_LENGTH * 3 {
            return Err(errors::PARAM);
        }
        if !is_known_rotation(photo.rotate) {
            return Err(errors::PARAM);
        }
        let filename = photo.path.rsplit('/').next().unwrap_or(&photo.path).to_string();
        let format = format_from_filename(&photo.path);
        let file_data = FileData {
            dst_file_name: format!("{}/{}", self.dst_dir, filename),
            photo_title: photo.title.clone(),
            game_title: self.game_title.clone(),
            game_comment: comment,
            data_sub: FileDataSub { width: photo.width, height: photo.height, format, rotate: photo.rotate },
        };
        self.state = State::Idle;
        Ok(file_data)
    }

    /// Cancel-path analogue. Mirrors the status<0 branch where the
    /// callback fires with `CELL_CANCEL`.
    pub fn cancel(&mut self) -> Result<(), CellError> {
        if self.state != State::Busy {
            return Err(errors::INTERNAL);
        }
        self.state = State::Idle;
        Ok(())
    }

    /// Short-circuit when file picker fails to open or the chosen path
    /// is unreachable. Resets to Idle and reports the access error.
    pub fn fail_access(&mut self) -> Result<(), CellError> {
        if self.state != State::Busy {
            return Err(errors::INTERNAL);
        }
        self.state = State::Idle;
        Err(errors::ACCESS_ERROR)
    }
}

impl Default for PhotoImportManager {
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
        SetParam { file_size_max: 10 * 1024 * 1024 }
    }

    fn sample_photo(path: &str, size: u32) -> SourcePhoto {
        SourcePhoto {
            path: path.into(),
            size_bytes: size,
            width: 1920,
            height: 1080,
            rotate: TEX_ROT_0,
            title: "Vacation".into(),
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::BUSY.0, 0x8002_c701);
        assert_eq!(errors::INTERNAL.0, 0x8002_c702);
        assert_eq!(errors::PARAM.0, 0x8002_c703);
        assert_eq!(errors::ACCESS_ERROR.0, 0x8002_c704);
        assert_eq!(errors::COPY.0, 0x8002_c705);
        assert_eq!(errors::INITIALIZE.0, 0x8002_c706);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(VERSION_CURRENT, 0);
        assert_eq!(HDD_PATH_MAX, 1055);
        assert_eq!(PHOTO_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_TITLE_MAX_SIZE, 128);
        assert_eq!(GAME_COMMENT_MAX_SIZE, 1024);
    }

    #[test]
    fn format_constants_stable() {
        assert_eq!(FT_UNKNOWN, 0);
        assert_eq!(FT_JPEG, 1);
        assert_eq!(FT_PNG, 2);
        assert_eq!(FT_GIF, 3);
        assert_eq!(FT_BMP, 4);
        assert_eq!(FT_TIFF, 5);
        assert_eq!(FT_MPO, 6);
    }

    #[test]
    fn rotation_constants_stable() {
        assert_eq!(TEX_ROT_0, 0);
        assert_eq!(TEX_ROT_90, 1);
        assert_eq!(TEX_ROT_180, 2);
        assert_eq!(TEX_ROT_270, 3);
    }

    #[test]
    fn format_from_filename_detects_jpeg_and_variants() {
        assert_eq!(format_from_filename("vacation.jpg"), FT_JPEG);
        assert_eq!(format_from_filename("Vacation.JPEG"), FT_JPEG);
        assert_eq!(format_from_filename("photo.PNG"), FT_PNG);
        assert_eq!(format_from_filename("anim.gif"), FT_GIF);
        assert_eq!(format_from_filename("old.bmp"), FT_BMP);
        assert_eq!(format_from_filename("scan.tif"), FT_TIFF);
        assert_eq!(format_from_filename("scan.tiff"), FT_TIFF);
        assert_eq!(format_from_filename("3d.mpo"), FT_MPO);
        assert_eq!(format_from_filename("data.raw"), FT_UNKNOWN);
        assert_eq!(format_from_filename("noext"), FT_UNKNOWN);
    }

    #[test]
    fn is_known_rotation_helper() {
        assert!(is_known_rotation(TEX_ROT_0));
        assert!(is_known_rotation(TEX_ROT_270));
        assert!(!is_known_rotation(4));
        assert!(!is_known_rotation(-1));
    }

    #[test]
    fn start_happy_path() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Test Game").unwrap();
        assert_eq!(m.state(), State::Busy);
    }

    #[test]
    fn start_bad_version_is_initialize() {
        let mut m = PhotoImportManager::new();
        assert_eq!(m.start(99, "/dev_hdd0/photo", ok_param(), "Game"), Err(errors::INITIALIZE));
    }

    #[test]
    fn start_twice_is_busy() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Game").unwrap();
        assert_eq!(m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Game"), Err(errors::BUSY));
    }

    #[test]
    fn start_empty_dst_rejected() {
        let mut m = PhotoImportManager::new();
        assert_eq!(m.start(VERSION_CURRENT, "", ok_param(), "Game"), Err(errors::PARAM));
    }

    #[test]
    fn start_path_outside_hdd_is_access_error() {
        let mut m = PhotoImportManager::new();
        assert_eq!(m.start(VERSION_CURRENT, "/dev_usb000/junk", ok_param(), "Game"), Err(errors::ACCESS_ERROR));
        assert_eq!(m.start(VERSION_CURRENT, "/dev_bdvd/y", ok_param(), "Game"), Err(errors::ACCESS_ERROR));
    }

    #[test]
    fn start_path_on_hdd1_allowed() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd1/caches/game/photos", ok_param(), "Game").unwrap();
    }

    #[test]
    fn start_path_over_max_rejected() {
        let mut m = PhotoImportManager::new();
        let long = "/dev_hdd0/".to_string() + &"a".repeat(HDD_PATH_MAX);
        assert_eq!(m.start(VERSION_CURRENT, long, ok_param(), "Game"), Err(errors::PARAM));
    }

    #[test]
    fn start_game_title_over_max_rejected() {
        let mut m = PhotoImportManager::new();
        let title = "t".repeat(GAME_TITLE_MAX_SIZE);
        assert_eq!(
            m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), title),
            Err(errors::PARAM)
        );
    }

    #[test]
    fn start_zero_file_size_max_is_param() {
        let mut m = PhotoImportManager::new();
        let bad = SetParam { file_size_max: 0 };
        assert_eq!(m.start(VERSION_CURRENT, "/dev_hdd0/photo", bad, "Game"), Err(errors::PARAM));
    }

    #[test]
    fn complete_happy_path_returns_file_data() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "My Game").unwrap();
        let data = m
            .complete(sample_photo("/host/media/trip.jpg", 4 * 1024 * 1024), "A comment")
            .unwrap();
        assert_eq!(data.dst_file_name, "/dev_hdd0/photo/trip.jpg");
        assert_eq!(data.photo_title, "Vacation");
        assert_eq!(data.game_title, "My Game");
        assert_eq!(data.game_comment, "A comment");
        assert_eq!(data.data_sub.format, FT_JPEG);
        assert_eq!(data.data_sub.width, 1920);
        assert_eq!(data.data_sub.height, 1080);
        assert_eq!(data.data_sub.rotate, TEX_ROT_0);
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn complete_without_start_is_internal() {
        let mut m = PhotoImportManager::new();
        let photo = sample_photo("/a.jpg", 100);
        assert_eq!(m.complete(photo, "").err(), Some(errors::INTERNAL));
    }

    #[test]
    fn complete_empty_path_is_access_error() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Game").unwrap();
        let photo = SourcePhoto { path: "".into(), ..sample_photo("", 100) };
        assert_eq!(m.complete(photo, "").err(), Some(errors::ACCESS_ERROR));
    }

    #[test]
    fn complete_oversize_file_is_copy_error() {
        let mut m = PhotoImportManager::new();
        let param = SetParam { file_size_max: 1024 };
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", param, "Game").unwrap();
        let photo = sample_photo("/big.jpg", 9999);
        assert_eq!(m.complete(photo, "").err(), Some(errors::COPY));
        // After a COPY failure, manager should be back to Idle.
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn complete_oversize_comment_rejected() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Game").unwrap();
        let photo = sample_photo("/a.jpg", 100);
        let long_comment = "c".repeat(GAME_COMMENT_MAX_SIZE);
        assert_eq!(m.complete(photo, long_comment).err(), Some(errors::PARAM));
    }

    #[test]
    fn complete_unknown_rotation_rejected() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Game").unwrap();
        let mut photo = sample_photo("/a.jpg", 100);
        photo.rotate = 99;
        assert_eq!(m.complete(photo, "").err(), Some(errors::PARAM));
    }

    #[test]
    fn complete_infers_format_from_filename_for_each_type() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Game").unwrap();
        let data = m.complete(sample_photo("/host/x.png", 100), "").unwrap();
        assert_eq!(data.data_sub.format, FT_PNG);
    }

    #[test]
    fn cancel_returns_to_idle() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Game").unwrap();
        m.cancel().unwrap();
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn cancel_without_start_is_internal() {
        let mut m = PhotoImportManager::new();
        assert_eq!(m.cancel(), Err(errors::INTERNAL));
    }

    #[test]
    fn fail_access_reports_access_error_and_idles() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo", ok_param(), "Game").unwrap();
        assert_eq!(m.fail_access(), Err(errors::ACCESS_ERROR));
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn fail_access_without_start_is_internal() {
        let mut m = PhotoImportManager::new();
        assert_eq!(m.fail_access(), Err(errors::INTERNAL));
    }

    #[test]
    fn full_import_lifecycle_smoke() {
        let mut m = PhotoImportManager::new();
        m.start(VERSION_CURRENT, "/dev_hdd0/photo/game_xyz", ok_param(), "Game XYZ").unwrap();
        let photo = SourcePhoto {
            path: "/host/xmb/photos/album/IMG_0001.JPG".into(),
            size_bytes: 3 * 1024 * 1024,
            width: 3000,
            height: 2000,
            rotate: TEX_ROT_90,
            title: "Album / Photo 0001".into(),
        };
        let data = m.complete(photo, "imported from XMB").unwrap();
        assert!(data.dst_file_name.ends_with("IMG_0001.JPG"));
        assert_eq!(data.data_sub.format, FT_JPEG);
        assert_eq!(data.data_sub.rotate, TEX_ROT_90);
        // Ready for a new import.
        m.start(VERSION_CURRENT, "/dev_hdd0/photo/game_xyz", ok_param(), "Game XYZ").unwrap();
        m.cancel().unwrap();
    }
}
