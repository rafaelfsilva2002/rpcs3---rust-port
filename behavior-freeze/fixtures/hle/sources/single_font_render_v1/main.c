// single_font_render_v1 — cellFont byte-exact glyph rasterization.
// CC0 1.0 (public domain). See LICENSE.md.
//
// The cellFont RENDERING path: rasterize a glyph to an 8-bit coverage surface
// via the renderer lifecycle (createRenderer -> bindRenderer -> renderSurfaceInit
// -> renderCharGlyphImage). RPCS3 uses stbtt_GetCodepointBitmap; emu-core uses
// the vendored C stb_truetype.h (feature `cellfont-raster`), so the rendered
// surface is byte-exact.
//
// Renders 'A' (synthetic CC0 font, scale_y=32) into a 64x64 grayscale buffer at
// (0,0), then checksums the buffer. The golden 73114 is the stb_truetype v2
// rasterizer coverage sum (see rpcs3-hle-cellfont raster_calibration.rs).
//   any step fails -> 0xBADn ; checksum mismatch -> 0xBAD7 ; match -> 0xC0DE

#include <ppu-types.h>
#include <font/font.h>
#include <font/fontFT.h>
#include <sys/process.h>
#include <string.h>

#include "font_data.h" // g_font_data[], g_font_size

SYS_PROCESS_PARAM(1001, 0x10000);

// PSL1GHT font.h typo: declared "fontontSetScalePixel"; real symbol below.
extern s32 fontSetScalePixel(font *f, f32 w, f32 h);

static u32 g_filecache[256];
static unsigned char g_surface[64 * 64];
static u32 g_rendererbuf[1024];

int main(void)
{
    fontConfig config;
    fontConfig_initialize(&config);
    config.fileCache.buffer = g_filecache;
    config.fileCache.size = sizeof(g_filecache);
    if (fontInit(&config) != 0) {
        return 0xBAD0;
    }

    fontLibraryConfigFT ftcfg;
    fontLibraryConfigFT_initialize(&ftcfg);
    const fontLibrary *lib = NULL;
    if (fontInitLibraryFreeType(&ftcfg, &lib) != 0 || lib == NULL) {
        return 0xBAD1;
    }

    font f;
    if (fontOpenFontMemory(lib, (void *)g_font_data, g_font_size, 0, 0, &f) != 0) {
        return 0xBAD2;
    }
    if (fontSetScalePixel(&f, 32.0f, 32.0f) != 0) {
        return 0xBAD3;
    }

    fontRendererConfig rcfg;
    memset(&rcfg, 0, sizeof(rcfg));
    rcfg.bufferingPolicy.buffer = g_rendererbuf;
    rcfg.bufferingPolicy.initSize = sizeof(g_rendererbuf);
    rcfg.bufferingPolicy.maxSize = sizeof(g_rendererbuf);

    fontRenderer renderer;
    if (fontCreateRenderer(lib, &rcfg, &renderer) != 0) {
        return 0xBAD4;
    }
    if (fontBindRenderer(&f, &renderer) != 0) {
        return 0xBAD5;
    }

    fontRenderSurface surface;
    fontRenderSurfaceInit(&surface, g_surface, 64, 1, 64, 64);

    fontGlyphMetrics m;
    fontImageTransInfo trans;
    memset(&trans, 0, sizeof(trans));
    if (fontRenderCharGlyphImage(&f, 'A', &surface, 0.0f, 0.0f, &m, &trans) != 0) {
        return 0xBAD6;
    }

    u32 sum = 0;
    for (u32 i = 0; i < sizeof(g_surface); i++) {
        sum += g_surface[i];
    }
    if (sum != 73114) {
        return 0xBAD7;
    }

    return 0xC0DE;
}
