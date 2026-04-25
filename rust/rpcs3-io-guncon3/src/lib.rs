//! `rpcs3-io-guncon3` — Rust port of `rpcs3/Emu/Io/GunCon3.cpp`.
//!
//! Namco GunCon 3 light-gun emulator (VID=0x0b9a PID=0x0800). The interesting
//! bit is the per-packet obfuscation in `guncon3_encode`: a 256-byte key
//! table plus a 3-round per-byte mix whose operation (add / sub / xor) is
//! chosen by the low two bits of the current `KEY_TABLE` byte. Anything
//! that re-implements the device must produce byte-identical output, so
//! we freeze:
//!
//! - USB descriptor constants (cpp:130..160): VID/PID, interface class
//!   0xff (vendor-specific), two endpoints.
//! - `GunCon3Data` packed layout (cpp:58..90) — 19 bytes of button
//!   bitfields, gun x/y/z (int16 LE), four sticks, checksum, keyindex.
//! - The 256-byte `KEY_TABLE` from cpp:39..56, byte-exact.
//! - `guncon3_encode(&mut data, key)` from cpp:92..124:
//!   - `key_offset` seed from `key[1..=7]` + `data[14]` (keyindex) via a
//!     fixed XOR/ADD/SUB/XOR chain (cpp:96).
//!   - 3 rounds per data byte × 13 bytes (cpp:99..119), with op chosen by
//!     `KEY_TABLE[key_offset] & 3`: 0 = add, 1 = sub, else = xor.
//!   - Checksum computed over `data[0..=12]` using the shape at cpp:121.
//! - Button/bitfield decoder helpers so callers can compose input without
//!   reverse-engineering the bitfield packing.

use core::mem::size_of;

// USB descriptor constants (cpp:130..151).
pub const USB_VID: u16 = 0x0b9a;
pub const USB_PID: u16 = 0x0800;
pub const USB_BCD_DEVICE: u16 = 0x8000;
pub const USB_BCD_USB: u16 = 0x0110;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x08;
pub const USB_CONFIG_W_TOTAL_LENGTH: u16 = 0x0020;
pub const USB_CONFIG_NUM_INTERFACES: u8 = 0x01;
pub const USB_CONFIG_VALUE: u8 = 0x01;
pub const USB_CONFIG_BM_ATTRIBUTES: u8 = 0x00;
pub const USB_CONFIG_MAX_POWER: u8 = 0x32;
pub const USB_INTERFACE_CLASS_VENDOR: u8 = 0xff;
pub const USB_INTERFACE_NUM_ENDPOINTS: u8 = 0x02;

/// 256-byte key table (cpp:39..56). **Byte-exact** — any edit here changes
/// the cipher output and breaks game-side validation.
pub const KEY_TABLE: [u8; 256] = [
    0x91, 0xFD, 0x4C, 0x8B, 0x20, 0xC1, 0x7C, 0x09, 0x58, 0x14, 0xF6, 0x00, 0x52, 0x55, 0xBF, 0x41,
    0x75, 0xC0, 0x13, 0x30, 0xB5, 0xD0, 0x69, 0x85, 0x89, 0xBB, 0xD6, 0x88, 0xBC, 0x73, 0x18, 0x8D,
    0x58, 0xAB, 0x3D, 0x98, 0x5C, 0xF2, 0x48, 0xE9, 0xAC, 0x9F, 0x7A, 0x0C, 0x7C, 0x25, 0xD8, 0xFF,
    0xDC, 0x7D, 0x08, 0xDB, 0xBC, 0x18, 0x8C, 0x1D, 0xD6, 0x3C, 0x35, 0xE1, 0x2C, 0x14, 0x8E, 0x64,
    0x83, 0x39, 0xB0, 0xE4, 0x4E, 0xF7, 0x51, 0x7B, 0xA8, 0x13, 0xAC, 0xE9, 0x43, 0xC0, 0x08, 0x25,
    0x0E, 0x15, 0xC4, 0x20, 0x93, 0x13, 0xF5, 0xC3, 0x48, 0xCC, 0x47, 0x1C, 0xC5, 0x20, 0xDE, 0x60,
    0x55, 0xEE, 0xA0, 0x40, 0xB4, 0xE7, 0x74, 0x95, 0xB0, 0x46, 0xEC, 0xF0, 0xA5, 0xB8, 0x23, 0xC8,
    0x04, 0x06, 0xFC, 0x28, 0xCB, 0xF8, 0x17, 0x2C, 0x25, 0x1C, 0xCB, 0x18, 0xE3, 0x6C, 0x80, 0x85,
    0xDD, 0x7E, 0x09, 0xD9, 0xBC, 0x19, 0x8F, 0x1D, 0xD4, 0x3D, 0x37, 0xE1, 0x2F, 0x15, 0x8D, 0x64,
    0x06, 0x04, 0xFD, 0x29, 0xCF, 0xFA, 0x14, 0x2E, 0x25, 0x1F, 0xC9, 0x18, 0xE3, 0x6D, 0x81, 0x84,
    0x80, 0x3B, 0xB1, 0xE5, 0x4D, 0xF7, 0x51, 0x78, 0xA9, 0x13, 0xAD, 0xE9, 0x80, 0xC1, 0x0B, 0x25,
    0x93, 0xFC, 0x4D, 0x89, 0x23, 0xC2, 0x7C, 0x0B, 0x59, 0x15, 0xF6, 0x01, 0x50, 0x55, 0xBF, 0x81,
    0x75, 0xC3, 0x10, 0x31, 0xB5, 0xD3, 0x69, 0x84, 0x89, 0xBA, 0xD6, 0x89, 0xBD, 0x70, 0x19, 0x8E,
    0x58, 0xA8, 0x3D, 0x9B, 0x5D, 0xF0, 0x49, 0xE8, 0xAD, 0x9D, 0x7A, 0x0D, 0x7E, 0x24, 0xDA, 0xFC,
    0x0D, 0x14, 0xC5, 0x23, 0x91, 0x11, 0xF5, 0xC0, 0x4B, 0xCD, 0x44, 0x1C, 0xC5, 0x21, 0xDF, 0x61,
    0x54, 0xED, 0xA2, 0x81, 0xB7, 0xE5, 0x74, 0x94, 0xB0, 0x47, 0xEE, 0xF1, 0xA5, 0xBB, 0x21, 0xC8,
];

/// Packed report layout (cpp:58..90). 19 bytes total.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GunCon3Data {
    /// data[0] — bits: _, a2, a1, c2, ___ (cpp:61..65).
    pub btn_bits0: u8,
    /// data[1] — bits: _, b2, b1, _, _, trigger, _, c1 (cpp:67..73).
    pub btn_bits1: u8,
    /// data[2] — bits: ______, b3, a3 (cpp:75..77).
    pub btn_bits2: u8,
    /// data[3..=8] — gun position (int16 LE × 3).
    pub gun_x: i16,
    pub gun_y: i16,
    pub gun_z: i16,
    pub stick_bx: u8,
    pub stick_by: u8,
    pub stick_ax: u8,
    pub stick_ay: u8,
    pub checksum: u8,
    pub keyindex: u8,
}

const _: () = assert!(size_of::<GunCon3Data>() == 15);

// Button bits for btn_bits0 (cpp:61..65). Bit 0 is unused.
pub const BTN0_A2: u8 = 1 << 1;
pub const BTN0_A1: u8 = 1 << 2;
pub const BTN0_C2: u8 = 1 << 3;

// Button bits for btn_bits1 (cpp:67..73).
pub const BTN1_B2: u8 = 1 << 1;
pub const BTN1_B1: u8 = 1 << 2;
pub const BTN1_TRIGGER: u8 = 1 << 5;
pub const BTN1_C1: u8 = 1 << 7;

// Button bits for btn_bits2 (cpp:75..77).
pub const BTN2_B3: u8 = 1 << 6;
pub const BTN2_A3: u8 = 1 << 7;

/// Compute the initial key offset. Matches cpp:96:
/// `(((key[1] ^ key[2]) - key[3] - key[4]) ^ key[5]) + key[6] - key[7]) ^ data[14]`.
/// `keyindex` is `data[14]` from the report.
#[must_use]
pub fn initial_key_offset(key: &[u8; 8], keyindex: u8) -> u8 {
    let a = key[1] ^ key[2];
    let b = a.wrapping_sub(key[3]).wrapping_sub(key[4]);
    let c = b ^ key[5];
    let d = c.wrapping_add(key[6]).wrapping_sub(key[7]);
    d ^ keyindex
}

/// `guncon3_encode(gc, data, key)` from cpp:92..124, byte-exact.
///
/// Arguments:
/// - `data` — exactly the 15-byte `GunCon3Data` serialized in packed LE
///   order. Rewritten in place.
/// - `key` — the 8-byte session key (indices 0 through 7). `key[0]` is not
///   read by the cipher; the cpp version uses `++key_index` starting from
///   0 so the first read is `key[1]`.
pub fn guncon3_encode(data: &mut [u8; 15], key: &[u8; 8]) {
    let mut key_offset = initial_key_offset(key, data[14]);
    let mut key_index: u8 = 0;

    for i in 0..13 {
        let mut byte = data[i];
        for _j in 0..3 {
            let bkey = KEY_TABLE[key_offset as usize];
            key_index += 1;
            let keyr = key[key_index as usize];
            if key_index == 7 {
                key_index = 0;
            }

            byte = match bkey & 3 {
                0 => byte.wrapping_add(bkey).wrapping_add(keyr),
                1 => byte.wrapping_sub(bkey).wrapping_sub(keyr),
                _ => byte ^ bkey ^ keyr,
            };

            key_offset = key_offset.wrapping_add(1);
        }
        data[i] = byte;
    }

    // Checksum (cpp:121..123). `^` is left-to-right because C has no
    // parentheses here but the cpp reads as explicit groupings.
    let mut c = key[7]
        .wrapping_add(data[0])
        .wrapping_sub(data[1])
        .wrapping_sub(data[2]);
    c ^= data[3];
    c = c.wrapping_add(data[4]).wrapping_add(data[5]);
    c ^= data[6];
    c ^= data[7];
    c = c
        .wrapping_add(data[8])
        .wrapping_add(data[9])
        .wrapping_sub(data[10])
        .wrapping_sub(data[11]);
    c ^= data[12];
    data[13] = c;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_struct_size() {
        assert_eq!(size_of::<GunCon3Data>(), 15);
    }

    #[test]
    fn key_table_spot_checks() {
        assert_eq!(KEY_TABLE[0], 0x91);
        assert_eq!(KEY_TABLE[15], 0x41);
        assert_eq!(KEY_TABLE[255], 0xC8);
        assert_eq!(KEY_TABLE.len(), 256);
    }

    #[test]
    fn usb_vid_pid_match() {
        assert_eq!(USB_VID, 0x0b9a);
        assert_eq!(USB_PID, 0x0800);
        assert_eq!(USB_BCD_DEVICE, 0x8000);
        assert_eq!(USB_INTERFACE_CLASS_VENDOR, 0xff);
    }

    #[test]
    fn initial_key_offset_formula() {
        // Hand-computed: key = [0, 1, 2, 3, 4, 5, 6, 7], data[14] = 0.
        // a = 1 ^ 2 = 3
        // b = 3 - 3 - 4 = -4 (u8 wrap) = 0xFC
        // c = 0xFC ^ 5 = 0xF9
        // d = 0xF9 + 6 - 7 = 0xF8
        // result = 0xF8 ^ 0 = 0xF8
        assert_eq!(
            initial_key_offset(&[0, 1, 2, 3, 4, 5, 6, 7], 0),
            0xF8
        );
        assert_eq!(
            initial_key_offset(&[0, 1, 2, 3, 4, 5, 6, 7], 0xFF),
            0xF8 ^ 0xFF
        );
    }

    #[test]
    fn encode_is_deterministic_for_fixed_key() {
        // Zero report with a fixed key encodes to a stable byte string.
        let mut data = [0u8; 15];
        let key = [0, 1, 2, 3, 4, 5, 6, 7];
        guncon3_encode(&mut data, &key);

        // Two identical runs must match.
        let mut data2 = [0u8; 15];
        guncon3_encode(&mut data2, &key);
        assert_eq!(data, data2);

        // The first byte isn't 0 any more — the cipher mixed something in.
        assert_ne!(data[0], 0);
        // data[13] is the checksum set from data[0..=12] post-encode.
        assert_eq!(
            data[13],
            {
                let d = &data;
                let mut c = key[7]
                    .wrapping_add(d[0])
                    .wrapping_sub(d[1])
                    .wrapping_sub(d[2]);
                c ^= d[3];
                c = c.wrapping_add(d[4]).wrapping_add(d[5]);
                c ^= d[6];
                c ^= d[7];
                c = c
                    .wrapping_add(d[8])
                    .wrapping_add(d[9])
                    .wrapping_sub(d[10])
                    .wrapping_sub(d[11]);
                c ^ d[12]
            }
        );
    }

    #[test]
    fn encode_op_dispatch_bkey_lsb() {
        // Verify add / sub / xor dispatch by running a targeted frame.
        // Op is selected by `KEY_TABLE[key_offset] & 3`; we just need to
        // verify the output differs when the input differs, as a smoke
        // test that the inner loop wasn't optimized away.
        let key = [0; 8];
        let mut a = [0u8; 15];
        let mut b = [0u8; 15];
        b[0] = 1;
        guncon3_encode(&mut a, &key);
        guncon3_encode(&mut b, &key);
        assert_ne!(a, b, "different inputs must yield different ciphertext");
    }

    #[test]
    fn keyindex_wraps_at_7() {
        // Encoding 13 bytes × 3 rounds = 39 key increments. With wrap at 7
        // the loop must reach 7 five times without panicking. This test
        // exercises the full path.
        let mut data = [0xAA; 15];
        let key = [0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80];
        guncon3_encode(&mut data, &key);
        // Just ensure we got here without OOB.
        assert_ne!(data[13], 0xAA, "checksum must have been overwritten");
    }

    #[test]
    fn button_bit_masks_match_cpp() {
        assert_eq!(BTN0_A1, 0x04);
        assert_eq!(BTN0_A2, 0x02);
        assert_eq!(BTN0_C2, 0x08);
        assert_eq!(BTN1_B1, 0x04);
        assert_eq!(BTN1_B2, 0x02);
        assert_eq!(BTN1_TRIGGER, 0x20);
        assert_eq!(BTN1_C1, 0x80);
        assert_eq!(BTN2_A3, 0x80);
        assert_eq!(BTN2_B3, 0x40);
    }
}
