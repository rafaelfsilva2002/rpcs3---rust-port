//! R5.8 A.1 + A.2 — JSONL capture parser + transformer.
//!
//! Implements the JSONL capture schema specified in
//! `docs/SPU_TRACE_CAPTURE.md`. Two halves:
//!
//! - **A.1 [`parse_jsonl_trace`]** — decodes a JSONL string into
//!   `Vec<CapturedEvent>`, validating monotonic `seq`, side/kind
//!   agreement, terminal-only `final_state`, and per-field range
//!   constraints. Uses serde under the hood, with a thin wrapper that
//!   adds line-number context to errors.
//!
//! - **A.2 [`captured_events_to_trace`]** — runs the small SPU state
//!   machine documented in the schema (`SPU_RUNNING ↔ SPU_PARKED ↔
//!   SPU_FINISHED`) and emits the corresponding [`TraceEvent`]
//!   sequence. Each captured event is mapped per the schema doc's
//!   "Mapping to R5.5 `TraceEvent`" table; raw context-only events
//!   (`spu_rdch`/`spu_wrch`/`spu_rchcnt`/`spu_wake`) do not produce
//!   `TraceEvent`s but advance the state machine where relevant.
//!
//! **Round-trip invariant:** the doc's reference example (the R5.6
//! synthetic mailbox-command-protocol trace re-encoded as 24-event
//! JSONL) parses + transforms byte-exact to
//! `mailbox_command_protocol_trace()`. The
//! `transform_round_trip_matches_canonical_r5_6_trace` test enforces
//! this — it is the load-bearing correctness check for the entire
//! capture pipeline.

use std::collections::BTreeMap;

use serde::Deserialize;

use rpcs3_spu_thread::SpuParkReason;

use crate::{SpuWakeResultKind, TraceEvent};

// =====================================================================
// Captured-event shapes
// =====================================================================

/// One captured event in the JSONL trace stream. Internally tagged on
/// the wire-level `kind` field; each variant carries a struct payload
/// that includes the common header (`seq`, `side`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CapturedEvent {
    SpuRdch(SpuRdchEvent),
    SpuWrch(SpuWrchEvent),
    SpuRchcnt(SpuRchcntEvent),
    SpuPark(SpuParkEvent),
    SpuWake(SpuWakeEvent),
    SpuStop(SpuStopEvent),
    FinalState(FinalStateEvent),
    /// R5.9e.1 schema: metadata-only event documenting the SPU's local
    /// store contents at thread-creation time. The actual bytes live in
    /// a side-file referenced by `image_sha256`; this event carries only
    /// the lookup key, byte count, load address, and entry PC. Parser
    /// validates the metadata (R5.9e.2); writer emits it (R5.9e.3, not
    /// yet); replay engine resolves the side-file (R5.9e.5, not yet).
    SpuImage(SpuImageEvent),
    /// R6.7 A.1 schema: MFC command packet at the moment of the
    /// matching `spu_wrch ch21 (MFC_Cmd)` dispatch. References the
    /// pre-DMA EA bytes via a content-addressed `.dmachunk` side-file
    /// at `<trace>.dma/<sha>.dmachunk`. Parser (R6.7 A.2) validates
    /// the metadata fields + GET-only cmd code; side-file loading is
    /// deferred to A.3; replay state machine to A.4.
    SpuMfcCmd(SpuMfcCmdEvent),
    /// R6.7 A.1 schema: tag completion observed after the matching
    /// `spu_mfc_cmd` finished its synchronous dispatch on the C++
    /// side. Must appear strictly before any `spu_rdch ch24
    /// (RdTagStat)` that observes the tag.
    MfcDmaComplete(MfcDmaCompleteEvent),
    PpuPushInmbox(PpuPushInmboxEvent),
    PpuPopOutmbox(PpuPopOutmboxEvent),
    PpuSignal(PpuSignalEvent),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapturedSide {
    Spu,
    Ppu,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapturedParkReason {
    ChannelRead,
    ChannelWrite,
}

/// R5.9a multi-SPU schema: every SPU-side event optionally carries a
/// `target_spu` field identifying the SPU thread that emitted it. When
/// absent (single-SPU traces captured under R5.7/R5.8 schema), the
/// parser treats it as `target_spu = 0`. This preserves backward
/// compatibility with all existing single-SPU fixtures (notably
/// `R5_6_REFERENCE_JSONL`) without forcing a re-capture.
///
/// PPU-side events already require `target_spu` (their semantics
/// always reference a specific target SPU), so no change there.

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpuRdchEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub pc: u32,
    pub channel: u32,
    pub value: Option<u32>,
    pub would_stall: bool,
    #[serde(default)]
    pub target_spu: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpuWrchEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub pc: u32,
    pub channel: u32,
    pub value: u32,
    pub would_stall: bool,
    #[serde(default)]
    pub target_spu: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpuRchcntEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub pc: u32,
    pub channel: u32,
    pub count: u32,
    #[serde(default)]
    pub target_spu: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpuParkEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub pc: u32,
    pub reason: CapturedParkReason,
    pub channel: u32,
    /// Optional channel snapshot at park time. When `Some`, the
    /// transformer emits an `ExpectChannelState` immediately after
    /// the `ExpectSpuPark` for this park. When `None` (the field is
    /// omitted from JSON), no channel-state assertion is emitted —
    /// matches the schema's "park-only" minimal capture mode.
    #[serde(default)]
    pub channels_at_park: Option<CapturedChannels>,
    #[serde(default)]
    pub target_spu: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpuWakeEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub pc: u32,
    #[serde(default)]
    pub target_spu: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpuStopEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub pc: u32,
    pub stop_code: u32,
    #[serde(default)]
    pub target_spu: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FinalStateEvent {
    pub seq: u64,
    pub side: CapturedSide,
    /// Subset of registers the capture chose to assert at end-of-run.
    /// Each entry is a (`reg`, lane-0 `value`) pair. Omitted registers
    /// are NOT asserted (the transformer does not infer values).
    pub gpr_lane_zero: Vec<CapturedGprEntry>,
    pub channels: CapturedChannels,
    #[serde(default)]
    pub target_spu: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PpuPushInmboxEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub target_spu: u32,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PpuPopOutmboxEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub target_spu: u32,
    pub value: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PpuSignalEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub target_spu: u32,
    pub slot: u32,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CapturedGprEntry {
    pub reg: u32,
    pub value: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CapturedChannels {
    pub in_mbox: Option<u32>,
    pub out_mbox: Option<u32>,
    pub out_intr_mbox: Option<u32>,
    pub snr1: u32,
    pub snr2: u32,
}

/// R5.9e.1 schema (parser support landed R5.9e.2): metadata for a
/// captured SPU local-store image. The bytes themselves live in a
/// side-file at `<trace>.images/<sha256>.spuimg` (or in the centralized
/// `behavior-freeze/fixtures/spu/images/<sha256>.spuimg` for committed
/// fixtures). The parser validates this struct's fields; it does NOT
/// load the side-file. The replay engine (R5.9e.5, not yet implemented)
/// is responsible for resolving + hash-verifying the side-file. See
/// `docs/SPU_TRACE_CAPTURE.md` § "R5.9e.1 — SPU image metadata + side-file
/// layout" for the full schema.
///
/// `target_spu` is mandatory (no `Option` / no default-zero shim) — a
/// `spu_image` event without an explicit SPU identity makes no sense.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpuImageEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub target_spu: u32,
    pub image_sha256: String,
    pub load_addr: u32,
    pub size: u32,
    pub entry_pc: u32,
}

/// R6.7 A.1 schema: MFC command packet at `wrch ch21` dispatch. Carries
/// the cmd code + tag + transfer parameters and a content-addressed
/// reference to the pre-DMA EA bytes. The bytes themselves live in a
/// side-file at `<trace>.dma/<sha>.dmachunk` (or canonical CC0 store
/// at `behavior-freeze/fixtures/spu/dma/<sha>.dmachunk`); the A.2
/// parser does NOT load the side-file (deferred to A.3).
///
/// `target_spu` is mandatory: an MFC command without a specific SPU
/// thread context makes no sense.
///
/// **R6.7 scope (parser-level):** only cmd code 0x40 (GET, EA → LS)
/// is accepted; any other value is rejected with `UnsupportedMfcCmd`
/// per `docs/SPU_DMA_MFC_R6_7_DESIGN.md` § 3 / § 4.3. PUT, list, and
/// atomic primitives are out of scope and surface as parse errors.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SpuMfcCmdEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub target_spu: u32,
    pub pc: u32,
    pub cmd: u32,
    pub tag: u32,
    pub size: u32,
    pub lsa: u32,
    pub eah: u32,
    pub eal: u32,
    pub ea_chunk_sha256: String,
    // R8.4b — additive list-DMA fields. Present when cmd is a
    // list-DMA code (GETL = 0x44 in R8.4b scope; GETLB/GETLF/
    // PUTL/family in later phases). Absent for simple GET/PUT
    // (cmd 0x40 / 0x20) — `serde` deserializes missing fields
    // as `None` via `#[serde(default)]`, so existing R6.7/R8.1
    // traces parse byte-identical without any schema bump.
    //
    // When all four list fields are present, the writer captured
    // a list-DMA dispatch:
    // - `descriptor_sha256`: SHA-256 of the N-element descriptor
    //   array (matches `ea_chunk_sha256` slot value by writer
    //   convention so the simple-cmd parser still sees a valid
    //   SHA string in the legacy slot).
    // - `descriptor_size`: total descriptor bytes (= N * 8).
    // - `element_chunks`: per-element source-byte SHAs (N entries).
    //   Each SHA points to a `.dmachunk` in the existing pool.
    // - `element_sizes`: per-element transfer size `ts` (N entries).
    // - `element_eals`: per-element source EA (N entries).
    //
    // R8.4c will lift the parser canary and consume these via a
    // new `MfcReplayState::process_mfc_list_cmd` path. R8.4b
    // parses them but the replay/transform layers still reject
    // list cmds with `UnsupportedMfcListCmd`.
    #[serde(default)]
    pub descriptor_sha256: Option<String>,
    #[serde(default)]
    pub descriptor_size: Option<u32>,
    #[serde(default)]
    pub element_chunks: Option<Vec<String>>,
    #[serde(default)]
    pub element_sizes: Option<Vec<u32>>,
    #[serde(default)]
    pub element_eals: Option<Vec<u32>>,
}

/// R6.7 A.1 schema: tag-completion notification emitted after the C++
/// `process_mfc_cmd()` returned. Must appear strictly between the
/// matching `spu_mfc_cmd` and any `spu_rdch ch24` that observes the
/// tag completed. The `transferred_bytes` field is informational; for
/// plain GET it equals the dispatch's `size`. Future scope (partial
/// transfers, multi-tag fan-in) may decouple them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct MfcDmaCompleteEvent {
    pub seq: u64,
    pub side: CapturedSide,
    pub target_spu: u32,
    pub tag: u32,
    pub transferred_bytes: u32,
}

impl CapturedEvent {
    /// Header `seq` field, regardless of variant.
    #[must_use]
    pub fn seq(&self) -> u64 {
        match self {
            Self::SpuRdch(e) => e.seq,
            Self::SpuWrch(e) => e.seq,
            Self::SpuRchcnt(e) => e.seq,
            Self::SpuPark(e) => e.seq,
            Self::SpuWake(e) => e.seq,
            Self::SpuStop(e) => e.seq,
            Self::FinalState(e) => e.seq,
            Self::SpuImage(e) => e.seq,
            Self::SpuMfcCmd(e) => e.seq,
            Self::MfcDmaComplete(e) => e.seq,
            Self::PpuPushInmbox(e) => e.seq,
            Self::PpuPopOutmbox(e) => e.seq,
            Self::PpuSignal(e) => e.seq,
        }
    }

    /// Header `side` field, regardless of variant.
    #[must_use]
    pub fn side(&self) -> CapturedSide {
        match self {
            Self::SpuRdch(e) => e.side,
            Self::SpuWrch(e) => e.side,
            Self::SpuRchcnt(e) => e.side,
            Self::SpuPark(e) => e.side,
            Self::SpuWake(e) => e.side,
            Self::SpuStop(e) => e.side,
            Self::FinalState(e) => e.side,
            Self::SpuImage(e) => e.side,
            Self::SpuMfcCmd(e) => e.side,
            Self::MfcDmaComplete(e) => e.side,
            Self::PpuPushInmbox(e) => e.side,
            Self::PpuPopOutmbox(e) => e.side,
            Self::PpuSignal(e) => e.side,
        }
    }

    /// R5.9a multi-SPU: returns the `target_spu` for any event. PPU-side
    /// events always carry the field; SPU-side events carry it under
    /// R5.9+ writers but legacy R5.7/R5.8 single-SPU traces omit it,
    /// in which case this method returns 0 (single-SPU compatibility
    /// shim documented in `docs/SPU_TRACE_R5_9_MULTISPU_PLAN.md` § A.4).
    /// Use this accessor everywhere — never read `target_spu` directly
    /// off a variant struct, since the SPU-side `Option<u32>` requires
    /// the unwrap_or(0) collapse for legacy traces.
    #[must_use]
    pub fn target_spu(&self) -> u32 {
        match self {
            Self::SpuRdch(e) => e.target_spu.unwrap_or(0),
            Self::SpuWrch(e) => e.target_spu.unwrap_or(0),
            Self::SpuRchcnt(e) => e.target_spu.unwrap_or(0),
            Self::SpuPark(e) => e.target_spu.unwrap_or(0),
            Self::SpuWake(e) => e.target_spu.unwrap_or(0),
            Self::SpuStop(e) => e.target_spu.unwrap_or(0),
            Self::FinalState(e) => e.target_spu.unwrap_or(0),
            // R5.9e.2: spu_image's target_spu is mandatory (no Option, no
            // default-zero shim) — the schema requires it for the side-file
            // lookup to make sense.
            Self::SpuImage(e) => e.target_spu,
            // R6.7 A.1: target_spu mandatory on MFC events (the cmd
            // and the completion are SPU-thread-specific; no default-
            // zero shim).
            Self::SpuMfcCmd(e) => e.target_spu,
            Self::MfcDmaComplete(e) => e.target_spu,
            Self::PpuPushInmbox(e) => e.target_spu,
            Self::PpuPopOutmbox(e) => e.target_spu,
            Self::PpuSignal(e) => e.target_spu,
        }
    }

    /// Side that the variant's `kind` requires. Used by validation.
    fn required_side(&self) -> CapturedSide {
        match self {
            Self::SpuRdch(_)
            | Self::SpuWrch(_)
            | Self::SpuRchcnt(_)
            | Self::SpuPark(_)
            | Self::SpuWake(_)
            | Self::SpuStop(_)
            | Self::FinalState(_)
            | Self::SpuImage(_)
            | Self::SpuMfcCmd(_)
            | Self::MfcDmaComplete(_) => CapturedSide::Spu,
            Self::PpuPushInmbox(_) | Self::PpuPopOutmbox(_) | Self::PpuSignal(_) => {
                CapturedSide::Ppu
            }
        }
    }

    fn kind_label(&self) -> &'static str {
        match self {
            Self::SpuRdch(_) => "spu_rdch",
            Self::SpuWrch(_) => "spu_wrch",
            Self::SpuRchcnt(_) => "spu_rchcnt",
            Self::SpuPark(_) => "spu_park",
            Self::SpuWake(_) => "spu_wake",
            Self::SpuStop(_) => "spu_stop",
            Self::FinalState(_) => "final_state",
            Self::SpuImage(_) => "spu_image",
            Self::SpuMfcCmd(_) => "spu_mfc_cmd",
            Self::MfcDmaComplete(_) => "mfc_dma_complete",
            Self::PpuPushInmbox(_) => "ppu_push_inmbox",
            Self::PpuPopOutmbox(_) => "ppu_pop_outmbox",
            Self::PpuSignal(_) => "ppu_signal",
        }
    }

    /// R5.9e.2: returns true for SPU-side events that represent
    /// EXECUTED behavior on the SPU (channel ops, parks, wakes, stops,
    /// final_state). Returns false for `spu_image` (metadata-only),
    /// PPU-side events (executed on the PPU side), and any future
    /// metadata-only event kinds. Used by the per-SPU walk to enforce
    /// "spu_image must precede the SPU's first executed event".
    ///
    /// **R6.7 A.2:** `spu_mfc_cmd` and `mfc_dma_complete` are
    /// EXECUTED — the cmd dispatch happens at the SPU's PC, and the
    /// completion is the C++ executor's observable signal that the
    /// pre-DMA EA bytes have landed in LS. Counting them as executed
    /// keeps the `spu_image must precede first executed event` rule
    /// valid for traces that contain DMA.
    fn is_spu_executed(&self) -> bool {
        matches!(
            self,
            Self::SpuRdch(_)
                | Self::SpuWrch(_)
                | Self::SpuRchcnt(_)
                | Self::SpuPark(_)
                | Self::SpuWake(_)
                | Self::SpuStop(_)
                | Self::FinalState(_)
                | Self::SpuMfcCmd(_)
                | Self::MfcDmaComplete(_)
        )
    }
}

// =====================================================================
// Parser
// =====================================================================

/// Errors from [`parse_jsonl_trace`]. All variants carry the 1-based
/// line number so a malformed trace points at the offending line.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceParseError {
    /// `serde_json` failed to decode the line as a `CapturedEvent`.
    Json { line: usize, message: String },
    /// Two consecutive events have the same or decreasing `seq`.
    NonMonotonicSeq {
        line: usize,
        prev_seq: u64,
        got_seq: u64,
    },
    /// `side` field disagrees with the variant's expected side
    /// (e.g. a `spu_rdch` event with `"side": "ppu"`).
    SideKindMismatch {
        line: usize,
        seq: u64,
        kind: &'static str,
        got_side: CapturedSide,
        expected_side: CapturedSide,
    },
    /// SPU `pc` is not 4-byte aligned or out of LS range.
    BadPc {
        line: usize,
        seq: u64,
        pc: u32,
        reason: &'static str,
    },
    /// SPU channel id is out of the 7-bit range (0..128).
    BadChannel {
        line: usize,
        seq: u64,
        channel: u32,
    },
    /// Stop code exceeds the 14-bit range.
    BadStopCode {
        line: usize,
        seq: u64,
        stop_code: u32,
    },
    /// Signal slot is not 0 or 1.
    BadSignalSlot {
        line: usize,
        seq: u64,
        slot: u32,
    },
    /// `final_state.gpr_lane_zero[i].reg` >= 128.
    BadGprReg {
        line: usize,
        seq: u64,
        reg: u32,
    },
    /// A `final_state` event was followed by another event.
    ///
    /// **R5.9a deprecation note:** the parser no longer emits this
    /// variant. The single-SPU "final_state must be the last event"
    /// rule was generalized in R5.9a to "for each `target_spu`,
    /// final_state must precede no other event for that SPU". Single-
    /// SPU traces (where everything has `target_spu = 0`) reduce to
    /// the original constraint as a degenerate case. Failures that
    /// would have been `FinalStateNotTerminal` are now reported as
    /// `EventAfterFinalState` (event for an already-finalized SPU)
    /// or `DuplicateFinalState` (a second `final_state` for the same
    /// SPU). The variant is kept in the enum for downstream callers
    /// that pattern-match exhaustively, but new code paths should not
    /// produce it.
    FinalStateNotTerminal {
        final_state_index: usize,
        last_index: usize,
    },
    /// R5.9a — an event for an already-finalized SPU was observed.
    /// Replaces the single-SPU `FinalStateNotTerminal`. After a SPU
    /// emits `final_state`, no further event for that same SPU is
    /// allowed. Events for OTHER SPUs are perfectly fine — the rule
    /// is per-`target_spu`, not global.
    EventAfterFinalState {
        target_spu: u32,
        event_index: usize,
        final_state_index: usize,
    },
    /// R5.9a — a SPU emitted `final_state` more than once. Each SPU
    /// thread terminates exactly once.
    DuplicateFinalState {
        target_spu: u32,
        first_index: usize,
        second_index: usize,
    },
    /// R5.9e.2 — a SPU emitted more than one `spu_image` event.
    /// The schema permits exactly one image per SPU per trace; multiple
    /// snapshots (e.g., to handle code overlays) would require a
    /// different schema and are deferred to a future R5.9f or beyond.
    DuplicateSpuImage {
        target_spu: u32,
        first_index: usize,
        second_index: usize,
    },
    /// R5.9e.2 — a `spu_image` event for SPU X appeared after at least
    /// one executed SPU-side event for the same X. The image must be
    /// captured at thread-creation time, BEFORE any rdch/wrch/etc.
    /// PPU-side events for X may precede the image (the PPU can act on
    /// the SPU before the SPU starts running).
    ImageEventOutOfOrder {
        target_spu: u32,
        image_index: usize,
        first_event_index: usize,
    },
    /// R5.9e.2 — `spu_image.image_sha256` is not a 64-char lowercase
    /// hex string. The hash is the lookup key for the side-file; an
    /// invalid hash makes resolution impossible at replay time.
    BadImageHash {
        line: usize,
        seq: u64,
        target_spu: u32,
        reason: &'static str,
    },
    /// R5.9e.2 — `spu_image.size` is out of range. Must be in
    /// `4..=262144` (4 byte minimum to hold one instruction; 256 KB
    /// maximum = full SPU local store) and a multiple of 4.
    BadImageSize {
        line: usize,
        seq: u64,
        target_spu: u32,
        size: u32,
        reason: &'static str,
    },
    /// R5.9e.2 — `spu_image.load_addr` is out of range. Must be a
    /// multiple of 4 and `load_addr + size` must not overflow `u32` or
    /// exceed the 256 KB local store.
    BadImageLoadAddr {
        line: usize,
        seq: u64,
        target_spu: u32,
        load_addr: u32,
        reason: &'static str,
    },
    /// R5.9e.2 — `spu_image.entry_pc` is out of range. Must be a
    /// multiple of 4 and within the local store (`< 0x40000`).
    BadImageEntryPc {
        line: usize,
        seq: u64,
        target_spu: u32,
        entry_pc: u32,
        reason: &'static str,
    },
    /// R5.9e.2 — the trace contains a `spu_wrch` to `MFC_Cmd`
    /// (channel 21), which dispatches a DMA. R5.9e does not capture
    /// DMA endpoints (LS↔main-memory transfers) and therefore cannot
    /// replay a trace that depends on them. Document-level rationale
    /// in `docs/SPU_TRACE_R5_9E_REPLAY_PLAN.md` § D.1; the v1 strategy
    /// is to reject at parse time so the failure mode is sharp.
    /// SMC (self-modifying code via DMA-to-own-LS) is a strict subset
    /// of this case and is also surfaced here — there is no separate
    /// `UnsupportedSelfModifyingCode` variant in R5.9e.2 because the
    /// single-channel signature for SMC alone is not reliably
    /// distinguishable from generic DMA without observing the full
    /// `MFC_LSA`/`MFC_EAH`/...//`MFC_Cmd` register sequence; deferring
    /// finer detection to a future iteration when the writer surfaces
    /// per-DMA side-channel events.
    UnsupportedDmaInTrace {
        target_spu: u32,
        event_index: usize,
        channel: u32,
    },
    /// R6.7 A.2 — `spu_wrch ch21 (MFC_Cmd)` was observed but is NOT
    /// followed immediately by a matching `spu_mfc_cmd` event for the
    /// same `target_spu`. The R6.7 schema requires the additive
    /// metadata; without it the trace can't be classified (replay or
    /// reject) and the parser surfaces the structural break here.
    /// `wrch_event_index` points at the offending wrch.
    MalformedMfcSequence {
        target_spu: u32,
        wrch_event_index: usize,
        reason: &'static str,
    },
    /// R6.7 A.2 + R8.1 — `spu_mfc_cmd.cmd` is not in the
    /// supported subset. Currently accepted: `0x40` GET (R6.7 A.2)
    /// and `0x20` PUT (R8.1). PUT-list / GET-list variants surface
    /// as the more specific [`Self::UnsupportedMfcListCmd`]
    /// (R8.4a). Atomic / barrier / fence-flagged variants and
    /// any other code land here.
    UnsupportedMfcCmd {
        line: usize,
        seq: u64,
        target_spu: u32,
        cmd: u32,
    },
    /// R8.4a — `spu_mfc_cmd.cmd` is a recognized MFC list-DMA
    /// command (PUTL/PUTLB/PUTLF = 0x24-0x26, GETL/GETLB/GETLF
    /// = 0x44-0x46) but list-DMA is not yet supported by the
    /// replay/runtime stack. Granular variant (vs the generic
    /// [`Self::UnsupportedMfcCmd`]) so downstream tooling can
    /// distinguish "out of scope for now" from "never seen
    /// before". R8.4b/c/d will progressively implement the
    /// GETL subset of these codes (PUTL list-form deferred).
    /// See `docs/SPU_DMA_MFC_R6_7_DESIGN.md` § 19.
    UnsupportedMfcListCmd {
        line: usize,
        seq: u64,
        target_spu: u32,
        cmd: u32,
    },
    /// R6.7 A.2 — `spu_mfc_cmd.eah` is non-zero. PS3 user-space PPU is
    /// 32-bit; PSL1GHT homebrew always writes 0. Real games that use
    /// 64-bit lv2 kernel-space addressing are out of R6.7 scope. See
    /// design § 9.2.
    UnsupportedMfcEah {
        line: usize,
        seq: u64,
        target_spu: u32,
        eah: u32,
    },
    /// R6.7 A.2 — `spu_mfc_cmd.size` is out of range or has an
    /// unsupported alignment per design § 3 / § 5.5:
    /// allowed values are 1, 2, 4, 8 OR a multiple of 16 in
    /// `[16, 16384]`. Anything else surfaces as `BadDmaSize`.
    BadDmaSize {
        line: usize,
        seq: u64,
        target_spu: u32,
        size: u32,
        reason: &'static str,
    },
    /// R6.7 A.2 — `spu_mfc_cmd.lsa` is misaligned for the dispatched
    /// `size`, or `lsa + size` exceeds the 256 KiB local store. Per
    /// design § 3, sizes ≥ 16 require 16-byte alignment; the small
    /// sizes (1, 2, 4, 8) require natural alignment.
    BadDmaLsa {
        line: usize,
        seq: u64,
        target_spu: u32,
        lsa: u32,
        reason: &'static str,
    },
    /// R6.7 A.2 — `spu_mfc_cmd.ea_chunk_sha256` is not a 64-char
    /// lowercase-hex string. The hash is the side-file lookup key;
    /// an invalid hash makes resolution impossible at replay time.
    /// Mirrors the `BadImageHash` rule for `.spuimg` references.
    BadDmaSha {
        line: usize,
        seq: u64,
        target_spu: u32,
        reason: &'static str,
    },
    /// R6.7 A.2 — `spu_mfc_cmd.tag` or `mfc_dma_complete.tag` is out
    /// of the 5-bit range (`0..32`). Tags are stored in `ch_mfc_cmd.tag
    /// & 0x1f` on the SPU; values >= 32 are an MFC ABI violation.
    BadMfcTag {
        line: usize,
        seq: u64,
        target_spu: u32,
        tag: u32,
    },
    /// R6.7 A.2 — `mfc_dma_complete.transferred_bytes` is out of range.
    /// Allowed: `1..=16384` (matching the same bounds as `size`). The
    /// per-trace per-tag invariant (`transferred_bytes` matches the
    /// dispatched `size`) is checked at the transformer / replay layer
    /// (A.4); the parser only validates the standalone bounds.
    BadDmaTransferredBytes {
        line: usize,
        seq: u64,
        target_spu: u32,
        transferred_bytes: u32,
        reason: &'static str,
    },
}

impl std::fmt::Display for TraceParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json { line, message } => {
                write!(f, "trace parse error at line {line}: invalid JSON: {message}")
            }
            Self::NonMonotonicSeq { line, prev_seq, got_seq } => {
                write!(f, "trace parse error at line {line}: non-monotonic seq (prev={prev_seq}, got={got_seq})")
            }
            Self::SideKindMismatch { line, seq, kind, got_side, expected_side } => {
                write!(f, "trace parse error at line {line} (seq {seq}, kind {kind}): side {got_side:?} disagrees with expected {expected_side:?}")
            }
            Self::BadPc { line, seq, pc, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}): bad pc 0x{pc:x} ({reason})")
            }
            Self::BadChannel { line, seq, channel } => {
                write!(f, "trace parse error at line {line} (seq {seq}): channel {channel} out of 7-bit range")
            }
            Self::BadStopCode { line, seq, stop_code } => {
                write!(f, "trace parse error at line {line} (seq {seq}): stop_code 0x{stop_code:x} exceeds 14-bit range")
            }
            Self::BadSignalSlot { line, seq, slot } => {
                write!(f, "trace parse error at line {line} (seq {seq}): signal slot {slot} must be 0 or 1")
            }
            Self::BadGprReg { line, seq, reg } => {
                write!(f, "trace parse error at line {line} (seq {seq}): gpr reg {reg} >= 128")
            }
            Self::FinalStateNotTerminal { final_state_index, last_index } => {
                write!(f, "trace parse error: final_state at event index {final_state_index} but last event is index {last_index} (final_state must be terminal — single-SPU rule, deprecated by R5.9a)")
            }
            Self::EventAfterFinalState { target_spu, event_index, final_state_index } => {
                write!(f, "trace parse error: event at index {event_index} references target_spu {target_spu} but that SPU was already finalized at event index {final_state_index}")
            }
            Self::DuplicateFinalState { target_spu, first_index, second_index } => {
                write!(f, "trace parse error: duplicate final_state for target_spu {target_spu} at event index {second_index} (first final_state at index {first_index})")
            }
            Self::DuplicateSpuImage { target_spu, first_index, second_index } => {
                write!(f, "trace parse error: duplicate spu_image for target_spu {target_spu} at event index {second_index} (first spu_image at index {first_index}); R5.9e schema permits exactly one image per SPU per trace")
            }
            Self::ImageEventOutOfOrder { target_spu, image_index, first_event_index } => {
                write!(f, "trace parse error: spu_image at event index {image_index} for target_spu {target_spu} appears AFTER its first executed SPU event at index {first_event_index}; the image must precede all executed events for that SPU")
            }
            Self::BadImageHash { line, seq, target_spu, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad image_sha256 ({reason})")
            }
            Self::BadImageSize { line, seq, target_spu, size, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad image size {size} ({reason})")
            }
            Self::BadImageLoadAddr { line, seq, target_spu, load_addr, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad image load_addr 0x{load_addr:x} ({reason})")
            }
            Self::BadImageEntryPc { line, seq, target_spu, entry_pc, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad image entry_pc 0x{entry_pc:x} ({reason})")
            }
            Self::UnsupportedDmaInTrace { target_spu, event_index, channel } => {
                write!(f, "trace parse error: unsupported DMA at event index {event_index} for target_spu {target_spu} (spu_wrch to channel {channel} = MFC_Cmd dispatches DMA; R5.9e does not capture DMA endpoints — see docs/SPU_TRACE_R5_9E_REPLAY_PLAN.md § D.1)")
            }
            Self::MalformedMfcSequence { target_spu, wrch_event_index, reason } => {
                write!(f, "trace parse error: malformed MFC sequence at event index {wrch_event_index} for target_spu {target_spu} ({reason})")
            }
            Self::UnsupportedMfcCmd { line, seq, target_spu, cmd } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): unsupported MFC cmd code 0x{cmd:x} (parser accepts 0x40 GET and 0x20 PUT; list/atomic/barrier-flagged variants surface as more specific errors when known)")
            }
            Self::UnsupportedMfcListCmd { line, seq, target_spu, cmd } => {
                let mnemonic = match *cmd {
                    0x24 => "PUTL",
                    0x25 => "PUTLB",
                    0x26 => "PUTLF",
                    0x44 => "GETL",
                    0x45 => "GETLB",
                    0x46 => "GETLF",
                    _ => "list",
                };
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): MFC list-DMA cmd 0x{cmd:x} ({mnemonic}) not yet supported (R8.4a parser canary; R8.4b/c/d will implement GETL replay/runtime; PUTL deferred to R8.5+); see docs/SPU_DMA_MFC_R6_7_DESIGN.md § 19")
            }
            Self::UnsupportedMfcEah { line, seq, target_spu, eah } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): MFC eah 0x{eah:x} != 0 (R6.7 A.2 PS3 user-space PSL1GHT scope only — eah must be 0)")
            }
            Self::BadDmaSize { line, seq, target_spu, size, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad MFC size {size} ({reason})")
            }
            Self::BadDmaLsa { line, seq, target_spu, lsa, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad MFC lsa 0x{lsa:x} ({reason})")
            }
            Self::BadDmaSha { line, seq, target_spu, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad ea_chunk_sha256 ({reason})")
            }
            Self::BadMfcTag { line, seq, target_spu, tag } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad MFC tag {tag} (must be in 0..32)")
            }
            Self::BadDmaTransferredBytes { line, seq, target_spu, transferred_bytes, reason } => {
                write!(f, "trace parse error at line {line} (seq {seq}, target_spu {target_spu}): bad mfc_dma_complete transferred_bytes {transferred_bytes} ({reason})")
            }
        }
    }
}

impl std::error::Error for TraceParseError {}

/// Decode a JSONL trace string into `Vec<CapturedEvent>`. Empty lines
/// and lines starting with `#` are skipped. Validates seq monotonicity,
/// side/kind agreement, and per-field range constraints. Validates
/// terminal-`final_state` constraint after the full pass.
///
/// Returns an event vector preserving input order. Use
/// [`captured_events_to_trace`] to convert into the R5.5 `TraceEvent`
/// shape consumed by `replay_trace`.
pub fn parse_jsonl_trace(input: &str) -> Result<Vec<CapturedEvent>, TraceParseError> {
    let mut events = Vec::new();
    let mut last_seq: Option<u64> = None;

    for (idx, raw) in input.lines().enumerate() {
        let line_no = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let event: CapturedEvent =
            serde_json::from_str(line).map_err(|e| TraceParseError::Json {
                line: line_no,
                message: e.to_string(),
            })?;
        validate_event(&event, line_no, last_seq)?;
        last_seq = Some(event.seq());
        events.push(event);
    }

    // R5.9a multi-SPU per-target_spu finalization walk.
    //
    // The pre-R5.9 single-SPU rule was "exactly one final_state, must be
    // the last event in the trace". That rule generalizes cleanly to
    // "for each target_spu, final_state precedes no other event for the
    // same target_spu". For single-SPU traces (everything implicit
    // target_spu=0), the new rule reduces to the old as a degenerate
    // case — only one SPU exists, so its final_state being non-terminal
    // is detected by the same per-SPU pass.
    //
    // Two distinct failure modes are surfaced:
    //   - `EventAfterFinalState` — any event for SPU X after X's
    //     final_state. This includes a second final_state for X
    //     ONLY if it would also count as "after"; in practice the
    //     branch below catches duplicates first via the explicit
    //     FinalState match arm.
    //   - `DuplicateFinalState` — a second final_state for the same
    //     SPU X (each SPU terminates exactly once).
    //
    // The walk is HashMap<target_spu, final_state_event_index>; a SPU
    // is "finalized" iff present in the map. Map size is bounded by
    // the number of distinct SPUs in the trace (typically 1–8).
    let mut finalized: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    // R5.9e.2: track per-SPU `spu_image` event index and per-SPU first
    // executed event index to enforce uniqueness + ordering.
    let mut image_seen: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    let mut first_executed: std::collections::HashMap<u32, usize> =
        std::collections::HashMap::new();
    for (idx, ev) in events.iter().enumerate() {
        let tgt = ev.target_spu();

        // R5.9e.2 → R6.7 A.2 DMA detection.
        //
        // Pre-R6.7: any `spu_wrch ch21 (MFC_Cmd)` was an immediate
        // parse-time reject because the writer didn't yet emit the
        // additive metadata.
        //
        // R6.7 A.2: the writer (R6.7 A.1) emits `spu_mfc_cmd` IMMEDIATELY
        // after `spu_wrch ch21` for the same `target_spu`. The parser
        // accepts this pair and validates the metadata; the transformer
        // (still no-DMA-replay until A.4) rejects the resulting
        // `SpuMfcCmd` event with `TraceTransformError::UnsupportedDmaInTrace`.
        //
        // A bare `spu_wrch ch21` with NO matching `spu_mfc_cmd` follow-up
        // surfaces as `UnsupportedDmaInTrace` with the same shape as
        // before — backward-compatible with R5.9e.2 callers + tests.
        if let CapturedEvent::SpuWrch(w) = ev {
            if w.channel == MFC_CMD_CHANNEL {
                let next = events.get(idx + 1);
                let next_is_matching_mfc_cmd = matches!(
                    next,
                    Some(CapturedEvent::SpuMfcCmd(m)) if m.target_spu == tgt
                );
                if !next_is_matching_mfc_cmd {
                    return Err(TraceParseError::UnsupportedDmaInTrace {
                        target_spu: tgt,
                        event_index: idx,
                        channel: w.channel,
                    });
                }
            }
        }

        // R6.7 A.2: a `spu_mfc_cmd` event MUST be preceded by `spu_wrch
        // ch21` for the same `target_spu` at exactly events[idx-1].
        // This is the writer's emit ordering (cmd packet is finalized
        // by ch21 wrch; the additive metadata fires on the same lock).
        // Drift = malformed trace.
        if let CapturedEvent::SpuMfcCmd(_) = ev {
            let prev = if idx == 0 { None } else { events.get(idx - 1) };
            let prev_is_matching_ch21_wrch = matches!(
                prev,
                Some(CapturedEvent::SpuWrch(w))
                    if w.channel == MFC_CMD_CHANNEL && w.target_spu.unwrap_or(0) == tgt
            );
            if !prev_is_matching_ch21_wrch {
                return Err(TraceParseError::MalformedMfcSequence {
                    target_spu: tgt,
                    wrch_event_index: idx,
                    reason: "spu_mfc_cmd must be preceded by matching spu_wrch ch21 for the same target_spu",
                });
            }
        }

        if let Some(&prior_final_idx) = finalized.get(&tgt) {
            return Err(if matches!(ev, CapturedEvent::FinalState(_)) {
                TraceParseError::DuplicateFinalState {
                    target_spu: tgt,
                    first_index: prior_final_idx,
                    second_index: idx,
                }
            } else {
                TraceParseError::EventAfterFinalState {
                    target_spu: tgt,
                    event_index: idx,
                    final_state_index: prior_final_idx,
                }
            });
        }

        // R5.9e.2 spu_image uniqueness + ordering. The image must
        // appear at most once per target_spu, and BEFORE the SPU's
        // first executed event. PPU-side events for the same target
        // do NOT count as "executed" — the PPU may push to the SPU's
        // mailbox before the SPU starts running.
        if matches!(ev, CapturedEvent::SpuImage(_)) {
            if let Some(&first_img_idx) = image_seen.get(&tgt) {
                return Err(TraceParseError::DuplicateSpuImage {
                    target_spu: tgt,
                    first_index: first_img_idx,
                    second_index: idx,
                });
            }
            if let Some(&first_exec_idx) = first_executed.get(&tgt) {
                return Err(TraceParseError::ImageEventOutOfOrder {
                    target_spu: tgt,
                    image_index: idx,
                    first_event_index: first_exec_idx,
                });
            }
            image_seen.insert(tgt, idx);
        } else if ev.is_spu_executed() {
            first_executed.entry(tgt).or_insert(idx);
        }

        if matches!(ev, CapturedEvent::FinalState(_)) {
            finalized.insert(tgt, idx);
        }
    }

    Ok(events)
}

const SPU_LS_BYTES: u32 = 0x40000;
const CHANNEL_MAX: u32 = 127;
const STOP_CODE_MAX: u32 = 0x3FFF;

/// MFC command channel — `spu_wrch` to this channel dispatches a DMA.
/// Pre-R6.7 traces (no `spu_mfc_cmd` follow-up) are still rejected at
/// parse time with `UnsupportedDmaInTrace`. R6.7 A.2 traces that
/// include the additive `spu_mfc_cmd` event immediately after the
/// `wrch ch21` are accepted at parse time and validated; the
/// transformer rejects them with `TraceTransformError::UnsupportedDmaInTrace`
/// until the replay state machine (A.4) lands.
const MFC_CMD_CHANNEL: u32 = 21;

/// R6.7 A.2 — MFC cmd codes recognized by the parser. Initial scope:
/// only GET (0x40, EA → LS). Other codes (PUT, list, atomic, barrier-
/// flagged variants) surface as `UnsupportedMfcCmd`. See design § 3 /
/// § 4.3.
const MFC_GET_CMD: u32 = 0x40;
/// R8.1 — MFC PUT (LS → EA writes). Same validation surface as GET.
/// The `.dmachunk` side-file content semantics differ: for GET it
/// carries the EA bytes the SPU received; for PUT it carries the LS
/// bytes the SPU produced at dispatch time (verified during replay
/// by asserting `LS[lsa..lsa+size] == captured_chunk` at the moment
/// the interpreter reaches `wrch ch21`).
const MFC_PUT_CMD: u32 = 0x20;

/// R8.4a — MFC list-DMA command codes (full set). Codes not yet
/// implemented surface the granular [`TraceParseError::UnsupportedMfcListCmd`]
/// (vs the generic [`TraceParseError::UnsupportedMfcCmd`]).
///
/// Codes per `rpcs3-upstream-clean/rpcs3/Emu/Cell/MFC.h:7-15`:
///
/// | Code | Mnemonic | Direction | Modifiers          | Phase             |
/// |------|----------|-----------|--------------------|-------------------|
/// | 0x24 | PUTL     | LS → EA   | list               | R8.4e+ deferred   |
/// | 0x25 | PUTLB    | LS → EA   | list + barrier     | R8.4e+ deferred   |
/// | 0x26 | PUTLF    | LS → EA   | list + fence       | R8.4e+ deferred   |
/// | 0x44 | GETL     | EA → LS   | list               | **R8.4c accepts** |
/// | 0x45 | GETLB    | EA → LS   | list + barrier     | R8.4f deferred    |
/// | 0x46 | GETLF    | EA → LS   | list + fence       | R8.4f deferred    |
///
/// R8.4c lifts the canary for 0x44 GETL only. The other five codes
/// continue to reject with `UnsupportedMfcListCmd`.
const MFC_GETL_CMD: u32 = 0x44;
const MFC_LIST_CMDS_UNSUPPORTED: &[u32] = &[0x24, 0x25, 0x26, 0x45, 0x46];

fn is_mfc_list_cmd_unsupported(cmd: u32) -> bool {
    MFC_LIST_CMDS_UNSUPPORTED.contains(&cmd)
}

/// R6.7 A.2 — MFC tag is a 5-bit field (`ch_mfc_cmd.tag & 0x1f`).
const MFC_TAG_MAX: u32 = 31;

/// R6.7 A.2 — MFC transfer size hard cap (16 KiB). Larger sizes need
/// list DMA which is out of R6.7 scope.
const MFC_DMA_SIZE_MAX: u32 = 0x4000;

fn validate_event(
    event: &CapturedEvent,
    line: usize,
    last_seq: Option<u64>,
) -> Result<(), TraceParseError> {
    if let Some(prev_seq) = last_seq {
        if event.seq() <= prev_seq {
            return Err(TraceParseError::NonMonotonicSeq {
                line,
                prev_seq,
                got_seq: event.seq(),
            });
        }
    }

    if event.side() != event.required_side() {
        return Err(TraceParseError::SideKindMismatch {
            line,
            seq: event.seq(),
            kind: event.kind_label(),
            got_side: event.side(),
            expected_side: event.required_side(),
        });
    }

    match event {
        CapturedEvent::SpuRdch(e) => {
            check_pc(line, e.seq, e.pc)?;
            check_channel(line, e.seq, e.channel)?;
        }
        CapturedEvent::SpuWrch(e) => {
            check_pc(line, e.seq, e.pc)?;
            check_channel(line, e.seq, e.channel)?;
        }
        CapturedEvent::SpuRchcnt(e) => {
            check_pc(line, e.seq, e.pc)?;
            check_channel(line, e.seq, e.channel)?;
        }
        CapturedEvent::SpuPark(e) => {
            check_pc(line, e.seq, e.pc)?;
            check_channel(line, e.seq, e.channel)?;
        }
        CapturedEvent::SpuWake(e) => check_pc(line, e.seq, e.pc)?,
        CapturedEvent::SpuStop(e) => {
            check_pc(line, e.seq, e.pc)?;
            if e.stop_code > STOP_CODE_MAX {
                return Err(TraceParseError::BadStopCode {
                    line,
                    seq: e.seq,
                    stop_code: e.stop_code,
                });
            }
        }
        CapturedEvent::FinalState(e) => {
            for entry in &e.gpr_lane_zero {
                if entry.reg >= 128 {
                    return Err(TraceParseError::BadGprReg {
                        line,
                        seq: e.seq,
                        reg: entry.reg,
                    });
                }
            }
        }
        CapturedEvent::SpuImage(e) => validate_spu_image_event(e, line)?,
        CapturedEvent::SpuMfcCmd(e) => validate_spu_mfc_cmd_event(e, line)?,
        CapturedEvent::MfcDmaComplete(e) => validate_mfc_dma_complete_event(e, line)?,
        CapturedEvent::PpuPushInmbox(_) | CapturedEvent::PpuPopOutmbox(_) => {}
        CapturedEvent::PpuSignal(e) => {
            if e.slot > 1 {
                return Err(TraceParseError::BadSignalSlot {
                    line,
                    seq: e.seq,
                    slot: e.slot,
                });
            }
        }
    }

    Ok(())
}

/// R5.9e.2: validate `spu_image` metadata fields (hash format, size,
/// alignment, bounds). Does NOT load or hash the side-file — that is
/// the replay engine's job (R5.9e.5+, deferred). Per-line check;
/// per-SPU uniqueness + ordering live in the post-pass walk.
fn validate_spu_image_event(e: &SpuImageEvent, line: usize) -> Result<(), TraceParseError> {
    // Hash: must be exactly 64 lowercase hex chars.
    if e.image_sha256.len() != 64 {
        return Err(TraceParseError::BadImageHash {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            reason: "image_sha256 must be exactly 64 hex chars",
        });
    }
    if !e.image_sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(TraceParseError::BadImageHash {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            reason: "image_sha256 must be lowercase [0-9a-f] only (no uppercase, no non-hex)",
        });
    }

    // Size: 4..=262144, multiple of 4.
    if e.size == 0 {
        return Err(TraceParseError::BadImageSize {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            size: e.size,
            reason: "size must be > 0",
        });
    }
    if e.size > SPU_LS_BYTES {
        return Err(TraceParseError::BadImageSize {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            size: e.size,
            reason: "size > 256 KB (full SPU local store is 0x40000 bytes)",
        });
    }
    if e.size & 0x3 != 0 {
        return Err(TraceParseError::BadImageSize {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            size: e.size,
            reason: "size must be a multiple of 4 (SPU instructions are 4-byte aligned)",
        });
    }

    // load_addr: 4-byte aligned; load_addr + size must not exceed LS.
    if e.load_addr & 0x3 != 0 {
        return Err(TraceParseError::BadImageLoadAddr {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            load_addr: e.load_addr,
            reason: "load_addr must be 4-byte aligned",
        });
    }
    let end = e.load_addr.checked_add(e.size).ok_or(TraceParseError::BadImageLoadAddr {
        line,
        seq: e.seq,
        target_spu: e.target_spu,
        load_addr: e.load_addr,
        reason: "load_addr + size overflows u32",
    })?;
    if end > SPU_LS_BYTES {
        return Err(TraceParseError::BadImageLoadAddr {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            load_addr: e.load_addr,
            reason: "load_addr + size exceeds 256 KB local store",
        });
    }

    // entry_pc: 4-byte aligned and within LS.
    if e.entry_pc & 0x3 != 0 {
        return Err(TraceParseError::BadImageEntryPc {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            entry_pc: e.entry_pc,
            reason: "entry_pc must be 4-byte aligned",
        });
    }
    if e.entry_pc >= SPU_LS_BYTES {
        return Err(TraceParseError::BadImageEntryPc {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            entry_pc: e.entry_pc,
            reason: "entry_pc out of LS range (must be < 0x40000)",
        });
    }

    Ok(())
}

/// R6.7 A.2: validate `spu_mfc_cmd` metadata fields per design § 3
/// (cmd code subset), § 4.1 (field shape), § 4.3 (rejection codes), and
/// § 5.5 (size + alignment). Does NOT load `.dmachunk` — that's A.3.
/// Per-line check; per-target_spu ordering invariant
/// (`spu_mfc_cmd` immediately follows `spu_wrch ch21`) lives in the
/// post-pass walk.
fn validate_spu_mfc_cmd_event(e: &SpuMfcCmdEvent, line: usize) -> Result<(), TraceParseError> {
    check_pc(line, e.seq, e.pc)?;

    // R6.7: only GET (0x40). R8.1: also PUT (0x20). R8.4c: GETL
    // (0x44) also accepted, BUT with stricter additive-fields
    // validation (descriptor_sha256, descriptor_size,
    // element_chunks, element_sizes, element_eals must all be
    // present and internally consistent). Other list-DMA codes
    // (PUTL/PUTLB/PUTLF/GETLB/GETLF) still surface
    // `UnsupportedMfcListCmd`. Everything else (atomic, barrier,
    // sync, sndsig) still surfaces the generic `UnsupportedMfcCmd`.
    if e.cmd != MFC_GET_CMD && e.cmd != MFC_PUT_CMD && e.cmd != MFC_GETL_CMD {
        if is_mfc_list_cmd_unsupported(e.cmd) {
            return Err(TraceParseError::UnsupportedMfcListCmd {
                line,
                seq: e.seq,
                target_spu: e.target_spu,
                cmd: e.cmd,
            });
        }
        return Err(TraceParseError::UnsupportedMfcCmd {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            cmd: e.cmd,
        });
    }

    // R8.4c — GETL additive-fields validation. For cmd=0x44, all
    // five list fields MUST be present AND internally consistent:
    //   - descriptor_sha256 = 64 lower-hex
    //   - descriptor_size = e.size, multiple of 8, > 0, <= 0x800
    //   - element_chunks.len() == element_sizes.len()
    //     == element_eals.len() == descriptor_size / 8
    //   - each element_chunks entry = 64 lower-hex
    //   - each element_sizes entry > 0, <= 0x4000 (R6.7 simple-cmd cap)
    //   - each element_eals.eah = 0 (mirrors simple-cmd validation
    //     which enforces eah=0 on the parent event)
    //
    // For GET/PUT (0x40 / 0x20), the additive fields MUST be
    // absent (writer convention — schema additive).
    if e.cmd == MFC_GETL_CMD {
        validate_getl_additive_fields(e, line)?;
    } else {
        // R8.4c — for non-list cmds, the additive list fields
        // MUST be absent. Reject defensively if the writer ever
        // populates them on a simple GET/PUT (would indicate a
        // writer bug).
        if e.descriptor_sha256.is_some()
            || e.descriptor_size.is_some()
            || e.element_chunks.is_some()
            || e.element_sizes.is_some()
            || e.element_eals.is_some()
        {
            return Err(TraceParseError::UnsupportedMfcCmd {
                line,
                seq: e.seq,
                target_spu: e.target_spu,
                cmd: e.cmd,
            });
        }
    }

    if e.eah != 0 {
        return Err(TraceParseError::UnsupportedMfcEah {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            eah: e.eah,
        });
    }

    if e.tag > MFC_TAG_MAX {
        return Err(TraceParseError::BadMfcTag {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            tag: e.tag,
        });
    }

    // R8.4c — GETL: size/lsa validation is GETL-specific
    // (size = descriptor bytes, multiple of 8; lsa = dest base,
    // independent of descriptor size). The simple-cmd checks
    // below would reject valid GETL events (e.g. size=24 with
    // 3 elements isn't a multiple of 16). The additive-fields
    // validation called above (`validate_getl_additive_fields`)
    // already enforced size constraints + per-element sanity;
    // ea_chunk_sha256 was validated as a 64-hex string by the
    // writer's overload convention.
    if e.cmd == MFC_GETL_CMD {
        // Validate ea_chunk_sha256 shape (writer overloads it to
        // the descriptor SHA for GETL; we still require valid
        // 64-hex lowercase).
        validate_sha_hex_field(e, line, &e.ea_chunk_sha256, "ea_chunk_sha256")?;
        return Ok(());
    }

    // Size: 1, 2, 4, 8 OR multiple of 16 in [16, 16384].
    if e.size == 0 {
        return Err(TraceParseError::BadDmaSize {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            size: e.size,
            reason: "size must be > 0",
        });
    }
    if e.size > MFC_DMA_SIZE_MAX {
        return Err(TraceParseError::BadDmaSize {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            size: e.size,
            reason: "size > 0x4000 (16 KiB MFC simple-cmd cap; list DMA out of R6.7 scope)",
        });
    }
    let size_ok = matches!(e.size, 1 | 2 | 4 | 8) || (e.size >= 16 && e.size & 0xf == 0);
    if !size_ok {
        return Err(TraceParseError::BadDmaSize {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            size: e.size,
            reason: "size must be 1, 2, 4, 8, or a multiple of 16 in [16, 16384]",
        });
    }

    // LSA alignment: small sizes need natural alignment; >= 16 needs
    // 16-byte alignment. lsa + size must fit in the 256 KiB local store.
    let alignment_mask = match e.size {
        1 => 0,
        2 => 0x1,
        4 => 0x3,
        8 => 0x7,
        _ => 0xf, // size >= 16 → 16-byte
    };
    if e.lsa & alignment_mask != 0 {
        return Err(TraceParseError::BadDmaLsa {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            lsa: e.lsa,
            reason: "lsa not aligned for the dispatched size",
        });
    }
    let lsa_end = e.lsa.checked_add(e.size).ok_or(TraceParseError::BadDmaLsa {
        line,
        seq: e.seq,
        target_spu: e.target_spu,
        lsa: e.lsa,
        reason: "lsa + size overflows u32",
    })?;
    if lsa_end > SPU_LS_BYTES {
        return Err(TraceParseError::BadDmaLsa {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            lsa: e.lsa,
            reason: "lsa + size exceeds 256 KiB local store",
        });
    }

    // ea_chunk_sha256: 64-char lowercase hex. Mirrors BadImageHash.
    if e.ea_chunk_sha256.len() != 64 {
        return Err(TraceParseError::BadDmaSha {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            reason: "ea_chunk_sha256 must be exactly 64 hex chars",
        });
    }
    if !e.ea_chunk_sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(TraceParseError::BadDmaSha {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            reason: "ea_chunk_sha256 must be lowercase [0-9a-f] only (no uppercase, no non-hex)",
        });
    }

    Ok(())
}

/// R8.4c — validate a 64-char lowercase hex SHA string field on
/// an MFC cmd event. Used for `ea_chunk_sha256` (GETL overload)
/// and `descriptor_sha256` + each `element_chunks[i]`. Mirrors
/// the existing inline ea_chunk_sha256 validation in
/// `validate_spu_mfc_cmd_event`.
fn validate_sha_hex_field(
    e: &SpuMfcCmdEvent,
    line: usize,
    sha: &str,
    field_name: &'static str,
) -> Result<(), TraceParseError> {
    if sha.len() != 64 {
        // Reuse `BadDmaSha` — the operational meaning ("bad SHA
        // string in MFC event") transfers exactly; the `reason`
        // string carries the field-specific context.
        return Err(TraceParseError::BadDmaSha {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            reason: match field_name {
                "ea_chunk_sha256" => "ea_chunk_sha256 must be exactly 64 hex chars",
                "descriptor_sha256" => "descriptor_sha256 must be exactly 64 hex chars",
                "element_chunks[i]" => "element_chunks[i] must be exactly 64 hex chars",
                _ => "sha256 field must be exactly 64 hex chars",
            },
        });
    }
    if !sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err(TraceParseError::BadDmaSha {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            reason: match field_name {
                "ea_chunk_sha256" => {
                    "ea_chunk_sha256 must be lowercase [0-9a-f] only (no uppercase, no non-hex)"
                }
                "descriptor_sha256" => {
                    "descriptor_sha256 must be lowercase [0-9a-f] only (no uppercase, no non-hex)"
                }
                "element_chunks[i]" => {
                    "element_chunks[i] must be lowercase [0-9a-f] only (no uppercase, no non-hex)"
                }
                _ => "sha256 field must be lowercase [0-9a-f] only",
            },
        });
    }
    Ok(())
}

/// R8.4c — validate the five additive list-DMA fields on a GETL
/// `spu_mfc_cmd` event. All must be present; descriptor_size
/// must match `e.size`; element counts must match
/// `descriptor_size / 8`; per-element constraints (size > 0,
/// size <= 0x4000, sha shape valid). The structural rejection
/// of `sb` stall-and-notify bit + descriptor parsing per-element
/// (8-byte BE layout) happens later in the state machine
/// (`MfcReplayState::process_mfc_list_cmd`) — the parser only
/// validates the JSONL schema shape.
fn validate_getl_additive_fields(
    e: &SpuMfcCmdEvent,
    line: usize,
) -> Result<(), TraceParseError> {
    // All five fields MUST be Some.
    let desc_sha = e.descriptor_sha256.as_ref().ok_or(
        TraceParseError::UnsupportedMfcCmd {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            cmd: e.cmd,
        },
    )?;
    let desc_size = e.descriptor_size.ok_or(
        TraceParseError::UnsupportedMfcCmd {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            cmd: e.cmd,
        },
    )?;
    let elements = e.element_chunks.as_ref().ok_or(
        TraceParseError::UnsupportedMfcCmd {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            cmd: e.cmd,
        },
    )?;
    let sizes = e.element_sizes.as_ref().ok_or(
        TraceParseError::UnsupportedMfcCmd {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            cmd: e.cmd,
        },
    )?;
    let eals = e.element_eals.as_ref().ok_or(
        TraceParseError::UnsupportedMfcCmd {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            cmd: e.cmd,
        },
    )?;

    // descriptor_sha256 = 64-hex
    validate_sha_hex_field(e, line, desc_sha, "descriptor_sha256")?;

    // descriptor_size = e.size, multiple of 8, > 0, <= 0x800.
    if desc_size != e.size {
        return Err(TraceParseError::BadDmaSize {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            size: desc_size,
            reason: "descriptor_size must equal spu_mfc_cmd.size",
        });
    }
    if desc_size == 0 || desc_size > 0x800 || desc_size & 0x7 != 0 {
        return Err(TraceParseError::BadDmaSize {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            size: desc_size,
            reason: "GETL descriptor_size must be in (0, 0x800] and multiple of 8",
        });
    }

    // Element-count consistency.
    let element_count = (desc_size / 8) as usize;
    if elements.len() != element_count
        || sizes.len() != element_count
        || eals.len() != element_count
    {
        return Err(TraceParseError::MalformedMfcSequence {
            target_spu: e.target_spu,
            wrch_event_index: line,
            reason: "GETL element_chunks / element_sizes / element_eals length must equal descriptor_size / 8",
        });
    }

    // Per-element validation.
    for (i, chunk_sha) in elements.iter().enumerate() {
        validate_sha_hex_field(e, line, chunk_sha, "element_chunks[i]")?;
        let ts = sizes[i];
        if ts == 0 || ts > MFC_DMA_SIZE_MAX {
            return Err(TraceParseError::BadDmaSize {
                line,
                seq: e.seq,
                target_spu: e.target_spu,
                size: ts,
                reason: "GETL element_sizes[i] must be in (0, 0x4000] (R6.7 simple-cmd per-element cap)",
            });
        }
        // EAL = data EA for element i. No constraint beyond u32
        // (eah=0 enforced on the parent event; per-element eah
        // is implicitly 0). Index into `eals` here is just for
        // length consistency — value range is unconstrained.
        let _eal = eals[i];
    }

    Ok(())
}

/// R6.7 A.2: validate `mfc_dma_complete` metadata. Tag in 0..32;
/// transferred_bytes in 1..=16384. The per-trace per-tag invariant
/// (`transferred_bytes == size`) is the transformer's / replay's job
/// (A.4) — at parse time we only validate the standalone bounds.
fn validate_mfc_dma_complete_event(
    e: &MfcDmaCompleteEvent,
    line: usize,
) -> Result<(), TraceParseError> {
    if e.tag > MFC_TAG_MAX {
        return Err(TraceParseError::BadMfcTag {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            tag: e.tag,
        });
    }
    if e.transferred_bytes == 0 {
        return Err(TraceParseError::BadDmaTransferredBytes {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            transferred_bytes: e.transferred_bytes,
            reason: "transferred_bytes must be > 0",
        });
    }
    if e.transferred_bytes > MFC_DMA_SIZE_MAX {
        return Err(TraceParseError::BadDmaTransferredBytes {
            line,
            seq: e.seq,
            target_spu: e.target_spu,
            transferred_bytes: e.transferred_bytes,
            reason: "transferred_bytes > 0x4000 (16 KiB R6.7 cap)",
        });
    }
    Ok(())
}

fn check_pc(line: usize, seq: u64, pc: u32) -> Result<(), TraceParseError> {
    if pc & 0x3 != 0 {
        return Err(TraceParseError::BadPc {
            line,
            seq,
            pc,
            reason: "not 4-byte aligned",
        });
    }
    if pc >= SPU_LS_BYTES {
        return Err(TraceParseError::BadPc {
            line,
            seq,
            pc,
            reason: "out of LS range",
        });
    }
    Ok(())
}

fn check_channel(line: usize, seq: u64, channel: u32) -> Result<(), TraceParseError> {
    if channel > CHANNEL_MAX {
        return Err(TraceParseError::BadChannel {
            line,
            seq,
            channel,
        });
    }
    Ok(())
}

// =====================================================================
// Transformer
// =====================================================================

/// Errors from [`captured_events_to_trace`]. Always carry the
/// 0-based event index so callers can pinpoint the divergence in the
/// captured stream.
#[derive(Debug, Clone, PartialEq)]
pub enum TraceTransformError {
    /// A `final_state` event appeared before a terminal `spu_stop`.
    /// The schema requires final_state after the SPU has stopped.
    FinalStateBeforeStop {
        final_state_event_index: usize,
    },
    /// Trace ended without a `spu_stop` or `final_state`.
    UnterminatedTrace { event_count: usize },
    /// A signal slot was not 0 or 1 — should be caught by the
    /// parser, but defensive at the transformer layer too.
    InvalidSignalSlot {
        event_index: usize,
        slot: u32,
    },
    /// R5.9b: the single-SPU API [`captured_events_to_trace`] received a
    /// trace whose events touch more than one distinct `target_spu`.
    /// Callers that need to consume such a trace MUST switch to
    /// [`captured_events_to_traces_per_spu`], which returns one
    /// `Vec<TraceEvent>` per SPU. Returning an error here prevents
    /// legacy callers from silently flattening multi-SPU traces — which
    /// would mix events across SPUs and produce incorrect replay state.
    MultipleSpusUnsupportedBySingleSpuApi { spu_count: usize },
    /// R6.7 A.2: a parsed trace contains `spu_mfc_cmd` or
    /// `mfc_dma_complete` events. Parser-level validation succeeded
    /// (cmd is GET, fields are well-formed); the transformer still
    /// refuses the trace because the replay state machine
    /// (`MfcReplayState`) hasn't landed yet — `.dmachunk` side-files
    /// would be needed to populate LS, which is the A.3/A.4 scope.
    /// Surfaces with the per-SPU local index of the offending event
    /// so the failure mode is sharp. R6.7 A.2 explicitly does NOT
    /// silently ignore MFC events nor synthesize a fake-success path.
    UnsupportedDmaInTrace {
        event_index: usize,
        target_spu: u32,
        kind: &'static str,
    },
}

impl std::fmt::Display for TraceTransformError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FinalStateBeforeStop { final_state_event_index } => {
                write!(
                    f,
                    "trace transform error: final_state at event index {final_state_event_index} appears before terminal spu_stop"
                )
            }
            Self::UnterminatedTrace { event_count } => {
                write!(
                    f,
                    "trace transform error: trace has {event_count} events but no spu_stop / final_state terminator"
                )
            }
            Self::InvalidSignalSlot { event_index, slot } => {
                write!(
                    f,
                    "trace transform error at event {event_index}: signal slot {slot} not in {{0, 1}}"
                )
            }
            Self::MultipleSpusUnsupportedBySingleSpuApi { spu_count } => {
                write!(
                    f,
                    "trace transform error: single-SPU API received {spu_count} distinct target_spu ids; use captured_events_to_traces_per_spu instead"
                )
            }
            Self::UnsupportedDmaInTrace { event_index, target_spu, kind } => {
                write!(
                    f,
                    "trace transform error at per-SPU event index {event_index} for target_spu {target_spu}: \
                     unsupported {kind} (R6.7 A.2 parser accepts MFC events; replay state machine + .dmachunk \
                     loading are deferred to A.3/A.4)"
                )
            }
        }
    }
}

impl std::error::Error for TraceTransformError {}

/// Internal SPU state machine used by the transformer.
#[derive(Debug, Clone, Copy)]
enum SpuStateMachine {
    Running,
    Parked { reason: SpuParkReason },
    Finished,
}

/// Transform a parsed `&[CapturedEvent]` into a `Vec<TraceEvent>` per
/// `docs/SPU_TRACE_CAPTURE.md` § "Mapping to R5.5 `TraceEvent`".
///
/// **Single-SPU API.** Under R5.9b this is a thin wrapper over
/// [`captured_events_to_traces_per_spu`]: it groups by `target_spu` and
/// returns the unique group's `Vec<TraceEvent>`. If the input touches
/// more than one `target_spu`, returns
/// [`TraceTransformError::MultipleSpusUnsupportedBySingleSpuApi`] —
/// this prevents legacy callers from silently flattening multi-SPU
/// traces (which would mix unrelated SPUs' events into one replay
/// timeline and corrupt state).
///
/// Maps (per-SPU subsequence):
/// - `spu_park` → [`TraceEvent::ExpectSpuPark`] (+ optional
///   `ExpectChannelState` if `channels_at_park` is present).
/// - `ppu_push_inmbox` / `ppu_pop_outmbox` / `ppu_signal` →
///   corresponding `TraceEvent::Ppu*` with `expect_wake` projected
///   from current SPU state (`Ready` if the action satisfies the
///   current park, `StillBlocked` if parked on a different reason,
///   `NotParked` if SPU is running or finished).
/// - `spu_stop` → [`TraceEvent::ExpectSpuFinished`].
/// - `final_state` → one [`TraceEvent::ExpectChannelState`] +
///   one [`TraceEvent::ExpectGprWord`] per listed register.
///
/// Discards (state-machine context only): `spu_rdch`, `spu_wrch`,
/// `spu_rchcnt`, `spu_wake`. The schema's invariants ensure these
/// don't carry assertions the transformer would otherwise emit.
pub fn captured_events_to_trace(
    events: &[CapturedEvent],
) -> Result<Vec<TraceEvent>, TraceTransformError> {
    let per_spu = captured_events_to_traces_per_spu(events)?;
    match per_spu.len() {
        // Empty input — preserve pre-R5.9b behavior of erroring as
        // unterminated. The state machine over an empty subset would
        // produce the same outcome.
        0 => Err(TraceTransformError::UnterminatedTrace { event_count: 0 }),
        // Single SPU — drain the only entry and return its trace. This
        // is the path R5.7/R5.8 single-SPU traces (and the synthetic
        // `R5_6_REFERENCE_JSONL`) take, so behavior is identical to the
        // pre-R5.9b implementation.
        1 => Ok(per_spu
            .into_iter()
            .next()
            .expect("len == 1 above")
            .1),
        // Multi-SPU — refuse rather than silently flatten. The right
        // API for these traces is `captured_events_to_traces_per_spu`.
        spu_count => Err(TraceTransformError::MultipleSpusUnsupportedBySingleSpuApi {
            spu_count,
        }),
    }
}

/// R5.9b: group captured events by `target_spu` and run the per-SPU
/// transformer over each group. Returns one `Vec<TraceEvent>` per
/// SPU, keyed on the `target_spu` value.
///
/// Determinism: the returned `BTreeMap` is sorted by SPU id, so
/// iteration order is stable across runs. Within each value `Vec`,
/// events appear in the same relative order they had in the input
/// (the parser already validated globally-monotonic `seq`, so per-SPU
/// order is implied).
///
/// Empty input → empty `BTreeMap` (no error). Each non-empty group is
/// transformed with the same state-machine + invariants the single-SPU
/// API uses; per-group errors carry an `event_index` that is local to
/// the per-SPU subsequence (NOT the global event index). For
/// single-SPU traces (every event has `target_spu == 0`) the only
/// group is `0` and the indices are identical to the global ones.
///
/// **R5.9b scope:** parser-level discrimination is already in place
/// (R5.9a). Transformer is now per-SPU. Replay is still single-SPU
/// (deferred to R5.9e). Writer C++ does NOT yet emit `target_spu` on
/// SPU-side events (R5.9c); under the current writer all SPU events
/// collapse to id 0 via the parser's default-zero shim, and this
/// function would return either zero groups (for inputs that are all
/// PPU-side targeting non-zero ids — unlikely in practice) or one
/// group keyed `0`. After R5.9c, real captures will yield one group
/// per `lv2_id`.
pub fn captured_events_to_traces_per_spu(
    events: &[CapturedEvent],
) -> Result<BTreeMap<u32, Vec<TraceEvent>>, TraceTransformError> {
    // Group references by target_spu, preserving relative order. We
    // intentionally do NOT clone the events — refs are cheap and the
    // per-SPU transformer doesn't need ownership.
    let mut groups: BTreeMap<u32, Vec<&CapturedEvent>> = BTreeMap::new();
    for event in events {
        groups.entry(event.target_spu()).or_default().push(event);
    }

    let mut out: BTreeMap<u32, Vec<TraceEvent>> = BTreeMap::new();
    for (target_spu, group) in groups {
        let trace = transform_single_spu_subset(&group)?;
        out.insert(target_spu, trace);
    }
    Ok(out)
}

/// Per-SPU transformer state machine. Internal helper used by both
/// [`captured_events_to_trace`] (single-SPU wrapper) and
/// [`captured_events_to_traces_per_spu`] (multi-SPU API). Operates on a
/// borrowed slice of references so neither caller has to clone events.
fn transform_single_spu_subset(
    events: &[&CapturedEvent],
) -> Result<Vec<TraceEvent>, TraceTransformError> {
    let mut out = Vec::new();
    // RPCS3's writer may not emit an initial spu_park event when the
    // PPU writes a mailbox before the SPU has had a chance to run
    // (race-free single-round homebrew case: PPU pushes IN_MBOX
    // before sysSpuThreadGroupStart actually schedules the SPU). The
    // replay driver always step_spu()s first and the SPU parks on
    // its first blocking channel op, so when the trace's first
    // non-image event is a PPU action we must infer the matching
    // Parked state — otherwise the transformer emits expect_wake =
    // NotParked and the wake check fails against actual Ready.
    let mut state = infer_initial_state(events);

    for (idx, &event) in events.iter().enumerate() {
        match event {
            CapturedEvent::SpuRdch(_)
            | CapturedEvent::SpuWrch(_)
            | CapturedEvent::SpuRchcnt(_)
            | CapturedEvent::SpuWake(_) => {
                // Pure state-machine context — no TraceEvent emission.
            }
            CapturedEvent::SpuImage(_) => {
                // R5.9e.2: metadata-only event. The replay engine
                // (R5.9e.5+, deferred) consumes it via a separate
                // SpuProgram-builder path; the transformer's
                // event-by-event state machine has nothing to do for
                // it. NOT a state-machine context (it doesn't change
                // SPU running/parked/finished state) — explicitly
                // skipped so the transformer remains valid even when
                // the trace stream includes images.
            }
            CapturedEvent::SpuMfcCmd(_) | CapturedEvent::MfcDmaComplete(_) => {
                // R6.7 C.5 — MFC events are now legal context.
                //
                // The pre-replay step
                // (`crate::mfc_replay::apply_mfc_dma_pre_replay`)
                // walks the captured event stream BEFORE the SPU
                // runs, applies any GET DMA into a 256 KiB LS scratch,
                // and pre-populates the tag-stat queue
                // (`SpuProgram::initial_mfc_tag_stat_queue`). By the
                // time replay starts, the SPU's own `wrch ch16-21`
                // are no-ops on the channel side (ch21 returns Ok
                // without re-doing the DMA), and `rdch ch24` pops
                // pre-populated values. So at THIS layer (the trace
                // transformer that emits TraceEvents), MFC events are
                // pure context — they neither produce TraceEvents
                // nor advance the running/parked/finished state
                // machine. Same treatment as `spu_wrch ch16-23`
                // and `spu_rdch ch24`, which fall through to the
                // catch-all "context-only" arm above.
                //
                // Parser-level validation (`UnsupportedMfcCmd`,
                // `UnsupportedMfcEah`, etc.) still rejects malformed
                // MFC traces at parse time. The transformer only sees
                // events that already passed parser validation.
            }
            CapturedEvent::SpuPark(p) => {
                let reason = park_reason(p.reason, p.channel);
                out.push(TraceEvent::ExpectSpuPark {
                    reason,
                    pc: Some(p.pc),
                });
                state = SpuStateMachine::Parked { reason };
                if let Some(channels) = &p.channels_at_park {
                    out.push(channels_to_trace_event(channels));
                }
            }
            CapturedEvent::SpuStop(s) => {
                out.push(TraceEvent::ExpectSpuFinished {
                    stop_code: s.stop_code,
                });
                state = SpuStateMachine::Finished;
                // SYS_SPU_THREAD_STOP_GROUP_EXIT (0x101) and
                // SYS_SPU_THREAD_STOP_THREAD_EXIT (0x102) are
                // architecturally defined to have the lv2 kernel
                // drain OUT_MBOX as the exit/group-join status.
                // RPCS3's writer captures FinalState AFTER this
                // drain, so the channels.out_mbox field reads as
                // null even though the SPU just wrote it. Inject
                // a synthetic drain so the replay's channel state
                // matches the captured FinalState.
                if matches!(s.stop_code, 0x101 | 0x102) {
                    out.push(TraceEvent::PpuPopOutMbox {
                        expect: None,
                        expect_wake: None,
                    });
                }
            }
            CapturedEvent::FinalState(f) => {
                if !matches!(state, SpuStateMachine::Finished) {
                    return Err(TraceTransformError::FinalStateBeforeStop {
                        final_state_event_index: idx,
                    });
                }
                out.push(channels_to_trace_event(&f.channels));
                for entry in &f.gpr_lane_zero {
                    out.push(TraceEvent::ExpectGprWord {
                        reg: entry.reg as usize,
                        lane: 0,
                        value: entry.value,
                    });
                }
            }
            CapturedEvent::PpuPushInmbox(p) => {
                let expect_wake = wake_kind_for_push_inmbox(state);
                out.push(TraceEvent::PpuPushInMbox {
                    value: p.value,
                    expect_wake,
                });
                if matches!(expect_wake, SpuWakeResultKind::Ready) {
                    state = SpuStateMachine::Running;
                }
            }
            CapturedEvent::PpuPopOutmbox(p) => {
                let expect_wake = wake_kind_for_pop_outmbox(state);
                out.push(TraceEvent::PpuPopOutMbox {
                    expect: p.value,
                    expect_wake: Some(expect_wake),
                });
                if matches!(expect_wake, SpuWakeResultKind::Ready) {
                    state = SpuStateMachine::Running;
                }
            }
            CapturedEvent::PpuSignal(p) => {
                if p.slot > 1 {
                    return Err(TraceTransformError::InvalidSignalSlot {
                        event_index: idx,
                        slot: p.slot,
                    });
                }
                let expect_wake = wake_kind_for_signal(state, p.slot);
                out.push(TraceEvent::PpuSignal {
                    slot: p.slot as usize,
                    value: p.value,
                    expect_wake,
                });
                if matches!(expect_wake, SpuWakeResultKind::Ready) {
                    state = SpuStateMachine::Running;
                }
            }
        }
    }

    if !matches!(state, SpuStateMachine::Finished) {
        return Err(TraceTransformError::UnterminatedTrace {
            event_count: events.len(),
        });
    }

    Ok(out)
}

fn park_reason(reason: CapturedParkReason, channel: u32) -> SpuParkReason {
    match reason {
        CapturedParkReason::ChannelRead => SpuParkReason::ChannelRead { channel },
        CapturedParkReason::ChannelWrite => SpuParkReason::ChannelWrite { channel },
    }
}

fn channels_to_trace_event(c: &CapturedChannels) -> TraceEvent {
    TraceEvent::ExpectChannelState {
        in_mbox: c.in_mbox,
        out_mbox: c.out_mbox,
        out_intr_mbox: c.out_intr_mbox,
        snr1: c.snr1,
        snr2: c.snr2,
    }
}

/// Pick the transformer's initial SPU state based on the first
/// applicable captured event for this target_spu.
///
/// `SpuImage` is metadata; we skip it. Any SPU side event implies the
/// SPU has already run (and any preceding park would have been
/// captured), so the default `Running` is correct.
///
/// A leading PPU action implies that the PPU acted before the SPU
/// ran, in which case the replay driver's mandatory `step_spu()`
/// first call will park the SPU on whatever channel the program
/// blocks on. We pick the matching Parked variant so
/// `wake_kind_for_*` returns `Ready`, which is what the driver will
/// actually report.
fn infer_initial_state(events: &[&CapturedEvent]) -> SpuStateMachine {
    for &ev in events {
        match ev {
            CapturedEvent::SpuImage(_) => continue,
            CapturedEvent::PpuPushInmbox(_) => {
                return SpuStateMachine::Parked {
                    reason: SpuParkReason::ChannelRead { channel: 29 },
                };
            }
            CapturedEvent::PpuPopOutmbox(_) => {
                return SpuStateMachine::Parked {
                    reason: SpuParkReason::ChannelWrite { channel: 28 },
                };
            }
            CapturedEvent::PpuSignal(p) => {
                let channel = if p.slot == 0 { 3 } else { 4 };
                return SpuStateMachine::Parked {
                    reason: SpuParkReason::ChannelRead { channel },
                };
            }
            _ => return SpuStateMachine::Running,
        }
    }
    SpuStateMachine::Running
}

fn wake_kind_for_push_inmbox(state: SpuStateMachine) -> SpuWakeResultKind {
    match state {
        SpuStateMachine::Parked {
            reason: SpuParkReason::ChannelRead { channel: 29 },
        } => SpuWakeResultKind::Ready,
        SpuStateMachine::Parked { .. } => SpuWakeResultKind::StillBlocked,
        SpuStateMachine::Running | SpuStateMachine::Finished => SpuWakeResultKind::NotParked,
    }
}

fn wake_kind_for_pop_outmbox(state: SpuStateMachine) -> SpuWakeResultKind {
    match state {
        SpuStateMachine::Parked {
            reason: SpuParkReason::ChannelWrite { channel: 28 },
        } => SpuWakeResultKind::Ready,
        SpuStateMachine::Parked { .. } => SpuWakeResultKind::StillBlocked,
        SpuStateMachine::Running | SpuStateMachine::Finished => SpuWakeResultKind::NotParked,
    }
}

fn wake_kind_for_signal(state: SpuStateMachine, slot: u32) -> SpuWakeResultKind {
    let target_channel = if slot == 0 { 3 } else { 4 };
    match state {
        SpuStateMachine::Parked {
            reason: SpuParkReason::ChannelRead { channel },
        } if channel == target_channel => SpuWakeResultKind::Ready,
        SpuStateMachine::Parked { .. } => SpuWakeResultKind::StillBlocked,
        SpuStateMachine::Running | SpuStateMachine::Finished => SpuWakeResultKind::NotParked,
    }
}

// =====================================================================
// Reference JSONL fixture (the R5.6 synthetic trace re-encoded)
// =====================================================================

/// Reference JSONL trace mirroring the R5.6 synthetic
/// mailbox-command-protocol fixture. Hand-written from the schema
/// doc's reference example, with `channels_at_park` on the three
/// intermediate parks and `gpr_lane_zero` filtered to {r3, r6} to
/// match `mailbox_command_protocol_trace()` byte-exact through the
/// transformer.
///
/// Each event MUST be on a single line (JSONL invariant). Exposed
/// publicly so JIT-side tests in `rpcs3-spu-recompiler` can reuse
/// the same input as the differential round-trip test.
pub const R5_6_REFERENCE_JSONL: &str = "\
# R5.6 synthetic mailbox-command-protocol fixture as a JSONL trace.
# Program: rdch r3,IN(29); il r4,0xFF; ceq r5,r3,r4; brnz r5,+4(HALT);
#          ai r6,r3,0x29; wrch r6,OUT(28); br -6(LOOP); stop 0xD5

{\"seq\":0,\"side\":\"spu\",\"kind\":\"spu_rdch\",\"pc\":256,\"channel\":29,\"value\":null,\"would_stall\":true}
{\"seq\":1,\"side\":\"spu\",\"kind\":\"spu_park\",\"pc\":256,\"reason\":\"channel_read\",\"channel\":29}

{\"seq\":2,\"side\":\"ppu\",\"kind\":\"ppu_push_inmbox\",\"target_spu\":0,\"value\":1}
{\"seq\":3,\"side\":\"spu\",\"kind\":\"spu_wake\",\"pc\":256}
{\"seq\":4,\"side\":\"spu\",\"kind\":\"spu_rdch\",\"pc\":256,\"channel\":29,\"value\":1,\"would_stall\":false}
{\"seq\":5,\"side\":\"spu\",\"kind\":\"spu_wrch\",\"pc\":276,\"channel\":28,\"value\":42,\"would_stall\":false}

{\"seq\":6,\"side\":\"spu\",\"kind\":\"spu_rdch\",\"pc\":256,\"channel\":29,\"value\":null,\"would_stall\":true}
{\"seq\":7,\"side\":\"spu\",\"kind\":\"spu_park\",\"pc\":256,\"reason\":\"channel_read\",\"channel\":29,\"channels_at_park\":{\"in_mbox\":null,\"out_mbox\":42,\"out_intr_mbox\":null,\"snr1\":0,\"snr2\":0}}

{\"seq\":8,\"side\":\"ppu\",\"kind\":\"ppu_push_inmbox\",\"target_spu\":0,\"value\":2}
{\"seq\":9,\"side\":\"spu\",\"kind\":\"spu_wake\",\"pc\":256}
{\"seq\":10,\"side\":\"spu\",\"kind\":\"spu_rdch\",\"pc\":256,\"channel\":29,\"value\":2,\"would_stall\":false}
{\"seq\":11,\"side\":\"spu\",\"kind\":\"spu_wrch\",\"pc\":276,\"channel\":28,\"value\":43,\"would_stall\":true}
{\"seq\":12,\"side\":\"spu\",\"kind\":\"spu_park\",\"pc\":276,\"reason\":\"channel_write\",\"channel\":28,\"channels_at_park\":{\"in_mbox\":null,\"out_mbox\":42,\"out_intr_mbox\":null,\"snr1\":0,\"snr2\":0}}

{\"seq\":13,\"side\":\"ppu\",\"kind\":\"ppu_pop_outmbox\",\"target_spu\":0,\"value\":42}
{\"seq\":14,\"side\":\"spu\",\"kind\":\"spu_wake\",\"pc\":276}
{\"seq\":15,\"side\":\"spu\",\"kind\":\"spu_wrch\",\"pc\":276,\"channel\":28,\"value\":43,\"would_stall\":false}

{\"seq\":16,\"side\":\"spu\",\"kind\":\"spu_rdch\",\"pc\":256,\"channel\":29,\"value\":null,\"would_stall\":true}
{\"seq\":17,\"side\":\"spu\",\"kind\":\"spu_park\",\"pc\":256,\"reason\":\"channel_read\",\"channel\":29,\"channels_at_park\":{\"in_mbox\":null,\"out_mbox\":43,\"out_intr_mbox\":null,\"snr1\":0,\"snr2\":0}}

{\"seq\":18,\"side\":\"ppu\",\"kind\":\"ppu_push_inmbox\",\"target_spu\":0,\"value\":255}
{\"seq\":19,\"side\":\"spu\",\"kind\":\"spu_wake\",\"pc\":256}
{\"seq\":20,\"side\":\"spu\",\"kind\":\"spu_rdch\",\"pc\":256,\"channel\":29,\"value\":255,\"would_stall\":false}
{\"seq\":21,\"side\":\"spu\",\"kind\":\"spu_stop\",\"pc\":284,\"stop_code\":213}

{\"seq\":22,\"side\":\"ppu\",\"kind\":\"ppu_pop_outmbox\",\"target_spu\":0,\"value\":43}
{\"seq\":23,\"side\":\"spu\",\"kind\":\"final_state\",\"gpr_lane_zero\":[{\"reg\":3,\"value\":255},{\"reg\":6,\"value\":43}],\"channels\":{\"in_mbox\":null,\"out_mbox\":null,\"out_intr_mbox\":null,\"snr1\":0,\"snr2\":0}}
";

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{
        mailbox_command_protocol_program, mailbox_command_protocol_trace, replay_trace,
        InterpreterExecutor, SpuEventKind,
    };

    /// `parse_jsonl_trace` decodes the reference JSONL trace into 24
    /// captured events and validates key field shapes.
    #[test]
    fn parse_reference_jsonl_yields_24_events() {
        let events = parse_jsonl_trace(R5_6_REFERENCE_JSONL).expect("parse must succeed");
        assert_eq!(events.len(), 24, "reference example has 24 events");

        // First park: rdch on RDINMBOX(29) at pc 0x100, no channels_at_park.
        match &events[1] {
            CapturedEvent::SpuPark(e) => {
                assert_eq!(e.seq, 1);
                assert_eq!(e.pc, 0x100);
                assert_eq!(e.channel, 29);
                assert_eq!(e.reason, CapturedParkReason::ChannelRead);
                assert!(e.channels_at_park.is_none());
            }
            other => panic!("expected SpuPark at index 1, got {other:?}"),
        }

        // First PPU push.
        match &events[2] {
            CapturedEvent::PpuPushInmbox(e) => {
                assert_eq!(e.seq, 2);
                assert_eq!(e.value, 1);
            }
            other => panic!("expected PpuPushInmbox at index 2, got {other:?}"),
        }

        // First PPU pop (drains 0x2A=42).
        match &events[13] {
            CapturedEvent::PpuPopOutmbox(e) => {
                assert_eq!(e.seq, 13);
                assert_eq!(e.value, Some(42));
            }
            other => panic!("expected PpuPopOutmbox at index 13, got {other:?}"),
        }

        // final_state: r3=255, r6=43, channels all empty.
        match &events[23] {
            CapturedEvent::FinalState(e) => {
                assert_eq!(e.seq, 23);
                assert_eq!(e.gpr_lane_zero.len(), 2);
                assert_eq!(e.gpr_lane_zero[0].reg, 3);
                assert_eq!(e.gpr_lane_zero[0].value, 255);
                assert_eq!(e.gpr_lane_zero[1].reg, 6);
                assert_eq!(e.gpr_lane_zero[1].value, 43);
                assert!(e.channels.in_mbox.is_none());
                assert!(e.channels.out_mbox.is_none());
            }
            other => panic!("expected FinalState at index 23, got {other:?}"),
        }
    }

    /// **Load-bearing correctness check.** The transformer MUST
    /// produce a Vec<TraceEvent> byte-exact equal to
    /// `mailbox_command_protocol_trace()`. Compares via Debug
    /// formatting because `TraceEvent` doesn't derive PartialEq.
    #[test]
    fn transform_round_trip_matches_canonical_r5_6_trace() {
        let events = parse_jsonl_trace(R5_6_REFERENCE_JSONL).expect("parse must succeed");
        let transformed = captured_events_to_trace(&events).expect("transform must succeed");
        let canonical = mailbox_command_protocol_trace();

        assert_eq!(transformed.len(), canonical.len(),
            "transformed length {} must equal canonical length {}",
            transformed.len(), canonical.len());
        for (i, (t, c)) in transformed.iter().zip(canonical.iter()).enumerate() {
            assert_eq!(format!("{t:?}"), format!("{c:?}"),
                "transformed[{i}] must match canonical[{i}]\n  transformed: {t:?}\n  canonical:   {c:?}");
        }
    }

    /// Parse → transform → replay through interpreter. Result must
    /// match a direct replay of `mailbox_command_protocol_trace()`.
    #[test]
    fn replay_transformed_trace_through_interpreter() {
        let events = parse_jsonl_trace(R5_6_REFERENCE_JSONL).expect("parse must succeed");
        let trace = captured_events_to_trace(&events).expect("transform must succeed");

        let mut backend = InterpreterExecutor::default();
        let report = replay_trace(&mut backend, mailbox_command_protocol_program(), &trace)
            .expect("replay must succeed");

        assert!(matches!(
            report.final_event_kind,
            SpuEventKind::Finished { stop_code: 0xD5 }
        ));
        assert_eq!(report.records.len(), 16);
        assert_eq!(report.final_snapshot.park_state, None);
        assert_eq!(report.final_snapshot.channels.in_mbox, None);
        assert_eq!(report.final_snapshot.channels.out_mbox, None);
    }

    // -----------------------------------------------------------------
    // Negative tests — schema-level validation.
    // -----------------------------------------------------------------

    /// Non-monotonic seq must be rejected.
    #[test]
    fn parser_rejects_non_monotonic_seq() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_rdch","pc":256,"channel":29,"value":null,"would_stall":true}
{"seq":2,"side":"spu","kind":"spu_park","pc":256,"reason":"channel_read","channel":29}
{"seq":2,"side":"spu","kind":"spu_wake","pc":256}
"#;
        let err = parse_jsonl_trace(jsonl).expect_err("must reject non-monotonic seq");
        match err {
            TraceParseError::NonMonotonicSeq { prev_seq, got_seq, .. } => {
                assert_eq!(prev_seq, 2);
                assert_eq!(got_seq, 2);
            }
            other => panic!("expected NonMonotonicSeq, got {other:?}"),
        }
    }

    /// Side mismatched with kind must be rejected.
    #[test]
    fn parser_rejects_wrong_side_for_kind() {
        // spu_rdch with side="ppu" — wrong.
        let jsonl = r#"{"seq":0,"side":"ppu","kind":"spu_rdch","pc":256,"channel":29,"value":null,"would_stall":true}
"#;
        let err = parse_jsonl_trace(jsonl).expect_err("must reject side/kind mismatch");
        match err {
            TraceParseError::SideKindMismatch {
                kind, got_side, expected_side, ..
            } => {
                assert_eq!(kind, "spu_rdch");
                assert_eq!(got_side, CapturedSide::Ppu);
                assert_eq!(expected_side, CapturedSide::Spu);
            }
            other => panic!("expected SideKindMismatch, got {other:?}"),
        }
    }

    /// `final_state` followed by another event for the same SPU must
    /// be rejected. Migrated to R5.9a `EventAfterFinalState` from the
    /// pre-R5.9 `FinalStateNotTerminal` (which is now a deprecated
    /// variant; see the type doc).
    #[test]
    fn parser_rejects_final_state_not_terminal() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1}
{"seq":1,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}}
{"seq":2,"side":"ppu","kind":"ppu_push_inmbox","target_spu":0,"value":7}
"#;
        // All events implicitly target SPU 0 (PPU event explicitly,
        // SPU events via the missing-target_spu→0 shim). Final_state
        // at index 1 finalizes SPU 0; the PPU event at index 2 then
        // references the already-finalized SPU 0 → rejection.
        let err = parse_jsonl_trace(jsonl).expect_err("must reject events after final_state");
        match err {
            TraceParseError::EventAfterFinalState {
                target_spu,
                event_index,
                final_state_index,
            } => {
                assert_eq!(target_spu, 0);
                assert_eq!(final_state_index, 1);
                assert_eq!(event_index, 2);
            }
            other => panic!("expected EventAfterFinalState, got {other:?}"),
        }
    }

    /// Out-of-range channel id rejected.
    #[test]
    fn parser_rejects_bad_channel() {
        let jsonl = r#"{"seq":0,"side":"spu","kind":"spu_rdch","pc":256,"channel":999,"value":null,"would_stall":true}
"#;
        let err = parse_jsonl_trace(jsonl).expect_err("must reject 7-bit overflow channel");
        match err {
            TraceParseError::BadChannel { channel: 999, .. } => {}
            other => panic!("expected BadChannel, got {other:?}"),
        }
    }

    /// Unaligned PC rejected.
    #[test]
    fn parser_rejects_unaligned_pc() {
        let jsonl = r#"{"seq":0,"side":"spu","kind":"spu_rdch","pc":257,"channel":29,"value":null,"would_stall":true}
"#;
        let err = parse_jsonl_trace(jsonl).expect_err("must reject unaligned pc");
        match err {
            TraceParseError::BadPc { pc: 257, reason, .. } => {
                assert!(reason.contains("aligned"));
            }
            other => panic!("expected BadPc, got {other:?}"),
        }
    }

    /// Bad signal slot rejected.
    #[test]
    fn parser_rejects_bad_signal_slot() {
        let jsonl = r#"{"seq":0,"side":"ppu","kind":"ppu_signal","target_spu":0,"slot":7,"value":1}
"#;
        let err = parse_jsonl_trace(jsonl).expect_err("must reject slot >= 2");
        match err {
            TraceParseError::BadSignalSlot { slot: 7, .. } => {}
            other => panic!("expected BadSignalSlot, got {other:?}"),
        }
    }

    /// Transformer rejects unterminated trace (no spu_stop / final_state).
    #[test]
    fn transform_rejects_unterminated_trace() {
        let events = vec![CapturedEvent::SpuPark(SpuParkEvent {
            seq: 0,
            side: CapturedSide::Spu,
            pc: 0x100,
            reason: CapturedParkReason::ChannelRead,
            channel: 29,
            channels_at_park: None,
            target_spu: None,
        })];
        let err = captured_events_to_trace(&events)
            .expect_err("unterminated trace must error at the transformer layer");
        match err {
            TraceTransformError::UnterminatedTrace { event_count: 1 } => {}
            other => panic!("expected UnterminatedTrace, got {other:?}"),
        }
    }

    /// Transformer rejects final_state appearing before spu_stop in
    /// the captured stream.
    #[test]
    fn transform_rejects_final_state_before_stop() {
        let events = vec![
            CapturedEvent::SpuPark(SpuParkEvent {
                seq: 0,
                side: CapturedSide::Spu,
                pc: 0x100,
                reason: CapturedParkReason::ChannelRead,
                channel: 29,
                channels_at_park: None,
                target_spu: None,
            }),
            CapturedEvent::FinalState(FinalStateEvent {
                seq: 1,
                side: CapturedSide::Spu,
                gpr_lane_zero: vec![],
                channels: CapturedChannels {
                    in_mbox: None,
                    out_mbox: None,
                    out_intr_mbox: None,
                    snr1: 0,
                    snr2: 0,
                },
                target_spu: None,
            }),
        ];
        let err = captured_events_to_trace(&events)
            .expect_err("final_state before spu_stop must error");
        match err {
            TraceTransformError::FinalStateBeforeStop {
                final_state_event_index: 1,
            } => {}
            other => panic!("expected FinalStateBeforeStop, got {other:?}"),
        }
    }

    /// Transformer correctly classifies a PPU push that does NOT
    /// satisfy the current park reason as `StillBlocked` (no fake
    /// success). The transformer must NOT advance the SPU state on
    /// non-Ready wakes.
    #[test]
    fn transform_classifies_wrong_wake_as_still_blocked() {
        // SPU parks on WROUTMBOX(28); PPU pushes to in_mbox. Push
        // does not satisfy a wrch park, so wake is StillBlocked.
        let events = vec![
            CapturedEvent::SpuPark(SpuParkEvent {
                seq: 0,
                side: CapturedSide::Spu,
                pc: 0x100,
                reason: CapturedParkReason::ChannelWrite,
                channel: 28,
                channels_at_park: None,
                target_spu: None,
            }),
            CapturedEvent::PpuPushInmbox(PpuPushInmboxEvent {
                seq: 1,
                side: CapturedSide::Ppu,
                target_spu: 0,
                value: 7,
            }),
            CapturedEvent::SpuStop(SpuStopEvent {
                seq: 2,
                side: CapturedSide::Spu,
                pc: 0x104,
                stop_code: 1,
                target_spu: None,
            }),
        ];
        let trace = captured_events_to_trace(&events).expect("transform ok");
        // Expect: ExpectSpuPark, PpuPushInMbox{StillBlocked}, ExpectSpuFinished.
        assert_eq!(trace.len(), 3);
        match &trace[1] {
            TraceEvent::PpuPushInMbox {
                expect_wake: SpuWakeResultKind::StillBlocked,
                ..
            } => {}
            other => panic!("expected PpuPushInMbox StillBlocked, got {other:?}"),
        }
    }

    /// Comments and blank lines are ignored by the parser.
    #[test]
    fn parser_skips_comments_and_blanks() {
        let jsonl = "
# leading comment
{\"seq\":0,\"side\":\"spu\",\"kind\":\"spu_stop\",\"pc\":256,\"stop_code\":1}

# trailing blank line above
";
        let events = parse_jsonl_trace(jsonl).expect("parse must succeed");
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], CapturedEvent::SpuStop(_)));
    }

    // ---------------------------------------------------------------
    // R5.8 hardening contracts (2026-04-28)
    //
    // These freeze invariants discovered during the first real-trace
    // capture from `spurs_test.self`:
    // 1. multi-SPU traces (more than one `final_state`) MUST be rejected
    //    while the schema is single-SPU-only. spurs_test produced 6
    //    final_state events — the second one already breaks the contract.
    // 2. The parser MUST NOT silently sort events by `seq`. A trace with
    //    backward `seq` (N+1 then N) is rejected; the parser is not a
    //    sort filter.
    // 3. Once the parser returns Err, the transformer is unreachable —
    //    `?` propagation in any pipeline harness short-circuits before
    //    any transform work runs. Sanity-check that integration users
    //    never accidentally feed bad data into the transformer.
    // ---------------------------------------------------------------

    /// Migrated R5.8 hardening contract → R5.9a contract. The original
    /// test asserted that any second `final_state` was rejected (under
    /// the single-SPU rule). Under R5.9a the contract is sharper:
    /// "duplicate `final_state` for the SAME target_spu". A second
    /// final_state for a DIFFERENT SPU is accepted (covered by
    /// `parser_accepts_interleaved_multi_spu_final_states`); the
    /// rejection-path for repeated termination of the same SPU is
    /// preserved here.
    #[test]
    fn parser_rejects_duplicate_final_state_same_spu() {
        // Both final_state events have the same implicit target_spu=0.
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1}
{"seq":1,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}}
{"seq":2,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1}
{"seq":3,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}}
"#;
        let err = parse_jsonl_trace(jsonl)
            .expect_err("parser must reject duplicate final_state for same SPU");
        // Index 2 (second spu_stop, target_spu=0) hits "event after
        // finalized SPU 0" first — that's the canonical R5.9a error
        // path because it triggers BEFORE the second final_state is
        // even reached. The duplicate-final_state path is only taken
        // when the second final_state itself follows the first with
        // no intervening events for the same SPU.
        match err {
            TraceParseError::EventAfterFinalState {
                target_spu,
                event_index,
                final_state_index,
            } => {
                assert_eq!(target_spu, 0);
                assert_eq!(final_state_index, 1);
                assert_eq!(event_index, 2);
            }
            other => panic!("expected EventAfterFinalState, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // R5.9a parser-only multi-SPU contracts (2026-04-28)
    // ---------------------------------------------------------------

    /// Two SPUs each emit their own complete sequence; events
    /// interleave on `seq` order. Each SPU emits exactly one
    /// final_state. Parser accepts.
    #[test]
    fn parser_accepts_interleaved_multi_spu_final_states() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":1}
{"seq":1,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":2}
{"seq":2,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
{"seq":3,"side":"spu","kind":"spu_rchcnt","pc":260,"channel":29,"count":0,"target_spu":2}
{"seq":4,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":2}
"#;
        let events = parse_jsonl_trace(jsonl)
            .expect("parser must accept multi-SPU trace with per-SPU final_states");
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].target_spu(), 1);
        assert_eq!(events[2].target_spu(), 1);
        assert_eq!(events[3].target_spu(), 2);
        assert_eq!(events[4].target_spu(), 2);
    }

    /// After SPU 1's final_state, an event referencing SPU 1 is rejected.
    /// Events for OTHER SPUs in the same trace are unaffected; this test
    /// targets the same-SPU violation specifically.
    #[test]
    fn parser_rejects_event_after_final_state_same_spu() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":1}
{"seq":1,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
{"seq":2,"side":"spu","kind":"spu_rchcnt","pc":260,"channel":29,"count":0,"target_spu":1}
"#;
        let err = parse_jsonl_trace(jsonl)
            .expect_err("event for SPU 1 after SPU 1's final_state must be rejected");
        match err {
            TraceParseError::EventAfterFinalState {
                target_spu,
                event_index,
                final_state_index,
            } => {
                assert_eq!(target_spu, 1);
                assert_eq!(final_state_index, 1);
                assert_eq!(event_index, 2);
            }
            other => panic!("expected EventAfterFinalState for SPU 1, got {other:?}"),
        }
    }

    /// After SPU 1's final_state, events for SPU 2 must still be
    /// accepted — the per-SPU rule is what makes multi-SPU traces
    /// possible. Symmetric to the above.
    #[test]
    fn parser_allows_event_after_final_state_other_spu() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":1}
{"seq":1,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
{"seq":2,"side":"spu","kind":"spu_rchcnt","pc":260,"channel":29,"count":0,"target_spu":2}
{"seq":3,"side":"spu","kind":"spu_stop","pc":260,"stop_code":1,"target_spu":2}
{"seq":4,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":2}
"#;
        let events = parse_jsonl_trace(jsonl)
            .expect("events for SPU 2 after SPU 1's final_state must parse");
        assert_eq!(events.len(), 5);
    }

    /// Single-SPU backward compatibility: SPU-side events without a
    /// `target_spu` field are treated as `target_spu = 0`. This is the
    /// shim that keeps `R5_6_REFERENCE_JSONL` and every other R5.7/R5.8
    /// captured fixture working under the R5.9 parser unchanged.
    #[test]
    fn parser_defaults_missing_target_spu_to_zero() {
        // No target_spu on either SPU-side event.
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1}
{"seq":1,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}}
"#;
        let events = parse_jsonl_trace(jsonl).expect("legacy single-SPU trace must still parse");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].target_spu(), 0);
        assert_eq!(events[1].target_spu(), 0);
    }

    /// Direct duplicate-final_state path: two `final_state` events for
    /// the same SPU with NO intervening events. This exercises the
    /// `DuplicateFinalState` error variant (the
    /// `parser_rejects_duplicate_final_state_same_spu` test above
    /// covers the more common path where an intervening non-final
    /// event triggers `EventAfterFinalState` first).
    #[test]
    fn parser_rejects_back_to_back_duplicate_final_state() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
{"seq":1,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
"#;
        let err = parse_jsonl_trace(jsonl)
            .expect_err("two final_state for the same SPU must be rejected");
        match err {
            TraceParseError::DuplicateFinalState {
                target_spu,
                first_index,
                second_index,
            } => {
                assert_eq!(target_spu, 1);
                assert_eq!(first_index, 0);
                assert_eq!(second_index, 1);
            }
            other => panic!("expected DuplicateFinalState, got {other:?}"),
        }
    }

    /// Parser MUST NOT silently sort events. Out-of-order seq is rejected
    /// even when the events themselves are otherwise well-formed. This
    /// guards against a tempting but oracle-breaking workaround where a
    /// future contributor adds `events.sort_by_key(|e| e.seq())` to mask
    /// writer concurrency bugs (the kind that scaffolding patch v2 fixed).
    #[test]
    fn parser_does_not_auto_sort_backward_seq() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_rdch","pc":256,"channel":29,"value":null,"would_stall":true}
{"seq":2,"side":"spu","kind":"spu_park","pc":256,"reason":"channel_read","channel":29}
{"seq":1,"side":"spu","kind":"spu_wake","pc":256}
"#;
        // If the parser auto-sorted by seq, this would succeed (seq 0,1,2
        // is monotonic). The parser MUST treat the file order as truth
        // and reject the backward jump from 2 to 1.
        let err = parse_jsonl_trace(jsonl)
            .expect_err("parser must reject backward seq, not auto-sort");
        match err {
            TraceParseError::NonMonotonicSeq {
                prev_seq, got_seq, ..
            } => {
                assert_eq!(prev_seq, 2);
                assert_eq!(got_seq, 1);
            }
            other => panic!("expected NonMonotonicSeq, got {other:?}"),
        }
    }

    /// Transformer takes already-parsed `&[CapturedEvent]`, so a parser
    /// failure short-circuits any pipeline use of the transformer.
    /// This test documents the contract: an integration harness that
    /// calls `parse → transform` MUST surface the parse error and NEVER
    /// reach the transformer with malformed input.
    #[test]
    fn transformer_unreachable_when_parser_rejects() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_rdch","pc":256,"channel":29,"value":null,"would_stall":true}
{"seq":2,"side":"spu","kind":"spu_park","pc":256,"reason":"channel_read","channel":29}
{"seq":1,"side":"spu","kind":"spu_wake","pc":256}
"#;
        // Idiomatic pipeline integration:
        //   let events = parse_jsonl_trace(s)?;
        //   let trace = captured_events_to_trace(&events)?;
        // If parse returns Err, the `?` short-circuits and trace is
        // never bound. We exercise that behavior here without `?` so
        // we can assert on it explicitly.
        let parse_result = parse_jsonl_trace(jsonl);
        assert!(parse_result.is_err(), "parser must surface error first");

        // Even if a confused caller hand-fabricates a `Vec<CapturedEvent>`
        // bypassing the parser, the transformer enforces its own
        // invariants — but the contract is: in the pipeline,
        // `captured_events_to_trace` is only called when parse succeeded.
        // Confirm transformer is callable independently with valid input
        // (this documents that the boundary is the parser, not the
        // transformer, and the transformer is not a re-validator):
        let valid_jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1}
{"seq":1,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}}
"#;
        let events = parse_jsonl_trace(valid_jsonl).expect("valid trace parses");
        let trace = captured_events_to_trace(&events).expect("valid events transform");
        assert!(!trace.is_empty());
    }

    // ---------------------------------------------------------------
    // R5.9b transformer per-SPU contracts (2026-04-28)
    //
    // Validates that:
    //  1. captured_events_to_traces_per_spu groups events by target_spu
    //     into a deterministic BTreeMap, with each group transformed as
    //     a single-SPU subsequence.
    //  2. The legacy single-SPU API (captured_events_to_trace) still
    //     reproduces the canonical R5_6_REFERENCE_JSONL output via the
    //     wrapper.
    //  3. The legacy single-SPU API REJECTS multi-SPU traces with
    //     MultipleSpusUnsupportedBySingleSpuApi rather than silently
    //     flattening them — this is the load-bearing safety guarantee
    //     for callers that haven't migrated to the per-SPU API.
    //  4. PPU events targeting SPU N appear ONLY in SPU N's group, not
    //     in any other SPU's group.
    //  5. Within each per-SPU group, the relative order of TraceEvents
    //     follows the relative order of source CapturedEvents filtered
    //     to that SPU.
    // ---------------------------------------------------------------

    /// Two SPUs with minimal but complete (stop+final_state) timelines,
    /// interleaved in source order. The transformer produces a 2-key
    /// BTreeMap and each value is the SPU's expected R5.5 trace.
    #[test]
    fn transformer_per_spu_splits_two_spus() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":1}
{"seq":1,"side":"spu","kind":"spu_stop","pc":256,"stop_code":2,"target_spu":2}
{"seq":2,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
{"seq":3,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":2}
"#;
        let events = parse_jsonl_trace(jsonl).expect("parse must succeed");
        let groups = captured_events_to_traces_per_spu(&events)
            .expect("per-SPU transform must succeed");

        assert_eq!(groups.len(), 2, "expected 2 SPU groups, got {}", groups.len());
        assert!(groups.contains_key(&1));
        assert!(groups.contains_key(&2));

        // Each group: ExpectSpuFinished{stop_code} + ExpectChannelState
        // (final_state emits the channel state assertion).
        let spu1 = &groups[&1];
        let spu2 = &groups[&2];
        assert_eq!(spu1.len(), 2);
        assert_eq!(spu2.len(), 2);
        match &spu1[0] {
            TraceEvent::ExpectSpuFinished { stop_code: 1 } => {}
            other => panic!("spu1[0] should be ExpectSpuFinished {{1}}, got {other:?}"),
        }
        match &spu2[0] {
            TraceEvent::ExpectSpuFinished { stop_code: 2 } => {}
            other => panic!("spu2[0] should be ExpectSpuFinished {{2}}, got {other:?}"),
        }
    }

    /// The R5.9b per-SPU API on the legacy R5_6_REFERENCE_JSONL fixture
    /// MUST yield exactly one group (keyed `0` via the default-zero
    /// shim) and that group's `Vec<TraceEvent>` MUST match the legacy
    /// single-SPU API output byte-exact. This is the regression guard
    /// that the per-SPU API doesn't break the round-trip equivalence
    /// the canonical R5.6 trace establishes.
    #[test]
    fn per_spu_api_preserves_legacy_reference_jsonl_under_target_spu_zero() {
        let events = parse_jsonl_trace(R5_6_REFERENCE_JSONL).expect("parse must succeed");
        let groups = captured_events_to_traces_per_spu(&events)
            .expect("per-SPU transform must succeed");

        assert_eq!(groups.len(), 1, "single-SPU fixture must produce 1 group");
        assert!(groups.contains_key(&0), "legacy fixture defaults to target_spu 0");

        let per_spu_trace = &groups[&0];
        let legacy_trace = captured_events_to_trace(&events)
            .expect("legacy single-SPU API must still succeed");

        assert_eq!(
            per_spu_trace.len(),
            legacy_trace.len(),
            "per-SPU trace length must match legacy trace length"
        );
        for (i, (a, b)) in per_spu_trace.iter().zip(legacy_trace.iter()).enumerate() {
            assert_eq!(
                format!("{a:?}"),
                format!("{b:?}"),
                "per-SPU group[0][{i}] must equal legacy single-SPU trace[{i}]"
            );
        }
    }

    /// The legacy single-SPU API MUST refuse multi-SPU traces rather
    /// than silently flatten them. Parser accepts the input (R5.9a
    /// validates per-SPU termination), so the failure mode lands at
    /// the transformer's wrapper layer.
    #[test]
    fn single_spu_api_rejects_multi_spu_trace() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":1}
{"seq":1,"side":"spu","kind":"spu_stop","pc":256,"stop_code":2,"target_spu":2}
{"seq":2,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
{"seq":3,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":2}
"#;
        let events = parse_jsonl_trace(jsonl).expect("parser accepts multi-SPU under R5.9a");
        let err = captured_events_to_trace(&events)
            .expect_err("single-SPU API must refuse multi-SPU traces");
        match err {
            TraceTransformError::MultipleSpusUnsupportedBySingleSpuApi { spu_count } => {
                assert_eq!(spu_count, 2, "expected 2 SPUs, got {spu_count}");
            }
            other => panic!("expected MultipleSpusUnsupportedBySingleSpuApi, got {other:?}"),
        }
    }

    /// PPU event targeting SPU 1 MUST appear in SPU 1's per-SPU trace
    /// and MUST NOT appear in SPU 2's per-SPU trace. The grouping is
    /// strict: every event belongs to exactly one SPU's timeline.
    #[test]
    fn per_spu_transformer_does_not_mix_ppu_events() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_park","pc":256,"reason":"channel_read","channel":29,"target_spu":1}
{"seq":1,"side":"ppu","kind":"ppu_push_inmbox","target_spu":1,"value":42}
{"seq":2,"side":"spu","kind":"spu_wake","pc":256,"target_spu":1}
{"seq":3,"side":"spu","kind":"spu_stop","pc":260,"stop_code":1,"target_spu":1}
{"seq":4,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
{"seq":5,"side":"spu","kind":"spu_stop","pc":256,"stop_code":2,"target_spu":2}
{"seq":6,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":2}
"#;
        let events = parse_jsonl_trace(jsonl).expect("parse must succeed");
        let groups = captured_events_to_traces_per_spu(&events)
            .expect("per-SPU transform must succeed");

        assert_eq!(groups.len(), 2);
        let spu1 = &groups[&1];
        let spu2 = &groups[&2];

        // SPU 1 timeline includes the PPU push (between park and wake).
        let spu1_has_push = spu1
            .iter()
            .any(|e| matches!(e, TraceEvent::PpuPushInMbox { value: 42, .. }));
        assert!(spu1_has_push, "SPU 1 timeline must contain the ppu_push_inmbox(value=42)");

        // SPU 2 timeline must NOT contain ANY PpuPushInMbox event —
        // the only push in the trace targets SPU 1.
        let spu2_has_any_push = spu2
            .iter()
            .any(|e| matches!(e, TraceEvent::PpuPushInMbox { .. }));
        assert!(
            !spu2_has_any_push,
            "SPU 2 timeline must NOT contain any PpuPushInMbox; it targets SPU 1 only"
        );
    }

    // ---------------------------------------------------------------
    // R5.9e.2 parser support for `spu_image` metadata (2026-04-28)
    //
    // Validates that:
    //   1. A well-formed `spu_image` event parses into the matching
    //      CapturedEvent::SpuImage variant carrying every field
    //      verbatim (no shim, no defaults).
    //   2. Bad metadata (hash format, size out of range, alignment,
    //      load_addr+size overflow, entry_pc out of LS) is rejected
    //      with a specific error variant — never silently accepted,
    //      never swallowed by serde.
    //   3. Per-SPU rules are enforced post-parse: at most one
    //      `spu_image` per target_spu (DuplicateSpuImage); the image
    //      must precede the SPU's first executed event
    //      (ImageEventOutOfOrder); PPU-side events for the same target
    //      do NOT count as "executed" (so the PPU can act on the SPU
    //      before the SPU starts running, which is real captures).
    //   4. DMA-dispatching `spu_wrch` to MFC_Cmd (channel 21) is
    //      rejected with UnsupportedDmaInTrace — replay engine cannot
    //      handle DMA endpoints, so reject at parse time so the
    //      failure mode is sharp.
    //   5. Legacy traces without `spu_image` events still parse
    //      cleanly. R5_6_REFERENCE_JSONL and the synthetic
    //      single-SPU/multi-SPU contract tests must NOT regress.
    //   6. Transformer ignores `spu_image` events — they are
    //      metadata-only, not state-machine context, not emitted as
    //      TraceEvents.
    // ---------------------------------------------------------------

    /// Helper: a minimal valid spu_image JSONL line with the given hash.
    fn make_image_line(seq: u64, target_spu: u32, hash: &str) -> String {
        format!(
            r#"{{"seq":{seq},"side":"spu","kind":"spu_image","target_spu":{target_spu},"image_sha256":"{hash}","load_addr":0,"size":4096,"entry_pc":0}}"#
        )
    }

    /// Well-formed spu_image event roundtrips into a SpuImageEvent
    /// with every field intact.
    #[test]
    fn parser_accepts_valid_spu_image_metadata() {
        let hash = "0123456789abcdef".repeat(4);
        let jsonl = make_image_line(0, 1, &hash);
        let events = parse_jsonl_trace(&jsonl).expect("valid spu_image must parse");
        assert_eq!(events.len(), 1);
        match &events[0] {
            CapturedEvent::SpuImage(e) => {
                assert_eq!(e.seq, 0);
                assert_eq!(e.side, CapturedSide::Spu);
                assert_eq!(e.target_spu, 1);
                assert_eq!(e.image_sha256, hash);
                assert_eq!(e.load_addr, 0);
                assert_eq!(e.size, 4096);
                assert_eq!(e.entry_pc, 0);
            }
            other => panic!("expected SpuImage, got {other:?}"),
        }
        assert_eq!(events[0].target_spu(), 1);
        assert_eq!(events[0].kind_label(), "spu_image");
    }

    /// Bad image_sha256: wrong length OR uppercase OR non-hex chars
    /// must each be rejected with BadImageHash. We exercise three
    /// failure modes in one test to lock the rule explicitly.
    #[test]
    fn parser_rejects_bad_image_hash() {
        // Too short.
        let short = make_image_line(0, 1, "abc");
        let err = parse_jsonl_trace(&short).expect_err("short hash must reject");
        assert!(matches!(err, TraceParseError::BadImageHash { target_spu: 1, .. }), "got {err:?}");

        // Uppercase (the schema requires lowercase).
        let upper_hash = "ABCDEF0123456789".repeat(4);
        let upper = make_image_line(0, 1, &upper_hash);
        let err = parse_jsonl_trace(&upper).expect_err("uppercase hash must reject");
        assert!(matches!(err, TraceParseError::BadImageHash { target_spu: 1, .. }), "got {err:?}");

        // Non-hex char inside an otherwise-64-char string.
        let mut bad = "0123456789abcdef".repeat(4);
        // Replace one char with a non-hex letter.
        bad.replace_range(0..1, "z");
        let nonhex = make_image_line(0, 1, &bad);
        let err = parse_jsonl_trace(&nonhex).expect_err("non-hex char must reject");
        assert!(matches!(err, TraceParseError::BadImageHash { target_spu: 1, .. }), "got {err:?}");
    }

    /// Bad image size: zero, oversized (>256 KB), unaligned (% 4 != 0).
    #[test]
    fn parser_rejects_bad_image_size() {
        let hash = "0".repeat(64);
        // Zero size.
        let zero = format!(
            r#"{{"seq":0,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash}","load_addr":0,"size":0,"entry_pc":0}}"#
        );
        let err = parse_jsonl_trace(&zero).expect_err("size=0 must reject");
        assert!(matches!(err, TraceParseError::BadImageSize { size: 0, .. }), "got {err:?}");

        // Oversized (256 KB + 4 bytes).
        let over = format!(
            r#"{{"seq":0,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash}","load_addr":0,"size":262148,"entry_pc":0}}"#
        );
        let err = parse_jsonl_trace(&over).expect_err("size > 256 KB must reject");
        assert!(matches!(err, TraceParseError::BadImageSize { size: 262148, .. }), "got {err:?}");

        // Unaligned size (not a multiple of 4).
        let unaligned = format!(
            r#"{{"seq":0,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash}","load_addr":0,"size":4097,"entry_pc":0}}"#
        );
        let err = parse_jsonl_trace(&unaligned).expect_err("size not 4-aligned must reject");
        assert!(matches!(err, TraceParseError::BadImageSize { size: 4097, .. }), "got {err:?}");
    }

    /// load_addr + size must not overflow u32 nor exceed the 256 KB LS;
    /// load_addr must itself be 4-byte aligned.
    #[test]
    fn parser_rejects_bad_image_load_addr() {
        let hash = "0".repeat(64);
        // Unaligned load_addr.
        let unaligned = format!(
            r#"{{"seq":0,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash}","load_addr":1,"size":4096,"entry_pc":0}}"#
        );
        let err = parse_jsonl_trace(&unaligned).expect_err("unaligned load_addr must reject");
        assert!(matches!(err, TraceParseError::BadImageLoadAddr { load_addr: 1, .. }), "got {err:?}");

        // load_addr + size overflows the LS (load_addr=0x3FFFC, size=0x10).
        let over_ls = format!(
            r#"{{"seq":0,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash}","load_addr":262140,"size":16,"entry_pc":0}}"#
        );
        let err = parse_jsonl_trace(&over_ls).expect_err("load_addr+size > LS must reject");
        assert!(matches!(err, TraceParseError::BadImageLoadAddr { load_addr: 262140, .. }), "got {err:?}");
    }

    /// `spu_image` after at least one SPU-executed event for the same
    /// target_spu must be rejected. PPU-side events for the same target
    /// do NOT count as "executed" (PPU can push to a not-yet-running
    /// SPU's mailbox).
    #[test]
    fn parser_rejects_image_out_of_order() {
        let hash = "0".repeat(64);
        let jsonl = format!(r#"
{{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash}","load_addr":0,"size":4096,"entry_pc":0}}
"#);
        let err = parse_jsonl_trace(&jsonl)
            .expect_err("spu_image after executed event must reject");
        match err {
            TraceParseError::ImageEventOutOfOrder { target_spu, image_index, first_event_index } => {
                assert_eq!(target_spu, 1);
                assert_eq!(image_index, 1);
                assert_eq!(first_event_index, 0);
            }
            other => panic!("expected ImageEventOutOfOrder, got {other:?}"),
        }
    }

    /// Two `spu_image` events for the same target_spu must be rejected.
    /// (Different target_spus would be fine — distinct SPUs may share
    /// or differ in image.)
    #[test]
    fn parser_rejects_duplicate_spu_image() {
        let hash_a = "0".repeat(64);
        let hash_b = "1".repeat(64);
        let jsonl = format!(r#"
{{"seq":0,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash_a}","load_addr":0,"size":4096,"entry_pc":0}}
{{"seq":1,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash_b}","load_addr":0,"size":4096,"entry_pc":0}}
"#);
        let err = parse_jsonl_trace(&jsonl)
            .expect_err("duplicate spu_image for same target must reject");
        match err {
            TraceParseError::DuplicateSpuImage { target_spu, first_index, second_index } => {
                assert_eq!(target_spu, 1);
                assert_eq!(first_index, 0);
                assert_eq!(second_index, 1);
            }
            other => panic!("expected DuplicateSpuImage, got {other:?}"),
        }
    }

    /// `spu_wrch` to MFC_Cmd (channel 21) dispatches a DMA. R5.9e
    /// rejects such traces at parse time because DMA endpoints aren't
    /// captured. Surface UnsupportedDmaInTrace with target_spu and
    /// event_index pointing to the offending wrch.
    #[test]
    fn parser_rejects_dma_channel_trace_as_unsupported() {
        // spu_wrch on channel 21 (MFC_Cmd). pc + value are arbitrary.
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":1,"would_stall":false,"target_spu":1}
"#;
        let err = parse_jsonl_trace(jsonl)
            .expect_err("spu_wrch to MFC_Cmd must reject (DMA dispatch)");
        match err {
            TraceParseError::UnsupportedDmaInTrace { target_spu, event_index, channel } => {
                assert_eq!(target_spu, 1);
                assert_eq!(event_index, 0);
                assert_eq!(channel, 21);
            }
            other => panic!("expected UnsupportedDmaInTrace, got {other:?}"),
        }
    }

    /// SMC indicator detection note: per the R5.9e plan § A.5 / D.2,
    /// reliable single-channel SMC detection is not possible without
    /// observing the full MFC_LSA + MFC_Size + MFC_Cmd sequence. SMC
    /// is a strict subset of DMA, so the DMA gate already rejects
    /// SMC-bearing traces. This test documents that contract: a
    /// `spu_wrch` to MFC_RdAtomicStat (channel 27 — a READ-side
    /// channel that an SMC-aware writer would write to in a
    /// hypothetical future iteration) is currently NOT rejected as
    /// SMC; the trace would be rejected only when the corresponding
    /// `MFC_Cmd` write surfaces. R5.9e.2 deliberately implements only
    /// the DMA gate; SMC-specific detection is deferred to R5.9f.
    #[test]
    fn parser_does_not_yet_detect_smc_directly() {
        // spu_wrch on channel 27 (MFC_RdAtomicStat) — NOT MFC_Cmd, so
        // the current DMA gate doesn't fire. The trace parses (and the
        // event itself is well-formed). Future SMC-specific detection
        // would re-evaluate this; locking the current behavior here so
        // any future change to it is intentional.
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":27,"value":1,"would_stall":false,"target_spu":1}
"#;
        let events = parse_jsonl_trace(jsonl)
            .expect("non-MFC_Cmd channel does not (yet) trigger UnsupportedDmaInTrace");
        assert_eq!(events.len(), 1);
        // No assertions on SMC detection — by design for R5.9e.2.
    }

    /// `R5_6_REFERENCE_JSONL` has no `spu_image` event and must keep
    /// parsing cleanly. R5.9e.2 must NOT make `spu_image` mandatory.
    #[test]
    fn legacy_reference_jsonl_still_parses_without_spu_image() {
        let events = parse_jsonl_trace(R5_6_REFERENCE_JSONL).expect("R5.6 reference must still parse");
        assert_eq!(events.len(), 24, "R5.6 reference has 24 events");
        // None of those 24 are SpuImage.
        for ev in &events {
            assert!(!matches!(ev, CapturedEvent::SpuImage(_)),
                "R5_6_REFERENCE_JSONL must not contain spu_image events");
        }
    }

    /// The transformer must ignore `spu_image` events: it does not
    /// emit any TraceEvent for them and does not change the SPU state
    /// machine. A trace with one `spu_image` followed by a complete
    /// (stop+final_state) timeline transforms identically to the
    /// no-image version.
    #[test]
    fn transformer_ignores_spu_image_metadata_until_replay_stage() {
        let hash = "0".repeat(64);
        let with_image = format!(r#"
{{"seq":0,"side":"spu","kind":"spu_image","target_spu":1,"image_sha256":"{hash}","load_addr":0,"size":4096,"entry_pc":0}}
{{"seq":1,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":1}}
{{"seq":2,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}},"target_spu":1}}
"#);
        let without_image = r#"
{"seq":0,"side":"spu","kind":"spu_stop","pc":256,"stop_code":1,"target_spu":1}
{"seq":1,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
"#;

        let events_with = parse_jsonl_trace(&with_image).expect("with-image parses");
        let events_without = parse_jsonl_trace(without_image).expect("without-image parses");

        let trace_with = captured_events_to_trace(&events_with).expect("transform with-image");
        let trace_without = captured_events_to_trace(&events_without).expect("transform without");

        assert_eq!(trace_with.len(), trace_without.len(),
            "spu_image must NOT alter the TraceEvent count");
        for (i, (a, b)) in trace_with.iter().zip(trace_without.iter()).enumerate() {
            assert_eq!(format!("{a:?}"), format!("{b:?}"),
                "trace_with[{i}] must match trace_without[{i}] (image is metadata-only)");
        }

        // Per-SPU API: 1 group keyed 1, with the same Vec content.
        let groups = captured_events_to_traces_per_spu(&events_with).expect("per-SPU transform with image");
        assert_eq!(groups.len(), 1);
        assert!(groups.contains_key(&1));
    }

    /// Within a single SPU's per-SPU trace, the relative ordering of
    /// TraceEvents must follow the relative ordering of the source
    /// CapturedEvents filtered to that SPU. We verify this via two
    /// SPUs whose parks happen at distinct PCs in interleaved source
    /// order; each SPU's first emitted ExpectSpuPark must reference
    /// its own PC, not the other SPU's.
    #[test]
    fn per_spu_transformer_preserves_event_order_within_spu() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_park","pc":256,"reason":"channel_read","channel":29,"target_spu":1}
{"seq":1,"side":"spu","kind":"spu_park","pc":512,"reason":"channel_write","channel":28,"target_spu":2}
{"seq":2,"side":"ppu","kind":"ppu_push_inmbox","target_spu":1,"value":1}
{"seq":3,"side":"spu","kind":"spu_wake","pc":256,"target_spu":1}
{"seq":4,"side":"ppu","kind":"ppu_pop_outmbox","target_spu":2,"value":42}
{"seq":5,"side":"spu","kind":"spu_wake","pc":512,"target_spu":2}
{"seq":6,"side":"spu","kind":"spu_stop","pc":260,"stop_code":1,"target_spu":1}
{"seq":7,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":1}
{"seq":8,"side":"spu","kind":"spu_stop","pc":516,"stop_code":2,"target_spu":2}
{"seq":9,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0},"target_spu":2}
"#;
        let events = parse_jsonl_trace(jsonl).expect("parse must succeed");
        let groups = captured_events_to_traces_per_spu(&events)
            .expect("per-SPU transform must succeed");

        let spu1 = &groups[&1];
        let spu2 = &groups[&2];

        // SPU 1's first event is its own park (pc=256, channel_read=29),
        // NOT SPU 2's park (pc=512, channel_write=28).
        match &spu1[0] {
            TraceEvent::ExpectSpuPark { pc: Some(256), .. } => {}
            other => panic!("spu1[0] must be its own park at pc=256, got {other:?}"),
        }
        match &spu2[0] {
            TraceEvent::ExpectSpuPark { pc: Some(512), .. } => {}
            other => panic!("spu2[0] must be its own park at pc=512, got {other:?}"),
        }

        // Each SPU's PPU action follows its own park (Ready wake) and
        // precedes its own ExpectSpuFinished — i.e., the relative order
        // is preserved per-SPU even when the source stream interleaves.
        let spu1_push_idx = spu1
            .iter()
            .position(|e| matches!(e, TraceEvent::PpuPushInMbox { .. }))
            .expect("spu1 must contain its push");
        let spu1_finish_idx = spu1
            .iter()
            .position(|e| matches!(e, TraceEvent::ExpectSpuFinished { .. }))
            .expect("spu1 must contain its finish");
        assert!(
            spu1_push_idx < spu1_finish_idx,
            "spu1 push must precede finish (got push@{spu1_push_idx}, finish@{spu1_finish_idx})"
        );

        let spu2_pop_idx = spu2
            .iter()
            .position(|e| matches!(e, TraceEvent::PpuPopOutMbox { .. }))
            .expect("spu2 must contain its pop");
        let spu2_finish_idx = spu2
            .iter()
            .position(|e| matches!(e, TraceEvent::ExpectSpuFinished { .. }))
            .expect("spu2 must contain its finish");
        assert!(
            spu2_pop_idx < spu2_finish_idx,
            "spu2 pop must precede finish (got pop@{spu2_pop_idx}, finish@{spu2_finish_idx})"
        );
    }

    // =================================================================
    // R6.7 A.2 — DMA / MFC parser tests.
    //
    // Scope: parser-only. The transformer still rejects MFC events
    // with TraceTransformError::UnsupportedDmaInTrace until A.4 lands
    // the replay state machine. None of these tests load `.dmachunk`
    // side-files (A.3 scope) nor copy DMA bytes into LS (A.4 scope).
    //
    // Invariants under test:
    //   1. Well-formed `spu_wrch ch21 → spu_mfc_cmd` pair parses.
    //   2. `mfc_dma_complete` parses; transferred_bytes validated.
    //   3. A minimal MFC GET sequence (writer-emitted shape) parses
    //      end-to-end, including ch16-23 wrches, ch21 → spu_mfc_cmd
    //      → mfc_dma_complete → ch24 rdch.
    //   4. Non-GET cmd codes are rejected (UnsupportedMfcCmd).
    //   5. eah != 0 is rejected (UnsupportedMfcEah).
    //   6. Bad size (zero, oversized, bad alignment) is rejected
    //      (BadDmaSize).
    //   7. Bad lsa (alignment, lsa+size > LS) is rejected (BadDmaLsa).
    //   8. Bad sha (length, casing, non-hex) is rejected (BadDmaSha).
    //   9. Bad tag (>= 32) is rejected (BadMfcTag).
    //  10. Transformer rejects MFC traces with UnsupportedDmaInTrace.
    //  11. The 6 existing replay oracles stay green (no DMA in them;
    //      the gate is a no-op for non-DMA traces). Verified by the
    //      separate replay tests in `rpcs3-spu-recompiler` which run
    //      against the captured fixtures. Here we only re-confirm the
    //      reference R5.6 trace still parses + transforms.
    // =================================================================

    /// 64-char lowercase-hex SHA used by tests that need a well-formed
    /// `ea_chunk_sha256`. The actual content the SHA "addresses" doesn't
    /// matter at parse time — `.dmachunk` loading is A.3.
    const TEST_VALID_SHA: &str =
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn make_mfc_get_pair_jsonl(target_spu: u32, tag: u32, size: u32, lsa: u32, eal: u32) -> String {
        format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":{target_spu}}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":{target_spu},"pc":256,"cmd":64,"tag":{tag},"size":{size},"lsa":{lsa},"eah":0,"eal":{eal},"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
"#
        )
    }

    /// Positive: a well-formed `spu_wrch ch21 → spu_mfc_cmd` pair for
    /// a GET (cmd=0x40, eah=0, size=128 (16-byte aligned), lsa aligned,
    /// tag in range, sha well-formed) parses cleanly.
    #[test]
    fn parse_spu_mfc_cmd_get_event() {
        let jsonl = make_mfc_get_pair_jsonl(1, 3, 128, 0x3FF00, 0xD0010000);
        let events =
            parse_jsonl_trace(&jsonl).expect("well-formed MFC GET pair must parse");
        assert_eq!(events.len(), 2);
        match &events[1] {
            CapturedEvent::SpuMfcCmd(m) => {
                assert_eq!(m.target_spu, 1);
                assert_eq!(m.cmd, 0x40);
                assert_eq!(m.tag, 3);
                assert_eq!(m.size, 128);
                assert_eq!(m.lsa, 0x3FF00);
                assert_eq!(m.eah, 0);
                assert_eq!(m.eal, 0xD0010000);
                assert_eq!(m.ea_chunk_sha256, TEST_VALID_SHA);
            }
            other => panic!("expected SpuMfcCmd, got {other:?}"),
        }
        assert_eq!(events[1].kind_label(), "spu_mfc_cmd");
        assert_eq!(events[1].target_spu(), 1);
    }

    /// Positive: a well-formed `mfc_dma_complete` event parses. We
    /// stage it after a matching ch21+spu_mfc_cmd pair so the post-pass
    /// ordering invariants are satisfied.
    #[test]
    fn parse_mfc_dma_complete_event() {
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
{{"seq":2,"side":"spu","kind":"mfc_dma_complete","target_spu":1,"tag":3,"transferred_bytes":128}}
"#
        );
        let events = parse_jsonl_trace(&jsonl)
            .expect("well-formed mfc_dma_complete must parse");
        assert_eq!(events.len(), 3);
        match &events[2] {
            CapturedEvent::MfcDmaComplete(c) => {
                assert_eq!(c.target_spu, 1);
                assert_eq!(c.tag, 3);
                assert_eq!(c.transferred_bytes, 128);
            }
            other => panic!("expected MfcDmaComplete, got {other:?}"),
        }
        assert_eq!(events[2].kind_label(), "mfc_dma_complete");
    }

    /// Positive: the minimal writer-emitted MFC GET sequence parses.
    /// Mirrors what an R6.7 A.1 capture produces:
    ///   ch16 LSA → ch17 EAH → ch18 EAL → ch19 Size → ch20 TagID →
    ///   ch22 WrTagMask → ch23 WrTagUpdate → ch21 MFC_Cmd →
    ///   spu_mfc_cmd → mfc_dma_complete → ch24 RdTagStat → spu_stop.
    /// `ea_chunk_sha256` is well-formed but unused at parse time.
    #[test]
    fn parse_minimal_mfc_get_sequence_events() {
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":16,"value":0,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_wrch","pc":260,"channel":17,"value":0,"would_stall":false,"target_spu":1}}
{{"seq":2,"side":"spu","kind":"spu_wrch","pc":264,"channel":18,"value":4096,"would_stall":false,"target_spu":1}}
{{"seq":3,"side":"spu","kind":"spu_wrch","pc":268,"channel":19,"value":128,"would_stall":false,"target_spu":1}}
{{"seq":4,"side":"spu","kind":"spu_wrch","pc":272,"channel":20,"value":3,"would_stall":false,"target_spu":1}}
{{"seq":5,"side":"spu","kind":"spu_wrch","pc":276,"channel":22,"value":8,"would_stall":false,"target_spu":1}}
{{"seq":6,"side":"spu","kind":"spu_wrch","pc":280,"channel":23,"value":2,"would_stall":false,"target_spu":1}}
{{"seq":7,"side":"spu","kind":"spu_wrch","pc":284,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":8,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":284,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
{{"seq":9,"side":"spu","kind":"mfc_dma_complete","target_spu":1,"tag":3,"transferred_bytes":128}}
{{"seq":10,"side":"spu","kind":"spu_rdch","pc":288,"channel":24,"value":8,"would_stall":false,"target_spu":1}}
{{"seq":11,"side":"spu","kind":"spu_stop","pc":292,"stop_code":1,"target_spu":1}}
"#
        );
        let events = parse_jsonl_trace(&jsonl)
            .expect("minimal MFC GET sequence must parse");
        assert_eq!(events.len(), 12);
        // Spot-check the structure we depend on for A.4 replay design.
        assert!(matches!(events[7], CapturedEvent::SpuWrch(ref w) if w.channel == 21));
        assert!(matches!(events[8], CapturedEvent::SpuMfcCmd(_)));
        assert!(matches!(events[9], CapturedEvent::MfcDmaComplete(_)));
        assert!(matches!(events[10], CapturedEvent::SpuRdch(ref r) if r.channel == 24));
    }

    /// Negative: cmd != 0x40 and != 0x20 is rejected at parse time.
    /// R8.4a update: list-DMA codes (PUTL/PUTLB/PUTLF/GETL/GETLB/
    /// GETLF) now surface the granular `UnsupportedMfcListCmd`
    /// variant. Atomic / barrier / sync codes still surface the
    /// generic `UnsupportedMfcCmd`. The canary moves to GETLLAR
    /// (0xD0, atomic getllar) — definitely out of scope and not
    /// in any current R8.4 roadmap.
    #[test]
    fn reject_mfc_cmd_non_get_non_put_non_list() {
        // Canary: GETLLAR (0xD0, atomic reservation). Distinct from
        // both R6.7/R8.1 supported (0x40, 0x20) and R8.4 list-DMA
        // (0x24..0x26, 0x44..0x46) codes.
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":208,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":208,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
"#
        );
        let err = parse_jsonl_trace(&jsonl).expect_err("GETLLAR cmd must reject");
        assert!(
            matches!(err, TraceParseError::UnsupportedMfcCmd { cmd: 0xD0, .. }),
            "got {err:?}"
        );
    }

    /// R8.4a + R8.4c — list-DMA codes NOT YET supported
    /// (PUTL/PUTLB/PUTLF = 0x24/0x25/0x26, GETLB/GETLF =
    /// 0x45/0x46) surface the granular `UnsupportedMfcListCmd`
    /// variant. R8.4c LIFTED the canary for GETL (0x44) only —
    /// covered separately by `r8_4c_getl_parses_and_validates`.
    #[test]
    fn reject_mfc_list_cmds_with_granular_canary() {
        // R8.4c update: 0x44 GETL removed from this iterator
        // (now accepted by the parser; covered by
        // `r8_4c_getl_parses_and_validates`).
        let list_cmds: &[u32] = &[0x24, 0x25, 0x26, 0x45, 0x46];
        for &cmd in list_cmds {
            let jsonl = format!(
                r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":{cmd},"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":{cmd},"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
"#
            );
            let err = parse_jsonl_trace(&jsonl)
                .expect_err(&format!("list cmd 0x{cmd:x} must reject"));
            match err {
                TraceParseError::UnsupportedMfcListCmd { cmd: got_cmd, .. } => {
                    assert_eq!(got_cmd, cmd, "list-cmd canary preserves the cmd code");
                }
                other => panic!(
                    "list cmd 0x{cmd:x} should be UnsupportedMfcListCmd, got {other:?}"
                ),
            }
        }
    }

    /// R8.4c — when the writer emits a real GETL `spu_mfc_cmd`
    /// event (with all the additive descriptor / element fields),
    /// the parser MUST now ACCEPT it (R8.4a canary lifted for
    /// 0x44 only). The replay state machine consumes the
    /// additive fields; the transformer treats it as DMA
    /// context (same path as simple GET/PUT post-R6.7 C.5).
    #[test]
    fn r8_4c_getl_parses_and_validates() {
        let desc_sha = "11".repeat(32);
        let elem1_sha = "22".repeat(32);
        let elem2_sha = "33".repeat(32);
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":68,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":68,"tag":3,"size":16,"lsa":65536,"eah":0,"eal":384,"ea_chunk_sha256":"{desc_sha}","descriptor_sha256":"{desc_sha}","descriptor_size":16,"element_chunks":["{elem1_sha}","{elem2_sha}"],"element_sizes":[128,64],"element_eals":[268505472,268505600]}}
"#
        );
        let events = parse_jsonl_trace(&jsonl)
            .expect("R8.4c: GETL with valid additive fields must parse");
        // Should be 2 events (wrch + spu_mfc_cmd).
        assert_eq!(events.len(), 2);
        let mfc = match &events[1] {
            CapturedEvent::SpuMfcCmd(m) => m,
            other => panic!("expected SpuMfcCmd, got {other:?}"),
        };
        assert_eq!(mfc.cmd, 0x44);
        assert_eq!(mfc.descriptor_size, Some(16));
        assert_eq!(mfc.element_chunks.as_ref().unwrap().len(), 2);
        assert_eq!(mfc.element_sizes.as_deref(), Some(&[128u32, 64u32][..]));
    }

    /// R8.4c — GETL with missing additive fields must reject.
    #[test]
    fn r8_4c_getl_rejects_missing_descriptor_fields() {
        let desc_sha = "11".repeat(32);
        // descriptor_sha256 present but descriptor_size missing
        // → must reject.
        let jsonl_missing_size = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":68,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":68,"tag":3,"size":16,"lsa":65536,"eah":0,"eal":384,"ea_chunk_sha256":"{desc_sha}","descriptor_sha256":"{desc_sha}"}}
"#
        );
        let err = parse_jsonl_trace(&jsonl_missing_size)
            .expect_err("GETL with missing descriptor_size must reject");
        assert!(matches!(err, TraceParseError::UnsupportedMfcCmd { cmd: 0x44, .. }), "got {err:?}");
    }

    /// R8.4c — GETL with descriptor_size != e.size must reject.
    #[test]
    fn r8_4c_getl_rejects_inconsistent_descriptor_size() {
        let desc_sha = "11".repeat(32);
        let elem_sha = "22".repeat(32);
        // size=16 but descriptor_size=24 → inconsistent.
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":68,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":68,"tag":3,"size":16,"lsa":65536,"eah":0,"eal":384,"ea_chunk_sha256":"{desc_sha}","descriptor_sha256":"{desc_sha}","descriptor_size":24,"element_chunks":["{elem_sha}","{elem_sha}","{elem_sha}"],"element_sizes":[8,8,8],"element_eals":[4096,8192,12288]}}
"#
        );
        let err = parse_jsonl_trace(&jsonl)
            .expect_err("GETL with inconsistent descriptor_size must reject");
        assert!(matches!(err, TraceParseError::BadDmaSize { size: 24, .. }), "got {err:?}");
    }

    /// R8.4c — GETL with element count mismatch must reject.
    #[test]
    fn r8_4c_getl_rejects_element_count_mismatch() {
        let desc_sha = "11".repeat(32);
        let elem_sha = "22".repeat(32);
        // size=16 → expect 2 elements, but only 1 in element_chunks
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":68,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":68,"tag":3,"size":16,"lsa":65536,"eah":0,"eal":384,"ea_chunk_sha256":"{desc_sha}","descriptor_sha256":"{desc_sha}","descriptor_size":16,"element_chunks":["{elem_sha}"],"element_sizes":[128],"element_eals":[4096]}}
"#
        );
        let err = parse_jsonl_trace(&jsonl)
            .expect_err("GETL with element count mismatch must reject");
        assert!(matches!(err, TraceParseError::MalformedMfcSequence { .. }), "got {err:?}");
    }

    /// R8.4c — non-list cmds (GET/PUT) with stray list fields
    /// MUST reject (writer-bug defense).
    #[test]
    fn r8_4c_simple_get_with_stray_list_fields_rejects() {
        let sha = TEST_VALID_SHA.to_string();
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{sha}","descriptor_sha256":"{sha}","descriptor_size":16}}
"#
        );
        let err = parse_jsonl_trace(&jsonl).expect_err("GET with stray list fields must reject");
        assert!(matches!(err, TraceParseError::UnsupportedMfcCmd { cmd: 0x40, .. }), "got {err:?}");
    }

    /// R8.4b — existing GET/PUT events (without the new
    /// additive fields) MUST continue to deserialize. The
    /// new `Option<>` fields must default to `None`. Regression
    /// guard against accidentally making the schema additive
    /// fields required.
    #[test]
    fn r8_4b_existing_get_put_traces_still_parse_with_none_list_fields() {
        let sha = TEST_VALID_SHA.to_string();
        // GET — simple cmd 0x40, no list fields in JSON.
        let get_line = format!(
            r#"{{"seq":7,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{sha}"}}"#
        );
        let parsed: CapturedEvent = serde_json::from_str(&get_line)
            .expect("R8.4b: existing GET event must still deserialize");
        if let CapturedEvent::SpuMfcCmd(m) = parsed {
            assert_eq!(m.cmd, 0x40);
            assert!(m.descriptor_sha256.is_none());
            assert!(m.descriptor_size.is_none());
            assert!(m.element_chunks.is_none());
            assert!(m.element_sizes.is_none());
            assert!(m.element_eals.is_none());
        } else {
            panic!("expected SpuMfcCmd");
        }

        // PUT — simple cmd 0x20, no list fields.
        let put_line = format!(
            r#"{{"seq":7,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":32,"tag":3,"size":128,"lsa":65536,"eah":0,"eal":4096,"ea_chunk_sha256":"{sha}"}}"#
        );
        let parsed: CapturedEvent = serde_json::from_str(&put_line)
            .expect("R8.4b: existing PUT event must still deserialize");
        if let CapturedEvent::SpuMfcCmd(m) = parsed {
            assert_eq!(m.cmd, 0x20);
            assert!(m.descriptor_sha256.is_none());
            assert!(m.element_chunks.is_none());
        } else {
            panic!("expected SpuMfcCmd");
        }
    }

    /// R8.4a — non-list, non-supported codes (atomic, barrier,
    /// sync, sndsig) take the generic `UnsupportedMfcCmd` path.
    /// Ensures the granular `UnsupportedMfcListCmd` doesn't
    /// over-broaden.
    #[test]
    fn reject_non_list_non_supported_mfc_cmds_with_generic_canary() {
        let non_list_cmds: &[(u32, &str)] = &[
            (0xD0, "GETLLAR"),
            (0xB4, "PUTLLC"),
            (0xB0, "PUTLLUC"),
            (0xB8, "PUTQLLUC"),
            (0xC0, "BARRIER"),
            (0xCC, "SYNC"),
            (0xA0, "SNDSIG"),
        ];
        for &(cmd, mnemonic) in non_list_cmds {
            let jsonl = format!(
                r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":{cmd},"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":{cmd},"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
"#
            );
            let err = parse_jsonl_trace(&jsonl)
                .expect_err(&format!("{mnemonic} (0x{cmd:x}) must reject"));
            match err {
                TraceParseError::UnsupportedMfcCmd { cmd: got_cmd, .. } => {
                    assert_eq!(got_cmd, cmd, "{mnemonic} preserved cmd code");
                }
                other => panic!(
                    "{mnemonic} (0x{cmd:x}) should be UnsupportedMfcCmd, got {other:?}"
                ),
            }
        }
    }

    /// R8.1 — PUT (0x20) is accepted by the parser as a sibling of
    /// GET. Both surface as `SpuMfcCmd` events; the state machine
    /// distinguishes their semantics (GET writes LS, PUT asserts).
    #[test]
    fn accept_mfc_cmd_put_0x20() {
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":32,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":32,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
{{"seq":2,"side":"spu","kind":"mfc_dma_complete","target_spu":1,"tag":3,"transferred_bytes":128}}
"#
        );
        let events = parse_jsonl_trace(&jsonl).expect("PUT must parse");
        assert_eq!(events.len(), 3);
        let cmd_evt = events.iter().find_map(|e| match e {
            CapturedEvent::SpuMfcCmd(c) => Some(c),
            _ => None,
        }).expect("spu_mfc_cmd must be present");
        assert_eq!(cmd_evt.cmd, 0x20, "cmd must be PUT");
    }

    /// Negative: eah != 0 is rejected. PS3 user-space PSL1GHT is
    /// 32-bit; eah is always 0 in scope. Real games using 64-bit lv2
    /// kernel-space surface here.
    #[test]
    fn reject_mfc_eah_nonzero() {
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":1,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
"#
        );
        let err = parse_jsonl_trace(&jsonl).expect_err("eah != 0 must reject");
        assert!(
            matches!(err, TraceParseError::UnsupportedMfcEah { eah: 1, .. }),
            "got {err:?}"
        );
    }

    /// Negative: size == 0 is rejected (BadDmaSize).
    #[test]
    fn reject_mfc_size_zero() {
        let jsonl = make_mfc_get_pair_jsonl(1, 3, 0, 0, 4096);
        let err = parse_jsonl_trace(&jsonl).expect_err("size=0 must reject");
        assert!(
            matches!(err, TraceParseError::BadDmaSize { size: 0, .. }),
            "got {err:?}"
        );
    }

    /// Negative: size > 0x4000 (16 KiB R6.7 cap) is rejected.
    #[test]
    fn reject_mfc_size_too_large() {
        // 0x4001 = 16385 — just above the cap; also not 16-aligned, but
        // the cap check fires first.
        let jsonl = make_mfc_get_pair_jsonl(1, 3, 0x4010, 0, 4096);
        let err = parse_jsonl_trace(&jsonl).expect_err("size > 0x4000 must reject");
        assert!(
            matches!(err, TraceParseError::BadDmaSize { size: 0x4010, .. }),
            "got {err:?}"
        );
    }

    /// Negative: size with bad alignment (e.g. 24 — not in {1,2,4,8}
    /// nor a multiple of 16) is rejected.
    #[test]
    fn reject_mfc_size_bad_alignment() {
        let jsonl = make_mfc_get_pair_jsonl(1, 3, 24, 0, 4096);
        let err = parse_jsonl_trace(&jsonl).expect_err("size=24 must reject");
        assert!(
            matches!(err, TraceParseError::BadDmaSize { size: 24, .. }),
            "got {err:?}"
        );
    }

    /// Negative: lsa + size exceeds the 256 KiB local store
    /// (BadDmaLsa). lsa = 0x3FFF0, size = 32 → end = 0x40010 > 0x40000.
    #[test]
    fn reject_mfc_lsa_oob() {
        let jsonl = make_mfc_get_pair_jsonl(1, 3, 32, 0x3FFF0, 4096);
        let err = parse_jsonl_trace(&jsonl).expect_err("lsa+size > LS must reject");
        assert!(
            matches!(err, TraceParseError::BadDmaLsa { lsa: 0x3FFF0, .. }),
            "got {err:?}"
        );
    }

    /// Negative: lsa misaligned for the dispatched size (e.g. size=128
    /// requires 16-byte alignment; lsa=8 violates this).
    #[test]
    fn reject_mfc_lsa_misaligned() {
        let jsonl = make_mfc_get_pair_jsonl(1, 3, 128, 8, 4096);
        let err = parse_jsonl_trace(&jsonl).expect_err("misaligned lsa must reject");
        assert!(
            matches!(err, TraceParseError::BadDmaLsa { lsa: 8, .. }),
            "got {err:?}"
        );
    }

    /// Negative: ea_chunk_sha256 with wrong length, uppercase, or
    /// non-hex chars is rejected. Mirrors the BadImageHash test for
    /// `.spuimg`.
    #[test]
    fn reject_bad_dma_sha() {
        // Too short.
        let short_sha = "0123";
        let short = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{short_sha}"}}
"#
        );
        let err = parse_jsonl_trace(&short).expect_err("short sha must reject");
        assert!(matches!(err, TraceParseError::BadDmaSha { target_spu: 1, .. }), "got {err:?}");

        // Uppercase.
        let upper_sha = "ABCDEF0123456789".repeat(4);
        let upper = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{upper_sha}"}}
"#
        );
        let err = parse_jsonl_trace(&upper).expect_err("uppercase sha must reject");
        assert!(matches!(err, TraceParseError::BadDmaSha { target_spu: 1, .. }), "got {err:?}");

        // Non-hex char.
        let mut bad = "0123456789abcdef".repeat(4);
        bad.replace_range(0..1, "z");
        let nonhex = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{bad}"}}
"#
        );
        let err = parse_jsonl_trace(&nonhex).expect_err("non-hex sha must reject");
        assert!(matches!(err, TraceParseError::BadDmaSha { target_spu: 1, .. }), "got {err:?}");
    }

    /// Negative: tag >= 32 is rejected on `spu_mfc_cmd` AND on
    /// `mfc_dma_complete` independently.
    #[test]
    fn reject_bad_mfc_tag() {
        let on_cmd = make_mfc_get_pair_jsonl(1, 32, 128, 0, 4096);
        let err = parse_jsonl_trace(&on_cmd)
            .expect_err("tag=32 on spu_mfc_cmd must reject");
        assert!(
            matches!(err, TraceParseError::BadMfcTag { tag: 32, .. }),
            "got {err:?}"
        );

        let on_complete = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
{{"seq":2,"side":"spu","kind":"mfc_dma_complete","target_spu":1,"tag":99,"transferred_bytes":128}}
"#
        );
        let err = parse_jsonl_trace(&on_complete)
            .expect_err("tag=99 on mfc_dma_complete must reject");
        assert!(
            matches!(err, TraceParseError::BadMfcTag { tag: 99, .. }),
            "got {err:?}"
        );
    }

    /// Negative: bare `spu_wrch ch21` with NO follow-up `spu_mfc_cmd`
    /// is rejected with `UnsupportedDmaInTrace` — preserves the R5.9e.2
    /// gate semantics for legacy / partial captures. R6.7 traces with
    /// the additive `spu_mfc_cmd` parse fine instead.
    #[test]
    fn parser_still_rejects_bare_ch21_wrch_without_mfc_cmd_followup() {
        let jsonl = r#"
{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":1,"would_stall":false,"target_spu":1}
"#;
        let err = parse_jsonl_trace(jsonl)
            .expect_err("bare ch21 wrch must reject (no spu_mfc_cmd follow-up)");
        match err {
            TraceParseError::UnsupportedDmaInTrace { target_spu, event_index, channel } => {
                assert_eq!(target_spu, 1);
                assert_eq!(event_index, 0);
                assert_eq!(channel, 21);
            }
            other => panic!("expected UnsupportedDmaInTrace, got {other:?}"),
        }
    }

    /// Negative: `spu_mfc_cmd` not preceded by `spu_wrch ch21` at the
    /// previous index is rejected with `MalformedMfcSequence`.
    #[test]
    fn parser_rejects_orphan_spu_mfc_cmd() {
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
"#
        );
        let err = parse_jsonl_trace(&jsonl)
            .expect_err("orphan spu_mfc_cmd must reject");
        match err {
            TraceParseError::MalformedMfcSequence { target_spu: 1, wrch_event_index: 0, .. } => {}
            other => panic!("expected MalformedMfcSequence, got {other:?}"),
        }
    }

    /// R6.7 C.5 — the transformer now ACCEPTS valid GET MFC traces
    /// (drops MFC events as context, same as `spu_wrch` /
    /// `spu_rdch`). The pre-replay helper
    /// (`crate::mfc_replay::apply_mfc_dma_pre_replay`) is the layer
    /// that actually applies DMA bytes to LS + populates the
    /// rdch ch24 queue; the transformer itself only emits
    /// `TraceEvent`s for PPU actions + `ExpectSpuFinished` +
    /// `ExpectChannelState`. Updated from the A.2 / A.4
    /// rejection-test that asserted hard-reject.
    #[test]
    fn transformer_accepts_valid_get_mfc_trace_after_executor_wiring() {
        let jsonl = format!(
            r#"
{{"seq":0,"side":"spu","kind":"spu_wrch","pc":256,"channel":21,"value":64,"would_stall":false,"target_spu":1}}
{{"seq":1,"side":"spu","kind":"spu_mfc_cmd","target_spu":1,"pc":256,"cmd":64,"tag":3,"size":128,"lsa":0,"eah":0,"eal":4096,"ea_chunk_sha256":"{TEST_VALID_SHA}"}}
{{"seq":2,"side":"spu","kind":"mfc_dma_complete","target_spu":1,"tag":3,"transferred_bytes":128}}
{{"seq":3,"side":"spu","kind":"spu_stop","pc":260,"stop_code":1,"target_spu":1}}
{{"seq":4,"side":"spu","kind":"final_state","gpr_lane_zero":[],"channels":{{"in_mbox":null,"out_mbox":null,"out_intr_mbox":null,"snr1":0,"snr2":0}},"target_spu":1}}
"#
        );
        let events = parse_jsonl_trace(&jsonl).expect("parser accepts the MFC sequence");
        let trace = captured_events_to_trace(&events)
            .expect("transformer must accept valid GET MFC trace post Phase C wiring");
        // The transformed trace contains the PpuPopOutMbox synthetic
        // (from spu_stop with code 1, which doesn't trigger the
        // 0x101/0x102 drain), ExpectSpuFinished, and
        // ExpectChannelState from final_state. MFC events do NOT
        // contribute additional TraceEvents — they're pure context.
        // Specifically, no event of any kind references "MFC" or
        // "DMA" since those are pre-replay-applied.
        assert!(
            trace.iter().any(|e| matches!(e, TraceEvent::ExpectSpuFinished { stop_code: 1 })),
            "transform must include ExpectSpuFinished{{stop_code:1}}, got {trace:?}"
        );
    }

    /// R6.7 C.5 — `mfc_dma_complete` standalone (no preceding
    /// `spu_wrch ch21` + matching `spu_mfc_cmd`) is now also
    /// transformer-context. The parser would catch a malformed
    /// trace at parse time (via `MalformedMfcSequence` for an
    /// orphan `spu_mfc_cmd`); a hand-built event vec containing
    /// `mfc_dma_complete` alone bypasses the parser, so we verify
    /// the transformer treats it as inert context (no error, no
    /// TraceEvent emitted, state machine proceeds to spu_stop).
    #[test]
    fn transformer_treats_mfc_events_as_pure_context() {
        let events = vec![
            CapturedEvent::MfcDmaComplete(MfcDmaCompleteEvent {
                seq: 0,
                side: CapturedSide::Spu,
                target_spu: 1,
                tag: 3,
                transferred_bytes: 128,
            }),
            CapturedEvent::SpuStop(SpuStopEvent {
                seq: 1,
                side: CapturedSide::Spu,
                pc: 256,
                stop_code: 1,
                target_spu: Some(1),
            }),
            CapturedEvent::FinalState(FinalStateEvent {
                seq: 2,
                side: CapturedSide::Spu,
                gpr_lane_zero: Vec::new(),
                channels: CapturedChannels {
                    in_mbox: None,
                    out_mbox: None,
                    out_intr_mbox: None,
                    snr1: 0,
                    snr2: 0,
                },
                target_spu: Some(1),
            }),
        ];
        let trace = captured_events_to_trace(&events)
            .expect("transformer treats orphan mfc_dma_complete as inert context");
        assert!(
            trace.iter().any(|e| matches!(e, TraceEvent::ExpectSpuFinished { stop_code: 1 })),
            "transform must still produce ExpectSpuFinished, got {trace:?}"
        );
    }
}
