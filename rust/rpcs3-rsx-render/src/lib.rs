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

// =====================================================================
// Triangle rasterization (deterministic software reference)
// =====================================================================

/// A vertex already in screen space (pixel coordinates). The RSX viewport
/// transform + vertex program that produce these are a later slice; this
/// rasterizer takes screen-space vertices directly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScreenVertex {
    pub x: f32,
    pub y: f32,
}

impl ScreenVertex {
    #[must_use]
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Twice the signed area of triangle (a, b, p) — the edge function. Sign tells
/// which side of edge a→b the point p is on.
#[inline]
fn edge(a: ScreenVertex, b: ScreenVertex, px: f32, py: f32) -> f32 {
    (px - a.x) * (b.y - a.y) - (py - a.y) * (b.x - a.x)
}

/// Rasterize a flat-colored triangle into a 32-bit (A8R8G8B8) framebuffer using
/// the standard edge-function coverage test, sampling pixel centers at
/// `(x+0.5, y+0.5)`, over the triangle's framebuffer-clamped bounding box.
/// Winding-agnostic (orients by the signed area). Returns the number of pixels
/// written.
///
/// This is a DETERMINISTIC software reference — not byte-exact vs RPCS3's GPU
/// rasterizer (which differs at sub-pixel edges per the driver). The golden is
/// the coverage the edge-function rule produces, hand-verifiable for simple
/// triangles. (Shared-edge double-coverage between adjacent triangles — the
/// top-left fill rule — is a refinement for the strip/mesh slice.)
pub fn rasterize_triangle_flat(
    verts: [ScreenVertex; 3],
    color: u32,
    fb: &mut [u8],
    width: u32,
    height: u32,
    pitch: u32,
) -> usize {
    let [v0, v1, v2] = verts;
    let area = edge(v0, v1, v2.x, v2.y);
    if area == 0.0 {
        return 0; // degenerate
    }
    let ccw = area > 0.0;
    let clampx = |v: f32| (v as i64).clamp(0, i64::from(width)) as u32;
    let clampy = |v: f32| (v as i64).clamp(0, i64::from(height)) as u32;
    let min_x = clampx(v0.x.min(v1.x).min(v2.x).floor());
    let max_x = clampx(v0.x.max(v1.x).max(v2.x).ceil());
    let min_y = clampy(v0.y.min(v1.y).min(v2.y).floor());
    let max_y = clampy(v0.y.max(v1.y).max(v2.y).ceil());
    let px = color.to_be_bytes();
    let mut count = 0;
    for y in min_y..max_y {
        for x in min_x..max_x {
            let (cx, cy) = (f64::from(x) as f32 + 0.5, f64::from(y) as f32 + 0.5);
            let w0 = edge(v1, v2, cx, cy);
            let w1 = edge(v2, v0, cx, cy);
            let w2 = edge(v0, v1, cx, cy);
            let inside = if ccw {
                w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
            } else {
                w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
            };
            if inside {
                let off = (y * pitch + x * 4) as usize;
                if off + 4 <= fb.len() {
                    fb[off..off + 4].copy_from_slice(&px);
                    count += 1;
                }
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

    fn px(fb: &[u8], x: u32, y: u32, pitch: u32) -> u32 {
        let o = (y * pitch + x * 4) as usize;
        u32::from_be_bytes([fb[o], fb[o + 1], fb[o + 2], fb[o + 3]])
    }

    #[test]
    fn rasterize_right_triangle_coverage() {
        // 8x8 A8R8G8B8 fb (pitch 32). Right triangle (1,1)-(7,1)-(1,7), hypotenuse
        // x+y=8; pixel center (cx,cy) inside iff cx>=1, cy>=1, cx+cy<=8.
        let mut fb = vec![0u8; 8 * 32];
        let tri = [
            ScreenVertex::new(1.0, 1.0),
            ScreenVertex::new(7.0, 1.0),
            ScreenVertex::new(1.0, 7.0),
        ];
        let n = rasterize_triangle_flat(tri, 0xFF00_FF00, &mut fb, 8, 8, 32);
        // Interior pixel filled; far-corner outside the hypotenuse not filled.
        assert_eq!(px(&fb, 2, 2, 32), 0xFF00_FF00, "(2,2) interior");
        assert_eq!(px(&fb, 1, 1, 32), 0xFF00_FF00, "(1,1) near v0");
        assert_eq!(px(&fb, 6, 6, 32), 0, "(6,6) beyond hypotenuse");
        assert_eq!(px(&fb, 0, 0, 32), 0, "(0,0) outside (left/top of v0)");
        // Hand count of centers (cx=x+.5, cy=y+.5) with cx>=1, cy>=1, cx+cy<=8:
        //   y=1: x=1..6 within cx+cy<=8 -> 6; y=2:5; y=3:4; y=4:3; y=5:2; y=6:1 = 21.
        assert_eq!(n, 21, "covered pixel count");
    }

    #[test]
    fn rasterize_degenerate_triangle_is_noop() {
        let mut fb = vec![0u8; 8 * 32];
        let line = [
            ScreenVertex::new(0.0, 0.0),
            ScreenVertex::new(4.0, 0.0),
            ScreenVertex::new(8.0, 0.0), // collinear
        ];
        assert_eq!(rasterize_triangle_flat(line, 0xFFFF_FFFF, &mut fb, 8, 8, 32), 0);
        assert!(fb.iter().all(|&b| b == 0));
    }

    #[test]
    fn rasterize_clamps_to_framebuffer() {
        // A big triangle covering the whole 4x4 fb fills all 16 pixels (no OOB).
        let mut fb = vec![0u8; 4 * 16];
        let tri = [
            ScreenVertex::new(-10.0, -10.0),
            ScreenVertex::new(50.0, -10.0),
            ScreenVertex::new(-10.0, 50.0),
        ];
        let n = rasterize_triangle_flat(tri, 0xFFAB_CDEF, &mut fb, 4, 4, 16);
        assert_eq!(n, 16);
        assert!(fb.chunks_exact(4).all(|p| p == [0xFF, 0xAB, 0xCD, 0xEF]));
    }
}
