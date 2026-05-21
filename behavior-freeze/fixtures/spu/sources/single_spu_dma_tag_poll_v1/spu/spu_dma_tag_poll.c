// single_spu_dma_tag_poll_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.3b — repeated RdTagStat polling oracle (11th oracle).
// Two queued GETs (tag 3 + tag 5) + TWO ch24 reads with distinct
// masks in the same SPU session. Forces persistent
// `completed_tags` semantics: after both DMAs complete, the SPU
// reads ch24 twice with masks 0x08 and 0x20 respectively. Real
// Cell BE retains `completed_tags` across reads, returning the
// per-mask subset on each.
//
// MFC sequence:
//
//   1-6.  ch16..ch21 : GET #1 (tag 3, LSA 0x10000, EA1, size 128)
//   7-12. ch16..ch21 : GET #2 (tag 5, LSA 0x10100, EA2, size  64)
//
//   --- First ch24 read (mask = tag 3 only) ---
//   13.   ch22 WrTagMask    = 0x08
//   14.   ch23 WrTagUpdate  = MFC_TAG_UPDATE_ANY (= 1)
//   15.   ch24 RdTagStat    → 0x08 (tag 3 completed AND in mask)
//
//   --- Second ch24 read (mask = tag 5 only) ---
//   16.   ch22 WrTagMask    = 0x20
//   17.   ch23 WrTagUpdate  = MFC_TAG_UPDATE_ANY
//   18.   ch24 RdTagStat    → 0x20 (tag 5 completed AND in mask;
//                                    completed_tags retained from
//                                    before the first read)
//
// Status computation:
//
//   sum1     = sum(LS[LSA_DEST_1..+128])     = 0x1FC0
//   sum2     = sum(LS[LSA_DEST_2..+64])      = 0x1080
//   combined = (sum1 << 16) | sum2           = 0x1FC0_1080
//   packed   = (tag_stat_1 << 24) | (tag_stat_2 << 16)
//                                            = 0x0820_0000
//   status   = combined ^ packed ^ 0xCAFEBADC = 0xDD1E_AA5C
//
// Inlined exit.

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_DEST_1     ((uint32_t)0x10000)
#define LSA_DEST_2     ((uint32_t)0x10100)
#define DMA_SIZE_1     ((uint32_t)128)
#define DMA_SIZE_2     ((uint32_t)64)
#define DMA_TAG_1      ((uint32_t)3)
#define DMA_TAG_2      ((uint32_t)5)
#define TAG_MASK_1     ((uint32_t)(1u << 3))   // = 0x08
#define TAG_MASK_2     ((uint32_t)(1u << 5))   // = 0x20
#define MFC_GET        ((uint32_t)0x40)
#define STATUS_MASK    ((uint32_t)0xCAFEBADCu)

int main(uint64_t spu_id, uint64_t arg)
{
    uint32_t ea1 = (uint32_t)spu_id;
    uint32_t ea2 = (uint32_t)arg;

    // === MFC GET #1 dispatch (tag 3) ===
    spu_writech(MFC_LSA, LSA_DEST_1);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, ea1);
    spu_writech(MFC_Size, DMA_SIZE_1);
    spu_writech(MFC_TagID, DMA_TAG_1);
    spu_writech(MFC_Cmd, MFC_GET);

    // === MFC GET #2 dispatch (tag 5) ===
    spu_writech(MFC_LSA, LSA_DEST_2);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, ea2);
    spu_writech(MFC_Size, DMA_SIZE_2);
    spu_writech(MFC_TagID, DMA_TAG_2);
    spu_writech(MFC_Cmd, MFC_GET);

    // === First RdTagStat (mask = tag 3 only, ANY) ===
    //
    // Expects tag 3's completion bit (0x08) to be visible.
    spu_writech(MFC_WrTagMask, TAG_MASK_1);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ANY);
    uint32_t tag_stat_1 = spu_readch(MFC_RdTagStat);

    // === Second RdTagStat (mask = tag 5 only, ANY) ===
    //
    // Expects tag 5's completion bit (0x20) to be visible. The
    // persistent completed_tags register on real Cell BE retains
    // tag 5's bit despite the first read having consumed (or
    // observed) tag 3's bit. A drain-clear emulator implementation
    // stalls here because nothing has produced new tag-stat bits
    // since the first read.
    spu_writech(MFC_WrTagMask, TAG_MASK_2);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ANY);
    uint32_t tag_stat_2 = spu_readch(MFC_RdTagStat);

    // === Post-DMA combined checksum + both tag_stats ===
    volatile const uint8_t* ls_buf_1 =
        (volatile const uint8_t*)(uintptr_t)LSA_DEST_1;
    volatile const uint8_t* ls_buf_2 =
        (volatile const uint8_t*)(uintptr_t)LSA_DEST_2;

    uint32_t sum1 = 0;
    for (uint32_t i = 0; i < DMA_SIZE_1; i++) {
        sum1 += (uint32_t)ls_buf_1[i];
    }

    uint32_t sum2 = 0;
    for (uint32_t i = 0; i < DMA_SIZE_2; i++) {
        sum2 += (uint32_t)ls_buf_2[i];
    }

    uint32_t combined = (sum1 << 16) | (sum2 & 0xFFFFu);
    uint32_t packed = (tag_stat_1 << 24) | (tag_stat_2 << 16);
    uint32_t status = combined ^ packed ^ STATUS_MASK;

    spu_writech(SPU_WrOutMbox, status);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
