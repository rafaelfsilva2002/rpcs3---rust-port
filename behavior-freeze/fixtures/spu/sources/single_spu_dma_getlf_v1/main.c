// single_spu_dma_getlf_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.4f-a — first MFC GETLF list-DMA + fence oracle (16th
// oracle target). Identical to R8.4c GETL except cmd=0x46
// (GETL | MFC_FENCE_MASK).

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_getlf_bin[];
extern const u32 spu_dma_getlf_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

static u8 ea_buf1[128] __attribute__((aligned(128)));
static u8 ea_buf2[64]  __attribute__((aligned(128)));

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    for (u32 i = 0; i < 128; i++) {
        ea_buf1[i] = (u8)(i & 0xFF);
    }
    for (u32 i = 0; i < 64; i++) {
        ea_buf2[i] = 0x42;
    }

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[dma_getlf_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_getlf_bin, 0);
    if (ret) {
        printf("[dma_getlf_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_getlf_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_getlf_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);
    thread_args.arg0 = (u64)(uintptr_t)ea_buf1;
    thread_args.arg1 = (u64)(uintptr_t)ea_buf2;

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[dma_getlf_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_getlf_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[dma_getlf_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Predicted: cause=0x1, status=0xDF1EEA7F
    printf("[dma_getlf_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
