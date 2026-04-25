//! `rpcs3-hle-cellmsgdialog` — modal message/progress dialog HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellMsgDialog.cpp`. Every PS3 game
//! uses this for error popups, yes/no confirmations, and progress
//! bars during long operations (save, load, network). The state FSM
//! is tiny but the flag encoding in the `type` word is tricky —
//! icon/mute/bg/button/cursor/progressbar all packed into one u32.
//!
//! ## Entry points
//!
//! | HLE function                        | Rust wrapper                        |
//! |-------------------------------------|-------------------------------------|
//! | `cellMsgDialogOpen2`                | [`cell_msg_dialog_open`]            |
//! | `cellMsgDialogOpenErrorCode`        | [`cell_msg_dialog_open_error_code`] |
//! | `cellMsgDialogClose`                | [`cell_msg_dialog_close`]           |
//! | `cellMsgDialogAbort`                | [`cell_msg_dialog_abort`]           |
//! | `cellMsgDialogProgressBarSetMsg`    | [`cell_msg_dialog_progress_bar_set_msg`] |
//! | `cellMsgDialogProgressBarReset`     | [`cell_msg_dialog_progress_bar_reset`] |
//! | `cellMsgDialogProgressBarInc`       | [`cell_msg_dialog_progress_bar_inc`] |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact cellMsgDialog.h:20-21
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const PARAM: CellError = CellError(0x8002_B301);
    pub const DIALOG_NOT_OPENED: CellError = CellError(0x8002_B302);
}

// =====================================================================
// Type-word flag encoding
// =====================================================================

pub const SE_TYPE_MASK: u32 = 1 << 0;
pub const SE_TYPE_ERROR: u32 = 0;
pub const SE_TYPE_NORMAL: u32 = 1;

pub const SE_MUTE_MASK: u32 = 1 << 1;
pub const SE_MUTE_OFF: u32 = 0;
pub const SE_MUTE_ON: u32 = 1 << 1;

pub const BG_MASK: u32 = 1 << 2;
pub const BG_VISIBLE: u32 = 0;
pub const BG_INVISIBLE: u32 = 1 << 2;

pub const BUTTON_TYPE_MASK: u32 = 0b111 << 4;
pub const BUTTON_TYPE_NONE: u32 = 0 << 4;
pub const BUTTON_TYPE_YESNO: u32 = 1 << 4;
pub const BUTTON_TYPE_OK: u32 = 2 << 4;

pub const DISABLE_CANCEL_MASK: u32 = 1 << 7;
pub const DISABLE_CANCEL_OFF: u32 = 0;
pub const DISABLE_CANCEL_ON: u32 = 1 << 7;

pub const DEFAULT_CURSOR_MASK: u32 = 1 << 8;
pub const DEFAULT_CURSOR_YES: u32 = 0;
pub const DEFAULT_CURSOR_NO: u32 = 1 << 8;

pub const PROGRESSBAR_MASK: u32 = 0b11 << 12;
pub const PROGRESSBAR_NONE: u32 = 0 << 12;
pub const PROGRESSBAR_SINGLE: u32 = 1 << 12;
pub const PROGRESSBAR_DOUBLE: u32 = 2 << 12;

/// Max characters the progress-bar subtitle accepts.
pub const PROGRESSBAR_STRING_SIZE: usize = 64;

// =====================================================================
// Button result codes
// =====================================================================

pub const BUTTON_NONE: i32 = -1;
pub const BUTTON_INVALID: i32 = 0;
pub const BUTTON_OK: i32 = 1;
pub const BUTTON_YES: i32 = 1;
pub const BUTTON_NO: i32 = 2;
pub const BUTTON_ESCAPE: i32 = 3;

pub const PROGRESSBAR_INDEX_SINGLE: u32 = 0;
pub const PROGRESSBAR_INDEX_DOUBLE_UPPER: u32 = 0;
pub const PROGRESSBAR_INDEX_DOUBLE_LOWER: u32 = 1;

// =====================================================================
// State machine
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogState {
    Closed,
    Open,
    WaitingUserInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeFlags(pub u32);

impl TypeFlags {
    #[must_use]
    pub fn button_type(self) -> u32 { self.0 & BUTTON_TYPE_MASK }
    #[must_use]
    pub fn progressbar(self) -> u32 { self.0 & PROGRESSBAR_MASK }
    #[must_use]
    pub fn is_error(self) -> bool { (self.0 & SE_TYPE_MASK) == SE_TYPE_ERROR }
    #[must_use]
    pub fn is_cancel_disabled(self) -> bool { (self.0 & DISABLE_CANCEL_MASK) != 0 }

    /// Returns `Err(PARAM)` if the button-type and progressbar fields
    /// disagree (PROGRESSBAR_* requires BUTTON_TYPE_NONE per firmware).
    pub fn validate(self) -> Result<(), CellError> {
        let prog = self.progressbar();
        let button = self.button_type();
        if prog != PROGRESSBAR_NONE && button != BUTTON_TYPE_NONE {
            return Err(errors::PARAM);
        }
        // Reserved bits (outside documented masks) must be zero.
        let allowed = SE_TYPE_MASK
            | SE_MUTE_MASK
            | BG_MASK
            | BUTTON_TYPE_MASK
            | DISABLE_CANCEL_MASK
            | DEFAULT_CURSOR_MASK
            | PROGRESSBAR_MASK;
        if self.0 & !allowed != 0 {
            return Err(errors::PARAM);
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct DialogManager {
    state: DialogState,
    flags: TypeFlags,
    message: String,
    upper_bar: u8,
    lower_bar: u8,
    upper_msg: String,
    lower_msg: String,
    last_button: i32,
}

impl Default for DialogManager {
    fn default() -> Self {
        Self {
            state: DialogState::Closed,
            flags: TypeFlags(0),
            message: String::new(),
            upper_bar: 0,
            lower_bar: 0,
            upper_msg: String::new(),
            lower_msg: String::new(),
            last_button: BUTTON_NONE,
        }
    }
}

impl DialogManager {
    #[must_use] pub fn state(&self) -> DialogState { self.state }
    #[must_use] pub fn progress_upper(&self) -> u8 { self.upper_bar }
    #[must_use] pub fn progress_lower(&self) -> u8 { self.lower_bar }
}

// =====================================================================
// Syscalls
// =====================================================================

/// `cellMsgDialogOpen2(type, msg, callback, userdata, extParam)`.
#[must_use]
pub fn cell_msg_dialog_open(
    m: &mut DialogManager,
    flags: TypeFlags,
    message: &str,
) -> Result<(), CellError> {
    flags.validate()?;
    if m.state != DialogState::Closed {
        return Err(errors::PARAM);
    }
    if message.len() > 256 {
        return Err(errors::PARAM);
    }
    m.flags = flags;
    m.message = message.to_owned();
    m.upper_bar = 0;
    m.lower_bar = 0;
    m.upper_msg.clear();
    m.lower_msg.clear();
    m.state = DialogState::Open;
    Ok(())
}

/// `cellMsgDialogOpenErrorCode(errorCode, callback, userdata, extParam)`.
#[must_use]
pub fn cell_msg_dialog_open_error_code(
    m: &mut DialogManager,
    error_code: u32,
) -> Result<(), CellError> {
    if m.state != DialogState::Closed {
        return Err(errors::PARAM);
    }
    m.flags = TypeFlags(SE_TYPE_ERROR | BUTTON_TYPE_OK);
    m.message = format!("Error 0x{error_code:08X}");
    m.state = DialogState::Open;
    Ok(())
}

/// `cellMsgDialogClose(delay)` — dismiss dialog with OK result.
#[must_use]
pub fn cell_msg_dialog_close(
    m: &mut DialogManager,
    _delay_us: u32,
) -> Result<(), CellError> {
    if m.state == DialogState::Closed {
        return Err(errors::DIALOG_NOT_OPENED);
    }
    m.state = DialogState::Closed;
    m.last_button = match m.flags.button_type() {
        BUTTON_TYPE_YESNO => BUTTON_YES,
        BUTTON_TYPE_OK => BUTTON_OK,
        _ => BUTTON_NONE,
    };
    Ok(())
}

/// `cellMsgDialogAbort()` — dismiss as ESCAPE.
#[must_use]
pub fn cell_msg_dialog_abort(m: &mut DialogManager) -> Result<(), CellError> {
    if m.state == DialogState::Closed {
        return Err(errors::DIALOG_NOT_OPENED);
    }
    if m.flags.is_cancel_disabled() {
        return Err(errors::PARAM);
    }
    m.state = DialogState::Closed;
    m.last_button = BUTTON_ESCAPE;
    Ok(())
}

/// Test helper: read the last button result.
#[must_use]
pub fn last_button(m: &DialogManager) -> i32 { m.last_button }

/// `cellMsgDialogProgressBarSetMsg(progressBarIndex, msgString)`.
#[must_use]
pub fn cell_msg_dialog_progress_bar_set_msg(
    m: &mut DialogManager,
    bar_index: u32,
    msg: &str,
) -> Result<(), CellError> {
    if m.state == DialogState::Closed {
        return Err(errors::DIALOG_NOT_OPENED);
    }
    if msg.len() > PROGRESSBAR_STRING_SIZE {
        return Err(errors::PARAM);
    }
    match (m.flags.progressbar(), bar_index) {
        (PROGRESSBAR_SINGLE, 0) | (PROGRESSBAR_DOUBLE, 0) => m.upper_msg = msg.to_owned(),
        (PROGRESSBAR_DOUBLE, 1) => m.lower_msg = msg.to_owned(),
        _ => return Err(errors::PARAM),
    }
    Ok(())
}

/// `cellMsgDialogProgressBarReset(progressBarIndex)` — zeros the bar.
#[must_use]
pub fn cell_msg_dialog_progress_bar_reset(
    m: &mut DialogManager,
    bar_index: u32,
) -> Result<(), CellError> {
    if m.state == DialogState::Closed {
        return Err(errors::DIALOG_NOT_OPENED);
    }
    match (m.flags.progressbar(), bar_index) {
        (PROGRESSBAR_SINGLE, 0) | (PROGRESSBAR_DOUBLE, 0) => m.upper_bar = 0,
        (PROGRESSBAR_DOUBLE, 1) => m.lower_bar = 0,
        _ => return Err(errors::PARAM),
    }
    Ok(())
}

/// `cellMsgDialogProgressBarInc(progressBarIndex, delta)`. Saturates at 100.
#[must_use]
pub fn cell_msg_dialog_progress_bar_inc(
    m: &mut DialogManager,
    bar_index: u32,
    delta: u32,
) -> Result<(), CellError> {
    if m.state == DialogState::Closed {
        return Err(errors::DIALOG_NOT_OPENED);
    }
    let bar = match (m.flags.progressbar(), bar_index) {
        (PROGRESSBAR_SINGLE, 0) | (PROGRESSBAR_DOUBLE, 0) => &mut m.upper_bar,
        (PROGRESSBAR_DOUBLE, 1) => &mut m.lower_bar,
        _ => return Err(errors::PARAM),
    };
    *bar = (*bar as u32).saturating_add(delta).min(100) as u8;
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_match_cpp() {
        assert_eq!(errors::PARAM.0, 0x8002_B301);
        assert_eq!(errors::DIALOG_NOT_OPENED.0, 0x8002_B302);
    }

    #[test]
    fn type_masks_encode_correctly() {
        assert_eq!(BUTTON_TYPE_NONE, 0);
        assert_eq!(BUTTON_TYPE_YESNO, 0x10);
        assert_eq!(BUTTON_TYPE_OK, 0x20);
        assert_eq!(PROGRESSBAR_SINGLE, 0x1000);
        assert_eq!(PROGRESSBAR_DOUBLE, 0x2000);
        assert_eq!(SE_MUTE_ON, 0x02);
    }

    #[test]
    fn button_result_codes_match_cpp() {
        assert_eq!(BUTTON_NONE, -1);
        assert_eq!(BUTTON_OK, 1);
        assert_eq!(BUTTON_YES, 1);
        assert_eq!(BUTTON_NO, 2);
        assert_eq!(BUTTON_ESCAPE, 3);
    }

    // --- flag validation ------------------------------------------

    #[test]
    fn flags_combining_progressbar_with_buttons_is_param() {
        let f = TypeFlags(PROGRESSBAR_SINGLE | BUTTON_TYPE_YESNO);
        assert_eq!(f.validate().unwrap_err(), errors::PARAM);
    }

    #[test]
    fn flags_with_reserved_bits_set_is_param() {
        let f = TypeFlags(0x1 << 20);
        assert_eq!(f.validate().unwrap_err(), errors::PARAM);
    }

    #[test]
    fn flags_valid_combinations_pass() {
        TypeFlags(BUTTON_TYPE_OK).validate().unwrap();
        TypeFlags(PROGRESSBAR_DOUBLE | BG_INVISIBLE | SE_MUTE_ON).validate().unwrap();
        TypeFlags(BUTTON_TYPE_YESNO | DEFAULT_CURSOR_NO | DISABLE_CANCEL_ON).validate().unwrap();
    }

    // --- open / close ---------------------------------------------

    #[test]
    fn open_then_close_yes_no_returns_yes() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(BUTTON_TYPE_YESNO), "ok?").unwrap();
        assert_eq!(m.state(), DialogState::Open);
        cell_msg_dialog_close(&mut m, 0).unwrap();
        assert_eq!(last_button(&m), BUTTON_YES);
    }

    #[test]
    fn open_with_ok_button_close_returns_ok() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(BUTTON_TYPE_OK), "done").unwrap();
        cell_msg_dialog_close(&mut m, 0).unwrap();
        assert_eq!(last_button(&m), BUTTON_OK);
    }

    #[test]
    fn open_twice_without_close_is_param() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(BUTTON_TYPE_OK), "a").unwrap();
        assert_eq!(
            cell_msg_dialog_open(&mut m, TypeFlags(BUTTON_TYPE_OK), "b").unwrap_err(),
            errors::PARAM,
        );
    }

    #[test]
    fn open_with_message_over_256_chars_is_param() {
        let mut m = DialogManager::default();
        let long = "x".repeat(257);
        assert_eq!(
            cell_msg_dialog_open(&mut m, TypeFlags(BUTTON_TYPE_OK), &long).unwrap_err(),
            errors::PARAM,
        );
    }

    #[test]
    fn open_with_invalid_flags_is_param() {
        let mut m = DialogManager::default();
        let bad = TypeFlags(PROGRESSBAR_SINGLE | BUTTON_TYPE_YESNO);
        assert_eq!(
            cell_msg_dialog_open(&mut m, bad, "").unwrap_err(),
            errors::PARAM,
        );
    }

    #[test]
    fn close_when_not_open_is_dialog_not_opened() {
        let mut m = DialogManager::default();
        assert_eq!(
            cell_msg_dialog_close(&mut m, 0).unwrap_err(),
            errors::DIALOG_NOT_OPENED,
        );
    }

    // --- abort ----------------------------------------------------

    #[test]
    fn abort_yields_escape_button() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(BUTTON_TYPE_YESNO), "sure?").unwrap();
        cell_msg_dialog_abort(&mut m).unwrap();
        assert_eq!(last_button(&m), BUTTON_ESCAPE);
    }

    #[test]
    fn abort_disabled_by_flag_returns_param() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(
            &mut m,
            TypeFlags(BUTTON_TYPE_YESNO | DISABLE_CANCEL_ON),
            "no cancel",
        )
        .unwrap();
        assert_eq!(
            cell_msg_dialog_abort(&mut m).unwrap_err(),
            errors::PARAM,
        );
    }

    // --- error-code dialog ---------------------------------------

    #[test]
    fn open_error_code_formats_message() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open_error_code(&mut m, 0x8002_B404).unwrap();
        assert_eq!(m.state(), DialogState::Open);
        assert!(m.message.contains("8002B404"));
    }

    // --- progress bars -------------------------------------------

    #[test]
    fn progress_bar_inc_single_saturates_at_100() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(PROGRESSBAR_SINGLE), "").unwrap();
        cell_msg_dialog_progress_bar_inc(&mut m, 0, 70).unwrap();
        cell_msg_dialog_progress_bar_inc(&mut m, 0, 70).unwrap();
        assert_eq!(m.progress_upper(), 100);
    }

    #[test]
    fn progress_bar_inc_wrong_index_is_param() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(PROGRESSBAR_SINGLE), "").unwrap();
        assert_eq!(
            cell_msg_dialog_progress_bar_inc(&mut m, 1, 10).unwrap_err(),
            errors::PARAM,
        );
    }

    #[test]
    fn progress_bar_inc_without_progress_flag_is_param() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(BUTTON_TYPE_OK), "").unwrap();
        assert_eq!(
            cell_msg_dialog_progress_bar_inc(&mut m, 0, 10).unwrap_err(),
            errors::PARAM,
        );
    }

    #[test]
    fn progress_bar_double_tracks_both_bars() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(PROGRESSBAR_DOUBLE), "").unwrap();
        cell_msg_dialog_progress_bar_inc(&mut m, 0, 25).unwrap();
        cell_msg_dialog_progress_bar_inc(&mut m, 1, 50).unwrap();
        assert_eq!(m.progress_upper(), 25);
        assert_eq!(m.progress_lower(), 50);
    }

    #[test]
    fn progress_bar_reset_zeros_selected_bar() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(PROGRESSBAR_DOUBLE), "").unwrap();
        cell_msg_dialog_progress_bar_inc(&mut m, 0, 40).unwrap();
        cell_msg_dialog_progress_bar_inc(&mut m, 1, 60).unwrap();
        cell_msg_dialog_progress_bar_reset(&mut m, 0).unwrap();
        assert_eq!(m.progress_upper(), 0);
        assert_eq!(m.progress_lower(), 60);
    }

    #[test]
    fn progress_bar_msg_over_64_chars_is_param() {
        let mut m = DialogManager::default();
        cell_msg_dialog_open(&mut m, TypeFlags(PROGRESSBAR_SINGLE), "").unwrap();
        let long = "x".repeat(65);
        assert_eq!(
            cell_msg_dialog_progress_bar_set_msg(&mut m, 0, &long).unwrap_err(),
            errors::PARAM,
        );
    }

    #[test]
    fn progress_bar_ops_without_open_is_dialog_not_opened() {
        let mut m = DialogManager::default();
        assert_eq!(
            cell_msg_dialog_progress_bar_inc(&mut m, 0, 1).unwrap_err(),
            errors::DIALOG_NOT_OPENED,
        );
        assert_eq!(
            cell_msg_dialog_progress_bar_reset(&mut m, 0).unwrap_err(),
            errors::DIALOG_NOT_OPENED,
        );
    }
}
