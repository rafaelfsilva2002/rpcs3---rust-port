// single_gcm_setdisplay_v1 — R13.4 probe: first flip-path NID.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Extends single_gcm_draw_v1 (R13.3) by adding ONE call to
// gcmSetDisplayBuffer — the entry point of the PSL1GHT flip/display
// flow. This is the first call that likely resolves to a real
// (non-inline) cellGcm PRX NID — so it surfaces the next unmet
// import in EmuCore. We DO NOT add the rest of the flip path
// (gcmSetFlip / gcmGetFlipStatus / rsxFlushBuffer) yet — one new
// NID at a time, keep the slice honest.
//
// Behaviour: rsxInit -> clear -> draw -> gcmSetDisplayBuffer -> label
// -> return 0xC0DE.  Display-buffer parameters are placeholders
// (640x480 RGBA at offset 0, pitch=640*4) — the call's return value
// is ignored.

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <rsx/gcm_sys.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CB_SIZE   (0x10000)
#define HOST_SIZE (1 * 1024 * 1024)
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024 * 1024)));

int main(void)
{
    gcmContextData *ctx;
    rsxInit(&ctx, CB_SIZE, HOST_SIZE, host_buffer);

    rsxSetClearColor(ctx, 0xff202020);
    rsxClearSurface(ctx, GCM_CLEAR_M);
    rsxDrawVertexArray(ctx, GCM_TYPE_TRIANGLES, 0, 3);

    // R13.4 probe: full flip path. Each call may resolve to a real
    // cellGcmSys PRX NID; the probe runs strict-syscall to surface
    // whichever one is the first unmet dependency. R13.4a empirical
    // finding (uncommitted): gcmSetDisplayBuffer alone runs clean
    // (returns 0 via unimplemented-import fast-path; PSL1GHT
    // tolerates). Adding the rest below.
    gcmSetDisplayBuffer(0, 0, 640 * 4, 640, 480);
    gcmSetFlip(ctx, 0);
    rsxFlushBuffer(ctx);
    while (gcmGetFlipStatus() != 0) { /* spin */ }
    gcmResetFlipStatus();

    rsxSetWriteCommandLabel(ctx, 0, 0x12345678);

    return 0xC0DE;
}
