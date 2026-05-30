// single_jpgdec_decode_v1 — cellJpgDec pixel decode (byte-exact via stb_image).
// CC0 1.0 (public domain). See LICENSE.md.
//
// cellJpgDec Create -> Open -> ReadHeader -> SetParameter(RGBA) -> DecodeData.
// RPCS3 decodes with stbi_load_from_memory(...,4) (cellJpgDec.cpp:231); emu-core
// uses the SAME vendored stb_image (rpcs3-stb-image, feature image-decode), so the
// RGBA pixels are byte-exact. Embeds a real 16x16 JPEG (jpg_data.h via gen_jpg.py)
// and checksums the decoded RGBA buffer against the stb_image golden.

#include <ppu-types.h>
#include <jpgdec/jpgdec.h>
#include <sys/process.h>
#include <string.h>

#include "jpg_data.h" // g_jpg_data[], g_jpg_size

SYS_PROCESS_PARAM(1001, 0x10000);

#define W 16
#define H 16
#define GOLDEN 159529u // stb_image RGBA checksum (jpgdec_golden_checksum test)

static unsigned char g_out[W * H * 4];

int main(void)
{
    s32 handle = 0;
    jpgDecThreadInParam tin;
    jpgDecThreadOutParam tout;
    memset(&tin, 0, sizeof(tin));
    memset(&tout, 0, sizeof(tout));
    tin.spu_enable = JPGDEC_SPU_THREAD_DISABLE;
    if (jpgDecCreate(&handle, &tin, &tout) != 0) {
        return 0xBAD0;
    }

    jpgDecSource src;
    memset(&src, 0, sizeof(src));
    src.stream_sel = JPGDEC_BUFFER;
    src.stream_ptr = (void *)g_jpg_data;
    src.stream_size = g_jpg_size;
    jpgDecOpnInfo opn;
    memset(&opn, 0, sizeof(opn));
    s32 sub = 0;
    if (jpgDecOpen(handle, &sub, &src, &opn) != 0) {
        return 0xBAD1;
    }

    jpgDecInfo info;
    memset(&info, 0, sizeof(info));
    if (jpgDecReadHeader(handle, sub, &info) != 0) {
        return 0xBAD2;
    }
    if (info.width != W || info.height != H) {
        return 0xBAD3;
    }

    jpgDecInParam inp;
    jpgDecOutParam outp;
    memset(&inp, 0, sizeof(inp));
    memset(&outp, 0, sizeof(outp));
    inp.down_scale = 1;
    inp.quality_mode = 0;
    inp.output_mode = JPGDEC_TOP_TO_BOTTOM;
    inp.color_space = JPGDEC_RGBA;
    inp.alpha = 0xFF;
    if (jpgDecSetParameter(handle, sub, &inp, &outp) != 0) {
        return 0xBAD4;
    }
    if (outp.width != W || outp.height != H || outp.num_comp != 4) {
        return 0xBAD5;
    }

    jpgDecDataCtrlParam dcp;
    memset(&dcp, 0, sizeof(dcp));
    dcp.output_bytes_per_line = W * 4;
    jpgDecDataInfo dout;
    memset(&dout, 0, sizeof(dout));
    if (jpgDecDecodeData(handle, sub, g_out, &dcp, &dout) != 0) {
        return 0xBAD6;
    }
    if (dout.decode_status != JPGDEC_STATUS_FINISH) {
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
