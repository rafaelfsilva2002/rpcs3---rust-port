// single_spu_dma_tag_poll_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.3b — first repeated-RdTagStat polling oracle (11th oracle).
// Mirrors R8.2 / R8.3a multi-DMA setup (two queued GETs, distinct
// tags, distinct EAs, distinct sizes, distinct LSAs) but the SPU
// performs TWO ch24 reads in the same session with different
// `WrTagMask` registers, embedding BOTH returned values into the
// canonical OUT_MBOX status.
//
// Why repeated polling matters:
//
// Cell BE exposes `completed_tags` as a persistent register that
// retains state across `rdch ch24` reads. The hardware semantic is
// "ch24 returns `completed_tags & wr_tag_mask` without clearing
// `completed_tags`". Real-world SPU programs poll multiple tag
// subsets across the same wait window (e.g. an SPURS-like dispatch
// loop reading task-group A's tags, then task-group B's, both
// without re-dispatching).
//
// The R8.3a engine fix (drain-OR-AND) handles ONE ch24 read per
// session correctly: drains the queue, ORs into an aggregate,
// ANDs with the current mask. But the drain empties the queue,
// so a SECOND ch24 read in the same session stalls — the queue
// is empty, no producer has pushed anything new. That's the gap
// R8.3b targets: it forces the persistent `completed_tags: u32`
// refactor by failing the second read with the current drain-clear
// implementation.
//
// Expected behaviour:
//   1. PPU fills ea_buf1 (128 B, counting pattern) → sum1 = 0x1FC0.
//   2. PPU fills ea_buf2 (64 B, constant 0x42)    → sum2 = 0x1080.
//   3. SPU dispatches GET #1 (tag 3) and GET #2 (tag 5) back-to-back.
//   4. SPU writes WrTagMask = 0x08 (tag 3 only), WrTagUpdate = ANY.
//   5. SPU reads RdTagStat → tag_stat_1 = 0x08 (tag 3 completed).
//   6. SPU writes WrTagMask = 0x20 (tag 5 only), WrTagUpdate = ANY.
//   7. SPU reads RdTagStat → tag_stat_2 = 0x20 (tag 5 completed —
//      retained in completed_tags despite the first read).
//   8. SPU computes:
//        combined = (sum1 << 16) | sum2          # = 0x1FC0_1080
//        packed   = (tag_stat_1 << 24) | (tag_stat_2 << 16)
//                                                # = 0x0820_0000
//        status   = combined ^ packed ^ 0xCAFEBADC
//                                                # = 0xDD1E_AA5C
//   9. SPU writes status to OUT_MBOX, halts via stop 0x101.
//  10. PPU joins; lv2 reads OUT_MBOX as the group-exit status.
//
// Predicted canonical TTY (real C++ RPCS3, persistent completed_tags):
//   `[dma_tag_poll_v1] OK cause=0x1 status=0xdd1eaa5c`
//
// Predicted failure modes BEFORE the persistent-state refactor:
//   - Bridge OFF (pure C++): OK, canonical 0xDD1EAA5C.
//   - Bridge ON (Rust drain-clear semantic): second rdch stalls;
//     bridge falls back to C++ at the stall outcome; C++ then
//     resumes and produces canonical. TTY = canonical but the
//     Rust bridge log shows a stall fallback (not full delegation).
//   - Replay (current drain-clear pre-replay queue): first read
//     drains queue [0x08, 0x20], returns 0x28 & 0x08 = 0x08; second
//     read stalls with empty queue → replay test fails.
//
// Post-refactor expected:
//   - Bridge ON: full delegation, no fallback.
//   - Replay: both reads return captured values, byte-identical.
//
// Canonical computation (Python reference):
//
//   sum1 = sum(i & 0xFF for i in range(128))     # = 0x1FC0
//   sum2 = 64 * 0x42                              # = 0x1080
//   tag_stat_1 = 0x08
//   tag_stat_2 = 0x20
//   combined = (sum1 << 16) | sum2                # = 0x1FC01080
//   packed   = (tag_stat_1 << 24) | (tag_stat_2 << 16)
//                                                 # = 0x08200000
//   status   = combined ^ packed ^ 0xCAFEBADC     # = 0xDD1EAA5C
//
// Status 0xDD1EAA5C is the load-bearing acceptance — only
// achievable when (a) both DMAs copied real bytes, AND (b) BOTH
// ch24 reads observed the captured tag bits via persistent
// completed_tags semantics. Any divergence — wrong DMA bytes,
// stalled second read, masked-wrong tag bit — produces a
// distinctively wrong canonical.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_tag_poll_bin[];
extern const u32 spu_dma_tag_poll_bin_size;

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
        printf("[dma_tag_poll_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_tag_poll_bin, 0);
    if (ret) {
        printf("[dma_tag_poll_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_tag_poll_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_tag_poll_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
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
        printf("[dma_tag_poll_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_tag_poll_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[dma_tag_poll_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Expected: cause=0x1 (GROUP_EXIT), status=0xDD1EAA5C (predicted
    // canonical for real Cell BE / C++ persistent completed_tags).
    printf("[dma_tag_poll_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
