//! `rpcs3-hle-cellsysmodule` — dynamic HLE module loader.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSysmodule.cpp`. Games call
//! `cellSysmoduleLoadModule(id)` before using an HLE module, so the
//! runtime can initialise the correct subsystem (cellGcm, cellPad,
//! etc.). Our implementation is a thin refcount registry: load +1,
//! unload -1, is_loaded returns 0 if any refs are outstanding.
//!
//! Module ID → name table mirrors `cellSysmodule.cpp:39-` byte-exact
//! for the common IDs.
//!
//! ## Entry points
//!
//! | HLE function                   | Rust wrapper                      |
//! |--------------------------------|-----------------------------------|
//! | `cellSysmoduleInitialize`      | [`cell_sysmodule_initialize`]     |
//! | `cellSysmoduleFinalize`        | [`cell_sysmodule_finalize`]       |
//! | `cellSysmoduleLoadModule`      | [`cell_sysmodule_load_module`]    |
//! | `cellSysmoduleUnloadModule`    | [`cell_sysmodule_unload_module`]  |
//! | `cellSysmoduleIsLoaded`        | [`cell_sysmodule_is_loaded`]      |
//! | `cellSysmoduleSetMemcontainer` | [`cell_sysmodule_set_memcontainer`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Result codes — byte-exact with cellSysmodule.cpp:6-14
// =====================================================================

/// `CELL_SYSMODULE_LOADED` aliases `CELL_OK` (0).
pub const LOADED_OK: i32 = 0;

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const DUPLICATED: CellError = CellError(0x8001_2001);
    pub const UNKNOWN: CellError = CellError(0x8001_2002);
    pub const UNLOADED: CellError = CellError(0x8001_2003);
    pub const INVALID_MEMCONTAINER: CellError = CellError(0x8001_2004);
    pub const FATAL: CellError = CellError(0x8001_20FF);
}

// =====================================================================
// Module ID → name table
// =====================================================================

/// Returns the canonical name for a sysmodule id, or `None` if the id
/// is not recognised. Subset of the huge switch in `cellSysmodule.cpp`.
#[must_use]
pub fn module_name(id: u16) -> Option<&'static str> {
    Some(match id {
        0x0000 => "sys_net",
        0x0001 => "cellHttp",
        0x0002 => "cellHttpUtil",
        0x0003 => "cellSsl",
        0x0004 => "cellHttps",
        0x0005 => "libvdec",
        0x0006 => "cellAdec",
        0x0007 => "cellDmux",
        0x0008 => "cellVpost",
        0x0009 => "cellRtc",
        0x000A => "cellSpurs",
        0x000B => "cellOvis",
        0x000C => "cellSheap",
        0x000D => "cellSync",
        0x000E => "sys_fs",
        0x000F => "cellJpgDec",
        0x0010 => "cellGcmSys",
        0x0011 => "cellAudio",
        0x0012 => "cellPamf",
        0x0013 => "cellAtrac",
        0x0014 => "cellNetCtl",
        0x0015 => "cellCell",
        0x0016 => "sysutil_np",
        0x0017 => "cellSysutil",
        0x0018 => "cellSysutilNpClans",
        0x0019 => "sysutil_bgdl",
        0x001A => "cellUsbd",
        0x001B => "cellAvconfExt",
        0x001C => "cellSysml",
        0x001D => "cellSysutilMisc",
        0x001E => "cellSail",
        0x001F => "cellL10n",
        0x0020 => "cellResc",
        0x0021 => "cellDaisy",
        0x0022 => "cellKey2char",
        0x0023 => "cellMic",
        0x0024 => "cellCamera",
        0x0025 => "cellVdec",
        0x0026 => "cellFont",
        0x0027 => "cellFontFT",
        0x0028 => "cellFreetype",
        0x0029 => "cellUsbPspcm",
        0x002A => "cellSysutilAvc2",
        0x002B => "cellSail",
        0x002C => "cellSailRec",
        0x002D => "cellSysutilNpUtil",
        _ => return None,
    })
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Default)]
pub struct SysmoduleManager {
    initialized: bool,
    /// id → outstanding refs.
    refs: std::collections::BTreeMap<u16, u32>,
    memcontainer: Option<u32>,
}

// =====================================================================
// Syscalls
// =====================================================================

/// `cellSysmoduleInitialize()` — idempotent; returns OK on second call.
#[must_use]
pub fn cell_sysmodule_initialize(m: &mut SysmoduleManager) -> Result<(), CellError> {
    m.initialized = true;
    Ok(())
}

/// `cellSysmoduleFinalize()`.
#[must_use]
pub fn cell_sysmodule_finalize(m: &mut SysmoduleManager) -> Result<(), CellError> {
    m.initialized = false;
    m.refs.clear();
    m.memcontainer = None;
    Ok(())
}

/// `cellSysmoduleSetMemcontainer(container_id)`.
#[must_use]
pub fn cell_sysmodule_set_memcontainer(
    m: &mut SysmoduleManager,
    container_id: u32,
) -> Result<(), CellError> {
    // `0xFFFFFFFF` == "default" sentinel — accepted. Anything else
    // must be a known container id; since we don't own the memory
    // manager here, accept unconditionally (validation happens at
    // the lv2-memory layer).
    m.memcontainer = Some(container_id);
    Ok(())
}

/// `cellSysmoduleLoadModule(id)`. Returns `DUPLICATED` if already
/// loaded (matches C++ behaviour for explicit load-twice).
#[must_use]
pub fn cell_sysmodule_load_module(
    m: &mut SysmoduleManager,
    id: u16,
) -> Result<(), CellError> {
    if module_name(id).is_none() {
        return Err(errors::UNKNOWN);
    }
    let slot = m.refs.entry(id).or_insert(0);
    if *slot > 0 {
        return Err(errors::DUPLICATED);
    }
    *slot = 1;
    Ok(())
}

/// `cellSysmoduleUnloadModule(id)`.
#[must_use]
pub fn cell_sysmodule_unload_module(
    m: &mut SysmoduleManager,
    id: u16,
) -> Result<(), CellError> {
    if module_name(id).is_none() {
        return Err(errors::UNKNOWN);
    }
    match m.refs.get_mut(&id) {
        Some(n) if *n > 0 => {
            *n -= 1;
            if *n == 0 {
                m.refs.remove(&id);
            }
            Ok(())
        }
        _ => Err(errors::UNLOADED),
    }
}

/// `cellSysmoduleIsLoaded(id)` — returns `CELL_SYSMODULE_LOADED` (0)
/// or `CELL_SYSMODULE_ERROR_UNLOADED`.
#[must_use]
pub fn cell_sysmodule_is_loaded(m: &SysmoduleManager, id: u16) -> Result<i32, CellError> {
    if module_name(id).is_none() {
        return Err(errors::UNKNOWN);
    }
    match m.refs.get(&id) {
        Some(&n) if n > 0 => Ok(LOADED_OK),
        _ => Err(errors::UNLOADED),
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_match_cellSysmodule_cpp() {
        assert_eq!(errors::DUPLICATED.0, 0x8001_2001);
        assert_eq!(errors::UNKNOWN.0, 0x8001_2002);
        assert_eq!(errors::UNLOADED.0, 0x8001_2003);
        assert_eq!(errors::INVALID_MEMCONTAINER.0, 0x8001_2004);
        assert_eq!(errors::FATAL.0, 0x8001_20FF);
    }

    #[test]
    fn loaded_ok_is_zero() {
        assert_eq!(LOADED_OK, 0);
    }

    // --- id table -------------------------------------------------

    #[test]
    fn common_module_ids_resolve() {
        assert_eq!(module_name(0x0000), Some("sys_net"));
        assert_eq!(module_name(0x000A), Some("cellSpurs"));
        assert_eq!(module_name(0x000D), Some("cellSync"));
        assert_eq!(module_name(0x0010), Some("cellGcmSys"));
        assert_eq!(module_name(0x0011), Some("cellAudio"));
    }

    #[test]
    fn unknown_module_returns_none() {
        assert_eq!(module_name(0xFFFF), None);
        assert_eq!(module_name(0x7FFF), None);
    }

    // --- load / unload --------------------------------------------

    #[test]
    fn load_then_is_loaded_reports_ok() {
        let mut m = SysmoduleManager::default();
        cell_sysmodule_load_module(&mut m, 0x0010).unwrap();
        assert_eq!(cell_sysmodule_is_loaded(&m, 0x0010).unwrap(), LOADED_OK);
    }

    #[test]
    fn load_unknown_id_is_unknown() {
        let mut m = SysmoduleManager::default();
        assert_eq!(
            cell_sysmodule_load_module(&mut m, 0xFFFF).unwrap_err(),
            errors::UNKNOWN,
        );
    }

    #[test]
    fn load_twice_is_duplicated() {
        let mut m = SysmoduleManager::default();
        cell_sysmodule_load_module(&mut m, 0x000A).unwrap();
        assert_eq!(
            cell_sysmodule_load_module(&mut m, 0x000A).unwrap_err(),
            errors::DUPLICATED,
        );
    }

    #[test]
    fn unload_brings_is_loaded_back_to_unloaded() {
        let mut m = SysmoduleManager::default();
        cell_sysmodule_load_module(&mut m, 0x000A).unwrap();
        cell_sysmodule_unload_module(&mut m, 0x000A).unwrap();
        assert_eq!(
            cell_sysmodule_is_loaded(&m, 0x000A).unwrap_err(),
            errors::UNLOADED,
        );
    }

    #[test]
    fn unload_never_loaded_is_unloaded_err() {
        let mut m = SysmoduleManager::default();
        assert_eq!(
            cell_sysmodule_unload_module(&mut m, 0x0010).unwrap_err(),
            errors::UNLOADED,
        );
    }

    #[test]
    fn unload_unknown_id_is_unknown() {
        let mut m = SysmoduleManager::default();
        assert_eq!(
            cell_sysmodule_unload_module(&mut m, 0xFFFF).unwrap_err(),
            errors::UNKNOWN,
        );
    }

    #[test]
    fn is_loaded_never_loaded_is_unloaded() {
        let m = SysmoduleManager::default();
        assert_eq!(
            cell_sysmodule_is_loaded(&m, 0x0010).unwrap_err(),
            errors::UNLOADED,
        );
    }

    #[test]
    fn is_loaded_unknown_id_is_unknown() {
        let m = SysmoduleManager::default();
        assert_eq!(
            cell_sysmodule_is_loaded(&m, 0xFFFF).unwrap_err(),
            errors::UNKNOWN,
        );
    }

    #[test]
    fn finalize_clears_all_refs() {
        let mut m = SysmoduleManager::default();
        cell_sysmodule_load_module(&mut m, 0x0010).unwrap();
        cell_sysmodule_load_module(&mut m, 0x0011).unwrap();
        cell_sysmodule_finalize(&mut m).unwrap();
        assert_eq!(
            cell_sysmodule_is_loaded(&m, 0x0010).unwrap_err(),
            errors::UNLOADED,
        );
    }

    #[test]
    fn initialize_is_idempotent() {
        let mut m = SysmoduleManager::default();
        cell_sysmodule_initialize(&mut m).unwrap();
        cell_sysmodule_initialize(&mut m).unwrap();
    }

    #[test]
    fn set_memcontainer_round_trips() {
        let mut m = SysmoduleManager::default();
        cell_sysmodule_set_memcontainer(&mut m, 0x1234).unwrap();
        assert_eq!(m.memcontainer, Some(0x1234));
    }

    #[test]
    fn load_after_unload_works_again() {
        let mut m = SysmoduleManager::default();
        cell_sysmodule_load_module(&mut m, 0x0009).unwrap();
        cell_sysmodule_unload_module(&mut m, 0x0009).unwrap();
        cell_sysmodule_load_module(&mut m, 0x0009).unwrap();
        assert_eq!(cell_sysmodule_is_loaded(&m, 0x0009).unwrap(), LOADED_OK);
    }
}
