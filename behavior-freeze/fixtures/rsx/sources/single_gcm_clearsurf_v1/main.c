// single_gcm_clearsurf_v1 — GPU backend: REAL libgcm surface-setup + clear.
// CC0 1.0 (public domain). See LICENSE.md.
//
// First end-to-end behavior-freeze oracle for the RSX render layer: rsxSetSurface
// (16x16 A8R8G8B8 at color offset 0) -> rsxSetClearColor(0xAABBCCDD) ->
// rsxClearSurface(0xF3). The captured NV4097 stream decodes (rpcs3-rsx-state
// replay_gcm) to a SurfaceDescriptor + COLOR_CLEAR_VALUE + CLEAR_SURFACE mask;
// rpcs3-rsx-render::execute_clear then fills the render target, which the test
// checks pixel-for-pixel. A clear writes a constant color, so this is byte-exact
// with RPCS3 (which clears to the same value).
//
// Small 16x16 surface at offset 0 keeps the reference framebuffer tiny (1 KiB).

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CB_SIZE   (0x10000)
#define HOST_SIZE (1 * 1024 * 1024)
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024 * 1024)));

#define FB_WIDTH     16
#define FB_HEIGHT    16
#define COLOR_PITCH  (FB_WIDTH * 4) /* 64 */
#define COLOR_OFFSET 0u             /* slot A at local-mem offset 0 */
#define CLEAR_COLOR  0xAABBCCDDu

int main(void)
{
    gcmContextData *ctx;
    rsxInit(&ctx, CB_SIZE, HOST_SIZE, host_buffer);

    gcmSurface surface;
    memset(&surface, 0, sizeof(surface));
    surface.type             = GCM_SURFACE_TYPE_LINEAR;
    surface.antiAlias        = GCM_SURFACE_CENTER_1;
    surface.colorFormat      = GCM_SURFACE_A8R8G8B8;
    surface.colorTarget      = GCM_SURFACE_TARGET_0;
    surface.colorLocation[0] = GCM_LOCATION_RSX;
    surface.colorOffset[0]   = COLOR_OFFSET;
    surface.colorPitch[0]    = COLOR_PITCH;
    surface.depthFormat      = GCM_SURFACE_ZETA_Z24S8;
    surface.depthLocation    = GCM_LOCATION_RSX;
    surface.depthOffset      = 0x00100000u;
    surface.depthPitch       = COLOR_PITCH;
    surface.width            = FB_WIDTH;
    surface.height           = FB_HEIGHT;
    surface.x                = 0;
    surface.y                = 0;
    rsxSetSurface(ctx, &surface);

    rsxSetClearColor(ctx, CLEAR_COLOR);
    rsxClearSurface(ctx, 0xF3); // color (R|G|B|A) + depth + stencil

    rsxSetWriteCommandLabel(ctx, 0, 0x12345678); // frame marker

    return 0xC0DE;
}
