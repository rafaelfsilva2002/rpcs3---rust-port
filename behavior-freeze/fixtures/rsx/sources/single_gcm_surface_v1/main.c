// single_gcm_surface_v1 — R13.5c: validate SurfaceDescriptor from REAL libgcm.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Extends the R13 RSX fixtures by calling rsxSetSurface(ctx, &surface) with a
// fully-filled gcmSurface (640x480, A8R8G8B8 color into target 0, Z24S8 depth,
// distinctive color/depth offsets + pitches). PSL1GHT librsx expands this
// inline into the NV4097_SET_SURFACE_* method block (FORMAT, COLOR_TARGET,
// COLOR_*OFFSET, PITCH_*, ZETA_OFFSET, PITCH_Z, CLIP_H/V). rpcs3-rsx-state's
// surface() parses that block into a SurfaceDescriptor, exposed on
// RsxSnapshot.surface — so the captured stream validates a whole Camada-B
// descriptor struct against REAL libgcm bytes.
//
// Pure command-stream test: no real framebuffer is allocated — the offsets are
// just the values emitted into SET_SURFACE_*OFFSET. Real rendering is the
// deferred GPU-backend tail.
//
// Behaviour: rsxInit -> rsxSetSurface(640x480 A8R8G8B8 + Z24S8) -> label
//            -> return 0xC0DE.

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CB_SIZE   (0x10000)         // 64 KB command buffer
#define HOST_SIZE (1 * 1024 * 1024) // 1 MB io region
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024 * 1024)));

#define FB_WIDTH     640
#define FB_HEIGHT    480
#define COLOR_PITCH  (FB_WIDTH * 4)   /* 2560 = 0xA00 */
#define COLOR_OFFSET 0x00010000u      /* distinctive, 64 KB aligned */
#define DEPTH_PITCH  (FB_WIDTH * 4)
#define DEPTH_OFFSET 0x00200000u      /* distinctive */

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
    surface.depthOffset      = DEPTH_OFFSET;
    surface.depthPitch       = DEPTH_PITCH;
    surface.width            = FB_WIDTH;
    surface.height           = FB_HEIGHT;
    surface.x                = 0;
    surface.y                = 0;

    rsxSetSurface(ctx, &surface);

    rsxSetWriteCommandLabel(ctx, 0, 0x12345678); // frame marker

    return 0xC0DE;
}
