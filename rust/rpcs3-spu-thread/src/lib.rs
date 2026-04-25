//! `rpcs3-spu-thread` — Synergistic Processing Unit (SPU) state.
//!
//! Mirrors `rpcs3/Emu/Cell/SPUThread.h:627+` (`class spu_thread`) plus
//! the MFC constants from `rpcs3/Emu/Cell/MFC.h`.
//!
//! State container only — no execution loop, no JIT. Interpreter +
//! recompilers land in future `rpcs3-spu-*` crates.
//!
//! ## What is ABI-frozen
//!
//! * `SPU_ID_BASE = 0x0200_0000` (SPUThread.h:643).
//! * `SPU_LS_SIZE = 0x40000` = 256 KB Local Store per SPU
//!   (SPUThread.h:139).
//! * 128 × 128-bit GPRs (`std::array<v128, 128> gpr`).
//! * `spu_mfc_cmd` layout: 16 bytes, `{cmd: u8, tag: u8, size: u16, lsa: u32, eal: u32, eah: u32}`.
//! * MFC command opcodes (PUT=0x20, GET=0x40, GETLLAR=0xD0, PUTLLC=0xB4, …).
//! * MFC atomic status values.

use rpcs3_cpu_thread::{CpuState, ThreadClass};

// =====================================================================
// Constants
// =====================================================================

/// SPU thread-class discriminant.
pub const SPU_ID_BASE: u32 = 0x0200_0000;

/// Local Store size per SPU: 256 KB.
pub const SPU_LS_SIZE: usize = 0x40000;

/// Number of SPU general-purpose registers (128-bit each).
pub const SPU_GPR_COUNT: usize = 128;

/// MFC command queue depth (SPU-side).
pub const MFC_QUEUE_DEPTH: usize = 16;

/// Reservation block granularity shared with the PPU (128 bytes).
pub const RESERVATION_BLOCK: usize = 128;

// =====================================================================
// MFC opcodes (MFC.h:5-36)
// =====================================================================

/// MFC DMA command opcodes. See `enum MFC` at `MFC.h:5`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MfcCmd {
    Put = 0x20,
    PutB = 0x21,
    PutF = 0x22,
    PutL = 0x24,
    PutLB = 0x25,
    PutLF = 0x26,
    PutS = 0x28,
    PutBS = 0x29,
    PutFS = 0x2A,
    PutR = 0x30,
    PutRB = 0x31,
    PutRF = 0x32,
    PutRL = 0x34,
    PutRLB = 0x35,
    PutRLF = 0x36,
    Get = 0x40,
    GetB = 0x41,
    GetF = 0x42,
    GetL = 0x44,
    GetLB = 0x45,
    GetLF = 0x46,
    GetS = 0x48,
    GetBS = 0x49,
    GetFS = 0x4A,
    SndSig = 0xA0,
    SndSigB = 0xA1,
    SndSigF = 0xA2,
    PutLluc = 0xB0,
    Putllc = 0xB4,
    PutQLluc = 0xB8,
    Barrier = 0xC0,
    Eieio = 0xC8,
    Sync = 0xCC,
    GetLlar = 0xD0,
    /// Unknown / unimplemented. Used as fallback during decoding.
    Unknown = 0xFF,
}

impl MfcCmd {
    /// Convert a raw u8 to `MfcCmd`, returning `Unknown` for
    /// unrecognised opcodes.
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v {
            0x20 => Self::Put,
            0x21 => Self::PutB,
            0x22 => Self::PutF,
            0x24 => Self::PutL,
            0x25 => Self::PutLB,
            0x26 => Self::PutLF,
            0x28 => Self::PutS,
            0x29 => Self::PutBS,
            0x2A => Self::PutFS,
            0x30 => Self::PutR,
            0x31 => Self::PutRB,
            0x32 => Self::PutRF,
            0x34 => Self::PutRL,
            0x35 => Self::PutRLB,
            0x36 => Self::PutRLF,
            0x40 => Self::Get,
            0x41 => Self::GetB,
            0x42 => Self::GetF,
            0x44 => Self::GetL,
            0x45 => Self::GetLB,
            0x46 => Self::GetLF,
            0x48 => Self::GetS,
            0x49 => Self::GetBS,
            0x4A => Self::GetFS,
            0xA0 => Self::SndSig,
            0xA1 => Self::SndSigB,
            0xA2 => Self::SndSigF,
            0xB0 => Self::PutLluc,
            0xB4 => Self::Putllc,
            0xB8 => Self::PutQLluc,
            0xC0 => Self::Barrier,
            0xC8 => Self::Eieio,
            0xCC => Self::Sync,
            0xD0 => Self::GetLlar,
            _ => Self::Unknown,
        }
    }
}

/// Barrier/fence/list/start/result bit flags from `MFC_*_MASK` (MFC.h:25-29).
pub const MFC_BARRIER_MASK: u8 = 0x01;
pub const MFC_FENCE_MASK: u8 = 0x02;
pub const MFC_LIST_MASK: u8 = 0x04;
pub const MFC_START_MASK: u8 = 0x08;
pub const MFC_RESULT_MASK: u8 = 0x10;

// =====================================================================
// MFC atomic status (MFC.h:39-45)
// =====================================================================

/// Returned to the SPU via `ch_atomic_stat` on LL/SC completion.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MfcAtomicStatus {
    PutllcSuccess = 0,
    /// The SC failed because the reservation was lost.
    PutllcFailure = 1,
    PutllucSuccess = 2,
    GetllarSuccess = 4,
}

// =====================================================================
// MFC tag update operation (MFC.h:48-53)
// =====================================================================

#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MfcTagUpdate {
    Immediate = 0,
    Any = 1,
    All = 2,
}

// =====================================================================
// spu_mfc_cmd (MFC.h:89-99)
// =====================================================================

/// MFC command packet. 16 bytes on disk and in memory.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpuMfcCmd {
    pub cmd: u8,
    pub tag: u8,
    pub size: u16,
    pub lsa: u32,
    pub eal: u32,
    pub eah: u32,
}

const _: () = {
    assert!(core::mem::size_of::<SpuMfcCmd>() == 16);
};

// =====================================================================
// SpuThread — state container
// =====================================================================

pub struct SpuThread {
    /// Thread id (`SPU_ID_BASE | index`), matching `cpu_thread::id`.
    pub id: u32,

    /// Atomic state bitset, same as PPU/cpu_thread.
    pub state: CpuState,

    // -- Execution state --

    /// Current program counter (SPU Local-Store address).
    pub pc: u32,
    /// `base_pc` from recompiler bookkeeping.
    pub base_pc: u32,

    /// 128 × 128-bit registers.
    pub gpr: [u128; SPU_GPR_COUNT],

    /// Floating-point status/control register (simplified — real C++
    /// uses `SPU_FPSCR` with bit fields; we expose the packed form and
    /// will split it out when the interpreter crate needs to).
    pub fpscr: u32,

    // -- MFC --

    /// Current MFC command being assembled.
    pub ch_mfc_cmd: SpuMfcCmd,

    /// MFC command queue (up to 16 pending DMA ops).
    pub mfc_queue: [SpuMfcCmd; MFC_QUEUE_DEPTH],
    pub mfc_size: u32,
    pub mfc_barrier: u32,
    pub mfc_fence: u32,

    // -- Reservation (LL/SC) --

    pub rtime: u64,
    pub raddr: u32,
    pub rdata: [u8; RESERVATION_BLOCK],

    /// 256 KB Local Store. Boxed so the struct itself stays compact
    /// (stack-returning a `SpuThread` without the Box would overflow).
    pub ls: Box<[u8; SPU_LS_SIZE]>,

    // -- Channels --
    //
    // Stub fields for now. Real channel implementation (with blocking
    // reads, waiter wake-up, backing queues) is its own crate.
    pub ch_tag_mask: u32,
    pub ch_stall_mask: u32,
    pub snr_config: u64,

    /// Channel state exposed to `rdch`/`wrch`/`rchcnt` opcodes.
    pub channels: SpuChannels,
}

// =====================================================================
// SpuChannels — mailbox + event + signal state accessed via rdch/wrch
// =====================================================================

/// Channel numbers (subset the interpreter handles).
pub mod ch {
    /// Read event status (bit mask of pending events).
    pub const SPU_RDEVENTSTAT: u32 = 0;
    /// Write event mask.
    pub const SPU_WREVENTMASK: u32 = 1;
    /// Write event acknowledge (clear bits in stat).
    pub const SPU_WREVENTACK: u32 = 2;
    /// Read SNR1 (signal notify 1).
    pub const SPU_RDSIGNOTIFY1: u32 = 3;
    /// Read SNR2.
    pub const SPU_RDSIGNOTIFY2: u32 = 4;
    /// Write decrementer.
    pub const SPU_WRDEC: u32 = 7;
    /// Read decrementer.
    pub const SPU_RDDEC: u32 = 8;
    /// Read event mask.
    pub const SPU_RDEVENTMASK: u32 = 22;
    /// Read machine status.
    pub const SPU_RDMACHSTAT: u32 = 23;
    /// SPU → PPU outgoing mailbox.
    pub const SPU_WROUTMBOX: u32 = 28;
    /// PPU → SPU incoming mailbox.
    pub const SPU_RDINMBOX: u32 = 29;
    /// SPU → PPU outgoing interrupt mailbox.
    pub const SPU_WROUTINTRMBOX: u32 = 30;
}

/// State behind the SPU channel namespace. Dispatch reads/writes via
/// [`SpuChannels::read`] / [`SpuChannels::write`].
#[derive(Debug, Clone, Default)]
pub struct SpuChannels {
    /// Event status bitmap (read via RDEVENTSTAT).
    pub event_stat: u32,
    /// Event mask (wrch SPU_WrEventMask writes here, rdch reads).
    pub event_mask: u32,
    /// Two SNR slots (signal notify): read-only to the SPU.
    pub snr: [u32; 2],
    /// Decrementer — 32-bit down counter set by WRDEC, read by RDDEC.
    pub decrementer: u32,
    /// Machine status — read-only. Bit 0 = interrupt enable.
    pub machine_status: u32,
    /// Outgoing mailbox (SPU → PPU). Single slot.
    pub out_mbox: Option<u32>,
    /// Incoming mailbox (PPU → SPU). Single slot.
    pub in_mbox: Option<u32>,
    /// Outgoing interrupt mailbox (SPU → PPU, with IRQ).
    pub out_intr_mbox: Option<u32>,
}

/// Outcome of a channel op when the channel is empty/full. Matches
/// SPU hardware semantics: the SPU stalls until the channel is
/// drained/filled, but in our synchronous port we surface it as an
/// explicit enum the caller decides how to handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelStatus {
    Ok,
    WouldStall,
    BadChannel,
}

impl SpuChannels {
    /// Push a mailbox value from the PPU side. Returns true if there
    /// was room (slot was empty); false if the mailbox was already full.
    pub fn ppu_push_inmbox(&mut self, value: u32) -> bool {
        if self.in_mbox.is_some() {
            return false;
        }
        self.in_mbox = Some(value);
        true
    }

    /// Drain SPU → PPU mailbox. Returns None if empty.
    pub fn ppu_pop_outmbox(&mut self) -> Option<u32> {
        self.out_mbox.take()
    }

    /// Drain SPU → PPU interrupt mailbox.
    pub fn ppu_pop_out_intr_mbox(&mut self) -> Option<u32> {
        self.out_intr_mbox.take()
    }

    /// Called by the PPU/emu-core to trigger SNR1/2 on the SPU.
    pub fn signal(&mut self, which: usize, value: u32) -> bool {
        if which >= 2 { return false; }
        self.snr[which] |= value;
        // Mark signal event pending in event_stat bit (subset mapping).
        self.event_stat |= match which { 0 => 0x00000001, _ => 0x00000002 };
        true
    }

    /// `rdch` — blocking read from channel. Our port returns
    /// [`ChannelStatus::WouldStall`] instead of blocking.
    pub fn read(&mut self, channel: u32) -> Result<u32, ChannelStatus> {
        use ch::*;
        match channel {
            SPU_RDEVENTSTAT => Ok(self.event_stat & self.event_mask),
            SPU_RDEVENTMASK => Ok(self.event_mask),
            SPU_RDSIGNOTIFY1 => {
                let v = self.snr[0];
                self.snr[0] = 0;
                self.event_stat &= !0x00000001;
                Ok(v)
            }
            SPU_RDSIGNOTIFY2 => {
                let v = self.snr[1];
                self.snr[1] = 0;
                self.event_stat &= !0x00000002;
                Ok(v)
            }
            SPU_RDDEC => Ok(self.decrementer),
            SPU_RDMACHSTAT => Ok(self.machine_status),
            SPU_RDINMBOX => {
                match self.in_mbox.take() {
                    Some(v) => Ok(v),
                    None => Err(ChannelStatus::WouldStall),
                }
            }
            _ => Err(ChannelStatus::BadChannel),
        }
    }

    /// `wrch` — blocking write.
    pub fn write(&mut self, channel: u32, value: u32) -> Result<(), ChannelStatus> {
        use ch::*;
        match channel {
            SPU_WREVENTMASK => { self.event_mask = value; Ok(()) }
            SPU_WREVENTACK => { self.event_stat &= !value; Ok(()) }
            SPU_WRDEC => { self.decrementer = value; Ok(()) }
            SPU_WROUTMBOX => {
                if self.out_mbox.is_some() {
                    return Err(ChannelStatus::WouldStall);
                }
                self.out_mbox = Some(value);
                Ok(())
            }
            SPU_WROUTINTRMBOX => {
                if self.out_intr_mbox.is_some() {
                    return Err(ChannelStatus::WouldStall);
                }
                self.out_intr_mbox = Some(value);
                Ok(())
            }
            _ => Err(ChannelStatus::BadChannel),
        }
    }

    /// `rchcnt` — how many values are currently readable (for read
    /// channels) or how many slots are free (for write channels).
    pub fn count(&self, channel: u32) -> Result<u32, ChannelStatus> {
        use ch::*;
        let count = match channel {
            // Read channels: 1 if data available.
            SPU_RDINMBOX => if self.in_mbox.is_some() { 1 } else { 0 },
            SPU_RDSIGNOTIFY1 => if self.snr[0] != 0 { 1 } else { 0 },
            SPU_RDSIGNOTIFY2 => if self.snr[1] != 0 { 1 } else { 0 },
            SPU_RDEVENTSTAT | SPU_RDEVENTMASK | SPU_RDDEC | SPU_RDMACHSTAT => 1,
            // Write channels: 1 slot available if not full.
            SPU_WREVENTMASK | SPU_WREVENTACK | SPU_WRDEC => 1,
            SPU_WROUTMBOX => if self.out_mbox.is_none() { 1 } else { 0 },
            SPU_WROUTINTRMBOX => if self.out_intr_mbox.is_none() { 1 } else { 0 },
            _ => return Err(ChannelStatus::BadChannel),
        };
        Ok(count)
    }
}

impl core::fmt::Debug for SpuThread {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SpuThread")
            .field("id", &format_args!("0x{:08x}", self.id))
            .field("pc", &format_args!("0x{:05x}", self.pc))
            .field("mfc_size", &self.mfc_size)
            .field("raddr", &format_args!("0x{:08x}", self.raddr))
            .field("ls_bytes", &self.ls.len())
            .finish_non_exhaustive()
    }
}

impl SpuThread {
    /// Fresh SPU thread. LS is zero-filled; all registers cleared.
    #[must_use]
    pub fn new(index: u32) -> Self {
        let id = SPU_ID_BASE | (index & 0x00FF_FFFF);
        Self {
            id,
            state: CpuState::initial(),
            pc: 0,
            base_pc: 0,
            gpr: [0u128; SPU_GPR_COUNT],
            fpscr: 0,
            ch_mfc_cmd: SpuMfcCmd::default(),
            mfc_queue: [SpuMfcCmd::default(); MFC_QUEUE_DEPTH],
            mfc_size: 0,
            mfc_barrier: u32::MAX,
            mfc_fence: u32::MAX,
            rtime: 0,
            raddr: 0,
            rdata: [0u8; RESERVATION_BLOCK],
            ls: Box::new([0u8; SPU_LS_SIZE]),
            ch_tag_mask: 0,
            ch_stall_mask: 0,
            snr_config: 0,
            channels: SpuChannels::default(),
        }
    }

    /// SPU discriminant from thread id (high byte). Always `Spu` for
    /// properly-constructed threads.
    #[must_use]
    pub fn thread_class(id: u32) -> ThreadClass {
        ThreadClass::from_id(id)
    }

    /// Enqueue an MFC command. Returns `true` on success, `false` if
    /// the 16-slot queue is full.
    pub fn mfc_enqueue(&mut self, cmd: SpuMfcCmd) -> bool {
        if self.mfc_size as usize >= MFC_QUEUE_DEPTH {
            return false;
        }
        self.mfc_queue[self.mfc_size as usize] = cmd;
        self.mfc_size += 1;
        true
    }

    /// Read `len` bytes from Local Store at `lsa`. Wraps modulo
    /// `SPU_LS_SIZE` (matches the C++ `lsa % SPU_LS_SIZE` masking).
    /// Returns `None` if the request crosses the 256 KB boundary in
    /// a single call (the C++ code also forbids this).
    pub fn ls_read(&self, lsa: u32, len: usize) -> Option<&[u8]> {
        let start = (lsa as usize) & (SPU_LS_SIZE - 1);
        let end = start.checked_add(len)?;
        if end > SPU_LS_SIZE {
            return None;
        }
        Some(&self.ls[start..end])
    }

    /// Write bytes into Local Store.
    pub fn ls_write(&mut self, lsa: u32, data: &[u8]) -> bool {
        let start = (lsa as usize) & (SPU_LS_SIZE - 1);
        let Some(end) = start.checked_add(data.len()) else {
            return false;
        };
        if end > SPU_LS_SIZE {
            return false;
        }
        self.ls[start..end].copy_from_slice(data);
        true
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Constants -------------------------------------------------

    #[test]
    fn id_base_is_0x02000000() {
        assert_eq!(SPU_ID_BASE, 0x0200_0000);
    }

    #[test]
    fn ls_size_is_256kb() {
        assert_eq!(SPU_LS_SIZE, 256 * 1024);
        assert_eq!(SPU_LS_SIZE, 0x40000);
    }

    #[test]
    fn gpr_count_is_128() {
        assert_eq!(SPU_GPR_COUNT, 128);
    }

    #[test]
    fn mfc_queue_depth_is_16() {
        assert_eq!(MFC_QUEUE_DEPTH, 16);
    }

    // -- MFC opcodes -----------------------------------------------

    #[test]
    fn mfc_primary_opcodes_frozen() {
        assert_eq!(MfcCmd::Put as u8, 0x20);
        assert_eq!(MfcCmd::Get as u8, 0x40);
        assert_eq!(MfcCmd::GetLlar as u8, 0xD0);
        assert_eq!(MfcCmd::Putllc as u8, 0xB4);
        assert_eq!(MfcCmd::PutLluc as u8, 0xB0);
        assert_eq!(MfcCmd::Barrier as u8, 0xC0);
        assert_eq!(MfcCmd::Eieio as u8, 0xC8);
        assert_eq!(MfcCmd::Sync as u8, 0xCC);
    }

    #[test]
    fn mfc_cmd_from_u8_known_values() {
        assert_eq!(MfcCmd::from_u8(0x20), MfcCmd::Put);
        assert_eq!(MfcCmd::from_u8(0x40), MfcCmd::Get);
        assert_eq!(MfcCmd::from_u8(0xD0), MfcCmd::GetLlar);
        assert_eq!(MfcCmd::from_u8(0xB4), MfcCmd::Putllc);
    }

    #[test]
    fn mfc_cmd_from_u8_unknown_is_unknown() {
        assert_eq!(MfcCmd::from_u8(0x00), MfcCmd::Unknown);
        assert_eq!(MfcCmd::from_u8(0x99), MfcCmd::Unknown);
    }

    #[test]
    fn mfc_mask_bits_frozen() {
        assert_eq!(MFC_BARRIER_MASK, 0x01);
        assert_eq!(MFC_FENCE_MASK, 0x02);
        assert_eq!(MFC_LIST_MASK, 0x04);
        assert_eq!(MFC_START_MASK, 0x08);
        assert_eq!(MFC_RESULT_MASK, 0x10);
    }

    // -- MFC atomic status -----------------------------------------

    #[test]
    fn mfc_atomic_status_values_frozen() {
        assert_eq!(MfcAtomicStatus::PutllcSuccess as u32, 0);
        assert_eq!(MfcAtomicStatus::PutllcFailure as u32, 1);
        assert_eq!(MfcAtomicStatus::PutllucSuccess as u32, 2);
        assert_eq!(MfcAtomicStatus::GetllarSuccess as u32, 4);
    }

    #[test]
    fn mfc_tag_update_values_frozen() {
        assert_eq!(MfcTagUpdate::Immediate as u32, 0);
        assert_eq!(MfcTagUpdate::Any as u32, 1);
        assert_eq!(MfcTagUpdate::All as u32, 2);
    }

    // -- SpuMfcCmd layout ------------------------------------------

    #[test]
    fn spu_mfc_cmd_is_16_bytes() {
        assert_eq!(core::mem::size_of::<SpuMfcCmd>(), 16);
    }

    #[test]
    fn spu_mfc_cmd_default_is_zero() {
        let c = SpuMfcCmd::default();
        assert_eq!(c.cmd, 0);
        assert_eq!(c.tag, 0);
        assert_eq!(c.size, 0);
        assert_eq!(c.lsa, 0);
        assert_eq!(c.eal, 0);
        assert_eq!(c.eah, 0);
    }

    // -- SpuThread defaults ----------------------------------------

    #[test]
    fn new_thread_has_default_state() {
        let t = SpuThread::new(0);
        assert_eq!(t.id, SPU_ID_BASE);
        assert_eq!(t.pc, 0);
        assert_eq!(t.base_pc, 0);
        assert_eq!(t.gpr, [0u128; SPU_GPR_COUNT]);
        assert_eq!(t.mfc_size, 0);
        assert_eq!(t.mfc_barrier, u32::MAX);
        assert_eq!(t.mfc_fence, u32::MAX);
        assert_eq!(t.raddr, 0);
        assert_eq!(t.rdata, [0u8; RESERVATION_BLOCK]);
        assert_eq!(t.ls.len(), SPU_LS_SIZE);
    }

    #[test]
    fn new_thread_index_encodes_into_id() {
        let t = SpuThread::new(0xABCDEF);
        assert_eq!(t.id, SPU_ID_BASE | 0xABCDEF);
    }

    #[test]
    fn thread_class_for_spu_id_is_spu() {
        let t = SpuThread::new(5);
        assert_eq!(SpuThread::thread_class(t.id), ThreadClass::Spu);
    }

    #[test]
    fn initial_state_is_stop_plus_wait() {
        let t = SpuThread::new(0);
        assert!(t.state.is_stopped());
    }

    // -- MFC queue operations --------------------------------------

    #[test]
    fn mfc_enqueue_succeeds_until_full() {
        let mut t = SpuThread::new(0);
        for i in 0..MFC_QUEUE_DEPTH {
            let cmd = SpuMfcCmd { cmd: MfcCmd::Get as u8, tag: i as u8, ..Default::default() };
            assert!(t.mfc_enqueue(cmd));
        }
        assert_eq!(t.mfc_size, MFC_QUEUE_DEPTH as u32);

        let overflow = SpuMfcCmd { cmd: MfcCmd::Put as u8, ..Default::default() };
        assert!(!t.mfc_enqueue(overflow));
    }

    #[test]
    fn mfc_queue_preserves_insertion_order() {
        let mut t = SpuThread::new(0);
        for i in 0..3 {
            let cmd = SpuMfcCmd { cmd: MfcCmd::Get as u8, tag: i, ..Default::default() };
            assert!(t.mfc_enqueue(cmd));
        }
        assert_eq!(t.mfc_queue[0].tag, 0);
        assert_eq!(t.mfc_queue[1].tag, 1);
        assert_eq!(t.mfc_queue[2].tag, 2);
    }

    // -- Local Store read/write ------------------------------------

    #[test]
    fn ls_write_then_read_roundtrip() {
        let mut t = SpuThread::new(0);
        let data = [0xDEu8, 0xAD, 0xBE, 0xEF];
        assert!(t.ls_write(0x1000, &data));
        assert_eq!(t.ls_read(0x1000, 4), Some(data.as_ref()));
    }

    #[test]
    fn ls_read_out_of_bounds_returns_none() {
        let t = SpuThread::new(0);
        // Last 4 bytes of LS = OK
        assert!(t.ls_read((SPU_LS_SIZE - 4) as u32, 4).is_some());
        // Going beyond = None
        assert!(t.ls_read((SPU_LS_SIZE - 3) as u32, 4).is_none());
    }

    #[test]
    fn ls_write_wraps_on_lsa_modulo() {
        let mut t = SpuThread::new(0);
        // lsa = SPU_LS_SIZE (wraps to 0) — matches `lsa % SPU_LS_SIZE`
        assert!(t.ls_write(SPU_LS_SIZE as u32, &[0xAA]));
        assert_eq!(t.ls[0], 0xAA);
    }

    // -- Register access -------------------------------------------

    #[test]
    fn gpr_write_and_read_back() {
        let mut t = SpuThread::new(0);
        t.gpr[7] = 0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00;
        assert_eq!(t.gpr[7], 0x1122_3344_5566_7788_99AA_BBCC_DDEE_FF00);
    }
}
