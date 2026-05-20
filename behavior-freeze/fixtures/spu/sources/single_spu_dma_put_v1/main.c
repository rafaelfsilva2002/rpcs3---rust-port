// single_spu_dma_put_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R8.1 first replay-validated DMA PUT fixture. Symmetric to R6.7
// A.5 GET: instead of EA → LS (SPU receives), this one is LS → EA
// (SPU produces). PPU allocates a 128-byte EA buffer ZERO-FILLED,
// passes the EA via `sysSpuThreadArgument.arg0`, joins the SPU,
// then reads back the EA to verify the SPU's PUT landed.
//
// Behaviour (deterministic):
//   1. PPU allocates 128-byte ea_buf, zero-fills it. Buffer is
//      128-byte aligned (MFC PUT requires 16-byte alignment for
//      sizes >= 16).
//   2. PPU passes EA via thread arg[0]; sysSpuThreadGroupStart.
//   3. SPU fills LS[lsa..lsa+128] with `buf[i] = i & 0xFF`.
//   4. SPU dispatches MFC PUT (cmd=0x20) from lsa to EA, size 128.
//   5. SPU waits for tag via WrTagMask=1<<3, WrTagUpdate=ALL,
//      rdch ch24 (blocks until the PUT lands).
//   6. SPU writes a sentinel `0xC0FFEECA` to OUT_MBOX and halts
//      via stop 0x101 (SYS_SPU_THREAD_STOP_GROUP_EXIT).
//   7. PPU joins; lv2 reads OUT_MBOX as the group-exit status
//      (= sentinel).
//   8. PPU reads back the EA buffer, computes sum, XORs with
//      0xCAFEBABE to produce ea_status. Both numbers go on TTY.
//
// Canonical computation (reference Python, executed below for
// self-verification by anyone reading this source):
//
//   sentinel = 0xC0FFEECA
//   buf = [i & 0xFF for i in range(128)]
//   ea_after_put = buf                              # SPU writes buf to EA
//   sum_of_ea = sum(ea_after_put)                   # = 8128 = 0x1FC0
//   ea_status = sum_of_ea ^ 0xCAFEBABE              # = 0xCAFEA57E
//
// Expected TTY:
//   [dma_put_v1] OK cause=0x1 spu=0xc0ffeeca ea_status=0xcafea57e
//
// The fixture is the load-bearing R8.1 PUT oracle:
//   - cause=0x1 (GROUP_EXIT) proves the SPU stopped cleanly.
//   - spu=0xc0ffeeca proves the SPU reached its post-PUT sentinel
//     (i.e. the rdch ch24 unblocked, which means the tag completed,
//     which means the PUT was acknowledged by RPCS3's MFC).
//   - ea_status=0xcafea57e proves the PUT BYTES actually landed in
//     EA. A silent fake-PUT path (PPU still sees zeros) produces
//     ea_status = 0 ^ 0xCAFEBABE = 0xCAFEBABE (different).
//   - A wrong-byte-pattern PUT (e.g., the SPU writes garbage)
//     produces a third distinctive ea_status.
// All three failure modes are observable from a single TTY line.

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_put_bin[];
extern const u32 spu_dma_put_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

// 128-byte aligned buffer in BSS, ZERO-FILLED (matches "no fake
// PUT" invariant: if the SPU never PUTs, PPU reads back zeros).
static u8 ea_buf[128] __attribute__((aligned(128)));

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    // Explicitly zero ea_buf (BSS is zero-initialised but we make
    // the contract visible to the reader).
    memset(ea_buf, 0, sizeof(ea_buf));

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[dma_put_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_put_bin, 0);
    if (ret) {
        printf("[dma_put_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_put_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_put_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);
    // Same arg0 → r3 PSL1GHT convention as the GET fixture
    // (verified in RPCS3 `lv2/sys_spu.cpp:1229`).
    thread_args.arg0 = (u64)(uintptr_t)ea_buf;

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[dma_put_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_put_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[dma_put_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // PPU-side EA readback + verification. The SPU's OUT_MBOX
    // sentinel is in `status`; the SPU's actual PUT'd bytes are
    // now in ea_buf (the EA we handed to the SPU).
    u32 ea_sum = 0;
    for (u32 i = 0; i < 128; i++) {
        ea_sum += ea_buf[i];
    }
    const u32 ea_status = ea_sum ^ 0xCAFEBABE;

    // Expected: cause=0x1 (GROUP_EXIT), spu=0xC0FFEECA (sentinel),
    // ea_status=0xCAFEA57E (sum 8128 XOR 0xCAFEBABE).
    printf("[dma_put_v1] OK cause=0x%x spu=0x%x ea_status=0x%x\n",
           (unsigned)cause, (unsigned)status, (unsigned)ea_status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
