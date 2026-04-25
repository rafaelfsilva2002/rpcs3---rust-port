//! `rpcs3-hle-cellscreenshot` — screenshot capture HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellScreenshot.cpp`. Lets games stamp
//! screenshots with a title / comment and optionally blend in a PNG
//! overlay (e.g. a watermark) before the XMB saves the shot.
//!
//! ## Entry points covered
//!
//! | HLE function                           | Rust wrapper                               |
//! |----------------------------------------|--------------------------------------------|
//! | `cellScreenShotEnable`                 | [`ScreenshotManager::enable`]              |
//! | `cellScreenShotDisable`                | [`ScreenshotManager::disable`]             |
//! | `cellScreenShotSetParameter`           | [`ScreenshotManager::set_parameter`]       |
//! | `cellScreenShotSetOverlayImage`        | [`ScreenshotManager::set_overlay_image`]   |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellScreenshot.h:6-13
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const INTERNAL: CellError = CellError(0x8002_d101);
    pub const PARAM: CellError = CellError(0x8002_d102);
    pub const DECODE: CellError = CellError(0x8002_d103);
    pub const NOSPACE: CellError = CellError(0x8002_d104);
    pub const UNSUPPORTED_COLOR_FORMAT: CellError = CellError(0x8002_d105);
}

// =====================================================================
// Size constants (cellScreenshot.h:15-20)
// =====================================================================

pub const PHOTO_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_TITLE_MAX_LENGTH: usize = 64;
pub const GAME_COMMENT_MAX_SIZE: usize = 1024;

/// Overlay filesystem roots allowed by the PS3 shell (used by SetOverlayImage).
pub const ALLOWED_OVERLAY_ROOTS: &[&str] = &["/dev_hdd0", "/dev_hdd1", "/dev_bdvd"];

// =====================================================================
// Types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetParam {
    pub photo_title: String,
    pub game_title: String,
    pub game_comment: String,
}

impl SetParam {
    fn validate(&self) -> Result<(), CellError> {
        // PS3 UTF-8 convention is 3 bytes / char → title fields fit up to
        // MAX_LENGTH * 3 bytes. `GAME_COMMENT_MAX_SIZE` is raw bytes.
        if self.photo_title.len() >= PHOTO_TITLE_MAX_LENGTH * 3 {
            return Err(errors::PARAM);
        }
        if self.game_title.len() >= GAME_TITLE_MAX_LENGTH * 3 {
            return Err(errors::PARAM);
        }
        if self.game_comment.len() >= GAME_COMMENT_MAX_SIZE {
            return Err(errors::PARAM);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub dir_name: String,
    pub file_name: String,
    pub offset_x: i32,
    pub offset_y: i32,
}

// =====================================================================
// Manager — `screenshot_manager` in C++
// =====================================================================

#[derive(Clone, Debug)]
pub struct ScreenshotManager {
    enabled: bool,
    params: SetParam,
    overlay: Option<Overlay>,
}

impl ScreenshotManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            enabled: false,
            params: SetParam {
                photo_title: String::new(),
                game_title: String::new(),
                game_comment: String::new(),
            },
            overlay: None,
        }
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn params(&self) -> &SetParam {
        &self.params
    }

    #[must_use]
    pub fn overlay(&self) -> Option<&Overlay> {
        self.overlay.as_ref()
    }

    // ----------------- Enable / disable -----------------

    /// `cellScreenShotEnable()` — idempotent; the C++ code simply sets
    /// `is_enabled = true` without erroring on repeat.
    pub fn enable(&mut self) -> Result<(), CellError> {
        self.enabled = true;
        Ok(())
    }

    /// `cellScreenShotDisable()` — idempotent.
    pub fn disable(&mut self) -> Result<(), CellError> {
        self.enabled = false;
        Ok(())
    }

    // ----------------- Parameters -----------------

    /// `cellScreenShotSetParameter(param)`.
    pub fn set_parameter(&mut self, param: SetParam) -> Result<(), CellError> {
        param.validate()?;
        self.params = param;
        Ok(())
    }

    /// `cellScreenShotSetOverlayImage(dir_name, file_name, offset_x,
    /// offset_y)`. Validates that the overlay path is on a writable HDD /
    /// BDVD root and that the offsets are non-negative (UI can't blit
    /// above / left of the screen).
    pub fn set_overlay_image(
        &mut self,
        dir_name: impl Into<String>,
        file_name: impl Into<String>,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<(), CellError> {
        let dir = dir_name.into();
        let file = file_name.into();
        if dir.is_empty() || file.is_empty() {
            return Err(errors::PARAM);
        }
        if !ALLOWED_OVERLAY_ROOTS.iter().any(|r| dir.starts_with(r)) {
            return Err(errors::PARAM);
        }
        if offset_x < 0 || offset_y < 0 {
            return Err(errors::PARAM);
        }
        // Simple file-ext gate. PNG is canonical for the overlay slot;
        // the real lib rejects other formats with DECODE.
        let lower = file.to_ascii_lowercase();
        if !lower.ends_with(".png") {
            return Err(errors::DECODE);
        }
        self.overlay = Some(Overlay { dir_name: dir, file_name: file, offset_x, offset_y });
        Ok(())
    }

    /// Clear any overlay assigned via `set_overlay_image`.
    pub fn clear_overlay(&mut self) {
        self.overlay = None;
    }

    /// Returns the joined path the XMB shell would use to load the
    /// overlay PNG. Returns None if no overlay has been set. Mirrors
    /// `screenshot_info::get_overlay_path`.
    #[must_use]
    pub fn overlay_path(&self) -> Option<String> {
        self.overlay.as_ref().map(|o| {
            let sep = if o.dir_name.ends_with('/') { "" } else { "/" };
            format!("{}{}{}", o.dir_name, sep, o.file_name)
        })
    }

    /// Sanitized title accessors — mirror the C++ helpers that trim at
    /// max length. Used by the shell before stamping metadata.
    #[must_use]
    pub fn get_photo_title(&self) -> String {
        self.params.photo_title.chars().take(PHOTO_TITLE_MAX_LENGTH).collect()
    }

    #[must_use]
    pub fn get_game_title(&self) -> String {
        self.params.game_title.chars().take(GAME_TITLE_MAX_LENGTH).collect()
    }

    #[must_use]
    pub fn get_game_comment(&self) -> String {
        // GAME_COMMENT_MAX_SIZE is bytes, not chars. Truncate defensively.
        let mut out = String::new();
        for ch in self.params.game_comment.chars() {
            if out.len() + ch.len_utf8() >= GAME_COMMENT_MAX_SIZE {
                break;
            }
            out.push(ch);
        }
        out
    }
}

impl Default for ScreenshotManager {
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
        SetParam {
            photo_title: "Screenshot".into(),
            game_title: "My Game".into(),
            game_comment: "Cool shot".into(),
        }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::INTERNAL.0, 0x8002_d101);
        assert_eq!(errors::PARAM.0, 0x8002_d102);
        assert_eq!(errors::DECODE.0, 0x8002_d103);
        assert_eq!(errors::NOSPACE.0, 0x8002_d104);
        assert_eq!(errors::UNSUPPORTED_COLOR_FORMAT.0, 0x8002_d105);
    }

    #[test]
    fn constants_stable() {
        assert_eq!(PHOTO_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_TITLE_MAX_LENGTH, 64);
        assert_eq!(GAME_COMMENT_MAX_SIZE, 1024);
        assert_eq!(ALLOWED_OVERLAY_ROOTS.len(), 3);
    }

    #[test]
    fn fresh_manager_is_disabled() {
        let m = ScreenshotManager::new();
        assert!(!m.is_enabled());
        assert!(m.overlay().is_none());
    }

    #[test]
    fn enable_disable_idempotent() {
        let mut m = ScreenshotManager::new();
        m.enable().unwrap();
        assert!(m.is_enabled());
        m.enable().unwrap();
        assert!(m.is_enabled());
        m.disable().unwrap();
        assert!(!m.is_enabled());
        m.disable().unwrap();
        assert!(!m.is_enabled());
    }

    #[test]
    fn set_parameter_happy_path() {
        let mut m = ScreenshotManager::new();
        m.set_parameter(ok_param()).unwrap();
        assert_eq!(m.params().photo_title, "Screenshot");
        assert_eq!(m.params().game_title, "My Game");
        assert_eq!(m.params().game_comment, "Cool shot");
    }

    #[test]
    fn set_parameter_oversized_photo_title_rejected() {
        let mut m = ScreenshotManager::new();
        let mut p = ok_param();
        p.photo_title = "x".repeat(PHOTO_TITLE_MAX_LENGTH * 3);
        assert_eq!(m.set_parameter(p), Err(errors::PARAM));
    }

    #[test]
    fn set_parameter_oversized_game_title_rejected() {
        let mut m = ScreenshotManager::new();
        let mut p = ok_param();
        p.game_title = "x".repeat(GAME_TITLE_MAX_LENGTH * 3);
        assert_eq!(m.set_parameter(p), Err(errors::PARAM));
    }

    #[test]
    fn set_parameter_oversized_comment_rejected() {
        let mut m = ScreenshotManager::new();
        let mut p = ok_param();
        p.game_comment = "x".repeat(GAME_COMMENT_MAX_SIZE);
        assert_eq!(m.set_parameter(p), Err(errors::PARAM));
    }

    #[test]
    fn set_overlay_image_happy_path() {
        let mut m = ScreenshotManager::new();
        m.set_overlay_image("/dev_hdd0/game/data", "watermark.png", 100, 200).unwrap();
        let o = m.overlay().unwrap();
        assert_eq!(o.file_name, "watermark.png");
        assert_eq!(o.offset_x, 100);
        assert_eq!(o.offset_y, 200);
    }

    #[test]
    fn set_overlay_empty_dir_rejected() {
        let mut m = ScreenshotManager::new();
        assert_eq!(m.set_overlay_image("", "x.png", 0, 0), Err(errors::PARAM));
    }

    #[test]
    fn set_overlay_empty_file_rejected() {
        let mut m = ScreenshotManager::new();
        assert_eq!(m.set_overlay_image("/dev_hdd0/x", "", 0, 0), Err(errors::PARAM));
    }

    #[test]
    fn set_overlay_foreign_root_rejected() {
        let mut m = ScreenshotManager::new();
        assert_eq!(
            m.set_overlay_image("/dev_usb000/x", "a.png", 0, 0),
            Err(errors::PARAM)
        );
    }

    #[test]
    fn set_overlay_all_allowed_roots_accepted() {
        for root in ALLOWED_OVERLAY_ROOTS {
            let mut m = ScreenshotManager::new();
            m.set_overlay_image(format!("{root}/sub"), "a.png", 0, 0).unwrap();
        }
    }

    #[test]
    fn set_overlay_negative_offset_rejected() {
        let mut m = ScreenshotManager::new();
        assert_eq!(
            m.set_overlay_image("/dev_hdd0/x", "a.png", -1, 0),
            Err(errors::PARAM)
        );
        assert_eq!(
            m.set_overlay_image("/dev_hdd0/x", "a.png", 0, -10),
            Err(errors::PARAM)
        );
    }

    #[test]
    fn set_overlay_non_png_is_decode_error() {
        let mut m = ScreenshotManager::new();
        assert_eq!(
            m.set_overlay_image("/dev_hdd0/x", "img.jpg", 0, 0),
            Err(errors::DECODE)
        );
        assert_eq!(
            m.set_overlay_image("/dev_hdd0/x", "img.bmp", 0, 0),
            Err(errors::DECODE)
        );
    }

    #[test]
    fn set_overlay_png_case_insensitive() {
        let mut m = ScreenshotManager::new();
        m.set_overlay_image("/dev_hdd0/x", "IMG.PNG", 0, 0).unwrap();
    }

    #[test]
    fn clear_overlay_removes_assignment() {
        let mut m = ScreenshotManager::new();
        m.set_overlay_image("/dev_hdd0/x", "a.png", 0, 0).unwrap();
        assert!(m.overlay().is_some());
        m.clear_overlay();
        assert!(m.overlay().is_none());
    }

    #[test]
    fn overlay_path_joins_dir_and_file() {
        let mut m = ScreenshotManager::new();
        m.set_overlay_image("/dev_hdd0/x", "a.png", 0, 0).unwrap();
        assert_eq!(m.overlay_path().as_deref(), Some("/dev_hdd0/x/a.png"));
    }

    #[test]
    fn overlay_path_handles_trailing_slash() {
        let mut m = ScreenshotManager::new();
        m.set_overlay_image("/dev_hdd0/x/", "a.png", 0, 0).unwrap();
        assert_eq!(m.overlay_path().as_deref(), Some("/dev_hdd0/x/a.png"));
    }

    #[test]
    fn overlay_path_none_when_unset() {
        let m = ScreenshotManager::new();
        assert!(m.overlay_path().is_none());
    }

    #[test]
    fn get_photo_title_truncates_to_max() {
        let mut m = ScreenshotManager::new();
        let mut p = ok_param();
        p.photo_title = "a".repeat(100);
        m.set_parameter(p).unwrap();
        assert_eq!(m.get_photo_title().chars().count(), PHOTO_TITLE_MAX_LENGTH);
    }

    #[test]
    fn get_game_title_truncates_to_max() {
        let mut m = ScreenshotManager::new();
        let mut p = ok_param();
        p.game_title = "b".repeat(100);
        m.set_parameter(p).unwrap();
        assert_eq!(m.get_game_title().chars().count(), GAME_TITLE_MAX_LENGTH);
    }

    #[test]
    fn get_game_comment_truncates_safely() {
        let mut m = ScreenshotManager::new();
        let mut p = ok_param();
        p.game_comment = "c".repeat(GAME_COMMENT_MAX_SIZE - 1);
        m.set_parameter(p).unwrap();
        assert!(m.get_game_comment().len() < GAME_COMMENT_MAX_SIZE);
    }

    #[test]
    fn get_photo_title_empty_default() {
        let m = ScreenshotManager::new();
        assert_eq!(m.get_photo_title(), "");
    }

    #[test]
    fn full_screenshot_lifecycle_smoke() {
        let mut m = ScreenshotManager::new();
        m.enable().unwrap();
        m.set_parameter(ok_param()).unwrap();
        m.set_overlay_image("/dev_hdd0/game/data", "watermark.png", 50, 50).unwrap();
        assert!(m.is_enabled());
        assert_eq!(m.overlay_path().as_deref(), Some("/dev_hdd0/game/data/watermark.png"));
        assert_eq!(m.get_photo_title(), "Screenshot");
        m.clear_overlay();
        m.disable().unwrap();
        assert!(!m.is_enabled());
    }
}
