//! Rust port of `rpcs3/Emu/Cell/Modules/cellGameExec.cpp`.
//!
//! 10 PRX entry points under the module name `cellGameExec`. Most are
//! hooks for PlayStation Home (defunct) or chained-game launch; the
//! firmware stubs them out. Two do real work:
//!
//!  - `cellGameSetExitParam(execdata)` stores a `u32` that the next
//!    game in the chain reads via `cellGameGetBootGameInfo`.
//!  - `cellGameGetBootGameInfo(&type, &dir, &execdata)` publishes the
//!    current boot source (`DISC` / `HDD`) and, on HDD, copies the
//!    `Emu.GetDir()` name (≤ `CELL_GAME_DIRNAME_SIZE` bytes).
//!
//! The module name is byte-exact at cpp:136
//! `DECLARE(ppu_module_manager::cellGameExec)("cellGameExec", ...)`.
//!
//! Error codes come from `cellGame.h` (already declared but re-exported
//! here to keep the crate independent of `rpcs3-hle-cellgame`):
//!
//! | name                            | value           |
//! |---------------------------------|-----------------|
//! | `CELL_GAME_ERROR_PARAM`         | `0x8002_CB07`   |
//! | `CELL_GAME_ERROR_NOAPP`         | `0x8002_CB08`   |
//! | `CELL_HDDGAME_ERROR_INTERNAL`   | `0x8002_BA03`   |

#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::string::String;
use rpcs3_emu_types::CellError;

/// Byte-exact at cpp:136.
pub const MODULE_NAME: &str = "cellGameExec";

/// REG_FUNC order at cpp:138-147.
pub const REGISTERED_ENTRY_POINTS: &[&str] = &[
    "cellGameSetExitParam",
    "cellGameGetHomeDataExportPath",
    "cellGameGetHomePath",
    "cellGameGetHomeDataImportPath",
    "cellGameGetHomeLaunchOptionPath",
    "cellGameExecGame",
    "cellGameDeleteGame",
    "cellGameGetBootGameInfo",
    "cellGameGetExitGameInfo",
    "cellGameGetList",
];

// --- Error codes (byte-exact cellGame.h) --------------------------------

pub const CELL_GAME_ERROR_PARAM: CellError = CellError(0x8002_CB07);
pub const CELL_GAME_ERROR_NOAPP: CellError = CellError(0x8002_CB08);
pub const CELL_HDDGAME_ERROR_INTERNAL: CellError = CellError(0x8002_BA03);

// --- Constants (cellGame.h:55/67-68) ------------------------------------

/// Max bytes (including NUL) that fit in the directory-name buffer the
/// firmware hands to `cellGameGetBootGameInfo`. A C string whose length
/// is `>= CELL_GAME_DIRNAME_SIZE` cannot be copied and triggers
/// `CELL_HDDGAME_ERROR_INTERNAL` at cpp:113-115.
pub const CELL_GAME_DIRNAME_SIZE: usize = 32;

/// `CellGameGameType` discriminants at cpp:67-68.
pub const CELL_GAME_GAMETYPE_DISC: u32 = 1;
pub const CELL_GAME_GAMETYPE_HDD: u32 = 2;

// --- Game type enum ------------------------------------------------------

/// Strongly-typed mirror for what the firmware reports via
/// [`Emu.GetBootSourceType`](https://rpcs3.net). Callers parse the
/// packed `u32` with [`Self::from_u32`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellGameGameType {
    Disc,
    Hdd,
}

impl CellGameGameType {
    #[must_use]
    pub const fn from_u32(v: u32) -> Option<Self> {
        match v {
            CELL_GAME_GAMETYPE_DISC => Some(Self::Disc),
            CELL_GAME_GAMETYPE_HDD => Some(Self::Hdd),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_u32(self) -> u32 {
        match self {
            Self::Disc => CELL_GAME_GAMETYPE_DISC,
            Self::Hdd => CELL_GAME_GAMETYPE_HDD,
        }
    }
}

// --- Boot info output ---------------------------------------------------

/// Values captured by `cellGameGetBootGameInfo` (cpp:91-122).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootGameInfo {
    /// Mirrors the `*type` out-arg (cpp:102).
    pub game_type: u32,
    /// Mirrors the `dirName` out-arg. `None` when the boot source is
    /// `DISC` (cpp:109-119 only copies on `HDD`).
    pub dir_name: Option<String>,
    /// Mirrors the optional `*execdata` out-arg (cpp:104-107).
    pub exec_data: Option<u32>,
}

// --- Manager ------------------------------------------------------------

/// HLE singleton mirror of `struct game_exec_data` (cpp:10-13) plus the
/// ambient emulator state (`Emu.GetBootSourceType`, `Emu.GetDir`) the
/// entries read.
#[derive(Debug, Default)]
pub struct GameExec {
    execdata: u32,
    boot_source: Option<CellGameGameType>,
    boot_dir: String,
    // Counters let test code assert an entry was invoked without
    // inspecting its output arg buffers.
    set_exit_param_calls: u32,
    get_boot_game_info_calls: u32,
    get_exit_game_info_calls: u32,
    exec_game_calls: u32,
    delete_game_calls: u32,
    get_list_calls: u32,
}

impl GameExec {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            execdata: 0,
            boot_source: None,
            boot_dir: String::new(),
            set_exit_param_calls: 0,
            get_boot_game_info_calls: 0,
            get_exit_game_info_calls: 0,
            exec_game_calls: 0,
            delete_game_calls: 0,
            get_list_calls: 0,
        }
    }

    /// Inject the ambient emulator state that the real firmware pulls
    /// via `Emu.GetBootSourceType` + `Emu.GetDir`. Rust-only helper —
    /// the C++ doesn't need one because `g_fxo` is a process-global.
    pub fn set_boot_source(&mut self, source: CellGameGameType, dir: impl Into<String>) {
        self.boot_source = Some(source);
        self.boot_dir = dir.into();
    }

    #[must_use]
    pub fn execdata(&self) -> u32 {
        self.execdata
    }

    #[must_use]
    pub fn boot_source(&self) -> Option<CellGameGameType> {
        self.boot_source
    }

    #[must_use]
    pub fn boot_dir(&self) -> &str {
        &self.boot_dir
    }

    // --- counters ---

    #[must_use]
    pub fn set_exit_param_calls(&self) -> u32 {
        self.set_exit_param_calls
    }
    #[must_use]
    pub fn get_boot_game_info_calls(&self) -> u32 {
        self.get_boot_game_info_calls
    }
    #[must_use]
    pub fn get_exit_game_info_calls(&self) -> u32 {
        self.get_exit_game_info_calls
    }
    #[must_use]
    pub fn exec_game_calls(&self) -> u32 {
        self.exec_game_calls
    }
    #[must_use]
    pub fn delete_game_calls(&self) -> u32 {
        self.delete_game_calls
    }
    #[must_use]
    pub fn get_list_calls(&self) -> u32 {
        self.get_list_calls
    }

    // --- entry points ---

    /// `cellGameSetExitParam` (cpp:15-22). Stores the `u32` that the
    /// next boot-chained game reads via `cellGameGetBootGameInfo`.
    /// Always returns `CELL_OK` — there are no documented failure
    /// modes.
    pub fn set_exit_param(&mut self, execdata: u32) -> Result<(), CellError> {
        self.execdata = execdata;
        self.set_exit_param_calls = self.set_exit_param_calls.saturating_add(1);
        Ok(())
    }

    /// `cellGameGetHomeDataExportPath` (cpp:24-36). Caller supplies a
    /// buffer via `has_export_path = true`; PlayStation Home is defunct
    /// so the firmware returns `NOAPP` on any non-null request.
    /// Passing `None` short-circuits to `PARAM` matching the cpp:28-31
    /// null check.
    pub fn get_home_data_export_path(&self, has_export_path: bool) -> Result<(), CellError> {
        if !has_export_path {
            return Err(CELL_GAME_ERROR_PARAM);
        }
        Err(CELL_GAME_ERROR_NOAPP)
    }

    /// `cellGameGetHomePath` (cpp:38-50). Same as above but returns
    /// `CELL_OK` because the firmware documented this entry as "TODO".
    pub fn get_home_path(&self, has_home_path: bool) -> Result<(), CellError> {
        if !has_home_path {
            return Err(CELL_GAME_ERROR_PARAM);
        }
        Ok(())
    }

    /// `cellGameGetHomeDataImportPath` (cpp:52-64).
    pub fn get_home_data_import_path(&self, has_import_path: bool) -> Result<(), CellError> {
        if !has_import_path {
            return Err(CELL_GAME_ERROR_PARAM);
        }
        Err(CELL_GAME_ERROR_NOAPP)
    }

    /// `cellGameGetHomeLaunchOptionPath` (cpp:66-77). Both buffers
    /// required — either null yields `PARAM`. Defunct otherwise.
    pub fn get_home_launch_option_path(
        &self,
        has_common: bool,
        has_personal: bool,
    ) -> Result<(), CellError> {
        if !has_common || !has_personal {
            return Err(CELL_GAME_ERROR_PARAM);
        }
        Err(CELL_GAME_ERROR_NOAPP)
    }

    /// `cellGameExecGame` (cpp:79-83). Stub — the firmware TODO just
    /// returns `CELL_OK`. Included here so a test can verify the
    /// counter path.
    pub fn exec_game(&mut self) -> Result<(), CellError> {
        self.exec_game_calls = self.exec_game_calls.saturating_add(1);
        Ok(())
    }

    /// `cellGameDeleteGame` (cpp:85-89). Same stub shape as exec_game.
    pub fn delete_game(&mut self) -> Result<(), CellError> {
        self.delete_game_calls = self.delete_game_calls.saturating_add(1);
        Ok(())
    }

    /// `cellGameGetBootGameInfo` (cpp:91-122). Validates the
    /// caller-supplied `type` + `dirName` pointers (`execData` may be
    /// null — cpp:95 comment), resolves the boot source from
    /// `boot_source()`, and copies `boot_dir` when the source is HDD.
    /// Returns `CELL_HDDGAME_ERROR_INTERNAL` if the stored dir name is
    /// too long to fit (cpp:113-115 speculative error).
    pub fn get_boot_game_info(
        &mut self,
        has_type: bool,
        has_dir_name: bool,
        has_exec_data: bool,
    ) -> Result<BootGameInfo, CellError> {
        if !has_type || !has_dir_name {
            return Err(CELL_GAME_ERROR_PARAM);
        }

        let source_type = self.boot_source().unwrap_or(CellGameGameType::Hdd);

        let dir_name = if source_type == CellGameGameType::Hdd {
            if self.boot_dir.len() >= CELL_GAME_DIRNAME_SIZE {
                return Err(CELL_HDDGAME_ERROR_INTERNAL);
            }
            Some(self.boot_dir.clone())
        } else {
            None
        };

        let exec_data = if has_exec_data {
            Some(self.execdata)
        } else {
            None
        };

        self.get_boot_game_info_calls = self.get_boot_game_info_calls.saturating_add(1);
        Ok(BootGameInfo {
            game_type: source_type.as_u32(),
            dir_name,
            exec_data,
        })
    }

    /// `cellGameGetExitGameInfo` (cpp:124-128). Stub — returns
    /// `CELL_OK`. The real firmware would fill out the `execData` /
    /// `userData` pair with values passed through the exit-spawn
    /// chain; the stub doesn't track that yet.
    pub fn get_exit_game_info(&mut self) -> Result<(), CellError> {
        self.get_exit_game_info_calls = self.get_exit_game_info_calls.saturating_add(1);
        Ok(())
    }

    /// `cellGameGetList` (cpp:130-134).
    pub fn get_list(&mut self) -> Result<(), CellError> {
        self.get_list_calls = self.get_list_calls.saturating_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn module_name_byte_exact() {
        assert_eq!(MODULE_NAME, "cellGameExec");
    }

    #[test]
    fn registered_entry_points_exact_order() {
        assert_eq!(REGISTERED_ENTRY_POINTS.len(), 10);
        assert_eq!(REGISTERED_ENTRY_POINTS[0], "cellGameSetExitParam");
        assert_eq!(REGISTERED_ENTRY_POINTS[5], "cellGameExecGame");
        assert_eq!(REGISTERED_ENTRY_POINTS[7], "cellGameGetBootGameInfo");
        assert_eq!(REGISTERED_ENTRY_POINTS[9], "cellGameGetList");
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(CELL_GAME_ERROR_PARAM.0, 0x8002_CB07);
        assert_eq!(CELL_GAME_ERROR_NOAPP.0, 0x8002_CB08);
        assert_eq!(CELL_HDDGAME_ERROR_INTERNAL.0, 0x8002_BA03);
    }

    #[test]
    fn constants_byte_exact() {
        assert_eq!(CELL_GAME_DIRNAME_SIZE, 32);
        assert_eq!(CELL_GAME_GAMETYPE_DISC, 1);
        assert_eq!(CELL_GAME_GAMETYPE_HDD, 2);
    }

    #[test]
    fn game_type_roundtrip() {
        assert_eq!(CellGameGameType::Disc.as_u32(), 1);
        assert_eq!(CellGameGameType::Hdd.as_u32(), 2);
        assert_eq!(CellGameGameType::from_u32(1), Some(CellGameGameType::Disc));
        assert_eq!(CellGameGameType::from_u32(2), Some(CellGameGameType::Hdd));
        assert_eq!(CellGameGameType::from_u32(0), None);
        assert_eq!(CellGameGameType::from_u32(3), None);
    }

    #[test]
    fn set_exit_param_stores_value() {
        let mut g = GameExec::new();
        g.set_exit_param(0xDEAD_BEEF).unwrap();
        assert_eq!(g.execdata(), 0xDEAD_BEEF);
        assert_eq!(g.set_exit_param_calls(), 1);
    }

    #[test]
    fn home_data_export_path_null_is_param() {
        let g = GameExec::new();
        assert_eq!(
            g.get_home_data_export_path(false),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    #[test]
    fn home_data_export_path_returns_noapp() {
        let g = GameExec::new();
        assert_eq!(
            g.get_home_data_export_path(true),
            Err(CELL_GAME_ERROR_NOAPP)
        );
    }

    #[test]
    fn home_path_ok_when_buffer_provided() {
        let g = GameExec::new();
        g.get_home_path(true).unwrap();
        assert_eq!(g.get_home_path(false), Err(CELL_GAME_ERROR_PARAM));
    }

    #[test]
    fn home_data_import_path_null_is_param() {
        let g = GameExec::new();
        assert_eq!(
            g.get_home_data_import_path(false),
            Err(CELL_GAME_ERROR_PARAM)
        );
        assert_eq!(
            g.get_home_data_import_path(true),
            Err(CELL_GAME_ERROR_NOAPP)
        );
    }

    #[test]
    fn home_launch_option_path_both_required() {
        let g = GameExec::new();
        assert_eq!(
            g.get_home_launch_option_path(false, true),
            Err(CELL_GAME_ERROR_PARAM)
        );
        assert_eq!(
            g.get_home_launch_option_path(true, false),
            Err(CELL_GAME_ERROR_PARAM)
        );
        assert_eq!(
            g.get_home_launch_option_path(true, true),
            Err(CELL_GAME_ERROR_NOAPP)
        );
    }

    #[test]
    fn boot_info_null_type_is_param() {
        let mut g = GameExec::new();
        assert_eq!(
            g.get_boot_game_info(false, true, false),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    #[test]
    fn boot_info_null_dir_is_param() {
        let mut g = GameExec::new();
        assert_eq!(
            g.get_boot_game_info(true, false, false),
            Err(CELL_GAME_ERROR_PARAM)
        );
    }

    #[test]
    fn boot_info_hdd_reports_dir_and_execdata() {
        let mut g = GameExec::new();
        g.set_boot_source(CellGameGameType::Hdd, "BLES00123");
        g.set_exit_param(0xCAFEBABE).unwrap();
        let info = g.get_boot_game_info(true, true, true).unwrap();
        assert_eq!(info.game_type, CELL_GAME_GAMETYPE_HDD);
        assert_eq!(info.dir_name.as_deref(), Some("BLES00123"));
        assert_eq!(info.exec_data, Some(0xCAFEBABE));
    }

    #[test]
    fn boot_info_hdd_null_execdata_returns_none() {
        let mut g = GameExec::new();
        g.set_boot_source(CellGameGameType::Hdd, "TESTDIR");
        let info = g.get_boot_game_info(true, true, false).unwrap();
        assert_eq!(info.exec_data, None);
    }

    #[test]
    fn boot_info_disc_has_no_dir() {
        let mut g = GameExec::new();
        g.set_boot_source(CellGameGameType::Disc, "ignored");
        let info = g.get_boot_game_info(true, true, true).unwrap();
        assert_eq!(info.game_type, CELL_GAME_GAMETYPE_DISC);
        assert_eq!(info.dir_name, None);
    }

    #[test]
    fn boot_info_dirname_too_long_is_internal_error() {
        let mut g = GameExec::new();
        g.set_boot_source(
            CellGameGameType::Hdd,
            "A".repeat(CELL_GAME_DIRNAME_SIZE),
        );
        assert_eq!(
            g.get_boot_game_info(true, true, true),
            Err(CELL_HDDGAME_ERROR_INTERNAL)
        );
    }

    #[test]
    fn boot_info_dirname_boundary_accepted() {
        let mut g = GameExec::new();
        // Exactly CELL_GAME_DIRNAME_SIZE - 1 bytes fits (mimics the C++
        // `>=` check which rejects equality).
        let name = "A".repeat(CELL_GAME_DIRNAME_SIZE - 1);
        g.set_boot_source(CellGameGameType::Hdd, name.clone());
        let info = g.get_boot_game_info(true, true, false).unwrap();
        assert_eq!(info.dir_name.as_deref(), Some(name.as_str()));
    }

    #[test]
    fn boot_info_without_source_defaults_to_hdd() {
        // Emu.GetBootSourceType defaults in the real firmware; the port
        // treats a missing injection as HDD so games that never call
        // `set_boot_source` still reach the dir-copy path.
        let mut g = GameExec::new();
        let info = g.get_boot_game_info(true, true, false).unwrap();
        assert_eq!(info.game_type, CELL_GAME_GAMETYPE_HDD);
        assert_eq!(info.dir_name.as_deref(), Some(""));
    }

    #[test]
    fn exec_game_increments_counter() {
        let mut g = GameExec::new();
        g.exec_game().unwrap();
        g.exec_game().unwrap();
        assert_eq!(g.exec_game_calls(), 2);
    }

    #[test]
    fn delete_game_increments_counter() {
        let mut g = GameExec::new();
        g.delete_game().unwrap();
        assert_eq!(g.delete_game_calls(), 1);
    }

    #[test]
    fn get_exit_game_info_and_list_stubs_return_ok() {
        let mut g = GameExec::new();
        g.get_exit_game_info().unwrap();
        g.get_list().unwrap();
        assert_eq!(g.get_exit_game_info_calls(), 1);
        assert_eq!(g.get_list_calls(), 1);
    }

    #[test]
    fn full_gameexec_lifecycle_smoke() {
        let mut g = GameExec::new();

        // 1. Simulate boot from HDD with a known dir + execdata.
        g.set_boot_source(CellGameGameType::Hdd, "NPJA00001");
        g.set_exit_param(0x0BAD_F00D).unwrap();

        // 2. The booting game reads its info back.
        let info = g.get_boot_game_info(true, true, true).unwrap();
        assert_eq!(info.game_type, CELL_GAME_GAMETYPE_HDD);
        assert_eq!(info.dir_name.as_deref(), Some("NPJA00001"));
        assert_eq!(info.exec_data, Some(0x0BAD_F00D));

        // 3. PlayStation Home hooks all short-circuit to NOAPP /
        //    CELL_OK as documented.
        assert_eq!(
            g.get_home_data_export_path(true),
            Err(CELL_GAME_ERROR_NOAPP)
        );
        g.get_home_path(true).unwrap();
        assert_eq!(
            g.get_home_data_import_path(true),
            Err(CELL_GAME_ERROR_NOAPP)
        );
        assert_eq!(
            g.get_home_launch_option_path(true, true),
            Err(CELL_GAME_ERROR_NOAPP)
        );

        // 4. Stubby entries all return OK.
        g.exec_game().unwrap();
        g.delete_game().unwrap();
        g.get_exit_game_info().unwrap();
        g.get_list().unwrap();

        // 5. Counter trace matches dispatch order.
        assert_eq!(g.set_exit_param_calls(), 1);
        assert_eq!(g.get_boot_game_info_calls(), 1);
        assert_eq!(g.exec_game_calls(), 1);
        assert_eq!(g.delete_game_calls(), 1);
        assert_eq!(g.get_exit_game_info_calls(), 1);
        assert_eq!(g.get_list_calls(), 1);
    }

    #[test]
    fn disc_game_returns_type_without_dir() {
        let mut g = GameExec::new();
        g.set_boot_source(CellGameGameType::Disc, "never-used");
        g.set_exit_param(0x12345).unwrap();
        let info = g.get_boot_game_info(true, true, true).unwrap();
        assert_eq!(info.game_type, CELL_GAME_GAMETYPE_DISC);
        assert!(info.dir_name.is_none());
        assert_eq!(info.exec_data, Some(0x12345));
    }
}
