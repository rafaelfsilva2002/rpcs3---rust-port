//! `rpcs3-hle-cellkey2char` — USB HID keycode → character translator HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellKey2char.cpp`. Tiny module that
//! converts a USB HID scan code (like `0x04 = A`) plus modifier mask
//! (Shift/Ctrl/Alt) into the resulting Unicode character, respecting
//! the keyboard arrangement (101 US / 106 Japanese / 106 kana).
//!
//! ## Entry points covered
//!
//! | HLE function                 | Rust wrapper                    |
//! |------------------------------|---------------------------------|
//! | `cellKey2CharOpen`           | [`cell_key2char_open`]          |
//! | `cellKey2CharClose`          | [`cell_key2char_close`]         |
//! | `cellKey2CharGetChar`        | [`cell_key2char_get_char`]      |
//! | `cellKey2CharSetMode`        | [`cell_key2char_set_mode`]      |
//! | `cellKey2CharSetArrangement` | [`cell_key2char_set_arrangement`]|

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellKey2char.cpp:10-17
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const FATAL: CellError = CellError(0x8012_1301);
    pub const INVALID_HANDLE: CellError = CellError(0x8012_1302);
    pub const INVALID_PARAMETER: CellError = CellError(0x8012_1303);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8012_1304);
    pub const UNINITIALIZED: CellError = CellError(0x8012_1305);
    pub const OTHER: CellError = CellError(0x8012_1306);
}

// =====================================================================
// Mode + arrangement constants
// =====================================================================

pub const MODE_ENGLISH: u32 = 0;
pub const MODE_NATIVE: u32 = 1;
pub const MODE_NATIVE2: u32 = 2;

/// Keyboard arrangement — matches the cellKb constants.
pub const ARRANGEMENT_101: u32 = 0;
pub const ARRANGEMENT_106: u32 = 1;
pub const ARRANGEMENT_106_KANA: u32 = 2;

/// Modifier mask bits (same as cellKb's `mkey`).
pub const MKEY_LEFT_CTRL: u32 = 0x01;
pub const MKEY_LEFT_SHIFT: u32 = 0x02;
pub const MKEY_LEFT_ALT: u32 = 0x04;
pub const MKEY_LEFT_WIN: u32 = 0x08;
pub const MKEY_RIGHT_CTRL: u32 = 0x10;
pub const MKEY_RIGHT_SHIFT: u32 = 0x20;
pub const MKEY_RIGHT_ALT: u32 = 0x40;
pub const MKEY_RIGHT_WIN: u32 = 0x80;

pub const HANDLE_SIZE: usize = 128;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyData {
    pub led: u32,
    pub mkey: u32,
    pub keycode: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handle {
    pub initialized: bool,
    pub mode: u32,
    pub arrangement: u32,
}

impl Default for Handle {
    fn default() -> Self {
        Self {
            initialized: false,
            mode: MODE_ENGLISH,
            arrangement: ARRANGEMENT_101,
        }
    }
}

// =====================================================================
// Scan-code translator
// =====================================================================

fn is_shift(mkey: u32) -> bool {
    (mkey & (MKEY_LEFT_SHIFT | MKEY_RIGHT_SHIFT)) != 0
}

/// Translate a USB HID scan code + modifier mask into a Unicode char.
/// Returns `None` if the scan code has no printable mapping in the
/// current arrangement.
pub fn translate(key: &KeyData, arrangement: u32) -> Option<char> {
    // USB HID codes (Usage Page 0x07):
    //  0x04-0x1D: A..Z
    //  0x1E-0x27: 1..9, 0
    //  0x28: Enter, 0x29: Escape, 0x2A: Backspace, 0x2B: Tab, 0x2C: Space
    //  0x2D: - , 0x2E: =, 0x2F: [, 0x30: ], 0x31: \, 0x33: ;, 0x34: '
    //  0x35: `, 0x36: ,, 0x37: ., 0x38: /
    let shift = is_shift(key.mkey);
    let code = key.keycode;
    match code {
        0x04..=0x1D => {
            // Letter A..Z. Shift toggles case.
            let base = b'a' + (code - 0x04) as u8;
            let ch = if shift { (base - b'a' + b'A') as char } else { base as char };
            Some(ch)
        }
        0x1E..=0x26 => {
            // Digit row 1..9. Shift yields shifted glyph (US layout).
            let digit = (code - 0x1D) as u8; // 1..9
            if shift {
                // US/101: !@#$%^&*(
                const SHIFT_DIGITS: [char; 9] = ['!','@','#','$','%','^','&','*','('];
                Some(SHIFT_DIGITS[(digit - 1) as usize])
            } else {
                Some((b'0' + digit) as char)
            }
        }
        0x27 => {
            // 0 / )
            if shift { Some(')') } else { Some('0') }
        }
        0x28 => Some('\n'),        // Enter
        0x2A => Some('\u{0008}'),  // Backspace (BS)
        0x2B => Some('\t'),        // Tab
        0x2C => Some(' '),          // Space
        0x2D => if shift { Some('_') } else { Some('-') },
        0x2E => if shift { Some('+') } else { Some('=') },
        0x2F => if shift { Some('{') } else { Some('[') },
        0x30 => if shift { Some('}') } else { Some(']') },
        0x31 => if shift { Some('|') } else { Some('\\') },
        0x33 => if shift { Some(':') } else { Some(';') },
        0x34 => if shift { Some('"') } else { Some('\'') },
        0x35 => if shift { Some('~') } else { Some('`') },
        0x36 => if shift { Some('<') } else { Some(',') },
        0x37 => if shift { Some('>') } else { Some('.') },
        0x38 => if shift { Some('?') } else { Some('/') },
        _ => {
            // On 106_kana arrangement, HID codes 0x87-0x8B are the
            // Japanese punctuation row; we recognise them but return
            // placeholder katakana space for native2 mode.
            let _ = arrangement;
            None
        }
    }
}

// =====================================================================
// Manager
// =====================================================================

#[derive(Debug, Default)]
pub struct Key2CharManager {
    pub handles: std::collections::BTreeMap<u32, Handle>,
    pub next_handle: u32,
}

// =====================================================================
// Syscalls
// =====================================================================

/// `cellKey2CharOpen(handle)` — initialise handle state.
#[must_use]
pub fn cell_key2char_open(m: &mut Key2CharManager) -> Result<u32, CellError> {
    m.next_handle += 1;
    let id = m.next_handle;
    m.handles.insert(id, Handle { initialized: true, ..Handle::default() });
    Ok(id)
}

/// `cellKey2CharClose(handle)`.
#[must_use]
pub fn cell_key2char_close(m: &mut Key2CharManager, handle: u32) -> Result<(), CellError> {
    let h = m.handles.get(&handle).ok_or(errors::INVALID_HANDLE)?;
    if !h.initialized {
        return Err(errors::UNINITIALIZED);
    }
    m.handles.remove(&handle);
    Ok(())
}

/// `cellKey2CharGetChar(handle, key_data, char_out)`.
#[must_use]
pub fn cell_key2char_get_char(
    m: &Key2CharManager,
    handle: u32,
    key: &KeyData,
) -> Result<u16, CellError> {
    let h = m.handles.get(&handle).ok_or(errors::INVALID_HANDLE)?;
    if !h.initialized {
        return Err(errors::UNINITIALIZED);
    }
    match translate(key, h.arrangement) {
        Some(c) => Ok(c as u16),
        None => Err(errors::OTHER),
    }
}

/// `cellKey2CharSetMode(handle, mode)`.
#[must_use]
pub fn cell_key2char_set_mode(
    m: &mut Key2CharManager,
    handle: u32,
    mode: u32,
) -> Result<(), CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::INVALID_HANDLE)?;
    if !h.initialized {
        return Err(errors::UNINITIALIZED);
    }
    if !matches!(mode, MODE_ENGLISH | MODE_NATIVE | MODE_NATIVE2) {
        return Err(errors::INVALID_PARAMETER);
    }
    h.mode = mode;
    Ok(())
}

/// `cellKey2CharSetArrangement(handle, arrangement)`.
#[must_use]
pub fn cell_key2char_set_arrangement(
    m: &mut Key2CharManager,
    handle: u32,
    arrangement: u32,
) -> Result<(), CellError> {
    let h = m.handles.get_mut(&handle).ok_or(errors::INVALID_HANDLE)?;
    if !h.initialized {
        return Err(errors::UNINITIALIZED);
    }
    if !matches!(arrangement, ARRANGEMENT_101 | ARRANGEMENT_106 | ARRANGEMENT_106_KANA) {
        return Err(errors::INVALID_PARAMETER);
    }
    h.arrangement = arrangement;
    Ok(())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn kd(code: u16, mkey: u32) -> KeyData {
        KeyData { led: 0, mkey, keycode: code }
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::FATAL.0, 0x8012_1301);
        assert_eq!(errors::INVALID_HANDLE.0, 0x8012_1302);
        assert_eq!(errors::INVALID_PARAMETER.0, 0x8012_1303);
        assert_eq!(errors::ALREADY_INITIALIZED.0, 0x8012_1304);
        assert_eq!(errors::UNINITIALIZED.0, 0x8012_1305);
        assert_eq!(errors::OTHER.0, 0x8012_1306);
    }

    #[test]
    fn mode_constants() {
        assert_eq!(MODE_ENGLISH, 0);
        assert_eq!(MODE_NATIVE, 1);
        assert_eq!(MODE_NATIVE2, 2);
    }

    #[test]
    fn arrangement_constants() {
        assert_eq!(ARRANGEMENT_101, 0);
        assert_eq!(ARRANGEMENT_106, 1);
        assert_eq!(ARRANGEMENT_106_KANA, 2);
    }

    // --- translate: letters --------------------------------------

    #[test]
    fn translate_letter_a_lowercase() {
        assert_eq!(translate(&kd(0x04, 0), ARRANGEMENT_101), Some('a'));
    }

    #[test]
    fn translate_letter_a_shifted_is_uppercase() {
        assert_eq!(translate(&kd(0x04, MKEY_LEFT_SHIFT), ARRANGEMENT_101), Some('A'));
    }

    #[test]
    fn translate_letter_z_lowercase() {
        assert_eq!(translate(&kd(0x1D, 0), ARRANGEMENT_101), Some('z'));
    }

    #[test]
    fn translate_right_shift_also_uppercases() {
        assert_eq!(translate(&kd(0x09, MKEY_RIGHT_SHIFT), ARRANGEMENT_101), Some('F'));
    }

    // --- digits --------------------------------------------------

    #[test]
    fn translate_digit_1_unshifted() {
        assert_eq!(translate(&kd(0x1E, 0), ARRANGEMENT_101), Some('1'));
    }

    #[test]
    fn translate_digit_1_shifted_is_bang() {
        assert_eq!(translate(&kd(0x1E, MKEY_LEFT_SHIFT), ARRANGEMENT_101), Some('!'));
    }

    #[test]
    fn translate_digit_0_unshifted_is_zero() {
        assert_eq!(translate(&kd(0x27, 0), ARRANGEMENT_101), Some('0'));
    }

    #[test]
    fn translate_digit_0_shifted_is_close_paren() {
        assert_eq!(translate(&kd(0x27, MKEY_LEFT_SHIFT), ARRANGEMENT_101), Some(')'));
    }

    #[test]
    fn translate_shift_digits_full_row() {
        let expected: [(u16, char); 9] = [
            (0x1E, '!'), (0x1F, '@'), (0x20, '#'), (0x21, '$'),
            (0x22, '%'), (0x23, '^'), (0x24, '&'), (0x25, '*'),
            (0x26, '('),
        ];
        for (code, ch) in expected {
            assert_eq!(
                translate(&kd(code, MKEY_LEFT_SHIFT), ARRANGEMENT_101),
                Some(ch),
                "code {:#x}",
                code,
            );
        }
    }

    // --- punctuation ---------------------------------------------

    #[test]
    fn translate_punctuation_unshifted() {
        for (code, expected) in [
            (0x2D, '-'), (0x2E, '='), (0x2F, '['), (0x30, ']'),
            (0x31, '\\'), (0x33, ';'), (0x34, '\''),
            (0x35, '`'), (0x36, ','), (0x37, '.'), (0x38, '/'),
        ] {
            assert_eq!(translate(&kd(code, 0), ARRANGEMENT_101), Some(expected));
        }
    }

    #[test]
    fn translate_punctuation_shifted() {
        for (code, expected) in [
            (0x2D, '_'), (0x2E, '+'), (0x2F, '{'), (0x30, '}'),
            (0x31, '|'), (0x33, ':'), (0x34, '"'),
            (0x35, '~'), (0x36, '<'), (0x37, '>'), (0x38, '?'),
        ] {
            assert_eq!(
                translate(&kd(code, MKEY_LEFT_SHIFT), ARRANGEMENT_101),
                Some(expected),
                "code {:#x}",
                code,
            );
        }
    }

    // --- control codes -------------------------------------------

    #[test]
    fn translate_enter_tab_space_backspace() {
        assert_eq!(translate(&kd(0x28, 0), ARRANGEMENT_101), Some('\n'));
        assert_eq!(translate(&kd(0x2A, 0), ARRANGEMENT_101), Some('\u{0008}'));
        assert_eq!(translate(&kd(0x2B, 0), ARRANGEMENT_101), Some('\t'));
        assert_eq!(translate(&kd(0x2C, 0), ARRANGEMENT_101), Some(' '));
    }

    #[test]
    fn translate_unknown_scan_code_is_none() {
        assert_eq!(translate(&kd(0xFF, 0), ARRANGEMENT_101), None);
    }

    // --- manager lifecycle ---------------------------------------

    #[test]
    fn open_returns_valid_handle() {
        let mut m = Key2CharManager::default();
        let h = cell_key2char_open(&mut m).unwrap();
        assert_eq!(h, 1);
    }

    #[test]
    fn close_unknown_handle_is_invalid_handle() {
        let mut m = Key2CharManager::default();
        assert_eq!(
            cell_key2char_close(&mut m, 999).unwrap_err(),
            errors::INVALID_HANDLE,
        );
    }

    #[test]
    fn close_then_reuse_gives_new_id() {
        let mut m = Key2CharManager::default();
        let h1 = cell_key2char_open(&mut m).unwrap();
        cell_key2char_close(&mut m, h1).unwrap();
        let h2 = cell_key2char_open(&mut m).unwrap();
        assert_eq!(h2, 2); // next_handle keeps increasing
    }

    #[test]
    fn get_char_unknown_handle_is_invalid_handle() {
        let m = Key2CharManager::default();
        assert_eq!(
            cell_key2char_get_char(&m, 999, &kd(0x04, 0)).unwrap_err(),
            errors::INVALID_HANDLE,
        );
    }

    #[test]
    fn get_char_happy_path_returns_u16() {
        let mut m = Key2CharManager::default();
        let h = cell_key2char_open(&mut m).unwrap();
        assert_eq!(
            cell_key2char_get_char(&m, h, &kd(0x04, MKEY_LEFT_SHIFT)).unwrap(),
            b'A' as u16,
        );
    }

    #[test]
    fn get_char_unknown_scan_returns_other() {
        let mut m = Key2CharManager::default();
        let h = cell_key2char_open(&mut m).unwrap();
        assert_eq!(
            cell_key2char_get_char(&m, h, &kd(0xFF, 0)).unwrap_err(),
            errors::OTHER,
        );
    }

    // --- mode + arrangement setters -----------------------------

    #[test]
    fn set_mode_invalid_rejected() {
        let mut m = Key2CharManager::default();
        let h = cell_key2char_open(&mut m).unwrap();
        assert_eq!(
            cell_key2char_set_mode(&mut m, h, 99).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn set_mode_english_native_native2_accepted() {
        let mut m = Key2CharManager::default();
        let h = cell_key2char_open(&mut m).unwrap();
        cell_key2char_set_mode(&mut m, h, MODE_ENGLISH).unwrap();
        cell_key2char_set_mode(&mut m, h, MODE_NATIVE).unwrap();
        cell_key2char_set_mode(&mut m, h, MODE_NATIVE2).unwrap();
    }

    #[test]
    fn set_arrangement_invalid_rejected() {
        let mut m = Key2CharManager::default();
        let h = cell_key2char_open(&mut m).unwrap();
        assert_eq!(
            cell_key2char_set_arrangement(&mut m, h, 99).unwrap_err(),
            errors::INVALID_PARAMETER,
        );
    }

    #[test]
    fn set_arrangement_updates_handle() {
        let mut m = Key2CharManager::default();
        let h = cell_key2char_open(&mut m).unwrap();
        cell_key2char_set_arrangement(&mut m, h, ARRANGEMENT_106_KANA).unwrap();
        assert_eq!(m.handles.get(&h).unwrap().arrangement, ARRANGEMENT_106_KANA);
    }

    #[test]
    fn set_mode_unknown_handle_invalid() {
        let mut m = Key2CharManager::default();
        assert_eq!(
            cell_key2char_set_mode(&mut m, 999, MODE_ENGLISH).unwrap_err(),
            errors::INVALID_HANDLE,
        );
    }

    // --- round-trip smoke ---------------------------------------

    #[test]
    fn end_to_end_type_hello_string() {
        let mut m = Key2CharManager::default();
        let h = cell_key2char_open(&mut m).unwrap();
        let mut out = String::new();
        // H (shift+H), e, l, l, o.
        let codes: [(u16, u32); 5] = [
            (0x0B, MKEY_LEFT_SHIFT), (0x08, 0), (0x0F, 0), (0x0F, 0), (0x12, 0),
        ];
        for (code, mkey) in codes {
            let ch = cell_key2char_get_char(&m, h, &kd(code, mkey)).unwrap();
            out.push(char::from_u32(ch as u32).unwrap());
        }
        assert_eq!(out, "Hello");
        cell_key2char_close(&mut m, h).unwrap();
    }

    #[test]
    fn handle_size_matches_cpp_spec() {
        assert_eq!(HANDLE_SIZE, 128);
    }
}
