//! `rpcs3-io-usio` — Rust port of `rpcs3/Emu/Io/usio.cpp`.
//!
//! v406 USIO — Namco/Bandai arcade I/O board used in PS3-based arcade
//! conversions (most notably *Taiko no Tatsujin* and *Tekken* cabinets).
//! The PS3 talks to it over bulk USB endpoints and a vendor-specific
//! control protocol with **channels**: channel 0 = I/O + card reader,
//! channel 1 = firmware update (no-op in emulation), channel ≥ 2 = SRAM
//! page access (0x10 pages × 64 KiB each).
//!
//! Frozen here:
//!
//! - USB descriptor constants (cpp:66..120): VID=0x0b9a / PID=0x0910,
//!   three endpoints (0x01 bulk OUT, 0x82 bulk IN, 0x83 interrupt IN).
//! - `UsioBtn` enum (cpp:14..36, 18 variants including `count`).
//! - SRAM layout: `page_size = 0x10000`, `page_count = 0x10` (cpp:56..57).
//! - `C_HIT = 0x1800` — Taiko drum-hit magic word (cpp:205).
//! - Taiko digital-input bitmask table (cpp:234..271).
//! - Tekken digital-input bitmask table (cpp:321..397) including the
//!   per-player `shift = (player % 2) * 24`.
//! - Channel 0 register IDs (cpp:432..463, 496..521) with the ClearSram
//!   magic value `0x6666`.
//! - Canned responses for `GetBuffer` (0x0000) and `CardReaderCheck`
//!   (0x0080) — byte-exact arrays.
//! - Hopper register math: `reg 0x48/58/68/78` → `(reg - 0x48) / 0x10`.

/// Backup SRAM layout (cpp:56..57).
pub const PAGE_SIZE: usize = 0x10000;
pub const PAGE_COUNT: usize = 0x10;
pub const BACKUP_SIZE: usize = PAGE_SIZE * PAGE_COUNT;

// USB descriptor constants (cpp:66..120).
pub const USB_VID: u16 = 0x0b9a;
pub const USB_PID: u16 = 0x0910;
pub const USB_BCD_DEVICE: u16 = 0x0910;
pub const USB_BCD_USB: u16 = 0x0110;
pub const USB_DEVICE_CLASS: u8 = 0xff; // vendor-specific
pub const USB_DEVICE_PROTOCOL: u8 = 0xff;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x08;
pub const USB_CONFIG_W_TOTAL_LENGTH: u16 = 39;
pub const USB_CONFIG_BM_ATTRIBUTES: u8 = 0xc0;
pub const USB_CONFIG_MAX_POWER: u8 = 0x32;
pub const USB_INTERFACE_NUM_ENDPOINTS: u8 = 0x03;
pub const USB_ENDPOINT_BULK_OUT: u8 = 0x01;
pub const USB_ENDPOINT_BULK_IN: u8 = 0x82;
pub const USB_ENDPOINT_INTERRUPT_IN: u8 = 0x83;
pub const USB_ENDPOINT_BULK_W_MAX_PACKET_SIZE: u16 = 0x0040;
pub const USB_ENDPOINT_INTERRUPT_W_MAX_PACKET_SIZE: u16 = 0x0008;

/// Taiko drum hit magic value (cpp:205).
pub const C_HIT: u16 = 0x1800;

/// Tekken per-player bit-shift step (cpp:294).
pub const TEKKEN_PER_PLAYER_SHIFT: u32 = 24;

/// USIO button enum matching cpp:14..36 (18 variants).
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsioBtn {
    Test = 0,
    Coin = 1,
    Service = 2,
    Enter = 3,
    Up = 4,
    Down = 5,
    Left = 6,
    Right = 7,
    TaikoHitSideLeft = 8,
    TaikoHitSideRight = 9,
    TaikoHitCenterLeft = 10,
    TaikoHitCenterRight = 11,
    TekkenButton1 = 12,
    TekkenButton2 = 13,
    TekkenButton3 = 14,
    TekkenButton4 = 15,
    TekkenButton5 = 16,
    Count = 17,
}

/// Per-board I/O status (cpp:211 `m_io_status[0]`).
#[derive(Debug, Default, Clone, Copy)]
pub struct IoStatus {
    pub coin_counter: u16,
    pub test_on: bool,
    pub test_key_pressed: bool,
    pub coin_key_pressed: bool,
}

// ---------------------------------------------------------------------
// Taiko digital-input bitmasks (cpp:234..271, player 0 only).
// ---------------------------------------------------------------------
pub const TAIKO_SERVICE: u16 = 0x4000;
pub const TAIKO_ENTER: u16 = 0x0200;
pub const TAIKO_UP: u16 = 0x2000;
pub const TAIKO_DOWN: u16 = 0x1000;
pub const TAIKO_TEST_ON: u16 = 0x0080;

/// Taiko hit offsets inside the 0x60-byte input buffer (cpp:250..262).
pub const TAIKO_INPUT_BUF_SIZE: usize = 0x60;
pub const TAIKO_HIT_SIDE_LEFT_OFFSET: usize = 32;
pub const TAIKO_HIT_CENTER_LEFT_OFFSET: usize = 34;
pub const TAIKO_HIT_CENTER_RIGHT_OFFSET: usize = 36;
pub const TAIKO_HIT_SIDE_RIGHT_OFFSET: usize = 38;
/// Inter-player stride (cpp:210 `offset = player * 8`).
pub const TAIKO_PER_PLAYER_STRIDE: usize = 8;

/// Encode a Taiko drum hit into the 0x60-byte input buffer. `player` must
/// be 0 or 1 (2P setup). Returns whether the write fit.
pub fn encode_taiko_hit(
    buf: &mut [u8],
    player: u8,
    btn: UsioBtn,
) -> bool {
    assert!(buf.len() >= TAIKO_INPUT_BUF_SIZE);
    let base = match btn {
        UsioBtn::TaikoHitSideLeft => TAIKO_HIT_SIDE_LEFT_OFFSET,
        UsioBtn::TaikoHitCenterLeft => TAIKO_HIT_CENTER_LEFT_OFFSET,
        UsioBtn::TaikoHitCenterRight => TAIKO_HIT_CENTER_RIGHT_OFFSET,
        UsioBtn::TaikoHitSideRight => TAIKO_HIT_SIDE_RIGHT_OFFSET,
        _ => return false,
    };
    let offset = base + usize::from(player) * TAIKO_PER_PLAYER_STRIDE;
    if offset + 2 > buf.len() {
        return false;
    }
    buf[offset..offset + 2].copy_from_slice(&C_HIT.to_le_bytes());
    true
}

/// Write the 16-bit digital input + 16-bit coin counter at the fixed
/// offsets (cpp:277..278).
pub fn write_taiko_header(buf: &mut [u8], digital_input: u16, coin_counter: u16) {
    assert!(buf.len() >= 18);
    buf[0..2].copy_from_slice(&digital_input.to_le_bytes());
    buf[16..18].copy_from_slice(&coin_counter.to_le_bytes());
}

// ---------------------------------------------------------------------
// Tekken digital-input bitmasks (cpp:321..397).
// ---------------------------------------------------------------------
pub const TEKKEN_SERVICE: u64 = 0x0000_0000_0000_4000;
pub const TEKKEN_ENTER_BASE: u64 = 0x0080_0000;
pub const TEKKEN_UP_BASE: u64 = 0x0020_0000;
pub const TEKKEN_DOWN_BASE: u64 = 0x0010_0000;
pub const TEKKEN_LEFT_BASE: u64 = 0x0008_0000;
pub const TEKKEN_RIGHT_BASE: u64 = 0x0004_0000;
pub const TEKKEN_BUTTON1_BASE: u64 = 0x0002_0000;
pub const TEKKEN_BUTTON2_BASE: u64 = 0x0001_0000;
pub const TEKKEN_BUTTON3_BASE: u64 = 0x4000_0000;
pub const TEKKEN_BUTTON4_BASE: u64 = 0x2000_0000;
pub const TEKKEN_BUTTON5_BASE: u64 = 0x8000_0000;

/// Lightweight mirror input written on player 0 (cpp:328..397 `digital_input_lm`).
pub const TEKKEN_LM_ENTER: u16 = 0x0800;
pub const TEKKEN_LM_UP: u16 = 0x0200;
pub const TEKKEN_LM_DOWN: u16 = 0x0400;
pub const TEKKEN_LM_LEFT: u16 = 0x2000;
pub const TEKKEN_LM_RIGHT: u16 = 0x4000;
pub const TEKKEN_LM_SERVICE: u16 = 0x1000; // cpp:397 — service btn for p0.

/// Shift a Tekken per-player base mask by `(player % 2) * 24` bits.
#[must_use]
pub const fn tekken_mask_for_player(base: u64, player: u8) -> u64 {
    let shift = ((player % 2) as u32) * TEKKEN_PER_PLAYER_SHIFT;
    base << shift
}

pub const TEKKEN_INPUT_BUF_SIZE: usize = 0x180;

// ---------------------------------------------------------------------
// Channel 0 register IDs (cpp:432..521).
// ---------------------------------------------------------------------
pub const REG_SET_SYSTEM_ERROR: u16 = 0x0002;
pub const REG_CLEAR_SRAM: u16 = 0x000A;
pub const REG_CLEAR_SRAM_MAGIC: u16 = 0x6666;
pub const REG_SET_EXPANSION_MODE: u16 = 0x0028;
pub const REG_GET_BUFFER: u16 = 0x0000;
pub const REG_CARD_READER_CHECK_1: u16 = 0x0080;
pub const REG_CARD_READER_CHECK_2: u16 = 0x7000;
pub const REG_GET_TEKKEN_INPUT: u16 = 0x1000;

/// Hopper register base (cpp:448). Hoppers 0..=3 at offsets 0x48, 0x58,
/// 0x68, 0x78 (Request) and 0x4A, 0x5A, 0x6A, 0x7A (Limit).
#[must_use]
pub const fn hopper_index_from_reg(reg: u16) -> Option<(u8, HopperField)> {
    match reg {
        0x0048 => Some((0, HopperField::Request)),
        0x0058 => Some((1, HopperField::Request)),
        0x0068 => Some((2, HopperField::Request)),
        0x0078 => Some((3, HopperField::Request)),
        0x004A => Some((0, HopperField::Limit)),
        0x005A => Some((1, HopperField::Limit)),
        0x006A => Some((2, HopperField::Limit)),
        0x007A => Some((3, HopperField::Limit)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HopperField {
    Request,
    Limit,
}

// ---------------------------------------------------------------------
// Canned responses (byte-exact from cpp:501, 507).
// ---------------------------------------------------------------------

/// cpp:501 — 64-byte canned reply for `GetBuffer`.
pub const GET_BUFFER_RESPONSE: [u8; 64] = [
    0x7E, 0xE4, 0x00, 0x00, 0x74, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x80, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
];

/// cpp:507 — 16-byte canned reply for `CardReaderCheck1`.
pub const CARD_READER_CHECK_1_RESPONSE: [u8; 16] = [
    0x02, 0x03, 0x06, 0x00, 0xFF, 0x0F, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05, 0x10, 0x00,
];

// ---------------------------------------------------------------------
// SRAM access helpers (cpp:471..481).
// ---------------------------------------------------------------------

/// Check whether an SRAM write at `(channel, reg, size)` is in bounds.
/// Matches the cpp:477 condition:
/// `size > 0 && page < page_count && addr_end <= page_size`.
#[must_use]
pub const fn sram_write_in_bounds(channel: u8, reg: u16, size: usize) -> bool {
    if channel < 2 {
        return false;
    }
    let page = (channel - 2) as usize;
    if page >= PAGE_COUNT {
        return false;
    }
    let addr_end = reg as usize + size;
    size > 0 && addr_end <= PAGE_SIZE
}

/// Convert `(channel, reg)` into a linear offset inside `backup_memory`.
/// `channel` ≥ 2 encodes `page = channel - 2`.
#[must_use]
pub const fn sram_offset(channel: u8, reg: u16) -> Option<usize> {
    if channel < 2 {
        return None;
    }
    let page = (channel - 2) as usize;
    if page >= PAGE_COUNT {
        return None;
    }
    Some(PAGE_SIZE * page + reg as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_constants_frozen() {
        assert_eq!(USB_VID, 0x0b9a);
        assert_eq!(USB_PID, 0x0910);
        assert_eq!(USB_BCD_DEVICE, 0x0910);
        assert_eq!(USB_DEVICE_CLASS, 0xff);
        assert_eq!(USB_CONFIG_W_TOTAL_LENGTH, 39);
    }

    #[test]
    fn button_enum_order_matches_cpp() {
        assert_eq!(UsioBtn::Test as u32, 0);
        assert_eq!(UsioBtn::TaikoHitSideLeft as u32, 8);
        assert_eq!(UsioBtn::TekkenButton5 as u32, 16);
        assert_eq!(UsioBtn::Count as u32, 17);
    }

    #[test]
    fn sram_layout_constants() {
        assert_eq!(PAGE_SIZE, 0x10000);
        assert_eq!(PAGE_COUNT, 0x10);
        assert_eq!(BACKUP_SIZE, 0x10_0000);
    }

    #[test]
    fn c_hit_magic_word() {
        assert_eq!(C_HIT, 0x1800);
    }

    #[test]
    fn taiko_bitmasks_match_cpp() {
        assert_eq!(TAIKO_SERVICE, 0x4000);
        assert_eq!(TAIKO_ENTER, 0x0200);
        assert_eq!(TAIKO_UP, 0x2000);
        assert_eq!(TAIKO_DOWN, 0x1000);
        assert_eq!(TAIKO_TEST_ON, 0x0080);
    }

    #[test]
    fn taiko_hit_offsets_within_60_buffer() {
        for o in [
            TAIKO_HIT_SIDE_LEFT_OFFSET,
            TAIKO_HIT_CENTER_LEFT_OFFSET,
            TAIKO_HIT_CENTER_RIGHT_OFFSET,
            TAIKO_HIT_SIDE_RIGHT_OFFSET,
        ] {
            assert!(o + 2 <= TAIKO_INPUT_BUF_SIZE);
        }
    }

    #[test]
    fn encode_taiko_hit_writes_c_hit_at_correct_offset() {
        let mut buf = [0u8; TAIKO_INPUT_BUF_SIZE];
        assert!(encode_taiko_hit(&mut buf, 0, UsioBtn::TaikoHitSideLeft));
        assert_eq!(&buf[32..34], &C_HIT.to_le_bytes());
        // Player 1 shift by 8 bytes.
        let mut buf = [0u8; TAIKO_INPUT_BUF_SIZE];
        assert!(encode_taiko_hit(&mut buf, 1, UsioBtn::TaikoHitCenterRight));
        assert_eq!(&buf[36 + 8..36 + 10], &C_HIT.to_le_bytes());
    }

    #[test]
    fn encode_taiko_hit_rejects_non_hit_button() {
        let mut buf = [0u8; TAIKO_INPUT_BUF_SIZE];
        assert!(!encode_taiko_hit(&mut buf, 0, UsioBtn::Coin));
    }

    #[test]
    fn write_taiko_header_packs_digital_and_coin() {
        let mut buf = [0u8; TAIKO_INPUT_BUF_SIZE];
        write_taiko_header(&mut buf, 0x5180, 0x1234);
        assert_eq!(&buf[0..2], &0x5180u16.to_le_bytes());
        assert_eq!(&buf[16..18], &0x1234u16.to_le_bytes());
    }

    #[test]
    fn tekken_mask_for_player_shifts_by_24() {
        assert_eq!(tekken_mask_for_player(TEKKEN_ENTER_BASE, 0), 0x0080_0000);
        assert_eq!(
            tekken_mask_for_player(TEKKEN_ENTER_BASE, 1),
            0x0080_0000 << 24
        );
        // Player 2 → player%2 == 0 → same as player 0.
        assert_eq!(
            tekken_mask_for_player(TEKKEN_ENTER_BASE, 2),
            0x0080_0000
        );
    }

    #[test]
    fn tekken_bitmasks_match_cpp() {
        assert_eq!(TEKKEN_ENTER_BASE, 0x0080_0000);
        assert_eq!(TEKKEN_UP_BASE, 0x0020_0000);
        assert_eq!(TEKKEN_DOWN_BASE, 0x0010_0000);
        assert_eq!(TEKKEN_LEFT_BASE, 0x0008_0000);
        assert_eq!(TEKKEN_RIGHT_BASE, 0x0004_0000);
        assert_eq!(TEKKEN_BUTTON1_BASE, 0x0002_0000);
        assert_eq!(TEKKEN_BUTTON3_BASE, 0x4000_0000);
        assert_eq!(TEKKEN_BUTTON5_BASE, 0x8000_0000);
    }

    #[test]
    fn tekken_lm_bitmasks_match_cpp() {
        assert_eq!(TEKKEN_LM_ENTER, 0x0800);
        assert_eq!(TEKKEN_LM_UP, 0x0200);
        assert_eq!(TEKKEN_LM_DOWN, 0x0400);
        assert_eq!(TEKKEN_LM_LEFT, 0x2000);
        assert_eq!(TEKKEN_LM_RIGHT, 0x4000);
        assert_eq!(TEKKEN_LM_SERVICE, 0x1000);
    }

    #[test]
    fn channel_0_register_ids() {
        assert_eq!(REG_SET_SYSTEM_ERROR, 0x0002);
        assert_eq!(REG_CLEAR_SRAM, 0x000A);
        assert_eq!(REG_CLEAR_SRAM_MAGIC, 0x6666);
        assert_eq!(REG_SET_EXPANSION_MODE, 0x0028);
        assert_eq!(REG_GET_BUFFER, 0x0000);
        assert_eq!(REG_CARD_READER_CHECK_1, 0x0080);
        assert_eq!(REG_CARD_READER_CHECK_2, 0x7000);
        assert_eq!(REG_GET_TEKKEN_INPUT, 0x1000);
    }

    #[test]
    fn hopper_index_decoding_mirrors_cpp_division() {
        for (reg, want_idx, want_field) in [
            (0x0048u16, 0u8, HopperField::Request),
            (0x0058, 1, HopperField::Request),
            (0x0068, 2, HopperField::Request),
            (0x0078, 3, HopperField::Request),
            (0x004A, 0, HopperField::Limit),
            (0x005A, 1, HopperField::Limit),
            (0x006A, 2, HopperField::Limit),
            (0x007A, 3, HopperField::Limit),
        ] {
            let got = hopper_index_from_reg(reg).expect("known reg");
            assert_eq!(got, (want_idx, want_field), "reg {reg:#x}");
        }
        assert_eq!(hopper_index_from_reg(0x0049), None);
        assert_eq!(hopper_index_from_reg(0x0000), None);
    }

    #[test]
    fn get_buffer_response_size_and_preamble() {
        assert_eq!(GET_BUFFER_RESPONSE.len(), 64);
        assert_eq!(&GET_BUFFER_RESPONSE[..4], &[0x7E, 0xE4, 0x00, 0x00]);
        // Last 12 bytes zero (tail).
        assert_eq!(&GET_BUFFER_RESPONSE[55..64], &[0u8; 9]);
    }

    #[test]
    fn card_reader_check_1_response_bytes() {
        assert_eq!(CARD_READER_CHECK_1_RESPONSE.len(), 16);
        assert_eq!(
            CARD_READER_CHECK_1_RESPONSE,
            [
                0x02, 0x03, 0x06, 0x00, 0xFF, 0x0F, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x05,
                0x10, 0x00
            ]
        );
    }

    #[test]
    fn sram_write_bounds_channel_and_page() {
        // Channel 2 → page 0, reg 0 + 100 bytes fits.
        assert!(sram_write_in_bounds(2, 0, 100));
        // Last valid page = 0x11 (channel = 2 + 0xF = 0x11).
        assert!(sram_write_in_bounds(0x11, 0, 1));
        // Overflow within page.
        assert!(!sram_write_in_bounds(2, 0xFFFF, 2));
        // Page out of range.
        assert!(!sram_write_in_bounds(0x12, 0, 1));
        // Channel < 2 rejected.
        assert!(!sram_write_in_bounds(0, 0, 1));
        assert!(!sram_write_in_bounds(1, 0, 1));
        // size == 0 rejected.
        assert!(!sram_write_in_bounds(2, 0, 0));
    }

    #[test]
    fn sram_offset_math() {
        assert_eq!(sram_offset(2, 0), Some(0));
        assert_eq!(sram_offset(3, 0), Some(PAGE_SIZE));
        assert_eq!(sram_offset(2, 0x1234), Some(0x1234));
        assert_eq!(sram_offset(0x11, 0xFFFF), Some(PAGE_SIZE * 0xF + 0xFFFF));
        assert_eq!(sram_offset(0, 0), None);
        assert_eq!(sram_offset(0x12, 0), None);
    }
}
