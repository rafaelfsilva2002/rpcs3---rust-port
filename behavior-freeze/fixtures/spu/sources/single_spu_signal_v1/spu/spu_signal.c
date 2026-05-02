// single_spu_signal_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// Behaviour: blocking-read one 32-bit signal from SNR1 (channel 3 —
// SPU_RdSigNotify1), compute reply = sig + 0xFEED, write reply to
// OUT_MBOX (ch28), halt with stop 0x101 (SYS_SPU_THREAD_STOP_GROUP_EXIT).
//
// Same race-free single-round shape as single_spu_mailbox_v1 and
// single_spu_branch_loop_v1, but exercises the signal-notification
// channel (ch3 / SNR1) instead of IN_MBOX (ch29). The Rust replay
// pipeline already supports signal channels via `wake_kind_for_signal`
// (slot 0 → ch3, slot 1 → ch4) in the per-SPU transformer and via
// `PpuAction::Signal { slot, value }` in the lockstep driver, both
// added before R5.9e.7. This fixture exercises that path end-to-end
// against a real captured trace.
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

    // Blocking read of one 32-bit value from the PPU via SNR1 (ch3).
    uint32_t sig = spu_read_signal1();

    // Compute reply.
    uint32_t reply = sig + 0xFEEDu;

    // Inlined SYS_SPU_THREAD_STOP_GROUP_EXIT.
    spu_writech(SPU_WrOutMbox, reply);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
