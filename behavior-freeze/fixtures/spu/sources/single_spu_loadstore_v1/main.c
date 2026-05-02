// single_spu_loadstore_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// Loads a single SPU thread, pushes `seed = 0x10` via IN_MBOX, joins
// the SPU group, exits. Same race-free single-round shape as the
// other R5.11 fixtures.
//
// Behaviour:
//   1. PPU pushes seed=0x10 to SPU IN_MBOX via sysSpuThreadWriteMb.
//   2. SPU stores 8 words {(seed<<4)|0, ..., (seed<<4)|7} = {0x100..0x107}
//      to a stack-allocated volatile LS buffer (forces stqd emission).
//   3. SPU reads the buffer back (forces lqd emission), sums the 8
//      values: cs = 8*0x100 + (0+1+...+7) = 0x800 + 28 = 0x81C.
//   4. SPU writes cs=0x81C to OUT_MBOX, halts with stop 0x101.
//   5. PPU joins; lv2 reads OUT_MBOX as the group-exit status (= 0x81C).

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_loadstore_bin[];
extern const u32 spu_loadstore_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[ldst_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_loadstore_bin, 0);
    if (ret) {
        printf("[ldst_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "ldst_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[ldst_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
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
        printf("[ldst_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[ldst_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    // seed = 0x10 → cs = 8*((0x10<<4) | i_avg) ≈ 0x81C
    sysSpuThreadWriteMb(spu_thread_id, 0x10u);

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[ldst_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    printf("[ldst_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
