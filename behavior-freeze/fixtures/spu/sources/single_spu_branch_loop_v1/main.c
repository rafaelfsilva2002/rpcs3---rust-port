// single_spu_branch_loop_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// Loads a single SPU thread, pushes ONE command (cmd=10) via IN_MBOX,
// joins the SPU group, exits. Same race-free single-round shape as
// single_spu_mailbox_v1 (no DMA, no PPU spin-loop on OUT_MBOX —
// avoids PSL1GHT cooperative-thread MMIO complications).
//
// Behaviour:
//   1. PPU pushes cmd=10 to SPU IN_MBOX via sysSpuThreadWriteMb.
//   2. SPU runs Fibonacci loop for 10 iterations, computing Fib(10)=89.
//   3. SPU writes 89 to OUT_MBOX, halts with stop 0x101.
//   4. PPU joins the SPU group; lv2 kernel reads OUT_MBOX as the
//      group-exit status (= 89 = 0x59) and reports it via cause/status.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_branch_loop_bin[];
extern const u32 spu_branch_loop_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[brloop_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_branch_loop_bin, 0);
    if (ret) {
        printf("[brloop_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "brloop_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[brloop_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
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
        printf("[brloop_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[brloop_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    // Push exactly ONE command. cmd=10 → SPU computes Fib(10)=89.
    sysSpuThreadWriteMb(spu_thread_id, 10u);

    // Wait for the group to terminate. SPU's stop 0x101 ends the group;
    // lv2 reads OUT_MBOX (= 89) as the exit status.
    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[brloop_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    printf("[brloop_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
