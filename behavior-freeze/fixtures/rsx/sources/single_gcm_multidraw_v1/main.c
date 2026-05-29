// single_gcm_multidraw_v1 — R13.5a: validate MULTIPLE DrawCalls in one frame
// from REAL libgcm. CC0 1.0 (public domain). See LICENSE.md.
//
// rsxInit + TWO rsxDrawVertexArray calls (TRIANGLES 0..3, then TRIANGLES 10..6)
// in one frame. Each call emits SET_BEGIN_END(5) + DRAW_ARRAYS + SET_BEGIN_END(0),
// so the DrawTracker in rpcs3-rsx-state finalizes TWO separate DrawCall records.
// The draw oracle so far only ever produced ONE DrawCall — this validates the
// multi-draw path against REAL libgcm bytes.
//
// Pure command-stream test: no vertex buffers uploaded; the draw ranges are the
// values emitted into DRAW_ARRAYS.
//
// Behaviour: rsxInit -> draw(0,3) -> draw(10,6) -> label -> 0xC0DE.

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CB_SIZE   (0x10000)         // 64 KB command buffer
#define HOST_SIZE (1 * 1024 * 1024) // 1 MB io region
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024 * 1024)));

int main(void)
{
    gcmContextData *ctx;
    rsxInit(&ctx, CB_SIZE, HOST_SIZE, host_buffer);

    rsxDrawVertexArray(ctx, GCM_TYPE_TRIANGLES, 0, 3);  // draw 1 -> range (0,3)
    rsxDrawVertexArray(ctx, GCM_TYPE_TRIANGLES, 10, 6); // draw 2 -> range (10,6)

    rsxSetWriteCommandLabel(ctx, 0, 0x12345678); // frame marker

    return 0xC0DE;
}
