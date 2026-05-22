// single_spu_dma_putlb_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.4f-b — MFC PUTLB list-DMA + barrier dispatch (cmd=0x25).
// Identical to R8.4e PUTL SPU side except cmd=0x25 and SPU
// sentinel 0xC0FFEEBB (last byte BB = Barrier mnemonic).

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_SRC_BASE   ((uint32_t)0x10000)
#define DMA_TAG        ((uint32_t)3)
#define DMA_TAG_MASK   ((uint32_t)(1u << 3))
#define MFC_PUTLB      ((uint32_t)0x25)
#define SPU_SENTINEL   ((uint32_t)0xC0FFEEBBu)

#define EL_SIZE_1      128
#define EL_SIZE_2      64

typedef struct __attribute__((packed, aligned(8))) {
    uint8_t  sb;
    uint8_t  pad;
    uint16_t ts;
    uint32_t ea;
} list_element_t;

static list_element_t list_descriptors[2] __attribute__((aligned(8)));

int main(uint64_t spu_id, uint64_t arg)
{
    uint32_t ea1 = (uint32_t)spu_id;
    uint32_t ea2 = (uint32_t)arg;

    volatile uint8_t* ls_src_1 = (volatile uint8_t*)(uintptr_t)LSA_SRC_BASE;
    for (uint32_t i = 0; i < EL_SIZE_1; i++) ls_src_1[i] = (uint8_t)(i & 0xFFu);
    volatile uint8_t* ls_src_2 = (volatile uint8_t*)(uintptr_t)(LSA_SRC_BASE + EL_SIZE_1);
    for (uint32_t i = 0; i < EL_SIZE_2; i++) ls_src_2[i] = 0x42u;

    list_descriptors[0].sb  = 0;
    list_descriptors[0].pad = 0;
    list_descriptors[0].ts  = EL_SIZE_1;
    list_descriptors[0].ea  = ea1;
    list_descriptors[1].sb  = 0;
    list_descriptors[1].pad = 0;
    list_descriptors[1].ts  = EL_SIZE_2;
    list_descriptors[1].ea  = ea2;

    spu_writech(MFC_LSA, LSA_SRC_BASE);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, (uint32_t)(uintptr_t)&list_descriptors[0]);
    spu_writech(MFC_Size, (uint32_t)(sizeof(list_descriptors)));
    spu_writech(MFC_TagID, DMA_TAG);
    spu_writech(MFC_Cmd, MFC_PUTLB);

    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;

    spu_writech(SPU_WrOutMbox, SPU_SENTINEL);
    spu_stop(0x101);

    return 0;
}
