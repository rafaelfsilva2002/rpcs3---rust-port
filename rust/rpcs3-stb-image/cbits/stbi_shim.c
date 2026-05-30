// stbi_shim.c — thin C shim over the vendored stb_image.h, the SAME decoder
// RPCS3 cellJpgDec links (stbi_load_from_memory). Compiled only with the
// `decode` cargo feature (see build.rs). Limited to JPEG + PNG (the two cell
// image decoders we wire). RPCS3 forces 4 channels (RGBA); we mirror that so the
// output is byte-identical.

#define STB_IMAGE_IMPLEMENTATION
#define STBI_ONLY_JPEG
#define STBI_ONLY_PNG
#define STBI_NO_STDIO
#include "stb_image.h"

// Decode buf[0..len) to a forced-RGBA buffer (width*height*4 bytes). Fills
// *w/*h; returns the stb-malloc'd pixels (free with stbi_shim_free) or NULL on
// failure. Mirrors RPCS3 cellJpgDec.cpp:231 stbi_load_from_memory(...,&c,4).
unsigned char* stbi_shim_load_rgba(const unsigned char* buf, int len, int* w, int* h)
{
    int comp = 0;
    return stbi_load_from_memory(buf, len, w, h, &comp, 4);
}

void stbi_shim_free(unsigned char* p)
{
    stbi_image_free(p);
}
