//! `rpcs3-config` — Rust port of RPCS3 config files.
//!
//! **Phase 0 / Wave 1 scope:** `games.yml` only.
//!
//! The C++ reference lives at `rpcs3/Emu/games_config.cpp` and uses
//! yaml-cpp. We mirror its observable contract byte-for-byte:
//!
//! * Storage is a `BTreeMap<String, String>` (title_id → path).
//!   Lexicographic order on keys is preserved — matches `std::map`.
//! * **Load** silently discards entries where
//!   - the key is an empty string, OR
//!   - the value is not a YAML scalar, OR
//!   - the value is an empty string.
//!   If the root is not a map, the whole document is discarded
//!   (empty config) and a `NotAMap` error is reported.
//! * **Emit** produces `KEY: VALUE\n` lines, ordered by key. The
//!   format mirrors yaml-cpp's default block-map output for
//!   `std::map<string, string>` with simple ASCII values.

use std::collections::BTreeMap;

use yaml_rust2::{Yaml, YamlLoader};

// ---------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------

/// In-memory representation of `games.yml`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GamesConfig {
    games: BTreeMap<String, String>,
}

/// Mirrors `games_config::result` from `rpcs3/Emu/games_config.h:19-24`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddResult {
    /// The key did not exist; the pair was inserted.
    Success,
    /// The key already mapped to the same path; nothing changed.
    Exists,
}

/// Errors observable to the caller of `parse`. Non-fatal filters
/// (empty keys, empty values, non-scalar values) are NOT errors —
/// they are silently dropped to match the C++ behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Root node is not a YAML mapping. Matches `cfg_log.error(...)`
    /// path in `games_config.cpp:191` (silent when null).
    NotAMap,
    /// Underlying YAML scan/parse failure. Wraps `yaml-rust2` error.
    YamlScan(String),
}

// ---------------------------------------------------------------------
// Core API
// ---------------------------------------------------------------------

impl GamesConfig {
    /// Empty config, same as default-constructed `games_config`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parses `games.yml` content. The `(config, error)` pair mirrors
    /// the C++ `yaml_load` flow: the config is always returned (empty
    /// if the input was malformed) and the error is diagnostic.
    ///
    /// Empty input and all-whitespace input both produce an empty
    /// config with `None` error — same as C++ with an empty file
    /// (`result.IsNull()` branch).
    #[must_use]
    pub fn parse(yaml: &str) -> (Self, Option<ParseError>) {
        if yaml.trim().is_empty() {
            return (Self::default(), None);
        }

        let docs = match YamlLoader::load_from_str(yaml) {
            Ok(docs) => docs,
            Err(e) => return (Self::default(), Some(ParseError::YamlScan(e.to_string()))),
        };

        let root = match docs.into_iter().next() {
            Some(r) => r,
            None => return (Self::default(), None),
        };

        match root {
            Yaml::Hash(map) => {
                let mut games = BTreeMap::new();
                for (k, v) in map {
                    let (Yaml::String(key), Yaml::String(value)) = (k, v) else {
                        // Non-scalar value or non-string key: drop silently,
                        // matching the C++ filter in games_config.cpp:198.
                        continue;
                    };
                    if key.is_empty() || value.is_empty() {
                        continue;
                    }
                    games.insert(key, value);
                }
                (Self { games }, None)
            }
            Yaml::Null | Yaml::BadValue => (Self::default(), None),
            _ => (Self::default(), Some(ParseError::NotAMap)),
        }
    }

    /// Serializes the config to the exact byte sequence that yaml-cpp
    /// produces for `std::map<string,string>` with simple ASCII values.
    ///
    /// Format: `KEY: VALUE\n` per entry, sorted lex by KEY.
    /// Empty config → empty string (matches yaml-cpp for empty map).
    #[must_use]
    pub fn emit(&self) -> String {
        if self.games.is_empty() {
            // yaml-cpp emits "{}" for an explicitly empty map, but
            // RPCS3 always writes through a populated `m_games`; in
            // practice the file is deleted when empty. The canonical
            // observable output for "no games" is an empty file.
            return String::new();
        }

        // Estimate: ~32 bytes/entry average. Good for single alloc on
        // typical game lists (tens to low hundreds of entries).
        let mut out = String::with_capacity(self.games.len() * 48);
        for (k, v) in &self.games {
            debug_assert!(!k.is_empty() && !v.is_empty());
            out.push_str(k);
            out.push_str(": ");
            out.push_str(v);
            out.push('\n');
        }
        out
    }

    /// Matches `games_config::get_games` — returns a cloned view.
    #[must_use]
    pub fn games(&self) -> BTreeMap<String, String> {
        self.games.clone()
    }

    /// Matches `games_config::get_path(title_id)`:
    /// empty input → empty output; unknown title → empty output.
    #[must_use]
    pub fn get_path(&self, title_id: &str) -> &str {
        if title_id.is_empty() {
            return "";
        }
        self.games.get(title_id).map_or("", String::as_str)
    }

    /// Matches `games_config::add_game`.
    ///
    /// Semantic difference vs C++: we don't persist automatically
    /// (C++ has `m_save_on_dirty` — not our concern here; persistence
    /// is the caller's responsibility).
    pub fn add_game(&mut self, title_id: impl Into<String>, path: impl Into<String>) -> AddResult {
        let key = title_id.into();
        let value = path.into();

        match self.games.get(&key) {
            Some(existing) if *existing == value => AddResult::Exists,
            _ => {
                self.games.insert(key, value);
                AddResult::Success
            }
        }
    }

    /// Matches `games_config::remove_game`. Returns `true` if an
    /// entry was actually removed.
    pub fn remove_game(&mut self, title_id: &str) -> bool {
        self.games.remove(title_id).is_some()
    }

    /// Matches `games_config::add_external_hdd_game` — strips a
    /// trailing `/C00` or `\C00` before delegating to `add_game`.
    /// Mirrors `games_config.cpp:86-93`.
    pub fn add_external_hdd_game(
        &mut self,
        title_id: impl Into<String>,
        path: impl Into<String>,
    ) -> AddResult {
        let mut path = path.into();
        if path.ends_with("/C00") || path.ends_with("\\C00") {
            path.truncate(path.len() - 4);
        }
        self.add_game(title_id, path)
    }
}

// ---------------------------------------------------------------------
// Tests — match the parity style of rpcs3-utilities
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Parse --------------------------------------------------------

    #[test]
    fn empty_input_gives_empty_config() {
        let (cfg, err) = GamesConfig::parse("");
        assert_eq!(cfg.games().len(), 0);
        assert!(err.is_none());
    }

    #[test]
    fn whitespace_only_gives_empty_config() {
        let (cfg, err) = GamesConfig::parse("   \n\n\t  \n");
        assert_eq!(cfg.games().len(), 0);
        assert!(err.is_none());
    }

    #[test]
    fn yaml_null_gives_empty_config_no_error() {
        // Matches the C++ `result.IsNull()` branch at games_config.cpp:189
        let (cfg, err) = GamesConfig::parse("~\n");
        assert_eq!(cfg.games().len(), 0);
        assert!(err.is_none());
    }

    #[test]
    fn non_map_root_reports_error() {
        let (cfg, err) = GamesConfig::parse("- just a sequence\n- of items\n");
        assert_eq!(cfg.games().len(), 0);
        assert_eq!(err, Some(ParseError::NotAMap));
    }

    #[test]
    fn simple_map_parses() {
        let src = "BLES01234: /path/to/game\nBCES56789: /other/path\n";
        let (cfg, err) = GamesConfig::parse(src);
        assert!(err.is_none());
        assert_eq!(cfg.get_path("BLES01234"), "/path/to/game");
        assert_eq!(cfg.get_path("BCES56789"), "/other/path");
        assert_eq!(cfg.get_path("UNKNOWN"), "");
    }

    #[test]
    fn empty_key_entry_is_dropped() {
        // Matches games_config.cpp:198 `!entry.first.Scalar().empty()`.
        let src = "\"\": /some/path\nBLES01234: /game\n";
        let (cfg, _err) = GamesConfig::parse(src);
        assert_eq!(cfg.games().len(), 1);
        assert_eq!(cfg.get_path("BLES01234"), "/game");
    }

    #[test]
    fn empty_value_entry_is_dropped() {
        // Matches games_config.cpp:198 `!entry.second.Scalar().empty()`.
        let src = "BLES01234: \"\"\nBCES56789: /ok\n";
        let (cfg, _err) = GamesConfig::parse(src);
        assert_eq!(cfg.games().len(), 1);
        assert_eq!(cfg.get_path("BCES56789"), "/ok");
    }

    #[test]
    fn non_scalar_value_is_dropped() {
        // Matches games_config.cpp:198 `entry.second.IsScalar()`.
        let src = "BLES01234:\n  - nested\n  - list\nBCES56789: /ok\n";
        let (cfg, _err) = GamesConfig::parse(src);
        assert_eq!(cfg.games().len(), 1);
        assert_eq!(cfg.get_path("BCES56789"), "/ok");
    }

    #[test]
    fn empty_title_id_lookup_is_empty() {
        let (cfg, _) = GamesConfig::parse("BLES01234: /game\n");
        // Matches games_config.cpp:32 early-return for empty title_id.
        assert_eq!(cfg.get_path(""), "");
    }

    // -- Emit ---------------------------------------------------------

    #[test]
    fn empty_config_emits_empty_string() {
        assert_eq!(GamesConfig::new().emit(), "");
    }

    #[test]
    fn emit_order_is_lexicographic() {
        let mut cfg = GamesConfig::new();
        // Insert in non-sorted order on purpose.
        cfg.add_game("NPEB00001", "/p3");
        cfg.add_game("BCES00001", "/p1");
        cfg.add_game("BLES00001", "/p2");

        let out = cfg.emit();
        assert_eq!(out, "BCES00001: /p1\nBLES00001: /p2\nNPEB00001: /p3\n");
    }

    // -- Round-trip ---------------------------------------------------

    #[test]
    fn round_trip_preserves_content() {
        let src = "BCES00001: /p1\nBLES00001: /p2\nNPEB00001: /p3\n";
        let (cfg, err) = GamesConfig::parse(src);
        assert!(err.is_none());
        assert_eq!(cfg.emit(), src);
    }

    #[test]
    fn round_trip_sorts_unsorted_input() {
        // Input out of order, but output always sorted — this is the
        // contract with `std::map` on the C++ side.
        let src = "NPEB00001: /p3\nBCES00001: /p1\nBLES00001: /p2\n";
        let (cfg, _) = GamesConfig::parse(src);
        let sorted = "BCES00001: /p1\nBLES00001: /p2\nNPEB00001: /p3\n";
        assert_eq!(cfg.emit(), sorted);
    }

    // -- Mutations ----------------------------------------------------

    #[test]
    fn add_game_returns_success_on_insert() {
        let mut cfg = GamesConfig::new();
        assert_eq!(cfg.add_game("BLES01234", "/a"), AddResult::Success);
        assert_eq!(cfg.get_path("BLES01234"), "/a");
    }

    #[test]
    fn add_game_returns_exists_for_same_path() {
        let mut cfg = GamesConfig::new();
        cfg.add_game("BLES01234", "/a");
        assert_eq!(cfg.add_game("BLES01234", "/a"), AddResult::Exists);
    }

    #[test]
    fn add_game_overwrites_when_path_changes() {
        let mut cfg = GamesConfig::new();
        cfg.add_game("BLES01234", "/old");
        assert_eq!(cfg.add_game("BLES01234", "/new"), AddResult::Success);
        assert_eq!(cfg.get_path("BLES01234"), "/new");
    }

    #[test]
    fn remove_game_true_when_existed() {
        let mut cfg = GamesConfig::new();
        cfg.add_game("X", "/y");
        assert!(cfg.remove_game("X"));
        assert!(!cfg.remove_game("X"));
    }

    #[test]
    fn add_external_hdd_strips_c00_suffix_fwd_slash() {
        // Matches games_config.cpp:89-92.
        let mut cfg = GamesConfig::new();
        cfg.add_external_hdd_game("BLES01234", "/mnt/disk/BLES01234/C00");
        assert_eq!(cfg.get_path("BLES01234"), "/mnt/disk/BLES01234");
    }

    #[test]
    fn add_external_hdd_strips_c00_suffix_back_slash() {
        let mut cfg = GamesConfig::new();
        cfg.add_external_hdd_game("BLES01234", "C:\\games\\BLES01234\\C00");
        assert_eq!(cfg.get_path("BLES01234"), "C:\\games\\BLES01234");
    }

    #[test]
    fn add_external_hdd_leaves_path_without_c00() {
        let mut cfg = GamesConfig::new();
        cfg.add_external_hdd_game("BLES01234", "/games/BLES01234");
        assert_eq!(cfg.get_path("BLES01234"), "/games/BLES01234");
    }

    #[test]
    fn add_external_hdd_does_not_strip_mid_path_c00() {
        // Only strips TRAILING /C00 — confirms the ends_with guard.
        let mut cfg = GamesConfig::new();
        cfg.add_external_hdd_game("BLES01234", "/games/C00/something");
        assert_eq!(cfg.get_path("BLES01234"), "/games/C00/something");
    }
}
