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

    /// R5.4a: explicit parking model. `Some(state)` when the SPU
    /// thread is parked on a channel op waiting for the counterpart
    /// (mailbox refill / drain) to land. `None` otherwise. The
    /// interpreter sets this when `step()` would have stalled; an
    /// external scheduler (or test) clears it via `clear_park()` once
    /// the parking condition is resolved. R5.4a does NOT implement a
    /// concurrent scheduler — this field is just the data model.
    pub park_state: Option<SpuParkState>,
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
    // R6.7 C.1 — MFC param + cmd + tag-stat channels for replay-mode
    // GET-only DMA support. The C++ side (`rpcs3/Emu/Cell/SPUThread.cpp`)
    // implements full MFC semantics; here we only model the subset
    // needed to let a captured GET trace replay byte-identical via a
    // pre-applied `MfcReplayState` (see crate `rpcs3-spu-differential`'s
    // `mfc_replay::apply_mfc_dma_pre_replay`). Runtime-mode MFC (a
    // live game dispatching real DMA) is out of scope — wrch ch21 in
    // runtime mode is a no-op here, and rdch ch24 stalls if the
    // replay pre-population didn't seed a value.
    pub const MFC_LSA: u32 = 16;
    pub const MFC_EAH: u32 = 17;
    pub const MFC_EAL: u32 = 18;
    pub const MFC_SIZE: u32 = 19;
    pub const MFC_TAG_ID: u32 = 20;
    pub const MFC_CMD: u32 = 21;
    pub const MFC_WR_TAG_MASK: u32 = 22;
    pub const MFC_WR_TAG_UPDATE: u32 = 23;
    pub const MFC_RD_TAG_STAT: u32 = 24;
    pub const MFC_RD_TAG_MASK: u32 = 25;
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
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    // R6.7 C.2 — MFC channel state. These fields back wrch ch16-23 and
    // rdch ch24-25 so a captured GET trace can replay end-to-end through
    // the SPU executor. They are populated either by the SPU's own
    // wrch instructions (the ch16-20, 22, 23 cases) OR by a pre-replay
    // helper (`mfc_replay::apply_mfc_dma_pre_replay` in crate
    // `rpcs3-spu-differential`) that walks the captured event stream
    // before the SPU starts running.
    /// MFC LSA (ch16). Set by `wrch ch16`. The R6.7 C.3 design
    /// pre-applies the GET DMA at this LS offset before the SPU runs,
    /// so subsequent `wrch ch21` (MFC_Cmd) is a no-op.
    pub mfc_lsa: u32,
    /// MFC EAH (ch17). Always 0 in PSL1GHT user-space scope.
    pub mfc_eah: u32,
    /// MFC EAL (ch18). Caller-supplied effective address low half.
    pub mfc_eal: u32,
    /// MFC Size (ch19). Transfer size in bytes (1, 2, 4, 8, or
    /// multiple of 16 in [16, 16384]).
    pub mfc_size: u32,
    /// MFC TagID (ch20). 5-bit tag (0..32).
    pub mfc_tag_id: u32,
    /// MFC WrTagMask (ch22 in write direction). Bitmask of tags the
    /// next rdch ch24 will inspect.
    pub mfc_wr_tag_mask: u32,
    /// MFC WrTagUpdate (ch23 in write direction). 0=Immediate /
    /// 1=Any / 2=All wait mode.
    pub mfc_wr_tag_update: u32,
    /// Pre-populated queue of `rdch ch24 (RdTagStat)` values, in the
    /// order the SPU reads them. Each rdch ch24 pops one value. Empty
    /// queue + the SPU calling rdch ch24 → `WouldStall` (the SPU
    /// parks; runtime-mode MFC is out of R6.7 scope so this is the
    /// expected halting behaviour for any unanticipated read).
    pub mfc_tag_stat_queue: std::collections::VecDeque<u32>,
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

/// R5.4a: why an SPU thread is currently parked. The variant carries
/// the channel id so a future scheduler can route mailbox/signal
/// arrivals to the right parked thread. `BadChannel` and other error
/// outcomes are NOT modeled here — those still surface as
/// `Error::Unimplemented` from the interpreter without parking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuParkReason {
    /// Parked on `rdch ch=channel` waiting for a value to arrive.
    /// Cleared when an external producer pushes to the channel.
    ChannelRead { channel: u32 },
    /// Parked on `wrch ch=channel` waiting for capacity to drain.
    /// Cleared when an external consumer drains the channel.
    ChannelWrite { channel: u32 },
}

/// R5.4a: captured state of a parked SPU thread. PC is the address of
/// the channel op the thread parked on — re-running from this PC
/// after the parking condition resolves will retry the same channel
/// op (the original semantics of "blocking" SPU channel reads/writes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuParkState {
    /// Address of the `rdch`/`wrch` instruction the thread parked on.
    pub pc: u32,
    /// Why the thread parked.
    pub reason: SpuParkReason,
}

/// R5.4b: outcome of a wake attempt against a parked SPU thread.
///
/// Wake never executes the parked instruction itself — it only checks
/// whether the parking condition is now satisfied (because some
/// external producer/consumer touched the channel) and, if so, clears
/// `park_state` and returns the saved PC so the caller can re-run.
/// The caller is responsible for actually running the SPU from `pc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuWakeResult {
    /// `park_state == None` — no wake was warranted. The wake helper
    /// is a no-op in this case (modulo any side effect from the
    /// helper's primary action — e.g., `ppu_push_inmbox` still pushes).
    NotParked,
    /// `park_state` exists, but the channel's blocking condition is
    /// still unmet (mailbox still empty / full, signal still 0). The
    /// thread stays parked, `park_state` is unchanged.
    StillBlocked,
    /// `park_state` was satisfied. `park_state` is cleared and the
    /// saved PC is returned so the caller can re-run the channel op
    /// from its original address. **Re-execution of the channel op is
    /// the caller's job** — wake itself does not advance PC or
    /// consume any value.
    Ready { pc: u32 },
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
                // R5.11: match Cell BE semantics — rdch on an unsignaled
                // SNR channel must stall (count == 0). The R5.11 signal
                // fixture (`single_spu_signal_v1`) needs this to park
                // the SPU on its initial `rdch ch3` so the captured
                // `ppu_signal` event has something to wake; without
                // this, replay races past the read with snr=0 and the
                // backends diverge from the captured OUT_MBOX value.
                // Same shape as IN_MBOX read.
                if self.snr[0] == 0 {
                    return Err(ChannelStatus::WouldStall);
                }
                let v = self.snr[0];
                self.snr[0] = 0;
                self.event_stat &= !0x00000001;
                Ok(v)
            }
            SPU_RDSIGNOTIFY2 => {
                if self.snr[1] == 0 {
                    return Err(ChannelStatus::WouldStall);
                }
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
            // R6.7 C.4 — RdTagStat (ch24): pop the next pre-populated
            // tag-stat value. Empty queue → `WouldStall` (the SPU
            // parks; runtime-mode MFC is out of R6.7 scope, so an
            // unexpected empty-queue read indicates the captured
            // trace's pre-replay setup is incomplete or the SPU
            // diverged from the trace).
            ch::MFC_RD_TAG_STAT => match self.mfc_tag_stat_queue.pop_front() {
                Some(v) => Ok(v),
                None => Err(ChannelStatus::WouldStall),
            },
            // R6.7 C.4 — RdTagMask (ch25): stateless read of the
            // current tag-mask register.
            ch::MFC_RD_TAG_MASK => Ok(self.mfc_wr_tag_mask),
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
            // R6.7 C.2 — MFC param channels. ch16-20 are simple
            // stash-only stores; ch22 / ch23 set the wait-mask /
            // wait-mode for the matching RdTagStat read. None of
            // these stall (the C++ side never stalls on these
            // channels either — they're write-only register slots).
            MFC_LSA => { self.mfc_lsa = value; Ok(()) }
            MFC_EAH => { self.mfc_eah = value; Ok(()) }
            MFC_EAL => { self.mfc_eal = value; Ok(()) }
            MFC_SIZE => { self.mfc_size = value; Ok(()) }
            MFC_TAG_ID => { self.mfc_tag_id = value; Ok(()) }
            MFC_WR_TAG_MASK => { self.mfc_wr_tag_mask = value; Ok(()) }
            MFC_WR_TAG_UPDATE => { self.mfc_wr_tag_update = value; Ok(()) }
            // R6.7 C.3 — wrch ch21 (MFC_Cmd). In replay mode the DMA
            // has ALREADY been pre-applied to LS by
            // `apply_mfc_dma_pre_replay`, so the actual cmd dispatch
            // is a no-op here — we just acknowledge the write so the
            // SPU's wrch instruction completes and PC advances.
            // Runtime-mode MFC (live game) is out of R6.7 scope; a
            // future C.D phase would dispatch via FFI back to RPCS3
            // vm:: accessors here.
            MFC_CMD => Ok(()),
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
            // R6.7 C.2 — MFC param channels are always writable
            // (single-slot register stores, never stall). MFC_CMD
            // is also always writable in replay mode. ch22 and ch23
            // (MFC_WR_TAG_MASK / MFC_WR_TAG_UPDATE) deliberately
            // omitted from this arm — those channel numbers are
            // already covered by SPU_RDEVENTMASK / SPU_RDMACHSTAT
            // above, and the SPU ABI permits the same channel id
            // to serve different roles in the read vs write
            // direction.
            MFC_LSA | MFC_EAH | MFC_EAL | MFC_SIZE | MFC_TAG_ID | MFC_CMD => 1,
            // R6.7 C.4 — RdTagStat readable count = queue depth;
            // RdTagMask is always readable (count = 1).
            MFC_RD_TAG_STAT => self.mfc_tag_stat_queue.len() as u32,
            MFC_RD_TAG_MASK => 1,
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
            park_state: None,
        }
    }

    /// R5.4a: true iff this SPU thread is currently parked on a
    /// channel op waiting for the counterpart to refill/drain.
    #[must_use]
    pub fn is_parked(&self) -> bool {
        self.park_state.is_some()
    }

    /// R5.4a: record that this SPU thread parked at `pc` for `reason`.
    /// The interpreter calls this from `step()` when an `rdch` would
    /// block on an empty channel or a `wrch` on a full one. PC is the
    /// address of the channel-op instruction itself (NOT pc+4) — that's
    /// the correct address to re-run once the parking condition is
    /// resolved.
    pub fn park_on_channel(&mut self, pc: u32, reason: SpuParkReason) {
        self.park_state = Some(SpuParkState { pc, reason });
    }

    /// R5.4a: clear any park state. Caller is responsible for calling
    /// this once the parking condition has been resolved (e.g. a value
    /// was pushed into `in_mbox`, a slot was drained from `out_mbox`)
    /// and the SPU should be allowed to re-execute the channel op.
    /// Does not touch GPRs / LS / SpuChannels.
    pub fn clear_park(&mut self) {
        self.park_state = None;
    }

    /// R5.4a: PC at which the thread parked, or `None` if not parked.
    #[must_use]
    pub fn parked_pc(&self) -> Option<u32> {
        self.park_state.map(|p| p.pc)
    }

    /// R5.4a: reason the thread parked, or `None` if not parked.
    #[must_use]
    pub fn parked_reason(&self) -> Option<SpuParkReason> {
        self.park_state.map(|p| p.reason)
    }

    /// R5.4b: check whether the channel condition behind a parked
    /// state is now satisfied; clear `park_state` and return the saved
    /// PC if it is. **Does not execute or advance PC.** The caller
    /// must re-run the SPU from the returned PC to actually retry the
    /// channel op.
    ///
    /// Returns:
    /// - `NotParked` if `park_state == None`.
    /// - `StillBlocked` if parked but the condition is still unmet.
    /// - `Ready { pc }` if the condition is satisfied; `park_state`
    ///   is cleared as a side effect.
    ///
    /// Conditions per parking reason:
    /// - `ChannelRead { 29 }` (RDINMBOX): `in_mbox.is_some()`.
    /// - `ChannelRead { 3 }` (RDSIGNOTIFY1): `snr[0] != 0`.
    /// - `ChannelRead { 4 }` (RDSIGNOTIFY2): `snr[1] != 0`.
    /// - `ChannelWrite { 28 }` (WROUTMBOX): `out_mbox.is_none()`.
    /// - `ChannelWrite { 30 }` (WROUTINTRMBOX): `out_intr_mbox.is_none()`.
    /// - Any other channel: stays `StillBlocked` (no resolution path
    ///   defined; defensive).
    pub fn try_resolve_park(&mut self) -> SpuWakeResult {
        let park = match self.park_state {
            Some(p) => p,
            None => return SpuWakeResult::NotParked,
        };
        use ch::*;
        let satisfied = match park.reason {
            SpuParkReason::ChannelRead { channel } => match channel {
                SPU_RDINMBOX => self.channels.in_mbox.is_some(),
                SPU_RDSIGNOTIFY1 => self.channels.snr[0] != 0,
                SPU_RDSIGNOTIFY2 => self.channels.snr[1] != 0,
                _ => false,
            },
            SpuParkReason::ChannelWrite { channel } => match channel {
                SPU_WROUTMBOX => self.channels.out_mbox.is_none(),
                SPU_WROUTINTRMBOX => self.channels.out_intr_mbox.is_none(),
                _ => false,
            },
        };
        if satisfied {
            self.clear_park();
            SpuWakeResult::Ready { pc: park.pc }
        } else {
            SpuWakeResult::StillBlocked
        }
    }

    /// R5.4b: PPU-side helper that pushes `value` to `in_mbox` (if
    /// empty) and then attempts to wake any thread parked on
    /// `rdch ch=29`. The push is best-effort — if `in_mbox` was
    /// already full the push is a no-op, but the wake check still
    /// runs (which would normally find the existing value satisfies
    /// the park condition).
    pub fn ppu_push_inmbox_and_try_wake(&mut self, value: u32) -> SpuWakeResult {
        let _ = self.channels.ppu_push_inmbox(value);
        self.try_resolve_park()
    }

    /// R5.4b: PPU-side helper that drains `out_mbox` (returns the old
    /// value) and then attempts to wake any thread parked on
    /// `wrch ch=28`. Returns `(drained_value, wake_result)`.
    pub fn ppu_pop_outmbox_and_try_wake(&mut self) -> (Option<u32>, SpuWakeResult) {
        let drained = self.channels.ppu_pop_outmbox();
        let wake = self.try_resolve_park();
        (drained, wake)
    }

    /// R5.4b: PPU-side helper that pushes a signal into `snr[slot]`
    /// (OR-merged per SPU semantics) and then attempts to wake any
    /// thread parked on the corresponding `rdch ch=3/4`.
    pub fn signal_and_try_wake(&mut self, slot: usize, value: u32) -> SpuWakeResult {
        let _ = self.channels.signal(slot, value);
        self.try_resolve_park()
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

    // =================================================================
    // R5.4a — Channel parking model
    // =================================================================

    #[test]
    fn park_state_is_none_for_fresh_thread() {
        let t = SpuThread::new(0);
        assert!(!t.is_parked());
        assert!(t.park_state.is_none());
        assert!(t.parked_pc().is_none());
        assert!(t.parked_reason().is_none());
    }

    #[test]
    fn park_on_channel_records_pc_and_reason() {
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x108, SpuParkReason::ChannelRead { channel: 29 });
        assert!(t.is_parked());
        assert_eq!(t.parked_pc(), Some(0x108));
        assert_eq!(t.parked_reason(),
                   Some(SpuParkReason::ChannelRead { channel: 29 }));
    }

    #[test]
    fn park_on_channel_overwrites_previous_park() {
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x100, SpuParkReason::ChannelRead { channel: 29 });
        t.park_on_channel(0x200, SpuParkReason::ChannelWrite { channel: 28 });
        // Latest wins.
        assert_eq!(t.parked_pc(), Some(0x200));
        assert_eq!(t.parked_reason(),
                   Some(SpuParkReason::ChannelWrite { channel: 28 }));
    }

    #[test]
    fn clear_park_does_not_touch_other_state() {
        let mut t = SpuThread::new(0);
        t.gpr[3] = 0xCAFE;
        t.pc = 0x100;
        let _ = t.ls_write(0x40, &[0xAA, 0xBB, 0xCC, 0xDD]);
        t.channels.event_mask = 0x12345;
        t.park_on_channel(0x108, SpuParkReason::ChannelWrite { channel: 28 });

        t.clear_park();

        assert!(!t.is_parked());
        assert_eq!(t.parked_pc(), None);
        assert_eq!(t.parked_reason(), None);
        // Untouched:
        assert_eq!(t.gpr[3], 0xCAFE);
        assert_eq!(t.pc, 0x100);
        assert_eq!(t.ls_read(0x40, 4), Some([0xAA, 0xBB, 0xCC, 0xDD].as_ref()));
        assert_eq!(t.channels.event_mask, 0x12345);
    }

    #[test]
    fn park_state_round_trip_through_clone() {
        // SpuParkReason and SpuParkState are Copy + PartialEq, so the
        // park_state Option<...> survives a clone of SpuThread's
        // shape via the same patterns used in snapshots.
        let original = SpuParkState {
            pc: 0x10C,
            reason: SpuParkReason::ChannelRead { channel: 4 },
        };
        let copy = original;
        assert_eq!(copy, original);
    }

    // =================================================================
    // R5.4b — Explicit wake API
    // =================================================================

    #[test]
    fn try_resolve_park_not_parked_returns_not_parked() {
        let mut t = SpuThread::new(0);
        assert_eq!(t.try_resolve_park(), SpuWakeResult::NotParked);
        assert!(!t.is_parked());
    }

    #[test]
    fn try_resolve_park_rdch_inmbox_empty_still_blocked() {
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x108, SpuParkReason::ChannelRead { channel: ch::SPU_RDINMBOX });
        // in_mbox is None (empty) — still blocked.
        assert_eq!(t.try_resolve_park(), SpuWakeResult::StillBlocked);
        assert!(t.is_parked(), "park_state must NOT be cleared on StillBlocked");
        assert_eq!(t.parked_pc(), Some(0x108));
    }

    #[test]
    fn try_resolve_park_rdch_inmbox_filled_returns_ready() {
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x108, SpuParkReason::ChannelRead { channel: ch::SPU_RDINMBOX });
        // External producer pushes a value.
        assert!(t.channels.ppu_push_inmbox(0xABCD));
        match t.try_resolve_park() {
            SpuWakeResult::Ready { pc } => assert_eq!(pc, 0x108),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(!t.is_parked(), "park_state must be cleared on Ready");
        // Mailbox value is still there for the SPU to consume on resume.
        assert_eq!(t.channels.in_mbox, Some(0xABCD));
    }

    #[test]
    fn try_resolve_park_wrch_outmbox_full_still_blocked() {
        let mut t = SpuThread::new(0);
        t.channels.out_mbox = Some(0xAA);  // pre-fill
        t.park_on_channel(0x10C, SpuParkReason::ChannelWrite { channel: ch::SPU_WROUTMBOX });
        assert_eq!(t.try_resolve_park(), SpuWakeResult::StillBlocked);
        assert!(t.is_parked());
    }

    #[test]
    fn try_resolve_park_wrch_outmbox_drained_returns_ready() {
        let mut t = SpuThread::new(0);
        t.channels.out_mbox = Some(0xAA);
        t.park_on_channel(0x10C, SpuParkReason::ChannelWrite { channel: ch::SPU_WROUTMBOX });
        // External consumer drains.
        assert_eq!(t.channels.ppu_pop_outmbox(), Some(0xAA));
        match t.try_resolve_park() {
            SpuWakeResult::Ready { pc } => assert_eq!(pc, 0x10C),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(!t.is_parked());
    }

    #[test]
    fn try_resolve_park_signotify_no_signal_still_blocked() {
        // signotify never stalls in our interpreter, but the wake API
        // is defined for it (defensive — manual park works).
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x110, SpuParkReason::ChannelRead { channel: ch::SPU_RDSIGNOTIFY1 });
        // snr[0] = 0 → still blocked.
        assert_eq!(t.try_resolve_park(), SpuWakeResult::StillBlocked);
    }

    #[test]
    fn try_resolve_park_signotify_after_signal_returns_ready() {
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x110, SpuParkReason::ChannelRead { channel: ch::SPU_RDSIGNOTIFY1 });
        assert!(t.channels.signal(0, 0xDEADBEEF));
        match t.try_resolve_park() {
            SpuWakeResult::Ready { pc } => assert_eq!(pc, 0x110),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(!t.is_parked());
        assert_eq!(t.channels.snr[0], 0xDEADBEEF);
    }

    #[test]
    fn try_resolve_park_unknown_channel_stays_blocked() {
        // Park on a channel without a defined resolution — defensive
        // fallback: stays StillBlocked, never auto-clears.
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x100, SpuParkReason::ChannelRead { channel: 99 });
        assert_eq!(t.try_resolve_park(), SpuWakeResult::StillBlocked);
        assert!(t.is_parked());
    }

    #[test]
    fn ppu_push_inmbox_and_try_wake_resolves_park() {
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x108, SpuParkReason::ChannelRead { channel: ch::SPU_RDINMBOX });
        match t.ppu_push_inmbox_and_try_wake(0x12345) {
            SpuWakeResult::Ready { pc } => assert_eq!(pc, 0x108),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(!t.is_parked());
        assert_eq!(t.channels.in_mbox, Some(0x12345));
    }

    #[test]
    fn ppu_pop_outmbox_and_try_wake_drains_and_resolves() {
        let mut t = SpuThread::new(0);
        t.channels.out_mbox = Some(0xCAFE);
        t.park_on_channel(0x10C, SpuParkReason::ChannelWrite { channel: ch::SPU_WROUTMBOX });
        let (drained, wake) = t.ppu_pop_outmbox_and_try_wake();
        assert_eq!(drained, Some(0xCAFE));
        match wake {
            SpuWakeResult::Ready { pc } => assert_eq!(pc, 0x10C),
            other => panic!("expected Ready, got {other:?}"),
        }
        assert!(!t.is_parked());
    }

    #[test]
    fn signal_and_try_wake_resolves_signotify_park() {
        let mut t = SpuThread::new(0);
        t.park_on_channel(0x114, SpuParkReason::ChannelRead { channel: ch::SPU_RDSIGNOTIFY2 });
        match t.signal_and_try_wake(1, 0xFACE) {
            SpuWakeResult::Ready { pc } => assert_eq!(pc, 0x114),
            other => panic!("expected Ready, got {other:?}"),
        }
    }

    #[test]
    fn ppu_push_inmbox_and_try_wake_when_not_parked_is_noop_for_park() {
        let mut t = SpuThread::new(0);
        // Not parked. Push still happens (returns NotParked).
        let r = t.ppu_push_inmbox_and_try_wake(0x42);
        assert_eq!(r, SpuWakeResult::NotParked);
        assert_eq!(t.channels.in_mbox, Some(0x42),
                   "push must still happen even if no park to resolve");
    }

    #[test]
    fn wake_does_not_alter_gpr_or_ls_if_still_blocked() {
        let mut t = SpuThread::new(0);
        t.gpr[5] = 0xFEEDFACE;
        let _ = t.ls_write(0x40, &[0xAA, 0xBB, 0xCC, 0xDD]);
        t.park_on_channel(0x108, SpuParkReason::ChannelRead { channel: ch::SPU_RDINMBOX });
        // No producer; in_mbox empty — StillBlocked.
        assert_eq!(t.try_resolve_park(), SpuWakeResult::StillBlocked);
        // Untouched:
        assert_eq!(t.gpr[5], 0xFEEDFACE);
        assert_eq!(t.ls_read(0x40, 4), Some([0xAA, 0xBB, 0xCC, 0xDD].as_ref()));
        assert!(t.is_parked());
    }

    // =================================================================
    // R6.7 C.2 — MFC channel dispatch tests
    //
    // The MFC param channels (16-20, 22, 23) are write-only register
    // stores; the SPU writes them to assemble a DMA cmd packet but the
    // actual transfer happens on `wrch ch21` (which in REPLAY mode is
    // a no-op because `apply_mfc_dma_pre_replay` already injected the
    // bytes into LS). `rdch ch24` (RdTagStat) pops the next pre-
    // populated tag-stat value; `rdch ch25` (RdTagMask) is a stateless
    // mirror of the wr_tag_mask register. None of the MFC channels
    // ever return BadChannel after Phase C.
    // =================================================================

    #[test]
    fn mfc_param_channels_stash_value_and_never_stall() {
        let mut t = SpuThread::new(0);
        // Each ch16-20 + ch22 + ch23 wrch should succeed and store
        // the value in the matching SpuChannels field.
        assert_eq!(t.channels.write(ch::MFC_LSA, 0x3FF00), Ok(()));
        assert_eq!(t.channels.mfc_lsa, 0x3FF00);
        assert_eq!(t.channels.write(ch::MFC_EAH, 0), Ok(()));
        assert_eq!(t.channels.mfc_eah, 0);
        assert_eq!(t.channels.write(ch::MFC_EAL, 0xD0010000), Ok(()));
        assert_eq!(t.channels.mfc_eal, 0xD0010000);
        assert_eq!(t.channels.write(ch::MFC_SIZE, 128), Ok(()));
        assert_eq!(t.channels.mfc_size, 128);
        assert_eq!(t.channels.write(ch::MFC_TAG_ID, 3), Ok(()));
        assert_eq!(t.channels.mfc_tag_id, 3);
        assert_eq!(t.channels.write(ch::MFC_WR_TAG_MASK, 1u32 << 3), Ok(()));
        assert_eq!(t.channels.mfc_wr_tag_mask, 1u32 << 3);
        assert_eq!(t.channels.write(ch::MFC_WR_TAG_UPDATE, 2), Ok(()));
        assert_eq!(t.channels.mfc_wr_tag_update, 2);
        // ch21 (MFC_Cmd) accepted as no-op in replay mode.
        assert_eq!(t.channels.write(ch::MFC_CMD, 0x40), Ok(()));
    }

    #[test]
    fn mfc_rdtagstat_pops_pre_populated_queue_or_stalls_when_empty() {
        let mut t = SpuThread::new(0);
        // Empty queue → WouldStall.
        assert_eq!(
            t.channels.read(ch::MFC_RD_TAG_STAT),
            Err(ChannelStatus::WouldStall)
        );
        // Pre-populate two tag-stat values.
        t.channels.mfc_tag_stat_queue.push_back(1u32 << 3);
        t.channels.mfc_tag_stat_queue.push_back(1u32 << 5);
        assert_eq!(t.channels.read(ch::MFC_RD_TAG_STAT), Ok(1u32 << 3));
        assert_eq!(t.channels.read(ch::MFC_RD_TAG_STAT), Ok(1u32 << 5));
        // Drained → next read stalls again.
        assert_eq!(
            t.channels.read(ch::MFC_RD_TAG_STAT),
            Err(ChannelStatus::WouldStall)
        );
    }

    #[test]
    fn mfc_rdtagmask_mirrors_wr_tag_mask_register() {
        let mut t = SpuThread::new(0);
        t.channels.mfc_wr_tag_mask = 0x1234;
        // Stateless read — calling twice returns the same value.
        assert_eq!(t.channels.read(ch::MFC_RD_TAG_MASK), Ok(0x1234));
        assert_eq!(t.channels.read(ch::MFC_RD_TAG_MASK), Ok(0x1234));
    }

    #[test]
    fn mfc_channel_count_reports_correct_capacity() {
        let mut t = SpuThread::new(0);
        // Param/cmd write channels: always 1 free slot.
        assert_eq!(t.channels.count(ch::MFC_LSA), Ok(1));
        assert_eq!(t.channels.count(ch::MFC_EAH), Ok(1));
        assert_eq!(t.channels.count(ch::MFC_EAL), Ok(1));
        assert_eq!(t.channels.count(ch::MFC_SIZE), Ok(1));
        assert_eq!(t.channels.count(ch::MFC_TAG_ID), Ok(1));
        assert_eq!(t.channels.count(ch::MFC_CMD), Ok(1));
        // RdTagStat: count = queue depth.
        assert_eq!(t.channels.count(ch::MFC_RD_TAG_STAT), Ok(0));
        t.channels.mfc_tag_stat_queue.push_back(0xAA);
        t.channels.mfc_tag_stat_queue.push_back(0xBB);
        assert_eq!(t.channels.count(ch::MFC_RD_TAG_STAT), Ok(2));
        // RdTagMask: always 1 (stateless).
        assert_eq!(t.channels.count(ch::MFC_RD_TAG_MASK), Ok(1));
    }
}
