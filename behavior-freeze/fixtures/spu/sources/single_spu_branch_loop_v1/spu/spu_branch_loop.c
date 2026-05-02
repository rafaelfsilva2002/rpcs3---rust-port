// single_spu_branch_loop_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// Behaviour: blocking-read one 32-bit `cmd` from IN_MBOX, run a
// deterministic Fibonacci-style loop for `cmd` iterations using
// pure 32-bit adds + branch (no multiplication, no DMA, no extra
// channels), write Fib(cmd) to OUT_MBOX, halt with stop 0x101
// (SYS_SPU_THREAD_STOP_GROUP_EXIT — the lv2 kernel reads OUT_MBOX
// as the group-join exit status).
//
// Why Fibonacci: pure 32-bit adds + comparison + back-edge branch
// gives the compiler nothing to close-form (Binet's formula is
// not in any SPU compiler's strength-reduction set), so we get a
// genuine loop with branch instructions in the captured trace.
// No multiplication keeps the produced binary inside the
// interpreter subset that R5.10a..p has cleared.
//
// Inlined exit (avoid pulling libsputhread which has ROTQBY etc.
// outside the iteration-1 SPU interpreter subset):
//   spu_writech(SPU_WrOutMbox, b);
//   spu_stop(0x101);

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

int main(uint64_t spu_id, uint64_t arg)
{
    (void)spu_id;
    (void)arg;

    // Blocking read of one 32-bit value from the PPU via IN_MBOX (ch29).
    uint32_t cmd = spu_read_in_mbox();

    // Fibonacci loop. For cmd iterations:
    //   t = a + b; a = b; b = t;
    // Starting (a, b) = (0, 1), after `cmd` iterations `b` is Fib(cmd).
    // (Fib(10) = 89, Fib(15) = 610, Fib(20) = 6765 — all fit in 32-bit.)
    uint32_t a = 0;
    uint32_t b = 1;
    for (uint32_t i = 0; i < cmd; i++) {
        uint32_t t = a + b;
        a = b;
        b = t;
    }

    // Inlined SYS_SPU_THREAD_STOP_GROUP_EXIT.
    spu_writech(SPU_WrOutMbox, b);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
