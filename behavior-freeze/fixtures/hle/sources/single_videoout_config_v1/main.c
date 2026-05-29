// single_videoout_config_v1 — cellVideoOutGetConfiguration HLE fixture.
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellVideoOutGetConfiguration(PRIMARY, &cfg, NULL) via PSL1GHT's
// videoGetConfiguration. Reuses the VideoOutManager field added for
// single_videoout_resavail_v1 — only the GetConfiguration NID is new. Pre-wire
// the return-0 import stub never writes the struct, so cfg.resolution stays 0;
// once routed, the crate returns the primary port's config (default 720p), and
// the arm serialises resolution(@0)/format(@1)/aspect(@2)/pitch(@12, BE u32).
//
//   getconfig != 0 -> return 0x0BAD       (call error)
//   else           -> return cfg.resolution  (0 pre-wire; 2 = 720p post-wire)
//
// Behaviour: rsx-free, SPU-free, pure cellVideoOut HLE call.

#include <ppu-types.h>
#include <sysutil/video.h>
#include <sys/process.h>
#include <string.h>

SYS_PROCESS_PARAM(1001, 0x10000);

int main(void)
{
    videoConfiguration cfg;
    memset(&cfg, 0, sizeof(cfg)); // sentinel: the return-0 stub never writes

    s32 r = videoGetConfiguration(VIDEO_PRIMARY, &cfg, NULL);
    if (r != 0) {
        return 0x0BAD;
    }
    return cfg.resolution; // 0 pre-wire (struct untouched); 2 (720p) post-wire
}
