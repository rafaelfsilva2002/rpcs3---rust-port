//! Deterministic software RSX renderer — executes decoded NV4097 methods into a
//! render-target buffer. The first slice is `NV4097_CLEAR_SURFACE`.
//!
//! Unlike RPCS3's GPU backends (Vulkan/GL/D3D12, whose pixel output is driver-
//! dependent and not byte-reproducible), this is a deterministic CPU reference.
//! A CLEAR fills the surface with a constant value, so for the clear path it is
//! byte-exact with RPCS3 (which clears to the same color). Later slices (triangle
//! rasterization, texture sampling, fragment shaders) are the software reference
//! the behavior-freeze oracles validate against computed expectations.

use rpcs3_rsx_state::RsxState;

// NV4097_CLEAR_SURFACE mask bits (CELL_GCM_CLEAR_*).
pub const CLEAR_Z: u32 = 0x01;
pub const CLEAR_S: u32 = 0x02;
pub const CLEAR_R: u32 = 0x10;
pub const CLEAR_G: u32 = 0x20;
pub const CLEAR_B: u32 = 0x40;
pub const CLEAR_A: u32 = 0x80;
/// Any color channel selected for clear.
pub const CLEAR_COLOR: u32 = CLEAR_R | CLEAR_G | CLEAR_B | CLEAR_A;

// CELL_GCM_SURFACE color format codes (gcm_enums.h).
pub const SURFACE_R5G6B5: u8 = 3;
pub const SURFACE_X8R8G8B8_Z8R8G8B8: u8 = 4;
pub const SURFACE_X8R8G8B8_O8R8G8B8: u8 = 5;
pub const SURFACE_A8R8G8B8: u8 = 8;
pub const SURFACE_A8B8G8R8: u8 = 16;

/// Bytes per pixel of a color surface format.
#[must_use]
pub fn bytes_per_pixel(format: u8) -> usize {
    match format {
        1 | 2 | SURFACE_R5G6B5 => 2, // 16-bit
        _ => 4,                      // 32-bit (A8R8G8B8 / X8R8G8B8 / A8B8G8R8 / ...)
    }
}

/// The per-pixel clear bytes for `format`. 32-bit formats store the clear value
/// big-endian ([A,R,G,B]); 16-bit takes the low half (BE).
#[must_use]
fn pixel_bytes(color: u32, format: u8) -> [u8; 4] {
    let be = color.to_be_bytes();
    if bytes_per_pixel(format) == 2 {
        [be[2], be[3], 0, 0]
    } else {
        be
    }
}

/// Execute `NV4097_CLEAR_SURFACE` into `mem` (the RSX local-memory buffer that the
/// surface `color_offset` indexes). Fills the slot-A color buffer across the
/// surface-clip rectangle, pitch-strided, with the clear value. Returns the
/// number of pixels written (0 when the mask selects no color channel).
pub fn execute_clear(state: &RsxState, mem: &mut [u8]) -> usize {
    if state.clear_surface_mask() & CLEAR_COLOR == 0 {
        return 0;
    }
    let sd = state.surface();
    let (w, h) = (sd.clip.0 as usize, sd.clip.1 as usize);
    let offset = sd.color_offset[0] as usize;
    let pitch = sd.color_pitch[0] as usize;
    let bpp = bytes_per_pixel(sd.color_format);
    let px = pixel_bytes(state.color_clear_value(), sd.color_format);
    let mut count = 0;
    for y in 0..h {
        let row = offset + y * pitch;
        for x in 0..w {
            let p = row + x * bpp;
            if p + bpp <= mem.len() {
                mem[p..p + bpp].copy_from_slice(&px[..bpp]);
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use rpcs3_rsx_state::{
        RsxState, CLEAR_SURFACE, COLOR_CLEAR_VALUE, SURFACE_CLIP_HORIZONTAL,
        SURFACE_CLIP_VERTICAL, SURFACE_COLOR_A_OFFSET, SURFACE_FORMAT, SURFACE_PITCH_A,
    };

    fn setup_4x4_a8r8g8b8(color: u32, mask: u32) -> RsxState {
        let mut s = RsxState::new();
        s.write(SURFACE_FORMAT, u32::from(SURFACE_A8R8G8B8));
        s.write(SURFACE_PITCH_A, 16); // 4 px * 4 bpp
        s.write(SURFACE_COLOR_A_OFFSET, 0);
        s.write(SURFACE_CLIP_HORIZONTAL, 4 << 16); // width 4 (high half)
        s.write(SURFACE_CLIP_VERTICAL, 4 << 16); // height 4
        s.write(COLOR_CLEAR_VALUE, color);
        s.write(CLEAR_SURFACE, mask);
        s
    }

    #[test]
    fn clear_fills_a8r8g8b8_surface() {
        let s = setup_4x4_a8r8g8b8(0xFF11_2233, CLEAR_COLOR);
        let mut mem = vec![0u8; 4 * 4 * 4];
        assert_eq!(execute_clear(&s, &mut mem), 16);
        for px in mem.chunks_exact(4) {
            assert_eq!(px, [0xFF, 0x11, 0x22, 0x33]); // ARGB big-endian
        }
    }

    #[test]
    fn clear_with_no_color_mask_is_noop() {
        // Z-only clear must not touch the color buffer.
        let s = setup_4x4_a8r8g8b8(0xFF11_2233, CLEAR_Z | CLEAR_S);
        let mut mem = vec![0u8; 4 * 4 * 4];
        assert_eq!(execute_clear(&s, &mut mem), 0);
        assert!(mem.iter().all(|&b| b == 0));
    }

    #[test]
    fn clear_respects_pitch_padding() {
        // pitch 24 (> 16) leaves 8 padding bytes per row untouched.
        let mut s = setup_4x4_a8r8g8b8(0x0102_0304, CLEAR_COLOR);
        s.write(SURFACE_PITCH_A, 24);
        let mut mem = vec![0u8; 24 * 4];
        assert_eq!(execute_clear(&s, &mut mem), 16);
        // Row 0: 16 bytes of color, then 8 zero padding bytes.
        assert_eq!(&mem[0..4], [0x01, 0x02, 0x03, 0x04]);
        assert_eq!(&mem[16..24], [0u8; 8]);
    }
}
