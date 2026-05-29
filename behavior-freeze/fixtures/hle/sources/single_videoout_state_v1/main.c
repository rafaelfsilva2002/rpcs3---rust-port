// single_videoout_state_v1 — cellVideoOutGetState HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellVideoOutGetState(PRIMARY, 0, &state) via PSL1GHT's videoGetState.
// Sixth cellVideoOut function, reusing the VideoOutManager field — only the
// GetState NID is new. Pre-wire the return-0 import stub never writes the struct;
// once routed, emu-core fills the videoState (the primary is enabled at 720p).
//
// videoState layout: state@0 (u8), colorSpace@1 (u8), padding[6], displayMode@8
// { resolution@8, scanMode@9, conversion@10, aspect@11, padding[2], refreshRates@14 }.
// The state byte is ENABLED=0 (not distinguishable from the stub), so we key off
// colorSpace (1) and displayMode.resolution (720p=2):
//
//   ret != 0 -> return 0x0BAD
//   else     -> return (colorSpace << 8) | resolution   (0 pre-wire; 0x102 post-wire)
//
// Behaviour: rsx-free, SPU-free, pure cellVideoOut HLE call.

#include <ppu-types.h>
#include <sysutil/video.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    videoState st;
    memset(&st, 0, sizeof(st)); // sentinel: the return-0 stub never writes

    s32 r = videoGetState(VIDEO_PRIMARY, 0, &st);
    if (r != 0) {
        return 0x0BAD;
    }
    // 0 pre-wire; (1<<8)|2 = 0x102 post-wire (colorSpace=1, resolution=720p=2).
    return ((s32)st.colorSpace << 8) | (s32)st.displayMode.resolution;
}
