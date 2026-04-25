//! `rpcs3-io-kamenrider` — Rust port of `rpcs3/Emu/Io/KamenRider.cpp`.
//!
//! Kamen Rider Summoner "rider gate" NFC-portal emulator. Games talk to the
//! portal through short USB control messages; the portal replies with a
//! 64-byte buffer whose fields follow a fixed shape. We freeze:
//!
//! - `generate_checksum(buf, num)` (cpp:19..28) — simple 8-bit sum wrap.
//! - `get_blank_response` preamble (cpp:42..46): `[0x55, 0x02, cmd, seq, checksum]`.
//! - Wake reply (cpp:48..55): 29-byte magic sequence the portal emits once
//!   on first talk.
//! - `get_list_tags` — writes 9-byte records (`0x09` + 7-byte uid) for every
//!   `present` figure into `reply[4..]`, then checksum at `reply[index]`.
//!   `reply[1]` (payload length) grows by 8 per figure (cpp:58..75).
//! - `query_block` (cpp:77..94) — `reply = [0x55, 0x13, cmd, seq, 0x00]`
//!   plus 16 data bytes at `reply[5..21]` when `sector<5 && block<4`, then
//!   checksum at `reply[21]`.
//! - `write_block` (cpp:96..112) — mutates the 320-byte figure data and
//!   emits a blank response.
//! - Figure data layout: 5 sectors × 4 blocks × 16 bytes = 320 bytes
//!   (cpp:16 `0x14 * 0x10`).
//! - Figure-removed response (cpp:141..144): `[0x56, 0x09, 0x09, 0x00,
//!   uid..7, checksum]` at `reply[11]`.
//!
//! Filesystem I/O and USB wiring stay out; this crate gives a frontend the
//! byte-exact reply machinery.

use core::cmp::Ordering;

/// Maximum number of figure slots the gate tracks (cpp:39 — falls back to
/// slot 7, so 8 slots total 0..=7).
pub const MAX_FIGURES: usize = 8;

/// 5 sectors × 4 blocks × 16 bytes of NFC data per figure (cpp:16).
pub const FIGURE_DATA_SIZE: usize = 5 * 4 * 16;

pub const REPLY_SIZE: usize = 64;

/// 8-bit wrap-sum checksum (cpp:19..28).
#[must_use]
pub fn generate_checksum(buf: &[u8], num_bytes: usize) -> u8 {
    assert!(num_bytes <= buf.len());
    let mut sum: u32 = 0;
    for &b in &buf[..num_bytes] {
        sum = sum.wrapping_add(u32::from(b));
    }
    (sum & 0xFF) as u8
}

/// `get_blank_response(cmd, seq)` (cpp:42..46): the 5-byte preamble that
/// every non-data reply starts with.
#[must_use]
pub fn blank_response(command: u8, sequence: u8) -> [u8; REPLY_SIZE] {
    let mut reply = [0u8; REPLY_SIZE];
    reply[0] = 0x55;
    reply[1] = 0x02;
    reply[2] = command;
    reply[3] = sequence;
    reply[4] = generate_checksum(&reply, 4);
    reply
}

/// 29-byte wake reply (cpp:48..55). Emitted once when the game pokes the
/// gate after boot.
#[must_use]
pub fn wake_response(command: u8, sequence: u8) -> [u8; REPLY_SIZE] {
    let mut reply = [0u8; REPLY_SIZE];
    reply[0] = 0x55;
    reply[1] = 0x1a;
    reply[2] = command;
    reply[3] = sequence;
    // Tail bytes verbatim from cpp:53..55.
    let tail: [u8; 25] = [
        0x00, 0x07, 0x00, 0x03, 0x02, 0x09, 0x20, 0x03, 0xf5, 0x00, 0x19, 0x42, 0x52,
        0xb7, 0xb9, 0xa1, 0xae, 0x2b, 0x88, 0x42, 0x05, 0xfe, 0xe0, 0x1c, 0xac,
    ];
    reply[4..4 + tail.len()].copy_from_slice(&tail);
    reply
}

/// Minimal figure state used by `list_tags` / `query_block` / `write_block`.
#[derive(Debug, Clone)]
pub struct FigureSlot {
    pub present: bool,
    pub uid: [u8; 7],
    pub data: [u8; FIGURE_DATA_SIZE],
}

impl Default for FigureSlot {
    fn default() -> Self {
        Self { present: false, uid: [0; 7], data: [0; FIGURE_DATA_SIZE] }
    }
}

/// Writes a `list_tags` reply (cpp:58..75). One 9-byte record per present
/// figure: `[0x09, uid[0..7]]`. `reply[1]` (payload length) grows by 8 per
/// figure starting from the base `0x02`; the cpp `+= 8` reflects the 7 uid
/// bytes plus the `0x09` tag byte minus one byte of preamble that already
/// exists — preserving byte-exact reply.
pub fn list_tags(
    figures: &[FigureSlot],
    command: u8,
    sequence: u8,
) -> [u8; REPLY_SIZE] {
    let mut reply = [0u8; REPLY_SIZE];
    reply[0] = 0x55;
    reply[1] = 0x02;
    reply[2] = command;
    reply[3] = sequence;

    let mut index = 4usize;
    for fig in figures {
        if !fig.present {
            continue;
        }
        // 9-byte record: tag + 7 data bytes. The cpp writes
        // `figure.data.data()` (not the uid). We mirror that.
        reply[index] = 0x09;
        reply[index + 1..index + 8].copy_from_slice(&fig.data[..7]);
        index += 8;
        reply[1] = reply[1].wrapping_add(8);
    }
    reply[index] = generate_checksum(&reply, index);
    reply
}

/// `query_block(uid, sector, block)` (cpp:77..94). Returns the 64-byte
/// reply; `reply[5..21]` carries the 16-byte block when inputs are valid
/// and the figure is present.
pub fn query_block(
    figures: &[FigureSlot],
    command: u8,
    sequence: u8,
    uid: &[u8; 7],
    sector: u8,
    block: u8,
) -> [u8; REPLY_SIZE] {
    let mut reply = [0u8; REPLY_SIZE];
    reply[0] = 0x55;
    reply[1] = 0x13;
    reply[2] = command;
    reply[3] = sequence;
    reply[4] = 0x00;

    if let Some(fig) = find_figure_by_uid(figures, uid) {
        if sector < 5 && block < 4 {
            let start = usize::from(sector) * 4 * 16 + usize::from(block) * 16;
            reply[5..21].copy_from_slice(&fig.data[start..start + 16]);
        }
    }
    reply[21] = generate_checksum(&reply, 21);
    reply
}

/// `write_block(uid, sector, block, data)` (cpp:96..112). Mutates the
/// figure's data then returns a blank response.
pub fn write_block(
    figures: &mut [FigureSlot],
    command: u8,
    sequence: u8,
    uid: &[u8; 7],
    sector: u8,
    block: u8,
    to_write: &[u8; 16],
) -> [u8; REPLY_SIZE] {
    if let Some(fig) = find_figure_by_uid_mut(figures, uid) {
        if sector < 5 && block < 4 {
            let start = usize::from(sector) * 4 * 16 + usize::from(block) * 16;
            fig.data[start..start + 16].copy_from_slice(to_write);
        }
    }
    blank_response(command, sequence)
}

/// `figure_removed` async message (cpp:141..144). 12-byte payload with
/// checksum at `reply[11]`.
#[must_use]
pub fn figure_removed_response(uid: &[u8; 7]) -> [u8; REPLY_SIZE] {
    let mut reply = [0u8; REPLY_SIZE];
    reply[0] = 0x56;
    reply[1] = 0x09;
    reply[2] = 0x09;
    reply[3] = 0x00;
    reply[4..4 + 7].copy_from_slice(uid);
    reply[11] = generate_checksum(&reply, 11);
    reply
}

/// `get_figure_by_uid` cpp:30..40 — linear scan, fall back to slot 7 if no
/// match. Returns `None` when the fallback slot itself isn't present so
/// the caller can decide whether to treat that as "not found" or "use
/// slot 7".
#[must_use]
pub fn find_figure_by_uid<'a>(
    figures: &'a [FigureSlot],
    uid: &[u8; 7],
) -> Option<&'a FigureSlot> {
    for fig in figures {
        if matches_uid(&fig.uid, uid) == Ordering::Equal {
            return Some(fig);
        }
    }
    // Fall back to slot 7 per cpp:39 (index into slot array).
    figures.get(7).filter(|f| f.present)
}

fn find_figure_by_uid_mut<'a>(
    figures: &'a mut [FigureSlot],
    uid: &[u8; 7],
) -> Option<&'a mut FigureSlot> {
    let mut match_idx: Option<usize> = None;
    for (i, fig) in figures.iter().enumerate() {
        if matches_uid(&fig.uid, uid) == Ordering::Equal {
            match_idx = Some(i);
            break;
        }
    }
    if let Some(i) = match_idx {
        return figures.get_mut(i);
    }
    figures.get_mut(7).filter(|f| f.present)
}

fn matches_uid(a: &[u8; 7], b: &[u8; 7]) -> Ordering {
    for i in 0..7 {
        match a[i].cmp(&b[i]) {
            Ordering::Equal => continue,
            other => return other,
        }
    }
    Ordering::Equal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_sums_and_masks() {
        assert_eq!(generate_checksum(&[0; 4], 4), 0);
        assert_eq!(generate_checksum(&[1, 2, 3, 4], 4), 10);
        // Sum > 255 wraps.
        assert_eq!(generate_checksum(&[0xFF, 0xFF], 2), 0xFE);
    }

    #[test]
    fn blank_response_preamble_and_checksum() {
        let r = blank_response(0x42, 0x07);
        assert_eq!(r[0], 0x55);
        assert_eq!(r[1], 0x02);
        assert_eq!(r[2], 0x42);
        assert_eq!(r[3], 0x07);
        // checksum = (0x55 + 0x02 + 0x42 + 0x07) & 0xFF = 0xA0
        assert_eq!(r[4], 0xA0);
        // Rest must be zeroed.
        assert_eq!(r[5], 0);
        assert_eq!(r[63], 0);
    }

    #[test]
    fn wake_response_magic_bytes() {
        let r = wake_response(0x10, 0x00);
        assert_eq!(r[0], 0x55);
        assert_eq!(r[1], 0x1a);
        assert_eq!(r[2], 0x10);
        // Last magic byte of the 29-byte payload per cpp:55.
        assert_eq!(r[28], 0xac);
        assert_eq!(r[27], 0x1c);
        assert_eq!(r[29], 0, "after magic comes zeros");
    }

    #[test]
    fn list_tags_no_figures_has_base_length() {
        let figures: Vec<FigureSlot> = (0..8).map(|_| FigureSlot::default()).collect();
        let r = list_tags(&figures, 0x30, 0x01);
        assert_eq!(r[1], 0x02, "base length with no figures present");
        // Checksum of [0x55, 0x02, 0x30, 0x01] = 0x88
        assert_eq!(r[4], 0x88);
    }

    #[test]
    fn list_tags_one_figure_bumps_length() {
        let mut figures: Vec<FigureSlot> = (0..8).map(|_| FigureSlot::default()).collect();
        figures[0].present = true;
        figures[0].data[0] = 0xAA;
        figures[0].data[1] = 0xBB;
        figures[0].data[2] = 0xCC;
        figures[0].data[3] = 0xDD;
        figures[0].data[4] = 0xEE;
        figures[0].data[5] = 0xFF;
        figures[0].data[6] = 0x11;
        let r = list_tags(&figures, 0x30, 0x01);
        assert_eq!(r[1], 0x0A, "base 0x02 + 8 per figure");
        assert_eq!(r[4], 0x09, "tag byte");
        assert_eq!(&r[5..12], &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11]);
    }

    #[test]
    fn query_block_copies_16_bytes_when_valid() {
        let mut figures: Vec<FigureSlot> = (0..8).map(|_| FigureSlot::default()).collect();
        figures[2].present = true;
        figures[2].uid = [1, 2, 3, 4, 5, 6, 7];
        // Sector 1, block 2 → offset 1*64 + 2*16 = 96.
        for i in 0..16 {
            figures[2].data[96 + i] = 0xA0 + i as u8;
        }
        let r = query_block(&figures, 0x40, 0x02, &[1, 2, 3, 4, 5, 6, 7], 1, 2);
        assert_eq!(r[0], 0x55);
        assert_eq!(r[1], 0x13);
        assert_eq!(&r[5..21], &(0xA0u8..).take(16).collect::<Vec<_>>()[..]);
    }

    #[test]
    fn query_block_returns_zero_data_when_absent() {
        let figures: Vec<FigureSlot> = (0..8).map(|_| FigureSlot::default()).collect();
        let r = query_block(&figures, 0x40, 0x02, &[9; 7], 0, 0);
        assert_eq!(&r[5..21], &[0u8; 16]);
    }

    #[test]
    fn query_block_ignores_out_of_range() {
        let mut figures: Vec<FigureSlot> = (0..8).map(|_| FigureSlot::default()).collect();
        figures[0].present = true;
        figures[0].uid = [1; 7];
        figures[0].data[0] = 0xFF;
        let r = query_block(&figures, 0x40, 0x02, &[1; 7], 5, 0);
        assert_eq!(&r[5..21], &[0u8; 16], "sector >= 5 should not fill data");
    }

    #[test]
    fn write_block_mutates_figure_data() {
        let mut figures: Vec<FigureSlot> = (0..8).map(|_| FigureSlot::default()).collect();
        figures[3].present = true;
        figures[3].uid = [9; 7];
        let payload = [0x55u8; 16];
        let r = write_block(&mut figures, 0x50, 0x03, &[9; 7], 2, 3, &payload);
        assert_eq!(r[1], 0x02, "write returns a blank response");
        // Sector 2, block 3 → offset 2*64 + 3*16 = 176.
        assert_eq!(&figures[3].data[176..192], &[0x55u8; 16]);
    }

    #[test]
    fn figure_removed_response_checksum() {
        let r = figure_removed_response(&[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(r[0], 0x56);
        assert_eq!(r[1], 0x09);
        assert_eq!(r[2], 0x09);
        assert_eq!(r[3], 0x00);
        assert_eq!(&r[4..11], &[1, 2, 3, 4, 5, 6, 7]);
        // checksum = (0x56 + 0x09 + 0x09 + 0x00 + 1+2+3+4+5+6+7) & 0xFF
        let expected = (0x56u16 + 0x09 + 0x09 + 0 + 1 + 2 + 3 + 4 + 5 + 6 + 7) & 0xFF;
        assert_eq!(r[11], expected as u8);
    }

    #[test]
    fn find_figure_by_uid_falls_back_to_slot_7() {
        let mut figures: Vec<FigureSlot> = (0..8).map(|_| FigureSlot::default()).collect();
        figures[7].present = true;
        figures[7].uid = [0xAA; 7];
        let f = find_figure_by_uid(&figures, &[0xBB; 7]).expect("fallback");
        assert_eq!(f.uid, [0xAA; 7]);
    }

    #[test]
    fn constants_match_cpp() {
        assert_eq!(FIGURE_DATA_SIZE, 0x14 * 0x10);
        assert_eq!(REPLY_SIZE, 64);
        assert_eq!(MAX_FIGURES, 8);
    }
}
