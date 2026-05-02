// game_like_mailbox_signal_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R6.6 game-like fixture. PPU drives a TWO-input + final-status
// handshake using IN_MBOX (round 1) + SNR1 (round 2), with a
// `sysUsleep(100ms)` between writes to ensure the SPU parks on
// the SNR1 read (forces real `spu_park`/`spu_wake` events in the
// trace and `stall_iters >= 1` in the bridge log).
//
// Behaviour:
//   1. PPU pushes seed=0x21 → IN_MBOX via sysSpuThreadWriteMb.
//   2. SPU reads, initializes a 16-word volatile LS buffer with
//      `(seed << 4) ^ i`, runs a 16-iter mix loop accumulating a
//      checksum via shift + xor, then blocks on rdch ch3.
//   3. PPU sleeps 100ms (`sysUsleep`) — gives SPU time to consume
//      IN_MBOX + run loop 1 + park on SNR1.
//   4. PPU sends sig=0x07 → SNR1 via sysSpuThreadWriteSignal.
//   5. SPU resumes, runs 8-iter final mix combining sig + buf,
//      writes the final checksum to OUT_MBOX, halts with stop
//      0x101.
//   6. PPU joins; lv2 reads OUT_MBOX = 0x051A03C9 as the group-
//      exit status (canonical for inputs seed=0x21, sig=0x07).
//
// The 0x051A03C9 status bit-encodes whether BOTH inputs reached
// the SPU AND were consumed in the right order. A bridge bug
// that drops the persistent handle on the SNR1 stall would
// re-execute loop 1 with a fresh buffer, producing a different
// (detectable) checksum.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <sys/systime.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_game_like_bin[];
extern const u32 spu_game_like_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[game_like_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_game_like_bin, 0);
    if (ret) {
        printf("[game_like_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "game_like_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[game_like_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
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
        printf("[game_like_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[game_like_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    // Round 1: push seed 0x21 → IN_MBOX. SPU reads, runs init +
    // loop 1, then blocks on rdch ch3.
    sysSpuThreadWriteMb(spu_thread_id, 0x21u);

    // Force a real SPU stall: 100ms sleep gives the SPU plenty of
    // time to consume IN_MBOX, complete loop 1 (16 iterations
    // of trivial arithmetic + LS access), and reach the rdch ch3
    // instruction at which it parks (SNR1 still empty).
    sysUsleep(100000u);

    // Round 2: send sig 0x07 → SNR1 (signal slot 0). SPU resumes.
    ret = sysSpuThreadWriteSignal(spu_thread_id, 0, 0x07u);
    if (ret) {
        printf("[game_like_v1] sysSpuThreadWriteSignal: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[game_like_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Expected: cause=0x1 (GROUP_EXIT), status=0x051A03C9
    // (canonical for inputs seed=0x21, sig=0x07; computed in
    // README.md via reference Python implementation).
    printf("[game_like_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
