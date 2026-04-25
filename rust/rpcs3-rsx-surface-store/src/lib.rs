//! `rpcs3-rsx-surface-store` — Rust port of
//! `rpcs3/Emu/RSX/Common/surface_store.cpp`.
//!
//! Multi-render-target helpers: which RSX render targets are active for a
//! given `surface_target` enum value, and the pitch math for each color
//! format. Byte-exact against cpp:10..80 and the RSX enum values in
//! `gcm_enums.h:910..941`.
//!
//! Frozen:
//!
//! - `SurfaceTarget` enum: `None=0, A=1, B=2, MRT1=0x13, MRT2=0x17, MRT3=0x1F`.
//! - `SurfaceColorFormat` enum: B8/G8B8/X1R5G5B5/R5G6B5/X8R8G8B8/A8R8G8B8
//!   etc with raw GCM values.
//! - `get_rtt_indexes(target) -> &[u8]` table (cpp:10..22).
//! - `get_mrt_buffers_count(target) -> u8` (cpp:24..36).
//! - `get_aligned_pitch(format, width)` → 256-byte aligned pitch
//!   (cpp:38..58).
//! - `get_packed_pitch(format, width)` → unaligned byte count
//!   (cpp:60..80).

/// `surface_target` from `gcm_enums.h:1266..1274` with raw CELL_GCM values.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceTarget {
    None = 0,
    SurfaceA = 1,
    SurfaceB = 2,
    SurfacesAB = 0x13,
    SurfacesABC = 0x17,
    SurfacesABCD = 0x1F,
}

/// `surface_color_format` from `gcm_enums.h:928..941`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceColorFormat {
    X1R5G5B5_Z1R5G5B5 = 1,
    X1R5G5B5_O1R5G5B5 = 2,
    R5G6B5 = 3,
    X8R8G8B8_Z8R8G8B8 = 4,
    X8R8G8B8_O8R8G8B8 = 5,
    A8R8G8B8 = 8,
    B8 = 9,
    G8B8 = 10,
    W16Z16Y16X16 = 11,
    W32Z32Y32X32 = 12,
    X32 = 13,
    X8B8G8R8_Z8B8G8R8 = 14,
    X8B8G8R8_O8B8G8R8 = 15,
    A8B8G8R8 = 16,
}

/// Indices into the MRT binding list for each surface_target value
/// (cpp:10..22).
#[must_use]
pub const fn get_rtt_indexes(target: SurfaceTarget) -> &'static [u8] {
    match target {
        SurfaceTarget::None => &[],
        SurfaceTarget::SurfaceA => &[0],
        SurfaceTarget::SurfaceB => &[1],
        SurfaceTarget::SurfacesAB => &[0, 1],
        SurfaceTarget::SurfacesABC => &[0, 1, 2],
        SurfaceTarget::SurfacesABCD => &[0, 1, 2, 3],
    }
}

/// Number of active color attachments for a `surface_target` (cpp:24..36).
#[must_use]
pub const fn get_mrt_buffers_count(target: SurfaceTarget) -> u8 {
    match target {
        SurfaceTarget::None => 0,
        SurfaceTarget::SurfaceA => 1,
        SurfaceTarget::SurfaceB => 1,
        SurfaceTarget::SurfacesAB => 2,
        SurfaceTarget::SurfacesABC => 3,
        SurfaceTarget::SurfacesABCD => 4,
    }
}

/// Bytes-per-pixel for each color format (internal helper).
#[must_use]
const fn bytes_per_pixel(format: SurfaceColorFormat) -> usize {
    match format {
        SurfaceColorFormat::B8 => 1,
        SurfaceColorFormat::G8B8
        | SurfaceColorFormat::X1R5G5B5_O1R5G5B5
        | SurfaceColorFormat::X1R5G5B5_Z1R5G5B5
        | SurfaceColorFormat::R5G6B5 => 2,
        SurfaceColorFormat::A8B8G8R8
        | SurfaceColorFormat::X8B8G8R8_O8B8G8R8
        | SurfaceColorFormat::X8B8G8R8_Z8B8G8R8
        | SurfaceColorFormat::X8R8G8B8_O8R8G8B8
        | SurfaceColorFormat::X8R8G8B8_Z8R8G8B8
        | SurfaceColorFormat::X32
        | SurfaceColorFormat::A8R8G8B8 => 4,
        SurfaceColorFormat::W16Z16Y16X16 => 8,
        SurfaceColorFormat::W32Z32Y32X32 => 16,
    }
}

/// Round `value` up to the nearest multiple of `alignment`.
#[must_use]
const fn align_up(value: usize, alignment: usize) -> usize {
    value.div_ceil(alignment) * alignment
}

/// `get_aligned_pitch(format, width)` (cpp:38..58). Rounds the packed
/// pitch up to 256 bytes (RSX's minimum surface pitch alignment).
#[must_use]
pub const fn get_aligned_pitch(format: SurfaceColorFormat, width: u32) -> usize {
    let bytes = bytes_per_pixel(format) * width as usize;
    align_up(bytes, 256)
}

/// `get_packed_pitch(format, width)` (cpp:60..80). No alignment padding.
#[must_use]
pub const fn get_packed_pitch(format: SurfaceColorFormat, width: u32) -> usize {
    bytes_per_pixel(format) * width as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_target_raw_values() {
        assert_eq!(SurfaceTarget::None as u8, 0);
        assert_eq!(SurfaceTarget::SurfaceA as u8, 1);
        assert_eq!(SurfaceTarget::SurfaceB as u8, 2);
        assert_eq!(SurfaceTarget::SurfacesAB as u8, 0x13);
        assert_eq!(SurfaceTarget::SurfacesABC as u8, 0x17);
        assert_eq!(SurfaceTarget::SurfacesABCD as u8, 0x1F);
    }

    #[test]
    fn surface_color_format_raw_values() {
        assert_eq!(SurfaceColorFormat::B8 as u8, 9);
        assert_eq!(SurfaceColorFormat::A8R8G8B8 as u8, 8);
        assert_eq!(SurfaceColorFormat::A8B8G8R8 as u8, 16);
        assert_eq!(SurfaceColorFormat::W32Z32Y32X32 as u8, 12);
    }

    #[test]
    fn rtt_indexes_tables() {
        assert_eq!(get_rtt_indexes(SurfaceTarget::None), &[]);
        assert_eq!(get_rtt_indexes(SurfaceTarget::SurfaceA), &[0]);
        assert_eq!(get_rtt_indexes(SurfaceTarget::SurfaceB), &[1]);
        assert_eq!(get_rtt_indexes(SurfaceTarget::SurfacesAB), &[0, 1]);
        assert_eq!(get_rtt_indexes(SurfaceTarget::SurfacesABC), &[0, 1, 2]);
        assert_eq!(get_rtt_indexes(SurfaceTarget::SurfacesABCD), &[0, 1, 2, 3]);
    }

    #[test]
    fn mrt_buffer_counts() {
        assert_eq!(get_mrt_buffers_count(SurfaceTarget::None), 0);
        assert_eq!(get_mrt_buffers_count(SurfaceTarget::SurfaceA), 1);
        assert_eq!(get_mrt_buffers_count(SurfaceTarget::SurfaceB), 1);
        assert_eq!(get_mrt_buffers_count(SurfaceTarget::SurfacesAB), 2);
        assert_eq!(get_mrt_buffers_count(SurfaceTarget::SurfacesABC), 3);
        assert_eq!(get_mrt_buffers_count(SurfaceTarget::SurfacesABCD), 4);
    }

    #[test]
    fn aligned_pitch_b8_rounds_up_to_256() {
        // 100 bytes → aligned to 256.
        assert_eq!(get_aligned_pitch(SurfaceColorFormat::B8, 100), 256);
        // Exactly 256 stays at 256.
        assert_eq!(get_aligned_pitch(SurfaceColorFormat::B8, 256), 256);
        // 257 → 512.
        assert_eq!(get_aligned_pitch(SurfaceColorFormat::B8, 257), 512);
    }

    #[test]
    fn aligned_pitch_4bpp_formats() {
        // width 100 * 4bpp = 400 → 512.
        assert_eq!(get_aligned_pitch(SurfaceColorFormat::A8R8G8B8, 100), 512);
        assert_eq!(get_aligned_pitch(SurfaceColorFormat::X32, 100), 512);
        // width 64 * 4bpp = 256 — already aligned.
        assert_eq!(get_aligned_pitch(SurfaceColorFormat::A8R8G8B8, 64), 256);
    }

    #[test]
    fn aligned_pitch_16bpp_wide_formats() {
        // W32Z32Y32X32 is 16 bytes per pixel.
        assert_eq!(get_aligned_pitch(SurfaceColorFormat::W32Z32Y32X32, 16), 256);
        assert_eq!(get_aligned_pitch(SurfaceColorFormat::W32Z32Y32X32, 17), 512);
    }

    #[test]
    fn packed_pitch_no_alignment() {
        assert_eq!(get_packed_pitch(SurfaceColorFormat::B8, 100), 100);
        assert_eq!(get_packed_pitch(SurfaceColorFormat::G8B8, 100), 200);
        assert_eq!(get_packed_pitch(SurfaceColorFormat::A8R8G8B8, 100), 400);
        assert_eq!(get_packed_pitch(SurfaceColorFormat::W16Z16Y16X16, 100), 800);
        assert_eq!(get_packed_pitch(SurfaceColorFormat::W32Z32Y32X32, 100), 1600);
    }

    #[test]
    fn packed_pitch_is_divides_aligned_pitch() {
        for fmt in [SurfaceColorFormat::B8, SurfaceColorFormat::R5G6B5, SurfaceColorFormat::A8R8G8B8] {
            for w in [1u32, 64, 100, 256, 1024, 1920] {
                let packed = get_packed_pitch(fmt, w);
                let aligned = get_aligned_pitch(fmt, w);
                assert!(aligned >= packed, "fmt {:?} w {} aligned {} < packed {}", fmt, w, aligned, packed);
                assert_eq!(aligned % 256, 0);
            }
        }
    }
}
