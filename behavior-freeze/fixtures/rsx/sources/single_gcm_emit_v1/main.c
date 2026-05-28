// single_gcm_emit_v1 — R13.2: non-empty REAL capture through the full
// cellGcm init path. CC0 1.0 (public domain). See LICENSE.md.
//
// Unlike R12.11b (single_gcm_clear_v1), which set up a gcmContextData
// BY HAND over a static buffer to dodge the cellGcm HLE, this fixture
// drives the REAL rsxInit (now unblocked by R13.1's cellGcmInitBody +
// cellGcmGetConfiguration HLE), then emits a small frame through the
// context rsxInit returns. PSL1GHT's librsx writes the NV4097 method
// words INLINE into the command buffer the context points at
// (begin = ioAddress + 4096), advancing context->current.
//
// No TTY dump and no cellGcmFlush/SetFlip (those are new NIDs): the
// emulator-side capture test reads [begin .. current) straight out of
// guest memory after the run (the R12.11a capture_command_buffer path)
// and decodes it with replay_gcm — a REAL full-gcm-path stream.
//
// Behaviour: rsxInit -> set clear color -> clear surface -> write
// command label (frame marker) -> return 0xC0DE.

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CB_SIZE   (0x10000)        // 64 KB command buffer
#define HOST_SIZE (1 * 1024 * 1024) // 1 MB io region
static u8 host_buffer[HOST_SIZE] __attribute__((aligned(1024 * 1024)));

int main(void)
{
    gcmContextData *ctx;
    rsxInit(&ctx, CB_SIZE, HOST_SIZE, host_buffer);

    // Real PSL1GHT command emission through the cellGcm-init'd context
    // (inline NV4097 method writes into the io command buffer).
    rsxSetClearColor(ctx, 0xff202020);
    rsxClearSurface(ctx, 0xf3); // color + depth + stencil
    rsxSetWriteCommandLabel(ctx, 0, 0x12345678); // frame-end marker

    return 0xC0DE;
}
