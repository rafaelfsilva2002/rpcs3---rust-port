//! `rpcs3-rsx-state` — the RSX method register file.
//!
//! The RSX command stream (decoded by [`rpcs3_rsx_fifo`]) is a series
//! of writes to a flat register space addressed by method index
//! (byte offset / 4). This crate models that register file as a plain
//! array plus typed accessors for the most common NV4097 register
//! groups, and applies a batch of FIFO writes in order.
//!
//! It is pure state — no rendering, no GPU backend. The decode → state
//! pipeline (`FifoEngine::run` → `RsxState::apply`) is fully testable
//! against a captured GCM command stream, the natural shape of a
//! future replay oracle.
//!
//! ## Register space
//!
//! RSX methods occupy byte offsets `0..0x10000`, i.e. register
//! indices `0..0x4000` (16384 u32 slots). Method constants below are
//! given as **register indices** (the C++ `NV4097_*` byte offsets
//! divided by 4).

use rpcs3_rsx_fifo::{FifoEngine, FifoError};

/// Number of u32 method registers (`0x10000` bytes / 4).
pub const METHOD_COUNT: usize = 0x4000;

// =====================================================================
// Common NV4097 method register indices (byte offset / 4)
// =====================================================================

/// `NV4097_SET_SURFACE_CLIP_HORIZONTAL` (0x0200).
pub const SURFACE_CLIP_HORIZONTAL: u32 = 0x0200 >> 2;
/// `NV4097_SET_SURFACE_CLIP_VERTICAL` (0x0204).
pub const SURFACE_CLIP_VERTICAL: u32 = 0x0204 >> 2;
/// `NV4097_SET_COLOR_CLEAR_VALUE` (0x1D90).
pub const COLOR_CLEAR_VALUE: u32 = 0x1D90 >> 2;
/// `NV4097_SET_ZSTENCIL_CLEAR_VALUE` (0x1D8C).
pub const ZSTENCIL_CLEAR_VALUE: u32 = 0x1D8C >> 2;
/// `NV4097_CLEAR_SURFACE` (0x1D94).
pub const CLEAR_SURFACE: u32 = 0x1D94 >> 2;
/// `NV4097_SET_BEGIN_END` (0x1808).
pub const BEGIN_END: u32 = 0x1808 >> 2;
/// `NV4097_DRAW_ARRAYS` (0x1814).
pub const DRAW_ARRAYS: u32 = 0x1814 >> 2;
/// `NV4097_DRAW_INDEX_ARRAY` (0x1824).
pub const DRAW_INDEX_ARRAY: u32 = 0x1824 >> 2;
/// `NV4097_SET_VIEWPORT_HORIZONTAL` (0x0A00).
pub const VIEWPORT_HORIZONTAL: u32 = 0x0A00 >> 2;
/// `NV4097_SET_VIEWPORT_VERTICAL` (0x0A04).
pub const VIEWPORT_VERTICAL: u32 = 0x0A04 >> 2;

// -- NV406E (DMA channel) control methods (R12.4) --------------------
/// `NV406E_SET_REFERENCE` (0x0050).
pub const SET_REFERENCE: u32 = 0x0050 >> 2;
/// `NV406E_SEMAPHORE_OFFSET` (0x0064).
pub const SEMAPHORE_OFFSET: u32 = 0x0064 >> 2;
/// `NV406E_SEMAPHORE_ACQUIRE` (0x0068).
pub const SEMAPHORE_ACQUIRE: u32 = 0x0068 >> 2;
/// `NV406E_SEMAPHORE_RELEASE` (0x006C).
pub const SEMAPHORE_RELEASE: u32 = 0x006C >> 2;
/// `NV4097_SET_SEMAPHORE_OFFSET` / back-end write label (0x1D6C).
pub const BACKEND_WRITE_SEMAPHORE_RELEASE: u32 = 0x1D70 >> 2;

// =====================================================================
// Method classification + effect recognition (R12.4)
// =====================================================================

/// Coarse RSX method class for a register. Bands are approximate —
/// the precise per-method table is built in later slices; this is a
/// diagnostic grouping for the common regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodGroup {
    /// NV406E DMA-channel control (semaphore / reference), byte
    /// offsets `0x40..0x100`.
    ChannelDma,
    /// NV4097 Curie 3D state + commands, byte offsets `0x100..0x4000`.
    Graphics3D,
    /// Anything outside the recognized bands (M2MF, image blit,
    /// surface 2D, vendor-specific) — refined in later slices.
    Other,
}

/// Classify a method register into its coarse [`MethodGroup`].
#[must_use]
pub fn classify(reg: u32) -> MethodGroup {
    let byte = reg << 2;
    match byte {
        0x40..=0xFF => MethodGroup::ChannelDma,
        0x100..=0x3FFF => MethodGroup::Graphics3D,
        _ => MethodGroup::Other,
    }
}

/// The observable effect of applying one method write. Plain state
/// writes are [`MethodEffect::SetState`]; control methods that do
/// something beyond updating the register surface as their own
/// variant so the command-processor layer can act on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MethodEffect {
    /// Ordinary register write (the common case).
    SetState,
    /// `NV406E_SET_REFERENCE` — publish the channel reference value.
    SetReference(u32),
    /// `NV406E_SEMAPHORE_ACQUIRE` — wait until the semaphore reads
    /// the given value.
    SemaphoreAcquire(u32),
    /// `NV406E_SEMAPHORE_RELEASE` / back-end write — store the value
    /// to the bound semaphore.
    SemaphoreRelease(u32),
    /// `NV4097_CLEAR_SURFACE` — clear with the given mask.
    ClearSurface(u32),
    /// `NV4097_SET_BEGIN_END` — begin (non-zero mode) / end (0) a
    /// primitive block.
    BeginEnd(u32),
}

// =====================================================================
// Register file
// =====================================================================

/// The RSX method register file: a flat `u32` array indexed by method
/// register index, plus typed accessors.
#[derive(Clone)]
pub struct RsxState {
    methods: Box<[u32; METHOD_COUNT]>,
}

impl Default for RsxState {
    fn default() -> Self {
        Self::new()
    }
}

impl RsxState {
    /// A zeroed register file.
    #[must_use]
    pub fn new() -> Self {
        Self { methods: Box::new([0u32; METHOD_COUNT]) }
    }

    /// Read a method register. Out-of-range indices read 0 (the RSX
    /// address space is fixed; a malformed index can't fault here).
    #[must_use]
    pub fn read(&self, reg: u32) -> u32 {
        self.methods.get(reg as usize).copied().unwrap_or(0)
    }

    /// Write a method register. Out-of-range writes are dropped
    /// (mirrors hardware ignoring unmapped method offsets rather than
    /// faulting the command processor).
    pub fn write(&mut self, reg: u32, arg: u32) {
        if let Some(slot) = self.methods.get_mut(reg as usize) {
            *slot = arg;
        }
    }

    /// Apply a batch of `(register, arg)` writes in order (the output
    /// of [`FifoEngine::run`]).
    pub fn apply(&mut self, writes: &[(u32, u32)]) {
        for &(reg, arg) in writes {
            self.write(reg, arg);
        }
    }

    /// Write the register AND classify the write into a
    /// [`MethodEffect`]. Control methods (semaphore, clear,
    /// begin/end) return their own variant so the command-processor
    /// layer can act; everything else is `SetState`.
    pub fn dispatch(&mut self, reg: u32, arg: u32) -> MethodEffect {
        self.write(reg, arg);
        match reg {
            SET_REFERENCE => MethodEffect::SetReference(arg),
            SEMAPHORE_ACQUIRE => MethodEffect::SemaphoreAcquire(arg),
            SEMAPHORE_RELEASE | BACKEND_WRITE_SEMAPHORE_RELEASE => {
                MethodEffect::SemaphoreRelease(arg)
            }
            CLEAR_SURFACE => MethodEffect::ClearSurface(arg),
            BEGIN_END => MethodEffect::BeginEnd(arg),
            _ => MethodEffect::SetState,
        }
    }

    /// Apply a batch via [`Self::dispatch`], returning the sequence of
    /// non-`SetState` effects in order (control events the command
    /// processor must handle).
    pub fn apply_with_effects(&mut self, writes: &[(u32, u32)]) -> Vec<MethodEffect> {
        let mut effects = Vec::new();
        for &(reg, arg) in writes {
            let e = self.dispatch(reg, arg);
            if e != MethodEffect::SetState {
                effects.push(e);
            }
        }
        effects
    }

    /// Convenience: run the FIFO over `buf` and apply every write.
    pub fn run_and_apply(
        &mut self,
        engine: &mut FifoEngine,
        buf: &[u8],
    ) -> Result<(), FifoError> {
        let writes = engine.run(buf)?;
        self.apply(&writes);
        Ok(())
    }

    // -- Typed accessors for common register groups ----------------

    /// 32-bit packed clear color (`NV4097_SET_COLOR_CLEAR_VALUE`).
    #[must_use]
    pub fn color_clear_value(&self) -> u32 {
        self.read(COLOR_CLEAR_VALUE)
    }

    /// Packed Z/stencil clear value.
    #[must_use]
    pub fn zstencil_clear_value(&self) -> u32 {
        self.read(ZSTENCIL_CLEAR_VALUE)
    }

    /// The last `NV4097_CLEAR_SURFACE` mask written.
    #[must_use]
    pub fn clear_surface_mask(&self) -> u32 {
        self.read(CLEAR_SURFACE)
    }

    /// Surface clip as `(x_origin, width)` from
    /// `SET_SURFACE_CLIP_HORIZONTAL` (low 16 = origin, high 16 = size).
    #[must_use]
    pub fn surface_clip_horizontal(&self) -> (u16, u16) {
        let v = self.read(SURFACE_CLIP_HORIZONTAL);
        ((v & 0xFFFF) as u16, (v >> 16) as u16)
    }

    /// Surface clip as `(y_origin, height)`.
    #[must_use]
    pub fn surface_clip_vertical(&self) -> (u16, u16) {
        let v = self.read(SURFACE_CLIP_VERTICAL);
        ((v & 0xFFFF) as u16, (v >> 16) as u16)
    }

    /// Current `NV4097_SET_BEGIN_END` primitive type (0 = END /
    /// no primitive in progress).
    #[must_use]
    pub fn begin_end(&self) -> u32 {
        self.read(BEGIN_END)
    }
}

// =====================================================================
// R12.5 — draw-call recognition
// =====================================================================

/// Whether a draw sources vertices directly (DRAW_ARRAYS) or through
/// an index buffer (DRAW_INDEX_ARRAY).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawKind {
    /// `NV4097_DRAW_ARRAYS` — sequential vertices.
    Arrays,
    /// `NV4097_DRAW_INDEX_ARRAY` — indexed vertices.
    Indexed,
}

/// A completed draw: the primitive type set by `SET_BEGIN_END` plus
/// every `(first, count)` vertex range issued between begin and end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawCall {
    /// Primitive mode from `SET_BEGIN_END` (1=points, 2=lines,
    /// 3=line-loop, 4=line-strip, 5=triangles, 8=quads, ... per NV).
    pub primitive: u32,
    /// Arrays vs indexed.
    pub kind: DrawKind,
    /// `(first_vertex, count)` ranges accumulated for this draw.
    pub ranges: Vec<(u32, u32)>,
}

/// Decode a `DRAW_ARRAYS` / `DRAW_INDEX_ARRAY` arg word into a
/// `(first, count)` range: `first = arg & 0xFFFFFF`,
/// `count = ((arg >> 24) & 0xFF) + 1`.
#[must_use]
pub fn decode_draw_range(arg: u32) -> (u32, u32) {
    let first = arg & 0x00FF_FFFF;
    let count = ((arg >> 24) & 0xFF) + 1;
    (first, count)
}

/// Accumulates draw state across a write stream, emitting a
/// [`DrawCall`] when `SET_BEGIN_END` closes a primitive block.
///
/// Usage: feed every `(register, arg)` write via [`Self::process`];
/// it returns `Some(DrawCall)` on the END that finalizes a draw.
#[derive(Debug, Clone, Default)]
pub struct DrawTracker {
    primitive: Option<u32>,
    kind: DrawKind,
    ranges: Vec<(u32, u32)>,
}

impl Default for DrawKind {
    fn default() -> Self {
        DrawKind::Arrays
    }
}

impl DrawTracker {
    /// A fresh tracker (no primitive in progress).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Process one method write. Returns `Some(DrawCall)` when an
    /// END (`SET_BEGIN_END` with arg 0) finalizes a draw that issued
    /// at least one range; otherwise `None`.
    pub fn process(&mut self, reg: u32, arg: u32) -> Option<DrawCall> {
        match reg {
            BEGIN_END if arg != 0 => {
                // Begin: start a fresh primitive block.
                self.primitive = Some(arg);
                self.ranges.clear();
                None
            }
            BEGIN_END => {
                // End: emit the accumulated draw, if any ranges.
                let prim = self.primitive.take()?;
                if self.ranges.is_empty() {
                    return None;
                }
                Some(DrawCall {
                    primitive: prim,
                    kind: self.kind,
                    ranges: core::mem::take(&mut self.ranges),
                })
            }
            DRAW_ARRAYS => {
                if self.primitive.is_some() {
                    self.kind = DrawKind::Arrays;
                    self.ranges.push(decode_draw_range(arg));
                }
                None
            }
            DRAW_INDEX_ARRAY => {
                if self.primitive.is_some() {
                    self.kind = DrawKind::Indexed;
                    self.ranges.push(decode_draw_range(arg));
                }
                None
            }
            _ => None,
        }
    }

    /// True when a primitive block is open (between begin and end).
    #[must_use]
    pub fn in_primitive(&self) -> bool {
        self.primitive.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpcs3_rsx_fifo::FifoEngine;

    fn words(ws: &[u32]) -> Vec<u8> {
        let mut v = Vec::with_capacity(ws.len() * 4);
        for w in ws {
            v.extend_from_slice(&w.to_be_bytes());
        }
        v
    }

    #[test]
    fn write_read_round_trip() {
        let mut s = RsxState::new();
        s.write(COLOR_CLEAR_VALUE, 0xDEAD_BEEF);
        assert_eq!(s.color_clear_value(), 0xDEAD_BEEF);
    }

    #[test]
    fn out_of_range_write_dropped_read_zero() {
        let mut s = RsxState::new();
        s.write(0xFFFF_FFFF, 0x1234);
        assert_eq!(s.read(0xFFFF_FFFF), 0);
    }

    #[test]
    fn apply_batch_in_order() {
        let mut s = RsxState::new();
        s.apply(&[(BEGIN_END, 8), (BEGIN_END, 0)]);
        // last write wins.
        assert_eq!(s.begin_end(), 0);
    }

    #[test]
    fn surface_clip_unpacks_origin_and_size() {
        let mut s = RsxState::new();
        // width 1280 (0x500) in high half, origin 0 in low.
        s.write(SURFACE_CLIP_HORIZONTAL, (1280u32 << 16) | 0);
        assert_eq!(s.surface_clip_horizontal(), (0, 1280));
        s.write(SURFACE_CLIP_VERTICAL, (720u32 << 16) | 0);
        assert_eq!(s.surface_clip_vertical(), (0, 720));
    }

    #[test]
    fn fifo_to_state_pipeline() {
        // A GCM stream that sets the clear color then issues
        // CLEAR_SURFACE — decode via the engine, apply to state.
        let h_color = (1 << 18) | (COLOR_CLEAR_VALUE << 2); // 1 arg
        let h_clear = (1 << 18) | (CLEAR_SURFACE << 2);
        let buf = words(&[h_color, 0x00FF_8040, h_clear, 0xF3]);
        let mut eng = FifoEngine::new(0, 16);
        let mut state = RsxState::new();
        state.run_and_apply(&mut eng, &buf).unwrap();
        assert_eq!(state.color_clear_value(), 0x00FF_8040);
        assert_eq!(state.clear_surface_mask(), 0xF3);
    }

    #[test]
    fn increment_run_fills_consecutive_registers() {
        // viewport horizontal then vertical via a 2-arg increment
        // (the two registers are adjacent: 0x0A00>>2, 0x0A04>>2).
        let h = (2 << 18) | (VIEWPORT_HORIZONTAL << 2);
        let buf = words(&[h, 0x0050_0000, 0x002D_0000]);
        let mut eng = FifoEngine::new(0, 12);
        let mut state = RsxState::new();
        state.run_and_apply(&mut eng, &buf).unwrap();
        assert_eq!(state.read(VIEWPORT_HORIZONTAL), 0x0050_0000);
        assert_eq!(state.read(VIEWPORT_VERTICAL), 0x002D_0000);
    }

    #[test]
    fn method_constants_match_byte_offsets() {
        assert_eq!(COLOR_CLEAR_VALUE, 0x1D90 / 4);
        assert_eq!(CLEAR_SURFACE, 0x1D94 / 4);
        assert_eq!(BEGIN_END, 0x1808 / 4);
        assert_eq!(DRAW_ARRAYS, 0x1814 / 4);
    }

    // ---- R12.4: classify + dispatch effects -----------------------

    #[test]
    fn classify_bands() {
        assert_eq!(classify(SEMAPHORE_RELEASE), MethodGroup::ChannelDma);
        assert_eq!(classify(COLOR_CLEAR_VALUE), MethodGroup::Graphics3D);
        assert_eq!(classify(CLEAR_SURFACE), MethodGroup::Graphics3D);
        // byte 0 (reg 0) is below the ChannelDma band → Other.
        assert_eq!(classify(0), MethodGroup::Other);
        // a very high register is Other.
        assert_eq!(classify(0x3000), MethodGroup::Other);
    }

    #[test]
    fn dispatch_plain_write_is_setstate() {
        let mut s = RsxState::new();
        assert_eq!(s.dispatch(VIEWPORT_HORIZONTAL, 0x123), MethodEffect::SetState);
        assert_eq!(s.read(VIEWPORT_HORIZONTAL), 0x123);
    }

    #[test]
    fn dispatch_clear_surface_effect() {
        let mut s = RsxState::new();
        assert_eq!(s.dispatch(CLEAR_SURFACE, 0xF3), MethodEffect::ClearSurface(0xF3));
        // register still updated.
        assert_eq!(s.clear_surface_mask(), 0xF3);
    }

    #[test]
    fn dispatch_semaphore_and_begin_end() {
        let mut s = RsxState::new();
        assert_eq!(
            s.dispatch(SEMAPHORE_RELEASE, 0x42),
            MethodEffect::SemaphoreRelease(0x42)
        );
        assert_eq!(
            s.dispatch(SEMAPHORE_ACQUIRE, 0x7),
            MethodEffect::SemaphoreAcquire(0x7)
        );
        assert_eq!(s.dispatch(SET_REFERENCE, 0x99), MethodEffect::SetReference(0x99));
        assert_eq!(s.dispatch(BEGIN_END, 8), MethodEffect::BeginEnd(8));
        assert_eq!(s.dispatch(BEGIN_END, 0), MethodEffect::BeginEnd(0));
    }

    #[test]
    fn apply_with_effects_filters_setstate() {
        let mut s = RsxState::new();
        let writes = [
            (VIEWPORT_HORIZONTAL, 0x10),   // SetState (filtered)
            (CLEAR_SURFACE, 0xF3),         // effect
            (COLOR_CLEAR_VALUE, 0xABCD),   // SetState (filtered)
            (SEMAPHORE_RELEASE, 0x1),      // effect
        ];
        let effects = s.apply_with_effects(&writes);
        assert_eq!(
            effects,
            vec![MethodEffect::ClearSurface(0xF3), MethodEffect::SemaphoreRelease(0x1)]
        );
        // plain writes still landed.
        assert_eq!(s.read(VIEWPORT_HORIZONTAL), 0x10);
        assert_eq!(s.color_clear_value(), 0xABCD);
    }

    // ---- R12.5: draw-call recognition -----------------------------

    #[test]
    fn decode_draw_range_unpacks_first_count() {
        // first=0x100, count=0x40 → arg = ((0x40-1)<<24)|0x100.
        let arg = ((0x40u32 - 1) << 24) | 0x100;
        assert_eq!(decode_draw_range(arg), (0x100, 0x40));
    }

    #[test]
    fn draw_tracker_emits_call_on_end() {
        let mut t = DrawTracker::new();
        assert_eq!(t.process(BEGIN_END, 5), None); // begin triangles
        assert!(t.in_primitive());
        // one DRAW_ARRAYS: first 0, count 3.
        let arg = ((3u32 - 1) << 24) | 0;
        assert_eq!(t.process(DRAW_ARRAYS, arg), None);
        // end → emits.
        let call = t.process(BEGIN_END, 0).unwrap();
        assert_eq!(call.primitive, 5);
        assert_eq!(call.kind, DrawKind::Arrays);
        assert_eq!(call.ranges, vec![(0, 3)]);
        assert!(!t.in_primitive());
    }

    #[test]
    fn draw_tracker_accumulates_multiple_ranges() {
        let mut t = DrawTracker::new();
        t.process(BEGIN_END, 8); // quads
        t.process(DRAW_ARRAYS, ((4u32 - 1) << 24) | 0);
        t.process(DRAW_ARRAYS, ((4u32 - 1) << 24) | 4);
        let call = t.process(BEGIN_END, 0).unwrap();
        assert_eq!(call.ranges, vec![(0, 4), (4, 4)]);
    }

    #[test]
    fn draw_tracker_indexed_kind() {
        let mut t = DrawTracker::new();
        t.process(BEGIN_END, 5);
        t.process(DRAW_INDEX_ARRAY, ((6u32 - 1) << 24) | 0);
        let call = t.process(BEGIN_END, 0).unwrap();
        assert_eq!(call.kind, DrawKind::Indexed);
        assert_eq!(call.ranges, vec![(0, 6)]);
    }

    #[test]
    fn draw_tracker_empty_block_emits_nothing() {
        let mut t = DrawTracker::new();
        t.process(BEGIN_END, 5);
        // no DRAW_ARRAYS issued.
        assert_eq!(t.process(BEGIN_END, 0), None);
    }

    #[test]
    fn draw_arrays_outside_begin_ignored() {
        let mut t = DrawTracker::new();
        // DRAW_ARRAYS with no open primitive → ignored.
        assert_eq!(t.process(DRAW_ARRAYS, 0x0300_0000), None);
        assert!(!t.in_primitive());
    }

    #[test]
    fn draw_tracker_full_pipeline_from_fifo() {
        // GCM stream: begin(triangles), draw_arrays(0,3), end.
        let h_begin = (1 << 18) | (BEGIN_END << 2);
        let h_draw = (1 << 18) | (DRAW_ARRAYS << 2);
        let h_end = (1 << 18) | (BEGIN_END << 2);
        let draw_arg = ((3u32 - 1) << 24) | 0;
        let buf = words(&[h_begin, 5, h_draw, draw_arg, h_end, 0]);
        let mut eng = FifoEngine::new(0, 24);
        let writes = eng.run(&buf).unwrap();
        let mut t = DrawTracker::new();
        let mut calls = Vec::new();
        for &(reg, arg) in &writes {
            if let Some(c) = t.process(reg, arg) {
                calls.push(c);
            }
        }
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].primitive, 5);
        assert_eq!(calls[0].ranges, vec![(0, 3)]);
    }
}
