// single_spu_dma_putlb_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.4f-b — first MFC PUTLB list-DMA + barrier oracle (17th
// oracle target). Identical to R8.4e PUTL except cmd=0x25
// (PUTL | MFC_BARRIER_MASK). Per do_list_transfer the
// barrier bit is stripped before the per-element copy; per
// do_dma_check the barrier persistence has no observable
// effect for a single-SPU fresh-tag fixture.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_putlb_bin[];
extern const u32 spu_dma_putlb_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

static u8 ea_dst1[128] __attribute__((aligned(128)));
static u8 ea_dst2[64]  __attribute__((aligned(128)));

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    for (u32 i = 0; i < 128; i++) ea_dst1[i] = 0xAA;
    for (u32 i = 0; i < 64; i++)  ea_dst2[i] = 0xAA;

    ret = sysSpuInitialize(1, 0);
    if (ret) { printf("[dma_putlb_v1] sysSpuInitialize: 0x%08x\n", ret); return 1; }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_putlb_bin, 0);
    if (ret) { printf("[dma_putlb_v1] sysSpuImageImport: 0x%08x\n", ret); return 1; }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_putlb_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) { printf("[dma_putlb_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret); return 1; }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);
    thread_args.arg0 = (u64)(uintptr_t)ea_dst1;
    thread_args.arg1 = (u64)(uintptr_t)ea_dst2;

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image, &thread_attr, &thread_args);
    if (ret) { printf("[dma_putlb_v1] sysSpuThreadInitialize: 0x%08x\n", ret); return 1; }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) { printf("[dma_putlb_v1] sysSpuThreadGroupStart: 0x%08x\n", ret); return 1; }

    u32 cause, spu_sentinel;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &spu_sentinel);
    if (ret) { printf("[dma_putlb_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret); return 1; }

    u32 sum_ea1 = 0;
    for (u32 i = 0; i < 128; i++) sum_ea1 += (u32)ea_dst1[i];
    u32 sum_ea2 = 0;
    for (u32 i = 0; i < 64; i++)  sum_ea2 += (u32)ea_dst2[i];
    u32 combined = (sum_ea1 << 16) | (sum_ea2 & 0xFFFFu);
    u32 ea_status = combined ^ 0xBEEFCABBu;

    // Predicted: cause=0x1, spu=0xc0ffeebb, ea_status=0xa12fda3b
    printf("[dma_putlb_v1] OK cause=0x%x spu=0x%x ea_status=0x%x\n",
           (unsigned)cause, (unsigned)spu_sentinel, (unsigned)ea_status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
