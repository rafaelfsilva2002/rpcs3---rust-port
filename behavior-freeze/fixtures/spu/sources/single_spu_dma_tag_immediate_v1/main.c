// single_spu_dma_tag_immediate_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.3c — first IMMEDIATE-wait-mode oracle (12th oracle).
// Mirrors R8.3b shape (two queued GETs + two ch24 reads in the
// same SPU session) but uses `WrTagUpdate = IMMEDIATE` (= 0)
// instead of ANY (= 1). The two reads use different masks
// (0x08 then 0x28) to probe whether the IMMEDIATE read clears
// bits from `completed_tags`.
//
// Hypothesis (to be confirmed by real RPCS3 capture):
//
// - Cell BE / C++ RPCS3 semantic: IMMEDIATE returns
//   `completed_tags & wr_tag_mask` WITHOUT waiting AND WITHOUT
//   clearing. Both reads return their per-mask subset of the
//   persistent `completed_tags = 0x28`.
//
//   Predicted ts1 = 0x08 (mask 0x08), ts2 = 0x28 (mask 0x28).
//
// - If RPCS3 implements per-bit clear on IMMEDIATE read:
//
//   ts1 = 0x08; after the read, tag 3 bit cleared from
//   completed_tags (now 0x20). ts2 with mask 0x28 = 0x20.
//
// - If RPCS3 implements full clear on IMMEDIATE read:
//
//   ts1 = 0x08; ts2 = 0 (completed_tags cleared entirely).
//
// The fixture captures whichever behavior RPCS3 actually
// produces and locks it as canonical via the embed.
//
// Status formula (independent of which behavior RPCS3 shows):
//
//   sum1     = 0x1FC0  (counting pattern, 128 B)
//   sum2     = 0x1080  (constant 0x42, 64 B)
//   combined = (sum1 << 16) | sum2          = 0x1FC0_1080
//   packed   = (ts1 << 24) | (ts2 << 16)
//   status   = combined ^ packed ^ 0xCAFE5A1E
//
// Predicted statuses per behavior:
//   No clear (Cell BE canonical):
//     ts1=0x08 ts2=0x28 packed=0x0828_0000 status=0xDD164A9E
//   Per-bit clear on read:
//     ts1=0x08 ts2=0x20 packed=0x0820_0000 status=0xDD1E_4A9E (= R8.3b-1ish; coincidence not load-bearing)
//   Full clear:
//     ts1=0x08 ts2=0x00 packed=0x0800_0000 status=0xDD3E_4A9E
//
// The XOR with `0xCAFE5A1E` (distinct from prior masks)
// ensures the status doesn't collide accidentally with R8.3a
// or R8.3b canonicals.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_tag_immediate_bin[];
extern const u32 spu_dma_tag_immediate_bin_size;

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
        printf("[dma_tag_immediate_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_tag_immediate_bin, 0);
    if (ret) {
        printf("[dma_tag_immediate_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_tag_immediate_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_tag_immediate_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
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
        printf("[dma_tag_immediate_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_tag_immediate_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[dma_tag_immediate_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Expected: cause=0x1 (GROUP_EXIT), status=<captured>.
    // Predicted no-clear: 0xDD164A9E.
    printf("[dma_tag_immediate_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
