//! `rpcs3-hle-cellfont` — text rendering HLE.
//!
//! Ports `rpcs3/Emu/Cell/Modules/cellFont.cpp`. Games use this module
//! to render Japanese/Latin glyphs for in-game text (menus, HUDs,
//! dialog). The API is split into three object layers:
//!
//! 1. **Library** — initialised once per process via `cellFontInit`.
//! 2. **Font** — opened from either a system type (Rodin/Matisse/etc)
//!    or a TTF/OTF file path.
//! 3. **Renderer** — bound to a font, produces glyph bitmaps.
//!
//! Our port tracks all three with opaque handles and uses a
//! [`FontBackend`] trait for actual glyph rasterisation.
//!
//! ## Entry points covered
//!
//! | HLE function                       | Rust wrapper                        |
//! |------------------------------------|-------------------------------------|
//! | `cellFontInit`                     | [`cell_font_init`]                  |
//! | `cellFontEnd`                      | [`cell_font_end`]                   |
//! | `cellFontOpenFontset`              | [`cell_font_open_fontset`]          |
//! | `cellFontOpenFontFile`             | [`cell_font_open_font_file`]        |
//! | `cellFontCloseFont`                | [`cell_font_close_font`]            |
//! | `cellFontCreateRenderer`           | [`cell_font_create_renderer`]       |
//! | `cellFontDestroyRenderer`          | [`cell_font_destroy_renderer`]      |
//! | `cellFontBindRenderer`             | [`cell_font_bind_renderer`]         |
//! | `cellFontUnbindRenderer`           | [`cell_font_unbind_renderer`]       |
//! | `cellFontRenderCharGlyphImage`     | [`cell_font_render_char_glyph_image`] |
//! | `cellFontGetHorizontalLayout`      | [`cell_font_get_horizontal_layout`] |
//! | `cellFontSetScalePixel`            | [`cell_font_set_scale_pixel`]       |

use rpcs3_emu_types::CellError;

// =====================================================================
// Error codes — byte-exact with cellFont.h:8-32
// =====================================================================

pub mod errors {
    use rpcs3_emu_types::CellError;

    pub const FATAL: CellError = CellError(0x8054_0001);
    pub const INVALID_PARAMETER: CellError = CellError(0x8054_0002);
    pub const UNINITIALIZED: CellError = CellError(0x8054_0003);
    pub const INITIALIZE_FAILED: CellError = CellError(0x8054_0004);
    pub const INVALID_CACHE_BUFFER: CellError = CellError(0x8054_0005);
    pub const ALREADY_INITIALIZED: CellError = CellError(0x8054_0006);
    pub const ALLOCATION_FAILED: CellError = CellError(0x8054_0007);
    pub const NO_SUPPORT_FONTSET: CellError = CellError(0x8054_0008);
    pub const OPEN_FAILED: CellError = CellError(0x8054_0009);
    pub const READ_FAILED: CellError = CellError(0x8054_000A);
    pub const FONT_OPEN_FAILED: CellError = CellError(0x8054_000B);
    pub const FONT_NOT_FOUND: CellError = CellError(0x8054_000C);
    pub const FONT_OPEN_MAX: CellError = CellError(0x8054_000D);
    pub const FONT_CLOSE_FAILED: CellError = CellError(0x8054_000E);
    pub const ALREADY_OPENED: CellError = CellError(0x8054_000F);
    pub const NO_SUPPORT_FUNCTION: CellError = CellError(0x8054_0010);
    pub const NO_SUPPORT_CODE: CellError = CellError(0x8054_0011);
    pub const NO_SUPPORT_GLYPH: CellError = CellError(0x8054_0012);
    pub const BUFFER_SIZE_NOT_ENOUGH: CellError = CellError(0x8054_0016);
    pub const RENDERER_ALREADY_BIND: CellError = CellError(0x8054_0020);
    pub const RENDERER_UNBIND: CellError = CellError(0x8054_0021);
    pub const RENDERER_INVALID: CellError = CellError(0x8054_0022);
    pub const RENDERER_ALLOCATION_FAILED: CellError = CellError(0x8054_0023);
    pub const ENOUGH_RENDERING_BUFFER: CellError = CellError(0x8054_0024);
    pub const NO_SUPPORT_SURFACE: CellError = CellError(0x8054_0040);
}

// =====================================================================
// Font-type enum (subset of most-used PS3 system fonts)
// =====================================================================

pub const TYPE_RODIN_SANS_SERIF_LATIN: u32 = 0x00;
pub const TYPE_RODIN_SANS_SERIF_LIGHT_LATIN: u32 = 0x01;
pub const TYPE_RODIN_SANS_SERIF_BOLD_LATIN: u32 = 0x02;
pub const TYPE_NEWRODIN_GOTHIC_JAPANESE: u32 = 0x08;
pub const TYPE_NEWRODIN_GOTHIC_LIGHT_JAPANESE: u32 = 0x09;
pub const TYPE_NEWRODIN_GOTHIC_BOLD_JAPANESE: u32 = 0x0A;
pub const TYPE_YD_GOTHIC_KOREAN: u32 = 0x0C;
pub const TYPE_RODIN_SANS_SERIF_LATIN2: u32 = 0x18;
pub const TYPE_MATISSE_SERIF_LATIN: u32 = 0x20;
pub const TYPE_SEURAT_MARU_GOTHIC_LATIN: u32 = 0x40;
pub const TYPE_VAGR_SANS_SERIF_ROUND: u32 = 0x43;

/// Upper limit on concurrent open fonts (matches C++).
pub const MAX_FONTS: u32 = 16;

// =====================================================================
// Data model
// =====================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HorizontalLayout {
    pub base_line_y: f32,
    pub line_height: f32,
    pub effect_height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphMetrics {
    pub width: u32,
    pub height: u32,
    pub advance_x: f32,
    pub advance_y: f32,
}

/// Produced by [`cell_font_render_char_glyph_image`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlyphImage {
    pub width: u32,
    pub height: u32,
    /// 8-bit grayscale coverage bitmap, length == width*height.
    pub pixels: Vec<u8>,
}

// =====================================================================
// Byte-exact glyph metrics (stb_truetype — the engine RPCS3 cellFont uses)
// =====================================================================

/// Horizontal layout matching `CellFontHorizontalLayout` (cellFont.cpp:536):
/// three f32 fields (BE in guest memory).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellHorizontalLayout {
    pub base_line_y: f32,
    pub line_height: f32,
    pub effect_height: f32,
}

/// Glyph metrics matching `CellFontGlyphMetrics` (cellFont.cpp:894-901): eight
/// f32 fields (BE in guest memory). Produced bit-for-bit by [`StbttFont`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CellGlyphMetrics {
    pub width: f32,
    pub height: f32,
    pub h_bearing_x: f32,
    pub h_bearing_y: f32,
    pub h_advance: f32,
    pub v_bearing_x: f32,
    pub v_bearing_y: f32,
    pub v_advance: f32,
}

/// A parsed TrueType font via `stb_truetype` — the byte-exact engine RPCS3's
/// cellFont links (`#include <stb_truetype.h>`). The host-side analogue of
/// RPCS3's in-guest `stbtt_fontinfo` hack (cellFont.cpp:138).
pub struct StbttFont {
    info: stb_truetype::FontInfo<Vec<u8>>,
    /// Raw font bytes — kept only for the rasterizer (the C shim re-inits the
    /// font from these). Pure-metrics builds don't carry them.
    #[cfg(feature = "cellfont-raster")]
    data: Vec<u8>,
}

impl core::fmt::Debug for StbttFont {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("StbttFont(..)")
    }
}

impl StbttFont {
    /// Parse a font from memory (mirrors `stbtt_InitFont`). `None` on failure —
    /// the caller maps that to `CELL_FONT_ERROR_FONT_OPEN_FAILED`
    /// (cellFont.cpp:140-143).
    #[must_use]
    pub fn open(data: Vec<u8>) -> Option<Self> {
        #[cfg(feature = "cellfont-raster")]
        {
            let info = stb_truetype::FontInfo::new(data.clone(), 0)?;
            Some(Self { info, data })
        }
        #[cfg(not(feature = "cellfont-raster"))]
        {
            stb_truetype::FontInfo::new(data, 0).map(|info| Self { info })
        }
    }

    /// Byte-exact port of `cellFontGetHorizontalLayout` (cellFont.cpp:552-558):
    /// `baseLineY = ascent*scale`, `lineHeight = (ascent-descent+lineGap)*scale`,
    /// `effectHeight = lineGap*scale`, with `scale = scale_for_pixel_height(scale_y)`.
    #[must_use]
    pub fn horizontal_layout(&self, scale_y: f32) -> CellHorizontalLayout {
        let scale = self.info.scale_for_pixel_height(scale_y);
        let v = self.info.get_v_metrics();
        CellHorizontalLayout {
            base_line_y: v.ascent as f32 * scale,
            line_height: (v.ascent - v.descent + v.line_gap) as f32 * scale,
            effect_height: v.line_gap as f32 * scale,
        }
    }

    /// Byte-exact port of `cellFontGetCharGlyphMetrics` (cellFont.cpp:887-901).
    /// `scale_y` is `CellFont.scale_y` (set by `cellFontSetScalePixel`); the
    /// computation is `(int table value) * scale` in IEEE-754 single, identical
    /// to RPCS3's host-side stb_truetype.
    #[must_use]
    pub fn char_glyph_metrics(&self, code: u32, scale_y: f32) -> CellGlyphMetrics {
        let scale = self.info.scale_for_pixel_height(scale_y);
        let (x0, y0, x1, y1) = match self.info.get_codepoint_box(code) {
            Some(r) => (
                i32::from(r.x0),
                i32::from(r.y0),
                i32::from(r.x1),
                i32::from(r.y1),
            ),
            None => (0, 0, 0, 0),
        };
        let hm = self.info.get_codepoint_h_metrics(code);
        CellGlyphMetrics {
            width: (x1 - x0) as f32 * scale,
            height: (y1 - y0) as f32 * scale,
            h_bearing_x: hm.left_side_bearing as f32 * scale,
            h_bearing_y: 0.0,
            h_advance: hm.advance_width as f32 * scale,
            v_bearing_x: 0.0,
            v_bearing_y: 0.0,
            v_advance: 0.0,
        }
    }

    /// Rasterize `code` to an 8-bit coverage bitmap via the vendored C
    /// stb_truetype (feature `cellfont-raster`) — the SAME `stbtt_GetCodepointBitmap`
    /// RPCS3 calls (cellFont.cpp:713). Returns `None` when the feature is off or
    /// the glyph has no bitmap (stbtt returns null, cellFont.cpp:715).
    #[cfg(feature = "cellfont-raster")]
    #[must_use]
    pub fn render_glyph_coverage(&self, code: u32, scale_y: f32) -> Option<GlyphCoverage> {
        let (mut width, mut height, mut xoff, mut yoff, mut base_line_y) = (0, 0, 0, 0, 0);
        // SAFETY: `data` is a valid font buffer; the shim writes only the out
        // params and returns an stbtt-owned bitmap we copy then free.
        let cov = unsafe {
            let ptr = raster_ffi::stbtt_shim_render(
                self.data.as_ptr(),
                0,
                code,
                scale_y,
                &mut width,
                &mut height,
                &mut xoff,
                &mut yoff,
                &mut base_line_y,
            );
            if ptr.is_null() {
                return None;
            }
            let len = (width.max(0) as usize) * (height.max(0) as usize);
            let pixels = core::slice::from_raw_parts(ptr, len).to_vec();
            raster_ffi::stbtt_shim_free(ptr);
            pixels
        };
        Some(GlyphCoverage {
            width,
            height,
            xoff,
            yoff,
            base_line_y,
            pixels: cov,
        })
    }

    /// Pure-Rust build: no rasterizer (the `stb_truetype` crate has none), so
    /// rendering is unavailable. Mirrors stbtt returning a null bitmap.
    #[cfg(not(feature = "cellfont-raster"))]
    #[must_use]
    pub fn render_glyph_coverage(&self, _code: u32, _scale_y: f32) -> Option<GlyphCoverage> {
        None
    }

    /// Render `code` into an 8-bit grayscale surface, byte-exact to RPCS3's
    /// `cellFontRenderCharGlyphImage` blit (cellFont.cpp:726-740): the surface is
    /// `surface_w`-strided, `surface_h` rows; the coverage lands at row
    /// `y + ypos + yoff + baseLineY`, col `x + xpos`, with the same u32-cast
    /// bounds checks. No-op (returns false) when there is no bitmap (or the
    /// `cellfont-raster` feature is off). Shared by the emu-core arm and the
    /// calibration test so they cannot drift.
    pub fn render_into(
        &self,
        code: u32,
        scale_y: f32,
        surface: &mut [u8],
        surface_w: u32,
        surface_h: u32,
        x: f32,
        y: f32,
    ) -> bool {
        let Some(cov) = self.render_glyph_coverage(code, scale_y) else {
            return false;
        };
        let (w, h) = (cov.width.max(0) as u32, cov.height.max(0) as u32);
        for ypos in 0..h {
            // (u32)y + ypos + yoff + baseLineY >= (u32)surface_h  (cellFont.cpp:729)
            let row = (y as u32)
                .wrapping_add(ypos)
                .wrapping_add(cov.yoff as u32)
                .wrapping_add(cov.base_line_y as u32);
            if row >= surface_h {
                break;
            }
            for xpos in 0..w {
                let col = (x as u32).wrapping_add(xpos);
                if col >= surface_w {
                    break;
                }
                let idx = (row as usize) * (surface_w as usize) + (col as usize);
                let src = (ypos as usize) * (w as usize) + (xpos as usize);
                if idx < surface.len() && src < cov.pixels.len() {
                    surface[idx] = cov.pixels[src];
                }
            }
        }
        true
    }
}

/// A rasterized glyph: stbtt's 8-bit coverage bitmap + placement offsets and the
/// `baseLineY` cellFont uses (`(int)(ascent*scale)`, cellFont.cpp:723).
#[derive(Debug, Clone)]
pub struct GlyphCoverage {
    pub width: i32,
    pub height: i32,
    pub xoff: i32,
    pub yoff: i32,
    pub base_line_y: i32,
    pub pixels: Vec<u8>,
}

#[cfg(feature = "cellfont-raster")]
mod raster_ffi {
    extern "C" {
        pub fn stbtt_shim_render(
            font: *const u8,
            fontoffset: i32,
            code: u32,
            scale_y: f32,
            width: *mut i32,
            height: *mut i32,
            xoff: *mut i32,
            yoff: *mut i32,
            base_line_y: *mut i32,
        ) -> *mut u8;
        pub fn stbtt_shim_free(p: *mut u8);
    }
}

// =====================================================================
// Backend trait
// =====================================================================

pub trait FontBackend {
    /// Open a system-supplied font; `type_code` is one of the
    /// `TYPE_*` constants. Returns some backend-specific handle.
    fn open_system_font(&mut self, type_code: u32) -> Result<u64, CellError>;
    /// Open a font from a file path (TTF/OTF).
    fn open_font_file(&mut self, path: &str) -> Result<u64, CellError>;
    /// Release a backend font handle.
    fn close_font(&mut self, handle: u64) -> Result<(), CellError>;

    fn horizontal_layout(&self, handle: u64) -> Result<HorizontalLayout, CellError>;
    fn render_glyph(
        &mut self,
        handle: u64,
        code_point: u32,
        scale_px: f32,
    ) -> Result<GlyphImage, CellError>;
}

/// Deterministic in-memory backend — every glyph is a filled square.
#[derive(Debug, Default)]
pub struct StubFontBackend {
    next_handle: u64,
    opened: std::collections::BTreeSet<u64>,
}

impl FontBackend for StubFontBackend {
    fn open_system_font(&mut self, type_code: u32) -> Result<u64, CellError> {
        if !matches!(
            type_code,
            TYPE_RODIN_SANS_SERIF_LATIN
            | TYPE_RODIN_SANS_SERIF_LIGHT_LATIN
            | TYPE_RODIN_SANS_SERIF_BOLD_LATIN
            | TYPE_NEWRODIN_GOTHIC_JAPANESE
            | TYPE_NEWRODIN_GOTHIC_LIGHT_JAPANESE
            | TYPE_NEWRODIN_GOTHIC_BOLD_JAPANESE
            | TYPE_YD_GOTHIC_KOREAN
            | TYPE_RODIN_SANS_SERIF_LATIN2
            | TYPE_MATISSE_SERIF_LATIN
            | TYPE_SEURAT_MARU_GOTHIC_LATIN
            | TYPE_VAGR_SANS_SERIF_ROUND,
        ) {
            return Err(errors::NO_SUPPORT_FONTSET);
        }
        self.next_handle += 1;
        let h = self.next_handle;
        self.opened.insert(h);
        Ok(h)
    }
    fn open_font_file(&mut self, path: &str) -> Result<u64, CellError> {
        if path.is_empty() || !path.starts_with('/') {
            return Err(errors::OPEN_FAILED);
        }
        self.next_handle += 1;
        let h = self.next_handle;
        self.opened.insert(h);
        Ok(h)
    }
    fn close_font(&mut self, handle: u64) -> Result<(), CellError> {
        if !self.opened.remove(&handle) {
            return Err(errors::FONT_NOT_FOUND);
        }
        Ok(())
    }
    fn horizontal_layout(&self, _handle: u64) -> Result<HorizontalLayout, CellError> {
        Ok(HorizontalLayout {
            base_line_y: 12.0,
            line_height: 16.0,
            effect_height: 20.0,
        })
    }
    fn render_glyph(
        &mut self,
        _handle: u64,
        _code_point: u32,
        scale_px: f32,
    ) -> Result<GlyphImage, CellError> {
        let side = scale_px.round().max(1.0) as u32;
        Ok(GlyphImage {
            width: side,
            height: side,
            pixels: vec![0xFF; (side * side) as usize],
        })
    }
}

// =====================================================================
// Manager state
// =====================================================================

#[derive(Debug)]
pub struct FontManager {
    initialized: bool,
    /// font_handle → (backend handle, scale)
    fonts: std::collections::BTreeMap<u32, (u64, f32)>,
    next_font_handle: u32,
    /// renderer_handle → Option<font_handle>
    renderers: std::collections::BTreeMap<u32, Option<u32>>,
    next_renderer_handle: u32,
}

impl Default for FontManager {
    fn default() -> Self {
        Self {
            initialized: false,
            fonts: std::collections::BTreeMap::new(),
            next_font_handle: 0,
            renderers: std::collections::BTreeMap::new(),
            next_renderer_handle: 0,
        }
    }
}

// =====================================================================
// Syscalls
// =====================================================================

fn ensure_init(m: &FontManager) -> Result<(), CellError> {
    if m.initialized { Ok(()) } else { Err(errors::UNINITIALIZED) }
}

#[must_use]
pub fn cell_font_init(m: &mut FontManager) -> Result<(), CellError> {
    if m.initialized {
        return Err(errors::ALREADY_INITIALIZED);
    }
    m.initialized = true;
    Ok(())
}

#[must_use]
pub fn cell_font_end(m: &mut FontManager) -> Result<(), CellError> {
    ensure_init(m)?;
    *m = FontManager::default();
    Ok(())
}

/// `cellFontOpenFontset(library, fontset, font_out)`.
#[must_use]
pub fn cell_font_open_fontset<B: FontBackend + ?Sized>(
    m: &mut FontManager,
    backend: &mut B,
    type_code: u32,
) -> Result<u32, CellError> {
    ensure_init(m)?;
    if m.fonts.len() as u32 >= MAX_FONTS {
        return Err(errors::FONT_OPEN_MAX);
    }
    let backend_handle = backend.open_system_font(type_code)?;
    m.next_font_handle += 1;
    let fh = m.next_font_handle;
    m.fonts.insert(fh, (backend_handle, 16.0));
    Ok(fh)
}

/// `cellFontOpenFontFile(library, path, sub_num, unique_id, font_out)`.
#[must_use]
pub fn cell_font_open_font_file<B: FontBackend + ?Sized>(
    m: &mut FontManager,
    backend: &mut B,
    path: &str,
) -> Result<u32, CellError> {
    ensure_init(m)?;
    if m.fonts.len() as u32 >= MAX_FONTS {
        return Err(errors::FONT_OPEN_MAX);
    }
    let backend_handle = backend.open_font_file(path)?;
    m.next_font_handle += 1;
    let fh = m.next_font_handle;
    m.fonts.insert(fh, (backend_handle, 16.0));
    Ok(fh)
}

/// `cellFontCloseFont(font)`.
#[must_use]
pub fn cell_font_close_font<B: FontBackend + ?Sized>(
    m: &mut FontManager,
    backend: &mut B,
    font_handle: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    let (be, _) = m.fonts.remove(&font_handle).ok_or(errors::FONT_NOT_FOUND)?;
    backend.close_font(be)
}

/// `cellFontCreateRenderer(library, cfg, renderer_out)`.
#[must_use]
pub fn cell_font_create_renderer(m: &mut FontManager) -> Result<u32, CellError> {
    ensure_init(m)?;
    m.next_renderer_handle += 1;
    let rh = m.next_renderer_handle;
    m.renderers.insert(rh, None);
    Ok(rh)
}

/// `cellFontDestroyRenderer(renderer)`.
#[must_use]
pub fn cell_font_destroy_renderer(
    m: &mut FontManager,
    renderer_handle: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    match m.renderers.get(&renderer_handle) {
        Some(Some(_)) => return Err(errors::RENDERER_ALREADY_BIND),
        Some(None) => { m.renderers.remove(&renderer_handle); Ok(()) }
        None => Err(errors::RENDERER_INVALID),
    }
}

/// `cellFontBindRenderer(font, renderer)`.
#[must_use]
pub fn cell_font_bind_renderer(
    m: &mut FontManager,
    font_handle: u32,
    renderer_handle: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    if !m.fonts.contains_key(&font_handle) {
        return Err(errors::FONT_NOT_FOUND);
    }
    let slot = m.renderers.get_mut(&renderer_handle).ok_or(errors::RENDERER_INVALID)?;
    if slot.is_some() {
        return Err(errors::RENDERER_ALREADY_BIND);
    }
    *slot = Some(font_handle);
    Ok(())
}

/// `cellFontUnbindRenderer(font)` — finds the renderer currently
/// bound to `font_handle` and releases it.
#[must_use]
pub fn cell_font_unbind_renderer(
    m: &mut FontManager,
    font_handle: u32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    let mut found = false;
    for (_, bound) in m.renderers.iter_mut() {
        if *bound == Some(font_handle) {
            *bound = None;
            found = true;
        }
    }
    if found { Ok(()) } else { Err(errors::RENDERER_UNBIND) }
}

/// `cellFontSetScalePixel(font, scale_x, scale_y)`.
#[must_use]
pub fn cell_font_set_scale_pixel(
    m: &mut FontManager,
    font_handle: u32,
    scale_x: f32,
) -> Result<(), CellError> {
    ensure_init(m)?;
    if !scale_x.is_finite() || scale_x <= 0.0 || scale_x > 1024.0 {
        return Err(errors::INVALID_PARAMETER);
    }
    let entry = m.fonts.get_mut(&font_handle).ok_or(errors::FONT_NOT_FOUND)?;
    entry.1 = scale_x;
    Ok(())
}

/// `cellFontGetHorizontalLayout(font, layout_out)`.
#[must_use]
pub fn cell_font_get_horizontal_layout<B: FontBackend + ?Sized>(
    m: &FontManager,
    backend: &B,
    font_handle: u32,
) -> Result<HorizontalLayout, CellError> {
    ensure_init(m)?;
    let (be, _) = m.fonts.get(&font_handle).ok_or(errors::FONT_NOT_FOUND)?;
    backend.horizontal_layout(*be)
}

/// `cellFontRenderCharGlyphImage(renderer, code, glyph_out)`.
#[must_use]
pub fn cell_font_render_char_glyph_image<B: FontBackend + ?Sized>(
    m: &mut FontManager,
    backend: &mut B,
    renderer_handle: u32,
    code_point: u32,
) -> Result<GlyphImage, CellError> {
    ensure_init(m)?;
    let font_handle = m
        .renderers
        .get(&renderer_handle)
        .copied()
        .ok_or(errors::RENDERER_INVALID)?
        .ok_or(errors::RENDERER_UNBIND)?;
    let (be, scale) = *m.fonts.get(&font_handle).ok_or(errors::FONT_NOT_FOUND)?;
    backend.render_glyph(be, code_point, scale)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn init() -> (FontManager, StubFontBackend) {
        let mut m = FontManager::default();
        cell_font_init(&mut m).unwrap();
        (m, StubFontBackend::default())
    }

    // --- constants ------------------------------------------------

    #[test]
    fn error_codes_byte_exact_vs_cpp() {
        assert_eq!(errors::FATAL.0, 0x8054_0001);
        assert_eq!(errors::UNINITIALIZED.0, 0x8054_0003);
        assert_eq!(errors::FONT_NOT_FOUND.0, 0x8054_000C);
        assert_eq!(errors::RENDERER_ALREADY_BIND.0, 0x8054_0020);
        assert_eq!(errors::NO_SUPPORT_SURFACE.0, 0x8054_0040);
    }

    #[test]
    fn type_constants_match_system_fonts() {
        assert_eq!(TYPE_RODIN_SANS_SERIF_LATIN, 0x00);
        assert_eq!(TYPE_RODIN_SANS_SERIF_BOLD_LATIN, 0x02);
        assert_eq!(TYPE_NEWRODIN_GOTHIC_JAPANESE, 0x08);
        assert_eq!(TYPE_YD_GOTHIC_KOREAN, 0x0C);
        assert_eq!(TYPE_MATISSE_SERIF_LATIN, 0x20);
    }

    // --- init / end ----------------------------------------------

    #[test]
    fn init_twice_is_already_initialized() {
        let mut m = FontManager::default();
        cell_font_init(&mut m).unwrap();
        assert_eq!(cell_font_init(&mut m).unwrap_err(), errors::ALREADY_INITIALIZED);
    }

    #[test]
    fn ops_without_init_are_uninitialized() {
        let mut m = FontManager::default();
        let mut b = StubFontBackend::default();
        assert_eq!(
            cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap_err(),
            errors::UNINITIALIZED,
        );
    }

    // --- fontset open / close ------------------------------------

    #[test]
    fn open_known_fontset_returns_handle() {
        let (mut m, mut b) = init();
        let h = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        assert_eq!(h, 1);
    }

    #[test]
    fn open_unknown_fontset_is_no_support() {
        let (mut m, mut b) = init();
        assert_eq!(
            cell_font_open_fontset(&mut m, &mut b, 0xDEAD).unwrap_err(),
            errors::NO_SUPPORT_FONTSET,
        );
    }

    #[test]
    fn open_fontset_max_is_font_open_max() {
        let (mut m, mut b) = init();
        for _ in 0..MAX_FONTS {
            cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        }
        assert_eq!(
            cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap_err(),
            errors::FONT_OPEN_MAX,
        );
    }

    #[test]
    fn close_unknown_font_is_not_found() {
        let (mut m, mut b) = init();
        assert_eq!(
            cell_font_close_font(&mut m, &mut b, 999).unwrap_err(),
            errors::FONT_NOT_FOUND,
        );
    }

    // --- file open -----------------------------------------------

    #[test]
    fn open_font_file_with_valid_path() {
        let (mut m, mut b) = init();
        let h = cell_font_open_font_file(&mut m, &mut b, "/dev_hdd0/font/SCE-PS3-MT-R.ttf").unwrap();
        assert_eq!(h, 1);
    }

    #[test]
    fn open_font_file_rejects_relative_path() {
        let (mut m, mut b) = init();
        assert_eq!(
            cell_font_open_font_file(&mut m, &mut b, "relative.ttf").unwrap_err(),
            errors::OPEN_FAILED,
        );
    }

    // --- renderer ------------------------------------------------

    #[test]
    fn create_and_destroy_renderer() {
        let (mut m, _) = init();
        let r = cell_font_create_renderer(&mut m).unwrap();
        cell_font_destroy_renderer(&mut m, r).unwrap();
    }

    #[test]
    fn destroy_unknown_renderer_is_invalid() {
        let (mut m, _) = init();
        assert_eq!(
            cell_font_destroy_renderer(&mut m, 999).unwrap_err(),
            errors::RENDERER_INVALID,
        );
    }

    #[test]
    fn destroy_bound_renderer_is_already_bind() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        let r = cell_font_create_renderer(&mut m).unwrap();
        cell_font_bind_renderer(&mut m, f, r).unwrap();
        assert_eq!(
            cell_font_destroy_renderer(&mut m, r).unwrap_err(),
            errors::RENDERER_ALREADY_BIND,
        );
    }

    #[test]
    fn bind_already_bound_is_already_bind() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        let r = cell_font_create_renderer(&mut m).unwrap();
        cell_font_bind_renderer(&mut m, f, r).unwrap();
        assert_eq!(
            cell_font_bind_renderer(&mut m, f, r).unwrap_err(),
            errors::RENDERER_ALREADY_BIND,
        );
    }

    #[test]
    fn bind_unknown_font_is_font_not_found() {
        let (mut m, _) = init();
        let r = cell_font_create_renderer(&mut m).unwrap();
        assert_eq!(
            cell_font_bind_renderer(&mut m, 999, r).unwrap_err(),
            errors::FONT_NOT_FOUND,
        );
    }

    #[test]
    fn unbind_without_bind_is_renderer_unbind() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        assert_eq!(
            cell_font_unbind_renderer(&mut m, f).unwrap_err(),
            errors::RENDERER_UNBIND,
        );
    }

    #[test]
    fn unbind_releases_renderer() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        let r = cell_font_create_renderer(&mut m).unwrap();
        cell_font_bind_renderer(&mut m, f, r).unwrap();
        cell_font_unbind_renderer(&mut m, f).unwrap();
        // After unbind, destroying the renderer is allowed again.
        cell_font_destroy_renderer(&mut m, r).unwrap();
    }

    // --- scale ---------------------------------------------------

    #[test]
    fn set_scale_pixel_valid_range() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        cell_font_set_scale_pixel(&mut m, f, 24.0).unwrap();
    }

    #[test]
    fn set_scale_pixel_rejects_zero_negative_nan() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        assert_eq!(cell_font_set_scale_pixel(&mut m, f, 0.0).unwrap_err(), errors::INVALID_PARAMETER);
        assert_eq!(cell_font_set_scale_pixel(&mut m, f, -10.0).unwrap_err(), errors::INVALID_PARAMETER);
        assert_eq!(cell_font_set_scale_pixel(&mut m, f, f32::NAN).unwrap_err(), errors::INVALID_PARAMETER);
        assert_eq!(cell_font_set_scale_pixel(&mut m, f, 9999.0).unwrap_err(), errors::INVALID_PARAMETER);
    }

    // --- rendering -----------------------------------------------

    #[test]
    fn render_glyph_uses_current_scale() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        cell_font_set_scale_pixel(&mut m, f, 32.0).unwrap();
        let r = cell_font_create_renderer(&mut m).unwrap();
        cell_font_bind_renderer(&mut m, f, r).unwrap();
        let img = cell_font_render_char_glyph_image(&mut m, &mut b, r, b'A' as u32).unwrap();
        assert_eq!(img.width, 32);
        assert_eq!(img.height, 32);
        assert_eq!(img.pixels.len(), 32 * 32);
    }

    #[test]
    fn render_without_bind_is_renderer_unbind() {
        let (mut m, mut b) = init();
        let r = cell_font_create_renderer(&mut m).unwrap();
        assert_eq!(
            cell_font_render_char_glyph_image(&mut m, &mut b, r, b'A' as u32).unwrap_err(),
            errors::RENDERER_UNBIND,
        );
    }

    #[test]
    fn render_with_unknown_renderer_is_invalid() {
        let (mut m, mut b) = init();
        assert_eq!(
            cell_font_render_char_glyph_image(&mut m, &mut b, 999, b'A' as u32).unwrap_err(),
            errors::RENDERER_INVALID,
        );
    }

    #[test]
    fn horizontal_layout_returns_backend_values() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_RODIN_SANS_SERIF_LATIN).unwrap();
        let layout = cell_font_get_horizontal_layout(&m, &b, f).unwrap();
        assert_eq!(layout.line_height, 16.0);
    }

    #[test]
    fn full_lifecycle_smoke() {
        let (mut m, mut b) = init();
        let f = cell_font_open_fontset(&mut m, &mut b, TYPE_NEWRODIN_GOTHIC_JAPANESE).unwrap();
        cell_font_set_scale_pixel(&mut m, f, 20.0).unwrap();
        let r = cell_font_create_renderer(&mut m).unwrap();
        cell_font_bind_renderer(&mut m, f, r).unwrap();
        let _img = cell_font_render_char_glyph_image(&mut m, &mut b, r, 0x3042).unwrap(); // あ
        cell_font_unbind_renderer(&mut m, f).unwrap();
        cell_font_destroy_renderer(&mut m, r).unwrap();
        cell_font_close_font(&mut m, &mut b, f).unwrap();
        cell_font_end(&mut m).unwrap();
    }
}
