// single_spu_mailbox_multi_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R6.4b stall-bound fixture. PPU drives a TWO-round handshake
// using IN_MBOX for round 1 and SNR1 for round 2. The two-rounds
// shape produces a guaranteed StallRead between them — exactly
// what the C++ ↔ Rust SPU bridge's persistent-handle path
// (R6.4b) needs to handle.
//
// Why IN_MBOX for round 1 + SNR1 for round 2:
//   PSL1GHT exposes `sysSpuThreadWriteMb` (PPU → IN_MBOX) and
//   `sysSpuThreadWriteSignal` (PPU → SNR1/SNR2) but does NOT
//   expose a way for the PPU to read a cooperative SPU's
//   OUT_MBOX, so a round 1 OUT_MBOX → PPU drain → round 2
//   IN_MBOX design isn't expressible via PSL1GHT. Mixing the
//   two channels delivers the same load-bearing behaviour
//   (a SPU stall between two PPU-pushed values) while staying
//   inside the supported syscall surface.
//
// Behaviour:
//   1. PPU pushes 0x100 → IN_MBOX via sysSpuThreadWriteMb.
//   2. SPU reads, computes partial = 0x100 + 0xA1 = 0x1A1,
//      then blocks on SNR1.
//   3. PPU sends 0x200 → SNR1 via sysSpuThreadWriteSignal.
//   4. SPU reads SNR1 → 0x200, computes
//      reply = (0x200 + 0xB2) + partial = 0x2B2 + 0x1A1 = 0x453,
//      writes OUT_MBOX, stops 0x101.
//   5. PPU joins; lv2 reads OUT_MBOX = 0x453 as the group-exit
//      status. status = 0x453 proves BOTH rounds were observed.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <sys/systime.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_mailbox_multi_bin[];
extern const u32 spu_mailbox_multi_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[mbmulti_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_mailbox_multi_bin, 0);
    if (ret) {
        printf("[mbmulti_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "mbmulti_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[mbmulti_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[mbmulti_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[mbmulti_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    // Round 1: push 0x100 to IN_MBOX. SPU reads, computes partial,
    // then blocks on SNR1.
    sysSpuThreadWriteMb(spu_thread_id, 0x100u);

    // Force a real SPU stall before round 2: sleep 100 ms so the
    // SPU thread is dispatched, consumes IN_MBOX, advances to the
    // `rdch ch3` instruction, and parks (SNR1 still empty). The
    // delay turns this fixture from "PPU buffers both inputs before
    // SPU runs" (one shot, no park) into "PPU sends → SPU consumes
    // → SPU parks → PPU sends second → SPU resumes" — the
    // multi-round/stall shape that the C++↔Rust SPU bridge's
    // persistent-handle path (R6.4b) must handle, and that the
    // replay engine's per-SPU transformer expects.
    sysUsleep(100000u);  // 100ms

    // Round 2: send 0x200 to SNR1 (signal slot 0). SPU resumes.
    ret = sysSpuThreadWriteSignal(spu_thread_id, 0, 0x200u);
    if (ret) {
        printf("[mbmulti_v1] sysSpuThreadWriteSignal: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[mbmulti_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Expected: cause=0x1 (GROUP_EXIT), status=0x453 (= 0x1A1 + 0x2B2).
    printf("[mbmulti_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
