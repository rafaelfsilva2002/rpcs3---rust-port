// single_gcm_texture_v1 — R13.5e: validate TextureDescriptor from REAL libgcm.
// CC0 1.0 (public domain). See LICENSE.md.
//
// rsxInit + rsxLoadTexture(unit 0, 256x128 A8R8G8B8 linear, offset 0x200000) +
// rsxTextureControl(enable unit 0). PSL1GHT librsx expands these inline into the
// NV4097_SET_TEXTURE_* method block (OFFSET, FORMAT, ADDRESS, CONTROL0/1,
// FILTER, IMAGE_RECT, BORDER_COLOR). rpcs3-rsx-state's texture() parses the
// block into a TextureDescriptor, collected into RsxSnapshot.textures — so the
// captured stream validates another whole Camada-B descriptor struct against
// REAL libgcm bytes (the texture method addresses 0x1A00/04/0C/18 already match
// RPCS3 gcm_enums.h, so this confirms the texture decode rather than fixing it).
//
// Pure command-stream test: no texel data is uploaded — the offset is just the
// value emitted into SET_TEXTURE_OFFSET. Real texture pixel-decode is the
// deferred GPU-backend tail (Camada D).
//
// Behaviour: rsxInit -> rsxLoadTexture + rsxTextureControl -> label -> 0xC0DE.

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CB_SIZE   (0x10000)         // 64 KB command buffer
#define HOST_SIZE (1 * 1024 * 1024) // 1 MB io region
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024 * 1024)));

#define TEX_WIDTH  256
#define TEX_HEIGHT 128
#define TEX_OFFSET 0x00200000u // distinctive

int main(void)
{
    gcmContextData *ctx;
    rsxInit(&ctx, CB_SIZE, HOST_SIZE, host_buffer);

    gcmTexture tex;
    memset(&tex, 0, sizeof(tex));
    tex.format    = GCM_TEXTURE_FORMAT_A8R8G8B8 | GCM_TEXTURE_FORMAT_LIN | GCM_TEXTURE_FORMAT_NRM;
    tex.mipmap    = 1;
    tex.dimension = GCM_TEXTURE_DIMS_2D;
    tex.cubemap   = GCM_FALSE;
    tex.remap     = 0;
    tex.width     = TEX_WIDTH;
    tex.height    = TEX_HEIGHT;
    tex.depth     = 1;
    tex.location  = GCM_LOCATION_RSX;
    tex.pitch     = TEX_WIDTH * 4;
    tex.offset    = TEX_OFFSET;

    rsxLoadTexture(ctx, 0, &tex);
    rsxTextureControl(ctx, 0, GCM_TRUE, 0, 0, 0); // enable unit 0

    rsxSetWriteCommandLabel(ctx, 0, 0x12345678); // frame marker

    return 0xC0DE;
}
