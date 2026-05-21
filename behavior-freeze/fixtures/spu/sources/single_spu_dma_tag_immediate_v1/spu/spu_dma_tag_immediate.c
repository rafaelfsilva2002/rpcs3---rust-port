// single_spu_dma_tag_immediate_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.3c — IMMEDIATE wait mode + clearing-semantics probe.
// Two queued GETs (tags 3 + 5) + TWO ch24 reads with
// `WrTagUpdate = IMMEDIATE` (= 0) and distinct masks (0x08
// then 0x28). The two reads, with the first being a strict
// subset of the second's mask, probe whether the read clears
// bits from `completed_tags`.
//
// MFC sequence:
//
//   1-6.  ch16..ch21 : GET #1 (tag 3, LSA 0x10000, EA1, size 128)
//   7-12. ch16..ch21 : GET #2 (tag 5, LSA 0x10100, EA2, size  64)
//
//   --- First ch24 read (mask = tag 3 only, IMMEDIATE) ---
//   13.   ch22 WrTagMask    = 0x08
//   14.   ch23 WrTagUpdate  = MFC_TAG_UPDATE_IMMEDIATE (= 0)
//   15.   ch24 RdTagStat    → ts1 (captured by trace writer)
//
//   --- Second ch24 read (mask = both tags, IMMEDIATE) ---
//   16.   ch22 WrTagMask    = 0x28
//   17.   ch23 WrTagUpdate  = MFC_TAG_UPDATE_IMMEDIATE (= 0)
//   18.   ch24 RdTagStat    → ts2 (captured by trace writer)
//
// Status computation:
//
//   sum1     = sum(LS[LSA_DEST_1..+128])  = 0x1FC0
//   sum2     = sum(LS[LSA_DEST_2..+64])   = 0x1080
//   combined = (sum1 << 16) | sum2        = 0x1FC0_1080
//   packed   = (ts1 << 24) | (ts2 << 16)
//   status   = combined ^ packed ^ 0xCAFE5A1E
//
// Real RPCS3 capture determines the canonical (ts1, ts2)
// pair. The replay/runtime must reproduce both values from
// the captured trace exactly.

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_DEST_1     ((uint32_t)0x10000)
#define LSA_DEST_2     ((uint32_t)0x10100)
#define DMA_SIZE_1     ((uint32_t)128)
#define DMA_SIZE_2     ((uint32_t)64)
#define DMA_TAG_1      ((uint32_t)3)
#define DMA_TAG_2      ((uint32_t)5)
#define TAG_MASK_1     ((uint32_t)(1u << 3))                  // = 0x08
#define TAG_MASK_FULL  ((uint32_t)((1u << 3) | (1u << 5)))    // = 0x28
#define MFC_GET        ((uint32_t)0x40)
#define STATUS_MASK    ((uint32_t)0xCAFE5A1Eu)

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

    // === First IMMEDIATE read (mask = tag 3) ===
    spu_writech(MFC_WrTagMask, TAG_MASK_1);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_IMMEDIATE);
    uint32_t ts1 = spu_readch(MFC_RdTagStat);

    // === Second IMMEDIATE read (mask = both tags) ===
    //
    // If the first read cleared the tag 3 bit from
    // `completed_tags`, this read returns 0x20. If
    // `completed_tags` is persistent across reads (Cell BE
    // canonical), this read returns 0x28.
    spu_writech(MFC_WrTagMask, TAG_MASK_FULL);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_IMMEDIATE);
    uint32_t ts2 = spu_readch(MFC_RdTagStat);

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
    uint32_t packed = (ts1 << 24) | (ts2 << 16);
    uint32_t status = combined ^ packed ^ STATUS_MASK;

    spu_writech(SPU_WrOutMbox, status);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
