// game_like_mailbox_signal_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R6.6 game-like fixture: combines IN_MBOX + SNR1 inputs with a
// 16-element LS-buffered checksum loop and a final mixing loop.
// Designed to exercise multiple bridge paths in one program:
//   - rdch ch29 (IN_MBOX, R5.9e.7 path)
//   - stqd / lqd via volatile buffer (R5.11b loadstore path)
//   - branch + loop ISA (R5.11 branch_loop path)
//   - rdch ch3 (SNR1, R5.11/R6.3c signal path)
//   - StallRead between IN_MBOX consumption and SNR1 read
//     (R6.4b multi-round persistent-handle path)
//   - wrch ch28 + stop 0x101 (R5.9e.7 group-exit path)
//
// Behaviour (deterministic):
//   seed = rdch ch29  (PPU pushes 0x21)
//   buf[i] = (seed << 4) ^ i,  for i in 0..16
//   cs = seed
//   loop 1 (16 iters):
//     v = buf[i]                  ; lqd from volatile LS
//     cs = cs + v                 ; accumulate
//     cs = cs ^ (cs << 1)         ; mix via shift + xor
//   sig = rdch ch3   (PPU sends signal 0x07)  ← StallRead point
//   loop 2 (8 iters):
//     cs = cs + sig
//     cs = cs ^ buf[i]            ; lqd from same buffer
//   wrch ch28, cs                 ; final OUT_MBOX = 0x051A03C9
//   stop 0x101
//
// Canonical inputs (seed=0x21, sig=0x07) produce OUT_MBOX =
// 0x051A03C9. Verified by reference Python computation in the
// fixture's README.md. Different inputs produce different
// outputs; the bit-mixing makes the output a sensitive function
// of BOTH inputs (changing seed by 1 bit OR sig by 1 bit
// cascades through all 32 bits of the result).
//
// Inlined exit (avoid pulling libsputhread which has ROTQBY etc.
// outside the iteration-1 SPU interpreter subset).

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

int main(uint64_t spu_id, uint64_t arg)
{
    (void)spu_id;
    (void)arg;

    // Round 1: read seed from IN_MBOX (ch29).
    uint32_t seed = spu_read_in_mbox();

    // Stack-allocated volatile buffer forces real LS round-trips
    // (stqd in init loop, lqd in mix loops). 64 bytes / 16 words.
    volatile uint32_t buf[16];

    // Initialize buf with pattern that depends on seed.
    for (uint32_t i = 0; i < 16; i++) {
        buf[i] = (seed << 4) ^ i;
    }

    // Loop 1: 16-iter mix combining LS reads + shift/xor.
    uint32_t cs = seed;
    for (uint32_t i = 0; i < 16; i++) {
        uint32_t v = buf[i];
        cs = cs + v;
        cs = cs ^ (cs << 1);
    }

    // Round 2: read signal from SNR1 (ch3). The PPU has been
    // sleeping ~100ms via sysUsleep, so the SPU has had time to
    // consume IN_MBOX, run loop 1, and reach this rdch — at
    // which point ch3 is still empty and the SPU parks. The
    // bridge's persistent-handle multi-round loop handles the
    // park+wake transparently.
    uint32_t sig = spu_read_signal1();

    // Loop 2: 8-iter final mix.
    for (uint32_t i = 0; i < 8; i++) {
        cs = cs + sig;
        cs = cs ^ buf[i];
    }

    // Inlined SYS_SPU_THREAD_STOP_GROUP_EXIT.
    spu_writech(SPU_WrOutMbox, cs);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
