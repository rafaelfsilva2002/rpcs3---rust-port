// single_spu_dma_get_v1 — PPU side
// CC0 1.0 (public domain). See LICENSE.md.
//
// R6.7 A.5 first replay-validated DMA GET fixture. PPU allocates a
// 128-byte CC0/deterministic buffer, fills it with a counting
// pattern (`buf[i] = i & 0xFF`), passes the buffer's effective
// address to the SPU via `sysSpuThreadArgument.arg[0]`, and joins
// the thread group. The SPU runs a complete MFC GET sequence
// (ch16..ch23 wrches + rdch ch24), reads back the DMA'd bytes
// from LS, computes a deterministic checksum, writes it to
// OUT_MBOX, and halts with `stop 0x101` (SYS_SPU_THREAD_STOP_GROUP_EXIT).
//
// Behaviour (deterministic):
//   1. PPU fills 128-byte EA buffer with `i & 0xFF` for i in 0..128.
//      Buffer is 128-byte aligned (MFC ABI requires 16-byte alignment
//      for sizes >= 16).
//   2. PPU passes EA via thread arg[0]; sysSpuThreadGroupStart.
//   3. SPU GETs the 128 bytes from EA into LS at lsa=0x10000.
//   4. SPU sums all 128 bytes (= 8128 = 0x1FC0), XORs with 0xDEADBEEF
//      → final cs = 0xDEADA12F.
//   5. SPU writes cs to OUT_MBOX, halts with stop 0x101.
//   6. PPU joins; lv2 reads OUT_MBOX as the group-exit status.
//
// Expected TTY: `[dma_get_v1] OK cause=0x1 status=0xdeada12f`
//
// Canonical computation (reference Python, executed below for
// self-verification by anyone reading this source):
//
//   buf = [i & 0xFF for i in range(128)]
//   sum_of_buf = sum(buf)                        # = 8128 = 0x1FC0
//   cs = sum_of_buf ^ 0xDEADBEEF                 # = 0xDEADA12F
//
// The fixture is the load-bearing R6.7 GET-only oracle: the only
// way to produce status=0xDEADA12F is to (a) actually load the
// pre-DMA EA bytes into LS, AND (b) compute the deterministic
// post-DMA sum + XOR. A bridge bug that drops the GET silently
// (zero-fills LS) would produce status=0x21524111 (= 0 ^
// 0xDEADBEEF, then post-XOR adjustment, see README.md for
// canonical wrong-paths).

#include <ppu-types.h>
#include <ppu-asm.h>
#include <sys/spu.h>
#include <sys/process.h>
#include <stdio.h>
#include <string.h>

extern const u8  spu_dma_get_bin[];
extern const u32 spu_dma_get_bin_size;

SYS_PROCESS_PARAM(1001, 0x10000);

// 128-byte aligned buffer in BSS. Aligned to a 128-byte boundary so
// the MFC GET's 16-byte alignment requirement is comfortably met.
// Size = 128 bytes = exactly the MFC GET transfer size.
static u8 ea_buf[128] __attribute__((aligned(128)));

int main(s32 argc, const char *argv[])
{
    (void)argc;
    (void)argv;

    s32 ret;

    // Fill EA buffer with a deterministic counting pattern. The SPU
    // will GET these bytes into LS and compute their sum.
    for (u32 i = 0; i < 128; i++) {
        ea_buf[i] = (u8)(i & 0xFF);
    }

    ret = sysSpuInitialize(1, 0);
    if (ret) {
        printf("[dma_get_v1] sysSpuInitialize: 0x%08x\n", ret);
        return 1;
    }

    sysSpuImage spu_image;
    ret = sysSpuImageImport(&spu_image, spu_dma_get_bin, 0);
    if (ret) {
        printf("[dma_get_v1] sysSpuImageImport: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadGroupAttribute group_attr;
    sysSpuThreadGroupAttributeInitialize(group_attr);
    sysSpuThreadGroupAttributeName(group_attr, "dma_get_v1");
    sysSpuThreadGroupAttributeType(group_attr, SPU_THREAD_GROUP_TYPE_NORMAL);

    sys_spu_group_t group_id;
    ret = sysSpuThreadGroupCreate(&group_id, 1, 100, &group_attr);
    if (ret) {
        printf("[dma_get_v1] sysSpuThreadGroupCreate: 0x%08x\n", ret);
        return 1;
    }

    sysSpuThreadAttribute thread_attr;
    sysSpuThreadAttributeInitialize(thread_attr);
    sysSpuThreadAttributeName(thread_attr, "spu_0");
    sysSpuThreadAttributeOption(thread_attr, SPU_THREAD_ATTR_NONE);

    sysSpuThreadArgument thread_args;
    sysSpuThreadArgumentInitialize(thread_args);
    // PSL1GHT struct uses fields `arg0..arg3` (each u64). The lv2
    // kernel writes `arg0` → SPU r3, `arg1` → r4, `arg2` → r5,
    // `arg3` → r6 — verified in RPCS3 `lv2/sys_spu.cpp:1229..1232`:
    //   thread->gpr[3] = v128::from64(0, args[0]);  // r3 = arg0
    //   thread->gpr[4] = v128::from64(0, args[1]);  // r4 = arg1
    //   ...
    // The SPU's `int main(uint64_t spu_id, uint64_t arg)` reads its
    // first u64 from r3 (= arg0). We stash the 32-bit EA pointer
    // in arg0; the SPU recovers it with `(uint32_t)spu_id` (the low
    // 32 bits of the u64 we wrote).
    thread_args.arg0 = (u64)(uintptr_t)ea_buf;

    sys_spu_thread_t spu_thread_id;
    ret = sysSpuThreadInitialize(&spu_thread_id, group_id, 0, &spu_image,
                                 &thread_attr, &thread_args);
    if (ret) {
        printf("[dma_get_v1] sysSpuThreadInitialize: 0x%08x\n", ret);
        return 1;
    }

    ret = sysSpuThreadGroupStart(group_id);
    if (ret) {
        printf("[dma_get_v1] sysSpuThreadGroupStart: 0x%08x\n", ret);
        return 1;
    }

    u32 cause, status;
    ret = sysSpuThreadGroupJoin(group_id, &cause, &status);
    if (ret) {
        printf("[dma_get_v1] sysSpuThreadGroupJoin: 0x%08x\n", ret);
        return 1;
    }

    // Expected: cause=0x1 (GROUP_EXIT), status=0xDEADA12F (the SPU's
    // OUT_MBOX value computed from the GET'd EA bytes).
    printf("[dma_get_v1] OK cause=0x%x status=0x%x\n",
           (unsigned)cause, (unsigned)status);

    sysSpuThreadGroupDestroy(group_id);
    sysSpuImageClose(&spu_image);

    return 0;
}
