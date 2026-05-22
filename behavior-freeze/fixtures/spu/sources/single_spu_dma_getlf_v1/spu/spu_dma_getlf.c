// single_spu_dma_getlf_v1 — SPU side
// CC0 1.0 (public domain). See ../LICENSE.md.
//
// R8.4f-a — MFC GETLF list-DMA + fence dispatch (cmd=0x46).
// Identical to R8.4c GETL SPU side except cmd=0x46 and
// status mask 0xC0DEFAFF (last byte FF = Fence mnemonic).

#include <spu_mfcio.h>
#include <spu_intrinsics.h>
#include <stdint.h>

#define LSA_DEST_BASE  ((uint32_t)0x10000)
#define DMA_TAG        ((uint32_t)3)
#define DMA_TAG_MASK   ((uint32_t)(1u << 3))
#define MFC_GETLF      ((uint32_t)0x46)
#define STATUS_MASK    ((uint32_t)0xC0DEFAFFu)

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

    list_descriptors[0].sb  = 0;
    list_descriptors[0].pad = 0;
    list_descriptors[0].ts  = EL_SIZE_1;
    list_descriptors[0].ea  = ea1;

    list_descriptors[1].sb  = 0;
    list_descriptors[1].pad = 0;
    list_descriptors[1].ts  = EL_SIZE_2;
    list_descriptors[1].ea  = ea2;

    spu_writech(MFC_LSA, LSA_DEST_BASE);
    spu_writech(MFC_EAH, 0u);
    spu_writech(MFC_EAL, (uint32_t)(uintptr_t)&list_descriptors[0]);
    spu_writech(MFC_Size, (uint32_t)(sizeof(list_descriptors)));
    spu_writech(MFC_TagID, DMA_TAG);
    spu_writech(MFC_Cmd, MFC_GETLF);

    spu_writech(MFC_WrTagMask, DMA_TAG_MASK);
    spu_writech(MFC_WrTagUpdate, MFC_TAG_UPDATE_ALL);
    uint32_t tag_stat = spu_readch(MFC_RdTagStat);
    (void)tag_stat;

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

    return 0;
}
