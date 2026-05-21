//! R6.7 A.4 — MFC replay state machine for GET-only DMA.
//!
//! Wire-format reference: `docs/SPU_DMA_MFC_R6_7_DESIGN.md` § 6
//! ("Replay state machine"). This module ports § 6.1's `MfcReplayState`
//! data model + § 6.2's event dispatch rules into Rust.
//!
//! ## Scope of this module (A.4)
//!
//! - The `MfcReplayState` data type with pending-cmd fields, tag-mask /
//!   tag-update state, and in-flight + completed-tag tracking.
//! - Event-by-event walk methods that consume captured MFC events:
//!   `process_wrch` (ch16-23), `process_mfc_cmd` (loads `.dmachunk`
//!   from A.3 and copies bytes into a caller-supplied `ls` buffer),
//!   `process_mfc_dma_complete`, `process_rdch_tagstat`.
//! - Strict validation per design § 4.3: cmd must be 0x40, eah must be
//!   0, sizes must be in scope, ordering must hold, tag-stat oracle
//!   must match the captured value.
//! - Comprehensive unit tests covering all happy-path and error-path
//!   transitions.
//!
//! ## NOT in scope at this phase
//!
//! - **Integration into the SPU executor.** The Rust SPU thread
//!   (`rpcs3-spu-thread`) doesn't yet handle MFC channels (16-25)
//!   in its `ch::` module. Wiring `MfcReplayState` into the
//!   interpreter / recompiler so a captured DMA trace replays
//!   end-to-end requires the Phase C work (C.1-C.4 in the design
//!   doc § 8). A.4 lands the state machine as standalone
//!   infrastructure; the transformer continues to hard-reject
//!   MFC traces with `TraceTransformError::UnsupportedDmaInTrace`
//!   until C lands.
//!
//! - **Bridge runtime DMA.** Phase D scope; explicitly out of A.4.
//!
//! - **PUT, list, atomic primitives.** GET (cmd 0x40) only. The state
//!   machine refuses any other cmd code with `UnsupportedMfcCmd`.
//!
//! - **EAH != 0.** PSL1GHT 32-bit user space only. `UnsupportedMfcEah`
//!   on any non-zero high half.
//!
//! ## Design notes
//!
//! Both Interpreter and Recompiler executors get their own fresh
//! `MfcReplayState` instance for each replay run, fed the same
//! captured event stream + same `.dmachunk` bytes (resolved via the
//! A.3 loader from the same per-trace + canonical paths). This is
//! design-doc § 6.3 Option A — independent instances driven by
//! identical inputs, not a shared mutable instance, because RPCS3's
//! `replay_per_spu_traces_with` already runs each backend in its own
//! turn. Determinism comes from the inputs, not from instance
//! sharing.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};

use crate::dma_chunk::{resolve_dma_chunk_side_file, DmaChunkLoadError};
use crate::trace_fmt::{CapturedEvent, MfcDmaCompleteEvent, SpuMfcCmdEvent};
use crate::SpuProgram;

// =====================================================================
// Constants — mirror the parser's MFC subset.
// =====================================================================

/// SPU local store size (256 KiB).
const SPU_LS_SIZE: usize = 0x40000;

/// MFC cmd code: simple GET (EA → LS).
const MFC_GET_CMD: u32 = 0x40;

/// R8.1 — MFC cmd code: simple PUT (LS → EA). Replay treats PUT as
/// an "assert the LS bytes at dispatch match the captured chunk" +
/// "no-op on the simulated EA" (we don't have a runtime EA in
/// replay; the captured `.dmachunk` carries the SPU's output bytes
/// taken at the moment of `wrch ch21`). The runtime bridge path
/// (R8.1 Phase B) supplies a callback that memcpys the LS bytes to
/// `vm::_ptr<u8>(eal)`.
const MFC_PUT_CMD: u32 = 0x20;

/// MFC tag is a 5-bit field.
const MFC_TAG_MAX: u32 = 31;

/// Maximum MFC simple-cmd transfer size (16 KiB).
const MFC_DMA_SIZE_MAX: u32 = 0x4000;

/// MFC channel ids (mirrors the C++ side + `rpcs3/Emu/Cell/SPUThread.cpp`
/// case labels). Used by `process_wrch` to dispatch on channel id and
/// by `process_rdch_tagstat` to enforce ch24 only at the rdch entry
/// point.
mod ch {
    pub const MFC_LSA: u32 = 16;
    pub const MFC_EAH: u32 = 17;
    pub const MFC_EAL: u32 = 18;
    pub const MFC_SIZE: u32 = 19;
    pub const MFC_TAG_ID: u32 = 20;
    pub const MFC_CMD: u32 = 21;
    pub const WR_TAG_MASK: u32 = 22;
    pub const WR_TAG_UPDATE: u32 = 23;
    pub const RD_TAG_STAT: u32 = 24;
}

/// MFC tag-update wait modes (`MFC_WrTagUpdate` values, mirrored from
/// `rpcs3/Emu/Cell/SPUThread.cpp` MFC_TAG_UPDATE_*).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MfcTagUpdate {
    /// Mode 0 — `IMMEDIATE`. Returns the current tag stat without
    /// waiting (rdch ch24 returns whatever bits are set in
    /// `completed_tags & wr_tag_mask`).
    Immediate,
    /// Mode 1 — `ANY`. Waits until at least one tag in the mask has
    /// completed.
    Any,
    /// Mode 2 — `ALL`. Waits until ALL tags in the mask have completed.
    All,
}

impl MfcTagUpdate {
    fn from_value(value: u32) -> Result<Self, MfcReplayError> {
        match value {
            0 => Ok(Self::Immediate),
            1 => Ok(Self::Any),
            2 => Ok(Self::All),
            other => Err(MfcReplayError::UnsupportedMfcRdTagMode { mode: other }),
        }
    }
}

// =====================================================================
// Data model
// =====================================================================

/// Pending MFC command packet being assembled by ch16-20 wrches before
/// the matching ch21 dispatches the transfer. All `Option`s start
/// `None`; each ch16-20 write fills in one slot. ch21 dispatch reads
/// every slot — any missing field is a `MissingMfcParam` error.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingMfcCmd {
    pub lsa: Option<u32>,
    pub eah: Option<u32>,
    pub eal: Option<u32>,
    pub size: Option<u32>,
    pub tag: Option<u32>,
}

impl PendingMfcCmd {
    fn require_complete(&self) -> Result<(u32, u32, u32, u32, u32), MfcReplayError> {
        Ok((
            self.lsa.ok_or(MfcReplayError::MissingMfcParam { which: "lsa (ch16)" })?,
            self.eah.ok_or(MfcReplayError::MissingMfcParam { which: "eah (ch17)" })?,
            self.eal.ok_or(MfcReplayError::MissingMfcParam { which: "eal (ch18)" })?,
            self.size.ok_or(MfcReplayError::MissingMfcParam { which: "size (ch19)" })?,
            self.tag.ok_or(MfcReplayError::MissingMfcParam { which: "tag (ch20)" })?,
        ))
    }

    fn clear(&mut self) {
        *self = Self::default();
    }
}

/// One in-flight MFC GET. Created on `process_mfc_cmd`; promoted to
/// "completed" via `process_mfc_dma_complete`; cleared on
/// `process_rdch_tagstat` once the matching tag bit is observed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MfcInFlight {
    pub cmd: u32,
    pub size: u32,
    pub lsa: u32,
    pub ea: u64,
    pub chunk_sha256: String,
}

/// Errors from the MFC replay state machine. Every variant is sharp so
/// the caller can distinguish "wrong cmd code" from "missing chunk
/// file" from "capture went out of order". None of these are
/// recoverable in-place — they indicate a malformed trace, a tampered
/// side-file, or an unsupported subset.
#[derive(Debug, Clone, PartialEq)]
pub enum MfcReplayError {
    /// A required ch16-20 wrch was missing when ch21 dispatched.
    MissingMfcParam { which: &'static str },
    /// The follow-up `spu_mfc_cmd` event's field disagrees with the
    /// pending state assembled from ch16-20. R6.7 A.1 writer emits
    /// both atomically, so a divergence means the trace was tampered
    /// or two SPUs interleaved their events somehow.
    MfcCmdMismatch {
        field: &'static str,
        pending: u32,
        observed: u32,
    },
    /// Caller invoked `process_mfc_cmd` without first observing
    /// ch21 wrch — the dispatch order is enforced strictly.
    MissingMfcCmdEvent,
    /// `process_rdch_tagstat` was called but the in-flight tags
    /// matching `wr_tag_mask` haven't all received their
    /// `mfc_dma_complete` yet.
    MissingMfcDmaComplete { tag: u32 },
    /// `mfc_dma_complete.transferred_bytes` differs from the
    /// dispatch's `size`, OR the tag isn't in flight.
    MfcDmaCompleteMismatch { reason: &'static str },
    /// `.dmachunk` side-file resolution / verification failed.
    /// Carries the underlying [`DmaChunkLoadError`] so the caller
    /// can surface a precise diagnostic (missing path, sha mismatch,
    /// etc.).
    DmaChunkLoad(DmaChunkLoadError),
    /// `process_rdch_tagstat` saw a captured value that doesn't
    /// match what the state machine would compute from its current
    /// completed-tag bitmask + wait mode.
    TagStatMismatch {
        captured: u32,
        oracle: u32,
        wr_tag_mask: u32,
        mode: MfcTagUpdate,
    },
    /// The `wr_tag_update` value is not 0 (Immediate), 1 (Any), or
    /// 2 (All).
    UnsupportedMfcRdTagMode { mode: u32 },
    /// Caller invoked a method with a channel id outside the MFC
    /// range the state machine handles.
    UnsupportedMfcChannel { channel: u32 },
    /// Defense-in-depth: `process_mfc_cmd` was called with an event
    /// whose `cmd` field is not `0x40 (GET)`. The parser already
    /// rejects this with `UnsupportedMfcCmd`, but the state machine
    /// re-checks so it can be invoked with hand-built events.
    UnsupportedMfcCmd { cmd: u32 },
    /// Defense-in-depth: `process_mfc_cmd` was called with an event
    /// whose `eah` field is non-zero. Same parser-side rejection
    /// (`UnsupportedMfcEah`); re-checked here.
    UnsupportedMfcEah { eah: u32 },
    /// Defense-in-depth: tag out of 5-bit range. Same parser-side
    /// rejection (`BadMfcTag`); re-checked here.
    BadMfcTag { tag: u32 },
    /// Defense-in-depth: size 0 or > 16 KiB.
    BadDmaSize { size: u32, reason: &'static str },
    /// Defense-in-depth: lsa + size exceeds 256 KiB local store.
    BadDmaLsa { lsa: u32, size: u32, reason: &'static str },
    /// Caller-supplied LS buffer is not exactly 256 KiB.
    BadLsBufferSize { actual: usize, expected: usize },
    /// R8.1 — PUT replay assertion failure. The SPU's LS bytes at
    /// `[lsa..lsa+size]` at the moment `process_mfc_cmd` was called
    /// for a `cmd=0x20 (PUT)` event did NOT match the captured
    /// `.dmachunk`. This is the load-bearing replay invariant for
    /// PUT: the captured chunk IS the SPU's output at dispatch time;
    /// any divergence in the replay run's LS bytes is a real Rust ↔
    /// C++ correctness gap.
    PutLsBytesMismatch {
        tag: u32,
        lsa: u32,
        size: u32,
        /// First-byte-of-mismatch offset within the chunk (for fast
        /// diagnostics; the caller may want to dump both buffers).
        first_diff_offset: u32,
        captured: u8,
        observed: u8,
    },
}

impl std::fmt::Display for MfcReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingMfcParam { which } => {
                write!(f, "MFC replay error: missing pending param {which} at ch21 dispatch")
            }
            Self::MfcCmdMismatch { field, pending, observed } => {
                write!(
                    f,
                    "MFC replay error: spu_mfc_cmd.{field}=0x{observed:x} disagrees with pending state 0x{pending:x}"
                )
            }
            Self::MissingMfcCmdEvent => {
                write!(f, "MFC replay error: process_mfc_cmd called without prior ch21 wrch")
            }
            Self::MissingMfcDmaComplete { tag } => {
                write!(f, "MFC replay error: rdch ch24 observed before mfc_dma_complete for tag {tag}")
            }
            Self::MfcDmaCompleteMismatch { reason } => {
                write!(f, "MFC replay error: mfc_dma_complete mismatch ({reason})")
            }
            Self::DmaChunkLoad(e) => {
                write!(f, "MFC replay error: .dmachunk load failed — {e}")
            }
            Self::TagStatMismatch { captured, oracle, wr_tag_mask, mode } => {
                write!(
                    f,
                    "MFC replay error: rdch ch24 captured value 0x{captured:x} disagrees with oracle 0x{oracle:x} (wr_tag_mask=0x{wr_tag_mask:x}, mode={mode:?})"
                )
            }
            Self::UnsupportedMfcRdTagMode { mode } => {
                write!(f, "MFC replay error: unsupported wr_tag_update mode {mode}")
            }
            Self::UnsupportedMfcChannel { channel } => {
                write!(f, "MFC replay error: unsupported MFC channel {channel}")
            }
            Self::UnsupportedMfcCmd { cmd } => {
                write!(f, "MFC replay error: unsupported MFC cmd 0x{cmd:x} (only 0x40 GET is in scope)")
            }
            Self::UnsupportedMfcEah { eah } => {
                write!(f, "MFC replay error: unsupported eah 0x{eah:x} (must be 0 in PSL1GHT scope)")
            }
            Self::BadMfcTag { tag } => {
                write!(f, "MFC replay error: bad tag {tag} (must be 0..32)")
            }
            Self::BadDmaSize { size, reason } => {
                write!(f, "MFC replay error: bad size {size} ({reason})")
            }
            Self::BadDmaLsa { lsa, size, reason } => {
                write!(f, "MFC replay error: bad lsa 0x{lsa:x} for size {size} ({reason})")
            }
            Self::BadLsBufferSize { actual, expected } => {
                write!(
                    f,
                    "MFC replay error: caller supplied LS buffer of {actual} bytes; expected exactly {expected} (256 KiB)"
                )
            }
            Self::PutLsBytesMismatch {
                tag,
                lsa,
                size,
                first_diff_offset,
                captured,
                observed,
            } => {
                write!(
                    f,
                    "MFC replay error (R8.1 PUT): SPU's LS bytes at \
                     [0x{lsa:x}..0x{end:x}] (tag={tag} size={size}) do NOT match the captured \
                     `.dmachunk` content. First diff at offset {first_diff_offset}: \
                     captured byte = 0x{captured:02x}, replay-observed byte = 0x{observed:02x}. \
                     The captured chunk IS the SPU's PUT output at dispatch time; a divergence \
                     indicates the replay run's SPU bytecode produced different bytes than the \
                     capture run — investigate as a real correctness gap, do NOT regenerate the \
                     chunk.",
                    end = lsa + size,
                )
            }
        }
    }
}

impl std::error::Error for MfcReplayError {}

impl From<DmaChunkLoadError> for MfcReplayError {
    fn from(e: DmaChunkLoadError) -> Self {
        Self::DmaChunkLoad(e)
    }
}

/// Per-SPU MFC replay state machine. Built once per SPU per replay
/// run; consumes captured MFC events in stream order; mutates a
/// caller-supplied `ls` buffer when ch21 dispatches a GET.
///
/// **Lifecycle:**
/// 1. `new(trace_path, canonical_dma_dir)` — store paths used by the
///    .dmachunk loader.
/// 2. Walk events in order. For each:
///    - `spu_wrch ch16-20`: `process_wrch(ch, value)` updates `pending`.
///    - `spu_wrch ch22`: stores `wr_tag_mask`.
///    - `spu_wrch ch23`: stores `wr_tag_update` (Immediate/Any/All).
///    - `spu_wrch ch21`: marks the dispatch armed (the next event MUST
///      be `spu_mfc_cmd`; anything else is an error).
///    - `spu_mfc_cmd`: `process_mfc_cmd(event, ls, ...)` validates +
///      loads chunk + copies bytes into `ls[lsa..lsa+size]`.
///    - `mfc_dma_complete`: marks the tag completed.
///    - `spu_rdch ch24`: `process_rdch_tagstat(value)` returns the
///      oracle stat and validates against `value`.
///
/// **Determinism:** the state machine is purely value-determined.
/// Identical event streams + identical .dmachunk bytes → identical
/// output. Both the Interpreter and Recompiler can drive their own
/// instance with the same inputs and produce byte-identical LS.
pub struct MfcReplayState {
    trace_path: PathBuf,
    canonical_dma_dir: PathBuf,
    pending: PendingMfcCmd,
    /// Tag-mask written by ch22.
    wr_tag_mask: u32,
    /// Tag-update mode written by ch23. None until first ch23 wrch.
    wr_tag_update: Option<MfcTagUpdate>,
    /// True iff the most recent SPU-side event was `spu_wrch ch21`,
    /// meaning the next captured event MUST be `spu_mfc_cmd` for the
    /// same target SPU. Cleared by `process_mfc_cmd`.
    awaiting_mfc_cmd_event: bool,
    /// Tags currently dispatched but not yet completed (no
    /// `mfc_dma_complete` observed).
    in_flight: BTreeMap<u32, MfcInFlight>,
    /// Bitmask of completed-but-not-yet-observed-via-ch24 tags.
    completed_tags: u32,
}

impl MfcReplayState {
    /// Create a fresh state machine. `trace_path` is the path to the
    /// JSONL capture (e.g. `path/to/capture.jsonl`); the loader
    /// derives the per-trace `.dma/` subdir from it. `canonical_dma_dir`
    /// is the CC0 shared store (typically
    /// `behavior-freeze/fixtures/spu/dma`).
    #[must_use]
    pub fn new(trace_path: impl Into<PathBuf>, canonical_dma_dir: impl Into<PathBuf>) -> Self {
        Self {
            trace_path: trace_path.into(),
            canonical_dma_dir: canonical_dma_dir.into(),
            pending: PendingMfcCmd::default(),
            wr_tag_mask: 0,
            wr_tag_update: None,
            awaiting_mfc_cmd_event: false,
            in_flight: BTreeMap::new(),
            completed_tags: 0,
        }
    }

    /// Non-mutating access to the current pending packet. Useful for
    /// tests / diagnostics.
    #[must_use]
    pub fn pending(&self) -> &PendingMfcCmd {
        &self.pending
    }

    /// Non-mutating access to the bitmask of completed-but-unobserved
    /// tags.
    #[must_use]
    pub fn completed_tags(&self) -> u32 {
        self.completed_tags
    }

    /// Process a `spu_wrch` to one of the MFC channels (16, 17, 18,
    /// 19, 20, 21, 22, 23). Returns `Ok(())` on success. ch21
    /// (MFC_Cmd) does NOT load the chunk here — it just arms the
    /// state for the next captured `spu_mfc_cmd` event.
    pub fn process_wrch(&mut self, channel: u32, value: u32) -> Result<(), MfcReplayError> {
        match channel {
            ch::MFC_LSA => self.pending.lsa = Some(value),
            ch::MFC_EAH => self.pending.eah = Some(value),
            ch::MFC_EAL => self.pending.eal = Some(value),
            ch::MFC_SIZE => self.pending.size = Some(value),
            ch::MFC_TAG_ID => self.pending.tag = Some(value),
            ch::MFC_CMD => {
                // Defensive cmd-code check on the wrch value (the
                // matching `spu_mfc_cmd` event's `cmd` field is
                // re-checked in `process_mfc_cmd`). R8.1: PUT (0x20)
                // is accepted alongside GET (0x40).
                let cmd = value & 0xff;
                if cmd != MFC_GET_CMD && cmd != MFC_PUT_CMD {
                    return Err(MfcReplayError::UnsupportedMfcCmd { cmd });
                }
                self.awaiting_mfc_cmd_event = true;
            }
            ch::WR_TAG_MASK => self.wr_tag_mask = value,
            ch::WR_TAG_UPDATE => {
                self.wr_tag_update = Some(MfcTagUpdate::from_value(value)?);
            }
            other => return Err(MfcReplayError::UnsupportedMfcChannel { channel: other }),
        }
        Ok(())
    }

    /// Process the captured `spu_mfc_cmd` event. Validates the cmd /
    /// eah / size / lsa / tag fields, ensures the prior ch21 wrch
    /// armed the dispatch, loads the `.dmachunk` via the A.3 loader,
    /// and copies the bytes into `ls[lsa..lsa+size]`. Records the
    /// in-flight tag.
    ///
    /// `ls` MUST be exactly 256 KiB (`SPU_LS_SIZE`). Mismatch surfaces
    /// as `BadLsBufferSize`.
    ///
    /// **R8.1 — Caller contract for PUT (cmd=0x20):** `ls` MUST be the
    /// SPU's LS at dispatch time. The method ASSERTS
    /// `ls[lsa..lsa+size] == captured chunk bytes`. Calling this
    /// method before the SPU executed (i.e., on an LS that is still
    /// the initial image) is a misuse — use
    /// [`Self::process_mfc_cmd_pre_replay`] from the pre-replay
    /// helper layer and defer PUT verification to a post-replay step.
    pub fn process_mfc_cmd(
        &mut self,
        event: &SpuMfcCmdEvent,
        ls: &mut [u8],
    ) -> Result<(), MfcReplayError> {
        if ls.len() != SPU_LS_SIZE {
            return Err(MfcReplayError::BadLsBufferSize {
                actual: ls.len(),
                expected: SPU_LS_SIZE,
            });
        }

        if !self.awaiting_mfc_cmd_event {
            return Err(MfcReplayError::MissingMfcCmdEvent);
        }
        self.awaiting_mfc_cmd_event = false;

        // Defensive subset checks (parser already enforces, but we
        // re-validate so callers feeding hand-built events can't
        // bypass). R8.1 accepts both GET (0x40) and PUT (0x20).
        if event.cmd != MFC_GET_CMD && event.cmd != MFC_PUT_CMD {
            return Err(MfcReplayError::UnsupportedMfcCmd { cmd: event.cmd });
        }
        if event.eah != 0 {
            return Err(MfcReplayError::UnsupportedMfcEah { eah: event.eah });
        }
        if event.tag > MFC_TAG_MAX {
            return Err(MfcReplayError::BadMfcTag { tag: event.tag });
        }
        if event.size == 0 {
            return Err(MfcReplayError::BadDmaSize {
                size: event.size,
                reason: "size must be > 0",
            });
        }
        if event.size > MFC_DMA_SIZE_MAX {
            return Err(MfcReplayError::BadDmaSize {
                size: event.size,
                reason: "size > 0x4000 (16 KiB R6.7 cap)",
            });
        }
        let size_ok =
            matches!(event.size, 1 | 2 | 4 | 8) || (event.size >= 16 && event.size & 0xf == 0);
        if !size_ok {
            return Err(MfcReplayError::BadDmaSize {
                size: event.size,
                reason: "size must be 1, 2, 4, 8, or a multiple of 16 in [16, 16384]",
            });
        }
        let lsa_end =
            event.lsa.checked_add(event.size).ok_or(MfcReplayError::BadDmaLsa {
                lsa: event.lsa,
                size: event.size,
                reason: "lsa + size overflows u32",
            })?;
        if lsa_end as usize > SPU_LS_SIZE {
            return Err(MfcReplayError::BadDmaLsa {
                lsa: event.lsa,
                size: event.size,
                reason: "lsa + size exceeds 256 KiB local store",
            });
        }

        // Cross-check captured event vs pending packet assembled from
        // ch16-20 wrches. The R6.7 A.1 writer emits both atomically,
        // so any divergence is a tampered / interleaved trace.
        let (p_lsa, p_eah, p_eal, p_size, p_tag) = self.pending.require_complete()?;
        if event.lsa != p_lsa {
            return Err(MfcReplayError::MfcCmdMismatch {
                field: "lsa",
                pending: p_lsa,
                observed: event.lsa,
            });
        }
        if event.eah != p_eah {
            return Err(MfcReplayError::MfcCmdMismatch {
                field: "eah",
                pending: p_eah,
                observed: event.eah,
            });
        }
        if event.eal != p_eal {
            return Err(MfcReplayError::MfcCmdMismatch {
                field: "eal",
                pending: p_eal,
                observed: event.eal,
            });
        }
        if event.size != p_size {
            return Err(MfcReplayError::MfcCmdMismatch {
                field: "size",
                pending: p_size,
                observed: event.size,
            });
        }
        if event.tag != p_tag {
            return Err(MfcReplayError::MfcCmdMismatch {
                field: "tag",
                pending: p_tag,
                observed: event.tag,
            });
        }

        // Load + verify the .dmachunk via A.3 loader (size + SHA
        // checked atomically there).
        //
        // The semantics of the chunk content differ by cmd:
        //   GET (0x40): chunk carries the EA-source bytes the SPU
        //     received → we WRITE them into LS at [lsa..lsa+size]
        //     because there is no real EA in replay.
        //   PUT (0x20): chunk carries the LS-source bytes the SPU
        //     PRODUCED at dispatch. The dispatch-time assertion
        //     `ls[lsa..lsa+size] == bytes` is the contract — but
        //     ONLY when the LS we are inspecting IS the SPU's LS at
        //     dispatch time (i.e., the state machine is being driven
        //     in-line with the executor). When this method is called
        //     from the pre-replay helper [`apply_mfc_dma_pre_replay`]
        //     before the SPU has executed, the LS still holds the
        //     initial image bytes (typically zero at the LSA), so the
        //     assertion would spuriously fail. The helper therefore
        //     uses [`Self::process_mfc_cmd_pre_replay`], which skips
        //     the LS check; the test layer verifies post-replay that
        //     the final LS matches the captured chunk.
        let bytes = resolve_dma_chunk_side_file(
            &self.trace_path,
            &self.canonical_dma_dir,
            &event.ea_chunk_sha256,
            Some(event.size as usize),
        )?;
        debug_assert_eq!(bytes.len(), event.size as usize);

        let lo = event.lsa as usize;
        let hi = lo + event.size as usize;
        match event.cmd {
            MFC_GET_CMD => {
                ls[lo..hi].copy_from_slice(&bytes);
            }
            MFC_PUT_CMD => {
                // R8.1 — PUT replay assertion. Caller MUST guarantee
                // that the `ls` buffer is the SPU's LS at dispatch
                // time. A divergence indicates a real correctness
                // gap in the Rust SPU stack — do NOT weaken this
                // assertion to "make replay pass".
                for (offset, (cap, obs)) in bytes.iter().zip(&ls[lo..hi]).enumerate() {
                    if cap != obs {
                        return Err(MfcReplayError::PutLsBytesMismatch {
                            tag: event.tag,
                            lsa: event.lsa,
                            size: event.size,
                            first_diff_offset: offset as u32,
                            captured: *cap,
                            observed: *obs,
                        });
                    }
                }
            }
            // unreachable: validated above
            other => return Err(MfcReplayError::UnsupportedMfcCmd { cmd: other }),
        }

        // Record in-flight; will be promoted to completed by
        // process_mfc_dma_complete.
        let ea = ((event.eah as u64) << 32) | (event.eal as u64);
        self.in_flight.insert(
            event.tag,
            MfcInFlight {
                cmd: event.cmd,
                size: event.size,
                lsa: event.lsa,
                ea,
                chunk_sha256: event.ea_chunk_sha256.clone(),
            },
        );

        // Reset pending so the next cmd starts from a clean slate.
        // ch22/ch23 state (tag mask + update mode) is NOT reset — those
        // persist across multiple dispatches in the same wait round.
        self.pending.clear();

        Ok(())
    }

    /// R8.1 — pre-replay variant of [`Self::process_mfc_cmd`]. Same
    /// validations, chunk SHA + size verification, in-flight
    /// registration, and pending-packet cross-check. Differs ONLY in
    /// the per-cmd LS handling:
    ///
    /// - **GET (0x40):** identical to [`Self::process_mfc_cmd`] — the
    ///   captured chunk is COPIED into `ls[lsa..lsa+size]` so the SPU
    ///   reads the right bytes when it later steps past `wrch ch21`.
    /// - **PUT (0x20):** the LS bytes assertion is DEFERRED. The
    ///   chunk is still loaded (which validates the side-file SHA +
    ///   size via the A.3 loader), but no comparison to `ls[..]`
    ///   happens. Callers (typically [`apply_mfc_dma_pre_replay`])
    ///   are responsible for verifying post-replay that the SPU's
    ///   final LS at `[lsa..lsa+size]` matches the captured chunk.
    ///
    /// This split exists because the pre-replay helper runs BEFORE
    /// the SPU executes — there is no way to inspect dispatch-time LS
    /// at that point. Doing the PUT assertion in-line with the
    /// executor (a future R-phase) would restore the
    /// [`Self::process_mfc_cmd`] contract.
    pub fn process_mfc_cmd_pre_replay(
        &mut self,
        event: &SpuMfcCmdEvent,
        ls: &mut [u8],
    ) -> Result<(), MfcReplayError> {
        if event.cmd != MFC_PUT_CMD {
            return self.process_mfc_cmd(event, ls);
        }

        // PUT path: temporarily stage the captured chunk into `ls`
        // before calling `process_mfc_cmd`, so the dispatch-time
        // assertion sees a vacuous match. The bytes we wrote are then
        // discarded by restoring the prior contents — the SPU's own
        // executed bytecode populates the real LS during replay. This
        // keeps all the validation + in-flight + pending bookkeeping
        // shared with the AssertNow path without duplicating ~150
        // lines of defensive checks.
        if ls.len() != SPU_LS_SIZE {
            return Err(MfcReplayError::BadLsBufferSize {
                actual: ls.len(),
                expected: SPU_LS_SIZE,
            });
        }

        let lsa_end =
            event.lsa.checked_add(event.size).ok_or(MfcReplayError::BadDmaLsa {
                lsa: event.lsa,
                size: event.size,
                reason: "lsa + size overflows u32",
            })?;
        if lsa_end as usize > SPU_LS_SIZE {
            return Err(MfcReplayError::BadDmaLsa {
                lsa: event.lsa,
                size: event.size,
                reason: "lsa + size exceeds 256 KiB local store",
            });
        }

        let lo = event.lsa as usize;
        let hi = lo + event.size as usize;
        let saved: Vec<u8> = ls[lo..hi].to_vec();

        let staged = resolve_dma_chunk_side_file(
            &self.trace_path,
            &self.canonical_dma_dir,
            &event.ea_chunk_sha256,
            Some(event.size as usize),
        )?;
        ls[lo..hi].copy_from_slice(&staged);

        let result = self.process_mfc_cmd(event, ls);

        // Always restore — the staged bytes were a vacuous-match
        // scaffold, not the real LS state to carry into replay.
        ls[lo..hi].copy_from_slice(&saved);

        result
    }

    /// Process a captured `mfc_dma_complete` event. Validates that the
    /// tag is in flight and that `transferred_bytes` matches the
    /// dispatched `size`, then promotes the tag to completed.
    pub fn process_mfc_dma_complete(
        &mut self,
        event: &MfcDmaCompleteEvent,
    ) -> Result<(), MfcReplayError> {
        if event.tag > MFC_TAG_MAX {
            return Err(MfcReplayError::BadMfcTag { tag: event.tag });
        }

        let in_flight = self
            .in_flight
            .remove(&event.tag)
            .ok_or(MfcReplayError::MfcDmaCompleteMismatch {
                reason: "tag not in flight",
            })?;
        if event.transferred_bytes != in_flight.size {
            // Re-insert so a future retry would see the in-flight
            // state (even though this error is non-recoverable in
            // current shape).
            self.in_flight.insert(event.tag, in_flight);
            return Err(MfcReplayError::MfcDmaCompleteMismatch {
                reason: "transferred_bytes != dispatched size",
            });
        }

        self.completed_tags |= 1u32 << event.tag;
        Ok(())
    }

    /// Process a captured `spu_rdch ch24` (`MFC_RdTagStat`). Returns
    /// the oracle tag stat (computed from the in-flight + completed
    /// state) and validates against the captured `value`. Clears the
    /// observed bits.
    ///
    /// **Wait-mode semantics** per design § 9.3:
    /// - `Immediate`: returns whatever is currently set in
    ///   `completed_tags & wr_tag_mask`. The captured value MUST
    ///   match exactly.
    /// - `Any`: at least one tag in the mask must be completed. The
    ///   returned value is the mask of completed-and-in-flight tags
    ///   intersected with `wr_tag_mask`.
    /// - `All`: every tag in the mask must be completed. The returned
    ///   value is `wr_tag_mask` exactly (or a strict superset for
    ///   defensive reasons).
    pub fn process_rdch_tagstat(&mut self, captured_value: u32) -> Result<u32, MfcReplayError> {
        let mode = self
            .wr_tag_update
            .ok_or(MfcReplayError::UnsupportedMfcRdTagMode { mode: u32::MAX })?;
        let mask = self.wr_tag_mask;
        let observed_now = self.completed_tags & mask;

        match mode {
            MfcTagUpdate::Immediate => {
                // No wait — return what's currently set.
            }
            MfcTagUpdate::Any => {
                if observed_now == 0 {
                    // Pick any tag in the mask that's missing its
                    // mfc_dma_complete to surface in the error.
                    let missing_tag = mask.trailing_zeros();
                    return Err(MfcReplayError::MissingMfcDmaComplete { tag: missing_tag });
                }
            }
            MfcTagUpdate::All => {
                if observed_now != mask {
                    let missing = mask & !observed_now;
                    let missing_tag = missing.trailing_zeros();
                    return Err(MfcReplayError::MissingMfcDmaComplete { tag: missing_tag });
                }
            }
        }

        if captured_value != observed_now {
            return Err(MfcReplayError::TagStatMismatch {
                captured: captured_value,
                oracle: observed_now,
                wr_tag_mask: mask,
                mode,
            });
        }

        // R8.3c — DO NOT clear `completed_tags` after read. Cell BE
        // semantics retain the register across reads; the matching
        // `SpuChannels::read(MFC_RD_TAG_STAT)` in `rpcs3-spu-thread`
        // (R8.3b persistent semantic) returns
        // `completed_tags & wr_tag_mask` without clearing, so the
        // pre-replay state machine MUST match. Without this, a
        // fixture whose ch24 reads have overlapping masks (R8.3c:
        // first mask 0x08 ⊂ second mask 0x28) trips
        // `TagStatMismatch` because the bit shared between masks
        // would be cleared after the first read and missing from
        // the second oracle.
        //
        // The previous clear was a legacy from R6.7 A.4 when no
        // oracle exercised repeated reads. R8.3b (separate ANY-mode
        // masks, no overlap) didn't surface the issue because each
        // read cleared only bits unique to its mask. R8.3c forces
        // the alignment.
        Ok(observed_now)
    }
}

// =====================================================================
// R6.7 C.3 — Pre-replay DMA application helper
// =====================================================================

/// Outcome of [`apply_mfc_dma_pre_replay`]: a fresh [`SpuProgram`] with
/// any captured GET DMA already injected into a single 256 KiB LS
/// segment, plus the queue of `rdch ch24 (RdTagStat)` values the SPU
/// will pop in order during replay. The caller plumbs the queue into
/// the backend's `SpuChannels::mfc_tag_stat_queue` (typically via
/// `SpuProgram::with_mfc_tag_stat_queue`).
#[derive(Debug, Clone)]
pub struct DmaPreReplayPlan {
    /// New program: same `entry_pc`, `max_steps`, `initial_gpr_overrides`
    /// as the input program, but with `segments` replaced by a single
    /// segment at lsa=0 holding the post-DMA 256 KiB LS image.
    pub program: SpuProgram,
    /// Tag-stat values the SPU will read via `rdch ch24` during replay,
    /// in the captured order. Empty if the trace contains zero MFC
    /// events.
    pub tag_stat_queue: VecDeque<u32>,
    /// Number of GET dispatches applied to LS (= count of
    /// `SpuMfcCmd` events processed). Mostly informational for
    /// callers / tests / diagnostics.
    pub dispatched_get_count: u32,
}

/// Walk a captured event slice (filtered to a single `target_spu`) and:
///
/// 1. Build a 256 KiB LS scratch buffer initialized from the input
///    `program`'s segments (so the SPU's image bytes stay where they
///    were captured).
/// 2. For each `SpuWrch` to MFC params (ch16-23), drive
///    [`MfcReplayState::process_wrch`].
/// 3. For each `SpuMfcCmd`, drive [`MfcReplayState::process_mfc_cmd`]
///    — this loads the `.dmachunk` via the A.3 loader and copies
///    bytes into the scratch LS at `[lsa..lsa+size]`.
/// 4. For each `MfcDmaComplete`, drive
///    [`MfcReplayState::process_mfc_dma_complete`].
/// 5. For each `SpuRdch` to ch24 (`RdTagStat`), validate the captured
///    value against the state machine's oracle and append it to the
///    tag-stat queue the SPU will pop during replay.
/// 6. Replace `program.segments` with a single segment at lsa=0
///    containing the post-DMA scratch LS, and bundle the tag-stat
///    queue + dispatch count into a [`DmaPreReplayPlan`].
///
/// **Pre-replay-only.** This function is the bridge between the A.3
/// loader and the SPU executor. The actual SPU runs unchanged after
/// the plan is applied — the SPU's own `wrch ch16-21` instructions
/// during replay are no-ops (or rather, store-only stashes that don't
/// re-do the DMA), and `rdch ch24` pops from the queue this function
/// pre-populates. This is the simplest correct integration for the
/// R6.7 GET-only subset; runtime DMA + multi-stage GET-after-PUT are
/// out of scope.
///
/// **Limitations of the pre-application model:**
/// - If the SPU writes to the GET destination LS region BEFORE its
///   own `wrch ch21` would have dispatched the GET, those writes
///   still happen in pre-application order (immediately at thread
///   start) but the SPU's later writes will overwrite them. This
///   isn't a correctness issue for the R6.7 fixture
///   (`single_spu_dma_get_v1`) where the SPU dispatches GET first
///   and only reads from the destination after — the design § 9.7
///   PUT discussion notes this trade-off.
/// - Multiple non-overlapping GETs in the same trace ALL get
///   pre-applied; their order matches the captured event order.
pub fn apply_mfc_dma_pre_replay(
    events: &[CapturedEvent],
    trace_path: &Path,
    canonical_dma_dir: &Path,
    program: SpuProgram,
) -> Result<DmaPreReplayPlan, MfcReplayError> {
    // Build the scratch LS from the input program's segments so the
    // captured image bytes remain in place. The default-zero buffer
    // is then overlaid with each segment, and finally with each
    // GET's chunk bytes at the captured (lsa, size).
    let mut ls = vec![0u8; SPU_LS_SIZE];
    for seg in &program.segments {
        let lo = seg.lsa as usize;
        let hi = lo
            .checked_add(seg.data.len())
            .ok_or(MfcReplayError::BadDmaLsa {
                lsa: seg.lsa,
                size: seg.data.len() as u32,
                reason: "segment lsa + data overflows usize",
            })?;
        if hi > SPU_LS_SIZE {
            return Err(MfcReplayError::BadDmaLsa {
                lsa: seg.lsa,
                size: seg.data.len() as u32,
                reason: "segment lsa + data exceeds 256 KiB LS",
            });
        }
        ls[lo..hi].copy_from_slice(&seg.data);
    }

    let mut state = MfcReplayState::new(trace_path, canonical_dma_dir);
    let mut tag_stat_queue: VecDeque<u32> = VecDeque::new();
    let mut dispatched_get_count: u32 = 0;

    for event in events {
        match event {
            CapturedEvent::SpuWrch(w) if (16..=23).contains(&w.channel) => {
                state.process_wrch(w.channel, w.value)?;
            }
            CapturedEvent::SpuMfcCmd(m) => {
                state.process_mfc_cmd_pre_replay(m, &mut ls)?;
                dispatched_get_count = dispatched_get_count.saturating_add(1);
            }
            CapturedEvent::MfcDmaComplete(c) => {
                state.process_mfc_dma_complete(c)?;
            }
            CapturedEvent::SpuRdch(r) if r.channel == 24 => {
                let captured_value = r.value.unwrap_or(0);
                let oracle = state.process_rdch_tagstat(captured_value)?;
                debug_assert_eq!(oracle, captured_value);
                tag_stat_queue.push_back(captured_value);
            }
            _ => {
                // Other captured events (PPU push/pop, SNR signals,
                // park/wake/stop/final_state, spu_image, non-MFC
                // wrches, non-ch24 rdches) are NOT consumed here —
                // they pass through to the trace transformer's
                // existing path. The pre-replay plan only handles
                // the MFC subset.
            }
        }
    }

    // Construct the post-DMA program: same metadata, single LS segment.
    let post_dma_program = SpuProgram {
        entry_pc: program.entry_pc,
        segments: vec![crate::SpuSegment { lsa: 0, data: ls }],
        max_steps: program.max_steps,
        initial_gpr_overrides: program.initial_gpr_overrides,
        // The tag-stat queue is delivered via the returned plan, not
        // baked into the program here — caller decides whether to
        // call `with_mfc_tag_stat_queue` on this program. That keeps
        // the helper composable with callers who already have a
        // queue from another source.
        initial_mfc_tag_stat_queue: Vec::new(),
    };

    Ok(DmaPreReplayPlan {
        program: post_dma_program,
        tag_stat_queue,
        dispatched_get_count,
    })
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use crate::trace_fmt::CapturedSide;

    use super::*;

    /// Synthetic counting-pattern bytes + their SHA-256 lowercase hex.
    fn synthetic_chunk(size: usize) -> (Vec<u8>, String) {
        let bytes: Vec<u8> = (0..size).map(|i| (i & 0xFF) as u8).collect();
        let mut h = Sha256::new();
        h.update(&bytes);
        let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        (bytes, hex)
    }

    /// Write `bytes` to `<tmp>/<trace_name>.dma/<sha>.dmachunk`,
    /// returning the resolved trace_path + canonical_dir to feed
    /// `MfcReplayState::new`.
    fn setup_per_trace_chunk(
        tmp: &TempDir,
        trace_name: &str,
        sha: &str,
        bytes: &[u8],
    ) -> (PathBuf, PathBuf) {
        let trace_path = tmp.path().join(trace_name);
        let mut dir_str = trace_path.as_os_str().to_owned();
        dir_str.push(".dma");
        let dma_dir = PathBuf::from(dir_str);
        std::fs::create_dir_all(&dma_dir).unwrap();
        let chunk_path = dma_dir.join(format!("{sha}.dmachunk"));
        let mut f = std::fs::File::create(&chunk_path).unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        let canonical = tmp.path().join("canonical_dma");
        std::fs::create_dir_all(&canonical).unwrap();
        (trace_path, canonical)
    }

    fn make_mfc_cmd_event(
        target_spu: u32,
        tag: u32,
        size: u32,
        lsa: u32,
        eal: u32,
        sha: &str,
    ) -> SpuMfcCmdEvent {
        SpuMfcCmdEvent {
            seq: 100,
            side: CapturedSide::Spu,
            target_spu,
            pc: 256,
            cmd: MFC_GET_CMD,
            tag,
            size,
            lsa,
            eah: 0,
            eal,
            ea_chunk_sha256: sha.to_owned(),
        }
    }

    fn make_dma_complete_event(target_spu: u32, tag: u32, transferred_bytes: u32) -> MfcDmaCompleteEvent {
        MfcDmaCompleteEvent {
            seq: 101,
            side: CapturedSide::Spu,
            target_spu,
            tag,
            transferred_bytes,
        }
    }

    /// Drive a full ch16→ch17→ch18→ch19→ch20→ch22→ch23→ch21 wrch
    /// sequence into the state machine, leaving `awaiting_mfc_cmd_event`
    /// true. Used as a setup step in the happy-path tests.
    fn arm_mfc_get(
        st: &mut MfcReplayState,
        lsa: u32,
        eah: u32,
        eal: u32,
        size: u32,
        tag: u32,
        wr_tag_mask: u32,
        wr_tag_update_value: u32,
    ) {
        st.process_wrch(ch::MFC_LSA, lsa).unwrap();
        st.process_wrch(ch::MFC_EAH, eah).unwrap();
        st.process_wrch(ch::MFC_EAL, eal).unwrap();
        st.process_wrch(ch::MFC_SIZE, size).unwrap();
        st.process_wrch(ch::MFC_TAG_ID, tag).unwrap();
        st.process_wrch(ch::WR_TAG_MASK, wr_tag_mask).unwrap();
        st.process_wrch(ch::WR_TAG_UPDATE, wr_tag_update_value).unwrap();
        st.process_wrch(ch::MFC_CMD, MFC_GET_CMD).unwrap();
    }

    #[test]
    fn mfc_replay_copies_get_chunk_into_ls() {
        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let (trace_path, canonical) = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);

        let mut st = MfcReplayState::new(trace_path, canonical);
        let mut ls = vec![0u8; SPU_LS_SIZE];

        arm_mfc_get(&mut st, /*lsa=*/0x1000, 0, 0xD000, 128, 3, /*mask=*/1 << 3, /*mode=*/2);
        let cmd = make_mfc_cmd_event(1, 3, 128, 0x1000, 0xD000, &sha);
        st.process_mfc_cmd(&cmd, &mut ls).expect("GET dispatch must copy chunk into LS");

        let lo = 0x1000usize;
        let hi = lo + 128;
        assert_eq!(&ls[lo..hi], bytes.as_slice(), "LS at [lsa..lsa+size] must equal chunk bytes");

        // Bytes outside the GET window are still zero.
        assert!(ls[..lo].iter().all(|&b| b == 0));
        assert!(ls[hi..].iter().all(|&b| b == 0));

        // Now complete + observe.
        let complete = make_dma_complete_event(1, 3, 128);
        st.process_mfc_dma_complete(&complete).expect("matching complete must succeed");
        assert_eq!(st.completed_tags(), 1u32 << 3);

        // rdch ch24: ALL mode + mask = (1<<3), all completed → returns
        // the mask exactly. Captured value must match.
        let stat = st.process_rdch_tagstat(1u32 << 3).expect("tag stat must match oracle");
        assert_eq!(stat, 1u32 << 3);
        // R8.3c: completed_tags is NOT cleared on read — Cell BE
        // persistent register semantic. The bit stays set so
        // subsequent reads with the same mask return the same value.
        assert_eq!(st.completed_tags(), 1u32 << 3);
        let stat2 = st.process_rdch_tagstat(1u32 << 3).expect("re-read returns same value");
        assert_eq!(stat2, 1u32 << 3);
        assert_eq!(st.completed_tags(), 1u32 << 3);
    }

    #[test]
    fn mfc_replay_rejects_missing_chunk() {
        let tmp = TempDir::new().unwrap();
        // No .dmachunk written.
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");
        std::fs::create_dir_all(&canonical).unwrap();

        let (_, sha) = synthetic_chunk(128);
        let mut st = MfcReplayState::new(trace_path, canonical);
        let mut ls = vec![0u8; SPU_LS_SIZE];

        arm_mfc_get(&mut st, 0, 0, 0xD000, 128, 0, 1, 2);
        let cmd = make_mfc_cmd_event(1, 0, 128, 0, 0xD000, &sha);
        let err = st.process_mfc_cmd(&cmd, &mut ls).expect_err("missing chunk must error");
        match err {
            MfcReplayError::DmaChunkLoad(DmaChunkLoadError::MissingDmaChunk { .. }) => {}
            other => panic!("expected DmaChunkLoad(MissingDmaChunk), got {other:?}"),
        }
    }

    #[test]
    fn mfc_replay_rejects_sha_mismatch() {
        let tmp = TempDir::new().unwrap();
        let (bytes, real_sha) = synthetic_chunk(128);
        // Write the file under the REAL sha, but the cmd event will
        // claim a different bogus sha — loader can't find a chunk
        // matching the bogus sha → MissingDmaChunk. Different from
        // the "filename lies about contents" case in dma_chunk's
        // sha-mismatch test.
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &real_sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        let bogus_sha = "f".repeat(64);
        let mut st = MfcReplayState::new(trace_path, canonical);
        let mut ls = vec![0u8; SPU_LS_SIZE];

        arm_mfc_get(&mut st, 0, 0, 0xD000, 128, 0, 1, 2);
        let cmd = make_mfc_cmd_event(1, 0, 128, 0, 0xD000, &bogus_sha);
        let err = st.process_mfc_cmd(&cmd, &mut ls)
            .expect_err("event sha not on disk must error");
        match err {
            MfcReplayError::DmaChunkLoad(DmaChunkLoadError::MissingDmaChunk { .. }) => {}
            other => panic!("expected DmaChunkLoad(MissingDmaChunk), got {other:?}"),
        }
    }

    #[test]
    fn mfc_replay_rejects_cmd_mismatch() {
        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        let mut st = MfcReplayState::new(trace_path, canonical);
        let mut ls = vec![0u8; SPU_LS_SIZE];

        // Pending: lsa=0x1000. Event: lsa=0x2000. Mismatch must fire.
        arm_mfc_get(&mut st, /*lsa=*/0x1000, 0, 0xD000, 128, 3, 1 << 3, 2);
        let cmd = make_mfc_cmd_event(1, 3, 128, /*lsa=*/0x2000, 0xD000, &sha);
        let err = st.process_mfc_cmd(&cmd, &mut ls).expect_err("lsa mismatch must error");
        match err {
            MfcReplayError::MfcCmdMismatch { field: "lsa", pending: 0x1000, observed: 0x2000 } => {}
            other => panic!("expected MfcCmdMismatch lsa, got {other:?}"),
        }
    }

    #[test]
    fn mfc_replay_rejects_missing_complete_before_tagstat() {
        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        let mut st = MfcReplayState::new(trace_path, canonical);
        let mut ls = vec![0u8; SPU_LS_SIZE];

        arm_mfc_get(&mut st, 0x1000, 0, 0xD000, 128, 5, /*mask=*/1 << 5, /*mode=*/1);
        let cmd = make_mfc_cmd_event(1, 5, 128, 0x1000, 0xD000, &sha);
        st.process_mfc_cmd(&cmd, &mut ls).unwrap();

        // No process_mfc_dma_complete before rdch.
        let err = st.process_rdch_tagstat(1 << 5)
            .expect_err("rdch ch24 before complete (mode=Any) must error");
        match err {
            MfcReplayError::MissingMfcDmaComplete { tag: 5 } => {}
            other => panic!("expected MissingMfcDmaComplete, got {other:?}"),
        }
    }

    #[test]
    fn mfc_replay_rejects_bad_transferred_bytes() {
        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        let mut st = MfcReplayState::new(trace_path, canonical);
        let mut ls = vec![0u8; SPU_LS_SIZE];

        arm_mfc_get(&mut st, 0, 0, 0xD000, 128, 3, 1 << 3, 2);
        let cmd = make_mfc_cmd_event(1, 3, 128, 0, 0xD000, &sha);
        st.process_mfc_cmd(&cmd, &mut ls).unwrap();

        // Captured complete claims 64 bytes — disagrees with dispatched 128.
        let complete = make_dma_complete_event(1, 3, 64);
        let err = st.process_mfc_dma_complete(&complete).expect_err("transferred_bytes mismatch must error");
        match err {
            MfcReplayError::MfcDmaCompleteMismatch { reason } => {
                assert!(reason.contains("transferred_bytes"), "got reason {reason}");
            }
            other => panic!("expected MfcDmaCompleteMismatch, got {other:?}"),
        }
    }

    #[test]
    fn mfc_replay_handles_wr_tag_mask_update_basic() {
        // Two tags: 3 and 5. mask = (1<<3)|(1<<5) = 0x28. Mode ALL
        // requires both completed before the rdch ch24. Confirm the
        // oracle returns 0x28 only after both completes.
        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        let mut st = MfcReplayState::new(trace_path, canonical);
        let mut ls = vec![0u8; SPU_LS_SIZE];

        // Dispatch tag 3.
        arm_mfc_get(&mut st, 0x1000, 0, 0xD000, 128, 3, /*mask=*/0x28, /*mode=*/2);
        let cmd3 = make_mfc_cmd_event(1, 3, 128, 0x1000, 0xD000, &sha);
        st.process_mfc_cmd(&cmd3, &mut ls).unwrap();

        // Dispatch tag 5 with a NEW pending packet (ch16-20 again).
        arm_mfc_get(&mut st, 0x2000, 0, 0xD080, 128, 5, /*mask=*/0x28, /*mode=*/2);
        let cmd5 = make_mfc_cmd_event(1, 5, 128, 0x2000, 0xD080, &sha);
        st.process_mfc_cmd(&cmd5, &mut ls).unwrap();

        // Only tag 3 complete — ALL mode rdch must error (5 missing).
        st.process_mfc_dma_complete(&make_dma_complete_event(1, 3, 128)).unwrap();
        let err = st.process_rdch_tagstat(0x28)
            .expect_err("ALL mode requires every tag in mask completed");
        assert!(matches!(err, MfcReplayError::MissingMfcDmaComplete { tag: 5 }), "got {err:?}");

        // Now complete tag 5 — both bits set; rdch returns mask exactly.
        st.process_mfc_dma_complete(&make_dma_complete_event(1, 5, 128)).unwrap();
        let stat = st.process_rdch_tagstat(0x28).expect("ALL mode + both completed must match");
        assert_eq!(stat, 0x28);
    }

    /// R6.7 C.5 — Phase C closure point: with executor wiring
    /// landed (ch16-25 in `rpcs3-spu-thread::ch::`, MFC fields on
    /// `SpuChannels`, pre-application via
    /// [`apply_mfc_dma_pre_replay`]), the transformer now ACCEPTS
    /// valid GET traces. This test (renamed from the A.4 rejection
    /// gate) verifies the new policy: parse + transform succeed,
    /// no `UnsupportedDmaInTrace` surfaces. The companion test
    /// `transformer_accepts_valid_get_mfc_trace_after_executor_wiring`
    /// in `trace_fmt::tests` pins the same invariant from the
    /// transformer side.
    #[test]
    fn transformer_now_accepts_valid_get_mfc_trace_after_executor_wiring() {
        use crate::trace_fmt::{captured_events_to_traces_per_spu, parse_jsonl_trace};

        let valid_sha = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{valid_sha}"}}
{{"seq":2,"side":"spu","kind":"mfc_dma_complete","target_spu":1,"tag":3,"transferred_bytes":128}}
{{"seq":3,"side":"spu","kind":"spu_stop","pc":260,"stop_code":1,"target_spu":1}}
{{"seq":4,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}},"target_spu":1}}
"#
        );
        let events = parse_jsonl_trace(&jsonl).expect("parser still accepts the MFC sequence");
        let _per_spu = captured_events_to_traces_per_spu(&events)
            .expect("transformer must accept valid MFC trace post Phase C — pre-replay layer takes over");
    }

    /// A.4 invariant: the 6 existing oracle replay fixtures don't
    /// contain MFC events, so the new state machine is dormant for
    /// them. We re-confirm the canonical R5.6 reference still parses
    /// + transforms cleanly (the load-bearing legacy invariant).
    #[test]
    fn existing_non_dma_traces_unchanged() {
        use crate::trace_fmt::{
            captured_events_to_trace, parse_jsonl_trace, R5_6_REFERENCE_JSONL,
        };
        let events = parse_jsonl_trace(R5_6_REFERENCE_JSONL)
            .expect("R5.6 reference still parses");
        let trace = captured_events_to_trace(&events)
            .expect("R5.6 reference still transforms");
        // 16 TraceEvents — same number A.2 + A.3 produced.
        assert_eq!(trace.len(), 16, "R5.6 transform must be byte-stable");
    }

    #[test]
    fn process_mfc_cmd_rejects_missing_ch21_armed() {
        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        let mut st = MfcReplayState::new(trace_path, canonical);
        let mut ls = vec![0u8; SPU_LS_SIZE];

        // Skip ch21 — emit cmd event without arming.
        st.process_wrch(ch::MFC_LSA, 0).unwrap();
        st.process_wrch(ch::MFC_EAH, 0).unwrap();
        st.process_wrch(ch::MFC_EAL, 0xD000).unwrap();
        st.process_wrch(ch::MFC_SIZE, 128).unwrap();
        st.process_wrch(ch::MFC_TAG_ID, 3).unwrap();

        let cmd = make_mfc_cmd_event(1, 3, 128, 0, 0xD000, &sha);
        let err = st.process_mfc_cmd(&cmd, &mut ls)
            .expect_err("missing ch21 wrch arming must error");
        assert!(matches!(err, MfcReplayError::MissingMfcCmdEvent), "got {err:?}");
    }

    #[test]
    fn process_wrch_rejects_unsupported_cmd_code() {
        let tmp = TempDir::new().unwrap();
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");
        std::fs::create_dir_all(&canonical).unwrap();
        let mut st = MfcReplayState::new(trace_path, canonical);

        // R8.1 update: GET (0x40) and PUT (0x20) are both in scope.
        // GETL (0x44, list variant) is the new canary — out of R8.1
        // scope (list DMA defers to R8.2+).
        let err = st.process_wrch(ch::MFC_CMD, 0x44)
            .expect_err("GETL cmd must be rejected");
        assert!(matches!(err, MfcReplayError::UnsupportedMfcCmd { cmd: 0x44 }), "got {err:?}");

        // Sanity: PUT (0x20) is NOW accepted under R8.1.
        let mut st2 = MfcReplayState::new(tmp.path().join("c2.jsonl"), tmp.path().join("d2"));
        std::fs::create_dir_all(tmp.path().join("d2")).unwrap();
        st2.process_wrch(ch::MFC_CMD, 0x20)
            .expect("PUT cmd must now be accepted (R8.1)");
    }

    #[test]
    fn process_wrch_rejects_unknown_channel() {
        let tmp = TempDir::new().unwrap();
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");
        std::fs::create_dir_all(&canonical).unwrap();
        let mut st = MfcReplayState::new(trace_path, canonical);

        let err = st.process_wrch(99, 0)
            .expect_err("non-MFC channel must be rejected");
        assert!(matches!(err, MfcReplayError::UnsupportedMfcChannel { channel: 99 }), "got {err:?}");
    }

    #[test]
    fn process_mfc_cmd_rejects_bad_ls_buffer_size() {
        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        let mut st = MfcReplayState::new(trace_path, canonical);
        // Wrong-size LS buffer (smaller than 256 KiB).
        let mut ls_too_small = vec![0u8; 4096];

        arm_mfc_get(&mut st, 0, 0, 0xD000, 128, 0, 1, 2);
        let cmd = make_mfc_cmd_event(1, 0, 128, 0, 0xD000, &sha);
        let err = st.process_mfc_cmd(&cmd, &mut ls_too_small)
            .expect_err("undersized LS buffer must error");
        match err {
            MfcReplayError::BadLsBufferSize { actual: 4096, expected: 0x40000 } => {}
            other => panic!("expected BadLsBufferSize, got {other:?}"),
        }
    }

    // =================================================================
    // R6.7 C.3 — apply_mfc_dma_pre_replay tests
    //
    // The pre-replay helper takes a CapturedEvent slice, threads it
    // through MfcReplayState, and produces a DmaPreReplayPlan whose
    // SpuProgram has the post-DMA LS image as a single segment AND
    // whose tag_stat_queue is ready for the executor's
    // SpuChannels::mfc_tag_stat_queue. These tests verify the helper
    // composes A.3 (loader) + A.4 (state machine) + C.4 (program
    // builder field) correctly.
    // =================================================================

    use crate::trace_fmt::{SpuRdchEvent, SpuStopEvent, SpuWrchEvent};

    /// Build the canonical 8-event GET sequence as a Vec<CapturedEvent>:
    /// ch16-20 + ch22-23 + ch21 wrches → spu_mfc_cmd → mfc_dma_complete
    /// → rdch ch24. Tag-stat sequence + lsa parametrized so tests can
    /// exercise variations.
    fn build_get_event_sequence(
        target_spu: u32,
        lsa: u32,
        size: u32,
        tag: u32,
        sha: &str,
    ) -> Vec<CapturedEvent> {
        let mask = 1u32 << tag;
        vec![
            CapturedEvent::SpuWrch(SpuWrchEvent {
                seq: 0, side: CapturedSide::Spu, pc: 256, channel: 16,
                value: lsa, would_stall: false, target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuWrch(SpuWrchEvent {
                seq: 1, side: CapturedSide::Spu, pc: 260, channel: 17,
                value: 0, would_stall: false, target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuWrch(SpuWrchEvent {
                seq: 2, side: CapturedSide::Spu, pc: 264, channel: 18,
                value: 0xD0010000, would_stall: false, target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuWrch(SpuWrchEvent {
                seq: 3, side: CapturedSide::Spu, pc: 268, channel: 19,
                value: size, would_stall: false, target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuWrch(SpuWrchEvent {
                seq: 4, side: CapturedSide::Spu, pc: 272, channel: 20,
                value: tag, would_stall: false, target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuWrch(SpuWrchEvent {
                seq: 5, side: CapturedSide::Spu, pc: 276, channel: 22,
                value: mask, would_stall: false, target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuWrch(SpuWrchEvent {
                seq: 6, side: CapturedSide::Spu, pc: 280, channel: 23,
                value: 2, would_stall: false, target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuWrch(SpuWrchEvent {
                seq: 7, side: CapturedSide::Spu, pc: 284, channel: 21,
                value: MFC_GET_CMD, would_stall: false, target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuMfcCmd(SpuMfcCmdEvent {
                seq: 8, side: CapturedSide::Spu, target_spu, pc: 284,
                cmd: MFC_GET_CMD, tag, size, lsa, eah: 0, eal: 0xD0010000,
                ea_chunk_sha256: sha.to_owned(),
            }),
            CapturedEvent::MfcDmaComplete(MfcDmaCompleteEvent {
                seq: 9, side: CapturedSide::Spu, target_spu,
                tag, transferred_bytes: size,
            }),
            CapturedEvent::SpuRdch(SpuRdchEvent {
                seq: 10, side: CapturedSide::Spu, pc: 288, channel: 24,
                value: Some(mask), would_stall: false,
                target_spu: Some(target_spu),
            }),
            CapturedEvent::SpuStop(SpuStopEvent {
                seq: 11, side: CapturedSide::Spu, pc: 292, stop_code: 1,
                target_spu: Some(target_spu),
            }),
        ]
    }

    #[test]
    fn apply_mfc_dma_pre_replay_injects_chunk_into_ls_segment() {
        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        let events = build_get_event_sequence(1, 0x1000, 128, 3, &sha);
        // Input program: a tiny code segment at LS=0x100 (entry_pc).
        let input = SpuProgram::new(0x100, 1_000_000)
            .with_segment(0x100, vec![0xAB; 16]);

        let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &canonical, input)
            .expect("pre-replay must apply DMA + collect tag-stat");

        // Plan has one consolidated 256 KiB LS segment.
        assert_eq!(plan.program.segments.len(), 1);
        assert_eq!(plan.program.segments[0].lsa, 0);
        assert_eq!(plan.program.segments[0].data.len(), SPU_LS_SIZE);

        // Original code segment bytes preserved at 0x100.
        let ls = &plan.program.segments[0].data;
        assert_eq!(&ls[0x100..0x110], &[0xAB; 16][..],
            "input program's segment bytes preserved through pre-replay");

        // GET destination at lsa=0x1000 holds the chunk bytes.
        assert_eq!(&ls[0x1000..0x1080], bytes.as_slice(),
            "GET destination LS region holds the chunk's bytes");

        // Tag-stat queue populated with the captured rdch ch24 value.
        assert_eq!(plan.tag_stat_queue.len(), 1);
        assert_eq!(plan.tag_stat_queue[0], 1u32 << 3);

        // Dispatched count is exactly 1 (one SpuMfcCmd consumed).
        assert_eq!(plan.dispatched_get_count, 1);

        // Other program metadata flowed through unchanged.
        assert_eq!(plan.program.entry_pc, 0x100);
        assert_eq!(plan.program.max_steps, 1_000_000);
    }

    #[test]
    fn apply_mfc_dma_pre_replay_returns_empty_plan_for_non_dma_trace() {
        let tmp = TempDir::new().unwrap();
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");
        std::fs::create_dir_all(&canonical).unwrap();

        // No MFC events; just a stop. The helper should produce a
        // pass-through plan: same LS bytes, empty tag-stat queue,
        // zero dispatches.
        let events = vec![CapturedEvent::SpuStop(SpuStopEvent {
            seq: 0, side: CapturedSide::Spu, pc: 256,
            stop_code: 1, target_spu: Some(1),
        })];

        let input = SpuProgram::new(0x100, 1)
            .with_segment(0x100, vec![0xCD; 32]);

        let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &canonical, input)
            .expect("pre-replay on non-DMA trace must succeed");

        // The resulting LS is 256 KiB with the input bytes preserved at 0x100.
        let ls = &plan.program.segments[0].data;
        assert_eq!(&ls[0x100..0x120], &[0xCD; 32][..]);
        assert!(plan.tag_stat_queue.is_empty());
        assert_eq!(plan.dispatched_get_count, 0);
    }

    #[test]
    fn apply_mfc_dma_pre_replay_propagates_loader_errors() {
        let tmp = TempDir::new().unwrap();
        // No .dmachunk on disk — MissingDmaChunk surfaces from A.3
        // through the state machine to the helper.
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");
        std::fs::create_dir_all(&canonical).unwrap();

        let (_, sha) = synthetic_chunk(128);
        let events = build_get_event_sequence(1, 0x1000, 128, 3, &sha);
        let input = SpuProgram::new(0x100, 1);

        let err = apply_mfc_dma_pre_replay(&events, &trace_path, &canonical, input)
            .expect_err("missing chunk must propagate to caller");
        match err {
            MfcReplayError::DmaChunkLoad(DmaChunkLoadError::MissingDmaChunk { .. }) => {}
            other => panic!("expected DmaChunkLoad(MissingDmaChunk), got {other:?}"),
        }
    }

    /// R6.7 C — end-to-end: a synthetic SPU program runs the full
    /// MFC GET sequence (wrch ch16-23 + wrch ch21 + rdch ch24 +
    /// stop) through the actual InterpreterExecutor with pre-applied
    /// DMA. Asserts the post-DMA LS contains the chunk bytes AND
    /// the SPU stopped with the captured stop_code.
    ///
    /// This is the load-bearing Phase C integration test: it exercises
    /// every layer (A.3 loader, A.4 state machine, C.1 channels, C.2
    /// dispatch, C.3 pre-replay, C.4 program plumbing).
    #[test]
    fn replay_executor_get_dma_copies_chunk_to_ls() {
        use rpcs3_spu_interpreter::encode;
        use crate::{ExecutionStopReason, InterpreterExecutor, SpuExecutor};

        let tmp = TempDir::new().unwrap();
        let (bytes, sha) = synthetic_chunk(128);
        let _ = setup_per_trace_chunk(&tmp, "capture.jsonl", &sha, &bytes);
        let trace_path = tmp.path().join("capture.jsonl");
        let canonical = tmp.path().join("canonical_dma");

        // Synthetic SPU bytecode: do `wrch chN, rN` for ch16-20 + ch22-23
        // + ch21, then `rdch r10, ch24`, then stop. We pre-load the GPRs
        // with the param values via `with_initial_gpr` so we don't need
        // immediates (which would require il/ila instructions and more
        // complex assembly).
        //
        // r1 = lsa (0x1000)         → wrch ch16, r1
        // r2 = eah (0)              → wrch ch17, r2
        // r3 = eal (0xD001_0000)    → wrch ch18, r3
        // r4 = size (128)           → wrch ch19, r4
        // r5 = tag (3)              → wrch ch20, r5
        // r6 = mask (1<<3)          → wrch ch22, r6
        // r7 = mode (2 = ALL)       → wrch ch23, r7
        // r8 = cmd (0x40)           → wrch ch21, r8
        // rdch r10, ch24
        // stop 1
        let mut code: Vec<u8> = Vec::new();
        let push = |c: &mut Vec<u8>, w: u32| c.extend_from_slice(&w.to_be_bytes());
        push(&mut code, encode::wrch(1, 16));
        push(&mut code, encode::wrch(2, 17));
        push(&mut code, encode::wrch(3, 18));
        push(&mut code, encode::wrch(4, 19));
        push(&mut code, encode::wrch(5, 20));
        push(&mut code, encode::wrch(6, 22));
        push(&mut code, encode::wrch(7, 23));
        push(&mut code, encode::wrch(8, 21));
        push(&mut code, encode::rdch(10, 24));
        push(&mut code, encode::stop(1));

        // GPR preferred slot is at the high u32 (lane 0) of the u128 —
        // shift left by 96. The interpreter reads `split_lanes(...)[0]`
        // for wrch's rt value, which corresponds to the top u32.
        let gpr = |v: u32| (v as u128) << 96;
        let lsa = 0x1000u32;
        let mask = 1u32 << 3;
        let entry_pc = 0x200u32; // arbitrary, 4-byte aligned
        let input = SpuProgram::new(entry_pc, 100)
            .with_segment(entry_pc, code)
            .with_initial_gpr(1, gpr(lsa))
            .with_initial_gpr(2, gpr(0))
            .with_initial_gpr(3, gpr(0xD001_0000))
            .with_initial_gpr(4, gpr(128))
            .with_initial_gpr(5, gpr(3))
            .with_initial_gpr(6, gpr(mask))
            .with_initial_gpr(7, gpr(2))
            .with_initial_gpr(8, gpr(0x40));

        // Pre-replay applies the GET DMA into LS at lsa=0x1000 +
        // populates the tag-stat queue with `mask`.
        let events = build_get_event_sequence(1, lsa, 128, 3, &sha);
        let plan = apply_mfc_dma_pre_replay(&events, &trace_path, &canonical, input)
            .expect("pre-replay applies DMA + collects tag-stat");

        let program = plan.program
            .with_mfc_tag_stat_queue(plan.tag_stat_queue.into_iter().collect());

        // Execute through the real InterpreterExecutor.
        let mut executor = InterpreterExecutor::default();
        let result = executor.execute(&program);

        // SPU stopped with code 1 (the stop instruction we encoded).
        match &result.stop_reason {
            ExecutionStopReason::Stop(1) => {}
            other => panic!("expected Stop(1), got {other:?} (steps={})", result.steps_executed),
        }

        // Post-DMA LS holds the chunk bytes at lsa..lsa+size.
        let ls_bytes = result.final_state.ls.as_ref();
        assert_eq!(
            &ls_bytes[lsa as usize..(lsa as usize + 128)],
            bytes.as_slice(),
            "post-execution LS must have the GET chunk bytes at lsa"
        );

        // r10 (the rdch ch24 destination) holds the tag-stat value
        // (`mask` = 1<<3). Preferred slot is lane 0 = top u32 of u128.
        let r10 = result.final_state.gpr[10];
        let r10_lane0 = (r10 >> 96) as u32;
        assert_eq!(r10_lane0, mask, "r10 must hold the popped tag-stat value");
    }
}
