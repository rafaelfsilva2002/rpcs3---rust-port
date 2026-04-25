//! `rpcs3-io-skylander` — Rust port of `rpcs3/Emu/Io/Skylander.cpp`.
//!
//! Skylanders PortalMaster (Activision) — 8-slot NFC portal with a
//! queued status broadcast. Games query `Q<slot><block>` to read 16-byte
//! chunks from a 1024-byte figure, `W` to write, `A` to activate, `C r g
//! b` to set LEDs, `M` to probe audio firmware, `R` to shut down. Status
//! is packed as two bits per slot (8 slots → 16-bit mask).
//!
//! Frozen behavior:
//!
//! - USB descriptor constants (cpp:197..202): VID=0x1430, PID=0x0150,
//!   bcdDevice=0x0100, HID 0x0111.
//! - 8 figure slots × 1024-byte (0x40 × 0x10) storage (cpp:19).
//! - `get_status` packs `status_i` into bits `2*i..2*i+1` starting from
//!   slot 7 (cpp:78..90), writes `[0x53, status_lo, status_hi, 0, 0,
//!   interrupt_counter++, 0x01, ...zeros]` to the 32-byte reply.
//! - `query_block` reply: `['Q', (0x10 | slot) if present else slot,
//!   block, data...16]`.
//! - `write_block` reply: `['W', (0x10 | slot) if present else slot,
//!   block]`.
//! - Status queue drain: `activate` pushes `3, 1` for every present
//!   figure; `deactivate` collapses the queue to the last value and
//!   masks with `& 1`; `remove_skylander` pushes `2, 0` and sets status
//!   to 2 (cpp:139..154).
//! - Load slot selection: prefer the slot whose `last_id` matches the
//!   figure's serial; else the lowest free slot (cpp:163..180).
//! - `A` activate response: `[0x41, seq, 0xFF, 0x77, zeros...]`.
//! - `R` shutdown response: `[0x52, 0x02, 0x18, zeros...]`.
//! - `M` audio firmware response: `[0x4D, seq, 0x00, 0x19]`.

/// Number of figure slots on the real portal (cpp loop `for int i = 7; i >= 0`).
pub const SLOT_COUNT: usize = 8;

/// Storage per figure (cpp:19 — 0x40 pages × 16 bytes).
pub const FIGURE_DATA_SIZE: usize = 0x40 * 0x10;

pub const REPLY_SIZE: usize = 0x20;

// USB descriptor constants (cpp:197..202).
pub const USB_VID: u16 = 0x1430;
pub const USB_PID: u16 = 0x0150;
pub const USB_BCD_DEVICE: u16 = 0x0100;
pub const USB_BCD_USB: u16 = 0x0200;
pub const USB_MAX_PACKET_SIZE_0: u8 = 0x40;
pub const USB_CONFIG_W_TOTAL_LENGTH: u16 = 0x0029;
pub const USB_CONFIG_MAX_POWER: u8 = 0xFA;
pub const USB_INTERFACE_CLASS_HID: u8 = 0x03;
pub const USB_HID_BCD: u16 = 0x0111;
pub const USB_HID_DESCRIPTOR_LENGTH: u16 = 0x001d;
pub const USB_ENDPOINT_IN_ADDRESS: u8 = 0x81;
pub const USB_ENDPOINT_OUT_ADDRESS: u8 = 0x02;
pub const USB_ENDPOINT_W_MAX_PACKET_SIZE: u16 = 0x0040;
pub const USB_ENDPOINT_B_INTERVAL: u8 = 0x01;

pub const INTERRUPT_LATENCY_US_DEFAULT: u64 = 22_000;
pub const INTERRUPT_LATENCY_US_AUDIO: u64 = 1_000;
pub const CONTROL_LATENCY_US: u64 = 100;

/// A single figure slot. `status` is a 2-bit state:
/// bit 0 = present-on-portal, bit 1 = recently-placed/removed event pending.
#[derive(Debug, Clone)]
pub struct SkylanderSlot {
    pub status: u8,
    pub last_id: u32,
    pub queued_status: std::collections::VecDeque<u8>,
    pub data: std::boxed::Box<[u8; FIGURE_DATA_SIZE]>,
}

impl Default for SkylanderSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl SkylanderSlot {
    pub fn new() -> Self {
        Self {
            status: 0,
            last_id: 0,
            queued_status: std::collections::VecDeque::new(),
            data: std::boxed::Box::new([0u8; FIGURE_DATA_SIZE]),
        }
    }
}

/// Full portal state.
pub struct SkyPortal {
    pub slots: [SkylanderSlot; SLOT_COUNT],
    pub activated: bool,
    pub interrupt_counter: u8,
    pub leds: (u8, u8, u8),
}

impl SkyPortal {
    pub fn new() -> Self {
        Self {
            slots: core::array::from_fn(|_| SkylanderSlot::new()),
            activated: false,
            interrupt_counter: 0,
            leds: (0, 0, 0),
        }
    }

    /// `activate()` (cpp:23..43). On transition inactive→active, enqueue
    /// `3, 1` for every present figure so the next few status polls
    /// announce the placements to the game.
    pub fn activate(&mut self) {
        if self.activated {
            return;
        }
        for s in self.slots.iter_mut() {
            if s.status & 1 != 0 {
                s.queued_status.push_back(3);
                s.queued_status.push_back(1);
            }
        }
        self.activated = true;
    }

    /// `deactivate()` (cpp:45..62). Collapses each slot's queue down to
    /// the last enqueued value, then masks with `& 1` so only "present"
    /// state survives the shutdown.
    pub fn deactivate(&mut self) {
        for s in self.slots.iter_mut() {
            if let Some(&last) = s.queued_status.back() {
                s.status = last;
                s.queued_status.clear();
            }
            s.status &= 1;
        }
        self.activated = false;
    }

    /// `set_leds(r, g, b)` (cpp:64..70).
    pub fn set_leds(&mut self, r: u8, g: u8, b: u8) {
        self.leds = (r, g, b);
    }

    /// `get_status(reply)` (cpp:72..97). Packs 2 bits per slot into
    /// `reply[1..=2]` starting from slot 7 down to slot 0, increments
    /// the interrupt counter, and sets the magic bytes.
    pub fn get_status(&mut self, reply: &mut [u8; REPLY_SIZE]) {
        let mut status: u16 = 0;
        for i in (0..SLOT_COUNT).rev() {
            let s = &mut self.slots[i];
            if let Some(next) = s.queued_status.pop_front() {
                s.status = next;
            }
            status <<= 2;
            status |= u16::from(s.status);
        }

        reply.fill(0);
        reply[0] = 0x53;
        reply[1] = (status & 0xFF) as u8;
        reply[2] = ((status >> 8) & 0xFF) as u8;
        reply[5] = self.interrupt_counter;
        self.interrupt_counter = self.interrupt_counter.wrapping_add(1);
        reply[6] = 0x01;
    }

    /// `query_block(sky_num, block, reply)` (cpp:99..116). 16-byte data
    /// copy when the slot is present; otherwise only the 3-byte header.
    pub fn query_block(&self, sky_num: u8, block: u8, reply: &mut [u8; REPLY_SIZE]) {
        assert!(usize::from(sky_num) < SLOT_COUNT);
        assert!(block < 0x40);
        let slot = &self.slots[usize::from(sky_num)];
        reply[0] = b'Q';
        reply[2] = block;
        if slot.status & 1 != 0 {
            reply[1] = 0x10 | sky_num;
            let start = usize::from(block) * 16;
            reply[3..3 + 16].copy_from_slice(&slot.data[start..start + 16]);
        } else {
            reply[1] = sky_num;
            for b in reply[3..3 + 16].iter_mut() {
                *b = 0;
            }
        }
    }

    /// `write_block(sky_num, block, payload, reply)` (cpp:118..137).
    pub fn write_block(
        &mut self,
        sky_num: u8,
        block: u8,
        payload: &[u8; 16],
        reply: &mut [u8; REPLY_SIZE],
    ) {
        assert!(usize::from(sky_num) < SLOT_COUNT);
        assert!(block < 0x40);
        let slot = &mut self.slots[usize::from(sky_num)];
        reply[0] = b'W';
        reply[2] = block;
        if slot.status & 1 != 0 {
            reply[1] = 0x10 | sky_num;
            let start = usize::from(block) * 16;
            slot.data[start..start + 16].copy_from_slice(payload);
        } else {
            reply[1] = sky_num;
        }
    }

    /// `remove_skylander(sky_num)` (cpp:139..154). Marks the slot
    /// transient-removing (status=2) and queues the `2, 0` transition
    /// the game will observe next. Returns whether removal actually
    /// happened.
    pub fn remove_skylander(&mut self, sky_num: u8) -> bool {
        assert!(usize::from(sky_num) < SLOT_COUNT);
        let slot = &mut self.slots[usize::from(sky_num)];
        if slot.status & 1 != 0 {
            slot.status = 2;
            slot.queued_status.push_back(2);
            slot.queued_status.push_back(0);
            true
        } else {
            false
        }
    }

    /// `load_skylander(buf)` (cpp:156..192). Picks the slot matching
    /// `last_id == serial`, else the lowest free slot. Returns the
    /// chosen slot index, or `None` if every slot is occupied.
    pub fn load_skylander(&mut self, buf: &[u8; FIGURE_DATA_SIZE]) -> Option<u8> {
        let sky_serial = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let mut found_slot: u8 = 0xFF;
        for i in 0..SLOT_COUNT as u8 {
            let s = &self.slots[usize::from(i)];
            if s.status & 1 == 0 {
                if s.last_id == sky_serial {
                    found_slot = i;
                    break;
                }
                if i < found_slot {
                    found_slot = i;
                }
            }
        }
        if found_slot == 0xFF {
            return None;
        }
        let s = &mut self.slots[usize::from(found_slot)];
        s.data.copy_from_slice(buf);
        s.status = 3;
        s.queued_status.push_back(3);
        s.queued_status.push_back(1);
        s.last_id = sky_serial;
        Some(found_slot)
    }
}

impl Default for SkyPortal {
    fn default() -> Self {
        Self::new()
    }
}

/// Canned replies for control transfers. Byte-exact from cpp:240..305.
pub mod replies {
    use super::REPLY_SIZE;

    /// `A` activate — `[0x41, seq, 0xFF, 0x77, zeros...]`.
    #[must_use]
    pub fn activate(sequence: u8) -> [u8; REPLY_SIZE] {
        let mut r = [0u8; REPLY_SIZE];
        r[0] = 0x41;
        r[1] = sequence;
        r[2] = 0xFF;
        r[3] = 0x77;
        r
    }

    /// `R` shutdown — `[0x52, 0x02, 0x18, zeros...]`.
    #[must_use]
    pub fn shutdown() -> [u8; REPLY_SIZE] {
        let mut r = [0u8; REPLY_SIZE];
        r[0] = 0x52;
        r[1] = 0x02;
        r[2] = 0x18;
        r
    }

    /// `M` audio firmware version — `[0x4D, seq, 0x00, 0x19, zeros...]`.
    #[must_use]
    pub fn audio_firmware(sequence: u8) -> [u8; REPLY_SIZE] {
        let mut r = [0u8; REPLY_SIZE];
        r[0] = 0x4D;
        r[1] = sequence;
        r[2] = 0x00;
        r[3] = 0x19;
        r
    }

    /// `J` sync status — only the tag byte.
    #[must_use]
    pub fn sync_status() -> [u8; REPLY_SIZE] {
        let mut r = [0u8; REPLY_SIZE];
        r[0] = 0x4A;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants() {
        assert_eq!(SLOT_COUNT, 8);
        assert_eq!(FIGURE_DATA_SIZE, 1024);
        assert_eq!(REPLY_SIZE, 32);
        assert_eq!(USB_VID, 0x1430);
        assert_eq!(USB_PID, 0x0150);
    }

    #[test]
    fn activate_enqueues_for_present_figures_only() {
        let mut p = SkyPortal::new();
        p.slots[0].status = 1;
        p.slots[3].status = 0;
        p.slots[5].status = 1;
        p.activate();
        assert!(p.activated);
        assert_eq!(p.slots[0].queued_status.len(), 2);
        assert!(p.slots[3].queued_status.is_empty());
        assert_eq!(p.slots[5].queued_status.len(), 2);
    }

    #[test]
    fn activate_idempotent() {
        let mut p = SkyPortal::new();
        p.slots[0].status = 1;
        p.activate();
        let queued = p.slots[0].queued_status.len();
        p.activate();
        assert_eq!(p.slots[0].queued_status.len(), queued, "second activate is a no-op");
    }

    #[test]
    fn deactivate_collapses_queue_and_masks_with_1() {
        let mut p = SkyPortal::new();
        p.slots[0].status = 3;
        p.slots[0].queued_status.push_back(3);
        p.slots[0].queued_status.push_back(7); // arbitrary "last"
        p.activated = true;
        p.deactivate();
        // Last queued was 7, then masked to 7 & 1 = 1.
        assert_eq!(p.slots[0].status, 1);
        assert!(p.slots[0].queued_status.is_empty());
        assert!(!p.activated);
    }

    #[test]
    fn get_status_packs_slot_bits_from_high_to_low() {
        let mut p = SkyPortal::new();
        p.slots[0].status = 0b01;
        p.slots[1].status = 0b10;
        p.slots[7].status = 0b11;
        let mut reply = [0u8; REPLY_SIZE];
        p.get_status(&mut reply);
        assert_eq!(reply[0], 0x53);
        // bits: slot7=bits 14-15 (0b11), slot1=bits 2-3 (0b10), slot0=bits 0-1 (0b01)
        // So status = (3 << 14) | (0b10 << 2) | 0b01 = 0xC009
        let status = u16::from_le_bytes([reply[1], reply[2]]);
        assert_eq!(status, 0xC000 | (0b10 << 2) | 0b01);
        assert_eq!(reply[5], 0);
        assert_eq!(reply[6], 0x01);

        // Counter bumps.
        let mut reply2 = [0u8; REPLY_SIZE];
        p.get_status(&mut reply2);
        assert_eq!(reply2[5], 1);
    }

    #[test]
    fn get_status_drains_queued_status_per_call() {
        let mut p = SkyPortal::new();
        p.slots[0].status = 0;
        p.slots[0].queued_status.push_back(3);
        p.slots[0].queued_status.push_back(1);

        let mut r = [0u8; REPLY_SIZE];
        p.get_status(&mut r);
        assert_eq!(p.slots[0].status, 3);
        p.get_status(&mut r);
        assert_eq!(p.slots[0].status, 1);
        p.get_status(&mut r);
        // Queue empty — status stays.
        assert_eq!(p.slots[0].status, 1);
    }

    #[test]
    fn query_block_copies_when_present() {
        let mut p = SkyPortal::new();
        p.slots[2].status = 1;
        for i in 0..16 {
            p.slots[2].data[16 * 3 + i] = 0xA0 + i as u8;
        }
        let mut r = [0u8; REPLY_SIZE];
        p.query_block(2, 3, &mut r);
        assert_eq!(r[0], b'Q');
        assert_eq!(r[1], 0x12, "0x10 | slot");
        assert_eq!(r[2], 3);
        assert_eq!(&r[3..19], &(0xA0u8..).take(16).collect::<std::vec::Vec<_>>()[..]);
    }

    #[test]
    fn query_block_returns_bare_header_when_absent() {
        let p = SkyPortal::new();
        let mut r = [0u8; REPLY_SIZE];
        p.query_block(4, 0x10, &mut r);
        assert_eq!(r[0], b'Q');
        assert_eq!(r[1], 4);
        assert_eq!(r[2], 0x10);
        // No data copied.
        assert_eq!(&r[3..19], &[0u8; 16]);
    }

    #[test]
    fn write_block_mutates_when_present() {
        let mut p = SkyPortal::new();
        p.slots[1].status = 1;
        let payload = [0x55u8; 16];
        let mut r = [0u8; REPLY_SIZE];
        p.write_block(1, 5, &payload, &mut r);
        assert_eq!(r[0], b'W');
        assert_eq!(r[1], 0x11);
        assert_eq!(r[2], 5);
        assert_eq!(&p.slots[1].data[80..96], &[0x55u8; 16]);
    }

    #[test]
    fn remove_skylander_queues_2_0() {
        let mut p = SkyPortal::new();
        p.slots[0].status = 1;
        assert!(p.remove_skylander(0));
        assert_eq!(p.slots[0].status, 2);
        assert_eq!(p.slots[0].queued_status.len(), 2);
        assert!(!p.remove_skylander(0), "already transient");
    }

    #[test]
    fn load_skylander_prefers_last_id_match() {
        let mut p = SkyPortal::new();
        p.slots[3].last_id = 0x1234_5678;
        let mut buf = [0u8; FIGURE_DATA_SIZE];
        buf[0..4].copy_from_slice(&0x1234_5678u32.to_le_bytes());
        let slot = p.load_skylander(&buf).expect("slot");
        assert_eq!(slot, 3, "matching last_id wins over lowest free");
        assert_eq!(p.slots[3].status, 3);
    }

    #[test]
    fn load_skylander_falls_back_to_lowest_free() {
        let mut p = SkyPortal::new();
        p.slots[0].status = 1; // occupied
        let mut buf = [0u8; FIGURE_DATA_SIZE];
        buf[0..4].copy_from_slice(&0xAAAA_AAAAu32.to_le_bytes());
        let slot = p.load_skylander(&buf).expect("slot");
        assert_eq!(slot, 1);
    }

    #[test]
    fn replies_activate_shutdown_audio_sync() {
        let a = replies::activate(0x42);
        assert_eq!(&a[..4], &[0x41, 0x42, 0xFF, 0x77]);

        let s = replies::shutdown();
        assert_eq!(&s[..3], &[0x52, 0x02, 0x18]);

        let m = replies::audio_firmware(0x10);
        assert_eq!(&m[..4], &[0x4D, 0x10, 0x00, 0x19]);

        let j = replies::sync_status();
        assert_eq!(j[0], 0x4A);
    }
}
