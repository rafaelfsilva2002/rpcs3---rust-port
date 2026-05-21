// single_spu_dma_putl_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.4e — MFC PUTL list-DMA dispatch (cmd=0x24). Symmetric inverse
// of R8.4b GETL: instead of pulling EA bytes into LS, this pushes
// LS bytes into EA via a 2-element descriptor list. The
// descriptor layout is identical to GETL (Cell BE list-DMA puts
// the descriptor in LS for both directions).
//
// Cell BE list_element layout (8 bytes per element, native BE
// on SPU which is already big-endian):
//
//   struct list_element {
//       uint8_t  sb;   // stall-and-notify (bit 0x80; we use 0)
//       uint8_t  pad;
//       uint16_t ts;   // transfer size (bytes)
//       uint32_t ea;   // External Address Low (data EA per element)
//   };
//
// PUTL dispatch channel sequence:
//   ch16 MFC_LSA      = 0x10000       (source base in LS)
//   ch17 MFC_EAH      = 0
//   ch18 MFC_EAL      = LS offset of list_element[] array
//   ch19 MFC_Size     = 16            (= 2 elements * 8 bytes)
//   ch20 MFC_TagID    = 3
//   ch21 MFC_Cmd      = 0x24 PUTL
//
// Note: for PUTL, ch16 is the LS *source* base (mfc_lsa), and
// ch18 is the LS offset of the descriptor list (NOT a data EA).
// Per-element ea field points to the EA destination.

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_SRC_BASE   ((uint32_t)0x10000)
#define DMA_TAG        ((uint32_t)3)
#define DMA_TAG_MASK   ((uint32_t)(1u << 3))
#define MFC_PUTL       ((uint32_t)0x24)
#define SPU_SENTINEL   ((uint32_t)0xC0FFEEBAu)

#define EL_SIZE_1      128
#define EL_SIZE_2      64

// 8-byte aligned 2-element list. SPU is big-endian, so no
// htons/htonl needed for the descriptor fields.
typedef struct __attribute__((packed, aligned(8))) {
    uint8_t  sb;
    uint8_t  pad;
    uint16_t ts;
    uint32_t ea;
} list_element_t;

static list_element_t list_descriptors[2] __attribute__((aligned(8)));

int main(uint64_t spu_id, uint64_t arg)
{
    // PSL1GHT convention: arg0 → r3 (EA1), arg1 → r4 (EA2).
    uint32_t ea1 = (uint32_t)spu_id;
    uint32_t ea2 = (uint32_t)arg;

    // === Fill LS source regions with deterministic content ===
    // Element 0 source: 128 B counting pattern (i & 0xFF).
    // Same SHA as R6.7 GET / R8.1 PUT / R8.2..R8.4b chunks —
    // perfect dedup in canonical .dmachunk pool.
    volatile uint8_t* ls_src_1 =
        (volatile uint8_t*)(uintptr_t)LSA_SRC_BASE;
    for (uint32_t i = 0; i < EL_SIZE_1; i++) {
        ls_src_1[i] = (uint8_t)(i & 0xFFu);
    }
    // Element 1 source: 64 B constant 0x42. Same SHA as
    // R8.2..R8.4b second chunks.
    volatile uint8_t* ls_src_2 =
        (volatile uint8_t*)(uintptr_t)(LSA_SRC_BASE + EL_SIZE_1);
    for (uint32_t i = 0; i < EL_SIZE_2; i++) {
        ls_src_2[i] = 0x42u;
    }

    // === Build the descriptor list in LS ===
    list_descriptors[0].sb  = 0;
    list_descriptors[0].pad = 0;
    list_descriptors[0].ts  = EL_SIZE_1;
    list_descriptors[0].ea  = ea1;

    list_descriptors[1].sb  = 0;
    list_descriptors[1].pad = 0;
    list_descriptors[1].ts  = EL_SIZE_2;
    list_descriptors[1].ea  = ea2;

    // === MFC PUTL dispatch (tag 3) ===
    spu_writech(MFC_LSA, LSA_SRC_BASE);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, (uint32_t)(uintptr_t)&list_descriptors[0]);
    spu_writech(MFC_Size, (uint32_t)(sizeof(list_descriptors)));
    spu_writech(MFC_TagID, DMA_TAG);
    spu_writech(MFC_Cmd, MFC_PUTL);

    // === Tag wait (ALL mode) ===
    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;  // expected = DMA_TAG_MASK; not asserted

    // === Emit canonical SPU sentinel + halt ===
    // The sentinel is FIXED. The PPU separately sums both EA
    // destination buffers (which the DMA filled atomically) and
    // computes ea_status = ((sum_ea1<<16)|sum_ea2) ^ 0xBEEFCAFE.
    // Canonical: spu=0xC0FFEEBA, ea_status=0xA12FDA7E.
    spu_writech(SPU_WrOutMbox, SPU_SENTINEL);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
