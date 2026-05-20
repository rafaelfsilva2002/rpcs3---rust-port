// single_spu_dma_get_any_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.3a — first ANY-wait-mode replay-validated fixture (10th oracle).
// Mirrors R8.2 (two queued MFC GETs, distinct tags, distinct EAs,
// distinct sizes, distinct LSAs) but uses WrTagUpdate = ANY (= 1)
// instead of ALL (= 2). The SPU embeds the actual ch24 returned
// value into the canonical OUT_MBOX status — robust to whatever
// RPCS3 / the Rust state machine chooses to return.
//
// Why ANY needs the dynamic embed:
//
// ALL mode is deterministic: it returns the full mask exactly
// when every bit's tag has completed. ANY mode returns whatever
// subset of completed tags exists at the moment of the rdch.
// In RPCS3, DMAs are dispatched synchronously by the C++ executor
// (process_mfc_cmd runs the copy at wrch ch21 time), so by the
// time the SPU reaches ch24 BOTH tag completes have fired and
// ANY returns the full mask (0x28). On real hardware, only one
// tag might have completed when the SPU first reads ch24.
//
// To keep the oracle resilient against backend choices, the SPU
// reads the tag_stat value and EMBEDS IT into the canonical
// status:
//
//   combined = (sum1 << 16) | sum2
//   status   = combined ^ (tag_stat << 24) ^ 0xBEEFBEAD
//
// The (tag_stat << 24) shift places the returned mask into the
// high byte of a separate XOR factor. Any tag_stat value in
// {0x8, 0x20, 0x28} produces a different status, so the
// fixture detects both "DMA copied the bytes" AND "what the
// state machine returned for ch24".
//
// Behaviour (deterministic up to ch24's mask choice):
//   1. PPU fills ea_buf1 (128 B, counting pattern) → sum1 = 0x1FC0.
//   2. PPU fills ea_buf2 (64 B, constant 0x42)    → sum2 = 0x1080.
//   3. SPU dispatches GET #1 (tag 3) and GET #2 (tag 5) back-to-back.
//   4. SPU writes WrTagMask = 0x28, WrTagUpdate = ANY.
//   5. SPU reads RdTagStat → tag_stat (RPCS3 sync DMA: 0x28).
//   6. SPU computes status = (0x1FC0_1080) ^ (tag_stat << 24) ^
//      0xBEEFBEAD. For tag_stat=0x28: status = 0x892F_AE2D.
//   7. SPU writes status to OUT_MBOX, halts via stop 0x101.
//   8. PPU joins; lv2 reads OUT_MBOX as the group-exit status.
//
// Expected TTY (post-capture observation will fix the exact value):
//   `[dma_get_any_v1] OK cause=0x1 status=0x892f_ae2d`
//   (= 0x1FC0_1080 ^ 0x2800_0000 ^ 0xBEEF_BEAD, assuming RPCS3
//    returns the full mask. If a future capture observes a
//    different ch24 value, the status changes deterministically
//    and the canonical is the captured one.)
//
// Canonical computation (Python reference, RPCS3 sync DMA case):
//
//   buf1 = [i & 0xFF for i in range(128)]
//   buf2 = [0x42 for _ in range(64)]
//   sum1 = sum(buf1)                         # = 0x1FC0
//   sum2 = sum(buf2)                         # = 0x1080
//   tag_stat = 0x28                          # ANY in RPCS3 sync = full mask
//   combined = (sum1 << 16) | sum2           # = 0x1FC0_1080
//   status = combined ^ (tag_stat << 24) ^ 0xBEEFBEAD
//          = 0x1FC0_1080 ^ 0x2800_0000 ^ 0xBEEF_BEAD
//          = 0x892F_AE2D
//
// Failure mode catalogue:
//
//   - Silent fake-DMA path (both LS zero-fill, ch24 still 0x28):
//     status = 0 ^ 0x28000000 ^ 0xBEEFBEAD = 0x96EFBEAD
//   - Both DMAs OK but ch24 returns 0 (state machine bug, ANY
//     mode returning nothing): status XOR adds 0 in high byte,
//     yielding 0x1FC0_1080 ^ 0xBEEF_BEAD = 0xA12F_AEAD (distinctively
//     wrong; high byte is canonical sum-XOR but missing the
//     0x28 contribution).
//   - Only one GET completed (ANY returns 0x8 or 0x20):
//     status = combined ^ (0x08 << 24) ^ 0xBEEFBEAD = 0xA92F_AEAD
//             or         (0x20 << 24)              = 0x812F_AEAD
//     Each distinct, fixture catches both.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_get_any_bin[];
extern const u32 spu_dma_get_any_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

// EA buffer #1: 128 bytes, counting pattern, aligned to 128 bytes.
// Same content as R8.2 GET #1 → its .dmachunk SHA-256 deduplicates
// with the existing canonical pool entry (471fb943…).
static u8 ea_buf1[128] __attribute__((aligned(128)));

// EA buffer #2: 64 bytes, constant 0x42 pattern, aligned to 128 bytes.
// Same content as R8.2 GET #2 → its .dmachunk SHA-256 deduplicates
// with the c422e707… pool entry.
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
        printf("[dma_get_any_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_get_any_bin, 0);
    if (ret) {
        printf("[dma_get_any_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_get_any_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_get_any_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
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
        printf("[dma_get_any_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_get_any_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[dma_get_any_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Expected: cause=0x1 (GROUP_EXIT), status=<post-capture canonical>.
    // For RPCS3 sync DMA, status = 0x892FAE2D.
    printf("[dma_get_any_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
