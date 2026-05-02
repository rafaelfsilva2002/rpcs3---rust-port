// single_spu_mailbox_multi_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R6.4b stall-bound oracle fixture. The SPU consumes ONE value
// from IN_MBOX (round 1), holds the partial result in r3 across
// a guaranteed stall, then consumes ONE value from SNR1 (round 2)
// and produces a final OUT_MBOX that depends on BOTH inputs.
//
// Why mixed IN_MBOX + SNR1: PSL1GHT's lv2 syscall surface for
// cooperative SPU threads exposes `sysSpuThreadWriteMb` (PPU →
// IN_MBOX) and `sysSpuThreadWriteSignal` (PPU → SNR1/SNR2), but
// has NO public path for the PPU to read a cooperative SPU's
// OUT_MBOX. So a round 1 OUT_MBOX → PPU drain → round 2 IN_MBOX
// design (the FFI test's pattern) cannot be expressed end-to-end
// via PSL1GHT. Using IN_MBOX for round 1 + SNR1 for round 2
// produces the SAME load-bearing behaviour the C++↔Rust bridge
// needs to handle (a guaranteed StallRead between two PPU-pushed
// values) while staying inside the PSL1GHT-supportable API.
//
// Behaviour:
//   1. Block on IN_MBOX (ch29). Read 32-bit `cmd1` from PPU.
//   2. Compute partial = cmd1 + 0xA1; hold in r3.
//      (NO OUT_MBOX write yet — final output reflects BOTH rounds.)
//   3. Block on SNR1 (ch3). Read 32-bit `cmd2` from PPU. ← StallRead
//      surfaces here on the first run; the persistent-handle bridge
//      must keep the rust_spu_t* alive, drain the corresponding
//      RPCS3 channel via pop_wait, and resume on the same handle.
//   4. Compute reply = (cmd2 + 0xB2) + partial.
//      For the canonical inputs cmd1 = 0x100, cmd2 = 0x200:
//          partial = 0x100 + 0xA1 = 0x1A1
//          (cmd2 + 0xB2) = 0x200 + 0xB2 = 0x2B2
//          reply   = 0x1A1 + 0x2B2 = 0x453
//      0x453 is observable IFF the SPU saw BOTH inputs. If round 1
//      was silently skipped (e.g. a buggy stateless bridge replayed
//      the program from PC=0 after refilling SNR1), partial would
//      be 0 and reply would be 0x2B2 — a detectable corruption.
//   5. Write reply to OUT_MBOX (ch28).
//   6. Stop with code 0x101 (= SYS_SPU_THREAD_STOP_GROUP_EXIT).
//      lv2 kernel reads OUT_MBOX as the group-exit status.
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

    // Round 1: read IN_MBOX (ch29), accumulate.
    uint32_t cmd1 = spu_read_in_mbox();
    uint32_t partial = cmd1 + 0xA1u;

    // Round 2: read SNR1 (ch3). The SPU executor parks here
    // until the PPU calls sysSpuThreadWriteSignal(slot=0, value).
    uint32_t cmd2 = spu_read_signal1();

    // Final reply combines both rounds.
    uint32_t reply = (cmd2 + 0xB2u) + partial;

    // Inlined SYS_SPU_THREAD_STOP_GROUP_EXIT.
    spu_writech(SPU_WrOutMbox, reply);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
