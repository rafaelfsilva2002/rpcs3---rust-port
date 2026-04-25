//! `rpcs3-io-infinity` — Rust port of `rpcs3/Emu/Io/Infinity.cpp`.
//!
//! Disney Infinity Base USB portal emulator. Protocol is byte-level
//! custom + SHA1/AES crypto for figure key derivation. This crate freezes
//! the algorithmic core; SHA1 / AES themselves are re-used from
//! `rpcs3-crypto` (not linked here — we expose the small helpers that
//! sit on top of them so tests are deterministic without crypto deps).
//!
//! Frozen:
//!
//! - `SHA1_CONSTANT` (32 B) from cpp:13..16 — prefix used when hashing UID.
//! - USB descriptor constants (cpp:363..368): VID=0x0E6F / PID=0x0129.
//! - 64-bit `SCRAMBLE_MASK = 0x8E55AA1B3999E8AA` (cpp:73, 92).
//! - `scramble(num, garbage)` and `descramble(scrambled)` (cpp:71..114).
//! - Custom Jenkins-variant PRNG with **23 warmup rounds** (cpp:116..149).
//! - `derive_figure_position(pos)` — group slots 0/1/2 → 1, 3/4/5 → 2,
//!   6/7/8 → 3, else 0 (cpp:183..203).
//! - `generate_checksum` (sum & 0xFF, cpp:28..36).
//! - Reply preambles: `[0xAA, 0x01, seq, chk]` blank,
//!   `[0xAA, 0x09, seq, scrambled..8, chk]` challenge response,
//!   `[0xAA, 0x12, seq, 0x00, data..16, chk]` query_block,
//!   `[0xAA, 0x02, seq, 0x00, chk]` write_block,
//!   `[0xAA, 0x09, seq, 0x00, uid..7, chk]` figure_identifier,
//!   `[0xAB, 0x04, position, 0x09, order, 0x00/0x01, chk]` added/removed.
//! - `aes_key_from_sha1_output(sha1)` — reverses byte order within each
//!   4-byte group (cpp:318..325). Tests can exercise this without running
//!   SHA1/AES.
//! - `extract_figure_number(decrypted_block)` — `bytes[1] << 16 | bytes[2]
//!   << 8 | bytes[3]` (cpp:332..333, 24-bit BE u24).
//! - `file_block_for_page(block)` — `block == 0 ? 1 : block * 4` (cpp:215).

/// Prefix used when computing the SHA1 of UID to derive the AES key
/// (cpp:13..16). Ends with "(c) Disney 2013" ASCII minus the last byte
/// (the UID[0..=6] is appended before hashing).
pub const SHA1_CONSTANT: [u8; 32] = [
    0xAF, 0x62, 0xD2, 0xEC, 0x04, 0x91, 0x96, 0x8C, 0xC5, 0x2A, 0x1A, 0x71, 0x65, 0xF8, 0x65,
    0xFE, 0x28, 0x63, 0x29, 0x20, 0x44, 0x69, 0x73, 0x6e, 0x65, 0x79, 0x20, 0x32, 0x30, 0x31, 0x33,
    0x00,
];
// The 32nd byte (index 31) is omitted in cpp:305 `std::vector<u8>
// sha1_calc = {SHA1_CONSTANT.begin(), SHA1_CONSTANT.end() - 1}`, so only
// 31 bytes feed the SHA1. Expose the range explicitly:
pub const SHA1_PREFIX: &[u8] = &[
    0xAF, 0x62, 0xD2, 0xEC, 0x04, 0x91, 0x96, 0x8C, 0xC5, 0x2A, 0x1A, 0x71, 0x65, 0xF8, 0x65,
    0xFE, 0x28, 0x63, 0x29, 0x20, 0x44, 0x69, 0x73, 0x6e, 0x65, 0x79, 0x20, 0x32, 0x30, 0x31, 0x33,
];

// USB descriptor constants (cpp:363..368).
pub const USB_VID: u16 = 0x0E6F;
pub const USB_PID: u16 = 0x0129;
pub const USB_BCD_DEVICE: u16 = 0x0200;
pub const USB_BCD_USB: u16 = 0x0200;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x20;
pub const USB_CONFIG_MAX_POWER: u8 = 0xFA;
pub const USB_INTERFACE_CLASS_HID: u8 = 0x03;
pub const USB_HID_BCD: u16 = 0x0111;
pub const USB_ENDPOINT_IN_ADDRESS: u8 = 0x81;
pub const USB_ENDPOINT_OUT_ADDRESS: u8 = 0x01;
pub const USB_ENDPOINT_W_MAX_PACKET_SIZE: u16 = 0x0020;

/// Bit mask used by scramble/descramble (cpp:73, 92). **Byte-exact.**
pub const SCRAMBLE_MASK: u64 = 0x8E55_AA1B_3999_E8AA;

pub const FIGURE_SLOTS: usize = 9;
pub const FIGURE_DATA_SIZE: usize = 0x14 * 0x10;
pub const REPLY_SIZE: usize = 32;

/// 8-bit sum-wrap checksum (cpp:28..36).
#[must_use]
pub fn generate_checksum(buf: &[u8], num_bytes: usize) -> u8 {
    assert!(num_bytes <= buf.len());
    let mut sum: u32 = 0;
    for &b in &buf[..num_bytes] {
        sum = sum.wrapping_add(u32::from(b));
    }
    (sum & 0xFF) as u8
}

/// `scramble(num_to_scramble, garbage)` (cpp:90..114). 64-bit result where
/// each bit position is sourced from `num_to_scramble` if the corresponding
/// `SCRAMBLE_MASK` bit (MSB-first) is 1, else from `garbage`. Output bits
/// are produced MSB-first; inputs are consumed LSB-first.
#[must_use]
pub fn scramble(num_to_scramble: u32, garbage: u32) -> u64 {
    let mut mask = SCRAMBLE_MASK;
    let mut num = num_to_scramble;
    let mut garb = garbage;
    let mut ret: u64 = 0;
    for _ in 0..64 {
        ret <<= 1;
        if (mask & 1) != 0 {
            ret |= u64::from(num & 1);
            num >>= 1;
        } else {
            ret |= u64::from(garb & 1);
            garb >>= 1;
        }
        mask >>= 1;
    }
    ret
}

/// `descramble(num_to_descramble)` (cpp:71..88). Extracts only the bits
/// that were sourced from "real" data (mask bit == 1, MSB-first).
#[must_use]
pub fn descramble(num_to_descramble: u64) -> u32 {
    let mut mask = SCRAMBLE_MASK;
    let mut num = num_to_descramble;
    let mut ret: u32 = 0;
    for _ in 0..64 {
        if (mask & 0x8000_0000_0000_0000) != 0 {
            ret = (ret << 1) | (num as u32 & 1);
        }
        num >>= 1;
        mask <<= 1;
    }
    ret
}

/// Infinity-specific Jenkins-variant PRNG (cpp:116..149). Note the
/// **23 warmup rounds** (vs dimensions' 42) and a different body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfinityRng {
    pub a: u32,
    pub b: u32,
    pub c: u32,
    pub d: u32,
}

impl InfinityRng {
    pub const INIT_A: u32 = 0xF1EA_5EED;
    pub const WARMUP_ROUNDS: u32 = 23;

    /// `generate_seed(seed)` (cpp:116..127).
    #[must_use]
    pub fn seed(seed: u32) -> Self {
        let mut rng = Self { a: Self::INIT_A, b: seed, c: seed, d: seed };
        for _ in 0..Self::WARMUP_ROUNDS {
            rng.next();
        }
        rng
    }

    /// `get_next()` (cpp:129..149). Returns the new `random_d`.
    pub fn next(&mut self) -> u32 {
        let a = self.a;
        let b = self.b;
        let c = self.c;
        // cpp:134 `ret = rotl(random_b, 27);`
        let mut ret = b.rotate_left(27);
        // cpp:136 `temp = (a + ((ret ^ 0xFFFFFFFF) + 1));` — a + (-ret).
        let temp = a.wrapping_add((!ret).wrapping_add(1));
        // cpp:137 `b ^= rotl(c, 17);`
        let b_new = b ^ c.rotate_left(17);
        // cpp:138 `a = random_d;`
        let a_from_d = self.d;
        // cpp:139 `c += a;` (a here is the new a = random_d).
        let c_new = c.wrapping_add(a_from_d);
        // cpp:140 `ret = b + temp;`
        ret = b_new.wrapping_add(temp);
        // cpp:141 `a += temp;` (a here is still random_d).
        let a_new = a_from_d.wrapping_add(temp);

        self.c = a_new;
        self.a = b_new;
        self.b = c_new;
        self.d = ret;
        ret
    }
}

/// `derive_figure_position(position)` (cpp:183..203). Groups the 9 raw
/// slot indices into 3 "zones" the game cares about; returns `0` for
/// any out-of-range slot.
#[must_use]
pub const fn derive_figure_position(position: u8) -> u8 {
    match position {
        0 | 1 | 2 => 1,
        3 | 4 | 5 => 2,
        6 | 7 | 8 => 3,
        _ => 0,
    }
}

/// `get_blank_response(sequence)` (cpp:38..44).
#[must_use]
pub fn blank_response(sequence: u8) -> [u8; REPLY_SIZE] {
    let mut r = [0u8; REPLY_SIZE];
    r[0] = 0xaa;
    r[1] = 0x01;
    r[2] = sequence;
    r[3] = generate_checksum(&r, 3);
    r
}

/// `get_next_and_scramble(sequence, reply)` (cpp:55..69).
pub fn next_and_scramble(rng: &mut InfinityRng, sequence: u8) -> [u8; REPLY_SIZE] {
    let next_random = rng.next();
    let scrambled = scramble(next_random, 0);
    let mut r = [0u8; REPLY_SIZE];
    r[0] = 0xAA;
    r[1] = 0x09;
    r[2] = sequence;
    // cpp packs MSB-first into r[3..=10].
    r[3] = ((scrambled >> 56) & 0xFF) as u8;
    r[4] = ((scrambled >> 48) & 0xFF) as u8;
    r[5] = ((scrambled >> 40) & 0xFF) as u8;
    r[6] = ((scrambled >> 32) & 0xFF) as u8;
    r[7] = ((scrambled >> 24) & 0xFF) as u8;
    r[8] = ((scrambled >> 16) & 0xFF) as u8;
    r[9] = ((scrambled >> 8) & 0xFF) as u8;
    r[10] = (scrambled & 0xFF) as u8;
    r[11] = generate_checksum(&r, 11);
    r
}

/// `descramble_and_seed(buf, sequence)` (cpp:46..53). Reads bytes 4..=11
/// as a big-endian u64, descrambles down to a u32 seed, and returns the
/// seeded RNG plus a blank 4-byte reply.
pub fn descramble_and_seed(buf: &[u8], sequence: u8) -> (InfinityRng, [u8; REPLY_SIZE]) {
    assert!(buf.len() >= 12);
    let value: u64 = (u64::from(buf[4]) << 56)
        | (u64::from(buf[5]) << 48)
        | (u64::from(buf[6]) << 40)
        | (u64::from(buf[7]) << 32)
        | (u64::from(buf[8]) << 24)
        | (u64::from(buf[9]) << 16)
        | (u64::from(buf[10]) << 8)
        | u64::from(buf[11]);
    let seed = descramble(value);
    let rng = InfinityRng::seed(seed);
    (rng, blank_response(sequence))
}

/// `file_block_for_page(block)` (cpp:215, 234). Page 0 maps to the first
/// data block (file block 1); page N (N >= 1) maps to file block N*4.
#[must_use]
pub const fn file_block_for_page(page: u8) -> u8 {
    if page == 0 { 1 } else { page.wrapping_mul(4) }
}

/// Compose a `query_block` reply (cpp:205..221). `data_block` is the
/// 16-byte chunk read from the figure at offset `file_block * 16`.
/// `None` (no figure present / OOB) zeros out the data region.
pub fn query_block_reply(
    sequence: u8,
    data_block: Option<&[u8; 16]>,
) -> [u8; REPLY_SIZE] {
    let mut r = [0u8; REPLY_SIZE];
    r[0] = 0xaa;
    r[1] = 0x12;
    r[2] = sequence;
    r[3] = 0x00;
    if let Some(block) = data_block {
        r[4..4 + 16].copy_from_slice(block);
    }
    r[20] = generate_checksum(&r, 20);
    r
}

/// `write_block` reply shape (cpp:223..241).
pub fn write_block_reply(sequence: u8) -> [u8; REPLY_SIZE] {
    let mut r = [0u8; REPLY_SIZE];
    r[0] = 0xaa;
    r[1] = 0x02;
    r[2] = sequence;
    r[3] = 0x00;
    r[4] = generate_checksum(&r, 4);
    r
}

/// `get_figure_identifier` reply (cpp:243..259). Copies 7 UID bytes into
/// r[4..=10] when `figure_present`; zeros otherwise.
pub fn figure_identifier_reply(
    sequence: u8,
    uid: Option<&[u8; 7]>,
) -> [u8; REPLY_SIZE] {
    let mut r = [0u8; REPLY_SIZE];
    r[0] = 0xaa;
    r[1] = 0x09;
    r[2] = sequence;
    r[3] = 0x00;
    if let Some(uid_bytes) = uid {
        r[4..4 + 7].copy_from_slice(uid_bytes);
    }
    r[11] = generate_checksum(&r, 11);
    r
}

/// Figure added/removed event response (cpp:291..293, 353..355).
/// `removed == true` → last byte `0x01`; `removed == false` → `0x00`.
#[must_use]
pub fn figure_change_response(position: u8, order_added: u8, removed: bool) -> [u8; REPLY_SIZE] {
    let mut r = [0u8; REPLY_SIZE];
    r[0] = 0xab;
    r[1] = 0x04;
    r[2] = position;
    r[3] = 0x09;
    r[4] = order_added;
    r[5] = if removed { 0x01 } else { 0x00 };
    r[6] = generate_checksum(&r, 6);
    r
}

/// `present_figures` reply packing (cpp:151..169). For each present figure
/// at index `i`, writes `[slot_base | order_added, 0x09]` pairs starting
/// at `r[3]`. Slot bases: `0x10` for i==0, `0x20` for 1..=3, `0x30` for 4+.
pub fn present_figures_reply(
    sequence: u8,
    figures: &[(bool, u8)],
) -> [u8; REPLY_SIZE] {
    let mut r = [0u8; REPLY_SIZE];
    let mut x: usize = 3;
    for (i, (present, order_added)) in figures.iter().enumerate() {
        let slot_base = if i == 0 {
            0x10
        } else if i < 4 {
            0x20
        } else {
            0x30
        };
        if *present {
            r[x] = slot_base + *order_added;
            r[x + 1] = 0x09;
            x += 2;
        }
    }
    r[0] = 0xaa;
    // cpp:166 sets r[1] = x - 2 (sub the 3-byte preamble minus 1).
    r[1] = x.wrapping_sub(2) as u8;
    r[2] = sequence;
    r[x] = generate_checksum(&r, x);
    r
}

/// Derive the AES-128 key from a 20-byte SHA1 output (cpp:318..325).
/// Within each 4-byte group, bytes are reversed: `key[4i + x] = sha1[4i + (3-x)]`.
/// First 16 bytes of SHA1 feed the key; last 4 are discarded.
#[must_use]
pub fn aes_key_from_sha1_output(sha1: &[u8; 20]) -> [u8; 16] {
    let mut key = [0u8; 16];
    for i in 0..4 {
        for x in 0..4 {
            key[x + i * 4] = sha1[(3 - x) + i * 4];
        }
    }
    key
}

/// Extract the 24-bit figure number from the AES-decrypted block (cpp:332..333).
/// `n = bytes[1] << 16 | bytes[2] << 8 | bytes[3]`.
#[must_use]
pub const fn extract_figure_number(decrypted_block: &[u8; 16]) -> u32 {
    ((decrypted_block[1] as u32) << 16)
        | ((decrypted_block[2] as u32) << 8)
        | (decrypted_block[3] as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_constant_first_last() {
        assert_eq!(SHA1_CONSTANT.len(), 32);
        assert_eq!(SHA1_CONSTANT[0], 0xAF);
        // cpp declares `std::array<u8, 32>` with 31 explicit values; the
        // 32nd is default-initialized to 0.
        assert_eq!(SHA1_CONSTANT[30], 0x33);
        assert_eq!(SHA1_CONSTANT[31], 0x00);
        assert_eq!(SHA1_PREFIX.len(), 31, "cpp drops the last byte");
        // "(c) Disney 2013" ASCII starts at offset 16.
        assert_eq!(&SHA1_CONSTANT[16..19], b"(c)");
        assert_eq!(&SHA1_CONSTANT[20..26], b"Disney");
        assert_eq!(&SHA1_CONSTANT[27..31], b"2013");
    }

    #[test]
    fn usb_vid_pid() {
        assert_eq!(USB_VID, 0x0E6F);
        assert_eq!(USB_PID, 0x0129);
        assert_eq!(USB_BCD_DEVICE, 0x0200);
    }

    #[test]
    fn scramble_mask_byte_exact() {
        assert_eq!(SCRAMBLE_MASK, 0x8E55_AA1B_3999_E8AA);
    }

    #[test]
    fn scramble_descramble_round_trip() {
        for &v in &[0u32, 1, 0xDEAD_BEEF, 0x1234_5678, 0xFFFF_FFFF] {
            let s = scramble(v, 0);
            let d = descramble(s);
            assert_eq!(d, v, "round trip for {:#x}", v);
        }
    }

    #[test]
    fn descramble_ignores_garbage_bits() {
        let s1 = scramble(0x1234_5678, 0);
        let s2 = scramble(0x1234_5678, 0xFFFF_FFFF);
        // Garbage changes the scrambled output...
        assert_ne!(s1, s2);
        // ...but descramble recovers the same value.
        assert_eq!(descramble(s1), 0x1234_5678);
        assert_eq!(descramble(s2), 0x1234_5678);
    }

    #[test]
    fn infinity_rng_seed_determinism() {
        let a = InfinityRng::seed(0x42);
        let b = InfinityRng::seed(0x42);
        assert_eq!(a, b);
        let c = InfinityRng::seed(0x43);
        assert_ne!(a, c);
    }

    #[test]
    fn infinity_rng_warmup_is_23_rounds_not_42() {
        // Confirm distinct-from-dimensions behavior: same seed processed
        // through our RNG should yield a specific state.
        let rng = InfinityRng::seed(0);
        // d should be non-zero after 23 warmup rounds with seed=0
        // (the transformation is non-trivial even on zero seed).
        assert_ne!(rng.d, 0);
        assert_eq!(InfinityRng::WARMUP_ROUNDS, 23);
    }

    #[test]
    fn derive_figure_position_maps() {
        for p in 0..=2 {
            assert_eq!(derive_figure_position(p), 1);
        }
        for p in 3..=5 {
            assert_eq!(derive_figure_position(p), 2);
        }
        for p in 6..=8 {
            assert_eq!(derive_figure_position(p), 3);
        }
        assert_eq!(derive_figure_position(9), 0);
        assert_eq!(derive_figure_position(255), 0);
    }

    #[test]
    fn blank_response_preamble_and_checksum() {
        let r = blank_response(0x05);
        assert_eq!(&r[..3], &[0xaa, 0x01, 0x05]);
        assert_eq!(r[3], generate_checksum(&[0xaa, 0x01, 0x05], 3));
    }

    #[test]
    fn next_and_scramble_layout() {
        let mut rng = InfinityRng::seed(0x10);
        let r = next_and_scramble(&mut rng, 0x42);
        assert_eq!(r[0], 0xAA);
        assert_eq!(r[1], 0x09);
        assert_eq!(r[2], 0x42);
        assert_eq!(r[11], generate_checksum(&r, 11));
        // Re-running with fresh seed produces identical reply.
        let mut rng2 = InfinityRng::seed(0x10);
        let r2 = next_and_scramble(&mut rng2, 0x42);
        assert_eq!(r, r2);
    }

    #[test]
    fn descramble_and_seed_recovers_seed() {
        // Construct a fake control transfer buffer: bytes 4..=11 encode
        // a scrambled seed (MSB-first).
        let seed_value = 0xCAFE_BABE;
        let scrambled = scramble(seed_value, 0);
        let mut buf = [0u8; 12];
        buf[4] = ((scrambled >> 56) & 0xFF) as u8;
        buf[5] = ((scrambled >> 48) & 0xFF) as u8;
        buf[6] = ((scrambled >> 40) & 0xFF) as u8;
        buf[7] = ((scrambled >> 32) & 0xFF) as u8;
        buf[8] = ((scrambled >> 24) & 0xFF) as u8;
        buf[9] = ((scrambled >> 16) & 0xFF) as u8;
        buf[10] = ((scrambled >> 8) & 0xFF) as u8;
        buf[11] = (scrambled & 0xFF) as u8;
        let (rng, reply) = descramble_and_seed(&buf, 0x01);
        // RNG was seeded with the recovered seed value.
        assert_eq!(rng, InfinityRng::seed(seed_value));
        assert_eq!(&reply[..3], &[0xaa, 0x01, 0x01]);
    }

    #[test]
    fn file_block_for_page_first_is_one_others_times_four() {
        assert_eq!(file_block_for_page(0), 1);
        assert_eq!(file_block_for_page(1), 4);
        assert_eq!(file_block_for_page(2), 8);
        assert_eq!(file_block_for_page(4), 16);
    }

    #[test]
    fn query_block_reply_layout() {
        let data = [0xAAu8; 16];
        let r = query_block_reply(0x07, Some(&data));
        assert_eq!(r[0], 0xaa);
        assert_eq!(r[1], 0x12);
        assert_eq!(r[2], 0x07);
        assert_eq!(r[3], 0x00);
        assert_eq!(&r[4..20], &[0xAAu8; 16]);
        assert_eq!(r[20], generate_checksum(&r, 20));
    }

    #[test]
    fn query_block_reply_no_figure_zeros_data() {
        let r = query_block_reply(0x07, None);
        assert_eq!(&r[4..20], &[0u8; 16]);
    }

    #[test]
    fn write_block_reply_layout() {
        let r = write_block_reply(0x08);
        assert_eq!(&r[..4], &[0xaa, 0x02, 0x08, 0x00]);
        assert_eq!(r[4], generate_checksum(&r, 4));
    }

    #[test]
    fn figure_identifier_reply_layout() {
        let uid = [1, 2, 3, 4, 5, 6, 7];
        let r = figure_identifier_reply(0x09, Some(&uid));
        assert_eq!(&r[..4], &[0xaa, 0x09, 0x09, 0x00]);
        assert_eq!(&r[4..11], &uid);
        assert_eq!(r[11], generate_checksum(&r, 11));
    }

    #[test]
    fn figure_change_response_added_vs_removed() {
        let added = figure_change_response(2, 0x42, false);
        assert_eq!(&added[..6], &[0xab, 0x04, 2, 0x09, 0x42, 0x00]);
        assert_eq!(added[6], generate_checksum(&added, 6));

        let removed = figure_change_response(3, 0x77, true);
        assert_eq!(&removed[..6], &[0xab, 0x04, 3, 0x09, 0x77, 0x01]);
    }

    #[test]
    fn present_figures_layout_slot_bases() {
        // slot 0 → base 0x10, slot 1..=3 → base 0x20, slot 4..=8 → base 0x30.
        let figs = [
            (true, 0x01u8), // i=0 → 0x10 + 0x01 = 0x11
            (false, 0u8),   // i=1, absent
            (true, 0x02),   // i=2 → 0x20 + 0x02 = 0x22
            (false, 0u8),
            (true, 0x03),   // i=4 → 0x30 + 0x03 = 0x33
        ];
        let r = present_figures_reply(0x01, &figs);
        assert_eq!(r[0], 0xaa);
        // 3 present figures × 2 bytes each = 6 payload bytes at offsets 3..9.
        assert_eq!(r[3], 0x11);
        assert_eq!(r[4], 0x09);
        assert_eq!(r[5], 0x22);
        assert_eq!(r[6], 0x09);
        assert_eq!(r[7], 0x33);
        assert_eq!(r[8], 0x09);
        // r[1] = x - 2 = 9 - 2 = 7.
        assert_eq!(r[1], 7);
        assert_eq!(r[2], 0x01);
    }

    #[test]
    fn aes_key_from_sha1_reverses_byte_order_per_group() {
        let sha1: [u8; 20] = [
            0x01, 0x02, 0x03, 0x04, // group 0 → reversed: 0x04, 0x03, 0x02, 0x01
            0x10, 0x20, 0x30, 0x40, // group 1 → 0x40, 0x30, 0x20, 0x10
            0xA0, 0xB0, 0xC0, 0xD0, // group 2 → 0xD0, 0xC0, 0xB0, 0xA0
            0x11, 0x22, 0x33, 0x44, // group 3 → 0x44, 0x33, 0x22, 0x11
            0xAA, 0xBB, 0xCC, 0xDD, // tail: ignored
        ];
        let key = aes_key_from_sha1_output(&sha1);
        assert_eq!(
            key,
            [
                0x04, 0x03, 0x02, 0x01, 0x40, 0x30, 0x20, 0x10, 0xD0, 0xC0, 0xB0, 0xA0, 0x44, 0x33,
                0x22, 0x11
            ]
        );
    }

    #[test]
    fn extract_figure_number_reads_24_bit_be() {
        let block: [u8; 16] = [
            0x00, 0x12, 0x34, 0x56, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        assert_eq!(extract_figure_number(&block), 0x0012_3456);
    }
}
