// single_gcm_indexdraw_v1 — R13.5b: validate indexed draw + IndexArray from
// REAL libgcm. CC0 1.0 (public domain). See LICENSE.md.
//
// rsxInit + rsxDrawIndexArray(GCM_TYPE_TRIANGLES, offset 0x10000, count 3,
// GCM_INDEX_TYPE_16B, RSX). PSL1GHT librsx expands this into
// SET_INDEX_ARRAY_ADDRESS + SET_INDEX_ARRAY_DMA + SET_BEGIN_END(5) +
// DRAW_INDEX_ARRAY + SET_BEGIN_END(0). The DrawTracker in rpcs3-rsx-state
// recognises this as a DrawKind::Indexed DrawCall, and index_array() parses the
// IndexArray descriptor — first INDEXED-draw path validated against REAL libgcm
// bytes (the draw oracle so far only covered DrawKind::Arrays).
//
// Pure command-stream test: no real index/vertex buffers are uploaded — the
// offset is just the value emitted into SET_INDEX_ARRAY_ADDRESS.
//
// Behaviour: rsxInit -> rsxDrawIndexArray -> label -> 0xC0DE.

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CB_SIZE   (0x10000)         // 64 KB command buffer
#define HOST_SIZE (1 * 1024 * 1024) // 1 MB io region
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024 * 1024)));

#define IDX_OFFSET 0x00010000u // index buffer offset (distinctive)
#define IDX_COUNT  3           // one triangle (3 indices)

int main(void)
{
    gcmContextData *ctx;
    rsxInit(&ctx, CB_SIZE, HOST_SIZE, host_buffer);

    // rsxDrawIndexArray(ctx, type, offset, count, data_type, location)
    rsxDrawIndexArray(ctx, GCM_TYPE_TRIANGLES, IDX_OFFSET, IDX_COUNT,
                      GCM_INDEX_TYPE_16B, GCM_LOCATION_RSX);

    rsxSetWriteCommandLabel(ctx, 0, 0x12345678); // frame marker

    return 0xC0DE;
}
