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
use rpcs3_rsx_vertex_data::VertexBaseType;

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
/// `NV4097_SET_VERTEX_DATA_ARRAY_FORMAT(0)` (0x1740). 16 attributes,
/// one register each.
pub const VERTEX_DATA_ARRAY_FORMAT: u32 = 0x1740 >> 2;
/// `NV4097_SET_VERTEX_DATA_ARRAY_OFFSET(0)` (0x1680). 16 attributes.
pub const VERTEX_DATA_ARRAY_OFFSET: u32 = 0x1680 >> 2;
/// Number of vertex attribute inputs.
pub const VERTEX_ATTRIB_COUNT: u32 = 16;
/// `NV4097_SET_INDEX_ARRAY_ADDRESS` (0x181C) — byte offset of the
/// index buffer in the bound DMA context.
pub const INDEX_ARRAY_ADDRESS: u32 = 0x181C >> 2;
/// `NV4097_SET_INDEX_ARRAY_DMA` (0x1820) — packs the index element
/// type (bit 4) and the DMA context location (low 4 bits).
pub const INDEX_ARRAY_DMA: u32 = 0x1820 >> 2;
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

// =====================================================================
// R12.6 — vertex attribute format parsing (Camada B)
// =====================================================================

/// A decoded vertex attribute array descriptor, from the
/// `SET_VERTEX_DATA_ARRAY_FORMAT` + `..._OFFSET` register pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VertexAttribute {
    /// Element base type (float / unorm8 / snorm16 / ...).
    pub base_type: VertexBaseType,
    /// Component count (1..=4).
    pub count: u8,
    /// Byte stride between consecutive vertices (0 = tightly packed
    /// / constant).
    pub stride: u8,
    /// Instancing frequency divider (0 = per-vertex).
    pub frequency: u16,
    /// Raw offset register value (low 31 bits = byte offset into the
    /// bound DMA buffer; bit 31 selects main vs local memory).
    pub offset: u32,
}

impl VertexAttribute {
    /// True when this attribute reads from main (system) memory
    /// rather than local (video) memory — bit 31 of the offset reg.
    #[must_use]
    pub fn in_main_memory(&self) -> bool {
        self.offset & 0x8000_0000 != 0
    }

    /// Byte offset into the bound buffer (offset reg with the
    /// memory-select bit masked off).
    #[must_use]
    pub fn byte_offset(&self) -> u32 {
        self.offset & 0x7FFF_FFFF
    }
}

/// Map the 4-bit type field of a `SET_VERTEX_DATA_ARRAY_FORMAT` word
/// to a [`VertexBaseType`]. Returns `None` for the undefined code 0
/// or any reserved value.
#[must_use]
pub fn vertex_base_type_from_code(code: u32) -> Option<VertexBaseType> {
    match code & 0xF {
        1 => Some(VertexBaseType::S1),
        2 => Some(VertexBaseType::F),
        3 => Some(VertexBaseType::Sf),
        4 => Some(VertexBaseType::Ub),
        5 => Some(VertexBaseType::S32k),
        6 => Some(VertexBaseType::Cmp),
        7 => Some(VertexBaseType::Ub256),
        _ => None,
    }
}

/// Decode a `SET_VERTEX_DATA_ARRAY_FORMAT` register word into its
/// `(base_type, count, stride, frequency)` fields.
///
/// Bit layout (RPCS3 `rsx::registers`): type = bits 0..3,
/// count = bits 4..7, stride = bits 8..15, frequency = bits 16..31.
/// Returns `None` when count is 0 (the attribute is disabled) or the
/// type code is undefined.
#[must_use]
pub fn decode_vertex_format(word: u32) -> Option<(VertexBaseType, u8, u8, u16)> {
    let count = ((word >> 4) & 0xF) as u8;
    if count == 0 {
        return None;
    }
    let base_type = vertex_base_type_from_code(word)?;
    let stride = ((word >> 8) & 0xFF) as u8;
    let frequency = ((word >> 16) & 0xFFFF) as u16;
    Some((base_type, count, stride, frequency))
}

impl RsxState {
    /// Read and decode vertex attribute `index` (0..16). Returns
    /// `None` when the attribute is disabled (count 0) or its type
    /// code is undefined.
    #[must_use]
    pub fn vertex_attribute(&self, index: u32) -> Option<VertexAttribute> {
        if index >= VERTEX_ATTRIB_COUNT {
            return None;
        }
        let fmt = self.read(VERTEX_DATA_ARRAY_FORMAT + index);
        let (base_type, count, stride, frequency) = decode_vertex_format(fmt)?;
        let offset = self.read(VERTEX_DATA_ARRAY_OFFSET + index);
        Some(VertexAttribute { base_type, count, stride, frequency, offset })
    }

    /// Collect every enabled vertex attribute as `(index, attr)`.
    #[must_use]
    pub fn enabled_vertex_attributes(&self) -> Vec<(u32, VertexAttribute)> {
        (0..VERTEX_ATTRIB_COUNT)
            .filter_map(|i| self.vertex_attribute(i).map(|a| (i, a)))
            .collect()
    }
}

// =====================================================================
// R12.7 — index buffer descriptor (Camada B)
// =====================================================================

/// Index element width for indexed draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexType {
    /// 32-bit indices (`NV4097` type code 0).
    U32,
    /// 16-bit indices (type code 1).
    U16,
}

/// The bound index array, from `SET_INDEX_ARRAY_ADDRESS` +
/// `SET_INDEX_ARRAY_DMA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexArray {
    /// Byte offset of the index buffer in the bound DMA context.
    pub address: u32,
    /// Index element width.
    pub index_type: IndexType,
    /// DMA context location selector (low 4 bits of the DMA reg).
    pub location: u8,
}

impl IndexType {
    /// Size of one index in bytes.
    #[must_use]
    pub fn size_bytes(self) -> u32 {
        match self {
            IndexType::U32 => 4,
            IndexType::U16 => 2,
        }
    }
}

impl RsxState {
    /// The currently-bound index array. The DMA register's bit 4
    /// selects the index width (0 = u32, 1 = u16); its low 4 bits
    /// select the DMA context.
    #[must_use]
    pub fn index_array(&self) -> IndexArray {
        let address = self.read(INDEX_ARRAY_ADDRESS);
        let dma = self.read(INDEX_ARRAY_DMA);
        let index_type = if dma & 0x10 != 0 {
            IndexType::U16
        } else {
            IndexType::U32
        };
        IndexArray { address, index_type, location: (dma & 0xF) as u8 }
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

    // ---- R12.6: vertex attribute format parsing ------------------

    /// Build a SET_VERTEX_DATA_ARRAY_FORMAT word.
    fn fmt(type_code: u32, count: u32, stride: u32, freq: u32) -> u32 {
        (type_code & 0xF) | ((count & 0xF) << 4) | ((stride & 0xFF) << 8)
            | ((freq & 0xFFFF) << 16)
    }

    #[test]
    fn decode_vertex_format_fields() {
        // type=2 (F), count=3, stride=12, freq=0.
        let w = fmt(2, 3, 12, 0);
        assert_eq!(
            decode_vertex_format(w),
            Some((VertexBaseType::F, 3, 12, 0))
        );
    }

    #[test]
    fn decode_vertex_format_disabled_when_count_zero() {
        let w = fmt(2, 0, 12, 0);
        assert_eq!(decode_vertex_format(w), None);
    }

    #[test]
    fn decode_vertex_format_undefined_type_rejected() {
        // type code 0 = undefined, but count != 0.
        let w = fmt(0, 3, 12, 0);
        assert_eq!(decode_vertex_format(w), None);
    }

    #[test]
    fn vertex_base_type_code_table() {
        assert_eq!(vertex_base_type_from_code(1), Some(VertexBaseType::S1));
        assert_eq!(vertex_base_type_from_code(2), Some(VertexBaseType::F));
        assert_eq!(vertex_base_type_from_code(4), Some(VertexBaseType::Ub));
        assert_eq!(vertex_base_type_from_code(7), Some(VertexBaseType::Ub256));
        assert_eq!(vertex_base_type_from_code(0), None);
        assert_eq!(vertex_base_type_from_code(8), None);
    }

    #[test]
    fn rsx_state_reads_vertex_attribute() {
        let mut s = RsxState::new();
        // attribute 2: F, count 4, stride 16; offset in main memory.
        s.write(VERTEX_DATA_ARRAY_FORMAT + 2, fmt(2, 4, 16, 0));
        s.write(VERTEX_DATA_ARRAY_OFFSET + 2, 0x8000_1000);
        let a = s.vertex_attribute(2).unwrap();
        assert_eq!(a.base_type, VertexBaseType::F);
        assert_eq!(a.count, 4);
        assert_eq!(a.stride, 16);
        assert!(a.in_main_memory());
        assert_eq!(a.byte_offset(), 0x1000);
    }

    #[test]
    fn disabled_attribute_reads_none() {
        let s = RsxState::new();
        // nothing written → format word 0 → count 0 → disabled.
        assert_eq!(s.vertex_attribute(0), None);
        assert_eq!(s.vertex_attribute(16), None); // out of range
    }

    #[test]
    fn enabled_vertex_attributes_filters() {
        let mut s = RsxState::new();
        s.write(VERTEX_DATA_ARRAY_FORMAT + 0, fmt(2, 3, 12, 0));
        s.write(VERTEX_DATA_ARRAY_FORMAT + 5, fmt(4, 4, 4, 0));
        let enabled = s.enabled_vertex_attributes();
        assert_eq!(enabled.len(), 2);
        assert_eq!(enabled[0].0, 0);
        assert_eq!(enabled[1].0, 5);
        assert_eq!(enabled[1].1.base_type, VertexBaseType::Ub);
    }

    // ---- R12.7: index buffer descriptor --------------------------

    #[test]
    fn index_array_u32_default() {
        let mut s = RsxState::new();
        s.write(INDEX_ARRAY_ADDRESS, 0x0010_0000);
        s.write(INDEX_ARRAY_DMA, 0x0000_0000); // bit4 clear → u32
        let ia = s.index_array();
        assert_eq!(ia.address, 0x0010_0000);
        assert_eq!(ia.index_type, IndexType::U32);
        assert_eq!(ia.index_type.size_bytes(), 4);
    }

    #[test]
    fn index_array_u16_and_location() {
        let mut s = RsxState::new();
        s.write(INDEX_ARRAY_ADDRESS, 0x2000);
        s.write(INDEX_ARRAY_DMA, 0x10 | 0x0B); // bit4 set → u16, loc 0xB
        let ia = s.index_array();
        assert_eq!(ia.index_type, IndexType::U16);
        assert_eq!(ia.index_type.size_bytes(), 2);
        assert_eq!(ia.location, 0x0B);
        assert_eq!(ia.address, 0x2000);
    }

    #[test]
    fn index_array_from_fifo() {
        let h_addr = (1 << 18) | (INDEX_ARRAY_ADDRESS << 2);
        let h_dma = (1 << 18) | (INDEX_ARRAY_DMA << 2);
        let buf = words(&[h_addr, 0x4000, h_dma, 0x10]);
        let mut eng = FifoEngine::new(0, 16);
        let mut s = RsxState::new();
        s.run_and_apply(&mut eng, &buf).unwrap();
        let ia = s.index_array();
        assert_eq!(ia.address, 0x4000);
        assert_eq!(ia.index_type, IndexType::U16);
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
