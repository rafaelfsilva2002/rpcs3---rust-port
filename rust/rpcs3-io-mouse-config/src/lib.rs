//! `rpcs3-io-mouse-config` — Rust port of `rpcs3/Emu/Io/mouse_config.cpp` + `.h`.
//!
//! Single shared 8-button mouse binding config (the comment at cpp:5
//! explains: RPCS3 uses one config rather than 127 per-mouse ones for
//! simplicity). First three buttons default to "Mouse Left/Right/Middle";
//! buttons 4..8 are empty by default (games can bind them via the UI).
//!
//! Frozen from the cpp header:
//!
//! - Config file name: `config_mouse.yml`.
//! - Default bindings: Button 1 = "Mouse Left", 2 = "Mouse Right",
//!   3 = "Mouse Middle", 4..8 = "".
//! - `get_button(code)` maps `CELL_MOUSE_BUTTON_1..=8` → the matching
//!   binding field; anything else is out-of-range (cpp throws, we return
//!   `None`).

pub const CONFIG_FILE_NAME: &str = "config_mouse.yml";

/// `CELL_MOUSE_BUTTON_*` from `<Emu/Cell/Modules/cellMouse.h>`. The codes
/// are 1-based to match the cpp enum.
pub const CELL_MOUSE_BUTTON_1: i32 = 1;
pub const CELL_MOUSE_BUTTON_2: i32 = 2;
pub const CELL_MOUSE_BUTTON_3: i32 = 4;
pub const CELL_MOUSE_BUTTON_4: i32 = 8;
pub const CELL_MOUSE_BUTTON_5: i32 = 16;
pub const CELL_MOUSE_BUTTON_6: i32 = 32;
pub const CELL_MOUSE_BUTTON_7: i32 = 64;
pub const CELL_MOUSE_BUTTON_8: i32 = 128;

pub const DEFAULT_BUTTON_1: &str = "Mouse Left";
pub const DEFAULT_BUTTON_2: &str = "Mouse Right";
pub const DEFAULT_BUTTON_3: &str = "Mouse Middle";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MouseConfig {
    pub button_1: String,
    pub button_2: String,
    pub button_3: String,
    pub button_4: String,
    pub button_5: String,
    pub button_6: String,
    pub button_7: String,
    pub button_8: String,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            button_1: DEFAULT_BUTTON_1.to_string(),
            button_2: DEFAULT_BUTTON_2.to_string(),
            button_3: DEFAULT_BUTTON_3.to_string(),
            button_4: String::new(),
            button_5: String::new(),
            button_6: String::new(),
            button_7: String::new(),
            button_8: String::new(),
        }
    }
}

impl MouseConfig {
    /// `get_button(code)` (cpp:44..58). Returns `None` for out-of-range
    /// codes (cpp throws).
    pub fn get_button(&self, code: i32) -> Option<&str> {
        match code {
            CELL_MOUSE_BUTTON_1 => Some(&self.button_1),
            CELL_MOUSE_BUTTON_2 => Some(&self.button_2),
            CELL_MOUSE_BUTTON_3 => Some(&self.button_3),
            CELL_MOUSE_BUTTON_4 => Some(&self.button_4),
            CELL_MOUSE_BUTTON_5 => Some(&self.button_5),
            CELL_MOUSE_BUTTON_6 => Some(&self.button_6),
            CELL_MOUSE_BUTTON_7 => Some(&self.button_7),
            CELL_MOUSE_BUTTON_8 => Some(&self.button_8),
            _ => None,
        }
    }

    pub fn get_button_mut(&mut self, code: i32) -> Option<&mut String> {
        match code {
            CELL_MOUSE_BUTTON_1 => Some(&mut self.button_1),
            CELL_MOUSE_BUTTON_2 => Some(&mut self.button_2),
            CELL_MOUSE_BUTTON_3 => Some(&mut self.button_3),
            CELL_MOUSE_BUTTON_4 => Some(&mut self.button_4),
            CELL_MOUSE_BUTTON_5 => Some(&mut self.button_5),
            CELL_MOUSE_BUTTON_6 => Some(&mut self.button_6),
            CELL_MOUSE_BUTTON_7 => Some(&mut self.button_7),
            CELL_MOUSE_BUTTON_8 => Some(&mut self.button_8),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_name() {
        assert_eq!(CONFIG_FILE_NAME, "config_mouse.yml");
    }

    #[test]
    fn default_bindings_match_cpp_header() {
        let c = MouseConfig::default();
        assert_eq!(c.button_1, "Mouse Left");
        assert_eq!(c.button_2, "Mouse Right");
        assert_eq!(c.button_3, "Mouse Middle");
        assert_eq!(c.button_4, "");
        assert_eq!(c.button_5, "");
        assert_eq!(c.button_6, "");
        assert_eq!(c.button_7, "");
        assert_eq!(c.button_8, "");
    }

    #[test]
    fn get_button_maps_codes_to_fields() {
        let c = MouseConfig::default();
        assert_eq!(c.get_button(CELL_MOUSE_BUTTON_1), Some("Mouse Left"));
        assert_eq!(c.get_button(CELL_MOUSE_BUTTON_3), Some("Mouse Middle"));
        assert_eq!(c.get_button(CELL_MOUSE_BUTTON_8), Some(""));
        assert_eq!(c.get_button(0), None);
        assert_eq!(c.get_button(9999), None);
    }

    #[test]
    fn get_button_mut_allows_rebind() {
        let mut c = MouseConfig::default();
        if let Some(b) = c.get_button_mut(CELL_MOUSE_BUTTON_4) {
            *b = "Mouse Back".to_string();
        }
        assert_eq!(c.button_4, "Mouse Back");
        assert_eq!(c.get_button(CELL_MOUSE_BUTTON_4), Some("Mouse Back"));
    }

    #[test]
    fn button_codes_are_powers_of_two() {
        for c in [
            CELL_MOUSE_BUTTON_1, CELL_MOUSE_BUTTON_2, CELL_MOUSE_BUTTON_3, CELL_MOUSE_BUTTON_4,
            CELL_MOUSE_BUTTON_5, CELL_MOUSE_BUTTON_6, CELL_MOUSE_BUTTON_7, CELL_MOUSE_BUTTON_8,
        ] {
            assert!(c > 0 && (c & (c - 1)) == 0, "{c} not power of two");
        }
    }
}
