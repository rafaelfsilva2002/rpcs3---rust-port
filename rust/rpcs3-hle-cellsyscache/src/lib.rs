//! `rpcs3-hle-cellsyscache` — HDD1 game-data cache HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellSysCache.cpp`. Games mount a
//! per-title cache under `/dev_hdd1/caches/{cache_id}` for scratch
//! data (extracted assets, level caches). When the ID changes from
//! a previous run, the runtime clears the old cache directory.
//!
//! ## Entry points covered
//!
//! | HLE function                  | Rust wrapper                    |
//! |-------------------------------|---------------------------------|
//! | `cellSysCacheMount`           | [`cell_sys_cache_mount`]        |
//! | `cellSysCacheClear`           | [`cell_sys_cache_clear`]        |
//!
//! ## Frozen constants (from `cellSysutil.h:244-258`)
//!
//! | Const                   | Value       |
//! |-------------------------|-------------|
//! | `RET_OK_CLEARED`        | 0           |
//! | `RET_OK_RELAYED`        | 1           |
//! | `ID_SIZE`               | 32          |
//! | `PATH_MAX`              | 1055        |
//! | `ERROR_ACCESS_ERROR`    | 0x8002BC01  |
//! | `ERROR_INTERNAL`        | 0x8002BC02  |
//! | `ERROR_PARAM`           | 0x8002BC03  |
//! | `ERROR_NOTMOUNTED`      | 0x8002BC04  |

use rpcs3_emu_types::CellError;

// =====================================================================
// Frozen constants
// =====================================================================

pub const RET_OK_CLEARED: i32 = 0;
pub const RET_OK_RELAYED: i32 = 1;
pub const ID_SIZE: usize = 32;
pub const PATH_MAX: usize = 1055;

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const ACCESS_ERROR: CellError = CellError(0x8002_BC01);
    pub const INTERNAL: CellError = CellError(0x8002_BC02);
    pub const PARAM: CellError = CellError(0x8002_BC03);
    pub const NOTMOUNTED: CellError = CellError(0x8002_BC04);
}

/// Root under which caches live (relative to /dev_hdd1).
pub const CACHE_ROOT: &str = "/dev_hdd1/caches";

// =====================================================================
// Mount outcome
// =====================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountResult {
    /// Canonical path the cache is mounted at.
    pub path: String,
    /// Whether the prior cache was cleared (new ID) or reused.
    pub status: i32,
}

// =====================================================================
// ID validation
// =====================================================================

/// Matches `sysutil_check_name_string(name, 1, CELL_SYSCACHE_ID_SIZE)`.
/// IDs must be 1..=31 printable ASCII chars, no path separators, no NUL.
pub fn validate_cache_id(id: &str) -> Result<(), CellError> {
    if id.is_empty() || id.len() >= ID_SIZE {
        return Err(errors::PARAM);
    }
    for ch in id.bytes() {
        if ch < 0x20 || ch > 0x7E {
            return Err(errors::PARAM);
        }
        if matches!(ch, b'/' | b'\\' | b'\0' | b':' | b'*' | b'?' | b'<' | b'>' | b'|') {
            return Err(errors::PARAM);
        }
    }
    Ok(())
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Default)]
pub struct SysCacheManager {
    /// Currently mounted cache id (None if not mounted).
    pub current_id: Option<String>,
    /// Title id prefix — if non-empty, used when constructing cache
    /// directory names (matches C++ `Emu.GetTitleID()+"_"`).
    pub title_prefix: String,
    /// When true, old cache is retained when a new ID is mounted.
    pub retain_caches: bool,
}

// =====================================================================
// Syscalls
// =====================================================================

/// Build the full filesystem path for a given cache id.
fn build_path(mgr: &SysCacheManager, cache_id: &str) -> String {
    if mgr.title_prefix.is_empty() {
        format!("{CACHE_ROOT}/{cache_id}")
    } else {
        format!("{CACHE_ROOT}/{}_{cache_id}", mgr.title_prefix)
    }
}

/// `cellSysCacheMount(param)` — register a cache ID. If the new ID
/// differs from whatever was previously mounted (and retain is off),
/// the old cache is considered cleared.
#[must_use]
pub fn cell_sys_cache_mount(
    mgr: &mut SysCacheManager,
    new_id: &str,
) -> Result<MountResult, CellError> {
    validate_cache_id(new_id)?;
    let path = build_path(mgr, new_id);
    if path.len() > PATH_MAX {
        return Err(errors::PARAM);
    }

    let status = match &mgr.current_id {
        Some(prev) if prev == new_id => RET_OK_RELAYED,
        Some(_prev) if mgr.retain_caches => RET_OK_RELAYED,
        _ => RET_OK_CLEARED,
    };
    mgr.current_id = Some(new_id.to_owned());
    Ok(MountResult { path, status })
}

/// `cellSysCacheClear()` — always clears the currently-mounted cache.
/// Returns `ERROR_NOTMOUNTED` if no cache is active.
#[must_use]
pub fn cell_sys_cache_clear(mgr: &mut SysCacheManager) -> Result<i32, CellError> {
    if mgr.current_id.is_none() {
        return Err(errors::NOTMOUNTED);
    }
    Ok(RET_OK_CLEARED)
}

/// Helper for emu core integration — returns the current cache path,
/// or `None` if not mounted. Not a real LV2 syscall, matches the
/// `cellSysCache.cpp:get_path()` helper.
#[must_use]
pub fn cache_path(mgr: &SysCacheManager) -> Option<String> {
    mgr.current_id.as_ref().map(|id| build_path(mgr, id))
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::ACCESS_ERROR.0, 0x8002_BC01);
        assert_eq!(errors::INTERNAL.0, 0x8002_BC02);
        assert_eq!(errors::PARAM.0, 0x8002_BC03);
        assert_eq!(errors::NOTMOUNTED.0, 0x8002_BC04);
    }

    #[test]
    fn return_codes_match() {
        assert_eq!(RET_OK_CLEARED, 0);
        assert_eq!(RET_OK_RELAYED, 1);
        assert_eq!(ID_SIZE, 32);
        assert_eq!(PATH_MAX, 1055);
    }

    // --- id validation --------------------------------------------

    #[test]
    fn validate_accepts_normal_ids() {
        validate_cache_id("level1").unwrap();
        validate_cache_id("SAVE_2024").unwrap();
        validate_cache_id("a").unwrap();
    }

    #[test]
    fn validate_rejects_empty() {
        assert_eq!(validate_cache_id("").unwrap_err(), errors::PARAM);
    }

    #[test]
    fn validate_rejects_over_31_chars() {
        let long = "x".repeat(32);
        assert_eq!(validate_cache_id(&long).unwrap_err(), errors::PARAM);
    }

    #[test]
    fn validate_accepts_31_chars_exactly() {
        validate_cache_id(&"x".repeat(31)).unwrap();
    }

    #[test]
    fn validate_rejects_path_separators() {
        assert_eq!(validate_cache_id("a/b").unwrap_err(), errors::PARAM);
        assert_eq!(validate_cache_id("a\\b").unwrap_err(), errors::PARAM);
    }

    #[test]
    fn validate_rejects_control_chars() {
        assert_eq!(validate_cache_id("a\tb").unwrap_err(), errors::PARAM);
        assert_eq!(validate_cache_id("a\nb").unwrap_err(), errors::PARAM);
    }

    #[test]
    fn validate_rejects_reserved_windows_chars() {
        for bad in ["a:b", "a*b", "a?b", "a<b", "a>b", "a|b"] {
            assert_eq!(
                validate_cache_id(bad).unwrap_err(),
                errors::PARAM,
                "expected PARAM for {bad}",
            );
        }
    }

    // --- mount / path construction --------------------------------

    #[test]
    fn mount_with_no_title_prefix() {
        let mut m = SysCacheManager::default();
        let r = cell_sys_cache_mount(&mut m, "level1").unwrap();
        assert_eq!(r.path, "/dev_hdd1/caches/level1");
        assert_eq!(r.status, RET_OK_CLEARED);
    }

    #[test]
    fn mount_with_title_prefix_uses_underscore() {
        let mut m = SysCacheManager {
            title_prefix: "BLUS12345".into(),
            ..SysCacheManager::default()
        };
        let r = cell_sys_cache_mount(&mut m, "level1").unwrap();
        assert_eq!(r.path, "/dev_hdd1/caches/BLUS12345_level1");
    }

    #[test]
    fn mount_same_id_twice_returns_relayed() {
        let mut m = SysCacheManager::default();
        cell_sys_cache_mount(&mut m, "same").unwrap();
        let r = cell_sys_cache_mount(&mut m, "same").unwrap();
        assert_eq!(r.status, RET_OK_RELAYED);
    }

    #[test]
    fn mount_different_id_returns_cleared() {
        let mut m = SysCacheManager::default();
        cell_sys_cache_mount(&mut m, "first").unwrap();
        let r = cell_sys_cache_mount(&mut m, "second").unwrap();
        assert_eq!(r.status, RET_OK_CLEARED);
        assert_eq!(m.current_id.as_deref(), Some("second"));
    }

    #[test]
    fn mount_different_id_with_retain_returns_relayed() {
        let mut m = SysCacheManager {
            retain_caches: true,
            ..SysCacheManager::default()
        };
        cell_sys_cache_mount(&mut m, "first").unwrap();
        let r = cell_sys_cache_mount(&mut m, "second").unwrap();
        assert_eq!(r.status, RET_OK_RELAYED);
    }

    #[test]
    fn mount_rejects_invalid_id() {
        let mut m = SysCacheManager::default();
        assert_eq!(
            cell_sys_cache_mount(&mut m, "bad/id").unwrap_err(),
            errors::PARAM,
        );
    }

    // --- clear ----------------------------------------------------

    #[test]
    fn clear_without_mount_returns_notmounted() {
        let mut m = SysCacheManager::default();
        assert_eq!(cell_sys_cache_clear(&mut m).unwrap_err(), errors::NOTMOUNTED);
    }

    #[test]
    fn clear_after_mount_succeeds() {
        let mut m = SysCacheManager::default();
        cell_sys_cache_mount(&mut m, "data").unwrap();
        assert_eq!(cell_sys_cache_clear(&mut m).unwrap(), RET_OK_CLEARED);
        // `clear` in C++ keeps the mount point; only the contents are wiped.
        assert_eq!(m.current_id.as_deref(), Some("data"));
    }

    // --- cache_path helper ----------------------------------------

    #[test]
    fn cache_path_returns_none_when_unmounted() {
        let m = SysCacheManager::default();
        assert_eq!(cache_path(&m), None);
    }

    #[test]
    fn cache_path_returns_mounted_path() {
        let mut m = SysCacheManager::default();
        cell_sys_cache_mount(&mut m, "abc").unwrap();
        assert_eq!(cache_path(&m).as_deref(), Some("/dev_hdd1/caches/abc"));
    }
}
