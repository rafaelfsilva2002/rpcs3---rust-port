// single_gcm_clear_v1 — PPU-only RSX command-stream capture fixture
// CC0 1.0 (public domain). See LICENSE.md.
//
// R12.11b — produces a REAL GCM command stream from PSL1GHT's own
// libgcm/librsx command-emission functions (Tier 3 byte origin), then
// dumps the raw command-buffer bytes via sysTtyWrite so the emulator
// can capture them and feed them to the Rust decoder (replay_gcm).
//
// Crucially this does NOT call gcmInitDefault / map RSX IO memory: the
// rsxSet*/rsxClearSurface functions just write NV4097 method words
// into the command buffer the context points at (pure memory writes,
// no GPU). We set up a gcmContextData by hand over a static buffer,
// emit a small frame, and dump [begin, current).
//
// Behaviour:
//   set clear color -> clear surface -> write-command-label (frame
//   marker) -> sysTtyWrite a HEX dump of the command words to TTY
//   channel 0 -> return 0xC0DE.
//
// The dump is one 8-digit lowercase hex word per line (the same
// `.gcmhex` format the Rust oracle parses). Hex is emitted (not raw
// bytes) because the emulator's TTY capture is UTF-8 lossy; hex is
// ASCII-safe and round-trips exactly.

#include <ppu-types.h>
#include <rsx/rsx.h>
#include <sys/tty.h>
#include <sys/process.h>

SYS_PROCESS_PARAM(1001, 0x10000);

// Command buffer the manual context writes into. 1024 words is far
// more than this small frame needs, so the context callback (which we
// leave NULL) is never triggered.
static u32 cmdbuf[1024];

// Hex output buffer: up to 9 chars (8 hex + newline) per word.
static char hexbuf[1024 * 9];
static const char HEX[] = "0123456789abcdef";

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    gcmContextData ctx;
    ctx.begin = cmdbuf;
    ctx.current = cmdbuf;
    ctx.end = cmdbuf + (sizeof(cmdbuf) / sizeof(cmdbuf[0]));
    ctx.callback = (gcmContextCallback)0; // never called: buffer is large

    // Real PSL1GHT command emission (inline; writes method words).
    rsxSetClearColor(&ctx, 0xff202020);
    rsxClearSurface(&ctx, 0xf3); // color + depth + stencil
    rsxSetWriteCommandLabel(&ctx, 0, 0x12345678); // frame-end marker

    // Words written so far = current - begin.
    u32 len_words = (u32)(ctx.current - ctx.begin);

    // Format each command word as 8 lowercase hex digits + newline.
    u32 p = 0;
    for (u32 i = 0; i < len_words; i++) {
        u32 w = cmdbuf[i];
        for (int s = 28; s >= 0; s -= 4) {
            hexbuf[p++] = HEX[(w >> s) & 0xf];
        }
        hexbuf[p++] = '\n';
    }

    u32 written = 0;
    sysTtyWrite(0, (const void *)hexbuf, p, &written);

    return 0xC0DE;
}
