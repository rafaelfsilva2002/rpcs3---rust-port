// single_gcm_viewport_v1 — R13.5d: validate viewport state registers from REAL
// libgcm. CC0 1.0 (public domain). See LICENSE.md.
//
// rsxInit + rsxSetViewport(0,0,640,480,...) + label. PSL1GHT librsx emits the
// NV4097_SET_VIEWPORT_HORIZONTAL/VERTICAL (+ depth range / scale / offset) state
// methods. Unlike surface/texture/draw, viewport is a *SetState* group not
// exposed in RsxSnapshot — so the test decodes the captured stream into an
// RsxState (rpcs3-rsx-state run_and_apply) and reads VIEWPORT_HORIZONTAL/VERTICAL
// directly, validating the viewport register encoding against REAL libgcm bytes
// (no production/snapshot change needed).
//
// Behaviour: rsxInit -> rsxSetViewport(640x480) -> label -> 0xC0DE.

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CB_SIZE   (0x10000)         // 64 KB command buffer
#define HOST_SIZE (1 * 1024 * 1024) // 1 MB io region
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024 * 1024)));

#define VP_W 640
#define VP_H 480

int main(void)
{
    gcmContextData *ctx;
    rsxInit(&ctx, CB_SIZE, HOST_SIZE, host_buffer);

    // scale/offset map NDC -> the 640x480 viewport (standard half-dimension
    // mapping). They land in SET_VIEWPORT_SCALE/OFFSET registers the test does
    // not read; only HORIZONTAL/VERTICAL (x|w<<16, y|h<<16) are asserted.
    float scale[4]  = { VP_W / 2.0f, VP_H / 2.0f, 0.5f, 0.0f };
    float offset[4] = { VP_W / 2.0f, VP_H / 2.0f, 0.5f, 0.0f };
    rsxSetViewport(ctx, 0, 0, VP_W, VP_H, 0.0f, 1.0f, scale, offset);

    rsxSetWriteCommandLabel(ctx, 0, 0x12345678); // frame marker

    return 0xC0DE;
}
