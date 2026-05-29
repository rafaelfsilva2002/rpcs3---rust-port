// single_spu_selfcompute_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// SELF-CONTAINED: takes NO input — no IN_MBOX read, no DMA, no thread args.
// It runs a fixed integer compute loop, writes the result to OUT_MBOX (ch28),
// and halts with stop 0x101.
//
// Why no input: EmuCore::run_self executes the SPU group SYNCHRONOUSLY at
// sys_spu_thread_group_start (it runs the SPU to completion before the PPU
// continues). So any SPU that blocks on IN_MBOX waiting for a later PPU push
// (every existing mailbox/signal/branch oracle) stalls forever in that model,
// and the DMA oracles need live EA<->LS transfer the standalone interpreter
// doesn't perform. A no-input compute SPU is the one shape that boots cleanly
// through that path — used to validate the SPU JIT backend end-to-end.
//
// Kept inside the SPU interpreter's opcode subset (il/ai/a/load-store/branch
// + spu_writech + spu_stop), like single_spu_mailbox_v1.

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

int main(uint64_t spu_id, uint64_t arg)
{
    (void)spu_id;
    (void)arg;

    // `volatile` on the bound forces the loop to run at runtime (defeats
    // constant-folding), so the recompiler actually compiles + executes a
    // real hot loop rather than a folded constant.
    volatile uint32_t n = 1000u;
    uint32_t acc = 0u;
    for (uint32_t i = 1u; i <= n; i++) {
        acc += i; // sum(1..=1000) = 500500 = 0x0007A314
    }

    // Inlined spu_thread_group_exit (avoids linking libsputhread, which would
    // pull in ROTQBY etc. outside the interpreter subset):
    spu_writech(SPU_WrOutMbox, acc);
    spu_stop(0x101);

    return 0; // unreachable
}
