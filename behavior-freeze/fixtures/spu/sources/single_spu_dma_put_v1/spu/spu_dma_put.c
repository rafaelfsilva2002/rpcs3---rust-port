// single_spu_dma_put_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.1 first replay-validated DMA PUT fixture. Symmetric to the
// R6.7 A.5 GET fixture but inverts the DMA direction. The SPU
// fills a 128-byte buffer in LS with a counting pattern, dispatches
// MFC PUT (cmd=0x20) to copy those bytes from LS to EA, waits for
// completion via WrTagMask + WrTagUpdate + RdTagStat, writes a
// sentinel to OUT_MBOX, and halts via stop 0x101.
//
// MFC PUT channel sequence:
//
//   1. (fill LS[lsa..lsa+128] with `i & 0xFF` via store loop)
//   2. ch16 MFC_LSA      ← 0x10000 (LS source)
//   3. ch17 MFC_EAH      ← 0       (PS3 user-space PPU is 32-bit)
//   4. ch18 MFC_EAL      ← <ea>    (low 32 bits of EA pointer)
//   5. ch19 MFC_Size     ← 128
//   6. ch20 MFC_TagID    ← 3
//   7. ch21 MFC_Cmd      ← 0x20  (PUT, LS → EA)
//   8. ch22 WrTagMask    ← 1<<3
//   9. ch23 WrTagUpdate  ← MFC_TAG_UPDATE_ALL (= 2)
//  10. ch24 RdTagStat    → returns mask = 1<<3 once PUT completes
//  11. ch28 OUT_MBOX     ← 0xC0FFEECA (sentinel)
//  12. stop 0x101
//
// Status word `0xC0FFEECA` is the SPU's group-exit sentinel —
// proves the SPU reached the post-PUT path. The PPU verifies the
// PUT BYTES actually landed by reading EA back and computing
// `ea_status = sum(ea) ^ 0xCAFEBABE`. For the canonical counting
// pattern (sum=8128=0x1FC0), `ea_status = 0xCAFEA57E`.
//
// Canonical computation (Python):
//   sentinel = 0xC0FFEECA
//   buf = [i & 0xFF for i in range(128)]
//   sum_of_buf = sum(buf) = 8128 = 0x1FC0
//   ea_status = sum_of_buf ^ 0xCAFEBABE = 0xCAFEA57E
//
// `0xCAFEA57E` is the load-bearing acceptance — only achievable
// when the MFC PUT actually copied the LS bytes to EA AND the SPU
// computed the deterministic source pattern. Any silent fake-PUT
// path (EA stays zero) produces `0 ^ 0xCAFEBABE = 0xCAFEBABE` (a
// different ea_status).
//
// Inlined exit (avoid pulling libsputhread).

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_SRC      ((uint32_t)0x10000)
#define DMA_SIZE     ((uint32_t)128)
#define DMA_TAG      ((uint32_t)3)
#define DMA_TAG_MASK ((uint32_t)(1u << 3))
#define MFC_PUT      ((uint32_t)0x20)
#define SENTINEL     ((uint32_t)0xC0FFEECA)

int main(uint64_t spu_id, uint64_t arg)
{
    (void)arg;

    // Same PSL1GHT arg0 → r3 convention as the GET fixture.
    // `(uint32_t)spu_id` recovers the EA pointer from the low 32
    // bits of the first u64 thread arg.
    uint32_t ea = (uint32_t)spu_id;

    // === Fill LS source buffer ===
    //
    // Write the deterministic counting pattern `i & 0xFF` for
    // i in 0..128 into LS at [LSA_SRC..LSA_SRC+128]. Volatile
    // pointer keeps the compiler honest about the side effects
    // the SPU is about to PUT to EA. Sum of this buffer is
    // 8128 = 0x1FC0, the load-bearing constant the PPU checks.
    volatile uint8_t* ls_buf = (volatile uint8_t*)(uintptr_t)LSA_SRC;
    for (uint32_t i = 0; i < DMA_SIZE; i++) {
        ls_buf[i] = (uint8_t)(i & 0xFF);
    }

    // === MFC PUT sequence ===
    //
    // Same ordering as the GET fixture (params first → cmd
    // dispatches → wait setup → blocking read), but with cmd 0x20
    // (PUT) instead of 0x40 (GET). The R6.7 A.1 writer extension
    // (extended R8.1) captures this sequence: ch16-21 + spu_mfc_cmd
    // (with cmd=0x20 + a `.dmachunk` carrying the LS source bytes
    // at dispatch time) + mfc_dma_complete + ch22-23 + rdch ch24.
    spu_writech(MFC_LSA, LSA_SRC);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, ea);
    spu_writech(MFC_Size, DMA_SIZE);
    spu_writech(MFC_TagID, DMA_TAG);
    spu_writech(MFC_Cmd, MFC_PUT);

    // Wait for the dispatched tag to complete.
    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;  // expected = DMA_TAG_MASK; not asserted here

    // === Sentinel to OUT_MBOX ===
    //
    // The SPU's group-exit status (which lv2 reads from OUT_MBOX
    // when stop 0x101 fires) is the sentinel `0xC0FFEECA`. The PPU
    // gets this in `sysSpuThreadGroupJoin`'s `status` out-param.
    spu_writech(SPU_WrOutMbox, SENTINEL);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
