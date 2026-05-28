// single_gcm_draw_v1 — R13.3: extend R13.2 with a real DRAW_ARRAYS.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Builds on single_gcm_emit_v1 (R13.2) by adding rsxDrawVertexArray,
// which emits the NV4097_SET_BEGIN_END(type) + NV4097_DRAW_ARRAYS +
// NV4097_SET_BEGIN_END(0) sequence inline into the cellGcm-init'd
// context. The DrawTracker in rpcs3-rsx-state recognises this as a
// complete DrawCall, so the captured stream decodes to a snapshot
// with a non-empty draw_calls list (first real-libgcm draw call
// captured through the FULL cellGcm path).
//
// No vertex buffer / shader binding: this fixture is a pure
// command-stream test — replay_gcm validates the METHODS appear in
// the captured bytes, which is what matters for the behaviour-freeze
// pipeline. Real rendering is the deferred GPU-backend tail.
//
// Behaviour: rsxInit -> clear -> draw 3 verts as TRIANGLES from
// index 0 -> command label -> return 0xC0DE.

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

    rsxSetClearColor(ctx, 0xff202020);
    rsxClearSurface(ctx, GCM_CLEAR_M); // 0xF3 mask

    // First REAL libgcm draw call through the cellGcm-init'd context:
    // GCM_TYPE_TRIANGLES = 5, start = 0, count = 3 (single triangle).
    rsxDrawVertexArray(ctx, GCM_TYPE_TRIANGLES, 0, 3);

    rsxSetWriteCommandLabel(ctx, 0, 0x12345678); // frame marker

    return 0xC0DE;
}
