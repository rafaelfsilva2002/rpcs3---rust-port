// single_spu_mailbox_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// Behaviour (canonical PSL1GHT single-round, INLINED to keep the
// executed instruction stream within the iteration-1 SPU interpreter
// subset; the libsputhread spu_thread_group_exit symbol pulls in the
// SPU C runtime + ROTQBY etc which the interpreter doesn't yet cover):
//
//   1. Block on IN_MBOX (ch29). Read 32-bit command from PPU.
//   2. Compute reply = cmd + 0x29.
//   3. Write reply to OUT_MBOX (ch28) directly via spu_writech.
//   4. Stop with code 0x101 (= SYS_SPU_THREAD_STOP_GROUP_EXIT) directly.
//
// Steps 3+4 are exactly what spu_thread_group_exit does — we just inline
// them to avoid linking libsputhread, keeping the binary minimal.

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

int main(uint64_t spu_id, uint64_t arg)
{
    (void)spu_id;
    (void)arg;

    // Blocking read of one 32-bit value from the PPU via IN_MBOX (ch29).
    uint32_t cmd = spu_read_in_mbox();

    // Compute reply.
    uint32_t reply = cmd + 0x29u;

    // Inlined spu_thread_group_exit:
    //   spu_writech(SPU_WrOutMbox, status);
    //   spu_stop(0x101);
    spu_writech(SPU_WrOutMbox, reply);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
