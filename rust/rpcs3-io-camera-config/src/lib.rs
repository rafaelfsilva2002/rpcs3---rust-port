//! `rpcs3-io-camera-config` — Rust port of `rpcs3/Emu/Io/camera_config.cpp` + `.h`.
//!
//! Camera binding config: 6 fields (width/height/min_fps/max_fps/format/colorspace)
//! serialized as a single comma-separated string under the map key
//! `"<handler>-<camera>"` inside `cfg_camera.cameras`.
//!
//! Frozen:
//!
//! - `camera.yml` config file name (cpp:11).
//! - `CameraSetting::to_string()` format: `"{width},{height},{min_fps},{max_fps},{format},{colorspace}"` (cpp:68..71).
//! - `from_string(text)` validation: exactly 6 CSV fields, integer parse for 4 fields, double parse for 2 FPS fields, reset all to 0 on any failure (cpp:73..126).
//! - Map key composition `"<handler>-<camera>"` (cpp:42, 65).
//! - Empty handler/camera rejected in set (cpp:53..63).
//! - `MEMBER_COUNT = 6` (header:20).

pub const CONFIG_FILE_NAME: &str = "camera.yml";
pub const MEMBER_COUNT: usize = 6;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CameraSetting {
    pub width: i32,
    pub height: i32,
    pub min_fps: f64,
    pub max_fps: f64,
    pub format: i32,
    pub colorspace: i32,
}

impl CameraSetting {
    /// `to_string()` (cpp:68..71). Matches `%d,%d,%f,%f,%d,%d`.
    #[must_use]
    pub fn to_string_cpp(&self) -> String {
        format!(
            "{},{},{},{},{},{}",
            self.width, self.height, self.min_fps, self.max_fps, self.format, self.colorspace
        )
    }

    /// `from_string(text)` (cpp:73..126). Silently resets every field to 0
    /// if parsing fails at any step, mirroring cpp. Empty input is a no-op
    /// (cpp:75..78).
    pub fn from_string(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let parts: Vec<&str> = text.split(',').collect();

        if parts.len() != MEMBER_COUNT {
            return;
        }

        let ints: Option<(i32, i32, i32, i32)> = (|| {
            Some((
                parts[0].parse::<i32>().ok()?,
                parts[1].parse::<i32>().ok()?,
                parts[4].parse::<i32>().ok()?,
                parts[5].parse::<i32>().ok()?,
            ))
        })();

        let doubles: Option<(f64, f64)> = (|| {
            Some((parts[2].parse::<f64>().ok()?, parts[3].parse::<f64>().ok()?))
        })();

        if let (Some((w, h, f, cs)), Some((min, max))) = (ints, doubles) {
            self.width = w;
            self.height = h;
            self.min_fps = min;
            self.max_fps = max;
            self.format = f;
            self.colorspace = cs;
        } else {
            // cpp:119..124 zeros every field on any parse failure.
            self.width = 0;
            self.height = 0;
            self.min_fps = 0.0;
            self.max_fps = 0.0;
            self.format = 0;
            self.colorspace = 0;
        }
    }
}

/// Compose the `<handler>-<camera>` map key used in `cfg_camera.cameras`
/// (cpp:42, 65).
#[must_use]
pub fn map_key(handler: &str, camera: &str) -> String {
    format!("{handler}-{camera}")
}

/// Whether a `set_camera_setting(handler, camera, ...)` call would be
/// accepted (cpp:53..63). Both sides must be non-empty.
#[must_use]
pub const fn is_settable(handler: &str, camera: &str) -> bool {
    !handler.is_empty() && !camera.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_and_member_count() {
        assert_eq!(CONFIG_FILE_NAME, "camera.yml");
        assert_eq!(MEMBER_COUNT, 6);
    }

    #[test]
    fn to_string_format() {
        let s = CameraSetting { width: 640, height: 480, min_fps: 15.0, max_fps: 30.0, format: 2, colorspace: 1 };
        assert_eq!(s.to_string_cpp(), "640,480,15,30,2,1");
    }

    #[test]
    fn from_string_round_trip() {
        let mut s = CameraSetting::default();
        s.from_string("1280,720,30,60,3,2");
        assert_eq!(s.width, 1280);
        assert_eq!(s.height, 720);
        assert_eq!(s.min_fps, 30.0);
        assert_eq!(s.max_fps, 60.0);
        assert_eq!(s.format, 3);
        assert_eq!(s.colorspace, 2);
    }

    #[test]
    fn from_string_empty_leaves_unchanged() {
        let mut s = CameraSetting { width: 999, height: 999, ..Default::default() };
        s.from_string("");
        assert_eq!(s.width, 999, "empty string should not touch fields");
    }

    #[test]
    fn from_string_wrong_arity_ignored() {
        let mut s = CameraSetting { width: 999, ..Default::default() };
        s.from_string("1,2,3"); // 3 fields, not 6.
        assert_eq!(s.width, 999);
    }

    #[test]
    fn from_string_parse_failure_zeroes_all() {
        let mut s = CameraSetting {
            width: 1920, height: 1080, min_fps: 30.0, max_fps: 60.0, format: 1, colorspace: 1,
        };
        // "abc" isn't a valid integer → full reset (cpp:119..124).
        s.from_string("abc,720,30,60,1,1");
        assert_eq!(s, CameraSetting::default());
    }

    #[test]
    fn map_key_composition() {
        assert_eq!(map_key("qt", "device0"), "qt-device0");
        assert_eq!(map_key("", ""), "-");
    }

    #[test]
    fn is_settable_rejects_empties() {
        assert!(is_settable("qt", "cam0"));
        assert!(!is_settable("", "cam0"));
        assert!(!is_settable("qt", ""));
        assert!(!is_settable("", ""));
    }

    #[test]
    fn defaults_all_zero() {
        let s = CameraSetting::default();
        assert_eq!(s.width, 0);
        assert_eq!(s.height, 0);
        assert_eq!(s.min_fps, 0.0);
        assert_eq!(s.max_fps, 0.0);
        assert_eq!(s.format, 0);
        assert_eq!(s.colorspace, 0);
    }
}
