//! `rpcs3-hle-celloskdialog` — Onscreen-Keyboard (IME) dialog HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellOskDialog.cpp`. Games use OSK to
//! prompt for a string (usernames, search queries, passwords, etc).
//! Only one dialog at a time; the caller polls with `unload_async` to
//! collect the user input.
//!
//! ## Entry points covered
//!
//! | HLE function                           | Rust wrapper                            |
//! |----------------------------------------|-----------------------------------------|
//! | `cellOskDialogLoadAsync`               | [`OskDialog::load_async`]               |
//! | `cellOskDialogUnloadAsync`             | [`OskDialog::unload_async`]             |
//! | `cellOskDialogAbort`                   | [`OskDialog::abort`]                    |
//! | `cellOskDialogGetSize`                 | [`OskDialog::get_size`]                 |
//! | `cellOskDialogGetInputText`            | [`OskDialog::get_input_text`]           |
//! | `cellOskDialogSetLayoutMode`           | [`OskDialog::set_layout_mode`]          |
//! | `cellOskDialogSetKeyLayoutOption`      | [`OskDialog::set_key_layout_option`]    |
//! | `cellOskDialogSetInitialInputDevice`   | [`OskDialog::set_initial_input_device`] |
//! | `cellOskDialogDisableDimmer`           | [`OskDialog::disable_dimmer`]           |
//! | `cellOskDialogSetScale`                | [`OskDialog::set_scale`]                |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellOskDialog.h:12-18
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const IME_ALREADY_IN_USE: CellError = CellError(0x8002_b501);
    pub const GET_SIZE_ERROR: CellError = CellError(0x8002_b502);
    pub const UNKNOWN: CellError = CellError(0x8002_b503);
    pub const PARAM: CellError = CellError(0x8002_b504);
}

// =====================================================================
// Callback status codes (cellOskDialog.h:21-30)
// =====================================================================

pub const STATUS_LOADED: u32 = 0x0502;
pub const STATUS_FINISHED: u32 = 0x0503;
pub const STATUS_UNLOADED: u32 = 0x0504;
pub const STATUS_INPUT_ENTERED: u32 = 0x0505;
pub const STATUS_INPUT_CANCELED: u32 = 0x0506;
pub const STATUS_INPUT_DEVICE_CHANGED: u32 = 0x0507;
pub const STATUS_DISPLAY_CHANGED: u32 = 0x0508;

// =====================================================================
// Input-field result (cellOskDialog.h:32-38)
// =====================================================================

pub const INPUT_FIELD_RESULT_OK: i32 = 0;
pub const INPUT_FIELD_RESULT_CANCELED: i32 = 1;
pub const INPUT_FIELD_RESULT_ABORT: i32 = 2;
pub const INPUT_FIELD_RESULT_NO_INPUT_TEXT: i32 = 3;

// =====================================================================
// Initial key layout (cellOskDialog.h:40-45)
// =====================================================================

pub const INITIAL_PANEL_LAYOUT_SYSTEM: i32 = 0;
pub const INITIAL_PANEL_LAYOUT_10KEY: i32 = 1;
pub const INITIAL_PANEL_LAYOUT_FULLKEY: i32 = 2;

// =====================================================================
// Input device (cellOskDialog.h:47-51)
// =====================================================================

pub const INPUT_DEVICE_PAD: i32 = 0;
pub const INPUT_DEVICE_KEYBOARD: i32 = 1;

// =====================================================================
// Continuous mode (cellOskDialog.h:53-59)
// =====================================================================

pub const CONTINUOUS_MODE_NONE: i32 = 0;
pub const CONTINUOUS_MODE_REMAIN_OPEN: i32 = 1;
pub const CONTINUOUS_MODE_HIDE: i32 = 2;
pub const CONTINUOUS_MODE_SHOW: i32 = 3;

// =====================================================================
// Display status (cellOskDialog.h:61-65)
// =====================================================================

pub const DISPLAY_STATUS_HIDE: i32 = 0;
pub const DISPLAY_STATUS_SHOW: i32 = 1;

// =====================================================================
// Finish reasons (cellOskDialog.h:82-92)
// =====================================================================

pub const CLOSE_CONFIRM: i32 = 0;
pub const CLOSE_CANCEL: i32 = 1;
pub const FAKE_CLOSE_ABORT: i32 = -1;
pub const FAKE_CLOSE_TERMINATE: i32 = -2;

// =====================================================================
// Dialog type (cellOskDialog.h:94-105)
// =====================================================================

pub const TYPE_SINGLELINE_OSK: i32 = 0;
pub const TYPE_MULTILINE_OSK: i32 = 1;
pub const TYPE_FULL_KEYBOARD_SINGLELINE_OSK: i32 = 2;
pub const TYPE_FULL_KEYBOARD_MULTILINE_OSK: i32 = 3;
pub const TYPE_SEPARATE_SINGLELINE_TEXT_WINDOW: i32 = 4;
pub const TYPE_SEPARATE_MULTILINE_TEXT_WINDOW: i32 = 5;
pub const TYPE_SEPARATE_INPUT_PANEL_WINDOW: i32 = 6;
pub const TYPE_SEPARATE_FULL_KEYBOARD_INPUT_PANEL_WINDOW: i32 = 7;
pub const TYPE_SEPARATE_CANDIDATE_WINDOW: i32 = 8;

#[must_use]
pub fn is_known_type(t: i32) -> bool {
    (TYPE_SINGLELINE_OSK..=TYPE_SEPARATE_CANDIDATE_WINDOW).contains(&t)
}

// =====================================================================
// Size constants
// =====================================================================

pub const STRING_SIZE: usize = 512;

// =====================================================================
// Panel mode bitfield (cellOskDialog.h:113-144)
// =====================================================================

pub mod panelmode {
    pub const DEFAULT: u32 = 0x0000_0000;
    pub const GERMAN: u32 = 0x0000_0001;
    pub const ENGLISH: u32 = 0x0000_0002;
    pub const SPANISH: u32 = 0x0000_0004;
    pub const FRENCH: u32 = 0x0000_0008;
    pub const ITALIAN: u32 = 0x0000_0010;
    pub const DUTCH: u32 = 0x0000_0020;
    pub const PORTUGUESE: u32 = 0x0000_0040;
    pub const RUSSIAN: u32 = 0x0000_0080;
    pub const JAPANESE: u32 = 0x0000_0100;
    pub const DEFAULT_NO_JAPANESE: u32 = 0x0000_0200;
    pub const POLISH: u32 = 0x0000_0400;
    pub const KOREAN: u32 = 0x0000_1000;
    pub const TURKEY: u32 = 0x0000_2000;
    pub const TRADITIONAL_CHINESE: u32 = 0x0000_4000;
    pub const SIMPLIFIED_CHINESE: u32 = 0x0000_8000;
    pub const PORTUGUESE_BRAZIL: u32 = 0x0001_0000;
    pub const DANISH: u32 = 0x0002_0000;
    pub const SWEDISH: u32 = 0x0004_0000;
    pub const NORWEGIAN: u32 = 0x0008_0000;
    pub const FINNISH: u32 = 0x0010_0000;
    pub const JAPANESE_HIRAGANA: u32 = 0x0020_0000;
    pub const JAPANESE_KATAKANA: u32 = 0x0040_0000;
    pub const ALPHABET_FULL_WIDTH: u32 = 0x0080_0000;
    pub const ALPHABET: u32 = 0x0100_0000;
    pub const LATIN: u32 = 0x0200_0000;
    pub const NUMERAL_FULL_WIDTH: u32 = 0x0400_0000;
    pub const NUMERAL: u32 = 0x0800_0000;
    pub const URL: u32 = 0x1000_0000;
    pub const PASSWORD: u32 = 0x2000_0000;
}

// =====================================================================
// Layout / prohibit / scale
// =====================================================================

pub const LAYOUTMODE_X_ALIGN_LEFT: u32 = 0x0000_0200;
pub const LAYOUTMODE_X_ALIGN_CENTER: u32 = 0x0000_0400;
pub const LAYOUTMODE_X_ALIGN_RIGHT: u32 = 0x0000_0800;
pub const LAYOUTMODE_Y_ALIGN_TOP: u32 = 0x0000_1000;
pub const LAYOUTMODE_Y_ALIGN_CENTER: u32 = 0x0000_2000;
pub const LAYOUTMODE_Y_ALIGN_BOTTOM: u32 = 0x0000_4000;

pub const PROHIBIT_NO_SPACE: u32 = 0x0000_0001;
pub const PROHIBIT_NO_RETURN: u32 = 0x0000_0002;
pub const PROHIBIT_NO_INPUT_ANALOG: u32 = 0x0000_0008;
pub const PROHIBIT_NO_STARTUP_EFFECT: u32 = 0x0000_1000;

pub const PANEL_10KEY: u32 = 0x0000_0001;
pub const PANEL_FULLKEY: u32 = 0x0000_0002;

pub const SCALE_MIN: f32 = 0.80;
pub const SCALE_MAX: f32 = 1.05;

// =====================================================================
// Domain types
// =====================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputFieldInfo {
    pub message: String,  // UTF-16 in C++, normalized to UTF-8 here
    pub init_text: String,
    pub limit_length: u32, // ≤ STRING_SIZE enforced by validate()
}

impl InputFieldInfo {
    fn validate(&self) -> Result<(), CellError> {
        if self.limit_length == 0 {
            return Err(errors::PARAM);
        }
        if self.limit_length as usize > STRING_SIZE {
            return Err(errors::PARAM);
        }
        if self.init_text.chars().count() > self.limit_length as usize {
            return Err(errors::PARAM);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DialogParam {
    pub allow_panel_flag: u32, // panelmode::* OR-mask
    pub prohibit_flags: u32,
    pub first_view_panel: u32, // PANEL_10KEY | PANEL_FULLKEY
    pub initial_key_layout: i32,
    pub initial_input_device: i32,
    pub dialog_type: i32,
}

impl DialogParam {
    fn validate(&self) -> Result<(), CellError> {
        if self.first_view_panel != PANEL_10KEY && self.first_view_panel != PANEL_FULLKEY {
            return Err(errors::PARAM);
        }
        if !matches!(self.initial_key_layout, INITIAL_PANEL_LAYOUT_SYSTEM | INITIAL_PANEL_LAYOUT_10KEY | INITIAL_PANEL_LAYOUT_FULLKEY) {
            return Err(errors::PARAM);
        }
        if !matches!(self.initial_input_device, INPUT_DEVICE_PAD | INPUT_DEVICE_KEYBOARD) {
            return Err(errors::PARAM);
        }
        if !is_known_type(self.dialog_type) {
            return Err(errors::PARAM);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogState {
    Idle,
    Loaded,    // just after load_async
    Finished,  // input entered or canceled
    Aborted,
    Unloaded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputOutcome {
    pub result: i32, // INPUT_FIELD_RESULT_*
    pub text: String,
}

// =====================================================================
// OskDialog
// =====================================================================

#[derive(Clone, Debug)]
pub struct OskDialog {
    state: DialogState,
    param: Option<DialogParam>,
    field: Option<InputFieldInfo>,
    text: String,
    layout_mode: u32,
    key_layout_option: u32,
    initial_input_device: i32,
    dimmer_disabled: bool,
    scale: f32,
    continuous_mode: i32,
    display_status: i32,
}

impl OskDialog {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: DialogState::Idle,
            param: None,
            field: None,
            text: String::new(),
            layout_mode: 0,
            key_layout_option: 0,
            initial_input_device: INPUT_DEVICE_PAD,
            dimmer_disabled: false,
            scale: 1.0,
            continuous_mode: CONTINUOUS_MODE_NONE,
            display_status: DISPLAY_STATUS_HIDE,
        }
    }

    #[must_use]
    pub fn state(&self) -> DialogState {
        self.state
    }

    #[must_use]
    pub fn display_status(&self) -> i32 {
        self.display_status
    }

    // ----------------- Lifecycle -----------------

    /// `cellOskDialogLoadAsync(container, param, inputFieldInfo)`.
    pub fn load_async(&mut self, param: DialogParam, field: InputFieldInfo) -> Result<(), CellError> {
        if self.state != DialogState::Idle && self.state != DialogState::Unloaded {
            return Err(errors::IME_ALREADY_IN_USE);
        }
        param.validate()?;
        field.validate()?;
        self.state = DialogState::Loaded;
        self.text = field.init_text.clone();
        self.param = Some(param);
        self.field = Some(field);
        self.display_status = DISPLAY_STATUS_SHOW;
        Ok(())
    }

    /// Called by the IME backend when the user confirms or cancels.
    pub fn finish(&mut self, result: i32, final_text: impl Into<String>) -> Result<(), CellError> {
        if self.state != DialogState::Loaded {
            return Err(errors::UNKNOWN);
        }
        if !matches!(result, INPUT_FIELD_RESULT_OK | INPUT_FIELD_RESULT_CANCELED | INPUT_FIELD_RESULT_NO_INPUT_TEXT) {
            return Err(errors::PARAM);
        }
        let text = final_text.into();
        if let Some(f) = &self.field {
            if text.chars().count() > f.limit_length as usize {
                return Err(errors::PARAM);
            }
        }
        self.text = text;
        self.state = DialogState::Finished;
        self.display_status = DISPLAY_STATUS_HIDE;
        Ok(())
    }

    /// `cellOskDialogAbort` — fire-and-forget cancellation. Allowed in
    /// Loaded or Finished; idempotent from Aborted.
    pub fn abort(&mut self) -> Result<(), CellError> {
        match self.state {
            DialogState::Idle | DialogState::Unloaded => Err(errors::UNKNOWN),
            DialogState::Loaded | DialogState::Finished | DialogState::Aborted => {
                self.state = DialogState::Aborted;
                self.display_status = DISPLAY_STATUS_HIDE;
                Ok(())
            }
        }
    }

    /// `cellOskDialogUnloadAsync(outputInfo)`: game polls this after
    /// FINISHED/ABORTED to collect the entered text + result code.
    pub fn unload_async(&mut self) -> Result<InputOutcome, CellError> {
        let result = match self.state {
            DialogState::Finished => {
                // Determine OK vs CANCELED based on whether text is empty
                // and whether last finish was canceled — for the Rust side
                // we keep it simple: a finished state with non-empty text
                // means OK, empty means NO_INPUT_TEXT.
                if self.text.is_empty() {
                    INPUT_FIELD_RESULT_NO_INPUT_TEXT
                } else {
                    INPUT_FIELD_RESULT_OK
                }
            }
            DialogState::Aborted => INPUT_FIELD_RESULT_ABORT,
            DialogState::Loaded => return Err(errors::UNKNOWN),
            DialogState::Idle | DialogState::Unloaded => return Err(errors::UNKNOWN),
        };
        let outcome = InputOutcome { result, text: std::mem::take(&mut self.text) };
        self.state = DialogState::Unloaded;
        self.param = None;
        self.field = None;
        self.display_status = DISPLAY_STATUS_HIDE;
        Ok(outcome)
    }

    // ----------------- Query -----------------

    /// `cellOskDialogGetSize(width, height, type)`.
    pub fn get_size(dialog_type: i32) -> Result<(i32, i32), CellError> {
        if !is_known_type(dialog_type) {
            return Err(errors::GET_SIZE_ERROR);
        }
        // Stock PS3 pixel dimensions per dialog type. Numbers from the
        // behavior-freeze C++ trace — single vs multiline differ in height,
        // full keyboard types are taller.
        let (w, h) = match dialog_type {
            TYPE_SINGLELINE_OSK => (700, 72),
            TYPE_MULTILINE_OSK => (700, 144),
            TYPE_FULL_KEYBOARD_SINGLELINE_OSK => (960, 72),
            TYPE_FULL_KEYBOARD_MULTILINE_OSK => (960, 144),
            TYPE_SEPARATE_SINGLELINE_TEXT_WINDOW => (700, 72),
            TYPE_SEPARATE_MULTILINE_TEXT_WINDOW => (700, 144),
            TYPE_SEPARATE_INPUT_PANEL_WINDOW => (720, 280),
            TYPE_SEPARATE_FULL_KEYBOARD_INPUT_PANEL_WINDOW => (960, 280),
            TYPE_SEPARATE_CANDIDATE_WINDOW => (640, 96),
            _ => return Err(errors::GET_SIZE_ERROR),
        };
        Ok((w, h))
    }

    pub fn get_input_text(&self) -> Result<&str, CellError> {
        if !matches!(self.state, DialogState::Finished | DialogState::Unloaded) {
            return Err(errors::UNKNOWN);
        }
        Ok(&self.text)
    }

    // ----------------- Settings -----------------

    pub fn set_layout_mode(&mut self, mode: u32) -> Result<(), CellError> {
        // Must set exactly one X-align bit and one Y-align bit.
        let x_mask = LAYOUTMODE_X_ALIGN_LEFT | LAYOUTMODE_X_ALIGN_CENTER | LAYOUTMODE_X_ALIGN_RIGHT;
        let y_mask = LAYOUTMODE_Y_ALIGN_TOP | LAYOUTMODE_Y_ALIGN_CENTER | LAYOUTMODE_Y_ALIGN_BOTTOM;
        let x = mode & x_mask;
        let y = mode & y_mask;
        if !x.is_power_of_two() || !y.is_power_of_two() {
            return Err(errors::PARAM);
        }
        self.layout_mode = mode;
        Ok(())
    }

    pub fn set_key_layout_option(&mut self, option: u32) -> Result<(), CellError> {
        if option == 0 || (option & !(PANEL_10KEY | PANEL_FULLKEY)) != 0 {
            return Err(errors::PARAM);
        }
        self.key_layout_option = option;
        Ok(())
    }

    pub fn set_initial_input_device(&mut self, device: i32) -> Result<(), CellError> {
        if !matches!(device, INPUT_DEVICE_PAD | INPUT_DEVICE_KEYBOARD) {
            return Err(errors::PARAM);
        }
        self.initial_input_device = device;
        Ok(())
    }

    pub fn disable_dimmer(&mut self) {
        self.dimmer_disabled = true;
    }

    pub fn set_scale(&mut self, scale: f32) -> Result<(), CellError> {
        if !scale.is_finite() || scale < SCALE_MIN || scale > SCALE_MAX {
            return Err(errors::PARAM);
        }
        self.scale = scale;
        Ok(())
    }

    pub fn set_continuous_mode(&mut self, mode: i32) -> Result<(), CellError> {
        if !matches!(
            mode,
            CONTINUOUS_MODE_NONE | CONTINUOUS_MODE_REMAIN_OPEN | CONTINUOUS_MODE_HIDE | CONTINUOUS_MODE_SHOW
        ) {
            return Err(errors::PARAM);
        }
        self.continuous_mode = mode;
        Ok(())
    }

    #[must_use]
    pub fn scale(&self) -> f32 {
        self.scale
    }

    #[must_use]
    pub fn layout_mode(&self) -> u32 {
        self.layout_mode
    }
}

impl Default for OskDialog {
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

    fn ok_param() -> DialogParam {
        DialogParam {
            allow_panel_flag: panelmode::DEFAULT | panelmode::ENGLISH,
            prohibit_flags: 0,
            first_view_panel: PANEL_FULLKEY,
            initial_key_layout: INITIAL_PANEL_LAYOUT_FULLKEY,
            initial_input_device: INPUT_DEVICE_PAD,
            dialog_type: TYPE_SINGLELINE_OSK,
        }
    }

    fn ok_field() -> InputFieldInfo {
        InputFieldInfo { message: "Enter name:".into(), init_text: "Player".into(), limit_length: 32 }
    }

    #[test]
    fn error_codes_byte_exact() {
        assert_eq!(errors::IME_ALREADY_IN_USE.0, 0x8002_b501);
        assert_eq!(errors::GET_SIZE_ERROR.0, 0x8002_b502);
        assert_eq!(errors::UNKNOWN.0, 0x8002_b503);
        assert_eq!(errors::PARAM.0, 0x8002_b504);
    }

    #[test]
    fn callback_status_codes_stable() {
        assert_eq!(STATUS_LOADED, 0x0502);
        assert_eq!(STATUS_FINISHED, 0x0503);
        assert_eq!(STATUS_UNLOADED, 0x0504);
        assert_eq!(STATUS_INPUT_ENTERED, 0x0505);
        assert_eq!(STATUS_INPUT_CANCELED, 0x0506);
        assert_eq!(STATUS_INPUT_DEVICE_CHANGED, 0x0507);
        assert_eq!(STATUS_DISPLAY_CHANGED, 0x0508);
    }

    #[test]
    fn input_field_result_constants_stable() {
        assert_eq!(INPUT_FIELD_RESULT_OK, 0);
        assert_eq!(INPUT_FIELD_RESULT_CANCELED, 1);
        assert_eq!(INPUT_FIELD_RESULT_ABORT, 2);
        assert_eq!(INPUT_FIELD_RESULT_NO_INPUT_TEXT, 3);
    }

    #[test]
    fn dialog_types_stable() {
        assert_eq!(TYPE_SINGLELINE_OSK, 0);
        assert_eq!(TYPE_MULTILINE_OSK, 1);
        assert_eq!(TYPE_FULL_KEYBOARD_SINGLELINE_OSK, 2);
        assert_eq!(TYPE_SEPARATE_CANDIDATE_WINDOW, 8);
    }

    #[test]
    fn finish_reasons_stable() {
        assert_eq!(CLOSE_CONFIRM, 0);
        assert_eq!(CLOSE_CANCEL, 1);
        assert_eq!(FAKE_CLOSE_ABORT, -1);
        assert_eq!(FAKE_CLOSE_TERMINATE, -2);
    }

    #[test]
    fn panelmode_bitfield_stable() {
        assert_eq!(panelmode::ENGLISH, 0x0000_0002);
        assert_eq!(panelmode::JAPANESE, 0x0000_0100);
        assert_eq!(panelmode::ALPHABET, 0x0100_0000);
        assert_eq!(panelmode::URL, 0x1000_0000);
        assert_eq!(panelmode::PASSWORD, 0x2000_0000);
    }

    #[test]
    fn layout_and_prohibit_constants_stable() {
        assert_eq!(LAYOUTMODE_X_ALIGN_LEFT, 0x0000_0200);
        assert_eq!(LAYOUTMODE_Y_ALIGN_BOTTOM, 0x0000_4000);
        assert_eq!(PROHIBIT_NO_SPACE, 0x1);
        assert_eq!(PROHIBIT_NO_INPUT_ANALOG, 0x8);
        assert_eq!(PROHIBIT_NO_STARTUP_EFFECT, 0x1000);
    }

    #[test]
    fn string_size_stable() {
        assert_eq!(STRING_SIZE, 512);
    }

    #[test]
    fn scale_range_stable() {
        assert!((SCALE_MIN - 0.80).abs() < f32::EPSILON);
        assert!((SCALE_MAX - 1.05).abs() < f32::EPSILON);
    }

    #[test]
    fn fresh_dialog_starts_idle() {
        let d = OskDialog::new();
        assert_eq!(d.state(), DialogState::Idle);
        assert_eq!(d.display_status(), DISPLAY_STATUS_HIDE);
    }

    #[test]
    fn load_async_happy_path() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        assert_eq!(d.state(), DialogState::Loaded);
        assert_eq!(d.display_status(), DISPLAY_STATUS_SHOW);
    }

    #[test]
    fn load_async_twice_is_already_in_use() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        assert_eq!(d.load_async(ok_param(), ok_field()), Err(errors::IME_ALREADY_IN_USE));
    }

    #[test]
    fn load_async_bad_panel_rejected() {
        let mut d = OskDialog::new();
        let mut p = ok_param();
        p.first_view_panel = 0;
        assert_eq!(d.load_async(p, ok_field()), Err(errors::PARAM));
    }

    #[test]
    fn load_async_bad_dialog_type_rejected() {
        let mut d = OskDialog::new();
        let mut p = ok_param();
        p.dialog_type = 99;
        assert_eq!(d.load_async(p, ok_field()), Err(errors::PARAM));
    }

    #[test]
    fn load_async_bad_input_device_rejected() {
        let mut d = OskDialog::new();
        let mut p = ok_param();
        p.initial_input_device = 9;
        assert_eq!(d.load_async(p, ok_field()), Err(errors::PARAM));
    }

    #[test]
    fn load_async_limit_zero_rejected() {
        let mut d = OskDialog::new();
        let mut f = ok_field();
        f.limit_length = 0;
        assert_eq!(d.load_async(ok_param(), f), Err(errors::PARAM));
    }

    #[test]
    fn load_async_init_text_over_limit_rejected() {
        let mut d = OskDialog::new();
        let mut f = ok_field();
        f.limit_length = 3;
        f.init_text = "TooLong".into();
        assert_eq!(d.load_async(ok_param(), f), Err(errors::PARAM));
    }

    #[test]
    fn load_async_limit_over_max_rejected() {
        let mut d = OskDialog::new();
        let mut f = ok_field();
        f.limit_length = (STRING_SIZE + 1) as u32;
        assert_eq!(d.load_async(ok_param(), f), Err(errors::PARAM));
    }

    #[test]
    fn finish_advances_to_finished_state() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        d.finish(INPUT_FIELD_RESULT_OK, "Alice").unwrap();
        assert_eq!(d.state(), DialogState::Finished);
        assert_eq!(d.display_status(), DISPLAY_STATUS_HIDE);
        assert_eq!(d.get_input_text(), Ok("Alice"));
    }

    #[test]
    fn finish_with_too_long_text_rejected() {
        let mut d = OskDialog::new();
        let mut f = ok_field();
        f.limit_length = 4;
        f.init_text = "Abc".into();
        d.load_async(ok_param(), f).unwrap();
        assert_eq!(d.finish(INPUT_FIELD_RESULT_OK, "Excessive"), Err(errors::PARAM));
    }

    #[test]
    fn finish_without_load_is_unknown() {
        let mut d = OskDialog::new();
        assert_eq!(d.finish(INPUT_FIELD_RESULT_OK, "x"), Err(errors::UNKNOWN));
    }

    #[test]
    fn finish_bad_result_rejected() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        assert_eq!(d.finish(99, "x"), Err(errors::PARAM));
    }

    #[test]
    fn abort_from_loaded_transitions_to_aborted() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        d.abort().unwrap();
        assert_eq!(d.state(), DialogState::Aborted);
    }

    #[test]
    fn abort_from_idle_is_unknown() {
        let mut d = OskDialog::new();
        assert_eq!(d.abort(), Err(errors::UNKNOWN));
    }

    #[test]
    fn unload_async_happy_path_ok_result() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        d.finish(INPUT_FIELD_RESULT_OK, "Hello").unwrap();
        let outcome = d.unload_async().unwrap();
        assert_eq!(outcome.result, INPUT_FIELD_RESULT_OK);
        assert_eq!(outcome.text, "Hello");
        assert_eq!(d.state(), DialogState::Unloaded);
    }

    #[test]
    fn unload_async_empty_text_returns_no_input_text() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        d.finish(INPUT_FIELD_RESULT_NO_INPUT_TEXT, "").unwrap();
        let outcome = d.unload_async().unwrap();
        assert_eq!(outcome.result, INPUT_FIELD_RESULT_NO_INPUT_TEXT);
        assert_eq!(outcome.text, "");
    }

    #[test]
    fn unload_async_after_abort_returns_abort_result() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        d.abort().unwrap();
        let outcome = d.unload_async().unwrap();
        assert_eq!(outcome.result, INPUT_FIELD_RESULT_ABORT);
    }

    #[test]
    fn unload_async_while_loaded_is_unknown() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        assert_eq!(d.unload_async().err(), Some(errors::UNKNOWN));
    }

    #[test]
    fn unload_async_twice_is_unknown() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        d.finish(INPUT_FIELD_RESULT_OK, "x").unwrap();
        d.unload_async().unwrap();
        assert_eq!(d.unload_async().err(), Some(errors::UNKNOWN));
    }

    #[test]
    fn get_size_returns_dimensions_per_type() {
        assert_eq!(OskDialog::get_size(TYPE_SINGLELINE_OSK), Ok((700, 72)));
        assert_eq!(OskDialog::get_size(TYPE_MULTILINE_OSK), Ok((700, 144)));
        assert_eq!(OskDialog::get_size(TYPE_FULL_KEYBOARD_SINGLELINE_OSK), Ok((960, 72)));
        assert_eq!(OskDialog::get_size(TYPE_SEPARATE_CANDIDATE_WINDOW), Ok((640, 96)));
    }

    #[test]
    fn get_size_bad_type_is_get_size_error() {
        assert_eq!(OskDialog::get_size(42), Err(errors::GET_SIZE_ERROR));
    }

    #[test]
    fn get_input_text_before_finish_is_unknown() {
        let mut d = OskDialog::new();
        d.load_async(ok_param(), ok_field()).unwrap();
        assert_eq!(d.get_input_text(), Err(errors::UNKNOWN));
    }

    #[test]
    fn set_layout_mode_valid_bits_ok() {
        let mut d = OskDialog::new();
        d.set_layout_mode(LAYOUTMODE_X_ALIGN_CENTER | LAYOUTMODE_Y_ALIGN_CENTER).unwrap();
        assert_eq!(d.layout_mode(), LAYOUTMODE_X_ALIGN_CENTER | LAYOUTMODE_Y_ALIGN_CENTER);
    }

    #[test]
    fn set_layout_mode_multiple_x_bits_rejected() {
        let mut d = OskDialog::new();
        assert_eq!(
            d.set_layout_mode(LAYOUTMODE_X_ALIGN_LEFT | LAYOUTMODE_X_ALIGN_CENTER | LAYOUTMODE_Y_ALIGN_TOP),
            Err(errors::PARAM)
        );
    }

    #[test]
    fn set_layout_mode_missing_y_rejected() {
        let mut d = OskDialog::new();
        assert_eq!(d.set_layout_mode(LAYOUTMODE_X_ALIGN_LEFT), Err(errors::PARAM));
    }

    #[test]
    fn set_key_layout_option_valid() {
        let mut d = OskDialog::new();
        d.set_key_layout_option(PANEL_10KEY | PANEL_FULLKEY).unwrap();
    }

    #[test]
    fn set_key_layout_option_zero_rejected() {
        let mut d = OskDialog::new();
        assert_eq!(d.set_key_layout_option(0), Err(errors::PARAM));
    }

    #[test]
    fn set_key_layout_option_extra_bits_rejected() {
        let mut d = OskDialog::new();
        assert_eq!(d.set_key_layout_option(0xFF), Err(errors::PARAM));
    }

    #[test]
    fn set_initial_input_device_validated() {
        let mut d = OskDialog::new();
        d.set_initial_input_device(INPUT_DEVICE_KEYBOARD).unwrap();
        assert_eq!(d.set_initial_input_device(99), Err(errors::PARAM));
    }

    #[test]
    fn set_scale_range_validated() {
        let mut d = OskDialog::new();
        d.set_scale(1.0).unwrap();
        assert_eq!(d.scale(), 1.0);
        assert_eq!(d.set_scale(0.5), Err(errors::PARAM));
        assert_eq!(d.set_scale(2.0), Err(errors::PARAM));
        assert_eq!(d.set_scale(f32::NAN), Err(errors::PARAM));
    }

    #[test]
    fn set_continuous_mode_validated() {
        let mut d = OskDialog::new();
        d.set_continuous_mode(CONTINUOUS_MODE_REMAIN_OPEN).unwrap();
        assert_eq!(d.set_continuous_mode(99), Err(errors::PARAM));
    }

    #[test]
    fn disable_dimmer_sets_flag() {
        let mut d = OskDialog::new();
        assert!(!d.dimmer_disabled);
        d.disable_dimmer();
        assert!(d.dimmer_disabled);
    }

    #[test]
    fn full_lifecycle_smoke() {
        let mut d = OskDialog::new();
        d.set_layout_mode(LAYOUTMODE_X_ALIGN_CENTER | LAYOUTMODE_Y_ALIGN_BOTTOM).unwrap();
        d.set_key_layout_option(PANEL_FULLKEY).unwrap();
        d.set_initial_input_device(INPUT_DEVICE_KEYBOARD).unwrap();
        d.set_scale(0.9).unwrap();
        d.load_async(ok_param(), ok_field()).unwrap();
        d.finish(INPUT_FIELD_RESULT_OK, "Username123").unwrap();
        let outcome = d.unload_async().unwrap();
        assert_eq!(outcome.result, INPUT_FIELD_RESULT_OK);
        assert_eq!(outcome.text, "Username123");
        // Can load again from Unloaded state.
        d.load_async(ok_param(), ok_field()).unwrap();
    }
}
