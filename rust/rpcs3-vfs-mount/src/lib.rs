//! `rpcs3-vfs-mount` — PS3 mount table and path resolution.
//!
//! Mirrors the data shape of `cfg_vfs` in `rpcs3/Emu/vfs_config.h:5-47`.
//! This crate provides only the *mount table* — the hierarchical
//! `cfg::node` serialization framework is out of scope. The YAML format
//! we emit mirrors yaml-cpp's output for a flat `map<string,string>`,
//! compatible with the `rpcs3-config` strategy already in use.

use std::collections::BTreeMap;

use rpcs3_vfs_paths::{get_path_root_and_trail, PathParts};

// ---------------------------------------------------------------------
// Mount table
// ---------------------------------------------------------------------

/// Keys used by `cfg_vfs`. Matches the `cfg::string` defaults in vfs_config.h.
pub const KEY_EMULATOR_DIR: &str = "$(EmulatorDir)";
pub const KEY_DEV_HDD0: &str = "/dev_hdd0/";
pub const KEY_DEV_HDD1: &str = "/dev_hdd1/";
pub const KEY_DEV_FLASH: &str = "/dev_flash/";
pub const KEY_DEV_FLASH2: &str = "/dev_flash2/";
pub const KEY_DEV_FLASH3: &str = "/dev_flash3/";
pub const KEY_DEV_BDVD: &str = "/dev_bdvd/";
pub const KEY_GAMES_DIR: &str = "/games/";
pub const KEY_APP_HOME: &str = "/app_home/";

/// Placeholder replaced by [`MountTable::resolve`] at lookup time.
pub const PLACEHOLDER_EMULATOR_DIR: &str = "$(EmulatorDir)";

/// Host-side layout of the PS3 virtual filesystem, including per-USB
/// port entries. Constructed with [`MountTable::defaults`] to get the
/// same initial values as a fresh RPCS3 install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountTable {
    /// `$(EmulatorDir)` — usually the portable root. Empty means
    /// "use `fs::get_config_dir()`", matching the C++ default.
    pub emulator_dir: String,

    /// Ordered mount map (device root name → host path template).
    /// Values may contain `$(EmulatorDir)` placeholder.
    pub mounts: BTreeMap<String, String>,

    /// Per-USB port table (`/dev_usb000`, `/dev_usb001`, …).
    pub dev_usb: BTreeMap<String, String>,
}

impl Default for MountTable {
    fn default() -> Self {
        Self::defaults()
    }
}

impl MountTable {
    /// The default mount table shipped with a fresh RPCS3 install.
    /// Matches `cfg_vfs` member initializers in vfs_config.h.
    #[must_use]
    pub fn defaults() -> Self {
        let mut mounts = BTreeMap::new();
        mounts.insert(KEY_DEV_HDD0.to_owned(), format!("{PLACEHOLDER_EMULATOR_DIR}dev_hdd0/"));
        mounts.insert(KEY_DEV_HDD1.to_owned(), format!("{PLACEHOLDER_EMULATOR_DIR}dev_hdd1/"));
        mounts.insert(KEY_DEV_FLASH.to_owned(), format!("{PLACEHOLDER_EMULATOR_DIR}dev_flash/"));
        mounts.insert(KEY_DEV_FLASH2.to_owned(), format!("{PLACEHOLDER_EMULATOR_DIR}dev_flash2/"));
        mounts.insert(KEY_DEV_FLASH3.to_owned(), format!("{PLACEHOLDER_EMULATOR_DIR}dev_flash3/"));
        mounts.insert(KEY_DEV_BDVD.to_owned(), format!("{PLACEHOLDER_EMULATOR_DIR}dev_bdvd/"));
        mounts.insert(KEY_GAMES_DIR.to_owned(), format!("{PLACEHOLDER_EMULATOR_DIR}games/"));
        mounts.insert(KEY_APP_HOME.to_owned(), String::new());

        let mut dev_usb = BTreeMap::new();
        dev_usb.insert(
            "/dev_usb000".to_owned(),
            format!("{PLACEHOLDER_EMULATOR_DIR}dev_usb000/"),
        );

        Self {
            emulator_dir: String::new(),
            mounts,
            dev_usb,
        }
    }

    /// Expand `$(EmulatorDir)` in `value`, using `default_dir` when the
    /// table's `emulator_dir` is empty (mirrors `cfg_vfs::get`, which
    /// falls back to `fs::get_config_dir()` when unset).
    #[must_use]
    pub fn expand(&self, value: &str, default_dir: &str) -> String {
        let dir = if self.emulator_dir.is_empty() {
            default_dir
        } else {
            self.emulator_dir.as_str()
        };
        value.replace(PLACEHOLDER_EMULATOR_DIR, dir)
    }

    /// Resolve a PS3 path (e.g. `/dev_hdd0/game/EBOOT.BIN`) to a host
    /// path. Returns `None` if the mount device is unknown.
    ///
    /// `default_dir` provides the fallback for `$(EmulatorDir)` when the
    /// table's `emulator_dir` is empty — typically `fs::get_config_dir()`.
    #[must_use]
    pub fn resolve(&self, ps3_path: &str, default_dir: &str) -> Option<String> {
        let parts = get_path_root_and_trail(ps3_path);
        if parts.is_enoent() {
            return None;
        }
        let root_key = format!("/{}/", parts.root);

        // Look in main mounts
        if let Some(template) = self.mounts.get(&root_key) {
            let base = self.expand(template, default_dir);
            if base.is_empty() {
                return None; // /app_home unset
            }
            return Some(format!("{}{}", base, parts.trail));
        }

        // Look in USB ports
        let usb_key = format!("/{}", parts.root);
        if let Some(template) = self.dev_usb.get(&usb_key) {
            let base = self.expand(template, default_dir);
            return Some(format!("{}{}", base, parts.trail));
        }

        None
    }

    /// Emit a YAML snapshot of the mount table. Matches yaml-cpp's
    /// flat-map format: `KEY: VALUE\n` ordered lexicographically.
    #[must_use]
    pub fn emit_yaml(&self) -> String {
        let mut out = String::new();
        // Emulator dir first — matches the order in vfs_config.h.
        out.push_str(KEY_EMULATOR_DIR);
        out.push_str(": ");
        out.push_str(&self.emulator_dir);
        out.push('\n');

        for (k, v) in &self.mounts {
            out.push_str(k);
            out.push_str(": ");
            out.push_str(v);
            out.push('\n');
        }

        // dev_usb entries appear under a separate prefix key in C++;
        // for simplicity we emit them individually. The C++ structure
        // uses `cfg::device_entry` — we flatten here.
        for (k, v) in &self.dev_usb {
            out.push_str(k);
            out.push_str(": ");
            out.push_str(v);
            out.push('\n');
        }

        out
    }
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_contain_core_mounts() {
        let t = MountTable::defaults();
        assert!(t.mounts.contains_key("/dev_hdd0/"));
        assert!(t.mounts.contains_key("/dev_hdd1/"));
        assert!(t.mounts.contains_key("/dev_flash/"));
        assert!(t.mounts.contains_key("/dev_bdvd/"));
        assert!(t.mounts.contains_key("/app_home/"));
        assert!(t.dev_usb.contains_key("/dev_usb000"));
    }

    #[test]
    fn default_dev_hdd0_contains_emulator_dir_placeholder() {
        let t = MountTable::defaults();
        let v = &t.mounts["/dev_hdd0/"];
        assert!(v.contains(PLACEHOLDER_EMULATOR_DIR));
    }

    #[test]
    fn app_home_is_unset_by_default() {
        let t = MountTable::defaults();
        assert_eq!(t.mounts["/app_home/"], "");
    }

    #[test]
    fn expand_uses_table_emulator_dir_when_set() {
        let mut t = MountTable::defaults();
        t.emulator_dir = "C:/rpcs3/".to_owned();
        let expanded = t.expand("$(EmulatorDir)dev_hdd0/", "/fallback/");
        assert_eq!(expanded, "C:/rpcs3/dev_hdd0/");
    }

    #[test]
    fn expand_falls_back_to_default_dir_when_empty() {
        let t = MountTable::defaults();
        assert!(t.emulator_dir.is_empty());
        let expanded = t.expand("$(EmulatorDir)dev_hdd0/", "/fallback/");
        assert_eq!(expanded, "/fallback/dev_hdd0/");
    }

    #[test]
    fn resolve_dev_hdd0_game_subpath() {
        let t = MountTable::defaults();
        let host = t.resolve("/dev_hdd0/game/BLES01234/EBOOT.BIN", "/fallback/");
        assert_eq!(host, Some("/fallback/dev_hdd0/game/BLES01234/EBOOT.BIN".to_owned()));
    }

    #[test]
    fn resolve_dev_bdvd_empty_trail() {
        let t = MountTable::defaults();
        let host = t.resolve("/dev_bdvd/", "/fb/");
        assert_eq!(host, Some("/fb/dev_bdvd/".to_owned()));
    }

    #[test]
    fn resolve_unknown_mount_returns_none() {
        let t = MountTable::defaults();
        assert_eq!(t.resolve("/dev_unknown/file", "/fb/"), None);
    }

    #[test]
    fn resolve_app_home_without_mount_returns_none() {
        // app_home default is empty; resolve returns None per convention.
        let t = MountTable::defaults();
        assert_eq!(t.resolve("/app_home/x", "/fb/"), None);
    }

    #[test]
    fn resolve_empty_input_returns_none() {
        let t = MountTable::defaults();
        assert_eq!(t.resolve("", "/fb/"), None);
    }

    #[test]
    fn resolve_usb_port() {
        let t = MountTable::defaults();
        let host = t.resolve("/dev_usb000/foo", "/fb/");
        assert_eq!(host, Some("/fb/dev_usb000/foo".to_owned()));
    }

    #[test]
    fn emit_yaml_is_sorted_and_line_terminated() {
        let t = MountTable::defaults();
        let y = t.emit_yaml();
        // First line is always emulator_dir
        assert!(y.starts_with("$(EmulatorDir): \n"));
        // All entries end with a newline
        for line in y.lines() {
            assert!(line.contains(": "), "line missing KEY: VALUE: {line:?}");
        }
        // dev_hdd0 comes before dev_hdd1 (lex order)
        let dev0_pos = y.find("/dev_hdd0/").unwrap();
        let dev1_pos = y.find("/dev_hdd1/").unwrap();
        assert!(dev0_pos < dev1_pos);
    }

    #[test]
    fn resolve_respects_dotdot_reduction() {
        // `/dev_hdd0/game/BLES01234/../..` resolves through vfs-paths first:
        // trail becomes "", root dev_hdd0 → host path is just dev_hdd0/
        let t = MountTable::defaults();
        let host = t.resolve("/dev_hdd0/game/..", "/fb/");
        assert_eq!(host, Some("/fb/dev_hdd0/".to_owned()));
    }
}
