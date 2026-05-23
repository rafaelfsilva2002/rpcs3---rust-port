// single_spu_dma_getl_stall_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.5d D.3 — first MFC GETL stall-and-notify oracle source
// (19th oracle target). The PPU prepares THREE deterministic
// EA buffers, packs all three addresses into the two 64-bit
// thread args (ea1 + ea2 in arg0, ea3 in arg1 high), and joins.
// The SPU constructs a 3-element list descriptor array in LS
// with element 1's sb bit set to 0x80, dispatches MFC GETL
// (cmd=0x44), reads the stall mask via ch25, acknowledges via
// ch26, then waits for full-list tag-stat completion via ch24
// and computes the canonical OUT_MBOX status from all three
// copied regions. Halts via stop 0x101.
//
// Behaviour (deterministic):
//
// 1. PPU allocates ea_buf1 (128 B counting pattern 0..127),
//    ea_buf2 (64 B constant 0x42), ea_buf3 (96 B constant 0x11).
//    Patterns 1 + 2 match the existing GETL fixture so their
//    `.dmachunk` side-files deduplicate with the canonical pool.
//    Pattern 3 is new (96 B of 0x11) and creates one new
//    side-file (sha computed by the writer at capture time).
// 2. PPU packs:
//      thread_args.arg0 = ((u64)EA1 << 32) | EA2
//      thread_args.arg1 = ((u64)EA3 << 32)
// 3. SPU unpacks (PSL1GHT convention: arg0 → r3, arg1 → r4):
//      ea1 = r3 >> 32
//      ea2 = r3 & 0xFFFFFFFF
//      ea3 = r4 >> 32
// 4. SPU builds a 3-element list_element[] in LS:
//      list[0] = { sb=0,    pad=0, ts=128, ea=EA1 }
//      list[1] = { sb=0x80, pad=0, ts= 64, ea=EA2 }  // STALL
//      list[2] = { sb=0,    pad=0, ts= 96, ea=EA3 }
// 5. SPU dispatches MFC GETL:
//      ch16 LSA      = 0x10000          (destination base)
//      ch17 EAH      = 0
//      ch18 EAL      = LS offset of list (NOT data EA)
//      ch19 Size     = 24               (= 3 * sizeof(list_element))
//      ch20 TagID    = 3
//      ch21 Cmd      = 0x44 GETL
// 6. RPCS3 walks the descriptors:
//      element 0: LS[0x10000..0x10080] ← ea_buf1 (counting)
//      element 1: LS[0x10080..0x100C0] ← ea_buf2 (0x42)
//                 ← transfer COMPLETES (Cell BE Sec. 12.5),
//                   then sb&0x80 raises stall bit 1<<3=0x08
//                   on the tag's MFC_RdListStallStat channel.
// 7. SPU reads ch25 → stall_mask = 0x08 (mask of stalled tags).
// 8. SPU writes ch26 ← 3 (tag id, NOT bitmask).
// 9. RPCS3 clears the stall bit, resumes the descriptor walk:
//      element 2: LS[0x100C0..0x10120] ← ea_buf3 (0x11)
// 10. SPU waits via ch22/ch23/ch24 (mask=0x08, ALL) → tag_stat
//     = 0x08.
// 11. SPU sums all three LS regions:
//      sum1 = sum(LS[0x10000..0x10080]) = 0x1FC0  (= 8128 dec)
//      sum2 = sum(LS[0x10080..0x100C0]) = 0x1080  (= 4224 dec)
//      sum3 = sum(LS[0x100C0..0x10120]) = 0x0660  (= 1632 dec)
//      combined = (sum1 << 16) | ((sum2 + sum3) & 0xFFFF)
//               = (0x1FC0 << 16) | 0x16E0
//               = 0x1FC0_16E0
//      status   = combined ^ 0xC0DEFADA = 0xDF1E_EC3A
// 12. SPU writes status to OUT_MBOX, halts with stop 0x101.
// 13. PPU joins; lv2 reads OUT_MBOX as group-exit status.
//
// Predicted canonical TTY (verify on real RPCS3):
//   `[getl_stall_v1] OK cause=0x1 status=0xdf1eec3a`
//
// Failure mode catalogue:
//   - Stall handshake dropped (RPCS3 ignores sb&0x80 and just
//     completes the list): canary fires per R8.5b — capture
//     refuses to emit the JSONL events.
//   - Element 1 NOT transferred before stall (Cell BE Sec. 12.5
//     violation): sum2 = 0; status diverges to
//     (0x1FC0 << 16) | (0x660) ^ 0xC0DEFADA = 0xDF1E_FCBA.
//   - Element 2 NOT transferred after ack: sum3 = 0; status
//     diverges to (0x1FC0 << 16) | (0x1080) ^ 0xC0DEFADA
//     = 0xDF1E_EA5A (= GETL_v1 status — distinct fingerprint).
//   - Stall mask read returns wrong tag: ch25 would return
//     a different value; not asserted on SPU side (smoke
//     fingerprint via OUT_MBOX is sufficient).
//
// Canonical computation (Python reference):
//   buf1 = [i & 0xFF for i in range(128)]
//   buf2 = [0x42 for _ in range(64)]
//   buf3 = [0x11 for _ in range(96)]
//   sum1 = sum(buf1)                       # 0x1FC0
//   sum2 = sum(buf2)                       # 0x1080
//   sum3 = sum(buf3)                       # 0x0660
//   combined = (sum1 << 16) | ((sum2 + sum3) & 0xFFFF)
//   status   = combined ^ 0xC0DEFADA       # 0xDF1EEC3A

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_getl_stall_bin[];
extern const u32 spu_dma_getl_stall_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

static u8 ea_buf1[128] __attribute__((aligned(128)));
static u8 ea_buf2[64]  __attribute__((aligned(128)));
static u8 ea_buf3[96]  __attribute__((aligned(128)));

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
    for (u32 i = 0; i < 96; i++) {
        ea_buf3[i] = 0x11;
    }

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[getl_stall_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_getl_stall_bin, 0);
    if (ret) {
        printf("[getl_stall_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "getl_stall_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[getl_stall_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    // Pack: arg0 high = EA1, arg0 low = EA2; arg1 high = EA3.
    // EAs fit in 32 bits (RPCS3 main RAM mapping); SPU code
    // unpacks the same way.
    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);
    thread_args.arg0 = (((u64)(uintptr_t)ea_buf1) << 32)
                     | ((u64)(uintptr_t)ea_buf2);
    thread_args.arg1 = ((u64)(uintptr_t)ea_buf3) << 32;

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[getl_stall_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[getl_stall_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[getl_stall_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Predicted: cause=0x1, status=0xDF1EEC3A (verify by capture).
    printf("[getl_stall_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
