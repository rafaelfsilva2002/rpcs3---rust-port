//! `rpcs3-io-turntable` — Rust port of `rpcs3/Emu/Io/Turntable.cpp`.
//!
//! DJ Hero Turntable USB controller emulator. Byte-exact frozen:
//!
//! - USB descriptor constants (cpp:45..50): VID=0x12BA/PID=0x0140,
//!   bcdDevice=0x0005, HID 0x0110, two interrupt endpoints (0x81 IN,
//!   0x02 OUT, 64B packet, 10ms interval).
//! - `turntable_btn` enum order (17 variants including `count`).
//! - Face button / start-select / platter bitmasks (cpp:106..157).
//! - 27-byte interrupt report preamble (cpp:102..160): buf[2]=0x0F
//!   (dpad_none), buf[3/4/5/6]=0x80 (turntables idle), buf[20]=0x02,
//!   buf[22]=0x02, buf[24/26]=0x02.
//! - Dpad state machine (cpp:211..270): `dpad_up` / `dpad_down` /
//!   `dpad_left` / `dpad_right` each consult buf[2] to fold into diagonals.
//! - Double-press `~` toggle trick (cpp:135..142): if a byte represents
//!   two buttons and both are pressed, the NOT runs twice and yields
//!   0x00 again. `apply_button` does one `~=` per press so a single slot
//!   acts like the cpp's `buf[X] = ~buf[X]`.
//! - Analog encoders (cpp:287..294): crossfader and effects dial split
//!   their 8-bit value into a (low<<2 | 0x3F) 6-bit chunk in buf[N] and a
//!   (high>>6 | 0xC0) 2-bit chunk in buf[N+1]. Crossfader additionally
//!   inverts via `255 - value` (cpp:288).
//! - Right-turntable quirk (cpp:279..285): `max(1, 255 - value)` with a
//!   127→128 re-center snap because DJ Hero refuses the center-off-by-one.
//! - Interrupt latency override 1ms (cpp:100, normally 10ms at 100Hz).

/// Minimum interrupt transfer size (cpp:93).
pub const MIN_BUF_SIZE: usize = 27;

pub const INTERRUPT_EXPECTED_LATENCY_US: u64 = 1_000;

// USB descriptor constants (cpp:45..50).
pub const USB_VID: u16 = 0x12BA;
pub const USB_PID: u16 = 0x0140;
pub const USB_BCD_DEVICE: u16 = 0x0005;
pub const USB_BCD_USB: u16 = 0x0100;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x40;
pub const USB_CONFIG_W_TOTAL_LENGTH: u16 = 0x0029;
pub const USB_CONFIG_MAX_POWER: u8 = 0x19;
pub const USB_INTERFACE_CLASS_HID: u8 = 0x03;
pub const USB_HID_BCD: u16 = 0x0110;
pub const USB_HID_DESCRIPTOR_LENGTH: u16 = 0x0089;
pub const USB_ENDPOINT_IN_ADDRESS: u8 = 0x81;
pub const USB_ENDPOINT_OUT_ADDRESS: u8 = 0x02;
pub const USB_ENDPOINT_BM_ATTRIBUTES: u8 = 0x03;
pub const USB_ENDPOINT_W_MAX_PACKET_SIZE: u16 = 0x0040;
pub const USB_ENDPOINT_B_INTERVAL: u8 = 0x0a;

/// Button enum in cpp declaration order (`turntable_config.h`).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurntableBtn {
    Blue = 0,
    Green = 1,
    Red = 2,
    DpadUp = 3,
    DpadDown = 4,
    DpadLeft = 5,
    DpadRight = 6,
    Start = 7,
    Select = 8,
    Square = 9,
    Circle = 10,
    Cross = 11,
    Triangle = 12,
    RightTurntable = 13,
    Crossfader = 14,
    EffectsDial = 15,
    Count = 16,
}

// Face button masks for buf[0] (cpp:106..109).
pub const FACE_SQUARE: u8 = 0x01;
pub const FACE_CROSS: u8 = 0x02;
pub const FACE_CIRCLE: u8 = 0x04;
pub const FACE_TRIANGLE: u8 = 0x08;

// Start/Select/PS masks for buf[1] (cpp:112..115).
pub const SS_SELECT: u8 = 0x01;
pub const SS_START: u8 = 0x02;
pub const SS_PS: u8 = 0x10;

// Dpad byte values for buf[2] (cpp:119..127).
pub const DPAD_UP: u8 = 0x00;
pub const DPAD_UP_RIGHT: u8 = 0x01;
pub const DPAD_RIGHT: u8 = 0x02;
pub const DPAD_RIGHT_DOWN: u8 = 0x03;
pub const DPAD_DOWN: u8 = 0x04;
pub const DPAD_DOWN_LEFT: u8 = 0x05;
pub const DPAD_LEFT: u8 = 0x06;
pub const DPAD_UP_LEFT: u8 = 0x07;
pub const DPAD_NONE: u8 = 0x0F;

// Platter button bitmasks for buf[23] (cpp:150..157).
pub const PLATTER_R_GREEN: u8 = 0x01;
pub const PLATTER_R_RED: u8 = 0x02;
pub const PLATTER_R_BLUE: u8 = 0x04;
pub const PLATTER_L_GREEN: u8 = 0x10;
pub const PLATTER_L_RED: u8 = 0x20;
pub const PLATTER_L_BLUE: u8 = 0x40;

/// Fill `buf` with the neutral 27-byte preamble (cpp:102..160).
pub fn reset_report(buf: &mut [u8]) {
    assert!(
        buf.len() >= MIN_BUF_SIZE,
        "DJ Hero turntable report needs >= {MIN_BUF_SIZE} bytes"
    );
    for b in buf.iter_mut() {
        *b = 0;
    }
    buf[2] = DPAD_NONE;
    buf[3] = 0x80;
    buf[4] = 0x80;
    buf[5] = 0x80; // Left turntable idle
    buf[6] = 0x80; // Right turntable idle
    buf[20] = 0x02;
    buf[22] = 0x02;
    buf[24] = 0x02;
    buf[26] = 0x02;
}

/// Folds a press of `dpad_up` into the current `buf[2]` (cpp:226..239).
#[must_use]
pub const fn dpad_up_fold(buf2: u8) -> u8 {
    match buf2 {
        DPAD_RIGHT => DPAD_UP_RIGHT,
        DPAD_LEFT => DPAD_UP_LEFT,
        _ => DPAD_UP,
    }
}

#[must_use]
pub const fn dpad_down_fold(buf2: u8) -> u8 {
    match buf2 {
        DPAD_RIGHT => DPAD_RIGHT_DOWN,
        DPAD_LEFT => DPAD_DOWN_LEFT,
        _ => DPAD_DOWN,
    }
}

#[must_use]
pub const fn dpad_left_fold(buf2: u8) -> u8 {
    match buf2 {
        DPAD_UP => DPAD_UP_LEFT,
        DPAD_DOWN => DPAD_DOWN_LEFT,
        _ => DPAD_LEFT,
    }
}

#[must_use]
pub const fn dpad_right_fold(buf2: u8) -> u8 {
    match buf2 {
        DPAD_UP => DPAD_UP_RIGHT,
        DPAD_DOWN => DPAD_RIGHT_DOWN,
        _ => DPAD_RIGHT,
    }
}

/// Right-turntable value encode (cpp:278..285). Takes an 8-bit axis value,
/// inverts via `255 - value`, floors at 1 (DJ Hero refuses 0), and snaps
/// 127 → 128 so the center is correct.
#[must_use]
pub fn encode_right_turntable(value: u16) -> u8 {
    let inverted = (255_i32 - value as i32).max(1) as u8;
    if inverted == 127 { 128 } else { inverted }
}

/// Crossfader encode (cpp:288..289). Returns `(low, high)` bytes.
#[must_use]
pub const fn encode_crossfader(value: u16) -> (u8, u8) {
    let inverted = 255u16.wrapping_sub(value);
    let low = ((inverted & 0x3F) << 2) as u8;
    let high = ((inverted & 0xC0) >> 6) as u8;
    (low, high)
}

/// Effects-dial encode (cpp:292..293). Returns `(low, high)` bytes. No
/// inversion (unlike the crossfader).
#[must_use]
pub const fn encode_effects_dial(value: u16) -> (u8, u8) {
    let low = ((value & 0x3F) << 2) as u8;
    let high = ((value & 0xC0) >> 6) as u8;
    (low, high)
}

/// Apply a single button press to the report buffer. `value` only matters
/// for `RightTurntable` / `Crossfader` / `EffectsDial`.
pub fn apply_button(buf: &mut [u8], btn: TurntableBtn, value: u16) -> bool {
    assert!(
        buf.len() >= MIN_BUF_SIZE,
        "DJ Hero turntable report needs >= {MIN_BUF_SIZE} bytes"
    );
    match btn {
        TurntableBtn::Blue => {
            buf[0] |= FACE_SQUARE;
            buf[7] ^= 0xFF; // `~` toggle trick
            buf[23] |= PLATTER_R_BLUE;
            true
        }
        TurntableBtn::Green => {
            buf[0] |= FACE_CROSS;
            buf[9] ^= 0xFF;
            buf[23] |= PLATTER_R_GREEN;
            true
        }
        TurntableBtn::Red => {
            buf[0] |= FACE_CIRCLE;
            buf[12] ^= 0xFF;
            buf[23] |= PLATTER_R_RED;
            true
        }
        TurntableBtn::Triangle => {
            buf[0] |= FACE_TRIANGLE;
            buf[11] ^= 0xFF;
            true
        }
        TurntableBtn::Cross => {
            buf[0] |= FACE_CROSS;
            buf[9] ^= 0xFF;
            true
        }
        TurntableBtn::Circle => {
            buf[0] |= FACE_CIRCLE;
            buf[12] ^= 0xFF;
            true
        }
        TurntableBtn::Square => {
            buf[0] |= FACE_SQUARE;
            buf[7] ^= 0xFF;
            true
        }
        TurntableBtn::DpadDown => {
            buf[2] = dpad_down_fold(buf[2]);
            buf[10] ^= 0xFF;
            true
        }
        TurntableBtn::DpadUp => {
            buf[2] = dpad_up_fold(buf[2]);
            buf[9] ^= 0xFF;
            true
        }
        TurntableBtn::DpadLeft => {
            buf[2] = dpad_left_fold(buf[2]);
            buf[8] ^= 0xFF;
            true
        }
        TurntableBtn::DpadRight => {
            buf[2] = dpad_right_fold(buf[2]);
            buf[7] ^= 0xFF;
            true
        }
        TurntableBtn::Start => {
            buf[1] |= SS_START;
            true
        }
        TurntableBtn::Select => {
            buf[1] |= SS_SELECT;
            true
        }
        TurntableBtn::RightTurntable => {
            buf[6] = encode_right_turntable(value);
            true
        }
        TurntableBtn::Crossfader => {
            let (low, high) = encode_crossfader(value);
            buf[21] = low;
            buf[22] = high;
            true
        }
        TurntableBtn::EffectsDial => {
            let (low, high) = encode_effects_dial(value);
            buf[19] = low;
            buf[20] = high;
            true
        }
        TurntableBtn::Count => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btn_enum_order() {
        assert_eq!(TurntableBtn::Blue as u32, 0);
        assert_eq!(TurntableBtn::Count as u32, 16);
        assert_eq!(TurntableBtn::EffectsDial as u32, 15);
    }

    #[test]
    fn usb_vid_pid() {
        assert_eq!(USB_VID, 0x12BA);
        assert_eq!(USB_PID, 0x0140);
        assert_eq!(USB_BCD_DEVICE, 0x0005);
        assert_eq!(USB_ENDPOINT_IN_ADDRESS, 0x81);
        assert_eq!(USB_ENDPOINT_OUT_ADDRESS, 0x02);
    }

    #[test]
    fn reset_report_preamble() {
        let mut buf = [0xffu8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        assert_eq!(buf[2], DPAD_NONE);
        assert_eq!(buf[3], 0x80);
        assert_eq!(buf[5], 0x80);
        assert_eq!(buf[6], 0x80);
        assert_eq!(buf[20], 0x02);
        assert_eq!(buf[22], 0x02);
        assert_eq!(buf[24], 0x02);
        assert_eq!(buf[26], 0x02);
        // Gaps zeroed.
        assert_eq!(buf[13], 0);
        assert_eq!(buf[25], 0);
    }

    #[test]
    fn dpad_fold_no_prior() {
        assert_eq!(dpad_up_fold(DPAD_NONE), DPAD_UP);
        assert_eq!(dpad_down_fold(DPAD_NONE), DPAD_DOWN);
        assert_eq!(dpad_left_fold(DPAD_NONE), DPAD_LEFT);
        assert_eq!(dpad_right_fold(DPAD_NONE), DPAD_RIGHT);
    }

    #[test]
    fn dpad_fold_combinations() {
        // Up pressed after Right → Up-Right
        assert_eq!(dpad_up_fold(DPAD_RIGHT), DPAD_UP_RIGHT);
        assert_eq!(dpad_up_fold(DPAD_LEFT), DPAD_UP_LEFT);
        // Down + Right → Right-Down
        assert_eq!(dpad_down_fold(DPAD_RIGHT), DPAD_RIGHT_DOWN);
        assert_eq!(dpad_down_fold(DPAD_LEFT), DPAD_DOWN_LEFT);
        // Left after Up/Down
        assert_eq!(dpad_left_fold(DPAD_UP), DPAD_UP_LEFT);
        assert_eq!(dpad_left_fold(DPAD_DOWN), DPAD_DOWN_LEFT);
        // Right after Up/Down
        assert_eq!(dpad_right_fold(DPAD_UP), DPAD_UP_RIGHT);
        assert_eq!(dpad_right_fold(DPAD_DOWN), DPAD_RIGHT_DOWN);
    }

    #[test]
    fn face_button_masks_or_into_buf0_and_platter() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        apply_button(&mut buf, TurntableBtn::Blue, 0);
        assert_eq!(buf[0], FACE_SQUARE);
        assert_eq!(buf[7], 0xFF); // ~0 = 0xFF
        assert_eq!(buf[23], PLATTER_R_BLUE);

        apply_button(&mut buf, TurntableBtn::Green, 0);
        assert_eq!(buf[0], FACE_SQUARE | FACE_CROSS);
        assert_eq!(buf[9], 0xFF);
        assert_eq!(buf[23], PLATTER_R_BLUE | PLATTER_R_GREEN);
    }

    #[test]
    fn double_press_not_trick_collapses_to_zero() {
        // Blue + Square both press buf[7] = ~buf[7]. After two flips the
        // byte is back to 0, mirroring cpp:135..142 ("NOTed twice").
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        apply_button(&mut buf, TurntableBtn::Blue, 0);
        assert_eq!(buf[7], 0xFF);
        apply_button(&mut buf, TurntableBtn::Square, 0);
        assert_eq!(buf[7], 0x00);
    }

    #[test]
    fn start_select_masks() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        apply_button(&mut buf, TurntableBtn::Start, 0);
        apply_button(&mut buf, TurntableBtn::Select, 0);
        assert_eq!(buf[1], SS_START | SS_SELECT);
    }

    #[test]
    fn right_turntable_inverts_and_floors() {
        // value 0 → inverted 255 → pass-through.
        assert_eq!(encode_right_turntable(0), 255);
        // value 255 → inverted 0 → floored to 1.
        assert_eq!(encode_right_turntable(255), 1);
        // value 127 → inverted 128.
        assert_eq!(encode_right_turntable(127), 128);
        // value 128 → inverted 127 → snapped to 128.
        assert_eq!(encode_right_turntable(128), 128);
    }

    #[test]
    fn crossfader_splits_and_inverts() {
        // value=0 → inverted=255=0xFF; low=(0x3F)<<2=0xFC, high=(0xC0)>>6=3.
        assert_eq!(encode_crossfader(0), (0xFC, 0x03));
        // value=255 → inverted=0 → (0, 0).
        assert_eq!(encode_crossfader(255), (0x00, 0x00));
    }

    #[test]
    fn effects_dial_splits_no_inversion() {
        // value=0xFF → low=(0x3F)<<2=0xFC, high=(0xC0)>>6=3.
        assert_eq!(encode_effects_dial(0xFF), (0xFC, 0x03));
        assert_eq!(encode_effects_dial(0x00), (0x00, 0x00));
        // value=0x40 → low=(0)<<2=0, high=(0x40)>>6=1.
        assert_eq!(encode_effects_dial(0x40), (0x00, 0x01));
    }

    #[test]
    fn dpad_up_then_right_yields_up_right() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        apply_button(&mut buf, TurntableBtn::DpadUp, 0);
        assert_eq!(buf[2], DPAD_UP);
        apply_button(&mut buf, TurntableBtn::DpadRight, 0);
        assert_eq!(buf[2], DPAD_UP_RIGHT);
    }

    #[test]
    fn count_is_noop() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        assert!(!apply_button(&mut buf, TurntableBtn::Count, 0));
    }
}
