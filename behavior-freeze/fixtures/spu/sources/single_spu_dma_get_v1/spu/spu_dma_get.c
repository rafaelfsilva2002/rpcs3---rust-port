// single_spu_dma_get_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R6.7 A.5 first replay-validated DMA GET fixture. The SPU receives
// the EA pointer in the second `main` argument (PSL1GHT convention:
// thread arg[0] → SPU r3..r6 preferred slot via lv2). The SPU then
// runs a complete MFC GET sequence:
//
//   1. ch16 MFC_LSA      ← 0x10000 (LS destination, 16-byte aligned)
//   2. ch17 MFC_EAH      ← 0     (PS3 user-space PPU is 32-bit; eah=0)
//   3. ch18 MFC_EAL      ← <ea>  (low 32 bits of EA pointer)
//   4. ch19 MFC_Size     ← 128   (transfer size in bytes)
//   5. ch20 MFC_TagID    ← 3     (arbitrary tag in 0..32)
//   6. ch21 MFC_Cmd      ← 0x40  (GET, EA → LS)
//   7. ch22 WrTagMask    ← 1<<3
//   8. ch23 WrTagUpdate  ← MFC_TAG_UPDATE_ALL (= 2)
//   9. ch24 RdTagStat    → returns mask = 1<<3 once GET completes
//
// After the wait completes, the LS region [0x10000..0x10080] holds
// the bytes the PPU wrote to EA. The SPU sums all 128 bytes, XORs
// with 0xDEADBEEF, and writes the result to OUT_MBOX (ch28). Then
// halts via `stop 0x101` (SYS_SPU_THREAD_STOP_GROUP_EXIT) so lv2
// reads OUT_MBOX as the group-exit status.
//
// Canonical computation (matches the PPU-side README):
//
//   buf[i] = i & 0xFF for i in 0..128       ; PPU fills the EA buffer
//   sum_of_buf = sum(buf) = 8128 = 0x1FC0   ; sum of 0..127 = 8128
//   cs = sum_of_buf ^ 0xDEADBEEF = 0xDEADA12F
//
// Status 0xDEADA12F is the load-bearing acceptance — only achievable
// when the MFC GET actually copied the EA bytes into LS AND the
// SPU computed the deterministic post-DMA sum + XOR. Any silent
// fake-DMA path (zero-fill LS) produces 0 ^ 0xDEADBEEF = 0xDEADBEEF
// (a different status).
//
// Inlined exit (avoid pulling libsputhread).

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_DEST     ((uint32_t)0x10000)
#define DMA_SIZE     ((uint32_t)128)
#define DMA_TAG      ((uint32_t)3)
#define DMA_TAG_MASK ((uint32_t)(1u << 3))
#define MFC_GET      ((uint32_t)0x40)

int main(uint64_t spu_id, uint64_t arg)
{
    (void)arg;

    // The PSL1GHT lv2 path writes `thread_args.arg0` to SPU r3 (low
    // u64 lane) — verified in RPCS3 `lv2/sys_spu.cpp:1229`. The SPU
    // C ABI's first u64 parameter (`spu_id`) is read from r3, so
    // `(uint32_t)spu_id` extracts the low 32 bits of arg0 = our EA.
    // The PPU set `thread_args.arg0 = (u64)(uintptr_t)ea_buf` in
    // main.c.
    uint32_t ea = (uint32_t)spu_id;

    // === MFC GET sequence ===
    //
    // Order: param channels (ch16..ch20) → cmd channel (ch21
    // synchronously dispatches the transfer in C++) → wait setup
    // (ch22 mask, ch23 mode = ALL) → blocking read (ch24 RdTagStat).
    //
    // Per Cell BE / R6.7 design, the C++ side of RPCS3 runs the
    // actual EA→LS copy in `process_mfc_cmd()` synchronously on
    // ch21 dispatch. By the time the SPU reaches ch24, the copy has
    // completed and the tag bit is set. The R6.7 A.1 writer extension
    // captures this sequence into the JSONL trace.
    spu_writech(MFC_LSA, LSA_DEST);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, ea);
    spu_writech(MFC_Size, DMA_SIZE);
    spu_writech(MFC_TagID, DMA_TAG);
    spu_writech(MFC_Cmd, MFC_GET);

    // Wait for the dispatched tag to complete.
    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;  // expected = DMA_TAG_MASK; not asserted here

    // === Post-DMA checksum ===
    //
    // Read the 128 bytes the GET deposited in LS at [LSA_DEST..LSA_DEST+128]
    // via volatile pointer. Counting-pattern PPU buffer means the
    // SPU sees buf[i] = i & 0xFF. Sum is 8128 (0x1FC0).
    volatile const uint8_t* ls_buf = (volatile const uint8_t*)(uintptr_t)LSA_DEST;
    uint32_t cs = 0;
    for (uint32_t i = 0; i < DMA_SIZE; i++) {
        cs += (uint32_t)ls_buf[i];
    }
    cs ^= 0xDEADBEEFu;  // final mix

    // OUT_MBOX = canonical 0xDEADA12F for inputs above. lv2 reads
    // OUT_MBOX as the group-exit status when stop 0x101 fires.
    spu_writech(SPU_WrOutMbox, cs);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
