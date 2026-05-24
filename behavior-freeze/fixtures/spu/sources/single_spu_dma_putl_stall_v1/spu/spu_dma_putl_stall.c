// single_spu_dma_putl_stall_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.5e E.3 — MFC PUTL list-DMA with stall-and-notify
// (sb & 0x80). Symmetric inverse of R8.5d D.3 GETL stall:
// instead of pulling EA bytes into LS, this pushes LS bytes
// into EA via a 3-element descriptor list with element 1
// carrying sb=0x80.
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
// PUTL dispatch channel sequence:
//   ch16 MFC_LSA      = 0x10000       (source base in LS)
//   ch17 MFC_EAH      = 0
//   ch18 MFC_EAL      = LS offset of list_element[] array
//   ch19 MFC_Size     = 24            (= 3 elements * 8 bytes)
//   ch20 MFC_TagID    = 3
//   ch21 MFC_Cmd      = 0x24 PUTL
//
// Cell BE Sec. 12.5 transfer-then-stall: when element 1 with
// sb&0x80 is encountered, the MFC completes the per-element
// transfer FIRST (LS[0x10080..0x100C0] copied to ea_buf2), THEN
// raises the stall bit. The SPU observes the stall via ch25,
// acknowledges via ch26, and the MFC resumes the walk from
// element 2 (LS[0x100C0..0x10120] → ea_buf3).

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_SRC_BASE   ((uint32_t)0x10000)
#define DMA_TAG        ((uint32_t)3)
#define DMA_TAG_MASK   ((uint32_t)(1u << 3))
#define MFC_PUTL       ((uint32_t)0x24)
#define SPU_SENTINEL   ((uint32_t)0xC0FFEEC3u)

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
    // PSL1GHT convention: PPU packs EA1+EA2 into arg0, EA3 into arg1 high.
    uint32_t ea1 = (uint32_t)(spu_id >> 32);
    uint32_t ea2 = (uint32_t)(spu_id & 0xFFFFFFFFu);
    uint32_t ea3 = (uint32_t)(arg >> 32);

    // === Fill LS source regions with deterministic content ===
    //
    // Same patterns as R8.5d D.3 GETL stall fixture so the
    // captured `.dmachunk` side-files deduplicate with the
    // canonical pool:
    //   - element 0: 128 B counting pattern (i & 0xFF), sum=0x1FC0
    //   - element 1:  64 B constant 0x42,              sum=0x1080
    //   - element 2:  96 B constant 0x11,              sum=0x0660
    volatile uint8_t* ls_src_1 =
        (volatile uint8_t*)(uintptr_t)LSA_SRC_BASE;
    for (uint32_t i = 0; i < EL_SIZE_1; i++) {
        ls_src_1[i] = (uint8_t)(i & 0xFFu);
    }
    volatile uint8_t* ls_src_2 =
        (volatile uint8_t*)(uintptr_t)(LSA_SRC_BASE + EL_SIZE_1);
    for (uint32_t i = 0; i < EL_SIZE_2; i++) {
        ls_src_2[i] = 0x42u;
    }
    volatile uint8_t* ls_src_3 =
        (volatile uint8_t*)(uintptr_t)(LSA_SRC_BASE + EL_SIZE_1 + EL_SIZE_2);
    for (uint32_t i = 0; i < EL_SIZE_3; i++) {
        ls_src_3[i] = 0x11u;
    }

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

    // === MFC PUTL dispatch (tag 3, 3-element list) ===
    spu_writech(MFC_LSA, LSA_SRC_BASE);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, (uint32_t)(uintptr_t)&list_descriptors[0]);
    spu_writech(MFC_Size, (uint32_t)(sizeof(list_descriptors)));
    spu_writech(MFC_TagID, DMA_TAG);
    spu_writech(MFC_Cmd, MFC_PUTL);

    // === Observe stall via ch25 MFC_RdListStallStat ===
    // MFC transfers element 0 (128 B) + element 1 (64 B —
    // including the stalled element per Cell BE Sec. 12.5),
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
    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;  // expected = DMA_TAG_MASK; not asserted

    // === Emit canonical SPU sentinel + halt ===
    //
    // The sentinel is FIXED (0xC0FFEEC3, distinct from
    // PUTL_v1's 0xC0FFEEBA and from getl_stall_v1's status).
    // The PPU separately sums all three EA destination buffers
    // (which the DMA filled atomically) and computes
    // `ea_status = ((sum_ea1 << 16) | ((sum_ea2 + sum_ea3) &
    // 0xFFFF)) ^ 0xBEEFCAFE`.
    // Canonical: spu=0xC0FFEEC3, ea_status=0xA12FDC1E.
    spu_writech(SPU_WrOutMbox, SPU_SENTINEL);
    spu_stop(0x101);

    // Unreachable.
    return 0;
}
