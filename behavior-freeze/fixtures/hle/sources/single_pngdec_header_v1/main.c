// single_pngdec_header_v1 — cellPngDec header parse (callback-driven, byte-exact).
// CC0 1.0 (public domain). See LICENSE.md.
//
// cellPngDec Create -> Open(BUFFER) -> ReadHeader. UNLIKE cellJpgDec, cellPngDec
// is CALLBACK-DRIVEN: Create + Open invoke the guest cbCtrlMalloc callback to
// allocate the handle/stream in guest memory (RPCS3 cellPngDec.cpp:344/390).
// emu-core drives those callbacks via EmuCore::call_guest_function (the R14
// guest re-entry infra). ReadHeader's width/height/etc. come from the PNG IHDR
// chunk (RPCS3 uses libpng's png_get_image_*; the values are the IHDR fields),
// parsed byte-exact by rpcs3_hle_cellpngdec::PngDec::parse_header.
//
// Embeds a minimal RGB PNG (320x240, depth 8; png_data.h via gen_png.py). No
// pixel decode is needed for the header.

#include <ppu-types.h>
#include <pngdec/pngdec.h>
#include <sys/process.h>
#include <string.h>

#include "png_data.h" // g_png_data[], g_png_size

SYS_PROCESS_PARAM(1001, 0x10000);

// Guest allocator the SYSTEM calls back into (bump allocator over a static pool).
static unsigned char g_pool[8192];
static u32 g_pool_off = 0;

static void *my_malloc(u32 size, void *arg)
{
    (void)arg;
    void *p = &g_pool[g_pool_off];
    g_pool_off += (size + 15u) & ~15u; // 16-byte aligned bump
    return p;
}

static void my_free(void *ptr, void *arg)
{
    (void)ptr;
    (void)arg;
}

int main(void)
{
    s32 handle = 0;
    pngDecThreadInParam tin;
    pngDecThreadOutParam tout;
    memset(&tin, 0, sizeof(tin));
    memset(&tout, 0, sizeof(tout));
    tin.spu_enable = PNGDEC_SPU_THREAD_DISABLE;
    tin.malloc_func = my_malloc;
    tin.free_func = my_free;
    if (pngDecCreate(&handle, &tin, &tout) != 0) {
        return 0xBAD0;
    }

    pngDecSource src;
    memset(&src, 0, sizeof(src));
    src.stream_sel = PNGDEC_BUFFER;
    src.stream_ptr = (void *)g_png_data;
    src.stream_size = g_png_size;
    src.spu_enable = PNGDEC_SPU_THREAD_DISABLE;

    pngDecOpnInfo opn;
    memset(&opn, 0, sizeof(opn));
    s32 sub = 0;
    if (pngDecOpen(handle, &sub, &src, &opn) != 0) {
        return 0xBAD1;
    }

    pngDecInfo info;
    memset(&info, 0, sizeof(info));
    if (pngDecReadHeader(handle, sub, &info) != 0) {
        return 0xBAD2;
    }

    if (info.width != 320) {
        return 0xBAD3;
    }
    if (info.height != 240) {
        return 0xBAD4;
    }
    if (info.num_comp != 3) {
        return 0xBAD5;
    }
    if (info.color_space != PNGDEC_RGB) {
        return 0xBAD6;
    }
    if (info.bit_depth != 8) {
        return 0xBAD7;
    }

    return 0xC0DE;
}
