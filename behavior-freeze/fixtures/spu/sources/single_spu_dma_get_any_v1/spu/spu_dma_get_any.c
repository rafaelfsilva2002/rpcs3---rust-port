// single_spu_dma_get_any_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.3a first ANY-wait-mode replay-validated fixture (10th oracle).
// Mirrors the R8.2 single_spu_dma_get_multi_v1 structure (two
// queued MFC GETs back-to-back, distinct tags, distinct EAs,
// distinct sizes, distinct LSAs) but uses `WrTagUpdate = ANY`
// (= MFC_TAG_UPDATE_ANY = 1) in place of ALL (= 2).
//
// Why ANY is interesting:
//
// ALL waits for EVERY bit in the mask before unblocking; the
// returned tag_stat is the mask exactly. ANY waits for AT LEAST
// ONE bit, returning the subset of completed-and-in-flight tags
// observable at that moment. In RPCS3, DMAs are dispatched
// synchronously by the C++ executor (process_mfc_cmd runs the
// EA→LS copy at wrch ch21 time), so by the time the SPU reaches
// ch24 BOTH tag completes have already fired and ANY returns
// the full mask (0x28). On real hardware this is racy; the
// oracle gate captures whatever RPCS3 actually returns and locks
// it as canonical.
//
// To make the fixture robust against the backend's tag_stat
// choice, the SPU embeds the actual ch24 returned value into
// the final status:
//
//   sum1 = sum(LS[LSA_DEST_1..LSA_DEST_1+128])  ; 0x1FC0
//   sum2 = sum(LS[LSA_DEST_2..LSA_DEST_2+64])   ; 0x1080
//   combined = (sum1 << 16) | sum2              ; 0x1FC0_1080
//   status   = combined ^ (tag_stat << 24) ^ 0xBEEFBEAD
//
// (tag_stat << 24) places the returned mask into the high byte
// of a separate XOR factor. Any value in {0x8, 0x20, 0x28}
// produces a distinct status — every backend choice round-trips
// through the oracle.
//
// MFC sequence (2 GETs queued before wait):
//
//   1-6.  ch16..ch21  : GET #1 (tag 3, LSA 0x10000, EA1, size 128)
//   7-12. ch16..ch21  : GET #2 (tag 5, LSA 0x10100, EA2, size  64)
//  13.    ch22 WrTagMask    = 0x28
//  14.    ch23 WrTagUpdate  = MFC_TAG_UPDATE_ANY (= 1)
//  15.    ch24 RdTagStat   → returns mask of completed-and-in-flight
//                            tags ∩ 0x28; in RPCS3 sync DMA: 0x28
//
// After the wait, the SPU reads both LS regions, computes the
// embedded status, writes OUT_MBOX, halts via stop 0x101.
//
// Inlined exit (avoid pulling libsputhread).

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_DEST_1   ((uint32_t)0x10000)
#define LSA_DEST_2   ((uint32_t)0x10100)
#define DMA_SIZE_1   ((uint32_t)128)
#define DMA_SIZE_2   ((uint32_t)64)
#define DMA_TAG_1    ((uint32_t)3)
#define DMA_TAG_2    ((uint32_t)5)
#define DMA_TAG_MASK ((uint32_t)((1u << 3) | (1u << 5)))   // = 0x28
#define MFC_GET      ((uint32_t)0x40)
#define STATUS_MASK  ((uint32_t)0xBEEFBEADu)

int main(uint64_t spu_id, uint64_t arg)
{
    // PSL1GHT convention: arg0 → r3, arg1 → r4 (each u64).
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

    // === Tag wait (ANY mode) ===
    //
    // Mask covers both dispatched tags. ANY mode requires at
    // least one tag bit in the mask to be set in `completed_tags`
    // before rdch ch24 unblocks. RdTagStat returns the mask of
    // completed-and-in-flight tags ∩ wr_tag_mask at that moment.
    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ANY);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);

    // === Post-DMA combined checksum + tag_stat embed ===
    //
    // Even if ANY returned a partial mask, we still read BOTH LS
    // regions — they were both populated by the synchronous C++
    // executor at wrch ch21 time. The sum computations always
    // yield 0x1FC0 + 0x1080. What CHANGES with backend choice is
    // the embedded tag_stat value (high byte XOR contribution).
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
    uint32_t status = combined ^ (tag_stat << 24) ^ STATUS_MASK;

    spu_writech(SPU_WrOutMbox, status);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
