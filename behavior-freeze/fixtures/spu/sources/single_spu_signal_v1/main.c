// single_spu_signal_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// Loads a single SPU thread, sends a signal-notification value via
// sysSpuThreadWriteSignal (signal=0, target SNR1), joins the SPU
// group, exits. Same race-free single-round shape as the mailbox
// fixtures — no DMA, no PPU spin-loop on OUT_MBOX.
//
// Behaviour:
//   1. PPU writes 0x1234 to SNR1 via sysSpuThreadWriteSignal.
//   2. SPU reads 0x1234 from ch3 (SPU_RdSigNotify1), computes
//      reply = 0x1234 + 0xFEED = 0x11121, writes to OUT_MBOX (ch28),
//      halts with stop 0x101.
//   3. PPU joins; lv2 reads OUT_MBOX as the group-exit status (= 0x11121).

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_signal_bin[];
extern const u32 spu_signal_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[sig_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_signal_bin, 0);
    if (ret) {
        printf("[sig_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "sig_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[sig_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
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
        printf("[sig_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[sig_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    // Send signal notification to SNR1 (signal index 0).
    // SPU's first instruction is `rdch ch3 (SPU_RdSigNotify1)` which
    // blocks until this fires.
    ret = sysSpuThreadWriteSignal(spu_thread_id, 0, 0x1234u);
    if (ret) {
        printf("[sig_v1] sysSpuThreadWriteSignal: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[sig_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    printf("[sig_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
