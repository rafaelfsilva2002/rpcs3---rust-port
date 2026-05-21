// single_spu_dma_putl_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.4e — first MFC PUTL list-DMA oracle (14th oracle target).
// Symmetric inverse of R8.4b/c GETL: the PPU allocates two
// destination EA buffers pre-initialized to a sentinel, the SPU
// fills its LS with deterministic source content, builds a 2-
// element list descriptor in LS, dispatches MFC PUTL (cmd=0x24),
// and waits via ch22/ch23/ch24. The DMA copies SPU LS bytes
// into the EA buffers atomically. The PPU joins, sums the EA
// buffers back, and prints the canonical TTY.
//
// Behaviour (deterministic):
//
// 1. PPU allocates ea_dst1 (128 B init 0xAA) and ea_dst2 (64 B
//    init 0xAA). The pre-PUTL sentinel exists so a dropped/
//    silent PUTL produces a DIFFERENT ea_status (wrong sums),
//    surfacing the failure rather than coincidentally matching.
// 2. PPU passes EA1 via arg0, EA2 via arg1.
// 3. SPU fills LS source regions with R8.2-shared patterns:
//      LS[0x10000..0x10080] = i & 0xFF (counting, sum=0x1FC0)
//      LS[0x10080..0x100C0] = 0x42      (constant, sum=0x1080)
// 4. SPU builds 2-element list_element[] in LS:
//      list[0] = { sb=0, pad=0, ts=128, ea=EA1 }
//      list[1] = { sb=0, pad=0, ts= 64, ea=EA2 }
// 5. SPU dispatches MFC PUTL:
//      ch16 LSA      = 0x10000          (source base in LS)
//      ch17 EAH      = 0
//      ch18 EAL      = LS offset of list (descriptor pointer —
//                                          Cell BE PUTL puts the
//                                          descriptor in LS, same
//                                          convention as GETL)
//      ch19 Size     = 16               (= 2 * sizeof(list_element))
//      ch20 TagID    = 3
//      ch21 Cmd      = 0x24 PUTL
// 6. RPCS3 reads the descriptor list from LS, walks elements,
//    copies each `ts` bytes FROM LS at cumulative offset TO
//    EA = item.ea:
//      element 0: LS[0x10000..0x10080] → ea_dst1
//      element 1: LS[0x10080..0x100C0] → ea_dst2
// 7. SPU waits via ch22/ch23/ch24 (mask=0x08, ALL).
// 8. SPU writes spu_sentinel = 0xC0FFEEBA to OUT_MBOX, halts
//    with stop 0x101.
// 9. PPU joins; lv2 reads OUT_MBOX as group-exit status, then
//    sums both EA buffers:
//      sum_ea1 = sum(ea_dst1[0..128]) = 0x1FC0
//      sum_ea2 = sum(ea_dst2[0..64])  = 0x1080
//      combined = (sum_ea1 << 16) | sum_ea2 = 0x1FC01080
//      ea_status = combined ^ 0xBEEFCAFE     = 0xA12FDA7E
//
// Predicted canonical TTY (verify on real RPCS3):
//   `[dma_putl_v1] OK cause=0x1 spu=0xc0ffeeba ea_status=0xa12fda7e`
//
// Failure mode catalogue:
//   - List dispatch dropped (EA buffers stay 0xAA):
//       sum_ea1=0x5500 (0xAA*128), sum_ea2=0x2A80 (0xAA*64),
//       ea_status=0xEBBFE43E (distinct from canonical 0xA12FDA7E).
//   - Element 0 dropped: ea1 stays 0xAA, ea2 gets 0x42; distinct
//     wrong sums.
//   - Element 1 dropped: inverse.
//   - Descriptor format wrong / EAs swapped: distinctive sums.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_putl_bin[];
extern const u32 spu_dma_putl_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

static u8 ea_dst1[128] __attribute__((aligned(128)));
static u8 ea_dst2[64]  __attribute__((aligned(128)));

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    // Pre-PUTL sentinel: every byte 0xAA. A failed PUTL leaves
    // these intact, surfacing as a distinct (wrong) ea_status.
    for (u32 i = 0; i < 128; i++) {
        ea_dst1[i] = 0xAA;
    }
    for (u32 i = 0; i < 64; i++) {
        ea_dst2[i] = 0xAA;
    }

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[dma_putl_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_putl_bin, 0);
    if (ret) {
        printf("[dma_putl_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_putl_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_putl_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);
    thread_args.arg0 = (u64)(uintptr_t)ea_dst1;
    thread_args.arg1 = (u64)(uintptr_t)ea_dst2;

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[dma_putl_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_putl_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, spu_sentinel;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &spu_sentinel);
    if (ret) {
        printf("[dma_putl_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Sum both EA destination buffers — they MUST have been
    // overwritten by the PUTL with the SPU's LS source content
    // (counting pattern + constant 0x42).
    u32 sum_ea1 = 0;
    for (u32 i = 0; i < 128; i++) {
        sum_ea1 += (u32)ea_dst1[i];
    }
    u32 sum_ea2 = 0;
    for (u32 i = 0; i < 64; i++) {
        sum_ea2 += (u32)ea_dst2[i];
    }
    u32 combined = (sum_ea1 << 16) | (sum_ea2 & 0xFFFFu);
    u32 ea_status = combined ^ 0xBEEFCAFEu;

    // Predicted (verify by capture):
    //   cause=0x1
    //   spu=0xc0ffeeba (sentinel from SPU OUT_MBOX)
    //   ea_status=0xa12fda7e (=0x1FC01080 ^ 0xBEEFCAFE)
    printf("[dma_putl_v1] OK cause=0x%x spu=0x%x ea_status=0x%x\n",
           (unsigned)cause, (unsigned)spu_sentinel, (unsigned)ea_status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
