// single_game_paramint_v1 — cellGameGetParamInt HLE fixture (probe).
// CC0 1.0 (public domain). See LICENSE.md.
//
// Calls cellGameGetParamInt(PARENTAL_LEVEL, &v) via PSL1GHT's sysGameGetParamInt.
// NOTE the param-id numbering: PSL1GHT's sysutil/game.h omits APP_VERSION, so its
// SYS_GAME_PARAMID_* values diverge from the real PS3 / RPCS3 numbering for ids
// >= 102. We pass the raw real-PS3 id 103 (CELL_GAME_PARAMID_PARENTAL_LEVEL),
// which rpcs3-hle-cellgame treats as an INT parameter.
//
// Pre-wire the return-0 import stub never writes `v`; once routed to
// rpcs3-hle-cellgame backed by a fixed GameState provider, v = the configured
// parental level.
//
//   ret != 0 -> return 0x0BAD     (call error)
//   else     -> return v           (0x55 sentinel pre-wire; provider value post-wire)
//
// Behaviour: rsx-free, SPU-free, pure cellGame HLE call.

#include <ppu-types.h>
#include <sysutil/game.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

#define CELL_GAME_PARAMID_PARENTAL_LEVEL 103 /* real PS3 / RPCS3 numbering */

int main(void)
{
    s32 v = 0x55; // sentinel: the return-0 stub never writes this
    s32 ret = sysGameGetParamInt(CELL_GAME_PARAMID_PARENTAL_LEVEL, &v);
    if (ret != 0) {
        return 0x0BAD;
    }
    return v; // 0x55 pre-wire; configured parental level post-wire
}
