// single_spu_dma_getl_stall_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.5d D.3 — MFC GETL list-DMA with stall-and-notify
// (sb & 0x80). Builds a 3-element descriptor list in LS,
// dispatches GETL via cmd=0x44, observes the stall on
// element 1 via ch25 (MFC_RdListStallStat), acknowledges via
// ch26 (MFC_WrListStallAck), then waits for full-list
// tag-stat completion via ch24.
//
// Cell BE list_element layout (8 bytes per element, native BE
// on SPU which is already big-endian):
//
//   struct list_element {
//       uint8_t  sb;   // stall-and-notify (bit 0x80 = stall here)
//       uint8_t  pad;
//       uint16_t ts;   // transfer size (bytes)
//       uint32_t ea;   // External Address Low (data EA per element)
//   };
//
// GETL dispatch channel sequence:
//   ch16 MFC_LSA      = 0x10000       (destination base)
//   ch17 MFC_EAH      = 0
//   ch18 MFC_EAL      = LS offset of list_element[] array
//   ch19 MFC_Size     = 24            (= 3 elements * 8 bytes)
//   ch20 MFC_TagID    = 3
//   ch21 MFC_Cmd      = 0x44 GETL
//
// Cell BE Sec. 12.5 transfer-then-stall: when element 1 with
// sb&0x80 is encountered, the MFC completes the per-element
// transfer FIRST (LS[0x10080..0x100C0] gets element 1's 64
// bytes), THEN raises the stall bit for the tag. The SPU
// observes the stall via ch25, acknowledges via ch26, and
// the MFC resumes the walk from element 2.

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
#define EL_SIZE_3      96

// 8-byte aligned 3-element list. SPU is big-endian, so no
// htons/htonl needed for the descriptor fields.
typedef struct __attribute__((packed, aligned(8))) {
    uint8_t  sb;
    uint8_t  pad;
    uint16_t ts;
    uint32_t ea;
} list_element_t;

static list_element_t list_descriptors[3] __attribute__((aligned(8)));

int main(uint64_t spu_id, uint64_t arg)
{
    // PSL1GHT convention: SPU thread args are packed into spu_id
    // / arg high+low halves. The PPU host packs ea1 into spu_id
    // high, ea2 into spu_id low, ea3 into arg high.
    uint32_t ea1 = (uint32_t)(spu_id >> 32);
    uint32_t ea2 = (uint32_t)(spu_id & 0xFFFFFFFFu);
    uint32_t ea3 = (uint32_t)(arg >> 32);

    // === Build the descriptor list in LS ===
    list_descriptors[0].sb  = 0;
    list_descriptors[0].pad = 0;
    list_descriptors[0].ts  = EL_SIZE_1;
    list_descriptors[0].ea  = ea1;

    list_descriptors[1].sb  = 0x80;   // STALL HERE post-transfer
    list_descriptors[1].pad = 0;
    list_descriptors[1].ts  = EL_SIZE_2;
    list_descriptors[1].ea  = ea2;

    list_descriptors[2].sb  = 0;
    list_descriptors[2].pad = 0;
    list_descriptors[2].ts  = EL_SIZE_3;
    list_descriptors[2].ea  = ea3;

    // === MFC GETL dispatch (tag 3, 3-element list) ===
    spu_writech(MFC_LSA, LSA_DEST_BASE);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, (uint32_t)(uintptr_t)&list_descriptors[0]);
    spu_writech(MFC_Size, (uint32_t)(sizeof(list_descriptors)));
    spu_writech(MFC_TagID, DMA_TAG);
    spu_writech(MFC_Cmd, MFC_GETL);

    // === Observe stall via ch25 MFC_RdListStallStat ===
    // The MFC transfers element 0 (128 B) and element 1 (64 B)
    // — including the stalled element per Cell BE Sec. 12.5 —
    // then raises the per-tag stall bit. ch25 read returns the
    // 32-bit mask of stalled tags (destructive: returns then
    // clears).
    uint32_t stall_mask = spu_readch(MFC_RdListStallStat);
    (void)stall_mask;  // expected = DMA_TAG_MASK (= 0x08); not asserted

    // === Acknowledge via ch26 MFC_WrListStallAck ===
    // ch26 takes a tag id (NOT a bitmask). The MFC then resumes
    // the descriptor walk from element 2.
    spu_writech(MFC_WrListStallAck, DMA_TAG);

    // === Tag-stat wait (ALL mode) ===
    // After ack + resume, the full list completes (element 2
    // copied to LS[0x100C0..0x10120]) and the MFC raises the
    // tag-stat bit normally.
    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;  // expected = DMA_TAG_MASK; not asserted

    // === Post-DMA: sum all 3 copied regions + compute status ===
    //
    // Element 0 lands at LS[LSA_DEST_BASE..+EL_SIZE_1]
    //                    = 0x10000..0x10080  (128 B)
    // Element 1 lands at LS[+EL_SIZE_1..+EL_SIZE_2]
    //                    = 0x10080..0x100C0  (64 B; the stalled
    //                                          element is fully
    //                                          transferred BEFORE
    //                                          stall raise)
    // Element 2 lands at LS[+EL_SIZE_1+EL_SIZE_2..+EL_SIZE_3]
    //                    = 0x100C0..0x10120  (96 B; post-ack
    //                                          resume)
    volatile const uint8_t* ls_buf_1 =
        (volatile const uint8_t*)(uintptr_t)LSA_DEST_BASE;
    volatile const uint8_t* ls_buf_2 =
        (volatile const uint8_t*)(uintptr_t)(LSA_DEST_BASE + EL_SIZE_1);
    volatile const uint8_t* ls_buf_3 =
        (volatile const uint8_t*)(uintptr_t)(LSA_DEST_BASE + EL_SIZE_1 + EL_SIZE_2);

    uint32_t sum1 = 0;
    for (uint32_t i = 0; i < EL_SIZE_1; i++) {
        sum1 += (uint32_t)ls_buf_1[i];
    }

    uint32_t sum2 = 0;
    for (uint32_t i = 0; i < EL_SIZE_2; i++) {
        sum2 += (uint32_t)ls_buf_2[i];
    }

    uint32_t sum3 = 0;
    for (uint32_t i = 0; i < EL_SIZE_3; i++) {
        sum3 += (uint32_t)ls_buf_3[i];
    }

    // Pack: top 16 bits = sum1, low 16 bits = (sum2 + sum3).
    // sum1 = 0x1FC0, sum2 = 0x1080, sum3 = 0x0660, so
    //   combined = (0x1FC0 << 16) | 0x16E0 = 0x1FC0_16E0
    //   status   = 0x1FC0_16E0 ^ 0xC0DEFADA = 0xDF1E_EC3A
    uint32_t combined = (sum1 << 16) | ((sum2 + sum3) & 0xFFFFu);
    uint32_t status = combined ^ STATUS_MASK;

    spu_writech(SPU_WrOutMbox, status);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
