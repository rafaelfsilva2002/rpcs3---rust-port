//! `rpcs3-io-gametablet` — Rust port of `rpcs3/Emu/Io/GameTablet.cpp`.
//!
//! THQ uDraw Game Tablet emulator. Freezes:
//!
//! - USB descriptor constants: VID=0x20d6 / PID=0xcb17, bcdDevice=0x0108,
//!   HID class 0x03, two interrupt endpoints (0x83 IN, 0x04 OUT).
//! - The `GameTablet_data` struct layout (`#pragma pack(push, 1)` at
//!   cpp:11..50) — 27 bytes including button bitfields, analog sticks,
//!   pen state, position (hi/lo bytes), and 4 BE u16 accelerometer/unk
//!   slots. Size asserted at compile time.
//! - Dpad encoding (cpp:52..63): 0..=7 compass points, 0x0F = None.
//! - Neutral defaults written in `interrupt_transfer` cpp:174..179:
//!   sticks=0x80, pressure=0x72, pen=0x00, pos_hi=0x0F, pos_lo=0xFF,
//!   accel_*=0x0200, unk=0x0200.
//! - Dpad encoder (cpp:263..280): cascade of up/down/left/right combos
//!   preserved in that exact priority order.
//! - Position mapping (cpp:303..316): tablet_max_x=1920, tablet_max_y=1080,
//!   `tablet_x = mouse_x * 1920 / mouse_max_x ^ noise_x` with `noise_*` a
//!   1-bit LSB toggle (Instant Artist dislikes a pen held perfectly still).
//! - Pressure mapping (cpp:312): CELL_MOUSE_BUTTON_1 → 0xbb, idle → 0x72.
//! - Pen state (cpp:311): 0x40 when mouse is active.
//! - LED SET_REPORT decoding (cpp:143..147): four bits at `buf[2] & 0x0F`.

/// Dpad byte values (cpp:52..63).
pub const DPAD_NORTH: u8 = 0;
pub const DPAD_NE: u8 = 1;
pub const DPAD_EAST: u8 = 2;
pub const DPAD_SE: u8 = 3;
pub const DPAD_SOUTH: u8 = 4;
pub const DPAD_SW: u8 = 5;
pub const DPAD_WEST: u8 = 6;
pub const DPAD_NW: u8 = 7;
pub const DPAD_NONE: u8 = 0x0F;

/// USB descriptor constants (cpp:69..118).
pub const USB_VID: u16 = 0x20d6;
pub const USB_PID: u16 = 0xcb17;
pub const USB_BCD_DEVICE: u16 = 0x0108;
pub const USB_BCD_USB: u16 = 0x0200;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x08;
pub const USB_CONFIG_W_TOTAL_LENGTH: u16 = 0x0029;
pub const USB_INTERFACE_CLASS_HID: u8 = 0x03;
pub const USB_HID_BCD: u16 = 0x0110;
pub const USB_HID_DESCRIPTOR_LENGTH: u16 = 0x0089;
pub const USB_ENDPOINT_IN_ADDRESS: u8 = 0x83;
pub const USB_ENDPOINT_OUT_ADDRESS: u8 = 0x04;
pub const USB_ENDPOINT_BM_ATTRIBUTES: u8 = 0x03;
pub const USB_ENDPOINT_W_MAX_PACKET_SIZE: u16 = 0x0040;
pub const USB_ENDPOINT_B_INTERVAL: u8 = 0x0a;
pub const MANUFACTURER_STRING: &str = "THQ Inc";
pub const PRODUCT_STRING: &str = "THQ uDraw Game Tablet for PS3";

pub const TABLET_MAX_X: i32 = 1920;
pub const TABLET_MAX_Y: i32 = 1080;
pub const INTERRUPT_EXPECTED_LATENCY_US: u64 = 6_000;
pub const CONTROL_EXPECTED_LATENCY_US: u64 = 100;

/// 27-byte input report (cpp:11..50). Packed — bitfields collapsed into
/// `btn_bits0` / `btn_bits1` so the Rust mirror has the same on-wire
/// footprint without requiring dangerous bitfield manipulation.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameTabletData {
    /// Bits: 0=square, 1=cross, 2=circle, 3=triangle, 4..7 reserved.
    pub btn_bits0: u8,
    /// Bits: 0=select, 1=start, 2..3 reserved, 4=PS, 5..7 reserved.
    pub btn_bits1: u8,
    pub dpad: u8,
    pub stick_lx: u8,
    pub stick_ly: u8,
    pub stick_rx: u8,
    pub stick_ry: u8,
    pub _pad0: [u8; 4],
    pub pen: u8,
    pub _pad1: u8,
    pub pressure: u8,
    pub _pad2: u8,
    pub pos_x_hi: u8,
    pub pos_y_hi: u8,
    pub pos_x_lo: u8,
    pub pos_y_lo: u8,
    pub accel_x: u16,
    pub accel_y: u16,
    pub accel_z: u16,
    pub unk: u16,
}

const _: () = assert!(core::mem::size_of::<GameTabletData>() == 27);

// Button bitmasks for `btn_bits0` (cpp:14..17).
pub const BTN0_SQUARE: u8 = 1 << 0;
pub const BTN0_CROSS: u8 = 1 << 1;
pub const BTN0_CIRCLE: u8 = 1 << 2;
pub const BTN0_TRIANGLE: u8 = 1 << 3;

// Button bitmasks for `btn_bits1` (cpp:20..24).
pub const BTN1_SELECT: u8 = 1 << 0;
pub const BTN1_START: u8 = 1 << 1;
pub const BTN1_PS: u8 = 1 << 4;

impl GameTabletData {
    /// Neutral defaults written in `interrupt_transfer` cpp:174..179.
    #[must_use]
    pub fn neutral() -> Self {
        Self {
            btn_bits0: 0,
            btn_bits1: 0,
            dpad: DPAD_NONE,
            stick_lx: 0x80,
            stick_ly: 0x80,
            stick_rx: 0x80,
            stick_ry: 0x80,
            _pad0: [0; 4],
            pen: 0x00,
            _pad1: 0,
            pressure: 0x72,
            _pad2: 0,
            pos_x_hi: 0x0F,
            pos_y_hi: 0x0F,
            pos_x_lo: 0xFF,
            pos_y_lo: 0xFF,
            accel_x: 0x0200,
            accel_y: 0x0200,
            accel_z: 0x0200,
            unk: 0x0200,
        }
    }
}

/// Encodes a (up, right, down, left) digital-pad state into the tablet's
/// dpad byte. Mirrors the cpp:263..280 cascade byte-for-byte; priority is
/// "diagonal pairs win over adjacent singles." Combinations that don't
/// match any clause (e.g. up+down) fall through to whatever came before,
/// mimicking the C++ "else-if" chain starting from `DPAD_NONE`.
#[must_use]
pub fn encode_dpad(up: bool, right: bool, down: bool, left: bool) -> u8 {
    if !up && !right && !down && !left {
        DPAD_NONE
    } else if up && !left && !right {
        DPAD_NORTH
    } else if up && right {
        DPAD_NE
    } else if right && !up && !down {
        DPAD_EAST
    } else if down && right {
        DPAD_SE
    } else if down && !left && !right {
        DPAD_SOUTH
    } else if down && left {
        DPAD_SW
    } else if left && !up && !down {
        DPAD_WEST
    } else if up && left {
        DPAD_NW
    } else {
        // Up+Down or Left+Right with no other friend — cpp leaves the
        // initial DPAD_None value untouched.
        DPAD_NONE
    }
}

/// Noise state for pen position — 1-bit LSB toggle (cpp:301..302, 308..309).
#[derive(Debug, Clone, Copy, Default)]
pub struct PenNoise {
    pub noise_x: u8,
    pub noise_y: u8,
}

impl PenNoise {
    pub fn toggle(&mut self) {
        self.noise_x ^= 0x1;
        self.noise_y ^= 0x1;
    }
}

/// Result of mapping a mouse cursor position into tablet coordinates
/// (cpp:303..316). `noise` is flipped as a side-effect after the
/// computation (the cpp does this pre-narrow).
#[must_use]
pub fn map_pen_position(
    mouse_x: i32,
    mouse_y: i32,
    mouse_x_max: i32,
    mouse_y_max: i32,
    noise: &mut PenNoise,
) -> (u8, u8, u8, u8) {
    // Match the cpp's early-return in the dumper: the caller already
    // gated on `x_max > 0 && y_max > 0`, so we don't re-check here — but
    // we do avoid UB from a divide by zero.
    let tablet_x = if mouse_x_max > 0 {
        (mouse_x * TABLET_MAX_X / mouse_x_max) ^ i32::from(noise.noise_x)
    } else {
        0
    };
    let tablet_y = if mouse_y_max > 0 {
        (mouse_y * TABLET_MAX_Y / mouse_y_max) ^ i32::from(noise.noise_y)
    } else {
        0
    };
    noise.toggle();

    let pos_x_hi = ((tablet_x / 0x100) & 0xFF) as u8;
    let pos_y_hi = ((tablet_y / 0x100) & 0xFF) as u8;
    let pos_x_lo = ((tablet_x % 0x100) & 0xFF) as u8;
    let pos_y_lo = ((tablet_y % 0x100) & 0xFF) as u8;
    (pos_x_hi, pos_y_hi, pos_x_lo, pos_y_lo)
}

/// Pressure mapping (cpp:312). `primary_held` ↔ CELL_MOUSE_BUTTON_1.
#[must_use]
pub const fn pressure_from_mouse(primary_held: bool) -> u8 {
    if primary_held { 0xbb } else { 0x72 }
}

/// Decodes the 4 LED bits from a SET_REPORT payload (cpp:143..147).
/// The C++ logs `buf[2] & 1/2/4/8` — we return a `[bool; 4]` in the same
/// order (LED 0..3).
#[must_use]
pub const fn decode_leds(buf2: u8) -> [bool; 4] {
    [
        buf2 & 0x01 != 0,
        buf2 & 0x02 != 0,
        buf2 & 0x04 != 0,
        buf2 & 0x08 != 0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_struct_size() {
        assert_eq!(core::mem::size_of::<GameTabletData>(), 27);
    }

    #[test]
    fn neutral_defaults_match_cpp() {
        let d = GameTabletData::neutral();
        // Read packed fields via local copies to avoid references to packed.
        let dpad = d.dpad;
        let stick_lx = d.stick_lx;
        let stick_ly = d.stick_ly;
        let stick_rx = d.stick_rx;
        let stick_ry = d.stick_ry;
        let pressure = d.pressure;
        let pen = d.pen;
        let pos_x_hi = d.pos_x_hi;
        let pos_y_hi = d.pos_y_hi;
        let pos_x_lo = d.pos_x_lo;
        let pos_y_lo = d.pos_y_lo;
        let accel_x = d.accel_x;
        let accel_y = d.accel_y;
        let accel_z = d.accel_z;
        let unk = d.unk;
        assert_eq!(dpad, DPAD_NONE);
        assert_eq!(stick_lx, 0x80);
        assert_eq!(stick_ly, 0x80);
        assert_eq!(stick_rx, 0x80);
        assert_eq!(stick_ry, 0x80);
        assert_eq!(pressure, 0x72);
        assert_eq!(pen, 0x00);
        assert_eq!(pos_x_hi, 0x0F);
        assert_eq!(pos_y_hi, 0x0F);
        assert_eq!(pos_x_lo, 0xFF);
        assert_eq!(pos_y_lo, 0xFF);
        assert_eq!(accel_x, 0x0200);
        assert_eq!(accel_y, 0x0200);
        assert_eq!(accel_z, 0x0200);
        assert_eq!(unk, 0x0200);
    }

    #[test]
    fn usb_vid_pid() {
        assert_eq!(USB_VID, 0x20d6);
        assert_eq!(USB_PID, 0xcb17);
        assert_eq!(USB_BCD_DEVICE, 0x0108);
        assert_eq!(USB_ENDPOINT_IN_ADDRESS, 0x83);
        assert_eq!(USB_ENDPOINT_OUT_ADDRESS, 0x04);
    }

    #[test]
    fn dpad_encoding_ordinals() {
        // cpp:52..63 — exact compass wheel.
        assert_eq!(DPAD_NORTH, 0);
        assert_eq!(DPAD_EAST, 2);
        assert_eq!(DPAD_SOUTH, 4);
        assert_eq!(DPAD_WEST, 6);
        assert_eq!(DPAD_NONE, 0x0F);
    }

    #[test]
    fn encode_dpad_single_directions() {
        assert_eq!(encode_dpad(true, false, false, false), DPAD_NORTH);
        assert_eq!(encode_dpad(false, true, false, false), DPAD_EAST);
        assert_eq!(encode_dpad(false, false, true, false), DPAD_SOUTH);
        assert_eq!(encode_dpad(false, false, false, true), DPAD_WEST);
    }

    #[test]
    fn encode_dpad_diagonals() {
        assert_eq!(encode_dpad(true, true, false, false), DPAD_NE);
        assert_eq!(encode_dpad(false, true, true, false), DPAD_SE);
        assert_eq!(encode_dpad(false, false, true, true), DPAD_SW);
        assert_eq!(encode_dpad(true, false, false, true), DPAD_NW);
    }

    #[test]
    fn encode_dpad_nothing_pressed_is_none() {
        assert_eq!(encode_dpad(false, false, false, false), DPAD_NONE);
    }

    #[test]
    fn encode_dpad_up_plus_down_prefers_up() {
        // The cpp cascade at :265 — `up && !left && !right` — captures
        // up+down because neither left nor right is asserted. Left+right
        // without up/down similarly falls through to `right && !up && !down`.
        assert_eq!(encode_dpad(true, false, true, false), DPAD_NORTH);
        assert_eq!(encode_dpad(false, true, false, true), DPAD_EAST);
    }

    #[test]
    fn pen_noise_toggles_lsb() {
        let mut n = PenNoise::default();
        n.toggle();
        assert_eq!(n.noise_x, 1);
        assert_eq!(n.noise_y, 1);
        n.toggle();
        assert_eq!(n.noise_x, 0);
        assert_eq!(n.noise_y, 0);
    }

    #[test]
    fn map_pen_position_center_with_noise() {
        let mut n = PenNoise { noise_x: 0, noise_y: 0 };
        let (hi_x, hi_y, lo_x, lo_y) = map_pen_position(960, 540, 1920, 1080, &mut n);
        // tablet_x = 960 * 1920 / 1920 = 960 = 0x03C0 → hi=0x03, lo=0xC0.
        assert_eq!(hi_x, 0x03);
        assert_eq!(lo_x, 0xC0);
        // tablet_y = 540 * 1080 / 1080 = 540 = 0x021C → hi=0x02, lo=0x1C.
        assert_eq!(hi_y, 0x02);
        assert_eq!(lo_y, 0x1C);
        // noise toggled.
        assert_eq!(n.noise_x, 1);
        assert_eq!(n.noise_y, 1);
    }

    #[test]
    fn map_pen_position_noise_xor_flips_lsb() {
        let mut n = PenNoise { noise_x: 1, noise_y: 1 };
        // Mapping 0 → 0 ^ 1 = 1 → hi=0, lo=1.
        let (hi_x, hi_y, lo_x, lo_y) = map_pen_position(0, 0, 1920, 1080, &mut n);
        assert_eq!(hi_x, 0x00);
        assert_eq!(lo_x, 0x01);
        assert_eq!(hi_y, 0x00);
        assert_eq!(lo_y, 0x01);
    }

    #[test]
    fn pressure_mouse_primary() {
        assert_eq!(pressure_from_mouse(true), 0xbb);
        assert_eq!(pressure_from_mouse(false), 0x72);
    }

    #[test]
    fn decode_leds_bits() {
        assert_eq!(decode_leds(0b0000), [false, false, false, false]);
        assert_eq!(decode_leds(0b0101), [true, false, true, false]);
        assert_eq!(decode_leds(0b1111), [true; 4]);
        assert_eq!(decode_leds(0xF0), [false; 4]); // upper nibble ignored
    }

    #[test]
    fn button_bitmasks_match_cpp_fields() {
        // cpp:14..17 & cpp:20..23 — bit ordering.
        assert_eq!(BTN0_SQUARE, 0x01);
        assert_eq!(BTN0_TRIANGLE, 0x08);
        assert_eq!(BTN1_SELECT, 0x01);
        assert_eq!(BTN1_START, 0x02);
        assert_eq!(BTN1_PS, 0x10);
    }
}
