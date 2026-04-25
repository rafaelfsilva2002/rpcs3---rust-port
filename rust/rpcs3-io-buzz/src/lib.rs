//! `rpcs3-io-buzz` — Rust port of `rpcs3/Emu/Io/Buzz.cpp`.
//!
//! Emulates the Logitech Buzz! buzzer USB controller. The C++ class
//! registers a synthetic USB device and serves interrupt transfers that
//! pack button presses from up to 4 Buzz buzzers into a 5-byte report.
//! Full USB stack / Qt pad-input wiring is out of scope; this crate
//! freezes the parts that a driver must stay compatible with:
//!
//! - The `buzz_btn` enum order (Red/Yellow/Green/Orange/Blue + count).
//! - USB device/config descriptor byte layout (VID/PID/bcd values, endpoint
//!   address, packet size) from cpp:36..79.
//! - The hardcoded interrupt-transfer preamble: `[0x7f, 0x7f, 0x00, 0x00,
//!   0xf0]` (cpp:155..159).
//! - The button-bit packing formula `buf[2 + (btn_idx + 5*player_idx) / 8]
//!   |= 1 << ((btn_idx + 5*player_idx) % 8)` (cpp:187..203), which packs
//!   up to 4 players × 5 buttons = 20 bits into a 3-byte tail.
//! - `make_instance(controller_index)` dispatch: 0 → players 0..3, else →
//!   players 4..6 (cpp:89..99). The PS3 supports 7 pads total so player 8
//!   never exists.
//! - The expected interrupt timing (6 ms) and transfer count (5 bytes).
use core::mem::size_of;

/// Button enum in cpp declaration order (`buzz_config.h:5..14`).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuzzBtn {
    Red = 0,
    Yellow = 1,
    Green = 2,
    Orange = 3,
    Blue = 4,
    Count = 5,
}

impl BuzzBtn {
    #[must_use]
    pub const fn index(self) -> Option<u32> {
        match self {
            Self::Red => Some(0),
            Self::Yellow => Some(1),
            Self::Green => Some(2),
            Self::Orange => Some(3),
            Self::Blue => Some(4),
            Self::Count => None,
        }
    }
}

// USB descriptor constants from cpp:36..79. All values are byte-exact — a
// host that enumerates the device must see these to stay compatible.
pub const USB_VID: u16 = 0x054c;
pub const USB_PID: u16 = 0x0002;
pub const USB_BCD_DEVICE: u16 = 0x05a1;
pub const USB_BCD_VERSION: u16 = 0x0200;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x08;
pub const USB_NUM_CONFIGURATIONS: u8 = 0x01;
pub const USB_CONFIG_W_TOTAL_LENGTH: u16 = 0x0022;
pub const USB_CONFIG_NUM_INTERFACES: u8 = 0x01;
pub const USB_CONFIG_VALUE: u8 = 0x01;
pub const USB_CONFIG_BM_ATTRIBUTES: u8 = 0x80;
pub const USB_CONFIG_MAX_POWER: u8 = 0x32;
pub const USB_INTERFACE_CLASS_HID: u8 = 0x03;
pub const USB_HID_BCD: u16 = 0x0111;
pub const USB_HID_COUNTRY_CODE: u8 = 0x33;
pub const USB_HID_DESCRIPTOR_TYPE: u8 = 0x22;
pub const USB_HID_DESCRIPTOR_LENGTH: u16 = 0x004e;
pub const USB_ENDPOINT_ADDRESS: u8 = 0x81;
pub const USB_ENDPOINT_BM_ATTRIBUTES: u8 = 0x03;
pub const USB_ENDPOINT_W_MAX_PACKET_SIZE: u16 = 0x0008;
pub const USB_ENDPOINT_B_INTERVAL: u8 = 0x0a;

pub const MANUFACTURER_STRING: &str = "Logitech";
pub const PRODUCT_STRING: &str = "Logitech Buzz(tm) Controller V1";

/// Every interrupt transfer starts with this 5-byte preamble before any
/// button bits are ORed in (cpp:155..159).
pub const INTERRUPT_PREAMBLE: [u8; 5] = [0x7f, 0x7f, 0x00, 0x00, 0xf0];

/// Number of bytes reported by an interrupt transfer (`expected_count = 5`
/// in cpp:147).
pub const INTERRUPT_TRANSFER_BYTES: u32 = 5;

/// Expected latency of an interrupt transfer (`6 ms`, cpp:149..150).
pub const INTERRUPT_EXPECTED_LATENCY_US: u64 = 6_000;

/// Expected latency of a control transfer (`nearly instant`, cpp:112).
pub const CONTROL_EXPECTED_LATENCY_US: u64 = 100;

/// Number of buttons per Buzz buzzer (cpp:190..202, one bit per btn slot).
pub const BUTTONS_PER_BUZZER: u32 = 5;

/// Max players supported by the emulated device — the PS3 has 7 pad slots
/// total (cpp:96..98).
pub const MAX_PLAYERS: u32 = 7;

/// `make_instance(controller_index)` dispatch table (cpp:89..99).
/// - `controller_index == 0` → buzzers 0..3
/// - otherwise → buzzers 4..6 (player 8 isn't supported)
///
/// Returns the inclusive `[first, last]` player range, or `None` for
/// invalid indexes (`>1` — the cpp falls through to the 4..6 branch
/// regardless, which we mirror).
#[must_use]
pub const fn controller_range(controller_index: u32) -> (u32, u32) {
    if controller_index == 0 {
        (0, 3)
    } else {
        (4, 6)
    }
}

/// Returns the minimum interrupt-transfer buffer size required for a given
/// `last_controller` index — matches the `max_index` formula at cpp:143.
///
/// The formula is `max_index = 2 + (4 + 5 * last_controller) / 8`, and the
/// cpp `ensure(buf_size > max_index)` means the buffer has to be at least
/// `max_index + 1` bytes.
#[must_use]
pub const fn min_interrupt_buf_size(last_controller: u32) -> u32 {
    2 + (4 + BUTTONS_PER_BUZZER * last_controller) / 8 + 1
}

/// Result of `pack_button_press` — either the bit was written successfully
/// or the buffer was too small to hold the target byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackOutcome {
    Written { byte_offset: usize, bit: u8 },
    BufferTooSmall { needed_offset: usize, buf_len: usize },
    InvalidButton,
}

/// Packs one `buzz_btn` press from a given player into the interrupt
/// buffer, OR-ing a single bit. Mirrors cpp:187..203 byte-for-byte:
///
/// ```text
/// idx = btn_idx + 5 * player_slot
/// buf[2 + idx/8] |= 1 << (idx % 8)
/// ```
///
/// `player_slot` is the zero-based slot inside the current controller's
/// `[first..=last]` range — i.e. `i - m_first_controller` in the cpp loop.
pub fn pack_button_press(buf: &mut [u8], btn: BuzzBtn, player_slot: u32) -> PackOutcome {
    let btn_idx = match btn.index() {
        Some(x) => x,
        None => return PackOutcome::InvalidButton,
    };
    let idx = btn_idx + BUTTONS_PER_BUZZER * player_slot;
    let byte_offset = 2 + (idx / 8) as usize;
    let bit = 1u8 << (idx % 8);
    if byte_offset >= buf.len() {
        return PackOutcome::BufferTooSmall { needed_offset: byte_offset, buf_len: buf.len() };
    }
    buf[byte_offset] |= bit;
    PackOutcome::Written { byte_offset, bit }
}

/// Fill `buf` with the zeroed-and-preambled state a fresh interrupt transfer
/// must start from (cpp:152..159). Returns how many bytes were written.
pub fn reset_interrupt_buffer(buf: &mut [u8]) -> usize {
    for b in buf.iter_mut() {
        *b = 0;
    }
    let n = INTERRUPT_PREAMBLE.len().min(buf.len());
    buf[..n].copy_from_slice(&INTERRUPT_PREAMBLE[..n]);
    n
}

/// Minimal `#[repr(C)]` mirror of the `UsbDeviceDescriptor` fields written
/// at cpp:37..49. Not the full libusb struct — just the values Buzz pins.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuzzDeviceDescriptorValues {
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size_0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}

impl BuzzDeviceDescriptorValues {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            bcd_usb: USB_BCD_VERSION,
            b_device_class: 0,
            b_device_sub_class: 0,
            b_device_protocol: 0,
            b_max_packet_size_0: USB_MAX_PACKET_SIZE_0,
            id_vendor: USB_VID,
            id_product: USB_PID,
            bcd_device: USB_BCD_DEVICE,
            i_manufacturer: 0x02,
            i_product: 0x01,
            i_serial_number: 0x00,
            b_num_configurations: USB_NUM_CONFIGURATIONS,
        }
    }
}

impl Default for BuzzDeviceDescriptorValues {
    fn default() -> Self {
        Self::new()
    }
}

/// 16 bytes of descriptor values (without the 2-byte bLength/bDescriptorType
/// header that lives on the wrapper in cpp). The full USB descriptor on the
/// wire is 18 bytes, but the values struct itself is 16.
const _: () = assert!(size_of::<BuzzDeviceDescriptorValues>() == 16);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn btn_enum_order_matches_cpp() {
        assert_eq!(BuzzBtn::Red as u32, 0);
        assert_eq!(BuzzBtn::Yellow as u32, 1);
        assert_eq!(BuzzBtn::Green as u32, 2);
        assert_eq!(BuzzBtn::Orange as u32, 3);
        assert_eq!(BuzzBtn::Blue as u32, 4);
        assert_eq!(BuzzBtn::Count as u32, 5);
        assert_eq!(BuzzBtn::Count.index(), None);
    }

    #[test]
    fn usb_vid_pid_and_bcd_values() {
        assert_eq!(USB_VID, 0x054c);
        assert_eq!(USB_PID, 0x0002);
        assert_eq!(USB_BCD_DEVICE, 0x05a1);
        assert_eq!(USB_BCD_VERSION, 0x0200);
    }

    #[test]
    fn interrupt_preamble_exact() {
        assert_eq!(INTERRUPT_PREAMBLE, [0x7f, 0x7f, 0x00, 0x00, 0xf0]);
        assert_eq!(INTERRUPT_TRANSFER_BYTES, 5);
        assert_eq!(INTERRUPT_EXPECTED_LATENCY_US, 6_000);
    }

    #[test]
    fn reset_interrupt_buffer_zeros_then_preamble() {
        let mut buf = [0xffu8; 8];
        let n = reset_interrupt_buffer(&mut buf);
        assert_eq!(n, 5);
        assert_eq!(&buf[..5], &INTERRUPT_PREAMBLE);
        assert_eq!(&buf[5..], &[0, 0, 0]);
    }

    #[test]
    fn controller_range_dispatch() {
        assert_eq!(controller_range(0), (0, 3));
        assert_eq!(controller_range(1), (4, 6));
        assert_eq!(controller_range(99), (4, 6));
    }

    #[test]
    fn min_interrupt_buf_size_cpp_formula() {
        // cpp:143 max_index = 2 + (4 + 5 * last_controller) / 8,
        // ensure(buf_size > max_index) → buf must be >= max_index + 1.
        // last_controller=3 → max_index = 2 + 19/8 = 2 + 2 = 4 → min size 5
        assert_eq!(min_interrupt_buf_size(3), 5);
        // last_controller=6 → max_index = 2 + 34/8 = 2 + 4 = 6 → min size 7
        assert_eq!(min_interrupt_buf_size(6), 7);
    }

    #[test]
    fn pack_button_first_player_red_hits_byte2_bit0() {
        let mut buf = [0u8; 7];
        let out = pack_button_press(&mut buf, BuzzBtn::Red, 0);
        assert_eq!(out, PackOutcome::Written { byte_offset: 2, bit: 1 });
        assert_eq!(buf[2], 0x01);
    }

    #[test]
    fn pack_button_first_player_blue_hits_byte2_bit4() {
        let mut buf = [0u8; 7];
        let out = pack_button_press(&mut buf, BuzzBtn::Blue, 0);
        assert_eq!(out, PackOutcome::Written { byte_offset: 2, bit: 0x10 });
    }

    #[test]
    fn pack_button_second_player_red_hits_byte2_bit5() {
        // idx = 0 + 5 * 1 = 5 → byte 2, bit 5.
        let mut buf = [0u8; 7];
        pack_button_press(&mut buf, BuzzBtn::Red, 1).to_written();
        assert_eq!(buf[2], 0x20);
    }

    #[test]
    fn pack_button_third_player_green_wraps_to_byte3() {
        // idx = 2 + 5 * 2 = 12 → byte 2 + 12/8 = 3, bit 12%8 = 4.
        let mut buf = [0u8; 7];
        pack_button_press(&mut buf, BuzzBtn::Green, 2).to_written();
        assert_eq!(buf[3], 0x10);
    }

    #[test]
    fn pack_button_fourth_player_blue_hits_byte4() {
        // idx = 4 + 5 * 3 = 19 → byte 2 + 19/8 = 4, bit 19%8 = 3.
        let mut buf = [0u8; 7];
        pack_button_press(&mut buf, BuzzBtn::Blue, 3).to_written();
        assert_eq!(buf[4], 0x08);
    }

    #[test]
    fn pack_button_small_buffer_rejected() {
        let mut buf = [0u8; 4];
        let out = pack_button_press(&mut buf, BuzzBtn::Blue, 3);
        assert_eq!(
            out,
            PackOutcome::BufferTooSmall { needed_offset: 4, buf_len: 4 }
        );
    }

    #[test]
    fn pack_button_count_is_invalid() {
        let mut buf = [0u8; 8];
        let out = pack_button_press(&mut buf, BuzzBtn::Count, 0);
        assert_eq!(out, PackOutcome::InvalidButton);
    }

    #[test]
    fn device_descriptor_struct_size_and_values() {
        let d = BuzzDeviceDescriptorValues::new();
        assert_eq!(d.id_vendor, USB_VID);
        assert_eq!(d.id_product, USB_PID);
        assert_eq!(d.bcd_device, 0x05a1);
        assert_eq!(d.b_num_configurations, 1);
        // 16 bytes — just the values; the 2-byte bLength/bDescriptorType
        // header lives on the wrapper in cpp.
        assert_eq!(size_of::<BuzzDeviceDescriptorValues>(), 16);
    }

    impl PackOutcome {
        fn to_written(self) {
            match self {
                PackOutcome::Written { .. } => {}
                other => panic!("expected Written, got {other:?}"),
            }
        }
    }
}
