//! `rpcs3-io-ghltar` — Rust port of `rpcs3/Emu/Io/GHLtar.cpp`.
//!
//! Emulates the Guitar Hero Live (GHL) guitar controller. What we freeze:
//!
//! - `ghltar_btn` enum order (16 variants including `count`).
//! - USB device / config descriptor constants: VID=0x12BA (Sony Licensed),
//!   PID=0x074B, HID interface class, two interrupt endpoints (0x81 IN,
//!   0x01 OUT).
//! - Interrupt-transfer baseline: 27 bytes minimum, 1 ms latency (cpp:94,
//!   101 — overridden from the usual 6 ms so input feels tight).
//! - The 27-byte report preamble (cpp:105..144): neutral state with
//!   `buf[2]=0x0F` (D-pad none), `buf[3]=0x80`, `buf[4]=0x80` (strummer
//!   idle), `buf[5/6/19]=0x80`, plus the mysterious always-on bytes at
//!   22/24/26.
//! - Fret hex masks (cpp:107..112): `W1=0x01`, `B1=0x02`, `B2=0x04`,
//!   `B3=0x08`, `W2=0x10`, `W3=0x20` — packed via `|=` into `buf[0]`.
//! - Button masks (cpp:115..119): `HeroPower=0x01`, `Start=0x02`,
//!   `GHTV=0x04`, `Sync=0x10`.
//! - Analog mapping cpp:207..216 — whammy inverts `~value + 1`, tilt writes
//!   `value` to `buf[19]` and snaps `buf[5]` to 0x00 or 0xFF at the
//!   `<=0x10` / `>=0xF0` thresholds.
//!
//! The pad-thread handler + full USB stack stay out; this crate gives an
//! engine any frontend can drive without depending on those layers.

pub const MIN_BUF_SIZE: usize = 27;

/// Transfer latency in microseconds (cpp:101 — 1ms for snappier input).
pub const INTERRUPT_EXPECTED_LATENCY_US: u64 = 1_000;

// USB descriptor constants (cpp:44..49).
pub const USB_VID: u16 = 0x12BA;
pub const USB_PID: u16 = 0x074B;
pub const USB_BCD_DEVICE: u16 = 0x0100;
pub const USB_BCD_USB: u16 = 0x0200;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x20;
pub const USB_CONFIG_W_TOTAL_LENGTH: u16 = 0x0029;
pub const USB_CONFIG_VALUE: u8 = 0x01;
pub const USB_CONFIG_BM_ATTRIBUTES: u8 = 0x80;
pub const USB_CONFIG_MAX_POWER: u8 = 0x96;
pub const USB_INTERFACE_NUM_ENDPOINTS: u8 = 0x02;
pub const USB_INTERFACE_CLASS_HID: u8 = 0x03;
pub const USB_HID_BCD: u16 = 0x0111;
pub const USB_HID_DESCRIPTOR_LENGTH: u16 = 0x001d;
pub const USB_ENDPOINT_IN_ADDRESS: u8 = 0x81;
pub const USB_ENDPOINT_OUT_ADDRESS: u8 = 0x01;
pub const USB_ENDPOINT_BM_ATTRIBUTES: u8 = 0x03;
pub const USB_ENDPOINT_W_MAX_PACKET_SIZE: u16 = 0x0020;
pub const USB_ENDPOINT_B_INTERVAL: u8 = 0x01;

/// Complete set of buttons a GHL guitar exposes. Order matches cpp enum.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GhltarBtn {
    W1 = 0,
    W2 = 1,
    W3 = 2,
    B1 = 3,
    B2 = 4,
    B3 = 5,
    Start = 6,
    HeroPower = 7,
    Ghtv = 8,
    StrumDown = 9,
    StrumUp = 10,
    DpadLeft = 11,
    DpadRight = 12,
    Whammy = 13,
    Tilt = 14,
    Count = 15,
}

// Fret hex masks (cpp:107..112).
pub const FRET_W1: u8 = 0x01;
pub const FRET_B1: u8 = 0x02;
pub const FRET_B2: u8 = 0x04;
pub const FRET_B3: u8 = 0x08;
pub const FRET_W2: u8 = 0x10;
pub const FRET_W3: u8 = 0x20;

// Button masks (cpp:115..119).
pub const BUTTON_HERO_POWER: u8 = 0x01;
pub const BUTTON_START: u8 = 0x02;
pub const BUTTON_GHTV: u8 = 0x04;
pub const BUTTON_SYNC: u8 = 0x10;

// D-pad values (cpp:122..131).
pub const DPAD_UP: u8 = 0x00;
pub const DPAD_UP_LEFT: u8 = 0x01;
pub const DPAD_LEFT: u8 = 0x02;
pub const DPAD_LEFT_DOWN: u8 = 0x03;
pub const DPAD_DOWN: u8 = 0x04;
pub const DPAD_DOWN_RIGHT: u8 = 0x05;
pub const DPAD_RIGHT: u8 = 0x06;
pub const DPAD_UP_RIGHT: u8 = 0x07;
pub const DPAD_NONE: u8 = 0x0F;

/// Strummer idle/down/up values (cpp:133, 187, 190).
pub const STRUMMER_IDLE: u8 = 0x80;
pub const STRUMMER_DOWN: u8 = 0xFF;
pub const STRUMMER_UP: u8 = 0x00;

/// Tilt thresholds (cpp:212..215) that force `buf[5]` to 0x00 / 0xFF.
pub const TILT_HIGH_THRESHOLD: u8 = 0xF0;
pub const TILT_LOW_THRESHOLD: u8 = 0x10;

/// Fill `buf` with the neutral report state emitted before any input is
/// applied (cpp:103..144). `buf` must be at least `MIN_BUF_SIZE` bytes.
pub fn reset_report(buf: &mut [u8]) {
    assert!(buf.len() >= MIN_BUF_SIZE, "GHL report requires >= {} bytes", MIN_BUF_SIZE);
    for b in buf.iter_mut() {
        *b = 0;
    }
    buf[0] = 0x00;
    buf[1] = 0x00;
    buf[2] = DPAD_NONE;
    buf[3] = 0x80;
    buf[4] = STRUMMER_IDLE;
    buf[5] = 0x80;
    buf[6] = 0x80;
    buf[19] = 0x80;
    buf[22] = 0x01;
    buf[24] = 0x02;
    buf[26] = 0x02;
}

/// Apply one pressed button to the report, mirroring cpp:161..220. `value`
/// only matters for `Whammy` / `Tilt`; it's ignored for digital buttons.
///
/// Returns whether the button had any visible effect (only `Count` is a
/// no-op by construction).
pub fn apply_button(buf: &mut [u8], btn: GhltarBtn, value: u16) -> bool {
    assert!(buf.len() >= MIN_BUF_SIZE, "GHL report requires >= {} bytes", MIN_BUF_SIZE);
    match btn {
        GhltarBtn::W1 => {
            buf[0] = buf[0].wrapping_add(FRET_W1);
            true
        }
        GhltarBtn::B1 => {
            buf[0] = buf[0].wrapping_add(FRET_B1);
            true
        }
        GhltarBtn::B2 => {
            buf[0] = buf[0].wrapping_add(FRET_B2);
            true
        }
        GhltarBtn::B3 => {
            buf[0] = buf[0].wrapping_add(FRET_B3);
            true
        }
        GhltarBtn::W2 => {
            buf[0] = buf[0].wrapping_add(FRET_W2);
            true
        }
        GhltarBtn::W3 => {
            buf[0] = buf[0].wrapping_add(FRET_W3);
            true
        }
        GhltarBtn::StrumDown => {
            buf[4] = STRUMMER_DOWN;
            true
        }
        GhltarBtn::StrumUp => {
            buf[4] = STRUMMER_UP;
            true
        }
        GhltarBtn::DpadLeft => {
            buf[2] = DPAD_LEFT;
            true
        }
        GhltarBtn::DpadRight => {
            buf[2] = DPAD_RIGHT;
            true
        }
        GhltarBtn::Start => {
            buf[1] = buf[1].wrapping_add(BUTTON_START);
            true
        }
        GhltarBtn::HeroPower => {
            buf[1] = buf[1].wrapping_add(BUTTON_HERO_POWER);
            true
        }
        GhltarBtn::Ghtv => {
            buf[1] = buf[1].wrapping_add(BUTTON_GHTV);
            true
        }
        GhltarBtn::Whammy => {
            // cpp:208 — `~(value) + 1` computed in u16 then truncated to u8.
            // That's a 16-bit negation; preserve the low byte.
            let neg = (!value).wrapping_add(1);
            buf[6] = neg as u8;
            true
        }
        GhltarBtn::Tilt => {
            buf[19] = value as u8;
            if buf[19] >= TILT_HIGH_THRESHOLD {
                buf[5] = 0xFF;
            } else if buf[19] <= TILT_LOW_THRESHOLD {
                buf[5] = 0x00;
            }
            true
        }
        GhltarBtn::Count => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btn_enum_order_matches_cpp() {
        assert_eq!(GhltarBtn::W1 as u32, 0);
        assert_eq!(GhltarBtn::Count as u32, 15);
        // spot-check the middle
        assert_eq!(GhltarBtn::Ghtv as u32, 8);
        assert_eq!(GhltarBtn::Tilt as u32, 14);
    }

    #[test]
    fn usb_vid_pid_match() {
        assert_eq!(USB_VID, 0x12BA);
        assert_eq!(USB_PID, 0x074B);
        assert_eq!(USB_BCD_DEVICE, 0x0100);
        assert_eq!(USB_ENDPOINT_IN_ADDRESS, 0x81);
        assert_eq!(USB_ENDPOINT_OUT_ADDRESS, 0x01);
    }

    #[test]
    fn reset_report_preamble_bytes() {
        let mut buf = [0xffu8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        assert_eq!(buf[0], 0x00);
        assert_eq!(buf[1], 0x00);
        assert_eq!(buf[2], 0x0F);
        assert_eq!(buf[3], 0x80);
        assert_eq!(buf[4], 0x80);
        assert_eq!(buf[5], 0x80);
        assert_eq!(buf[6], 0x80);
        assert_eq!(buf[19], 0x80);
        assert_eq!(buf[22], 0x01);
        assert_eq!(buf[24], 0x02);
        assert_eq!(buf[26], 0x02);
        // Untouched gap bytes should be zeroed.
        assert_eq!(buf[7], 0x00);
        assert_eq!(buf[21], 0x00);
    }

    #[test]
    fn fret_masks_or_into_buf0() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        apply_button(&mut buf, GhltarBtn::W1, 0);
        apply_button(&mut buf, GhltarBtn::B2, 0);
        apply_button(&mut buf, GhltarBtn::W3, 0);
        assert_eq!(buf[0], FRET_W1 + FRET_B2 + FRET_W3, "0x25");
    }

    #[test]
    fn button_masks_cumulative() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        apply_button(&mut buf, GhltarBtn::Start, 0);
        apply_button(&mut buf, GhltarBtn::Ghtv, 0);
        apply_button(&mut buf, GhltarBtn::HeroPower, 0);
        assert_eq!(buf[1], BUTTON_START + BUTTON_GHTV + BUTTON_HERO_POWER);
    }

    #[test]
    fn strum_overrides_idle() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        apply_button(&mut buf, GhltarBtn::StrumDown, 0);
        assert_eq!(buf[4], STRUMMER_DOWN);
        apply_button(&mut buf, GhltarBtn::StrumUp, 0);
        assert_eq!(buf[4], STRUMMER_UP);
    }

    #[test]
    fn dpad_left_and_right() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        apply_button(&mut buf, GhltarBtn::DpadLeft, 0);
        assert_eq!(buf[2], DPAD_LEFT);
        apply_button(&mut buf, GhltarBtn::DpadRight, 0);
        assert_eq!(buf[2], DPAD_RIGHT);
    }

    #[test]
    fn whammy_two_complement_truncation() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        // cpp: buf[6] = ~value + 1 (in u16 then narrowed)
        // value=0 → ~0+1 = 0 (u16 wraps from 0xFFFF+1=0) → 0
        apply_button(&mut buf, GhltarBtn::Whammy, 0);
        assert_eq!(buf[6], 0x00);
        // value=1 → ~1+1 = 0xFFFE+1=0xFFFF → low byte 0xFF
        reset_report(&mut buf);
        apply_button(&mut buf, GhltarBtn::Whammy, 1);
        assert_eq!(buf[6], 0xFF);
        // value=0x80 → ~0x80+1 = 0xFF7F+1=0xFF80 → low byte 0x80
        reset_report(&mut buf);
        apply_button(&mut buf, GhltarBtn::Whammy, 0x80);
        assert_eq!(buf[6], 0x80);
    }

    #[test]
    fn tilt_writes_and_thresholds() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        // Mid-range tilt: buf[19] gets the value, buf[5] unchanged.
        apply_button(&mut buf, GhltarBtn::Tilt, 0x40);
        assert_eq!(buf[19], 0x40);
        assert_eq!(buf[5], 0x80, "untouched between thresholds");
        // High tilt: buf[5] → 0xFF.
        apply_button(&mut buf, GhltarBtn::Tilt, 0xF5);
        assert_eq!(buf[19], 0xF5);
        assert_eq!(buf[5], 0xFF);
        // Low tilt: buf[5] → 0x00.
        apply_button(&mut buf, GhltarBtn::Tilt, 0x05);
        assert_eq!(buf[19], 0x05);
        assert_eq!(buf[5], 0x00);
    }

    #[test]
    fn count_is_noop() {
        let mut buf = [0u8; MIN_BUF_SIZE];
        reset_report(&mut buf);
        let prev = buf;
        let acted = apply_button(&mut buf, GhltarBtn::Count, 0);
        assert!(!acted);
        assert_eq!(buf, prev);
    }

    #[test]
    fn interrupt_latency_is_1ms() {
        assert_eq!(INTERRUPT_EXPECTED_LATENCY_US, 1_000);
        assert_eq!(MIN_BUF_SIZE, 27);
    }
}
