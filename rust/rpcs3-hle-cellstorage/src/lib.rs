//! `rpcs3-hle-cellstorage` — USB storage data import/export HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellStorage.cpp`. Games use the
//! `cellStorageData*` surface to copy a single file between the PS3
//! HDD and a USB mass-storage device. Only two files are canonical:
//! `IMPORT.BIN` (USB → HDD) and `EXPORT.BIN` (HDD → USB).
//!
//! ## Entry points covered
//!
//! | HLE function                   | Rust wrapper                          |
//! |--------------------------------|---------------------------------------|
//! | `cellStorageDataImport`        | [`StorageManager::import`]            |
//! | `cellStorageDataExport`        | [`StorageManager::export`]            |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellStorage.h:3-10
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const BUSY: CellError = CellError(0x8002_be01);
    pub const INTERNAL: CellError = CellError(0x8002_be02);
    pub const PARAM: CellError = CellError(0x8002_be03);
    pub const ACCESS_ERROR: CellError = CellError(0x8002_be04);
    pub const FAILURE: CellError = CellError(0x8002_be05);
}

// =====================================================================
// Version + size constants (cellStorage.h:12-28)
// =====================================================================

pub const VERSION_CURRENT: u32 = 0;
pub const VERSION_DST_FILENAME: u32 = 1;

pub const HDD_PATH_MAX: usize = 1055;
pub const MEDIA_PATH_MAX: usize = 1024;
pub const FILENAME_MAX: usize = 64;
pub const FILESIZE_MAX: u64 = 1024 * 1024 * 1024; // 1 GiB
pub const TITLE_MAX: usize = 256;

pub const IMPORT_FILENAME: &str = "IMPORT.BIN";
pub const EXPORT_FILENAME: &str = "EXPORT.BIN";

// =====================================================================
// Types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetParam {
    pub file_size_max: u32,
    pub title: String,
}

impl SetParam {
    fn validate(&self) -> Result<(), CellError> {
        if self.file_size_max == 0 {
            return Err(errors::PARAM);
        }
        if u64::from(self.file_size_max) > FILESIZE_MAX {
            return Err(errors::PARAM);
        }
        if self.title.is_empty() || self.title.len() > TITLE_MAX {
            return Err(errors::PARAM);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Import, // USB → HDD
    Export, // HDD → USB
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Busy,
}

#[derive(Clone, Debug)]
pub struct StorageManager {
    state: State,
}

impl StorageManager {
    #[must_use]
    pub fn new() -> Self {
        Self { state: State::Idle }
    }

    #[must_use]
    pub fn state(&self) -> State {
        self.state
    }

    /// `check_path` — paths must be ≤ HDD_PATH_MAX, live under one of
    /// the HDD/USB roots, and have no `..` traversal.
    #[must_use]
    pub fn check_hdd_path(path: &str) -> bool {
        if path.is_empty() || path.len() > HDD_PATH_MAX {
            return false;
        }
        if !path.starts_with("/dev_hdd0") && !path.starts_with("/dev_hdd1") {
            return false;
        }
        !path.contains("..")
    }

    #[must_use]
    pub fn check_media_path(path: &str) -> bool {
        if path.is_empty() || path.len() > MEDIA_PATH_MAX {
            return false;
        }
        if !(path.starts_with("/dev_usb") || path.starts_with("/dev_ms") || path.starts_with("/dev_cf")) {
            return false;
        }
        !path.contains("..")
    }

    #[must_use]
    pub fn check_filename(name: &str) -> bool {
        if name.is_empty() || name.len() > FILENAME_MAX {
            return false;
        }
        // ASCII only, no separators, no traversal.
        for c in name.chars() {
            let ok = c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.';
            if !ok {
                return false;
            }
        }
        !name.contains("..")
    }

    /// `cellStorageDataImport(version, media_path, setParam, cb, ud)`.
    /// Copies `IMPORT.BIN` from the USB device at `media_path` into the
    /// game's HDD save slot. Returns the destination path on success.
    pub fn import(
        &mut self,
        version: u32,
        media_path: &str,
        hdd_dst_dir: &str,
        param: &SetParam,
    ) -> Result<String, CellError> {
        self.begin(version, Direction::Import, media_path, hdd_dst_dir, param)
    }

    /// `cellStorageDataExport(version, media_path, setParam, cb, ud)`.
    pub fn export(
        &mut self,
        version: u32,
        media_path: &str,
        hdd_src_dir: &str,
        param: &SetParam,
    ) -> Result<String, CellError> {
        self.begin(version, Direction::Export, media_path, hdd_src_dir, param)
    }

    fn begin(
        &mut self,
        version: u32,
        direction: Direction,
        media_path: &str,
        hdd_path: &str,
        param: &SetParam,
    ) -> Result<String, CellError> {
        if self.state == State::Busy {
            return Err(errors::BUSY);
        }
        if version != VERSION_CURRENT && version != VERSION_DST_FILENAME {
            return Err(errors::PARAM);
        }
        if !Self::check_media_path(media_path) {
            return Err(errors::ACCESS_ERROR);
        }
        if !Self::check_hdd_path(hdd_path) {
            return Err(errors::ACCESS_ERROR);
        }
        param.validate()?;

        self.state = State::Busy;

        let filename = match direction {
            Direction::Import => IMPORT_FILENAME,
            Direction::Export => EXPORT_FILENAME,
        };
        let full = match direction {
            Direction::Import => format!("{hdd_path}/{filename}"),
            Direction::Export => format!("{media_path}/{filename}"),
        };
        Ok(full)
    }

    /// Test hook: signal that the current transfer failed (e.g. media
    /// not present, full). The C++ callback surface reports FAILURE.
    pub fn complete_with_failure(&mut self) -> Result<(), CellError> {
        if self.state != State::Busy {
            return Err(errors::INTERNAL);
        }
        self.state = State::Idle;
        Err(errors::FAILURE)
    }

    /// Happy-path completion — transition Busy → Idle.
    pub fn complete(&mut self) -> Result<(), CellError> {
        if self.state != State::Busy {
            return Err(errors::INTERNAL);
        }
        self.state = State::Idle;
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), CellError> {
        if self.state != State::Busy {
            return Err(errors::INTERNAL);
        }
        self.state = State::Idle;
        Ok(())
    }
}

impl Default for StorageManager {
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
        SetParam { file_size_max: 1_000_000, title: "Save File".into() }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::BUSY.0, 0x8002_be01);
        assert_eq!(errors::INTERNAL.0, 0x8002_be02);
        assert_eq!(errors::PARAM.0, 0x8002_be03);
        assert_eq!(errors::ACCESS_ERROR.0, 0x8002_be04);
        assert_eq!(errors::FAILURE.0, 0x8002_be05);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(VERSION_CURRENT, 0);
        assert_eq!(VERSION_DST_FILENAME, 1);
        assert_eq!(HDD_PATH_MAX, 1055);
        assert_eq!(MEDIA_PATH_MAX, 1024);
        assert_eq!(FILENAME_MAX, 64);
        assert_eq!(FILESIZE_MAX, 1024 * 1024 * 1024);
        assert_eq!(TITLE_MAX, 256);
    }

    #[test]
    fn canonical_filenames_stable() {
        assert_eq!(IMPORT_FILENAME, "IMPORT.BIN");
        assert_eq!(EXPORT_FILENAME, "EXPORT.BIN");
    }

    #[test]
    fn check_hdd_path_whitelist() {
        assert!(StorageManager::check_hdd_path("/dev_hdd0/game/slot1"));
        assert!(StorageManager::check_hdd_path("/dev_hdd1/cache"));
        assert!(!StorageManager::check_hdd_path("/dev_usb/thing"));
        assert!(!StorageManager::check_hdd_path("/tmp/x"));
        assert!(!StorageManager::check_hdd_path(""));
        assert!(!StorageManager::check_hdd_path("/dev_hdd0/../secret"));
    }

    #[test]
    fn check_hdd_path_too_long_rejected() {
        let long = "/dev_hdd0/".to_string() + &"a".repeat(HDD_PATH_MAX);
        assert!(!StorageManager::check_hdd_path(&long));
    }

    #[test]
    fn check_media_path_whitelist() {
        assert!(StorageManager::check_media_path("/dev_usb000/games"));
        assert!(StorageManager::check_media_path("/dev_ms/photos"));
        assert!(StorageManager::check_media_path("/dev_cf/raw"));
        assert!(!StorageManager::check_media_path("/dev_hdd0/x"));
        assert!(!StorageManager::check_media_path("/dev_bdvd/x"));
        assert!(!StorageManager::check_media_path(""));
        assert!(!StorageManager::check_media_path("/dev_usb000/../up"));
    }

    #[test]
    fn check_filename_ascii_only() {
        assert!(StorageManager::check_filename("IMPORT.BIN"));
        assert!(StorageManager::check_filename("save_01.dat"));
        assert!(StorageManager::check_filename("a-b-c.txt"));
        assert!(!StorageManager::check_filename(""));
        assert!(!StorageManager::check_filename("file with space.bin"));
        assert!(!StorageManager::check_filename("../up"));
        assert!(!StorageManager::check_filename("ファイル.bin"));
    }

    #[test]
    fn check_filename_too_long_rejected() {
        let long = "a".repeat(FILENAME_MAX + 1);
        assert!(!StorageManager::check_filename(&long));
    }

    #[test]
    fn set_param_validate_zero_size_rejected() {
        let mut p = ok_param();
        p.file_size_max = 0;
        assert_eq!(p.validate(), Err(errors::PARAM));
    }

    #[test]
    fn set_param_validate_empty_title_rejected() {
        let mut p = ok_param();
        p.title = String::new();
        assert_eq!(p.validate(), Err(errors::PARAM));
    }

    #[test]
    fn set_param_validate_oversized_title_rejected() {
        let mut p = ok_param();
        p.title = "a".repeat(TITLE_MAX + 1);
        assert_eq!(p.validate(), Err(errors::PARAM));
    }

    #[test]
    fn import_happy_path_returns_hdd_destination() {
        let mut m = StorageManager::new();
        let dst = m.import(VERSION_CURRENT, "/dev_usb000/saves", "/dev_hdd0/game/slot", &ok_param()).unwrap();
        assert_eq!(dst, "/dev_hdd0/game/slot/IMPORT.BIN");
        assert_eq!(m.state(), State::Busy);
    }

    #[test]
    fn export_happy_path_returns_media_destination() {
        let mut m = StorageManager::new();
        let dst = m.export(VERSION_CURRENT, "/dev_usb000/saves", "/dev_hdd0/game/slot", &ok_param()).unwrap();
        assert_eq!(dst, "/dev_usb000/saves/EXPORT.BIN");
        assert_eq!(m.state(), State::Busy);
    }

    #[test]
    fn import_bad_version_rejected() {
        let mut m = StorageManager::new();
        assert_eq!(
            m.import(99, "/dev_usb000/x", "/dev_hdd0/y", &ok_param()),
            Err(errors::PARAM)
        );
    }

    #[test]
    fn import_bad_media_path_is_access_error() {
        let mut m = StorageManager::new();
        assert_eq!(
            m.import(VERSION_CURRENT, "/tmp/x", "/dev_hdd0/y", &ok_param()),
            Err(errors::ACCESS_ERROR)
        );
    }

    #[test]
    fn import_bad_hdd_path_is_access_error() {
        let mut m = StorageManager::new();
        assert_eq!(
            m.import(VERSION_CURRENT, "/dev_usb000/x", "/tmp/y", &ok_param()),
            Err(errors::ACCESS_ERROR)
        );
    }

    #[test]
    fn import_twice_is_busy() {
        let mut m = StorageManager::new();
        m.import(VERSION_CURRENT, "/dev_usb000/x", "/dev_hdd0/y", &ok_param()).unwrap();
        assert_eq!(
            m.import(VERSION_CURRENT, "/dev_usb000/z", "/dev_hdd0/w", &ok_param()),
            Err(errors::BUSY)
        );
    }

    #[test]
    fn import_traversal_attempt_is_access_error() {
        let mut m = StorageManager::new();
        assert_eq!(
            m.import(VERSION_CURRENT, "/dev_usb000/../etc", "/dev_hdd0/ok", &ok_param()),
            Err(errors::ACCESS_ERROR)
        );
    }

    #[test]
    fn complete_returns_to_idle() {
        let mut m = StorageManager::new();
        m.import(VERSION_CURRENT, "/dev_usb000/x", "/dev_hdd0/y", &ok_param()).unwrap();
        m.complete().unwrap();
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn complete_without_busy_is_internal() {
        let mut m = StorageManager::new();
        assert_eq!(m.complete(), Err(errors::INTERNAL));
    }

    #[test]
    fn complete_with_failure_returns_to_idle_and_reports() {
        let mut m = StorageManager::new();
        m.import(VERSION_CURRENT, "/dev_usb000/x", "/dev_hdd0/y", &ok_param()).unwrap();
        assert_eq!(m.complete_with_failure(), Err(errors::FAILURE));
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn cancel_returns_to_idle() {
        let mut m = StorageManager::new();
        m.import(VERSION_CURRENT, "/dev_usb000/x", "/dev_hdd0/y", &ok_param()).unwrap();
        m.cancel().unwrap();
        assert_eq!(m.state(), State::Idle);
    }

    #[test]
    fn cancel_without_busy_is_internal() {
        let mut m = StorageManager::new();
        assert_eq!(m.cancel(), Err(errors::INTERNAL));
    }

    #[test]
    fn version_dst_filename_accepted() {
        let mut m = StorageManager::new();
        m.import(VERSION_DST_FILENAME, "/dev_usb000/x", "/dev_hdd0/y", &ok_param()).unwrap();
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut m = StorageManager::new();
        let import_dst = m
            .import(VERSION_CURRENT, "/dev_usb000/ps3/saves", "/dev_hdd0/game/MYGAME00001", &ok_param())
            .unwrap();
        assert!(import_dst.ends_with("/IMPORT.BIN"));
        m.complete().unwrap();
        let export_dst = m
            .export(VERSION_CURRENT, "/dev_usb000/ps3/saves", "/dev_hdd0/game/MYGAME00001", &ok_param())
            .unwrap();
        assert!(export_dst.ends_with("/EXPORT.BIN"));
        m.complete().unwrap();
        // After failure the manager must be re-usable.
        m.import(VERSION_CURRENT, "/dev_usb000/ps3/saves", "/dev_hdd0/game/MYGAME00001", &ok_param()).unwrap();
        assert_eq!(m.complete_with_failure(), Err(errors::FAILURE));
        assert_eq!(m.state(), State::Idle);
    }
}
