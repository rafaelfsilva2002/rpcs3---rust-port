// single_jpgdec_header_v1 — cellJpgDec header parse (byte-exact).
// CC0 1.0 (public domain). See LICENSE.md.
//
// cellJpgDec Create -> Open(BUFFER) -> ReadHeader. RPCS3's cellJpgDecReadHeader
// (cellJpgDec.cpp:146-178) parses the JPEG header bytes MANUALLY (no decode, no
// callbacks): it walks the JFIF segment chain to the FF C0 SOF0 marker and reads
// width/height from it (numComponents hardcoded 3, colorSpace RGB). emu-core
// ports that parse exactly (segment length uses the RPCS3 *0xFF quirk).
//
// Embeds a minimal JFIF (SOI + APP0 + SOF0 with width=320, height=240). No pixel
// decode is needed for the header. Checks the parsed fields -> 0xC0DE.

#include <ppu-types.h>
#include <jpgdec/jpgdec.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

// Minimal JFIF: SOI + APP0(len 16) + SOF0(precision 8, height 240, width 320,
// 3 components) + EOI. Only the header is parsed; it need not be decodable.
static const unsigned char g_jpg[] = {
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01,
    0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, // APP0 (16-byte segment)
    0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0xF0, 0x01, 0x40, 0x03, // SOF0 h=240 w=320
    0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF, 0xD9,
};

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
    src.stream_ptr = (void *)g_jpg;
    src.stream_size = sizeof(g_jpg);
    src.spu_enable = JPGDEC_SPU_THREAD_DISABLE;

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

    if (info.width != 320) {
        return 0xBAD3;
    }
    if (info.height != 240) {
        return 0xBAD4;
    }
    if (info.num_comp != 3) {
        return 0xBAD5;
    }
    if (info.color_space != JPGDEC_RGB) {
        return 0xBAD6;
    }

    return 0xC0DE;
}
