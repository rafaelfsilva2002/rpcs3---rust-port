// single_spu_dma_getl_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.4b — first MFC GETL list-DMA oracle (13th oracle target).
// The PPU prepares TWO deterministic EA buffers, passes both
// addresses to the SPU via thread_args.arg0/arg1, and joins.
// The SPU constructs a 2-element list descriptor array in LS,
// dispatches MFC GETL (cmd=0x44), waits via ch22/ch23/ch24,
// computes the canonical OUT_MBOX status from copied bytes,
// halts via stop 0x101.
//
// Behaviour (deterministic):
//
// 1. PPU allocates ea_buf1 (128 B counting pattern) and
//    ea_buf2 (64 B constant 0x42) — same patterns as
//    R8.2 / R8.3a/b/c so the captured .dmachunk side-files
//    deduplicate with the existing canonical pool.
// 2. PPU passes EA1 via arg0, EA2 via arg1.
// 3. SPU builds a 2-element list_element[] in LS at a
//    statically allocated offset:
//      list[0] = { sb=0, pad=0, ts=128, ea=EA1 }
//      list[1] = { sb=0, pad=0, ts= 64, ea=EA2 }
// 4. SPU dispatches MFC GETL:
//      ch16 LSA      = 0x10000          (destination base)
//      ch17 EAH      = 0
//      ch18 EAL      = LS offset of list (NOT data EA — Cell
//                                          BE GETL convention)
//      ch19 Size     = 16               (= 2 * sizeof(list_element))
//      ch20 TagID    = 3
//      ch21 Cmd      = 0x44 GETL
// 5. RPCS3 reads the descriptor list from LS, walks elements,
//    copies each `ts` bytes from EA=item.ea into LS at
//    cumulative offset:
//      element 0: LS[0x10000..0x10080] ← ea_buf1 (counting)
//      element 1: LS[0x10080..0x100C0] ← ea_buf2 (0x42)
// 6. SPU waits via ch22/ch23/ch24 (mask=0x08, ALL).
// 7. SPU sums both LS regions:
//      sum1 = sum(LS[0x10000..0x10080]) = 0x1FC0
//      sum2 = sum(LS[0x10080..0x100C0]) = 0x1080
//      combined = (sum1 << 16) | sum2  = 0x1FC0_1080
//      status   = combined ^ 0xC0DEFADA = 0xDF1E_EA5A
// 8. SPU writes status to OUT_MBOX, halts with stop 0x101.
// 9. PPU joins; lv2 reads OUT_MBOX as group-exit status.
//
// Predicted canonical TTY (verify on real RPCS3):
//   `[dma_getl_v1] OK cause=0x1 status=0xdf1eea5a`
//
// Failure mode catalogue:
//   - List dispatch dropped (zero-fill LS): status = 0xC0DEFADA.
//   - Element 0 dropped: status = sum2-only XOR mask.
//   - Element 1 dropped: status = sum1-only XOR mask.
//   - Descriptor format wrong (swapped elements / wrong EA
//     interpretation): distinctively wrong sums.
//
// Canonical computation (Python reference):
//   buf1 = [i & 0xFF for i in range(128)]
//   buf2 = [0x42 for _ in range(64)]
//   sum1 = sum(buf1)                       # 0x1FC0
//   sum2 = sum(buf2)                       # 0x1080
//   combined = (sum1 << 16) | sum2         # 0x1FC01080
//   status   = combined ^ 0xC0DEFADA       # 0xDF1EEA5A

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_getl_bin[];
extern const u32 spu_dma_getl_bin_size;

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
        printf("[dma_getl_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_getl_bin, 0);
    if (ret) {
        printf("[dma_getl_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_getl_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_getl_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
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
        printf("[dma_getl_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_getl_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[dma_getl_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Predicted: cause=0x1, status=0xDF1EEA5A (verify by capture).
    printf("[dma_getl_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
