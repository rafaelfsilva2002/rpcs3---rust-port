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
/// `NV4097_SET_VIEWPORT_HORIZONTAL` (0x0A00).
pub const VIEWPORT_HORIZONTAL: u32 = 0x0A00 >> 2;
/// `NV4097_SET_VIEWPORT_VERTICAL` (0x0A04).
pub const VIEWPORT_VERTICAL: u32 = 0x0A04 >> 2;

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
}
