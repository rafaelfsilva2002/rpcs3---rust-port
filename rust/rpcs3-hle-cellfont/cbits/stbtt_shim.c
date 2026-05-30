// stbtt_shim.c — thin C shim over the vendored stb_truetype.h, the SAME engine
// RPCS3 cellFont links. Compiled only with the `cellfont-raster` cargo feature
// (see build.rs). Exposes exactly what cellFontRenderCharGlyphImage needs
// (cellFont.cpp:710-723): the 8-bit coverage bitmap + placement offsets +
// baseLineY, so the Rust blit reproduces RPCS3 byte-for-byte.

#define STB_TRUETYPE_IMPLEMENTATION
#include "stb_truetype.h"

// Rasterize `code` from `font` (fontoffset 0) at pixel height `scale_y`.
// On success returns the stbtt-malloc'd coverage bitmap (width*height bytes) and
// fills width/height/xoff/yoff (stbtt_GetCodepointBitmap) + base_line_y
// ((int)(ascent*scale), cellFont.cpp:723). Returns NULL if InitFont fails or the
// glyph has no bitmap. Free the result with stbtt_shim_free.
unsigned char* stbtt_shim_render(const unsigned char* font, int fontoffset,
                                 unsigned int code, float scale_y,
                                 int* width, int* height, int* xoff, int* yoff,
                                 int* base_line_y)
{
    stbtt_fontinfo info;
    if (!stbtt_InitFont(&info, font, fontoffset)) {
        return 0;
    }
    float scale = stbtt_ScaleForPixelHeight(&info, scale_y);
    unsigned char* box = stbtt_GetCodepointBitmap(&info, scale, scale, (int)code,
                                                  width, height, xoff, yoff);
    int ascent, descent, lineGap;
    stbtt_GetFontVMetrics(&info, &ascent, &descent, &lineGap);
    *base_line_y = (int)(ascent * scale);
    return box;
}

void stbtt_shim_free(unsigned char* p)
{
    stbtt_FreeBitmap(p, 0);
}
