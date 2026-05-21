// single_spu_dma_getl_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.4b — MFC GETL list-DMA dispatch. Builds a 2-element
// descriptor list in LS, dispatches GETL via cmd=0x44, waits,
// computes canonical status from the copied bytes.
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
// GETL dispatch channel sequence:
//   ch16 MFC_LSA      = 0x10000       (destination base)
//   ch17 MFC_EAH      = 0
//   ch18 MFC_EAL      = LS offset of list_element[] array
//   ch19 MFC_Size     = 16            (= 2 elements * 8 bytes)
//   ch20 MFC_TagID    = 3
//   ch21 MFC_Cmd      = 0x44 GETL
//
// Note: for GETL, ch18 is the LS offset of the descriptor list,
// not a data EA. The descriptors themselves are in LS; each
// descriptor's `ea` field points to data in EA memory.

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_DEST_BASE  ((uint32_t)0x10000)
#define DMA_TAG        ((uint32_t)3)
#define DMA_TAG_MASK   ((uint32_t)(1u << 3))
#define MFC_GETL       ((uint32_t)0x44)
#define STATUS_MASK    ((uint32_t)0xC0DEFADAu)

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

    // === Build the descriptor list in LS ===
    list_descriptors[0].sb  = 0;
    list_descriptors[0].pad = 0;
    list_descriptors[0].ts  = EL_SIZE_1;
    list_descriptors[0].ea  = ea1;

    list_descriptors[1].sb  = 0;
    list_descriptors[1].pad = 0;
    list_descriptors[1].ts  = EL_SIZE_2;
    list_descriptors[1].ea  = ea2;

    // === MFC GETL dispatch (tag 3) ===
    spu_writech(MFC_LSA, LSA_DEST_BASE);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, (uint32_t)(uintptr_t)&list_descriptors[0]);
    spu_writech(MFC_Size, (uint32_t)(sizeof(list_descriptors)));
    spu_writech(MFC_TagID, DMA_TAG);
    spu_writech(MFC_Cmd, MFC_GETL);

    // === Tag wait (ALL mode) ===
    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;  // expected = DMA_TAG_MASK; not asserted

    // === Post-DMA: sum both copied regions + compute status ===
    //
    // Element 0 lands at LS[LSA_DEST_BASE..+EL_SIZE_1] = 0x10000..0x10080
    // Element 1 lands at LS[0x10080..+EL_SIZE_2]       = 0x10080..0x100C0
    // (Cell BE list semantics: subsequent elements land at
    //  cumulative-offset = base + sum of aligned-up ts of
    //  preceding elements; for ts=128 aligned to 16 = 128, no
    //  extra padding here.)
    volatile const uint8_t* ls_buf_1 =
        (volatile const uint8_t*)(uintptr_t)LSA_DEST_BASE;
    volatile const uint8_t* ls_buf_2 =
        (volatile const uint8_t*)(uintptr_t)(LSA_DEST_BASE + EL_SIZE_1);

    uint32_t sum1 = 0;
    for (uint32_t i = 0; i < EL_SIZE_1; i++) {
        sum1 += (uint32_t)ls_buf_1[i];
    }

    uint32_t sum2 = 0;
    for (uint32_t i = 0; i < EL_SIZE_2; i++) {
        sum2 += (uint32_t)ls_buf_2[i];
    }

    uint32_t combined = (sum1 << 16) | (sum2 & 0xFFFFu);
    uint32_t status = combined ^ STATUS_MASK;

    spu_writech(SPU_WrOutMbox, status);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
