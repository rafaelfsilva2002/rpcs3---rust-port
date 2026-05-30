//! Vendored stb_image.h decoder (JPEG/PNG -> forced RGBA), behind the `decode`
//! feature. This is the SAME decoder RPCS3 cellJpgDec links
//! (`stbi_load_from_memory(..., 4)`), so the pixel output is byte-exact. Default
//! builds (feature off) keep the workspace pure-Rust and return `None`.

#[cfg(feature = "decode")]
mod ffi {
    extern "C" {
        pub fn stbi_shim_load_rgba(buf: *const u8, len: i32, w: *mut i32, h: *mut i32) -> *mut u8;
        pub fn stbi_shim_free(p: *mut u8);
    }
}

/// Decode JPEG/PNG `bytes` to a forced-RGBA pixel buffer (`width*height*4`),
/// mirroring RPCS3's `stbi_load_from_memory(..., &comp, 4)`. Returns
/// `(width, height, rgba)`, or `None` on a decode failure or when the `decode`
/// feature is off (no stb_image is linked, so nothing can be decoded).
#[must_use]
pub fn decode_rgba(bytes: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    #[cfg(not(feature = "decode"))]
    {
        let _ = bytes;
        None
    }
    #[cfg(feature = "decode")]
    {
        if bytes.is_empty() || bytes.len() > i32::MAX as usize {
            return None;
        }
        let (mut w, mut h) = (0i32, 0i32);
        // SAFETY: `bytes` is a valid slice; the shim only reads `len` bytes and
        // returns an stb-malloc'd buffer of w*h*4 we copy then free.
        unsafe {
            let ptr = ffi::stbi_shim_load_rgba(bytes.as_ptr(), bytes.len() as i32, &mut w, &mut h);
            if ptr.is_null() || w <= 0 || h <= 0 {
                if !ptr.is_null() {
                    ffi::stbi_shim_free(ptr);
                }
                return None;
            }
            let n = (w as usize) * (h as usize) * 4;
            let pixels = core::slice::from_raw_parts(ptr, n).to_vec();
            ffi::stbi_shim_free(ptr);
            Some((w as u32, h as u32, pixels))
        }
    }
}
