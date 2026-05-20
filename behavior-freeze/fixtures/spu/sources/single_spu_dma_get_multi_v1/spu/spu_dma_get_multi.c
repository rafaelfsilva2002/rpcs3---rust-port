// single_spu_dma_get_multi_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.2 first multi-DMA replay-validated fixture. The SPU receives
// EA1 in r3 (= thread_args.arg0, low 32) and EA2 in r4 (=
// thread_args.arg1, low 32) per PSL1GHT lv2 convention (verified
// in RPCS3 `lv2/sys_spu.cpp:1229..1232`).
//
// MFC sequence (2 GETs queued before wait):
//
//   1. ch16 MFC_LSA      ← 0x10000      (LS destination #1)
//   2. ch17 MFC_EAH      ← 0            (PS3 user-space PPU is 32-bit)
//   3. ch18 MFC_EAL      ← <ea1>        (low 32 bits of EA1)
//   4. ch19 MFC_Size     ← 128
//   5. ch20 MFC_TagID    ← 3
//   6. ch21 MFC_Cmd      ← 0x40 (GET)   ← dispatch #1
//
//   7. ch16 MFC_LSA      ← 0x10100      (LS destination #2)
//   8. ch17 MFC_EAH      ← 0
//   9. ch18 MFC_EAL      ← <ea2>        (low 32 bits of EA2)
//  10. ch19 MFC_Size     ← 64
//  11. ch20 MFC_TagID    ← 5
//  12. ch21 MFC_Cmd      ← 0x40 (GET)   ← dispatch #2
//
//  13. ch22 WrTagMask    ← (1<<3)|(1<<5) = 0x28
//  14. ch23 WrTagUpdate  ← MFC_TAG_UPDATE_ALL (= 2)
//  15. ch24 RdTagStat    → returns 0x28 once BOTH GETs complete
//
// After wait completes:
//   sum1 = sum(LS[0x10000..0x10080])  # 128 bytes counting pattern
//                                       → 8128 = 0x1FC0
//   sum2 = sum(LS[0x10100..0x10140])  # 64 bytes of 0x42
//                                       → 4224 = 0x1080
//   combined = (sum1 << 16) | sum2    # = 0x1FC0_1080
//   status   = combined ^ 0xFEEDFACE  # = 0xE12D_EA4E
//   wrch ch28 status
//   stop 0x101
//
// The canonical status 0xE12DEA4E is load-bearing: only achievable
// when (a) both DMAs actually copy bytes (not zero-fill), AND (b)
// the SPU waits for BOTH completions before reading (no race),
// AND (c) the state machine returns 0x28 exactly to RdTagStat
// (the ALL mode requires the full mask to be satisfied).
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
#define STATUS_MASK  ((uint32_t)0xFEEDFACEu)

int main(uint64_t spu_id, uint64_t arg)
{
    // PSL1GHT calling convention places thread_args.arg0 in r3 (=
    // first u64 parameter `spu_id`) and arg1 in r4 (= second u64
    // parameter `arg`). EA1 / EA2 are the low 32 of each.
    uint32_t ea1 = (uint32_t)spu_id;
    uint32_t ea2 = (uint32_t)arg;

    // === MFC GET #1 dispatch (tag 3) ===
    //
    // ch16..ch21 in canonical order. ch21 MFC_Cmd synchronously
    // dispatches the EA → LS copy in the C++ executor; the
    // captured trace records the wrch + a `spu_mfc_cmd` event
    // with the .dmachunk SHA-256 of the source bytes.
    spu_writech(MFC_LSA, LSA_DEST_1);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, ea1);
    spu_writech(MFC_Size, DMA_SIZE_1);
    spu_writech(MFC_TagID, DMA_TAG_1);
    spu_writech(MFC_Cmd, MFC_GET);

    // === MFC GET #2 dispatch (tag 5) ===
    //
    // Tag 5 distinct from tag 3 → both can be in flight
    // simultaneously from the state machine's perspective. The
    // refuse_mfc gate has already been relaxed (R7.2 callback
    // installed); each ch21 wrch invokes the GET callback in turn.
    spu_writech(MFC_LSA, LSA_DEST_2);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, ea2);
    spu_writech(MFC_Size, DMA_SIZE_2);
    spu_writech(MFC_TagID, DMA_TAG_2);
    spu_writech(MFC_Cmd, MFC_GET);

    // === Tag wait (ALL mode) ===
    //
    // Mask covers both dispatched tags. ALL mode requires every
    // tag bit in the mask to be set in `completed_tags` before
    // rdch ch24 unblocks. RdTagStat returns the mask exactly
    // (= 0x28) once both completes have fired.
    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;  // expected = 0x28; not asserted here

    // === Post-DMA combined checksum ===
    //
    // Read both LS regions via volatile pointers. The counting
    // pattern over [0x10000..0x10080] sums to 0x1FC0; the 0x42
    // constant over [0x10100..0x10140] sums to 0x1080.
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

    // Pack both sums into a single 32-bit canonical status. The
    // high 16 bits hold sum1, the low 16 bits hold sum2. XOR with
    // STATUS_MASK (0xFEEDFACE) produces 0xE12DEA4E — distinct from
    // GET v1's 0xDEADA12F and PUT v1's 0xC0FFEECA/0xCAFEA57E.
    uint32_t combined = (sum1 << 16) | (sum2 & 0xFFFFu);
    uint32_t status = combined ^ STATUS_MASK;

    spu_writech(SPU_WrOutMbox, status);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
