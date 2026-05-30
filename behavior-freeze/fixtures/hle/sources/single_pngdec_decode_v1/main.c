// single_pngdec_decode_v1 — cellPngDec pixel decode (via vendored stb_image).
// CC0 1.0 (public domain). See LICENSE.md.
//
// cellPngDec Create -> Open -> ReadHeader -> SetParameter(RGBA) -> DecodeData.
// RPCS3 decodes with libpng; emu-core uses the vendored stb_image. For a BASELINE
// PNG (no gamma/sRGB/interlace) both spec-decode to identical pixels, so the RGBA
// output is byte-exact. Create/Open are callback-driven (guest cbCtrlMalloc, via
// the call_guest_function OPD path). Embeds a real 16x16 RGB PNG (png_data.h).

#include <ppu-types.h>
#include <pngdec/pngdec.h>
#include <sys/process.h>
#include <string.h>

#include "png_data.h" // g_png_data[], g_png_size

SYS_PROCESS_PARAM(1001, 0x10000);

#define W 16
#define H 16
#define GOLDEN 143104u // stb_image RGBA checksum (pngdec_golden_checksum)

static unsigned char g_pool[8192];
static u32 g_pool_off = 0;
static void *my_malloc(u32 size, void *arg)
{
    (void)arg;
    void *p = &g_pool[g_pool_off];
    g_pool_off += (size + 15u) & ~15u;
    return p;
}
static void my_free(void *ptr, void *arg) { (void)ptr; (void)arg; }

static unsigned char g_out[W * H * 4];

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
    if (info.width != W || info.height != H) {
        return 0xBAD3;
    }

    pngDecInParam inp;
    pngDecOutParam outp;
    memset(&inp, 0, sizeof(inp));
    memset(&outp, 0, sizeof(outp));
    inp.output_mode = PNGDEC_TOP_TO_BOTTOM;
    inp.color_space = PNGDEC_RGBA;
    inp.bit_depth = 8;
    inp.pack_flag = 0;
    inp.alpha_select = 0;
    inp.alpha = 0xFF;
    if (pngDecSetParameter(handle, sub, &inp, &outp) != 0) {
        return 0xBAD4;
    }
    if (outp.width != W || outp.height != H || outp.num_comp != 4) {
        return 0xBAD5;
    }

    pngDecDataCtrlParam dcp;
    memset(&dcp, 0, sizeof(dcp));
    dcp.output_bytes_per_line = W * 4;
    pngDecDataInfo dout;
    memset(&dout, 0, sizeof(dout));
    if (pngDecDecodeData(handle, sub, g_out, &dcp, &dout) != 0) {
        return 0xBAD6;
    }
    if (dout.decode_status != PNGDEC_STATUS_FINISH) {
        return 0xBAD7;
    }

    u32 sum = 0;
    for (u32 i = 0; i < sizeof(g_out); i++) {
        sum += g_out[i];
    }
    if (sum != GOLDEN) {
        return 0xBAD8;
    }

    return 0xC0DE;
}
