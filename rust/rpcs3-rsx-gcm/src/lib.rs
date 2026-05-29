//! `rpcs3-rsx-gcm` — RSX/GCM command-buffer builder.
//!
//! Ports the command-*emission* layer of PSL1GHT's `libgcm` / `librsx`
//! (`gcmSetSurface`, `rsxClearSurface`, `rsxDrawVertexArray`, ...): the
//! functions a homebrew calls, each of which writes NV4097 / NV406E
//! method words into the RSX command buffer.
//!
//! This is the **producer** half that pairs with the decoder in
//! `rpcs3-rsx-fifo`: [`GcmContext`] emits a real command stream from
//! high-level GCM calls, and that stream decodes back through
//! `replay_gcm` to the exact resource descriptors. The round-trip
//! oracle (this crate's tests) proves emit and decode are inverse.
//!
//! ## Provenance tiers
//!
//! - Tier 1 (R12.10a): hand-authored hex stream.
//! - **Tier 2 (this crate, R12.10b):** stream *emitted* by ported
//!   libgcm command logic — real byte-origin from emission code, not
//!   hand-typed hex.
//! - Tier 3 (deferred): stream captured from PSL1GHT's *binary*
//!   libgcm executing (needs a cellGcm HLE in the emu core) or from
//!   RPCS3's `.rrc` RSX capture. Both would route into / replace this
//!   builder's output.
//!
//! ## Encoding
//!
//! A method write is a FIFO header `(count << 18) | (byte_offset)`
//! followed by `count` argument words (increment form: the method
//! byte-offset advances by 4 per arg). All words are stored
//! big-endian by [`GcmContext::finish`]. Method byte-offsets mirror
//! RPCS3 `rsx_methods.h`; the round-trip oracle verifies they agree
//! with the decoder's register indices (offset / 4).

// =====================================================================
// NV method byte-offsets (mirror rsx_methods.h; decoder uses /4)
// =====================================================================

const M_SURFACE_CLIP_HORIZONTAL: u32 = 0x0200;
const M_SURFACE_CLIP_VERTICAL: u32 = 0x0204;
const M_SURFACE_FORMAT: u32 = 0x0208;
const M_SURFACE_COLOR_A_OFFSET: u32 = 0x0210;
const M_SURFACE_ZETA_OFFSET: u32 = 0x0214;
const M_SURFACE_PITCH_A: u32 = 0x020C; // was wrongly 0x0218 (= COLOR_BOFFSET) per RPCS3 gcm_enums.h; fixed R13.5c
const M_SURFACE_COLOR_TARGET: u32 = 0x0220;
const M_SURFACE_PITCH_Z: u32 = 0x022C;
const M_COLOR_CLEAR_VALUE: u32 = 0x1D90;
const M_CLEAR_SURFACE: u32 = 0x1D94;
const M_VERTEX_DATA_ARRAY_FORMAT: u32 = 0x1740;
const M_VERTEX_DATA_ARRAY_OFFSET: u32 = 0x1680;
const M_INDEX_ARRAY_ADDRESS: u32 = 0x181C;
const M_INDEX_ARRAY_DMA: u32 = 0x1820;
const M_TEXTURE_OFFSET: u32 = 0x1A00;
const M_TEXTURE_FORMAT: u32 = 0x1A04;
const M_TEXTURE_CONTROL0: u32 = 0x1A0C;
const M_TEXTURE_IMAGE_RECT: u32 = 0x1A18;
const M_TEXTURE_STRIDE: u32 = 0x20;
const M_BEGIN_END: u32 = 0x1808;
const M_DRAW_ARRAYS: u32 = 0x1814;
const M_SEMAPHORE_RELEASE: u32 = 0x006C;

// =====================================================================
// Vertex base type codes (emission side; match rsx-vertex-data)
// =====================================================================

/// Float (4-byte) vertex component.
pub const VTX_TYPE_FLOAT: u32 = 2;
/// Unsigned-normalized 8-bit vertex component.
pub const VTX_TYPE_UNORM8: u32 = 4;
/// Signed-normalized 16-bit vertex component.
pub const VTX_TYPE_SNORM16: u32 = 1;

// =====================================================================
// GcmContext — the command builder
// =====================================================================

/// Builds an RSX command stream by emitting method writes, mirroring
/// the libgcm command-emission functions a homebrew calls.
#[derive(Debug, Clone, Default)]
pub struct GcmContext {
    words: Vec<u32>,
}

impl GcmContext {
    /// A fresh, empty command buffer.
    #[must_use]
    pub fn new() -> Self {
        Self { words: Vec::new() }
    }

    /// Byte length the stream will occupy (= PUT after these commands).
    #[must_use]
    pub fn put(&self) -> u32 {
        (self.words.len() * 4) as u32
    }

    /// The raw command words (host-endian).
    #[must_use]
    pub fn words(&self) -> &[u32] {
        &self.words
    }

    /// Finish: serialize the command buffer to big-endian bytes (the
    /// on-the-wire ring format the decoder consumes).
    #[must_use]
    pub fn finish(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.words.len() * 4);
        for w in &self.words {
            v.extend_from_slice(&w.to_be_bytes());
        }
        v
    }

    // -- low-level emission ----------------------------------------

    /// Emit an increment-method run: header + `args`. The method
    /// byte-offset advances by 4 per argument on the decode side.
    pub fn method(&mut self, byte_offset: u32, args: &[u32]) {
        debug_assert!(!args.is_empty(), "method run needs >=1 arg");
        let count = args.len() as u32;
        self.words.push((count << 18) | (byte_offset & 0x0003_FFFC));
        self.words.extend_from_slice(args);
    }

    /// Emit a single-argument method write.
    pub fn method1(&mut self, byte_offset: u32, arg: u32) {
        self.method(byte_offset, &[arg]);
    }

    // -- high-level GCM calls (libgcm-equivalent) ------------------

    /// `gcmSetSurfaceClip`-equivalent: render-target clip extents.
    pub fn set_surface_clip(&mut self, width: u16, height: u16) {
        self.method1(M_SURFACE_CLIP_HORIZONTAL, u32::from(width) << 16);
        self.method1(M_SURFACE_CLIP_VERTICAL, u32::from(height) << 16);
    }

    /// `gcmSetColorBuffer` / surface format + a single MRT-A target.
    #[allow(clippy::too_many_arguments)]
    pub fn set_surface(
        &mut self,
        color_format: u8,
        depth_format: u8,
        target: u32,
        color_a_offset: u32,
        color_a_pitch: u32,
        zeta_offset: u32,
        zeta_pitch: u32,
    ) {
        let fmt = u32::from(color_format) | (u32::from(depth_format) << 5);
        self.method1(M_SURFACE_FORMAT, fmt);
        self.method1(M_SURFACE_COLOR_TARGET, target);
        self.method1(M_SURFACE_COLOR_A_OFFSET, color_a_offset);
        self.method1(M_SURFACE_PITCH_A, color_a_pitch);
        self.method1(M_SURFACE_ZETA_OFFSET, zeta_offset);
        self.method1(M_SURFACE_PITCH_Z, zeta_pitch);
    }

    /// `rsxSetClearColor`.
    pub fn set_clear_color(&mut self, argb: u32) {
        self.method1(M_COLOR_CLEAR_VALUE, argb);
    }

    /// `rsxClearSurface`.
    pub fn clear_surface(&mut self, mask: u32) {
        self.method1(M_CLEAR_SURFACE, mask);
    }

    /// `rsxBindVertexArrayAttrib`: one vertex attribute array.
    pub fn set_vertex_data_array(
        &mut self,
        index: u32,
        base_type: u32,
        count: u32,
        stride: u32,
        offset: u32,
    ) {
        let fmt = (base_type & 0xF)
            | ((count & 0xF) << 4)
            | ((stride & 0xFF) << 8);
        self.method1(M_VERTEX_DATA_ARRAY_FORMAT + index * 4, fmt);
        self.method1(M_VERTEX_DATA_ARRAY_OFFSET + index * 4, offset);
    }

    /// `rsxSetIndexArray*`: bind the index buffer. `u16_indices`
    /// selects the 16-bit element width.
    pub fn set_index_array(&mut self, address: u32, u16_indices: bool) {
        self.method1(M_INDEX_ARRAY_ADDRESS, address);
        self.method1(M_INDEX_ARRAY_DMA, if u16_indices { 0x10 } else { 0 });
    }

    /// `rsxLoadTexture` (descriptor subset): bind a texture unit.
    #[allow(clippy::too_many_arguments)]
    pub fn set_texture(
        &mut self,
        unit: u32,
        offset: u32,
        location: u32,
        dimension: u32,
        format: u32,
        mipmap: u32,
        width: u16,
        height: u16,
    ) {
        let base = unit * M_TEXTURE_STRIDE;
        let fmt = (location & 0x3)
            | ((dimension & 0xF) << 4)
            | ((format & 0xFF) << 8)
            | ((mipmap & 0xFFFF) << 16);
        self.method1(M_TEXTURE_OFFSET + base, offset);
        self.method1(M_TEXTURE_FORMAT + base, fmt);
        self.method1(M_TEXTURE_CONTROL0 + base, 0x8000_0000); // enable
        self.method1(
            M_TEXTURE_IMAGE_RECT + base,
            (u32::from(width) << 16) | u32::from(height),
        );
    }

    /// `rsxDrawVertexArray`: begin/end-bracketed array draw.
    pub fn draw_arrays(&mut self, primitive: u32, first: u32, count: u32) {
        debug_assert!(count >= 1 && count <= 256, "draw count 1..=256");
        self.method1(M_BEGIN_END, primitive);
        let arg = (first & 0x00FF_FFFF) | (((count - 1) & 0xFF) << 24);
        self.method1(M_DRAW_ARRAYS, arg);
        self.method1(M_BEGIN_END, 0); // end
    }

    /// `NV406E` semaphore release (frame-end marker).
    pub fn semaphore_release(&mut self, value: u32) {
        self.method1(M_SEMAPHORE_RELEASE, value);
    }
}

// =====================================================================
// R12.11a — command-buffer capture mechanism (Tier 3 foundation)
// =====================================================================

/// Errors from capturing a command buffer out of a memory image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    /// `get`/`put` not 4-byte aligned.
    Misaligned,
    /// `get > put` (the consumer can't be ahead of the producer in a
    /// single linear segment).
    GetPastPut,
    /// `[base+get .. base+put)` exceeds the memory image.
    OutOfBounds,
}

/// Models the RSX DMA control: where the command buffer lives in guest
/// memory and how far the producer (PUT) and consumer (GET) have
/// advanced. This is the cellGcm-side state a real fixture sets up via
/// `cellGcmInit` + `cellGcmFlush`; R12.11a captures the bytes the
/// homebrew's inline libgcm wrote into `[base+get .. base+put)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcmControl {
    /// Byte offset of the command buffer in the memory image.
    pub command_base: u32,
    /// Producer offset (relative to `command_base`) — how far the
    /// homebrew has written. = bytes of valid command data.
    pub put: u32,
    /// Consumer offset (relative to `command_base`). Normally 0 at
    /// capture time (nothing consumed yet).
    pub get: u32,
}

impl GcmControl {
    /// A control block for a command buffer at `command_base`, with the
    /// producer having written `put` bytes (consumer at 0).
    #[must_use]
    pub fn new(command_base: u32, put: u32) -> Self {
        Self { command_base, put, get: 0 }
    }
}

/// Capture the live command-stream bytes from a memory image: the
/// slice `mem[command_base+get .. command_base+put]`. This is exactly
/// what a real fixture capture reads after the homebrew flushes — the
/// returned bytes feed straight into the decoder (`replay_gcm` /
/// `FifoEngine`) with `put - get` as the decode PUT.
pub fn capture_command_buffer<'a>(
    mem: &'a [u8],
    control: &GcmControl,
) -> Result<&'a [u8], CaptureError> {
    if control.get & 0x3 != 0 || control.put & 0x3 != 0 {
        return Err(CaptureError::Misaligned);
    }
    if control.get > control.put {
        return Err(CaptureError::GetPastPut);
    }
    let start = control
        .command_base
        .checked_add(control.get)
        .ok_or(CaptureError::OutOfBounds)? as usize;
    let end = control
        .command_base
        .checked_add(control.put)
        .ok_or(CaptureError::OutOfBounds)? as usize;
    mem.get(start..end).ok_or(CaptureError::OutOfBounds)
}

impl GcmContext {
    /// Write this context's emitted command words into `mem` at
    /// `command_base` (big-endian), as a homebrew's inline libgcm
    /// would, and return the [`GcmControl`] describing the result
    /// (PUT = bytes written). `mem` must be large enough; returns
    /// [`CaptureError::OutOfBounds`] otherwise.
    ///
    /// Bridges the Tier-2 producer to the Tier-3 capture mechanism:
    /// emit → write-to-memory → `capture_command_buffer` → decoder,
    /// exercising the exact byte-snapshot path the real fixture uses.
    pub fn write_into(
        &self,
        mem: &mut [u8],
        command_base: u32,
    ) -> Result<GcmControl, CaptureError> {
        let bytes = self.finish();
        let start = command_base as usize;
        let end = start
            .checked_add(bytes.len())
            .ok_or(CaptureError::OutOfBounds)?;
        let dst = mem.get_mut(start..end).ok_or(CaptureError::OutOfBounds)?;
        dst.copy_from_slice(&bytes);
        Ok(GcmControl::new(command_base, bytes.len() as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method1_emits_header_and_arg() {
        let mut c = GcmContext::new();
        c.method1(M_SURFACE_FORMAT, 0x48);
        // header = (1<<18) | 0x0208, then arg.
        assert_eq!(c.words(), &[(1 << 18) | 0x0208, 0x48]);
    }

    #[test]
    fn multi_arg_method_run() {
        let mut c = GcmContext::new();
        c.method(M_SURFACE_CLIP_HORIZONTAL, &[0xAA, 0xBB]);
        assert_eq!(c.words(), &[(2 << 18) | 0x0200, 0xAA, 0xBB]);
    }

    #[test]
    fn put_and_finish_lengths() {
        let mut c = GcmContext::new();
        c.set_clear_color(0xFF00FF00);
        assert_eq!(c.put(), 8); // header + arg = 2 words
        assert_eq!(c.finish().len(), 8);
    }

    #[test]
    fn finish_is_big_endian() {
        let mut c = GcmContext::new();
        c.set_clear_color(0x1122_3344);
        let b = c.finish();
        // second word (the arg) is the clear color, big-endian.
        assert_eq!(&b[4..8], &[0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn draw_arrays_brackets_with_begin_end() {
        let mut c = GcmContext::new();
        c.draw_arrays(5, 0, 3);
        // BEGIN_END(5), DRAW_ARRAYS(first0,count3), BEGIN_END(0).
        assert_eq!(
            c.words(),
            &[
                (1 << 18) | 0x1808, 5,
                (1 << 18) | 0x1814, 0x0200_0000,
                (1 << 18) | 0x1808, 0,
            ]
        );
    }

    #[test]
    fn vertex_array_emits_format_and_offset() {
        let mut c = GcmContext::new();
        c.set_vertex_data_array(1, VTX_TYPE_UNORM8, 4, 4, 0x8000_2000);
        assert_eq!(
            c.words(),
            &[
                (1 << 18) | (0x1740 + 4), 0x0444, // format reg+1
                (1 << 18) | (0x1680 + 4), 0x8000_2000, // offset reg+1
            ]
        );
    }

    // ---- R12.11a: capture mechanism ------------------------------

    #[test]
    fn write_into_then_capture_round_trips_bytes() {
        let mut c = GcmContext::new();
        c.set_clear_color(0x1122_3344);
        c.clear_surface(0xF3);
        let emitted = c.finish();

        // memory image with the command buffer at offset 0x1000.
        let mut mem = vec![0u8; 0x4000];
        let control = c.write_into(&mut mem, 0x1000).unwrap();
        assert_eq!(control.command_base, 0x1000);
        assert_eq!(control.put, emitted.len() as u32);
        assert_eq!(control.get, 0);

        let captured = capture_command_buffer(&mem, &control).unwrap();
        assert_eq!(captured, emitted.as_slice());
    }

    #[test]
    fn capture_respects_get_offset() {
        let mut mem = vec![0u8; 0x100];
        // put 16 bytes of marker data at base 0x10.
        for (i, b) in mem[0x10..0x20].iter_mut().enumerate() {
            *b = i as u8;
        }
        let control = GcmControl { command_base: 0x10, get: 4, put: 12 };
        let captured = capture_command_buffer(&mem, &control).unwrap();
        // [base+4 .. base+12) = bytes 4..12 of the marker.
        assert_eq!(captured, &[4, 5, 6, 7, 8, 9, 10, 11]);
    }

    #[test]
    fn capture_rejects_misaligned_get_past_put_and_oob() {
        let mem = vec![0u8; 0x20];
        assert_eq!(
            capture_command_buffer(&mem, &GcmControl { command_base: 0, get: 1, put: 8 }),
            Err(CaptureError::Misaligned)
        );
        assert_eq!(
            capture_command_buffer(&mem, &GcmControl { command_base: 0, get: 12, put: 8 }),
            Err(CaptureError::GetPastPut)
        );
        assert_eq!(
            capture_command_buffer(&mem, &GcmControl { command_base: 0x18, get: 0, put: 0x20 }),
            Err(CaptureError::OutOfBounds)
        );
    }

    #[test]
    fn write_into_out_of_bounds() {
        let mut c = GcmContext::new();
        c.set_clear_color(0); // 8 bytes
        let mut mem = vec![0u8; 4]; // too small
        assert_eq!(c.write_into(&mut mem, 0), Err(CaptureError::OutOfBounds));
    }
}
