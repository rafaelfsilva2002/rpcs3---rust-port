// single_spu_loadstore_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// Behaviour: blocking-read one 32-bit `seed` from IN_MBOX, write a
// deterministic 8-word pattern derived from `seed` into a stack-
// allocated `volatile` Local Store buffer (forces stqd emission
// against r1-relative offsets), read the buffer back (forces lqd
// emission), accumulate a sum-checksum, write the checksum to
// OUT_MBOX, halt with stop 0x101.
//
// `volatile` on the buffer is load-bearing — without it, GCC -O2
// can keep the values in registers across the two loops and skip
// LS access entirely, defeating the purpose of the fixture. The
// buffer lives on the SPU stack (r1-relative), so allocation is
// `ai r1, r1, -32` from the kernel-set initial SP (0x3FFF0).
//
// The pattern `(seed << 4) | i` keeps each store value cheap to
// compute (shli + or-immediate, both in iteration-1 ISA) and
// produces a checksum that depends meaningfully on the seed
// (sum = 8*(seed << 4) + 28 = (seed << 7) + 28). The XOR-based
// alternative for 8 consecutive integers cancels to zero
// regardless of seed, which would be a poor regression sentinel.
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

    // Blocking read of one 32-bit value from the PPU via IN_MBOX (ch29).
    uint32_t seed = spu_read_in_mbox();

    // Stack-allocated volatile Local Store buffer (32 bytes / 8 words /
    // 2 quadwords). `volatile` forces real LS round-trips: stqd in the
    // first loop, lqd in the second.
    volatile uint32_t buffer[8];

    // Store: deterministic pattern that depends on `seed`.
    for (uint32_t i = 0; i < 8; i++) {
        buffer[i] = (seed << 4) | i;
    }

    // Load + sum-checksum.
    uint32_t cs = 0;
    for (uint32_t i = 0; i < 8; i++) {
        cs += buffer[i];
    }

    // Inlined SYS_SPU_THREAD_STOP_GROUP_EXIT.
    spu_writech(SPU_WrOutMbox, cs);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
