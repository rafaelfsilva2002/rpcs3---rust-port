// single_font_init_v1 — cellFont init/end lifecycle + fc_size invariant.
// CC0 1.0 (public domain). See LICENSE.md.
//
// The cellFont entry path: fontInit (PSL1GHT inline) calls the local
// fontGetStubRevisionFlags then the real cellFontInitializeWithRevision NID,
// which rejects a file-cache smaller than 24 bytes (cellFont.cpp:54). fontEnd
// is cellFontEnd. No glyph rendering / FreeType — that giant tail (real font
// parsing + rasterization) is deferred; this is the behavior-freezable init
// slice.
//
// Self-contained oracle: exercises the fc_size>=24 invariant in BOTH directions
//   init(size=0)        must be REJECTED   -> else 0xBAD2
//   init(size>=24)      must SUCCEED       -> else 0xBAD0
//   fontEnd             must SUCCEED       -> else 0xBAD1
//   all correct                            -> 0xC0DE

#include <ppu-types.h>
#include <font/font.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

static u32 g_filecache[256]; // 1 KiB file cache (>= 24 bytes)

int main(void)
{
    fontConfig config;
    fontConfig_initialize(&config);

    // Bad config: fileCache.size == 0 (< 24) must be rejected.
    config.fileCache.buffer = g_filecache;
    config.fileCache.size = 0;
    if (fontInit(&config) == 0) {
        return 0xBAD2; // should have failed with INVALID_PARAMETER
    }

    // Proper config: a real cache >= 24 bytes must succeed.
    config.fileCache.size = sizeof(g_filecache);
    if (fontInit(&config) != 0) {
        return 0xBAD0;
    }

    if (fontEnd() != 0) {
        return 0xBAD1;
    }

    return 0xC0DE;
}
