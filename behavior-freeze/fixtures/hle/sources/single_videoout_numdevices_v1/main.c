// single_videoout_numdevices_v1 — cellVideoOut device-count HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellVideoOutGetNumberOfDevice(VIDEO_PRIMARY) via PSL1GHT's
// videoGetNumberOfDevice. Like cellVideoOutGetResolution this is STATELESS, and
// it returns the count directly in r3 (no OUT pointer). Pre-wire the permissive
// return-0 stub returns r3=0; once emu-core routes the NID to
// rpcs3-hle-cellvideoout::cell_video_out_get_number_of_device, the primary port
// reports 1 connected device.
//
//   ret < 0   -> return 0x0BAD     (call error)
//   ret == n  -> return n          (0 pre-wire; 1 post-wire = primary connected)
//
// Behaviour: rsx-free, SPU-free, pure cellVideoOut HLE call.

#include <ppu-types.h>
#include <sysutil/video.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    s32 n = videoGetNumberOfDevice(VIDEO_PRIMARY);
    if (n < 0) {
        return 0x0BAD;
    }
    return n; // 0 pre-wire (return-0 stub); 1 post-wire (primary connected)
}
