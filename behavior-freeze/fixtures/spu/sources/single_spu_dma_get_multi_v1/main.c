// single_spu_dma_get_multi_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.2 — first multi-DMA replay-validated fixture. The SPU dispatches
// TWO MFC GETs (distinct tags, distinct EAs, distinct sizes, distinct
// LSAs), then waits via WrTagMask = (1<<3)|(1<<5) + WrTagUpdate = ALL
// + RdTagStat. Both DMAs MUST complete before the rdch ch24 unblocks.
//
// Why this matters:
//
// All prior DMA-bound oracles (R6.7 A.5 GET v1 + R8.1 PUT v1) dispatch
// exactly ONE DMA. This is the first fixture exercising:
//   - Multiple in-flight tags (the state machine's in_flight set
//     transitions through size 2 instead of just 0..1).
//   - WrTagUpdate=ALL with multi-bit mask (vs single-bit in the prior
//     fixtures, which still went through the ALL mode but with a
//     degenerate mask). RdTagStat MUST exactly equal the mask only
//     after BOTH tags' mfc_dma_complete events have fired.
//   - The captured trace contains 2 spu_mfc_cmd events + 2
//     mfc_dma_complete events + 2 distinct .dmachunk side-files.
//
// Behaviour (deterministic):
//   1. PPU allocates two distinct EA buffers (ea_buf1 size 128 +
//      ea_buf2 size 64), both 128-byte aligned.
//   2. PPU fills ea_buf1 with counting pattern `i & 0xFF` for
//      i in 0..128 (sum1 = 8128 = 0x1FC0). Same content as GET v1
//      → its .dmachunk SHA-256 deduplicates with the existing pool
//      entry (471fb943…).
//   3. PPU fills ea_buf2 with constant pattern 0x42 for 64 bytes
//      (sum2 = 64 * 0x42 = 4224 = 0x1080). New content → fresh
//      content-addressed SHA in the canonical .dmachunk pool.
//   4. PPU passes EA1 via thread arg[0] (→ SPU r3) and EA2 via
//      thread arg[1] (→ SPU r4); sysSpuThreadGroupStart.
//   5. SPU dispatches GET #1 (tag=3, EA1 → LS@0x10000, size 128)
//      and GET #2 (tag=5, EA2 → LS@0x10100, size 64) BACK TO BACK
//      before any tag wait — both in flight simultaneously from
//      the state machine's perspective.
//   6. SPU writes WrTagMask = 0x28 and WrTagUpdate = ALL, then
//      blocks on RdTagStat. The rdch returns 0x28 once both
//      tag completions have fired.
//   7. SPU computes:
//        sum1 = sum(LS[0x10000..0x10080])  # = 0x1FC0
//        sum2 = sum(LS[0x10100..0x10140])  # = 0x1080
//        combined = (sum1 << 16) | sum2    # = 0x1FC0_1080
//        status   = combined ^ 0xFEEDFACE  # = 0xE12D_EA4E
//   8. SPU writes status to OUT_MBOX, halts with stop 0x101.
//   9. PPU joins; lv2 reads OUT_MBOX as the group-exit status.
//
// Expected TTY: `[dma_get_multi_v1] OK cause=0x1 status=0xe12dea4e`
//
// Canonical computation (reference Python):
//
//   buf1 = [i & 0xFF for i in range(128)]
//   buf2 = [0x42 for _ in range(64)]
//   sum1 = sum(buf1)                       # = 8128 = 0x1FC0
//   sum2 = sum(buf2)                       # = 4224 = 0x1080
//   combined = (sum1 << 16) | sum2         # = 0x1FC01080
//   status   = combined ^ 0xFEEDFACE       # = 0xE12DEA4E
//
// Status 0xE12DEA4E is the load-bearing acceptance — only achievable
// when:
//   (a) both EA → LS DMAs actually copy the bytes (not silently
//       fake-filled / zero-filled).
//   (b) the SPU correctly waits for BOTH tags before reading LS
//       (otherwise it might race and see partial data → wrong sum).
//   (c) the state machine returns 0x28 to RdTagStat exactly once
//       both completions have fired (off by one → wrong status).
//
// A silent fake-DMA path (zero-fill both LS regions) would yield
// `status = 0 ^ 0xFEEDFACE = 0xFEEDFACE` (distinctively wrong).
// A "GET #2 dropped" path would yield `0x1FC0_0000 ^ 0xFEEDFACE =
// 0xE12D_FACE` (distinctively wrong but visually close to canonical
// in the leading bytes — useful for human debug).

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_get_multi_bin[];
extern const u32 spu_dma_get_multi_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

// EA buffer #1: 128 bytes, counting pattern, aligned to 128 bytes
// (MFC ABI requires 16-byte alignment for size >= 16).
static u8 ea_buf1[128] __attribute__((aligned(128)));

// EA buffer #2: 64 bytes, constant 0x42 pattern, aligned to 128 bytes
// for symmetry.
static u8 ea_buf2[64]  __attribute__((aligned(128)));

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    // Fill EA buffer #1 with the counting pattern `i & 0xFF`. Same
    // content as the GET v1 / PUT v1 fixture's payload → the
    // resulting .dmachunk SHA-256 (471fb943…) is already in the
    // canonical pool and need not be re-committed.
    for (u32 i = 0; i < 128; i++) {
        ea_buf1[i] = (u8)(i & 0xFF);
    }

    // Fill EA buffer #2 with the constant pattern 0x42. The choice
    // of 0x42 is arbitrary but distinct from the counting pattern;
    // it produces sum2 = 64 * 0x42 = 4224 = 0x1080. This is a
    // brand-new .dmachunk content (new SHA-256 → fresh pool entry).
    for (u32 i = 0; i < 64; i++) {
        ea_buf2[i] = 0x42;
    }

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[dma_get_multi_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_get_multi_bin, 0);
    if (ret) {
        printf("[dma_get_multi_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_get_multi_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_get_multi_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);
    // PSL1GHT lv2 writes thread_args.arg0 → SPU r3, arg1 → r4
    // (each as a u64). Stash EA1 in arg0 and EA2 in arg1 — the SPU
    // will recover them via the SPU calling convention (u64 args
    // placed lane 0 = high 32 / lane 1 = low 32 in r3 / r4).
    thread_args.arg0 = (u64)(uintptr_t)ea_buf1;
    thread_args.arg1 = (u64)(uintptr_t)ea_buf2;

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[dma_get_multi_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_get_multi_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[dma_get_multi_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Expected: cause=0x1 (GROUP_EXIT), status=0xE12DEA4E.
    printf("[dma_get_multi_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
