// single_videoout_resolution_v1 — cellVideoOut HLE-crate integration fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellVideoOutGetResolution(VIDEO_RESOLUTION_720, &res) via PSL1GHT's
// videoGetResolution. This is a STATELESS table lookup (resolution id ->
// width/height), so no EmuCore state field is needed. Pre-wire, the permissive
// return-0 stub answers the import with r3=0 but never writes `res`, so the
// zero-initialised struct stays 0×0. Once emu-core routes the NID to
// rpcs3-hle-cellvideoout::cell_video_out_get_resolution, the homebrew reads the
// real 1280×720.
//
// videoResolution is { u16 width; u16 height; } (4 bytes, big-endian on PS3).
// The return value packs both fields so the EmuCore test asserts them via the
// exit code:
//   ret != 0                 -> return 0x0BAD                 (call failed)
//   ret == 0, w<<16 | h      -> return packed res             (0 pre-wire; 0x050002D0 wired)
//                                                              (0x0500=1280, 0x02D0=720)
//
// Behaviour: rsx-free, SPU-free, pure cellVideoOut HLE call.

#include <ppu-types.h>
#include <sysutil/video.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    videoResolution res;
    res.width = 0;  // sentinel: the return-0 stub leaves these unwritten
    res.height = 0;

    s32 ret = videoGetResolution(VIDEO_RESOLUTION_720, &res);
    if (ret != 0) {
        return 0x0BAD;
    }
    // 0 pre-wire (struct untouched); 0x050002D0 (1280<<16 | 720) once wired.
    return ((s32)res.width << 16) | (s32)res.height;
}
