//! `rpcs3-hle-cellgame` — HLE surface of `cellGame.cpp`.
//!
//! Ports the entry-point HLE functions every PS3 game calls during
//! boot. Data sources (PARAM.SFO, mount paths, game type) are supplied
//! through the [`GameState`] trait — this crate itself owns no state.
//!
//! ## Scope (iteration 1)
//!
//! * `cellGameBootCheck` — reports game type, attributes, size, dirname.
//! * `cellGameContentPermit` — acknowledges boot, returns content paths.
//! * `cellGameDataCheck` — validates a GD or HG directory.
//! * `cellGameGetParamInt` — PSF integer parameter lookup.
//! * `cellGameGetParamString` — PSF string parameter lookup.
//! * `cellGamePatchCheck`, `cellGameGetSizeKB`, `cellGameDataGetSizeKB`
//!   (stubs returning `Ok` / well-known constants).

use rpcs3_emu_types::CellError;

// =====================================================================
// Cell errors specific to cellGame (cellGame.cpp defines these)
// =====================================================================

pub const CELL_GAME_ERROR_NOTFOUND: CellError = CellError(0x8002_CB04);
pub const CELL_GAME_ERROR_BROKEN: CellError = CellError(0x8002_CB07);
pub const CELL_GAME_ERROR_INTERNAL: CellError = CellError(0x8002_CB08);
pub const CELL_GAME_ERROR_PARAM: CellError = CellError(0x8002_CB09);
pub const CELL_GAME_ERROR_NOAPP: CellError = CellError(0x8002_CB0B);
pub const CELL_GAME_ERROR_ACCESS_ERROR: CellError = CellError(0x8002_CB0C);
pub const CELL_GAME_ERROR_NOSPACE: CellError = CellError(0x8002_CB0E);
pub const CELL_GAME_ERROR_NOTSUPPORTED: CellError = CellError(0x8002_CB0F);
pub const CELL_GAME_ERROR_BUSY: CellError = CellError(0x8002_CB14);
pub const CELL_GAME_ERROR_FAILURE: CellError = CellError(0x8002_CB15);

// =====================================================================
// Game-type constants (cellGame.h)
// =====================================================================

/// `CELL_GAME_GAMETYPE_*` values used by `cellGameBootCheck`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameType {
    Sys = 0,
    Disc = 1,
    Hdd = 2,
    GameData = 3,
    Home = 4,
}

/// `CELL_GAME_ATTRIBUTE_*` flag bits.
pub const ATTRIBUTE_PATCH: u32 = 0x1;
pub const ATTRIBUTE_APP_HOME: u32 = 0x2;
pub const ATTRIBUTE_DEBUG: u32 = 0x4;
pub const ATTRIBUTE_XMBBUY: u32 = 0x8;
pub const ATTRIBUTE_COMMERCE2_BROWSER: u32 = 0x10;
pub const ATTRIBUTE_INVITE_MESSAGE: u32 = 0x20;
pub const ATTRIBUTE_CUSTOM_DATA_MESSAGE: u32 = 0x40;
pub const ATTRIBUTE_WEB_BROWSER: u32 = 0x80;

// =====================================================================
// PSF parameter IDs — mirror cellGame.h `CELL_GAME_PARAMID_*`
// =====================================================================

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GameParamId {
    Title = 0,
    TitleDefault = 1,
    TitleJapanese = 2,
    TitleEnglish = 3,
    TitleFrench = 4,
    TitleSpanish = 5,
    TitleGerman = 6,
    TitleItalian = 7,
    TitleDutch = 8,
    TitlePortuguese = 9,
    TitleRussian = 10,
    TitleKorean = 11,
    TitleChineseT = 12,
    TitleChineseS = 13,
    TitleFinnish = 14,
    TitleSwedish = 15,
    TitleDanish = 16,
    TitleNorwegian = 17,
    TitlePolish = 18,
    TitleBrazilianPortuguese = 19,
    TitleId = 100,
    Version = 101,
    AppVersion = 102,
    ParentalLevel = 103,
    Resolution = 104,
    SoundFormat = 105,
    PsBootable = 106,
    Remoteplay = 107,
    PsVitaTitleId = 108,
    Unknown = -1,
}

impl GameParamId {
    #[must_use]
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::Title,
            1 => Self::TitleDefault,
            2 => Self::TitleJapanese,
            3 => Self::TitleEnglish,
            4 => Self::TitleFrench,
            5 => Self::TitleSpanish,
            6 => Self::TitleGerman,
            7 => Self::TitleItalian,
            8 => Self::TitleDutch,
            9 => Self::TitlePortuguese,
            10 => Self::TitleRussian,
            11 => Self::TitleKorean,
            12 => Self::TitleChineseT,
            13 => Self::TitleChineseS,
            14 => Self::TitleFinnish,
            15 => Self::TitleSwedish,
            16 => Self::TitleDanish,
            17 => Self::TitleNorwegian,
            18 => Self::TitlePolish,
            19 => Self::TitleBrazilianPortuguese,
            100 => Self::TitleId,
            101 => Self::Version,
            102 => Self::AppVersion,
            103 => Self::ParentalLevel,
            104 => Self::Resolution,
            105 => Self::SoundFormat,
            106 => Self::PsBootable,
            107 => Self::Remoteplay,
            108 => Self::PsVitaTitleId,
            _ => Self::Unknown,
        }
    }

    /// True if this param id holds a string value. False = integer.
    #[must_use]
    pub const fn is_string(self) -> bool {
        !matches!(
            self,
            Self::ParentalLevel
                | Self::Resolution
                | Self::SoundFormat
                | Self::PsBootable
                | Self::Remoteplay
                | Self::Unknown
        )
    }
}

// =====================================================================
// Data returned by cellGameBootCheck / ContentPermit
// =====================================================================

/// Result of `cellGameBootCheck` — the caller writes these four values
/// into guest memory pointers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootCheck {
    pub game_type: GameType,
    pub attributes: u32,
    pub size_kb: u64,
    /// Game directory name (max 31 chars + NUL).
    pub dir_name: String,
}

/// Result of `cellGameContentPermit` — paths for subsequent fs I/O.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentPermit {
    pub content_info_path: String,
    pub usrdir_path: String,
}

pub const CELL_GAME_DIRNAME_SIZE: usize = 32;
pub const CELL_GAME_PATH_MAX: usize = 128;

// =====================================================================
// GameState trait — emu core provides it
// =====================================================================

/// Everything `cellGame*` needs about the running title. The emu core
/// ties this to the resolved PARAM.SFO of the booting game.
pub trait GameState {
    fn game_type(&self) -> GameType;
    fn attributes(&self) -> u32;
    fn size_kb(&self) -> u64;
    /// Directory name under `/dev_hdd0/game/` — typically the title id.
    fn dir_name(&self) -> &str;
    fn content_info_path(&self) -> &str;
    fn usrdir_path(&self) -> &str;

    /// Lookup a PSF string parameter. Returns `None` if not present.
    fn psf_string(&self, id: GameParamId) -> Option<&str>;
    /// Lookup a PSF integer parameter.
    fn psf_int(&self, id: GameParamId) -> Option<i32>;

    /// True if the game data directory for `dir_name` exists on disk
    /// (used by `cellGameDataCheck`).
    fn game_data_exists(&self, dir_name: &str) -> bool;
}

// =====================================================================
// HLE functions
// =====================================================================

/// `cellGameBootCheck(type*, attributes*, size*, dirName*)`.
/// Mirrors cellGame.cpp:742.
#[must_use]
pub fn cell_game_boot_check<G: GameState + ?Sized>(game: &G) -> BootCheck {
    BootCheck {
        game_type: game.game_type(),
        attributes: game.attributes(),
        size_kb: game.size_kb(),
        dir_name: truncate_dirname(game.dir_name()),
    }
}

/// `cellGameContentPermit(contentInfoPath*, usrdirPath*)`.
/// Mirrors cellGame.cpp:943.
#[must_use]
pub fn cell_game_content_permit<G: GameState + ?Sized>(game: &G) -> ContentPermit {
    ContentPermit {
        content_info_path: game.content_info_path().to_owned(),
        usrdir_path: game.usrdir_path().to_owned(),
    }
}

/// `cellGameDataCheck(type, dirName, size*)`.
/// Mirrors cellGame.cpp:864. Fails with `CELL_GAME_ERROR_PARAM` if
/// `type` isn't `GameData` or `Hdd`, or dir_name is empty/too long.
#[must_use]
pub fn cell_game_data_check<G: GameState + ?Sized>(
    game: &G,
    type_val: u32,
    dir_name: &str,
) -> Result<u64, CellError> {
    let Ok(kind) = game_type_from_u32(type_val) else {
        return Err(CELL_GAME_ERROR_PARAM);
    };
    if !matches!(kind, GameType::GameData | GameType::Hdd) {
        return Err(CELL_GAME_ERROR_PARAM);
    }
    if dir_name.is_empty() || dir_name.len() >= CELL_GAME_DIRNAME_SIZE {
        return Err(CELL_GAME_ERROR_PARAM);
    }
    if !game.game_data_exists(dir_name) {
        return Err(CELL_GAME_ERROR_NOTFOUND);
    }
    Ok(game.size_kb())
}

/// `cellGameGetParamInt(id, value*)`.
/// Mirrors cellGame.cpp:1393.
#[must_use]
pub fn cell_game_get_param_int<G: GameState + ?Sized>(
    game: &G,
    id: i32,
) -> Result<i32, CellError> {
    let pid = GameParamId::from_i32(id);
    if matches!(pid, GameParamId::Unknown) {
        return Err(CELL_GAME_ERROR_PARAM);
    }
    if pid.is_string() {
        // C++ returns CELL_GAME_ERROR_PARAM when the caller asks for
        // int on a string-typed id.
        return Err(CELL_GAME_ERROR_PARAM);
    }
    game.psf_int(pid).ok_or(CELL_GAME_ERROR_PARAM)
}

/// `cellGameGetParamString(id, buf, bufsize)` — returns the string;
/// caller is responsible for copying into guest memory with truncation.
///
/// Mirrors cellGame.cpp:1521.
#[must_use]
pub fn cell_game_get_param_string<'a, G: GameState + ?Sized>(
    game: &'a G,
    id: i32,
    bufsize: u32,
) -> Result<&'a str, CellError> {
    if bufsize == 0 {
        return Err(CELL_GAME_ERROR_PARAM);
    }
    let pid = GameParamId::from_i32(id);
    if matches!(pid, GameParamId::Unknown) {
        return Err(CELL_GAME_ERROR_PARAM);
    }
    if !pid.is_string() {
        return Err(CELL_GAME_ERROR_PARAM);
    }
    game.psf_string(pid).ok_or(CELL_GAME_ERROR_PARAM)
}

/// `cellGamePatchCheck(size*, reserved)`.
/// Mirrors cellGame.cpp:824. Stub returning game size; caller may add
/// further patch-state checks.
#[must_use]
pub fn cell_game_patch_check<G: GameState + ?Sized>(game: &G) -> Result<u64, CellError> {
    Ok(game.size_kb())
}

/// `cellGameGetSizeKB(size*)`.
/// Mirrors cellGame.cpp:1604.
#[must_use]
pub fn cell_game_get_size_kb<G: GameState + ?Sized>(game: &G) -> i32 {
    // C++ returns a truncated s32.
    let kb = game.size_kb().min(i32::MAX as u64);
    kb as i32
}

// =====================================================================
// Helpers
// =====================================================================

fn truncate_dirname(s: &str) -> String {
    if s.len() >= CELL_GAME_DIRNAME_SIZE {
        s[..CELL_GAME_DIRNAME_SIZE - 1].to_owned()
    } else {
        s.to_owned()
    }
}

fn game_type_from_u32(v: u32) -> Result<GameType, ()> {
    match v {
        0 => Ok(GameType::Sys),
        1 => Ok(GameType::Disc),
        2 => Ok(GameType::Hdd),
        3 => Ok(GameType::GameData),
        4 => Ok(GameType::Home),
        _ => Err(()),
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    #[derive(Default)]
    struct TestGame {
        game_type: Option<GameType>,
        attributes: u32,
        size_kb: u64,
        dir_name: String,
        content_info_path: String,
        usrdir_path: String,
        psf_strings: HashMap<i32, String>,
        psf_ints: HashMap<i32, i32>,
        data_dirs: HashSet<String>,
    }

    impl GameState for TestGame {
        fn game_type(&self) -> GameType {
            self.game_type.unwrap_or(GameType::Hdd)
        }
        fn attributes(&self) -> u32 {
            self.attributes
        }
        fn size_kb(&self) -> u64 {
            self.size_kb
        }
        fn dir_name(&self) -> &str {
            &self.dir_name
        }
        fn content_info_path(&self) -> &str {
            &self.content_info_path
        }
        fn usrdir_path(&self) -> &str {
            &self.usrdir_path
        }
        fn psf_string(&self, id: GameParamId) -> Option<&str> {
            self.psf_strings.get(&(id as i32)).map(String::as_str)
        }
        fn psf_int(&self, id: GameParamId) -> Option<i32> {
            self.psf_ints.get(&(id as i32)).copied()
        }
        fn game_data_exists(&self, dir_name: &str) -> bool {
            self.data_dirs.contains(dir_name)
        }
    }

    fn demo_game() -> TestGame {
        let mut g = TestGame::default();
        g.game_type = Some(GameType::Hdd);
        g.attributes = ATTRIBUTE_PATCH | ATTRIBUTE_WEB_BROWSER;
        g.size_kb = 4096;
        g.dir_name = "BLES01234".into();
        g.content_info_path = "/dev_hdd0/game/BLES01234".into();
        g.usrdir_path = "/dev_hdd0/game/BLES01234/USRDIR".into();
        g.psf_strings.insert(GameParamId::Title as i32, "Demo Title".into());
        g.psf_strings.insert(GameParamId::TitleId as i32, "BLES01234".into());
        g.psf_strings.insert(GameParamId::AppVersion as i32, "01.00".into());
        g.psf_ints.insert(GameParamId::ParentalLevel as i32, 5);
        g.psf_ints.insert(GameParamId::Resolution as i32, 0x3F);
        g.data_dirs.insert("DATA_DIR1".into());
        g
    }

    // -- BootCheck -------------------------------------------------

    #[test]
    fn boot_check_reports_known_fields() {
        let g = demo_game();
        let r = cell_game_boot_check(&g);
        assert_eq!(r.game_type, GameType::Hdd);
        assert_eq!(r.attributes, 0x81);
        assert_eq!(r.size_kb, 4096);
        assert_eq!(r.dir_name, "BLES01234");
    }

    #[test]
    fn boot_check_truncates_dirname() {
        let mut g = demo_game();
        g.dir_name = "A".repeat(CELL_GAME_DIRNAME_SIZE + 5);
        let r = cell_game_boot_check(&g);
        assert_eq!(r.dir_name.len(), CELL_GAME_DIRNAME_SIZE - 1);
    }

    // -- ContentPermit --------------------------------------------

    #[test]
    fn content_permit_returns_paths() {
        let g = demo_game();
        let r = cell_game_content_permit(&g);
        assert_eq!(r.content_info_path, "/dev_hdd0/game/BLES01234");
        assert_eq!(r.usrdir_path, "/dev_hdd0/game/BLES01234/USRDIR");
    }

    // -- DataCheck ------------------------------------------------

    #[test]
    fn data_check_bad_type_is_param_error() {
        let g = demo_game();
        // type=0 (Sys) is not a valid target for DataCheck.
        assert_eq!(
            cell_game_data_check(&g, GameType::Sys as u32, "DATA_DIR1"),
            Err(CELL_GAME_ERROR_PARAM)
        );
        // type=99 (unknown)
        assert_eq!(
            cell_game_data_check(&g, 99, "DATA_DIR1"),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    #[test]
    fn data_check_empty_dirname_is_param_error() {
        let g = demo_game();
        assert_eq!(
            cell_game_data_check(&g, GameType::GameData as u32, ""),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    #[test]
    fn data_check_missing_dir_is_notfound() {
        let g = demo_game();
        assert_eq!(
            cell_game_data_check(&g, GameType::GameData as u32, "UNKNOWN"),
            Err(CELL_GAME_ERROR_NOTFOUND)
        );
    }

    #[test]
    fn data_check_existing_dir_returns_size() {
        let g = demo_game();
        assert_eq!(
            cell_game_data_check(&g, GameType::GameData as u32, "DATA_DIR1"),
            Ok(4096)
        );
    }

    // -- GetParamInt / String ------------------------------------

    #[test]
    fn get_param_int_returns_stored_value() {
        let g = demo_game();
        assert_eq!(
            cell_game_get_param_int(&g, GameParamId::ParentalLevel as i32),
            Ok(5)
        );
    }

    #[test]
    fn get_param_int_rejects_string_id() {
        let g = demo_game();
        assert_eq!(
            cell_game_get_param_int(&g, GameParamId::TitleId as i32),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    #[test]
    fn get_param_int_unknown_id_is_param() {
        let g = demo_game();
        assert_eq!(cell_game_get_param_int(&g, 9999), Err(CELL_GAME_ERROR_PARAM));
    }

    #[test]
    fn get_param_int_missing_value_is_param() {
        let g = TestGame::default();
        assert_eq!(
            cell_game_get_param_int(&g, GameParamId::Resolution as i32),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    #[test]
    fn get_param_string_returns_title() {
        let g = demo_game();
        assert_eq!(
            cell_game_get_param_string(&g, GameParamId::Title as i32, 128),
            Ok("Demo Title")
        );
    }

    #[test]
    fn get_param_string_zero_buf_is_param() {
        let g = demo_game();
        assert_eq!(
            cell_game_get_param_string(&g, GameParamId::Title as i32, 0),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    #[test]
    fn get_param_string_rejects_int_id() {
        let g = demo_game();
        assert_eq!(
            cell_game_get_param_string(&g, GameParamId::ParentalLevel as i32, 128),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    // -- GetSizeKB / PatchCheck ----------------------------------

    #[test]
    fn get_size_kb_returns_game_size() {
        let g = demo_game();
        assert_eq!(cell_game_get_size_kb(&g), 4096);
    }

    #[test]
    fn patch_check_returns_size_ok() {
        let g = demo_game();
        assert_eq!(cell_game_patch_check(&g), Ok(4096));
    }

    // -- Constants + param id classification ---------------------

    #[test]
    fn game_type_ordinals_frozen() {
        assert_eq!(GameType::Sys as u32, 0);
        assert_eq!(GameType::Disc as u32, 1);
        assert_eq!(GameType::Hdd as u32, 2);
        assert_eq!(GameType::GameData as u32, 3);
        assert_eq!(GameType::Home as u32, 4);
    }

    #[test]
    fn attribute_flag_values_frozen() {
        assert_eq!(ATTRIBUTE_PATCH, 0x1);
        assert_eq!(ATTRIBUTE_APP_HOME, 0x2);
        assert_eq!(ATTRIBUTE_DEBUG, 0x4);
        assert_eq!(ATTRIBUTE_XMBBUY, 0x8);
        assert_eq!(ATTRIBUTE_WEB_BROWSER, 0x80);
    }

    #[test]
    fn param_id_classification() {
        // String params
        assert!(GameParamId::Title.is_string());
        assert!(GameParamId::TitleId.is_string());
        assert!(GameParamId::AppVersion.is_string());
        // Int params
        assert!(!GameParamId::ParentalLevel.is_string());
        assert!(!GameParamId::Resolution.is_string());
        assert!(!GameParamId::PsBootable.is_string());
    }

    #[test]
    fn error_codes_are_in_cellgame_facility() {
        for e in [
            CELL_GAME_ERROR_NOTFOUND,
            CELL_GAME_ERROR_BROKEN,
            CELL_GAME_ERROR_PARAM,
            CELL_GAME_ERROR_NOSPACE,
        ] {
            // All cellGame errors start with 0x8002CB__.
            assert_eq!(e.0 & 0xFFFF_FF00, 0x8002_CB00);
        }
    }
}
