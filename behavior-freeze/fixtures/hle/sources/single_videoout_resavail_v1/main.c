// single_videoout_resavail_v1 — cellVideoOutGetResolutionAvailability fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellVideoOutGetResolutionAvailability(PRIMARY, 720p, 16:9, 0) via
// PSL1GHT's videoGetResolutionAvailability. Stateful: the result comes from
// EmuCore's VideoOutManager (default supported_resolutions includes 720p). The
// count is returned directly in r3. Pre-wire the return-0 import stub gives 0;
// once routed, the primary port reports 720p as available (1).
//
//   ret == n -> return n   (0 pre-wire; 1 post-wire = 720p available)
//
// Behaviour: rsx-free, SPU-free, pure cellVideoOut HLE call.

#include <ppu-types.h>
#include <sysutil/video.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    s32 r = videoGetResolutionAvailability(
        VIDEO_PRIMARY, VIDEO_RESOLUTION_720, VIDEO_ASPECT_16_9, 0);
    return r; // 0 pre-wire (return-0 stub); 1 post-wire (720p supported)
}
