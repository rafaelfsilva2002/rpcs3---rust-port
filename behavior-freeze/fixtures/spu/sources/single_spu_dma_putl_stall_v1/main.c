// single_spu_dma_putl_stall_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.5e E.3 — first MFC PUTL stall-and-notify oracle source
// (20th oracle target). Symmetric inverse of R8.5d D.3 GETL
// stall: the PPU allocates THREE destination EA buffers
// pre-initialized to a 0xAA sentinel, the SPU fills its LS
// with deterministic source content, builds a 3-element list
// descriptor with element 1 sb=0x80, dispatches MFC PUTL
// (cmd=0x24), observes the stall via ch25, acknowledges via
// ch26, and waits via ch22/ch23/ch24 for full-list tag-stat
// completion. The PPU joins, sums all three EA buffers, and
// prints the canonical TTY.
//
// Behaviour (deterministic):
//
// 1. PPU allocates THREE EA destination buffers pre-set to
//    0xAA so a dropped/silent PUTL produces a distinct
//    (wrong) ea_status rather than coincidentally matching:
//      ea_dst1 (128 B)
//      ea_dst2 ( 64 B)
//      ea_dst3 ( 96 B)
// 2. PPU packs:
//      thread_args.arg0 = ((u64)EA1 << 32) | EA2
//      thread_args.arg1 = ((u64)EA3 << 32)
// 3. SPU unpacks (PSL1GHT convention: arg0 → r3, arg1 → r4):
//      ea1 = r3 >> 32
//      ea2 = r3 & 0xFFFFFFFF
//      ea3 = r4 >> 32
// 4. SPU fills LS source regions with R8.2-shared patterns
//    (perfect dedup with canonical .dmachunk pool):
//      LS[0x10000..0x10080] = i & 0xFF (counting, sum=0x1FC0)
//      LS[0x10080..0x100C0] = 0x42      (constant, sum=0x1080)
//      LS[0x100C0..0x10120] = 0x11      (constant, sum=0x0660)
// 5. SPU builds 3-element list_element[] in LS:
//      list[0] = { sb=0,    pad=0, ts=128, ea=EA1 }
//      list[1] = { sb=0x80, pad=0, ts= 64, ea=EA2 }   // STALL
//      list[2] = { sb=0,    pad=0, ts= 96, ea=EA3 }
// 6. SPU dispatches MFC PUTL:
//      ch16 LSA      = 0x10000          (source base in LS)
//      ch17 EAH      = 0
//      ch18 EAL      = LS offset of list (descriptor pointer)
//      ch19 Size     = 24               (= 3 * sizeof(list_element))
//      ch20 TagID    = 3
//      ch21 Cmd      = 0x24 PUTL
// 7. RPCS3 walks the descriptors:
//      element 0: LS[0x10000..0x10080] → ea_dst1
//      element 1: LS[0x10080..0x100C0] → ea_dst2
//                 ← transfer COMPLETES (Cell BE Sec. 12.5)
//                   BEFORE the stall bit is raised; ea_dst2
//                   contains 0x42 bytes after this point.
// 8. SPU reads ch25 → stall_mask = 0x08.
// 9. SPU writes ch26 ← 3 (tag id, NOT bitmask).
// 10. RPCS3 resumes:
//      element 2: LS[0x100C0..0x10120] → ea_dst3
// 11. MFC raises the tag-stat bit normally; SPU waits via
//     ch22/ch23/ch24 (mask=0x08, ALL) → tag_stat = 0x08.
// 12. SPU writes spu_sentinel = 0xC0FFEEC3 to OUT_MBOX, halts
//     with stop 0x101.
// 13. PPU joins; lv2 reads OUT_MBOX as group-exit status,
//     then sums all three EA buffers:
//      sum_ea1 = sum(ea_dst1[0..128]) = 0x1FC0 (= 8128 dec)
//      sum_ea2 = sum(ea_dst2[0..64])  = 0x1080 (= 4224 dec)
//      sum_ea3 = sum(ea_dst3[0..96])  = 0x0660 (= 1632 dec)
//      combined = (sum_ea1 << 16) | ((sum_ea2 + sum_ea3) & 0xFFFF)
//               = (0x1FC0 << 16) | 0x16E0
//               = 0x1FC0_16E0
//      ea_status = combined ^ 0xBEEFCAFE = 0xA12F_DC1E
//
// Predicted canonical TTY (verify on real RPCS3):
//   `[putl_stall_v1] OK cause=0x1 spu=0xc0ffeec3 ea_status=0xa12fdc1e`
//
// Failure mode catalogue:
//   - Stall handshake dropped (RPCS3 ignores sb&0x80 and just
//     completes the list): R8.5b writer canary fires; capture
//     refuses to emit the JSONL events.
//   - Element 1 NOT transferred before stall (Cell BE Sec. 12.5
//     violation): ea_dst2 stays 0xAA; sum_ea2 = 0xA80 instead
//     of 0x1080; distinct (wrong) ea_status.
//   - Element 2 NOT transferred after ack: ea_dst3 stays 0xAA;
//     sum_ea3 = 0x3F90 (= 0xAA * 96 = 16272) instead of
//     0x660; distinct (wrong) ea_status.
//   - All elements dropped: ea_status = (0x5500 << 16) | 0x65140 & 0xFFFF
//     ^ 0xBEEFCAFE (all 0xAA sums); also distinct.
//
// Canonical computation (Python reference):
//   buf1 = [i & 0xFF for i in range(128)]
//   buf2 = [0x42 for _ in range(64)]
//   buf3 = [0x11 for _ in range(96)]
//   sum1 = sum(buf1)                       # 0x1FC0
//   sum2 = sum(buf2)                       # 0x1080
//   sum3 = sum(buf3)                       # 0x0660
//   combined = (sum1 << 16) | ((sum2 + sum3) & 0xFFFF)
//   ea_status = combined ^ 0xBEEFCAFE      # 0xA12FDC1E

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_putl_stall_bin[];
extern const u32 spu_dma_putl_stall_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

static u8 ea_dst1[128] __attribute__((aligned(128)));
static u8 ea_dst2[64]  __attribute__((aligned(128)));
static u8 ea_dst3[96]  __attribute__((aligned(128)));

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    // Pre-PUTL sentinel: every byte 0xAA. A failed/dropped PUTL
    // element leaves these intact, surfacing as a distinct
    // (wrong) ea_status rather than coincidentally matching.
    for (u32 i = 0; i < 128; i++) {
        ea_dst1[i] = 0xAA;
    }
    for (u32 i = 0; i < 64; i++) {
        ea_dst2[i] = 0xAA;
    }
    for (u32 i = 0; i < 96; i++) {
        ea_dst3[i] = 0xAA;
    }

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[putl_stall_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_putl_stall_bin, 0);
    if (ret) {
        printf("[putl_stall_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "putl_stall_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[putl_stall_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    // Pack: arg0 high = EA1, arg0 low = EA2; arg1 high = EA3.
    // (Same packing as getl_stall_v1; SPU code unpacks the same way.)
    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);
    thread_args.arg0 = (((u64)(uintptr_t)ea_dst1) << 32)
                     | ((u64)(uintptr_t)ea_dst2);
    thread_args.arg1 = ((u64)(uintptr_t)ea_dst3) << 32;

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[putl_stall_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[putl_stall_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, spu_sentinel;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &spu_sentinel);
    if (ret) {
        printf("[putl_stall_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Sum all three EA destination buffers — they MUST have
    // been overwritten by the PUTL with the SPU's LS source
    // content (counting pattern + 0x42 + 0x11).
    u32 sum_ea1 = 0;
    for (u32 i = 0; i < 128; i++) {
        sum_ea1 += (u32)ea_dst1[i];
    }
    u32 sum_ea2 = 0;
    for (u32 i = 0; i < 64; i++) {
        sum_ea2 += (u32)ea_dst2[i];
    }
    u32 sum_ea3 = 0;
    for (u32 i = 0; i < 96; i++) {
        sum_ea3 += (u32)ea_dst3[i];
    }
    u32 combined = (sum_ea1 << 16) | ((sum_ea2 + sum_ea3) & 0xFFFFu);
    u32 ea_status = combined ^ 0xBEEFCAFEu;

    // Predicted (verify by capture):
    //   cause=0x1
    //   spu=0xC0FFEEC3 (sentinel from SPU OUT_MBOX)
    //   ea_status=0xA12FDC1E (=0x1FC016E0 ^ 0xBEEFCAFE)
    printf("[putl_stall_v1] OK cause=0x%x spu=0x%x ea_status=0x%x\n",
           (unsigned)cause, (unsigned)spu_sentinel, (unsigned)ea_status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
